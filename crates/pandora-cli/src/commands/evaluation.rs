use super::{load_config, parse_options, require_config_file, session_scope, session_store};
use crate::commands::run::evaluation_receipt_json;
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::{EvaluationEngine, GoldenCase, GoldenSetReport, MAX_GOLDEN_CASES};
use pandora_types::{
    EvaluationReceipt, EvaluationRequest, EvaluationStatus, ExecutionId, SessionId,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::fs;
use std::io::Read;
use std::path::Path;

const MAX_EVALUATION_INPUT_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Deserialize)]
struct GoldenSetInput {
    cases: Vec<GoldenCaseInput>,
}

#[derive(Debug, Deserialize)]
struct GoldenCaseInput {
    id: String,
    execution_id: String,
    output: String,
    expected_output: String,
    #[serde(default)]
    policy_violations: Vec<String>,
    terminal_failure: Option<String>,
}

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("evaluation requires 'golden' or 'inspect'"))?;
    match subcommand.as_str() {
        "golden" => golden(&args[1..]),
        "inspect" => inspect(&args[1..]),
        _ => Err(CliError::usage(format!(
            "unknown evaluation command '{subcommand}'"
        ))),
    }
}

fn inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "session", "execution"],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "evaluation inspect does not accept positional arguments",
        ));
    }
    let session_id = parsed
        .value("session")
        .ok_or_else(|| CliError::usage("evaluation inspect requires '--session <id>'"))
        .and_then(|value| {
            SessionId::new(value.to_owned()).map_err(|_| CliError::usage("session ID is invalid"))
        })?;
    let execution_id = parsed
        .value("execution")
        .map(|value| {
            ExecutionId::new(value.to_owned())
                .map_err(|_| CliError::usage("execution ID is invalid"))
        })
        .transpose()?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;
    let (principal, tenant, workspace) = session_scope();
    let snapshot = store
        .resume(&session_id, &principal, &tenant, &workspace)
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let receipts = snapshot
        .evaluations()
        .iter()
        .filter(|receipt| {
            execution_id
                .as_ref()
                .is_none_or(|id| receipt.execution_id() == id)
        })
        .collect::<Vec<_>>();
    if execution_id.is_some() && receipts.is_empty() {
        return Err(CliError::execution(
            "evaluation execution was not found in the session",
            json!({"session_id": session_id, "execution_id": execution_id}),
        ));
    }
    let (passed, failed, review_required) = status_counts(&receipts);
    let count = receipts.len();
    Ok(success(
        "evaluation inspect",
        json!({
            "session_id": session_id,
            "execution_id": execution_id,
            "count": count,
            "result_counts": {
                "passed": passed,
                "failed": failed,
                "human_review_required": review_required,
            },
            "receipts": receipts
                .iter()
                .map(|receipt| evaluation_receipt_json(receipt))
                .collect::<Vec<_>>(),
            "durability": "session-store",
        }),
        format!("Inspected {count} evaluation receipt(s) for {}", session_id),
    ))
}

fn status_counts(receipts: &[&EvaluationReceipt]) -> (usize, usize, usize) {
    receipts.iter().fold((0, 0, 0), |mut counts, receipt| {
        for result in receipt.results() {
            match result.status() {
                EvaluationStatus::Passed => counts.0 += 1,
                EvaluationStatus::Failed => counts.1 += 1,
                EvaluationStatus::HumanReviewRequired => counts.2 += 1,
            }
        }
        counts
    })
}

fn golden(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["input", "fail-on-failure"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "evaluation golden does not accept positional arguments",
        ));
    }
    let input = parsed
        .value("input")
        .ok_or_else(|| CliError::usage("evaluation golden requires '--input <path>'"))?;
    let bytes = read_bounded(Path::new(input))?;
    let cases = parse_cases(&bytes)?;
    let report = EvaluationEngine::new()
        .evaluate_golden_set(cases)
        .map_err(|error| CliError::usage(format!("invalid golden set: {error:?}")))?;
    let data = report_value(&report);
    if parsed.values.contains_key("fail-on-failure") && report.failed() > 0 {
        return Err(CliError::execution("golden-set evaluation failed", data));
    }
    Ok(success(
        "evaluation golden",
        data,
        format!(
            "Golden set: {}/{} passed (digest {})",
            report.passed(),
            report.total(),
            report.digest()
        ),
    ))
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, CliError> {
    let metadata = fs::metadata(path).map_err(|error| {
        CliError::execution(
            "could not read golden-set input",
            json!({"path": path, "error": error.to_string()}),
        )
    })?;
    if metadata.len() > MAX_EVALUATION_INPUT_BYTES {
        return Err(CliError::usage(format!(
            "golden-set input exceeds {MAX_EVALUATION_INPUT_BYTES} bytes"
        )));
    }
    let file = fs::File::open(path).map_err(|error| {
        CliError::execution(
            "could not open golden-set input",
            json!({"path": path, "error": error.to_string()}),
        )
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_EVALUATION_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            CliError::execution(
                "could not read golden-set input",
                json!({"path": path, "error": error.to_string()}),
            )
        })?;
    if bytes.len() as u64 > MAX_EVALUATION_INPUT_BYTES {
        return Err(CliError::usage(format!(
            "golden-set input exceeds {MAX_EVALUATION_INPUT_BYTES} bytes"
        )));
    }
    Ok(bytes)
}

fn parse_cases(bytes: &[u8]) -> Result<Vec<GoldenCase>, CliError> {
    let input = serde_json::from_slice::<GoldenSetInput>(bytes)
        .map_err(|error| CliError::usage(format!("invalid golden-set JSON: {error}")))?;
    if input.cases.len() > MAX_GOLDEN_CASES {
        return Err(CliError::usage(format!(
            "golden set contains more than {MAX_GOLDEN_CASES} cases"
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
            .map_err(|error| CliError::usage(format!("invalid golden case: {error}")))?;
            if let Some(failure) = case.terminal_failure {
                evaluation = evaluation
                    .with_terminal_failure(failure)
                    .map_err(|error| CliError::usage(format!("invalid golden case: {error}")))?;
            }
            GoldenCase::new(case.id, evaluation, case.expected_output)
                .map_err(|error| CliError::usage(format!("invalid golden case: {error:?}")))
        })
        .collect()
}

fn report_value(report: &GoldenSetReport) -> Value {
    json!({
        "total": report.total(),
        "passed": report.passed(),
        "failed": report.failed(),
        "digest": report.digest(),
        "cases": report.cases().iter().map(|case| {
            let result = case.result();
            json!({
                "id": case.id(),
                "kind": result.kind().as_str(),
                "status": result.status().as_str(),
                "score": result.score(),
                "reason": result.reason(),
                "advisory": result.advisory(),
            })
        }).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_cases, status_counts};
    use pandora_types::{
        EvaluationKind, EvaluationReceipt, EvaluationResult, EvaluationStatus, ExecutionId,
        SessionId, Timestamp,
    };

    #[test]
    fn parses_bounded_golden_case_shape() {
        let cases = parse_cases(
            br#"{"cases":[{"id":"case-a","execution_id":"exec-a","output":"done","expected_output":"done"}]}"#,
        )
        .unwrap();
        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0].id(), "case-a");
    }

    #[test]
    fn rejects_missing_required_fields() {
        let error = parse_cases(br#"{"cases":[{"id":"case-a"}]}"#).unwrap_err();
        assert!(error.message.contains("invalid golden-set JSON"));
    }

    #[test]
    fn preserves_terminal_failure_for_trajectory_evaluation() {
        let cases = parse_cases(
            br#"{"cases":[{"id":"case-a","execution_id":"exec-a","output":"done","expected_output":"done","terminal_failure":"stopped"}]}"#,
        )
        .unwrap();
        assert!(cases[0].evaluation().terminal_failure().is_some());
    }

    #[test]
    fn counts_all_persisted_evaluation_result_statuses() {
        let receipt = EvaluationReceipt::new(
            SessionId::new("session-a").unwrap(),
            ExecutionId::new("execution-a").unwrap(),
            Timestamp::from_unix_seconds(1),
            vec![
                EvaluationResult::new(
                    EvaluationKind::Trajectory,
                    EvaluationStatus::Passed,
                    100,
                    "ok",
                    false,
                )
                .unwrap(),
                EvaluationResult::new(
                    EvaluationKind::Outcome,
                    EvaluationStatus::Failed,
                    0,
                    "failed",
                    false,
                )
                .unwrap(),
                EvaluationResult::new(
                    EvaluationKind::Policy,
                    EvaluationStatus::HumanReviewRequired,
                    50,
                    "review",
                    false,
                )
                .unwrap(),
            ],
        )
        .unwrap();

        assert_eq!(status_counts(&[&receipt]), (1, 1, 1));
    }
}
