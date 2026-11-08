#![forbid(unsafe_code)]

//! Adversarial Stress & Empirical Challenge Suite: Phases 18 & 19 (Theorems 35–38)
//!
//! Conducts exhaustive empirical stress-testing on:
//! - Phase 18: Mean-Field Games & Price-of-Anarchy Bounding (`mean_field_game.rs`)
//! - Phase 19: PAC-Bayes Generalization & Policy Promotion Gate (`pac_bayes_monitor.rs`)

use pandora_runtime::mean_field_game::{
    MeanFieldGame, MfgError, MfgResourceConfig, THEORETICAL_POA_UPPER_BOUND, project_onto_simplex,
};
use pandora_runtime::pac_bayes_monitor::{
    PacBayesConfig, PacBayesError, PacBayesMonitor, PolicyPromotionVerdict, binary_kl,
};

// =========================================================================
// SECTION 1: PHASE 18 (MEAN-FIELD GAMES & PRICE-OF-ANARCHY BOUNDING)
// =========================================================================

#[test]
fn test_mfg_extreme_k1_and_invalid_parameter_rejection() {
    // 1. Single resource (K=1) must return InsufficientResources
    let k1_res = vec![MfgResourceConfig::new("r1", 1.0, 1.0).unwrap()];
    let res_k1 = MeanFieldGame::new(k1_res, 1.0);
    assert!(
        matches!(res_k1, Err(MfgError::InsufficientResources { count: 1 })),
        "K=1 must return InsufficientResources error"
    );

    // 2. Empty resources (K=0) must return InsufficientResources
    let res_k0 = MeanFieldGame::new(vec![], 1.0);
    assert!(
        matches!(res_k0, Err(MfgError::InsufficientResources { count: 0 })),
        "K=0 must return InsufficientResources error"
    );

    // 3. Non-positive temperature tau <= 0
    let two_res = vec![
        MfgResourceConfig::new("r1", 1.0, 1.0).unwrap(),
        MfgResourceConfig::new("r2", 1.0, 1.0).unwrap(),
    ];
    assert!(
        matches!(
            MeanFieldGame::new(two_res.clone(), 0.0),
            Err(MfgError::InvalidTemperature { .. })
        ),
        "tau = 0.0 must be rejected"
    );
    assert!(
        matches!(
            MeanFieldGame::new(two_res.clone(), -2.5),
            Err(MfgError::InvalidTemperature { .. })
        ),
        "tau < 0.0 must be rejected"
    );
    assert!(
        matches!(
            MeanFieldGame::new(two_res.clone(), f64::NAN),
            Err(MfgError::InvalidTemperature { .. })
        ),
        "tau = NaN must be rejected"
    );

    // 4. Invalid base cost or congestion slope
    assert!(MfgResourceConfig::new("bad_base", -1.0, 1.0).is_err());
    assert!(MfgResourceConfig::new("bad_slope", 1.0, -0.5).is_err());
    assert!(MfgResourceConfig::new("nan_base", f64::NAN, 1.0).is_err());
    assert!(MfgResourceConfig::new("nan_slope", 1.0, f64::NAN).is_err());
}

#[test]
fn test_mfg_k100_massive_swarm_and_simplex_invariance() {
    // 100 competing resources with heterogeneous uncongested costs and congestion slopes
    let mut resources = Vec::with_capacity(100);
    for i in 0..100 {
        let base_cost = 0.1 * (i as f64);
        let slope = 0.5 + 0.1 * ((i % 10) as f64);
        resources.push(MfgResourceConfig::new(format!("gpu_node_{i}"), base_cost, slope).unwrap());
    }

    // Set tau high enough for contraction: beta_max = 1.4 => tau = 2.0 => kappa = 1.4 / 4 = 0.35
    let game = MeanFieldGame::new(resources, 2.0).expect("100-resource MFG creation");
    assert_eq!(game.num_resources(), 100);
    assert!(game.is_strict_contraction());

    let eq = game
        .compute_equilibrium(500, 1e-8)
        .expect("100-resource convergence");

    assert_eq!(eq.equilibrium_distribution.len(), 100);

    // Simplex Invariant: sum(m_i) == 1.0 and each m_i >= 0
    let mut sum = 0.0;
    for &m_i in &eq.equilibrium_distribution {
        assert!(m_i >= 0.0, "distribution element must be non-negative");
        sum += m_i;
    }
    assert!(
        (sum - 1.0).abs() < 1e-10,
        "100-resource simplex sum deviation: {}",
        (sum - 1.0).abs()
    );

    // Verify fixed-point response identity Phi(m*) == m*
    let phi = game
        .best_response(&eq.equilibrium_distribution)
        .expect("best response");
    let mut l1_error = 0.0;
    for (&m_i, &phi_i) in eq.equilibrium_distribution.iter().zip(phi.iter()) {
        l1_error += (m_i - phi_i).abs();
    }
    assert!(
        l1_error < 1e-7,
        "Fixed point residual error too large: {l1_error}"
    );
}

#[test]
fn test_mfg_extreme_high_congestion_slopes() {
    // Very high congestion slopes: beta = 10,000
    let resources = vec![
        MfgResourceConfig::new("r1", 1.0, 10_000.0).unwrap(),
        MfgResourceConfig::new("r2", 2.0, 10_000.0).unwrap(),
    ];

    // High beta with moderate tau => beta_max / (2 * tau) = 10000 / 20 = 500 >> 1
    // Engine automatically damps alpha: (2 * 10 / (2 * 10 + 10000)) = 20 / 10020 ~ 0.001996 -> clamped to 0.05
    let game = MeanFieldGame::new(resources, 10.0).expect("high congestion game");
    assert!(!game.is_strict_contraction());

    // Run computation: check that damping keeps iteration stable (no NaNs or infinities)
    let eq = game.compute_equilibrium(1000, 1e-5);
    match eq {
        Ok(res) => {
            let sum: f64 = res.equilibrium_distribution.iter().sum();
            assert!((sum - 1.0).abs() < 1e-6);
            for &x in &res.equilibrium_distribution {
                assert!((0.0..=1.0).contains(&x));
                assert!(!x.is_nan());
            }
        }
        Err(MfgError::ConvergenceFailed {
            residual,
            iterations,
        }) => {
            // When kappa >> 1, convergence failure is mathematically legitimate
            assert!(iterations >= 1000);
            assert!(!residual.is_nan());
        }
        Err(e) => panic!("Unexpected error: {e}"),
    }
}

#[test]
fn test_mfg_extreme_temperatures_zero_and_infinite() {
    let resources = vec![
        MfgResourceConfig::new("cheap", 1.0, 1.0).unwrap(),
        MfgResourceConfig::new("expensive", 5.0, 1.0).unwrap(),
    ];

    // 1. Extreme Low Temperature tau = 1e-6 (near zero entropic noise)
    // Max-subtraction log-domain stabilization must prevent underflow/overflow/NaN
    let low_temp_game = MeanFieldGame::with_damping(resources.clone(), 1e-6, 0.01).unwrap();
    let uniform = vec![0.5, 0.5];
    let resp_low = low_temp_game.best_response(&uniform).expect("resp low");
    assert!(
        (resp_low.iter().sum::<f64>() - 1.0).abs() < 1e-8,
        "Low temp response must sum to 1.0"
    );
    // Cheap resource cost: 1.0 + 0.5 = 1.5; Expensive resource cost: 5.0 + 0.5 = 5.5
    // Under tau = 1e-6, cheap resource gets 100% of mass
    assert!(
        resp_low[0] > 0.99999,
        "Low temp should allocate essentially all mass to cheaper resource: got {}",
        resp_low[0]
    );
    assert!(resp_low[1] < 1e-5);

    // 2. Extreme High Temperature tau = 1e8 (entropy dominates, costs negligible)
    let high_temp_game = MeanFieldGame::new(resources, 1e8).unwrap();
    let resp_high = high_temp_game.best_response(&uniform).expect("resp high");
    // Under tau -> infinity, best response must converge to exact uniform distribution 0.5
    assert!(
        (resp_high[0] - 0.5).abs() < 1e-6,
        "High temp must yield uniform distribution: got {}",
        resp_high[0]
    );
    assert!(
        (resp_high[1] - 0.5).abs() < 1e-6,
        "High temp must yield uniform distribution: got {}",
        resp_high[1]
    );
}

#[test]
fn test_simplex_projection_fuzz_and_pathological_inputs() {
    // 1. Degenerate empty input
    assert!(project_onto_simplex(&[]).is_empty());

    // 2. Single element
    let p1 = project_onto_simplex(&[42.0]);
    assert_eq!(p1, vec![1.0]);

    // 3. All negative inputs
    let all_neg = vec![-100.0, -50.0, -10.0];
    let p_neg = project_onto_simplex(&all_neg);
    assert!((p_neg.iter().sum::<f64>() - 1.0).abs() < 1e-10);
    for &x in &p_neg {
        assert!(x >= 0.0);
    }
    // Greatest element (-10.0) gets 1.0
    assert_eq!(p_neg[2], 1.0);
    assert_eq!(p_neg[0], 0.0);
    assert_eq!(p_neg[1], 0.0);

    // 4. Extreme magnitude spread
    let extreme_spread = vec![1e7, -1e7, 0.0];
    let p_spread = project_onto_simplex(&extreme_spread);
    assert!((p_spread.iter().sum::<f64>() - 1.0).abs() < 1e-10);
    assert_eq!(p_spread[0], 1.0);
    assert_eq!(p_spread[1], 0.0);
    assert_eq!(p_spread[2], 0.0);

    // 5. High-dimensional vector K=500
    let mut big_v = Vec::with_capacity(500);
    for i in 0..500 {
        big_v.push(((i as f64) * 0.37).sin());
    }
    let p_big = project_onto_simplex(&big_v);
    assert_eq!(p_big.len(), 500);
    let sum_big: f64 = p_big.iter().sum();
    assert!(
        (sum_big - 1.0).abs() < 1e-10,
        "Simplex projection sum error: {}",
        (sum_big - 1.0).abs()
    );
    for &x in &p_big {
        assert!(x >= 0.0);
    }
}

#[test]
fn test_mfg_geometric_convergence_under_contraction() {
    // Test Theorem 35: When kappa < 1, ||m^(t) - m*||_1 <= 2 * kappa^t
    let resources = vec![
        MfgResourceConfig::new("r1", 0.5, 1.0).unwrap(),
        MfgResourceConfig::new("r2", 1.0, 1.2).unwrap(),
        MfgResourceConfig::new("r3", 1.5, 0.8).unwrap(),
    ];
    // beta_max = 1.2. Set tau = 2.0 => kappa = 1.2 / 4.0 = 0.30 < 1.0
    let tau = 2.0;
    // Pure Picard iteration (alpha = 1.0)
    let game = MeanFieldGame::with_damping(resources, tau, 1.0).unwrap();
    let kappa = game.contraction_modulus();
    assert!((kappa - 0.30).abs() < 1e-6);

    // First compute high-precision reference fixed point m*
    let eq_ref = game
        .compute_equilibrium(200, 1e-12)
        .expect("ref equilibrium");
    let m_star = &eq_ref.equilibrium_distribution;

    // Now trace step-by-step iterates
    let k = game.num_resources();
    let mut m = vec![1.0 / (k as f64); k];

    for t in 1..=10 {
        m = game.best_response(&m).expect("step");
        let mut err = 0.0;
        for i in 0..k {
            err += (m[i] - m_star[i]).abs();
        }
        let theoretical_bound = 2.0 * kappa.powi(t);
        assert!(
            err <= theoretical_bound + 1e-8,
            "Step {t}: empirical error {err} exceeds theoretical bound {theoretical_bound}"
        );
    }
}

#[test]
fn test_mfg_price_of_anarchy_bounds_and_zero_cost() {
    // 1. Zero congestion slopes: cost is strictly uncongested constant
    // In uncongested networks, Nash equilibrium equals social optimum => PoA == 1.0
    let uncongested = vec![
        MfgResourceConfig::new("c1", 2.0, 0.0).unwrap(),
        MfgResourceConfig::new("c2", 2.0, 0.0).unwrap(),
    ];
    let game_uncongested = MeanFieldGame::new(uncongested, 1.0).unwrap();
    let poa_uncongested = game_uncongested.compute_price_of_anarchy().unwrap();
    assert!(
        (poa_uncongested.price_of_anarchy - 1.0).abs() < 1e-3,
        "PoA for uncongested network must be 1.0, got {}",
        poa_uncongested.price_of_anarchy
    );

    // 2. Degenerate zero-cost network: base_cost = 0, beta = 0
    let zero_cost = vec![
        MfgResourceConfig::new("free1", 0.0, 0.0).unwrap(),
        MfgResourceConfig::new("free2", 0.0, 0.0).unwrap(),
    ];
    let game_free = MeanFieldGame::new(zero_cost, 1.0).unwrap();
    let poa_free = game_free.compute_price_of_anarchy().unwrap();
    // Must gracefully return PoA = 1.0 without dividing by zero
    assert_eq!(poa_free.price_of_anarchy, 1.0);
    assert!(poa_free.bound_satisfied);

    // 3. Pigou affine network: must be <= 4/3
    let pigou = vec![
        MfgResourceConfig::new("congested", 0.0, 1.0).unwrap(),
        MfgResourceConfig::new("uncongested", 1.0, 0.0).unwrap(),
    ];
    let game_pigou = MeanFieldGame::with_damping(pigou, 0.01, 0.05).unwrap();
    let poa_pigou = game_pigou.compute_price_of_anarchy().unwrap();
    assert!(poa_pigou.bound_satisfied);
    assert!(poa_pigou.price_of_anarchy <= THEORETICAL_POA_UPPER_BOUND + 1e-4);
    assert!(poa_pigou.price_of_anarchy >= 1.0 - 1e-4);

    // 4. Multi-resource random network: PoA must NEVER exceed 4/3
    let multi = vec![
        MfgResourceConfig::new("n1", 0.1, 2.5).unwrap(),
        MfgResourceConfig::new("n2", 0.8, 1.2).unwrap(),
        MfgResourceConfig::new("n3", 1.5, 0.3).unwrap(),
        MfgResourceConfig::new("n4", 0.4, 4.0).unwrap(),
    ];
    let game_multi = MeanFieldGame::new(multi, 0.5).unwrap();
    let poa_multi = game_multi.compute_price_of_anarchy().unwrap();
    assert!(poa_multi.bound_satisfied);
    assert!(
        poa_multi.price_of_anarchy <= THEORETICAL_POA_UPPER_BOUND + 1e-6,
        "PoA exceeded 4/3: {}",
        poa_multi.price_of_anarchy
    );
    assert!(poa_multi.price_of_anarchy >= 1.0 - 1e-4);
}

// =========================================================================
// SECTION 2: PHASE 19 (PAC-BAYES GENERALIZATION & PROMOTION GATE)
// =========================================================================

#[test]
fn test_pac_bayes_boundary_empirical_risk() {
    let monitor = PacBayesMonitor::new();

    // 1. Empirical risk = 0.0: Maurer exact analytical limit p = 1 - exp(-Delta)
    let cert_zero = monitor
        .certify_risk(0.0, 500, 1.0, 0.05)
        .expect("cert zero");
    assert!(cert_zero.bound_verified);
    assert!(cert_zero.maurer_risk_bound > 0.0);
    assert!(cert_zero.maurer_risk_bound <= 1.0);
    // Analytical check: 1 - exp(-Delta)
    let expected_zero = 1.0 - (-cert_zero.complexity_delta).exp();
    assert!(
        (cert_zero.maurer_risk_bound - expected_zero).abs() < 1e-8,
        "Maurer bound at L=0 must match analytical 1 - exp(-Delta)"
    );
    // Fast rate check: at n=500, Maurer bound is much tighter than Pinsker
    assert!(cert_zero.maurer_risk_bound < cert_zero.pinsker_risk_bound * 0.5);

    // 2. Empirical risk = 1.0: Must saturate at 1.0
    let cert_one = monitor.certify_risk(1.0, 500, 1.0, 0.05).expect("cert one");
    assert!(cert_one.bound_verified);
    assert_eq!(cert_one.maurer_risk_bound, 1.0);
    assert_eq!(cert_one.pinsker_risk_bound, 1.0);

    // 3. Near-boundary stability: L = 1e-10 and L = 1.0 - 1e-10
    let cert_near_zero = monitor.certify_risk(1e-10, 500, 1.0, 0.05).unwrap();
    assert!(cert_near_zero.maurer_risk_bound >= 1e-10);
    assert!(cert_near_zero.maurer_risk_bound <= 1.0);

    let cert_near_one = monitor.certify_risk(1.0 - 1e-10, 500, 1.0, 0.05).unwrap();
    assert!(cert_near_one.maurer_risk_bound >= 1.0 - 1e-10);
    assert!(cert_near_one.maurer_risk_bound <= 1.0);

    // 4. Invalid risk values
    assert!(matches!(
        monitor.certify_risk(-0.01, 100, 0.0, 0.05),
        Err(PacBayesError::InvalidRisk { .. })
    ));
    assert!(matches!(
        monitor.certify_risk(1.01, 100, 0.0, 0.05),
        Err(PacBayesError::InvalidRisk { .. })
    ));
    assert!(matches!(
        monitor.certify_risk(f64::NAN, 100, 0.0, 0.05),
        Err(PacBayesError::InvalidRisk { .. })
    ));
}

#[test]
fn test_pac_bayes_sample_size_extremes_n1_and_n1e6() {
    let monitor = PacBayesMonitor::new();

    // 1. Single sample n = 1
    let cert_n1 = monitor.certify_risk(0.05, 1, 0.2, 0.05).expect("n=1 cert");
    assert!(cert_n1.bound_verified);
    assert!(cert_n1.maurer_risk_bound >= 0.05);
    assert!(cert_n1.maurer_risk_bound <= 1.0);
    assert!(cert_n1.complexity_delta > 0.0);

    // 2. Zero samples n = 0 must error
    assert!(matches!(
        monitor.certify_risk(0.05, 0, 0.2, 0.05),
        Err(PacBayesError::InsufficientSamples(0))
    ));

    // 3. Massive sample size n = 1,000,000 (10^6)
    let cert_huge = monitor
        .certify_risk(0.05, 1_000_000, 2.0, 0.01)
        .expect("huge n cert");
    assert!(cert_huge.bound_verified);
    // At n = 10^6, generalization gap Delta -> 0, so Maurer bound converges tightly to L_hat
    // Theoretical gap: sqrt(2 * L * (1 - L) * Delta) ~ sqrt(0.095 * 1.42e-5) ~ 0.00116
    assert!(
        (cert_huge.maurer_risk_bound - 0.05).abs() < 2e-3,
        "At n=10^6, Maurer bound should be extremely close to empirical risk 0.05: got {}",
        cert_huge.maurer_risk_bound
    );
    assert!(cert_huge.maurer_risk_bound >= 0.05);
}

#[test]
fn test_pac_bayes_zero_kl_and_extreme_delta() {
    let monitor = PacBayesMonitor::new();

    // 1. D_KL(Q || P) = 0.0 (Posterior identical to Prior)
    let cert_zero_kl = monitor.certify_risk(0.04, 800, 0.0, 0.05).unwrap();
    assert!(cert_zero_kl.bound_verified);
    assert!(cert_zero_kl.maurer_risk_bound >= 0.04);
    assert!(cert_zero_kl.maurer_risk_bound <= 0.10);

    // 2. Extreme confidence delta -> 0 (delta = 1e-10)
    let cert_small_delta = monitor.certify_risk(0.04, 800, 0.0, 1e-10).unwrap();
    assert!(cert_small_delta.bound_verified);
    // Smaller delta => higher confidence => larger complexity Delta => higher risk bound
    assert!(
        cert_small_delta.maurer_risk_bound > cert_zero_kl.maurer_risk_bound,
        "Risk bound for delta=1e-10 ({}) must exceed bound for delta=0.05 ({})",
        cert_small_delta.maurer_risk_bound,
        cert_zero_kl.maurer_risk_bound
    );
    assert!(cert_small_delta.maurer_risk_bound <= 1.0);

    // 3. Invalid delta values
    assert!(matches!(
        monitor.certify_risk(0.05, 100, 0.0, 0.0),
        Err(PacBayesError::InvalidDelta(..))
    ));
    assert!(matches!(
        monitor.certify_risk(0.05, 100, 0.0, 1.0),
        Err(PacBayesError::InvalidDelta(..))
    ));
    assert!(matches!(
        monitor.certify_risk(0.05, 100, 0.0, -0.1),
        Err(PacBayesError::InvalidDelta(..))
    ));
    assert!(matches!(
        monitor.certify_risk(0.05, 100, 0.0, f64::NAN),
        Err(PacBayesError::InvalidDelta(..))
    ));
}

#[test]
fn test_pac_bayes_maurer_strict_bounds_and_monotonicities() {
    let monitor = PacBayesMonitor::new();

    // 1. Strict monotonicity w.r.t KL divergence
    let n = 1000;
    let delta = 0.05;
    let l_hat = 0.03;
    let kl_values = [0.0, 0.5, 1.0, 2.0, 5.0, 10.0];
    let mut prev_bound = 0.0;
    for &kl in &kl_values {
        let cert = monitor.certify_risk(l_hat, n, kl, delta).unwrap();
        assert!(
            cert.maurer_risk_bound >= l_hat,
            "Maurer bound must be >= empirical risk"
        );
        assert!(cert.maurer_risk_bound <= 1.0, "Maurer bound must be <= 1.0");
        if kl > 0.0 {
            assert!(
                cert.maurer_risk_bound > prev_bound,
                "Maurer bound must be strictly increasing in KL divergence"
            );
        }
        assert!(
            cert.maurer_risk_bound <= cert.pinsker_risk_bound + 1e-10,
            "Maurer bound must be tighter than Pinsker"
        );
        prev_bound = cert.maurer_risk_bound;
    }

    // 2. Strict monotonicity w.r.t confidence delta
    let delta_values = [0.001, 0.01, 0.05, 0.1, 0.2, 0.5];
    let mut prev_bound_delta = 1.0;
    for &d in &delta_values {
        let cert = monitor.certify_risk(l_hat, n, 1.0, d).unwrap();
        // As delta increases, required confidence 1 - delta decreases, so bound must strictly decrease
        assert!(
            cert.maurer_risk_bound < prev_bound_delta,
            "Maurer bound must be strictly decreasing in delta (decreasing confidence)"
        );
        prev_bound_delta = cert.maurer_risk_bound;
    }

    // 3. Strict monotonicity w.r.t sample size n
    let sample_sizes = [50, 100, 250, 500, 1000, 5000, 20000];
    let mut prev_bound_n = 1.0;
    for &s in &sample_sizes {
        let cert = monitor.certify_risk(l_hat, s, 1.0, delta).unwrap();
        // As n increases, sample complexity decreases, so bound must strictly decrease
        assert!(
            cert.maurer_risk_bound < prev_bound_n,
            "Maurer bound must be strictly decreasing in sample size n"
        );
        prev_bound_n = cert.maurer_risk_bound;
    }

    // 4. Exact inversion check: kl(L_hat || L_maurer) == Delta (within 1e-5)
    for &s in &[200, 500, 1000] {
        let cert = monitor.certify_risk(0.04, s, 0.8, 0.05).unwrap();
        let computed_kl = binary_kl(0.04, cert.maurer_risk_bound);
        assert!(
            (computed_kl - cert.complexity_delta).abs() < 1e-5,
            "Root inversion residue: {}",
            (computed_kl - cert.complexity_delta).abs()
        );
    }
}

#[test]
fn test_pac_bayes_catoni_gibbs_temperature_stability_and_zero_variance() {
    let prior = vec![0.25, 0.25, 0.25, 0.25];

    // 1. Zero-variance losses: all candidates have identical loss 0.05
    let zero_var_losses = vec![0.05, 0.05, 0.05, 0.05];
    let (beta_zv, post_zv, bound_zv) =
        PacBayesMonitor::compute_optimal_gibbs_posterior(&prior, &zero_var_losses, 500, 0.05)
            .expect("zero variance gibbs");

    assert!(beta_zv > 0.0, "beta must be positive");
    assert!(beta_zv < 1000.0, "beta must be in valid domain (0, 2n)");
    assert!((0.0..=1.0).contains(&bound_zv));
    // When losses are identical, posterior remains exactly uniform
    for &w in &post_zv {
        assert!(
            (w - 0.25).abs() < 1e-4,
            "Uniform posterior expected under equal losses, got {w}"
        );
    }

    // 2. All zero losses: [0, 0, 0, 0]
    let all_zero_losses = vec![0.0, 0.0, 0.0, 0.0];
    let (beta_z, _post_z, bound_z) =
        PacBayesMonitor::compute_optimal_gibbs_posterior(&prior, &all_zero_losses, 200, 0.05)
            .expect("all zero losses");
    assert!(beta_z > 0.0 && beta_z < 400.0);
    assert!((0.0..=1.0).contains(&bound_z));

    // 3. All one losses: [1, 1, 1, 1]
    let all_one_losses = vec![1.0, 1.0, 1.0, 1.0];
    let (beta_one, _post_one, bound_one) =
        PacBayesMonitor::compute_optimal_gibbs_posterior(&prior, &all_one_losses, 200, 0.05)
            .expect("all one losses");
    assert!(beta_one > 0.0 && beta_one < 400.0);
    assert_eq!(bound_one, 1.0); // Saturated risk

    // 4. Single sample n = 1 Catoni optimization
    let diverse_losses = vec![0.02, 0.10, 0.40, 0.80];
    let (beta_n1, post_n1, bound_n1) =
        PacBayesMonitor::compute_optimal_gibbs_posterior(&prior, &diverse_losses, 1, 0.05)
            .expect("n=1 catoni");
    // For n=1, domain of beta is (0, 2)
    assert!(
        beta_n1 > 0.0 && beta_n1 < 2.0,
        "For n=1, beta must lie in (0, 2), got {beta_n1}"
    );
    assert!((0.0..=1.0).contains(&bound_n1));
    // Best policy (0.02 loss) must have highest posterior mass
    assert!(post_n1[0] > post_n1[1]);
    assert!(post_n1[1] > post_n1[2]);
    assert!(post_n1[2] > post_n1[3]);
}

#[test]
fn test_pac_bayes_policy_promotion_gate_transitions() {
    let monitor = PacBayesMonitor::with_config(PacBayesConfig {
        delta: 0.05,
        target_risk: 0.08,
        risk_margin: 0.01,
    })
    .unwrap();

    // 1. High-performing candidate: empirical losses low, sufficient sample size
    // Promotion gate should approve (PromoteToProduction)
    let good_losses = vec![0.01, 0.03, 0.05];
    let good_gate = monitor
        .run_promotion_gate(&good_losses, 2000)
        .expect("good gate");
    assert_eq!(
        good_gate.verdict,
        PolicyPromotionVerdict::PromoteToProduction
    );
    assert!(good_gate.certified_risk_bound <= 0.08);
    assert!(good_gate.risk_margin >= 0.0);

    // 2. Mediocre candidate: empirical losses high
    // Promotion gate should reject (RejectHighGeneralizationRisk)
    let bad_losses = vec![0.15, 0.25, 0.35];
    let bad_gate = monitor
        .run_promotion_gate(&bad_losses, 2000)
        .expect("bad gate");
    assert_eq!(
        bad_gate.verdict,
        PolicyPromotionVerdict::RejectHighGeneralizationRisk
    );
    assert!(bad_gate.certified_risk_bound > 0.08);
    assert!(bad_gate.risk_margin < 0.0);

    // 3. Exact threshold boundary evaluation
    // If certificate.maurer_risk_bound == target_risk, verdict is PromoteToProduction
    let cert_exact = pandora_runtime::pac_bayes_monitor::PacBayesCertificate {
        empirical_risk: 0.05,
        kl_divergence: 0.5,
        sample_size: 1000,
        delta: 0.05,
        maurer_risk_bound: 0.08, // Exactly equals target_risk
        pinsker_risk_bound: 0.12,
        complexity_delta: 0.01,
        bound_verified: true,
    };
    assert_eq!(
        monitor.evaluate_promotion_gate(&cert_exact, 0.08),
        PolicyPromotionVerdict::PromoteToProduction
    );

    let cert_slightly_over = pandora_runtime::pac_bayes_monitor::PacBayesCertificate {
        maurer_risk_bound: 0.080001,
        ..cert_exact
    };
    assert_eq!(
        monitor.evaluate_promotion_gate(&cert_slightly_over, 0.08),
        PolicyPromotionVerdict::RejectHighGeneralizationRisk
    );
}
