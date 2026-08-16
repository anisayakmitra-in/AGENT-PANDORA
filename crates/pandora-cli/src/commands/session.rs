use super::{load_config, parse_options, require_config_file, session_scope, session_store};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::{ObservabilityEngine, sessions::SessionSnapshot};
use pandora_types::{EventType, ObservabilitySample, SessionId};
use serde_json::json;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("session requires 'list', 'resume', or 'inspect'"))?;
    match subcommand.as_str() {
        "list" => list(&args[1..]),
        "resume" => resume(&args[1..]),
        "inspect" => inspect(&args[1..]),
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
            "l1_evidence_count": snapshot.l1_evidence_count(),
            "events": events,
        }),
        format!(
            "Resumed {} with {event_count} event(s)",
            snapshot.session().id()
        ),
    ))
}

fn inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "session inspect requires exactly one session ID",
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
    let last_event_type = snapshot.events().last().map(|event| event.event_type());
    let last_event_timestamp = snapshot
        .recorded_events()
        .last()
        .and_then(|event| event.recorded_at())
        .map(|value| value.as_unix_seconds());
    let event_count = snapshot.events().len();
    Ok(success(
        "session inspect",
        json!({
            "session_id": snapshot.session().id(),
            "metadata": {
                "principal_id": snapshot.session().principal_id(),
                "tenant_id": snapshot.session().tenant_id(),
                "workspace_id": snapshot.session().workspace_id(),
                "created_at": snapshot.session().created_at().as_unix_seconds(),
            },
            "event_count": event_count,
            "agent_message_count": snapshot.agent_messages().len(),
            "l1_evidence_count": snapshot.l1_evidence_count(),
            "last_event_timestamp": last_event_timestamp,
            "last_event_type": last_event_type,
            "observability": session_observability(&snapshot)?,
        }),
        format!(
            "Inspected {} with {event_count} event(s)",
            snapshot.session().id()
        ),
    ))
}

fn session_observability(snapshot: &SessionSnapshot) -> Result<serde_json::Value, CliError> {
    let engine = ObservabilityEngine::new();
    let mut uninstrumented_event_count = 0usize;

    for recorded in snapshot.recorded_events() {
        let Some(recorded_at) = recorded.recorded_at() else {
            uninstrumented_event_count = uninstrumented_event_count.saturating_add(1);
            continue;
        };
        let event = recorded.event();
        let mut sample = ObservabilitySample::new(
            snapshot.session().id().as_str(),
            event.event_id().as_str(),
            None,
            recorded.sequence(),
            event.clone(),
            recorded_at,
            0,
            0,
            0,
            None,
            None,
        )
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
        if let Some(code) = observability_error_code(event.event_type()) {
            sample = sample
                .with_error_code(code)
                .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
        }
        engine.record(sample).map_err(|_| {
            CliError::internal("session observability projection is invalid", json!({}))
        })?;
    }

    let projection = engine.snapshot();
    let span_count = projection
        .traces()
        .iter()
        .map(|trace| trace.spans().len())
        .sum::<usize>();
    let reliability_bps = if span_count > 0 {
        Some(projection.reliability_bps())
    } else {
        None
    };
    Ok(json!({
        "trace_count": projection.traces().len(),
        "span_count": span_count,
        "uninstrumented_event_count": uninstrumented_event_count,
        "error_count": projection.error_count(),
        "reliability_bps": reliability_bps,
    }))
}

fn observability_error_code(event_type: EventType) -> Option<&'static str> {
    match event_type {
        EventType::ExecutionFailed => Some("execution_failed"),
        EventType::PolicyDenied => Some("policy_denied"),
        _ => None,
    }
}
