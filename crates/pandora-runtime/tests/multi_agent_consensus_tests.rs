#![forbid(unsafe_code)]

use pandora_runtime::multi_agent_collab::ExternalAgentKind;
use pandora_runtime::multi_agent_consensus::{
    AgentPatchProposal, ProposalScoreVector, ProposalVerdict, SwarmCodeConsensusEngine,
};

#[test]
fn test_score_vector_composite_and_distance() {
    let sv1 = ProposalScoreVector::new(1.0, 1.0, 1.0, 1.0);
    assert!((sv1.composite_score() - 1.0).abs() < 1e-6);

    let sv2 = ProposalScoreVector::new(0.0, 0.0, 0.0, 0.0);
    assert!((sv2.composite_score() - 0.0).abs() < 1e-6);

    // Euclidean distance in 4D: sqrt(4) = 2.0
    let dist = sv1.distance(&sv2);
    assert!((dist - 2.0).abs() < 1e-6);

    // Clamping checks
    let sv3 = ProposalScoreVector::new(-1.0, 2.0, 0.5, 0.5);
    assert_eq!(sv3.test_pass_rate, 0.0);
    assert_eq!(sv3.confidence, 1.0);
}

#[test]
fn test_swarm_consensus_healthy_quorum() {
    let engine = SwarmCodeConsensusEngine::new(0.60, 0.35);

    let proposals = vec![
        AgentPatchProposal::new(
            "prop-1",
            ExternalAgentKind::Agy,
            "Refactor auth logic",
            "Removes redundant check and improves speed",
            "diff --git a/auth.rs\n+ new_check();",
            ProposalScoreVector::new(0.95, 0.92, 0.88, 0.99),
            120,
        ),
        AgentPatchProposal::new(
            "prop-2",
            ExternalAgentKind::ClaudeCode,
            "Auth refinement",
            "Identical semantic refactoring",
            "diff --git a/auth.rs\n+ new_check_optimized();",
            ProposalScoreVector::new(0.94, 0.90, 0.86, 0.98),
            115,
        ),
        AgentPatchProposal::new(
            "prop-3",
            ExternalAgentKind::CodexCli,
            "Auth module speedup",
            "Cache token validation",
            "diff --git a/auth.rs\n+ cached_check();",
            ProposalScoreVector::new(0.92, 0.89, 0.90, 0.96),
            110,
        ),
        AgentPatchProposal::new(
            "prop-4",
            ExternalAgentKind::CursorAgent,
            "Clean auth calls",
            "Direct lookup",
            "diff --git a/auth.rs\n+ direct();",
            ProposalScoreVector::new(0.93, 0.91, 0.85, 0.97),
            105,
        ),
    ];

    let decision = engine.resolve_consensus(&proposals, 1700000000000).unwrap();

    assert_eq!(decision.total_proposals, 4);
    assert_eq!(decision.honest_quorum_size, 4);
    assert_eq!(decision.byzantine_filtered_count, 0);
    assert!(decision.consensus_confidence >= 0.80);
    assert!(!decision.consensus_hash.is_empty());
    assert_eq!(decision.winning_proposal_id, "prop-1");

    for verdict in decision.proposal_verdicts.values() {
        assert_eq!(*verdict, ProposalVerdict::ConsensusQuorum);
    }
}

#[test]
fn test_swarm_consensus_byzantine_filtering() {
    let engine = SwarmCodeConsensusEngine::new(0.60, 0.35);

    let proposals = vec![
        AgentPatchProposal::new(
            "honest-1",
            ExternalAgentKind::Agy,
            "Clean HNSW indexing",
            "Valid indexing optimization",
            "diff --git a/hnsw.rs\n+ optimize();",
            ProposalScoreVector::new(0.95, 0.94, 0.90, 1.0),
            100,
        ),
        AgentPatchProposal::new(
            "honest-2",
            ExternalAgentKind::ClaudeCode,
            "Clean HNSW indexing",
            "Valid indexing optimization",
            "diff --git a/hnsw.rs\n+ optimize_fast();",
            ProposalScoreVector::new(0.93, 0.91, 0.88, 1.0),
            95,
        ),
        AgentPatchProposal::new(
            "honest-3",
            ExternalAgentKind::GeminiCli,
            "Clean HNSW indexing",
            "Valid indexing optimization",
            "diff --git a/hnsw.rs\n+ optimize_v2();",
            ProposalScoreVector::new(0.92, 0.90, 0.89, 0.98),
            90,
        ),
        // Byzantine proposal: hallucinatory scores and zero invariant compliance
        AgentPatchProposal::new(
            "byz-adversarial",
            ExternalAgentKind::Hermes,
            "Bypass invariants completely",
            "Delete checks",
            "diff --git a/hnsw.rs\n- check();",
            ProposalScoreVector::new(0.10, 0.20, 0.30, 0.0),
            500,
        ),
    ];

    let decision = engine.resolve_consensus(&proposals, 1700000000000).unwrap();

    assert_eq!(decision.total_proposals, 4);
    assert_eq!(decision.honest_quorum_size, 3);
    assert_eq!(decision.byzantine_filtered_count, 1);

    let byz_verdict = decision
        .proposal_verdicts
        .get("byz-adversarial")
        .expect("byzantine verdict present");
    assert!(
        *byz_verdict == ProposalVerdict::FilteredByzantine
            || *byz_verdict == ProposalVerdict::RejectedBelowThreshold
    );

    // Winning proposal must be honest
    assert!(decision.winning_proposal_id.starts_with("honest-"));
}

#[test]
fn test_consensus_hash_deterministic() {
    let engine = SwarmCodeConsensusEngine::new(0.50, 0.30);

    let proposals = vec![AgentPatchProposal::new(
        "prop-det",
        ExternalAgentKind::Devin,
        "Fix deterministic behavior",
        "Deterministic test patch",
        "diff --git a/det.rs\n+ det();",
        ProposalScoreVector::new(0.9, 0.9, 0.9, 0.9),
        50,
    )];

    let d1 = engine.resolve_consensus(&proposals, 12345).unwrap();
    let d2 = engine.resolve_consensus(&proposals, 12345).unwrap();

    assert_eq!(d1.consensus_hash, d2.consensus_hash);
    assert_eq!(d1.winning_proposal_id, d2.winning_proposal_id);
}
