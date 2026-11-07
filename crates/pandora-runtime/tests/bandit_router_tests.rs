#![forbid(unsafe_code)]

use pandora_runtime::bandit_router::{
    BanditArm, BanditModelRouter, BanditStrategy, RoutingContext, RoutingReward, SafePrng,
};

#[test]
fn test_safe_prng_distributions() {
    let mut rng = SafePrng::new(42);

    // Uniform distribution bounds
    for _ in 0..100 {
        let u = rng.sample_uniform_f64();
        assert!((0.0..1.0).contains(&u), "uniform float out of [0, 1): {u}");
    }

    // Normal distribution mean and bounds
    let mut normal_sum = 0.0;
    for _ in 0..500 {
        normal_sum += rng.sample_standard_normal();
    }
    let normal_mean = normal_sum / 500.0;
    assert!(normal_mean.abs() < 0.2, "standard normal mean drifted: {normal_mean}");

    // Beta(1.0, 1.0) is standard uniform [0, 1]
    let mut beta_sum = 0.0;
    for _ in 0..500 {
        let b = rng.sample_beta(1.0, 1.0);
        assert!((0.0..=1.0).contains(&b));
        beta_sum += b;
    }
    let beta_mean = beta_sum / 500.0;
    assert!((beta_mean - 0.5).abs() < 0.1, "Beta(1,1) mean drifted: {beta_mean}");

    // Beta(9.0, 1.0) expected mean is 9 / 10 = 0.90
    let mut biased_sum = 0.0;
    for _ in 0..500 {
        biased_sum += rng.sample_beta(9.0, 1.0);
    }
    let biased_mean = biased_sum / 500.0;
    assert!((biased_mean - 0.90).abs() < 0.08, "Beta(9,1) mean drifted: {biased_mean}");
}

#[test]
fn test_ucb1_initial_exploration() {
    let mut router = BanditModelRouter::new(BanditStrategy::Ucb1 { c: 1.414 });
    router.register_arm(BanditArm::new("arm_1", "gpt-4o", "openai"));
    router.register_arm(BanditArm::new("arm_2", "claude-3-5-sonnet", "anthropic"));
    router.register_arm(BanditArm::new("arm_3", "gemini-1-5-pro", "google"));

    let ctx = RoutingContext::new(256, 0.5, 0.5, 2000.0);

    // Initial 3 selections must explore all 3 distinct unpulled arms
    let mut pulled = Vec::new();
    for _ in 0..3 {
        let arm_id = router.select_arm(&ctx).expect("arm selected");
        assert!(!pulled.contains(&arm_id), "unpulled arm should be selected first");
        pulled.push(arm_id.clone());
        router.record_reward(&arm_id, 0.5);
    }
    assert_eq!(pulled.len(), 3);
}

#[test]
fn test_ucb1_regret_bounds_and_convergence() {
    let mut router = BanditModelRouter::with_seed(BanditStrategy::Ucb1 { c: 1.414 }, 1337);
    router.register_arm(BanditArm::new("arm_optimal", "fast-model", "prov_a"));
    router.register_arm(BanditArm::new("arm_medium", "mid-model", "prov_b"));
    router.register_arm(BanditArm::new("arm_slow", "slow-model", "prov_c"));

    let true_rewards = [
        ("arm_optimal", 0.90),
        ("arm_medium", 0.50),
        ("arm_slow", 0.20),
    ];

    let ctx = RoutingContext::new(512, 0.4, 0.8, 1500.0);
    let total_rounds = 400;

    for _ in 0..total_rounds {
        let chosen_id = router.select_arm(&ctx).expect("selection succeeded");
        let true_reward = true_rewards
            .iter()
            .find(|(id, _)| *id == chosen_id.as_str())
            .map(|(_, r)| *r)
            .unwrap_or(0.0);
        router.record_reward(&chosen_id, true_reward);
    }

    let optimal_arm = router.get_arm("arm_optimal").unwrap();
    let medium_arm = router.get_arm("arm_medium").unwrap();
    let slow_arm = router.get_arm("arm_slow").unwrap();

    // Optimal arm must dominate pull count
    assert!(
        optimal_arm.pull_count() > medium_arm.pull_count() * 2,
        "optimal arm should be pulled significantly more than medium arm"
    );
    assert!(
        optimal_arm.pull_count() > slow_arm.pull_count() * 4,
        "optimal arm should be pulled significantly more than slow arm"
    );

    // Cumulative regret must stay within theoretical minimax bound
    let cumulative_regret = router.cumulative_regret(0.90);
    let theoretical_bound = BanditModelRouter::theoretical_minimax_regret(3, total_rounds);

    assert!(
        cumulative_regret < theoretical_bound,
        "cumulative regret {cumulative_regret} exceeded minimax bound {theoretical_bound}"
    );
}

#[test]
fn test_thompson_sampling_convergence() {
    let mut router = BanditModelRouter::with_seed(BanditStrategy::ThompsonSampling, 9999);
    router.register_arm(BanditArm::new("arm_high", "model-high", "prov1"));
    router.register_arm(BanditArm::new("arm_low", "model-low", "prov2"));

    let ctx = RoutingContext::new(100, 0.2, 0.9, 1000.0);

    // Provide higher reward to arm_high on average
    for _ in 0..300 {
        let arm_id = router.select_arm(&ctx).expect("arm selected");
        let reward = if arm_id == "arm_high" { 0.85 } else { 0.25 };
        router.record_reward(&arm_id, reward);
    }

    let high_arm = router.get_arm("arm_high").unwrap();
    let low_arm = router.get_arm("arm_low").unwrap();

    assert!(
        high_arm.pull_count() > low_arm.pull_count() * 2,
        "Thompson sampling should favor arm_high (high: {}, low: {})",
        high_arm.pull_count(),
        low_arm.pull_count()
    );
    assert!(high_arm.alpha() > low_arm.alpha());
}

#[test]
fn test_epsilon_greedy_behavior() {
    // Pure exploitation: epsilon = 0.0
    let mut router = BanditModelRouter::with_seed(BanditStrategy::EpsilonGreedy { epsilon: 0.0 }, 777);
    router.register_arm(BanditArm::new("arm_a", "m1", "p1"));
    router.register_arm(BanditArm::new("arm_b", "m2", "p2"));

    router.record_reward("arm_a", 0.9);
    router.record_reward("arm_b", 0.3);

    let ctx = RoutingContext::default_for_prompt("summarize this test code");

    // Pure exploitation must always pick arm_a
    for _ in 0..20 {
        let arm = router.select_arm(&ctx).unwrap();
        assert_eq!(arm, "arm_a");
    }
}

#[test]
fn test_routing_reward_calculation() {
    let reward_calc = RoutingReward {
        weight_success: 0.50,
        weight_latency: 0.30,
        weight_cost: 0.20,
    };

    // Perfect call: succeeded, 0 ms latency, 0 tokens
    let r_perfect = reward_calc.calculate(true, 0.0, 1000.0, 0.0, 1000.0);
    assert!((r_perfect - 1.0).abs() < 1e-6);

    // Failed call: not succeeded, at max latency, at max tokens
    let r_failed = reward_calc.calculate(false, 1000.0, 1000.0, 1000.0, 1000.0);
    assert!((r_failed - 0.0).abs() < 1e-6);

    // Intermediate call: succeeded, 50% latency, 50% tokens
    let r_mid = reward_calc.calculate(true, 500.0, 1000.0, 500.0, 1000.0);
    // 0.50*1.0 + 0.30*0.5 + 0.20*0.5 = 0.50 + 0.15 + 0.10 = 0.75
    assert!((r_mid - 0.75).abs() < 1e-6);
}

#[test]
fn test_contextual_reward_learning() {
    let mut router = BanditModelRouter::with_seed(BanditStrategy::ThompsonSampling, 12345);
    let arm = BanditArm::new("arm_context", "model-c", "prov-c").with_context_dim(5);
    router.register_arm(arm);

    let ctx = RoutingContext::new(2048, 0.9, 0.8, 4000.0);
    assert_eq!(ctx.to_features().len(), 5);

    for _ in 0..20 {
        router.record_contextual_reward("arm_context", 1.0, &ctx);
    }

    let arm_ref = router.get_arm("arm_context").unwrap();
    assert_eq!(arm_ref.pull_count(), 20);
    assert!(arm_ref.feature_weights().iter().any(|&w| w > 0.0));
}
