use super::parse_options;
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::{EvaluationEngine, GoldenCase, GoldenSetReport, MAX_GOLDEN_CASES};
use pandora_types::{EvaluationRequest, ExecutionId};
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
        .ok_or_else(|| CliError::usage("evaluation requires 'golden'"))?;
    if subcommand != "golden" {
        return Err(CliError::usage(format!(
            "unknown evaluation command '{subcommand}'"
        )));
    }
    golden(&args[1..])
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
    use super::parse_cases;

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
}
