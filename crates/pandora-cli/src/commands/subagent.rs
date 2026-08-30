use super::{
    RuntimeConfig, load_config, parse_options, require_config_file, session_scope, session_store,
    timestamp,
};
use crate::output::{CliError, CommandResult, success};
use pandora_harnesses::{CODING_HARNESS_ID, canonical_harness_binding_digest};
use pandora_runtime::executors::WorkspaceRoot;
use pandora_runtime::{
    ApprovalStore, ExecutionController, FleetBudget, FleetEngine, FleetError, FleetNode,
    GitWorktreeExecutor, SubagentCleanupContext, SubagentCoordinator, SubagentCoordinatorError,
    SubagentRecord, SubagentRunControl, SubagentScope, SubagentSpawnContext, SubagentStore,
    SubagentStoreError,
};
use pandora_types::{
    Capability, EffectOutcome, EffectReceipt, ExecutionId, HarnessId, JobId, JobWorkerId,
    PolicyContext, SessionId, SubagentBudgets, SubagentHarnessBinding, SubagentId, SubagentRequest,
    SubagentStatus,
};
use serde_json::{Value, json};
use std::fs;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_MAX_TURNS: u32 = 8;
const DEFAULT_MAX_TOOLS: u32 = 16;
const DEFAULT_MAX_TOKENS: u32 = 32_000;
const DEFAULT_MAX_DURATION_SECONDS: u64 = 300;
const DEFAULT_MAX_DELEGATION_DEPTH: u8 = 1;
const DEFAULT_MAX_RESULT_BYTES: usize = 8_192;
const SUBAGENT_FLEET_WORKER_CLASS: &str = "local-subagent-worker";
const SUBAGENT_WORKER_LEASE_DURATION_SECONDS: u64 = 3_600;
const SUBAGENT_WORKER_HEARTBEAT_SECONDS: u64 = 10;
const SUBAGENT_WORKER_RESTART_STALE_AFTER_SECONDS: u64 = 30;

static NEXT_SUBAGENT_ID: AtomicU64 = AtomicU64::new(1);

struct ActiveSubagentSupervisor {
    fleet: Arc<FleetEngine>,
    node_id: String,
    lease_id: String,
    execution_id: String,
}

impl Drop for ActiveSubagentSupervisor {
    fn drop(&mut self) {
        let now = timestamp().as_unix_seconds();
        let _ = self.fleet.release_lease(&self.lease_id);
        let _ = self.fleet.drain_supervisor(&self.node_id, now);
        let _ = self.fleet.stop_supervisor(&self.node_id, now);
    }
}

fn start_subagent_supervisor(config: &RuntimeConfig) -> Result<ActiveSubagentSupervisor, CliError> {
    let fleet =
        Arc::new(FleetEngine::open(config.data_dir().join("fleet.sqlite3")).map_err(fleet_error)?);
    let process_id = std::process::id();
    let node_id = "subagent-worker".to_owned();
    let now = timestamp().as_unix_seconds();
    let node = FleetNode::new(
        node_id.clone(),
        env!("CARGO_PKG_VERSION"),
        SUBAGENT_FLEET_WORKER_CLASS,
        ["subagent.work".to_owned()],
        now,
    )
    .map_err(fleet_error)?;
    match fleet.register_node(&node) {
        Ok(_) | Err(FleetError::NodeAlreadyRegistered) => {}
        Err(error) => return Err(fleet_error(error)),
    }
    match fleet.start_supervisor_for_process(&node_id, process_id, now) {
        Ok(_) => {}
        Err(FleetError::SupervisorAlreadyRunning) => {
            fleet
                .restart_supervisor_for_process(
                    &node_id,
                    process_id,
                    now,
                    SUBAGENT_WORKER_RESTART_STALE_AFTER_SECONDS,
                )
                .map_err(fleet_error)?;
        }
        Err(error) => return Err(fleet_error(error)),
    }
    let lease_id = format!("subagent-process-lease-{process_id}-{now}");
    let execution_id = format!("subagent-process:{process_id}");
    if let Err(error) = fleet.acquire_lease(
        lease_id.clone(),
        node_id.clone(),
        execution_id.clone(),
        FleetBudget::new(0, 0, SUBAGENT_WORKER_LEASE_DURATION_SECONDS, 0),
        now,
        SUBAGENT_WORKER_LEASE_DURATION_SECONDS,
    ) {
        let _ = fleet.drain_supervisor(&node_id, now);
        let _ = fleet.stop_supervisor(&node_id, now);
        return Err(fleet_error(error));
    }
    Ok(ActiveSubagentSupervisor {
        fleet,
        node_id,
        lease_id,
        execution_id,
    })
}

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("subagent requires a subcommand"))?;
    match subcommand.as_str() {
        "spawn" => spawn(&args[1..]),
        "work" => work(&args[1..]),
        "list" => list(&args[1..]),
        "inspect" => inspect(&args[1..]),
        "cancel" => cancel(&args[1..]),
        "mark-interrupted" => mark_interrupted(&args[1..]),
        "cleanup" => cleanup(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown subagent command '{unknown}'"
        ))),
    }
}

fn work(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "max-agents"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "subagent work does not accept positional arguments",
        ));
    }
    let max_agents = parse_max_agents(parsed.value("max-agents"))?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let supervisor = start_subagent_supervisor(&config)?;
    let store = SubagentStore::open(config.data_dir().join("jobs.sqlite3"))
        .map_err(subagent_store_error)?;
    let sessions = session_store(&config)?;
    let approvals = ApprovalStore::open(config.data_dir().join("sessions.sqlite3"))
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let (principal, tenant, workspace) = session_scope();
    let scope = SubagentScope::new(principal, tenant, workspace);
    let results = Mutex::new(Vec::new());
    let failures = Mutex::new(None);
    let barrier = Barrier::new(max_agents);
    let worker_process_id = std::process::id();
    let heartbeat_active = Arc::new(AtomicBool::new(true));
    let heartbeat_failed = Arc::new(AtomicBool::new(false));
    let heartbeat = Arc::clone(&heartbeat_active);
    let heartbeat_failed_for_thread = Arc::clone(&heartbeat_failed);
    let heartbeat_fleet = Arc::clone(&supervisor.fleet);
    let heartbeat_node = supervisor.node_id.clone();
    let heartbeat_lease = supervisor.lease_id.clone();
    let heartbeat_execution = supervisor.execution_id.clone();
    let heartbeat_thread = thread::spawn(move || {
        while heartbeat.load(Ordering::Acquire) {
            thread::sleep(Duration::from_secs(SUBAGENT_WORKER_HEARTBEAT_SECONDS));
            if !heartbeat.load(Ordering::Acquire) {
                break;
            }
            let now = timestamp().as_unix_seconds();
            if heartbeat_fleet
                .heartbeat_supervisor_for_process(&heartbeat_node, worker_process_id, now)
                .and_then(|_| {
                    heartbeat_fleet.renew_lease(
                        &heartbeat_lease,
                        &heartbeat_execution,
                        now,
                        SUBAGENT_WORKER_LEASE_DURATION_SECONDS,
                    )
                })
                .is_err()
            {
                heartbeat_failed_for_thread.store(true, Ordering::Release);
                break;
            }
        }
    });
    thread::scope(|threads| {
        for _ in 0..max_agents {
            threads.spawn(|| {
                barrier.wait();
                if let Err(error) = supervisor.fleet.heartbeat_supervisor_for_process(
                    &supervisor.node_id,
                    worker_process_id,
                    timestamp().as_unix_seconds(),
                ) {
                    *failures
                        .lock()
                        .expect("worker failure mutex should not poison") =
                        Some(fleet_error(error));
                    return;
                }
                let worker = match allocate_worker_id() {
                    Ok(worker) => worker,
                    Err(error) => {
                        *failures
                            .lock()
                            .expect("worker failure mutex should not poison") = Some(error);
                        return;
                    }
                };
                loop {
                    let claimed = match store.claim_next(&scope, &worker, timestamp()) {
                        Ok(claimed) => claimed,
                        Err(error) => {
                            *failures
                                .lock()
                                .expect("worker failure mutex should not poison") =
                                Some(subagent_store_error(error));
                            return;
                        }
                    };
                    let Some(claimed) = claimed else {
                        return;
                    };
                    let control = SubagentRunControl::new(
                        &store,
                        claimed.id(),
                        &scope,
                        claimed.request().budgets(),
                    );
                    let outcome = match super::subagent_run::execute_trusted_subagent(
                        super::subagent_run::TrustedSubagentRun {
                            config: &config,
                            record: &claimed,
                            store: &sessions,
                            approval_store: &approvals,
                            control: &control,
                        },
                    ) {
                        Ok(result) => (SubagentStatus::Completed, Ok(result)),
                        Err(error) => (terminal_status(&error), Err(error)),
                    };
                    let outcome = terminal_result(
                        outcome.0,
                        outcome.1,
                        claimed.request().budgets().max_result_bytes(),
                    );
                    let finished = match store.finish(
                        claimed.id(),
                        &worker,
                        outcome.0,
                        &outcome.1,
                        timestamp(),
                    ) {
                        Ok(finished) => finished,
                        Err(error) => {
                            *failures
                                .lock()
                                .expect("worker failure mutex should not poison") =
                                Some(subagent_store_error(error));
                            return;
                        }
                    };
                    results
                        .lock()
                        .expect("worker result mutex should not poison")
                        .push(finished);
                    if let Err(error) = supervisor.fleet.heartbeat_supervisor_for_process(
                        &supervisor.node_id,
                        worker_process_id,
                        timestamp().as_unix_seconds(),
                    ) {
                        *failures
                            .lock()
                            .expect("worker failure mutex should not poison") =
                            Some(fleet_error(error));
                        return;
                    }
                }
            });
        }
    });
    heartbeat_active.store(false, Ordering::Release);
    let _ = heartbeat_thread.join();
    if heartbeat_failed.load(Ordering::Acquire) {
        return Err(CliError::execution(
            "subagent worker supervisor heartbeat failed",
            json!({"code": "worker_supervisor_heartbeat_failed"}),
        ));
    }
    if let Some(error) = failures
        .into_inner()
        .expect("worker failure mutex should not poison")
    {
        return Err(error);
    }
    let mut results = results
        .into_inner()
        .expect("worker result mutex should not poison");
    results.sort_by(|left, right| left.id().as_str().cmp(right.id().as_str()));
    let subagents = results
        .iter()
        .map(subagent_json)
        .collect::<Result<Vec<_>, _>>()?;
    let processed_count = subagents.len();
    Ok(success(
        "subagent work",
        json!({
            "worker_count": max_agents,
            "processed_count": processed_count,
            "subagents": subagents,
        }),
        format!("Processed {processed_count} subagent(s)"),
    ))
}

fn inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "subagent inspect requires exactly one subagent ID",
        ));
    }
    let id = parse_subagent_id(&parsed.positionals[0])?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = SubagentStore::open(config.data_dir().join("jobs.sqlite3"))
        .map_err(subagent_store_error)?;
    let record = store
        .inspect(&id, &current_scope())
        .map_err(subagent_store_error)?;
    Ok(success(
        "subagent inspect",
        subagent_json(&record)?,
        format!("{} is {:?}", record.id(), record.status()),
    ))
}

fn cancel(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "subagent cancel requires exactly one subagent ID",
        ));
    }
    let id = parse_subagent_id(&parsed.positionals[0])?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = SubagentStore::open(config.data_dir().join("jobs.sqlite3"))
        .map_err(subagent_store_error)?;
    let record = store
        .request_cancel(&id, &current_scope(), timestamp())
        .map_err(subagent_store_error)?;
    Ok(success(
        "subagent cancel",
        subagent_json(&record)?,
        format!("Cancellation requested for {}", record.id()),
    ))
}

fn mark_interrupted(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "reason", "yes"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "subagent mark-interrupted requires exactly one subagent ID",
        ));
    }
    if parsed.value("yes").is_none() {
        return Err(CliError::usage("subagent mark-interrupted requires --yes"));
    }
    let reason = parsed
        .value("reason")
        .filter(|reason| !reason.trim().is_empty())
        .ok_or_else(|| {
            CliError::usage("subagent mark-interrupted requires a non-empty --reason")
        })?;
    let id = parse_subagent_id(&parsed.positionals[0])?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = SubagentStore::open(config.data_dir().join("jobs.sqlite3"))
        .map_err(subagent_store_error)?;
    let record = store
        .mark_interrupted(&id, &current_scope(), reason, timestamp())
        .map_err(subagent_store_error)?;
    Ok(success(
        "subagent mark-interrupted",
        subagent_json(&record)?,
        format!("Marked {} interrupted", record.id()),
    ))
}

fn cleanup(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "yes"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "subagent cleanup requires exactly one subagent ID",
        ));
    }
    if parsed.value("yes").is_none() {
        return Err(CliError::usage("subagent cleanup requires --yes"));
    }
    let id = parse_subagent_id(&parsed.positionals[0])?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = SubagentStore::open(config.data_dir().join("jobs.sqlite3"))
        .map_err(subagent_store_error)?;
    let managed_root = config.data_dir().join("subagents");
    fs::create_dir_all(&managed_root)
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let controller = worktree_controller(
        config.workspace_dir(),
        super::run::configured_harnesses(&config, Some(CODING_HARNESS_ID), None)?,
    )?;
    let executor = GitWorktreeExecutor::new(config.workspace_dir(), &managed_root)
        .map_err(|error| CliError::execution(error.code(), json!({})))?;
    let scope = current_scope();
    let coordinator = SubagentCoordinator::new(&store, &controller, &executor);
    let record = coordinator
        .cleanup(
            &id,
            SubagentCleanupContext::new(
                scope,
                allocate_session_id("subagent-cleanup-session")?,
                allocate_execution_id("subagent-cleanup-execution")?,
            ),
            timestamp(),
        )
        .map_err(subagent_coordinator_error)?;
    Ok(success(
        "subagent cleanup",
        subagent_json(&record)?,
        format!("Cleaned up {}", record.id()),
    ))
}

fn spawn(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "session",
            "execution",
            "commit",
            "provider",
            "harness",
            "harness-version",
            "max-turns",
            "max-tools",
            "max-tokens",
            "max-duration",
            "max-depth",
            "max-result-bytes",
        ],
    )?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage("subagent spawn requires exactly one task"));
    }
    if parsed.value("harness-version").is_some() && parsed.value("harness").is_none() {
        return Err(CliError::usage("--harness-version requires --harness <id>"));
    }
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let parent_session_id = parse_session_id(required_option(&parsed, "session")?)?;
    let parent_execution_id = parse_execution_id(required_option(&parsed, "execution")?)?;
    let (principal, tenant, workspace) = session_scope();
    let sessions = session_store(&config)?;
    let snapshot = sessions
        .resume(&parent_session_id, &principal, &tenant, &workspace)
        .map_err(|error| CliError::execution(error.to_string(), json!({})))?;
    if !snapshot
        .evaluations()
        .iter()
        .any(|evaluation| evaluation.execution_id() == &parent_execution_id)
    {
        return Err(CliError::execution(
            "parent execution does not belong to the parent session",
            json!({
                "session_id": parent_session_id,
                "execution_id": parent_execution_id,
            }),
        ));
    }
    let provider_name = parsed
        .value("provider")
        .or_else(|| config.active_provider())
        .ok_or_else(|| {
            CliError::configuration("no active provider profile is configured", json!({}))
        })?;
    if config.provider_profile(provider_name).is_none() {
        return Err(CliError::configuration(
            "requested provider profile is not configured",
            json!({"provider_profile": provider_name}),
        ));
    }
    let requested_harness = parsed.value("harness").unwrap_or(CODING_HARNESS_ID);
    let harnesses = super::run::configured_harnesses(
        &config,
        Some(requested_harness),
        parsed.value("harness-version"),
    )?;
    super::run::require_runnable_harness(&harnesses, Some(requested_harness))?;
    let harness_id = HarnessId::new(match requested_harness {
        "coding" => CODING_HARNESS_ID.to_owned(),
        value => value.to_owned(),
    })
    .map_err(|_| CliError::usage("Harness ID is invalid"))?;
    let harness = harnesses
        .find(&harness_id)
        .ok_or_else(|| CliError::execution("requested harness is not supported", json!({})))?;
    let harness_binding = SubagentHarnessBinding::new(
        harness.manifest().id().clone(),
        harness.manifest().version(),
    )
    .map_err(subagent_contract_error)?;
    let harness_binding_digest = canonical_harness_binding_digest(harness.manifest());
    let budgets = SubagentBudgets::new(
        parse_u32(&parsed, "max-turns", DEFAULT_MAX_TURNS)?,
        parse_u32(&parsed, "max-tools", DEFAULT_MAX_TOOLS)?,
        parse_u32(&parsed, "max-tokens", DEFAULT_MAX_TOKENS)?,
        parse_u64(&parsed, "max-duration", DEFAULT_MAX_DURATION_SECONDS)?,
        parse_u8(&parsed, "max-depth", DEFAULT_MAX_DELEGATION_DEPTH)?,
        parse_usize(&parsed, "max-result-bytes", DEFAULT_MAX_RESULT_BYTES)?,
    )
    .map_err(subagent_contract_error)?;
    let exact_commit = match parsed.value("commit") {
        Some(commit) => commit.to_owned(),
        None => current_commit(config.workspace_dir())?,
    };
    let request = SubagentRequest::new(
        parent_session_id.clone(),
        parent_execution_id.clone(),
        1,
        exact_commit,
        &parsed.positionals[0],
        budgets,
    )
    .map_err(subagent_contract_error)?
    .with_provider_profile(provider_name)
    .map_err(subagent_contract_error)?
    .with_harness(harness_binding);
    let scope = SubagentScope::new(principal, tenant, workspace);
    let store = SubagentStore::open(config.data_dir().join("jobs.sqlite3"))
        .map_err(subagent_store_error)?;
    let controller = worktree_controller(config.workspace_dir(), harnesses)?;
    let managed_root = config.data_dir().join("subagents");
    fs::create_dir_all(&managed_root)
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let executor = GitWorktreeExecutor::new(config.workspace_dir(), &managed_root)
        .map_err(|error| CliError::execution(error.code(), json!({})))?;
    let context = SubagentSpawnContext::new(
        allocate_subagent_id("subagent")?,
        allocate_job_id()?,
        scope,
        allocate_session_id("subagent-session")?,
        allocate_execution_id("subagent-execution")?,
        parent_session_id,
        parent_execution_id,
        Some(super::subagent_run::provider_binding_digest(
            &config,
            provider_name,
        )?),
        Some(harness_binding_digest),
    );
    let coordinator = SubagentCoordinator::new(&store, &controller, &executor);
    let record = coordinator
        .spawn(context, request, timestamp())
        .map_err(subagent_coordinator_error)?;
    Ok(success(
        "subagent spawn",
        subagent_json(&record)?,
        format!("Queued {}", record.id()),
    ))
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "subagent list does not accept positional arguments",
        ));
    }
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = SubagentStore::open(config.data_dir().join("jobs.sqlite3"))
        .map_err(subagent_store_error)?;
    let (principal, tenant, workspace) = session_scope();
    let scope = SubagentScope::new(principal, tenant, workspace);
    let subagents = store
        .list(&scope)
        .map_err(subagent_store_error)?
        .iter()
        .map(subagent_json)
        .collect::<Result<Vec<_>, _>>()?;
    let count = subagents.len();
    Ok(success(
        "subagent list",
        json!({"count": count, "subagents": subagents}),
        format!("{count} subagent(s)"),
    ))
}

fn subagent_json(record: &SubagentRecord) -> Result<Value, CliError> {
    let request = record.request();
    let harness = request
        .harness()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|_| CliError::internal("could not serialize subagent Harness", json!({})))?;
    Ok(json!({
        "subagent_id": record.id(),
        "job_id": record.job_id(),
        "scope": {
            "principal_id": record.scope().principal_id(),
            "tenant_id": record.scope().tenant_id(),
            "workspace_id": record.scope().workspace_id(),
        },
        "parent": {
            "session_id": request.parent_session_id(),
            "execution_id": request.parent_execution_id(),
        },
        "child": {
            "session_id": record.child_session_id(),
            "execution_id": record.child_execution_id(),
        },
        "request": {
            "delegation_depth": request.delegation_depth(),
            "exact_commit": request.exact_commit(),
            "task": request.task(),
            "budgets": request.budgets(),
            "provider_profile": request.provider_profile(),
            "harness": harness,
        },
        "lifecycle": {
            "status": record.status(),
            "created_at": record.created_at().as_unix_seconds(),
            "started_at": record.started_at().map(|value| value.as_unix_seconds()),
            "finished_at": record.finished_at().map(|value| value.as_unix_seconds()),
        },
        "worktree": {
            "state": record.worktree_state(),
            "repository": record.repository_path(),
            "path": record.worktree_path(),
        },
        "worker": {
            "id": record.worker_id().map(|value| value.as_str()),
            "cancel_requested_at": record.cancel_requested_at().map(|value| value.as_unix_seconds()),
        },
        "receipts": {
            "create": record.create_receipt().map(effect_receipt_json),
            "remove": record.remove_receipt().map(effect_receipt_json),
        },
        "result": stable_result(record.result()),
    }))
}

fn effect_receipt_json(receipt: &EffectReceipt) -> Value {
    let outcome = match receipt.outcome() {
        EffectOutcome::Succeeded => json!({"status": "succeeded"}),
        EffectOutcome::Failed { code } => json!({"status": "failed", "code": code}),
        EffectOutcome::Denied { reason } => json!({"status": "denied", "reason": reason}),
    };
    json!({
        "receipt_id": receipt.receipt_id().as_str(),
        "outcome": outcome,
    })
}

fn worktree_controller(
    workspace: &std::path::Path,
    harnesses: pandora_harnesses::HarnessCatalog,
) -> Result<ExecutionController, CliError> {
    let workspace = WorkspaceRoot::new(workspace)
        .map_err(|_| CliError::configuration("workspace path is invalid", json!({})))?;
    let policy = PolicyContext::new(1, [Capability::ProcessExecute], []);
    Ok(ExecutionController::with_policy_and_harnesses(
        workspace, policy, harnesses,
    ))
}

fn required_option<'a>(parsed: &'a super::ParsedArgs, name: &str) -> Result<&'a str, CliError> {
    parsed
        .value(name)
        .ok_or_else(|| CliError::usage(format!("subagent spawn requires --{name} <value>")))
}

fn current_scope() -> SubagentScope {
    let (principal, tenant, workspace) = session_scope();
    SubagentScope::new(principal, tenant, workspace)
}

fn parse_subagent_id(value: &str) -> Result<SubagentId, CliError> {
    SubagentId::new(value.to_owned()).map_err(|_| CliError::usage("subagent ID is invalid"))
}

fn parse_max_agents(value: Option<&str>) -> Result<usize, CliError> {
    let count = value
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|_| CliError::usage("subagent work --max-agents must be an integer from 1 to 8"))?
        .unwrap_or(1);
    if !(1..=8).contains(&count) {
        return Err(CliError::usage(
            "subagent work --max-agents must be an integer from 1 to 8",
        ));
    }
    Ok(count)
}

fn terminal_status(error: &CliError) -> SubagentStatus {
    if error.code == "approval_required" {
        return SubagentStatus::ApprovalRequired;
    }
    if error.code == "agent_controlled_stop"
        && error.details.get("reason").and_then(Value::as_str) == Some("cancelled")
    {
        return SubagentStatus::Cancelled;
    }
    SubagentStatus::Failed
}

fn terminal_result(
    status: SubagentStatus,
    outcome: Result<Value, CliError>,
    max_result_bytes: usize,
) -> (SubagentStatus, Value) {
    let result = match outcome {
        Ok(_) => json!({
            "code": "completed",
            "status": terminal_status_text(status),
        }),
        Err(error) => {
            let mut result = serde_json::Map::new();
            result.insert("code".to_owned(), Value::String(error.code.to_owned()));
            result.insert(
                "status".to_owned(),
                Value::String(terminal_status_text(status).to_owned()),
            );
            if error.code == "agent_controlled_stop"
                && error.details.get("reason").and_then(Value::as_str) == Some("cancelled")
            {
                result.insert("reason".to_owned(), Value::String("cancelled".to_owned()));
            }
            Value::Object(result)
        }
    };
    match serde_json::to_vec(&result) {
        Ok(bytes) if bytes.len() <= max_result_bytes => (status, result),
        _ => (SubagentStatus::Failed, json!(0)),
    }
}

fn stable_result(result: Option<&Value>) -> Option<Value> {
    let result = result?.as_object()?;
    let code = result.get("code")?.as_str()?;
    match code {
        "completed" if exact_summary(result, "completed", "completed") => {
            Some(json!({"code": "completed", "status": "completed"}))
        }
        "approval_required" if exact_summary(result, "approval_required", "approval_required") => {
            Some(json!({"code": "approval_required", "status": "approval_required"}))
        }
        "agent_controlled_stop"
            if result.len() == 3
                && result.get("code").and_then(Value::as_str) == Some("agent_controlled_stop")
                && result.get("status").and_then(Value::as_str) == Some("cancelled")
                && result.get("reason").and_then(Value::as_str) == Some("cancelled") =>
        {
            Some(json!({
                "code": "agent_controlled_stop",
                "status": "cancelled",
                "reason": "cancelled",
            }))
        }
        "worker_interrupted"
            if result.get("outcome_known").and_then(Value::as_bool) == Some(false)
                && result.keys().all(|key| {
                    matches!(
                        key.as_str(),
                        "code" | "outcome_known" | "reason" | "worker_id"
                    )
                })
                && result
                    .get("reason")
                    .is_none_or(|value| value.as_str().is_some_and(is_bounded_text))
                && result
                    .get("worker_id")
                    .is_none_or(|value| value.as_str().is_some_and(is_bounded_text)) =>
        {
            Some(json!({"code": "worker_interrupted", "outcome_known": false}))
        }
        "subagent_binding_changed"
            if exact_summary(result, "subagent_binding_changed", "failed") =>
        {
            Some(json!({"code": "subagent_binding_changed", "status": "failed"}))
        }
        "subagent_worktree_changed"
            if exact_summary(result, "subagent_worktree_changed", "failed") =>
        {
            Some(json!({"code": "subagent_worktree_changed", "status": "failed"}))
        }
        "execution_failed" if exact_summary(result, "execution_failed", "failed") => {
            Some(json!({"code": "execution_failed", "status": "failed"}))
        }
        _ => None,
    }
}

fn exact_summary(result: &serde_json::Map<String, Value>, code: &str, status: &str) -> bool {
    result.len() == 2
        && result.get("code").and_then(Value::as_str) == Some(code)
        && result.get("status").and_then(Value::as_str) == Some(status)
}

fn is_bounded_text(value: &str) -> bool {
    value.len() <= 256 && !value.chars().any(char::is_control)
}

fn terminal_status_text(status: SubagentStatus) -> &'static str {
    match status {
        SubagentStatus::ApprovalRequired => "approval_required",
        SubagentStatus::Completed => "completed",
        SubagentStatus::Failed => "failed",
        SubagentStatus::Cancelled => "cancelled",
        SubagentStatus::Preparing
        | SubagentStatus::Queued
        | SubagentStatus::Running
        | SubagentStatus::Interrupted => "failed",
    }
}

fn parse_session_id(value: &str) -> Result<SessionId, CliError> {
    SessionId::new(value.to_owned()).map_err(|_| CliError::usage("session ID is invalid"))
}

fn parse_execution_id(value: &str) -> Result<ExecutionId, CliError> {
    ExecutionId::new(value.to_owned()).map_err(|_| CliError::usage("execution ID is invalid"))
}

fn parse_u32(parsed: &super::ParsedArgs, name: &str, default: u32) -> Result<u32, CliError> {
    parsed
        .value(name)
        .map(|value| {
            value
                .parse::<u32>()
                .map_err(|_| CliError::usage(format!("--{name} must be an unsigned integer")))
        })
        .transpose()
        .map(|value| value.unwrap_or(default))
}

fn parse_u64(parsed: &super::ParsedArgs, name: &str, default: u64) -> Result<u64, CliError> {
    parsed
        .value(name)
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|_| CliError::usage(format!("--{name} must be an unsigned integer")))
        })
        .transpose()
        .map(|value| value.unwrap_or(default))
}

fn parse_u8(parsed: &super::ParsedArgs, name: &str, default: u8) -> Result<u8, CliError> {
    parsed
        .value(name)
        .map(|value| {
            value
                .parse::<u8>()
                .map_err(|_| CliError::usage(format!("--{name} must be an unsigned integer")))
        })
        .transpose()
        .map(|value| value.unwrap_or(default))
}

fn parse_usize(parsed: &super::ParsedArgs, name: &str, default: usize) -> Result<usize, CliError> {
    parsed
        .value(name)
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|_| CliError::usage(format!("--{name} must be an unsigned integer")))
        })
        .transpose()
        .map(|value| value.unwrap_or(default))
}

fn current_commit(workspace: &std::path::Path) -> Result<String, CliError> {
    let output = Command::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| CliError::execution("workspace is not a Git repository", json!({})))?;
    let commit = String::from_utf8(output.stdout)
        .map_err(|_| CliError::execution("workspace Git commit is invalid", json!({})))?;
    if !output.status.success() {
        return Err(CliError::execution(
            "workspace is not a Git repository",
            json!({}),
        ));
    }
    Ok(commit.trim().to_ascii_lowercase())
}

fn allocate_subagent_id(prefix: &str) -> Result<SubagentId, CliError> {
    SubagentId::new(allocated_id(prefix))
        .map_err(|_| CliError::internal("could not allocate a subagent ID", json!({})))
}

fn allocate_job_id() -> Result<JobId, CliError> {
    JobId::new(allocated_id("subagent-job"))
        .map_err(|_| CliError::internal("could not allocate a subagent job ID", json!({})))
}

fn allocate_session_id(prefix: &str) -> Result<SessionId, CliError> {
    SessionId::new(allocated_id(prefix))
        .map_err(|_| CliError::internal("could not allocate a subagent session ID", json!({})))
}

fn allocate_execution_id(prefix: &str) -> Result<ExecutionId, CliError> {
    ExecutionId::new(allocated_id(prefix))
        .map_err(|_| CliError::internal("could not allocate a subagent execution ID", json!({})))
}

fn allocate_worker_id() -> Result<JobWorkerId, CliError> {
    JobWorkerId::new(allocated_id("subagent-worker"))
        .map_err(|_| CliError::internal("could not allocate a subagent worker ID", json!({})))
}

fn allocated_id(prefix: &str) -> String {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let sequence = NEXT_SUBAGENT_ID.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{}-{nonce}-{sequence}", std::process::id())
}

fn subagent_contract_error(error: pandora_types::SubagentContractError) -> CliError {
    CliError::usage(error.to_string())
}

fn subagent_coordinator_error(error: SubagentCoordinatorError) -> CliError {
    CliError::execution(error.to_string(), json!({}))
}

fn fleet_error(error: FleetError) -> CliError {
    CliError::execution(error.to_string(), json!({}))
}

fn subagent_store_error(error: SubagentStoreError) -> CliError {
    let message = error.to_string();
    match error {
        SubagentStoreError::SubagentNotFound | SubagentStoreError::InvalidTransition { .. } => {
            CliError::execution(message, json!({}))
        }
        _ => CliError::internal(message, json!({})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_result_removes_approval_payloads_and_provider_details() {
        let error = CliError::approval(
            "approval required",
            json!({
                "approval_id": "approval-secret",
                "provider_response": "provider-secret",
                "reason": "approval_required",
            }),
        );

        let result = terminal_result(SubagentStatus::ApprovalRequired, Err(error), 8_192);

        assert_eq!(result.0, SubagentStatus::ApprovalRequired);
        assert_eq!(result.1["code"], "approval_required");
        assert_eq!(result.1["status"], "approval_required");
        assert!(!result.1.to_string().contains("approval_id"));
        assert!(!result.1.to_string().contains("approval-secret"));
        assert!(!result.1.to_string().contains("provider-secret"));
    }

    #[test]
    fn stable_result_rejects_arbitrary_legacy_shapes() {
        for result in [
            json!("approval-secret"),
            json!(["provider-secret"]),
            json!({"reason": {"approval_id": "approval-secret"}}),
            json!({"code": "completed", "raw_response": "provider-secret"}),
        ] {
            assert_eq!(stable_result(Some(&result)), None, "{result}");
        }
    }

    #[test]
    fn stable_result_keeps_a_valid_known_terminal_summary() {
        let result = json!({
            "code": "worker_interrupted",
            "outcome_known": false,
        });

        assert_eq!(stable_result(Some(&result)), Some(result));
    }

    #[test]
    fn stable_result_keeps_a_valid_cancelled_summary() {
        let result = json!({
            "code": "agent_controlled_stop",
            "status": "cancelled",
            "reason": "cancelled",
        });

        assert_eq!(stable_result(Some(&result)), Some(result));
    }

    #[test]
    fn terminal_result_keeps_an_exact_budget_boundary() {
        let result = json!({"code": "completed", "status": "completed"});
        let budget = serde_json::to_vec(&result)
            .expect("terminal result should serialize")
            .len();

        assert_eq!(
            terminal_result(SubagentStatus::Completed, Ok(json!({})), budget),
            (SubagentStatus::Completed, result)
        );
    }

    #[test]
    fn terminal_result_overflow_fails_with_a_value_that_fits_the_tiny_budget() {
        let (status, result) = terminal_result(
            SubagentStatus::Completed,
            Ok(json!({"output": "provider response that does not fit"})),
            1,
        );

        assert_eq!(status, SubagentStatus::Failed);
        assert_eq!(result, json!(0));
        assert_eq!(serde_json::to_vec(&result).unwrap().len(), 1);
    }
}
