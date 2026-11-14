#![forbid(unsafe_code)]

//! Phase 312: Adversarial Challenge Suite for Milestone 1
//! Empirical testing of TropicalComplexityPrior, Dynamic Arm Selection, and Safety Invariants.

use pandora_runtime::bandit_router::{
    BanditArm, BanditModelRouter, BanditRoleArm, BanditStrategy, ContextualThompsonSamplingRouter,
    LinTsArmState, RoleCapability, SpeculativeRoutingPlan, TaskComplexityFeatures,
    TropicalComplexityPrior,
};
use pandora_runtime::tropical_nn::{TropicalLayer, TropicalNetwork};
use std::path::Path;

// ============================================================================
// CHALLENGE 1: EXTREME INPUTS (EMPTY, SINGLE-LINE, MASSIVE, BINARY GARBAGE)
// ============================================================================

#[test]
fn test_challenge_empty_input() {
    let prior = TropicalComplexityPrior::from_default_path();
    let score = prior.score_source("");
    assert!(
        (0.0..=1.0).contains(&score),
        "score for empty string must be in [0, 1], got {score}"
    );

    let feat = TaskComplexityFeatures::extract(Path::new("test.rs"), "", "empty instruction");
    assert_eq!(feat.line_count, 0);
    assert_eq!(feat.max_ast_depth, 0);
    assert!((0.0..=1.0).contains(&feat.language_complexity));
}

#[test]
fn test_challenge_single_line_input() {
    let prior = TropicalComplexityPrior::from_default_path();
    let single_line = "x = 42";
    let score = prior.score_source(single_line);
    assert!(
        (0.0..=1.0).contains(&score),
        "score for single-line string must be in [0, 1], got {score}"
    );

    let feat = TaskComplexityFeatures::extract(Path::new("test.py"), single_line, "single line");
    assert_eq!(feat.line_count, 1);
    assert!((0.0..=1.0).contains(&feat.language_complexity));
}

#[test]
fn test_challenge_massive_input() {
    let prior = TropicalComplexityPrior::from_default_path();
    // 50,000 lines of nested repetitive code
    let mut massive = String::with_capacity(2_000_000);
    for i in 0..50_000 {
        massive.push_str("    pub fn compute_");
        massive.push_str(&i.to_string());
        massive.push_str("(x: i64) -> i64 { if x > 0 { for _ in 0..5 { x + 1; } } x }\n");
    }

    let score = prior.score_source(&massive);
    assert!(
        (0.0..=1.0).contains(&score),
        "score for massive string must be in [0, 1], got {score}"
    );
    assert!(
        !score.is_nan() && !score.is_infinite(),
        "score must be finite, got {score}"
    );

    let feat =
        TaskComplexityFeatures::extract(Path::new("massive.rs"), &massive, "process massive file");
    assert_eq!(feat.line_count, 50_000);
    assert!((0.0..=1.0).contains(&feat.language_complexity));

    let vec = feat.to_vector();
    for (i, v) in vec.iter().enumerate() {
        assert!(
            (0.0..=1.0).contains(v),
            "feature vector element {i} must be in [0, 1], got {v}"
        );
    }
}

#[test]
fn test_challenge_binary_garbage_and_unicode_escapes() {
    let prior = TropicalComplexityPrior::from_default_path();

    // Arbitrary binary bytes lossily decoded, null bytes, unicode control chars, emojis
    let raw_bytes: Vec<u8> = (0..=255).cycle().take(10_000).collect();
    let lossy_string = String::from_utf8_lossy(&raw_bytes).to_string();

    let score = prior.score_source(&lossy_string);
    assert!(
        (0.0..=1.0).contains(&score),
        "score for binary garbage must be in [0, 1], got {score}"
    );
    assert!(!score.is_nan());

    let weird_unicode = "\0\t\r\n\u{FFFF}\u{FEFF}\u{200B}\u{202E}fn \u{1F980}() { \u{1F916} }";
    let unicode_score = prior.score_source(weird_unicode);
    assert!(
        (0.0..=1.0).contains(&unicode_score),
        "score for weird unicode must be in [0, 1], got {unicode_score}"
    );
    assert!(!unicode_score.is_nan());
}

// ============================================================================
// CHALLENGE 2: SCORE BOUNDS [0.0, 1.0] ACROSS SYNTHETIC TROPICAL NETWORKS
// ============================================================================

#[test]
fn test_challenge_score_source_bounded_even_with_extreme_weights() {
    // Construct synthetic network with extreme positive weights -> raw forward ~ +1000.0
    let l_extreme_pos = TropicalLayer::new(16, 1, vec![1000.0; 16], Some(vec![500.0])).unwrap();
    let net_pos = TropicalNetwork::new("extreme_pos", "ast", vec![l_extreme_pos]).unwrap();
    let prior_pos = TropicalComplexityPrior::with_network(Some(net_pos));

    let score_pos = prior_pos.score_complexity(&[2.0; 16]);
    assert!(
        (0.0..=1.0).contains(&score_pos),
        "extreme pos score must be bounded in [0, 1], got {score_pos}"
    );
    assert!(
        (score_pos - 1.0).abs() < 1e-6,
        "extreme pos score should saturate near 1.0, got {score_pos}"
    );

    // Construct synthetic network with extreme negative weights -> raw forward ~ -1000.0
    let l_extreme_neg = TropicalLayer::new(16, 1, vec![-1000.0; 16], Some(vec![-500.0])).unwrap();
    let net_neg = TropicalNetwork::new("extreme_neg", "ast", vec![l_extreme_neg]).unwrap();
    let prior_neg = TropicalComplexityPrior::with_network(Some(net_neg));

    let score_neg = prior_neg.score_complexity(&[0.0; 16]);
    assert!(
        (0.0..=1.0).contains(&score_neg),
        "extreme neg score must be bounded in [0, 1], got {score_neg}"
    );
    assert!(
        (0.0..1e-6).contains(&score_neg),
        "extreme neg score should saturate near 0.0, got {score_neg}"
    );
}

// ============================================================================
// CHALLENGE 3: TAU ADJUSTED BOUNDED IN [0.20, 0.85] UNDER ALL INPUTS
// ============================================================================

#[test]
fn test_challenge_tau_adjusted_bounds() {
    // Formula: tau = clamp(0.50 + 0.25 * (C - 0.50), 0.20, 0.85)
    let test_complexities = [
        0.0, 0.15, 0.30, 0.50, 0.70, 0.85, 1.0,
        // Edge cases beyond [0, 1] if un-clamped
        -10.0, -1.0, 2.0, 100.0,
    ];

    let mut router = ContextualThompsonSamplingRouter::new(7, 1.0, 0.2);
    router.register_arm(BanditRoleArm {
        arm_id: "draft_test".to_string(),
        model_id: "drafter".to_string(),
        provider_id: "test".to_string(),
        role: RoleCapability::Drafter,
        state: LinTsArmState::new(7, 1.0),
    });
    router.register_arm(BanditRoleArm {
        arm_id: "critic_test".to_string(),
        model_id: "critic".to_string(),
        provider_id: "test".to_string(),
        role: RoleCapability::Critic,
        state: LinTsArmState::new(7, 1.0),
    });
    router.register_arm(BanditRoleArm {
        arm_id: "frontier_test".to_string(),
        model_id: "frontier".to_string(),
        provider_id: "test".to_string(),
        role: RoleCapability::PureFrontier,
        state: LinTsArmState::new(7, 1.0),
    });

    for &comp in &test_complexities {
        let feat = TaskComplexityFeatures::new(50, 2, comp, 1, 0, 50);
        let plan = router.select_routing_plan(&feat);

        match plan {
            SpeculativeRoutingPlan::SpeculativeDrafting {
                acceptance_threshold,
                ..
            } => {
                assert!(
                    (0.20..=0.85).contains(&acceptance_threshold),
                    "tau {acceptance_threshold} must be in [0.20, 0.85] for complexity {comp}"
                );
            }
            SpeculativeRoutingPlan::PureFrontier { ref reason, .. } => {
                // Check threshold in reason message if formatted
                assert!(reason.contains("PureFrontier") || reason.contains("threshold"));
            }
        }

        // Direct mathematical check
        let c_clamped = comp.clamp(0.0, 1.0);
        let tau = (0.50 + 0.25 * (c_clamped - 0.50)).clamp(0.20, 0.85);
        assert!((0.20..=0.85).contains(&tau));
        assert!((0.375..=0.625).contains(&tau));
    }
}

// ============================================================================
// CHALLENGE 4: MISSING MODEL FALLBACK GRACEFULLY RETURNS 0.5 WITHOUT PANIC
// ============================================================================

#[test]
fn test_challenge_missing_model_fallback() {
    let nonexistent_paths = [
        Path::new("this_does_not_exist.pnn"),
        Path::new("/bad/path/file.pnn"),
        Path::new(""),
    ];

    for path in &nonexistent_paths {
        let prior = TropicalComplexityPrior::from_pnn_path(path);
        assert!(!prior.has_network());
        assert_eq!(prior.digest(), None);
        assert_eq!(
            prior.score_source("fn main() { println!(\"hello\"); }"),
            0.50
        );
        assert_eq!(prior.score_complexity(&[1.0; 16]), 0.50);
    }
}

// ============================================================================
// CHALLENGE 5: MONOTONICITY OF ARM SELECTION TOWARD PureFrontier
// ============================================================================

#[test]
fn test_challenge_monotonicity_toward_pure_frontier() {
    let mut router = ContextualThompsonSamplingRouter::new(7, 1.0, 0.2);

    router.register_arm(BanditRoleArm {
        arm_id: "draft_arm".to_string(),
        model_id: "draft_model".to_string(),
        provider_id: "local".to_string(),
        role: RoleCapability::Drafter,
        state: LinTsArmState::new(7, 1.0),
    });
    router.register_arm(BanditRoleArm {
        arm_id: "critic_arm".to_string(),
        model_id: "critic_model".to_string(),
        provider_id: "anthropic".to_string(),
        role: RoleCapability::Critic,
        state: LinTsArmState::new(7, 1.0),
    });
    router.register_arm(BanditRoleArm {
        arm_id: "frontier_arm".to_string(),
        model_id: "frontier_model".to_string(),
        provider_id: "anthropic".to_string(),
        role: RoleCapability::PureFrontier,
        state: LinTsArmState::new(7, 1.0),
    });

    // Train drafter primarily on intercept/bias with high reward ~0.55 on simple code (C=0.1)
    // and lower reward ~0.35 on complex code (C=0.9)
    let simple_feat = TaskComplexityFeatures::new(50, 2, 0.10, 1, 0, 50).to_vector();
    let complex_feat = TaskComplexityFeatures::new(50, 2, 0.90, 1, 0, 50).to_vector();
    for _ in 0..50 {
        router.record_outcome("draft_arm", &simple_feat, 0.55);
        router.record_outcome("draft_arm", &complex_feat, 0.40);
        router.record_outcome("critic_arm", &simple_feat, 0.90);
        router.record_outcome("critic_arm", &complex_feat, 0.90);
        router.record_outcome("frontier_arm", &simple_feat, 0.85);
        router.record_outcome("frontier_arm", &complex_feat, 0.85);
    }

    let mut seen_frontier = false;
    let mut transitions = 0;

    for step in 0..=20 {
        let comp = step as f64 / 20.0;
        let feat = TaskComplexityFeatures::new(50, 2, comp, 1, 0, 50);
        let plan = router.select_routing_plan(&feat);

        match plan {
            SpeculativeRoutingPlan::SpeculativeDrafting { .. } => {
                assert!(
                    !seen_frontier,
                    "Monotonicity violation: flipped back to SpeculativeDrafting at C={comp} after entering PureFrontier"
                );
            }
            SpeculativeRoutingPlan::PureFrontier { .. } => {
                if !seen_frontier {
                    seen_frontier = true;
                    transitions += 1;
                }
            }
        }
    }

    assert!(seen_frontier, "Must reach PureFrontier at high complexity");
    assert_eq!(
        transitions, 1,
        "Must have exactly one clean monotonic transition point from Drafting to Frontier"
    );
}

// ============================================================================
// CHALLENGE 6: MAX-PLUS SEMIRING ALGEBRAIC AXIOMS & ZERO MULTIPLICATIONS
// ============================================================================

#[test]
fn test_challenge_max_plus_semiring_axioms_and_zero_multiplication() {
    // Max-plus semiring (R U {-inf}, max, +):
    // Addition: a (+) b = max(a, b) with zero element -inf
    // Multiplication: a (*) b = a + b with unit element 0

    // 1. Idempotency: x (+) x = x
    let x = 4.2f32;
    assert_eq!(x.max(x), x);

    // 2. Zero element: x (+) (-inf) = x
    assert_eq!(x.max(f32::NEG_INFINITY), x);

    // 3. Multiplicative identity: x (*) 0 = x (i.e. x + 0 = x)
    assert_eq!(x + 0.0, x);

    // 4. Distributivity of (*) over (+): a (*) (b (+) c) = (a (*) b) (+) (a (*) c)
    let a = 3.5f32;
    let b = -1.2f32;
    let c = 7.8f32;
    let lhs = a + b.max(c);
    let rhs = (a + b).max(a + c);
    assert!((lhs - rhs).abs() < 1e-6);

    // 5. Test TropicalLayer::forward executes purely via max and addition
    let layer = TropicalLayer::new(
        4,
        2,
        vec![
            1.0, 2.0, 3.0, 4.0, // row 0
            -1.0, 0.0, 5.0, -2.0, // row 1
        ],
        Some(vec![0.5, -0.5]),
    )
    .unwrap();

    let input = [10.0f32, -5.0, 2.0, 0.0];
    let output = layer.forward(&input).unwrap();

    // Row 0: max(1+10, 2-5, 3+2, 4+0) + 0.5 = max(11, -3, 5, 4) + 0.5 = 11 + 0.5 = 11.5
    // Row 1: max(-1+10, 0-5, 5+2, -2+0) - 0.5 = max(9, -5, 7, -2) - 0.5 = 9 - 0.5 = 8.5
    assert_eq!(output.len(), 2);
    assert!((output[0] - 11.5).abs() < 1e-6);
    assert!((output[1] - 8.5).abs() < 1e-6);
}

// ============================================================================
// CHALLENGE 7: DYNAMIC ROUTING END-TO-END PRIOR WIRING ON REAL CODE
// ============================================================================

#[test]
fn test_challenge_dynamic_routing_end_to_end_wiring() {
    let prior = TropicalComplexityPrior::from_default_path();

    let simple_code = "pub fn add_one(x: i32) -> i32 { x + 1 }";
    let complex_code = r#"
        use std::sync::{Arc, Mutex, RwLock};
        use std::collections::{HashMap, BTreeMap};
        pub struct ConsensusManager<T: Clone + Send + Sync> {
            nodes: Arc<RwLock<HashMap<String, BTreeMap<u64, Vec<T>>>>>,
            lock: Arc<Mutex<usize>>,
        }
        impl<T: Clone + Send + Sync> ConsensusManager<T> {
            pub async fn vote(&mut self, term: u64, log: Vec<T>) -> Result<bool, String> {
                for i in 0..10 {
                    if i > 5 {
                        match log.get(i) {
                            Some(val) => {
                                let mut guard = self.nodes.write().map_err(|e| e.to_string())?;
                                guard.entry(format!("term_{term}")).or_default().insert(term, vec![val.clone()]);
                            }
                            None => { return Err("failed to achieve consensus".to_string()); }
                        }
                    }
                }
                Ok(true)
            }
        }
    "#;

    let simple_comp = prior.score_source(simple_code);
    let complex_comp = prior.score_source(complex_code);

    assert!(
        (0.0..=1.0).contains(&simple_comp),
        "simple score bounded in [0, 1]"
    );
    assert!(
        (0.0..=1.0).contains(&complex_comp),
        "complex score bounded in [0, 1]"
    );

    if prior.has_network() {
        assert!(
            complex_comp > simple_comp,
            "complex code ({complex_comp:.4}) must have higher score than simple code ({simple_comp:.4})"
        );
    }

    // Now test both routers with this prior
    let mut bandit_router = BanditModelRouter::new(BanditStrategy::ThompsonSampling)
        .with_complexity_prior(prior.clone());
    bandit_router.register_arm(BanditArm::new("fast-arm", "qwen", "ollama").with_context_dim(5));
    bandit_router
        .register_arm(BanditArm::new("deep-arm", "claude", "anthropic").with_context_dim(5));

    let chosen_simple = bandit_router.route_for_source(simple_code, None);
    assert!(chosen_simple.is_some());

    let chosen_complex = bandit_router.route_for_source(complex_code, None);
    assert!(chosen_complex.is_some());

    // Test ContextualThompsonSamplingRouter plan selection
    let mut cts_router =
        ContextualThompsonSamplingRouter::new(7, 1.0, 0.2).with_complexity_prior(prior);
    cts_router.register_arm(BanditRoleArm::new(
        "drafter_arm",
        "qwen-7b",
        "ollama",
        RoleCapability::Drafter,
        7,
        1.0,
    ));
    cts_router.register_arm(BanditRoleArm::new(
        "critic_arm",
        "claude-sonnet",
        "anthropic",
        RoleCapability::Critic,
        7,
        1.0,
    ));
    cts_router.register_arm(BanditRoleArm::new(
        "frontier_arm",
        "claude-opus",
        "anthropic",
        RoleCapability::PureFrontier,
        7,
        1.0,
    ));

    // Train drafter to perform well on simple code features and poorly on complex code features
    let simple_feat = TaskComplexityFeatures::extract_with_prior(
        Path::new("simple.rs"),
        simple_code,
        "add one",
        cts_router.complexity_prior(),
    );
    let complex_feat = TaskComplexityFeatures::extract_with_prior(
        Path::new("complex.rs"),
        complex_code,
        "fix consensus deadlock error: failed",
        cts_router.complexity_prior(),
    );

    let sx = simple_feat.to_vector();
    let cx = complex_feat.to_vector();

    for _ in 0..40 {
        cts_router.record_outcome("drafter_arm", &sx, 0.60);
        cts_router.record_outcome("drafter_arm", &cx, 0.30);
        cts_router.record_outcome("critic_arm", &sx, 0.90);
        cts_router.record_outcome("critic_arm", &cx, 0.90);
        cts_router.record_outcome("frontier_arm", &sx, 0.85);
        cts_router.record_outcome("frontier_arm", &cx, 0.85);
    }

    let plan_simple = cts_router.select_routing_plan(&simple_feat);
    let plan_complex = cts_router.select_routing_plan(&complex_feat);

    match plan_simple {
        SpeculativeRoutingPlan::SpeculativeDrafting {
            acceptance_threshold,
            predicted_speedup,
            ..
        } => {
            assert!(predicted_speedup > 1.05);
            assert!(acceptance_threshold < 0.60);
        }
        SpeculativeRoutingPlan::PureFrontier { ref reason, .. } => {
            panic!("Expected SpeculativeDrafting for simple code, got PureFrontier: {reason}");
        }
    }

    match plan_complex {
        SpeculativeRoutingPlan::PureFrontier { .. } => {}
        SpeculativeRoutingPlan::SpeculativeDrafting { .. } => {
            panic!("Expected PureFrontier for complex consensus code, got SpeculativeDrafting");
        }
    }
}

// ============================================================================
// CHALLENGE 8: ROUTER PRIOR MUTATION & BUILDER METHODS
// ============================================================================

#[test]
fn test_challenge_router_prior_mutation_and_builder() {
    let dummy_prior = TropicalComplexityPrior::from_pnn_path(Path::new("nonexistent.pnn"));
    let mut router = BanditModelRouter::new(BanditStrategy::Ucb1 { c: 1.414 })
        .with_complexity_prior(dummy_prior);

    assert!(!router.complexity_prior().has_network());

    // Mutate via complexity_prior_mut
    let real_prior = TropicalComplexityPrior::from_default_path();
    *router.complexity_prior_mut() = real_prior;

    // Check if network status reflects the update
    let has_net = router.complexity_prior().has_network();
    if Path::new(".pandora/model_tpan.pnn").exists()
        || Path::new("../../.pandora/model_tpan.pnn").exists()
    {
        assert!(has_net);
    }
}

// ============================================================================
// CHALLENGE 9: NAN, SUBNORMAL, AND INF FLOAT RESILIENCE
// ============================================================================

#[test]
fn test_challenge_nan_subnormal_and_inf_float_resilience() {
    let prior = TropicalComplexityPrior::from_default_path();

    let mut extreme_features = [0.0f32; 16];
    extreme_features[0] = f32::MAX;
    extreme_features[1] = f32::MIN_POSITIVE;
    extreme_features[2] = 1e-38;
    extreme_features[3] = -1e-38;

    let score = prior.score_complexity(&extreme_features);
    assert!(!score.is_nan(), "score must not be NaN");
    assert!(
        (0.0..=1.0).contains(&score),
        "score must remain bounded in [0, 1], got {score}"
    );

    // Even with infinities, score_complexity squashes via sigmoid or handles cleanly
    extreme_features[4] = f32::INFINITY;
    let score_inf = prior.score_complexity(&extreme_features);
    assert!(!score_inf.is_nan(), "score with inf must not be NaN");
    assert!(
        (0.0..=1.0).contains(&score_inf),
        "score with inf must remain bounded in [0, 1], got {score_inf}"
    );
}
