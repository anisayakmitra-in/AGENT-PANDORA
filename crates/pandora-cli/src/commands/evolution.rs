use super::{load_config, parse_options, require_config_file, timestamp};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::{
    EvaluationEngine, EvolutionEngine, EvolutionError, EvolutionRecord, HoldoutCase,
    HoldoutSetReport, MAX_HOLDOUT_CASES,
};
use pandora_types::{
    ArtifactId, ArtifactSignature, EvaluationRequest, EvolutionPolicy, EvolutionSource,
    ExecutionId, HoldoutEvaluation, MutationProposal, ParliamentApproval, PrincipalId, ProposalId,
    RequestDigest, Timestamp,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::fs;
use std::io::Read;
use std::path::Path;

const DEFAULT_LIST_LIMIT: usize = 64;
const MAX_LIST_LIMIT: usize = 256;
const MAX_HOLDOUT_INPUT_BYTES: u64 = 4 * 1024 * 1024;

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

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("evolution requires 'list' or 'inspect'"))?;
    match subcommand.as_str() {
        "list" => list(&args[1..]),
        "inspect" => inspect(&args[1..]),
        "submit" => submit(&args[1..]),
        "evaluate" => evaluate(&args[1..]),
        "approve" => approve(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown evolution command '{unknown}', expected 'list', 'inspect', 'submit', 'evaluate', or 'approve'"
        ))),
    }
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
    Ok(success(
        "evolution inspect",
        record_value(&record),
        format!("Inspected evolution proposal {proposal_id}"),
    ))
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
            "could not read holdout input",
            json!({"path": path, "error": error.to_string()}),
        )
    })?;
    if metadata.len() > MAX_HOLDOUT_INPUT_BYTES {
        return Err(CliError::usage(format!(
            "holdout input exceeds {MAX_HOLDOUT_INPUT_BYTES} bytes"
        )));
    }
    let file = fs::File::open(path).map_err(|error| {
        CliError::execution(
            "could not open holdout input",
            json!({"path": path, "error": error.to_string()}),
        )
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_HOLDOUT_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            CliError::execution(
                "could not read holdout input",
                json!({"path": path, "error": error.to_string()}),
            )
        })?;
    if bytes.len() as u64 > MAX_HOLDOUT_INPUT_BYTES {
        return Err(CliError::usage(format!(
            "holdout input exceeds {MAX_HOLDOUT_INPUT_BYTES} bytes"
        )));
    }
    Ok(bytes)
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

#[cfg(test)]
mod tests {
    use super::{parse_approval, parse_holdout_cases, parse_limit, parse_proposal};

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
}
