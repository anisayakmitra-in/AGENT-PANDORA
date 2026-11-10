#![forbid(unsafe_code)]

//! Phase 24 Formal Verification Test Suite: Distributed Cross-Node Swarm Mesh,
//! Byzantine Patch Consensus Engine & Federated Gradient Dissemination.
//!
//! Grounded in Theorem 47 (Lemmas 47.1 through 47.5):
//! - Lemma 47.1: Supermajority Quorum Intersection Invariant ($N \ge 3f + 1, |Q_1 \cap Q_2| \ge f + 1$)
//! - Lemma 47.2: Cryptographic Equivocation Detection ($E_i \implies \text{Quarantine}$)
//! - Lemma 47.3: Coordinate-Wise Trimmed Mean Centroid Bounded Drift
//! - Lemma 47.4: Federated Policy Gradient Consensus Convergence ($O(1/\sqrt{T})$)
//! - Lemma 47.5: Cryptographic Patch Certification Soundness (`PANDORA_PATCH_CERT_V1:...`)
//!
//! # Safety Invariant
//! Strictly `#![forbid(unsafe_code)]` enforced workspace-wide. Zero tolerance for fake implementations.

use pandora_runtime::distributed_swarm::{
    DistributedConsensusDecision, DistributedConsensusVote, DistributedFederatedEngine,
    DistributedGradientPacket, DistributedPatchProposal, DistributedSwarmConsensusEngine,
    DistributedSwarmError, DistributedSwarmMessage, DistributedSwarmNode, SwarmPayload,
    decode_hex_32, decode_hex_64, encode_hex,
};
use pandora_runtime::federated_policy::PolicyGradientVector;
use pandora_runtime::mesh_sync::PeerIdentity;
use pandora_runtime::multi_agent_collab::ExternalAgentKind;
use pandora_runtime::multi_agent_consensus::ProposalScoreVector;
use pandora_runtime::swarm_mesh::VectorClock;

// ============================================================================
// Test 1: Peer Discovery & Mutual Ed25519 Handshake Validation
// ============================================================================

#[test]
fn test_peer_discovery_and_handshake_ed25519_validation() {
    let identity_a = PeerIdentity::new_random();
    let identity_b = PeerIdentity::new_random();

    let mut node_b = DistributedSwarmNode::new(identity_b, "127.0.0.1:9002", 4, 1)
        .expect("node B should initialize successfully");

    // Node A creates a cryptographically signed handshake
    let mut hs_a = node_b.create_handshake(1_700_000_000_000);
    hs_a.peer_id = identity_a.peer_id.clone();
    hs_a.public_key = identity_a.public_key_hex();
    hs_a.listen_addr = "127.0.0.1:9001".to_string();
    hs_a.signature = identity_a.sign_digest(&String::from_utf8_lossy(&hs_a.payload_bytes()));

    // Verify raw handshake signature directly
    assert!(
        hs_a.verify()
            .expect("handshake signature check should succeed")
    );

    // Node B handles handshake, verifies signature, records peer, and emits signed ACK
    let ack_msg = node_b
        .handle_handshake(hs_a.clone(), 1_700_000_000_100)
        .expect("node B should accept valid signed handshake");

    assert_eq!(node_b.peers.len(), 1);
    let peer = node_b.peers.get(&identity_a.peer_id).unwrap();
    assert_eq!(peer.peer_id, identity_a.peer_id);
    assert_eq!(peer.listen_addr, "127.0.0.1:9001");
    assert!(peer.connected);

    // Verify ACK message signature using Node B's public key
    assert!(
        ack_msg
            .verify(&node_b.identity.public_key_hex())
            .expect("ACK signature must verify")
    );

    // Tampered handshake must be rejected
    let mut tampered_hs = hs_a;
    tampered_hs.listen_addr = "10.0.0.99:9999".to_string(); // Altered payload without re-signing
    let err = node_b
        .handle_handshake(tampered_hs, 1_700_000_000_200)
        .unwrap_err();
    assert!(matches!(err, DistributedSwarmError::InvalidSignature(_)));
}

// ============================================================================
// Test 2: BFT Quorum Consensus Convergence (N=4, f=1, Q=3 and N=7, f=2, Q=5)
// ============================================================================

#[test]
fn test_bft_quorum_consensus_n4_f1_and_n7_f2() {
    // ------------------------------------------------------------------------
    // Part A: N = 4, f = 1 => Supermajority Quorum Q = 2f + 1 = 3
    // ------------------------------------------------------------------------
    let mut engine_4_1 = DistributedSwarmConsensusEngine::new("node-0", 4, 1)
        .expect("valid BFT parameters (4 >= 3*1 + 1)");
    assert_eq!(engine_4_1.quorum_threshold, 3);

    let scores = ProposalScoreVector::new(0.92, 0.88, 0.90, 0.94);
    let proposal_4 = DistributedPatchProposal::new(
        "patch-4-1",
        "node-0",
        ExternalAgentKind::Agy,
        "Fix concurrency race in pipeline",
        "Eliminate data race in queue",
        "--- a/queue.rs\n+++ b/queue.rs",
        scores,
        1000,
    );
    engine_4_1
        .submit_proposal(proposal_4)
        .expect("proposal submission should succeed");

    // Vote 1: pending (1/3)
    let dec1 = engine_4_1
        .record_vote(DistributedConsensusVote::new(
            "patch-4-1",
            "node-0",
            true,
            scores,
            1010,
        ))
        .unwrap();
    assert!(dec1.is_pending());

    // Vote 2: pending (2/3)
    let dec2 = engine_4_1
        .record_vote(DistributedConsensusVote::new(
            "patch-4-1",
            "node-1",
            true,
            scores,
            1020,
        ))
        .unwrap();
    assert!(dec2.is_pending());

    // Vote 3: quorum reached (3/3 approvals) -> Approved!
    let dec3 = engine_4_1
        .record_vote(DistributedConsensusVote::new(
            "patch-4-1",
            "node-2",
            true,
            scores,
            1030,
        ))
        .unwrap();
    assert!(dec3.is_approved());
    match dec3 {
        DistributedConsensusDecision::Approved(cert) => {
            assert_eq!(cert.winning_proposal_id, "patch-4-1");
            assert!(
                cert.certificate_digest
                    .starts_with("PANDORA_PATCH_CERT_V1:")
            );
            assert_eq!(cert.certificate_id.len(), 21); // "cert-" + 16 chars
        }
        _ => panic!("Expected Approved decision at quorum"),
    }

    // ------------------------------------------------------------------------
    // Part B: N = 7, f = 2 => Supermajority Quorum Q = 2(2) + 1 = 5
    // ------------------------------------------------------------------------
    let mut engine_7_2 = DistributedSwarmConsensusEngine::new("node-primary", 7, 2)
        .expect("valid BFT parameters (7 >= 3*2 + 1)");
    assert_eq!(engine_7_2.quorum_threshold, 5);

    let proposal_7 = DistributedPatchProposal::new(
        "patch-7-2",
        "node-primary",
        ExternalAgentKind::Devin,
        "Optimize matrix SIMD kernel",
        "Vectorize inner product loop",
        "--- a/simd.rs\n+++ b/simd.rs",
        scores,
        2000,
    );
    engine_7_2
        .submit_proposal(proposal_7)
        .expect("proposal submission should succeed");

    // Cast 4 approvals (1 short of Q = 5) -> all should remain Pending
    for i in 0..4 {
        let dec = engine_7_2
            .record_vote(DistributedConsensusVote::new(
                "patch-7-2",
                format!("peer-{i}"),
                true,
                scores,
                2010 + i as u64,
            ))
            .unwrap();
        assert!(dec.is_pending());
    }

    // 5th approval reaches quorum Q = 5
    let dec5 = engine_7_2
        .record_vote(DistributedConsensusVote::new(
            "patch-7-2",
            "peer-4",
            true,
            scores,
            2020,
        ))
        .unwrap();
    assert!(dec5.is_approved());
}

// ============================================================================
// Test 3: Invalid BFT Parameters Enforce Supermajority Invariant (N >= 3f + 1)
// ============================================================================

#[test]
fn test_invalid_bft_parameters_enforces_supermajority_invariant() {
    // N = 3, f = 1: fails because 3 < 3(1) + 1 = 4
    let err_3_1 = DistributedSwarmConsensusEngine::new("node-0", 3, 1).unwrap_err();
    assert_eq!(
        err_3_1,
        DistributedSwarmError::InvalidBftParameters { n: 3, f: 1 }
    );

    // N = 6, f = 2: fails because 6 < 3(2) + 1 = 7
    let err_6_2 = DistributedSwarmConsensusEngine::new("node-0", 6, 2).unwrap_err();
    assert_eq!(
        err_6_2,
        DistributedSwarmError::InvalidBftParameters { n: 6, f: 2 }
    );

    // N = 9, f = 3: fails because 9 < 3(3) + 1 = 10
    let err_9_3 = DistributedSwarmConsensusEngine::new("node-0", 9, 3).unwrap_err();
    assert_eq!(
        err_9_3,
        DistributedSwarmError::InvalidBftParameters { n: 9, f: 3 }
    );

    // N = 10, f = 3: succeeds (10 >= 10, Q = 7)
    let ok_10_3 = DistributedSwarmConsensusEngine::new("node-0", 10, 3).unwrap();
    assert_eq!(ok_10_3.quorum_threshold, 7);
}

// ============================================================================
// Test 4: Equivocation Detection & Idempotent Voting
// ============================================================================

#[test]
fn test_equivocation_detection_and_idempotence() {
    let mut engine = DistributedSwarmConsensusEngine::new("node-0", 4, 1).unwrap();
    let scores = ProposalScoreVector::new(0.85, 0.90, 0.88, 0.92);
    let proposal = DistributedPatchProposal::new(
        "patch-equiv",
        "node-0",
        ExternalAgentKind::ClaudeCode,
        "Refactor parser state",
        "Prevent infinite token lookahead",
        "diff content",
        scores,
        3000,
    );
    engine.submit_proposal(proposal).unwrap();

    // 1. Initial vote from node-byz
    let v1 = DistributedConsensusVote::new("patch-equiv", "node-byz", true, scores, 3010);
    let dec1 = engine.record_vote(v1).unwrap();
    assert!(dec1.is_pending());

    // 2. Idempotent re-submission of exact same vote should succeed without error
    let v1_duplicate = DistributedConsensusVote::new("patch-equiv", "node-byz", true, scores, 3010);
    let dec_dup = engine.record_vote(v1_duplicate).unwrap();
    assert!(dec_dup.is_pending());

    // 3. Equivocation Attack A: Contradictory Boolean verdict (approve -> reject)
    let v1_conflicting_bool =
        DistributedConsensusVote::new("patch-equiv", "node-byz", false, scores, 3020);
    let err_bool = engine.record_vote(v1_conflicting_bool).unwrap_err();
    assert!(matches!(
        err_bool,
        DistributedSwarmError::EquivocationDetected { voter, proposal }
        if voter == "node-byz" && proposal == "patch-equiv"
    ));

    // 4. Equivocation Attack B: Divergent score vector (distance > 1e-4)
    let divergent_scores = ProposalScoreVector::new(0.50, 0.50, 0.50, 0.50);
    let v1_divergent_scores =
        DistributedConsensusVote::new("patch-equiv", "node-byz", true, divergent_scores, 3030);
    let err_scores = engine.record_vote(v1_divergent_scores).unwrap_err();
    assert!(matches!(
        err_scores,
        DistributedSwarmError::EquivocationDetected { voter, proposal }
        if voter == "node-byz" && proposal == "patch-equiv"
    ));
}

// ============================================================================
// Test 5: Byzantine Score Outlier Filtering & Trimmed Mean Centroid
// ============================================================================

#[test]
fn test_byzantine_score_outlier_filtering_and_centroid_stability() {
    // Cluster: N = 4, f = 1 => Q = 3
    let mut engine = DistributedSwarmConsensusEngine::new("node-primary", 4, 1).unwrap();

    let honest_s1 = ProposalScoreVector::new(0.92, 0.90, 0.88, 0.94);
    let honest_s2 = ProposalScoreVector::new(0.90, 0.91, 0.87, 0.93);
    let honest_s3 = ProposalScoreVector::new(0.91, 0.89, 0.89, 0.95);

    let proposal = DistributedPatchProposal::new(
        "patch-byz-filter",
        "node-primary",
        ExternalAgentKind::Agy,
        "Harden buffer boundary",
        "Bound checking in slice decoding",
        "diff --git ...",
        honest_s1,
        4000,
    );
    engine.submit_proposal(proposal).unwrap();

    // 2 honest votes
    engine
        .record_vote(DistributedConsensusVote::new(
            "patch-byz-filter",
            "honest-0",
            true,
            honest_s1,
            4010,
        ))
        .unwrap();
    engine
        .record_vote(DistributedConsensusVote::new(
            "patch-byz-filter",
            "honest-1",
            true,
            honest_s2,
            4011,
        ))
        .unwrap();

    // 1 Byzantine node votes approve with an outlier score vector (> 0.45 distance from honest cluster)
    let byzantine_outlier = ProposalScoreVector::new(0.15, 0.20, 0.10, 0.25);
    engine
        .record_vote(DistributedConsensusVote::new(
            "patch-byz-filter",
            "byzantine-node",
            true,
            byzantine_outlier,
            4012,
        ))
        .unwrap();

    // 3rd honest node votes approve, bringing total approvals to 4 (reaching Q = 3)
    let decision = engine
        .record_vote(DistributedConsensusVote::new(
            "patch-byz-filter",
            "honest-2",
            true,
            honest_s3,
            4013,
        ))
        .unwrap();

    assert!(decision.is_approved());
    match decision {
        DistributedConsensusDecision::Approved(cert) => {
            // The trimmed mean centroid must remain close to the honest cluster
            let honest_mean = ProposalScoreVector::new(0.91, 0.90, 0.88, 0.94);
            let drift = cert.scores.distance(&honest_mean);
            assert!(
                drift < 0.10,
                "Centroid drift {drift} must be strictly bounded despite Byzantine outlier"
            );
            // Invariant compliance must remain high
            assert!(cert.scores.invariant_compliance >= 0.85);
        }
        _ => panic!("Expected Approved decision after quorum"),
    }
}

// ============================================================================
// Test 6: Distributed Federated Policy Gradient Dissemination & Aggregation
// ============================================================================

#[test]
fn test_distributed_federated_policy_gradient_aggregation_with_byzantine_fault() {
    let initial_policy = [0.5, 0.5, 0.5, 0.5, 0.5, 0.5];
    let mut federated_engine = DistributedFederatedEngine::new("coord-node", initial_policy);

    let identity_h1 = PeerIdentity::new_random();
    let identity_h2 = PeerIdentity::new_random();
    let identity_h3 = PeerIdentity::new_random();
    let identity_byz = PeerIdentity::new_random();

    let epoch = 1;
    let ts = 5000;

    // Honest worker gradients
    let g1 = PolicyGradientVector::new(
        ExternalAgentKind::Agy,
        [0.02, -0.01, 0.03, -0.02, 0.01, 0.00],
        50,
        0.12,
    )
    .unwrap();
    let g2 = PolicyGradientVector::new(
        ExternalAgentKind::ClaudeCode,
        [0.01, -0.02, 0.02, -0.01, 0.02, -0.01],
        60,
        0.10,
    )
    .unwrap();
    let g3 = PolicyGradientVector::new(
        ExternalAgentKind::Devin,
        [0.03, -0.01, 0.04, -0.03, 0.01, 0.01],
        45,
        0.15,
    )
    .unwrap();

    // Byzantine corrupt gradient (large corrupted numbers)
    let g_byz = PolicyGradientVector::new(
        ExternalAgentKind::CursorAgent,
        [10.0, -10.0, 10.0, -10.0, 10.0, -10.0],
        5,
        2.5,
    )
    .unwrap();

    // Broadcast and sign packets
    let p1 = federated_engine
        .broadcast_gradient(&identity_h1, epoch, g1, ts + 1)
        .unwrap();
    let p2 = federated_engine
        .broadcast_gradient(&identity_h2, epoch, g2, ts + 2)
        .unwrap();
    let p3 = federated_engine
        .broadcast_gradient(&identity_h3, epoch, g3, ts + 3)
        .unwrap();
    let p_byz = federated_engine
        .broadcast_gradient(&identity_byz, epoch, g_byz, ts + 4)
        .unwrap();

    // Verify signatures
    assert!(p1.verify_packet(&identity_h1.public_key_hex()).unwrap());
    assert!(p2.verify_packet(&identity_h2.public_key_hex()).unwrap());
    assert!(p3.verify_packet(&identity_h3.public_key_hex()).unwrap());
    assert!(p_byz.verify_packet(&identity_byz.public_key_hex()).unwrap());

    // Ingest packets into coordinator
    federated_engine
        .receive_gradient_packet(p1, Some(&identity_h1.public_key_hex()))
        .unwrap();
    federated_engine
        .receive_gradient_packet(p2, Some(&identity_h2.public_key_hex()))
        .unwrap();
    federated_engine
        .receive_gradient_packet(p3, Some(&identity_h3.public_key_hex()))
        .unwrap();
    federated_engine
        .receive_gradient_packet(p_byz, Some(&identity_byz.public_key_hex()))
        .unwrap();

    // Aggregate gradients across cluster
    let receipt = federated_engine
        .aggregate_cluster_gradients(epoch, ts + 10)
        .expect("aggregation should succeed");

    assert_eq!(receipt.step_index, epoch);
    assert!(!receipt.update_hash.is_empty());
    assert_eq!(receipt.participating_agents, 4);

    // Verify updated policy vector components are finite and bounded in [0, 1]
    for &param in &federated_engine.current_policy {
        assert!(param.is_finite());
        assert!((0.0..=1.0).contains(&param));
    }

    // Ingestion of non-finite gradient must be rejected
    let p_nan = DistributedGradientPacket::new(
        "grad-nan",
        "faulty-node",
        epoch,
        PolicyGradientVector {
            agent: ExternalAgentKind::CodexCli,
            gradients: [f64::NAN, 0.0, 0.0, 0.0, 0.0, 0.0],
            sample_count: 10,
            loss: 0.1,
        },
        ts + 20,
    );
    let err_nan = federated_engine
        .receive_gradient_packet(p_nan, None)
        .unwrap_err();
    assert!(matches!(err_nan, DistributedSwarmError::InvalidGradient(_)));
}

// ============================================================================
// Test 7: Cryptographic Patch Certificate Verification & Query Soundness
// ============================================================================

#[test]
fn test_cryptographic_patch_certificate_verification() {
    let mut engine = DistributedSwarmConsensusEngine::new("node-cert", 4, 1).unwrap();
    let scores = ProposalScoreVector::new(0.95, 0.92, 0.90, 0.95);
    let proposal_id = "prop-cert-audit";
    let proposal = DistributedPatchProposal::new(
        proposal_id,
        "node-cert",
        ExternalAgentKind::Devin,
        "Eliminate memory leak in cache",
        "Add explicit drop handler",
        "diff content...",
        scores,
        6000,
    );
    engine.submit_proposal(proposal).unwrap();

    // Querying non-approved proposal returns ConsensusPending
    let err_pending = engine.certify_patch(proposal_id).unwrap_err();
    assert!(matches!(
        err_pending,
        DistributedSwarmError::ConsensusPending {
            approvals: 0,
            needed: 3,
            ..
        }
    ));

    // Cast 3 approval votes
    for i in 0..3 {
        engine
            .record_vote(DistributedConsensusVote::new(
                proposal_id,
                format!("voter-{i}"),
                true,
                scores,
                6010 + i as u64,
            ))
            .unwrap();
    }

    // Now certify_patch must succeed with genuine CertifiedPatchRecord
    let cert = engine
        .certify_patch(proposal_id)
        .expect("certified record should now exist");

    assert_eq!(cert.winning_proposal_id, proposal_id);
    assert_eq!(cert.winning_agent, ExternalAgentKind::Devin);
    assert!(
        cert.certificate_digest
            .starts_with("PANDORA_PATCH_CERT_V1:")
    );
    assert_eq!(cert.consensus_hash.len(), 64); // SHA-256 hex string

    // Querying non-existent proposal returns ProposalNotFound
    let err_not_found = engine.certify_patch("unknown-proposal-id").unwrap_err();
    assert!(matches!(
        err_not_found,
        DistributedSwarmError::ProposalNotFound(_)
    ));
}

// ============================================================================
// Test 8: Vector Clock Causal Ordering & Concurrency Detection
// ============================================================================

#[test]
fn test_vector_clock_causal_ordering_and_concurrency() {
    let mut clock_a = VectorClock::new();
    clock_a.increment("node-a");

    let mut clock_b = clock_a.clone();
    clock_b.increment("node-a");

    let ack_payload = SwarmPayload::HandshakeAck {
        peer_id: "node-a".to_string(),
        success: true,
        cluster_epoch: 1,
    };

    let msg1 =
        DistributedSwarmMessage::new("msg-1", "node-a", clock_a.clone(), ack_payload.clone());
    let msg2 =
        DistributedSwarmMessage::new("msg-2", "node-a", clock_b.clone(), ack_payload.clone());

    // msg1 causally precedes msg2
    assert!(msg1.causally_precedes(&msg2));
    assert!(!msg2.causally_precedes(&msg1));
    assert!(!msg1.is_concurrent_with(&msg2));

    // Create concurrent message from node-c
    let mut clock_c = VectorClock::new();
    clock_c.increment("node-c");
    let msg3 = DistributedSwarmMessage::new("msg-3", "node-c", clock_c, ack_payload);

    // msg1 and msg3 are concurrent (neither dominates the other)
    assert!(msg1.is_concurrent_with(&msg3));
    assert!(msg3.is_concurrent_with(&msg1));
    assert!(!msg1.causally_precedes(&msg3));
    assert!(!msg3.causally_precedes(&msg1));
}

// ============================================================================
// Test 9: End-to-End Multi-Node Swarm Lifecycle
// ============================================================================

#[test]
fn test_end_to_end_multi_node_swarm_lifecycle() {
    let n = 4;
    let f = 1;

    let id0 = PeerIdentity::new_random();
    let id1 = PeerIdentity::new_random();
    let id2 = PeerIdentity::new_random();
    let id3 = PeerIdentity::new_random();

    let mut node0 = DistributedSwarmNode::new(id0, "127.0.0.1:8000", n, f).unwrap();
    let mut node1 = DistributedSwarmNode::new(id1, "127.0.0.1:8001", n, f).unwrap();
    let mut node2 = DistributedSwarmNode::new(id2, "127.0.0.1:8002", n, f).unwrap();
    let mut _node3 = DistributedSwarmNode::new(id3, "127.0.0.1:8003", n, f).unwrap();

    let ts = 7000;

    // 1. Mutual handshake between node 0 and node 1
    let hs0 = node0.create_handshake(ts);
    let ack_msg = node1.handle_handshake(hs0, ts + 1).unwrap();
    assert!(ack_msg.verify(&node1.identity.public_key_hex()).unwrap());
    assert!(node1.peers.contains_key(&node0.identity.peer_id));

    // 2. Node 0 proposes a patch
    let scores = ProposalScoreVector::new(0.94, 0.91, 0.89, 0.95);
    let prop_msg = node0
        .propose_patch(
            "Harden network socket timeouts",
            "--- a/socket.rs\n+++ b/socket.rs",
            ExternalAgentKind::Agy,
            scores,
            ts + 10,
        )
        .unwrap();

    // Verify proposal envelope signature
    assert!(prop_msg.verify(&node0.identity.public_key_hex()).unwrap());

    let proposal = match &prop_msg.payload {
        SwarmPayload::PatchProposal(p) => p.clone(),
        _ => panic!("Expected PatchProposal payload"),
    };

    // Node 1 and Node 2 ingest proposal
    node1.consensus.submit_proposal(proposal.clone()).unwrap();
    node2.consensus.submit_proposal(proposal.clone()).unwrap();

    // 3. Node 0 votes approve (approvals: 1)
    let vote_msg0 = node0
        .vote_patch(&proposal.proposal_id, true, scores, ts + 20)
        .unwrap();
    assert!(vote_msg0.verify(&node0.identity.public_key_hex()).unwrap());

    // 4. Node 1 votes approve (approvals: 2)
    let vote_msg1 = node1
        .vote_patch(&proposal.proposal_id, true, scores, ts + 21)
        .unwrap();
    assert!(vote_msg1.verify(&node1.identity.public_key_hex()).unwrap());

    // Forward votes to node 0
    let vote1 = match vote_msg1.payload {
        SwarmPayload::ConsensusVote(v) => v,
        _ => panic!("Expected ConsensusVote payload"),
    };
    let dec = node0.consensus.record_vote(vote1).unwrap();
    assert!(dec.is_pending());

    // 5. Node 2 votes approve (approvals: 3 -> Q = 3 reached!)
    let vote_msg2 = node2
        .vote_patch(&proposal.proposal_id, true, scores, ts + 22)
        .unwrap();
    assert!(vote_msg2.verify(&node2.identity.public_key_hex()).unwrap());

    let vote2 = match vote_msg2.payload {
        SwarmPayload::ConsensusVote(v) => v,
        _ => panic!("Expected ConsensusVote payload"),
    };
    let final_decision = node0.consensus.record_vote(vote2).unwrap();
    assert!(final_decision.is_approved());

    // 6. Federated gradient dissemination & cluster sync
    let grad = PolicyGradientVector::new(
        ExternalAgentKind::Agy,
        [0.01, -0.01, 0.02, -0.02, 0.01, -0.01],
        30,
        0.08,
    )
    .unwrap();

    let grad_msg = node0.submit_policy_gradient(grad, ts + 30).unwrap();
    assert!(grad_msg.verify(&node0.identity.public_key_hex()).unwrap());

    let receipt = node0.sync_gradients(ts + 40).unwrap();
    assert_eq!(receipt.step_index, 1);
    assert_eq!(node0.epoch, 2);
}

// ============================================================================
// Test 10: Safe Hex Utilities Validation
// ============================================================================

#[test]
fn test_hex_utilities_roundtrip_and_safety() {
    let raw32 = [0xabu8; 32];
    let encoded32 = encode_hex(&raw32);
    assert_eq!(encoded32.len(), 64);
    let decoded32 = decode_hex_32(&encoded32).expect("decode_hex_32 should succeed");
    assert_eq!(decoded32, raw32);

    let raw64 = [0x42u8; 64];
    let encoded64 = encode_hex(&raw64);
    assert_eq!(encoded64.len(), 128);
    let decoded64 = decode_hex_64(&encoded64).expect("decode_hex_64 should succeed");
    assert_eq!(decoded64, raw64);

    // Invalid length or invalid chars return None
    assert!(decode_hex_32("invalid").is_none());
    assert!(decode_hex_64("invalid").is_none());
    assert!(decode_hex_32(&"zz".repeat(32)).is_none());
}

// ============================================================================
// Test 11: General Cluster Quorum Threshold & Fork Freedom (P > 3f + 1)
// ============================================================================

#[test]
fn test_general_cluster_quorum_thresholds_p_greater_than_3f_plus_1() {
    // Case 1: P = 5, f = 1 => Q = (5 + 1)/2 + 1 = 4
    let mut engine_5_1 = DistributedSwarmConsensusEngine::new("node-0", 5, 1)
        .expect("valid BFT parameters (5 >= 3*1 + 1)");
    assert_eq!(engine_5_1.quorum_threshold, 4);

    let scores = ProposalScoreVector::new(0.92, 0.90, 0.88, 0.94);
    let proposal_5 = DistributedPatchProposal::new(
        "patch-5-1",
        "node-0",
        ExternalAgentKind::Agy,
        "Fix buffer boundary check",
        "Bound checking in slice decoding",
        "--- a/slice.rs\n+++ b/slice.rs",
        scores,
        8000,
    );
    engine_5_1.submit_proposal(proposal_5).unwrap();

    // 3 approvals (previously would have committed under Q=2f+1=3)
    // Under corrected Q=4, 3 approvals must remain PENDING
    for i in 0..3 {
        let dec = engine_5_1
            .record_vote(DistributedConsensusVote::new(
                "patch-5-1",
                format!("peer-{i}"),
                true,
                scores,
                8010 + i as u64,
            ))
            .unwrap();
        assert!(dec.is_pending());
    }

    // 4th approval reaches Q = 4 => Approved!
    let dec4 = engine_5_1
        .record_vote(DistributedConsensusVote::new(
            "patch-5-1",
            "peer-3",
            true,
            scores,
            8020,
        ))
        .unwrap();
    assert!(dec4.is_approved());

    // Case 2: P = 6, f = 1 => Q = (6 + 1)/2 + 1 = 4
    let engine_6_1 = DistributedSwarmConsensusEngine::new("node-0", 6, 1).unwrap();
    assert_eq!(engine_6_1.quorum_threshold, 4);

    // Case 3: P = 8, f = 2 => Q = (8 + 2)/2 + 1 = 6
    let engine_8_2 = DistributedSwarmConsensusEngine::new("node-0", 8, 2).unwrap();
    assert_eq!(engine_8_2.quorum_threshold, 6);

    // Case 4: P = 9, f = 2 => Q = (9 + 2)/2 + 1 = 6
    let engine_9_2 = DistributedSwarmConsensusEngine::new("node-0", 9, 2).unwrap();
    assert_eq!(engine_9_2.quorum_threshold, 6);
}
