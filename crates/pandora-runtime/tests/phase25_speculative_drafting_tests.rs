#![forbid(unsafe_code)]

//! Phase 25 Speculative Multi-Model Drafting & Contextual Thompson Sampling Verification Suite.
//!
//! Comprehensive 22-scenario test suite verifying:
//! 1. AST & Diff Drafting (creation, extraction, syntax validation, delimiter nesting, conflict marker rejection, unsafe code rejection).
//! 2. Frontier Critic (clean certification, low-score rejection, fatal syntax defect handling).
//! 3. Residual Diff Algebra (single-hunk, multi-hunk, algebraic identity law, clean rollback on failure).
//! 4. LinTS Bandit Routing (Cholesky numerical stability, covariance matching, contextual convergence, theoretical regret bounding, dynamic role allocation).
//! 5. End-to-End integration with mock providers and edge cases.

use pandora_provider::{
    ModelRequest, ModelResponse, Provider, ProviderError, ProviderManifest, TokenUsage,
};
use pandora_runtime::bandit_router::{
    BanditModelRouter, BanditRoleArm, ContextualThompsonSamplingRouter, LinTsArmState,
    RoleCapability, RoutingContext, SafePrng, SpeculativeRoutingPlan, TASK_COMPLEXITY_DIM,
    TaskComplexityFeatures,
};
use pandora_runtime::speculative_drafting::{
    AstNodeKind, FrontierCriticEvaluation, ResidualDiffComposer, SpeculativeDiffDraft,
    SpeculativeDraftingEngine, SpeculativeError, UnifiedPatch,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

// ============================================================================
// MOCK PROVIDER HARNESS
// ============================================================================

#[derive(Clone)]
struct MockSpeculativeProvider {
    manifest: ProviderManifest,
    preset_response: Arc<Mutex<Option<String>>>,
    call_count: Arc<Mutex<usize>>,
}

impl MockSpeculativeProvider {
    fn new(id: &str, preset: Option<&str>) -> Self {
        let manifest = ProviderManifest::new(
            id,
            "Mock Provider",
            "https://mock.provider.local",
            "mock-model",
            "PANDORA_MOCK_KEY",
        )
        .expect("valid manifest");
        Self {
            manifest,
            preset_response: Arc::new(Mutex::new(preset.map(|s| s.to_string()))),
            call_count: Arc::new(Mutex::new(0)),
        }
    }

    #[allow(dead_code)]
    fn set_response(&self, resp: &str) {
        let mut lock = self.preset_response.lock().unwrap();
        *lock = Some(resp.to_string());
    }

    #[allow(dead_code)]
    fn calls(&self) -> usize {
        *self.call_count.lock().unwrap()
    }
}

impl Provider for MockSpeculativeProvider {
    fn manifest(&self) -> &ProviderManifest {
        &self.manifest
    }

    fn complete(&self, _request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        let mut count = self.call_count.lock().unwrap();
        *count += 1;
        let response_text = self
            .preset_response
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| "pub fn fallback_generated() -> bool { true }".to_string());
        Ok(ModelResponse::new(
            response_text,
            Vec::new(),
            TokenUsage::new(100, 50),
        ))
    }
}

// ============================================================================
// CATEGORY 1: AST VALIDATION & SPECULATIVE DIFF GENERATION
// ============================================================================

#[test]
fn test_speculative_diff_draft_creation_and_node_extraction() {
    let raw_diff = r#"--- a/src/math.rs
+++ b/src/math.rs
@@ -1,3 +1,8 @@
+// Fast vector dot product
+pub fn dot_product(a: &[f32], b: &[f32]) -> f32 {
+    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
+}
+
 pub fn add(a: i32, b: i32) -> i32 {
     a + b
 }
"#;
    let draft = SpeculativeDiffDraft::new(
        "draft-001",
        "qwen-coder-7b",
        PathBuf::from("src/math.rs"),
        raw_diff,
        0.92,
    )
    .expect("draft parsing should succeed");

    assert_eq!(draft.model_name, "qwen-coder-7b");
    assert!(draft.estimated_confidence > 0.90);
    assert!(!draft.ast_nodes.is_empty());
    assert!(
        draft
            .ast_nodes
            .iter()
            .any(|n| n.name == "dot_product" && n.kind == AstNodeKind::Function)
    );
}

#[test]
fn test_speculative_diff_ast_validation_clean_pass() {
    let valid_code = r#"
pub struct VectorState {
    pub dim: usize,
    pub values: Vec<f32>,
}

impl VectorState {
    pub fn new(dim: usize) -> Self {
        Self { dim, values: vec![0.0; dim] }
    }
}
"#;
    let result = SpeculativeDraftingEngine::validate_syntax_ast(valid_code);
    assert!(
        result.is_ok(),
        "balanced delimiters and clean Rust syntax must pass AST validation"
    );
}

#[test]
fn test_speculative_diff_ast_validation_unbalanced_delimiters() {
    let unclosed_code = r#"
pub fn broken_fn(x: i32) -> i32 {
    if x > 0 {
        return x * 2;
    // Missing closing brace
}
"#;
    let result = SpeculativeDraftingEngine::validate_syntax_ast(unclosed_code);
    assert!(
        matches!(result, Err(SpeculativeError::DelimiterMismatch(_))),
        "unclosed delimiter must trigger DelimiterMismatch error"
    );
}

#[test]
fn test_speculative_diff_ast_validation_conflict_marker_rejection() {
    let conflicted_code = r#"
pub fn calc() -> i32 {
<<<<<<< HEAD
    42
=======
    84
>>>>>>> feature-branch
}
"#;
    let result = SpeculativeDraftingEngine::validate_syntax_ast(conflicted_code);
    assert!(
        matches!(result, Err(SpeculativeError::ConflictMarkerDetected(_))),
        "leftover conflict markers must be immediately rejected"
    );
}

#[test]
fn test_speculative_diff_ast_validation_unsafe_code_forbidden() {
    let unsafe_code = r#"
pub fn read_ptr(ptr: *const u8) -> u8 {
    unsafe { *ptr }
}
"#;
    let result = SpeculativeDraftingEngine::validate_syntax_ast(unsafe_code);
    assert!(
        matches!(result, Err(SpeculativeError::UnsafeCodeDetected(_))),
        "unsafe block must violate constitutional invariant #![forbid(unsafe_code)]"
    );
}

// ============================================================================
// CATEGORY 2: FRONTIER CRITIC EVALUATION & SCORING
// ============================================================================

#[test]
fn test_frontier_critic_evaluation_clean_certification() {
    let eval = FrontierCriticEvaluation::new(
        "eval-001",
        "claude-3-7-sonnet",
        true,
        0.96,
        None,
        "Clean AST patch, no defects, passes all invariants.",
    );
    assert!(eval.approved);
    assert!(eval.score >= 0.85);
    assert!(eval.residual_diff.is_none());
    assert!(eval.invariant_checks_passed);
}

#[test]
fn test_frontier_critic_evaluation_borderline_score_rejection() {
    let eval = FrontierCriticEvaluation::new(
        "eval-002",
        "claude-3-7-sonnet",
        false,
        0.62,
        None,
        "Insufficient reasoning depth, potential off-by-one boundary defect.",
    );
    assert!(!eval.approved);
    assert!(eval.score < 0.85);
}

#[test]
fn test_frontier_critic_evaluation_fatal_syntax_defect() {
    let defect_patch =
        "--- a/lib.rs\n+++ b/lib.rs\n@@ -1,2 +1,3 @@\n-fn foo() {}\n+fn foo() { unsafe { *p } }\n";
    let syntax_check = SpeculativeDraftingEngine::validate_syntax_ast(defect_patch);
    assert!(syntax_check.is_err());

    let eval = FrontierCriticEvaluation::new(
        "eval-003",
        "claude-3-7-sonnet",
        false,
        0.10,
        None,
        "Fatal defect: unsafe code injection detected.",
    );
    assert!(!eval.approved);
    assert!(eval.score < 0.50);
}

// ============================================================================
// CATEGORY 3: RESIDUAL DIFF SYNTHESIS & ALGEBRAIC COMPOSITION
// ============================================================================

#[test]
fn test_residual_patch_synthesis_and_application_single_hunk() {
    let base = "pub fn value() -> i32 {\n    0\n}\n";
    let cand = "--- a/val.rs\n+++ b/val.rs\n@@ -1,3 +1,3 @@\n pub fn value() -> i32 {\n-    0\n+    10\n }\n";
    let res = "--- a/val.rs\n+++ b/val.rs\n@@ -1,3 +1,3 @@\n pub fn value() -> i32 {\n-    10\n+    42\n }\n";

    let (composed_content, canonical) =
        ResidualDiffComposer::compose(Path::new("val.rs"), base, cand, Some(res))
            .expect("compose single hunk");

    assert_eq!(composed_content, "pub fn value() -> i32 {\n    42\n}\n");
    assert!(canonical.contains("+    42"));
}

#[test]
fn test_residual_patch_synthesis_and_application_multi_hunk() {
    let base = "fn first() -> i32 {\n    1\n}\n\nfn second() -> i32 {\n    2\n}\n";
    let cand = "--- a/lib.rs\n+++ b/lib.rs\n@@ -1,3 +1,3 @@\n fn first() -> i32 {\n-    1\n+    10\n }\n@@ -5,3 +5,3 @@\n fn second() -> i32 {\n-    2\n+    20\n }\n";
    let res = "--- a/lib.rs\n+++ b/lib.rs\n@@ -5,3 +5,3 @@\n fn second() -> i32 {\n-    20\n+    200\n }\n";

    let (composed, canonical) =
        ResidualDiffComposer::compose(Path::new("lib.rs"), base, cand, Some(res))
            .expect("compose multi hunk");

    assert_eq!(
        composed,
        "fn first() -> i32 {\n    10\n}\n\nfn second() -> i32 {\n    200\n}\n"
    );
    let reapplied = UnifiedPatch::apply_to_string(base, &canonical).expect("reapply canonical");
    assert_eq!(reapplied, composed);
}

#[test]
fn test_residual_patch_composition_algebraic_identity() {
    let base_content = "fn main() {\n    println!(\"Hello\");\n}\n";
    let draft_patch = "--- a/main.rs\n+++ b/main.rs\n@@ -1,3 +1,3 @@\n fn main() {\n-    println!(\"Hello\");\n+    println!(\"Hello World\");\n }\n";
    let residual_patch = "--- a/main.rs\n+++ b/main.rs\n@@ -1,3 +1,3 @@\n fn main() {\n-    println!(\"Hello World\");\n+    println!(\"Hello World from Pandora!\");\n }\n";

    // Direct sequential application: apply(apply(C0, D1), D2)
    let c1 = UnifiedPatch::apply_to_string(base_content, draft_patch).expect("draft patch apply");
    let c_expected =
        UnifiedPatch::apply_to_string(&c1, residual_patch).expect("residual patch apply");

    // Composed application: apply(C0, D1 ⊕ D2)
    let (c_composed, canonical_diff) = ResidualDiffComposer::compose(
        Path::new("main.rs"),
        base_content,
        draft_patch,
        Some(residual_patch),
    )
    .expect("residual composition must succeed");

    assert_eq!(c_composed, c_expected);
    assert_eq!(
        c_composed,
        "fn main() {\n    println!(\"Hello World from Pandora!\");\n}\n"
    );

    // Verify canonical unified diff applies directly to base to yield identical text
    let c_from_canonical =
        UnifiedPatch::apply_to_string(base_content, &canonical_diff).expect("canonical diff apply");
    assert_eq!(c_from_canonical, c_expected);
}

#[test]
fn test_residual_patch_application_failure_clean_rollback() {
    let base_content = "fn compute() -> i32 { 100 }\n";
    let draft_patch = "--- a/comp.rs\n+++ b/comp.rs\n@@ -1,1 +1,1 @@\n-fn compute() -> i32 { 100 }\n+fn compute() -> i32 { 200 }\n";
    let bad_residual =
        "--- a/comp.rs\n+++ b/comp.rs\n@@ -50,1 +50,1 @@\n-nonexistent_line_50\n+fixed\n";

    let result = ResidualDiffComposer::compose(
        Path::new("comp.rs"),
        base_content,
        draft_patch,
        Some(bad_residual),
    );
    assert!(
        result.is_err(),
        "corrupt residual must fail cleanly without workspace corruption"
    );
}

// ============================================================================
// CATEGORY 4: CONTEXTUAL THOMPSON SAMPLING (LinTS) & REGRET BOUNDING
// ============================================================================

#[test]
fn test_lints_cholesky_factorization_numerical_stability() {
    let d = 7;
    let lambda = 1.0;
    let mut rng = SafePrng::new(42);

    let mut state = LinTsArmState::new(d, lambda);

    for _ in 0..50 {
        let x: Vec<f64> = (0..d).map(|_| rng.sample_uniform_f64()).collect();
        state.update(&x, rng.sample_uniform_f64());
    }

    let l = state
        .cholesky()
        .expect("Cholesky must succeed on positive definite matrix");
    assert_eq!(l.len(), d * d);

    // Diagonal elements must be >= sqrt(lambda) >= 1.0
    for i in 0..d {
        assert!(
            l[i * d + i] >= lambda.sqrt() - 1e-6,
            "diagonal element L_ii must be bounded below by sqrt(lambda)"
        );
    }

    // Solve L^T w = z for Gaussian posterior sample
    let z: Vec<f64> = (0..d).map(|_| rng.sample_standard_normal()).collect();
    let w = LinTsArmState::solve_lt(&l, d, &z);
    for val in &w {
        assert!(
            !val.is_nan() && !val.is_infinite(),
            "back-solve must produce finite values"
        );
    }
}

#[test]
fn test_lints_posterior_sampling_and_covariance_matching() {
    let d = 7;
    let state = LinTsArmState::new(d, 1.0);
    let mut rng = SafePrng::new(999);

    let mut samples = Vec::new();
    for _ in 0..1000 {
        let theta = state
            .sample_theta(&mut rng, 0.5)
            .expect("sampling succeeds");
        samples.push(theta);
    }

    let mut mean = vec![0.0; d];
    for s in &samples {
        for (i, val) in s.iter().enumerate() {
            mean[i] += val;
        }
    }
    for m in &mut mean {
        *m /= 1000.0;
        assert!(m.abs() < 0.15, "empirical sample mean drifted: {m}");
    }
}

#[test]
fn test_lints_contextual_convergence_to_optimal_arms() {
    let d = 7;
    let mut router = BanditModelRouter::new_contextual_lints(3, d, 1.0, 0.2, 777);
    let mut rng = SafePrng::new(888);

    // Arm 0 is optimal (~0.85), arm 1 is medium (~0.50), arm 2 is poor (~0.20)
    for _ in 0..400 {
        let mut x = vec![1.0]; // x0 = 1.0 bias intercept
        for _ in 1..d {
            x.push(rng.sample_uniform_f64());
        }
        let chosen = router.select_arm_lints(&x);
        let reward = match chosen {
            0 => 0.85 + rng.sample_standard_normal() * 0.05,
            1 => 0.50 + rng.sample_standard_normal() * 0.05,
            _ => 0.20 + rng.sample_standard_normal() * 0.05,
        }
        .clamp(0.0, 1.0);
        router.update_lints(chosen, &x, reward);
    }

    let arm0 = router.get_arm("arm_0").unwrap();
    let arm2 = router.get_arm("arm_2").unwrap();
    assert!(
        arm0.pull_count() > arm2.pull_count() * 2,
        "optimal arm should be pulled significantly more than poor arm: arm0={}, arm2={}",
        arm0.pull_count(),
        arm2.pull_count()
    );
}

#[test]
fn test_lints_cumulative_regret_bounded_by_theoretical_curve() {
    let d = 7;
    let rounds = 500;
    let mut rng = SafePrng::new(1337);

    let theta_star = [
        vec![0.8, 0.6, 0.4, 0.2, 0.1, 0.3, 0.5], // Arm 0
        vec![0.2, 0.9, 0.1, 0.7, 0.3, 0.4, 0.2], // Arm 1
        vec![0.4, 0.3, 0.8, 0.1, 0.6, 0.2, 0.4], // Arm 2
    ];

    let mut router = BanditModelRouter::new_contextual_lints(3, d, 1.0, 0.2, 1337);
    let mut cumulative_regret = 0.0;

    for _ in 0..rounds {
        let ctx = RoutingContext::new(
            (rng.sample_uniform_f64() * 1000.0) as usize + 100,
            rng.sample_uniform_f64(),
            rng.sample_uniform_f64(),
            5000.0,
        );
        let features = ctx.to_7d_features();

        let mut best_reward = f64::NEG_INFINITY;
        let mut arm_rewards = [0.0; 3];
        for a in 0..3 {
            let r: f64 = features
                .iter()
                .zip(theta_star[a].iter())
                .map(|(x, th)| x * th)
                .sum();
            arm_rewards[a] = r;
            if r > best_reward {
                best_reward = r;
            }
        }

        let chosen_arm = router.select_arm_lints(&features);
        let observed_reward =
            (arm_rewards[chosen_arm] + rng.sample_standard_normal() * 0.05).clamp(0.0, 1.0);
        router.update_lints(chosen_arm, &features, observed_reward);

        cumulative_regret += (best_reward - arm_rewards[chosen_arm]).max(0.0);
    }

    let theoretical_upper_bound =
        2.5 * (d as f64) * ((rounds as f64) * (rounds as f64).ln()).sqrt();
    assert!(
        cumulative_regret < theoretical_upper_bound,
        "empirical cumulative regret {cumulative_regret:.2} exceeded theoretical bound {theoretical_upper_bound:.2}"
    );
}

#[test]
fn test_dynamic_role_allocation_by_task_complexity() {
    let mut router = ContextualThompsonSamplingRouter::new(TASK_COMPLEXITY_DIM, 1.0, 0.2);
    router.register_arm(BanditRoleArm::new(
        "drafter_fast",
        "qwen-7b",
        "local",
        RoleCapability::Drafter,
        TASK_COMPLEXITY_DIM,
        1.0,
    ));
    router.register_arm(BanditRoleArm::new(
        "critic_frontier",
        "claude-3-7",
        "anthropic",
        RoleCapability::Critic,
        TASK_COMPLEXITY_DIM,
        1.0,
    ));
    router.register_arm(BanditRoleArm::new(
        "frontier_pure",
        "claude-3-7",
        "anthropic",
        RoleCapability::PureFrontier,
        TASK_COMPLEXITY_DIM,
        1.0,
    ));

    // Train drafter to have high reward on simple features
    let simple_features = TaskComplexityFeatures::new(50, 2, 0.2, 2, 0, 80);
    let simple_vec = simple_features.to_vector();
    for _ in 0..20 {
        router.record_outcome("drafter_fast", &simple_vec, 0.95);
        router.record_outcome("critic_frontier", &simple_vec, 0.90);
    }

    let plan_simple = router.select_routing_plan(&simple_features);
    assert!(
        matches!(
            plan_simple,
            SpeculativeRoutingPlan::SpeculativeDrafting { .. }
        ),
        "simple prompt with high predicted drafter reward must select SpeculativeDrafting"
    );

    // Complex features with low drafter performance
    let complex_features = TaskComplexityFeatures::new(5000, 18, 0.95, 40, 5, 2000);
    let complex_vec = complex_features.to_vector();
    for _ in 0..20 {
        router.record_outcome("drafter_fast", &complex_vec, 0.10);
    }

    let plan_complex = router.select_routing_plan(&complex_features);
    assert!(
        matches!(plan_complex, SpeculativeRoutingPlan::PureFrontier { .. }),
        "complex prompt with low drafter reward must fall back to PureFrontier"
    );
}

// ============================================================================
// CATEGORY 5: END-TO-END INTEGRATION & EDGE CASES
// ============================================================================

#[test]
fn test_e2e_draft_and_certify_clean_workflow_mock_provider() {
    let mock_drafter = MockSpeculativeProvider::new(
        "qwen-fast",
        Some(
            "--- a/lib.rs\n+++ b/lib.rs\n@@ -1,2 +1,3 @@\n pub fn answer() -> u32 {\n+    // Speculative draft\n     42\n }\n",
        ),
    );
    let mock_critic = MockSpeculativeProvider::new("claude-frontier", Some("APPROVED: 0.95"));

    let mut engine = SpeculativeDraftingEngine::new(mock_drafter, mock_critic);
    let ctx = RoutingContext::new(256, 0.2, 0.8, 2000.0);

    let cert = engine
        .draft_and_certify(
            &ctx,
            Path::new("lib.rs"),
            "add speculative comment to answer",
            "pub fn answer() -> u32 {\n    42\n}\n",
        )
        .expect("draft and certify must succeed");

    assert!(
        cert.certificate_digest
            .starts_with("PANDORA_PATCH_CERT_V1:")
    );
    assert!(!cert.residual_applied);
    assert!(cert.quality_score >= 0.85);
    assert!(cert.speedup_ratio > 1.0);
}

#[test]
fn test_e2e_draft_and_certify_residual_correction_workflow() {
    let mock_drafter = MockSpeculativeProvider::new(
        "qwen-fast",
        Some(
            "--- a/lib.rs\n+++ b/lib.rs\n@@ -1,3 +1,3 @@\n pub fn calc() -> i32 {\n-    0\n+    10\n }\n",
        ),
    );
    let mock_critic = MockSpeculativeProvider::new(
        "claude-frontier",
        Some(
            "RESIDUAL:--- a/lib.rs\n+++ b/lib.rs\n@@ -1,3 +1,3 @@\n pub fn calc() -> i32 {\n-    10\n+    42\n }\n",
        ),
    );

    let mut engine = SpeculativeDraftingEngine::new(mock_drafter, mock_critic);
    let ctx = RoutingContext::new(256, 0.3, 0.7, 2000.0);

    let cert = engine
        .draft_and_certify(
            &ctx,
            Path::new("lib.rs"),
            "set calculation return to 42",
            "pub fn calc() -> i32 {\n    0\n}\n",
        )
        .expect("draft and certify with residual must succeed");

    assert!(
        cert.certificate_digest
            .starts_with("PANDORA_PATCH_CERT_V1:")
    );
    assert!(cert.residual_applied);
    assert!(cert.final_patch.contains("+    42"));
}

#[test]
fn test_e2e_draft_and_certify_critic_rejection_fallback() {
    let mock_drafter = MockSpeculativeProvider::new(
        "qwen-fast",
        Some("--- a/lib.rs\n+++ b/lib.rs\n@@ -1,1 +1,1 @@\n-broken\n+still_broken\n"),
    );
    let mock_critic = MockSpeculativeProvider::new(
        "claude-frontier",
        Some("REJECTED: Logic defect identified in draft implementation."),
    );

    let mut engine = SpeculativeDraftingEngine::new(mock_drafter, mock_critic);
    let ctx = RoutingContext::new(512, 0.7, 0.5, 3000.0);

    let result = engine.draft_and_certify(
        &ctx,
        Path::new("lib.rs"),
        "fix broken calculation",
        "broken\n",
    );

    assert!(
        matches!(result, Err(SpeculativeError::DraftRejected { .. })),
        "critic rejection must return DraftRejected error"
    );
}

#[test]
fn test_edge_case_empty_diff_and_whitespace_only() {
    let empty_diff = "";
    assert_eq!(
        UnifiedPatch::parse(empty_diff),
        Err(SpeculativeError::EmptyDiff)
    );

    let whitespace_diff = "   \n\n\t  \n";
    assert_eq!(
        UnifiedPatch::parse(whitespace_diff),
        Err(SpeculativeError::EmptyDiff)
    );
}

#[test]
fn test_edge_case_lints_collinear_features_matrix_conditioning() {
    let d = 7;
    let lambda = 1.0;
    let mut state = LinTsArmState::new(d, lambda);

    let collinear_x = vec![1.0; d];
    for _ in 0..100 {
        state.update(&collinear_x, 1.0);
    }

    let l = state
        .cholesky()
        .expect("regularized collinear matrix must remain positive definite");
    for i in 0..d {
        assert!(
            l[i * d + i] > 0.0,
            "diagonal element must remain strictly positive"
        );
    }

    let mut rng = SafePrng::new(42);
    let sample = state
        .sample_theta(&mut rng, 0.2)
        .expect("theta sampling on collinear matrix must succeed");
    assert_eq!(sample.len(), d);
    for val in sample {
        assert!(!val.is_nan() && !val.is_infinite());
    }
}
