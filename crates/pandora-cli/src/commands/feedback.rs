use super::parse_options;
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::{CodingFeedbackInput, CodingFeedbackLoop, RunLoop};
use pandora_types::{
    AdaptationCandidate, AdaptationPolicy, AdaptationRequest, AdaptationTarget, EvaluationRequest,
    EvaluationResult, ExecutionId, LoopDecision, LoopTermination, PlanId, RequestDigest,
    RunLoopConfig, RunLoopId, RunLoopSnapshot, SessionId, Usage,
};
use serde_json::{Value, json};

pub(super) const MAX_FEEDBACK_TEXT_BYTES: usize = 65_536;
const MAX_FEEDBACK_FAILURE_BYTES: usize = 4_096;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("feedback requires 'coding'"))?;
    match subcommand.as_str() {
        "coding" => coding(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown feedback command '{unknown}'"
        ))),
    }
}

fn coding(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "execution",
            "session",
            "request-digest",
            "expected-output",
            "output",
            "terminal-failure",
            "retryable",
        ],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "feedback coding does not accept positional arguments",
        ));
    }
    let execution_id = required_id(&parsed, "execution", "execution ID", ExecutionId::new)?;
    let session_id = required_id(&parsed, "session", "session ID", SessionId::new)?;
    let request_digest = required_id(
        &parsed,
        "request-digest",
        "request digest",
        RequestDigest::new,
    )?;
    let expected_output = required_text(&parsed, "expected-output", MAX_FEEDBACK_TEXT_BYTES)?;
    let output = required_text(&parsed, "output", MAX_FEEDBACK_TEXT_BYTES)?;
    let terminal_failure = parsed
        .value("terminal-failure")
        .map(|value| bounded_text(value, MAX_FEEDBACK_FAILURE_BYTES, "terminal failure"))
        .transpose()?;
    let retryable = parsed.value("retryable").is_some();

    let mut evaluation =
        EvaluationRequest::new(execution_id.clone(), Vec::new(), output, Vec::new())
            .map_err(|error| CliError::usage(format!("invalid coding feedback output: {error}")))?;
    if let Some(failure) = terminal_failure {
        evaluation = evaluation
            .with_terminal_failure(failure)
            .map_err(|error| CliError::usage(format!("invalid terminal failure: {error}")))?;
    }

    let candidates = if retryable {
        vec![
            AdaptationCandidate::new(
                "coding.safe_retry",
                AdaptationTarget::recovery("coding.safe_retry")
                    .map_err(|error| CliError::usage(error.to_string()))?,
                100,
                true,
                false,
                0,
                0,
            )
            .map_err(|error| CliError::usage(error.to_string()))?,
        ]
    } else {
        Vec::new()
    };
    let adaptation = AdaptationRequest::new(
        execution_id.clone(),
        session_id.clone(),
        request_digest.clone(),
        None,
        candidates,
    )
    .map_err(|error| CliError::usage(format!("invalid adaptation request: {error}")))?;
    let mut feedback = new_feedback_loop()?;
    let result = feedback
        .record_iteration(
            CodingFeedbackInput::new(
                evaluation,
                expected_output,
                adaptation,
                Usage::new(0, 0, 0, 0),
                retryable,
            )
            .map_err(|error| CliError::usage(error.to_string()))?,
            super::timestamp(),
        )
        .map_err(|error| CliError::execution(error.to_string(), json!({})))?;

    let decision = decision_name(result.decision());
    let human = format!("Coding feedback for {execution_id}: {decision}");
    Ok(success(
        "feedback coding",
        json!({
            "session_id": session_id,
            "execution_id": execution_id,
            "request_digest": request_digest,
            "decision": decision,
            "evaluations": result.evaluations().iter().map(evaluation_value).collect::<Vec<_>>(),
            "reflexion": result.reflexion().map(reflexion_value),
            "adaptation": result.adaptation().map(adaptation_value),
            "loop": snapshot_value(result.snapshot()),
            "durability": "report-only",
        }),
        human,
    ))
}

pub(super) fn new_feedback_loop() -> Result<CodingFeedbackLoop, CliError> {
    let policy = AdaptationPolicy::new(1, 4, 1_000_000, 300_000)
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let config = RunLoopConfig::new(
        3,
        100_000,
        64,
        300,
        10_000_000,
        2,
        LoopTermination::GoalReached,
    )
    .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let mut run_loop = RunLoop::new(
        RunLoopId::new("coding-feedback").expect("built-in run-loop ID is valid"),
        PlanId::new("coding-feedback").expect("built-in plan ID is valid"),
        config,
    )
    .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    run_loop
        .start()
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    CodingFeedbackLoop::new(run_loop, policy, 3)
        .map_err(|error| CliError::internal(error.to_string(), json!({})))
}

fn required_id<T, F>(
    parsed: &super::ParsedArgs,
    option: &str,
    label: &str,
    constructor: F,
) -> Result<T, CliError>
where
    F: FnOnce(String) -> Result<T, pandora_types::IdError>,
{
    parsed
        .value(option)
        .ok_or_else(|| CliError::usage(format!("feedback coding requires '--{option} <value>'")))
        .and_then(|value| {
            constructor(value.to_owned())
                .map_err(|_| CliError::usage(format!("{label} is invalid")))
        })
}

fn required_text(
    parsed: &super::ParsedArgs,
    option: &str,
    limit: usize,
) -> Result<String, CliError> {
    parsed
        .value(option)
        .ok_or_else(|| CliError::usage(format!("feedback coding requires '--{option} <text>'")))
        .and_then(|value| bounded_text(value, limit, option))
}

pub(super) fn bounded_text(value: &str, limit: usize, label: &str) -> Result<String, CliError> {
    if value.len() > limit || value.chars().any(char::is_control) {
        return Err(CliError::usage(format!("{label} is invalid or too long")));
    }
    Ok(value.to_owned())
}

pub(super) fn feedback_value(result: &pandora_runtime::CodingFeedbackResult) -> Value {
    json!({
        "decision": decision_name(result.decision()),
        "evaluations": result.evaluations().iter().map(evaluation_value).collect::<Vec<_>>(),
        "reflexion": result.reflexion().map(reflexion_value),
        "adaptation": result.adaptation().map(adaptation_value),
        "loop": snapshot_value(result.snapshot()),
    })
}

fn decision_name(decision: LoopDecision) -> &'static str {
    match decision {
        LoopDecision::Continue => "continue",
        LoopDecision::Retry => "retry",
        LoopDecision::Completed => "completed",
        LoopDecision::Exhausted => "exhausted",
        LoopDecision::Cancelled => "cancelled",
    }
}

fn evaluation_value(result: &EvaluationResult) -> Value {
    json!({
        "kind": result.kind().as_str(),
        "status": result.status().as_str(),
        "score": result.score(),
        "reason": result.reason(),
        "advisory": result.advisory(),
    })
}

fn reflexion_value(artifact: &pandora_types::ReflexionArtifact) -> Value {
    json!({
        "execution_id": artifact.execution_id(),
        "summary": artifact.summary(),
        "failure_signals": artifact.failure_signals(),
        "lesson": artifact.lesson(),
        "created_at": artifact.created_at().as_unix_seconds(),
    })
}

fn adaptation_value(result: &pandora_runtime::AdaptationResult) -> Value {
    let decision = result.decision();
    json!({
        "decision": {
            "selected": decision.selected().map(adaptation_target_value),
            "changed": decision.changed(),
            "degraded": decision.degraded(),
            "reason": decision.reason(),
        },
        "receipt": {
            "execution_id": result.receipt().execution_id(),
            "session_id": result.receipt().session_id(),
            "request_digest": result.receipt().request_digest(),
            "policy_version": result.receipt().policy_version(),
            "selected": result.receipt().selected().map(adaptation_target_value),
            "changed": result.receipt().changed(),
            "degraded": result.receipt().degraded(),
            "reason": result.receipt().reason(),
            "recorded_at": result.receipt().recorded_at().as_unix_seconds(),
        },
    })
}

fn adaptation_target_value(target: &AdaptationTarget) -> Value {
    let kind = match target {
        AdaptationTarget::Harness(_) => "harness",
        AdaptationTarget::Gene(_) => "gene",
        AdaptationTarget::Skill(_) => "skill",
        AdaptationTarget::Provider(_) => "provider",
        AdaptationTarget::Workflow(_) => "workflow",
        AdaptationTarget::Recovery(_) => "recovery",
        AdaptationTarget::CapabilityReduction(_) => "capability_reduction",
    };
    json!({"kind": kind, "label": target.label()})
}

fn snapshot_value(snapshot: &RunLoopSnapshot) -> Value {
    json!({
        "id": snapshot.id(),
        "plan_id": snapshot.plan_id(),
        "state": format!("{:?}", snapshot.state()).to_ascii_lowercase(),
        "iterations": snapshot.iterations(),
        "retries": snapshot.retries(),
        "used_tokens": snapshot.used_tokens(),
        "used_tools": snapshot.used_tools(),
        "used_duration_seconds": snapshot.used_duration_seconds(),
        "used_cost_micros": snapshot.used_cost_micros(),
    })
}
