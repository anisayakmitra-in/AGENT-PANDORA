#![forbid(unsafe_code)]

//! Phase 25 Merkle Rollup Batch Aggregation & Inclusion Verification Tests (Theorem 48).

use pandora_runtime::assurance_receipt::UnifiedAssuranceReceipt;
use pandora_runtime::distributed_swarm::DistributedConsensusDecision;
use pandora_runtime::merkle_rollup::{
    EpochRollupHeader, RollupDecisionEntry, RollupEpoch, RollupError, TAG_EPOCH_HEADER,
    TAG_ROLLUP_LEAF, decode_hex_32, encode_hex,
};
use sha2::{Digest, Sha256};

fn create_sample_entry(idx: u64, patch_id: &str) -> RollupDecisionEntry {
    let consensus_hash = Sha256::digest(format!("consensus-hash-{idx}").as_bytes()).into();
    let cert_digest = format!(
        "PANDORA_PATCH_CERT_V1:{:x}",
        Sha256::digest(patch_id.as_bytes())
    );
    RollupDecisionEntry::new(
        idx,
        patch_id,
        consensus_hash,
        cert_digest,
        1731210000000 + idx * 1000,
    )
}

#[test]
fn test_hex_codec_and_epoch_header_domain_tag() {
    let bytes = [0x42u8; 32];
    let hex = encode_hex(&bytes);
    assert_eq!(hex.len(), 64);
    let decoded = decode_hex_32(&hex).expect("should decode valid 32-byte hex");
    assert_eq!(decoded, bytes);

    let h = EpochRollupHeader::compute_header_hash(1, 10, &bytes, &[0u8; 32], 12345);
    let mut manual = Sha256::new();
    manual.update([TAG_EPOCH_HEADER]);
    manual.update(1u64.to_be_bytes());
    manual.update(10u64.to_be_bytes());
    manual.update(bytes);
    manual.update([0u8; 32]);
    manual.update(12345u64.to_be_bytes());
    let expected: [u8; 32] = manual.finalize().into();
    assert_eq!(h, expected);
}

#[test]
fn test_rollup_leaf_hash_domain_tag_isolation() {
    let entry = create_sample_entry(0, "prop-alpha");
    let leaf_hash = entry.compute_leaf_hash();

    // Verify domain tag 0x03 was used as first byte in digest calculation
    let mut manual_hasher = Sha256::new();
    manual_hasher.update([TAG_ROLLUP_LEAF]);
    manual_hasher.update(0u64.to_be_bytes());
    manual_hasher.update((entry.patch_id.len() as u32).to_be_bytes());
    manual_hasher.update(entry.patch_id.as_bytes());
    manual_hasher.update(entry.consensus_hash);
    manual_hasher.update((entry.certificate_digest.len() as u32).to_be_bytes());
    manual_hasher.update(entry.certificate_digest.as_bytes());
    manual_hasher.update(entry.timestamp_ms.to_be_bytes());
    let expected: [u8; 32] = manual_hasher.finalize().into();

    assert_eq!(leaf_hash, expected);
}

#[test]
fn test_rollup_epoch_power_of_two_batch() {
    for batch_size in [2, 4, 8, 16, 64] {
        let entries: Vec<_> = (0..batch_size)
            .map(|i| create_sample_entry(i as u64, &format!("prop-{i}")))
            .collect();

        let epoch = RollupEpoch::create_epoch(1, entries, [0u8; 32], 1731210000000)
            .expect("epoch creation should succeed");

        assert_eq!(epoch.header.decision_count, batch_size as u64);
        assert_eq!(
            epoch.tree_height(),
            (batch_size as f64).log2().ceil() as usize
        );
        assert_ne!(epoch.epoch_root(), [0u8; 32]);
        assert!(epoch.header.verify_header_hash());
    }
}

#[test]
fn test_rollup_epoch_odd_and_prime_batch_sizes() {
    for batch_size in [1, 3, 5, 7, 13, 31] {
        let entries: Vec<_> = (0..batch_size)
            .map(|i| create_sample_entry(i as u64, &format!("prop-{i}")))
            .collect();

        let epoch = RollupEpoch::create_epoch(1, entries, [0u8; 32], 1731210000000)
            .expect("epoch creation should succeed");

        assert_eq!(epoch.header.decision_count, batch_size as u64);
        assert!(epoch.header.verify_header_hash());

        // Verify every leaf has a verifiable inclusion proof despite odd trailing leaf duplications
        for i in 0..batch_size {
            let proof = epoch
                .generate_inclusion_proof(i)
                .expect("proof should generate");
            assert!(RollupEpoch::verify_inclusion(
                &epoch.epoch_root(),
                &proof,
                &epoch.entries[i]
            ));
        }
    }
}

#[test]
fn test_rollup_inclusion_proof_log_time_verification() {
    let m = 32;
    let entries: Vec<_> = (0..m)
        .map(|i| create_sample_entry(i as u64, &format!("prop-{i}")))
        .collect();

    let epoch = RollupEpoch::create_epoch(1, entries, [0u8; 32], 1731210000000).unwrap();

    for i in 0..m {
        let proof = epoch.generate_inclusion_proof(i).unwrap();
        assert_eq!(proof.leaf_index, i);
        assert_eq!(proof.steps.len(), 5); // log2(32) = 5
        assert!(proof.verify());
        assert!(RollupEpoch::verify_inclusion(
            &epoch.epoch_root(),
            &proof,
            &epoch.entries[i]
        ));
    }
}

#[test]
fn test_rollup_inclusion_proof_tamper_rejection() {
    let entries: Vec<_> = (0..8)
        .map(|i| create_sample_entry(i as u64, &format!("prop-{i}")))
        .collect();

    let epoch = RollupEpoch::create_epoch(1, entries, [0u8; 32], 1731210000000).unwrap();
    let mut proof = epoch.generate_inclusion_proof(3).unwrap();

    // 1. Tamper sibling hash in proof step
    proof.steps[0].sibling_hash[0] ^= 0xFF;
    assert!(!proof.verify());
    assert!(!RollupEpoch::verify_inclusion(
        &epoch.epoch_root(),
        &proof,
        &epoch.entries[3]
    ));

    // 2. Tamper leaf hash directly
    let mut proof2 = epoch.generate_inclusion_proof(3).unwrap();
    proof2.leaf_hash[0] ^= 0xAA;
    assert!(!RollupEpoch::verify_inclusion(
        &epoch.epoch_root(),
        &proof2,
        &epoch.entries[3]
    ));

    // 3. Target mismatch against foreign entry
    let proof3 = epoch.generate_inclusion_proof(3).unwrap();
    let foreign_entry = create_sample_entry(999, "prop-forged");
    assert!(!RollupEpoch::verify_inclusion(
        &epoch.epoch_root(),
        &proof3,
        &foreign_entry
    ));
}

#[test]
fn test_epoch_header_sequential_hash_chaining() {
    let entries1: Vec<_> = (0..4).map(|i| create_sample_entry(i, "p1")).collect();
    let epoch1 = RollupEpoch::create_epoch(1, entries1, [0u8; 32], 1000).unwrap();

    let entries2: Vec<_> = (0..4).map(|i| create_sample_entry(i, "p2")).collect();
    let epoch2 = RollupEpoch::create_epoch(2, entries2, epoch1.epoch_root(), 2000).unwrap();

    let entries3: Vec<_> = (0..4).map(|i| create_sample_entry(i, "p3")).collect();
    let epoch3 = RollupEpoch::create_epoch(3, entries3, epoch2.epoch_root(), 3000).unwrap();

    assert!(epoch2.header.verify_chain(&epoch1.epoch_root()).is_ok());
    assert!(epoch3.header.verify_chain(&epoch2.epoch_root()).is_ok());

    // Tampered predecessor chain link must fail
    let forged_prev_root = [0xFFu8; 32];
    assert!(epoch3.header.verify_chain(&forged_prev_root).is_err());
}

#[test]
fn test_rollup_compression_ratio_bounds() {
    let m = 64;
    let entries: Vec<_> = (0..m).map(|i| create_sample_entry(i, "patch")).collect();
    let epoch = RollupEpoch::create_epoch(1, entries, [0u8; 32], 1000).unwrap();

    let ratio = epoch.compression_ratio(65_536);
    let savings = epoch.storage_savings_pct(65_536);

    // 64 decisions * 64 KB = 4 MB. Rollup footprint = 120 bytes.
    // Ratio ~ 34,952x, savings > 99.99%
    assert!(ratio >= 1000.0);
    assert!(savings >= 99.9);
}

#[test]
fn test_unified_assurance_receipt_with_phase25_attestation() {
    let entries: Vec<_> = (0..8).map(|i| create_sample_entry(i, "p")).collect();
    let epoch = RollupEpoch::create_epoch(1, entries, [0u8; 32], 1000).unwrap();

    let receipt = UnifiedAssuranceReceipt::generate_with_phase25(
        "run-phase25-rollup-test",
        epoch.epoch_root(),
        8,
        true,
    )
    .expect("receipt generation should succeed");

    assert!(receipt.all_invariants_verified);
    assert_eq!(receipt.certificates.len(), 13);
    assert_eq!(receipt.phases_certified, 13);

    let phase25_cert = receipt.certificates.iter().find(|c| c.phase == 25).unwrap();
    assert_eq!(phase25_cert.theorems, "Theorem 48");
    assert!(phase25_cert.invariant_verified);
    assert!(receipt.verify_hash_integrity());
}

#[test]
fn test_rollup_error_handling_boundaries() {
    // Empty decisions rejection
    let empty_res = RollupEpoch::create_epoch(1, Vec::new(), [0u8; 32], 1000);
    assert_eq!(empty_res, Err(RollupError::EmptyDecisions));

    // Out of bounds index
    let entries: Vec<_> = (0..4).map(|i| create_sample_entry(i, "p")).collect();
    let epoch = RollupEpoch::create_epoch(1, entries, [0u8; 32], 1000).unwrap();
    let oob_res = epoch.generate_inclusion_proof(10);
    assert!(matches!(
        oob_res,
        Err(RollupError::IndexOutOfBounds {
            index: 10,
            total: 4
        })
    ));

    // Unapproved decision rejection
    let pending_decision = DistributedConsensusDecision::Pending {
        current_approvals: 1,
        current_rejections: 0,
        quorum_needed: 3,
    };
    let unapproved_res = RollupDecisionEntry::from_consensus_decision(&pending_decision, 0);
    assert!(matches!(
        unapproved_res,
        Err(RollupError::UnapprovedDecision(_))
    ));
}
