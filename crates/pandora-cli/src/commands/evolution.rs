use super::{load_config, parse_options, require_config_file};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::{EvolutionEngine, EvolutionError, EvolutionRecord};
use pandora_types::{EvolutionPolicy, ProposalId};
use serde_json::{Value, json};

const DEFAULT_LIST_LIMIT: usize = 64;
const MAX_LIST_LIMIT: usize = 256;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("evolution requires 'list' or 'inspect'"))?;
    match subcommand.as_str() {
        "list" => list(&args[1..]),
        "inspect" => inspect(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown evolution command '{unknown}'"
        ))),
    }
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

fn evolution_error(error: EvolutionError) -> CliError {
    let message = error.to_string();
    match error {
        EvolutionError::NotFound => CliError::execution(message, json!({})),
        _ => CliError::internal(message, json!({})),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_limit;

    #[test]
    fn limits_are_bounded_for_operator_queries() {
        assert_eq!(parse_limit(None).unwrap(), 64);
        assert_eq!(parse_limit(Some("256")).unwrap(), 256);
        assert!(parse_limit(Some("0")).is_err());
        assert!(parse_limit(Some("257")).is_err());
    }
}
