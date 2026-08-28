use super::{load_config, parse_options, require_config_file, session_scope, timestamp};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::{
    OrchestrationRunRecord, OrchestrationRunStatus, OrchestrationStore, OrchestrationStoreError,
};
use pandora_types::{
    GovernedOrchestrationPlan, JobWorkerId, OrchestrationRole, OrchestrationRoleReceipt,
    OrchestrationRunId, RoleAssignment, RoleId,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_ORCHESTRATION_INPUT_BYTES: u64 = 1024 * 1024;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("orchestration requires a subcommand"))?;
    match subcommand.as_str() {
        "roles" => roles(&args[1..]),
        "submit" => submit(&args[1..]),
        "claim" => claim(&args[1..]),
        "complete" => complete(&args[1..]),
        "list" => list(&args[1..]),
        "inspect" => inspect(&args[1..]),
        "cancel" => cancel(&args[1..]),
        "mark-interrupted" => mark_interrupted(&args[1..]),
        "resume" => resume(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown orchestration command '{unknown}'"
        ))),
    }
}

fn roles(args: &[String]) -> Result<CommandResult, CliError> {
    if !args.is_empty() {
        return Err(CliError::usage(
            "orchestration roles does not accept arguments",
        ));
    }
    let roles = OrchestrationRole::standard()
        .into_iter()
        .map(|role| role.as_str().to_owned())
        .collect::<Vec<_>>();
    Ok(success(
        "orchestration roles",
        json!({"roles": roles}),
        "planner, maker, critic, verifier",
    ))
}

fn submit(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "input", "id"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "orchestration submit does not accept positional arguments",
        ));
    }
    let input = parsed
        .value("input")
        .ok_or_else(|| CliError::usage("orchestration submit requires '--input <path>'"))?;
    let plan: GovernedOrchestrationPlan = read_json(Path::new(input), "orchestration plan")?;
    let run_id = match parsed.value("id") {
        Some(value) => parse_run_id(value)?,
        None => allocate_run_id()?,
    };
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = store(&config)?;
    let (principal, tenant, workspace) = session_scope();
    let record = store
        .submit(
            &run_id,
            &principal,
            &tenant,
            &workspace,
            &plan,
            timestamp(),
        )
        .map_err(store_error)?;
    Ok(success(
        "orchestration submit",
        record_json(&record)?,
        format!("Queued orchestration {}", record.run_id()),
    ))
}

fn claim(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "worker"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "orchestration claim does not accept positional arguments",
        ));
    }
    let worker = parse_worker(
        parsed
            .value("worker")
            .ok_or_else(|| CliError::usage("orchestration claim requires '--worker <id>'"))?,
    )?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = store(&config)?;
    let (principal, tenant, workspace) = session_scope();
    let Some(record) = store
        .claim_next(&principal, &tenant, &workspace, &worker, timestamp())
        .map_err(store_error)?
    else {
        return Ok(success(
            "orchestration claim",
            json!({"run": null, "status": "idle"}),
            "No queued orchestration runs",
        ));
    };
    let assignments = store
        .start_ready(
            record.run_id(),
            &principal,
            &tenant,
            &workspace,
            &worker,
            timestamp(),
        )
        .map_err(store_error)?;
    let record = store
        .inspect(record.run_id(), &principal, &tenant, &workspace)
        .map_err(store_error)?;
    Ok(success(
        "orchestration claim",
        json!({
            "run": record_json(&record)?,
            "assignments": assignment_json(&assignments, record.plan())?,
        }),
        format!(
            "Claimed {} with {} ready role(s)",
            record.run_id(),
            assignments.len()
        ),
    ))
}

fn complete(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "worker", "role", "receipt"],
    )?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "orchestration complete requires exactly one run ID",
        ));
    }
    let run_id = parse_run_id(&parsed.positionals[0])?;
    let worker = parse_worker(
        parsed
            .value("worker")
            .ok_or_else(|| CliError::usage("orchestration complete requires '--worker <id>'"))?,
    )?;
    let role_id = RoleId::new(
        parsed
            .value("role")
            .ok_or_else(|| CliError::usage("orchestration complete requires '--role <id>'"))?
            .to_owned(),
    )
    .map_err(|_| CliError::usage("orchestration role ID is invalid"))?;
    let receipt_path = parsed
        .value("receipt")
        .ok_or_else(|| CliError::usage("orchestration complete requires '--receipt <path>'"))?;
    let receipt: OrchestrationRoleReceipt =
        read_json(Path::new(receipt_path), "orchestration role receipt")?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = store(&config)?;
    let (principal, tenant, workspace) = session_scope();
    let record = store
        .complete_role(
            &run_id,
            &principal,
            &tenant,
            &workspace,
            &worker,
            &role_id,
            &receipt,
            timestamp(),
        )
        .map_err(store_error)?;
    let assignments = if record.status() == OrchestrationRunStatus::Running {
        store
            .start_ready(
                &run_id,
                &principal,
                &tenant,
                &workspace,
                &worker,
                timestamp(),
            )
            .map_err(store_error)?
    } else {
        Vec::new()
    };
    let record = store
        .inspect(&run_id, &principal, &tenant, &workspace)
        .map_err(store_error)?;
    Ok(success(
        "orchestration complete",
        json!({
            "run": record_json(&record)?,
            "assignments": assignment_json(&assignments, record.plan())?,
        }),
        format!(
            "Recorded {} for role {}; run is {}",
            receipt.receipt_id(),
            role_id,
            record.status().as_str()
        ),
    ))
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "orchestration list does not accept positional arguments",
        ));
    }
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = store(&config)?;
    let (principal, tenant, workspace) = session_scope();
    let runs = store
        .list(&principal, &tenant, &workspace)
        .map_err(store_error)?
        .iter()
        .map(record_json)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(success(
        "orchestration list",
        json!({"count": runs.len(), "runs": runs}),
        format!("{} orchestration run(s)", runs.len()),
    ))
}

fn inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "orchestration inspect requires exactly one run ID",
        ));
    }
    let run_id = parse_run_id(&parsed.positionals[0])?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = store(&config)?;
    let (principal, tenant, workspace) = session_scope();
    let record = store
        .inspect(&run_id, &principal, &tenant, &workspace)
        .map_err(store_error)?;
    Ok(success(
        "orchestration inspect",
        record_json(&record)?,
        format!("{} is {}", run_id, record.status().as_str()),
    ))
}

fn cancel(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "orchestration cancel requires exactly one run ID",
        ));
    }
    let run_id = parse_run_id(&parsed.positionals[0])?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = store(&config)?;
    let (principal, tenant, workspace) = session_scope();
    let record = store
        .cancel(&run_id, &principal, &tenant, &workspace, timestamp())
        .map_err(store_error)?;
    Ok(success(
        "orchestration cancel",
        record_json(&record)?,
        format!("Cancelled orchestration {run_id}"),
    ))
}

fn mark_interrupted(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "reason", "yes"],
    )?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "orchestration mark-interrupted requires exactly one run ID",
        ));
    }
    if parsed.value("yes").is_none() {
        return Err(CliError::usage(
            "orchestration mark-interrupted requires '--yes'; reconcile active role effects first",
        ));
    }
    let reason = parsed
        .value("reason")
        .filter(|reason| !reason.trim().is_empty())
        .ok_or_else(|| {
            CliError::usage("orchestration mark-interrupted requires a non-empty '--reason'")
        })?;
    let run_id = parse_run_id(&parsed.positionals[0])?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = store(&config)?;
    let (principal, tenant, workspace) = session_scope();
    let record = store
        .mark_interrupted(
            &run_id,
            &principal,
            &tenant,
            &workspace,
            reason,
            timestamp(),
        )
        .map_err(store_error)?;
    Ok(success(
        "orchestration mark-interrupted",
        record_json(&record)?,
        format!("Marked orchestration {run_id} interrupted"),
    ))
}

fn resume(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "orchestration resume requires exactly one run ID",
        ));
    }
    let run_id = parse_run_id(&parsed.positionals[0])?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = store(&config)?;
    let (principal, tenant, workspace) = session_scope();
    let record = store
        .resume(&run_id, &principal, &tenant, &workspace, timestamp())
        .map_err(store_error)?;
    Ok(success(
        "orchestration resume",
        record_json(&record)?,
        format!("Requeued orchestration {run_id}"),
    ))
}

fn store(
    config: &pandora_runtime::config::RuntimeConfig,
) -> Result<OrchestrationStore, CliError> {
    OrchestrationStore::open(config.data_dir().join("orchestration.sqlite3"))
        .map_err(store_error)
}

fn parse_run_id(value: &str) -> Result<OrchestrationRunId, CliError> {
    OrchestrationRunId::new(value.to_owned())
        .map_err(|_| CliError::usage("orchestration run ID is invalid"))
}

fn parse_worker(value: &str) -> Result<JobWorkerId, CliError> {
    JobWorkerId::new(value.to_owned())
        .map_err(|_| CliError::usage("orchestration worker ID is invalid"))
}

fn allocate_run_id() -> Result<OrchestrationRunId, CliError> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    OrchestrationRunId::new(format!("orchestration-{}-{nonce}", std::process::id()))
        .map_err(|_| CliError::internal("could not allocate an orchestration run ID", json!({})))
}

fn read_json<T: DeserializeOwned>(path: &Path, label: &str) -> Result<T, CliError> {
    let metadata = std::fs::metadata(path).map_err(|_| {
        CliError::usage(format!("could not read {label} from {}", path.display()))
    })?;
    if metadata.len() > MAX_ORCHESTRATION_INPUT_BYTES {
        return Err(CliError::usage(format!(
            "{label} exceeds the {} byte limit",
            MAX_ORCHESTRATION_INPUT_BYTES
        )));
    }
    let bytes = std::fs::read(path).map_err(|_| {
        CliError::usage(format!("could not read {label} from {}", path.display()))
    })?;
    serde_json::from_slice(&bytes).map_err(|_| CliError::usage(format!("{label} is invalid JSON")))
}

fn record_json(record: &OrchestrationRunRecord) -> Result<Value, CliError> {
    let plan = serde_json::to_value(record.plan())
        .map_err(|_| CliError::internal("could not serialize orchestration plan", json!({})))?;
    let receipts = serde_json::to_value(record.role_receipts())
        .map_err(|_| CliError::internal("could not serialize orchestration receipts", json!({})))?;
    Ok(json!({
        "run_id": record.run_id(),
        "principal_id": record.principal_id(),
        "tenant_id": record.tenant_id(),
        "coordinator_workspace_id": record.coordinator_workspace_id(),
        "plan": plan,
        "status": record.status().as_str(),
        "worker_id": record.worker_id().map(JobWorkerId::as_str),
        "active_roles": record.snapshot().active_roles(),
        "completed_roles": record.snapshot().completed_roles(),
        "handoffs_used": record.snapshot().handoffs_used(),
        "role_receipts": receipts,
        "interruption_reason": record.interruption_reason(),
        "created_at": record.created_at().as_unix_seconds(),
        "updated_at": record.updated_at().as_unix_seconds(),
    }))
}

fn assignment_json(
    assignments: &[RoleAssignment],
    plan: &GovernedOrchestrationPlan,
) -> Result<Vec<Value>, CliError> {
    assignments
        .iter()
        .map(|assignment| {
            let repository = plan.repository_for_role(assignment.id()).ok_or_else(|| {
                CliError::internal(
                    "orchestration role has no repository binding",
                    json!({"role_id": assignment.id()}),
                )
            })?;
            Ok(json!({
                "role_id": assignment.id(),
                "role": assignment.role().as_str(),
                "harness_id": assignment.harness_id(),
                "depends_on": assignment.depends_on(),
                "repository_id": repository.repository_id(),
                "workspace_id": repository.workspace_id(),
                "exact_commit": repository.exact_commit(),
            }))
        })
        .collect()
}

fn store_error(error: OrchestrationStoreError) -> CliError {
    let message = error.to_string();
    match error {
        OrchestrationStoreError::Contract(_) | OrchestrationStoreError::InvalidIdentifier => {
            CliError::usage(message)
        }
        OrchestrationStoreError::RunNotFound
        | OrchestrationStoreError::RunOwnedByAnotherWorker
        | OrchestrationStoreError::DuplicateReceipt
        | OrchestrationStoreError::ActiveRolesRequireReconciliation
        | OrchestrationStoreError::InvalidTransition { .. }
        | OrchestrationStoreError::Orchestration(_) => CliError::execution(message, json!({})),
        _ => CliError::internal(message, json!({})),
    }
}
