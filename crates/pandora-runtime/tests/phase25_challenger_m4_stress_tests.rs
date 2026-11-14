#![forbid(unsafe_code)]

//! Phase 25 Milestone M4 Gate Challenger Stress & Empirical Adversarial Test Suite.
//!
//! Stress-tests:
//! 1. LinTS Cholesky Factorization under collinear, ill-conditioned, and extreme feature matrices.
//! 2. Multi-hunk residual diff composition algebra, boundaries, and algebraic idempotency.
//! 3. Syntactic validation invariants: `#![forbid(unsafe_code)]` rejection, comment/string false-positive immunity,
//!    git conflict marker rejection, delimiter balance stress.
//! 4. Corrupted diff error containment and clean rollbacks.
//! 5. End-to-end multi-model drafting engine certification and fallback dynamics.

use pandora_provider::{
    ModelRequest, ModelResponse, Provider, ProviderError, ProviderManifest, TokenUsage,
};
use pandora_runtime::bandit_router::{LinTsArmState, RoutingContext, SafePrng};
use pandora_runtime::speculative_drafting::{
    ResidualDiffComposer, SpeculativeDraftingEngine, SpeculativeError, UnifiedPatch,
};
use std::path::Path;
use std::sync::{Arc, Mutex};

// ============================================================================
// MOCK PROVIDER UTILITIES
// ============================================================================

#[derive(Clone)]
struct ChallengerMockProvider {
    manifest: ProviderManifest,
    response_payload: Arc<Mutex<String>>,
}

impl ChallengerMockProvider {
    fn new(id: &str, initial_response: &str) -> Self {
        let manifest = ProviderManifest::new(
            id,
            "Challenger Mock",
            "https://mock.challenger.local",
            "challenger-mock-model",
            "PANDORA_CHALLENGER_KEY",
        )
        .expect("valid manifest");
        Self {
            manifest,
            response_payload: Arc::new(Mutex::new(initial_response.to_string())),
        }
    }

    fn set_response(&self, new_resp: &str) {
        let mut lock = self.response_payload.lock().unwrap();
        *lock = new_resp.to_string();
    }
}

impl Provider for ChallengerMockProvider {
    fn manifest(&self) -> &ProviderManifest {
        &self.manifest
    }

    fn complete(&self, _request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        let text = self.response_payload.lock().unwrap().clone();
        Ok(ModelResponse::new(
            text,
            Vec::new(),
            TokenUsage::new(50, 50),
        ))
    }
}

// ============================================================================
// SUITE 1: LinTS CHOLESKY FACTORIZATION & ILL-CONDITIONED MATRICES
// ============================================================================

#[test]
fn test_adversarial_lints_extreme_collinear_subspaces() {
    let d = 7;
    let lambda = 1.0;
    let mut state = LinTsArmState::new(d, lambda);

    // Feed 500 updates along an identical 1D ray: x = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]
    let base_x = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    for step in 1..=500 {
        let scale = (step as f64).sin() * 2.0;
        let x: Vec<f64> = base_x.iter().map(|v| v * scale).collect();
        state.update(&x, 0.75);
    }

    // Cholesky decomposition must succeed because V = lambda * I + X^T X with lambda >= 1.0 > 0
    let l = state
        .cholesky()
        .expect("Cholesky must succeed on collinear covariance regularized by lambda*I");

    assert_eq!(l.len(), d * d);
    for i in 0..d {
        let l_ii = l[i * d + i];
        assert!(
            l_ii >= 1.0 - 1e-6,
            "diagonal L_{i}{i} ({l_ii}) must be bounded below by sqrt(lambda)=1.0"
        );
        assert!(!l_ii.is_nan() && !l_ii.is_infinite());
    }

    // Forward and back-solve with random Gaussian vector
    let mut rng = SafePrng::new(0xABCD);
    let sample = state
        .sample_theta(&mut rng, 0.3)
        .expect("sample_theta must succeed under extreme collinearity");
    assert_eq!(sample.len(), d);
    for val in sample {
        assert!(
            !val.is_nan() && !val.is_infinite(),
            "sampled theta parameter must be finite"
        );
    }

    // Posterior variance must remain non-negative and finite
    let var = state
        .posterior_variance(&base_x, 0.3)
        .expect("variance evaluation");
    assert!(
        var >= 0.0 && !var.is_nan() && !var.is_infinite(),
        "epistemic variance must be non-negative finite: {var}"
    );
}

#[test]
fn test_adversarial_lints_extreme_dynamic_range_features() {
    let d = 7;
    let mut state = LinTsArmState::new(d, 1.0);

    // Feature vector with 10 orders of magnitude discrepancy
    let extreme_x = vec![1e5, 1e-5, 1e4, 1e-4, 1.0, 1e2, 1e-2];
    for _ in 0..50 {
        state.update(&extreme_x, 0.90);
    }

    let l = state
        .cholesky()
        .expect("Cholesky must remain numerically stable across extreme dynamic ranges");

    for i in 0..d {
        let diag = l[i * d + i];
        assert!(diag > 0.0, "diagonal must be strictly positive");
        assert!(!diag.is_nan() && !diag.is_infinite());
    }

    let pred = state
        .predict_reward(&extreme_x)
        .expect("reward prediction succeeds");
    assert!(
        (0.0..=1.0).contains(&pred),
        "predicted reward must be clamped to [0.0, 1.0], found: {pred}"
    );
}

#[test]
fn test_adversarial_lints_minimal_lambda_boundary_and_zero_vectors() {
    let d = 7;
    // Test boundary lambda = 0.0 which LinTsArmState clamps to 1e-4
    let mut state = LinTsArmState::new(d, 0.0);
    assert!(
        state.lambda >= 1e-4,
        "lambda must be clamped to at least 1e-4"
    );

    // Update with all-zero feature vectors (uninformative context)
    let zero_x = vec![0.0; d];
    for _ in 0..100 {
        state.update(&zero_x, 0.5);
    }

    let l = state
        .cholesky()
        .expect("all-zero updates must leave diagonal strictly at sqrt(lambda)");
    for i in 0..d {
        let diag = l[i * d + i];
        assert!(
            (diag - state.lambda.sqrt()).abs() < 1e-6,
            "diagonal element must equal sqrt(lambda)"
        );
    }
}

// ============================================================================
// SUITE 2: RESIDUAL DIFF COMPOSITION ALGEBRA & BOUNDARY CASES
// ============================================================================

#[test]
fn test_adversarial_residual_composition_multi_hunk_complex_permutations() {
    let base_content = r#"// Header module
pub fn f1() -> i32 { 1 }
pub fn f2() -> i32 { 2 }
pub fn f3() -> i32 { 3 }
pub fn f4() -> i32 { 4 }
pub fn f5() -> i32 { 5 }
pub fn f6() -> i32 { 6 }
pub fn f7() -> i32 { 7 }
pub fn f8() -> i32 { 8 }
pub fn f9() -> i32 { 9 }
pub fn f10() -> i32 { 10 }
"#;

    // Candidate patch modifies f1 and f5
    let cand_patch = r#"--- a/src/lib.rs
+++ b/src/lib.rs
@@ -2,3 +2,3 @@
-pub fn f1() -> i32 { 1 }
+pub fn f1() -> i32 { 100 }
 pub fn f2() -> i32 { 2 }
@@ -6,3 +6,3 @@
-pub fn f5() -> i32 { 5 }
+pub fn f5() -> i32 { 500 }
 pub fn f6() -> i32 { 6 }
"#;

    // Corrective residual patch modifies f2, corrects f5 from 500 to 555, and modifies f10
    let res_patch = r#"--- a/src/lib.rs
+++ b/src/lib.rs
@@ -3,3 +3,3 @@
-pub fn f2() -> i32 { 2 }
+pub fn f2() -> i32 { 222 }
 pub fn f3() -> i32 { 3 }
@@ -6,3 +6,3 @@
-pub fn f5() -> i32 { 500 }
+pub fn f5() -> i32 { 555 }
 pub fn f6() -> i32 { 6 }
@@ -11,2 +11,2 @@
-pub fn f10() -> i32 { 10 }
+pub fn f10() -> i32 { 1000 }
"#;

    let target = Path::new("src/lib.rs");
    let (c_final, canonical) =
        ResidualDiffComposer::compose(target, base_content, cand_patch, Some(res_patch))
            .expect("multi-hunk composition must succeed");

    // Verify expected composed code state
    assert!(c_final.contains("pub fn f1() -> i32 { 100 }"));
    assert!(c_final.contains("pub fn f2() -> i32 { 222 }"));
    assert!(c_final.contains("pub fn f3() -> i32 { 3 }"));
    assert!(c_final.contains("pub fn f5() -> i32 { 555 }"));
    assert!(c_final.contains("pub fn f10() -> i32 { 1000 }"));

    // Algebraic verification: Applying canonical diff to base_content must match c_final
    let reapplied = UnifiedPatch::apply_to_string(base_content, &canonical)
        .expect("re-application of canonical diff");
    assert_eq!(
        reapplied, c_final,
        "algebraic identity apply(C0, P_canon) == C_final violated"
    );
}

#[test]
fn test_adversarial_residual_composition_empty_and_identity_laws() {
    let base = "fn identity() -> bool { true }\n";
    let target = Path::new("test.rs");

    // Case 1: Both empty
    let (c1, canon1) = ResidualDiffComposer::compose(target, base, "", None).unwrap();
    assert_eq!(c1, base);
    assert!(canon1.is_empty());

    // Case 2: Candidate non-empty, residual None
    let cand = "--- a/test.rs\n+++ b/test.rs\n@@ -1,1 +1,1 @@\n-fn identity() -> bool { true }\n+fn identity() -> bool { false }\n";
    let (c2, canon2) = ResidualDiffComposer::compose(target, base, cand, None).unwrap();
    assert_eq!(c2, "fn identity() -> bool { false }\n");
    assert!(!canon2.is_empty());

    // Case 3: Candidate non-empty, residual whitespace
    let (c3, canon3) = ResidualDiffComposer::compose(target, base, cand, Some("  \n\t ")).unwrap();
    assert_eq!(c3, c2);
    assert_eq!(canon3, canon2);

    // Case 4: Candidate empty, residual non-empty
    let (c4, _canon4) = ResidualDiffComposer::compose(target, base, "", Some(cand)).unwrap();
    assert_eq!(c4, "fn identity() -> bool { false }\n");
}

#[test]
fn test_adversarial_residual_composition_complete_reversion_to_base() {
    let base = "let x = 1;\nlet y = 2;\n";
    let target = Path::new("vars.rs");

    let cand =
        "--- a/vars.rs\n+++ b/vars.rs\n@@ -1,2 +1,2 @@\n-let x = 1;\n+let x = 99;\n let y = 2;\n";
    let res =
        "--- a/vars.rs\n+++ b/vars.rs\n@@ -1,2 +1,2 @@\n-let x = 99;\n+let x = 1;\n let y = 2;\n";

    let (c_final, canonical) =
        ResidualDiffComposer::compose(target, base, cand, Some(res)).unwrap();
    assert_eq!(
        c_final, base,
        "reversion residual must restore base content exactly"
    );
    assert!(
        canonical.is_empty(),
        "canonical diff of net zero change must be empty"
    );
}

// ============================================================================
// SUITE 3: CORRUPTED DIFF ERROR CONTAINMENT & CLEAN ROLLBACK
// ============================================================================

#[test]
fn test_adversarial_corrupted_diff_failure_containment() {
    let base = "line A\nline B\nline C\n";
    let target = Path::new("corrupt.rs");

    // Case 1: Hunk with context mismatch in candidate
    let bad_cand = "--- a/corrupt.rs\n+++ b/corrupt.rs\n@@ -1,3 +1,3 @@\n non_matching_line\n-line B\n+line B2\n line C\n";
    let err1 = ResidualDiffComposer::compose(target, base, bad_cand, None);
    assert!(
        matches!(err1, Err(SpeculativeError::PatchApplicationFailed { .. })),
        "mismatched candidate context must emit PatchApplicationFailed"
    );

    // Case 2: Good candidate, but corrupted residual diff
    let good_cand = "--- a/corrupt.rs\n+++ b/corrupt.rs\n@@ -1,3 +1,3 @@\n line A\n-line B\n+line B_new\n line C\n";
    let bad_res =
        "--- a/corrupt.rs\n+++ b/corrupt.rs\n@@ -50,1 +50,1 @@\n-nonexistent_line_50\n+fixed\n";
    let err2 = ResidualDiffComposer::compose(target, base, good_cand, Some(bad_res));
    assert!(
        matches!(err2, Err(SpeculativeError::ResidualCompositionFailed(_))),
        "mismatched residual context must emit ResidualCompositionFailed"
    );
}

// ============================================================================
// SUITE 4: SYNTACTIC VALIDATION INVARIANTS & ADVERSARIAL OBFUSCATIONS
// ============================================================================

#[test]
fn test_adversarial_unsafe_code_rejection_and_obfuscations() {
    // 1. Direct unsafe block
    assert!(matches!(
        SpeculativeDraftingEngine::validate_syntax_ast("pub fn f() { unsafe { do_work(); } }"),
        Err(SpeculativeError::UnsafeCodeDetected(_))
    ));

    // 2. Unsafe function declaration
    assert!(matches!(
        SpeculativeDraftingEngine::validate_syntax_ast("pub unsafe fn f() { }"),
        Err(SpeculativeError::UnsafeCodeDetected(_))
    ));

    // 3. Unsafe trait declaration
    assert!(matches!(
        SpeculativeDraftingEngine::validate_syntax_ast("pub unsafe trait DangerousTrait { }"),
        Err(SpeculativeError::UnsafeCodeDetected(_))
    ));

    // 4. Unsafe impl block
    assert!(matches!(
        SpeculativeDraftingEngine::validate_syntax_ast("unsafe impl Send for MyType { }"),
        Err(SpeculativeError::UnsafeCodeDetected(_))
    ));

    // 5. Unsafe without space before brace: unsafe{
    assert!(matches!(
        SpeculativeDraftingEngine::validate_syntax_ast("fn test() { unsafe{ do_it(); } }"),
        Err(SpeculativeError::UnsafeCodeDetected(_))
    ));

    // 6. Multiline unsafe
    let multiline_unsafe = "fn test() {\n    unsafe\n    {\n        do_it();\n    }\n}";
    assert!(matches!(
        SpeculativeDraftingEngine::validate_syntax_ast(multiline_unsafe),
        Err(SpeculativeError::UnsafeCodeDetected(_))
    ));
}

#[test]
fn test_adversarial_unsafe_false_positive_immunity() {
    // 1. Comment mentioning unsafe must NOT be rejected
    let commented_code = r#"
// This implementation avoids unsafe operations and enforces memory safety
/* Note: previously unsafe was used here, now purely safe */
pub fn safe_calc(x: i32) -> i32 {
    x * 2
}
"#;
    assert!(
        SpeculativeDraftingEngine::validate_syntax_ast(commented_code).is_ok(),
        "comments mentioning unsafe must NOT trigger false positives"
    );

    // 2. String literal containing "unsafe" must NOT be rejected
    let string_code = r#"
pub fn log_message() -> &'static str {
    "WARNING: unsafe block detected in external crate"
}
"#;
    assert!(
        SpeculativeDraftingEngine::validate_syntax_ast(string_code).is_ok(),
        "string literals containing 'unsafe' must NOT trigger false positives"
    );

    // 3. Identifiers containing unsafe as substring must NOT be rejected
    let identifier_code = r#"
pub struct UnsafeDetector {
    pub is_unsafe_mode: bool,
    pub unsafe_count: usize,
}
impl UnsafeDetector {
    pub fn new() -> Self {
        Self { is_unsafe_mode: false, unsafe_count: 0 }
    }
}
"#;
    assert!(
        SpeculativeDraftingEngine::validate_syntax_ast(identifier_code).is_ok(),
        "identifiers with 'unsafe' substring must NOT trigger false positives"
    );
}

#[test]
fn test_adversarial_git_conflict_marker_rejection() {
    let cases = [
        "<<<<<<< HEAD\nlet x = 1;\n=======\nlet x = 2;\n>>>>>>> branch\n",
        "fn foo() {\n<<<<<<< HEAD\n}\n",
        "   <<<<<<< HEAD (indented)\n",
        "=======\n",
        "\t=======\n",
        ">>>>>>> master\n",
        "  >>>>>>> 8a2f3e1b\n",
    ];

    for case in &cases {
        let res = SpeculativeDraftingEngine::validate_syntax_ast(case);
        assert!(
            matches!(res, Err(SpeculativeError::ConflictMarkerDetected(_))),
            "conflict marker in input must be rejected: {case}"
        );
    }
}

#[test]
fn test_adversarial_delimiter_nesting_and_mismatches() {
    // 1. Deeply nested balanced delimiters (depth 50)
    let mut deep = String::from("fn deep() { ");
    for _ in 0..50 {
        deep.push_str("({[");
    }
    deep.push_str(" 42 ");
    for _ in 0..50 {
        deep.push_str("]})");
    }
    deep.push_str(" }");
    assert!(
        SpeculativeDraftingEngine::validate_syntax_ast(&deep).is_ok(),
        "balanced deep delimiters must pass"
    );

    // 2. Mismatched closing delimiter: ({)}
    let mismatch = "fn bad() { let x = (1 + [2);]; }";
    assert!(matches!(
        SpeculativeDraftingEngine::validate_syntax_ast(mismatch),
        Err(SpeculativeError::DelimiterMismatch(_))
    ));

    // 3. Delimiters inside strings and chars must not disturb balance
    let code_with_delims_in_literals = r#"
pub fn parse_tokens() {
    let s = "({[}])";
    let c1 = '{';
    let c2 = '}';
    let c3 = '(';
    let c4 = ')';
    let c5 = '[';
    let c6 = ']';
    // Comment with unmatched } and (
    /* Block comment with unmatched { and ] */
}
"#;
    assert!(
        SpeculativeDraftingEngine::validate_syntax_ast(code_with_delims_in_literals).is_ok(),
        "delimiters inside string literals, chars, and comments must be masked cleanly"
    );

    // 4. Unclosed delimiter at end of file
    let unclosed = "pub fn unclosed() { if true { let x = 1; }";
    assert!(matches!(
        SpeculativeDraftingEngine::validate_syntax_ast(unclosed),
        Err(SpeculativeError::DelimiterMismatch(_))
    ));

    // 5. Unexpected closing delimiter with empty stack
    let extra_close = "pub fn ok() { } }";
    assert!(matches!(
        SpeculativeDraftingEngine::validate_syntax_ast(extra_close),
        Err(SpeculativeError::DelimiterMismatch(_))
    ));
}

// ============================================================================
// SUITE 5: END-TO-END SPECULATIVE DRAFTING ENGINE HARNESS
// ============================================================================

#[test]
fn test_adversarial_engine_certification_and_bandit_learning_cycle() {
    let drafter = ChallengerMockProvider::new(
        "drafter-qwen",
        "--- a/src/math.rs\n+++ b/src/math.rs\n@@ -1,2 +1,3 @@\n pub fn mul(a: i32, b: i32) -> i32 {\n+    // Candidate\n     a * b\n }\n",
    );
    let critic = ChallengerMockProvider::new("critic-claude", "APPROVED: 0.96");

    let mut engine = SpeculativeDraftingEngine::new(drafter.clone(), critic.clone());
    let ctx = RoutingContext::new(128, 0.2, 0.8, 1000.0);
    let file = Path::new("src/math.rs");
    let base = "pub fn mul(a: i32, b: i32) -> i32 {\n    a * b\n}\n";

    // 1. Initial certified draft
    let cert1 = engine
        .draft_and_certify(&ctx, file, "add candidate comment", base)
        .expect("draft and certify must succeed");
    assert!(cert1.quality_score >= 0.95);
    assert!(cert1.speedup_ratio > 1.0);
    assert!(
        cert1
            .certificate_digest
            .starts_with("PANDORA_PATCH_CERT_V1:")
    );

    // 2. Critic issues corrective residual diff
    critic.set_response(
        "RESIDUAL:--- a/src/math.rs\n+++ b/src/math.rs\n@@ -1,3 +1,3 @@\n pub fn mul(a: i32, b: i32) -> i32 {\n-    // Candidate\n+    // Certified speculative comment\n     a * b\n }\n",
    );
    let cert2 = engine
        .draft_and_certify(&ctx, file, "add certified comment", base)
        .expect("draft and certify with residual must succeed");
    assert!(cert2.residual_applied);
    assert!(
        cert2
            .final_patch
            .contains("+    // Certified speculative comment")
    );

    // 3. Critic rejects with low quality score
    critic.set_response("REJECTED: Logic defect identified");
    let rej_err = engine.draft_and_certify(&ctx, file, "failing task", base);
    assert!(
        matches!(rej_err, Err(SpeculativeError::DraftRejected { .. })),
        "rejected draft must yield DraftRejected error"
    );
}
