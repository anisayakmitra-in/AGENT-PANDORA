use serde_json::Value;
use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_pandora"))
        .args(args)
        .output()
        .expect("feedback command should start")
}

fn json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("feedback output should be JSON")
}

#[test]
fn coding_feedback_completes_verified_iteration() {
    let output = run(&[
        "feedback",
        "coding",
        "--session",
        "session-feedback-1",
        "--execution",
        "execution-feedback-1",
        "--request-digest",
        "request-feedback-1",
        "--expected-output",
        "tests passed",
        "--output",
        "tests passed",
        "--json",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response = json(&output);
    assert_eq!(response["command"], "feedback coding");
    assert_eq!(response["decision"], "completed");
    assert_eq!(response["evaluations"].as_array().unwrap().len(), 3);
    assert_eq!(response["reflexion"], Value::Null);
    assert_eq!(response["durability"], "report-only");
}

#[test]
fn coding_feedback_selects_bounded_retry_after_failure() {
    let output = run(&[
        "feedback",
        "coding",
        "--session",
        "session-feedback-2",
        "--execution",
        "execution-feedback-2",
        "--request-digest",
        "request-feedback-2",
        "--expected-output",
        "tests passed",
        "--output",
        "tests failed",
        "--retryable",
        "--json",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response = json(&output);
    assert_eq!(response["decision"], "retry");
    assert_eq!(
        response["reflexion"]["failure_signals"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        response["adaptation"]["decision"]["selected"]["kind"],
        "recovery"
    );
    assert_eq!(
        response["adaptation"]["decision"]["selected"]["label"],
        "coding.safe_retry"
    );
}
