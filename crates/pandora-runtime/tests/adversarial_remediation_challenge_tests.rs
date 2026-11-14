#![forbid(unsafe_code)]

//! Empirical Adversarial Challenge Test Suite for Tripartite Merge Conflict Resolver.
//!
//! Authored by challenger_p25_m2_rem_1 to empirically probe:
//! 1. Memory Safety Evasion Vectors (tokens, whitespaces, comments, attributes, decoy comments, false-positive safety).
//! 2. Governor Role Authorization (strict rejection of Critic, Generator, and Self-Approval with UnauthorizedApproverRole).
//! 3. Critic Delimiter Verification (interleaved delimiters, unclosed delimiters, string/comment masking resilience).
//! 4. Duplicate Item Detection (`E0428`) and Attempt 2 Self-Healing.
//! 5. Cryptographic Attestation Subject Binding & Byzantine Quorum.

use pandora_runtime::canary_worktree::{
    CanaryGateTelemetry, CanaryPromotionGate, CanaryStagePipeline, GitWorktreeManager,
    PromotionAttestationStatement,
};
use pandora_runtime::merge_resolver::{
    CriticEvaluation, ResolutionCandidate, ResolutionStrategy, TripartiteMergeResolver,
    TripartiteRole,
};
use pandora_runtime::multi_agent_collab::ExternalAgentKind;
use pandora_runtime::multi_agent_consensus::{AgentPatchProposal, ProposalScoreVector};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn create_temp_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    dir.push(format!("pandora_chal_m2_{name}_{nanos}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("failed to create temp test directory");
    dir
}

fn create_staged_canary_worktree(
    repo_root: &Path,
    canary_task: &str,
) -> (String, CanaryStagePipeline) {
    let cert = format!("PANDORA_PATCH_CERT_V1:{:064x}", 0xDEAD_BEEF_u64);
    let branch = format!("pandora/canary/{canary_task}");
    let mut worktree = GitWorktreeManager::create_worktree(repo_root, &branch, &cert)
        .expect("worktree creation must succeed");

    let patch = AgentPatchProposal::new(
        format!("patch_{canary_task}"),
        ExternalAgentKind::ClaudeCode,
        "Adversarial test patch",
        "Validates canary dual-gating invariants",
        "+++ b/math.rs\n+// Verified math\npub fn sqrt_safe(x: f64) -> Option<f64> { if x >= 0.0 { Some(x.sqrt()) } else { None } }\n",
        ProposalScoreVector::new(0.98, 0.97, 0.95, 0.99),
        100,
    );

    let pipeline = CanaryStagePipeline::default().with_repo_root(repo_root);
    let receipt = pipeline
        .stage_candidate_patch(&mut worktree, &patch)
        .expect("staging must succeed");

    (receipt.canary_id, pipeline)
}

// ============================================================================
// 1. Memory Safety Evasion Vectors
// ============================================================================

#[test]
fn test_probe_memory_safety_evasion_tokens_and_spacing() {
    let resolver = TripartiteMergeResolver::default();

    let evasion_snippets = vec![
        ("unsafe_brace_no_space", "fn f() { unsafe{ let _ = 1; } }"),
        (
            "unsafe_newline_brace",
            "fn f() {\n    unsafe\n    { let _ = 1; }\n}",
        ),
        ("unsafe_tab_brace", "fn f() {\n\tunsafe\t{ let _ = 1; }\n}"),
        (
            "unsafe_comment_brace",
            "fn f() { unsafe/*decoy*/{ let _ = 1; } }",
        ),
        (
            "unsafe_multiline_comment",
            "fn f() { unsafe/*c1/*c2*/c3*/{ let _ = 1; } }",
        ),
        ("pub_unsafe_fn", "pub unsafe fn dangerous_call() {}"),
        (
            "pub_crate_unsafe_fn",
            "pub(crate) unsafe fn internal_call() {}",
        ),
        ("unsafe_trait", "pub unsafe trait UnsafeContract {}"),
        ("unsafe_impl", "unsafe impl Send for MyStruct {}"),
        ("unsafe_extern_c", "unsafe extern \"C\" fn foreign_abi() {}"),
        (
            "allow_unsafe_attribute",
            "#[allow(unsafe_code)]\npub fn bypass() {}",
        ),
        (
            "allow_unsafe_with_spaces",
            "#[allow(  unsafe_code  )]\npub fn bypass() {}",
        ),
        (
            "warn_unsafe_attribute",
            "#[warn(unsafe_code)]\npub fn warn_bypass() {}",
        ),
        (
            "inner_allow_unsafe",
            "#![allow(unsafe_code)]\npub fn module_bypass() {}",
        ),
    ];

    for (label, snippet) in evasion_snippets {
        let candidate = ResolutionCandidate {
            file_path: PathBuf::from(format!("crates/{label}.rs")),
            proposed_by: TripartiteRole::Generator,
            resolved_content: snippet.to_string(),
            rationale: format!("Evasion probe: {label}"),
            strategy: ResolutionStrategy::AdditiveUnion,
        };

        let verified = resolver.governor_verify(&candidate);
        assert!(
            !verified,
            "Governor FAILED to reject memory safety evasion vector: {label}\nSnippet:\n{snippet}"
        );

        let mock_critic = CriticEvaluation {
            passed: true,
            diagnostics: Vec::new(),
            raw_compiler_output: String::new(),
            duration_ms: 0,
        };
        let verdict =
            resolver.governor_evaluate(TripartiteRole::Governor, &candidate, &mock_critic);
        assert!(!verdict.approved);
        assert!(
            verdict
                .reasons
                .iter()
                .any(|r| r.contains("forbid(unsafe_code)")),
            "Verdict for {label} must cite strict #![forbid(unsafe_code)] violation: {:?}",
            verdict.reasons
        );
    }
}

#[test]
fn test_probe_compound_attribute_with_unsafe_block() {
    let resolver = TripartiteMergeResolver::default();

    // When an attacker attempts a compound lint attribute containing unsafe_code alongside an unsafe block:
    let candidate = ResolutionCandidate {
        file_path: PathBuf::from("crates/compound_exploit.rs"),
        proposed_by: TripartiteRole::Generator,
        resolved_content:
            "#[allow(unused, unsafe_code)]\npub fn exploit() { unsafe { let _ = 1; } }".to_string(),
        rationale: "Compound attribute with unsafe block".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };

    assert!(
        !resolver.governor_verify(&candidate),
        "Compound allow attribute with unsafe block MUST BE REJECTED by Governor keyword scanner"
    );
}

#[test]
fn test_probe_memory_safety_decoy_comments_and_forbid_headers() {
    let resolver = TripartiteMergeResolver::default();

    let decoy_cases = vec![
        (
            "single_line_comment_decoy",
            "// forbid(unsafe_code) is strictly maintained!\npub fn f() { unsafe { 42; } }",
        ),
        (
            "nested_block_comment_decoy",
            "/* /* #![forbid(unsafe_code)] */ */\npub fn f() { unsafe { 42; } }",
        ),
        (
            "genuine_header_plus_injected_unsafe",
            "#![forbid(unsafe_code)]\npub fn legit() -> i32 { 1 }\npub fn f() { unsafe { 42; } }",
        ),
        (
            "comment_surrounded_unsafe",
            "/* allow(unsafe_code) */ pub fn f() { unsafe { 0; } }",
        ),
    ];

    for (label, snippet) in decoy_cases {
        let candidate = ResolutionCandidate {
            file_path: PathBuf::from(format!("crates/{label}.rs")),
            proposed_by: TripartiteRole::Generator,
            resolved_content: snippet.to_string(),
            rationale: format!("Decoy probe: {label}"),
            strategy: ResolutionStrategy::AdditiveUnion,
        };

        let verified = resolver.governor_verify(&candidate);
        assert!(
            !verified,
            "Governor FAILED to reject unsafe block despite decoy in {label}"
        );
    }
}

#[test]
fn test_probe_memory_safety_false_positive_resilience() {
    let resolver = TripartiteMergeResolver::default();

    // Legitimate code containing words with "unsafe" as sub-string or inside strings/comments
    let safe_cases = vec![
        (
            "ident_unsafe_mode",
            "pub struct Config {\n    pub unsafe_mode: bool,\n}",
        ),
        (
            "ident_is_unsafe",
            "pub fn is_unsafe_context() -> bool { false }",
        ),
        (
            "string_literal_unsafe",
            "pub fn log_msg() {\n    let _ = \"Found unsafe block in foreign file\";\n}",
        ),
        (
            "raw_string_unsafe",
            "pub fn raw_test() {\n    let _ = r#\"unsafe { raw_payload(); }\"#;\n}",
        ),
        (
            "comment_unsafe",
            "// This module explicitly avoids unsafe constructs\npub fn clean() -> u32 { 100 }",
        ),
    ];

    for (label, snippet) in safe_cases {
        let candidate = ResolutionCandidate {
            file_path: PathBuf::from(format!("crates/{label}.rs")),
            proposed_by: TripartiteRole::Generator,
            resolved_content: snippet.to_string(),
            rationale: format!("False-positive probe: {label}"),
            strategy: ResolutionStrategy::AdditiveUnion,
        };

        let verified = resolver.governor_verify(&candidate);
        assert!(
            verified,
            "Governor incorrectly rejected legitimate code with false-positive on {label}"
        );
    }
}

// ============================================================================
// 2. Governor Role Authorization & Self-Approval
// ============================================================================

#[test]
fn test_probe_governor_role_authorization_matrix() {
    let resolver = TripartiteMergeResolver::default();

    let candidate_by_gen = ResolutionCandidate {
        file_path: PathBuf::from("crates/service.rs"),
        proposed_by: TripartiteRole::Generator,
        resolved_content: "pub fn compute() -> i32 { 42 }".to_string(),
        rationale: "Generator proposal".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };

    let candidate_by_critic = ResolutionCandidate {
        file_path: PathBuf::from("crates/service.rs"),
        proposed_by: TripartiteRole::Critic,
        resolved_content: "pub fn compute() -> i32 { 42 }".to_string(),
        rationale: "Critic proposal".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };

    let candidate_by_gov = ResolutionCandidate {
        file_path: PathBuf::from("crates/service.rs"),
        proposed_by: TripartiteRole::Governor,
        resolved_content: "pub fn compute() -> i32 { 42 }".to_string(),
        rationale: "Governor proposal".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };

    let mock_passed_critic = CriticEvaluation {
        passed: true,
        diagnostics: Vec::new(),
        raw_compiler_output: String::new(),
        duration_ms: 0,
    };

    // 1. Critic attempting to approve candidate proposed by Generator
    let verdict_critic = resolver.governor_evaluate(
        TripartiteRole::Critic,
        &candidate_by_gen,
        &mock_passed_critic,
    );
    assert!(
        !verdict_critic.approved,
        "Critic MUST NOT be authorized to approve"
    );
    assert!(
        verdict_critic.reasons[0].contains("UnauthorizedApproverRole"),
        "Must cite UnauthorizedApproverRole, got: {}",
        verdict_critic.reasons[0]
    );

    // 2. Generator attempting to approve candidate proposed by Critic
    let verdict_gen = resolver.governor_evaluate(
        TripartiteRole::Generator,
        &candidate_by_critic,
        &mock_passed_critic,
    );
    assert!(
        !verdict_gen.approved,
        "Generator MUST NOT be authorized to approve"
    );
    assert!(
        verdict_gen.reasons[0].contains("UnauthorizedApproverRole"),
        "Must cite UnauthorizedApproverRole, got: {}",
        verdict_gen.reasons[0]
    );

    // 3. Self-approval: Generator approving Generator
    let verdict_gen_self = resolver.governor_evaluate(
        TripartiteRole::Generator,
        &candidate_by_gen,
        &mock_passed_critic,
    );
    assert!(
        !verdict_gen_self.approved,
        "Self-approval MUST be forbidden"
    );
    assert!(
        verdict_gen_self.reasons[0].contains("SelfApprovalForbidden"),
        "Must cite SelfApprovalForbidden, got: {}",
        verdict_gen_self.reasons[0]
    );

    // 4. Self-approval: Governor approving Governor candidate
    let verdict_gov_self = resolver.governor_evaluate(
        TripartiteRole::Governor,
        &candidate_by_gov,
        &mock_passed_critic,
    );
    assert!(
        !verdict_gov_self.approved,
        "Governor self-approval MUST be forbidden"
    );
    assert!(
        verdict_gov_self.reasons[0].contains("SelfApprovalForbidden"),
        "Must cite SelfApprovalForbidden, got: {}",
        verdict_gov_self.reasons[0]
    );

    // 5. Valid authorization: Governor approving candidate proposed by Generator
    let verdict_gov_valid = resolver.governor_evaluate(
        TripartiteRole::Governor,
        &candidate_by_gen,
        &mock_passed_critic,
    );
    assert!(
        verdict_gov_valid.approved,
        "Governor approving Generator candidate must succeed: {:?}",
        verdict_gov_valid.reasons
    );
}

// ============================================================================
// 3. Delimiter Verification in Critic
// ============================================================================

#[test]
fn test_probe_critic_interleaved_delimiters() {
    let resolver = TripartiteMergeResolver::default();
    let temp_dir = create_temp_dir("critic_interleaved");
    let file_path = PathBuf::from("crates/interleaved.rs");

    let interleaved_cases = vec![
        ("curly_paren", "fn foo() { ( } )", "E_UNBALANCED_PARENS"),
        ("bracket_curly", "fn bar() [ { ] }", "E_UNBALANCED_BRACES"),
        ("paren_bracket", "fn baz() ( [ ) ]", "E_UNBALANCED_BRACKETS"),
    ];

    for (label, content, expected_code) in interleaved_cases {
        let candidate = ResolutionCandidate {
            file_path: file_path.clone(),
            proposed_by: TripartiteRole::Generator,
            resolved_content: content.to_string(),
            rationale: format!("Interleaved probe: {label}"),
            strategy: ResolutionStrategy::AdditiveUnion,
        };

        let eval = resolver.critic_verify(&temp_dir, &file_path, &candidate);
        assert!(
            !eval.passed,
            "Critic FAILED to reject interleaved delimiters for {label}:\n{content}"
        );
        assert!(
            eval.diagnostics
                .iter()
                .any(|d| d.code.as_deref() == Some(expected_code)),
            "Expected diagnostic {expected_code} for {label}, got: {:?}",
            eval.diagnostics
        );
    }

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_probe_critic_unclosed_delimiters() {
    let resolver = TripartiteMergeResolver::default();
    let temp_dir = create_temp_dir("critic_unclosed");
    let file_path = PathBuf::from("crates/unclosed.rs");

    let unclosed_cases = vec![
        (
            "unclosed_bracket",
            "pub fn arr() {\n    let _ = [1, 2, 3;\n}",
            "E_UNBALANCED_BRACKETS",
        ),
        (
            "unclosed_brace",
            "pub fn block() {\n    let _ = 42;\n",
            "E_UNBALANCED_BRACES",
        ),
        (
            "unclosed_paren",
            "pub fn call() {\n    foo(1, 2;\n}",
            "E_UNBALANCED_PARENS",
        ),
    ];

    for (label, content, expected_code) in unclosed_cases {
        let candidate = ResolutionCandidate {
            file_path: file_path.clone(),
            proposed_by: TripartiteRole::Generator,
            resolved_content: content.to_string(),
            rationale: format!("Unclosed probe: {label}"),
            strategy: ResolutionStrategy::AdditiveUnion,
        };

        let eval = resolver.critic_verify(&temp_dir, &file_path, &candidate);
        assert!(
            !eval.passed,
            "Critic FAILED to reject unclosed delimiter for {label}:\n{content}"
        );
        assert!(
            eval.diagnostics
                .iter()
                .any(|d| d.code.as_deref() == Some(expected_code)),
            "Expected diagnostic {expected_code} for {label}, got: {:?}",
            eval.diagnostics
        );
    }

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_probe_critic_delimiters_in_strings_and_comments() {
    let resolver = TripartiteMergeResolver::default();
    let temp_dir = create_temp_dir("critic_strings_comments");
    let file_path = PathBuf::from("crates/strings_comments.rs");

    // All these snippets have balanced outer delimiters, but internal unbalanced delimiters
    // inside string literals, raw strings, character literals, and comments.
    let valid_snippets = vec![
        ("string_with_brace", "pub fn f() {\n    let _s = \"{\";\n}"),
        (
            "string_with_interleaved",
            "pub fn f() {\n    let _s = \"{ ( } )\";\n}",
        ),
        (
            "raw_string_with_brackets",
            "pub fn f() {\n    let _r = r#\" [ { ] } \"#;\n}",
        ),
        (
            "multi_hash_raw_string",
            "pub fn f() {\n    let _r = r##\" \" [ ( \" \"##;\n}",
        ),
        (
            "char_literals",
            "pub fn f() {\n    let c1 = '{';\n    let c2 = '}';\n    let c3 = '(';\n    let c4 = ')';\n    let c5 = '[';\n    let c6 = ']';\n}",
        ),
        (
            "line_comment_with_brackets",
            "pub fn f() {\n    // [ { (\n    let _x = 1;\n}",
        ),
        (
            "block_comment_with_brackets",
            "pub fn f() {\n    /* [ { ( */\n    let _x = 1;\n}",
        ),
        (
            "nested_block_comment",
            "pub fn f() {\n    /* outer /* inner { [ ( */ */\n    let _x = 1;\n}",
        ),
    ];

    for (label, content) in valid_snippets {
        let candidate = ResolutionCandidate {
            file_path: file_path.clone(),
            proposed_by: TripartiteRole::Generator,
            resolved_content: content.to_string(),
            rationale: format!("String/comment delimiter masking probe: {label}"),
            strategy: ResolutionStrategy::AdditiveUnion,
        };

        let eval = resolver.critic_verify(&temp_dir, &file_path, &candidate);
        assert!(
            eval.passed,
            "Critic falsely flagged masked delimiter in {label}:\nContent:\n{content}\nDiagnostics:\n{:?}",
            eval.diagnostics
        );
    }

    let _ = fs::remove_dir_all(&temp_dir);
}

// ============================================================================
// 4. Duplicate Item Detection (`E0428`) & Attempt 2 Self-Healing
// ============================================================================

#[test]
fn test_probe_duplicate_item_e0428_and_attempt2_healing() {
    let resolver = TripartiteMergeResolver::new(3);
    let temp_dir = create_temp_dir("duplicate_healing");
    let file_path = PathBuf::from("crates/models.rs");
    let full_path = temp_dir.join(&file_path);
    fs::create_dir_all(full_path.parent().unwrap()).unwrap();

    // 3-way conflict where both ours and theirs define the exact same function name
    let conflict_content = r#"
pub fn prelude() -> &'static str { "ok" }

<<<<<<< HEAD
pub fn get_endpoint_id() -> u32 { 101 }
=======
pub fn get_endpoint_id() -> u32 { 202 }
>>>>>>> canary

pub fn postlude() -> bool { true }
"#;

    fs::write(&full_path, conflict_content).unwrap();

    // Resolve file content
    let report = resolver
        .resolve_file_content(&file_path, conflict_content, &temp_dir)
        .expect("resolve_file_content must succeed");

    assert!(report.success, "Tripartite self-healing must succeed");
    assert_eq!(
        report.iterations_used, 2,
        "Resolution must succeed on Attempt 2 (DiagnosticRemediation after E0428 on Attempt 1)"
    );

    // Verify iteration 1 produced E0428 diagnostic
    let iter1 = &report.iteration_history[0];
    assert_eq!(iter1.strategy, ResolutionStrategy::AdditiveUnion);
    assert!(!iter1.critic_evaluation.passed);
    assert!(
        iter1
            .critic_evaluation
            .diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("E0428")),
        "Iteration 1 Critic must emit E0428 duplicate definition diagnostic"
    );

    // Verify iteration 2 used DiagnosticRemediation and passed
    let iter2 = &report.iteration_history[1];
    assert_eq!(iter2.strategy, ResolutionStrategy::DiagnosticRemediation);
    assert!(iter2.critic_evaluation.passed);
    assert!(iter2.governor_verdict.as_ref().unwrap().approved);

    // Verify on-disk file content has exactly one definition of `get_endpoint_id`
    let resolved_on_disk = fs::read_to_string(&full_path).unwrap();
    assert_eq!(
        resolved_on_disk.matches("pub fn get_endpoint_id").count(),
        1,
        "Resolved content must have exactly one definition of get_endpoint_id"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_probe_duplicate_struct_and_enum_healing() {
    let resolver = TripartiteMergeResolver::new(3);
    let temp_dir = create_temp_dir("duplicate_types");
    let file_path = PathBuf::from("crates/types.rs");
    let full_path = temp_dir.join(&file_path);
    fs::create_dir_all(full_path.parent().unwrap()).unwrap();

    let conflict_content = r#"
<<<<<<< HEAD
pub struct Payload { pub id: u64 }
=======
pub struct Payload { pub id: u64, pub name: String }
>>>>>>> canary
"#;

    fs::write(&full_path, conflict_content).unwrap();

    let report = resolver
        .resolve_file_content(&file_path, conflict_content, &temp_dir)
        .expect("resolve_file_content must succeed");

    assert!(report.success, "Struct duplicate conflict must succeed");
    assert_eq!(report.iterations_used, 2, "Must succeed on Attempt 2");
    assert!(
        report.iteration_history[0]
            .critic_evaluation
            .diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("E0428")),
        "Attempt 1 must catch duplicate struct Payload with E0428"
    );

    let resolved = fs::read_to_string(&full_path).unwrap();
    assert_eq!(
        resolved.matches("pub struct Payload").count(),
        1,
        "Duplicate struct Payload must be healed to single declaration"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

// ============================================================================
// 5. Attestation Subject Binding & Byzantine Quorum Verification
// ============================================================================

#[test]
fn test_probe_attestation_subject_tampering_rejected() {
    let repo_root = create_temp_dir("attestation_tamper_probe");
    let (canary_id, pipeline) = create_staged_canary_worktree(&repo_root, "subject_tamper");

    let telemetry = CanaryGateTelemetry::default();
    pipeline
        .record_canary_telemetry(&canary_id, &telemetry)
        .unwrap();

    let receipt = pipeline
        .promote_canary(&canary_id, "main", false)
        .expect("promotion should succeed");

    let worktrees = GitWorktreeManager::list_worktrees(&repo_root).unwrap();
    let wt = worktrees.iter().find(|w| w.id == canary_id).unwrap();

    let attestation_file = wt.path.join(".canary_promotion_attestation.json");
    let bytes = fs::read(&attestation_file).unwrap();
    let original_statement: PromotionAttestationStatement = serde_json::from_slice(&bytes).unwrap();

    // Baseline: genuine statement verifies
    assert!(original_statement.verify_signature());
    assert_eq!(original_statement.signature_hex, receipt.signature_hex);

    // Tamper 1: Modify subject commit hash
    let mut tampered_commit = original_statement.clone();
    tampered_commit.subject[0].digest.insert(
        "sha256".to_string(),
        "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
    );
    assert!(
        !tampered_commit.verify_signature(),
        "Tampered subject commit hash MUST FAIL verify_signature()"
    );

    // Tamper 2: Modify subject name
    let mut tampered_name = original_statement.clone();
    tampered_name.subject[0].name = "tampered_canary_id".to_string();
    assert!(
        !tampered_name.verify_signature(),
        "Tampered subject name MUST FAIL verify_signature()"
    );

    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn test_probe_byzantine_quorum_bounds() {
    // approvals > total_nodes MUST fail
    assert!(
        !CanaryPromotionGate::verify_byzantine_quorum(11, 10, 3),
        "verify_byzantine_quorum must reject approvals > total_nodes"
    );

    // approvals exactly 2f + 1 passes
    assert!(
        CanaryPromotionGate::verify_byzantine_quorum(7, 10, 3),
        "approvals == 2f + 1 (7 for f=3) must pass"
    );

    // approvals 2f fails
    assert!(
        !CanaryPromotionGate::verify_byzantine_quorum(6, 10, 3),
        "approvals == 2f (6 for f=3) must fail"
    );

    // total_nodes < 3f + 1 fails
    assert!(
        !CanaryPromotionGate::verify_byzantine_quorum(7, 9, 3),
        "total_nodes < 3f + 1 (9 < 10 for f=3) must fail"
    );
}
