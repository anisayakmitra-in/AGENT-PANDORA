use super::{
    LOCAL_WORKSPACE, create_session, load_config, parse_options, require_config_file,
    session_store, timestamp,
};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::ExecutionController;
use pandora_runtime::executors::WorkspaceRoot;
use pandora_runtime::sessions::SessionStore;
use pandora_runtime::{ApprovalRequest, ApprovalStore, RunStatus, RuntimeError};
use pandora_types::{
    Capability, EventPayload, Operation, PolicyContext, Session, SessionId, TaskIntent, WorkspaceId,
};
use serde_json::{Value, json};

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "session", "gene"],
    )?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage("run requires exactly one task"));
    }
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;
    let approval_store = ApprovalStore::open(config.data_dir().join("sessions.sqlite3"))
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let workspace_id = WorkspaceId::new(LOCAL_WORKSPACE).expect("built-in workspace ID is valid");
    let session = match parsed.value("session") {
        Some(value) => resume_session(&store, value)?,
        None => create_session(&store, &workspace_id)?,
    };
    let workspace = WorkspaceRoot::new(config.workspace_dir()).map_err(|error| {
        let _ = error;
        CliError::configuration(
            "workspace path is invalid",
            json!({"workspace": config.workspace_dir()}),
        )
    })?;
    let mut intent = TaskIntent::new(parsed.positionals[0].clone())
        .map_err(|error| CliError::usage(error.to_string()))?;
    if let Some(gene) = parsed.value("gene") {
        let gene = pandora_types::GeneId::new(gene.to_owned())
            .map_err(|_| CliError::usage("Gene ID is invalid"))?;
        intent = intent.with_gene(gene);
    }
    let policy = PolicyContext::new(
        1,
        [Capability::FilesystemRead, Capability::FilesystemWrite],
        [Operation::Write],
    );
    let controller = ExecutionController::with_policy(workspace, policy);
    let summary = controller
        .run_at(intent, session.clone(), timestamp())
        .map_err(runtime_error)?;
    for event in summary.events() {
        store
            .append_event(
                session.id(),
                session.principal_id(),
                session.tenant_id(),
                session.workspace_id(),
                event,
            )
            .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    }
    let details = run_details(&summary, session.id());
    match summary.status() {
        RunStatus::Completed => Ok(success(
            "run",
            json!({
                "session_id": session.id(),
                "execution_id": summary.execution_id(),
                "harness_id": summary.selected_harness(),
                "gene_id": summary.selected_gene(),
                "status": "completed",
                "output": summary.output().map(output_text),
            }),
            format!(
                "Completed {} with {}",
                summary.selected_gene(),
                session.id()
            ),
        )),
        RunStatus::Denied { reason } => Err(CliError::policy(reason.clone(), details)),
        RunStatus::ApprovalRequired { reason } => {
            let approval = create_approval(
                &approval_store,
                &summary,
                session.id(),
                session.principal_id(),
                &parsed.positionals[0],
            )?;
            let details = add_detail(details, "approval_id", approval.id());
            Err(CliError::approval(reason.clone(), details))
        }
        RunStatus::Failed { code } => Err(CliError::execution(code.clone(), details)),
    }
}

fn resume_session(store: &SessionStore, value: &str) -> Result<Session, CliError> {
    let session_id =
        SessionId::new(value.to_owned()).map_err(|_| CliError::usage("session ID is invalid"))?;
    let (principal, tenant, workspace) = super::session_scope();
    store
        .resume(&session_id, &principal, &tenant, &workspace)
        .map(|snapshot| snapshot.session().clone())
        .map_err(|error| CliError::internal(error.to_string(), json!({})))
}

fn run_details(summary: &pandora_runtime::RunSummary, session_id: &SessionId) -> Value {
    json!({
        "session_id": session_id,
        "execution_id": summary.execution_id(),
        "harness_id": summary.selected_harness(),
        "gene_id": summary.selected_gene(),
        "status": match summary.status() {
            RunStatus::Completed => "completed",
            RunStatus::Denied { .. } => "denied",
            RunStatus::ApprovalRequired { .. } => "approval_required",
            RunStatus::Failed { .. } => "failed",
        },
    })
}

fn create_approval(
    store: &ApprovalStore,
    summary: &pandora_runtime::RunSummary,
    session_id: &SessionId,
    principal_id: &pandora_types::PrincipalId,
    task: &str,
) -> Result<pandora_runtime::PendingApproval, CliError> {
    let request_digest = summary
        .events()
        .iter()
        .find_map(|event| match event.payload() {
            EventPayload::Effect { request_digest, .. } => Some(request_digest.clone()),
            _ => None,
        })
        .ok_or_else(|| CliError::internal("approval request has no effect digest", json!({})))?;
    let expires_at = timestamp().as_unix_seconds().saturating_add(900);
    let request = ApprovalRequest::new(
        format!("approval-{}", summary.execution_id()),
        session_id.clone(),
        summary.execution_id().clone(),
        principal_id.clone(),
        summary.selected_gene().clone(),
        request_digest,
        approval_summary(task),
        1,
        pandora_types::Timestamp::from_unix_seconds(expires_at),
    )
    .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    store
        .create(request)
        .map_err(|error| CliError::internal(error.to_string(), json!({})))
}

fn approval_summary(task: &str) -> String {
    let mut parts = task.splitn(3, ':');
    let action = parts.next().unwrap_or("task");
    let path = parts.next().unwrap_or("workspace");
    format!("coding {action} operation for {path}")
}

fn add_detail(mut details: Value, key: &str, value: impl Into<Value>) -> Value {
    if let Value::Object(object) = &mut details {
        object.insert(key.to_owned(), value.into());
    }
    details
}

fn output_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn runtime_error(error: RuntimeError) -> CliError {
    match error {
        RuntimeError::InvalidIntent(message) => CliError::usage(message),
        RuntimeError::Denied(reason) => CliError::policy(reason, json!({})),
        RuntimeError::ApprovalRequired(reason) => CliError::approval(reason, json!({})),
        RuntimeError::UnsupportedHarness(_) => {
            CliError::execution("requested harness is not supported", json!({}))
        }
        RuntimeError::UnknownGene => {
            CliError::execution("requested Gene is not available", json!({}))
        }
        RuntimeError::Planning(_) => CliError::execution("Gene planning failed", json!({})),
        RuntimeError::Authorization(_) => {
            CliError::execution("effect authorization failed", json!({}))
        }
        RuntimeError::Permit(_) => CliError::execution("effect permit failed", json!({})),
        RuntimeError::Filesystem(_) => {
            CliError::execution("filesystem execution failed", json!({}))
        }
        RuntimeError::Process(_) => CliError::execution("process execution failed", json!({})),
        RuntimeError::UnsupportedOperation(_) => {
            CliError::execution("requested operation is not supported", json!({}))
        }
    }
}
