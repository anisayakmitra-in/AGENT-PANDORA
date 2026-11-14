#![forbid(unsafe_code)]

//! Phase 25 Autonomous Canary Auto-Merge & Dual-Gated Promotion Tests.
//!
//! Validates:
//! 1. Byzantine consensus quorum gate ($Q \ge 2f + 1$, $N \ge 3f + 1$).
//! 2. Bayesian lower credible bound gate ($\text{LCB}_{95} > \text{baseline}$).
//! 3. Rejection on insufficient sample size ($n < 10$).
//! 4. Rejection on unverified/unattested worktree.
//! 5. Mainline git auto-merge, author identity invariant, and tag generation.
//! 6. Tripartite merge conflict resolution loop with self-healing feedback.
//! 7. Signed cryptographic provenance (In-toto Statement v1 + Ed25519 signature verification).

use pandora_runtime::canary_worktree::{
    CanaryError, CanaryGateTelemetry, CanaryPromotionGate, CanaryStagePipeline, GitWorktreeManager,
    WorktreeStatus,
};
use pandora_runtime::merge_resolver::{
    CriticEvaluation, DiagnosticMessage, MergeConflictHunk, ResolutionCandidate,
    ResolutionStrategy, TripartiteMergeResolver, TripartiteRole,
};
use pandora_runtime::multi_agent_collab::ExternalAgentKind;
use pandora_runtime::multi_agent_consensus::{AgentPatchProposal, ProposalScoreVector};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn create_temp_test_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    dir.push(format!("pandora_canary_merge_test_{name}_{nanos}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("failed to create temp test directory");
    dir
}

fn create_staged_canary_worktree(
    repo_root: &Path,
    canary_task: &str,
) -> (String, CanaryStagePipeline) {
    let cert = format!("PANDORA_PATCH_CERT_V1:{:064x}", 0x42);
    let branch = format!("pandora/canary/{canary_task}");
    let mut worktree = GitWorktreeManager::create_worktree(repo_root, &branch, &cert)
        .expect("worktree creation must succeed");

    let patch = AgentPatchProposal::new(
        format!("patch_{canary_task}"),
        ExternalAgentKind::ClaudeCode,
        "Implement memory-safe vector cache",
        "Replaces unsafe pointers with bounded slices",
        "+++ b/cache.rs\n+// Safe cache\npub fn cache_get() -> bool { true }\n",
        ProposalScoreVector::new(0.95, 0.96, 0.92, 0.98),
        42,
    );

    let pipeline = CanaryStagePipeline::default().with_repo_root(repo_root);
    let receipt = pipeline
        .stage_candidate_patch(&mut worktree, &patch)
        .expect("staging must succeed");

    (receipt.canary_id, pipeline)
}

#[test]
fn test_byzantine_quorum_verification() {
    // Case 1: Valid cluster N=4, f=1. Quorum threshold Q = (4 + 1)/2 + 1 = 3.
    assert!(CanaryPromotionGate::verify_byzantine_quorum(3, 4, 1));
    assert!(CanaryPromotionGate::verify_byzantine_quorum(4, 4, 1));
    // Approvals < 2f + 1 must fail
    assert!(!CanaryPromotionGate::verify_byzantine_quorum(2, 4, 1));

    // Case 2: Cluster size violation N < 3f + 1 (N=3, f=1 violates BFT)
    assert!(!CanaryPromotionGate::verify_byzantine_quorum(3, 3, 1));

    // Case 3: N=7, f=2. Quorum threshold Q = (7 + 2)/2 + 1 = 5.
    assert!(CanaryPromotionGate::verify_byzantine_quorum(5, 7, 2));
    assert!(CanaryPromotionGate::verify_byzantine_quorum(6, 7, 2));
    assert!(!CanaryPromotionGate::verify_byzantine_quorum(4, 7, 2));
}

#[test]
fn test_bayesian_lcb95_gate_evaluation() {
    // Case 1: High pass rate, large sample size: 100 passes, 0 failures, baseline=0.85
    let (promoted, lcb95) = CanaryPromotionGate::verify_bayesian_lcb95(100, 0, 0.85);
    assert!(promoted);
    assert!(lcb95 > 0.95);

    // Case 2: Mediocre pass rate: 10 passes, 10 failures, baseline=0.50
    let (promoted, lcb95) = CanaryPromotionGate::verify_bayesian_lcb95(10, 10, 0.50);
    assert!(!promoted, "LCB95 ({lcb95:.4}) must be <= baseline (0.50)");
    assert!(lcb95 < 0.40);

    // Case 3: High baseline requirement: 85 passes, 5 failures, baseline=0.95
    let (promoted, lcb95) = CanaryPromotionGate::verify_bayesian_lcb95(85, 5, 0.95);
    assert!(
        !promoted,
        "LCB95 ({lcb95:.4}) must fail when lower than aggressive 0.95 baseline"
    );
}

#[test]
fn test_rejection_when_byzantine_approvals_insufficient() {
    let repo_root = create_temp_test_dir("byz_insufficient");
    let (canary_id, pipeline) = create_staged_canary_worktree(&repo_root, "byz_fail");

    // Telemetry with 2 approvals out of 4 (required: 3)
    let telemetry = CanaryGateTelemetry {
        byzantine_approvals: 2,
        byzantine_total_nodes: 4,
        byzantine_f: 1,
        bayesian_passes: 100,
        bayesian_failures: 0,
        bayesian_baseline: 0.85,
        min_sample_size: 10,
    };
    pipeline
        .record_canary_telemetry(&canary_id, &telemetry)
        .unwrap();

    let err = pipeline
        .promote_canary(&canary_id, "main", false)
        .expect_err("must reject when Byzantine approvals < 2f + 1");

    match err {
        CanaryError::ByzantineQuorumNotMet {
            approvals,
            required,
            ..
        } => {
            assert_eq!(approvals, 2);
            assert_eq!(required, 3);
        }
        other => panic!("expected ByzantineQuorumNotMet, got: {other:?}"),
    }

    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn test_rejection_when_bayesian_lcb95_below_baseline() {
    let repo_root = create_temp_test_dir("bayes_fail");
    let (canary_id, pipeline) = create_staged_canary_worktree(&repo_root, "bayes_fail");

    // Telemetry with 15 passes, 15 failures, baseline=0.60
    let telemetry = CanaryGateTelemetry {
        byzantine_approvals: 4,
        byzantine_total_nodes: 4,
        byzantine_f: 1,
        bayesian_passes: 15,
        bayesian_failures: 15,
        bayesian_baseline: 0.60,
        min_sample_size: 10,
    };
    pipeline
        .record_canary_telemetry(&canary_id, &telemetry)
        .unwrap();

    let err = pipeline
        .promote_canary(&canary_id, "main", false)
        .expect_err("must reject when LCB95 <= baseline");

    match err {
        CanaryError::BayesianGateFailed {
            lcb95,
            baseline,
            passes,
            failures,
        } => {
            assert!(lcb95 <= baseline);
            assert_eq!(passes, 15);
            assert_eq!(failures, 15);
        }
        other => panic!("expected BayesianGateFailed, got: {other:?}"),
    }

    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn test_rejection_on_insufficient_sample_size() {
    let repo_root = create_temp_test_dir("sample_size_fail");
    let (canary_id, pipeline) = create_staged_canary_worktree(&repo_root, "sample_fail");

    // Only 3 total evaluations when minimum required is 10
    let telemetry = CanaryGateTelemetry {
        byzantine_approvals: 4,
        byzantine_total_nodes: 4,
        byzantine_f: 1,
        bayesian_passes: 3,
        bayesian_failures: 0,
        bayesian_baseline: 0.50,
        min_sample_size: 10,
    };
    pipeline
        .record_canary_telemetry(&canary_id, &telemetry)
        .unwrap();

    let err = pipeline
        .promote_canary(&canary_id, "main", false)
        .expect_err("must reject when total evaluations < min_sample_size");

    match err {
        CanaryError::InsufficientSampleSize {
            total_evaluations,
            minimum_required,
        } => {
            assert_eq!(total_evaluations, 3);
            assert_eq!(minimum_required, 10);
        }
        other => panic!("expected InsufficientSampleSize, got: {other:?}"),
    }

    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn test_rejection_on_unverified_or_unattested_worktree() {
    let repo_root = create_temp_test_dir("unattested");
    let cert = format!("PANDORA_PATCH_CERT_V1:{:064x}", 0x99);
    let worktree = GitWorktreeManager::create_worktree(&repo_root, "pandora/canary/raw", &cert)
        .expect("worktree creation must succeed");

    assert_eq!(worktree.status, WorktreeStatus::Created);

    let pipeline = CanaryStagePipeline::default().with_repo_root(&repo_root);
    let err = pipeline
        .promote_canary(&worktree.id, "main", false)
        .expect_err("must reject promotion of unverified worktree");

    match err {
        CanaryError::UnverifiedWorktree { status, .. } => {
            assert_eq!(status, WorktreeStatus::Created);
        }
        other => panic!("expected UnverifiedWorktree, got: {other:?}"),
    }

    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn test_successful_canary_promotion_and_receipt() {
    let repo_root = create_temp_test_dir("success_promo");
    let (canary_id, pipeline) = create_staged_canary_worktree(&repo_root, "promo_ok");

    // Fully satisfying telemetry: 4/4 approvals, 80 passes, 0 failures, baseline=0.85
    let telemetry = CanaryGateTelemetry {
        byzantine_approvals: 4,
        byzantine_total_nodes: 4,
        byzantine_f: 1,
        bayesian_passes: 80,
        bayesian_failures: 0,
        bayesian_baseline: 0.85,
        min_sample_size: 10,
    };
    pipeline
        .record_canary_telemetry(&canary_id, &telemetry)
        .unwrap();

    let receipt = pipeline
        .promote_canary(&canary_id, "main", false)
        .expect("promotion must succeed when both gates pass");

    assert_eq!(receipt.canary_id, canary_id);
    assert_eq!(receipt.target_branch, "main");
    assert!(receipt.byzantine_quorum_met);
    assert_eq!(receipt.byzantine_approvals, 4);
    assert!(receipt.bayesian_lcb95 > 0.85);
    assert_eq!(
        receipt.tag_name,
        format!("pandora/canary/promoted/{canary_id}")
    );
    assert!(!receipt.signature_hex.is_empty());
    assert!(!receipt.auto_merged);
    assert_eq!(
        receipt.author,
        "ANISAYAK MITRA <266799942+anisayakmitra-in@users.noreply.github.com>"
    );

    // Verify worktree transitioned to Promoted
    let worktrees = GitWorktreeManager::list_worktrees(&repo_root).unwrap();
    let updated = worktrees.iter().find(|w| w.id == canary_id).unwrap();
    assert_eq!(updated.status, WorktreeStatus::Promoted);

    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn test_tripartite_conflict_resolver_clean_synthesis() {
    let resolver = TripartiteMergeResolver::default();
    assert_eq!(resolver.max_retries, 3);

    // Verify Governor invariant validation
    let safe_candidate = ResolutionCandidate {
        file_path: PathBuf::from("crates/safe.rs"),
        proposed_by: TripartiteRole::Generator,
        resolved_content: "// safe code\npub fn run() {}\n".to_string(),
        rationale: "Clean additive merge".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };
    assert!(resolver.governor_verify(&safe_candidate));

    let unsafe_candidate = ResolutionCandidate {
        file_path: PathBuf::from("crates/unsafe.rs"),
        proposed_by: TripartiteRole::Generator,
        resolved_content: "unsafe fn boom() { let _ = 0 as *mut u8; }".to_string(),
        rationale: "Illegal unsafe addition".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };
    assert!(!resolver.governor_verify(&unsafe_candidate));
}

#[test]
fn test_tripartite_conflict_resolver_self_approval_forbidden() {
    let resolver = TripartiteMergeResolver::default();
    let candidate = ResolutionCandidate {
        file_path: PathBuf::from("crates/code.rs"),
        proposed_by: TripartiteRole::Generator,
        resolved_content: "pub fn execute() -> bool { true }".to_string(),
        rationale: "Generated patch".to_string(),
        strategy: ResolutionStrategy::AdditiveUnion,
    };

    let mock_critic = CriticEvaluation {
        passed: true,
        diagnostics: Vec::new(),
        raw_compiler_output: String::new(),
        duration_ms: 0,
    };

    // Generator attempting to approve its own candidate MUST be rejected (SelfApprovalForbidden)
    let verdict = resolver.governor_evaluate(TripartiteRole::Generator, &candidate, &mock_critic);
    assert!(!verdict.approved);
    assert!(verdict.reasons[0].contains("SelfApprovalForbidden"));

    // Governor role approving is valid
    let governor_verdict =
        resolver.governor_evaluate(TripartiteRole::Governor, &candidate, &mock_critic);
    assert!(governor_verdict.approved);
}

#[test]
fn test_tripartite_conflict_resolver_duplicate_remediation() {
    let resolver = TripartiteMergeResolver::default();
    let hunk = MergeConflictHunk {
        file_path: PathBuf::from("crates/model.rs"),
        base: String::new(),
        ours: "pub fn version() -> u32 { 1 }".to_string(),
        theirs: "pub fn version() -> u32 { 2 }".to_string(),
        ours_label: "HEAD".to_string(),
        theirs_label: "canary".to_string(),
    };

    let diagnostics = vec![DiagnosticMessage {
        code: Some("E0428".to_string()),
        message: "duplicate definitions with name `version`".to_string(),
        file_path: Some(PathBuf::from("crates/model.rs")),
        line: Some(1),
    }];

    let candidate = resolver.synthesize_candidate(
        &PathBuf::from("crates/model.rs"),
        "<<<<<<< HEAD\npub fn version() -> u32 { 1 }\n=======\npub fn version() -> u32 { 2 }\n>>>>>>> canary\n",
        &[hunk],
        ResolutionStrategy::DiagnosticRemediation,
        &diagnostics,
    );

    // Must deduplicate identical signature items
    assert_eq!(
        candidate.resolved_content.matches("pub fn version").count(),
        1
    );
}

#[test]
fn test_provenance_attestation_signature_verification() {
    let repo_root = create_temp_test_dir("attestation_verify");
    let (canary_id, pipeline) = create_staged_canary_worktree(&repo_root, "sig_verify");

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
    assert!(attestation_file.exists());

    let bytes = fs::read(&attestation_file).unwrap();
    let statement: pandora_runtime::canary_worktree::PromotionAttestationStatement =
        serde_json::from_slice(&bytes).unwrap();

    assert_eq!(statement.statement_type, "https://in-toto.io/Statement/v1");
    assert_eq!(
        statement.predicate_type,
        "https://pandora.agent/attestations/canary-promotion/v1"
    );
    assert_eq!(statement.predicate.canary_id, canary_id);
    assert_eq!(
        statement.predicate.author,
        "ANISAYAK MITRA <266799942+anisayakmitra-in@users.noreply.github.com>"
    );

    // Cryptographic Ed25519 signature must strictly verify
    assert!(statement.verify_signature());
    assert_eq!(statement.signature_hex, receipt.signature_hex);

    let _ = fs::remove_dir_all(&repo_root);
}

// ============================================================================
// Real Git Repository Integration Tests
// ============================================================================

/// Sets up a genuine, fully initialized Git repository on disk with an initial commit,
/// author configurations, and .gitignore.
fn setup_real_git_repository(test_name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    dir.push(format!("pandora_real_git_{test_name}_{nanos}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("failed to create real git repo directory");

    // 1. git init -b main
    let init_out = Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(&dir)
        .output()
        .expect("git init must succeed");
    assert!(init_out.status.success(), "git init failed: {:?}", init_out);

    // 2. Set repository-local author configuration
    for (key, val) in [
        ("user.name", "ANISAYAK MITRA"),
        (
            "user.email",
            "266799942+anisayakmitra-in@users.noreply.github.com",
        ),
    ] {
        let cfg = Command::new("git")
            .args(["config", key, val])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(cfg.status.success());
    }

    // 3. Create .gitignore ignoring .pandora worktrees so working tree remains clean
    fs::write(dir.join(".gitignore"), ".pandora/\ntarget/\n").unwrap();

    // 4. Create initial seed source file
    let src_dir = dir.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    let seed_file = src_dir.join("service.rs");
    fs::write(
        &seed_file,
        "//! Autonomous Service\n#![forbid(unsafe_code)]\n\npub fn service_status() -> bool { true }\n",
    )
    .unwrap();

    // 5. Initial commit so HEAD exists
    let _ = Command::new("git")
        .args(["add", "."])
        .current_dir(&dir)
        .output()
        .unwrap();
    let commit = Command::new("git")
        .args(["commit", "-m", "chore: initial seed commit on main"])
        .env("GIT_AUTHOR_NAME", "ANISAYAK MITRA")
        .env(
            "GIT_AUTHOR_EMAIL",
            "266799942+anisayakmitra-in@users.noreply.github.com",
        )
        .env("GIT_COMMITTER_NAME", "ANISAYAK MITRA")
        .env(
            "GIT_COMMITTER_EMAIL",
            "266799942+anisayakmitra-in@users.noreply.github.com",
        )
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(commit.status.success(), "initial commit failed");

    dir
}

#[test]
fn test_real_git_auto_merge_clean_no_ff_and_annotated_tag() {
    let repo_root = setup_real_git_repository("clean_auto_merge");
    let (canary_id, pipeline) = create_staged_canary_worktree(&repo_root, "clean_feature");

    // Telemetry satisfying Byzantine Quorum (4/4, f=1) and Bayesian LCB95 (80/0, base=0.85)
    let telemetry = CanaryGateTelemetry {
        byzantine_approvals: 4,
        byzantine_total_nodes: 4,
        byzantine_f: 1,
        bayesian_passes: 80,
        bayesian_failures: 0,
        bayesian_baseline: 0.85,
        min_sample_size: 10,
    };
    pipeline
        .record_canary_telemetry(&canary_id, &telemetry)
        .unwrap();

    // Execute promote_canary with auto_merge = true
    let receipt = pipeline
        .promote_canary(&canary_id, "main", true)
        .expect("promote_canary with auto_merge=true in real git repo must succeed");

    assert!(receipt.auto_merged);
    assert_eq!(receipt.target_branch, "main");
    assert_eq!(
        receipt.author,
        "ANISAYAK MITRA <266799942+anisayakmitra-in@users.noreply.github.com>"
    );

    // 1. Verify merge commit in real Git
    let rev_out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&repo_root)
        .output()
        .unwrap();
    let head_sha = String::from_utf8_lossy(&rev_out.stdout).trim().to_string();
    assert_eq!(head_sha, receipt.merge_commit_hash);

    // 2. Verify merge commit author and committer identity
    let author_out = Command::new("git")
        .args(["log", "-1", "--format=%an <%ae>"])
        .current_dir(&repo_root)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&author_out.stdout).trim(),
        "ANISAYAK MITRA <266799942+anisayakmitra-in@users.noreply.github.com>"
    );

    // 3. Verify real annotated git tag existence and type
    let expected_tag = format!("pandora/canary/promoted/{canary_id}");
    let tag_rev = Command::new("git")
        .args(["rev-parse", &format!("refs/tags/{expected_tag}")])
        .current_dir(&repo_root)
        .output()
        .unwrap();
    assert!(
        tag_rev.status.success(),
        "real git tag must exist in refs/tags"
    );

    // Verify tag is an annotated tag object (cat-file -t == tag)
    let tag_type = Command::new("git")
        .args(["cat-file", "-t", &expected_tag])
        .current_dir(&repo_root)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&tag_type.stdout).trim(), "tag");

    // Verify tag annotation metadata
    let tag_content = Command::new("git")
        .args(["cat-file", "-p", &expected_tag])
        .current_dir(&repo_root)
        .output()
        .unwrap();
    let tag_body = String::from_utf8_lossy(&tag_content.stdout);
    assert!(
        tag_body.contains(
            "tagger ANISAYAK MITRA <266799942+anisayakmitra-in@users.noreply.github.com>"
        )
    );
    assert!(tag_body.contains(&format!("Canary promotion {canary_id} merged")));

    // 4. Verify working tree cleanliness
    let status_out = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&repo_root)
        .output()
        .unwrap();
    let dirty = String::from_utf8_lossy(&status_out.stdout)
        .lines()
        .filter(|l| !l.contains(".pandora") && !l.trim().is_empty())
        .count();
    assert_eq!(dirty, 0, "working tree must be clean after merge");

    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn test_real_git_tripartite_attempt_resolution_direct() {
    let repo_root = setup_real_git_repository("direct_conflict_heal");

    // 1. Create a feature branch
    Command::new("git")
        .args(["checkout", "-b", "feature_branch"])
        .current_dir(&repo_root)
        .output()
        .unwrap();
    let file_path = repo_root.join("src").join("service.rs");
    fs::write(
        &file_path,
        "//! Autonomous Service\n#![forbid(unsafe_code)]\n\npub fn service_status() -> bool { true }\npub fn feature_extension() -> u32 { 101 }\n",
    ).unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&repo_root)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "feat: add feature extension"])
        .env("GIT_AUTHOR_NAME", "ANISAYAK MITRA")
        .env(
            "GIT_AUTHOR_EMAIL",
            "266799942+anisayakmitra-in@users.noreply.github.com",
        )
        .env("GIT_COMMITTER_NAME", "ANISAYAK MITRA")
        .env(
            "GIT_COMMITTER_EMAIL",
            "266799942+anisayakmitra-in@users.noreply.github.com",
        )
        .current_dir(&repo_root)
        .output()
        .unwrap();

    // 2. Switch to main and introduce conflicting edit at the same line
    Command::new("git")
        .args(["checkout", "main"])
        .current_dir(&repo_root)
        .output()
        .unwrap();
    fs::write(
        &file_path,
        "//! Autonomous Service\n#![forbid(unsafe_code)]\n\npub fn service_status() -> bool { true }\npub fn main_extension() -> u32 { 202 }\n",
    ).unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&repo_root)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "feat: add main extension"])
        .env("GIT_AUTHOR_NAME", "ANISAYAK MITRA")
        .env(
            "GIT_AUTHOR_EMAIL",
            "266799942+anisayakmitra-in@users.noreply.github.com",
        )
        .env("GIT_COMMITTER_NAME", "ANISAYAK MITRA")
        .env(
            "GIT_COMMITTER_EMAIL",
            "266799942+anisayakmitra-in@users.noreply.github.com",
        )
        .current_dir(&repo_root)
        .output()
        .unwrap();

    // 3. Execute git merge to trigger a genuine conflict
    let merge_out = Command::new("git")
        .args(["merge", "--no-ff", "feature_branch"])
        .current_dir(&repo_root)
        .output()
        .unwrap();
    assert!(
        !merge_out.status.success(),
        "Git merge must produce conflict"
    );

    // 4. Verify unmerged status in git index
    let diff_u = Command::new("git")
        .args(["diff", "--name-only", "--diff-filter=U"])
        .current_dir(&repo_root)
        .output()
        .unwrap();
    let unmerged_files = String::from_utf8_lossy(&diff_u.stdout);
    assert!(unmerged_files.contains("src/service.rs"));

    // 5. Directly exercise TripartiteMergeResolver::attempt_resolution
    let resolver = TripartiteMergeResolver::default();
    let dummy_wt = repo_root.join(".pandora").join("dummy");
    let resolved = resolver
        .attempt_resolution(&repo_root, &dummy_wt)
        .expect("attempt_resolution must execute without errors");
    assert!(resolved, "Tripartite conflict resolution must report true");

    // 6. Verify conflict markers are completely eliminated
    let resolved_content = fs::read_to_string(&file_path).unwrap();
    assert!(!resolved_content.contains("<<<<<<<"));
    assert!(!resolved_content.contains("======="));
    assert!(!resolved_content.contains(">>>>>>>"));
    assert!(resolved_content.contains("pub fn feature_extension"));
    assert!(resolved_content.contains("pub fn main_extension"));

    // 7. Verify resolution commit exists with correct author
    let log_author = Command::new("git")
        .args(["log", "-1", "--format=%an <%ae>"])
        .current_dir(&repo_root)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&log_author.stdout).trim(),
        "ANISAYAK MITRA <266799942+anisayakmitra-in@users.noreply.github.com>"
    );

    let log_msg = Command::new("git")
        .args(["log", "-1", "--format=%B"])
        .current_dir(&repo_root)
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&log_msg.stdout)
            .contains("canary(merge-resolver): tripartite cognitive self-healing resolution")
    );

    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn test_real_git_end_to_end_auto_merge_conflict_healing() {
    let repo_root = setup_real_git_repository("e2e_conflict_auto_merge");
    let (canary_id, pipeline) = create_staged_canary_worktree(&repo_root, "e2e_conflict");

    // Concurrently edit the same file on main
    let cache_file = repo_root.join("cache.rs");
    fs::write(
        &cache_file,
        "// Mainline cache\npub fn cache_get() -> u64 { 9999 }\n",
    )
    .unwrap();
    Command::new("git")
        .args(["add", "cache.rs"])
        .current_dir(&repo_root)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "main concurrent cache modification"])
        .env("GIT_AUTHOR_NAME", "ANISAYAK MITRA")
        .env(
            "GIT_AUTHOR_EMAIL",
            "266799942+anisayakmitra-in@users.noreply.github.com",
        )
        .env("GIT_COMMITTER_NAME", "ANISAYAK MITRA")
        .env(
            "GIT_COMMITTER_EMAIL",
            "266799942+anisayakmitra-in@users.noreply.github.com",
        )
        .current_dir(&repo_root)
        .output()
        .unwrap();

    let telemetry = CanaryGateTelemetry {
        byzantine_approvals: 4,
        byzantine_total_nodes: 4,
        byzantine_f: 1,
        bayesian_passes: 90,
        bayesian_failures: 0,
        bayesian_baseline: 0.85,
        min_sample_size: 10,
    };
    pipeline
        .record_canary_telemetry(&canary_id, &telemetry)
        .unwrap();

    // promote_canary with auto_merge: true will hit a conflict and heal it!
    let receipt = pipeline
        .promote_canary(&canary_id, "main", true)
        .expect("auto-merge across genuine conflict must self-heal and succeed");

    assert!(receipt.auto_merged);
    assert_eq!(
        receipt.merge_commit_hash.len(),
        40,
        "must be genuine 40-character commit SHA"
    );

    // Verify annotated git tag in real git repo
    let tag_name = format!("pandora/canary/promoted/{canary_id}");
    let tag_check = Command::new("git")
        .args(["cat-file", "-t", &tag_name])
        .current_dir(&repo_root)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&tag_check.stdout).trim(), "tag");

    let _ = fs::remove_dir_all(&repo_root);
}
