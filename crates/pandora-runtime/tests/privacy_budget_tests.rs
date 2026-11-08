#![forbid(unsafe_code)]

use pandora_runtime::bandit_router::SafePrng;
use pandora_runtime::privacy_budget::{
    DistributedPrivacyTracker, GaussianMechanism, LaplaceMechanism, PrivacyAccountantError,
    PrivacyExpenditure, RdpBudget, RdpOrder, STANDARD_RDP_ORDERS, order_to_key,
};

#[test]
fn test_rdp_order_validation() {
    assert!(RdpOrder::new(2.0).is_ok());
    assert!(RdpOrder::new(1.0).is_err());
    assert!(RdpOrder::new(0.5).is_err());
    assert!(RdpOrder::new(-2.0).is_err());
    assert!(RdpOrder::new(f64::NAN).is_err());
    assert!(RdpOrder::new(f64::INFINITY).is_err());

    let order = RdpOrder::new(2.5).unwrap();
    assert_eq!(order.value(), 2.5);
    assert_eq!(order.key(), 2500);
}

#[test]
fn test_gaussian_mechanism_closed_form() {
    // Delta = 1.0, sigma = 2.0
    // eps(alpha) = alpha * 1^2 / (2 * 4) = alpha / 8
    let mech = GaussianMechanism::new(1.0, 2.0).expect("valid parameters");
    assert!((mech.rdp_epsilon(2.0).unwrap() - 0.25).abs() < 1e-9);
    assert!((mech.rdp_epsilon(8.0).unwrap() - 1.00).abs() < 1e-9);
    assert!((mech.rdp_epsilon(16.0).unwrap() - 2.00).abs() < 1e-9);

    // Invalid parameters
    assert!(GaussianMechanism::new(0.0, 2.0).is_err());
    assert!(GaussianMechanism::new(-1.0, 2.0).is_err());
    assert!(GaussianMechanism::new(1.0, 0.0).is_err());
    assert!(GaussianMechanism::new(1.0, -1.0).is_err());
}

#[test]
fn test_laplace_mechanism_closed_form() {
    let mech = LaplaceMechanism::new(1.0, 1.0).expect("valid parameters");
    let eps2 = mech.rdp_epsilon(2.0).unwrap();
    // Formula for alpha=2, sensitivity=1, b=1:
    // eps(2) = ln( 2/3 * e^1 + 1/3 * e^(-2) ) = ln( 2e/3 + 1/(3e^2) )
    let expected = ((2.0 * std::f64::consts::E / 3.0)
        + (1.0 / (3.0 * std::f64::consts::E * std::f64::consts::E)))
        .ln();
    assert!((eps2 - expected).abs() < 1e-9);

    // Monotonicity in order
    let eps4 = mech.rdp_epsilon(4.0).unwrap();
    let eps8 = mech.rdp_epsilon(8.0).unwrap();
    assert!(eps2 < eps4);
    assert!(eps4 < eps8);

    // Invalid parameters
    assert!(LaplaceMechanism::new(0.0, 1.0).is_err());
    assert!(LaplaceMechanism::new(1.0, 0.0).is_err());
}

#[test]
fn test_rdp_linear_composition_theorem_17() {
    // Mechanism 1: Gaussian with Delta=1, sigma=2 => eps(alpha) = alpha / 8
    let b1 = RdpBudget::from_gaussian(1.0, 2.0).unwrap();
    // Mechanism 2: Gaussian with Delta=2, sigma=2 => eps(alpha) = alpha * 4 / 8 = alpha / 2
    let b2 = RdpBudget::from_gaussian(2.0, 2.0).unwrap();

    let composite = b1.compose(&b2).unwrap();

    for &alpha in &STANDARD_RDP_ORDERS {
        let key = order_to_key(alpha);
        let e1 = b1.values.get(&key).copied().unwrap();
        let e2 = b2.values.get(&key).copied().unwrap();
        let ec = composite.values.get(&key).copied().unwrap();
        assert!(
            (ec - (e1 + e2)).abs() < 1e-9,
            "additivity failed for alpha={alpha}"
        );
    }
}

#[test]
fn test_optimal_approx_dp_conversion_theorem_18() {
    let k = 10;
    let sensitivity = 1.0;
    let sigma = 4.0;
    let delta = 1e-5;

    // Build k composed Gaussian mechanisms
    let single = RdpBudget::from_gaussian(sensitivity, sigma).unwrap();
    let mut composite = RdpBudget::zero();
    for _ in 0..k {
        composite.compose_in_place(&single);
    }

    let discrete_eps = composite.epsilon_at_delta(delta).unwrap();
    let (_opt_alpha, opt_eps) = composite.optimal_order_for_delta(delta).unwrap();
    assert_eq!(discrete_eps, opt_eps);

    // Compare against theoretical continuous optimum formula
    let (alpha_star, eps_star) =
        RdpBudget::continuous_gaussian_optimum(k, sensitivity, sigma, delta).unwrap();
    assert!(alpha_star > 1.0);
    assert!(eps_star > 0.0);

    // Discrete grid is an upper bound on continuous infimum, with gap <= 2%
    assert!(
        discrete_eps >= eps_star - 1e-9,
        "discrete infimum cannot be lower than continuous minimum"
    );
    let relative_gap = (discrete_eps - eps_star) / eps_star;
    assert!(
        relative_gap <= 0.02,
        "discretization gap {relative_gap} exceeded 2% bound (discrete={discrete_eps}, continuous={eps_star})"
    );
}

#[test]
fn test_distributed_tracker_admit_and_reject() {
    let max_eps = 1.5;
    let target_delta = 1e-5;
    let mut tracker = DistributedPrivacyTracker::new(max_eps, target_delta).unwrap();

    // Small query should be admitted
    let small_query = RdpBudget::from_gaussian(0.5, 4.0).unwrap();
    assert!(tracker.can_admit_query(&small_query).unwrap());

    // Record several small expenditures
    for i in 0..3 {
        let exp = PrivacyExpenditure {
            transaction_id: format!("tx_{i}"),
            agent_id: "agent_alpha".to_string(),
            query_id: format!("q_{i}"),
            mechanism_name: "gaussian".to_string(),
            sensitivity: 0.5,
            noise_parameter: 4.0,
            rdp_expenditure: small_query.clone(),
            timestamp_ms: i as u64,
        };
        let current_eps = tracker.record_expenditure(exp).unwrap();
        assert!(current_eps <= max_eps);
    }

    // Huge query that would exceed budget must be rejected
    let huge_query = RdpBudget::from_gaussian(5.0, 1.0).unwrap();
    assert!(!tracker.can_admit_query(&huge_query).unwrap());

    let exp_huge = PrivacyExpenditure {
        transaction_id: "tx_huge".to_string(),
        agent_id: "agent_beta".to_string(),
        query_id: "q_huge".to_string(),
        mechanism_name: "gaussian".to_string(),
        sensitivity: 5.0,
        noise_parameter: 1.0,
        rdp_expenditure: huge_query,
        timestamp_ms: 100,
    };
    match tracker.record_expenditure(exp_huge) {
        Err(PrivacyAccountantError::BudgetExceeded { projected, limit }) => {
            assert!(projected > limit);
            assert_eq!(limit, max_eps);
        }
        other => panic!("expected BudgetExceeded error, got: {:?}", other),
    }
}

#[test]
fn test_federated_reservation_lifecycle() {
    let mut tracker = DistributedPrivacyTracker::new(2.0, 1e-5).unwrap();
    let query_budget = RdpBudget::from_gaussian(1.0, 3.0).unwrap();

    // 1. Reserve budget
    let res_id = tracker
        .reserve("agent_1", "query_42", &query_budget)
        .expect("reservation should succeed");
    assert_eq!(tracker.active_reservations.len(), 1);

    // Active reservation is counted in can_admit_query
    let remaining_before = tracker.remaining_epsilon().unwrap();

    // 2. Rollback reservation (e.g. agent timed out or cancelled)
    tracker.release_reservation(&res_id).unwrap();
    assert_eq!(tracker.active_reservations.len(), 0);

    // Budget is intact and restored
    assert_eq!(tracker.cumulative_budget, RdpBudget::zero());
    let remaining_after_rollback = tracker.remaining_epsilon().unwrap();
    assert_eq!(remaining_before, remaining_after_rollback);

    // 3. New reservation and commit
    let res_id2 = tracker
        .reserve("agent_1", "query_43", &query_budget)
        .expect("second reservation should succeed");
    let committed_eps = tracker.commit_reservation(&res_id2).unwrap();
    assert!(committed_eps > 0.0);
    assert_eq!(tracker.active_reservations.len(), 0);
    assert_eq!(tracker.ledger.len(), 1);
    assert!(tracker.agent_budget("agent_1").is_some());
}

#[test]
fn test_safe_noise_generation() {
    let mut rng = SafePrng::new(1337);

    // Gaussian noise sampling
    let gauss = GaussianMechanism::new(1.0, 2.0).unwrap();
    let mut gauss_sum = 0.0;
    let n = 2000;
    for _ in 0..n {
        let val = gauss.add_noise(0.0, &mut rng);
        gauss_sum += val;
    }
    let mean = gauss_sum / (n as f64);
    assert!(
        mean.abs() < 0.15,
        "Gaussian perturbation mean drifted: {mean}"
    );

    // Laplace noise sampling
    let lap = LaplaceMechanism::new(1.0, 1.5).unwrap();
    let mut lap_sum = 0.0;
    for _ in 0..n {
        let val = lap.add_noise(0.0, &mut rng);
        lap_sum += val;
    }
    let lap_mean = lap_sum / (n as f64);
    assert!(
        lap_mean.abs() < 0.15,
        "Laplace perturbation mean drifted: {lap_mean}"
    );

    // Deterministic reproducibility
    let mut rng1 = SafePrng::new(42);
    let mut rng2 = SafePrng::new(42);
    let sample1 = gauss.add_noise(10.0, &mut rng1);
    let sample2 = gauss.add_noise(10.0, &mut rng2);
    assert_eq!(sample1, sample2);
}
