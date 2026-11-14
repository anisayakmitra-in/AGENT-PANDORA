#![forbid(unsafe_code)]

//! Phase 312 Milestone 3: Deterministic `pandora-harnesses` Decision Benchmark Tests
//! comparing `HeuristicDecisionDriver` and `MoiraDecisionDriver` on real task traces.

use pandora_harnesses::decision_benchmark::DecisionBenchmarkSuite;
use pandora_runtime::decision_engine::{
    DecisionDriver, HeuristicDecisionDriver, MoiraDecisionDriver, benchmark_moira_vs_heuristic,
};

#[test]
fn test_benchmark_moira_fallback_vs_heuristic_full_equivalence() {
    let heuristic = HeuristicDecisionDriver::new();
    let moira_fallback = MoiraDecisionDriver::new(None);
    assert!(!moira_fallback.has_loaded_network());

    let report = benchmark_moira_vs_heuristic(&moira_fallback, &heuristic)
        .expect("comparative benchmark succeeds");

    assert_eq!(report.heuristic.trace_count, 12);
    assert_eq!(report.moira.trace_count, 12);
    assert_eq!(report.agreement_count, 12);
    assert_eq!(report.agreement_rate, 1.0);
    assert_eq!(report.heuristic.branch_selection_accuracy, 1.0);
    assert_eq!(report.moira.branch_selection_accuracy, 1.0);
    assert!((report.margin_delta).abs() < 1e-9);
    assert!(report.heuristic.mean_latency_micros < 20_000.0);
    assert!(report.moira.mean_latency_micros < 20_000.0);
}

#[test]
fn test_benchmark_moira_calibrated_network_vs_heuristic_on_real_task_traces() {
    let heuristic = HeuristicDecisionDriver::new();
    let moira_calibrated = MoiraDecisionDriver::calibrated();
    assert!(moira_calibrated.has_loaded_network());

    let report = benchmark_moira_vs_heuristic(&moira_calibrated, &heuristic)
        .expect("comparative benchmark succeeds");

    assert_eq!(report.heuristic.trace_count, 12);
    assert_eq!(report.moira.trace_count, 12);
    assert_eq!(
        report.moira.branch_selection_accuracy, 1.0,
        "Calibrated Moira network must achieve 100% branch-selection accuracy across all 12 real task traces"
    );
    assert_eq!(
        report.agreement_rate, 1.0,
        "Calibrated Moira network must agree 100% with ground-truth heuristic decisions"
    );
    assert!(
        report.moira.mean_margin > 0.85,
        "Calibrated Moira mean margin ({}) should be high",
        report.moira.mean_margin
    );
    assert!(report.moira.max_latency_micros < 50_000);
}

#[test]
fn test_empty_benchmark_suite_handles_gracefully() {
    let empty_suite = DecisionBenchmarkSuite::new(Vec::new());
    let heuristic = HeuristicDecisionDriver::new();
    let moira = MoiraDecisionDriver::calibrated();

    let report = empty_suite
        .compare_drivers(
            |q| heuristic.evaluate_question(q),
            |q| moira.evaluate_question(q),
        )
        .expect("empty suite evaluates cleanly");

    assert_eq!(report.heuristic.trace_count, 0);
    assert_eq!(report.moira.trace_count, 0);
    assert_eq!(report.agreement_rate, 1.0);
}
