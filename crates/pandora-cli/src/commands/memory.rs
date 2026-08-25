use super::{load_config, parse_options, require_config_file, session_scope, timestamp};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::sessions::MAX_MEMORY_RECALL_RECORDS;
use pandora_runtime::{
    ApprovalError, ApprovalRequest, ApprovalStatus, ApprovalStore, MemoryEngine, MemoryError,
};
use pandora_types::{
    ExecutionId, GeneId, MemoryApproval, MemoryId, MemoryRecord, MemoryScope, MemoryTier,
    RequestDigest, Timestamp, hash_artifact,
};
use serde_json::json;

const MAX_L0_ENTRIES: usize = 64;
const MEMORY_PROMOTION_POLICY_VERSION: u32 = 1;
const MEMORY_PROMOTION_TTL_SECONDS: u64 = 900;
const MEMORY_PROMOTION_GENE: &str = "memory.promote";

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage("memory requires 'recall', 'audit', 'forget', or 'promote'")
    })?;
    match subcommand.as_str() {
        "recall" => recall(&args[1..]),
        "audit" => audit(&args[1..]),
        "forget" => forget(&args[1..]),
        "promote" => promote(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown memory command '{unknown}'"
        ))),
    }
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
