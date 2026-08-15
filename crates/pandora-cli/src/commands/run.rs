use super::{
    LOCAL_WORKSPACE, create_session, load_config, parse_options, require_config_file,
    session_store, timestamp,
};
use crate::output::{CliError, CommandResult, success};
use pandora_provider::{
    ChatMessage, FallbackPolicy, HttpProvider, ModelRequest, Provider, ProviderManifest,
    TraceMetadata, parse_and_validate,
};
use pandora_runtime::ExecutionController;
use pandora_runtime::config::RuntimeConfig;
use pandora_runtime::executors::WorkspaceRoot;
use pandora_runtime::sessions::SessionStore;
use pandora_runtime::{
    AgentLoop, AgentLoopError, ApprovalRequest, ApprovalStore, RunStatus, RuntimeError,
};
use pandora_types::{
    Capability, EventPayload, HarnessId, Operation, PolicyContext, Session, SessionId, TaskIntent,
    WorkspaceId,
};
use serde_json::{Value, json};
use std::time::Duration;

const MAX_PLANNED_TASK_BYTES: usize = 8 * 1024;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "session",
            "approval",
            "harness",
            "gene",
            "model",
            "plan",
            "agent",
        ],
    )?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage("run requires exactly one task"));
    }
    if parsed.value("approval").is_some()
        && (parsed.value("plan").is_some() || parsed.value("model").is_some())
    {
        return Err(CliError::usage(
            "--approval cannot be combined with --plan or --model",
        ));
    }
    if parsed.value("agent").is_some()
        && (parsed.value("approval").is_some()
            || parsed.value("plan").is_some()
            || parsed.value("harness").is_some()
            || parsed.value("gene").is_some())
    {
        return Err(CliError::usage(
            "--agent cannot be combined with --approval, --plan, --harness, or --gene",
        ));
    }
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;
    let approval_store = ApprovalStore::open(config.data_dir().join("sessions.sqlite3"))
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let workspace_id = WorkspaceId::new(LOCAL_WORKSPACE).expect("built-in workspace ID is valid");
    let principal = super::session_scope().0;
    let approval = parsed
        .value("approval")
        .map(|id| {
            approval_store
                .inspect(id, &principal)
                .map_err(approval_error)
        })
        .transpose()?;
    let session = match approval.as_ref() {
        Some(approval) => {
            if let Some(value) = parsed.value("session")
                && value != approval.session_id().as_str()
            {
                return Err(CliError::policy(
                    "approval is bound to a different session",
                    json!({}),
                ));
            }
            resume_session(&store, approval.session_id().as_str())?
        }
        None => match parsed.value("session") {
            Some(value) => resume_session(&store, value)?,
            None => create_session(&store, &workspace_id)?,
        },
    };
    let workspace = WorkspaceRoot::new(config.workspace_dir()).map_err(|error| {
        let _ = error;
        CliError::configuration(
            "workspace path is invalid",
            json!({"workspace": config.workspace_dir()}),
        )
    })?;
    let (task, planning_model) = if parsed.value("plan").is_some() {
        plan_task(
            &config,
            &session,
            &parsed.positionals[0],
            parsed.value("model"),
        )?
    } else {
        (parsed.positionals[0].clone(), None)
    };
    let policy = PolicyContext::new(
        1,
        [Capability::FilesystemRead, Capability::FilesystemWrite],
        [Operation::Write],
    );
    let controller = ExecutionController::with_policy(workspace, policy);
    if parsed.value("agent").is_some() {
        return execute_agent(
            &config,
            &controller,
            &session,
            &store,
            &approval_store,
            &task,
            parsed.value("model"),
        );
    }
    let mut intent =
        TaskIntent::new(task.clone()).map_err(|error| CliError::usage(error.to_string()))?;
    if let Some(harness) = parsed.value("harness") {
        let harness = match harness {
            "coding" => "coding-domain",
            value => value,
        };
        let harness = HarnessId::new(harness.to_owned())
            .map_err(|_| CliError::usage("Harness ID is invalid"))?;
        intent = intent.with_harness(harness);
    }
    if let Some(gene) = parsed.value("gene") {
        let gene = pandora_types::GeneId::new(gene.to_owned())
            .map_err(|_| CliError::usage("Gene ID is invalid"))?;
        intent = intent.with_gene(gene);
    }
    let summary = match parsed.value("approval") {
        Some(approval_id) => controller
            .run_with_approval(
                intent,
                session.clone(),
                &approval_store,
                approval_id,
                timestamp(),
            )
            .map_err(runtime_error)?,
        None => controller
            .run_at(intent, session.clone(), timestamp())
            .map_err(runtime_error)?,
    };
    append_events(&store, &session, summary.events())?;
    let details = run_details(&summary, session.id(), planning_model.as_deref());
    match summary.status() {
        RunStatus::Completed => {
            let data = add_planning(
                json!({
                "session_id": session.id(),
                "execution_id": summary.execution_id(),
                "harness_id": summary.selected_harness(),
                "gene_id": summary.selected_gene(),
                "status": "completed",
                "output": summary.output().map(output_text),
                }),
                planning_model.as_deref(),
            );
            Ok(success(
                "run",
                data,
                format!(
                    "Completed {} with {}",
                    summary.selected_gene(),
                    session.id()
                ),
            ))
        }
        RunStatus::Denied { reason } => Err(CliError::policy(reason.clone(), details)),
        RunStatus::ApprovalRequired { reason } => {
            let approval = create_approval(
                &approval_store,
                &summary,
                session.id(),
                session.principal_id(),
                &task,
            )?;
            let details = add_detail(details, "approval_id", approval.id());
            Err(CliError::approval(reason.clone(), details))
        }
        RunStatus::Failed { code } => Err(CliError::execution(code.clone(), details)),
    }
}

fn execute_agent(
    config: &RuntimeConfig,
    controller: &ExecutionController,
    session: &Session,
    store: &SessionStore,
    approval_store: &ApprovalStore,
    task: &str,
    model_override: Option<&str>,
) -> Result<CommandResult, CliError> {
    let base_url = config.provider_url().ok_or_else(|| {
        CliError::configuration(
            "agent mode requires a configured provider; run 'pandora provider set' first",
            json!({"config_path": config.config_path()}),
        )
    })?;
    let model = model_override
        .or(config.provider_model())
        .unwrap_or("default");
    let manifest = ProviderManifest::new(
        "openai-compatible",
        "OpenAI-compatible",
        base_url,
        model,
        "PANDORA_PROVIDER_API_KEY",
    )
    .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    let provider = HttpProvider::from_environment(manifest)
        .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    let loop_engine =
        AgentLoop::new(8, 16).map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let result = loop_engine.run(&provider, controller, session.clone(), task, timestamp());
    match result {
        Ok(summary) => {
            let run_count = summary.runs().len();
            for run in summary.runs() {
                append_events(store, session, run.events())?;
            }
            Ok(success(
                "run",
                json!({
                    "agent": true,
                    "session_id": session.id(),
                    "status": "completed",
                    "turns": summary.turns(),
                    "tool_calls": summary.tool_calls(),
                    "output": summary.final_text(),
                    "usage": usage_json(summary.usage()),
                    "runs": run_count,
                }),
                format!("Completed agent task in {}", session.id()),
            ))
        }
        Err(AgentLoopError::ApprovalRequired { reason, summary }) => {
            let run = summary.runs().last().ok_or_else(|| {
                CliError::internal("approval request has no execution summary", json!({}))
            })?;
            for current in summary.runs() {
                append_events(store, session, current.events())?;
            }
            let approval = create_approval(
                approval_store,
                run,
                session.id(),
                session.principal_id(),
                task,
            )?;
            Err(CliError::approval(
                reason,
                json!({
                    "agent": true,
                    "session_id": session.id(),
                    "turns": summary.turns(),
                    "tool_calls": summary.tool_calls(),
                    "approval_id": approval.id(),
                }),
            ))
        }
        Err(error) => Err(agent_error(error)),
    }
}

fn append_events(
    store: &SessionStore,
    session: &Session,
    events: &[pandora_types::RuntimeEvent],
) -> Result<(), CliError> {
    for event in events {
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
    Ok(())
}

fn usage_json(usage: &pandora_provider::TokenUsage) -> Value {
    json!({
        "prompt_tokens": usage.prompt_tokens(),
        "completion_tokens": usage.completion_tokens(),
        "total_tokens": usage.total_tokens(),
    })
}

fn agent_error(error: AgentLoopError) -> CliError {
    match error {
        AgentLoopError::InvalidBudget | AgentLoopError::InvalidTask => {
            CliError::usage(error.to_string())
        }
        AgentLoopError::Provider(error) => CliError::provider(error.to_string(), json!({})),
        AgentLoopError::EmptyResponse
        | AgentLoopError::ToolBudgetExceeded
        | AgentLoopError::TurnBudgetExceeded => CliError::execution(error.to_string(), json!({})),
        AgentLoopError::Execution(error) => runtime_error(error),
        AgentLoopError::ApprovalRequired { reason, .. } => CliError::approval(reason, json!({})),
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

fn run_details(
    summary: &pandora_runtime::RunSummary,
    session_id: &SessionId,
    planning_model: Option<&str>,
) -> Value {
    add_planning(
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
        }),
        planning_model,
    )
}

fn plan_task(
    config: &RuntimeConfig,
    session: &Session,
    request: &str,
    model_override: Option<&str>,
) -> Result<(String, Option<String>), CliError> {
    let base_url = config.provider_url().ok_or_else(|| {
        CliError::configuration(
            "planning requires a configured provider; run 'pandora provider set' first",
            json!({"config_path": config.config_path()}),
        )
    })?;
    let model = model_override
        .or(config.provider_model())
        .unwrap_or("default");
    let manifest = ProviderManifest::new(
        "openai-compatible",
        "OpenAI-compatible",
        base_url,
        model,
        "PANDORA_PROVIDER_API_KEY",
    )
    .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    let provider = HttpProvider::from_environment(manifest.clone())
        .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    let messages = vec![
        ChatMessage::system(
            "You are Pandora's planning layer. Return only JSON with one string field named task. "
                .to_owned()
                + "The task must use exactly one bounded format: read:<path>, search:<query>, "
                + "review:<path>, patch:<path>:<content>, or verify. Do not return shell commands, "
                + "credentials, additional fields, or paths outside the workspace.",
        )
        .map_err(|error| CliError::provider(error.to_string(), json!({})))?,
        ChatMessage::user(request.to_owned())
            .map_err(|error| CliError::provider(error.to_string(), json!({})))?,
    ];
    let request = ModelRequest::new(
        manifest.id().clone(),
        manifest.default_model().clone(),
        messages,
    )
    .and_then(|request| request.with_max_output_tokens(256))
    .and_then(|request| request.with_timeout(Duration::from_secs(60)))
    .map(|request| {
        request.with_trace_metadata(TraceMetadata::new().with_session_id(session.id().clone()))
    })
    .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    let response = provider
        .complete(request)
        .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    if !response.tool_calls().is_empty() {
        return Err(CliError::provider(
            "planner returned tool calls; planning cannot execute tools",
            json!({}),
        ));
    }
    let validated = parse_and_validate(
        response.text(),
        &json!({
            "type": "object",
            "required": ["task"],
            "properties": {"task": {"type": "string"}},
            "additionalProperties": false,
        }),
        None,
        FallbackPolicy::Reject,
    )
    .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    let planned = validated
        .value()
        .get("task")
        .and_then(Value::as_str)
        .filter(|task| !task.trim().is_empty())
        .ok_or_else(|| CliError::provider("planner returned an empty task", json!({})))?;
    if planned.len() > MAX_PLANNED_TASK_BYTES {
        return Err(CliError::provider(
            "planner task exceeds the size limit",
            json!({}),
        ));
    }
    Ok((
        planned.to_owned(),
        Some(manifest.default_model().as_str().to_owned()),
    ))
}

fn add_planning(mut details: Value, model: Option<&str>) -> Value {
    if let Some(model) = model
        && let Value::Object(object) = &mut details
    {
        object.insert(
            "planning".to_owned(),
            json!({"enabled": true, "model": model}),
        );
    }
    details
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
        RuntimeError::Approval(error) => approval_error(error),
        RuntimeError::ApprovalNotRequired => CliError::policy(
            "the supplied approval is not required by the active policy",
            json!({}),
        ),
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

fn approval_error(error: pandora_runtime::ApprovalError) -> CliError {
    match error {
        pandora_runtime::ApprovalError::Expired | pandora_runtime::ApprovalError::Terminal => {
            CliError::approval(error.to_string(), json!({}))
        }
        pandora_runtime::ApprovalError::ScopeMismatch
        | pandora_runtime::ApprovalError::DigestMismatch => {
            CliError::policy(error.to_string(), json!({}))
        }
        other => CliError::internal(other.to_string(), json!({})),
    }
}
