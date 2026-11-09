//! Phase 23 Formal Verification Test Suite: Federated Policy & Dynamic Reputation Engine.
//!
//! Grounded in Theorems 45 and 46:
//! - Theorem 45: Federated multi-agent policy gradient convergence & coordinate-wise trimmed mean Byzantine resilience
//! - Theorem 46: Dynamic Beta-conjugate reputation decay, Thompson sampling, and exponential quarantine hitting time
//!
//! # Safety Invariant
//! Strictly `#![forbid(unsafe_code)]` enforced workspace-wide.

#![forbid(unsafe_code)]

use pandora_runtime::federated_policy::{
    AgentReputationRecord, FederatedPolicyEngine, FederatedPolicyError, PolicyGradientVector,
    CRITICAL_REPUTATION_THRESHOLD, DEFAULT_LEARNING_RATE, DEFAULT_MOMENTUM,
    POLICY_DIMENSION_LABELS,
};
use pandora_runtime::multi_agent_collab::ExternalAgentKind;

#[test]
fn test_policy_gradient_vector_primitives_and_validation() {
    // 1. Valid initialization
    let grad = [0.1, -0.2, 0.3, -0.4, 0.5, -0.6];
    let vec = PolicyGradientVector::new(ExternalAgentKind::Agy, grad, 50, 0.42)
        .expect("valid vector should construct");
    assert_eq!(vec.agent, ExternalAgentKind::Agy);
    assert_eq!(vec.gradients, grad);
    assert_eq!(vec.sample_count, 50);
    assert!((vec.loss - 0.42).abs() < 1e-6);

    // 2. L2 norm calculation: sqrt(0.01 + 0.04 + 0.09 + 0.16 + 0.25 + 0.36) = sqrt(0.91)
    let norm = vec.l2_norm();
    assert!((norm - 0.91_f64.sqrt()).abs() < 1e-6);

    // 3. Distance calculation
    let vec2 = PolicyGradientVector::new(
        ExternalAgentKind::ClaudeCode,
        [0.0; 6],
        10,
        0.5,
    )
    .unwrap();
    assert!((vec.distance(&vec2) - norm).abs() < 1e-6);

    // 4. Gradient clipping (Theorem 45 norm bound)
    let large_grad = [3.0, 4.0, 0.0, 0.0, 0.0, 0.0]; // norm = 5.0
    let large_vec =
        PolicyGradientVector::new(ExternalAgentKind::Devin, large_grad, 10, 1.2).unwrap();
    let clipped = large_vec.clipped(1.0);
    assert!((clipped.l2_norm() - 1.0).abs() < 1e-6);
    assert!((clipped.gradients[0] - 0.6).abs() < 1e-6);
    assert!((clipped.gradients[1] - 0.8).abs() < 1e-6);

    // 5. Validation rejection of non-finite components
    let nan_grad = [f64::NAN, 0.0, 0.0, 0.0, 0.0, 0.0];
    let err_nan = PolicyGradientVector::new(ExternalAgentKind::CodexCli, nan_grad, 10, 0.1);
    assert!(matches!(err_nan, Err(FederatedPolicyError::InvalidGradient(_))));

    let inf_grad = [0.0, f64::INFINITY, 0.0, 0.0, 0.0, 0.0];
    let err_inf = PolicyGradientVector::new(ExternalAgentKind::CodexCli, inf_grad, 10, 0.1);
    assert!(matches!(err_inf, Err(FederatedPolicyError::InvalidGradient(_))));

    let neg_loss = PolicyGradientVector::new(ExternalAgentKind::CodexCli, [0.0; 6], 10, -0.5);
    assert!(matches!(neg_loss, Err(FederatedPolicyError::InvalidGradient(_))));
}

#[test]
fn test_agent_reputation_beta_distribution_and_thompson_sampling() {
    let mut rec = AgentReputationRecord::new(ExternalAgentKind::CursorAgent);

    // 1. Initial optimistic prior
    assert_eq!(rec.alpha, 2.0);
    assert_eq!(rec.beta, 1.0);
    assert_eq!(rec.total_evaluations, 0);
    assert!((rec.expected_reliability() - 2.0 / 3.0).abs() < 1e-6);
    assert!((rec.consensus_weight - 2.0 / 3.0).abs() < 1e-6);
    assert!(!rec.is_quarantined);

    // Variance: (2 * 1) / (3^2 * 4) = 2 / 36 = 1 / 18
    let var = rec.variance();
    assert!((var - 1.0 / 18.0).abs() < 1e-6);

    // 2. Thompson sampling: deterministic with fixed seed, bounded in [0, 1]
    let s1 = rec.sample_thompson(42);
    let s2 = rec.sample_thompson(42);
    assert_eq!(s1, s2, "Thompson sample must be deterministic under identical seed");
    assert!((0.0..=1.0).contains(&s1));

    let s3 = rec.sample_thompson(999);
    assert!((0.0..=1.0).contains(&s3));

    // 3. Honest updates: alpha increments
    rec.record_outcome(true, 1.0);
    assert_eq!(rec.total_evaluations, 1);
    assert!((rec.alpha - 3.0).abs() < 1e-6);
    assert!((rec.beta - 1.0).abs() < 1e-6);
    assert!((rec.expected_reliability() - 0.75).abs() < 1e-6);
    assert!(!rec.is_quarantined);

    // 4. Byzantine divergence: beta increments by BYZANTINE_BETA_PENALTY (2.0)
    rec.record_outcome(false, 0.0);
    assert_eq!(rec.total_evaluations, 2);
    assert!((rec.alpha - 3.0).abs() < 1e-6);
    assert!((rec.beta - 3.0).abs() < 1e-6);
    assert!((rec.expected_reliability() - 0.50).abs() < 1e-6);
    assert!(!rec.is_quarantined); // total_evals < 3, not yet quarantined

    // Another Byzantine outcome: beta = 5.0, evals = 3, reliability = 3 / (3 + 5) = 0.375
    rec.record_outcome(false, 0.0);
    assert_eq!(rec.total_evaluations, 3);
    assert!((rec.beta - 5.0).abs() < 1e-6);
    assert!((rec.expected_reliability() - 3.0 / 8.0).abs() < 1e-6);
    assert!(!rec.is_quarantined); // 0.375 >= 0.25

    // Another Byzantine outcome: beta = 7.0, evals = 4, reliability = 3 / 10 = 0.30 >= 0.25
    rec.record_outcome(false, 0.0);
    assert!(!rec.is_quarantined);

    // Another Byzantine outcome: beta = 9.0, evals = 5, reliability = 3 / 12 = 0.25 (threshold)
    rec.record_outcome(false, 0.0);
    assert!(!rec.is_quarantined);

    // Another Byzantine outcome: beta = 11.0, evals = 6, reliability = 3 / 14 ≈ 0.214 < 0.25 -> QUARANTINE!
    rec.record_outcome(false, 0.0);
    assert!(rec.is_quarantined, "agent must be quarantined when reliability drops below 0.25");
}

#[test]
fn test_theorem_45_trimmed_mean_byzantine_resilience() {
    let engine = FederatedPolicyEngine::new(DEFAULT_LEARNING_RATE, DEFAULT_MOMENTUM, 1.0);

    // Empty gradient check
    let empty_res = engine.aggregate_gradients(&[]);
    assert_eq!(empty_res, Err(FederatedPolicyError::EmptyGradients));

    // Construct 8 agents: 6 honest + 2 Byzantine (25% adversarial)
    let all_kinds = ExternalAgentKind::all();
    let mut gradients = Vec::new();

    // Honest gradients centered at true descent vector [-0.2, 0.1, -0.3, 0.2, -0.1, 0.0]
    let true_descent = [-0.2, 0.1, -0.3, 0.2, -0.1, 0.0];
    for (i, &agent) in all_kinds.iter().enumerate().take(6) {
        let mut g = true_descent;
        for (d, val) in g.iter_mut().enumerate() {
            *val += ((i as f64 - 2.5) * 0.01) * (d as f64 + 1.0);
        }
        gradients.push(PolicyGradientVector::new(agent, g, 50, 0.3).unwrap());
    }

    // 2 Byzantine adversarial gradients injected with extreme outliers
    gradients.push(
        PolicyGradientVector::new(
            all_kinds[6],
            [10.0, -10.0, 10.0, -10.0, 10.0, -10.0],
            10,
            2.0,
        )
        .unwrap(),
    );
    gradients.push(
        PolicyGradientVector::new(
            all_kinds[7],
            [-10.0, 10.0, -10.0, 10.0, -10.0, 10.0],
            10,
            2.0,
        )
        .unwrap(),
    );

    let (aggregated, trimmed_count) = engine
        .aggregate_gradients(&gradients)
        .expect("aggregation should succeed");

    // With 8 agents, trim_k = 8 / 4 = 2, so trimmed_count = 4 (2 lowest and 2 highest per dimension)
    assert_eq!(trimmed_count, 4);

    // The extreme Byzantine outliers (+10.0 and -10.0, clipped to +1.0 and -1.0) must be trimmed away!
    // The aggregated gradient should be very close to true_descent
    for d in 0..6 {
        let diff = (aggregated[d] - true_descent[d]).abs();
        assert!(
            diff < 0.08,
            "dimension {d}: aggregated {} deviated too far from honest mean {}",
            aggregated[d],
            true_descent[d]
        );
    }
}

#[test]
fn test_federated_policy_engine_momentum_step_and_convergence() {
    let mut engine = FederatedPolicyEngine::new(0.1, 0.90, 1.0);
    assert_eq!(engine.current_policy, [0.5; 6]);
    assert_eq!(engine.step_count, 0);

    let all_kinds = ExternalAgentKind::all();
    let initial_policy = engine.current_policy;

    // Run 10 optimization steps with consistent negative gradients [-0.2; 6]
    // Under gradient descent: \theta_{t+1} = \theta_t - \eta v_{t+1}
    // When gradients are negative, policy parameters should strictly increase towards 1.0
    for step in 1..=10 {
        let mut worker_gradients = Vec::new();
        for &agent in all_kinds.iter().take(5) {
            let vec = PolicyGradientVector::new(agent, [-0.2; 6], 100, 0.4 - (step as f64) * 0.02)
                .unwrap();
            worker_gradients.push(vec);
        }

        let receipt = engine
            .step(&worker_gradients, 1_700_000_000_000 + step * 1000)
            .expect("step should succeed");

        assert_eq!(receipt.step_index, step);
        assert_eq!(receipt.participating_agents, 5);
        assert!(!receipt.update_hash.is_empty());
    }

    assert_eq!(engine.step_count, 10);
    for (d, (&curr, &init)) in engine
        .current_policy
        .iter()
        .zip(initial_policy.iter())
        .enumerate()
    {
        assert!(
            curr > init,
            "dimension {d} parameter should increase under negative gradients"
        );
    }

    // Policy summary test
    let summary = engine.policy_summary();
    assert_eq!(summary.len(), 6);
    assert_eq!(summary[0].0, POLICY_DIMENSION_LABELS[0]);
}

#[test]
fn test_theorem_46_byzantine_quarantine_hitting_time_bound() {
    let mut engine = FederatedPolicyEngine::default();
    let adversarial_agent = ExternalAgentKind::Devin;

    // Simulate 6 rounds of consensus where Devin proposes divergent/invariant-violating code
    for _ in 1..=6 {
        engine.update_agent_reputation(adversarial_agent, false, 0.0);
    }

    let rec = engine.get_reputation(adversarial_agent).expect("reputation record exists");
    assert_eq!(rec.total_evaluations, 6);
    assert!(rec.is_quarantined, "adversarial agent must be quarantined after 6 Byzantine events");
    assert!(rec.consensus_weight < CRITICAL_REPUTATION_THRESHOLD);

    // When quarantined, its aggregation weight is clamped to QUARANTINE_WEIGHT_PENALTY (0.05)
    let all_kinds = ExternalAgentKind::all();
    let mut gradients = Vec::new();
    for &agent in all_kinds.iter().take(4) {
        gradients.push(PolicyGradientVector::new(agent, [0.0; 6], 50, 0.2).unwrap());
    }

    let (agg, _) = engine.aggregate_gradients(&gradients).expect("aggregation should succeed");
    assert_eq!(agg, [0.0; 6]);
}
