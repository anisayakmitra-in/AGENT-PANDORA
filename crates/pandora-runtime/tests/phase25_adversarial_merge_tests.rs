#![forbid(unsafe_code)]

//! Adversarial Stress-Tests for Tripartite Merge Conflict Resolver.
//!
//! Evaluates the 6 adversarial challenge dimensions:
//! 1. Complex 2-way and 3-way merge conflicts with overlapping edits and multiple hunks.
//! 2. Mismatched delimiters, unbalanced braces/parentheses/brackets, and literal/comment traps.
//! 3. Candidate size overflow (>2MB boundary).
//! 4. Injection of unsafe blocks, `#![allow(unsafe_code)]`, and bypass evasion.
//! 5. Generator / Critic attempting self-approval (`SelfApprovalForbidden` error).
//! 6. Iteration exhaustion beyond K=3 feedback loop attempts and clean rollback.

use pandora_runtime::merge_resolver::{
    CriticEvaluation, ResolutionCandidate, ResolutionStrategy, TripartiteMergeResolver,
    TripartiteRole,
};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn create_temp_test_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    dir.push(format!("pandora_adv_merge_test_{name}_{nanos}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("failed to create temp test directory");
    dir
}

// ============================================================================
// Dimension 1: Complex 2-way and 3-way merge conflicts with overlapping edits
// ============================================================================

#[test]
fn test_adv_2way_conflict_parsing() {
    let resolver = TripartiteMergeResolver::default();
    let file = PathBuf::from("src/model.rs");
    let content = r#"
pub fn prefix() {}
<<<<<<< HEAD
pub fn value() -> u32 { 10 }
=======
pub fn value() -> u32 { 20 }
>>>>>>> canary
pub fn suffix() {}
"#;
    let hunks = resolver.parse_conflict_hunks(&file, content);
    assert_eq!(hunks.len(), 1);
    assert_eq!(hunks[0].base, ""); // 2-way has empty base
    assert!(hunks[0].ours.contains("10"));
    assert!(hunks[0].theirs.contains("20"));
    assert_eq!(hunks[0].ours_label, "HEAD");
    assert_eq!(hunks[0].theirs_label, "canary");
}

#[test]
fn test_adv_3way_conflict_parsing() {
    let resolver = TripartiteMergeResolver::default();
    let file = PathBuf::from("src/engine.rs");
    let content = r#"
<<<<<<< HEAD
pub fn compute() -> i32 { 1 }
||||||| base
pub fn compute() -> i32 { 0 }
=======
pub fn compute() -> i32 { 2 }
>>>>>>> feat
"#;
    let hunks = resolver.parse_conflict_hunks(&file, content);
    assert_eq!(hunks.len(), 1);
    assert_eq!(hunks[0].base.trim(), "pub fn compute() -> i32 { 0 }");
    assert_eq!(hunks[0].ours.trim(), "pub fn compute() -> i32 { 1 }");
    assert_eq!(hunks[0].theirs.trim(), "pub fn compute() -> i32 { 2 }");
}

#[test]
fn test_adv_multiple_sequential_conflict_hunks() {
    let resolver = TripartiteMergeResolver::default();
    let file = PathBuf::from("src/multi.rs");
    let content = r#"
use std::fmt;

<<<<<<< HEAD
use std::collections::BTreeMap;
=======
use std::collections::HashMap;
>>>>>>> canary

pub fn a() -> u32 { 1 }

<<<<<<< HEAD
pub fn b() -> u32 { 10 }
=======
pub fn b() -> u32 { 20 }
>>>>>>> canary

pub fn end() {}
"#;
    let hunks = resolver.parse_conflict_hunks(&file, content);
    assert_eq!(hunks.len(), 2, "Must accurately parse multiple hunks");

    let candidate =
        resolver.synthesize_candidate(&file, content, &hunks, ResolutionStrategy::PreferOurs, &[]);
    assert!(!candidate.resolved_content.contains("<<<<<<<"));
    assert!(!candidate.resolved_content.contains(">>>>>>>"));
    assert!(candidate.resolved_content.contains("BTreeMap"));
    assert!(candidate.resolved_content.contains("b() -> u32 { 10 }"));
}

#[test]
fn test_adv_resolve_file_content_overlapping_edits_duplicate_emission() {
    let resolver = TripartiteMergeResolver::default();
    let repo_root = create_temp_test_dir("dup_emission");
    let file_path = repo_root.join("dup_test.rs");

    // Overlapping edits where both branches define conflicting versions of `pub fn version() -> u32`
    let conflict_content = r#"
pub fn preamble() -> bool { true }
<<<<<<< HEAD
pub fn version() -> u32 { 1 }
=======
pub fn version() -> u32 { 2 }
>>>>>>> canary
pub fn postamble() -> bool { true }
"#;
    fs::write(&file_path, conflict_content).unwrap();

    let report = resolver
        .resolve_file_content(&file_path, conflict_content, &repo_root)
        .expect("resolve_file_content must return Ok report");

    // EMPIRICAL VERIFICATION OF REMEDIATION:
    // Attempt 1 emits duplicate item under AdditiveUnion.
    // Critic detects duplicate function and emits E0428.
    // Attempt 2 selects DiagnosticRemediation and deduplicates pub fn version() -> u32.
    assert!(report.success, "Self-healing resolution must succeed");
    assert_eq!(
        report.iterations_used, 2,
        "Must heal on Attempt 2 via E0428 feedback"
    );

    let written = fs::read_to_string(&file_path).unwrap();
    let version_count = written.matches("pub fn version() -> u32").count();
    println!("Duplicate function count in resolved file: {version_count}");
    assert_eq!(
        version_count, 1,
        "Duplicate definition must be cleanly deduplicated"
    );

    let _ = fs::remove_dir_all(&repo_root);
}

// ============================================================================
// Dimension 2: Mismatched delimiters, unbalanced braces/parentheses/brackets
// ============================================================================

#[test]
fn test_adv_critic_rejects_unbalanced_curly_braces() {
    let resolver = TripartiteMergeResolver::default();
    let repo_root = create_temp_test_dir("unbalanced_braces");
    let file = PathBuf::from("src/bad_syntax.rs");

    // Missing closing brace
    let candidate = ResolutionCandidate {
        file_path: file.clone(),
        proposed_by: TripartiteRole::Generator,
        resolved_content: "pub fn broken() { let x = 42;".to_string(),
        rationale: "Broken syntax candidate".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };

    let eval = resolver.critic_verify(&repo_root, &file, &candidate);
    assert!(!eval.passed, "Critic must reject unbalanced curly braces");
    assert!(
        eval.diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("E_UNBALANCED_BRACES"))
    );

    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn test_adv_critic_rejects_unbalanced_parentheses() {
    let resolver = TripartiteMergeResolver::default();
    let repo_root = create_temp_test_dir("unbalanced_parens");
    let file = PathBuf::from("src/bad_parens.rs");

    // Missing closing parenthesis
    let candidate = ResolutionCandidate {
        file_path: file.clone(),
        proposed_by: TripartiteRole::Generator,
        resolved_content: "pub fn compute(a: i32, b: i32 -> i32 { a + b }".to_string(),
        rationale: "Broken parens candidate".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };

    let eval = resolver.critic_verify(&repo_root, &file, &candidate);
    assert!(!eval.passed, "Critic must reject unbalanced parentheses");
    assert!(
        eval.diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("E_UNBALANCED_PARENS"))
    );

    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn test_adv_critic_delimiters_behavior_empirical() {
    let resolver = TripartiteMergeResolver::default();
    let repo_root = create_temp_test_dir("delimiters_probe");
    let file = PathBuf::from("src/probe.rs");

    // Delimiters probe 1: Square brackets unclosed `[1, 2, 3;`
    let square_brackets_candidate = ResolutionCandidate {
        file_path: file.clone(),
        proposed_by: TripartiteRole::Generator,
        resolved_content: "pub fn arr() { let _arr = [1, 2, 3; }".to_string(),
        rationale: "Unclosed square bracket".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };
    let eval_sq = resolver.critic_verify(&repo_root, &file, &square_brackets_candidate);
    // Critic checks square brackets and rejects unclosed:
    assert!(
        !eval_sq.passed,
        "Critic must reject unclosed square bracket []"
    );
    assert!(
        eval_sq
            .diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("E_UNBALANCED_BRACKETS"))
    );

    // Delimiters probe 2: Interleaved delimiters `{ ( } )`
    let interleaved_candidate = ResolutionCandidate {
        file_path: file.clone(),
        proposed_by: TripartiteRole::Generator,
        resolved_content: "pub fn inter() { if (true } ) { }".to_string(),
        rationale: "Interleaved delimiters".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };
    let eval_inter = resolver.critic_verify(&repo_root, &file, &interleaved_candidate);
    // Critic delimiter stack rejects interleaved delimiters:
    assert!(
        !eval_inter.passed,
        "Critic must reject interleaved delimiters {{ ( }} )"
    );

    // Delimiters probe 3: Curly brace inside string literal
    let string_literal_candidate = ResolutionCandidate {
        file_path: file.clone(),
        proposed_by: TripartiteRole::Generator,
        resolved_content: "pub fn print_brace() { let _s = \"{\"; }".to_string(),
        rationale: "Brace in string literal".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };
    let eval_lit = resolver.critic_verify(&repo_root, &file, &string_literal_candidate);
    // Critic masks string literals, accepting valid string literals containing braces:
    assert!(
        eval_lit.passed,
        "Critic must accept valid string literal containing brace"
    );

    let _ = fs::remove_dir_all(&repo_root);
}

// ============================================================================
// Dimension 3: Candidate size overflow (>2MB)
// ============================================================================

#[test]
fn test_adv_candidate_size_boundary() {
    let resolver = TripartiteMergeResolver::default();
    assert_eq!(resolver.max_file_size_bytes, 2 * 1024 * 1024);

    let base_content = "pub fn dummy() {}\n";

    // Sub-test 3A: Candidate exactly at 2MB limit (2,097,152 bytes)
    let exact_limit = 2 * 1024 * 1024;
    let mut exact_content = String::with_capacity(exact_limit);
    exact_content.push_str("// safe comment\n");
    while exact_content.len() + base_content.len() <= exact_limit {
        exact_content.push_str(base_content);
    }
    while exact_content.len() < exact_limit {
        exact_content.push(' ');
    }
    assert_eq!(exact_content.len(), exact_limit);

    let candidate_exact = ResolutionCandidate {
        file_path: PathBuf::from("crates/exact.rs"),
        proposed_by: TripartiteRole::Generator,
        resolved_content: exact_content,
        rationale: "Exact 2MB candidate".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };
    assert!(
        resolver.governor_verify(&candidate_exact),
        "Candidate at exact 2MB limit must be accepted by Governor"
    );

    // Sub-test 3B: Candidate overflowing 2MB limit (2,097,153 bytes)
    let overflow_content = format!("{} ", candidate_exact.resolved_content);
    assert_eq!(overflow_content.len(), exact_limit + 1);

    let candidate_overflow = ResolutionCandidate {
        file_path: PathBuf::from("crates/overflow.rs"),
        proposed_by: TripartiteRole::Generator,
        resolved_content: overflow_content,
        rationale: "Overflow 2MB + 1 candidate".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };
    assert!(
        !resolver.governor_verify(&candidate_overflow),
        "Candidate exceeding 2MB limit must be rejected by Governor"
    );

    let mock_critic = CriticEvaluation {
        passed: true,
        diagnostics: Vec::new(),
        raw_compiler_output: String::new(),
        duration_ms: 0,
    };
    let verdict =
        resolver.governor_evaluate(TripartiteRole::Governor, &candidate_overflow, &mock_critic);
    assert!(!verdict.approved);
    assert!(verdict.reasons[0].contains("Diff size exceeded"));
}

// ============================================================================
// Dimension 4: Injection of unsafe blocks, allow(unsafe_code), and evasions
// ============================================================================

#[test]
fn test_adv_unsafe_injection_standard() {
    let resolver = TripartiteMergeResolver::default();

    // Standard `unsafe fn` injection
    let candidate = ResolutionCandidate {
        file_path: PathBuf::from("crates/inject1.rs"),
        proposed_by: TripartiteRole::Generator,
        resolved_content: "unsafe fn execute_danger() {}".to_string(),
        rationale: "Injected unsafe fn".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };
    assert!(
        !resolver.governor_verify(&candidate),
        "Governor must reject standard unsafe fn injection"
    );

    // Standard `unsafe {` injection
    let candidate2 = ResolutionCandidate {
        file_path: PathBuf::from("crates/inject2.rs"),
        proposed_by: TripartiteRole::Generator,
        resolved_content: "pub fn run() { unsafe { std::ptr::null::<u8>(); } }".to_string(),
        rationale: "Injected unsafe block".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };
    assert!(
        !resolver.governor_verify(&candidate2),
        "Governor must reject standard unsafe block injection"
    );
}

#[test]
fn test_adv_unsafe_injection_evasion_variants() {
    let resolver = TripartiteMergeResolver::default();
    let mock_critic = CriticEvaluation {
        passed: true,
        diagnostics: Vec::new(),
        raw_compiler_output: String::new(),
        duration_ms: 0,
    };

    // Evasion Variant A: `#![allow(unsafe_code)]` injection
    // Should be forbidden by strict #![forbid(unsafe_code)] invariant!
    let candidate_allow = ResolutionCandidate {
        file_path: PathBuf::from("crates/inject_allow.rs"),
        proposed_by: TripartiteRole::Generator,
        resolved_content: "#![allow(unsafe_code)]\npub fn ok() {}".to_string(),
        rationale: "Injected allow(unsafe_code)".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };
    let verdict_allow =
        resolver.governor_evaluate(TripartiteRole::Governor, &candidate_allow, &mock_critic);
    // Governor must strictly reject `#![allow(unsafe_code)]`
    assert!(
        !verdict_allow.approved,
        "Governor must reject `#![allow(unsafe_code)]` attribute!"
    );

    // Evasion Variant B: `unsafe{` without trailing space
    let candidate_no_space = ResolutionCandidate {
        file_path: PathBuf::from("crates/inject_no_space.rs"),
        proposed_by: TripartiteRole::Generator,
        resolved_content: "pub fn run() { unsafe{ std::ptr::null::<u8>(); } }".to_string(),
        rationale: "Injected unsafe without space".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };
    let verdict_no_space =
        resolver.governor_evaluate(TripartiteRole::Governor, &candidate_no_space, &mock_critic);
    // Governor must strictly reject `unsafe{` without space
    assert!(
        !verdict_no_space.approved,
        "Governor must reject `unsafe{{` without space"
    );

    // Evasion Variant C: comment containing `forbid(unsafe_code)` alongside `unsafe fn`
    let candidate_bypass_comment = ResolutionCandidate {
        file_path: PathBuf::from("crates/inject_comment_bypass.rs"),
        proposed_by: TripartiteRole::Generator,
        resolved_content: "// #![forbid(unsafe_code)]\nunsafe fn bypass() {}".to_string(),
        rationale: "Injected comment bypass with unsafe fn".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };
    let verdict_bypass_comment = resolver.governor_evaluate(
        TripartiteRole::Governor,
        &candidate_bypass_comment,
        &mock_critic,
    );
    // Governor must strictly reject unsafe fn even with decoy comment
    assert!(
        !verdict_bypass_comment.approved,
        "Governor must reject unsafe fn even if comment has `forbid(unsafe_code)`"
    );
}

// ============================================================================
// Dimension 5: Generator attempting self-approval (SelfApprovalForbidden error)
// ============================================================================

#[test]
fn test_adv_generator_self_approval_forbidden() {
    let resolver = TripartiteMergeResolver::default();
    let candidate = ResolutionCandidate {
        file_path: PathBuf::from("crates/legit.rs"),
        proposed_by: TripartiteRole::Generator,
        resolved_content: "pub fn normal() {}".to_string(),
        rationale: "Valid candidate".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };
    let mock_critic = CriticEvaluation {
        passed: true,
        diagnostics: Vec::new(),
        raw_compiler_output: String::new(),
        duration_ms: 0,
    };

    // 1. Generator self-approving MUST fail
    let verdict_gen =
        resolver.governor_evaluate(TripartiteRole::Generator, &candidate, &mock_critic);
    assert!(
        !verdict_gen.approved,
        "Generator MUST NOT be able to self-approve candidate"
    );
    assert!(
        verdict_gen.reasons[0].contains("SelfApprovalForbidden"),
        "Must specify SelfApprovalForbidden error reason"
    );

    // 2. Critic evaluating/approving MUST also fail if proposed_by is Critic
    let candidate_by_critic = ResolutionCandidate {
        file_path: PathBuf::from("crates/legit.rs"),
        proposed_by: TripartiteRole::Critic,
        resolved_content: "pub fn normal() {}".to_string(),
        rationale: "Critic candidate".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };
    let verdict_critic =
        resolver.governor_evaluate(TripartiteRole::Critic, &candidate_by_critic, &mock_critic);
    assert!(
        !verdict_critic.approved,
        "Critic cannot approve its own candidate"
    );

    // 3. Governor approving Generator's candidate is valid
    let verdict_gov =
        resolver.governor_evaluate(TripartiteRole::Governor, &candidate, &mock_critic);
    assert!(
        verdict_gov.approved,
        "Governor can approve Generator's candidate"
    );
}

// ============================================================================
// Dimension 6: Iteration exhaustion beyond K=3 feedback loop attempts & rollback
// ============================================================================

#[test]
fn test_adv_iteration_exhaustion_and_clean_rollback() {
    let resolver = TripartiteMergeResolver::new(3);
    let repo_root = create_temp_test_dir("exhaustion_test");
    let file_path = repo_root.join("conflict.rs");

    // Unresolvable conflict where both branches have broken syntax (unbalanced braces)
    // so Critic verification fails on all attempts (Additive, PreferOurs, PreferTheirs)
    let conflicted_content = r#"
<<<<<<< HEAD
pub fn broken_ours( {
=======
pub fn broken_theirs( {
>>>>>>> canary
"#;
    fs::write(&file_path, conflicted_content).unwrap();

    let report = resolver
        .resolve_file_content(&file_path, conflicted_content, &repo_root)
        .expect("resolve_file_content should return Ok with failure report");

    assert!(!report.success, "Unresolvable file must fail resolution");
    assert_eq!(
        report.iterations_used, 3,
        "Must exhaust exactly max_retries = 3 attempts"
    );
    assert_eq!(report.max_retries, 3);
    assert!(report.rollback_executed, "Rollback flag must be true");
    assert!(
        report.resolved_files.is_empty(),
        "No resolved files should be reported on failure"
    );
    assert_eq!(
        report.iteration_history.len(),
        3,
        "History must record all 3 failed attempts"
    );

    let _ = fs::remove_dir_all(&repo_root);
}
