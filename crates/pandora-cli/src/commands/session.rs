use super::{load_config, parse_options, require_config_file, session_scope, session_store};
use crate::output::{CliError, CommandResult, success};
use pandora_types::SessionId;
use serde_json::json;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("session requires 'list' or 'resume'"))?;
    match subcommand.as_str() {
        "list" => list(&args[1..]),
        "resume" => resume(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown session command '{unknown}'"
        ))),
    }
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;
    let (principal, tenant, workspace) = session_scope();
    let sessions = store
        .list(&principal, &tenant, &workspace)
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?
        .into_iter()
        .map(|session| {
            json!({
                "id": session.id(),
                "principal_id": session.principal_id(),
                "tenant_id": session.tenant_id(),
                "workspace_id": session.workspace_id(),
                "created_at": session.created_at().as_unix_seconds(),
            })
        })
        .collect::<Vec<_>>();
    let count = sessions.len();
    Ok(success(
        "session list",
        json!({"sessions": sessions}),
        format!("{count} session(s)"),
    ))
}

fn resume(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "session resume requires exactly one session ID",
        ));
    }
    let session_id = SessionId::new(parsed.positionals[0].clone())
        .map_err(|_| CliError::usage("session ID is invalid"))?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;
    let (principal, tenant, workspace) = session_scope();
    let snapshot = store
        .resume(&session_id, &principal, &tenant, &workspace)
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let events = serde_json::to_value(snapshot.events())
        .map_err(|_| CliError::internal("could not serialize session events", json!({})))?;
    let event_count = snapshot.events().len();
    Ok(success(
        "session resume",
        json!({
            "session_id": snapshot.session().id(),
            "event_count": event_count,
            "agent_message_count": snapshot.agent_messages().len(),
            "events": events,
        }),
        format!(
            "Resumed {} with {event_count} event(s)",
            snapshot.session().id()
        ),
    ))
}
