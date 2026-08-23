use super::run::{
    AgentOptions, configured_harnesses, execute_agent_core, require_runnable_harness,
};
use crate::output::CliError;
use pandora_runtime::config::{ProviderProfile, RuntimeConfig};
use pandora_runtime::executors::WorkspaceRoot;
use pandora_runtime::sessions::SessionStore;
use pandora_runtime::{AgentRunControl, ApprovalStore, ClaimedSubagent, ExecutionController};
use pandora_types::{Capability, Operation, PolicyContext, Session, WorkspaceId, hash_artifact};
use serde_json::{Value, json};
use std::process::{Command, Stdio};

pub(crate) struct TrustedSubagentRun<'a> {
    pub config: &'a RuntimeConfig,
    pub record: &'a ClaimedSubagent,
    pub store: &'a SessionStore,
    pub approval_store: &'a ApprovalStore,
    pub control: &'a dyn AgentRunControl,
}

pub(crate) fn execute_trusted_subagent(input: TrustedSubagentRun<'_>) -> Result<Value, CliError> {
    verify_bindings(input.config, input.record)?;
    let workspace = verified_worktree(input.record)?;
    let request = input.record.request();
    let harness = request.harness();
    let harnesses = configured_harnesses(
        input.config,
        harness.map(|binding| binding.harness_id().as_str()),
        harness.map(|binding| binding.version()),
    )?;
    require_runnable_harness(
        &harnesses,
        harness.map(|binding| binding.harness_id().as_str()),
    )?;
    let child_workspace = child_workspace_id(input.record)?;
    let session = Session::new(
        input.record.child_session_id().clone(),
        input.record.scope().principal_id().clone(),
        input.record.scope().tenant_id().clone(),
        child_workspace,
        super::timestamp(),
    );
    input
        .store
        .create(&session)
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let policy = PolicyContext::new(
        1,
        [
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
            Capability::ProcessExecute,
            Capability::ProviderInvoke,
        ],
        [Operation::Write, Operation::Execute],
    );
    let controller = ExecutionController::with_policy_and_harnesses(workspace, policy, harnesses);
    let result = execute_agent_core(
        input.config,
        &controller,
        &session,
        input.store,
        input.approval_store,
        AgentOptions {
            task: request.task(),
            task_class: "general",
            history: Vec::new(),
            approval_id: None,
            model_override: None,
            provider_name: request.provider_profile(),
            optimization: None,
            max_turns: request.budgets().max_turns(),
            max_tool_calls: request.budgets().max_tool_calls(),
            control: Some(input.control),
        },
    )?;
    Ok(result.data)
}

fn verify_bindings(config: &RuntimeConfig, record: &ClaimedSubagent) -> Result<(), CliError> {
    let provider_digest = record
        .request()
        .provider_profile()
        .map(|name| provider_binding_digest(config, name))
        .transpose()?;
    let harness_digest = record.request().harness().map(|binding| {
        let canonical = format!(
            "harness-binding-v1\0{}\0{}\0",
            binding.harness_id(),
            binding.version()
        );
        format!("harness-{}", hash_artifact(canonical.as_bytes()))
    });
    if record.provider_binding_digest() != provider_digest.as_deref()
        || record.harness_binding_digest() != harness_digest.as_deref()
    {
        return Err(binding_changed(record));
    }
    Ok(())
}

fn provider_binding_digest(config: &RuntimeConfig, name: &str) -> Result<String, CliError> {
    let profile = config
        .provider_profile(name)
        .ok_or_else(|| binding_changed_message("the bound provider profile is unavailable"))?;
    let mut canonical = provider_profile_binding("provider-binding-v1", profile);
    if let Some(fallback_name) = profile.fallback_provider() {
        let fallback = config.provider_profile(fallback_name).ok_or_else(|| {
            binding_changed_message("the bound fallback provider profile is unavailable")
        })?;
        canonical.push_str(&provider_profile_binding("fallback", fallback));
    }
    Ok(format!("provider-{}", hash_artifact(canonical.as_bytes())))
}

fn provider_profile_binding(prefix: &str, profile: &ProviderProfile) -> String {
    format!(
        "{prefix}\0{}\0{}\0{}\0{}\0",
        profile.name(),
        profile.base_url(),
        profile.model(),
        profile.api_key_env(),
    )
}

fn verified_worktree(record: &ClaimedSubagent) -> Result<WorkspaceRoot, CliError> {
    let output = Command::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(record.worktree_path())
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| worktree_changed(record))?;
    let head = String::from_utf8(output.stdout).map_err(|_| worktree_changed(record))?;
    if !output.status.success()
        || !head
            .trim()
            .eq_ignore_ascii_case(record.request().exact_commit())
    {
        return Err(worktree_changed(record));
    }
    WorkspaceRoot::new(record.worktree_path()).map_err(|_| worktree_changed(record))
}

fn child_workspace_id(record: &ClaimedSubagent) -> Result<WorkspaceId, CliError> {
    let digest = hash_artifact(record.id().as_str().as_bytes());
    let digest = digest
        .strip_prefix("sha256:")
        .expect("hash_artifact returns a SHA-256 digest");
    let workspace = WorkspaceId::new(format!("subagent-{digest}"))
        .map_err(|_| CliError::internal("could not derive the child workspace ID", json!({})))?;
    if &workspace == record.scope().workspace_id() {
        return Err(CliError::internal(
            "child workspace ID conflicts with the parent workspace",
            json!({"subagent_id": record.id()}),
        ));
    }
    Ok(workspace)
}

fn binding_changed(record: &ClaimedSubagent) -> CliError {
    trusted_execution_error(
        "subagent_binding_changed",
        "subagent provider or Harness binding changed after claim",
        json!({"subagent_id": record.id()}),
    )
}

fn binding_changed_message(message: &str) -> CliError {
    trusted_execution_error("subagent_binding_changed", message, json!({}))
}

fn worktree_changed(record: &ClaimedSubagent) -> CliError {
    trusted_execution_error(
        "subagent_worktree_changed",
        "subagent worktree no longer resolves to the bound commit",
        json!({
            "subagent_id": record.id(),
            "worktree": record.worktree_path(),
            "exact_commit": record.request().exact_commit(),
        }),
    )
}

fn trusted_execution_error(
    code: &'static str,
    message: impl Into<String>,
    details: Value,
) -> CliError {
    CliError {
        code,
        message: message.into(),
        details,
        exit_code: 50,
    }
}
