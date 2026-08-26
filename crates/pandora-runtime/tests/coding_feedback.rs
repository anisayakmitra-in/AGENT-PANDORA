use pandora_runtime::{CodingFeedbackError, CodingFeedbackInput, CodingFeedbackLoop, RunLoop};
use pandora_types::{
    AdaptationCandidate, AdaptationPolicy, AdaptationRequest, AdaptationTarget, EvaluationRequest,
    ExecutionId, LoopDecision, LoopTermination, PlanId, RequestDigest, RunLoopConfig, RunLoopId,
    RunLoopState, SessionId, Timestamp, Usage,
};

fn feedback_loop(max_retries: u32) -> CodingFeedbackLoop {
    feedback_loop_with_config(3, max_retries, LoopTermination::GoalReached)
}

fn feedback_loop_with_config(
    max_iterations: u32,
    max_retries: u32,
    termination: LoopTermination,
) -> CodingFeedbackLoop {
    let mut run_loop = RunLoop::new(
        RunLoopId::new("coding-feedback-1").unwrap(),
        PlanId::new("coding-domain").unwrap(),
        RunLoopConfig::new(
            max_iterations,
            10_000,
            20,
            300,
            100_000,
            max_retries,
            termination,
        )
        .unwrap(),
    )
    .unwrap();
    run_loop.start().unwrap();
    CodingFeedbackLoop::new(
        run_loop,
        AdaptationPolicy::new(1, 4, 10_000, 5_000).unwrap(),
        3,
    )
    .unwrap()
}

fn evaluation(execution_id: &str, output: &str) -> EvaluationRequest {
    EvaluationRequest::new(
        ExecutionId::new(execution_id).unwrap(),
        Vec::new(),
        output,
        Vec::new(),
    )
    .unwrap()
}

fn adaptation(execution_id: &str, candidates: Vec<AdaptationCandidate>) -> AdaptationRequest {
    AdaptationRequest::new(
        ExecutionId::new(execution_id).unwrap(),
        SessionId::new("session-1").unwrap(),
        RequestDigest::new("pandora-request-v1:sha256:coding-feedback").unwrap(),
        Some(AdaptationTarget::workflow("default-coding").unwrap()),
        candidates,
    )
    .unwrap()
}

fn candidate(id: &str, workflow: &str, approved: bool) -> AdaptationCandidate {
    AdaptationCandidate::new(
        id,
        AdaptationTarget::workflow(workflow).unwrap(),
        90,
        approved,
        false,
        100,
        10,
    )
    .unwrap()
}

fn recovery_candidate(id: &str, action: &str, approved: bool) -> AdaptationCandidate {
    AdaptationCandidate::new(
        id,
        AdaptationTarget::recovery(action).unwrap(),
        10,
        approved,
        false,
        100,
        10,
    )
    .unwrap()
}

fn input(
    execution_id: &str,
    output: &str,
    expected: &str,
    retryable: bool,
    candidates: Vec<AdaptationCandidate>,
) -> CodingFeedbackInput {
    CodingFeedbackInput::new(
        evaluation(execution_id, output),
        expected,
        adaptation(execution_id, candidates),
        Usage::new(100, 2, 5, 50),
        retryable,
    )
    .unwrap()
}

#[test]
fn verified_coding_iteration_completes_without_reflection_or_adaptation() {
    let mut feedback = feedback_loop(1);

    let result = feedback
        .record_iteration(
            input(
                "execution-1",
                "tests passed",
                "tests passed",
                false,
                Vec::new(),
            ),
            Timestamp::from_unix_seconds(10),
        )
        .unwrap();

    assert_eq!(result.decision(), LoopDecision::Completed);
    assert_eq!(result.evaluations().len(), 3);
    assert!(result.evaluations().iter().all(|result| result.passed()));
    assert_eq!(
        result.evaluation_receipt().session_id().as_str(),
        "session-1"
    );
    assert_eq!(
        result.evaluation_receipt().execution_id().as_str(),
        "execution-1"
    );
    assert!(!result.evaluation_receipt().can_authorize_permit());
    assert!(result.reflexion().is_none());
    assert!(result.adaptation().is_none());
    assert_eq!(result.snapshot().state(), RunLoopState::Completed);
}

#[test]
fn failed_retryable_iteration_distills_evidence_and_selects_approved_workflow() {
    let mut feedback = feedback_loop(1);
    let mut request = evaluation("execution-1", "compile failed");
    request = request
        .with_terminal_failure("verification_failed")
        .unwrap();
    let input = CodingFeedbackInput::new(
        request,
        "tests passed",
        adaptation(
            "execution-1",
            vec![candidate("repair", "repair-then-verify", true)],
        ),
        Usage::new(100, 2, 5, 50),
        true,
    )
    .unwrap();

    let result = feedback
        .record_iteration(input, Timestamp::from_unix_seconds(10))
        .unwrap();

    assert_eq!(result.decision(), LoopDecision::Retry);
    let reflexion = result.reflexion().expect("failure should be distilled");
    assert_eq!(reflexion.execution_id().as_str(), "execution-1");
    assert!(!reflexion.failure_signals().is_empty());
    assert_eq!(
        result
            .adaptation()
            .expect("retry should select an adaptation")
            .decision()
            .selected()
            .unwrap()
            .label(),
        "repair-then-verify"
    );
    assert_eq!(result.snapshot().state(), RunLoopState::Running);
}

#[test]
fn unapproved_candidates_cannot_be_selected_for_retry() {
    let mut feedback = feedback_loop(1);

    let result = feedback
        .record_iteration(
            input(
                "execution-1",
                "compile failed",
                "tests passed",
                true,
                vec![candidate("unsafe", "unapproved-repair", false)],
            ),
            Timestamp::from_unix_seconds(10),
        )
        .unwrap();

    assert_eq!(result.decision(), LoopDecision::Retry);
    let adaptation = result
        .adaptation()
        .expect("retry records a no-change decision");
    assert!(adaptation.decision().selected().is_none());
    assert!(adaptation.decision().degraded());
}

#[test]
fn non_retryable_failure_exhausts_without_selecting_another_strategy() {
    let mut feedback = feedback_loop(1);

    let result = feedback
        .record_iteration(
            input(
                "execution-1",
                "policy denied",
                "tests passed",
                false,
                vec![candidate("repair", "repair-then-verify", true)],
            ),
            Timestamp::from_unix_seconds(10),
        )
        .unwrap();

    assert_eq!(result.decision(), LoopDecision::Exhausted);
    assert!(result.reflexion().is_some());
    assert!(result.adaptation().is_none());
    assert_eq!(result.snapshot().state(), RunLoopState::Exhausted);
}

#[test]
fn policy_failure_cannot_be_reclassified_as_a_retryable_coding_error() {
    let mut feedback = feedback_loop(1);
    let request = evaluation("execution-1", "tests passed")
        .with_policy_violations(vec!["undeclared effect requested".to_owned()]);
    let input = CodingFeedbackInput::new(
        request,
        "tests passed",
        adaptation(
            "execution-1",
            vec![candidate("repair", "repair-then-verify", true)],
        ),
        Usage::new(100, 2, 5, 50),
        true,
    )
    .unwrap();

    let result = feedback
        .record_iteration(input, Timestamp::from_unix_seconds(10))
        .unwrap();

    assert_eq!(result.decision(), LoopDecision::Exhausted);
    assert!(result.adaptation().is_none());
    assert_eq!(result.snapshot().state(), RunLoopState::Exhausted);
}

#[test]
fn feedback_evidence_never_copies_the_model_output() {
    let mut feedback = feedback_loop(1);
    let secret_output = "PRIVATE_MODEL_OUTPUT_123";

    let result = feedback
        .record_iteration(
            input(
                "execution-1",
                secret_output,
                "tests passed",
                true,
                vec![candidate("repair", "repair-then-verify", true)],
            ),
            Timestamp::from_unix_seconds(10),
        )
        .unwrap();

    let reflexion = result.reflexion().unwrap();
    assert!(!reflexion.summary().contains(secret_output));
    assert!(!reflexion.lesson().contains(secret_output));
    assert!(
        reflexion
            .failure_signals()
            .iter()
            .all(|signal| !signal.contains(secret_output))
    );
}

#[test]
fn mismatched_execution_evidence_is_rejected_before_loop_progress() {
    let result = CodingFeedbackInput::new(
        evaluation("execution-1", "failed"),
        "tests passed",
        adaptation("execution-2", Vec::new()),
        Usage::new(1, 1, 1, 1),
        true,
    );

    assert!(matches!(
        result,
        Err(CodingFeedbackError::ExecutionMismatch)
    ));
}

#[test]
fn failed_no_progress_iteration_uses_failure_policy_instead_of_completing() {
    let mut feedback = feedback_loop_with_config(3, 1, LoopTermination::NoProgress);
    let request = evaluation("execution-1", "compile failed")
        .with_terminal_failure("verification_failed")
        .unwrap();
    let input = CodingFeedbackInput::new(
        request,
        "tests passed",
        adaptation(
            "execution-1",
            vec![candidate("repair", "repair-then-verify", true)],
        ),
        Usage::new(100, 2, 5, 50),
        true,
    )
    .unwrap();

    let result = feedback
        .record_iteration(input, Timestamp::from_unix_seconds(10))
        .unwrap();

    assert_eq!(result.decision(), LoopDecision::Retry);
    assert!(result.adaptation().is_some());
    assert_eq!(result.snapshot().state(), RunLoopState::Running);
}

#[test]
fn retry_uses_self_healing_for_recovery_candidates() {
    let mut feedback = feedback_loop(1);
    let request = evaluation("execution-1", "worker unavailable")
        .with_terminal_failure("worker_unavailable")
        .unwrap();
    let input = CodingFeedbackInput::new(
        request,
        "tests passed",
        adaptation(
            "execution-1",
            vec![
                candidate("replan", "replan", true),
                recovery_candidate("restart", "restart-worker", true),
            ],
        ),
        Usage::new(100, 2, 5, 50),
        true,
    )
    .unwrap();

    let result = feedback
        .record_iteration(input, Timestamp::from_unix_seconds(10))
        .unwrap();

    assert_eq!(result.decision(), LoopDecision::Retry);
    assert_eq!(
        result
            .adaptation()
            .unwrap()
            .decision()
            .selected()
            .unwrap()
            .label(),
        "restart-worker"
    );
}

#[test]
fn iteration_ceiling_exhausts_before_selecting_another_strategy() {
    let mut feedback = feedback_loop_with_config(1, 1, LoopTermination::GoalReached);

    let result = feedback
        .record_iteration(
            input(
                "execution-1",
                "compile failed",
                "tests passed",
                true,
                vec![candidate("repair", "repair-then-verify", true)],
            ),
            Timestamp::from_unix_seconds(10),
        )
        .unwrap();

    assert_eq!(result.decision(), LoopDecision::Exhausted);
    assert!(result.adaptation().is_none());
    assert_eq!(result.snapshot().iterations(), 1);
    assert!(RunLoop::from_snapshot(result.snapshot().clone()).is_ok());
}
