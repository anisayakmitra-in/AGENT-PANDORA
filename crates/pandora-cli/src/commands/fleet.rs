use super::{load_config, parse_options, session_scope, timestamp};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::{
    FleetBudget, FleetEngine, FleetError, FleetLease, FleetLeaseState, FleetNode, FleetSupervisor,
    FleetSupervisorState, JobStore, OrchestrationStore,
};
use pandora_types::JobStatus;
use serde_json::{Value, json};
use std::collections::BTreeMap;

const DEFAULT_SUPERVISOR_STALE_AFTER_SECONDS: u64 = 30;
const MAX_OPERATIONS_DETAIL_RECORDS: usize = 64;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("fleet requires 'dashboard', 'list', 'register', 'dispatch', 'lease', 'renew', 'release', 'expire', 'supervisor', 'quarantine', 'revoke', or 'kill'"))?;
    match subcommand.as_str() {
        "dashboard" => dashboard(&args[1..]),
        "list" => list(&args[1..]),
        "register" => register(&args[1..]),
        "dispatch" => dispatch(&args[1..]),
        "lease" => lease(&args[1..]),
        "renew" => renew(&args[1..]),
        "supervisor" => supervisor(&args[1..]),
        "release" => release(&args[1..]),
        "expire" => expire(&args[1..]),
        "quarantine" => transition(&args[1..], "quarantine", FleetEngine::quarantine_node),
        "revoke" => transition(&args[1..], "revoke", FleetEngine::revoke_node),
        "kill" => transition(&args[1..], "kill", FleetEngine::kill_node),
        unknown => Err(CliError::usage(format!(
            "unknown fleet command '{unknown}'"
        ))),
    }
}

fn dashboard(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "now", "stale-after"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "fleet dashboard does not accept positional arguments",
        ));
    }
    let now = parsed.value("now").map_or_else(
        || Ok(timestamp().as_unix_seconds()),
        |value| {
            value
                .parse()
                .map_err(|_| CliError::usage("--now must be an unsigned integer"))
        },
    )?;
    let stale_after = parsed.value("stale-after").map_or(
        Ok(DEFAULT_SUPERVISOR_STALE_AFTER_SECONDS),
        |value| {
            value
                .parse::<u64>()
                .map_err(|_| CliError::usage("--stale-after must be an unsigned integer"))
        },
    )?;
    if !(1..=86_400).contains(&stale_after) {
        return Err(CliError::usage(
            "--stale-after must be between 1 and 86400 seconds",
        ));
    }

    let config = load_config(&parsed)?;
    let fleet = FleetEngine::open(config.data_dir().join("fleet.sqlite3")).map_err(fleet_error)?;
    let jobs = JobStore::open(config.data_dir().join("jobs.sqlite3"))
        .map_err(|error| operations_error(error.to_string()))?;
    let orchestrations = OrchestrationStore::open(config.data_dir().join("orchestration.sqlite3"))
        .map_err(|error| operations_error(error.to_string()))?;
    let nodes = fleet.list_nodes().map_err(fleet_error)?;
    let leases = fleet.list_leases().map_err(fleet_error)?;
    let supervisors = fleet.list_supervisors().map_err(fleet_error)?;
    let (principal, tenant, workspace) = session_scope();
    let jobs = jobs
        .list(&principal, &tenant, &workspace)
        .map_err(|error| operations_error(error.to_string()))?;
    let orchestrations = orchestrations
        .list(&principal, &tenant, &workspace)
        .map_err(|error| operations_error(error.to_string()))?;

    let node_counts = count_states(
        ["ready", "quarantined", "revoked", "killed"],
        nodes.iter().map(|node| node.state().as_str()),
    );
    let lease_counts = count_states(
        ["active", "released", "expired", "revoked", "killed"],
        leases.iter().map(|lease| lease.state().as_str()),
    );
    let supervisor_counts = count_states(
        ["stopped", "running", "draining", "recovering"],
        supervisors
            .iter()
            .map(|supervisor| supervisor.state().as_str()),
    );
    let job_counts = count_states(
        [
            "queued",
            "running",
            "completed",
            "approval_required",
            "failed",
            "interrupted",
            "cancelled",
        ],
        jobs.iter().map(|job| job.status().as_str()),
    );
    let orchestration_counts = count_states(
        ["queued", "running", "completed", "interrupted", "cancelled"],
        orchestrations.iter().map(|run| run.status().as_str()),
    );

    let stale_supervisors = supervisors
        .iter()
        .filter(|supervisor| {
            supervisor.state() != FleetSupervisorState::Stopped
                && now.saturating_sub(supervisor.updated_at()) > stale_after
        })
        .map(|supervisor| {
            json!({
                "node_id": supervisor.node_id(),
                "state": supervisor.state().as_str(),
                "age_seconds": now.saturating_sub(supervisor.updated_at()),
            })
        })
        .collect::<Vec<_>>();
    let active_leases = leases
        .iter()
        .filter(|lease| lease.state() == FleetLeaseState::Active)
        .collect::<Vec<_>>();
    let overdue_active_leases = active_leases
        .iter()
        .filter(|lease| lease.expires_at() <= now)
        .count();
    let active_lease_details = active_leases
        .iter()
        .take(MAX_OPERATIONS_DETAIL_RECORDS)
        .map(|lease| {
            json!({
                "lease_id": lease.id(),
                "node_id": lease.node_id(),
                "age_seconds": now.saturating_sub(lease.issued_at()),
                "expires_in_seconds": lease.expires_at().saturating_sub(now),
                "overdue": lease.expires_at() <= now,
                "budget_ceiling": budget_value(lease.budget()),
            })
        })
        .collect::<Vec<_>>();
    let (budget_totals, budget_saturated) = active_leases.iter().fold(
        (FleetBudget::new(0, 0, 0, 0), false),
        |(totals, saturated), lease| {
            let (max_tokens, tokens_saturated) = totals
                .max_tokens()
                .overflowing_add(lease.budget().max_tokens());
            let (max_tools, tools_saturated) = totals
                .max_tools()
                .overflowing_add(lease.budget().max_tools());
            let (max_duration, duration_saturated) = totals
                .max_duration_seconds()
                .overflowing_add(lease.budget().max_duration_seconds());
            let (max_cost, cost_saturated) = totals
                .max_cost_micros()
                .overflowing_add(lease.budget().max_cost_micros());
            (
                FleetBudget::new(
                    if tokens_saturated {
                        u64::MAX
                    } else {
                        max_tokens
                    },
                    if tools_saturated { u64::MAX } else { max_tools },
                    if duration_saturated {
                        u64::MAX
                    } else {
                        max_duration
                    },
                    if cost_saturated { u64::MAX } else { max_cost },
                ),
                saturated
                    || tokens_saturated
                    || tools_saturated
                    || duration_saturated
                    || cost_saturated,
            )
        },
    );

    let ready_nodes = count(&node_counts, "ready");
    let running_supervisors = count(&supervisor_counts, "running");
    let queued_jobs = count(&job_counts, "queued");
    let running_jobs = count(&job_counts, "running");
    let queued_orchestrations = count(&orchestration_counts, "queued");
    let running_orchestrations = count(&orchestration_counts, "running");
    let queued_without_capacity = queued_jobs.saturating_add(queued_orchestrations) > 0
        && (ready_nodes == 0 || running_supervisors == 0);
    let health =
        if !stale_supervisors.is_empty() || overdue_active_leases > 0 || queued_without_capacity {
            "attention"
        } else if nodes.is_empty()
            && supervisors.is_empty()
            && leases.is_empty()
            && jobs.is_empty()
            && orchestrations.is_empty()
        {
            "idle"
        } else {
            "healthy"
        };

    let mut failures = jobs
        .iter()
        .filter(|job| matches!(job.status(), JobStatus::Failed | JobStatus::Interrupted))
        .map(|job| {
            let recorded_at = job.finished_at().unwrap_or_else(|| job.created_at());
            (
                recorded_at.as_unix_seconds(),
                json!({
                    "kind": "job",
                    "id": job.id(),
                    "status": job.status().as_str(),
                    "recorded_at": recorded_at.as_unix_seconds(),
                }),
            )
        })
        .chain(
            orchestrations
                .iter()
                .filter(|run| run.status().as_str() == "interrupted")
                .map(|run| {
                    (
                        run.updated_at().as_unix_seconds(),
                        json!({
                            "kind": "orchestration",
                            "id": run.run_id(),
                            "status": run.status().as_str(),
                            "recorded_at": run.updated_at().as_unix_seconds(),
                        }),
                    )
                }),
        )
        .collect::<Vec<_>>();
    failures.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    let failure_count = failures.len();
    let failures_truncated = failure_count > MAX_OPERATIONS_DETAIL_RECORDS;
    let failures = failures
        .into_iter()
        .take(MAX_OPERATIONS_DETAIL_RECORDS)
        .map(|(_, value)| value)
        .collect::<Vec<_>>();

    Ok(success(
        "fleet dashboard",
        json!({
            "generated_at": now,
            "health": {
                "status": health,
                "ready_nodes": ready_nodes,
                "running_supervisors": running_supervisors,
                "stale_supervisors": stale_supervisors.len(),
                "overdue_active_leases": overdue_active_leases,
                "queued_without_capacity": queued_without_capacity,
            },
            "fleet": {
                "nodes": {"total": nodes.len(), "by_state": node_counts},
                "supervisors": {
                    "total": supervisors.len(),
                    "by_state": supervisor_counts,
                    "stale": stale_supervisors,
                },
                "leases": {
                    "total": leases.len(),
                    "by_state": lease_counts,
                    "active": active_lease_details,
                    "active_details_truncated": active_leases.len() > MAX_OPERATIONS_DETAIL_RECORDS,
                },
            },
            "queue": {
                "jobs": {
                    "total": jobs.len(),
                    "by_status": job_counts,
                    "queued": queued_jobs,
                    "running": running_jobs,
                    "failure_count": jobs.iter().filter(|job| matches!(job.status(), JobStatus::Failed | JobStatus::Interrupted)).count(),
                },
                "orchestrations": {
                    "total": orchestrations.len(),
                    "by_status": orchestration_counts,
                    "queued": queued_orchestrations,
                    "running": running_orchestrations,
                    "failure_count": orchestrations.iter().filter(|run| run.status().as_str() == "interrupted").count(),
                },
            },
            "failures": {
                "count": failure_count,
                "records": failures,
                "records_truncated": failures_truncated,
            },
            "budget_ceilings": {
                "active_lease_count": active_leases.len(),
                "max_tokens": budget_totals.max_tokens(),
                "max_tools": budget_totals.max_tools(),
                "max_duration_seconds": budget_totals.max_duration_seconds(),
                "max_cost_micros": budget_totals.max_cost_micros(),
                "saturated": budget_saturated,
                "actual_spend_available": false,
            },
            "boundary": {
                "read_only": true,
                "runtime_authority": false,
                "budgets_are_ceilings_not_spend": true,
                "prompts_included": false,
                "outputs_included": false,
                "credentials_included": false,
                "hidden_reasoning_included": false,
            },
        }),
        format!(
            "Fleet operations are {health}: {queued_jobs} queued job(s), {queued_orchestrations} queued orchestration(s), {} active lease(s)",
            active_leases.len()
        ),
    ))
}

fn count_states<const N: usize>(
    expected: [&str; N],
    values: impl IntoIterator<Item = &'static str>,
) -> BTreeMap<String, usize> {
    let mut counts = expected
        .into_iter()
        .map(|state| (state.to_owned(), 0))
        .collect::<BTreeMap<_, _>>();
    for value in values {
        *counts.entry(value.to_owned()).or_default() += 1;
    }
    counts
}

fn count(counts: &BTreeMap<String, usize>, state: &str) -> usize {
    counts.get(state).copied().unwrap_or_default()
}

fn budget_value(budget: &FleetBudget) -> Value {
    json!({
        "max_tokens": budget.max_tokens(),
        "max_tools": budget.max_tools(),
        "max_duration_seconds": budget.max_duration_seconds(),
        "max_cost_micros": budget.max_cost_micros(),
    })
}

fn operations_error(message: String) -> CliError {
    CliError::execution(message, json!({}))
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "fleet list does not accept positional arguments",
        ));
    }
    let fleet = engine(&parsed)?;
    let nodes = fleet.list_nodes().map_err(fleet_error)?;
    let leases = fleet.list_leases().map_err(fleet_error)?;
    let supervisors = fleet.list_supervisors().map_err(fleet_error)?;
    Ok(success(
        "fleet list",
        json!({
            "nodes": nodes.iter().map(node_value).collect::<Vec<_>>(),
            "leases": leases.iter().map(lease_value).collect::<Vec<_>>(),
            "supervisors": supervisors.iter().map(supervisor_value).collect::<Vec<_>>(),
        }),
        format!(
            "Fleet has {} node(s) and {} lease(s)",
            nodes.len(),
            leases.len()
        ),
    ))
}

fn supervisor(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage("fleet supervisor requires 'list', 'start', 'drain', 'stop', 'recover', 'heartbeat', 'reconcile', 'reap', or 'restart'")
    })?;
    match subcommand.as_str() {
        "list" => supervisor_list(&args[1..]),
        "start" => supervisor_mutation(&args[1..], "start", FleetEngine::start_supervisor),
        "drain" => supervisor_mutation(&args[1..], "drain", FleetEngine::drain_supervisor),
        "stop" => supervisor_mutation(&args[1..], "stop", FleetEngine::stop_supervisor),
        "recover" => supervisor_mutation(&args[1..], "recover", FleetEngine::recover_supervisor),
        "heartbeat" => supervisor_heartbeat(&args[1..]),
        "reconcile" => supervisor_reconcile(&args[1..]),
        "reap" => supervisor_reap(&args[1..]),
        "restart" => supervisor_restart(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown fleet supervisor command '{unknown}'"
        ))),
    }
}

fn supervisor_list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "fleet supervisor list does not accept positional arguments",
        ));
    }
    let supervisors = engine(&parsed)?.list_supervisors().map_err(fleet_error)?;
    Ok(success(
        "fleet supervisor list",
        json!({
            "supervisors": supervisors.iter().map(supervisor_value).collect::<Vec<_>>(),
        }),
        format!("Found {} Fleet supervisor(s)", supervisors.len()),
    ))
}

fn supervisor_mutation(
    args: &[String],
    action: &'static str,
    apply: fn(&FleetEngine, &str, u64) -> Result<FleetSupervisor, FleetError>,
) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "now", "yes"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(format!(
            "fleet supervisor {action} requires exactly one node ID"
        )));
    }
    if action == "stop" && !parsed.values.contains_key("yes") {
        return Err(CliError::usage("fleet supervisor stop requires '--yes'"));
    }
    let now = parsed.value("now").map_or_else(
        || Ok(timestamp().as_unix_seconds()),
        |value| {
            value
                .parse()
                .map_err(|_| CliError::usage("--now must be an unsigned integer"))
        },
    )?;
    let supervisor = apply(&engine(&parsed)?, &parsed.positionals[0], now).map_err(fleet_error)?;
    Ok(success(
        "fleet supervisor",
        json!({"supervisor": supervisor_value(&supervisor)}),
        format!(
            "Fleet supervisor {} is {}",
            supervisor.node_id(),
            supervisor.state().as_str()
        ),
    ))
}

fn supervisor_heartbeat(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "now"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "fleet supervisor heartbeat requires exactly one node ID",
        ));
    }
    let now = parsed.value("now").map_or_else(
        || Ok(timestamp().as_unix_seconds()),
        |value| {
            value
                .parse()
                .map_err(|_| CliError::usage("--now must be an unsigned integer"))
        },
    )?;
    let supervisor = engine(&parsed)?
        .heartbeat_supervisor(&parsed.positionals[0], now)
        .map_err(fleet_error)?;
    Ok(success(
        "fleet supervisor heartbeat",
        json!({"supervisor": supervisor_value(&supervisor)}),
        format!(
            "Fleet supervisor {} is {}",
            supervisor.node_id(),
            supervisor.state().as_str()
        ),
    ))
}

fn supervisor_restart(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "now",
            "stale-after",
            "process-id",
            "node",
        ],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "fleet supervisor restart does not accept positional arguments",
        ));
    }
    let now = parsed.value("now").map_or_else(
        || Ok(timestamp().as_unix_seconds()),
        |value| {
            value
                .parse()
                .map_err(|_| CliError::usage("--now must be an unsigned integer"))
        },
    )?;
    let process_id = required(&parsed, "process-id")?
        .parse::<u32>()
        .map_err(|_| CliError::usage("--process-id must be an unsigned 32-bit integer"))?;
    let supervisor = engine(&parsed)?
        .restart_supervisor_for_process(
            required(&parsed, "node")?,
            process_id,
            now,
            number(&parsed, "stale-after")?,
        )
        .map_err(fleet_error)?;
    Ok(success(
        "fleet supervisor restart",
        json!({"supervisor": supervisor_value(&supervisor)}),
        format!(
            "Fleet supervisor {} restarted as generation {}",
            supervisor.node_id(),
            supervisor.generation()
        ),
    ))
}

fn supervisor_reap(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "now", "stale-after"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "fleet supervisor reap does not accept positional arguments",
        ));
    }
    let now = parsed.value("now").map_or_else(
        || Ok(timestamp().as_unix_seconds()),
        |value| {
            value
                .parse()
                .map_err(|_| CliError::usage("--now must be an unsigned integer"))
        },
    )?;
    let supervisors = engine(&parsed)?
        .reap_stale_supervisors(now, number(&parsed, "stale-after")?)
        .map_err(fleet_error)?;
    let count = supervisors.len();
    Ok(success(
        "fleet supervisor reap",
        json!({
            "reaped": count,
            "supervisors": supervisors.iter().map(supervisor_value).collect::<Vec<_>>(),
        }),
        format!("Reaped {count} stale Fleet supervisor(s)"),
    ))
}

fn supervisor_reconcile(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "now", "stale-after"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "fleet supervisor reconcile requires exactly one node ID",
        ));
    }
    let now = parsed.value("now").map_or_else(
        || Ok(timestamp().as_unix_seconds()),
        |value| {
            value
                .parse()
                .map_err(|_| CliError::usage("--now must be an unsigned integer"))
        },
    )?;
    let supervisor = engine(&parsed)?
        .reconcile_supervisor(&parsed.positionals[0], now, number(&parsed, "stale-after")?)
        .map_err(fleet_error)?;
    Ok(success(
        "fleet supervisor reconcile",
        json!({"supervisor": supervisor_value(&supervisor)}),
        format!(
            "Fleet supervisor {} is {}",
            supervisor.node_id(),
            supervisor.state().as_str()
        ),
    ))
}

fn register(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "version",
            "worker-class",
            "capabilities-json",
        ],
    )?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "fleet register requires exactly one node ID",
        ));
    }
    let version = required(&parsed, "version")?;
    let worker_class = required(&parsed, "worker-class")?;
    let capabilities = serde_json::from_str::<Vec<String>>(required(&parsed, "capabilities-json")?)
        .map_err(|_| CliError::usage("--capabilities-json must be a JSON array of strings"))?;
    let node = FleetNode::new(
        parsed.positionals[0].clone(),
        version,
        worker_class,
        capabilities,
        timestamp().as_unix_seconds(),
    )
    .map_err(fleet_error)?;
    let registered = engine(&parsed)?.register_node(&node).map_err(fleet_error)?;
    Ok(success(
        "fleet register",
        json!({"node": node_value(&registered)}),
        format!("Registered Fleet node {}", registered.id()),
    ))
}

fn dispatch(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "fleet dispatch requires exactly one capability",
        ));
    }
    let node = engine(&parsed)?
        .dispatch_node(&parsed.positionals[0])
        .map_err(fleet_error)?;
    Ok(success(
        "fleet dispatch",
        json!({"capability": parsed.positionals[0], "node": node_value(&node)}),
        format!("Selected Fleet node {}", node.id()),
    ))
}

fn lease(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "node",
            "execution",
            "max-tokens",
            "max-tools",
            "max-duration",
            "max-cost",
            "duration",
            "now",
        ],
    )?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage("fleet lease requires exactly one lease ID"));
    }
    let fleet = engine(&parsed)?;
    let lease = fleet
        .acquire_lease(
            parsed.positionals[0].clone(),
            required(&parsed, "node")?,
            required(&parsed, "execution")?,
            FleetBudget::new(
                number(&parsed, "max-tokens")?,
                number(&parsed, "max-tools")?,
                number(&parsed, "max-duration")?,
                number(&parsed, "max-cost")?,
            ),
            parsed.value("now").map_or_else(
                || Ok(timestamp().as_unix_seconds()),
                |value| {
                    value
                        .parse()
                        .map_err(|_| CliError::usage("--now must be an unsigned integer"))
                },
            )?,
            number(&parsed, "duration")?,
        )
        .map_err(fleet_error)?;
    Ok(success(
        "fleet lease",
        json!({"lease": lease_value(&lease)}),
        format!("Issued Fleet lease {}", lease.id()),
    ))
}

fn renew(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "execution", "duration", "now"],
    )?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage("fleet renew requires exactly one lease ID"));
    }
    let now = parsed.value("now").map_or_else(
        || Ok(timestamp().as_unix_seconds()),
        |value| {
            value
                .parse()
                .map_err(|_| CliError::usage("--now must be an unsigned integer"))
        },
    )?;
    let lease = engine(&parsed)?
        .renew_lease(
            &parsed.positionals[0],
            required(&parsed, "execution")?,
            now,
            number(&parsed, "duration")?,
        )
        .map_err(fleet_error)?;
    Ok(success(
        "fleet renew",
        json!({"lease": lease_value(&lease)}),
        format!("Renewed Fleet lease {}", lease.id()),
    ))
}

fn release(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "fleet release requires exactly one lease ID",
        ));
    }
    let lease = engine(&parsed)?
        .release_lease(&parsed.positionals[0])
        .map_err(fleet_error)?;
    Ok(success(
        "fleet release",
        json!({"lease": lease_value(&lease)}),
        format!("Released Fleet lease {}", lease.id()),
    ))
}

fn expire(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "now"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "fleet expire does not accept positional arguments",
        ));
    }
    let now = parsed.value("now").map_or_else(
        || Ok(timestamp().as_unix_seconds()),
        |value| {
            value
                .parse()
                .map_err(|_| CliError::usage("--now must be an unsigned integer"))
        },
    )?;
    let count = engine(&parsed)?.expire_leases(now).map_err(fleet_error)?;
    Ok(success(
        "fleet expire",
        json!({"expired": count, "now": now}),
        format!("Expired {count} Fleet lease(s)"),
    ))
}

fn transition(
    args: &[String],
    action: &'static str,
    apply: fn(&FleetEngine, &str) -> Result<(), FleetError>,
) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "yes"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(format!(
            "fleet {action} requires exactly one node ID"
        )));
    }
    if !parsed.values.contains_key("yes") {
        return Err(CliError::usage(format!("fleet {action} requires '--yes'")));
    }
    apply(&engine(&parsed)?, &parsed.positionals[0]).map_err(fleet_error)?;
    Ok(success(
        "fleet transition",
        json!({"action": action, "node_id": parsed.positionals[0]}),
        format!("Fleet node {} is now {action}d", parsed.positionals[0]),
    ))
}

fn engine(parsed: &super::ParsedArgs) -> Result<FleetEngine, CliError> {
    let config = load_config(parsed)?;
    FleetEngine::open(config.data_dir().join("fleet.sqlite3")).map_err(fleet_error)
}

fn required<'a>(parsed: &'a super::ParsedArgs, name: &str) -> Result<&'a str, CliError> {
    parsed
        .value(name)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| CliError::usage(format!("fleet requires '--{name} <value>'")))
}

fn number(parsed: &super::ParsedArgs, name: &str) -> Result<u64, CliError> {
    required(parsed, name)?
        .parse()
        .map_err(|_| CliError::usage(format!("--{name} must be an unsigned integer")))
}

fn node_value(node: &FleetNode) -> Value {
    json!({
        "id": node.id(),
        "implementation_version": node.implementation_version(),
        "worker_class": node.worker_class(),
        "capabilities": node.capabilities(),
        "state": node.state().as_str(),
        "registered_at": node.registered_at(),
    })
}

fn lease_value(lease: &FleetLease) -> Value {
    json!({
        "id": lease.id(),
        "node_id": lease.node_id(),
        "execution_id": lease.execution_id(),
        "budget": {
            "max_tokens": lease.budget().max_tokens(),
            "max_tools": lease.budget().max_tools(),
            "max_duration_seconds": lease.budget().max_duration_seconds(),
            "max_cost_micros": lease.budget().max_cost_micros(),
        },
        "issued_at": lease.issued_at(),
        "expires_at": lease.expires_at(),
        "state": lease.state().as_str(),
    })
}

fn supervisor_value(supervisor: &FleetSupervisor) -> Value {
    json!({
        "node_id": supervisor.node_id(),
        "state": supervisor.state().as_str(),
        "generation": supervisor.generation(),
        "process_id": supervisor.process_id(),
        "reason": supervisor.reason(),
        "updated_at": supervisor.updated_at(),
    })
}

fn fleet_error(error: FleetError) -> CliError {
    CliError::execution(error.to_string(), json!({}))
}
