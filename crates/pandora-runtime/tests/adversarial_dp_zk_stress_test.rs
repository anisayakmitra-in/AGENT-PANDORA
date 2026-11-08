#![forbid(unsafe_code)]

//! Adversarial empirical integration test suite for:
//! - Phase 9: Distributed Differential Privacy (DP Budget Tracker)
//! - Phase 10: Zero-Knowledge Succinct State Attestation
//!
//! Stress-tests edge orders, boundary parameters, large composition chains,
//! multi-agent concurrency, Goldilocks arithmetic extremes, roots of unity,
//! and exhaustive proof forgery/tampering attacks.

use std::sync::{Arc, Mutex};
use std::thread;

use pandora_runtime::bandit_router::SafePrng;
use pandora_runtime::privacy_budget::{
    DistributedPrivacyTracker, GaussianMechanism, LaplaceMechanism, PrivacyAccountantError,
    RdpBudget, RdpOrder, STANDARD_RDP_ORDERS, key_to_order, order_to_key,
};
use pandora_runtime::zk_attestation::{
    FiatShamirTranscript, FieldElement, GOLDILOCKS_PRIME, Polynomial, StateTransitionAir,
    ZkAttestationError,
};

// ============================================================================
// PART 1: PHASE 9 DIFFERENTIAL PRIVACY STRESS & ADVERSARIAL TESTING
// ============================================================================

/// Stress-test boundary, zero, negative, and non-finite Rényi orders and mechanism parameters.
#[test]
fn test_adv_dp_order_and_parameter_boundaries() {
    // 1. Exact boundary alpha = 1.0 (RDP requires alpha > 1.0)
    assert!(RdpOrder::new(1.0).is_err(), "alpha=1.0 must be rejected");
    assert!(
        RdpOrder::new(0.9999999).is_err(),
        "alpha < 1.0 must be rejected"
    );
    assert!(RdpOrder::new(0.0).is_err(), "alpha=0 must be rejected");
    assert!(
        RdpOrder::new(-1.0).is_err(),
        "negative alpha must be rejected"
    );
    assert!(
        RdpOrder::new(f64::NAN).is_err(),
        "NaN alpha must be rejected"
    );
    assert!(
        RdpOrder::new(f64::INFINITY).is_err(),
        "Infinite alpha must be rejected"
    );
    assert!(
        RdpOrder::new(f64::NEG_INFINITY).is_err(),
        "Neg-infinite alpha must be rejected"
    );

    // Orders barely above 1.0
    let epsilon_order = RdpOrder::new(1.0001);
    assert!(
        epsilon_order.is_ok(),
        "alpha slightly above 1.0 should be accepted"
    );

    // Order to key and back mapping stability
    for &alpha in &STANDARD_RDP_ORDERS {
        let key = order_to_key(alpha);
        let recovered = key_to_order(key);
        assert!(
            (recovered - alpha).abs() < 1e-6,
            "order key conversion mismatch for alpha={alpha}"
        );
    }

    // 2. Gaussian mechanism invalid parameters
    assert!(
        GaussianMechanism::new(0.0, 1.0).is_err(),
        "sensitivity=0 must fail"
    );
    assert!(
        GaussianMechanism::new(-0.5, 1.0).is_err(),
        "negative sensitivity must fail"
    );
    assert!(
        GaussianMechanism::new(f64::NAN, 1.0).is_err(),
        "NaN sensitivity must fail"
    );
    assert!(
        GaussianMechanism::new(1.0, 0.0).is_err(),
        "sigma=0 must fail"
    );
    assert!(
        GaussianMechanism::new(1.0, -1.0).is_err(),
        "negative sigma must fail"
    );
    assert!(
        GaussianMechanism::new(1.0, f64::NAN).is_err(),
        "NaN sigma must fail"
    );

    let g_mech = GaussianMechanism::new(1.0, 2.0).unwrap();
    assert!(
        g_mech.rdp_epsilon(1.0).is_err(),
        "order=1.0 must fail in Gaussian rdp_epsilon"
    );
    assert!(
        g_mech.rdp_epsilon(0.5).is_err(),
        "order<1.0 must fail in Gaussian rdp_epsilon"
    );
    assert!(
        g_mech.rdp_epsilon(f64::NAN).is_err(),
        "order=NaN must fail in Gaussian rdp_epsilon"
    );

    // Extreme noise scales
    let huge_noise = GaussianMechanism::new(1.0, 1e6).unwrap();
    assert!(
        huge_noise.rdp_epsilon(2.0).unwrap() < 1e-11,
        "huge noise must yield near-zero epsilon"
    );

    let tiny_noise = GaussianMechanism::new(1.0, 1e-3).unwrap();
    assert!(
        tiny_noise.rdp_epsilon(2.0).unwrap() > 1e5,
        "tiny noise must yield huge epsilon"
    );

    // 3. Laplace mechanism invalid parameters and numerical overflow
    assert!(
        LaplaceMechanism::new(0.0, 1.0).is_err(),
        "sensitivity=0 must fail"
    );
    assert!(
        LaplaceMechanism::new(-1.0, 1.0).is_err(),
        "negative sensitivity must fail"
    );
    assert!(
        LaplaceMechanism::new(1.0, 0.0).is_err(),
        "scale=0 must fail"
    );
    assert!(
        LaplaceMechanism::new(1.0, -0.1).is_err(),
        "negative scale must fail"
    );

    let l_mech = LaplaceMechanism::new(1.0, 1.0).unwrap();
    assert!(
        l_mech.rdp_epsilon(1.0).is_err(),
        "order=1.0 must fail in Laplace rdp_epsilon"
    );
    assert!(
        l_mech.rdp_epsilon(0.5).is_err(),
        "order<1.0 must fail in Laplace rdp_epsilon"
    );

    // Numerical overflow guard: lambda = sensitivity / scale is very large
    let overflow_mech = LaplaceMechanism::new(10_000.0, 0.01).unwrap(); // lambda = 1_000_000
    let overflow_res = overflow_mech.rdp_epsilon(64.0); // (64-1) * 1e6 > 700
    match overflow_res {
        Err(PrivacyAccountantError::NumericalOverflow(_)) => {
            // Correctly handled without panicking or producing Inf/NaN!
        }
        other => panic!("expected NumericalOverflow, got {:?}", other),
    }

    // 4. Target delta boundaries in RdpBudget
    let budget = RdpBudget::from_gaussian(1.0, 2.0).unwrap();
    assert!(budget.epsilon_at_delta(0.0).is_err(), "delta=0.0 must fail");
    assert!(budget.epsilon_at_delta(1.0).is_err(), "delta=1.0 must fail");
    assert!(budget.epsilon_at_delta(-0.01).is_err(), "delta<0 must fail");
    assert!(budget.epsilon_at_delta(1.5).is_err(), "delta>1 must fail");
    assert!(
        budget.epsilon_at_delta(f64::NAN).is_err(),
        "delta=NaN must fail"
    );
}

/// Stress-test sequential composition of 200 heterogenous queries (Theorem 17)
/// and verify strict linear additivity, lack of numerical drift, and monotonicity.
#[test]
fn test_adv_dp_large_chain_composition_200_queries() {
    let num_queries = 200;
    let sens_g = 1.0;
    let sigma_g = 3.0;
    let sens_l = 0.5;
    let scale_l = 2.0;

    let g_budget = RdpBudget::from_gaussian(sens_g, sigma_g).unwrap();
    let l_budget = RdpBudget::from_laplace(sens_l, scale_l).unwrap();

    let mut accumulated = RdpBudget::zero();
    let mut checkpoints = Vec::new();

    for i in 0..num_queries {
        if i % 2 == 0 {
            accumulated.compose_in_place(&g_budget);
        } else {
            accumulated.compose_in_place(&l_budget);
        }

        if (i + 1) == 10 || (i + 1) == 50 || (i + 1) == 100 || (i + 1) == 200 {
            let eps_delta = accumulated.epsilon_at_delta(1e-5).unwrap();
            checkpoints.push((i + 1, eps_delta));
        }
    }

    // Verify exact linear additivity:
    // 100 Gaussian mechanisms + 100 Laplace mechanisms
    let num_g = num_queries / 2;
    let num_l = num_queries / 2;

    for &alpha in &STANDARD_RDP_ORDERS {
        let key = order_to_key(alpha);
        let g_val = g_budget.values.get(&key).copied().unwrap();
        let l_val = l_budget.values.get(&key).copied().unwrap();
        let expected = (num_g as f64) * g_val + (num_l as f64) * l_val;
        let actual = accumulated.values.get(&key).copied().unwrap();

        assert!(
            (actual - expected).abs() < 1e-9,
            "composition mismatch at alpha={alpha}: expected {expected}, got {actual}"
        );
    }

    // Verify strict monotonic growth across composition steps
    for w in checkpoints.windows(2) {
        let (k1, eps1) = w[0];
        let (k2, eps2) = w[1];
        assert!(
            eps1 < eps2,
            "monotonicity violated: step {k1} eps={eps1} >= step {k2} eps={eps2}"
        );
    }
}

/// Stress-test tail bound conversion across failure probabilities delta from 1e-1 down to 1e-15.
/// Asserts strict monotonic convergence of epsilon(delta) as delta -> 0+.
#[test]
fn test_adv_dp_tail_bound_monotonic_convergence() {
    let budget = RdpBudget::from_gaussian(1.0, 4.0).unwrap();

    let deltas = [
        1e-1, 1e-2, 1e-3, 1e-4, 1e-5, 1e-6, 1e-8, 1e-10, 1e-12, 1e-15,
    ];

    let mut prev_eps = 0.0;
    for &delta in &deltas {
        let eps = budget.epsilon_at_delta(delta).unwrap();
        assert!(
            eps > prev_eps,
            "epsilon must strictly increase as delta decreases: delta={delta}, eps={eps}, prev_eps={prev_eps}"
        );
        prev_eps = eps;

        // Verify optimal order selection
        let (opt_alpha, opt_eps) = budget.optimal_order_for_delta(delta).unwrap();
        assert_eq!(eps, opt_eps);
        assert!(opt_alpha > 1.0);
    }
}

/// Empirical demonstration of Phase 9 bug:
/// Empirical verification of Phase 9 zero-budget tail bound fix:
/// `RdpBudget::zero()` has `is_empty() == true`, and `epsilon_at_delta` checks `self.is_empty()`,
/// returning exactly 0.0 for zero budgets.
/// This ensures small budgets (e.g. max_epsilon = 0.1) are not prematurely exhausted before any query.
#[test]
fn test_adv_dp_zero_budget_discretization_anomaly() {
    let zero_budget = RdpBudget::zero();

    // Invariant: RdpBudget::is_empty() correctly reports true
    assert!(zero_budget.is_empty(), "RdpBudget::zero() must be empty");

    // But self.values contains 16 keys with 0.0
    assert!(
        !zero_budget.values.is_empty(),
        "self.values contains 16 keys"
    );

    // epsilon_at_delta correctly returns 0.0 for zero budget via self.is_empty()
    let computed_eps = zero_budget.epsilon_at_delta(1e-5).unwrap();
    assert_eq!(
        computed_eps, 0.0,
        "computed_eps must be 0.0 for zero budget queries"
    );

    // Consequence on DistributedPrivacyTracker:
    // A tracker configured with global_max_epsilon = 0.1
    let tracker = DistributedPrivacyTracker::new(0.1, 1e-5).unwrap();

    // 1. Initial remaining epsilon is full budget 0.1 when 0 queries recorded
    let remaining = tracker.remaining_epsilon().unwrap();
    assert_eq!(remaining, 0.1, "remaining budget must be full max_epsilon");

    // 2. Initial utilization is 0.0 on a brand new tracker
    let util = tracker.budget_utilization().unwrap();
    assert_eq!(util, 0.0, "utilization must report 0%");

    // 3. can_admit_query admits zero-cost query
    let can_admit_zero = tracker.can_admit_query(&RdpBudget::zero()).unwrap();
    assert!(can_admit_zero, "can_admit_query admits zero budget queries");
}

/// Adversarial stress-test: multi-agent concurrent reservation and commit/rollback cycles.
/// Simulates 8 concurrent agents competing for a shared finite privacy budget.
/// Asserts no race condition allows overcommitting beyond global_max_epsilon.
#[test]
fn test_adv_dp_multi_agent_concurrent_reservation_saturation() {
    let global_max_eps = 5.0;
    let global_target_delta = 1e-5;
    let tracker = Arc::new(Mutex::new(
        DistributedPrivacyTracker::new(global_max_eps, global_target_delta).unwrap(),
    ));

    let num_agents = 8;
    let rounds_per_agent = 30;
    let mut handles = Vec::new();

    for agent_idx in 0..num_agents {
        let tracker_clone = Arc::clone(&tracker);
        let agent_id = format!("agent_{}", agent_idx);

        let handle = thread::spawn(move || {
            let mut prng = SafePrng::new(1000 + agent_idx as u64);
            let mut committed_count = 0;
            let mut released_count = 0;
            let mut rejected_count = 0;

            for round in 0..rounds_per_agent {
                let query_id = format!("q_{}_{}", agent_idx, round);
                let query_budget = RdpBudget::from_gaussian(0.2, 5.0).unwrap();

                // 1. Try to reserve budget
                let reservation_res = {
                    let mut lock = tracker_clone.lock().unwrap();
                    lock.reserve(&agent_id, &query_id, &query_budget)
                };

                match reservation_res {
                    Ok(res_id) => {
                        // Decide whether to commit or rollback
                        let do_commit = prng.sample_uniform_f64() > 0.3; // 70% commit, 30% rollback

                        if do_commit {
                            let mut lock = tracker_clone.lock().unwrap();
                            let commit_res = lock.commit_reservation(&res_id);
                            assert!(
                                commit_res.is_ok(),
                                "commit should succeed for active reservation"
                            );
                            committed_count += 1;
                        } else {
                            let mut lock = tracker_clone.lock().unwrap();
                            let release_res = lock.release_reservation(&res_id);
                            assert!(
                                release_res.is_ok(),
                                "release should succeed for active reservation"
                            );
                            released_count += 1;
                        }
                    }
                    Err(PrivacyAccountantError::BudgetExceeded { .. }) => {
                        // Correctly throttled when budget exhausted
                        rejected_count += 1;
                    }
                    Err(other) => panic!("unexpected error during reservation: {:?}", other),
                }
            }

            (committed_count, released_count, rejected_count)
        });

        handles.push(handle);
    }

    let mut total_committed = 0;
    let mut _total_released = 0;
    let mut _total_rejected = 0;

    for h in handles {
        let (c, r, rej) = h.join().unwrap();
        total_committed += c;
        _total_released += r;
        _total_rejected += rej;
    }

    let final_tracker = tracker.lock().unwrap();
    let current_dp = final_tracker.current_approx_dp().unwrap();

    // 1. Invariant: cumulative epsilon must NEVER exceed global_max_eps
    assert!(
        current_dp.0 <= global_max_eps + 1e-9,
        "privacy budget invariant breached! current={} > limit={}",
        current_dp.0,
        global_max_eps
    );

    // 2. Invariant: total allocated (cumulative + active reservations) <= global_max_eps
    let total_alloc_eps = final_tracker
        .total_allocated_rdp()
        .epsilon_at_delta(global_target_delta)
        .unwrap();
    assert!(
        total_alloc_eps <= global_max_eps + 1e-9,
        "total allocated budget exceeded limit: {total_alloc_eps} > {global_max_eps}"
    );

    // 3. Ledger length must match total committed
    assert_eq!(
        final_tracker.ledger.len(),
        total_committed,
        "ledger length does not match total committed queries"
    );

    // 4. Budget utilization must be in [0, 1]
    let util = final_tracker.budget_utilization().unwrap();
    assert!((0.0..=1.0).contains(&util));
}

/// Test reservation lifecycle attack vectors:
/// double-commit, commit non-existent, release non-existent, double-release,
/// overcommit rejection, and budget restoration after release.
#[test]
fn test_adv_dp_reservation_lifecycle_attacks() {
    let mut tracker = DistributedPrivacyTracker::new(5.0, 1e-5).unwrap();
    let query_budget = RdpBudget::from_gaussian(0.5, 4.0).unwrap();

    // 1. Valid reservation
    let res_id = tracker.reserve("agent_1", "q1", &query_budget).unwrap();

    // 2. Commit once -> OK
    assert!(tracker.commit_reservation(&res_id).is_ok());

    // 3. Double-commit attack -> must fail with ReservationAlreadyCommitted
    let double_commit = tracker.commit_reservation(&res_id);
    assert!(
        matches!(
            double_commit,
            Err(PrivacyAccountantError::ReservationAlreadyCommitted(_))
        ),
        "second commit on same reservation must fail with ReservationAlreadyCommitted"
    );

    // 4. Commit non-existent reservation
    let fake_commit = tracker.commit_reservation("res_non_existent_token");
    assert!(matches!(
        fake_commit,
        Err(PrivacyAccountantError::ReservationNotFound(_))
    ));

    // 5. Release non-existent reservation
    let fake_release = tracker.release_reservation("res_non_existent_token");
    assert!(matches!(
        fake_release,
        Err(PrivacyAccountantError::ReservationNotFound(_))
    ));

    // 6. Overcommit rejection and rollback recovery
    // Tracker has limit 5.0. Reserve a query that uses a substantial part of the budget.
    let big_query = RdpBudget::from_gaussian(1.5, 2.0).unwrap(); // eps ~2.5
    let res_big = tracker.reserve("agent_2", "q_big", &big_query).unwrap();

    // Now another massive query that would push past 5.0 must be rejected
    let extra_query = RdpBudget::from_gaussian(4.0, 1.0).unwrap(); // eps ~15.0
    let rej = tracker.reserve("agent_3", "q_extra", &extra_query);
    assert!(
        matches!(rej, Err(PrivacyAccountantError::BudgetExceeded { .. })),
        "query exceeding budget must be rejected"
    );

    // Release the big in-flight reservation
    tracker.release_reservation(&res_big).unwrap();

    // Double-release of res_big must now fail
    assert!(matches!(
        tracker.release_reservation(&res_big),
        Err(PrivacyAccountantError::ReservationNotFound(_))
    ));

    // Now a moderate query should be admitted because budget was restored!
    let admitted = tracker.reserve("agent_3", "q_admitted", &query_budget);
    assert!(
        admitted.is_ok(),
        "query should be admitted after releasing previous in-flight reservation"
    );
}

/// Stress-test Gaussian and Laplace noise sampling for statistical properties
/// (sample mean and variance within 3-sigma error margin).
#[test]
fn test_adv_dp_safe_noise_generation_statistical_properties() {
    let mut rng = SafePrng::new(424242);
    let sample_count = 10_000;

    // 1. Gaussian Mechanism: sigma = 2.5 => variance = 6.25
    let sigma = 2.5;
    let g_mech = GaussianMechanism::new(1.0, sigma).unwrap();
    let mut g_samples = Vec::with_capacity(sample_count);
    for _ in 0..sample_count {
        g_samples.push(g_mech.add_noise(0.0, &mut rng));
    }

    let g_mean = g_samples.iter().sum::<f64>() / (sample_count as f64);
    let g_var =
        g_samples.iter().map(|&x| (x - g_mean).powi(2)).sum::<f64>() / ((sample_count - 1) as f64);

    // Standard error of mean = sigma / sqrt(N) = 2.5 / 100 = 0.025. 3-sigma = 0.075
    assert!(
        g_mean.abs() < 0.1,
        "Gaussian sample mean {g_mean} deviates too much from 0.0"
    );
    // Relative variance error < 10%
    let g_var_expected = sigma * sigma;
    assert!(
        (g_var - g_var_expected).abs() / g_var_expected < 0.10,
        "Gaussian sample variance {g_var} deviates too much from expected {g_var_expected}"
    );

    // 2. Laplace Mechanism: scale b = 1.8 => variance = 2 * b^2 = 6.48
    let b = 1.8;
    let l_mech = LaplaceMechanism::new(1.0, b).unwrap();
    let mut l_samples = Vec::with_capacity(sample_count);
    for _ in 0..sample_count {
        l_samples.push(l_mech.add_noise(0.0, &mut rng));
    }

    let l_mean = l_samples.iter().sum::<f64>() / (sample_count as f64);
    let l_var =
        l_samples.iter().map(|&x| (x - l_mean).powi(2)).sum::<f64>() / ((sample_count - 1) as f64);

    assert!(
        l_mean.abs() < 0.1,
        "Laplace sample mean {l_mean} deviates too much from 0.0"
    );
    let l_var_expected = 2.0 * b * b;
    assert!(
        (l_var - l_var_expected).abs() / l_var_expected < 0.10,
        "Laplace sample variance {l_var} deviates too much from expected {g_var_expected}"
    );
}

// ============================================================================
// PART 2: PHASE 10 ZERO-KNOWLEDGE STATE ATTESTATION STRESS & SOUNDNESS
// ============================================================================

/// Stress-test Goldilocks field arithmetic under extreme boundary conditions:
/// u64::MAX, p - 1, p, addition wrapping, subtraction underflow, and multiplication overflow.
#[test]
fn test_adv_zk_goldilocks_boundary_arithmetic() {
    let p = GOLDILOCKS_PRIME; // 2^64 - 2^32 + 1 = 0xFFFF_FFFF_0000_0001
    let p_minus_1 = FieldElement::new(p - 1);
    let zero = FieldElement::ZERO;
    let one = FieldElement::ONE;

    // 1. Reduction of u64::MAX = 2^64 - 1
    // 2^64 - 1 = p + 2^32 - 2 => (2^64 - 1) mod p = 2^32 - 2
    let u64_max = u64::MAX;
    let red_max = FieldElement::new(u64_max);
    assert_eq!(
        red_max.value(),
        (1u64 << 32) - 2,
        "reduction of u64::MAX mod p mismatch"
    );

    // Reduction of p mod p must be 0
    let red_p = FieldElement::new(p);
    assert_eq!(red_p, zero, "p mod p must be zero");

    // 2. Addition boundaries
    assert_eq!(p_minus_1.add(one), zero, "(p-1) + 1 == 0 mod p");
    assert_eq!(
        p_minus_1.add(FieldElement::new(2)),
        one,
        "(p-1) + 2 == 1 mod p"
    );
    assert_eq!(
        p_minus_1.add(p_minus_1),
        FieldElement::new(p - 2),
        "(p-1) + (p-1) == p - 2 mod p"
    );

    // 3. Subtraction underflow
    assert_eq!(zero.sub(one), p_minus_1, "0 - 1 == p - 1 mod p");
    assert_eq!(zero.sub(p_minus_1), one, "0 - (p - 1) == 1 mod p");
    assert_eq!(
        one.sub(FieldElement::new(2)),
        p_minus_1,
        "1 - 2 == p - 1 mod p"
    );

    // 4. Multiplication boundaries
    // (-1) * (-1) == 1 mod p
    assert_eq!(p_minus_1.mul(p_minus_1), one, "(-1) * (-1) == 1 mod p");
    // (-1) * 2 == p - 2 mod p
    assert_eq!(
        p_minus_1.mul(FieldElement::new(2)),
        FieldElement::new(p - 2)
    );

    // Multiplication of 2^32 * 2^32:
    // (2^32)^2 = 2^64 = p + 2^32 - 1 => (2^32)^2 mod p = 2^32 - 1
    let two_32 = FieldElement::new(1u64 << 32);
    let two_32_sq = two_32.mul(two_32);
    assert_eq!(
        two_32_sq.value(),
        (1u64 << 32) - 1,
        "(2^32)^2 mod p must equal 2^32 - 1"
    );

    // 5. Inverses of boundary elements
    assert_eq!(one.inv(), Some(one), "1^-1 == 1");
    assert_eq!(p_minus_1.inv(), Some(p_minus_1), "(-1)^-1 == -1");
    assert_eq!(zero.inv(), None, "0^-1 must be None");

    // Invert two_32
    let two_32_inv = two_32.inv().expect("inversion should succeed");
    assert_eq!(two_32.mul(two_32_inv), one, "two_32 * two_32_inv == 1");
}

/// Stress-test modular exponentiation and Fermat's Little Theorem across extreme elements.
#[test]
fn test_adv_zk_powers_and_fermat_little_theorem() {
    let p = GOLDILOCKS_PRIME;
    let one = FieldElement::ONE;
    let zero = FieldElement::ZERO;

    // Powers of zero
    assert_eq!(zero.pow(0), one, "0^0 == 1 by convention");
    assert_eq!(zero.pow(1), zero, "0^1 == 0");
    assert_eq!(zero.pow(100), zero, "0^100 == 0");

    // Powers of one
    assert_eq!(one.pow(0), one);
    assert_eq!(one.pow(12345), one);

    // Fermat's Little Theorem: a^(p-1) == 1 mod p for all a != 0
    let test_elements = [
        FieldElement::new(2),
        FieldElement::new(3),
        FieldElement::new(7),
        FieldElement::new(1337),
        FieldElement::new(0x1_0000_0000),
        FieldElement::new(p - 2),
        FieldElement::new(p - 1),
    ];

    for &a in &test_elements {
        let flt = a.pow(p - 1);
        assert_eq!(flt, one, "Fermat's Little Theorem breached for a={:?}", a);
    }

    // Power homomorphism: (a * b)^k == a^k * b^k
    let a = FieldElement::new(12345);
    let b = FieldElement::new(67890);
    let k = 17u64;
    assert_eq!(
        a.mul(b).pow(k),
        a.pow(k).mul(b.pow(k)),
        "homomorphism (ab)^k == a^k b^k failed"
    );
}

/// Stress-test roots of unity generation, non-power-of-two rejection,
/// two-adicity limits, and discrete Fourier orthogonality.
#[test]
fn test_adv_zk_roots_of_unity_and_orthogonality() {
    // 1. Non-power-of-two domain sizes must be strictly rejected
    let invalid_sizes = [0, 1, 3, 5, 6, 7, 9, 10, 12, 15, 33, 100, 1000];
    for &size in &invalid_sizes {
        let res = FieldElement::primitive_root_of_unity(size);
        assert!(
            matches!(res, Err(ZkAttestationError::InvalidDomainSize { .. })),
            "size {size} must return InvalidDomainSize"
        );
    }

    // 2. Exceeding two-adicity exponent (GOLDILOCKS_TWO_ADICITY = 32)
    // Order 2^33 cannot exist in Goldilocks field
    let huge_order = 1usize << 33; // 2^33
    if std::mem::size_of::<usize>() >= 8 {
        let res = FieldElement::primitive_root_of_unity(huge_order);
        assert!(
            matches!(res, Err(ZkAttestationError::InvalidDomainSize { .. })),
            "order 2^33 must exceed two-adicity"
        );
    }

    // 3. Valid powers of two orders k = 2^1 .. 2^10
    for m in 1..=10 {
        let k = 1usize << m;
        let omega = FieldElement::primitive_root_of_unity(k).unwrap();

        // Property 1: omega^k == 1
        assert_eq!(omega.pow(k as u64), FieldElement::ONE);

        // Property 2: omega^(k/2) == p - 1 == -1
        assert_eq!(
            omega.pow((k / 2) as u64),
            FieldElement::new(GOLDILOCKS_PRIME - 1)
        );

        // Property 3: Orthogonality sum_{j=0}^{k-1} omega^j == 0 for k >= 2
        let mut sum = FieldElement::ZERO;
        let mut curr = FieldElement::ONE;
        for _ in 0..k {
            sum = sum.add(curr);
            curr = curr.mul(omega);
        }
        assert_eq!(
            sum,
            FieldElement::ZERO,
            "orthogonality failed for domain size k={k}"
        );
    }
}

/// Stress-test polynomial operations: zero polynomial, div_rem by zero,
/// higher-degree divisor, vanishing polynomial roots, and interpolation edge cases.
#[test]
fn test_adv_zk_polynomial_algebraic_singularities() {
    let zero_poly = Polynomial::zero();
    let p = FieldElement::new(42);
    let const_poly = Polynomial::constant(p);

    // 1. Zero polynomial properties
    assert!(zero_poly.is_zero());
    assert_eq!(zero_poly.degree(), 0);
    assert_eq!(zero_poly.lead_coeff(), FieldElement::ZERO);
    assert_eq!(zero_poly.eval(FieldElement::new(100)), FieldElement::ZERO);

    // 2. Division by zero polynomial must return DivisionByZero error
    let div_zero_res = const_poly.div_rem(&zero_poly);
    assert!(matches!(
        div_zero_res,
        Err(ZkAttestationError::DivisionByZero)
    ));

    // 3. Division of zero polynomial by non-zero polynomial
    let (q, r) = zero_poly.div_rem(&const_poly).unwrap();
    assert!(q.is_zero());
    assert!(r.is_zero());

    // 4. Division where dividend degree < divisor degree
    // P = 3X + 2, D = X^2 + 1 => Q = 0, R = 3X + 2
    let dividend = Polynomial::new(vec![FieldElement::new(2), FieldElement::new(3)]);
    let divisor = Polynomial::new(vec![
        FieldElement::ONE,
        FieldElement::ZERO,
        FieldElement::ONE,
    ]);
    let (q2, r2) = dividend.div_rem(&divisor).unwrap();
    assert!(
        q2.is_zero(),
        "quotient must be zero when deg(dividend) < deg(divisor)"
    );
    assert_eq!(r2, dividend, "remainder must equal dividend");

    // 5. Vanishing polynomial Z_H(X) = X^k - 1 over domain H
    let k = 8;
    let z_h = Polynomial::vanishing_poly(k);
    assert_eq!(z_h.degree(), k);

    let omega = FieldElement::primitive_root_of_unity(k).unwrap();
    let mut root = FieldElement::ONE;
    for idx in 0..k {
        let eval = z_h.eval(root);
        assert_eq!(
            eval,
            FieldElement::ZERO,
            "vanishing polynomial must evaluate to 0 at root {idx}"
        );
        root = root.mul(omega);
    }

    // Evaluating Z_H(X) at a non-root element (e.g. 2) must be non-zero: 2^8 - 1 = 255 != 0
    let non_root_eval = z_h.eval(FieldElement::new(2));
    assert_eq!(non_root_eval, FieldElement::new(255));

    // 6. Lagrange interpolation failure modes
    // Mismatched lengths
    let xs_bad = vec![FieldElement::ONE, FieldElement::new(2)];
    let ys_bad = vec![FieldElement::new(5)];
    assert!(matches!(
        Polynomial::from_evaluations(&xs_bad, &ys_bad),
        Err(ZkAttestationError::LengthMismatch { .. })
    ));

    // Empty vectors
    let empty_xs: Vec<FieldElement> = Vec::new();
    let empty_ys: Vec<FieldElement> = Vec::new();
    assert!(matches!(
        Polynomial::from_evaluations(&empty_xs, &empty_ys),
        Err(ZkAttestationError::LengthMismatch { .. })
    ));

    // Duplicate evaluation points
    let xs_dup = vec![FieldElement::new(5), FieldElement::new(5)];
    let ys_dup = vec![FieldElement::new(10), FieldElement::new(20)];
    assert!(matches!(
        Polynomial::from_evaluations(&xs_dup, &ys_dup),
        Err(ZkAttestationError::InterpolationError { .. })
    ));
}

/// Comprehensive adversarial forgery matrix for Zero-Knowledge State Attestation.
/// Generates a valid proof and systematically mutates every single component
/// (pre-state, post-state, action, quotient, challenge, commitments).
/// Asserts that `verify` and `verify_transition` strictly return `Ok(false)` for every mutation.
#[test]
fn test_adv_zk_attestation_comprehensive_forgery_matrix() {
    let domain_size = 16;
    let air = StateTransitionAir::new(domain_size).unwrap();

    let mut pre_state = Vec::with_capacity(domain_size);
    let mut action = Vec::with_capacity(domain_size);
    let mut post_state = Vec::with_capacity(domain_size);

    for i in 0..domain_size {
        let s = FieldElement::new(100 + i as u64);
        let a = FieldElement::new(10 + (i * 2) as u64);
        pre_state.push(s);
        action.push(a);
        post_state.push(s.add(a));
    }

    // 1. Generate legitimate proof
    let valid_proof = air
        .prove_transition(&pre_state, &action, &post_state)
        .expect("valid proof generation");

    // Succinct size <= 176 bytes
    assert!(
        valid_proof.proof_size_bytes() <= 176,
        "proof size must be <= 176 bytes"
    );

    // Legitimate proof passes verification
    assert!(
        air.verify_transition(&valid_proof).unwrap(),
        "legitimate proof must verify true"
    );
    assert!(
        valid_proof.verify_succinct(),
        "verify_succinct must succeed for valid proof"
    );

    // 2. Attack Vector 1: Tampered evaluation y_S (pre-state evaluation)
    let mut forged_1 = valid_proof.clone();
    forged_1.eval_pre_state = forged_1.eval_pre_state.add(FieldElement::ONE);
    assert!(
        !air.verify_transition(&forged_1).unwrap(),
        "tampered eval_pre_state must fail verification"
    );

    // 3. Attack Vector 2: Tampered evaluation y_{S'} (post-state evaluation)
    let mut forged_2 = valid_proof.clone();
    forged_2.eval_post_state = forged_2.eval_post_state.add(FieldElement::ONE);
    assert!(
        !air.verify_transition(&forged_2).unwrap(),
        "tampered eval_post_state must fail verification"
    );

    // 4. Attack Vector 3: Tampered evaluation y_A (action evaluation)
    let mut forged_3 = valid_proof.clone();
    forged_3.eval_action = forged_3.eval_action.add(FieldElement::ONE);
    assert!(
        !air.verify_transition(&forged_3).unwrap(),
        "tampered eval_action must fail verification"
    );

    // 5. Attack Vector 4: Tampered evaluation y_Q (quotient evaluation)
    let mut forged_4 = valid_proof.clone();
    forged_4.eval_quotient = forged_4.eval_quotient.add(FieldElement::ONE);
    assert!(
        !air.verify_transition(&forged_4).unwrap(),
        "tampered eval_quotient must fail verification"
    );

    // 6. Attack Vector 5: Tampered challenge zeta
    let mut forged_5 = valid_proof.clone();
    forged_5.challenge = forged_5.challenge.add(FieldElement::ONE);
    assert!(
        !air.verify_transition(&forged_5).unwrap(),
        "tampered challenge must fail verification"
    );

    // 7. Attack Vector 6: Tampered pre-state commitment
    let mut forged_6 = valid_proof.clone();
    forged_6.pre_state_commitment[0] ^= 0x55;
    assert!(
        !air.verify_transition(&forged_6).unwrap(),
        "tampered pre_state_commitment must fail verification"
    );

    // 8. Attack Vector 7: Tampered post-state commitment
    let mut forged_7 = valid_proof.clone();
    forged_7.post_state_commitment[15] ^= 0xAA;
    assert!(
        !air.verify_transition(&forged_7).unwrap(),
        "tampered post_state_commitment must fail verification"
    );

    // 9. Attack Vector 8: Tampered action commitment
    let mut forged_8 = valid_proof.clone();
    forged_8.action_commitment[31] ^= 0xFF;
    assert!(
        !air.verify_transition(&forged_8).unwrap(),
        "tampered action_commitment must fail verification"
    );

    // 10. Attack Vector 9: Tampered quotient commitment
    let mut forged_9 = valid_proof.clone();
    forged_9.quotient_commitment[0] ^= 0x01;
    assert!(
        !air.verify_transition(&forged_9).unwrap(),
        "tampered quotient_commitment must fail verification"
    );

    // 11. Attack Vector 10: Caller root override mismatches
    let bogus_hash = [0xEE; 32];
    assert!(
        !valid_proof.verify(Some(bogus_hash), None).unwrap(),
        "mismatched expected pre-state root must fail"
    );
    assert!(
        !valid_proof.verify(None, Some(bogus_hash)).unwrap(),
        "mismatched expected post-state root must fail"
    );

    // 12. Attack Vector 11: Domain size mismatch
    let air_wrong_domain = StateTransitionAir::new(8).unwrap();
    assert!(
        !air_wrong_domain.verify_transition(&valid_proof).unwrap(),
        "proof from domain size 16 must fail verification on domain size 8"
    );
}

/// Stress-test prover detection of illegal state transitions:
/// single coordinate violations, total state corruption, and vector length mismatches.
#[test]
fn test_adv_zk_illegal_transition_rejection_at_prover() {
    let domain_size = 8;
    let air = StateTransitionAir::new(domain_size).unwrap();

    let pre_state = vec![FieldElement::new(10); domain_size];
    let action = vec![FieldElement::new(5); domain_size];

    // 1. Single coordinate violation (16 instead of 15)
    let mut corrupted_post = vec![FieldElement::new(15); domain_size];
    corrupted_post[0] = FieldElement::new(16);

    let res1 = air.prove_transition(&pre_state, &action, &corrupted_post);
    assert!(matches!(
        res1,
        Err(ZkAttestationError::InvalidTransitionConstraint { .. })
    ));

    // 2. Completely zeroed post-state
    let zero_post = vec![FieldElement::ZERO; domain_size];
    let res2 = air.prove_transition(&pre_state, &action, &zero_post);
    assert!(matches!(
        res2,
        Err(ZkAttestationError::InvalidTransitionConstraint { .. })
    ));

    // 3. Length mismatches
    let short_pre = vec![FieldElement::new(10); domain_size - 1];
    assert!(matches!(
        air.prove_transition(&short_pre, &action, &corrupted_post),
        Err(ZkAttestationError::LengthMismatch { .. })
    ));

    let long_action = vec![FieldElement::new(5); domain_size + 1];
    assert!(matches!(
        air.prove_transition(&pre_state, &long_action, &corrupted_post),
        Err(ZkAttestationError::LengthMismatch { .. })
    ));
}

/// Stress-test multi-step cryptographic state transition chain:
/// S_0 -> S_1 -> S_2 -> ... -> S_10.
/// Verifies that sequential commitments bind each step securely and that
/// tampering with an intermediate link in the chain is strictly detected.
#[test]
fn test_adv_zk_multi_step_state_chain_verification() {
    let domain_size = 8;
    let air = StateTransitionAir::new(domain_size).unwrap();
    let num_steps = 10;

    let mut current_state = vec![FieldElement::new(100); domain_size];
    let mut proofs = Vec::with_capacity(num_steps);

    for step in 0..num_steps {
        let action = vec![FieldElement::new(step as u64 + 1); domain_size];
        let next_state: Vec<FieldElement> = current_state
            .iter()
            .zip(action.iter())
            .map(|(&s, &a)| s.add(a))
            .collect();

        let proof = air
            .prove_transition(&current_state, &action, &next_state)
            .unwrap();
        proofs.push(proof);
        current_state = next_state;
    }

    // 1. Verify entire valid chain
    for i in 0..num_steps {
        let valid = air.verify_transition(&proofs[i]).unwrap();
        assert!(valid, "step {i} proof must verify");

        if i > 0 {
            // Verify cryptographic continuity: previous post-state == current pre-state
            assert_eq!(
                proofs[i - 1].post_state_commitment,
                proofs[i].pre_state_commitment,
                "cryptographic chain continuity broken between step {} and {}",
                i - 1,
                i
            );
        }
    }

    // 2. Adversary attempts to replay or reorder transitions (swap step 2 and step 5)
    let reordered_step_5_pre = proofs[5].pre_state_commitment;
    let step_2_post = proofs[2].post_state_commitment;
    assert_ne!(
        step_2_post, reordered_step_5_pre,
        "reordered transitions must have mismatched commitments"
    );

    // Attempting to verify step 5 using step 2's post-state as expected pre-state must fail
    let replay_verify = proofs[5].verify(Some(step_2_post), None).unwrap();
    assert!(
        !replay_verify,
        "chain verification must reject reordered transitions"
    );
}

/// Stress-test Fiat-Shamir transcript entropy, domain separation, and forward secrecy.
#[test]
fn test_adv_fiat_shamir_entropy_and_forward_secrecy() {
    let mut transcript1 = FiatShamirTranscript::new("test_domain");
    let mut transcript2 = FiatShamirTranscript::new("test_domain");

    // Identical operations on identical transcripts produce identical challenges
    transcript1.append_message("agent", b"agent_007");
    transcript2.append_message("agent", b"agent_007");

    let ch1_a = transcript1.challenge_element("zeta");
    let ch2_a = transcript2.challenge_element("zeta");
    assert_eq!(ch1_a, ch2_a, "deterministic transcript must match");

    // Forward secrecy: successive calls on same transcript produce distinct challenges
    let ch1_b = transcript1.challenge_element("zeta");
    assert_ne!(
        ch1_a, ch1_b,
        "consecutive challenges must be distinct (forward secrecy)"
    );

    // Avalanche effect: 1-bit difference in input produces completely different challenge
    let mut transcript_diff = FiatShamirTranscript::new("test_domain");
    transcript_diff.append_message("agent", b"agent_008"); // 1 character different
    let ch_diff = transcript_diff.challenge_element("zeta");
    assert_ne!(
        ch1_a, ch_diff,
        "differing inputs must produce different challenge elements"
    );
}
