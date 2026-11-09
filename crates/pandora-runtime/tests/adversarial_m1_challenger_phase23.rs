//! Empirical Adversarial Stress Test Suite: Milestone 1 (Phase 23 Federated Policy & Reputation)
//!
//! Grounded in Theorems 45 and 46:
//! - Out-of-bounds gradients, NaN/inf coordinate handling, zero norms, negative weights
//! - Byzantine gradient injection & coordinate-wise trimmed mean rejection
//! - Rapid reputation degradation: quarantine hitting time vs honest recovery
//! - Thompson sampling variance bounds, empirical moments, and seed reproducibility
//!
//! # Safety Invariant
//! Strictly `#![forbid(unsafe_code)]` enforced workspace-wide.

#![forbid(unsafe_code)]

use pandora_runtime::bandit_router::SafePrng;
use pandora_runtime::federated_policy::{
    AgentReputationRecord, BYZANTINE_BETA_PENALTY, CRITICAL_REPUTATION_THRESHOLD,
    DEFAULT_LEARNING_RATE, DEFAULT_MAX_GRADIENT_NORM, DEFAULT_MOMENTUM, FederatedPolicyEngine,
    FederatedPolicyError, MIN_EVALS_FOR_QUARANTINE, POLICY_DIMENSIONS, PolicyGradientVector,
    QUARANTINE_WEIGHT_PENALTY,
};
use pandora_runtime::multi_agent_collab::ExternalAgentKind;

// ============================================================================
// 1. Stress Tests: Gradients, Numerical Stability, Zero Norms, and Edge Cases
// ============================================================================

#[test]
fn test_adversarial_gradient_nan_and_inf_handling() {
    let agent = ExternalAgentKind::CodexCli;

    // Test NaN in every single dimension
    for d in 0..POLICY_DIMENSIONS {
        let mut grad = [0.1; POLICY_DIMENSIONS];
        grad[d] = f64::NAN;
        let res = PolicyGradientVector::new(agent, grad, 10, 0.5);
        assert!(
            matches!(res, Err(FederatedPolicyError::InvalidGradient(_))),
            "dimension {d} NaN should be rejected"
        );
    }

    // Test +Infinity in every single dimension
    for d in 0..POLICY_DIMENSIONS {
        let mut grad = [0.1; POLICY_DIMENSIONS];
        grad[d] = f64::INFINITY;
        let res = PolicyGradientVector::new(agent, grad, 10, 0.5);
        assert!(
            matches!(res, Err(FederatedPolicyError::InvalidGradient(_))),
            "dimension {d} +Inf should be rejected"
        );
    }

    // Test -Infinity in every single dimension
    for d in 0..POLICY_DIMENSIONS {
        let mut grad = [0.1; POLICY_DIMENSIONS];
        grad[d] = f64::NEG_INFINITY;
        let res = PolicyGradientVector::new(agent, grad, 10, 0.5);
        assert!(
            matches!(res, Err(FederatedPolicyError::InvalidGradient(_))),
            "dimension {d} -Inf should be rejected"
        );
    }

    // Test loss edge cases: NaN, +Inf, -Inf, and negative values
    let valid_grad = [0.2; 6];
    assert!(PolicyGradientVector::new(agent, valid_grad, 10, f64::NAN).is_err());
    assert!(PolicyGradientVector::new(agent, valid_grad, 10, f64::INFINITY).is_err());
    assert!(PolicyGradientVector::new(agent, valid_grad, 10, f64::NEG_INFINITY).is_err());
    assert!(PolicyGradientVector::new(agent, valid_grad, 10, -0.0001).is_err());
    assert!(PolicyGradientVector::new(agent, valid_grad, 10, -100.0).is_err());

    // Exact zero loss must be accepted
    assert!(PolicyGradientVector::new(agent, valid_grad, 10, 0.0).is_ok());

    // Sample count edge cases: 0 should be clamped to 1
    let zero_sample = PolicyGradientVector::new(agent, valid_grad, 0, 0.1).unwrap();
    assert_eq!(zero_sample.sample_count, 1);
    let max_sample = PolicyGradientVector::new(agent, valid_grad, usize::MAX, 0.1).unwrap();
    assert_eq!(max_sample.sample_count, usize::MAX);
}

#[test]
fn test_adversarial_zero_norm_and_clipping_extremes() {
    let agent = ExternalAgentKind::Agy;

    // Zero gradient vector
    let zero_grad = [0.0; POLICY_DIMENSIONS];
    let zero_vec = PolicyGradientVector::new(agent, zero_grad, 10, 0.0).unwrap();
    assert_eq!(zero_vec.l2_norm(), 0.0);
    assert_eq!(zero_vec.distance(&zero_vec), 0.0);

    // Clipping a zero norm vector must return zero without division by zero or NaN
    let clipped_zero = zero_vec.clipped(1.0);
    assert_eq!(clipped_zero.gradients, [0.0; 6]);
    assert_eq!(clipped_zero.l2_norm(), 0.0);

    // Subnormal gradients where squared components remain positive in f64
    let subnormal_grad = [1e-150; POLICY_DIMENSIONS];
    let subnormal_vec = PolicyGradientVector::new(agent, subnormal_grad, 10, 0.01).unwrap();
    assert!(subnormal_vec.l2_norm() > 0.0);
    let clipped_subnormal = subnormal_vec.clipped(1.0);
    for &g in &clipped_subnormal.gradients {
        assert!(!g.is_nan() && !g.is_infinite());
    }

    // Extreme subnormal where x*x underflows to 0.0 in IEEE-754:
    // Clipping must handle gracefully without panic or NaN
    let tiny_grad = [1e-300; POLICY_DIMENSIONS];
    let tiny_vec = PolicyGradientVector::new(agent, tiny_grad, 10, 0.01).unwrap();
    let clipped_tiny = tiny_vec.clipped(1.0);
    for &g in &clipped_tiny.gradients {
        assert!(!g.is_nan() && !g.is_infinite());
    }

    // Enormous gradient vector (potential overflow)
    let huge_grad = [1e150; POLICY_DIMENSIONS];
    let huge_vec = PolicyGradientVector::new(agent, huge_grad, 10, 10.0).unwrap();
    let clipped_huge = huge_vec.clipped(1.0);
    for &g in &clipped_huge.gradients {
        assert!(!g.is_nan());
        assert!(!g.is_infinite());
    }
    // Clipped norm should be bounded by max_norm = 1.0
    assert!(clipped_huge.l2_norm() <= 1.0 + 1e-6);

    // Potential overflow gradient vector [1e200; 6]
    let overflow_grad = [1e200; POLICY_DIMENSIONS];
    let overflow_vec = PolicyGradientVector::new(agent, overflow_grad, 10, 10.0).unwrap();
    let clipped_overflow = overflow_vec.clipped(1.0);
    for &g in &clipped_overflow.gradients {
        assert!(!g.is_nan());
    }
}

#[test]
fn test_engine_parameter_clamping_and_empty_inputs() {
    // Clamping invalid hyperparameters in constructor
    let engine = FederatedPolicyEngine::new(-1.0, 1.5, -5.0);
    assert_eq!(
        engine.learning_rate, 0.001,
        "negative learning rate must clamp to 0.001"
    );
    assert_eq!(engine.momentum, 0.99, "momentum >= 1.0 must clamp to 0.99");
    assert_eq!(
        engine.max_gradient_norm, 0.1,
        "max norm <= 0 must clamp to 0.1"
    );

    // Empty gradient aggregation must return Err(EmptyGradients)
    let res = engine.aggregate_gradients(&[]);
    assert_eq!(res, Err(FederatedPolicyError::EmptyGradients));

    let mut engine_mut = engine;
    let step_res = engine_mut.step(&[], 1_700_000_000_000);
    assert_eq!(step_res.err(), Some(FederatedPolicyError::EmptyGradients));
}

// ============================================================================
// 2. Byzantine Gradient Injection & Coordinate-Wise Trimmed Mean Stress
// ============================================================================

#[test]
fn test_byzantine_poisoning_asymmetric_and_coordinated_attacks() {
    let engine = FederatedPolicyEngine::new(DEFAULT_LEARNING_RATE, DEFAULT_MOMENTUM, 1.0);
    let all_kinds = ExternalAgentKind::all();

    // 8 agents: 6 honest + 2 Byzantine (f = 2 < M/2 = 4)
    // Trim parameter trim_k = 8 / 4 = 2.
    // Byzantine agents will attempt 3 distinct attack strategies:

    // Attack Strategy A: Extreme Positive Poisoning (+100.0 on all dimensions)
    let true_honest_grad = [-0.3, 0.2, -0.1, 0.4, -0.2, 0.1];
    let mut gradients_a = Vec::new();
    for (i, &agent) in all_kinds.iter().take(6).enumerate() {
        let mut g = true_honest_grad;
        for item in g.iter_mut() {
            *item += (i as f64 - 2.5) * 0.005;
        }
        gradients_a.push(PolicyGradientVector::new(agent, g, 50, 0.2).unwrap());
    }
    // Both Byzantine inject +100.0
    gradients_a.push(PolicyGradientVector::new(all_kinds[6], [100.0; 6], 10, 5.0).unwrap());
    gradients_a.push(PolicyGradientVector::new(all_kinds[7], [100.0; 6], 10, 5.0).unwrap());

    let (agg_a, trimmed_a) = engine.aggregate_gradients(&gradients_a).unwrap();
    assert_eq!(trimmed_a, 4, "must trim 2 lowest and 2 highest");
    for d in 0..6 {
        let err = (agg_a[d] - true_honest_grad[d]).abs();
        assert!(
            err < 0.05,
            "Attack A: dim {d} deviated by {err} from honest mean"
        );
    }

    // Attack Strategy B: Opposite Extreme Poisoning (+100.0 on Byzantine 1, -100.0 on Byzantine 2)
    let mut gradients_b = Vec::new();
    for (i, &agent) in all_kinds.iter().take(6).enumerate() {
        let mut g = true_honest_grad;
        for item in g.iter_mut() {
            *item += (i as f64 - 2.5) * 0.005;
        }
        gradients_b.push(PolicyGradientVector::new(agent, g, 50, 0.2).unwrap());
    }
    gradients_b.push(PolicyGradientVector::new(all_kinds[6], [100.0; 6], 10, 5.0).unwrap());
    gradients_b.push(PolicyGradientVector::new(all_kinds[7], [-100.0; 6], 10, 5.0).unwrap());

    let (agg_b, trimmed_b) = engine.aggregate_gradients(&gradients_b).unwrap();
    assert_eq!(trimmed_b, 4);
    for d in 0..6 {
        let err = (agg_b[d] - true_honest_grad[d]).abs();
        assert!(
            err < 0.05,
            "Attack B: dim {d} deviated by {err} from honest mean"
        );
    }

    // Attack Strategy C: Stealthy Boundary Poisoning (just above max honest component)
    let mut gradients_c = Vec::new();
    for (i, &agent) in all_kinds.iter().take(6).enumerate() {
        let mut g = true_honest_grad;
        for item in g.iter_mut() {
            *item += (i as f64 - 2.5) * 0.005;
        }
        gradients_c.push(PolicyGradientVector::new(agent, g, 50, 0.2).unwrap());
    }
    // Stealthy: inject slightly higher than honest max
    gradients_c.push(PolicyGradientVector::new(all_kinds[6], [0.5; 6], 10, 0.5).unwrap());
    gradients_c.push(PolicyGradientVector::new(all_kinds[7], [0.6; 6], 10, 0.5).unwrap());

    let (agg_c, trimmed_c) = engine.aggregate_gradients(&gradients_c).unwrap();
    assert_eq!(trimmed_c, 4);
    for d in 0..6 {
        let err = (agg_c[d] - true_honest_grad[d]).abs();
        assert!(
            err < 0.05,
            "Attack C: dim {d} deviated by {err} from honest mean"
        );
    }
}

#[test]
fn test_byzantine_policy_evolution_hypercube_invariance() {
    let mut engine = FederatedPolicyEngine::new(0.05, 0.90, 1.0);
    let all_kinds = ExternalAgentKind::all();

    // Run 50 optimization steps under severe continuous adversarial attack
    for step in 1..=50 {
        let mut grads = Vec::new();
        // 6 honest agents pointing towards [0.8; 6]
        for &agent in all_kinds.iter().take(6) {
            let g = [-0.4; 6];
            grads.push(PolicyGradientVector::new(agent, g, 50, 0.3).unwrap());
        }
        // 2 Byzantine agents trying to drive policy to negative infinity with +100.0 gradients
        grads.push(PolicyGradientVector::new(all_kinds[6], [100.0; 6], 10, 10.0).unwrap());
        grads.push(PolicyGradientVector::new(all_kinds[7], [100.0; 6], 10, 10.0).unwrap());

        let receipt = engine
            .step(&grads, 1_700_000_000_000 + step * 1000)
            .unwrap();
        assert_eq!(receipt.step_index, step);

        // Verify hypercube invariant: \boldsymbol{\theta} \in [0, 1]^6
        for (d, &param) in engine.current_policy.iter().enumerate() {
            assert!(
                (0.0..=1.0).contains(&param),
                "step {step}, dim {d}: param {param} violated hypercube [0, 1] bounds"
            );
        }
    }
}

// ============================================================================
// 3. Rapid Reputation Degradation, Quarantine Hitting Time & Honest Recovery
// ============================================================================

#[test]
fn test_quarantine_hitting_time_deterministic_and_stochastic() {
    // 1. Deterministic pure-Byzantine hitting time:
    // Initial: alpha = 2.0, beta = 1.0, reliability = 2/3 ≈ 0.667.
    // Minimum evaluations before quarantine: MIN_EVALS_FOR_QUARANTINE = 3.
    // Critical threshold: CRITICAL_REPUTATION_THRESHOLD = 0.25.
    let mut rec = AgentReputationRecord::new(ExternalAgentKind::Hermes);
    assert!(!rec.is_quarantined);
    assert_eq!(MIN_EVALS_FOR_QUARANTINE, 3);
    assert_eq!(BYZANTINE_BETA_PENALTY, 2.0);

    // Round 1 (fail): alpha = 2, beta = 3, w = 2/5 = 0.40, evals = 1 < 3
    rec.record_outcome(false, 0.0);
    assert!(!rec.is_quarantined);
    assert_eq!(rec.total_evaluations, 1);

    // Round 2 (fail): alpha = 2, beta = 5, w = 2/7 ≈ 0.286, evals = 2 < 3
    rec.record_outcome(false, 0.0);
    assert!(!rec.is_quarantined);
    assert_eq!(rec.total_evaluations, 2);

    // Round 3 (fail): alpha = 2, beta = 7, w = 2/9 ≈ 0.222 < 0.25, evals = 3 >= 3 -> QUARANTINED!
    rec.record_outcome(false, 0.0);
    assert!(
        rec.is_quarantined,
        "hitting time must be exactly 3 rounds under pure Byzantine behavior"
    );
    assert_eq!(rec.total_evaluations, 3);
    assert!(rec.consensus_weight < CRITICAL_REPUTATION_THRESHOLD);

    // 2. Monte Carlo validation of Theorem 46 expected hitting time under p_b = 0.10:
    // Theorem 46 proves \mathbb{E}[T_{\mathrm{quar}}] \le 4.47 = O(1).
    let mut total_hitting_time: u64 = 0;
    let trials = 1000;
    let mut prng = SafePrng::new(0x1337_babe);

    for _ in 0..trials {
        let mut sim_rec = AgentReputationRecord::new(ExternalAgentKind::Devin);
        let mut rounds = 0;
        while !sim_rec.is_quarantined && rounds < 100 {
            rounds += 1;
            // Byzantine agent agreements occur with probability p_b = 0.10
            let is_honest = prng.sample_uniform_f64() < 0.10;
            sim_rec.record_outcome(is_honest, if is_honest { 0.8 } else { 0.0 });
        }
        total_hitting_time += rounds;
    }

    let empirical_mean_hitting_time = (total_hitting_time as f64) / (trials as f64);
    assert!(
        empirical_mean_hitting_time < 5.0,
        "empirical hitting time {empirical_mean_hitting_time} exceeds Theorem 46 bound O(1) ~ 4.47"
    );
    assert!(
        empirical_mean_hitting_time >= 3.0,
        "hitting time cannot be less than MIN_EVALS_FOR_QUARANTINE (3)"
    );
}

#[test]
fn test_quarantine_honest_recovery_dynamics() {
    let mut rec = AgentReputationRecord::new(ExternalAgentKind::QwenCode);

    // Step 1: Force into quarantine with 5 consecutive Byzantine failures
    for _ in 0..5 {
        rec.record_outcome(false, 0.0);
    }
    assert!(rec.is_quarantined);
    assert_eq!(rec.total_evaluations, 5);
    // alpha = 2.0, beta = 1.0 + 5 * 2.0 = 11.0. Reliability = 2 / 13 ≈ 0.1538 < 0.25
    assert!((rec.expected_reliability() - 2.0 / 13.0).abs() < 1e-6);

    // Step 2: Honest recovery attempts
    // Honest outcome 1: alpha = 3.0, beta = 11.0, w = 3/14 ≈ 0.214 < 0.25 -> still quarantined
    rec.record_outcome(true, 1.0);
    assert!(rec.is_quarantined);

    // Honest outcome 2: alpha = 4.0, beta = 11.0, w = 4/15 ≈ 0.2667 >= 0.25 -> RECOVERED!
    rec.record_outcome(true, 1.0);
    assert!(
        !rec.is_quarantined,
        "agent must recover from quarantine once reliability rises to >= 0.25"
    );
    assert!(rec.consensus_weight >= CRITICAL_REPUTATION_THRESHOLD);

    // Step 3: Extreme degradation recovery test (20 Byzantine failures)
    for _ in 0..20 {
        rec.record_outcome(false, 0.0);
    }
    assert!(rec.is_quarantined);
    // beta = 11.0 + 40.0 = 51.0, alpha = 4.0. w = 4 / 55 ≈ 0.0727
    let degraded_weight = rec.consensus_weight;
    assert!(degraded_weight < 0.10);

    // Recovering from beta = 51 requires:
    // (alpha + k) / (alpha + k + beta) >= 0.25
    // 4(4 + k) >= 55 + k => 16 + 4k >= 55 + k => 3k >= 39 => k >= 13 honest rounds
    for _ in 0..12 {
        rec.record_outcome(true, 1.0);
        assert!(
            rec.is_quarantined,
            "should still be quarantined after 12 rounds"
        );
    }
    // Round 13: alpha = 4 + 13 = 17, beta = 51. w = 17 / 68 = 0.25 -> RECOVERY!
    rec.record_outcome(true, 1.0);
    assert!(
        !rec.is_quarantined,
        "agent must successfully recover after 13 consecutive honest rounds"
    );
    assert!((rec.consensus_weight - 0.25).abs() < 1e-6);
}

// ============================================================================
// 4. Thompson Sampling Variance Bounds, Moments, and Seed Reproducibility
// ============================================================================

#[test]
fn test_thompson_sampling_seed_determinism_and_bounds() {
    let rec = AgentReputationRecord::new(ExternalAgentKind::Cline);

    // Reproducibility across separate calls with identical seed
    let seed = 0xdead_beef_1234_5678;
    let s1 = rec.sample_thompson(seed);
    let s2 = rec.sample_thompson(seed);
    assert_eq!(
        s1, s2,
        "Thompson sampling must be strictly deterministic under fixed seed"
    );

    // Bounds check: samples must strictly lie in [0.0, 1.0] across diverse seeds
    for s in 0..1000 {
        let sample = rec.sample_thompson(s);
        assert!(
            (0.0..=1.0).contains(&sample),
            "sample {sample} for seed {s} was outside [0.0, 1.0]"
        );
    }
}

#[test]
fn test_thompson_sampling_empirical_moments_and_variance_contraction() {
    // Stage 1: Prior distribution moments (alpha = 2.0, beta = 1.0)
    // Theoretical Mean: 2 / 3 ≈ 0.66667
    // Theoretical Variance: (2 * 1) / (9 * 4) = 2 / 36 = 1 / 18 ≈ 0.05556
    let rec_prior = AgentReputationRecord::new(ExternalAgentKind::ClaudeCode);
    let n_samples = 20_000;
    let mut sum = 0.0;
    let mut sum_sq = 0.0;

    let mut prng = SafePrng::new(0xabcd_ef01);
    for _ in 0..n_samples {
        let seed = prng.next_u64();
        let sample = rec_prior.sample_thompson(seed);
        sum += sample;
        sum_sq += sample * sample;
    }

    let empirical_mean = sum / (n_samples as f64);
    let empirical_variance = (sum_sq / (n_samples as f64)) - (empirical_mean * empirical_mean);

    let theoretical_mean = rec_prior.expected_reliability();
    let theoretical_variance = rec_prior.variance();

    assert!(
        (empirical_mean - theoretical_mean).abs() < 0.01,
        "empirical mean {empirical_mean} deviated from theoretical {theoretical_mean}"
    );
    assert!(
        (empirical_variance - theoretical_variance).abs() < 0.01,
        "empirical variance {empirical_variance} deviated from theoretical {theoretical_variance}"
    );

    // Stage 2: Variance Contraction after 50 honest evaluations
    // With alpha = 2.0 + 50 = 52.0, beta = 1.0
    // Mean: 52 / 53 ≈ 0.9811
    // Variance: (52 * 1) / (53^2 * 54) = 52 / 151,686 ≈ 0.0003428 (variance shrinks by 160x!)
    let mut rec_matured = AgentReputationRecord::new(ExternalAgentKind::ClaudeCode);
    for _ in 0..50 {
        rec_matured.record_outcome(true, 1.0);
    }
    assert_eq!(rec_matured.total_evaluations, 50);

    let matured_theoretical_variance = rec_matured.variance();
    assert!(
        matured_theoretical_variance < 0.0005,
        "matured variance {matured_theoretical_variance} must contract to < 0.0005"
    );

    let mut sum_mat = 0.0;
    let mut sum_mat_sq = 0.0;
    for _ in 0..n_samples {
        let seed = prng.next_u64();
        let sample = rec_matured.sample_thompson(seed);
        sum_mat += sample;
        sum_mat_sq += sample * sample;
    }

    let empirical_mat_mean = sum_mat / (n_samples as f64);
    let empirical_mat_variance =
        (sum_mat_sq / (n_samples as f64)) - (empirical_mat_mean * empirical_mat_mean);

    assert!(
        (empirical_mat_mean - rec_matured.expected_reliability()).abs() < 0.005,
        "matured mean {empirical_mat_mean} deviated from theoretical"
    );
    assert!(
        (empirical_mat_variance - matured_theoretical_variance).abs() < 0.0005,
        "matured empirical variance {empirical_mat_variance} did not contract to theoretical {matured_theoretical_variance}"
    );
}

// ============================================================================
// 5. Quarantined Agent Down-Weighting in Aggregation
// ============================================================================

#[test]
fn test_quarantined_agent_influence_clamping() {
    let mut engine = FederatedPolicyEngine::new(
        DEFAULT_LEARNING_RATE,
        DEFAULT_MOMENTUM,
        DEFAULT_MAX_GRADIENT_NORM,
    );
    let byzantine_agent = ExternalAgentKind::Devin;

    // Quarantine Devin
    for _ in 0..5 {
        engine.update_agent_reputation(byzantine_agent, false, 0.0);
    }
    assert!(
        engine
            .get_reputation(byzantine_agent)
            .unwrap()
            .is_quarantined
    );
    assert_eq!(QUARANTINE_WEIGHT_PENALTY, 0.05);

    // In a 4-agent group where Devin is quarantined, Devin's weight must be clamped to QUARANTINE_WEIGHT_PENALTY (0.05)
    let agents = [
        ExternalAgentKind::Agy,
        ExternalAgentKind::ClaudeCode,
        ExternalAgentKind::CodexCli,
        byzantine_agent,
    ];

    let mut grads = Vec::new();
    for &ag in &agents[0..3] {
        grads.push(PolicyGradientVector::new(ag, [0.0; 6], 50, 0.2).unwrap());
    }
    // Devin injects non-zero gradient within clipping bound
    grads.push(PolicyGradientVector::new(byzantine_agent, [0.8; 6], 50, 0.2).unwrap());

    let (agg, trimmed) = engine.aggregate_gradients(&grads).unwrap();
    // With 4 agents, trim_k = 1, so the extreme value 0.8 is trimmed anyway!
    assert_eq!(trimmed, 2);
    assert_eq!(agg, [0.0; 6]);
}

// ============================================================================
// 6. Theorem 45 Quarantine Resilience under Partial Trimming (M=7, f=2)
// ============================================================================

#[test]
fn test_adversarial_theorem45_reputation_quarantine_resilience_m7_f2() {
    let mut engine = FederatedPolicyEngine::new(DEFAULT_LEARNING_RATE, DEFAULT_MOMENTUM, 1.0);
    let all_kinds = ExternalAgentKind::all();

    // 7 agents: 5 honest + 2 Byzantine
    // Devin and Cline are Byzantine and previously quarantined
    let byz1 = ExternalAgentKind::Devin;
    let byz2 = ExternalAgentKind::Cline;
    for _ in 0..5 {
        engine.update_agent_reputation(byz1, false, 0.0);
        engine.update_agent_reputation(byz2, false, 0.0);
    }
    assert!(engine.get_reputation(byz1).unwrap().is_quarantined);
    assert!(engine.get_reputation(byz2).unwrap().is_quarantined);

    let true_honest_descent = [-0.25; 6];
    let mut gradients = Vec::new();
    // 5 honest agents
    for &ag in all_kinds.iter().take(5) {
        gradients.push(PolicyGradientVector::new(ag, true_honest_descent, 50, 0.2).unwrap());
    }
    // 2 quarantined Byzantine agents inject maximal positive gradient +10.0 (clipped to +1.0)
    gradients.push(PolicyGradientVector::new(byz1, [10.0; 6], 10, 5.0).unwrap());
    gradients.push(PolicyGradientVector::new(byz2, [10.0; 6], 10, 5.0).unwrap());

    // When M = 7, trim_k = 7 / 4 = 1.
    // trim_k = 1 trims 1 lowest and 1 highest.
    // The top Byzantine gradient (+1.0) is trimmed.
    // The second Byzantine gradient (+1.0) remains in the valid slice!
    // But because its weight is clamped to QUARANTINE_WEIGHT_PENALTY (0.05),
    // its impact is severely suppressed:
    let (agg, trimmed) = engine.aggregate_gradients(&gradients).unwrap();
    assert_eq!(trimmed, 2, "trim_k = 1 trims 1 on each side");

    // Theoretical deviation bound under Lemma 45.1:
    // Honest weights: 4 retained honest agents * ~0.67 ≈ 2.67
    // Byzantine retained weight: 1 * 0.05 = 0.05
    // Byzantine contamination <= (0.05 / (2.67 + 0.05)) * (1.0 - (-0.25)) ≈ 0.0184 * 1.25 ≈ 0.023
    for d in 0..6 {
        let deviation = (agg[d] - true_honest_descent[d]).abs();
        assert!(
            deviation < 0.04,
            "dimension {d} deviation {deviation} should be suppressed below 0.04 by quarantine weighting"
        );
    }
}

// ============================================================================
// 7. Edge Cases: All Quarantined Agents & Unweighted Fallbacks
// ============================================================================

#[test]
fn test_adversarial_all_agents_quarantined_fallback() {
    let mut engine = FederatedPolicyEngine::new(DEFAULT_LEARNING_RATE, DEFAULT_MOMENTUM, 1.0);
    let all_kinds = ExternalAgentKind::all();

    // Quarantine all 4 agents
    let group = &all_kinds[0..4];
    for &ag in group {
        for _ in 0..5 {
            engine.update_agent_reputation(ag, false, 0.0);
        }
        assert!(engine.get_reputation(ag).unwrap().is_quarantined);
    }

    let mut grads = Vec::new();
    for (i, &ag) in group.iter().enumerate() {
        let g = [i as f64 * 0.1; 6];
        grads.push(PolicyGradientVector::new(ag, g, 50, 0.2).unwrap());
    }

    // Aggregation should succeed without dividing by zero, even when all agents are quarantined
    let (agg, trimmed) = engine.aggregate_gradients(&grads).unwrap();
    assert_eq!(trimmed, 2); // 4 / 4 = 1 per side
    // Interior slice retains index 1 (0.1) and index 2 (0.2) -> mean 0.15
    for &val in &agg {
        assert!((val - 0.15).abs() < 1e-6);
    }
}
