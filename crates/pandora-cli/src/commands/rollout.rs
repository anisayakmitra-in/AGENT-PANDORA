use super::{load_config, parse_options, require_config_file, session_scope, session_store};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::RolloutSummary;
use pandora_types::{ExecutionId, SessionId};
use serde_json::json;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("rollout requires 'inspect'"))?;
    match subcommand.as_str() {
        "inspect" => inspect(&args[1..]),
        _ => Err(CliError::usage(format!(
            "unknown rollout command '{subcommand}'"
        ))),
    }
}

fn inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "session", "execution"],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "rollout inspect does not accept positional arguments",
        ));
    }
    let session_id = parsed
        .value("session")
        .ok_or_else(|| CliError::usage("rollout inspect requires '--session <id>'"))
        .and_then(|value| {
            SessionId::new(value.to_owned()).map_err(|_| CliError::usage("session ID is invalid"))
        })?;
    let execution_id = parsed
        .value("execution")
        .map(|value| {
            ExecutionId::new(value.to_owned())
                .map_err(|_| CliError::usage("execution ID is invalid"))
        })
        .transpose()?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;
    let (principal, tenant, workspace) = session_scope();
    let snapshot = store
        .resume(&session_id, &principal, &tenant, &workspace)
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let rollouts = snapshot
        .rollouts()
        .iter()
        .filter(|rollout| {
            execution_id
                .as_ref()
                .is_none_or(|id| rollout.execution_id() == id)
        })
        .collect::<Vec<_>>();
    if execution_id.is_some() && rollouts.is_empty() {
        return Err(CliError::execution(
            "rollout execution was not found in the session",
            json!({"session_id": session_id, "execution_id": execution_id}),
        ));
    }
    let count = rollouts.len();
    Ok(success(
        "rollout inspect",
        json!({
            "session_id": session_id,
            "execution_id": execution_id,
            "count": count,
            "rollouts": rollouts.iter().map(|rollout| rollout_value(rollout)).collect::<Vec<_>>(),
            "durability": "session-store",
        }),
        format!("Inspected {count} durable rollout summary(s) for {session_id}"),
    ))
}

fn rollout_value(rollout: &RolloutSummary) -> serde_json::Value {
    json!({
        "session_id": rollout.session_id(),
        "execution_id": rollout.execution_id(),
        "attempt": rollout.attempt(),
        "projection_version": rollout.projection_version(),
        "record_count": rollout.record_count(),
        "context_manifest_digest": rollout.context_manifest_digest(),
        "final_digest": rollout.final_digest(),
        "recorded_at": rollout.recorded_at().as_unix_seconds(),
    })
}
