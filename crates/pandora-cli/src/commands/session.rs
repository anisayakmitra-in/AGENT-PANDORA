use super::run::evaluation_receipt_json;
use super::{
    load_config, parse_options, require_config_file, session_scope, session_store, timestamp,
};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::{ObservabilityEngine, SessionJournalEngine, sessions::SessionSnapshot};
use pandora_types::{EventType, ObservabilitySample, SessionId};
use serde_json::json;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage(
            "session requires 'list', 'resume', 'inspect', 'events', 'rewind', 'fork', 'diff', 'prune', or 'vacuum'",
        )
    })?;
    match subcommand.as_str() {
        "list" => list(&args[1..]),
        "resume" => resume(&args[1..]),
        "inspect" => inspect(&args[1..]),
        "events" => events(&args[1..]),
        "rewind" => rewind(&args[1..]),
        "fork" => fork(&args[1..]),
        "diff" => diff(&args[1..]),
        "prune" => prune(&args[1..]),
        "vacuum" => vacuum(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown session command '{unknown}'"
        ))),
    }
}

/// Bounded page of session events.
///
/// The store already pages with a sequence cursor; this exposes it. Paging
/// rather than loading the whole log matters because `session resume` and
/// `session inspect` deserialize every event, so a long session costs
/// unbounded memory on a read that is only asking for the next slice.
fn events(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "after-sequence", "limit"],
    )?;
    // Validate the page bounds before touching the store. A bad `--limit` is a
    // usage error, and reporting it as an empty page would look like a session
    // with no events.
    let limit = parse_event_limit(parsed.value("limit"))?;
    let after_sequence = parsed
        .value("after-sequence")
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|_| CliError::usage("after-sequence must be a non-negative integer"))
        })
        .transpose()?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "session events requires exactly one session ID",
        ));
    }
    let session_id = SessionId::new(parsed.positionals[0].clone())
        .map_err(|_| CliError::usage("session ID is invalid"))?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;
    let (principal, tenant, workspace) = session_scope();
    let page = store
        .event_page(
            &session_id,
            &principal,
            &tenant,
            &workspace,
            after_sequence,
            limit,
        )
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let events = serde_json::to_value(page.events())
        .map_err(|_| CliError::internal("could not serialize session events", json!({})))?;
    let count = page.events().len();
    let next_sequence = page.next_sequence();
    Ok(success(
        "session events",
        json!({
            "session_id": session_id,
            "after_sequence": after_sequence,
            "limit": limit,
            "count": count,
            "has_more": next_sequence.is_some(),
            "next_sequence": next_sequence,
            "events": events,
        }),
        match next_sequence {
            Some(next) => format!(
                "{count} event(s) for {}; continue with --after-sequence {next}",
                session_id
            ),
            None => format!("{count} event(s) for {}; end of log", session_id),
        },
    ))
}

const DEFAULT_EVENT_LIMIT: u16 = 50;
const MAX_EVENT_LIMIT: u16 = 256;

fn parse_event_limit(value: Option<&str>) -> Result<u16, CliError> {
    let limit = value
        .map(|value| {
            value
                .parse::<u16>()
                .map_err(|_| CliError::usage("session events limit must be an integer"))
        })
        .transpose()?
        .unwrap_or(DEFAULT_EVENT_LIMIT);
    if limit == 0 || limit > MAX_EVENT_LIMIT {
        return Err(CliError::usage(format!(
            "session events limit must be between 1 and {MAX_EVENT_LIMIT}"
        )));
    }
    Ok(limit)
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
            "evaluation_count": snapshot.evaluations().len(),
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
            "evaluations": {
                "count": snapshot.evaluations().len(),
                "latest": snapshot.evaluations().last().map(evaluation_receipt_json),
            },
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

fn rewind(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "step", "yes"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "session rewind requires exactly one session ID positional argument",
        ));
    }
    let session_id = SessionId::new(parsed.positionals[0].clone())
        .map_err(|_| CliError::usage("session ID is invalid"))?;

    let step = parsed
        .value("step")
        .ok_or_else(|| CliError::usage("session rewind requires '--step <n>'"))?
        .parse::<usize>()
        .map_err(|_| CliError::usage("step must be a non-negative integer"))?;

    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;

    let result =
        SessionJournalEngine::rewind(&store, &session_id, step, Some(config.workspace_dir()))
            .map_err(|e| CliError::internal(e.to_string(), json!({})))?;

    let mut val = serde_json::to_value(&result)
        .map_err(|_| CliError::internal("could not serialize rewind result", json!({})))?;
    if let serde_json::Value::Object(ref mut map) = val {
        map.insert(
            "confirmed".to_string(),
            serde_json::json!(parsed.values.contains_key("yes")),
        );
    }

    Ok(success(
        "session rewind",
        val,
        format!(
            "Session {} rewound from step {} to step {} (target digest: {})",
            result.session_id,
            result.rolled_back_from_step,
            result.rolled_back_to_step,
            result.target_digest
        ),
    ))
}

fn fork(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "step", "prompt"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "session fork requires exactly one session ID positional argument",
        ));
    }
    let session_id = SessionId::new(parsed.positionals[0].clone())
        .map_err(|_| CliError::usage("session ID is invalid"))?;

    let step = parsed
        .value("step")
        .ok_or_else(|| CliError::usage("session fork requires '--step <n>'"))?
        .parse::<usize>()
        .map_err(|_| CliError::usage("step must be a non-negative integer"))?;

    let prompt = parsed
        .value("prompt")
        .ok_or_else(|| CliError::usage("session fork requires '--prompt \"<text>\"'"))?;

    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;
    let (principal, tenant, workspace) = session_scope();
    let now = timestamp();

    let result = SessionJournalEngine::fork(
        &store,
        &session_id,
        step,
        prompt,
        config.workspace_dir(),
        &principal,
        &tenant,
        &workspace,
        now,
    )
    .map_err(|e| CliError::internal(e.to_string(), json!({})))?;

    let val = serde_json::to_value(&result)
        .map_err(|_| CliError::internal("could not serialize fork result", json!({})))?;

    Ok(success(
        "session fork",
        val,
        format!(
            "Forked session {} from parent {} at step {} (branch: {})",
            result.forked_session_id, result.parent_session_id, result.at_step, result.git_branch
        ),
    ))
}

fn diff(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "step-a", "step-b"],
    )?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "session diff requires exactly one session ID positional argument",
        ));
    }
    let session_id = SessionId::new(parsed.positionals[0].clone())
        .map_err(|_| CliError::usage("session ID is invalid"))?;

    let step_a = parsed
        .value("step-a")
        .ok_or_else(|| CliError::usage("session diff requires '--step-a <m>'"))?
        .parse::<usize>()
        .map_err(|_| CliError::usage("step-a must be a non-negative integer"))?;

    let step_b = parsed
        .value("step-b")
        .ok_or_else(|| CliError::usage("session diff requires '--step-b <n>'"))?
        .parse::<usize>()
        .map_err(|_| CliError::usage("step-b must be a non-negative integer"))?;

    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;

    let result = SessionJournalEngine::diff(&store, &session_id, step_a, step_b)
        .map_err(|e| CliError::internal(e.to_string(), json!({})))?;

    let val = serde_json::to_value(&result)
        .map_err(|_| CliError::internal("could not serialize diff result", json!({})))?;

    Ok(success(
        "session diff",
        val,
        format!(
            "Diff for session {} between step {} ({}) and step {} ({})",
            result.session_id, result.step_a, result.digest_a, result.step_b, result.digest_b
        ),
    ))
}

fn parse_duration_seconds(raw: &str) -> Result<u64, CliError> {
    if let Some(days) = raw.strip_suffix('d') {
        let d = days
            .parse::<u64>()
            .map_err(|_| CliError::usage("invalid days format; expected e.g. '90d'"))?;
        Ok(d * 86400)
    } else if let Some(hours) = raw.strip_suffix('h') {
        let h = hours
            .parse::<u64>()
            .map_err(|_| CliError::usage("invalid hours format; expected e.g. '24h'"))?;
        Ok(h * 3600)
    } else if let Some(secs) = raw.strip_suffix('s') {
        let s = secs
            .parse::<u64>()
            .map_err(|_| CliError::usage("invalid seconds format; expected e.g. '300s'"))?;
        Ok(s)
    } else {
        raw.parse::<u64>().map_err(|_| {
            CliError::usage("invalid duration; expected e.g. '90d', '24h', or seconds")
        })
    }
}

fn prune(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "older-than",
            "dry-run",
            "yes",
        ],
    )?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;

    let older_than_secs = match parsed.value("older-than") {
        Some(val) => parse_duration_seconds(val)?,
        None => 90 * 86400,
    };

    let dry_run = parsed.has_flag("dry-run");
    let confirmed = parsed.has_flag("yes");

    if !dry_run && !confirmed {
        return Err(CliError::usage(
            "session prune requires '--yes' to execute, or '--dry-run' to preview candidates",
        ));
    }

    let (principal_id, tenant_id, workspace_id) = session_scope();
    let report = store
        .prune_sessions(
            &principal_id,
            &tenant_id,
            &workspace_id,
            older_than_secs,
            dry_run,
        )
        .map_err(|e| CliError::internal(e.to_string(), json!({})))?;

    let val = serde_json::to_value(&report)
        .map_err(|_| CliError::internal("could not serialize prune report", json!({})))?;

    let message = if dry_run {
        format!(
            "Dry-run prune: {} candidate session(s) older than {}s (out of {} examined)",
            report.pruned_session_ids.len(),
            report.older_than_seconds,
            report.sessions_examined
        )
    } else {
        format!(
            "Pruned {} session(s) and reclaimed {} event(s) older than {}s",
            report.sessions_pruned, report.events_reclaimed, report.older_than_seconds
        )
    };

    Ok(success("session prune", val, message))
}

fn vacuum(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "threshold",
            "force",
            "analyze",
        ],
    )?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;

    if parsed.has_flag("analyze") {
        let stats = store
            .storage_stats()
            .map_err(|e| CliError::internal(e.to_string(), json!({})))?;
        let val = serde_json::to_value(&stats)
            .map_err(|_| CliError::internal("could not serialize storage stats", json!({})))?;
        let text = format!(
            "Total Size:     {} bytes ({} pages x {} bytes)\nReclaimable:    {} bytes ({} free pages, {:.2}%)\nVacuum Advice:  {}",
            stats.total_size_bytes,
            stats.total_pages,
            stats.page_size_bytes,
            stats.reclaimable_bytes,
            stats.free_pages,
            stats.reclaimable_percent,
            if stats.reclaimable_percent >= 25.0 {
                "Recommended (reclaimable space >= 25%)"
            } else {
                "Optimal (fragmentation is low)"
            }
        );
        return Ok(success("session vacuum analyze", val, text));
    }

    let threshold = if parsed.has_flag("force") {
        Some(0.0)
    } else if let Some(raw) = parsed.value("threshold") {
        let t = raw
            .parse::<f64>()
            .map_err(|_| CliError::usage("threshold must be a valid float percentage e.g. 25.0"))?;
        Some(t)
    } else {
        Some(25.0)
    };

    let report = store
        .vacuum_database(threshold)
        .map_err(|e| CliError::internal(e.to_string(), json!({})))?;

    let val = serde_json::to_value(&report)
        .map_err(|_| CliError::internal("could not serialize vacuum report", json!({})))?;

    let text = if report.vacuum_executed {
        format!(
            "Vacuum executed: reclaimed {} bytes (Size: {} -> {} bytes, Reclaimable: {:.1}% -> {:.1}%)",
            report.reclaimed_bytes,
            report.pre_size_bytes,
            report.post_size_bytes,
            report.pre_reclaimable_pct,
            report.post_reclaimable_pct
        )
    } else {
        format!("Vacuum skipped: {}", report.reason)
    };

    Ok(success("session vacuum", val, text))
}
