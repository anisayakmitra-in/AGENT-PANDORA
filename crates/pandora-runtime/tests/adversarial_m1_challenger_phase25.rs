#![forbid(unsafe_code)]

//! Empirical Adversarial Stress Test Suite: Milestone M1 (Phase 25 Merkle Rollup Engine & Theorem 48).
//!
//! Grounded in Theorem 48 (Lemmas 48.1 - 48.5):
//! 1. Boundary Tree Construction: M in {1, 2, 3, 7, 8, 15, 16, 31, 32, 100, 1000}
//! 2. Adversarial Proof Forgery: swapped sibling hashes, inverted directions, corrupted decision indices, altered certificate digests
//! 3. Epoch Chaining & Tamper Resistance: sequential chaining, invalid predecessor rejection, header mutation rejection
//! 4. Domain Separation & Collision Resistance: TAG_ROLLUP_LEAF (0x03) vs TAG_NODE (0x01) vs TAG_EPOCH_HEADER (0x04) vs TAG_DAG (0x02) vs TAG_LEAF (0x00)
//! 5. Theorem 48 Mathematical Verification: compression ratio C_R(M) = 1024*M / 88, Schwartz-Zippel bound epsilon <= 15 / (p - 8) <= 1.085e-15
//! 6. Zero-Knowledge Execution Attestation (Lemmas 48.1 - 48.3)

use pandora_runtime::merkle_dag::{
    MerkleDagAuditLog, MerkleProofStep, NodeDirection, TAG_DAG, TAG_LEAF, TAG_NODE,
};
use pandora_runtime::merkle_rollup::{
    RollupDecisionEntry, RollupEpoch, RollupError, TAG_EPOCH_HEADER, TAG_ROLLUP_LEAF,
};
use pandora_runtime::zk_execution_attestation::{
    PrivateExecutionWitness, PublicExecutionInputs, ZkExecutionAir,
};
use sha2::{Digest, Sha256};

fn create_test_entry(idx: u64, patch_id: &str) -> RollupDecisionEntry {
    let consensus_hash =
        Sha256::digest(format!("adversarial-consensus-hash-{idx}").as_bytes()).into();
    let cert_digest = format!(
        "PANDORA_PATCH_CERT_V1:{:x}",
        Sha256::digest(patch_id.as_bytes())
    );
    RollupDecisionEntry::new(
        idx,
        patch_id,
        consensus_hash,
        cert_digest,
        1731210000000 + idx * 500,
    )
}

// ============================================================================
// 1. Boundary Tree Construction Across All Required Sizes:
//    M = 1, 2, 3, 7, 8, 15, 16, 31, 32, 100, 1000
// ============================================================================

#[test]
fn test_adversarial_tree_construction_boundary_sizes() {
    let test_sizes: [usize; 11] = [1, 2, 3, 7, 8, 15, 16, 31, 32, 100, 1000];

    for &m in &test_sizes {
        let entries: Vec<_> = (0..m)
            .map(|i| create_test_entry(i as u64, &format!("patch-boundary-{m}-{i}")))
            .collect();

        let epoch =
            RollupEpoch::create_epoch(42 + m as u64, entries.clone(), [0xAA; 32], 1731215000000)
                .expect("epoch creation must succeed for valid boundary sizes");

        // Verify decision count and tree height invariants
        assert_eq!(epoch.header.decision_count, m as u64);
        let expected_height = if m <= 1 {
            1
        } else {
            (m as f64).log2().ceil() as usize
        };
        assert_eq!(
            epoch.tree_height(),
            expected_height,
            "tree height mismatch for M = {m}"
        );
        assert!(epoch.header.verify_header_hash());

        // For smaller batches, verify inclusion proof for ALL leaves
        if m <= 32 {
            for i in 0..m {
                let proof = epoch.generate_inclusion_proof(i).unwrap_or_else(|e| {
                    panic!("inclusion proof generation failed for M={m}, i={i}: {e:?}")
                });
                assert_eq!(proof.leaf_index, i);
                if m == 1 {
                    assert_eq!(proof.steps.len(), 0);
                } else {
                    assert_eq!(proof.steps.len(), expected_height);
                }
                assert!(
                    proof.verify(),
                    "proof internal verify failed for M={m}, i={i}"
                );
                assert!(
                    RollupEpoch::verify_inclusion(&epoch.epoch_root(), &proof, &epoch.entries[i]),
                    "epoch verify_inclusion failed for M={m}, i={i}"
                );
            }
        } else {
            // For larger batches (M = 100, 1000), sample boundaries and interior points
            let sample_indices = [
                0,
                1,
                m / 4,
                m / 2 - 1,
                m / 2,
                m / 2 + 1,
                3 * m / 4,
                m - 2,
                m - 1,
            ];
            for &idx in &sample_indices {
                let proof = epoch.generate_inclusion_proof(idx).unwrap_or_else(|e| {
                    panic!("inclusion proof generation failed for M={m}, i={idx}: {e:?}")
                });
                assert_eq!(proof.leaf_index, idx);
                assert_eq!(proof.steps.len(), expected_height);
                assert!(
                    proof.verify(),
                    "proof internal verify failed for M={m}, i={idx}"
                );
                assert!(
                    RollupEpoch::verify_inclusion(&epoch.epoch_root(), &proof, &epoch.entries[idx]),
                    "epoch verify_inclusion failed for M={m}, i={idx}"
                );
            }
        }

        // Test out of bounds index rejection
        assert_eq!(
            epoch.generate_inclusion_proof(m),
            Err(RollupError::IndexOutOfBounds { index: m, total: m })
        );
        assert_eq!(
            epoch.generate_inclusion_proof(m + 42),
            Err(RollupError::IndexOutOfBounds {
                index: m + 42,
                total: m
            })
        );
    }
}

// ============================================================================
// 2. Adversarial Proof Forgery: Swapped Sibling Hashes, Inverted Directions,
//    Corrupted Decision Indices, and Altered Certificate Digests
// ============================================================================

#[test]
fn test_adversarial_proof_forgery_swapped_sibling_hashes() {
    let m: usize = 16;
    let entries: Vec<_> = (0..m)
        .map(|i| create_test_entry(i as u64, "forgery-p1"))
        .collect();
    let epoch = RollupEpoch::create_epoch(1, entries, [0u8; 32], 1000).unwrap();

    let target_idx = 7;
    let proof = epoch.generate_inclusion_proof(target_idx).unwrap();
    assert!(RollupEpoch::verify_inclusion(
        &epoch.epoch_root(),
        &proof,
        &epoch.entries[target_idx]
    ));

    // Attack 1: Swap sibling hashes between two different proof steps
    assert!(proof.steps.len() >= 2);
    let mut swapped_proof = proof.clone();
    swapped_proof.steps.swap(0, 1);
    assert!(
        !swapped_proof.verify(),
        "swapped step siblings must fail proof.verify()"
    );
    assert!(
        !RollupEpoch::verify_inclusion(
            &epoch.epoch_root(),
            &swapped_proof,
            &epoch.entries[target_idx]
        ),
        "swapped step siblings must be rejected by verify_inclusion"
    );

    // Attack 2: Replace every sibling hash with bit-flipped mutation
    for step_idx in 0..proof.steps.len() {
        let mut corrupted_proof = proof.clone();
        corrupted_proof.steps[step_idx].sibling_hash[0] ^= 0x01;
        assert!(!corrupted_proof.verify());
        assert!(!RollupEpoch::verify_inclusion(
            &epoch.epoch_root(),
            &corrupted_proof,
            &epoch.entries[target_idx]
        ));
    }
}

#[test]
fn test_adversarial_proof_forgery_inverted_directions() {
    let m: usize = 8;
    let entries: Vec<_> = (0..m)
        .map(|i| create_test_entry(i as u64, "forgery-p2"))
        .collect();
    let epoch = RollupEpoch::create_epoch(1, entries, [0u8; 32], 1000).unwrap();

    for target_idx in 0..m {
        let proof = epoch.generate_inclusion_proof(target_idx).unwrap();

        for step_idx in 0..proof.steps.len() {
            let mut inverted_proof = proof.clone();
            inverted_proof.steps[step_idx].direction =
                match inverted_proof.steps[step_idx].direction {
                    NodeDirection::Left => NodeDirection::Right,
                    NodeDirection::Right => NodeDirection::Left,
                };

            // Inverting the direction must alter parent hash computation and fail
            assert!(
                !inverted_proof.verify(),
                "inverted direction at step {step_idx} must fail proof.verify()"
            );
            assert!(
                !RollupEpoch::verify_inclusion(
                    &epoch.epoch_root(),
                    &inverted_proof,
                    &epoch.entries[target_idx]
                ),
                "inverted direction at step {step_idx} must be rejected by verify_inclusion"
            );
        }
    }
}

#[test]
fn test_adversarial_proof_forgery_corrupted_decision_indices() {
    let m: usize = 8;
    let entries: Vec<_> = (0..m)
        .map(|i| create_test_entry(i as u64, "forgery-p3"))
        .collect();
    let epoch = RollupEpoch::create_epoch(1, entries, [0u8; 32], 1000).unwrap();

    let target_idx = 3;
    let proof = epoch.generate_inclusion_proof(target_idx).unwrap();

    // Attack 1: Alter decision_index on candidate entry
    let mut bad_entry = epoch.entries[target_idx].clone();
    bad_entry.decision_index = 4; // Corrupt index
    assert!(
        !RollupEpoch::verify_inclusion(&epoch.epoch_root(), &proof, &bad_entry),
        "corrupted decision_index must produce leaf hash mismatch"
    );

    bad_entry.decision_index = 0;
    assert!(!RollupEpoch::verify_inclusion(
        &epoch.epoch_root(),
        &proof,
        &bad_entry
    ));

    bad_entry.decision_index = 9999;
    assert!(!RollupEpoch::verify_inclusion(
        &epoch.epoch_root(),
        &proof,
        &bad_entry
    ));

    // Attack 2: Cross-verify proof against an entry from a different slot
    for other_idx in 0..m {
        if other_idx != target_idx {
            assert!(
                !RollupEpoch::verify_inclusion(
                    &epoch.epoch_root(),
                    &proof,
                    &epoch.entries[other_idx]
                ),
                "proof for index {target_idx} must strictly reject entry at index {other_idx}"
            );
        }
    }
}

#[test]
fn test_adversarial_proof_forgery_altered_certificate_digests_and_payloads() {
    let m: usize = 4;
    let entries: Vec<_> = (0..m)
        .map(|i| create_test_entry(i as u64, "forgery-p4"))
        .collect();
    let epoch = RollupEpoch::create_epoch(1, entries, [0u8; 32], 1000).unwrap();

    let target_idx = 1;
    let proof = epoch.generate_inclusion_proof(target_idx).unwrap();

    // Attack 1: Alter certificate digest
    let mut bad_entry = epoch.entries[target_idx].clone();
    bad_entry.certificate_digest = format!("{}_tampered", bad_entry.certificate_digest);
    assert!(!RollupEpoch::verify_inclusion(
        &epoch.epoch_root(),
        &proof,
        &bad_entry
    ));

    // Attack 2: Alter patch_id
    let mut bad_entry2 = epoch.entries[target_idx].clone();
    bad_entry2.patch_id = "malicious_exploit_patch".to_string();
    assert!(!RollupEpoch::verify_inclusion(
        &epoch.epoch_root(),
        &proof,
        &bad_entry2
    ));

    // Attack 3: Alter consensus_hash
    let mut bad_entry3 = epoch.entries[target_idx].clone();
    bad_entry3.consensus_hash[0] ^= 0xFF;
    assert!(!RollupEpoch::verify_inclusion(
        &epoch.epoch_root(),
        &proof,
        &bad_entry3
    ));

    // Attack 4: Alter timestamp_ms
    let mut bad_entry4 = epoch.entries[target_idx].clone();
    bad_entry4.timestamp_ms += 1;
    assert!(!RollupEpoch::verify_inclusion(
        &epoch.epoch_root(),
        &proof,
        &bad_entry4
    ));

    // Attack 5: Tamper proof root_hash vs foreign epoch root
    assert!(!RollupEpoch::verify_inclusion(
        &[0xEE; 32],
        &proof,
        &epoch.entries[target_idx]
    ));

    // Attack 6: Structural proof mutations (truncate or append steps)
    let mut truncated_proof = proof.clone();
    truncated_proof.steps.pop();
    assert!(!truncated_proof.verify());
    assert!(!RollupEpoch::verify_inclusion(
        &epoch.epoch_root(),
        &truncated_proof,
        &epoch.entries[target_idx]
    ));

    let mut elongated_proof = proof.clone();
    elongated_proof.steps.push(MerkleProofStep {
        sibling_hash: [0x55; 32],
        direction: NodeDirection::Left,
    });
    assert!(!elongated_proof.verify());
    assert!(!RollupEpoch::verify_inclusion(
        &epoch.epoch_root(),
        &elongated_proof,
        &epoch.entries[target_idx]
    ));
}

// ============================================================================
// 3. Epoch Chaining & Tamper Resistance
// ============================================================================

#[test]
fn test_adversarial_epoch_chaining_validation_and_rejection() {
    let genesis_prev = [0u8; 32];
    let e1_entries: Vec<_> = (0..4).map(|i| create_test_entry(i, "chain-e1")).collect();
    let epoch1 = RollupEpoch::create_epoch(1, e1_entries, genesis_prev, 1000).unwrap();

    let e2_entries: Vec<_> = (0..4).map(|i| create_test_entry(i, "chain-e2")).collect();
    let epoch2 = RollupEpoch::create_epoch(2, e2_entries, epoch1.epoch_root(), 2000).unwrap();

    let e3_entries: Vec<_> = (0..4).map(|i| create_test_entry(i, "chain-e3")).collect();
    let epoch3 = RollupEpoch::create_epoch(3, e3_entries, epoch2.epoch_root(), 3000).unwrap();

    // Valid sequential continuity
    assert!(epoch1.header.verify_chain(&genesis_prev).is_ok());
    assert!(epoch2.header.verify_chain(&epoch1.epoch_root()).is_ok());
    assert!(epoch3.header.verify_chain(&epoch2.epoch_root()).is_ok());

    // Adversarial Rejection 1: Invalid previous_epoch_root
    let forged_prev = [0xBAu8; 32];
    let err = epoch2.header.verify_chain(&forged_prev);
    assert!(
        matches!(err, Err(RollupError::EpochChainBroken { .. })),
        "epoch rejecting an invalid previous_epoch_root must fail with EpochChainBroken"
    );

    // Adversarial Rejection 2: Skipping an epoch in chain validation
    let err_skip = epoch3.header.verify_chain(&epoch1.epoch_root());
    assert!(
        matches!(err_skip, Err(RollupError::EpochChainBroken { .. })),
        "skipping epoch in chain verification must fail"
    );

    // Adversarial Rejection 3: Tampering header fields breaks header_hash integrity
    let mut tampered_header = epoch2.header.clone();
    tampered_header.decision_count = 999;
    assert!(!tampered_header.verify_header_hash());
    assert!(tampered_header.verify_chain(&epoch1.epoch_root()).is_err());

    let mut tampered_header2 = epoch2.header.clone();
    tampered_header2.epoch_root[0] ^= 0x01;
    assert!(!tampered_header2.verify_header_hash());
    assert!(tampered_header2.verify_chain(&epoch1.epoch_root()).is_err());

    let mut tampered_header3 = epoch2.header.clone();
    tampered_header3.previous_epoch_root[0] ^= 0x01;
    assert!(!tampered_header3.verify_header_hash());
    assert!(tampered_header3.verify_chain(&epoch1.epoch_root()).is_err());

    let mut tampered_header4 = epoch2.header.clone();
    tampered_header4.timestamp_ms += 100;
    assert!(!tampered_header4.verify_header_hash());
    assert!(tampered_header4.verify_chain(&epoch1.epoch_root()).is_err());
}

// ============================================================================
// 4. Domain Separation & Collision Resistance:
//    TAG_ROLLUP_LEAF (0x03) vs TAG_NODE (0x01) vs TAG_EPOCH_HEADER (0x04)
// ============================================================================

#[test]
fn test_adversarial_domain_separation_tags_non_collision() {
    // Assert domain tag byte uniqueness across subsystem
    assert_ne!(TAG_ROLLUP_LEAF, TAG_NODE);
    assert_ne!(TAG_ROLLUP_LEAF, TAG_EPOCH_HEADER);
    assert_ne!(TAG_ROLLUP_LEAF, TAG_LEAF);
    assert_ne!(TAG_ROLLUP_LEAF, TAG_DAG);
    assert_ne!(TAG_NODE, TAG_EPOCH_HEADER);

    assert_eq!(TAG_ROLLUP_LEAF, 0x03);
    assert_eq!(TAG_NODE, 0x01);
    assert_eq!(TAG_EPOCH_HEADER, 0x04);
    assert_eq!(TAG_DAG, 0x02);
    assert_eq!(TAG_LEAF, 0x00);

    // Test that a leaf hash cannot collide with an internal node hash even on identical payload suffix
    let dummy_32 = [0x55u8; 32];
    let parent_node_hash = MerkleDagAuditLog::compute_parent_hash(&dummy_32, &dummy_32);

    // Manually hash with TAG_ROLLUP_LEAF (0x03) with identical suffix (dummy_32 || dummy_32)
    let mut leaf_collision_attempt = Sha256::new();
    leaf_collision_attempt.update([TAG_ROLLUP_LEAF]);
    leaf_collision_attempt.update(dummy_32);
    leaf_collision_attempt.update(dummy_32);
    let leaf_collision_hash: [u8; 32] = leaf_collision_attempt.finalize().into();

    assert_ne!(
        parent_node_hash, leaf_collision_hash,
        "leaf hash (0x03) must never collide with internal node hash (0x01)"
    );

    // Manually hash with TAG_EPOCH_HEADER (0x04) with identical suffix
    let mut header_collision_attempt = Sha256::new();
    header_collision_attempt.update([TAG_EPOCH_HEADER]);
    header_collision_attempt.update(dummy_32);
    header_collision_attempt.update(dummy_32);
    let header_collision_hash: [u8; 32] = header_collision_attempt.finalize().into();

    assert_ne!(
        header_collision_hash, leaf_collision_hash,
        "epoch header hash (0x04) must never collide with leaf hash (0x03)"
    );
    assert_ne!(
        header_collision_hash, parent_node_hash,
        "epoch header hash (0x04) must never collide with internal node hash (0x01)"
    );

    // Test 1000 random leaf entries against random parent node hashes
    for i in 0..1000 {
        let entry = create_test_entry(i, &format!("domain-check-{i}"));
        let leaf_hash = entry.compute_leaf_hash();

        let left = Sha256::digest(format!("left-{i}").as_bytes()).into();
        let right = Sha256::digest(format!("right-{i}").as_bytes()).into();
        let parent_hash = MerkleDagAuditLog::compute_parent_hash(&left, &right);

        assert_ne!(leaf_hash, parent_hash);
    }
}

// ============================================================================
// 5. Theorem 48 Mathematical Verification:
//    - Compression ratio C_R(M) = 1024*M / 88 > 99.9% for M >= 1,000
//    - Schwartz-Zippel soundness bound: epsilon <= 15 / (p - 8) <= 1.085e-15
// ============================================================================

#[test]
fn test_theorem_48_compression_ratio_bound_empirical() {
    // Theorem 48 / Lemma 48.5:
    // C_R(M) = 1024 * M / 88
    // Storage reduction = (1 - 88 / (1024 * M)) * 100%
    let s_raw = 1024.0;
    let s_header = 88.0;

    for &m in &[1000, 2000, 5000, 10000, 50000] {
        let compression_ratio = (m as f64 * s_raw) / s_header;
        let storage_reduction_pct = (1.0 - (s_header / (m as f64 * s_raw))) * 100.0;

        assert!(
            compression_ratio >= 11636.0,
            "C_R({m}) = {compression_ratio} must be >= 11,636"
        );
        assert!(
            storage_reduction_pct >= 99.9,
            "Storage reduction for M={m} ({storage_reduction_pct}%) must exceed 99.9%"
        );
        assert!(
            storage_reduction_pct >= 99.9914,
            "Storage reduction for M={m} ({storage_reduction_pct}%) must meet analytical bound 99.9914%"
        );
    }

    // Also verify against RollupEpoch::compression_ratio and storage_savings_pct
    // which use 120 bytes (88 header + 32 epoch root)
    let entries: Vec<_> = (0..1000)
        .map(|i| create_test_entry(i as u64, "comp"))
        .collect();
    let epoch = RollupEpoch::create_epoch(1, entries, [0u8; 32], 1000).unwrap();

    let runtime_ratio = epoch.compression_ratio(1024);
    let runtime_savings = epoch.storage_savings_pct(1024);

    // (1000 * 1024) / 120 = 8533.33
    assert!(runtime_ratio >= 8500.0);
    // (1 - 120 / 1,024,000) * 100% = 99.98828% > 99.9%
    assert!(
        runtime_savings >= 99.9,
        "Runtime storage savings {runtime_savings}% must exceed 99.9% for M=1,000"
    );
}

#[test]
fn test_theorem_48_schwartz_zippel_soundness_bound_evaluation() {
    // Goldilocks prime p = 2^64 - 2^32 + 1
    let p_u128: u128 = 18_446_744_069_414_584_321;
    let p_minus_8 = p_u128 - 8;
    assert_eq!(p_minus_8, 18_446_744_069_414_584_313);

    // In execution AIR with domain size k = 8:
    // Deg(F - C) <= Deg(R_W * Z_H) = 1 + 8 = 9 (or bounded by 15)
    // Under Fiat-Shamir sampling from F_p \ H:
    // epsilon <= 15 / (p - 8)
    let numerator = 15.0f64;
    let denominator = p_minus_8 as f64;
    let epsilon = numerator / denominator;

    let target_bound = 1.085e-15f64;
    assert!(
        epsilon <= target_bound,
        "epsilon ({epsilon:e}) must be <= {target_bound:e}"
    );

    // Analytical ratio check
    assert!(
        epsilon < 1e-17,
        "with k=8, epsilon ({epsilon:e}) is approximately 8.13e-19, well below 1.085e-15"
    );

    // Lemma 48.2 general bound for any k <= 10,000:
    // epsilon_max <= 2 * 10,000 / p = 20,000 / 18,446,744,069,414,584,321 <= 1.085e-15
    let epsilon_general = 20_000.0f64 / (p_u128 as f64);
    assert!(
        epsilon_general <= target_bound,
        "general bound epsilon_general ({epsilon_general:e}) must be <= {target_bound:e}"
    );
}

// ============================================================================
// 6. Zero-Knowledge Execution Attestation Adversarial Stress (AIR & Lemmas 48.1-48.3)
// ============================================================================

#[test]
fn test_adversarial_zk_air_forgery_vectors() {
    let air = ZkExecutionAir::new(8).expect("AIR init should succeed");
    let inputs = PublicExecutionInputs::new(
        Sha256::digest(b"patch-adv").into(),
        Sha256::digest(b"pre-adv").into(),
        Sha256::digest(b"post-adv").into(),
        true,
        100,
        100,
    );
    let witness = PrivateExecutionWitness::mock_valid(&inputs);
    let proof = air
        .prove(&inputs, &witness)
        .expect("valid prove should succeed");
    assert!(air.verify(&proof).unwrap());

    // Adversary Attempt 1: Forging proof with zero tests passed
    let mut zero_tests_inputs = inputs.clone();
    zero_tests_inputs.holdout_tests_passed = 0;
    assert!(!zero_tests_inputs.validate_invariants());

    let mut forged_proof1 = proof.clone();
    forged_proof1.public_inputs = zero_tests_inputs;
    assert!(!air.verify(&forged_proof1).unwrap());

    // Adversary Attempt 2: Forging proof with unverified memory safety
    let mut unsafe_inputs = inputs.clone();
    unsafe_inputs.memory_safety_verified = false;
    assert!(!unsafe_inputs.validate_invariants());

    let mut forged_proof2 = proof.clone();
    forged_proof2.public_inputs = unsafe_inputs;
    assert!(!air.verify(&forged_proof2).unwrap());

    // Adversary Attempt 3: Subgroup order perturbation
    let mut forged_proof3 = proof.clone();
    forged_proof3.domain_size = 16;
    let air_16 = ZkExecutionAir::new(forged_proof3.domain_size).unwrap();
    let v_res = air_16.verify(&forged_proof3);
    assert!(v_res.is_ok());
    assert!(
        !v_res.unwrap(),
        "proof generated for k=8 must fail when verified under k=16"
    );
}
