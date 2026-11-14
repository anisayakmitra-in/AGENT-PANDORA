#![forbid(unsafe_code)]

//! Adversarial Stress-Test Suite for Tripartite Merge Conflict Resolver.
//!
//! Empirically challenges:
//! 1. 3-way merge conflict parsing with malformed, nested, and incomplete delimiters.
//! 2. Cognitive governance invariant (Generator self-approval and non-Governor approval).
//! 3. Self-healing loop bounds (K=3 retry limit, abort, and rollback clean without staged artifacts).
//! 4. Memory safety enforcement (unsafe injection variants, bypass vectors, and forbid(unsafe_code)).

use pandora_runtime::merge_resolver::{
    CriticEvaluation, ResolutionCandidate, ResolutionStrategy, TripartiteMergeResolver,
    TripartiteRole,
};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn create_temp_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    dir.push(format!("pandora_adversarial_test_{name}_{nanos}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("failed to create temp dir");
    dir
}

// ============================================================================
// 1. Conflict Marker Parsing Challenges
// ============================================================================

#[test]
fn test_challenge_incomplete_conflict_markers_leak() {
    let resolver = TripartiteMergeResolver::default();
    let file_path = PathBuf::from("crates/broken.rs");

    // Malformed: has <<<<<<< and =======, but missing >>>>>>>
    let malformed_content = r#"
pub fn calculate() -> i32 {
<<<<<<< HEAD
    42
=======
    99
}
"#;

    let hunks = resolver.parse_conflict_hunks(&file_path, malformed_content);
    // Because >>>>>>> is missing, parse_conflict_hunks fails to extract any hunks:
    assert!(hunks.is_empty(), "Incomplete marker yields 0 hunks");

    // When resolve_file_content is called on this malformed content:
    let temp_dir = create_temp_dir("incomplete_markers");
    let full_path = temp_dir.join(&file_path);
    fs::create_dir_all(full_path.parent().unwrap()).unwrap();
    fs::write(&full_path, malformed_content).unwrap();

    let report = resolver
        .resolve_file_content(&file_path, malformed_content, &temp_dir)
        .expect("resolve_file_content should return Ok");

    // CRITICAL OBSERVATION:
    // File contains raw conflict markers, so resolution must fail and rollback:
    assert!(
        !report.success,
        "Incomplete delimiter conflict must fail resolution"
    );
    assert!(
        report.rollback_executed,
        "Rollback must be executed on incomplete conflict markers"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_challenge_nested_conflict_markers() {
    let resolver = TripartiteMergeResolver::default();
    let file_path = PathBuf::from("crates/nested.rs");

    let nested_content = r#"
<<<<<<< HEAD
first_level_ours
<<<<<<< INNER_HEAD
inner_ours
=======
inner_theirs
>>>>>>> INNER_BRANCH
=======
first_level_theirs
>>>>>>> OUTER_BRANCH
"#;

    let temp_dir = create_temp_dir("nested_markers");
    let report = resolver
        .resolve_file_content(&file_path, nested_content, &temp_dir)
        .expect("should return report");

    // Assert that nested conflict markers fail resolution safely and trigger rollback
    assert!(
        !report.success,
        "Nested conflict markers must fail resolution"
    );
    assert!(
        report.rollback_executed,
        "Rollback must be executed on nested markers"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

// ============================================================================
// 2. Cognitive Governance Invariant Challenges (Self-Approval & Approver Roles)
// ============================================================================

#[test]
fn test_challenge_self_approval_strictly_forbidden() {
    let resolver = TripartiteMergeResolver::default();
    let candidate = ResolutionCandidate {
        file_path: PathBuf::from("crates/code.rs"),
        proposed_by: TripartiteRole::Generator,
        resolved_content: "pub fn safe() {}".to_string(),
        rationale: "Adversarial self-approval test".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };

    let mock_critic = CriticEvaluation {
        passed: true,
        diagnostics: Vec::new(),
        raw_compiler_output: String::new(),
        duration_ms: 0,
    };

    // Generator attempting to approve its own candidate MUST be blocked
    let generator_verdict =
        resolver.governor_evaluate(TripartiteRole::Generator, &candidate, &mock_critic);
    assert!(!generator_verdict.approved);
    assert!(
        generator_verdict.reasons[0].contains("SelfApprovalForbidden"),
        "Must explicitly cite SelfApprovalForbidden"
    );

    // Critic attempting to approve its own proposal MUST be blocked
    let critic_candidate = ResolutionCandidate {
        file_path: PathBuf::from("crates/code.rs"),
        proposed_by: TripartiteRole::Critic,
        resolved_content: "pub fn safe() {}".to_string(),
        rationale: "Critic proposal".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };
    let critic_self_verdict =
        resolver.governor_evaluate(TripartiteRole::Critic, &critic_candidate, &mock_critic);
    assert!(!critic_self_verdict.approved);
    assert!(critic_self_verdict.reasons[0].contains("SelfApprovalForbidden"));

    // Governor attempting to approve its own proposal MUST be blocked
    let governor_candidate = ResolutionCandidate {
        file_path: PathBuf::from("crates/code.rs"),
        proposed_by: TripartiteRole::Governor,
        resolved_content: "pub fn safe() {}".to_string(),
        rationale: "Governor proposal".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };
    let gov_self_verdict =
        resolver.governor_evaluate(TripartiteRole::Governor, &governor_candidate, &mock_critic);
    assert!(!gov_self_verdict.approved);
    assert!(gov_self_verdict.reasons[0].contains("SelfApprovalForbidden"));
}

#[test]
fn test_challenge_critic_role_approver_bypass() {
    let resolver = TripartiteMergeResolver::default();
    let candidate = ResolutionCandidate {
        file_path: PathBuf::from("crates/code.rs"),
        proposed_by: TripartiteRole::Generator,
        resolved_content: "pub fn safe() {}".to_string(),
        rationale: "Generator proposal".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };

    let mock_critic = CriticEvaluation {
        passed: true,
        diagnostics: Vec::new(),
        raw_compiler_output: String::new(),
        duration_ms: 0,
    };

    // CHALLENGE: What happens if Critic acts as approver instead of Governor?
    let critic_approver_verdict =
        resolver.governor_evaluate(TripartiteRole::Critic, &candidate, &mock_critic);
    assert!(
        !critic_approver_verdict.approved,
        "Critic MUST NOT be able to approve candidate"
    );
    assert!(
        critic_approver_verdict.reasons[0].contains("UnauthorizedApproverRole"),
        "Must cite UnauthorizedApproverRole"
    );
}

// ============================================================================
// 3. Self-Healing Loop Bound ($K = 3$ iterations) & Rollback Cleanliness
// ============================================================================

#[test]
fn test_challenge_self_healing_k3_iteration_bound_and_rollback() {
    let resolver = TripartiteMergeResolver::new(3);
    let temp_dir = create_temp_dir("unsolvable_k3");
    let file_path = PathBuf::from("unsolvable.rs");
    let full_path = temp_dir.join(&file_path);

    // Genuinely unresolvable syntax: mismatched parens across all branches
    let unsolvable_conflict = r#"
<<<<<<< HEAD
fn broken_ours((( {
    let x = 1;
}
=======
fn broken_theirs((( {
    let y = 2;
}
>>>>>>> THEIRS
"#;
    fs::write(&full_path, unsolvable_conflict).unwrap();

    let report = resolver
        .resolve_file_content(&file_path, unsolvable_conflict, &temp_dir)
        .expect("resolve_file_content should return a report");

    // Invariant: Exactly K = 3 iterations must be attempted
    assert_eq!(
        report.iterations_used, 3,
        "Resolver must execute exactly K=3 iterations before aborting"
    );
    assert!(
        !report.success,
        "Unsolvable conflict must result in success=false"
    );
    assert!(
        report.rollback_executed,
        "Resolver must mark rollback_executed=true upon failure"
    );
    assert_eq!(
        report.iteration_history.len(),
        3,
        "History must record all 3 iterations"
    );

    // Verify strategy sequence:
    assert_eq!(
        report.iteration_history[0].strategy,
        ResolutionStrategy::AdditiveUnion
    );
    assert_eq!(
        report.iteration_history[1].strategy,
        ResolutionStrategy::PreferOurs
    );
    assert_eq!(
        report.iteration_history[2].strategy,
        ResolutionStrategy::PreferTheirs
    );

    // Verify disk content was NOT overwritten with broken candidate
    let content_after = fs::read_to_string(&full_path).unwrap();
    assert_eq!(
        content_after, unsolvable_conflict,
        "File on disk must not be corrupted or partially written on abort"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

// ============================================================================
// 4. Memory Safety Enforcement Challenges (Unsafe Injection & Bypass Vectors)
// ============================================================================

#[test]
fn test_challenge_memory_safety_standard_unsafe_rejection() {
    let resolver = TripartiteMergeResolver::default();
    let candidate = ResolutionCandidate {
        file_path: PathBuf::from("crates/unsafe_test.rs"),
        proposed_by: TripartiteRole::Generator,
        resolved_content: "pub fn do_bad() { unsafe { let p = 0 as *mut u8; } }".to_string(),
        rationale: "Inject unsafe".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };

    assert!(
        !resolver.governor_verify(&candidate),
        "Standard unsafe block must be strictly rejected by Governor"
    );
}

#[test]
fn test_challenge_memory_safety_forbid_unsafe_code_bypass() {
    let resolver = TripartiteMergeResolver::default();

    // VULNERABILITY ATTACK VECTOR 1:
    // Real Rust files have `#![forbid(unsafe_code)]` at the top.
    // In merge_resolver.rs line 752:
    // if candidate.resolved_content.contains("unsafe ") && !candidate.resolved_content.contains("forbid(unsafe_code)")
    // Because the file contains "forbid(unsafe_code)", the second clause is FALSE!
    let file_with_forbid_attribute = r#"#![forbid(unsafe_code)]

pub fn exploit() {
    unsafe {
        let ptr = 0x1234usize as *mut u64;
        *ptr = 42;
    }
}
"#;

    let candidate1 = ResolutionCandidate {
        file_path: PathBuf::from("crates/bypass.rs"),
        proposed_by: TripartiteRole::Generator,
        resolved_content: file_with_forbid_attribute.to_string(),
        rationale: "Bypass via existing forbid(unsafe_code) header".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };

    let approved1 = resolver.governor_verify(&candidate1);
    println!("Attack Vector 1 (forbid header bypass): approved={approved1}");
    assert!(
        !approved1,
        "Presence of 'forbid(unsafe_code)' in file MUST NOT bypass unsafe detection"
    );
}

#[test]
fn test_challenge_memory_safety_no_space_after_unsafe_bypass() {
    let resolver = TripartiteMergeResolver::default();

    // VULNERABILITY ATTACK VECTOR 2:
    // Code with `unsafe{` (no space after unsafe)
    let code_no_space = r#"
pub fn exploit() {
    unsafe{
        let ptr = 0x1234usize as *mut u64;
        *ptr = 42;
    }
}
"#;

    let candidate2 = ResolutionCandidate {
        file_path: PathBuf::from("crates/bypass2.rs"),
        proposed_by: TripartiteRole::Generator,
        resolved_content: code_no_space.to_string(),
        rationale: "Bypass via unsafe{ without space".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };

    let approved2 = resolver.governor_verify(&candidate2);
    println!("Attack Vector 2 (unsafe{{ without space): approved={approved2}");
    assert!(
        !approved2,
        "unsafe{{ without space MUST be rejected by Governor"
    );
}

#[test]
fn test_challenge_memory_safety_newline_after_unsafe_bypass() {
    let resolver = TripartiteMergeResolver::default();

    // VULNERABILITY ATTACK VECTOR 3:
    // Code with `unsafe\n{`
    let code_newline = "pub fn exploit() {\n    unsafe\n    { let _ = 1; }\n}";

    let candidate3 = ResolutionCandidate {
        file_path: PathBuf::from("crates/bypass3.rs"),
        proposed_by: TripartiteRole::Generator,
        resolved_content: code_newline.to_string(),
        rationale: "Bypass via unsafe\\n{".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };

    let approved3 = resolver.governor_verify(&candidate3);
    println!("Attack Vector 3 (unsafe\\n{{): approved={approved3}");
    assert!(!approved3, "unsafe\\n{{ MUST be rejected by Governor");
}

#[test]
fn test_challenge_memory_safety_comment_decoy_bypass() {
    let resolver = TripartiteMergeResolver::default();

    // VULNERABILITY ATTACK VECTOR 4:
    // Attacker puts `// forbid(unsafe_code)` in comment
    let code_decoy = r#"
// forbid(unsafe_code) is a good practice!
pub fn exploit() {
    unsafe {
        let ptr = 0xdeadbeef as *mut u32;
    }
}
"#;

    let candidate4 = ResolutionCandidate {
        file_path: PathBuf::from("crates/bypass4.rs"),
        proposed_by: TripartiteRole::Generator,
        resolved_content: code_decoy.to_string(),
        rationale: "Bypass via decoy comment".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };

    let approved4 = resolver.governor_verify(&candidate4);
    println!("Attack Vector 4 (decoy comment): approved={approved4}");
    assert!(
        !approved4,
        "Comment decoy 'forbid(unsafe_code)' MUST NOT bypass unsafe detection"
    );
}

#[test]
fn test_challenge_attempt_resolution_rollback_cleanliness() {
    let resolver = TripartiteMergeResolver::new(3);
    let temp_dir = create_temp_dir("attempt_res_cleanliness");
    let file_path = temp_dir.join("conflict.rs");

    // Conflict with unbalanced parens that cannot resolve
    let original_conflict = r#"
<<<<<<< HEAD
fn broken((( {
=======
fn broken((( {
>>>>>>> THEIRS
"#;
    fs::write(&file_path, original_conflict).unwrap();

    let res = resolver.attempt_resolution(&temp_dir, &temp_dir);
    assert!(res.is_ok());
    assert!(
        !res.unwrap(),
        "Unresolvable conflicts must return Ok(false)"
    );

    // Verify backup restoration: original file content is intact and not deleted or half-written
    let content_after = fs::read_to_string(&file_path).unwrap();
    assert_eq!(
        content_after, original_conflict,
        "attempt_resolution must restore original backup on failure"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}
