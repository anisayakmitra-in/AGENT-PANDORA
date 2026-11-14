#![forbid(unsafe_code)]

//! Phase 312: Integration Tests for Tropical Complexity Prior & Bandit Dynamic Task Routing.
//!
//! Validates:
//! 1. Zero-multiplication tropical inference scoring with `.pandora/model_tpan.pnn`.
//! 2. Dynamic arm selection adjusting between fast/cheap (`SpeculativeDrafting`) and
//!    deep reasoning (`PureFrontier`) based on target source complexity.
//! 3. Graceful fallback to neutral complexity prior (0.50) when the `.pnn` artifact
//!    is absent or corrupted, maintaining `#![forbid(unsafe_code)]` compliance.

use pandora_runtime::bandit_router::{
    BanditArm, BanditModelRouter, BanditRoleArm, BanditStrategy, ContextualThompsonSamplingRouter,
    LinTsArmState, RoleCapability, RoutingContext, SpeculativeRoutingPlan, TaskComplexityFeatures,
    TropicalComplexityPrior,
};
use pandora_runtime::tropical_nn::{TropicalLayer, TropicalNetwork};
use std::fs;
use std::path::{Path, PathBuf};

fn workspace_model_path() -> PathBuf {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    manifest
        .parent()
        .and_then(|p| p.parent())
        .map(|w| w.join(".pandora").join("model_tpan.pnn"))
        .unwrap_or_else(|| PathBuf::from(".pandora/model_tpan.pnn"))
}

const SIMPLE_SOURCE: &str = r#"
pub fn add(a: i64, b: i64) -> i64 {
    a + b
}
"#;

const COMPLEX_SOURCE: &str = r#"
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, RwLock};

pub struct DistributedConsensusEngine<T: Clone + Send + Sync> {
    pub quorum_size: usize,
    pub nodes: Arc<RwLock<HashMap<String, BTreeMap<u64, Vec<T>>>>>,
    pub leader: Option<String>,
}

impl<T: Clone + Send + Sync> DistributedConsensusEngine<T> {
    pub async fn execute_round(&mut self, term: u64, entries: Vec<T>) -> Result<bool, String> {
        let mut committed = 0usize;
        for i in 0..self.quorum_size {
            for j in 0..10 {
                if i * j > 15 {
                    match entries.get(i) {
                        Some(val) => {
                            let mut write_guard = self.nodes.write().map_err(|e| e.to_string())?;
                            let entry = write_guard.entry(format!("node_{i}")).or_default();
                            entry.insert(term + j as u64, vec![val.clone()]);
                            committed += 1;
                        }
                        None => {
                            if committed < self.quorum_size / 2 {
                                return Err(format!("insufficient quorum at round {term}"));
                            }
                        }
                    }
                }
            }
        }
        Ok(committed >= self.quorum_size)
    }
}
"#;

// ============================================================================
// TEST 1: ZERO-MULTIPLICATION TROPICAL SCORING WITH .pandora/model_tpan.pnn
// ============================================================================

#[test]
fn test_zero_multiplication_tropical_scoring_with_model_tpan() {
    let model_file = workspace_model_path();
    let prior = if model_file.exists() {
        TropicalComplexityPrior::from_pnn_path(&model_file)
    } else {
        TropicalComplexityPrior::from_default_path()
    };

    if prior.has_network() {
        // Verify model metadata & digest
        assert!(prior.has_network());
        let digest = prior.digest().expect("digest present for verified model");
        assert!(!digest.is_empty(), "tamper-evident SHA-256 digest exists");
        assert_eq!(digest.len(), 64, "SHA-256 hex string has 64 characters");
        assert_eq!(
            digest, "cddf207d78a0d920fcd0efc8a6f66ef15416949ec006886dba1f97c5abc50207",
            "active weights must match canonical model_tpan.pnn digest"
        );

        // Zero-multiplication forward pass scoring on AST features
        let simple_features = pandora_runtime::extract_ast_features(SIMPLE_SOURCE);
        let complex_features = pandora_runtime::extract_ast_features(COMPLEX_SOURCE);

        let simple_score = prior.score_complexity(&simple_features);
        let complex_score = prior.score_complexity(&complex_features);

        // Scores are valid probabilities in (0.0, 1.0)
        assert!(
            simple_score > 0.0 && simple_score < 1.0,
            "simple score {simple_score} within (0, 1)"
        );
        assert!(
            complex_score > 0.0 && complex_score < 1.0,
            "complex score {complex_score} within (0, 1)"
        );

        // Complex code with loops, generics, locks, and branches scores higher than basic add
        assert!(
            complex_score > simple_score,
            "complex source score ({complex_score:.4}) must exceed simple source score ({simple_score:.4})"
        );

        // Score directly from source
        let direct_simple = prior.score_source(SIMPLE_SOURCE);
        let direct_complex = prior.score_source(COMPLEX_SOURCE);
        assert_eq!(direct_simple, simple_score);
        assert_eq!(direct_complex, complex_score);
    } else {
        // Fallback behavior when model is not available
        assert_eq!(prior.score_source(SIMPLE_SOURCE), 0.50);
        assert_eq!(prior.score_source(COMPLEX_SOURCE), 0.50);
    }
}

// ============================================================================
// TEST 2: TASK COMPLEXITY FEATURE EXTRACTION VIA TROPICAL PRIOR
// ============================================================================

#[test]
fn test_task_complexity_features_extract_with_tropical_prior() {
    let model_file = workspace_model_path();
    let prior = TropicalComplexityPrior::from_pnn_path(&model_file);

    let simple_target = Path::new("math.py");
    let complex_target = Path::new("crates/runtime/src/consensus.rs");

    let simple_feat = TaskComplexityFeatures::extract_with_prior(
        simple_target,
        SIMPLE_SOURCE,
        "add two numbers",
        &prior,
    );
    let complex_feat = TaskComplexityFeatures::extract_with_prior(
        complex_target,
        COMPLEX_SOURCE,
        "fix deadlock error: lock poisoning failed",
        &prior,
    );

    // Verify AST and language complexity extraction
    assert!(simple_feat.line_count < complex_feat.line_count);
    assert!(simple_feat.max_ast_depth < complex_feat.max_ast_depth);
    assert!(simple_feat.dependency_count <= complex_feat.dependency_count);
    assert_eq!(simple_feat.error_count, 0);
    assert_eq!(complex_feat.error_count, 1);

    if prior.has_network() {
        assert!(
            complex_feat.language_complexity > simple_feat.language_complexity,
            "complex code ({:.3}) must have higher language_complexity than simple code ({:.3})",
            complex_feat.language_complexity,
            simple_feat.language_complexity
        );
    }

    // Verify 7-dimensional normalized feature vector
    let vec_simple = simple_feat.to_feature_vector();
    let vec_complex = complex_feat.to_feature_vector();

    assert_eq!(vec_simple[0], 1.0, "bias intercept x_0 = 1.0");
    assert_eq!(vec_complex[0], 1.0, "bias intercept x_0 = 1.0");

    for (i, val) in vec_simple.iter().enumerate() {
        assert!(
            *val >= 0.0 && *val <= 1.0,
            "feature x_{i} = {val} bounded in [0, 1]"
        );
    }
    for (i, val) in vec_complex.iter().enumerate() {
        assert!(
            *val >= 0.0 && *val <= 1.0,
            "feature x_{i} = {val} bounded in [0, 1]"
        );
    }
}

// ============================================================================
// TEST 3: DYNAMIC ARM SELECTION ADJUSTS THRESHOLD BASED ON COMPLEXITY
// ============================================================================

#[test]
fn test_dynamic_arm_selection_routes_based_on_source_complexity() {
    let mut router = ContextualThompsonSamplingRouter::new(7, 1.0, 0.2);

    // Register Drafter, Critic, and PureFrontier arms
    router.register_arm(BanditRoleArm {
        arm_id: "drafter_qwen".to_string(),
        model_id: "qwen2.5-coder-7b".to_string(),
        provider_id: "local_ollama".to_string(),
        role: RoleCapability::Drafter,
        state: LinTsArmState::new(7, 1.0),
    });
    router.register_arm(BanditRoleArm {
        arm_id: "critic_sonnet".to_string(),
        model_id: "claude-3-7-sonnet".to_string(),
        provider_id: "anthropic".to_string(),
        role: RoleCapability::Critic,
        state: LinTsArmState::new(7, 1.0),
    });
    router.register_arm(BanditRoleArm {
        arm_id: "frontier_deep".to_string(),
        model_id: "claude-3-7-sonnet-thinking".to_string(),
        provider_id: "anthropic".to_string(),
        role: RoleCapability::PureFrontier,
        state: LinTsArmState::new(7, 1.0),
    });

    // CASE A: Low-complexity task (C = 0.15)
    // Formula: tau_adjusted = 0.50 + 0.25 * (0.15 - 0.50) = 0.50 - 0.0875 = 0.4125
    let low_comp_feat = TaskComplexityFeatures::new(20, 1, 0.15, 0, 0, 50);

    // CASE B: High-complexity task (C = 0.90)
    // Formula: tau_adjusted = 0.50 + 0.25 * (0.90 - 0.50) = 0.50 + 0.1000 = 0.6000
    let high_comp_feat = TaskComplexityFeatures::new(1200, 12, 0.90, 25, 3, 500);

    // Provide training outcomes to calibrate drafter on low vs high complexity
    // (fast drafter achieves good reward ~0.52 on low complexity, drops to ~0.35 on high complexity)
    let low_x = low_comp_feat.to_vector();
    let high_x = high_comp_feat.to_vector();
    for _ in 0..30 {
        router.record_outcome("drafter_qwen", &low_x, 0.52);
        router.record_outcome("drafter_qwen", &high_x, 0.35);
        router.record_outcome("critic_sonnet", &low_x, 0.90);
        router.record_outcome("critic_sonnet", &high_x, 0.90);
        router.record_outcome("frontier_deep", &low_x, 0.85);
        router.record_outcome("frontier_deep", &high_x, 0.85);
    }

    // Drafter predicted reward (~0.52) >= 0.4125 -> routes to SpeculativeDrafting!
    let plan_low = router.select_routing_plan(&low_comp_feat);

    match plan_low {
        SpeculativeRoutingPlan::SpeculativeDrafting {
            drafter_model,
            acceptance_threshold,
            predicted_speedup,
            ..
        } => {
            assert_eq!(drafter_model, "qwen2.5-coder-7b");
            assert!(
                (acceptance_threshold - 0.4125).abs() < 1e-4,
                "acceptance threshold {acceptance_threshold} should match tau_adjusted 0.4125"
            );
            assert!(
                predicted_speedup > 1.05,
                "speedup factor must exceed 1.05 for speculative drafting"
            );
        }
        SpeculativeRoutingPlan::PureFrontier { reason, .. } => {
            panic!(
                "Low complexity task should route to SpeculativeDrafting, got PureFrontier: {reason}"
            );
        }
    }

    // Drafter predicted reward (~0.35) < 0.6000 -> routes to PureFrontier!
    let plan_high = router.select_routing_plan(&high_comp_feat);

    match plan_high {
        SpeculativeRoutingPlan::PureFrontier {
            frontier_model,
            reason,
            ..
        } => {
            assert_eq!(frontier_model, "claude-3-7-sonnet-thinking");
            assert!(
                reason.contains("0.90") || reason.contains("PureFrontier"),
                "reason should mention high complexity routing: {reason}"
            );
        }
        SpeculativeRoutingPlan::SpeculativeDrafting { .. } => {
            panic!("High complexity task should route to PureFrontier (deep reasoning)");
        }
    }
}

// ============================================================================
// TEST 4: END-TO-END SOURCE ROUTING VIA ROUTER COMPLEXITY PRIOR
// ============================================================================

#[test]
fn test_select_routing_plan_for_source_end_to_end() {
    let model_file = workspace_model_path();
    let prior = TropicalComplexityPrior::from_pnn_path(&model_file);

    let mut router =
        ContextualThompsonSamplingRouter::new(7, 1.0, 0.2).with_complexity_prior(prior.clone());

    router.register_arm(BanditRoleArm {
        arm_id: "draft_fast".to_string(),
        model_id: "qwen2.5-coder-7b".to_string(),
        provider_id: "local".to_string(),
        role: RoleCapability::Drafter,
        state: LinTsArmState::new(7, 1.0),
    });
    router.register_arm(BanditRoleArm {
        arm_id: "critic_deep".to_string(),
        model_id: "claude-3-7-sonnet".to_string(),
        provider_id: "anthropic".to_string(),
        role: RoleCapability::Critic,
        state: LinTsArmState::new(7, 1.0),
    });
    router.register_arm(BanditRoleArm {
        arm_id: "frontier_expert".to_string(),
        model_id: "claude-3-7-sonnet".to_string(),
        provider_id: "anthropic".to_string(),
        role: RoleCapability::PureFrontier,
        state: LinTsArmState::new(7, 1.0),
    });

    // Seed training: drafter performs well on simple helper tasks (~0.52), but poorly on complex tasks (~0.35)
    let simple_feat = TaskComplexityFeatures::extract_with_prior(
        Path::new("helper.py"),
        SIMPLE_SOURCE,
        "format variable",
        router.complexity_prior(),
    )
    .to_vector();
    let complex_feat = TaskComplexityFeatures::extract_with_prior(
        Path::new("consensus.rs"),
        COMPLEX_SOURCE,
        "investigate and fix deadlock error: thread panic failed",
        router.complexity_prior(),
    )
    .to_vector();
    for _ in 0..25 {
        router.record_outcome("draft_fast", &simple_feat, 0.58);
        router.record_outcome("draft_fast", &complex_feat, 0.35);
        router.record_outcome("critic_deep", &simple_feat, 0.90);
        router.record_outcome("critic_deep", &complex_feat, 0.90);
        router.record_outcome("frontier_expert", &simple_feat, 0.85);
        router.record_outcome("frontier_expert", &complex_feat, 0.85);
    }

    let simple_plan = router.select_routing_plan_for_source(
        Path::new("helper.py"),
        SIMPLE_SOURCE,
        "format variable",
    );
    let complex_plan = router.select_routing_plan_for_source(
        Path::new("consensus.rs"),
        COMPLEX_SOURCE,
        "investigate and fix deadlock error: thread panic failed",
    );

    // Simple code routes to fast/cheap drafting
    assert!(
        matches!(
            simple_plan,
            SpeculativeRoutingPlan::SpeculativeDrafting { .. }
        ),
        "simple code should select SpeculativeDrafting"
    );

    // Complex code routes to deep reasoning
    assert!(
        matches!(complex_plan, SpeculativeRoutingPlan::PureFrontier { .. }),
        "complex code should select PureFrontier"
    );
}

// ============================================================================
// TEST 5: GRACEFUL FALLBACK ON MISSING OR CORRUPTED MODEL ARTIFACT
// ============================================================================

#[test]
fn test_graceful_fallback_when_model_absent() {
    let absent_path = Path::new("nonexistent_dir/absent_model.pnn");
    let prior = TropicalComplexityPrior::from_pnn_path(absent_path);

    assert!(!prior.has_network());
    assert_eq!(prior.digest(), None);

    // Score defaults cleanly to 0.50 neutral without panicking
    let score = prior.score_complexity(&[0.8; 16]);
    assert_eq!(score, 0.50);

    let source_score = prior.score_source("fn main() {}");
    assert_eq!(source_score, 0.50);

    // Router with absent prior uses baseline threshold 0.50
    let mut router =
        ContextualThompsonSamplingRouter::new(7, 1.0, 0.2).with_complexity_prior(prior);

    assert!(!router.complexity_prior().has_network());

    // Neutral complexity C = 0.50 yields threshold = 0.50
    let neutral_feat = TaskComplexityFeatures::new(100, 3, 0.50, 2, 0, 100);
    router.register_arm(BanditRoleArm {
        arm_id: "frontier_arm".to_string(),
        model_id: "claude-3-7-sonnet".to_string(),
        provider_id: "anthropic".to_string(),
        role: RoleCapability::PureFrontier,
        state: LinTsArmState::new(7, 1.0),
    });
    let plan = router.select_routing_plan(&neutral_feat);
    assert!(matches!(plan, SpeculativeRoutingPlan::PureFrontier { .. }));
}

#[test]
fn test_graceful_fallback_when_model_corrupted() {
    let temp_dir = std::env::temp_dir();
    let corrupt_path = temp_dir.join("pandora_test_corrupt_tpan.pnn");

    // Write malformed JSON
    fs::write(&corrupt_path, b"{\"corrupted_json\": true, incomplete...").unwrap();

    // try_from_pnn_path returns Err
    let try_res = TropicalComplexityPrior::try_from_pnn_path(&corrupt_path);
    assert!(try_res.is_err(), "corrupted model should return Err");

    // from_pnn_path falls back cleanly to network: None without panicking
    let prior = TropicalComplexityPrior::from_pnn_path(&corrupt_path);
    assert!(!prior.has_network());
    assert_eq!(prior.digest(), None);
    assert_eq!(prior.score_source("pub fn add() {}"), 0.50);

    // Cleanup
    let _ = fs::remove_file(corrupt_path);
}

// ============================================================================
// TEST 6: BANDIT MODEL ROUTER COMPLEXITY PRIOR INTEGRATION
// ============================================================================

#[test]
fn test_bandit_model_router_complexity_prior_integration() {
    let router = BanditModelRouter::new(BanditStrategy::ThompsonSampling);

    // Has default prior
    assert!(
        router.complexity_prior().digest().is_some() || !router.complexity_prior().has_network()
    );

    // Can configure custom prior via builder
    let custom_prior = TropicalComplexityPrior::from_pnn_path(Path::new("dummy.pnn"));
    let mut router = router.with_complexity_prior(custom_prior);
    assert!(!router.complexity_prior().has_network());

    // Mutable accessor
    let mut_prior = router.complexity_prior_mut();
    assert!(!mut_prior.has_network());

    // Register candidate arms
    router.register_arm(
        BanditArm::new("qwen-fast", "qwen2.5-coder-7b", "ollama").with_context_dim(5),
    );
    router.register_arm(
        BanditArm::new("claude-smart", "claude-3-7-sonnet", "anthropic").with_context_dim(5),
    );

    // route_for_source executes without error
    let chosen = router.route_for_source(SIMPLE_SOURCE, None);
    assert!(chosen.is_some(), "arm selection succeeds for source code");

    // RoutingContext from source with prior
    let ctx = RoutingContext::from_source_with_prior(SIMPLE_SOURCE, router.complexity_prior());
    assert_eq!(ctx.query_length, SIMPLE_SOURCE.len());
    assert_eq!(ctx.task_complexity, 0.50);
}

// ============================================================================
// TEST 7: CRATE ROOT RE-EXPORTS VERIFICATION
// ============================================================================

#[test]
fn test_crate_root_reexports() {
    // Verify TropicalComplexityPrior is re-exported from crate root
    let default_prior = pandora_runtime::TropicalComplexityPrior::from_default_path();
    let score = default_prior.score_complexity(&[0.0; 16]);
    assert!(
        (0.0..=1.0).contains(&score),
        "score {score} must be within [0, 1]"
    );
    if default_prior.has_network() {
        let digest = default_prior.digest().expect("network digest present");
        assert_eq!(digest.len(), 64, "SHA-256 hex string has 64 characters");
    } else {
        assert_eq!(score, 0.50, "neutral fallback should be 0.50");
    }

    // Verify tropical_nn types are re-exported
    let layer =
        TropicalLayer::new(16, 1, vec![0.1; 16], None).expect("layer construction succeeds");
    assert_eq!(layer.in_features, 16);
    assert_eq!(layer.out_features, 1);

    let net = TropicalNetwork::new("test_reexport", "ast", vec![layer])
        .expect("network construction succeeds");
    assert_eq!(net.name, "test_reexport");

    let ast_features = pandora_runtime::extract_ast_features("fn main() {}");
    assert_eq!(ast_features.len(), 16);
}
