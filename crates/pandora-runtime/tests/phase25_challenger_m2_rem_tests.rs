#![forbid(unsafe_code)]

//! Phase 25 Milestone M2 Remediation Adversarial Challenger Test Suite.
//!
//! Empirically challenges and stress-tests:
//! 1. In-toto Statement v1 Attestation & Dalek Ed25519 Subject Binding:
//!    - Tampering with candidate merge commit hash in `subject[0].digest["merge_commit"]`
//!    - Tampering with canary name in `subject[0].name`
//!    - Tampering with attestation digest in `subject[0].digest["attestation"]`
//!    - Empty or extraneous subject vectors
//!    - Cross-canary subject swap attack
//!    - Statement metadata (`_type`, `predicateType`) tampering
//! 2. Byzantine Quorum Boundaries & Cluster Edge Cases:
//!    - Bounds probe: `approvals > total_nodes` across varied cluster configurations
//!    - Bounds probe: Sub-quorum approvals ($Q < Q_{req}$)
//!    - Bounds probe: Cluster node deficiency ($N \le 3f$) with unanimous approvals
//!    - Bounds probe: Crash fault tolerance edge case ($f = 0$)
//! 3. Real Git Auto-Merge Promotion & Invariants:
//!    - Dirty working tree rejections (untracked, modified, staged files vs permitted `.pandora/`)
//!    - Non-fast-forward merge with non-conflicting branch under strict author invariants
//!    - Non-fast-forward merge with unresolvable conflict: safe abort and clean rollback
//!    - Annotated git tag creation, tagger identity, and commit SHA target validation

use pandora_runtime::canary_worktree::{
    AttestationSubject, CanaryError, CanaryGateTelemetry, CanaryPromotionGate, CanaryStagePipeline,
    GitWorktreeManager, PromotionAttestationStatement,
};
use pandora_runtime::multi_agent_collab::ExternalAgentKind;
use pandora_runtime::multi_agent_consensus::{AgentPatchProposal, ProposalScoreVector};
use std::collections::BTreeMap;
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
    dir.push(format!("pandora_challenger_p25_m2_{name}_{nanos}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("failed to create temp test directory");
    dir
}

fn setup_real_git_repository(test_name: &str) -> PathBuf {
    let dir = create_temp_test_dir(test_name);

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
    assert!(
        commit.status.success(),
        "initial commit failed: {:?}",
        commit
    );

    dir
}

fn create_staged_canary_worktree(
    repo_root: &Path,
    canary_task: &str,
) -> (String, CanaryStagePipeline) {
    create_staged_canary_worktree_with_cert(repo_root, canary_task, 0x000A_11CE_0000)
}

fn create_staged_canary_worktree_with_cert(
    repo_root: &Path,
    canary_task: &str,
    cert_val: u64,
) -> (String, CanaryStagePipeline) {
    let cert = format!("PANDORA_PATCH_CERT_V1:{:x}", cert_val);
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

// ============================================================================
// SUITE 1: IN-TOTO STATEMENT V1 SUBJECT BINDING ATTACKS
// ============================================================================

#[test]
fn test_challenge_in_toto_tamper_subject_commit_hash_strictly_rejected() {
    let repo_root = create_temp_test_dir("in_toto_tamper_commit");
    let (canary_id, pipeline) = create_staged_canary_worktree(&repo_root, "tamper_commit");

    let telemetry = CanaryGateTelemetry::default();
    pipeline
        .record_canary_telemetry(&canary_id, &telemetry)
        .unwrap();

    let receipt = pipeline.promote_canary(&canary_id, "main", false).unwrap();
    let worktrees = GitWorktreeManager::list_worktrees(&repo_root).unwrap();
    let wt = worktrees.iter().find(|w| w.id == canary_id).unwrap();

    let attestation_path = wt.path.join(".canary_promotion_attestation.json");
    let bytes = fs::read(&attestation_path).unwrap();
    let original: PromotionAttestationStatement = serde_json::from_slice(&bytes).unwrap();

    // Baseline: legitimate statement passes verification strictly
    assert!(original.verify_signature(), "Genuine statement must verify");
    assert_eq!(original.signature_hex, receipt.signature_hex);

    // Adversarial Attack: Replace merge_commit hash in subject digest
    let mut tampered = original.clone();
    tampered.subject[0].digest.insert(
        "merge_commit".to_string(),
        "deadbeefcafebabe000011112222333344445555".to_string(),
    );

    assert!(
        !tampered.verify_signature(),
        "Tampering with subject['merge_commit'] digest MUST fail Dalek Ed25519 signature verification"
    );

    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn test_challenge_in_toto_tamper_subject_canary_name_strictly_rejected() {
    let repo_root = create_temp_test_dir("in_toto_tamper_name");
    let (canary_id, pipeline) = create_staged_canary_worktree(&repo_root, "tamper_name");

    let telemetry = CanaryGateTelemetry::default();
    pipeline
        .record_canary_telemetry(&canary_id, &telemetry)
        .unwrap();

    let _ = pipeline.promote_canary(&canary_id, "main", false).unwrap();
    let worktrees = GitWorktreeManager::list_worktrees(&repo_root).unwrap();
    let wt = worktrees.iter().find(|w| w.id == canary_id).unwrap();

    let attestation_path = wt.path.join(".canary_promotion_attestation.json");
    let bytes = fs::read(&attestation_path).unwrap();
    let original: PromotionAttestationStatement = serde_json::from_slice(&bytes).unwrap();

    // Baseline
    assert!(original.verify_signature());

    // Adversarial Attack: Replace canary name in subject
    let mut tampered = original.clone();
    tampered.subject[0].name = "pandora-canary-promotion-attacker-forged-id".to_string();

    assert!(
        !tampered.verify_signature(),
        "Tampering with subject[0].name MUST fail Dalek Ed25519 signature verification"
    );

    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn test_challenge_in_toto_tamper_subject_attestation_digest_strictly_rejected() {
    let repo_root = create_temp_test_dir("in_toto_tamper_att_digest");
    let (canary_id, pipeline) = create_staged_canary_worktree(&repo_root, "tamper_att_digest");

    let telemetry = CanaryGateTelemetry::default();
    pipeline
        .record_canary_telemetry(&canary_id, &telemetry)
        .unwrap();

    let _ = pipeline.promote_canary(&canary_id, "main", false).unwrap();
    let worktrees = GitWorktreeManager::list_worktrees(&repo_root).unwrap();
    let wt = worktrees.iter().find(|w| w.id == canary_id).unwrap();

    let attestation_path = wt.path.join(".canary_promotion_attestation.json");
    let bytes = fs::read(&attestation_path).unwrap();
    let original: PromotionAttestationStatement = serde_json::from_slice(&bytes).unwrap();

    assert!(original.verify_signature());

    // Adversarial Attack: Tamper attestation digest
    let mut tampered = original.clone();
    tampered.subject[0].digest.insert(
        "attestation".to_string(),
        "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_string(),
    );

    assert!(
        !tampered.verify_signature(),
        "Tampering with subject['attestation'] digest MUST fail Dalek Ed25519 signature verification"
    );

    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn test_challenge_in_toto_empty_or_extraneous_subject_strictly_rejected() {
    let repo_root = create_temp_test_dir("in_toto_subject_cardinality");
    let (canary_id, pipeline) = create_staged_canary_worktree(&repo_root, "cardinality");

    let telemetry = CanaryGateTelemetry::default();
    pipeline
        .record_canary_telemetry(&canary_id, &telemetry)
        .unwrap();

    let _ = pipeline.promote_canary(&canary_id, "main", false).unwrap();
    let worktrees = GitWorktreeManager::list_worktrees(&repo_root).unwrap();
    let wt = worktrees.iter().find(|w| w.id == canary_id).unwrap();

    let attestation_path = wt.path.join(".canary_promotion_attestation.json");
    let bytes = fs::read(&attestation_path).unwrap();
    let original: PromotionAttestationStatement = serde_json::from_slice(&bytes).unwrap();

    assert!(original.verify_signature());

    // Attack 1: Empty subject vector
    let mut empty_subj = original.clone();
    empty_subj.subject = Vec::new();
    assert!(
        !empty_subj.verify_signature(),
        "Empty subject vector MUST fail signature verification"
    );

    // Attack 2: Extraneous subject injected
    let mut extra_subj = original.clone();
    extra_subj.subject.push(AttestationSubject {
        name: "malicious-injected-subject".to_string(),
        digest: BTreeMap::new(),
    });
    assert!(
        !extra_subj.verify_signature(),
        "Extraneous injected subject MUST fail signature verification"
    );

    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn test_challenge_in_toto_cross_canary_subject_swap_strictly_rejected() {
    let repo_root = create_temp_test_dir("in_toto_cross_swap");
    let (canary_id_1, pipeline_1) =
        create_staged_canary_worktree_with_cert(&repo_root, "swap_alpha", 0xAAAA_1111);
    let (canary_id_2, pipeline_2) =
        create_staged_canary_worktree_with_cert(&repo_root, "swap_beta", 0xBBBB_2222);

    assert_ne!(canary_id_1, canary_id_2);

    let telemetry = CanaryGateTelemetry::default();
    pipeline_1
        .record_canary_telemetry(&canary_id_1, &telemetry)
        .unwrap();
    pipeline_2
        .record_canary_telemetry(&canary_id_2, &telemetry)
        .unwrap();

    let _ = pipeline_1
        .promote_canary(&canary_id_1, "main", false)
        .unwrap();
    let _ = pipeline_2
        .promote_canary(&canary_id_2, "main", false)
        .unwrap();

    let worktrees = GitWorktreeManager::list_worktrees(&repo_root).unwrap();
    let wt_1 = worktrees.iter().find(|w| w.id == canary_id_1).unwrap();
    let wt_2 = worktrees.iter().find(|w| w.id == canary_id_2).unwrap();

    let stmt_1: PromotionAttestationStatement = serde_json::from_slice(
        &fs::read(wt_1.path.join(".canary_promotion_attestation.json")).unwrap(),
    )
    .unwrap();
    let stmt_2: PromotionAttestationStatement = serde_json::from_slice(
        &fs::read(wt_2.path.join(".canary_promotion_attestation.json")).unwrap(),
    )
    .unwrap();

    assert!(stmt_1.verify_signature());
    assert!(stmt_2.verify_signature());
    assert_ne!(stmt_1.subject[0].name, stmt_2.subject[0].name);

    // Attack: Swap subjects between Statement 1 and Statement 2
    let mut forged_1 = stmt_1.clone();
    forged_1.subject = stmt_2.subject.clone();

    let mut forged_2 = stmt_2.clone();
    forged_2.subject = stmt_1.subject.clone();

    assert!(
        !forged_1.verify_signature(),
        "Cross-canary subject swapped into Statement 1 MUST fail verification"
    );
    assert!(
        !forged_2.verify_signature(),
        "Cross-canary subject swapped into Statement 2 MUST fail verification"
    );

    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn test_challenge_in_toto_statement_and_predicate_type_tamper_rejected() {
    let repo_root = create_temp_test_dir("in_toto_type_tamper");
    let (canary_id, pipeline) = create_staged_canary_worktree(&repo_root, "type_tamper");

    let telemetry = CanaryGateTelemetry::default();
    pipeline
        .record_canary_telemetry(&canary_id, &telemetry)
        .unwrap();

    let _ = pipeline.promote_canary(&canary_id, "main", false).unwrap();
    let worktrees = GitWorktreeManager::list_worktrees(&repo_root).unwrap();
    let wt = worktrees.iter().find(|w| w.id == canary_id).unwrap();

    let bytes = fs::read(wt.path.join(".canary_promotion_attestation.json")).unwrap();
    let original: PromotionAttestationStatement = serde_json::from_slice(&bytes).unwrap();
    assert!(original.verify_signature());

    // Attack 1: Statement type tampering
    let mut tampered_type = original.clone();
    tampered_type.statement_type = "https://in-toto.io/Statement/v2".to_string();
    assert!(
        !tampered_type.verify_signature(),
        "Tampered statement_type MUST fail verification"
    );

    // Attack 2: Predicate type tampering
    let mut tampered_pred_type = original.clone();
    tampered_pred_type.predicate_type = "https://evil.attack/v1".to_string();
    assert!(
        !tampered_pred_type.verify_signature(),
        "Tampered predicate_type MUST fail verification"
    );

    let _ = fs::remove_dir_all(&repo_root);
}

// ============================================================================
// SUITE 2: BYZANTINE QUORUM BOUNDARIES & CLUSTER EDGE CASES
// ============================================================================

#[test]
fn test_challenge_byzantine_approvals_exceeding_total_nodes_rejected() {
    // Stress test the bound: approvals > total_nodes MUST fail for any cluster.
    let cluster_cases = [
        // (total_nodes, byz_f, invalid_approvals_list)
        (4, 1, vec![5, 6, 10, 100, usize::MAX]),
        (7, 2, vec![8, 9, 20, 1000]),
        (10, 3, vec![11, 15, 100]),
        (16, 5, vec![17, 30]),
        (1, 0, vec![2, 5]),
        (3, 0, vec![4, 10]),
    ];

    let gate = CanaryPromotionGate::default();

    for (total_nodes, byz_f, invalid_approvals) in cluster_cases {
        for approvals in invalid_approvals {
            assert!(
                !CanaryPromotionGate::verify_byzantine_quorum(approvals, total_nodes, byz_f),
                "verify_byzantine_quorum must reject approvals={approvals} > total_nodes={total_nodes} (f={byz_f})"
            );

            let telemetry = CanaryGateTelemetry {
                byzantine_approvals: approvals,
                byzantine_total_nodes: total_nodes,
                byzantine_f: byz_f,
                bayesian_passes: 100,
                bayesian_failures: 0,
                bayesian_baseline: 0.85,
                min_sample_size: 10,
            };

            let err = gate.evaluate(&telemetry).expect_err(&format!(
                "evaluate() must reject approvals={approvals} > total_nodes={total_nodes}"
            ));

            match err {
                CanaryError::ByzantineQuorumNotMet {
                    approvals: app,
                    total_nodes: n,
                    byz_f: f,
                    ..
                } => {
                    assert_eq!(app, approvals);
                    assert_eq!(n, total_nodes);
                    assert_eq!(f, byz_f);
                }
                other => panic!("expected ByzantineQuorumNotMet, got: {other:?}"),
            }
        }
    }
}

#[test]
fn test_challenge_byzantine_sub_quorum_approvals_sweep() {
    // For valid cluster N=4, f=1: Q_required = (4+1)/2 + 1 = 3 (which is 2f + 1).
    for a in 0..3 {
        assert!(
            !CanaryPromotionGate::verify_byzantine_quorum(a, 4, 1),
            "Approvals {a} < 3 on N=4, f=1 must fail"
        );
    }
    assert!(CanaryPromotionGate::verify_byzantine_quorum(3, 4, 1));
    assert!(CanaryPromotionGate::verify_byzantine_quorum(4, 4, 1));

    // For valid cluster N=7, f=2: Q_required = (7+2)/2 + 1 = 5 (which is 2f + 1).
    for a in 0..5 {
        assert!(
            !CanaryPromotionGate::verify_byzantine_quorum(a, 7, 2),
            "Approvals {a} < 5 on N=7, f=2 must fail"
        );
    }
    assert!(CanaryPromotionGate::verify_byzantine_quorum(5, 7, 2));
    assert!(CanaryPromotionGate::verify_byzantine_quorum(6, 7, 2));
    assert!(CanaryPromotionGate::verify_byzantine_quorum(7, 7, 2));

    // For larger cluster N=10, f=1:
    // Quorum threshold Q = (10 + 1)/2 + 1 = 6.
    // 2f + 1 = 3, but approvals in {0, 1, 2, 3, 4, 5} must all fail!
    for a in 0..6 {
        assert!(
            !CanaryPromotionGate::verify_byzantine_quorum(a, 10, 1),
            "Approvals {a} < 6 on N=10, f=1 must fail despite a >= 2f + 1"
        );
    }
    for a in 6..=10 {
        assert!(
            CanaryPromotionGate::verify_byzantine_quorum(a, 10, 1),
            "Approvals {a} >= 6 on N=10, f=1 must succeed"
        );
    }
}

#[test]
fn test_challenge_byzantine_cluster_deficiency_sweep() {
    // In BFT, N >= 3f + 1 is mandatory.
    // If N <= 3f, verify_byzantine_quorum MUST reject even with 100% unanimous approval!
    for f in 1..=5 {
        let max_deficient_n = 3 * f;
        for n in 0..=max_deficient_n {
            // Unanimous approval approvals = n
            assert!(
                !CanaryPromotionGate::verify_byzantine_quorum(n, n, f),
                "Deficient cluster N={n} <= 3*{f} must reject even with unanimous {n}/{n} approvals"
            );
        }
    }
}

#[test]
fn test_challenge_byzantine_edge_f0_bounds() {
    // Crash-only / zero-fault assumption: f = 0.
    // N = 0: fails
    assert!(!CanaryPromotionGate::verify_byzantine_quorum(0, 0, 0));

    // N = 1, f = 0: Q = (1+0)/2 + 1 = 1.
    assert!(!CanaryPromotionGate::verify_byzantine_quorum(0, 1, 0));
    assert!(CanaryPromotionGate::verify_byzantine_quorum(1, 1, 0));
    assert!(!CanaryPromotionGate::verify_byzantine_quorum(2, 1, 0)); // 2 > 1

    // N = 3, f = 0: Q = (3+0)/2 + 1 = 2.
    assert!(!CanaryPromotionGate::verify_byzantine_quorum(0, 3, 0));
    assert!(!CanaryPromotionGate::verify_byzantine_quorum(1, 3, 0));
    assert!(CanaryPromotionGate::verify_byzantine_quorum(2, 3, 0));
    assert!(CanaryPromotionGate::verify_byzantine_quorum(3, 3, 0));
    assert!(!CanaryPromotionGate::verify_byzantine_quorum(4, 3, 0)); // 4 > 3
}

// ============================================================================
// SUITE 3: REAL GIT AUTO-MERGE PROMOTION & INVARIANTS
// ============================================================================

#[test]
fn test_challenge_real_git_dirty_working_tree_rejections() {
    let repo_root = setup_real_git_repository("dirty_tree_rejection");
    let (canary_id, pipeline) = create_staged_canary_worktree(&repo_root, "dirty_canary");

    let telemetry = CanaryGateTelemetry::default();
    pipeline
        .record_canary_telemetry(&canary_id, &telemetry)
        .unwrap();

    // 1. Untracked file rejection
    let dirty_untracked = repo_root.join("adversarial_untracked.txt");
    fs::write(&dirty_untracked, "untracked hostile content").unwrap();

    let err_untracked = pipeline
        .promote_canary(&canary_id, "main", true)
        .expect_err("must reject promotion when untracked file exists");

    match err_untracked {
        CanaryError::WorktreeDirty(details) => {
            assert!(details.contains("adversarial_untracked.txt"));
        }
        other => panic!("expected WorktreeDirty, got {other:?}"),
    }
    let _ = fs::remove_file(&dirty_untracked);

    // 2. Tracked modified file rejection
    let tracked_file = repo_root.join("src").join("service.rs");
    let original_content = fs::read_to_string(&tracked_file).unwrap();
    fs::write(
        &tracked_file,
        format!("{original_content}\n// dirty edit\n"),
    )
    .unwrap();

    let err_modified = pipeline
        .promote_canary(&canary_id, "main", true)
        .expect_err("must reject promotion when tracked file is modified");

    match err_modified {
        CanaryError::WorktreeDirty(details) => {
            assert!(details.contains("src/service.rs") || details.contains("service.rs"));
        }
        other => panic!("expected WorktreeDirty, got {other:?}"),
    }
    // Revert modified file
    fs::write(&tracked_file, &original_content).unwrap();

    // 3. Staged file rejection
    let dirty_staged = repo_root.join("staged.txt");
    fs::write(&dirty_staged, "staged hostile content").unwrap();
    let _ = Command::new("git")
        .args(["add", "staged.txt"])
        .current_dir(&repo_root)
        .output();

    let err_staged = pipeline
        .promote_canary(&canary_id, "main", true)
        .expect_err("must reject promotion when staged file exists");

    match err_staged {
        CanaryError::WorktreeDirty(details) => {
            assert!(details.contains("staged.txt"));
        }
        other => panic!("expected WorktreeDirty, got {other:?}"),
    }
    let _ = Command::new("git")
        .args(["rm", "-f", "staged.txt"])
        .current_dir(&repo_root)
        .output();

    // 4. Ephemeral .pandora directory MUST NOT trigger WorktreeDirty
    let pandora_cache = repo_root.join(".pandora").join("ephemeral_test.cache");
    let _ = fs::create_dir_all(repo_root.join(".pandora"));
    fs::write(&pandora_cache, "agent ephemeral telemetry").unwrap();

    let receipt = pipeline
        .promote_canary(&canary_id, "main", true)
        .expect("must succeed when only .pandora/ content is present");

    assert!(receipt.auto_merged);
    assert_eq!(receipt.canary_id, canary_id);

    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn test_challenge_real_git_non_fast_forward_merge_non_conflicting() {
    let repo_root = setup_real_git_repository("clean_no_ff");
    let (canary_id, pipeline) = create_staged_canary_worktree(&repo_root, "clean_no_ff");

    // Advance main with a concurrent disjoint commit
    let main_extra = repo_root.join("src").join("main_extra.rs");
    fs::write(
        &main_extra,
        "// Disjoint mainline feature\npub fn mainline_fn() -> u32 { 100 }\n",
    )
    .unwrap();
    let _ = Command::new("git")
        .args(["add", "."])
        .current_dir(&repo_root)
        .output();
    let _ = Command::new("git")
        .args(["commit", "-m", "feat: advance main concurrently"])
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
        .output();

    let telemetry = CanaryGateTelemetry::default();
    pipeline
        .record_canary_telemetry(&canary_id, &telemetry)
        .unwrap();

    let receipt = pipeline
        .promote_canary(&canary_id, "main", true)
        .expect("non-fast-forward merge must succeed");

    assert!(receipt.auto_merged);
    assert_eq!(receipt.target_branch, "main");
    assert_eq!(
        receipt.author,
        "ANISAYAK MITRA <266799942+anisayakmitra-in@users.noreply.github.com>"
    );

    // Verify HEAD is a merge commit with 2 parents
    let head_rev = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&repo_root)
        .output()
        .unwrap();
    let head_sha = String::from_utf8_lossy(&head_rev.stdout).trim().to_string();
    assert_eq!(head_sha, receipt.merge_commit_hash);

    let parent1 = Command::new("git")
        .args(["rev-parse", "HEAD^1"])
        .current_dir(&repo_root)
        .output()
        .unwrap();
    let parent2 = Command::new("git")
        .args(["rev-parse", "HEAD^2"])
        .current_dir(&repo_root)
        .output()
        .unwrap();
    assert!(parent1.status.success(), "HEAD must have parent 1");
    assert!(
        parent2.status.success(),
        "HEAD must have parent 2 (true non-fast-forward merge)"
    );

    // Verify both files exist
    assert!(repo_root.join("src").join("main_extra.rs").exists());
    assert!(repo_root.join("cache.rs").exists());

    // Verify merge commit author and committer
    let author_query = Command::new("git")
        .args(["log", "-1", "--format=%an <%ae>|%cn <%ce>"])
        .current_dir(&repo_root)
        .output()
        .unwrap();
    let author_str = String::from_utf8_lossy(&author_query.stdout)
        .trim()
        .to_string();
    assert_eq!(
        author_str,
        "ANISAYAK MITRA <266799942+anisayakmitra-in@users.noreply.github.com>|ANISAYAK MITRA <266799942+anisayakmitra-in@users.noreply.github.com>"
    );

    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn test_challenge_real_git_merge_conflicting_branches_unresolvable_aborts_cleanly() {
    let repo_root = setup_real_git_repository("conflict_abort_clean");

    // 1. First advance main with a commit on `shared.rs` that has unclosed delimiter
    let shared_file = repo_root.join("shared.rs");
    fs::write(&shared_file, "pub fn shared_ours( { 111\n").unwrap();
    let _ = Command::new("git")
        .args(["add", "shared.rs"])
        .current_dir(&repo_root)
        .output();
    let _ = Command::new("git")
        .args(["commit", "-m", "main bad commit"])
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
        .output();

    // 2. Create canary worktree on a new branch with conflicting unclosed delimiter
    let cert = format!("PANDORA_PATCH_CERT_V1:{:064x}", 0x777);
    let branch = "pandora/canary/conflict_abort_task";
    let mut worktree = GitWorktreeManager::create_worktree(&repo_root, branch, &cert).unwrap();

    // Canary commit has another unclosed delimiter
    let patch = AgentPatchProposal::new(
        "patch_conflict_abort",
        ExternalAgentKind::ClaudeCode,
        "Conflicting candidate patch",
        "Modifies shared.rs with another syntax error",
        "+++ b/shared.rs\n-pub fn shared_ours( { 111\n+pub fn shared_theirs( { 222\n",
        ProposalScoreVector::new(0.95, 0.95, 0.95, 0.95),
        10,
    );

    let pipeline = CanaryStagePipeline::default().with_repo_root(&repo_root);
    // Stage patch (fallback apply diff will write the line)
    // Note: sandbox verification checks memory safety and size bounds, which pass here
    let receipt = pipeline
        .stage_candidate_patch(&mut worktree, &patch)
        .unwrap();

    let telemetry = CanaryGateTelemetry::default();
    pipeline
        .record_canary_telemetry(&receipt.canary_id, &telemetry)
        .unwrap();

    // 3. Concurrently modify `shared.rs` on main so that git merge creates a genuine conflict
    fs::write(&shared_file, "pub fn shared_ours_variant( { 333\n").unwrap();
    let _ = Command::new("git")
        .args(["add", "shared.rs"])
        .current_dir(&repo_root)
        .output();
    let _ = Command::new("git")
        .args(["commit", "-m", "main conflicting syntax error"])
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
        .output();

    // Now promote with auto_merge = true:
    // Git merge will conflict on shared.rs.
    // In tripartite resolution:
    // Attempt 1: AdditiveUnion -> contains unclosed delimiters -> Critic rejects.
    // Attempt 2: PreferOurs -> contains unclosed delimiter -> Critic rejects.
    // Attempt 3: PreferTheirs -> contains unclosed delimiter -> Critic rejects.
    // All 3 attempts exhausted! Resolution fails.
    // execute_mainline_merge calls `git merge --abort` and returns `Err(CanaryError::MergeConflict)`.
    let promo_err = pipeline.promote_canary(&receipt.canary_id, "main", true);
    assert!(
        promo_err.is_err(),
        "Promotion with unresolvable conflict across all 3 strategies MUST fail"
    );

    match promo_err {
        Err(CanaryError::MergeConflict(_)) => {}
        other => panic!("expected CanaryError::MergeConflict, got: {other:?}"),
    }

    // Verify git working tree is cleanly aborted (no MERGE_HEAD)
    let merge_head = repo_root.join(".git").join("MERGE_HEAD");
    assert!(
        !merge_head.exists(),
        "MERGE_HEAD must not exist after failed promotion; git merge --abort must have been executed"
    );

    let status_out = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&repo_root)
        .output()
        .unwrap();
    let status_str = String::from_utf8_lossy(&status_out.stdout);
    let dirty_count = status_str
        .lines()
        .filter(|l| !l.contains(".pandora") && !l.trim().is_empty())
        .count();
    assert_eq!(
        dirty_count, 0,
        "Working tree must be clean after merge abort"
    );

    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn test_challenge_real_git_annotated_tag_and_tagger_identity() {
    let repo_root = setup_real_git_repository("annotated_tag_validation");
    let (canary_id, pipeline) = create_staged_canary_worktree(&repo_root, "tag_val");

    let telemetry = CanaryGateTelemetry::default();
    pipeline
        .record_canary_telemetry(&canary_id, &telemetry)
        .unwrap();

    let receipt = pipeline
        .promote_canary(&canary_id, "main", true)
        .expect("auto-merge promotion must succeed");

    let expected_tag = format!("pandora/canary/promoted/{canary_id}");
    assert_eq!(receipt.tag_name, expected_tag);

    // 1. Verify annotated tag object (cat-file -t == tag)
    let type_out = Command::new("git")
        .args(["cat-file", "-t", &expected_tag])
        .current_dir(&repo_root)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&type_out.stdout).trim(),
        "tag",
        "Created tag MUST be an annotated tag, not a lightweight reference"
    );

    // 2. Verify tag target commit is the merge commit
    let target_out = Command::new("git")
        .args(["rev-parse", &format!("{expected_tag}^{{commit}}")])
        .current_dir(&repo_root)
        .output()
        .unwrap();
    let target_sha = String::from_utf8_lossy(&target_out.stdout)
        .trim()
        .to_string();
    assert_eq!(target_sha, receipt.merge_commit_hash);

    // 3. Verify tagger name and email (git's %(taggeremail) already includes angle brackets)
    let tagger_out = Command::new("git")
        .args([
            "for-each-ref",
            "--format=%(taggername) %(taggeremail)",
            &format!("refs/tags/{expected_tag}"),
        ])
        .current_dir(&repo_root)
        .output()
        .unwrap();
    let tagger_str = String::from_utf8_lossy(&tagger_out.stdout)
        .trim()
        .to_string();
    assert_eq!(
        tagger_str, "ANISAYAK MITRA <266799942+anisayakmitra-in@users.noreply.github.com>",
        "Annotated tagger identity MUST strictly match authorized committer"
    );

    // 4. Verify tag message body
    let cat_p = Command::new("git")
        .args(["cat-file", "-p", &expected_tag])
        .current_dir(&repo_root)
        .output()
        .unwrap();
    let cat_body = String::from_utf8_lossy(&cat_p.stdout);
    assert!(cat_body.contains(&format!("Canary promotion {canary_id} merged")));

    let _ = fs::remove_dir_all(&repo_root);
}
