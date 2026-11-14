#![forbid(unsafe_code)]

//! Milestone M4 Empirical Challenge & Adversarial Stress Suite.
//!
//! Authored by Challenger `challenger_p25_m4_2` to independently challenge:
//! 1. Contextual Thompson Sampling Regret Bounds $R(T) < 2.5 \cdot d \sqrt{T \ln T}$ under stochastic & adversarial context sequences.
//! 2. LinTS Bayesian Epistemic Uncertainty Concentration ($\sigma^2(x) \to 0$).
//! 3. Dynamic role allocation spectrum across extreme task complexity feature vectors.
//! 4. Lexical AST scanner & Delimiter balancing stress testing (strings with delimiters, comments, escaped quotes).
//! 5. `#![forbid(unsafe_code)]` invariant enforcement (distinguishing strings vs genuine code).
//! 6. Residual Diff Composition Algebra ($P_{\text{cand}} \odot P_{\text{res}}$) with line shifts and clean rollback.
//! 7. End-to-end `draft_and_certify` mock provider pipeline, speedup ratio guarantees, and failure modes.
//! 8. Robustness against malformed diffs and pathological inputs (zero panics).

use pandora_provider::{
    ModelRequest, ModelResponse, Provider, ProviderError, ProviderManifest, TokenUsage,
};
use pandora_runtime::bandit_router::{
    BanditModelRouter, BanditRoleArm, ContextualThompsonSamplingRouter, LinTsArmState,
    RoleCapability, RoutingContext, SafePrng, SpeculativeRoutingPlan, TASK_COMPLEXITY_DIM,
    TaskComplexityFeatures,
};
use pandora_runtime::speculative_drafting::{
    ResidualDiffComposer, SpeculativeDraftingEngine, SpeculativeError, UnifiedPatch,
    mask_comments_and_strings,
};
use std::path::Path;
use std::sync::{Arc, Mutex};

// ============================================================================
// MOCK PROVIDER FOR ADVERSARIAL STRESS TESTING
// ============================================================================

#[derive(Clone)]
struct ChallengerMockProvider {
    manifest: ProviderManifest,
    canned_response: Arc<Mutex<Option<String>>>,
    call_counter: Arc<Mutex<usize>>,
    should_fail: Arc<Mutex<bool>>,
}

impl ChallengerMockProvider {
    fn new(id: &str, model: &str, canned: Option<&str>) -> Self {
        let manifest = ProviderManifest::new(
            id,
            "Challenger Mock Provider",
            "https://challenger.mock.local",
            model,
            "MOCK_KEY_P25_M4",
        )
        .expect("valid provider manifest");
        Self {
            manifest,
            canned_response: Arc::new(Mutex::new(canned.map(|s| s.to_string()))),
            call_counter: Arc::new(Mutex::new(0)),
            should_fail: Arc::new(Mutex::new(false)),
        }
    }

    fn set_should_fail(&self, fail: bool) {
        *self.should_fail.lock().unwrap() = fail;
    }

    #[allow(dead_code)]
    fn set_canned_response(&self, resp: &str) {
        *self.canned_response.lock().unwrap() = Some(resp.to_string());
    }

    fn call_count(&self) -> usize {
        *self.call_counter.lock().unwrap()
    }
}

impl Provider for ChallengerMockProvider {
    fn manifest(&self) -> &ProviderManifest {
        &self.manifest
    }

    fn complete(&self, _request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        let mut count = self.call_counter.lock().unwrap();
        *count += 1;

        if *self.should_fail.lock().unwrap() {
            return Err(ProviderError::Transport);
        }

        let resp_text = self
            .canned_response
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| "APPROVED: 0.95".to_string());

        Ok(ModelResponse::new(
            resp_text,
            Vec::new(),
            TokenUsage::new(120, 60),
        ))
    }
}

// ============================================================================
// CHALLENGE 1: REGRET BOUNDS UNDER MULTIPLE TIME HORIZONS & ADVERSARIAL NOISE
// ============================================================================

#[test]
fn test_challenger_regret_bound_multiple_horizons_and_dimensions() {
    let d = TASK_COMPLEXITY_DIM; // d = 7
    let mut rng = SafePrng::new(0x2026_1114_beef_1337);

    // True underlying unknown optimal parameter vectors for 3 arms
    let theta_arms = [
        vec![0.75, 0.50, 0.30, 0.40, 0.20, 0.10, 0.35], // Arm 0: generally superior
        vec![0.30, 0.80, 0.15, 0.25, 0.60, 0.30, 0.10], // Arm 1: specialized
        vec![0.20, 0.20, 0.20, 0.20, 0.20, 0.20, 0.20], // Arm 2: sub-optimal baseline
    ];

    // Evaluate across horizons: T = 50, 150, 300, 600
    for &horizon in &[50, 150, 300, 600] {
        let mut router =
            BanditModelRouter::new_contextual_lints(3, d, 1.0, 0.2, 0xcafe_0000 + horizon as u64);
        let mut cumulative_regret = 0.0;

        for _ in 0..horizon {
            // Generate non-trivial bounded feature vector x_t in [0, 1]^d with x_0 = 1.0
            let mut x = vec![1.0];
            for _ in 1..d {
                x.push(rng.sample_uniform_f64());
            }

            // Compute exact expected reward for each arm
            let mut best_arm_expected = f64::NEG_INFINITY;
            let mut expected_rewards = [0.0; 3];
            for a in 0..3 {
                let exp_r: f64 = x
                    .iter()
                    .zip(theta_arms[a].iter())
                    .map(|(xi, ti)| xi * ti)
                    .sum();
                expected_rewards[a] = exp_r;
                if exp_r > best_arm_expected {
                    best_arm_expected = exp_r;
                }
            }

            // Bandit arm selection
            let chosen = router.select_arm_lints(&x);

            // Add zero-mean Gaussian noise to observation
            let observation =
                (expected_rewards[chosen] + rng.sample_standard_normal() * 0.05).clamp(0.0, 1.0);
            router.update_lints(chosen, &x, observation);

            let instant_regret = (best_arm_expected - expected_rewards[chosen]).max(0.0);
            cumulative_regret += instant_regret;
        }

        let theoretical_bound =
            ContextualThompsonSamplingRouter::theoretical_regret_bound(d, horizon as u64);

        assert!(
            cumulative_regret < theoretical_bound,
            "Violation of regret bound at T={horizon}: cumulative regret {cumulative_regret:.3} >= bound {theoretical_bound:.3}"
        );

        // Average regret R(T)/T must satisfy R(T)/T < theoretical_bound / T
        let avg_regret = cumulative_regret / (horizon as f64);
        let max_avg_theoretical = theoretical_bound / (horizon as f64);
        assert!(
            avg_regret < max_avg_theoretical,
            "Average regret at T={horizon} ({avg_regret:.4}) exceeded theoretical bound ({max_avg_theoretical:.4})"
        );
    }
}

#[test]
fn test_challenger_regret_bound_adversarial_context_sequences() {
    let d = TASK_COMPLEXITY_DIM;
    let mut rng = SafePrng::new(0xdead_beef_0007);
    let horizon = 400;

    let mut router = BanditModelRouter::new_contextual_lints(3, d, 1.0, 0.2, 4242);
    let mut cumulative_regret = 0.0;

    let theta_arm_0 = [0.8, 0.1, 0.9, 0.1, 0.8, 0.1, 0.7];
    let theta_arm_1 = [0.1, 0.8, 0.1, 0.9, 0.1, 0.8, 0.2];
    let theta_arm_2 = [0.4, 0.4, 0.4, 0.4, 0.4, 0.4, 0.4];

    // Adversarial context sequence: flips sharply between complementary subspace features
    for t in 0..horizon {
        let mut x = vec![1.0];
        if t % 2 == 0 {
            // Emphasize even features (favors arm 0)
            x.extend_from_slice(&[0.9, 0.05, 0.85, 0.05, 0.9, 0.05]);
        } else {
            // Emphasize odd features (favors arm 1)
            x.extend_from_slice(&[0.05, 0.9, 0.05, 0.85, 0.05, 0.9]);
        }

        let r0: f64 = x.iter().zip(theta_arm_0.iter()).map(|(a, b)| a * b).sum();
        let r1: f64 = x.iter().zip(theta_arm_1.iter()).map(|(a, b)| a * b).sum();
        let r2: f64 = x.iter().zip(theta_arm_2.iter()).map(|(a, b)| a * b).sum();
        let best_r = r0.max(r1).max(r2);

        let chosen = router.select_arm_lints(&x);
        let actual_expected = match chosen {
            0 => r0,
            1 => r1,
            _ => r2,
        };

        let noisy_obs = (actual_expected + rng.sample_standard_normal() * 0.02).clamp(0.0, 1.0);
        router.update_lints(chosen, &x, noisy_obs);

        cumulative_regret += (best_r - actual_expected).max(0.0);
    }

    let theoretical_bound =
        ContextualThompsonSamplingRouter::theoretical_regret_bound(d, horizon as u64);
    assert!(
        cumulative_regret < theoretical_bound,
        "Adversarial alternating context sequence exceeded regret bound: R(T)={cumulative_regret:.2}, bound={theoretical_bound:.2}"
    );
}

// ============================================================================
// CHALLENGE 2: BAYESIAN EPISTEMIC UNCERTAINTY CONCENTRATION
// ============================================================================

#[test]
fn test_challenger_lints_epistemic_uncertainty_concentration() {
    let d = 5;
    let lambda = 1.0;
    let mut state = LinTsArmState::new(d, lambda);
    let direction_x = vec![1.0, 0.5, 0.2, 0.1, 0.8];

    // Initial variance before observations
    let initial_var = state
        .posterior_variance(&direction_x, 0.5)
        .expect("initial variance calculation succeeds");

    // Repeatedly update along direction_x
    for _ in 0..100 {
        state.update(&direction_x, 0.9);
    }

    let updated_var = state
        .posterior_variance(&direction_x, 0.5)
        .expect("updated variance calculation succeeds");

    assert!(
        updated_var < initial_var * 0.05,
        "Posterior epistemic variance must contract dramatically with repeated observations: initial={initial_var:.5}, updated={updated_var:.5}"
    );

    // Orthogonal direction should retain higher variance
    let orthogonal_x = vec![0.0, 0.0, 0.0, 1.0, 0.0];
    let orth_var = state
        .posterior_variance(&orthogonal_x, 0.5)
        .expect("orthogonal variance calculation succeeds");

    assert!(
        orth_var > updated_var,
        "Unexplored orthogonal direction should retain higher uncertainty than heavily sampled direction: orth_var={orth_var:.5}, updated_var={updated_var:.5}"
    );
}

// ============================================================================
// CHALLENGE 3: DYNAMIC ROLE ALLOCATION SPECTRUM
// ============================================================================

#[test]
fn test_challenger_dynamic_role_allocation_spectrum() {
    let mut router = ContextualThompsonSamplingRouter::new(TASK_COMPLEXITY_DIM, 1.0, 0.2);
    router.register_arm(BanditRoleArm::new(
        "qwen_drafter",
        "qwen-2.5-coder-7b",
        "local",
        RoleCapability::Drafter,
        TASK_COMPLEXITY_DIM,
        1.0,
    ));
    router.register_arm(BanditRoleArm::new(
        "claude_critic",
        "claude-3-7-sonnet",
        "anthropic",
        RoleCapability::Critic,
        TASK_COMPLEXITY_DIM,
        1.0,
    ));
    router.register_arm(BanditRoleArm::new(
        "claude_frontier",
        "claude-3-7-sonnet",
        "anthropic",
        RoleCapability::PureFrontier,
        TASK_COMPLEXITY_DIM,
        1.0,
    ));

    // Case A: Trivial task (e.g. 15 lines of Markdown / TOML, no ast depth, no errors)
    let trivial_features = TaskComplexityFeatures::new(15, 1, 0.15, 0, 0, 40);
    let trivial_vec = trivial_features.to_vector();
    for _ in 0..15 {
        router.record_outcome("qwen_drafter", &trivial_vec, 0.95);
        router.record_outcome("claude_critic", &trivial_vec, 0.92);
        router.record_outcome("claude_frontier", &trivial_vec, 0.80);
    }
    let plan_trivial = router.select_routing_plan(&trivial_features);
    assert!(
        matches!(
            plan_trivial,
            SpeculativeRoutingPlan::SpeculativeDrafting { .. }
        ),
        "Trivial documentation/config edit must route to SpeculativeDrafting"
    );

    // Case B: Huge monolith task with errors & deep AST
    let monster_features = TaskComplexityFeatures::new(8000, 24, 0.95, 50, 8, 3000);
    let monster_vec = monster_features.to_vector();
    for _ in 0..15 {
        router.record_outcome("qwen_drafter", &monster_vec, 0.12);
        router.record_outcome("claude_critic", &monster_vec, 0.50);
        router.record_outcome("claude_frontier", &monster_vec, 0.90);
    }
    let plan_monster = router.select_routing_plan(&monster_features);
    assert!(
        matches!(plan_monster, SpeculativeRoutingPlan::PureFrontier { .. }),
        "Massive, high-complexity monolithic task with low drafter accuracy must route to PureFrontier"
    );
}

// ============================================================================
// CHALLENGE 4: AST SCANNER, COMMENT MASKING & INVARIANT VALIDATION STRESS
// ============================================================================

#[test]
fn test_challenger_mask_comments_and_strings_complex_delimiters() {
    let source_with_delimiters_in_strings_and_comments = r#"
// A comment with unmatched delimiter { ( [
pub fn valid_function() -> &'static str {
    /* Block comment with { [ ( */
    let s = "String with delimiters { [ ( and escaped quote \" inside } ] )";
    let c = '{';
    let d = '}';
    s
}
"#;

    let _masked = mask_comments_and_strings(source_with_delimiters_in_strings_and_comments);
    assert!(!_masked.is_empty());

    // Inside masked output, delimiters within strings and comments must be replaced with spaces
    // while keeping the outer delimiters intact
    let validation = SpeculativeDraftingEngine::validate_syntax_ast(
        source_with_delimiters_in_strings_and_comments,
    );
    assert!(
        validation.is_ok(),
        "Delimiters inside strings and comments must not cause delimiter mismatch failures: {:?}",
        validation.err()
    );
}

#[test]
fn test_challenger_forbid_unsafe_code_precision() {
    // 1. Literal string "unsafe" must be tolerated
    let code_with_string_unsafe = r#"
pub fn log_status() -> &'static str {
    let message = "This operation is unsafe according to guidelines";
    message
}
"#;
    let res1 = SpeculativeDraftingEngine::validate_syntax_ast(code_with_string_unsafe);
    assert!(
        res1.is_ok(),
        "Literal string containing 'unsafe' should not trigger UnsafeCodeDetected: {:?}",
        res1.err()
    );

    // 2. Genuine unsafe block must be strictly blocked
    let genuine_unsafe_block = r#"
pub fn dangerous_transmute(x: u64) -> f64 {
    unsafe {
        std::mem::transmute(x)
    }
}
"#;
    let res2 = SpeculativeDraftingEngine::validate_syntax_ast(genuine_unsafe_block);
    assert!(
        matches!(res2, Err(SpeculativeError::UnsafeCodeDetected(_))),
        "Genuine unsafe block must be rejected immediately"
    );

    // 3. Genuine unsafe fn must be strictly blocked
    let genuine_unsafe_fn = r#"
pub unsafe fn raw_deref(ptr: *const u8) -> u8 {
    *ptr
}
"#;
    let res3 = SpeculativeDraftingEngine::validate_syntax_ast(genuine_unsafe_fn);
    assert!(
        matches!(res3, Err(SpeculativeError::UnsafeCodeDetected(_))),
        "Genuine unsafe function declaration must be rejected immediately"
    );
}

#[test]
fn test_challenger_conflict_marker_rejection() {
    let code_with_conflict = r#"
pub fn merged() -> i32 {
<<<<<<< HEAD
    10
=======
    20
>>>>>>> feature-branch
}
"#;
    let res = SpeculativeDraftingEngine::validate_syntax_ast(code_with_conflict);
    assert!(
        matches!(res, Err(SpeculativeError::ConflictMarkerDetected(_))),
        "Git conflict markers must be detected and rejected"
    );
}

// ============================================================================
// CHALLENGE 5: RESIDUAL DIFF COMPOSITION ALGEBRA & MULTI-HUNK LINE SHIFTS
// ============================================================================

#[test]
fn test_challenger_residual_diff_multi_hunk_line_shifts() {
    let base = r#"line 1
line 2
line 3
line 4
line 5
line 6
line 7
line 8
"#;

    // Candidate patch adds 2 lines at line 2
    let candidate = r#"--- a/file.txt
+++ b/file.txt
@@ -2,2 +2,4 @@
 line 2
+added draft A
+added draft B
 line 3
"#;

    // Corrective residual modifies line 6 (which has shifted down by 2 lines in c_draft!)
    let residual = r#"--- a/file.txt
+++ b/file.txt
@@ -7,2 +7,3 @@
 line 6
+added residual C
 line 7
"#;

    let target = Path::new("file.txt");
    let (c_final, canonical_diff) =
        ResidualDiffComposer::compose(target, base, candidate, Some(residual))
            .expect("multi-hunk shifted residual composition should succeed");

    assert!(c_final.contains("added draft A"));
    assert!(c_final.contains("added draft B"));
    assert!(c_final.contains("added residual C"));
    assert!(canonical_diff.contains("+added draft A"));
    assert!(canonical_diff.contains("+added residual C"));
}

#[test]
fn test_challenger_residual_diff_conflict_clean_rollback() {
    let base = "fn foo() -> i32 { 1 }\n";

    let candidate = r#"--- a/lib.rs
+++ b/lib.rs
@@ -1,1 +1,1 @@
-fn foo() -> i32 { 1 }
+fn foo() -> i32 { 2 }
"#;

    // Conflicting residual diff with invalid context line
    let conflicting_residual = r#"--- a/lib.rs
+++ b/lib.rs
@@ -1,1 +1,1 @@
-non_existent_context_line
+fn foo() -> i32 { 3 }
"#;

    let target = Path::new("lib.rs");
    let result = ResidualDiffComposer::compose(target, base, candidate, Some(conflicting_residual));

    assert!(
        matches!(result, Err(SpeculativeError::ResidualCompositionFailed(_))),
        "Conflicting residual must fail with ResidualCompositionFailed"
    );
}

// ============================================================================
// CHALLENGE 6: END-TO-END PIPELINE GUARANTEES & FAILURE MODES
// ============================================================================

#[test]
fn test_challenger_e2e_pipeline_full_workflow_and_certificate_properties() {
    let drafter = ChallengerMockProvider::new(
        "mock-drafter",
        "drafter-model",
        Some(
            "--- a/src/math.rs\n+++ b/src/math.rs\n@@ -1,2 +1,3 @@\n pub fn add(a: i32, b: i32) -> i32 {\n+    // certified fast add\n     a + b\n }\n",
        ),
    );
    let critic = ChallengerMockProvider::new("mock-critic", "critic-model", Some("APPROVED: 0.98"));

    let mut engine = SpeculativeDraftingEngine::new(drafter.clone(), critic.clone());
    let ctx = RoutingContext::new(256, 0.1, 0.9, 2500.0);
    let base = "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n";

    let cert = engine
        .draft_and_certify(&ctx, Path::new("src/math.rs"), "add comment", base)
        .expect("e2e draft and certify succeeds");

    assert!(
        cert.certificate_digest
            .starts_with("PANDORA_PATCH_CERT_V1:")
    );
    assert_eq!(
        cert.certificate_digest.len(),
        "PANDORA_PATCH_CERT_V1:".len() + 64
    );
    assert_eq!(cert.quality_score, 0.98);
    assert!(cert.speedup_ratio > 1.0);
    assert!(!cert.residual_applied);

    // Verify provider calls
    assert_eq!(drafter.call_count(), 1);
    assert_eq!(critic.call_count(), 1);
}

#[test]
fn test_challenger_e2e_pipeline_provider_failure_resilience() {
    let drafter = ChallengerMockProvider::new("mock-drafter", "drafter-model", None);
    let critic = ChallengerMockProvider::new("mock-critic", "critic-model", None);

    // Drafter suffers network error
    drafter.set_should_fail(true);

    let mut engine = SpeculativeDraftingEngine::new(drafter, critic);
    let ctx = RoutingContext::new(256, 0.1, 0.9, 2500.0);
    let base = "fn test() {}\n";

    let result = engine.draft_and_certify(&ctx, Path::new("test.rs"), "do work", base);
    assert!(
        matches!(result, Err(SpeculativeError::ProviderExecutionFailed(_))),
        "Provider network failure must be captured as ProviderExecutionFailed error without panicking"
    );
}

#[test]
fn test_challenger_e2e_pipeline_draft_syntax_rejection_before_certification() {
    // Drafter generates syntax with unbalanced brace
    let drafter = ChallengerMockProvider::new(
        "mock-drafter",
        "drafter-model",
        Some(
            "--- a/test.rs\n+++ b/test.rs\n@@ -1,1 +1,2 @@\n fn test() {}\n+pub fn broken() { if true { }\n",
        ),
    );
    let critic = ChallengerMockProvider::new(
        "mock-critic",
        "critic-model",
        Some("APPROVED: 0.99"), // Critic mistakenly approves malformed syntax
    );

    let mut engine = SpeculativeDraftingEngine::new(drafter, critic);
    let ctx = RoutingContext::new(128, 0.2, 0.8, 1500.0);
    let base = "fn test() {}\n";

    let result = engine.draft_and_certify(&ctx, Path::new("test.rs"), "syntax test", base);
    assert!(
        matches!(result, Err(SpeculativeError::DelimiterMismatch(_))),
        "Syntactically malformed certified content must be rejected during post-composition verification even if critic approved"
    );
}

// ============================================================================
// CHALLENGE 7: PATHOLOGICAL & MALFORMED DIFF FUZZING (ZERO PANIC GUARANTEE)
// ============================================================================

#[test]
fn test_challenger_malformed_diffs_zero_panic_guarantee() {
    let pathological_inputs = [
        "@@ -1,1 +1,1 @@",
        "@@ invalid header @@\n+test",
        "--- a/\n+++ b/\n@@ -0,0 +0,0 @@\n",
        "@@ -999999999,999999999 +999999999,999999999 @@\n",
        "--- a/f\n+++ b/f\n@@ -1,1 +1,1 @@\n-non_matching\n",
        "\x00\x01\x02\x03\x04\x05",
        "--- a/\u{1F980}\n+++ b/\u{1F980}\n@@ -1,1 +1,1 @@\n+\u{1F980}\n",
    ];

    for input in &pathological_inputs {
        // Parsing should either succeed or cleanly return an error — NEVER panic
        let res = UnifiedPatch::parse(input);
        if let Ok(patch) = res {
            let _ = patch.apply("sample base text\n");
        }
    }
}
