#![forbid(unsafe_code)]

//! Phase 24 Milestone M2 Empirical Challenger Stress Tests.
//!
//! Rigorous empirical stress-testing for Byzantine Consensus & P2P Mesh:
//! 1. Split-brain resistance:
//!    - N=4, f=1: Two partitions of size 2 ({node-0, node-1} and {node-2, node-3})
//!      cannot commit conflicting patches.
//!    - Split-brain cross-rejections lead to explicit rejection, never split-brain commit.
//!    - Byzantine bridging: even with 1 Byzantine node, conflicting patches cannot both commit.
//! 2. Equivocation detection:
//!    - Rejection of double-voting with contradictory booleans (true -> false, false -> true).
//!    - Rejection of double-voting with divergent score vectors (Euclidean distance > 1e-4).
//!    - Acceptance and idempotence for identical or sub-threshold score re-submissions.
//!    - Engine vote state integrity after equivocation rejection.
//! 3. Byzantine outlier score filtering:
//!    - Filtering of score vectors with distance > 0.45 from coordinate-wise trimmed centroid.
//!    - Verification that extreme scores (> 0.45 distance from trimmed centroid) are filtered.
//!    - Quorum failure when filtered outliers leave fewer than f + 1 honest nodes.
//!    - Pre-filtering of invariant-violating score vectors.

use pandora_runtime::distributed_swarm::{
    DistributedConsensusDecision, DistributedConsensusVote, DistributedPatchProposal,
    DistributedSwarmConsensusEngine, DistributedSwarmError,
};
use pandora_runtime::multi_agent_collab::ExternalAgentKind;
use pandora_runtime::multi_agent_consensus::ProposalScoreVector;

// ============================================================================
// 1. Split-Brain Resistance Tests
// ============================================================================

#[test]
fn test_split_brain_resistance_n4_f1_isolated_partitions_cannot_commit() {
    // Cluster parameters: N = 4, f = 1 => Quorum Q = 2f + 1 = 3.
    // Partition 1 consists of {node-0, node-1}.
    // Partition 2 consists of {node-2, node-3}.

    // Partition 1 local consensus engine
    let mut p1_engine = DistributedSwarmConsensusEngine::new("node-0", 4, 1)
        .expect("Valid BFT parameters (N=4, f=1)");
    assert_eq!(p1_engine.quorum_threshold, 3);

    // Partition 2 local consensus engine
    let mut p2_engine = DistributedSwarmConsensusEngine::new("node-2", 4, 1)
        .expect("Valid BFT parameters (N=4, f=1)");
    assert_eq!(p2_engine.quorum_threshold, 3);

    let scores_a = ProposalScoreVector::new(0.92, 0.90, 0.88, 0.94);
    let proposal_a = DistributedPatchProposal::new(
        "patch-partition-1",
        "node-0",
        ExternalAgentKind::Agy,
        "Patch A proposed in Partition 1",
        "Task Goal A",
        "--- a/lib.rs\n+++ b/lib.rs (Variant A)",
        scores_a,
        1000,
    );
    p1_engine.submit_proposal(proposal_a).unwrap();

    let scores_b = ProposalScoreVector::new(0.90, 0.92, 0.86, 0.95);
    let proposal_b = DistributedPatchProposal::new(
        "patch-partition-2",
        "node-2",
        ExternalAgentKind::Devin,
        "Conflicting Patch B proposed in Partition 2",
        "Task Goal B (Conflicting)",
        "--- a/lib.rs\n+++ b/lib.rs (Variant B)",
        scores_b,
        1000,
    );
    p2_engine.submit_proposal(proposal_b).unwrap();

    // In Partition 1: node-0 and node-1 vote approve on Patch A
    let dec_p1_vote1 = p1_engine
        .record_vote(DistributedConsensusVote::new(
            "patch-partition-1",
            "node-0",
            true,
            scores_a,
            1010,
        ))
        .unwrap();
    assert!(dec_p1_vote1.is_pending());

    let dec_p1_vote2 = p1_engine
        .record_vote(DistributedConsensusVote::new(
            "patch-partition-1",
            "node-1",
            true,
            scores_a,
            1020,
        ))
        .unwrap();
    assert!(
        dec_p1_vote2.is_pending(),
        "Partition 1 with 2 nodes cannot reach quorum Q=3"
    );

    // Partition 1 CANNOT commit Patch A
    let err_p1 = p1_engine.certify_patch("patch-partition-1").unwrap_err();
    assert!(
        matches!(
            err_p1,
            DistributedSwarmError::ConsensusPending {
                approvals: 2,
                needed: 3,
                ..
            }
        ),
        "Patch A must remain pending in Partition 1"
    );

    // In Partition 2: node-2 and node-3 vote approve on conflicting Patch B
    let dec_p2_vote1 = p2_engine
        .record_vote(DistributedConsensusVote::new(
            "patch-partition-2",
            "node-2",
            true,
            scores_b,
            1030,
        ))
        .unwrap();
    assert!(dec_p2_vote1.is_pending());

    let dec_p2_vote2 = p2_engine
        .record_vote(DistributedConsensusVote::new(
            "patch-partition-2",
            "node-3",
            true,
            scores_b,
            1040,
        ))
        .unwrap();
    assert!(
        dec_p2_vote2.is_pending(),
        "Partition 2 with 2 nodes cannot reach quorum Q=3"
    );

    // Partition 2 CANNOT commit Patch B
    let err_p2 = p2_engine.certify_patch("patch-partition-2").unwrap_err();
    assert!(
        matches!(
            err_p2,
            DistributedSwarmError::ConsensusPending {
                approvals: 2,
                needed: 3,
                ..
            }
        ),
        "Patch B must remain pending in Partition 2"
    );

    // Conclusion: Split-brain fork is strictly impossible because neither partition
    // has the supermajority quorum threshold Q = 3.
}

#[test]
fn test_split_brain_cross_partition_rejections_prevent_commit() {
    // If Partition 1 nodes reject Patch B, and Partition 2 nodes reject Patch A
    let mut engine_a = DistributedSwarmConsensusEngine::new("coordinator", 4, 1).unwrap();
    let scores = ProposalScoreVector::new(0.90, 0.90, 0.90, 0.90);

    let proposal_a = DistributedPatchProposal::new(
        "patch-a",
        "node-0",
        ExternalAgentKind::Agy,
        "Patch A",
        "Goal A",
        "diff A",
        scores,
        2000,
    );
    engine_a.submit_proposal(proposal_a).unwrap();

    // Partition 1 approves: node-0, node-1
    engine_a
        .record_vote(DistributedConsensusVote::new(
            "patch-a", "node-0", true, scores, 2010,
        ))
        .unwrap();
    engine_a
        .record_vote(DistributedConsensusVote::new(
            "patch-a", "node-1", true, scores, 2020,
        ))
        .unwrap();

    // Partition 2 rejects: node-2, node-3
    engine_a
        .record_vote(DistributedConsensusVote::new(
            "patch-a", "node-2", false, scores, 2030,
        ))
        .unwrap();
    let final_dec = engine_a
        .record_vote(DistributedConsensusVote::new(
            "patch-a", "node-3", false, scores, 2040,
        ))
        .unwrap();

    // With 2 rejections out of 4, Q=3 is permanently impossible (4 - 3 = 1 allowable rejection)
    assert!(final_dec.is_rejected());
    match final_dec {
        DistributedConsensusDecision::Rejected {
            approvals,
            rejections,
            ..
        } => {
            assert_eq!(approvals, 2);
            assert_eq!(rejections, 2);
        }
        _ => panic!("Expected Rejected decision when rejections exceed total_nodes - Q"),
    }
}

#[test]
fn test_split_brain_byzantine_bridging_cannot_commit_conflicting_patches() {
    // Attack scenario: Node 3 is Byzantine and tries to help BOTH partitions commit
    // by voting for Patch A in Partition 1 AND Patch B in Partition 2.
    // Can BOTH Patch A and Patch B commit simultaneously?
    let mut engine_a = DistributedSwarmConsensusEngine::new("node-0", 4, 1).unwrap();
    let mut engine_b = DistributedSwarmConsensusEngine::new("node-2", 4, 1).unwrap();
    let scores = ProposalScoreVector::new(0.92, 0.90, 0.88, 0.94);

    let prop_a = DistributedPatchProposal::new(
        "patch-a",
        "node-0",
        ExternalAgentKind::Agy,
        "Patch A",
        "Goal A",
        "diff A",
        scores,
        3000,
    );
    let prop_b = DistributedPatchProposal::new(
        "patch-b",
        "node-2",
        ExternalAgentKind::Devin,
        "Patch B",
        "Goal B",
        "diff B",
        scores,
        3000,
    );

    engine_a.submit_proposal(prop_a).unwrap();
    engine_b.submit_proposal(prop_b).unwrap();

    // P1 honest nodes vote for Patch A
    engine_a
        .record_vote(DistributedConsensusVote::new(
            "patch-a", "node-0", true, scores, 3010,
        ))
        .unwrap();
    engine_a
        .record_vote(DistributedConsensusVote::new(
            "patch-a", "node-1", true, scores, 3020,
        ))
        .unwrap();

    // Byzantine node-3 votes for Patch A -> Patch A reaches 3 approvals!
    let dec_a = engine_a
        .record_vote(DistributedConsensusVote::new(
            "patch-a", "node-3", true, scores, 3030,
        ))
        .unwrap();
    assert!(dec_a.is_approved(), "Patch A reached Q=3 with node-3");

    // P2 honest node-2 votes for Patch B
    engine_b
        .record_vote(DistributedConsensusVote::new(
            "patch-b", "node-2", true, scores, 3040,
        ))
        .unwrap();

    // Byzantine node-3 also votes for Patch B
    let dec_b = engine_b
        .record_vote(DistributedConsensusVote::new(
            "patch-b", "node-3", true, scores, 3050,
        ))
        .unwrap();

    // But Patch B has only 2 approvals {node-2, node-3}!
    // It is STILL Pending and CANNOT commit!
    assert!(
        dec_b.is_pending(),
        "Patch B only has 2 approvals and cannot commit"
    );
    let err_b = engine_b.certify_patch("patch-b").unwrap_err();
    assert!(matches!(
        err_b,
        DistributedSwarmError::ConsensusPending {
            approvals: 2,
            needed: 3,
            ..
        }
    ));

    // For Patch B to commit, another node would need to vote for it.
    // But nodes 0 and 1 are honest and did not vote for Patch B.
    // Thus Theorem 47 Lemma 47.1 holds: Split-brain simultaneous commit is impossible.
}

// ============================================================================
// 2. Equivocation Detection Stress Tests
// ============================================================================

#[test]
fn test_equivocation_detection_boolean_and_score_divergence() {
    let mut engine = DistributedSwarmConsensusEngine::new("verifier", 4, 1).unwrap();
    let base_scores = ProposalScoreVector::new(0.88, 0.90, 0.85, 0.92);

    let proposal = DistributedPatchProposal::new(
        "prop-equiv-test",
        "verifier",
        ExternalAgentKind::Agy,
        "Equivocation test proposal",
        "Verify Byzantine detection",
        "diff content",
        base_scores,
        4000,
    );
    engine.submit_proposal(proposal).unwrap();

    // Initial valid vote: approve = true
    let vote1 =
        DistributedConsensusVote::new("prop-equiv-test", "attacker", true, base_scores, 4010);
    let dec1 = engine.record_vote(vote1).unwrap();
    assert!(dec1.is_pending());

    // 1. Contradictory boolean: same voter, approve = false -> EquivocationDetected
    let vote_flip =
        DistributedConsensusVote::new("prop-equiv-test", "attacker", false, base_scores, 4020);
    let err_flip = engine.record_vote(vote_flip).unwrap_err();
    assert_eq!(
        err_flip,
        DistributedSwarmError::EquivocationDetected {
            voter: "attacker".to_string(),
            proposal: "prop-equiv-test".to_string(),
        }
    );

    // 2. Divergent score vector (distance > 1e-4):
    // Shift one coordinate by 0.001 (distance = 0.001 > 0.0001)
    let divergent_scores = ProposalScoreVector::new(0.881, 0.90, 0.85, 0.92);
    assert!(base_scores.distance(&divergent_scores) > 1e-4);

    let vote_divergent =
        DistributedConsensusVote::new("prop-equiv-test", "attacker", true, divergent_scores, 4030);
    let err_divergent = engine.record_vote(vote_divergent).unwrap_err();
    assert_eq!(
        err_divergent,
        DistributedSwarmError::EquivocationDetected {
            voter: "attacker".to_string(),
            proposal: "prop-equiv-test".to_string(),
        }
    );

    // 3. Sub-threshold score variation (distance <= 1e-4) is accepted as idempotent
    let sub_threshold_scores = ProposalScoreVector::new(0.88005, 0.90, 0.85, 0.92);
    assert!(base_scores.distance(&sub_threshold_scores) <= 1e-4);

    let vote_sub_threshold = DistributedConsensusVote::new(
        "prop-equiv-test",
        "attacker",
        true,
        sub_threshold_scores,
        4040,
    );
    let dec_sub = engine.record_vote(vote_sub_threshold).unwrap();
    assert!(dec_sub.is_pending());

    // 4. Exact duplicate vote: succeeds idempotently
    let vote_exact =
        DistributedConsensusVote::new("prop-equiv-test", "attacker", true, base_scores, 4010);
    let dec_exact = engine.record_vote(vote_exact).unwrap();
    assert!(dec_exact.is_pending());

    // 5. Engine integrity: other honest nodes can still vote and reach quorum
    engine
        .record_vote(DistributedConsensusVote::new(
            "prop-equiv-test",
            "honest-1",
            true,
            base_scores,
            4050,
        ))
        .unwrap();
    let dec_quorum = engine
        .record_vote(DistributedConsensusVote::new(
            "prop-equiv-test",
            "honest-2",
            true,
            base_scores,
            4060,
        ))
        .unwrap();

    assert!(
        dec_quorum.is_approved(),
        "Quorum Q=3 reached despite previous equivocation attempt"
    );
}

#[test]
fn test_equivocation_cross_proposal_isolation() {
    // Voting differently on two distinct proposals is NOT equivocation
    let mut engine = DistributedSwarmConsensusEngine::new("verifier", 4, 1).unwrap();
    let scores1 = ProposalScoreVector::new(0.90, 0.90, 0.90, 0.90);
    let scores2 = ProposalScoreVector::new(0.50, 0.50, 0.50, 0.50);

    let prop1 = DistributedPatchProposal::new(
        "prop-1",
        "verifier",
        ExternalAgentKind::Agy,
        "Proposal 1",
        "Goal 1",
        "diff 1",
        scores1,
        5000,
    );
    let prop2 = DistributedPatchProposal::new(
        "prop-2",
        "verifier",
        ExternalAgentKind::Devin,
        "Proposal 2",
        "Goal 2",
        "diff 2",
        scores2,
        5000,
    );

    engine.submit_proposal(prop1).unwrap();
    engine.submit_proposal(prop2).unwrap();

    // Node A approves prop1
    let dec1 = engine
        .record_vote(DistributedConsensusVote::new(
            "prop-1", "node-a", true, scores1, 5010,
        ))
        .unwrap();
    assert!(dec1.is_pending());

    // Node A rejects prop2 with completely different scores -> NOT equivocation
    let dec2 = engine
        .record_vote(DistributedConsensusVote::new(
            "prop-2", "node-a", false, scores2, 5020,
        ))
        .unwrap();
    assert!(dec2.is_pending());
}

// ============================================================================
// 3. Byzantine Outlier Score Filtering Tests
// ============================================================================

#[test]
fn test_byzantine_outlier_filtering_boundary_and_rejection() {
    // Setup N=4, f=1 => Q=3.
    let mut engine = DistributedSwarmConsensusEngine::new("leader", 4, 1).unwrap();
    assert_eq!(engine.max_centroid_divergence, 0.45);

    let honest_scores = ProposalScoreVector::new(0.90, 0.90, 0.90, 0.90);
    let prop = DistributedPatchProposal::new(
        "prop-filter",
        "leader",
        ExternalAgentKind::Agy,
        "Outlier filter test",
        "Test boundary > 0.45",
        "diff",
        honest_scores,
        6000,
    );
    engine.submit_proposal(prop).unwrap();

    // 3 honest votes cast
    engine
        .record_vote(DistributedConsensusVote::new(
            "prop-filter",
            "honest-1",
            true,
            honest_scores,
            6010,
        ))
        .unwrap();
    engine
        .record_vote(DistributedConsensusVote::new(
            "prop-filter",
            "honest-2",
            true,
            honest_scores,
            6020,
        ))
        .unwrap();
    engine
        .record_vote(DistributedConsensusVote::new(
            "prop-filter",
            "honest-3",
            true,
            honest_scores,
            6030,
        ))
        .unwrap();

    // With 3 honest votes (all at 0.90), a 4th vote is tested under N=4.
    // Case A: 4th vote has distance 0.40 <= 0.45 (acceptable drift)
    // Coords: 0.90 - 0.20 = 0.70 on all 4 coords => distance = sqrt(4 * 0.04) = 0.40
    let drift_scores = ProposalScoreVector::new(0.70, 0.70, 0.70, 0.70);
    assert!((honest_scores.distance(&drift_scores) - 0.40).abs() < 1e-9);

    let mut engine_acceptable = engine.clone();
    let dec_acceptable = engine_acceptable
        .record_vote(DistributedConsensusVote::new(
            "prop-filter",
            "node-drift",
            true,
            drift_scores,
            6040,
        ))
        .unwrap();

    assert!(
        dec_acceptable.is_approved(),
        "Vote with distance 0.40 <= 0.45 should be accepted into consensus"
    );

    // Case B: 4th vote has distance 0.80 > 0.45 (outlier filtered)
    // Coords: 0.90 - 0.40 = 0.50 on all 4 coords => distance = sqrt(4 * 0.16) = 0.80
    let outlier_scores = ProposalScoreVector::new(0.50, 0.50, 0.50, 0.50);
    assert!(honest_scores.distance(&outlier_scores) > 0.45);

    let mut engine_outlier = engine.clone();
    let dec_outlier = engine_outlier
        .record_vote(DistributedConsensusVote::new(
            "prop-filter",
            "byzantine-node",
            true,
            outlier_scores,
            6040,
        ))
        .unwrap();

    // With 3 honest + 1 outlier, honest count = 3 >= Q - f (3 - 1 = 2).
    // The outlier is filtered as Byzantine, leaving honest votes to form consensus!
    assert!(dec_outlier.is_approved());
    match dec_outlier {
        DistributedConsensusDecision::Approved(cert) => {
            // Trimmed centroid trims the outlier because m = 4, trim_k = 1.
            // Therefore, the certified scores match the honest cluster.
            let drift = cert.scores.distance(&honest_scores);
            assert!(
                drift < 0.01,
                "Outlier must be filtered, preserving true honest centroid (drift: {drift})"
            );
        }
        _ => panic!("Expected approved with filtered outlier"),
    }
}

#[test]
fn test_byzantine_collusion_filtered_outliers_fail_quorum() {
    // Attack scenario: 1 honest node and 2 Byzantine nodes approve.
    // Approvals = 3 (looks like Q=3 is reached!).
    // But the 2 Byzantine nodes submit divergent outlier scores on opposite sides of the space,
    // so both have distance > 0.45 from the centroid.
    // Since only 1 honest vote remains, honest_votes.len() < Q - f (1 < 2).
    // The engine MUST REJECT the proposal due to excessive Byzantine score divergence!
    let mut engine = DistributedSwarmConsensusEngine::new("leader", 4, 1).unwrap();

    // Honest node: centered at 0.65
    let honest_scores = ProposalScoreVector::new(0.65, 0.65, 0.65, 0.65);

    let prop = DistributedPatchProposal::new(
        "prop-collusion",
        "leader",
        ExternalAgentKind::Agy,
        "Collusion attack test",
        "Test rejection when outliers exceed Byzantine tolerance",
        "diff",
        honest_scores,
        7000,
    );
    engine.submit_proposal(prop).unwrap();

    // 1 honest approval
    engine
        .record_vote(DistributedConsensusVote::new(
            "prop-collusion",
            "honest-node",
            true,
            honest_scores,
            7010,
        ))
        .unwrap();

    // Byzantine 1: high scores (0.95, 0.95, 0.95, 0.95)
    // Distance from 0.65 is sqrt(4 * 0.30^2) = 0.60 > 0.45
    let byz_scores_1 = ProposalScoreVector::new(0.95, 0.95, 0.95, 0.95);

    // Byzantine 2: low scores but meeting viability (composite >= 0.50, inv >= 0.40)
    // Coords: (0.38, 0.38, 0.38, 0.92) => composite = 0.40(0.38) + 0.25(0.92) + 0.20(0.38) + 0.15(0.38) = 0.152 + 0.23 + 0.076 + 0.057 = 0.515 >= 0.50
    // Invariant = 0.92 >= 0.40
    let byz_scores_2 = ProposalScoreVector::new(0.38, 0.38, 0.38, 0.92);

    // Trimmed centroid across the 3 votes (k = 1 for m = 3, f = 1):
    // d0: sorted [0.38, 0.65, 0.95] => trimmed median = 0.65
    // d1: sorted [0.38, 0.65, 0.95] => trimmed median = 0.65
    // d2: sorted [0.38, 0.65, 0.95] => trimmed median = 0.65
    // d3: sorted [0.65, 0.92, 0.95] => trimmed median = 0.92
    // Centroid: (0.65, 0.65, 0.65, 0.92)
    // Distance from centroid to honest (0.65, 0.65, 0.65, 0.65):
    // diff = (0, 0, 0, -0.27) => dist = 0.27 <= 0.45 (HONEST PASSES!)
    // Distance from centroid to byz_1 (0.95, 0.95, 0.95, 0.95):
    // diff = (0.30, 0.30, 0.30, 0.03) => dist = sqrt(3*0.09 + 0.0009) = 0.520 > 0.45 (BYZ 1 FILTERED!)
    // Distance from centroid to byz_2 (0.38, 0.38, 0.38, 0.92):
    // diff = (-0.27, -0.27, -0.27, 0.0) => dist = sqrt(3*0.0729) = 0.468 > 0.45 (BYZ 2 FILTERED!)
    // Result: BOTH Byzantine votes are filtered (> 0.45 from trimmed centroid), honest vote passes!

    engine
        .record_vote(DistributedConsensusVote::new(
            "prop-collusion",
            "byz-node-1",
            true,
            byz_scores_1,
            7020,
        ))
        .unwrap();

    let dec = engine
        .record_vote(DistributedConsensusVote::new(
            "prop-collusion",
            "byz-node-2",
            true,
            byz_scores_2,
            7030,
        ))
        .unwrap();

    // The proposal MUST be rejected because 2 votes were filtered, leaving only 1 honest vote < Q - f (1 < 2)
    assert!(
        dec.is_rejected(),
        "Proposal must be rejected when honest votes < Q - f"
    );
    match dec {
        DistributedConsensusDecision::Rejected {
            reason, approvals, ..
        } => {
            assert_eq!(approvals, 3);
            assert!(
                reason.contains("excessive Byzantine score divergence"),
                "Reason was: {reason}"
            );
        }
        _ => panic!("Expected Rejected decision"),
    }
}

#[test]
fn test_invariant_compliance_pre_filtering() {
    // A vote with invariant_compliance < 0.40 or composite score < 0.50 is pre-filtered.
    let mut engine = DistributedSwarmConsensusEngine::new("leader", 4, 1).unwrap();
    let honest_scores = ProposalScoreVector::new(0.90, 0.90, 0.90, 0.90);

    let prop = DistributedPatchProposal::new(
        "prop-prefilter",
        "leader",
        ExternalAgentKind::Agy,
        "Invariant pre-filtering test",
        "Test invariant compliance < 0.40 rejection",
        "diff",
        honest_scores,
        8000,
    );
    engine.submit_proposal(prop).unwrap();

    // 1 honest vote
    engine
        .record_vote(DistributedConsensusVote::new(
            "prop-prefilter",
            "honest-node",
            true,
            honest_scores,
            8010,
        ))
        .unwrap();

    // 2 votes with failing invariant_compliance (0.20 < 0.40)
    let failing_scores = ProposalScoreVector::new(0.90, 0.90, 0.90, 0.20);
    assert!(failing_scores.invariant_compliance < 0.40);

    engine
        .record_vote(DistributedConsensusVote::new(
            "prop-prefilter",
            "failing-node-1",
            true,
            failing_scores,
            8020,
        ))
        .unwrap();

    let dec = engine
        .record_vote(DistributedConsensusVote::new(
            "prop-prefilter",
            "failing-node-2",
            true,
            failing_scores,
            8030,
        ))
        .unwrap();

    // Only 1 viable vote < Q - f (1 < 2)
    assert!(dec.is_rejected());
    match dec {
        DistributedConsensusDecision::Rejected { reason, .. } => {
            assert!(
                reason.contains("insufficient viable non-divergent votes"),
                "Reason was: {reason}"
            );
        }
        _ => panic!("Expected Rejected decision"),
    }
}
