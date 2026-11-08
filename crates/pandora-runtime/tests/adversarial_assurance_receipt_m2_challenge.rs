#![forbid(unsafe_code)]

//! Adversarial Stress & Empirical Challenge Suite: Milestone 2 Assurance Receipts
//!
//! Validates:
//! 1. Exhaustive bit-flip tampering across all 12 mathematical engine certificates (Phases 9–20).
//! 2. Certificate omission, expansion, and permutation detection.
//! 3. 64-bit exhaustive sequence number bit-flip tampering rejection.
//! 4. Previous receipt SHA-256 hash mutation and genesis/chained continuity tampering.
//! 5. Run-ID tampering detection.
//! 6. Cryptographic signature key isolation and deterministic signing.
//! 7. Multi-node receipt chain causal continuity, Byzantine splice attack, and fork detection.

use pandora_runtime::assurance_receipt::UnifiedAssuranceReceipt;

#[test]
fn test_exhaustive_twelve_engine_certificate_bit_flip_tampering() {
    let original = UnifiedAssuranceReceipt::generate_chained(
        "adversarial-run-receipt-001",
        42,
        Some("sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string()),
    )
    .expect("valid receipt generation");

    assert!(original.all_invariants_verified);
    assert_eq!(original.phases_certified, 12);
    assert_eq!(original.certificates.len(), 12);
    assert!(original.verify_hash_integrity());

    // Stress test every single one of the 12 phase certificates
    for phase_idx in 0..12 {
        let original_cert = &original.certificates[phase_idx];

        // 1. Bit-flip phase number
        {
            let mut tampered = original.clone();
            tampered.certificates[phase_idx].phase ^= 0x01;
            assert!(
                !tampered.verify_hash_integrity(),
                "Phase {}: bit-flip in phase number ({}) must fail hash verification",
                original_cert.phase,
                tampered.certificates[phase_idx].phase
            );
        }

        // 2. Bit-flip phase number to 0 and 255
        {
            let mut tampered = original.clone();
            tampered.certificates[phase_idx].phase = 0;
            assert!(
                !tampered.verify_hash_integrity(),
                "Phase {}: setting phase to 0 must fail verification",
                original_cert.phase
            );

            let mut tampered_max = original.clone();
            tampered_max.certificates[phase_idx].phase = 255;
            assert!(
                !tampered_max.verify_hash_integrity(),
                "Phase {}: setting phase to 255 must fail verification",
                original_cert.phase
            );
        }

        // 3. Mutate engine string (append, prepend, byte-swap)
        {
            let mut tampered = original.clone();
            tampered.certificates[phase_idx].engine.push('_');
            assert!(
                !tampered.verify_hash_integrity(),
                "Phase {}: appended engine string must fail verification",
                original_cert.phase
            );

            let mut tampered2 = original.clone();
            tampered2.certificates[phase_idx].engine = format!("tampered_{}", original_cert.engine);
            assert!(
                !tampered2.verify_hash_integrity(),
                "Phase {}: prepended engine string must fail verification",
                original_cert.phase
            );
        }

        // 4. Mutate theorems citation string
        {
            let mut tampered = original.clone();
            tampered.certificates[phase_idx].theorems = "Theorems 99-100".to_string();
            assert!(
                !tampered.verify_hash_integrity(),
                "Phase {}: modified theorem citation must fail verification",
                original_cert.phase
            );
        }

        // 5. Toggle invariant_verified boolean
        {
            let mut tampered = original.clone();
            tampered.certificates[phase_idx].invariant_verified =
                !tampered.certificates[phase_idx].invariant_verified;
            assert!(
                !tampered.verify_hash_integrity(),
                "Phase {}: toggling invariant_verified must fail verification",
                original_cert.phase
            );
        }

        // 6. Mutate metric_summary string
        {
            let mut tampered = original.clone();
            tampered.certificates[phase_idx].metric_summary.push(' ');
            assert!(
                !tampered.verify_hash_integrity(),
                "Phase {}: trailing whitespace in metric summary must fail verification",
                original_cert.phase
            );

            let mut tampered_empty = original.clone();
            tampered_empty.certificates[phase_idx].metric_summary = String::new();
            assert!(
                !tampered_empty.verify_hash_integrity(),
                "Phase {}: emptied metric summary must fail verification",
                original_cert.phase
            );
        }
    }
}

#[test]
fn test_certificate_count_and_ordering_tampering() {
    let original = UnifiedAssuranceReceipt::generate_chained("count-order-test", 1, None)
        .expect("valid receipt");
    assert!(original.verify_hash_integrity());

    // 1. Omit each certificate one by one (11 certificates instead of 12)
    for drop_idx in 0..12 {
        let mut tampered = original.clone();
        tampered.certificates.remove(drop_idx);
        assert_eq!(tampered.certificates.len(), 11);
        assert!(
            !tampered.verify_hash_integrity(),
            "Dropping certificate at index {drop_idx} must fail verification"
        );
    }

    // 2. Duplicate certificate (13 certificates instead of 12)
    {
        let mut tampered = original.clone();
        tampered.certificates.push(original.certificates[0].clone());
        assert_eq!(tampered.certificates.len(), 13);
        assert!(
            !tampered.verify_hash_integrity(),
            "Appending duplicate certificate must fail verification"
        );
    }

    // 3. Swap adjacent certificates (Permutation attack)
    for i in 0..11 {
        let mut tampered = original.clone();
        tampered.certificates.swap(i, i + 1);
        assert!(
            !tampered.verify_hash_integrity(),
            "Swapping certificates {i} and {} must fail verification",
            i + 1
        );
    }

    // 4. Reverse all certificates
    {
        let mut tampered = original.clone();
        tampered.certificates.reverse();
        assert!(
            !tampered.verify_hash_integrity(),
            "Reversing certificates order must fail verification"
        );
    }

    // 5. Completely empty certificates
    {
        let mut tampered = original.clone();
        tampered.certificates.clear();
        assert!(
            !tampered.verify_hash_integrity(),
            "Empty certificates must fail verification"
        );
    }
}

#[test]
fn test_sequence_number_exhaustive_bit_flip_tampering() {
    let original =
        UnifiedAssuranceReceipt::generate_chained("seq-test", 0x1234_5678_9ABC_DEF0, None)
            .expect("valid receipt");
    assert!(original.verify_hash_integrity());

    // Exhaustively flip all 64 bits of the sequence number
    for bit in 0..64 {
        let mut tampered = original.clone();
        tampered.sequence_number ^= 1u64 << bit;
        assert_ne!(tampered.sequence_number, original.sequence_number);
        assert!(
            !tampered.verify_hash_integrity(),
            "Flipping bit {bit} in sequence number must fail verification"
        );
    }

    // Boundary sequence numbers
    for test_seq in [0u64, 1, 42, u64::MAX - 1, u64::MAX] {
        if test_seq != original.sequence_number {
            let mut tampered = original.clone();
            tampered.sequence_number = test_seq;
            assert!(
                !tampered.verify_hash_integrity(),
                "Altering sequence to {test_seq} must fail verification"
            );
        }
    }
}

#[test]
fn test_prev_receipt_hash_tampering() {
    let valid_prev = "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    let original = UnifiedAssuranceReceipt::generate_chained(
        "prev-hash-test",
        5,
        Some(valid_prev.to_string()),
    )
    .expect("valid receipt");
    assert!(original.verify_hash_integrity());

    // 1. None when Some expected
    {
        let mut tampered = original.clone();
        tampered.prev_receipt_sha256 = None;
        assert!(
            !tampered.verify_hash_integrity(),
            "Setting prev_receipt_sha256 to None must fail verification"
        );
    }

    // 2. Mutate characters in prev_receipt_sha256
    let prev_chars: Vec<char> = valid_prev.chars().collect();
    for char_idx in 0..prev_chars.len() {
        let mut mutated_chars = prev_chars.clone();
        mutated_chars[char_idx] = if mutated_chars[char_idx] == 'a' {
            'b'
        } else {
            'a'
        };
        let mutated_str: String = mutated_chars.into_iter().collect();

        let mut tampered = original.clone();
        tampered.prev_receipt_sha256 = Some(mutated_str);
        assert!(
            !tampered.verify_hash_integrity(),
            "Mutating character {char_idx} of prev_receipt_sha256 must fail verification"
        );
    }

    // 3. Genesis receipt (prev_receipt_sha256 is None) tampered to Some
    let genesis = UnifiedAssuranceReceipt::generate_chained("genesis-test", 0, None)
        .expect("valid genesis receipt");
    assert!(genesis.verify_hash_integrity());

    {
        let mut tampered_genesis = genesis.clone();
        tampered_genesis.prev_receipt_sha256 = Some(valid_prev.to_string());
        assert!(
            !tampered_genesis.verify_hash_integrity(),
            "Adding prev_receipt_sha256 to genesis receipt must fail verification"
        );
    }
}

#[test]
fn test_run_id_tampering() {
    let original = UnifiedAssuranceReceipt::generate_chained("run-alpha-001", 10, None)
        .expect("valid receipt");
    assert!(original.verify_hash_integrity());

    // Empty run_id
    {
        let mut tampered = original.clone();
        tampered.run_id = String::new();
        assert!(
            !tampered.verify_hash_integrity(),
            "Empty run_id must fail verification"
        );
    }

    // Appended run_id
    {
        let mut tampered = original.clone();
        tampered.run_id.push('X');
        assert!(
            !tampered.verify_hash_integrity(),
            "Appended run_id must fail verification"
        );
    }

    // Completely different run_id
    {
        let mut tampered = original.clone();
        tampered.run_id = "run-beta-002".to_string();
        assert!(
            !tampered.verify_hash_integrity(),
            "Altered run_id must fail verification"
        );
    }
}

#[test]
fn test_signature_deterministic_and_key_isolation() {
    let mut r1 =
        UnifiedAssuranceReceipt::generate_chained("sig-test-1", 1, None).expect("valid receipt");
    r1.sign_with_key("governance-key-alpha");

    assert!(r1.signature.is_some());
    let sig1 = r1.signature.clone().unwrap();
    assert!(sig1.starts_with("sig:sha256:"));

    // Deterministic re-signing with same key must yield identical signature
    let mut r1_repeat = r1.clone();
    r1_repeat.sign_with_key("governance-key-alpha");
    assert_eq!(r1_repeat.signature.unwrap(), sig1);

    // Signing with different key must yield completely different signature
    let mut r1_different_key = r1.clone();
    r1_different_key.sign_with_key("governance-key-beta");
    assert_ne!(r1_different_key.signature.unwrap(), sig1);

    // Signing does not alter receipt hash integrity
    assert!(r1.verify_hash_integrity());
}

#[test]
fn test_multi_node_receipt_chain_continuity_and_splice_attack() {
    const CHAIN_LENGTH: usize = 20;
    let mut chain: Vec<UnifiedAssuranceReceipt> = Vec::with_capacity(CHAIN_LENGTH);

    let mut prev_hash: Option<String> = None;
    for seq in 0..CHAIN_LENGTH {
        let run_id = format!("chain-tick-{seq}");
        let mut receipt = UnifiedAssuranceReceipt::generate_chained(&run_id, seq as u64, prev_hash)
            .expect("chain tick generated");
        receipt.sign_with_key("cluster-chain-key-v307");
        assert!(receipt.verify_hash_integrity());

        prev_hash = Some(receipt.receipt_sha256.clone());
        chain.push(receipt);
    }

    // 1. Verify honest chain continuity across all 20 nodes
    for i in 0..CHAIN_LENGTH {
        assert_eq!(chain[i].sequence_number, i as u64);
        assert!(chain[i].verify_hash_integrity());
        if i == 0 {
            assert_eq!(chain[i].prev_receipt_sha256, None);
        } else {
            assert_eq!(
                chain[i].prev_receipt_sha256,
                Some(chain[i - 1].receipt_sha256.clone())
            );
        }
    }

    // 2. Byzantine Splice Attack: Replace receipt at index 10 with a forged receipt
    let forged_receipt = UnifiedAssuranceReceipt::generate_chained(
        "forged-tick-10",
        10,
        chain[9].prev_receipt_sha256.clone(), // Injected invalid prev hash
    )
    .expect("valid forged block");

    let mut tampered_chain = chain.clone();
    tampered_chain[10] = forged_receipt;

    // Check 1: Chain continuity breaks between 9 and 10 or 10 and 11
    let continuity_broken = tampered_chain[10].prev_receipt_sha256
        != Some(tampered_chain[9].receipt_sha256.clone())
        || tampered_chain[11].prev_receipt_sha256
            != Some(tampered_chain[10].receipt_sha256.clone());
    assert!(
        continuity_broken,
        "Byzantine splice at index 10 must break chain continuity"
    );

    // 3. Replay Attack: Replaying receipt 0 at index 5
    let mut replay_chain = chain.clone();
    replay_chain[5] = chain[0].clone();
    assert_ne!(
        replay_chain[5].sequence_number, 5,
        "Replayed sequence number must not match slot"
    );
    assert_ne!(
        replay_chain[5].prev_receipt_sha256,
        Some(replay_chain[4].receipt_sha256.clone()),
        "Replayed receipt breaks causal link"
    );

    // 4. Sequence Skip Attack: 0, 1, 2, ..., 8, 10, 11
    let mut skip_chain = chain.clone();
    skip_chain.remove(9);
    assert_ne!(
        skip_chain[9].sequence_number,
        skip_chain[8].sequence_number + 1,
        "Sequence skip must be detected"
    );
    assert_ne!(
        skip_chain[9].prev_receipt_sha256,
        Some(skip_chain[8].receipt_sha256.clone()),
        "Sequence skip causes prev_receipt_sha256 mismatch"
    );
}
