use super::{load_config, parse_options, require_config_file, session_scope, timestamp};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::sessions::{MAX_MEMORY_RECALL_RECORDS, SessionStore};
use pandora_runtime::{
    ApprovalError, ApprovalRequest, ApprovalStatus, ApprovalStore, MemoryEngine, MemoryError,
    MemorySynthesisScheduleError, MemorySynthesisScheduleStore,
};
use pandora_types::{
    ContextClassification, ExecutionId, GeneId, JobWorkerId, MemoryApproval, MemoryId, MemoryKind,
    MemoryRecord, MemoryScope, MemoryTier, RequestDigest, RunLoopId, Timestamp, hash_artifact,
};
use serde_json::json;
use std::collections::{HashMap, HashSet, VecDeque};

const MAX_L0_ENTRIES: usize = 64;
const MEMORY_PROMOTION_POLICY_VERSION: u32 = 1;
const MEMORY_PROMOTION_TTL_SECONDS: u64 = 900;
const MEMORY_PROMOTION_GENE: &str = "memory.promote";

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage(
            "memory requires 'recall', 'audit', 'forget', 'compact', 'promote', 'synthesize', 'consolidate', 'provenance', or 'schedule'",
        )
    })?;
    match subcommand.as_str() {
        "recall" => recall(&args[1..]),
        "audit" => audit(&args[1..]),
        "forget" => forget(&args[1..]),
        "compact" => compact(&args[1..]),
        "promote" => promote(&args[1..]),
        "synthesize" => synthesize(&args[1..]),
        "consolidate" => consolidate(&args[1..]),
        "provenance" => provenance(&args[1..]),
        "schedule" => schedule(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown memory command '{unknown}'"
        ))),
    }
}

fn schedule(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage(
            "memory schedule requires 'create', 'list', 'disable', 'claim', 'run', or 'runs'",
        )
    })?;
    match subcommand.as_str() {
        "create" => schedule_create(&args[1..]),
        "list" => schedule_list(&args[1..]),
        "disable" => schedule_disable(&args[1..]),
        "claim" => schedule_claim(&args[1..]),
        "run" => schedule_run(&args[1..]),
        "runs" => schedule_runs(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown memory schedule command '{unknown}'"
        ))),
    }
}

fn schedule_create(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "id",
            "name",
            "session",
            "provider",
            "memory-id",
            "kind",
            "summary",
            "classification",
            "interval-seconds",
        ],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "memory schedule create does not accept positional arguments",
        ));
    }
    let id = RunLoopId::new(
        required_option(&parsed, "id", "memory schedule create requires '--id <id>'")?.to_owned(),
    )
    .map_err(|_| CliError::usage("memory schedule ID is invalid"))?;
    let name = required_option(
        &parsed,
        "name",
        "memory schedule create requires '--name <name>'",
    )?;
    let session = parse_session_option(&parsed, "session")?;
    let provider = required_option(
        &parsed,
        "provider",
        "memory schedule create requires '--provider <name>'",
    )?;
    let memory_id = MemoryId::new(
        required_option(
            &parsed,
            "memory-id",
            "memory schedule create requires '--memory-id <id>'",
        )?
        .to_owned(),
    )
    .map_err(|_| CliError::usage("memory ID is invalid"))?;
    let kind = parse_memory_kind(parsed.value("kind"))?;
    let summary = required_option(
        &parsed,
        "summary",
        "memory schedule create requires '--summary <text>'",
    )?;
    let classification = parse_memory_classification(parsed.value("classification"))?;
    let interval_seconds = parsed
        .value("interval-seconds")
        .ok_or_else(|| {
            CliError::usage("memory schedule create requires '--interval-seconds <seconds>'")
        })?
        .parse::<u64>()
        .map_err(|_| CliError::usage("interval-seconds must be an integer"))?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let (principal, tenant, workspace) = session_scope();
    let schedule = memory_schedule_store(&config)?
        .create(
            &id,
            &principal,
            &tenant,
            &workspace,
            &session,
            name,
            provider,
            &memory_id,
            kind,
            summary,
            classification,
            interval_seconds,
            timestamp(),
        )
        .map_err(memory_schedule_error)?;
    Ok(success(
        "memory schedule create",
        schedule_value(&schedule),
        format!("Created memory synthesis schedule {}", schedule.id()),
    ))
}

fn schedule_list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "memory schedule list does not accept positional arguments",
        ));
    }
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let (principal, tenant, workspace) = session_scope();
    let schedules = memory_schedule_store(&config)?
        .list(&principal, &tenant, &workspace)
        .map_err(memory_schedule_error)?;
    let count = schedules.len();
    Ok(success(
        "memory schedule list",
        json!({
            "schedules": schedules.iter().map(schedule_value).collect::<Vec<_>>(),
            "count": count,
            "durability": "memory-schedule-store"
        }),
        format!("Listed {count} memory synthesis schedule(s)"),
    ))
}

fn schedule_disable(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "id"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "memory schedule disable does not accept positional arguments",
        ));
    }
    let id = memory_schedule_id(&parsed)?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let (principal, tenant, workspace) = session_scope();
    memory_schedule_store(&config)?
        .disable(&id, &principal, &tenant, &workspace)
        .map_err(memory_schedule_error)?;
    Ok(success(
        "memory schedule disable",
        json!({
            "id": id,
            "enabled": false,
            "durability": "memory-schedule-store"
        }),
        format!("Disabled memory synthesis schedule {id}"),
    ))
}

fn schedule_claim(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "worker", "limit"],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "memory schedule claim does not accept positional arguments",
        ));
    }
    let worker = memory_schedule_worker(&parsed)?;
    let limit = parsed
        .value("limit")
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|_| CliError::usage("limit must be an integer"))
        })
        .transpose()?
        .unwrap_or(1);
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let (principal, tenant, workspace) = session_scope();
    let runs = memory_schedule_store(&config)?
        .claim_due(&principal, &tenant, &workspace, &worker, timestamp(), limit)
        .map_err(memory_schedule_error)?;
    let count = runs.len();
    Ok(success(
        "memory schedule claim",
        json!({
            "runs": runs.iter().map(schedule_run_value).collect::<Vec<_>>(),
            "count": count,
            "worker": worker,
            "durability": "memory-schedule-store"
        }),
        format!("Claimed {count} due memory synthesis run(s)"),
    ))
}

fn schedule_run(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "id", "worker"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "memory schedule run does not accept positional arguments",
        ));
    }
    let id = memory_schedule_id(&parsed)?;
    let worker = memory_schedule_worker(&parsed)?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let (principal, tenant, workspace) = session_scope();
    let store = memory_schedule_store(&config)?;
    let schedule = store
        .list(&principal, &tenant, &workspace)
        .map_err(memory_schedule_error)?
        .into_iter()
        .find(|schedule| schedule.id() == &id)
        .ok_or_else(|| {
            CliError::execution(
                "memory synthesis schedule was not found",
                json!({"schedule_id": id}),
            )
        })?;
    let run = store
        .claim_due_for(
            &principal,
            &tenant,
            &workspace,
            Some(&id),
            &worker,
            timestamp(),
            1,
        )
        .map_err(memory_schedule_error)?
        .into_iter()
        .next()
        .ok_or_else(|| {
            CliError::execution(
                "memory synthesis schedule has no due run",
                json!({"schedule_id": id, "worker": worker}),
            )
        })?;
    let scope = MemoryScope::new(
        tenant.clone(),
        workspace.clone(),
        schedule.session_id().clone(),
        schedule.provider().to_owned(),
    )
    .map_err(|error| CliError::usage(error.to_string()))?;
    let engine = open_engine(&config, &principal)?;
    let outcome = (|| {
        let snapshot = engine
            .synthesis_snapshot(&scope, timestamp())
            .map_err(memory_error)?;
        let proposal = engine
            .propose_synthesis(
                &snapshot,
                schedule.memory_id().as_str().to_owned(),
                schedule.kind(),
                schedule.summary().to_owned(),
                schedule.classification(),
                timestamp(),
            )
            .map_err(memory_error)?;
        let committed = engine
            .commit_synthesis(&proposal, timestamp())
            .map_err(memory_error)?;
        Ok::<_, CliError>((snapshot.digest().to_owned(), committed))
    })();
    let finished_at = timestamp();
    match outcome {
        Ok((snapshot_digest, committed)) => {
            store
                .complete(
                    &id,
                    &principal,
                    &tenant,
                    &workspace,
                    run.scheduled_for(),
                    &worker,
                    true,
                    finished_at,
                    Some(&snapshot_digest),
                    Some(committed.id()),
                    None,
                )
                .map_err(memory_schedule_error)?;
            Ok(success(
                "memory schedule run",
                json!({
                    "schedule_id": id,
                    "run": schedule_run_value(&run),
                    "committed": record_value(&committed),
                    "snapshot_digest": snapshot_digest,
                    "durability": "memory-schedule-store",
                    "promotion_required": true
                }),
                format!("Completed scheduled memory synthesis {}", committed.id()),
            ))
        }
        Err(error) => {
            let failure = error.message.clone();
            store
                .complete(
                    &id,
                    &principal,
                    &tenant,
                    &workspace,
                    run.scheduled_for(),
                    &worker,
                    false,
                    finished_at,
                    None,
                    None,
                    Some(&failure),
                )
                .map_err(memory_schedule_error)?;
            Err(error)
        }
    }
}

fn schedule_runs(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "id"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "memory schedule runs does not accept positional arguments",
        ));
    }
    let requested = parsed
        .value("id")
        .map(|value| {
            RunLoopId::new(value.to_owned())
                .map_err(|_| CliError::usage("memory schedule ID is invalid"))
        })
        .transpose()?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let (principal, tenant, workspace) = session_scope();
    let runs = memory_schedule_store(&config)?
        .list_runs(&principal, &tenant, &workspace, requested.as_ref())
        .map_err(memory_schedule_error)?;
    let count = runs.len();
    Ok(success(
        "memory schedule runs",
        json!({
            "runs": runs.iter().map(schedule_run_value).collect::<Vec<_>>(),
            "count": count,
            "durability": "memory-schedule-store"
        }),
        format!("Listed {count} memory synthesis run(s)"),
    ))
}

fn consolidate(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "provider",
            "source-session",
            "target-session",
            "source-id",
            "target-id",
            "yes",
        ],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "memory consolidate does not accept positional arguments",
        ));
    }
    let provider = parsed
        .value("provider")
        .ok_or_else(|| CliError::usage("memory consolidate requires '--provider <name>'"))?;
    let source_session = parse_session_option(&parsed, "source-session")?;
    let target_session = parse_session_option(&parsed, "target-session")?;
    if source_session == target_session {
        return Err(CliError::usage(
            "memory consolidate requires different source and target sessions",
        ));
    }
    let source_id = MemoryId::new(
        parsed
            .value("source-id")
            .ok_or_else(|| {
                CliError::usage("memory consolidate requires '--source-id <memory-id>'")
            })?
            .to_owned(),
    )
    .map_err(|_| CliError::usage("source memory ID is invalid"))?;
    let target_id = MemoryId::new(
        parsed
            .value("target-id")
            .ok_or_else(|| {
                CliError::usage("memory consolidate requires '--target-id <memory-id>'")
            })?
            .to_owned(),
    )
    .map_err(|_| CliError::usage("target memory ID is invalid"))?;
    let (_, tenant, workspace) = session_scope();
    let source_scope = MemoryScope::new(
        tenant.clone(),
        workspace.clone(),
        source_session.clone(),
        provider.to_owned(),
    )
    .map_err(|error| CliError::usage(error.to_string()))?;
    let target_scope = MemoryScope::new(
        tenant,
        workspace,
        target_session.clone(),
        provider.to_owned(),
    )
    .map_err(|error| CliError::usage(error.to_string()))?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let principal = session_scope().0;
    let session_store = SessionStore::open(config.data_dir().join("sessions.sqlite3"))
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    session_store
        .resume(
            &target_session,
            &principal,
            target_scope.tenant_id(),
            target_scope.workspace_id(),
        )
        .map_err(|error| CliError::policy(error.to_string(), json!({})))?;
    let engine = open_engine(&config, &principal)?;
    let source = engine
        .try_recall(&source_scope, MemoryTier::L1, timestamp())
        .map_err(memory_error)?
        .into_iter()
        .find(|record| record.id() == &source_id)
        .ok_or_else(|| memory_error(MemoryError::NotFound))?;
    if source.classification() == ContextClassification::Sensitive {
        return Err(CliError::policy(
            "sensitive memory cannot cross session boundaries",
            json!({}),
        ));
    }
    let provenance = format!(
        "consolidated-from:tenant={};workspace={};session={};provider={};memory={};source-provenance={}",
        source_scope.tenant_id(),
        source_scope.workspace_id(),
        source_scope.session_id(),
        source_scope.provider(),
        source.id(),
        hash_artifact(source.provenance().as_bytes()),
    );
    let now = timestamp();
    let candidate = MemoryRecord::new_l1(
        target_id.as_str(),
        source.kind(),
        target_scope.clone(),
        source.summary().to_owned(),
        source.classification(),
        now,
        provenance,
    )
    .map_err(|error| CliError::execution(error.to_string(), json!({})))?;
    if !parsed.values.contains_key("yes") {
        return Ok(success(
            "memory consolidate",
            json!({
                "dry_run": true,
                "source": record_value(&source),
                "candidate": record_value(&candidate),
                "policy": "same-tenant-workspace-provider; L1 non-sensitive only",
                "durability": "session-store",
            }),
            format!(
                "Dry run: memory {} would be consolidated into {}",
                source.id(),
                candidate.id()
            ),
        ));
    }
    let consolidated = engine
        .distill_l1(
            candidate.scope().clone(),
            candidate.id().as_str(),
            candidate.kind(),
            candidate.summary().to_owned(),
            candidate.classification(),
            candidate.created_at(),
            candidate.provenance().to_owned(),
        )
        .map_err(memory_error)?;
    Ok(success(
        "memory consolidate",
        json!({
            "dry_run": false,
            "source": record_value(&source),
            "consolidated": record_value(&consolidated),
            "policy": "same-tenant-workspace-provider; L1 non-sensitive only",
            "durability": "session-store",
        }),
        format!(
            "Consolidated memory {} into session {}",
            consolidated.id(),
            target_session
        ),
    ))
}

fn parse_session_option(
    parsed: &super::ParsedArgs,
    name: &'static str,
) -> Result<pandora_types::SessionId, CliError> {
    let value = parsed
        .value(name)
        .ok_or_else(|| CliError::usage(format!("memory consolidate requires '--{name} <id>'")))?;
    pandora_types::SessionId::new(value.to_owned())
        .map_err(|_| CliError::usage(format!("{name} is invalid")))
}

fn provenance(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "session", "provider"],
    )?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "memory provenance requires exactly one memory ID",
        ));
    }
    let root_id = MemoryId::new(parsed.positionals[0].clone())
        .map_err(|_| CliError::usage("memory ID is invalid"))?;
    let (_, scope) = scope(&parsed)?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let principal = session_scope().0;
    let engine = open_engine(&config, &principal)?;
    let mut records = HashMap::new();
    for tier in [MemoryTier::L1, MemoryTier::L2] {
        for record in engine
            .try_recall(&scope, tier, timestamp())
            .map_err(memory_error)?
        {
            records.insert(record.id().clone(), record);
        }
    }
    if !records.contains_key(&root_id) {
        return Err(memory_error(MemoryError::NotFound));
    }

    const MAX_NODES: usize = 64;
    let mut queue = VecDeque::from([root_id.clone()]);
    let mut visited = HashSet::new();
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    while let Some(id) = queue.pop_front() {
        if !visited.insert(id.clone()) || nodes.len() >= MAX_NODES {
            continue;
        }
        let Some(record) = records.get(&id) else {
            continue;
        };
        nodes.push(record_value(record));
        for evidence_id in record.evidence_ids() {
            if records.contains_key(evidence_id) {
                edges.push(json!({ "from": record.id(), "to": evidence_id }));
                if !visited.contains(evidence_id) {
                    queue.push_back(evidence_id.clone());
                }
            }
        }
    }
    Ok(success(
        "memory provenance",
        json!({
            "root_id": root_id,
            "scope": scope_value(&scope),
            "nodes": nodes,
            "edges": edges,
            "bounded": true,
            "max_nodes": MAX_NODES,
            "durability": "session-store",
        }),
        format!("Inspected provenance for {root_id}"),
    ))
}

fn recall(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "session",
            "provider",
            "tier",
            "limit",
            "id",
        ],
    )?;
    let (session_id, scope) = scope(&parsed)?;
    let tier = parse_tier(parsed.value("tier"))?;
    let limit = parse_limit(parsed.value("limit"))?;
    let requested_id = parsed
        .value("id")
        .map(|value| MemoryId::new(value.to_owned()))
        .transpose()
        .map_err(|_| CliError::usage("memory ID is invalid"))?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let principal = session_scope().0;
    let engine = open_engine(&config, &principal)?;
    let mut records = engine
        .try_recall(&scope, tier, timestamp())
        .map_err(memory_error)?;
    if let Some(requested_id) = &requested_id {
        records.retain(|record| record.id() == requested_id);
        if records.is_empty() {
            return Err(memory_error(MemoryError::NotFound));
        }
    }
    records.truncate(usize::from(limit));
    let values = records.iter().map(record_value).collect::<Vec<_>>();
    let count = values.len();
    Ok(success(
        "memory recall",
        json!({
            "scope": scope_value(&scope),
            "tier": tier.as_str(),
            "records": values,
            "count": count,
            "limit": limit,
            "durability": "session-store",
        }),
        format!(
            "Recalled {count} {} record(s) for {session_id}",
            tier.as_str()
        ),
    ))
}

fn audit(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "session", "provider"],
    )?;
    let (_, scope) = scope(&parsed)?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let principal = session_scope().0;
    let engine = open_engine(&config, &principal)?;
    let entries = engine.try_audit(&scope).map_err(memory_error)?;
    let values = entries.iter().map(audit_value).collect::<Vec<_>>();
    let count = values.len();
    Ok(success(
        "memory audit",
        json!({
            "scope": scope_value(&scope),
            "entries": values,
            "count": count,
            "durability": "session-store",
        }),
        format!("Listed {count} memory audit record(s)"),
    ))
}

fn synthesize(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "session",
            "provider",
            "id",
            "kind",
            "summary",
            "classification",
            "yes",
        ],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "memory synthesize does not accept positional arguments",
        ));
    }
    let memory_id = MemoryId::new(
        parsed
            .value("id")
            .ok_or_else(|| CliError::usage("memory synthesize requires '--id <memory-id>'"))?
            .to_owned(),
    )
    .map_err(|_| CliError::usage("memory ID is invalid"))?;
    let kind = parse_memory_kind(parsed.value("kind"))?;
    let summary = parsed
        .value("summary")
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| CliError::usage("memory synthesize requires a non-empty '--summary'"))?;
    let classification = parse_memory_classification(parsed.value("classification"))?;
    if !matches!(
        classification,
        ContextClassification::Public | ContextClassification::Internal
    ) {
        return Err(CliError::usage(
            "memory synthesis classification must be 'public' or 'internal'",
        ));
    }
    let (_, scope) = scope(&parsed)?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let principal = session_scope().0;
    let engine = open_engine(&config, &principal)?;
    let now = timestamp();
    let snapshot = engine
        .synthesis_snapshot(&scope, now)
        .map_err(memory_error)?;
    let proposal = engine
        .propose_synthesis(
            &snapshot,
            memory_id.as_str().to_owned(),
            kind,
            summary,
            classification,
            now,
        )
        .map_err(memory_error)?;
    engine
        .verify_synthesis(&proposal, timestamp())
        .map_err(memory_error)?;
    let evidence_ids = proposal
        .evidence_ids()
        .iter()
        .map(|id| id.as_str())
        .collect::<Vec<_>>();
    if parsed.values.contains_key("yes") {
        let committed = engine
            .commit_synthesis(&proposal, timestamp())
            .map_err(memory_error)?;
        return Ok(success(
            "memory synthesize",
            json!({
                "dry_run": false,
                "committed": record_value(&committed),
                "snapshot_digest": proposal.snapshot_digest(),
                "evidence_ids": evidence_ids,
                "promotion_required": true,
            }),
            format!(
                "Committed synthesized memory {} as an L1 candidate",
                committed.id()
            ),
        ));
    }
    Ok(success(
        "memory synthesize",
        json!({
            "dry_run": true,
            "candidate": record_value(proposal.candidate()),
            "snapshot_digest": proposal.snapshot_digest(),
            "captured_at": snapshot.captured_at().as_unix_seconds(),
            "evidence_ids": evidence_ids,
            "would_commit": true,
            "promotion_required": true,
        }),
        format!(
            "Previewed synthesized memory from {} evidence record(s); rerun with --yes to commit",
            proposal.evidence_ids().len()
        ),
    ))
}

fn forget(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "session",
            "provider",
            "yes",
        ],
    )?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "memory forget requires exactly one memory ID",
        ));
    }
    let memory_id = MemoryId::new(parsed.positionals[0].clone())
        .map_err(|_| CliError::usage("memory ID is invalid"))?;
    let (_, scope) = scope(&parsed)?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let principal = session_scope().0;
    let engine = open_engine(&config, &principal)?;
    ensure_memory_exists(&engine, &scope, &memory_id)?;
    if !parsed.values.contains_key("yes") {
        return Ok(success(
            "memory forget",
            json!({
                "dry_run": true,
                "memory_id": memory_id,
                "scope": scope_value(&scope),
                "would_revoke": true,
            }),
            format!(
                "Dry run: memory {} would be revoked; rerun with --yes",
                memory_id
            ),
        ));
    }
    engine
        .forget(&scope, &memory_id, timestamp())
        .map_err(memory_error)?;
    Ok(success(
        "memory forget",
        json!({
            "dry_run": false,
            "memory_id": memory_id,
            "scope": scope_value(&scope),
            "revoked": true,
        }),
        format!("Revoked memory {memory_id}"),
    ))
}

fn compact(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "session",
            "provider",
            "before",
            "yes",
        ],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "memory compact does not accept positional arguments",
        ));
    }
    let revoked_before_or_at = required_option(
        &parsed,
        "before",
        "memory compact requires '--before <unix-seconds>'",
    )?
    .parse::<u64>()
    .map(Timestamp::from_unix_seconds)
    .map_err(|_| CliError::usage("memory compact before time must be Unix seconds"))?;
    let (_, scope) = scope(&parsed)?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let principal = session_scope().0;
    let engine = open_engine(&config, &principal)?;
    let compactable_records = engine
        .preview_compact_revoked(&scope, revoked_before_or_at)
        .map_err(memory_error)?;
    let boundary = json!({
        "tombstones_retained": true,
        "audit_retained": true,
        "secure_erasure_guaranteed": false,
        "storage_guidance": "Database pages, WAL files, backups, and storage snapshots require separate lifecycle controls.",
    });
    if !parsed.values.contains_key("yes") {
        return Ok(success(
            "memory compact",
            json!({
                "dry_run": true,
                "scope": scope_value(&scope),
                "revoked_before_or_at": revoked_before_or_at.as_unix_seconds(),
                "compactable_records": compactable_records,
                "would_compact": compactable_records > 0,
                "boundary": boundary,
            }),
            format!(
                "Dry run: {compactable_records} revoked memory record(s) are eligible for logical compaction; rerun with --yes"
            ),
        ));
    }
    let compacted_records = engine
        .compact_revoked(&scope, revoked_before_or_at)
        .map_err(memory_error)?;
    Ok(success(
        "memory compact",
        json!({
            "dry_run": false,
            "scope": scope_value(&scope),
            "revoked_before_or_at": revoked_before_or_at.as_unix_seconds(),
            "compacted_records": compacted_records,
            "boundary": boundary,
        }),
        format!(
            "Compacted {compacted_records} revoked logical memory record(s); tombstones and audit evidence were retained"
        ),
    ))
}

fn promote(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "session",
            "provider",
            "approval",
        ],
    )?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "memory promote requires exactly one L1 memory ID",
        ));
    }
    let memory_id = MemoryId::new(parsed.positionals[0].clone())
        .map_err(|_| CliError::usage("memory ID is invalid"))?;
    let (session_id, scope) = scope(&parsed)?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let principal = session_scope().0;
    let engine = open_engine(&config, &principal)?;
    let candidate = engine
        .try_recall(&scope, MemoryTier::L1, timestamp())
        .map_err(memory_error)?
        .into_iter()
        .find(|record| record.id() == &memory_id)
        .ok_or_else(|| memory_error(MemoryError::NotFound))?;
    let (execution_id, gene_id, request_digest) = promotion_identity(&scope, &candidate)?;
    let approvals = open_approval_store(&config)?;
    let Some(approval_id) = parsed.value("approval") else {
        return create_promotion_approval(
            &approvals,
            &session_id,
            &principal,
            &candidate,
            execution_id,
            gene_id,
            request_digest,
        );
    };
    let approval = approvals
        .inspect(approval_id, &principal)
        .map_err(approval_error)?;
    let now = timestamp();
    validate_promotion_approval(
        &approval,
        &session_id,
        &principal,
        &gene_id,
        &request_digest,
        now,
    )?;
    let approver = approval.approver_id().ok_or_else(|| {
        CliError::approval("memory promotion approval has no approver", json!({}))
    })?;
    let memory_approval = MemoryApproval::new(approval.id(), approver.as_str())
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let promoted = engine
        .promote_l2(&scope, &memory_id, Some(memory_approval), now)
        .map_err(memory_error)?;
    approvals
        .consume(
            approval_id,
            &principal,
            &session_id,
            &execution_id,
            &gene_id,
            &request_digest,
            timestamp(),
        )
        .map_err(approval_error)?;
    Ok(success(
        "memory promote",
        json!({
            "promoted": record_value(&promoted),
            "approval_id": approval_id,
            "approval_consumed": true,
        }),
        format!("Promoted memory {} to L2", promoted.id()),
    ))
}

fn create_promotion_approval(
    approvals: &ApprovalStore,
    session_id: &pandora_types::SessionId,
    principal: &pandora_types::PrincipalId,
    candidate: &MemoryRecord,
    execution_id: ExecutionId,
    gene_id: GeneId,
    request_digest: RequestDigest,
) -> Result<CommandResult, CliError> {
    let now = timestamp();
    let expires_at = Timestamp::from_unix_seconds(
        now.as_unix_seconds()
            .saturating_add(MEMORY_PROMOTION_TTL_SECONDS),
    );
    let approval_id = format!(
        "memory-promote-{}-{}",
        request_digest
            .as_str()
            .rsplit(':')
            .next()
            .unwrap_or("request"),
        now.as_unix_seconds()
    );
    let request = ApprovalRequest::new(
        approval_id,
        session_id.clone(),
        execution_id.clone(),
        principal.clone(),
        gene_id,
        request_digest.clone(),
        format!(
            "Promote memory {} ({}) for provider {}",
            candidate.id(),
            candidate.kind().as_str(),
            candidate.scope().provider()
        ),
        MEMORY_PROMOTION_POLICY_VERSION,
        expires_at,
    )
    .map_err(approval_error)?;
    let pending = approvals.create(request).map_err(approval_error)?;
    Err(CliError::approval(
        "memory promotion requires explicit approval",
        json!({
            "approval_id": pending.id(),
            "session_id": pending.session_id(),
            "execution_id": pending.execution_id(),
            "memory_id": candidate.id(),
            "expires_at": pending.expires_at().as_unix_seconds(),
        }),
    ))
}

fn validate_promotion_approval(
    approval: &pandora_runtime::PendingApproval,
    session_id: &pandora_types::SessionId,
    principal: &pandora_types::PrincipalId,
    gene_id: &GeneId,
    request_digest: &RequestDigest,
    now: Timestamp,
) -> Result<(), CliError> {
    let status = approval.status_at(now);
    if status != ApprovalStatus::Approved {
        return Err(CliError::approval(
            "memory promotion approval is not currently approved",
            json!({"status": status.as_str()}),
        ));
    }
    if approval.session_id() != session_id
        || approval.principal_id() != principal
        || approval.gene_id() != gene_id
        || approval.request_digest() != request_digest
    {
        return Err(CliError::policy(
            "memory promotion approval does not match the requested memory",
            json!({}),
        ));
    }
    Ok(())
}

fn promotion_identity(
    scope: &MemoryScope,
    candidate: &MemoryRecord,
) -> Result<(ExecutionId, GeneId, RequestDigest), CliError> {
    let material = format!(
        "session={};tenant={};workspace={};provider={};memory={};kind={};tier={}",
        scope.session_id(),
        scope.tenant_id(),
        scope.workspace_id(),
        scope.provider(),
        candidate.id(),
        candidate.kind().as_str(),
        candidate.tier().as_str(),
    );
    let hash = hash_artifact(material.as_bytes());
    let hash_suffix = hash.strip_prefix("sha256:").unwrap_or(&hash);
    let execution_id = ExecutionId::new(format!("memory-promote-{hash_suffix}"))
        .map_err(|_| CliError::internal("could not create memory approval identity", json!({})))?;
    let gene_id = GeneId::new(MEMORY_PROMOTION_GENE)
        .map_err(|_| CliError::internal("could not create memory approval gene", json!({})))?;
    let request_digest = RequestDigest::new(format!("pandora-memory-promote-v1:{hash}"))
        .map_err(|_| CliError::internal("could not create memory approval digest", json!({})))?;
    Ok((execution_id, gene_id, request_digest))
}

fn ensure_memory_exists(
    engine: &MemoryEngine,
    scope: &MemoryScope,
    memory_id: &MemoryId,
) -> Result<(), CliError> {
    for tier in [MemoryTier::L1, MemoryTier::L2] {
        if engine
            .try_recall(scope, tier, timestamp())
            .map_err(memory_error)?
            .iter()
            .any(|record| record.id() == memory_id)
        {
            return Ok(());
        }
    }
    Err(memory_error(MemoryError::NotFound))
}

fn open_engine(
    config: &pandora_runtime::config::RuntimeConfig,
    principal: &pandora_types::PrincipalId,
) -> Result<MemoryEngine, CliError> {
    MemoryEngine::open(
        config.data_dir().join("sessions.sqlite3"),
        MAX_L0_ENTRIES,
        principal.clone(),
    )
    .map_err(memory_error)
}

fn open_approval_store(
    config: &pandora_runtime::config::RuntimeConfig,
) -> Result<ApprovalStore, CliError> {
    ApprovalStore::open(config.data_dir().join("sessions.sqlite3")).map_err(approval_error)
}

fn scope(parsed: &super::ParsedArgs) -> Result<(pandora_types::SessionId, MemoryScope), CliError> {
    let session_id = parsed
        .value("session")
        .ok_or_else(|| CliError::usage("memory commands require '--session <id>'"))
        .and_then(|value| {
            pandora_types::SessionId::new(value.to_owned())
                .map_err(|_| CliError::usage("session ID is invalid"))
        })?;
    let provider = parsed
        .value("provider")
        .ok_or_else(|| CliError::usage("memory commands require '--provider <name>'"))?;
    let (_, tenant, workspace) = session_scope();
    let scope = MemoryScope::new(tenant, workspace, session_id.clone(), provider.to_owned())
        .map_err(|error| CliError::usage(error.to_string()))?;
    Ok((session_id, scope))
}

fn parse_tier(value: Option<&str>) -> Result<MemoryTier, CliError> {
    match value {
        Some("l1") => Ok(MemoryTier::L1),
        Some("l2") => Ok(MemoryTier::L2),
        Some("l0") => Err(CliError::usage(
            "L0 memory is process-local and cannot be recalled from the CLI",
        )),
        Some(_) => Err(CliError::usage("memory tier must be 'l1' or 'l2'")),
        None => Err(CliError::usage("memory recall requires '--tier <l1|l2>'")),
    }
}

fn parse_memory_kind(value: Option<&str>) -> Result<MemoryKind, CliError> {
    match value {
        Some("execution_evidence") => Ok(MemoryKind::ExecutionEvidence),
        Some("decision") => Ok(MemoryKind::Decision),
        Some("failure") => Ok(MemoryKind::Failure),
        Some("benchmark") => Ok(MemoryKind::Benchmark),
        Some("lesson") => Ok(MemoryKind::Lesson),
        Some("lineage") => Ok(MemoryKind::Lineage),
        Some("trace") | Some("policy_decision") | Some("replacement") => {
            Err(CliError::usage("memory synthesis kind must be an L1 kind"))
        }
        Some(_) => Err(CliError::usage(
            "memory kind must be execution_evidence, decision, failure, benchmark, lesson, or lineage",
        )),
        None => Ok(MemoryKind::Lesson),
    }
}

fn parse_memory_classification(value: Option<&str>) -> Result<ContextClassification, CliError> {
    match value {
        Some("public") => Ok(ContextClassification::Public),
        Some("internal") | None => Ok(ContextClassification::Internal),
        Some("sensitive") | Some("secret") => Err(CliError::usage(
            "memory synthesis classification must be 'public' or 'internal'",
        )),
        Some(_) => Err(CliError::usage(
            "memory classification must be public or internal",
        )),
    }
}

fn required_option<'a>(
    parsed: &'a super::ParsedArgs,
    name: &str,
    message: &'static str,
) -> Result<&'a str, CliError> {
    parsed.value(name).ok_or_else(|| CliError::usage(message))
}

fn parse_limit(value: Option<&str>) -> Result<u16, CliError> {
    let limit = value
        .map(|value| {
            value
                .parse::<u16>()
                .map_err(|_| CliError::usage("memory limit must be an integer"))
        })
        .transpose()?
        .unwrap_or(MAX_MEMORY_RECALL_RECORDS);
    if limit == 0 || limit > MAX_MEMORY_RECALL_RECORDS {
        return Err(CliError::usage(format!(
            "memory limit must be between 1 and {MAX_MEMORY_RECALL_RECORDS}"
        )));
    }
    Ok(limit)
}

fn memory_schedule_store(
    config: &pandora_runtime::config::RuntimeConfig,
) -> Result<MemorySynthesisScheduleStore, CliError> {
    MemorySynthesisScheduleStore::open(config.data_dir().join("memory-schedules.sqlite3"))
        .map_err(memory_schedule_error)
}

fn memory_schedule_id(parsed: &super::ParsedArgs) -> Result<RunLoopId, CliError> {
    RunLoopId::new(
        required_option(parsed, "id", "memory schedule command requires '--id <id>'")?.to_owned(),
    )
    .map_err(|_| CliError::usage("memory schedule ID is invalid"))
}

fn memory_schedule_worker(parsed: &super::ParsedArgs) -> Result<JobWorkerId, CliError> {
    JobWorkerId::new(
        required_option(
            parsed,
            "worker",
            "memory schedule command requires '--worker <id>'",
        )?
        .to_owned(),
    )
    .map_err(|_| CliError::usage("worker ID is invalid"))
}

fn schedule_value(schedule: &pandora_runtime::MemorySynthesisSchedule) -> serde_json::Value {
    json!({
        "id": schedule.id(),
        "name": schedule.name(),
        "session_id": schedule.session_id(),
        "provider": schedule.provider(),
        "memory_id": schedule.memory_id(),
        "kind": schedule.kind().as_str(),
        "summary": schedule.summary(),
        "classification": schedule.classification().as_str(),
        "interval_seconds": schedule.interval_seconds(),
        "next_run_at": schedule.next_run_at().as_unix_seconds(),
        "enabled": schedule.enabled(),
        "created_at": schedule.created_at().as_unix_seconds(),
        "last_claimed_at": schedule.last_claimed_at().map(|value| value.as_unix_seconds()),
        "run_count": schedule.run_count(),
        "scope": {
            "principal_id": schedule.principal_id(),
            "tenant_id": schedule.tenant_id(),
            "workspace_id": schedule.workspace_id(),
        }
    })
}

fn schedule_run_value(run: &pandora_runtime::MemorySynthesisScheduleRun) -> serde_json::Value {
    json!({
        "schedule_id": run.schedule_id(),
        "scheduled_for": run.scheduled_for().as_unix_seconds(),
        "status": run.status().as_str(),
        "worker_id": run.worker_id(),
        "claimed_at": run.claimed_at().map(|value| value.as_unix_seconds()),
        "lease_until": run.lease_until().map(|value| value.as_unix_seconds()),
        "finished_at": run.finished_at().map(|value| value.as_unix_seconds()),
        "snapshot_digest": run.snapshot_digest(),
        "result_memory_id": run.result_memory_id(),
        "failure": run.failure(),
    })
}

fn memory_schedule_error(error: MemorySynthesisScheduleError) -> CliError {
    CliError::execution(
        error.to_string(),
        json!({"durability": "memory-schedule-store"}),
    )
}

fn scope_value(scope: &MemoryScope) -> serde_json::Value {
    json!({
        "tenant_id": scope.tenant_id(),
        "workspace_id": scope.workspace_id(),
        "session_id": scope.session_id(),
        "provider": scope.provider(),
    })
}

fn record_value(record: &MemoryRecord) -> serde_json::Value {
    json!({
        "id": record.id(),
        "tier": record.tier().as_str(),
        "kind": record.kind().as_str(),
        "scope": scope_value(record.scope()),
        "summary": record.summary(),
        "classification": record.classification().as_str(),
        "created_at": record.created_at().as_unix_seconds(),
        "expires_at": record.expires_at().map(|value| value.as_unix_seconds()),
        "provenance": record.provenance(),
        "origin": record.origin().as_str(),
        "evidence_ids": record.evidence_ids(),
        "approval": record.approval().map(|approval| json!({
            "approval_id": approval.approval_id(),
            "approver": approval.approver(),
        })),
    })
}

fn audit_value(entry: &pandora_types::MemoryAuditEntry) -> serde_json::Value {
    json!({
        "memory_id": entry.memory_id(),
        "tier": entry.tier().as_str(),
        "action": memory_audit_action(entry.action()),
        "scope": scope_value(entry.scope()),
        "at": entry.at().as_unix_seconds(),
        "approval_id": entry.approval_id(),
    })
}

fn memory_audit_action(action: pandora_types::MemoryAuditAction) -> &'static str {
    match action {
        pandora_types::MemoryAuditAction::Added => "added",
        pandora_types::MemoryAuditAction::Promoted => "promoted",
        pandora_types::MemoryAuditAction::Revoked => "revoked",
    }
}

fn memory_error(error: MemoryError) -> CliError {
    match error {
        MemoryError::NotFound => CliError::execution("memory record was not found", json!({})),
        MemoryError::ApprovalRequired => {
            CliError::approval("memory promotion requires explicit approval", json!({}))
        }
        MemoryError::AlreadyPromoted => {
            CliError::execution("memory record is already promoted", json!({}))
        }
        MemoryError::Revoked => CliError::policy("memory record is revoked", json!({})),
        MemoryError::ScopeViolation => CliError::policy("memory scope does not match", json!({})),
        MemoryError::SynthesisNoEvidence => {
            CliError::execution("memory synthesis has no eligible evidence", json!({}))
        }
        MemoryError::SynthesisStale => {
            CliError::policy("memory synthesis evidence changed before commit", json!({}))
        }
        MemoryError::InvalidCapacity
        | MemoryError::InvalidRecord
        | MemoryError::SecretContent
        | MemoryError::AlreadyExists
        | MemoryError::CapacityExceeded
        | MemoryError::Contract(_) => CliError::execution(
            "memory operation rejected by the memory contract",
            json!({"error": format!("{error:?}")}),
        ),
        MemoryError::StoreUnavailable => {
            CliError::internal("memory store is unavailable", json!({}))
        }
    }
}

fn approval_error(error: ApprovalError) -> CliError {
    match error {
        ApprovalError::Expired | ApprovalError::Terminal => {
            CliError::approval(error.to_string(), json!({}))
        }
        ApprovalError::ScopeMismatch | ApprovalError::DigestMismatch => {
            CliError::policy(error.to_string(), json!({}))
        }
        other => CliError::internal(other.to_string(), json!({})),
    }
}
