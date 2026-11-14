#![forbid(unsafe_code)]

//! Phase 25 Empirical Adversarial Challenge Suite: Canary Auto-Merge Dual-Gating.
//!
//! Stress-tests and adversarially challenges:
//! 1. Byzantine Quorum Gate: boundary conditions (approvals = 2f vs 2f+1), cluster size constraints (N < 3f+1),
//!    equivocation detection, and contradictory vote tallying.
//! 2. Bayesian LCB95 Gate: boundary conditions (LCB95 = baseline, LCB95 < baseline), sample size bounds (n < 10),
//!    and noisy / high-variance evaluations.
//! 3. Cryptographic Provenance: In-toto Statement v1 signature forgery, key replacement, predicate tampering,
//!    and subject digest verification.
//! 4. Git Author Invariant: commits and annotated tags (`pandora/canary/promoted/<canary_id>`) under hostile
//!    git environment configurations.

use ed25519_dalek::{Signer, SigningKey};
use pandora_runtime::canary_worktree::{
    CanaryError, CanaryGateTelemetry, CanaryPromotionGate, CanaryStagePipeline, GitWorktreeManager,
    PromotionAttestationStatement,
};
use pandora_runtime::distributed_swarm::{
    DistributedConsensusDecision, DistributedConsensusVote, DistributedPatchProposal,
    DistributedSwarmConsensusEngine, DistributedSwarmError,
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
    dir.push(format!(
        "pandora_adversarial_{name}_{}_{nanos}",
        std::process::id()
    ));
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
// SUITE 1: BYZANTINE QUORUM GATE ADVERSARIAL CHALLENGES
// ============================================================================

#[test]
fn test_adversarial_byzantine_boundary_approvals_2f_fails() {
    // Exact boundary: approvals = 2f MUST fail for all valid clusters N >= 3f + 1
    let test_cases = [
        (4, 1),   // f=1: 2f = 2 (fails)
        (7, 2),   // f=2: 2f = 4 (fails)
        (10, 3),  // f=3: 2f = 6 (fails)
        (16, 5),  // f=5: 2f = 10 (fails)
        (31, 10), // f=10: 2f = 20 (fails)
    ];

    for (total_nodes, byz_f) in test_cases {
        let approvals = 2 * byz_f;
        assert!(
            !CanaryPromotionGate::verify_byzantine_quorum(approvals, total_nodes, byz_f),
            "Byzantine gate MUST FAIL when approvals = 2f ({approvals}) for N={total_nodes}, f={byz_f}"
        );

        // Sub-boundary: 2f - 1 approvals must also fail
        if approvals > 0 {
            assert!(
                !CanaryPromotionGate::verify_byzantine_quorum(approvals - 1, total_nodes, byz_f),
                "Byzantine gate MUST FAIL when approvals = 2f - 1 ({}) for N={total_nodes}, f={byz_f}",
                approvals - 1
            );
        }

        // Test via CanaryPromotionGate::evaluate()
        let gate = CanaryPromotionGate::default();
        let telemetry = CanaryGateTelemetry {
            byzantine_approvals: approvals,
            byzantine_total_nodes: total_nodes,
            byzantine_f: byz_f,
            bayesian_passes: 50,
            bayesian_failures: 0,
            bayesian_baseline: 0.85,
            min_sample_size: 10,
        };

        let err = gate
            .evaluate(&telemetry)
            .expect_err("gate must reject approvals = 2f");
        match err {
            CanaryError::ByzantineQuorumNotMet {
                approvals: app,
                required,
                byz_f: f,
                total_nodes: n,
            } => {
                assert_eq!(app, approvals);
                assert_eq!(f, byz_f);
                assert_eq!(n, total_nodes);
                assert!(
                    required > 2 * byz_f,
                    "required ({required}) must be >= 2f + 1 ({})",
                    2 * byz_f + 1
                );
            }
            other => panic!("expected ByzantineQuorumNotMet, got: {other:?}"),
        }
    }
}

#[test]
fn test_adversarial_byzantine_boundary_approvals_2f_plus_1_passes() {
    // Exact boundary: approvals = 2f + 1 MUST pass for minimal clusters N = 3f + 1
    let test_cases = [
        (4, 1),   // f=1: 2f + 1 = 3 (passes)
        (7, 2),   // f=2: 2f + 1 = 5 (passes)
        (10, 3),  // f=3: 2f + 1 = 7 (passes)
        (16, 5),  // f=5: 2f + 1 = 11 (passes)
        (31, 10), // f=10: 2f + 1 = 21 (passes)
    ];

    for (total_nodes, byz_f) in test_cases {
        let approvals = 2 * byz_f + 1;
        assert!(
            CanaryPromotionGate::verify_byzantine_quorum(approvals, total_nodes, byz_f),
            "Byzantine gate MUST PASS when approvals = 2f + 1 ({approvals}) for N={total_nodes}, f={byz_f}"
        );

        // Test via CanaryPromotionGate::evaluate()
        let gate = CanaryPromotionGate::default();
        let telemetry = CanaryGateTelemetry {
            byzantine_approvals: approvals,
            byzantine_total_nodes: total_nodes,
            byzantine_f: byz_f,
            bayesian_passes: 50,
            bayesian_failures: 0,
            bayesian_baseline: 0.85,
            min_sample_size: 10,
        };

        let result = gate.evaluate(&telemetry);
        assert!(
            result.is_ok(),
            "gate must evaluate to Ok when approvals = 2f + 1"
        );
    }
}

#[test]
fn test_adversarial_byzantine_invalid_cluster_size_rejected() {
    // Clusters violating BFT invariant N >= 3f + 1 must be strictly rejected
    // Even if approvals are 100% unanimous!
    let invalid_clusters = [
        (3, 1), // N = 3f
        (2, 1), // N < 3f
        (1, 1),
        (6, 2), // N = 3f
        (5, 2),
        (4, 2),
        (9, 3), // N = 3f
        (0, 0), // degenerate
    ];

    for (total_nodes, byz_f) in invalid_clusters {
        // Even with unanimous approval of all nodes:
        let unanimous = total_nodes;
        assert!(
            !CanaryPromotionGate::verify_byzantine_quorum(unanimous, total_nodes, byz_f),
            "Cluster N={total_nodes}, f={byz_f} violates N >= 3f + 1 and MUST BE REJECTED even with unanimous approvals"
        );

        let gate = CanaryPromotionGate::default();
        let telemetry = CanaryGateTelemetry {
            byzantine_approvals: unanimous,
            byzantine_total_nodes: total_nodes,
            byzantine_f: byz_f,
            bayesian_passes: 50,
            bayesian_failures: 0,
            bayesian_baseline: 0.85,
            min_sample_size: 10,
        };

        let err = gate
            .evaluate(&telemetry)
            .expect_err("invalid cluster must be rejected");
        match err {
            CanaryError::ByzantineQuorumNotMet { .. } => {}
            other => panic!("expected ByzantineQuorumNotMet, got: {other:?}"),
        }
    }
}

#[test]
fn test_adversarial_byzantine_equivocation_and_contradictory_votes() {
    // Tests equivocation detection in the underlying consensus engine
    let mut engine = DistributedSwarmConsensusEngine::new("test_cluster", 4, 1).unwrap();
    let proposal = DistributedPatchProposal::new(
        "patch_adv_1",
        "node_proposer",
        ExternalAgentKind::ClaudeCode,
        "Adversarial patch",
        "description",
        "diff",
        ProposalScoreVector::new(0.9, 0.9, 0.9, 0.9),
        1000,
    );
    let proposal_id = engine.submit_proposal(proposal).unwrap();

    // Node 1 votes YES with score vector (0.9, 0.9, 0.9, 0.9)
    let vote_yes = DistributedConsensusVote {
        proposal_id: proposal_id.clone(),
        voter_node_id: "node_1".to_string(),
        approve: true,
        verified_scores: ProposalScoreVector::new(0.9, 0.9, 0.9, 0.9),
        signature: "sig1".to_string(),
        timestamp_ms: 1000,
    };
    let decision = engine.record_vote(vote_yes).unwrap();
    match decision {
        DistributedConsensusDecision::Pending {
            current_approvals, ..
        } => {
            assert_eq!(current_approvals, 1);
        }
        _ => panic!("expected pending decision"),
    }

    // Byzantine Attack 1: Node 1 casts contradictory vote (approve: false)
    let vote_no = DistributedConsensusVote {
        proposal_id: proposal_id.clone(),
        voter_node_id: "node_1".to_string(),
        approve: false,
        verified_scores: ProposalScoreVector::new(0.9, 0.9, 0.9, 0.9),
        signature: "sig1_evil".to_string(),
        timestamp_ms: 1001,
    };
    let err_equivocation = engine
        .record_vote(vote_no)
        .expect_err("conflicting vote must be caught as equivocation");
    match err_equivocation {
        DistributedSwarmError::EquivocationDetected { voter, proposal } => {
            assert_eq!(voter, "node_1");
            assert_eq!(proposal, proposal_id);
        }
        other => panic!("expected EquivocationDetected, got: {other:?}"),
    }

    // Byzantine Attack 2: Node 1 votes YES again but with divergent scores
    let vote_divergent = DistributedConsensusVote {
        proposal_id: proposal_id.clone(),
        voter_node_id: "node_1".to_string(),
        approve: true,
        verified_scores: ProposalScoreVector::new(0.1, 0.1, 0.1, 0.1), // ||s1 - s2|| > 1e-4
        signature: "sig1_div".to_string(),
        timestamp_ms: 1002,
    };
    let err_divergent = engine
        .record_vote(vote_divergent)
        .expect_err("divergent score must be caught as equivocation");
    match err_divergent {
        DistributedSwarmError::EquivocationDetected { voter, .. } => {
            assert_eq!(voter, "node_1");
        }
        other => panic!("expected EquivocationDetected, got: {other:?}"),
    }

    // Honest Re-submission: Identical vote is idempotent and does NOT inflate approval count
    let vote_identical = DistributedConsensusVote {
        proposal_id: proposal_id.clone(),
        voter_node_id: "node_1".to_string(),
        approve: true,
        verified_scores: ProposalScoreVector::new(0.9, 0.9, 0.9, 0.9),
        signature: "sig1".to_string(),
        timestamp_ms: 1000,
    };
    let decision_idempotent = engine.record_vote(vote_identical).unwrap();
    match decision_idempotent {
        DistributedConsensusDecision::Pending {
            current_approvals, ..
        } => {
            assert_eq!(
                current_approvals, 1,
                "idempotent vote must not inflate approvals"
            );
        }
        _ => panic!("expected pending decision"),
    }
}

// ============================================================================
// SUITE 2: BAYESIAN LCB95 GATE ADVERSARIAL CHALLENGES
// ============================================================================

#[test]
fn test_adversarial_bayesian_boundary_lcb95_equals_baseline_fails() {
    // Strict inequality requirement: LCB95 > baseline
    // If LCB95 == baseline, promotion MUST fail.
    let (promoted_pre, lcb95) = CanaryPromotionGate::verify_bayesian_lcb95(50, 0, 0.80);
    assert!(promoted_pre);

    // Set baseline exactly equal to lcb95:
    let (promoted_exact, evaluated_lcb95) =
        CanaryPromotionGate::verify_bayesian_lcb95(50, 0, lcb95);
    assert!(
        !promoted_exact,
        "Promotion MUST FAIL when LCB95 ({evaluated_lcb95}) == baseline ({lcb95})"
    );

    // Evaluate via CanaryPromotionGate::evaluate()
    let gate = CanaryPromotionGate::default();
    let telemetry = CanaryGateTelemetry {
        byzantine_approvals: 3,
        byzantine_total_nodes: 4,
        byzantine_f: 1,
        bayesian_passes: 50,
        bayesian_failures: 0,
        bayesian_baseline: lcb95,
        min_sample_size: 10,
    };

    let err = gate
        .evaluate(&telemetry)
        .expect_err("LCB95 == baseline must fail");
    match err {
        CanaryError::BayesianGateFailed {
            lcb95: err_lcb,
            baseline: err_base,
            ..
        } => {
            assert!(
                (err_lcb - err_base).abs() < 1e-12,
                "LCB95 must match baseline exactly"
            );
        }
        other => panic!("expected BayesianGateFailed, got: {other:?}"),
    }
}

#[test]
fn test_adversarial_bayesian_boundary_lcb95_below_baseline_fails() {
    // If LCB95 < baseline by any epsilon, promotion MUST fail
    let (_, lcb95) = CanaryPromotionGate::verify_bayesian_lcb95(40, 5, 0.50);
    let baseline = lcb95 + 1e-4;

    let (promoted, ret_lcb95) = CanaryPromotionGate::verify_bayesian_lcb95(40, 5, baseline);
    assert!(
        !promoted,
        "Promotion MUST FAIL when LCB95 ({ret_lcb95}) < baseline ({baseline})"
    );

    let gate = CanaryPromotionGate::default();
    let telemetry = CanaryGateTelemetry {
        byzantine_approvals: 3,
        byzantine_total_nodes: 4,
        byzantine_f: 1,
        bayesian_passes: 40,
        bayesian_failures: 5,
        bayesian_baseline: baseline,
        min_sample_size: 10,
    };

    let err = gate
        .evaluate(&telemetry)
        .expect_err("LCB95 < baseline must fail");
    match err {
        CanaryError::BayesianGateFailed {
            lcb95: lcb,
            baseline: base,
            ..
        } => {
            assert!(lcb < base);
        }
        other => panic!("expected BayesianGateFailed, got: {other:?}"),
    }
}

#[test]
fn test_adversarial_bayesian_sample_size_boundary_n_less_than_10_rejected() {
    let gate = CanaryPromotionGate::default();

    // Test all sample sizes from n = 0 to n = 9 with 100% pass rate
    for n in 0..10u32 {
        let telemetry = CanaryGateTelemetry {
            byzantine_approvals: 3,
            byzantine_total_nodes: 4,
            byzantine_f: 1,
            bayesian_passes: n,
            bayesian_failures: 0,
            bayesian_baseline: 0.10, // Generous baseline
            min_sample_size: 10,
        };

        let err = gate
            .evaluate(&telemetry)
            .expect_err(&format!("n={n} < 10 MUST fail sample size gate"));
        match err {
            CanaryError::InsufficientSampleSize {
                total_evaluations,
                minimum_required,
            } => {
                assert_eq!(total_evaluations, n);
                assert_eq!(minimum_required, 10);
            }
            other => panic!("expected InsufficientSampleSize for n={n}, got: {other:?}"),
        }
    }

    // Boundary n = 10 (passes=10, failures=0) with 10 minimum evaluations MUST NOT return InsufficientSampleSize
    let telemetry_10 = CanaryGateTelemetry {
        byzantine_approvals: 3,
        byzantine_total_nodes: 4,
        byzantine_f: 1,
        bayesian_passes: 10,
        bayesian_failures: 0,
        bayesian_baseline: 0.50,
        min_sample_size: 10,
    };
    let res = gate.evaluate(&telemetry_10);
    assert!(
        res.is_ok(),
        "n=10 with 100% pass rate must pass sample size and Bayesian gates"
    );
}

#[test]
fn test_adversarial_bayesian_noisy_evaluations_and_high_variance() {
    // Scenario 1: High variance 50% split (50 passes, 50 failures).
    // Empirical pass rate = 50%, but baseline = 0.50.
    // LCB95 for Beta(51, 51) is approx 0.418. Must fail!
    let (promoted_5050, lcb_5050) = CanaryPromotionGate::verify_bayesian_lcb95(50, 50, 0.50);
    assert!(!promoted_5050);
    assert!(
        lcb_5050 < 0.45,
        "LCB95 ({lcb_5050:.4}) must account for Bayesian uncertainty"
    );

    // Scenario 2: Small sample optimistic noise: 8 passes, 2 failures (n=10).
    // Empirical pass rate = 80%. Baseline = 0.70.
    // LCB95 for Beta(9, 3) is approx 0.52 < 0.70. Gate must reject!
    let (promoted_noisy_opt, lcb_opt) = CanaryPromotionGate::verify_bayesian_lcb95(8, 2, 0.70);
    assert!(
        !promoted_noisy_opt,
        "Small sample optimistic noise (8/10 = 80%) must reject against baseline 0.70 (LCB95 = {lcb_opt:.4})"
    );

    // Scenario 3: Extreme baseline inputs:
    // NaN baseline must fail
    let (promoted_nan, _) = CanaryPromotionGate::verify_bayesian_lcb95(100, 0, f64::NAN);
    assert!(!promoted_nan, "NaN baseline must never promote");

    // Impossible baseline > 1.0 must fail
    let (promoted_over, _) = CanaryPromotionGate::verify_bayesian_lcb95(100, 0, 1.01);
    assert!(!promoted_over, "Baseline > 1.0 must never promote");
}

// ============================================================================
// SUITE 3: CRYPTOGRAPHIC PROVENANCE ADVERSARIAL FORGERY & TAMPERING
// ============================================================================

#[test]
fn test_adversarial_in_toto_signature_forgery_and_tampering() {
    let repo_root = create_temp_test_dir("in_toto_forgery");
    let (canary_id, pipeline) = create_staged_canary_worktree(&repo_root, "forge_test");

    let telemetry = CanaryGateTelemetry::default();
    pipeline
        .record_canary_telemetry(&canary_id, &telemetry)
        .unwrap();

    let receipt = pipeline.promote_canary(&canary_id, "main", false).unwrap();
    let worktrees = GitWorktreeManager::list_worktrees(&repo_root).unwrap();
    let wt = worktrees.iter().find(|w| w.id == canary_id).unwrap();

    let attestation_path = wt.path.join(".canary_promotion_attestation.json");
    let bytes = fs::read(&attestation_path).unwrap();
    let original_statement: PromotionAttestationStatement = serde_json::from_slice(&bytes).unwrap();

    // Baseline: genuine statement strictly verifies
    assert!(
        original_statement.verify_signature(),
        "Genuine statement must verify"
    );
    assert_eq!(original_statement.signature_hex, receipt.signature_hex);

    // ------------------------------------------------------------------------
    // Attack 1: Mutate signature (bit-flip)
    // ------------------------------------------------------------------------
    let mut forged_statement = original_statement.clone();
    let mut sig_chars: Vec<char> = forged_statement.signature_hex.chars().collect();
    // Flip first character
    sig_chars[0] = if sig_chars[0] == '0' { '1' } else { '0' };
    forged_statement.signature_hex = sig_chars.into_iter().collect();
    assert!(
        !forged_statement.verify_signature(),
        "Bit-flipped signature MUST BE REJECTED"
    );

    // ------------------------------------------------------------------------
    // Attack 2: Truncate signature
    // ------------------------------------------------------------------------
    let mut truncated_sig_statement = original_statement.clone();
    truncated_sig_statement.signature_hex = original_statement.signature_hex[..60].to_string();
    assert!(
        !truncated_sig_statement.verify_signature(),
        "Truncated signature MUST BE REJECTED"
    );

    // ------------------------------------------------------------------------
    // Attack 3: Non-hex invalid characters in signature
    // ------------------------------------------------------------------------
    let mut non_hex_statement = original_statement.clone();
    non_hex_statement.signature_hex = "Z".repeat(128);
    assert!(
        !non_hex_statement.verify_signature(),
        "Non-hex signature string MUST BE REJECTED"
    );

    // ------------------------------------------------------------------------
    // Attack 4: Rogue key-pair forgery (signed by an unapproved attacker key)
    // ------------------------------------------------------------------------
    let rogue_seed = [0x55u8; 32];
    let rogue_key = SigningKey::from_bytes(&rogue_seed);
    let predicate_bytes = serde_json::to_vec(&original_statement.predicate).unwrap();
    let rogue_sig = rogue_key.sign(&predicate_bytes);
    let mut hex_sig = String::new();
    for b in rogue_sig.to_bytes() {
        use std::fmt::Write;
        let _ = write!(hex_sig, "{b:02x}");
    }

    let mut rogue_sig_statement = original_statement.clone();
    rogue_sig_statement.signature_hex = hex_sig;
    assert!(
        !rogue_sig_statement.verify_signature(),
        "Signature produced by rogue key MUST BE REJECTED when verified against host key"
    );

    // ------------------------------------------------------------------------
    // Attack 5: Tamper with Predicate Fields
    // ------------------------------------------------------------------------
    // 5A. Tamper with canary_id
    let mut tampered_pred_1 = original_statement.clone();
    tampered_pred_1.predicate.canary_id = "canary_evil_override".to_string();
    assert!(
        !tampered_pred_1.verify_signature(),
        "Tampering with predicate.canary_id MUST BE REJECTED"
    );

    // 5B. Tamper with merge_commit_hash
    let mut tampered_pred_2 = original_statement.clone();
    tampered_pred_2.predicate.merge_commit_hash =
        "0000000000000000000000000000000000000000".to_string();
    assert!(
        !tampered_pred_2.verify_signature(),
        "Tampering with predicate.merge_commit_hash MUST BE REJECTED"
    );

    // 5C. Tamper with author identity
    let mut tampered_pred_3 = original_statement.clone();
    tampered_pred_3.predicate.author = "Attacker <attacker@evil.com>".to_string();
    assert!(
        !tampered_pred_3.verify_signature(),
        "Tampering with predicate.author MUST BE REJECTED"
    );

    // 5D. Tamper with byzantine_approvals count
    let mut tampered_pred_4 = original_statement.clone();
    tampered_pred_4.predicate.byzantine_approvals = 100;
    assert!(
        !tampered_pred_4.verify_signature(),
        "Tampering with predicate.byzantine_approvals MUST BE REJECTED"
    );

    // 5E. Tamper with bayesian_lcb95
    let mut tampered_pred_5 = original_statement.clone();
    tampered_pred_5.predicate.bayesian_lcb95 = 0.9999;
    assert!(
        !tampered_pred_5.verify_signature(),
        "Tampering with predicate.bayesian_lcb95 MUST BE REJECTED"
    );

    // ------------------------------------------------------------------------
    // Empirical Observation: Statement Subject Tampering
    // ------------------------------------------------------------------------
    // Check if statement.subject is bound by verify_signature():
    let mut tampered_subject = original_statement.clone();
    tampered_subject.subject[0].name = "attacker-tampered-subject".to_string();
    let subject_bound = !tampered_subject.verify_signature();
    assert!(
        subject_bound,
        "Statement subject MUST BE cryptographically bound in verify_signature()"
    );
    eprintln!(
        "[EMPIRICAL OBSERVATION] Is statement.subject cryptographically bound in verify_signature()? {subject_bound}"
    );

    let _ = fs::remove_dir_all(&repo_root);
}

// ============================================================================
// SUITE 4: GIT AUTHOR INVARIANT UNDER HOSTILE ENVIRONMENT
// ============================================================================

#[test]
fn test_adversarial_git_author_invariant_under_hostile_environment() {
    let repo_root = create_temp_test_dir("git_author_hostile");

    // Initialize a real Git repository in repo_root with a hostile default author
    let init_out = Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(&repo_root)
        .output()
        .expect("git init must succeed");
    assert!(init_out.status.success());

    // Set hostile local git config
    let _ = Command::new("git")
        .args(["config", "user.name", "Hostile Attacker"])
        .current_dir(&repo_root)
        .output();
    let _ = Command::new("git")
        .args(["config", "user.email", "attacker@adversary.com"])
        .current_dir(&repo_root)
        .output();

    // Create an initial commit on main
    let readme = repo_root.join("README.md");
    fs::write(&readme, "# AGENT-PANDORA\nInitial mainline commit.\n").unwrap();
    let _ = Command::new("git")
        .args(["add", "README.md"])
        .current_dir(&repo_root)
        .output();
    let _ = Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(&repo_root)
        .output();

    // Stage a candidate canary patch on branch `pandora/canary/adv_author`
    let cert = format!("PANDORA_PATCH_CERT_V1:{:064x}", 0xA11CE);
    let branch = "pandora/canary/adv_author";

    // Create branch in git
    let _ = Command::new("git")
        .args(["checkout", "-b", branch])
        .current_dir(&repo_root)
        .output();

    // Add a canary file on the canary branch
    let canary_file = repo_root.join("canary_feature.rs");
    fs::write(
        &canary_file,
        "// Canary feature implementation\npub fn canary_ok() -> bool { true }\n",
    )
    .unwrap();
    let _ = Command::new("git")
        .args(["add", "canary_feature.rs"])
        .current_dir(&repo_root)
        .output();
    let _ = Command::new("git")
        .args(["commit", "-m", "canary patch"])
        .current_dir(&repo_root)
        .output();

    // Switch back to main
    let _ = Command::new("git")
        .args(["checkout", "main"])
        .current_dir(&repo_root)
        .output();

    // Create worktree tracking metadata in .pandora/worktrees
    let mut worktree = GitWorktreeManager::create_worktree(&repo_root, branch, &cert).unwrap();
    let patch = AgentPatchProposal::new(
        "adv_patch",
        ExternalAgentKind::ClaudeCode,
        "Canary feature",
        "description",
        "diff",
        ProposalScoreVector::new(0.99, 0.99, 0.99, 0.99),
        42,
    );
    let pipeline = CanaryStagePipeline::default().with_repo_root(&repo_root);
    let receipt = pipeline
        .stage_candidate_patch(&mut worktree, &patch)
        .unwrap();

    let telemetry = CanaryGateTelemetry::default();
    pipeline
        .record_canary_telemetry(&receipt.canary_id, &telemetry)
        .unwrap();

    // Promote with auto_merge = true
    let promo_receipt = pipeline
        .promote_canary(&receipt.canary_id, "main", true)
        .expect("promotion with auto-merge must succeed");

    assert!(promo_receipt.auto_merged);

    // 1. Assert Git commit author and committer are strictly ANISAYAK MITRA
    let log_out = Command::new("git")
        .args(["log", "-1", "--format=%an <%ae>|%cn <%ce>"])
        .current_dir(&repo_root)
        .output()
        .expect("git log must succeed");
    let log_str = String::from_utf8_lossy(&log_out.stdout).trim().to_string();

    let expected_author = "ANISAYAK MITRA <266799942+anisayakmitra-in@users.noreply.github.com>";
    let expected_committer = "ANISAYAK MITRA <266799942+anisayakmitra-in@users.noreply.github.com>";
    let expected_combined = format!("{expected_author}|{expected_committer}");

    assert_eq!(
        log_str, expected_combined,
        "Git merge commit MUST strictly use author and committer '{expected_combined}', but got '{log_str}'"
    );

    // 2. Assert annotated tag tagger is strictly ANISAYAK MITRA
    let tag_name = format!("pandora/canary/promoted/{}", receipt.canary_id);
    let tag_out = Command::new("git")
        .args([
            "for-each-ref",
            "--format=%(taggername) %(taggeremail)",
            &format!("refs/tags/{tag_name}"),
        ])
        .current_dir(&repo_root)
        .output()
        .expect("git tag query must succeed");
    let tag_str = String::from_utf8_lossy(&tag_out.stdout).trim().to_string();

    assert_eq!(
        tag_str, expected_author,
        "Annotated tag '{tag_name}' tagger MUST strictly be '{expected_author}', but got '{tag_str}'"
    );

    // 3. Assert receipts strictly contain author identity
    assert_eq!(promo_receipt.author, expected_author);

    let _ = fs::remove_dir_all(&repo_root);
}
