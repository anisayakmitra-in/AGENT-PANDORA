use super::provider::configured_research_provider_for;
use super::{
    load_config, parse_options, require_config_file, session_scope, session_store, timestamp,
};
use crate::output::{CliError, CommandResult, success};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use pandora_provider::{
    ChatMessage, FallbackPolicy, ModelRequest, TraceMetadata, parse_and_validate,
};
use pandora_runtime::executors::WorkspaceRoot;
use pandora_runtime::{
    ArtifactCatalog, EvaluationEngine, EvolutionEngine, EvolutionError, EvolutionRecord,
    ExecutionController, FleetEngine, FleetLeaseState, HoldoutCase, HoldoutSetReport,
    MAX_HOLDOUT_CASES, MemoryEngine, PackageStore, ReplacementEngine, ReplacementError,
    ResearchArtifactError, ResearchArtifactStore,
};
use pandora_types::{
    ArtifactId, ArtifactSignature, CanaryResult, Capability, ContextClassification,
    EvaluationRequest, EvolutionPolicy, EvolutionSource, ExecutionId, HoldoutEvaluation,
    MemoryKind, MemoryScope, MemoryTier, MutationProposal, ParliamentApproval, PolicyContext,
    PrincipalId, ProposalId, RequestDigest, ResearchArtifactKind, SessionId, Timestamp,
    hash_artifact,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

const DEFAULT_LIST_LIMIT: usize = 64;
const MAX_LIST_LIMIT: usize = 256;
const MAX_HOLDOUT_INPUT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_RESEARCH_GENERATION_BYTES: usize = 64 * 1024;
const MAX_RESEARCH_EXPECTED_OUTCOME_BYTES: usize = 4 * 1024;
const MAX_RESEARCH_MEMORY_SUMMARY_BYTES: usize = 512;
const MAX_RESEARCH_EVALUATIONS: usize = 32;
const MAX_RESEARCH_MEMORIES: usize = 8;

#[derive(Debug, Deserialize)]
struct HoldoutSetInput {
    cases: Vec<HoldoutCaseInput>,
}

#[derive(Debug, Deserialize)]
struct ProposalInput {
    proposal_id: String,
    source: String,
    base_artifact: String,
    candidate_artifact: String,
    evidence_digest: String,
    expected_outcome: String,
    created_at: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ApprovalInput {
    proposal_id: String,
    approver: String,
    policy_version: u32,
    approved_at: Option<u64>,
    artifact_id: String,
    signer: String,
    signature: String,
}

#[derive(Debug, Deserialize)]
struct CanaryInput {
    proposal_id: String,
    passed: bool,
    failure_count: u32,
    note: String,
    evaluated_at: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct HoldoutCaseInput {
    id: String,
    execution_id: String,
    output: String,
    expected_output: String,
    baseline_output: String,
    #[serde(default)]
    policy_violations: Vec<String>,
    terminal_failure: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GeneratedCandidateOutput {
    proposal_id: String,
    expected_outcome: String,
    artifact_base64: String,
}

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| {
            CliError::usage(
                "evolution requires 'generate', 'list', 'inspect', 'submit', 'evaluate', 'approve', 'stage', 'canary', 'activate', or 'rollback'",
            )
        })?;
    match subcommand.as_str() {
        "generate" => generate(&args[1..]),
        "list" => list(&args[1..]),
        "inspect" => inspect(&args[1..]),
        "submit" => submit(&args[1..]),
        "evaluate" => evaluate(&args[1..]),
        "approve" => approve(&args[1..]),
        "stage" => stage(&args[1..]),
        "canary" => canary(&args[1..]),
        "activate" => activate(&args[1..]),
        "rollback" => rollback(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown evolution command '{unknown}', expected 'generate', 'list', 'inspect', 'submit', 'evaluate', 'approve', 'stage', 'canary', 'activate', or 'rollback'"
        ))),
    }
}

fn generate(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "session",
            "provider",
            "model",
            "kind",
            "target-id",
            "base",
            "output",
        ],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "evolution generate does not accept positional arguments",
        ));
    }
    let session_id = required_session_id(parsed.value("session"))?;
    let kind = required_research_kind(parsed.value("kind"))?;
    let target_id = required_option(&parsed, "target-id", "evolution generate")?;
    let base_path = required_option(&parsed, "base", "evolution generate")?;
    let output_path = required_option(&parsed, "output", "evolution generate")?;
    if Path::new(base_path) == Path::new(output_path) {
        return Err(CliError::usage(
            "research candidate output must not overwrite its base artifact",
        ));
    }
    if Path::new(output_path).exists() {
        return Err(CliError::usage(
            "research candidate output already exists; choose a new path",
        ));
    }
    let base = read_research_base(Path::new(base_path))?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let sessions = session_store(&config)?;
    let (principal, tenant, workspace_id) = session_scope();
    let snapshot = sessions
        .resume(&session_id, &principal, &tenant, &workspace_id)
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let model = parsed
        .value("model")
        .or_else(|| {
            parsed
                .value("provider")
                .and_then(|name| config.provider_profile(name).map(|profile| profile.model()))
        })
        .or(config.provider_model())
        .unwrap_or("default");
    let provider = configured_research_provider_for(
        &config,
        model,
        "research candidate generation",
        parsed.value("provider"),
    )?;
    let provider_id = provider.manifest().id().as_str().to_owned();
    let evidence = research_evidence(&config, &snapshot, &principal, &provider_id)?;
    let evidence_digest = hash_artifact(
        &serde_json::to_vec(&evidence)
            .map_err(|error| CliError::internal(error.to_string(), json!({})))?,
    );
    let messages = research_messages(kind, target_id, &base, &evidence, &evidence_digest)?;
    let manifest = provider.manifest().clone();
    let request = ModelRequest::new(
        manifest.id().clone(),
        manifest.default_model().clone(),
        messages,
    )
    .and_then(|request| request.with_max_output_tokens(8_192))
    .and_then(|request| request.with_timeout(Duration::from_secs(60)))
    .map(|request| request.with_trace_metadata(TraceMetadata::new().with_session_id(session_id)))
    .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    let workspace = WorkspaceRoot::new(config.workspace_dir())
        .map_err(|_| CliError::configuration("workspace path is invalid", json!({})))?;
    let controller = ExecutionController::with_policy(
        workspace,
        PolicyContext::new(1, [Capability::ProviderInvoke], []),
    );
    let response = controller
        .invoke_provider(provider.as_ref(), request, snapshot.session(), timestamp())
        .map_err(research_runtime_error)?
        .into_result()
        .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    if !response.tool_calls().is_empty() {
        return Err(CliError::provider(
            "research proposer returned tool calls; proposers cannot execute tools",
            json!({}),
        ));
    }
    let generated = parse_generated_candidate(response.text())?;
    if generated.expected_outcome.len() > MAX_RESEARCH_EXPECTED_OUTCOME_BYTES {
        return Err(CliError::provider(
            "research proposer expected outcome exceeds the size limit",
            json!({}),
        ));
    }
    let candidate = STANDARD
        .decode(generated.artifact_base64.as_bytes())
        .map_err(|_| CliError::provider("research proposer returned invalid base64", json!({})))?;
    if candidate.len() > MAX_RESEARCH_GENERATION_BYTES {
        return Err(CliError::provider(
            "research proposer candidate exceeds the generation size limit",
            json!({}),
        ));
    }
    let proposal = MutationProposal::new(
        generated.proposal_id,
        EvolutionSource::Gepa,
        ArtifactId::new(hash_artifact(&base))
            .map_err(|error| CliError::provider(error.to_string(), json!({})))?,
        ArtifactId::new(hash_artifact(&candidate))
            .map_err(|error| CliError::provider(error.to_string(), json!({})))?,
        RequestDigest::new(evidence_digest)
            .map_err(|error| CliError::provider(error.to_string(), json!({})))?,
        generated.expected_outcome,
        timestamp(),
    )
    .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    pandora_runtime::MutationEngine::new(EvolutionPolicy::research(1))
        .propose_gepa(proposal.clone())
        .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    let research =
        ResearchArtifactStore::open(config.data_dir().join("research-artifacts.sqlite3"))
            .map_err(research_artifact_error)?;
    research
        .stage_generated(
            &proposal,
            kind,
            target_id,
            &base,
            &candidate,
            &provider_id,
            timestamp(),
        )
        .map_err(research_artifact_error)?;
    EvolutionEngine::open(
        config.data_dir().join("evolution.sqlite3"),
        EvolutionPolicy::research(1),
    )
    .map_err(evolution_error)?
    .submit(proposal.clone())
    .map_err(evolution_error)?;
    write_new_candidate(Path::new(output_path), &candidate)?;
    Ok(success(
        "evolution generate",
        json!({
            "proposal_id": proposal.proposal_id(),
            "state": "proposed",
            "kind": kind.as_str(),
            "target_id": target_id,
            "base_artifact": proposal.base_artifact(),
            "candidate_artifact": proposal.candidate_artifact(),
            "evidence_digest": proposal.evidence_digest(),
            "provider": provider_id,
            "output": output_path,
            "runtime_authority_changed": false,
            "next_required": ["holdout evaluation", "regression checks", "Parliament approval", "stage", "canary", "activation"],
            "durability": "sqlite",
        }),
        format!(
            "Generated research-only {} candidate {}",
            kind.as_str(),
            proposal.proposal_id()
        ),
    ))
}

fn approve(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "input"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "evolution approve does not accept positional arguments",
        ));
    }
    let input = parsed
        .value("input")
        .ok_or_else(|| CliError::usage("evolution approve requires '--input <path>'"))?;
    let bytes = read_bounded(Path::new(input))?;
    let (proposal_id, approval, signature) = parse_approval(&bytes)?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let engine = open_engine(&config)?;
    engine
        .approve(&proposal_id, approval.clone(), signature.clone())
        .map_err(evolution_error)?;
    Ok(success(
        "evolution approve",
        json!({
            "proposal_id": proposal_id,
            "state": "approved",
            "approver": approval.approver(),
            "signer": signature.signer(),
            "durability": "sqlite",
        }),
        format!("Approved evolution proposal {proposal_id}"),
    ))
}

fn stage(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "id"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "evolution stage does not accept positional arguments",
        ));
    }
    let proposal_id = required_proposal_id(parsed.value("id"), "stage")?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    ReplacementEngine::new()
        .stage(&open_engine(&config)?, &proposal_id)
        .map_err(replacement_error)?;
    Ok(success(
        "evolution stage",
        json!({"proposal_id": proposal_id, "state": "staged", "durability": "sqlite"}),
        format!("Staged evolution proposal {proposal_id}"),
    ))
}

fn canary(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "input"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "evolution canary does not accept positional arguments",
        ));
    }
    let input = parsed
        .value("input")
        .ok_or_else(|| CliError::usage("evolution canary requires '--input <path>'"))?;
    let canary = parse_canary(&read_bounded(Path::new(input))?)?;
    let proposal_id = canary.proposal_id().clone();
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    ReplacementEngine::new()
        .record_canary(&open_engine(&config)?, canary.clone())
        .map_err(replacement_error)?;
    Ok(success(
        "evolution canary",
        json!({
            "proposal_id": proposal_id,
            "state": if canary.passed() { "canary_passed" } else { "canary_failed" },
            "passed": canary.passed(),
            "failure_count": canary.failure_count(),
            "durability": "sqlite",
        }),
        format!("Recorded canary result for evolution proposal {proposal_id}"),
    ))
}

fn activate(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "id"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "evolution activate does not accept positional arguments",
        ));
    }
    let proposal_id = required_proposal_id(parsed.value("id"), "activate")?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    ensure_evolution_quiescent(&config)?;
    let engine = open_engine(&config)?;
    let catalog = open_artifact_catalog(&config)?;
    let replacement = ReplacementEngine::new();
    replacement
        .reconcile_cataloged(&engine, &catalog, timestamp())
        .map_err(replacement_error)?;
    let record = engine.inspect(&proposal_id).map_err(evolution_error)?;
    let research =
        ResearchArtifactStore::open(config.data_dir().join("research-artifacts.sqlite3"))
            .map_err(research_artifact_error)?;
    let (receipt, activation_scope) = match research
        .inspect(&proposal_id)
        .map_err(research_artifact_error)?
    {
        Some(_) => {
            let candidate = research
                .validate_proposal(record.proposal())
                .map_err(research_artifact_error)?;
            if candidate.kind() == ResearchArtifactKind::WasmGene {
                let packages = PackageStore::open(config.data_dir().join("packages.sqlite3"))
                    .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
                if !packages
                    .contains_artifact(record.proposal().base_artifact())
                    .map_err(|error| CliError::internal(error.to_string(), json!({})))?
                    || !packages
                        .contains_artifact(record.proposal().candidate_artifact())
                        .map_err(|error| CliError::internal(error.to_string(), json!({})))?
                {
                    return Err(CliError::execution(
                        "WASM Gene candidates require package admission before activation",
                        json!({"proposal_id": proposal_id}),
                    ));
                }
            }
            (
                replacement
                    .activate_cataloged(&engine, &catalog, &proposal_id, timestamp())
                    .map_err(replacement_error)?,
                json!({
                    "kind": candidate.kind().as_str(),
                    "target_id": candidate.target_id(),
                    "provider": candidate.provider_id(),
                    "research_only": candidate.kind() != ResearchArtifactKind::WasmGene,
                }),
            )
        }
        None => {
            let packages = PackageStore::open(config.data_dir().join("packages.sqlite3"))
                .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
            (
                replacement
                    .activate_admitted(&engine, &packages, &catalog, &proposal_id, timestamp())
                    .map_err(replacement_error)?,
                json!({"kind": "package", "research_only": false}),
            )
        }
    };
    Ok(success(
        "evolution activate",
        json!({
            "proposal_id": proposal_id,
            "state": "active",
            "base_artifact": receipt.base_artifact(),
            "candidate_artifact": receipt.candidate_artifact(),
            "activated_at": receipt.activated_at(),
            "activation_scope": activation_scope,
            "runtime_authority_changed": false,
            "durability": "sqlite",
        }),
        format!("Activated admitted artifact for evolution proposal {proposal_id}"),
    ))
}

fn rollback(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "id", "reason"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "evolution rollback does not accept positional arguments",
        ));
    }
    let proposal_id = required_proposal_id(parsed.value("id"), "rollback")?;
    let reason = parsed
        .value("reason")
        .ok_or_else(|| CliError::usage("evolution rollback requires '--reason <text>'"))?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    ensure_evolution_quiescent(&config)?;
    let engine = open_engine(&config)?;
    let catalog = open_artifact_catalog(&config)?;
    let replacement = ReplacementEngine::new();
    replacement
        .reconcile_cataloged(&engine, &catalog, timestamp())
        .map_err(replacement_error)?;
    let receipt = replacement
        .rollback_admitted(&engine, &catalog, &proposal_id, timestamp(), reason)
        .map_err(replacement_error)?;
    Ok(success(
        "evolution rollback",
        json!({
            "proposal_id": proposal_id,
            "state": "rolled_back",
            "restored_artifact": receipt.restored_artifact(),
            "rolled_back_at": receipt.rolled_back_at(),
            "reason": receipt.reason(),
            "durability": "sqlite",
        }),
        format!("Rolled back evolution proposal {proposal_id}"),
    ))
}

fn submit(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "input"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "evolution submit does not accept positional arguments",
        ));
    }
    let input = parsed
        .value("input")
        .ok_or_else(|| CliError::usage("evolution submit requires '--input <path>'"))?;
    let bytes = read_bounded(Path::new(input))?;
    let proposal = parse_proposal(&bytes)?;
    let proposal_id = proposal.proposal_id().clone();
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let engine = open_engine(&config)?;
    engine.submit(proposal).map_err(evolution_error)?;
    Ok(success(
        "evolution submit",
        json!({
            "proposal_id": proposal_id,
            "state": "proposed",
            "durability": "sqlite",
        }),
        format!("Submitted evolution proposal {proposal_id}"),
    ))
}

fn evaluate(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "id", "input", "fail-on-failure"],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "evolution evaluate does not accept positional arguments",
        ));
    }
    let proposal_id = parsed
        .value("id")
        .ok_or_else(|| CliError::usage("evolution evaluate requires '--id <proposal-id>'"))
        .and_then(|value| {
            ProposalId::new(value.to_owned()).map_err(|_| CliError::usage("proposal ID is invalid"))
        })?;
    let input = parsed
        .value("input")
        .ok_or_else(|| CliError::usage("evolution evaluate requires '--input <path>'"))?;
    let bytes = read_bounded(Path::new(input))?;
    let cases = parse_holdout_cases(&bytes)?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let engine = open_engine(&config)?;
    engine.inspect(&proposal_id).map_err(evolution_error)?;
    let report = EvaluationEngine::new()
        .evaluate_holdout_set(cases)
        .map_err(|error| CliError::usage(format!("invalid holdout set: {error:?}")))?;
    let evaluation = HoldoutEvaluation::new(
        proposal_id.clone(),
        report.trajectory_score(),
        report.outcome_score(),
        report.holdout_passed(),
        report.policy_passed(),
        report.regression_passed(),
        timestamp(),
    )
    .with_holdout_digest(report.digest().to_owned())
    .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    engine
        .record_evaluation(evaluation)
        .map_err(evolution_error)?;
    let data = holdout_report_value(&proposal_id, &report);
    if parsed.values.contains_key("fail-on-failure") && !report.holdout_passed() {
        return Err(CliError::execution("holdout evaluation failed", data));
    }
    Ok(success(
        "evolution evaluate",
        data,
        format!(
            "Evaluated evolution proposal {proposal_id}: {}/{} holdout cases passed",
            report.passed(),
            report.total()
        ),
    ))
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "limit"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "evolution list does not accept positional arguments",
        ));
    }
    let limit = parse_limit(parsed.value("limit"))?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let engine = open_engine(&config)?;
    let mut records = engine.list().map_err(evolution_error)?;
    records.truncate(limit);
    let count = records.len();
    Ok(success(
        "evolution list",
        json!({
            "records": records.iter().map(summary_value).collect::<Vec<_>>(),
            "count": count,
            "limit": limit,
            "durability": "sqlite",
        }),
        format!("Listed {count} evolution proposal(s)"),
    ))
}

fn inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "id"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "evolution inspect does not accept positional arguments",
        ));
    }
    let proposal_id = parsed
        .value("id")
        .ok_or_else(|| CliError::usage("evolution inspect requires '--id <proposal-id>'"))
        .and_then(|value| {
            ProposalId::new(value.to_owned()).map_err(|_| CliError::usage("proposal ID is invalid"))
        })?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let engine = open_engine(&config)?;
    let record = engine.inspect(&proposal_id).map_err(evolution_error)?;
    let research =
        ResearchArtifactStore::open(config.data_dir().join("research-artifacts.sqlite3"))
            .map_err(research_artifact_error)?;
    let mut data = record_value(&record);
    if let Some(candidate) = research
        .inspect(&proposal_id)
        .map_err(research_artifact_error)?
        && let Some(object) = data.as_object_mut()
    {
        object.insert(
            "research_candidate".to_owned(),
            json!({
                "kind": candidate.kind().as_str(),
                "target_id": candidate.target_id(),
                "provider": candidate.provider_id(),
                "generated_at": candidate.generated_at().as_unix_seconds(),
            }),
        );
    }
    Ok(success(
        "evolution inspect",
        data,
        format!("Inspected evolution proposal {proposal_id}"),
    ))
}

fn ensure_evolution_quiescent(
    config: &pandora_runtime::config::RuntimeConfig,
) -> Result<(), CliError> {
    let fleet = FleetEngine::open(config.data_dir().join("fleet.sqlite3"))
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    fleet
        .expire_leases(timestamp().as_unix_seconds())
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let active = fleet
        .list_leases()
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?
        .into_iter()
        .filter(|lease| lease.state() == FleetLeaseState::Active)
        .map(|lease| {
            json!({
                "lease_id": lease.id(),
                "execution_id": lease.execution_id(),
                "expires_at": lease.expires_at(),
            })
        })
        .collect::<Vec<_>>();
    if active.is_empty() {
        Ok(())
    } else {
        Err(CliError::execution(
            "evolution mutation is blocked while executions are active",
            json!({"active_leases": active}),
        ))
    }
}

fn open_engine(
    config: &pandora_runtime::config::RuntimeConfig,
) -> Result<EvolutionEngine, CliError> {
    EvolutionEngine::open(
        config.data_dir().join("evolution.sqlite3"),
        EvolutionPolicy::production(1),
    )
    .map_err(evolution_error)
}

fn open_artifact_catalog(
    config: &pandora_runtime::config::RuntimeConfig,
) -> Result<ArtifactCatalog, CliError> {
    ArtifactCatalog::open(config.data_dir().join("artifact-catalog.sqlite3"))
        .map_err(|error| CliError::internal(error.to_string(), json!({})))
}

fn required_proposal_id(value: Option<&str>, command: &str) -> Result<ProposalId, CliError> {
    let value = value.ok_or_else(|| {
        CliError::usage(format!("evolution {command} requires '--id <proposal-id>'"))
    })?;
    ProposalId::new(value.to_owned()).map_err(|_| CliError::usage("proposal ID is invalid"))
}

fn required_session_id(value: Option<&str>) -> Result<SessionId, CliError> {
    let value = value
        .ok_or_else(|| CliError::usage("evolution generate requires '--session <session-id>'"))?;
    SessionId::new(value.to_owned()).map_err(|_| CliError::usage("session ID is invalid"))
}

fn required_research_kind(value: Option<&str>) -> Result<ResearchArtifactKind, CliError> {
    let value = value.ok_or_else(|| {
        CliError::usage("evolution generate requires '--kind prompt|skill|workflow|wasm_gene'")
    })?;
    ResearchArtifactKind::parse(value).ok_or_else(|| {
        CliError::usage("research kind must be prompt, skill, workflow, or wasm_gene")
    })
}

fn required_option<'a>(
    parsed: &'a super::ParsedArgs,
    name: &str,
    command: &str,
) -> Result<&'a str, CliError> {
    parsed
        .value(name)
        .ok_or_else(|| CliError::usage(format!("{command} requires '--{name} <value>'")))
}

fn parse_limit(value: Option<&str>) -> Result<usize, CliError> {
    let limit = value
        .map(str::parse)
        .transpose()
        .map_err(|_| CliError::usage("evolution limit must be an integer"))?
        .unwrap_or(DEFAULT_LIST_LIMIT);
    if !(1..=MAX_LIST_LIMIT).contains(&limit) {
        return Err(CliError::usage(format!(
            "evolution limit must be between 1 and {MAX_LIST_LIMIT}"
        )));
    }
    Ok(limit)
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, CliError> {
    let metadata = fs::metadata(path).map_err(|error| {
        CliError::execution(
            "could not read evolution input",
            json!({"path": path, "error": error.to_string()}),
        )
    })?;
    if metadata.len() > MAX_HOLDOUT_INPUT_BYTES {
        return Err(CliError::usage(format!(
            "evolution input exceeds {MAX_HOLDOUT_INPUT_BYTES} bytes"
        )));
    }
    let file = fs::File::open(path).map_err(|error| {
        CliError::execution(
            "could not open evolution input",
            json!({"path": path, "error": error.to_string()}),
        )
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_HOLDOUT_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            CliError::execution(
                "could not read evolution input",
                json!({"path": path, "error": error.to_string()}),
            )
        })?;
    if bytes.len() as u64 > MAX_HOLDOUT_INPUT_BYTES {
        return Err(CliError::usage(format!(
            "evolution input exceeds {MAX_HOLDOUT_INPUT_BYTES} bytes"
        )));
    }
    Ok(bytes)
}

fn read_research_base(path: &Path) -> Result<Vec<u8>, CliError> {
    let metadata = fs::metadata(path).map_err(|error| {
        CliError::execution(
            "could not read research base artifact",
            json!({"path": path, "error": error.to_string()}),
        )
    })?;
    if !metadata.is_file() || metadata.len() > MAX_RESEARCH_GENERATION_BYTES as u64 {
        return Err(CliError::usage(format!(
            "research base artifact must be a file no larger than {MAX_RESEARCH_GENERATION_BYTES} bytes"
        )));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|error| {
            CliError::execution(
                "could not open research base artifact",
                json!({"path": path, "error": error.to_string()}),
            )
        })?
        .take(MAX_RESEARCH_GENERATION_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            CliError::execution(
                "could not read research base artifact",
                json!({"path": path, "error": error.to_string()}),
            )
        })?;
    if bytes.is_empty() || bytes.len() > MAX_RESEARCH_GENERATION_BYTES {
        return Err(CliError::usage(format!(
            "research base artifact must contain 1 to {MAX_RESEARCH_GENERATION_BYTES} bytes"
        )));
    }
    Ok(bytes)
}

fn write_new_candidate(path: &Path, candidate: &[u8]) -> Result<(), CliError> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            CliError::execution(
                "could not create research candidate output",
                json!({"path": path, "error": error.to_string()}),
            )
        })?;
    file.write_all(candidate).map_err(|error| {
        CliError::execution(
            "could not write research candidate output",
            json!({"path": path, "error": error.to_string()}),
        )
    })?;
    file.sync_all().map_err(|error| {
        CliError::execution(
            "could not persist research candidate output",
            json!({"path": path, "error": error.to_string()}),
        )
    })
}

fn research_evidence(
    config: &pandora_runtime::config::RuntimeConfig,
    snapshot: &pandora_runtime::sessions::SessionSnapshot,
    principal: &PrincipalId,
    provider_id: &str,
) -> Result<Value, CliError> {
    let mut evaluations = snapshot
        .evaluations()
        .iter()
        .map(|receipt| {
            json!({
                "execution_id": receipt.execution_id(),
                "evaluated_at": receipt.evaluated_at().as_unix_seconds(),
                "results": receipt.results().iter().map(|result| json!({
                    "kind": result.kind().as_str(),
                    "status": result.status().as_str(),
                    "score": result.score(),
                    "advisory": result.advisory(),
                })).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    evaluations.sort_by(|left, right| {
        left["execution_id"]
            .as_str()
            .cmp(&right["execution_id"].as_str())
    });
    evaluations.truncate(MAX_RESEARCH_EVALUATIONS);
    let scope = MemoryScope::new(
        snapshot.session().tenant_id().clone(),
        snapshot.session().workspace_id().clone(),
        snapshot.session().id().clone(),
        provider_id,
    )
    .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let memory = MemoryEngine::open(
        config.data_dir().join("sessions.sqlite3"),
        2,
        principal.clone(),
    )
    .map_err(research_memory_error)?;
    let feedback_summaries = memory
        .try_recall(&scope, MemoryTier::L1, timestamp())
        .map_err(research_memory_error)?
        .into_iter()
        .filter(|memory| {
            memory.classification() == ContextClassification::Internal
                && memory.kind() == MemoryKind::Lesson
                && memory.provenance().starts_with("evaluation:")
        })
        .take(MAX_RESEARCH_MEMORIES)
        .map(|memory| {
            json!({
                "summary": bounded_text(memory.summary(), MAX_RESEARCH_MEMORY_SUMMARY_BYTES),
                "source": "evaluation_feedback",
            })
        })
        .collect::<Vec<_>>();
    let approved_memories = memory
        .try_recall(&scope, MemoryTier::L2, timestamp())
        .map_err(research_memory_error)?
        .into_iter()
        .filter(|memory| {
            memory.classification() == ContextClassification::Internal
                && memory.approval().is_some()
        })
        .take(MAX_RESEARCH_MEMORIES)
        .map(|memory| {
            json!({
                "id": memory.id(),
                "kind": memory.kind().as_str(),
                "summary": bounded_text(memory.summary(), MAX_RESEARCH_MEMORY_SUMMARY_BYTES),
                "approved": true,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "schema": "pandora.research-evidence.v1",
        "session_id": snapshot.session().id(),
        "evaluation_summaries": evaluations,
        "failed_run_feedback_count": snapshot.l1_evidence_count(),
        "feedback_summaries": feedback_summaries,
        "approved_internal_memories": approved_memories,
    }))
}

fn research_messages(
    kind: ResearchArtifactKind,
    target_id: &str,
    base: &[u8],
    evidence: &Value,
    evidence_digest: &str,
) -> Result<Vec<ChatMessage>, CliError> {
    let system = ChatMessage::system(
        "You are Pandora's untrusted research proposer. Produce exactly one bounded candidate artifact. You have no authority to call tools, approve, stage, activate, roll back, alter policy, or issue permits. Return only JSON with proposal_id, expected_outcome, and artifact_base64. proposal_id must be a short stable identifier. artifact_base64 must decode to a complete candidate of the requested kind, must differ from the base artifact, and must be no larger than 65536 bytes. For prompt and skill use UTF-8 text; for workflow use a JSON object; for wasm_gene use a valid WebAssembly binary. Do not include Markdown or extra fields."
            .to_owned(),
    )
    .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    let input = json!({
        "target_kind": kind.as_str(),
        "target_id": target_id,
        "base_artifact_base64": STANDARD.encode(base),
        "evidence_digest": evidence_digest,
        "bounded_research_evidence": evidence,
    });
    let user = ChatMessage::user(
        serde_json::to_string(&input)
            .map_err(|error| CliError::internal(error.to_string(), json!({})))?,
    )
    .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    Ok(vec![system, user])
}

fn parse_generated_candidate(text: &str) -> Result<GeneratedCandidateOutput, CliError> {
    let validated = parse_and_validate(
        text,
        &json!({
            "type": "object",
            "required": ["proposal_id", "expected_outcome", "artifact_base64"],
            "properties": {
                "proposal_id": {"type": "string"},
                "expected_outcome": {"type": "string"},
                "artifact_base64": {"type": "string"},
            },
            "additionalProperties": false,
        }),
        None,
        FallbackPolicy::Reject,
    )
    .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    serde_json::from_value(validated.value().clone())
        .map_err(|error| CliError::provider(error.to_string(), json!({})))
}

fn bounded_text(value: &str, maximum_bytes: usize) -> String {
    if value.len() <= maximum_bytes {
        return value.to_owned();
    }
    let mut end = 0;
    for (index, _) in value.char_indices() {
        if index > maximum_bytes {
            break;
        }
        end = index;
    }
    value[..end].to_owned()
}

fn parse_canary(bytes: &[u8]) -> Result<CanaryResult, CliError> {
    let input = serde_json::from_slice::<CanaryInput>(bytes)
        .map_err(|error| CliError::usage(format!("invalid canary JSON: {error}")))?;
    let proposal_id = ProposalId::new(input.proposal_id)
        .map_err(|error| CliError::usage(format!("invalid proposal ID: {error}")))?;
    let evaluated_at = input
        .evaluated_at
        .map(Timestamp::from_unix_seconds)
        .unwrap_or_else(timestamp);
    CanaryResult::new(
        proposal_id,
        input.passed,
        input.failure_count,
        input.note,
        evaluated_at,
    )
    .map_err(|error| CliError::usage(format!("invalid canary result: {error}")))
}

fn parse_holdout_cases(bytes: &[u8]) -> Result<Vec<HoldoutCase>, CliError> {
    let input = serde_json::from_slice::<HoldoutSetInput>(bytes)
        .map_err(|error| CliError::usage(format!("invalid holdout JSON: {error}")))?;
    if input.cases.len() > MAX_HOLDOUT_CASES {
        return Err(CliError::usage(format!(
            "holdout set contains more than {MAX_HOLDOUT_CASES} cases"
        )));
    }
    input
        .cases
        .into_iter()
        .map(|case| {
            let execution_id = ExecutionId::new(case.execution_id)
                .map_err(|error| CliError::usage(format!("invalid execution_id: {error}")))?;
            let mut evaluation = EvaluationRequest::new(
                execution_id,
                Vec::new(),
                case.output,
                case.policy_violations,
            )
            .map_err(|error| CliError::usage(format!("invalid holdout case: {error}")))?;
            if let Some(failure) = case.terminal_failure {
                evaluation = evaluation
                    .with_terminal_failure(failure)
                    .map_err(|error| CliError::usage(format!("invalid holdout case: {error}")))?;
            }
            HoldoutCase::new(
                case.id,
                evaluation,
                case.expected_output,
                case.baseline_output,
            )
            .map_err(|error| CliError::usage(format!("invalid holdout case: {error:?}")))
        })
        .collect()
}

fn parse_proposal(bytes: &[u8]) -> Result<MutationProposal, CliError> {
    let input = serde_json::from_slice::<ProposalInput>(bytes)
        .map_err(|error| CliError::usage(format!("invalid proposal JSON: {error}")))?;
    let source = match input.source.as_str() {
        "reflexion" => EvolutionSource::Reflexion,
        "gepa" => EvolutionSource::Gepa,
        "population" => EvolutionSource::Population,
        _ => {
            return Err(CliError::usage(
                "proposal source must be reflexion, gepa, or population",
            ));
        }
    };
    let created_at = input
        .created_at
        .map(Timestamp::from_unix_seconds)
        .unwrap_or_else(timestamp);
    MutationProposal::new(
        input.proposal_id,
        source,
        ArtifactId::new(input.base_artifact)
            .map_err(|error| CliError::usage(format!("invalid base artifact: {error}")))?,
        ArtifactId::new(input.candidate_artifact)
            .map_err(|error| CliError::usage(format!("invalid candidate artifact: {error}")))?,
        RequestDigest::new(input.evidence_digest)
            .map_err(|error| CliError::usage(format!("invalid evidence digest: {error}")))?,
        input.expected_outcome,
        created_at,
    )
    .map_err(|error| CliError::usage(format!("invalid proposal: {error}")))
}

fn parse_approval(
    bytes: &[u8],
) -> Result<(ProposalId, ParliamentApproval, ArtifactSignature), CliError> {
    let input = serde_json::from_slice::<ApprovalInput>(bytes)
        .map_err(|error| CliError::usage(format!("invalid approval JSON: {error}")))?;
    let proposal_id = ProposalId::new(input.proposal_id)
        .map_err(|error| CliError::usage(format!("invalid proposal ID: {error}")))?;
    let approver = PrincipalId::new(input.approver)
        .map_err(|error| CliError::usage(format!("invalid approver: {error}")))?;
    let approved_at = input
        .approved_at
        .map(Timestamp::from_unix_seconds)
        .unwrap_or_else(timestamp);
    let approval = ParliamentApproval::new(
        proposal_id.clone(),
        approver,
        input.policy_version,
        approved_at,
    );
    let artifact_id = ArtifactId::new(input.artifact_id)
        .map_err(|error| CliError::usage(format!("invalid artifact ID: {error}")))?;
    let signer = PrincipalId::new(input.signer)
        .map_err(|error| CliError::usage(format!("invalid signer: {error}")))?;
    let signature = ArtifactSignature::new(artifact_id, signer, input.signature)
        .map_err(|error| CliError::usage(format!("invalid artifact signature: {error}")))?;
    Ok((proposal_id, approval, signature))
}

fn summary_value(record: &EvolutionRecord) -> Value {
    let proposal = record.proposal();
    json!({
        "proposal_id": proposal.proposal_id(),
        "source": proposal.source().as_str(),
        "base_artifact": proposal.base_artifact(),
        "candidate_artifact": proposal.candidate_artifact(),
        "evidence_digest": proposal.evidence_digest(),
        "state": record.state().as_str(),
        "created_at": proposal.created_at().as_unix_seconds(),
    })
}

fn record_value(record: &EvolutionRecord) -> Value {
    let proposal = record.proposal();
    json!({
        "proposal": {
            "proposal_id": proposal.proposal_id(),
            "source": proposal.source().as_str(),
            "base_artifact": proposal.base_artifact(),
            "candidate_artifact": proposal.candidate_artifact(),
            "evidence_digest": proposal.evidence_digest(),
            "expected_outcome": proposal.expected_outcome(),
            "created_at": proposal.created_at().as_unix_seconds(),
        },
        "state": record.state().as_str(),
        "evaluation": record.evaluation().map(|evaluation| json!({
            "trajectory_score": evaluation.trajectory_score(),
            "outcome_score": evaluation.outcome_score(),
            "holdout_passed": evaluation.holdout_passed(),
            "policy_passed": evaluation.policy_passed(),
            "regression_passed": evaluation.regression_passed(),
            "holdout_digest": evaluation.holdout_digest(),
            "evaluated_at": evaluation.evaluated_at().as_unix_seconds(),
        })),
        "approval": record.approval().map(|approval| json!({
            "approver": approval.approver(),
            "policy_version": approval.policy_version(),
            "approved_at": approval.approved_at().as_unix_seconds(),
        })),
        "signature": record.signature().map(|signature| json!({
            "artifact_id": signature.artifact_id(),
            "signer": signature.signer(),
            "present": !signature.signature().is_empty(),
        })),
        "canary": record.canary().map(|canary| json!({
            "passed": canary.passed(),
            "failure_count": canary.failure_count(),
            "note": canary.note(),
            "evaluated_at": canary.evaluated_at().as_unix_seconds(),
        })),
        "durability": "sqlite",
    })
}

fn holdout_report_value(proposal_id: &ProposalId, report: &HoldoutSetReport) -> Value {
    json!({
        "proposal_id": proposal_id,
        "total": report.total(),
        "passed": report.passed(),
        "failed": report.failed(),
        "trajectory_score": report.trajectory_score(),
        "outcome_score": report.outcome_score(),
        "holdout_passed": report.holdout_passed(),
        "policy_passed": report.policy_passed(),
        "regression_passed": report.regression_passed(),
        "digest": report.digest(),
        "cases": report.cases().iter().map(|case| json!({
            "id": case.id(),
            "passed": case.passed(),
            "trajectory": evaluation_result_value(case.trajectory()),
            "outcome": evaluation_result_value(case.outcome()),
            "policy": evaluation_result_value(case.policy()),
            "regression": evaluation_result_value(case.regression()),
        })).collect::<Vec<_>>(),
        "durability": "sqlite",
    })
}

fn evaluation_result_value(result: &pandora_types::EvaluationResult) -> Value {
    json!({
        "kind": result.kind().as_str(),
        "status": result.status().as_str(),
        "score": result.score(),
        "reason": result.reason(),
        "advisory": result.advisory(),
    })
}

fn evolution_error(error: EvolutionError) -> CliError {
    let message = error.to_string();
    match error {
        EvolutionError::NotFound => CliError::execution(message, json!({})),
        _ => CliError::internal(message, json!({})),
    }
}

fn replacement_error(error: ReplacementError) -> CliError {
    let message = error.to_string();
    match error {
        ReplacementError::Evolution(EvolutionError::NotFound)
        | ReplacementError::ExecutionActive
        | ReplacementError::ExecutionNotFound
        | ReplacementError::BaseArtifactNotAdmitted
        | ReplacementError::CandidateArtifactNotAdmitted => CliError::execution(message, json!({})),
        _ => CliError::internal(message, json!({})),
    }
}

fn research_artifact_error(error: ResearchArtifactError) -> CliError {
    let message = error.to_string();
    match error {
        ResearchArtifactError::ProposalNotFound
        | ResearchArtifactError::ProposalMismatch
        | ResearchArtifactError::InvalidArtifact
        | ResearchArtifactError::InvalidProvider
        | ResearchArtifactError::InvalidTarget
        | ResearchArtifactError::ArtifactTooLarge => CliError::execution(message, json!({})),
        _ => CliError::internal(message, json!({})),
    }
}

fn research_runtime_error(error: pandora_runtime::RuntimeError) -> CliError {
    match error {
        pandora_runtime::RuntimeError::Provider(error) => {
            CliError::provider(error.to_string(), json!({}))
        }
        pandora_runtime::RuntimeError::Denied(reason) => CliError::policy(reason, json!({})),
        pandora_runtime::RuntimeError::ApprovalRequired(reason) => {
            CliError::approval(reason, json!({}))
        }
        _ => CliError::execution("research provider invocation was not authorized", json!({})),
    }
}

fn research_memory_error(error: pandora_runtime::MemoryError) -> CliError {
    match error {
        pandora_runtime::MemoryError::StoreUnavailable => {
            CliError::internal("research memory store is unavailable", json!({}))
        }
        _ => CliError::execution(
            "approved research memory could not be retrieved",
            json!({"error": format!("{error:?}")}),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        parse_approval, parse_canary, parse_generated_candidate, parse_holdout_cases, parse_limit,
        parse_proposal, required_proposal_id, required_research_kind,
    };

    #[test]
    fn limits_are_bounded_for_operator_queries() {
        assert_eq!(parse_limit(None).unwrap(), 64);
        assert_eq!(parse_limit(Some("256")).unwrap(), 256);
        assert!(parse_limit(Some("0")).is_err());
        assert!(parse_limit(Some("257")).is_err());
    }

    #[test]
    fn parses_bounded_holdout_case_shape() {
        let cases = parse_holdout_cases(
            br#"{"cases":[{"id":"case-a","execution_id":"exec-a","output":"candidate","expected_output":"candidate","baseline_output":"baseline"}]}"#,
        )
        .unwrap();

        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0].id(), "case-a");
    }

    #[test]
    fn rejects_holdout_case_without_a_regression_baseline() {
        let error = parse_holdout_cases(
            br#"{"cases":[{"id":"case-a","execution_id":"exec-a","output":"candidate","expected_output":"candidate"}]}"#,
        )
        .unwrap_err();

        assert!(error.message.contains("invalid holdout JSON"));
    }

    #[test]
    fn parses_bounded_proposal_sources_and_uses_current_time_by_default() {
        let proposal = parse_proposal(
            br#"{"proposal_id":"proposal-1","source":"gepa","base_artifact":"base-1","candidate_artifact":"candidate-1","evidence_digest":"evidence-1","expected_outcome":"improve verification reliability"}"#,
        )
        .unwrap();

        assert_eq!(proposal.proposal_id().as_str(), "proposal-1");
        assert_eq!(proposal.source().as_str(), "gepa");
        assert!(proposal.created_at().as_unix_seconds() > 0);
    }

    #[test]
    fn parses_bounded_approval_and_signature_input() {
        let (proposal_id, approval, signature) = parse_approval(
            br#"{"proposal_id":"proposal-1","approver":"parliament-1","policy_version":1,"artifact_id":"candidate-1","signer":"signer-1","signature":"signed-candidate"}"#,
        )
        .unwrap();

        assert_eq!(proposal_id.as_str(), "proposal-1");
        assert_eq!(approval.approver().as_str(), "parliament-1");
        assert_eq!(approval.policy_version(), 1);
        assert_eq!(signature.artifact_id().as_str(), "candidate-1");
        assert_eq!(signature.signer().as_str(), "signer-1");
    }

    #[test]
    fn parses_canary_evidence_without_granting_authority() {
        let canary = parse_canary(
            br#"{"proposal_id":"proposal-1","passed":true,"failure_count":0,"note":"shadow traffic passed"}"#,
        )
        .unwrap();

        assert_eq!(canary.proposal_id().as_str(), "proposal-1");
        assert!(canary.passed());
        assert_eq!(canary.failure_count(), 0);
        assert_eq!(canary.note(), "shadow traffic passed");
    }

    #[test]
    fn lifecycle_commands_require_a_valid_proposal_id() {
        assert!(required_proposal_id(None, "activate").is_err());
        assert!(required_proposal_id(Some("proposal-1"), "activate").is_ok());
        assert!(required_proposal_id(Some(""), "activate").is_err());
    }

    #[test]
    fn research_generation_requires_the_exact_json_contract() {
        let generated = parse_generated_candidate(
            r#"{"proposal_id":"research-1","expected_outcome":"improve outcome reliability","artifact_base64":"Y2FuZGlkYXRl"}"#,
        )
        .unwrap();
        assert_eq!(generated.proposal_id, "research-1");
        assert!(parse_generated_candidate(
            r#"{"proposal_id":"research-1","expected_outcome":"improve","artifact_base64":"Y2FuZGlkYXRl","extra":true}"#,
        )
        .is_err());
    }

    #[test]
    fn research_generation_limits_kinds_to_non_executable_candidate_classes() {
        assert!(required_research_kind(Some("prompt")).is_ok());
        assert!(required_research_kind(Some("skill")).is_ok());
        assert!(required_research_kind(Some("workflow")).is_ok());
        assert!(required_research_kind(Some("wasm_gene")).is_ok());
        assert!(required_research_kind(Some("shell")).is_err());
    }
}
