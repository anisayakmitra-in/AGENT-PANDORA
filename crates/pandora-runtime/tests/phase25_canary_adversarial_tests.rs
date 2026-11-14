#![forbid(unsafe_code)]

//! Phase 25 Adversarial Stress Tests: Dual-Gating & Canary Git Promotion.
//!
//! Stress-tests:
//! 1. Byzantine quorum edge cases ($Q < 2f + 1$, $N < 3f + 1$, $f = 0$, $Q = 0$).
//! 2. Bayesian credible bound boundary conditions ($n < 10$, $\text{LCB}_{95} \le \text{baseline}$, $\text{baseline} = 1.0$, $\text{baseline} = \text{NaN}$).
//! 3. Worktree lifecycle status rejections (`Created`, `PatchApplied`, `Verified`, `CleanedUp`).
//! 4. Real git repository dirty working tree rejection (`CanaryError::WorktreeDirty`).
//! 5. Permitted ephemeral `.pandora/` status isolation.
//! 6. Cryptographic In-toto Statement v1 signature tampering and forgery rejection.

use pandora_runtime::canary_worktree::{
    CanaryError, CanaryGateTelemetry, CanaryPromotionGate, CanaryStagePipeline, GitWorktreeManager,
    PromotionAttestationStatement, WorktreeStatus,
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
    dir.push(format!("pandora_canary_adv_{name}_{nanos}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("failed to create temp test directory");
    dir
}

fn create_staged_canary(repo_root: &Path, task_name: &str) -> (String, CanaryStagePipeline) {
    let cert = format!("PANDORA_PATCH_CERT_V1:{:064x}", 0xbeef);
    let branch = format!("pandora/canary/{task_name}");
    let mut worktree = GitWorktreeManager::create_worktree(repo_root, &branch, &cert)
        .expect("worktree creation must succeed");

    let patch = AgentPatchProposal::new(
        format!("adv_patch_{task_name}"),
        ExternalAgentKind::ClaudeCode,
        "Adversarial verification patch",
        "Validates edge cases and boundary conditions",
        "+++ b/adv.rs\n+// Adversarial probe\npub fn probe() -> u32 { 42 }\n",
        ProposalScoreVector::new(0.95, 0.95, 0.95, 0.95),
        100,
    );

    let pipeline = CanaryStagePipeline::default().with_repo_root(repo_root);
    let receipt = pipeline
        .stage_candidate_patch(&mut worktree, &patch)
        .expect("staging must succeed");

    (receipt.canary_id, pipeline)
}

// =========================================================================
// 1. Byzantine Quorum Edge Cases & Boundary Probes
// =========================================================================

#[test]
fn test_adversarial_byzantine_node_deficiency() {
    // In any BFT system, N must be >= 3f + 1 to tolerate f Byzantine nodes.
    // If N <= 3f, verify_byzantine_quorum must reject EVEN IF approvals == N (100% unanimous).

    // f = 1: 3f = 3. N in {0, 1, 2, 3} must all fail.
    for n in 0..=3 {
        assert!(
            !CanaryPromotionGate::verify_byzantine_quorum(n, n, 1),
            "Unanimous approvals on N={n} <= 3f (f=1) must fail"
        );
    }

    // f = 2: 3f = 6. N in {0..=6} must all fail even if approvals == N.
    for n in 0..=6 {
        assert!(
            !CanaryPromotionGate::verify_byzantine_quorum(n, n, 2),
            "Unanimous approvals on N={n} <= 3f (f=2) must fail"
        );
    }

    // f = 3: 3f = 9. N in {0..=9} must all fail even if approvals == N.
    for n in 0..=9 {
        assert!(
            !CanaryPromotionGate::verify_byzantine_quorum(n, n, 3),
            "Unanimous approvals on N={n} <= 3f (f=3) must fail"
        );
    }
}

#[test]
fn test_adversarial_byzantine_sub_quorum_approvals() {
    // For valid cluster N=4, f=1: Q_required = (4+1)/2 + 1 = 3 (which is 2f + 1).
    // Approvals in {0, 1, 2} must all fail.
    for a in 0..3 {
        assert!(
            !CanaryPromotionGate::verify_byzantine_quorum(a, 4, 1),
            "Approvals {a} < 3 on N=4, f=1 must fail"
        );
    }

    // For valid cluster N=7, f=2: Q_required = (7+2)/2 + 1 = 5 (which is 2f + 1).
    // Approvals in {0, 1, 2, 3, 4} must all fail.
    for a in 0..5 {
        assert!(
            !CanaryPromotionGate::verify_byzantine_quorum(a, 7, 2),
            "Approvals {a} < 5 on N=7, f=2 must fail"
        );
    }

    // For larger cluster N=10, f=1:
    // Quorum threshold Q = (10 + 1)/2 + 1 = 6.
    // 2f + 1 = 3, but 3 nodes cannot guarantee intersection with all quorums.
    // Therefore approvals in {0, 1, 2, 3, 4, 5} must all fail!
    for a in 0..6 {
        assert!(
            !CanaryPromotionGate::verify_byzantine_quorum(a, 10, 1),
            "Approvals {a} < 6 on N=10, f=1 must fail despite a >= 2f + 1"
        );
    }
    assert!(CanaryPromotionGate::verify_byzantine_quorum(6, 10, 1));
}

#[test]
fn test_adversarial_byzantine_edge_f_zero() {
    // Crash-only / zero-fault assumption: f = 0.
    // N = 0: fails
    assert!(!CanaryPromotionGate::verify_byzantine_quorum(0, 0, 0));

    // N = 1, f = 0: Q = (1+0)/2 + 1 = 1.
    assert!(!CanaryPromotionGate::verify_byzantine_quorum(0, 1, 0));
    assert!(CanaryPromotionGate::verify_byzantine_quorum(1, 1, 0));

    // N = 3, f = 0: Q = (3+0)/2 + 1 = 2.
    assert!(!CanaryPromotionGate::verify_byzantine_quorum(1, 3, 0));
    assert!(CanaryPromotionGate::verify_byzantine_quorum(2, 3, 0));
    assert!(CanaryPromotionGate::verify_byzantine_quorum(3, 3, 0));
}

#[test]
fn test_adversarial_byzantine_pipeline_rejections() {
    let repo_root = create_temp_test_dir("adv_byz_pipe");
    let (canary_id, pipeline) = create_staged_canary(&repo_root, "byz_pipe");

    // Case 1: Node deficiency N=3, f=1 (N <= 3f)
    let telemetry_n_def = CanaryGateTelemetry {
        byzantine_approvals: 3,
        byzantine_total_nodes: 3,
        byzantine_f: 1,
        bayesian_passes: 100,
        bayesian_failures: 0,
        bayesian_baseline: 0.85,
        min_sample_size: 10,
    };
    pipeline
        .record_canary_telemetry(&canary_id, &telemetry_n_def)
        .unwrap();

    let err = pipeline
        .promote_canary(&canary_id, "main", false)
        .expect_err("N <= 3f must be rejected");

    match err {
        CanaryError::ByzantineQuorumNotMet {
            total_nodes, byz_f, ..
        } => {
            assert_eq!(total_nodes, 3);
            assert_eq!(byz_f, 1);
        }
        other => panic!("expected ByzantineQuorumNotMet, got {other:?}"),
    }

    // Case 2: Zero approvals
    let telemetry_zero_app = CanaryGateTelemetry {
        byzantine_approvals: 0,
        byzantine_total_nodes: 4,
        byzantine_f: 1,
        bayesian_passes: 100,
        bayesian_failures: 0,
        bayesian_baseline: 0.85,
        min_sample_size: 10,
    };
    pipeline
        .record_canary_telemetry(&canary_id, &telemetry_zero_app)
        .unwrap();

    let err = pipeline
        .promote_canary(&canary_id, "main", false)
        .expect_err("0 approvals must be rejected");

    match err {
        CanaryError::ByzantineQuorumNotMet {
            approvals,
            required,
            ..
        } => {
            assert_eq!(approvals, 0);
            assert_eq!(required, 3);
        }
        other => panic!("expected ByzantineQuorumNotMet, got {other:?}"),
    }

    let _ = fs::remove_dir_all(&repo_root);
}

// =========================================================================
// 2. Bayesian Credible Bound Boundary Conditions & Exhaustion
// =========================================================================

#[test]
fn test_adversarial_bayesian_sample_size_exhaustion() {
    let repo_root = create_temp_test_dir("adv_bayes_sample");
    let (canary_id, pipeline) = create_staged_canary(&repo_root, "sample_exh");

    // Test n in {0, 1, 5, 9} when min_sample_size = 10. All must be rejected.
    let test_samples = [(0, 0), (1, 0), (3, 2), (9, 0), (5, 4)];
    for (p, f) in test_samples {
        let telemetry = CanaryGateTelemetry {
            byzantine_approvals: 4,
            byzantine_total_nodes: 4,
            byzantine_f: 1,
            bayesian_passes: p,
            bayesian_failures: f,
            bayesian_baseline: 0.50,
            min_sample_size: 10,
        };
        pipeline
            .record_canary_telemetry(&canary_id, &telemetry)
            .unwrap();

        let err = pipeline
            .promote_canary(&canary_id, "main", false)
            .expect_err(&format!("n={}+{} < 10 must be rejected", p, f));

        match err {
            CanaryError::InsufficientSampleSize {
                total_evaluations,
                minimum_required,
            } => {
                assert_eq!(total_evaluations, p + f);
                assert_eq!(minimum_required, 10);
            }
            other => panic!("expected InsufficientSampleSize, got {other:?}"),
        }
    }

    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn test_adversarial_bayesian_sample_size_boundary_ten() {
    // Exactly n = 10 (passes=10, failures=0) satisfies sample size requirement.
    // LCB95 for Beta(11, 1) at quantile 0.05 is 0.05^(1/11) ~= 0.7622.
    let (promoted_low_base, lcb95) = CanaryPromotionGate::verify_bayesian_lcb95(10, 0, 0.70);
    assert!(promoted_low_base);
    assert!(lcb95 > 0.70 && lcb95 < 0.80);

    // If baseline is 0.80, exactly the same telemetry MUST fail because 0.7622 <= 0.80
    let (promoted_high_base, _) = CanaryPromotionGate::verify_bayesian_lcb95(10, 0, 0.80);
    assert!(!promoted_high_base);
}

#[test]
fn test_adversarial_bayesian_exact_equality_and_boundary_conditions() {
    // 1. Strict inequality test: lower_bound_95 == baseline must FAIL.
    let (_, lcb95) = CanaryPromotionGate::verify_bayesian_lcb95(50, 2, 0.50);
    // Setting baseline to exactly lcb95:
    let (promoted_exact, _) = CanaryPromotionGate::verify_bayesian_lcb95(50, 2, lcb95);
    assert!(
        !promoted_exact,
        "Exact equality LCB95 == baseline MUST NOT promote (strict inequality required)"
    );

    // 2. Infinitesimally higher baseline:
    let (promoted_higher, _) = CanaryPromotionGate::verify_bayesian_lcb95(50, 2, lcb95 + 1e-9);
    assert!(!promoted_higher);

    // 3. Baseline = 1.0 (impossible to beat for any finite sample):
    let (promoted_one, _) = CanaryPromotionGate::verify_bayesian_lcb95(1000, 0, 1.0);
    assert!(!promoted_one, "Baseline 1.0 cannot be exceeded");

    // 4. Baseline = NaN:
    let (promoted_nan, _) = CanaryPromotionGate::verify_bayesian_lcb95(100, 0, f64::NAN);
    assert!(!promoted_nan, "NaN baseline must not promote");

    // 5. Total failure: 0 passes, 100 failures:
    let (promoted_zero_pass, lcb95_fail) = CanaryPromotionGate::verify_bayesian_lcb95(0, 100, 0.05);
    assert!(!promoted_zero_pass);
    assert!(lcb95_fail < 0.01);
}

// =========================================================================
// 3. Worktree Lifecycle Status Rejections
// =========================================================================

#[test]
fn test_adversarial_worktree_lifecycle_unverified_states() {
    let repo_root = create_temp_test_dir("adv_lifecycle");
    let cert = format!("PANDORA_PATCH_CERT_V1:{:064x}", 0x777);
    let mut worktree =
        GitWorktreeManager::create_worktree(&repo_root, "pandora/canary/states", &cert)
            .expect("worktree creation must succeed");

    let pipeline = CanaryStagePipeline::default().with_repo_root(&repo_root);

    // State 1: WorktreeStatus::Created -> CanaryError::UnverifiedWorktree
    worktree.status = WorktreeStatus::Created;
    let _ = fs::write(
        worktree.path.join(".canary_meta.json"),
        serde_json::to_vec(&worktree).unwrap(),
    );
    let err1 = pipeline
        .promote_canary(&worktree.id, "main", false)
        .unwrap_err();
    match err1 {
        CanaryError::UnverifiedWorktree { status, .. } => {
            assert_eq!(status, WorktreeStatus::Created)
        }
        other => panic!("expected UnverifiedWorktree for Created, got {other:?}"),
    }

    // State 2: WorktreeStatus::PatchApplied -> CanaryError::UnverifiedWorktree
    worktree.status = WorktreeStatus::PatchApplied;
    let _ = fs::write(
        worktree.path.join(".canary_meta.json"),
        serde_json::to_vec(&worktree).unwrap(),
    );
    let err2 = pipeline
        .promote_canary(&worktree.id, "main", false)
        .unwrap_err();
    match err2 {
        CanaryError::UnverifiedWorktree { status, .. } => {
            assert_eq!(status, WorktreeStatus::PatchApplied)
        }
        other => panic!("expected UnverifiedWorktree for PatchApplied, got {other:?}"),
    }

    // State 3: WorktreeStatus::Verified -> CanaryError::UnattestedWorktree
    worktree.status = WorktreeStatus::Verified;
    let _ = fs::write(
        worktree.path.join(".canary_meta.json"),
        serde_json::to_vec(&worktree).unwrap(),
    );
    let err3 = pipeline
        .promote_canary(&worktree.id, "main", false)
        .unwrap_err();
    match err3 {
        CanaryError::UnattestedWorktree { status, .. } => {
            assert_eq!(status, WorktreeStatus::Verified)
        }
        other => panic!("expected UnattestedWorktree for Verified, got {other:?}"),
    }

    // State 4: WorktreeStatus::CleanedUp -> CanaryError::WorktreeNotFound
    worktree.status = WorktreeStatus::CleanedUp;
    let _ = fs::write(
        worktree.path.join(".canary_meta.json"),
        serde_json::to_vec(&worktree).unwrap(),
    );
    let err4 = pipeline
        .promote_canary(&worktree.id, "main", false)
        .unwrap_err();
    match err4 {
        CanaryError::WorktreeNotFound(id) => assert_eq!(id, worktree.id),
        other => panic!("expected WorktreeNotFound for CleanedUp, got {other:?}"),
    }

    let _ = fs::remove_dir_all(&repo_root);
}

// =========================================================================
// 4. Git Working Tree Dirty Rejection & Status Isolation
// =========================================================================

#[test]
fn test_adversarial_git_dirty_working_tree_rejection() {
    let repo_root = create_temp_test_dir("adv_git_dirty");

    // Initialize real git repository
    let run_git = |args: &[&str]| {
        let status = Command::new("git")
            .args(args)
            .current_dir(&repo_root)
            .status()
            .expect("git execution failed");
        assert!(status.success(), "git {:?} must succeed", args);
    };

    run_git(&["init"]);
    run_git(&["config", "user.name", "Test Committer"]);
    run_git(&["config", "user.email", "test@example.com"]);

    // Create initial commit on main
    fs::write(repo_root.join("README.md"), "# Initial\n").unwrap();
    run_git(&["add", "README.md"]);
    run_git(&["commit", "-m", "initial commit"]);
    run_git(&["branch", "-M", "main"]);

    // Stage a canary worktree
    let (canary_id, pipeline) = create_staged_canary(&repo_root, "dirty_canary");

    // Provide valid telemetry
    let telemetry = CanaryGateTelemetry::default();
    pipeline
        .record_canary_telemetry(&canary_id, &telemetry)
        .unwrap();

    // Adversarially inject an uncommitted untracked dirty file into repo_root
    let dirty_file = repo_root.join("adversarial_untracked.txt");
    fs::write(&dirty_file, "malicious uncommitted content").unwrap();

    // Attempt auto-merge promotion -> MUST FAIL with CanaryError::WorktreeDirty
    let err = pipeline
        .promote_canary(&canary_id, "main", true)
        .expect_err("dirty worktree must be rejected from auto-merge promotion");

    match err {
        CanaryError::WorktreeDirty(details) => {
            assert!(
                details.contains("adversarial_untracked.txt"),
                "error message must report the dirty entry; got: {details}"
            );
        }
        other => panic!("expected WorktreeDirty error, got: {other:?}"),
    }

    // Clean up the dirty file
    let _ = fs::remove_file(&dirty_file);

    // Ephemeral .pandora/ content should NOT cause WorktreeDirty
    let pandora_ephemeral = repo_root.join(".pandora").join("ephemeral.tmp");
    let _ = fs::create_dir_all(repo_root.join(".pandora"));
    fs::write(&pandora_ephemeral, "ephemeral agent cache").unwrap();

    // Auto-merge promotion should now SUCCEED despite .pandora/ files
    let receipt = pipeline
        .promote_canary(&canary_id, "main", true)
        .expect("promotion should succeed when only .pandora/ files are present");

    assert_eq!(receipt.canary_id, canary_id);
    assert!(receipt.auto_merged);

    let _ = fs::remove_dir_all(&repo_root);
}

// =========================================================================
// 5. In-toto Cryptographic Provenance Tampering & Forgery Rejection
// =========================================================================

#[test]
fn test_adversarial_provenance_tampering_and_forgery() {
    let repo_root = create_temp_test_dir("adv_provenance");
    let (canary_id, pipeline) = create_staged_canary(&repo_root, "provenance_tamper");

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
    let original: PromotionAttestationStatement = serde_json::from_slice(&bytes).unwrap();

    // Invariant: Unmodified statement verifies
    assert!(original.verify_signature());
    assert_eq!(original.signature_hex, receipt.signature_hex);

    // Attack 1: Tampered merge commit hash (pointing to malicious backdoor commit)
    let mut tampered_commit = original.clone();
    tampered_commit.predicate.merge_commit_hash =
        "deadbeefcafebabe0123456789abcdef01234567".to_string();
    assert!(
        !tampered_commit.verify_signature(),
        "Tampered merge_commit_hash must fail signature verification"
    );

    // Attack 2: Tampered author identity (spoofing author)
    let mut tampered_author = original.clone();
    tampered_author.predicate.author = "Mallory <mallory@adversary.com>".to_string();
    assert!(
        !tampered_author.verify_signature(),
        "Tampered author identity must fail signature verification"
    );

    // Attack 3: Tampered Bayesian LCB95 (inflating statistical confidence)
    let mut tampered_lcb = original.clone();
    tampered_lcb.predicate.bayesian_lcb95 = 0.9999;
    assert!(
        !tampered_lcb.verify_signature(),
        "Tampered bayesian_lcb95 must fail signature verification"
    );

    // Attack 4: Tampered Byzantine approvals (forging consensus count)
    let mut tampered_approvals = original.clone();
    tampered_approvals.predicate.byzantine_approvals = 100;
    assert!(
        !tampered_approvals.verify_signature(),
        "Tampered byzantine_approvals must fail signature verification"
    );

    // Attack 5: Tampered canary ID
    let mut tampered_id = original.clone();
    tampered_id.predicate.canary_id = "forged_canary_target".to_string();
    assert!(
        !tampered_id.verify_signature(),
        "Tampered canary_id must fail signature verification"
    );

    // Attack 6: Forged signature (flipped bits)
    let mut tampered_sig = original.clone();
    let mut sig_chars: Vec<char> = tampered_sig.signature_hex.chars().collect();
    if sig_chars[0] == '0' {
        sig_chars[0] = '1';
    } else {
        sig_chars[0] = '0';
    }
    tampered_sig.signature_hex = sig_chars.into_iter().collect();
    assert!(
        !tampered_sig.verify_signature(),
        "Corrupted signature_hex must fail verification"
    );

    // Attack 7: Forged public key (replaced with another random key)
    let mut tampered_key = original.clone();
    tampered_key.public_key_hex =
        "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f".to_string();
    assert!(
        !tampered_key.verify_signature(),
        "Replaced public_key_hex must fail verification"
    );

    // Attack 8: Malformed hex strings (odd length or non-hex)
    let mut malformed_hex = original.clone();
    malformed_hex.signature_hex = "not_valid_hex_string_xyz".to_string();
    assert!(
        !malformed_hex.verify_signature(),
        "Malformed hex must gracefully return false without crashing"
    );

    let mut odd_hex = original.clone();
    odd_hex.public_key_hex = "abc".to_string();
    assert!(
        !odd_hex.verify_signature(),
        "Odd length hex must gracefully return false without crashing"
    );

    let _ = fs::remove_dir_all(&repo_root);
}
