#![forbid(unsafe_code)]

//! Milestone M1 Empirical Adversarial Challenge Test Harness (Theorem 48).
//!
//! Evaluates ZkExecutionAir, ZkExecutionProof, and Goldilocks field AIR arithmetic
//! under adversarial stress conditions, malicious forgery probes, parameter corruption,
//! Fiat-Shamir transcript tampering, zero-knowledge blinding entropy, and latency bounds.

use pandora_runtime::zk_attestation::{
    FiatShamirTranscript, FieldElement, GOLDILOCKS_PRIME, Polynomial, ZkAttestationError,
};
use pandora_runtime::zk_execution_attestation::{
    EXECUTION_TRANSCRIPT_DOMAIN, PrivateExecutionWitness, PublicExecutionInputs, ZkExecutionAir,
    ZkExecutionProof,
};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::time::Instant;

fn mock_valid_public_inputs() -> PublicExecutionInputs {
    let patch_digest = Sha256::digest(b"patch-code-adversarial-probe-v1").into();
    let pre_state_root = Sha256::digest(b"pre-state-root-anchor").into();
    let post_state_root = Sha256::digest(b"post-state-root-anchor").into();
    PublicExecutionInputs::new(patch_digest, pre_state_root, post_state_root, true, 50, 50)
}

fn mock_honest_witness(test_count: u32) -> PrivateExecutionWitness {
    PrivateExecutionWitness::clean_pass([0xAA; 32], [0xBB; 32], test_count, None)
}

// -----------------------------------------------------------------------------
// Test 1: Forgery Attempt via Prover - Memory Safety Violations
// -----------------------------------------------------------------------------
#[test]
fn test_adversarial_prover_rejects_memory_safety_violations() {
    let air = ZkExecutionAir::new(8).expect("AIR init");
    let inputs = mock_valid_public_inputs(); // claims memory_safety_verified = true

    for violations in [1, 2, 5, 10, 100, 1000] {
        let witness =
            PrivateExecutionWitness::with_violations([0x11; 32], [0x22; 32], violations, 50);
        let result = air.prove(&inputs, &witness);
        assert!(
            result.is_err(),
            "prover must reject execution with {} violations",
            violations
        );
        match result.unwrap_err() {
            ZkAttestationError::InvalidTransitionConstraint { remainder_degree } => {
                assert!(remainder_degree < 8, "remainder degree must be < 8");
            }
            other => panic!("expected InvalidTransitionConstraint, got {:?}", other),
        }
    }
}

// -----------------------------------------------------------------------------
// Test 2: Forgery Attempt via Prover - Holdout Test Failures
// -----------------------------------------------------------------------------
#[test]
fn test_adversarial_prover_rejects_failing_holdout_tests() {
    let air = ZkExecutionAir::new(8).expect("AIR init");
    let inputs = mock_valid_public_inputs(); // claims 50/50 passed

    for failed_count in [1, 2, 10, 25, 49, 50] {
        let mut executions = vec![true; 50];
        for item in executions.iter_mut().take(failed_count) {
            *item = false;
        }
        let witness =
            PrivateExecutionWitness::new([0x11; 32], [0x22; 32], 0, executions, Vec::new());
        let result = air.prove(&inputs, &witness);
        assert!(
            result.is_err(),
            "prover must reject execution with {} failed tests",
            failed_count
        );
        assert!(matches!(
            result.unwrap_err(),
            ZkAttestationError::InvalidTransitionConstraint { .. }
        ));
    }
}

// -----------------------------------------------------------------------------
// Test 3: Forgery Attempt via Prover - Mismatched Test Counts
// -----------------------------------------------------------------------------
#[test]
fn test_adversarial_prover_rejects_mismatched_test_counts() {
    let air = ZkExecutionAir::new(8).expect("AIR init");
    let inputs = mock_valid_public_inputs(); // claims 50 tests

    // Witness only executed 30 tests (all passed)
    let witness_short = PrivateExecutionWitness::clean_pass([0x11; 32], [0x22; 32], 30, None);
    let result = air.prove(&inputs, &witness_short);
    assert!(result.is_err(), "prover must reject test count mismatch");

    // Witness executed 60 tests (all passed)
    let witness_long = PrivateExecutionWitness::clean_pass([0x11; 32], [0x22; 32], 60, None);
    let result = air.prove(&inputs, &witness_long);
    assert!(result.is_err(), "prover must reject test count mismatch");
}

// -----------------------------------------------------------------------------
// Test 4: Verifier Invariant Rejection - Memory Safety Flag False
// -----------------------------------------------------------------------------
#[test]
fn test_adversarial_verifier_rejects_memory_safety_flag_false() {
    let air = ZkExecutionAir::new(8).expect("AIR init");
    let inputs = mock_valid_public_inputs();
    let witness = mock_honest_witness(50);
    let mut proof = air.prove(&inputs, &witness).expect("prove valid");

    proof.public_inputs.memory_safety_verified = false;
    let verified = air.verify(&proof).expect("verify call");
    assert!(
        !verified,
        "verifier must reject memory_safety_verified = false"
    );
}

// -----------------------------------------------------------------------------
// Test 5: Verifier Invariant Rejection - Holdout Test Invariants
// -----------------------------------------------------------------------------
#[test]
fn test_adversarial_verifier_rejects_corrupted_holdout_invariants() {
    let air = ZkExecutionAir::new(8).expect("AIR init");
    let inputs = mock_valid_public_inputs();
    let witness = mock_honest_witness(50);
    let proof = air.prove(&inputs, &witness).expect("prove valid");

    // Test count == 0
    let mut p1 = proof.clone();
    p1.public_inputs.holdout_test_count = 0;
    p1.public_inputs.holdout_tests_passed = 0;
    assert!(!air.verify(&p1).unwrap(), "must reject test_count == 0");

    // Tests passed < test count
    let mut p2 = proof.clone();
    p2.public_inputs.holdout_tests_passed = 49;
    assert!(
        !air.verify(&p2).unwrap(),
        "must reject tests_passed < count"
    );

    // Tests passed > test count
    let mut p3 = proof.clone();
    p3.public_inputs.holdout_tests_passed = 51;
    assert!(
        !air.verify(&p3).unwrap(),
        "must reject tests_passed > count"
    );
}

// -----------------------------------------------------------------------------
// Test 6: Fiat-Shamir Challenge Zeta Tampering (50 mutations)
// -----------------------------------------------------------------------------
#[test]
fn test_adversarial_verifier_rejects_fiat_shamir_zeta_tampering() {
    let air = ZkExecutionAir::new(8).expect("AIR init");
    let inputs = mock_valid_public_inputs();
    let witness = mock_honest_witness(50);
    let proof = air.prove(&inputs, &witness).expect("prove valid");

    for delta in 1..=50u64 {
        let mut p = proof.clone();
        p.challenge_zeta = p.challenge_zeta.add(FieldElement::new(delta));
        let verified = air.verify(&p).expect("verify");
        assert!(
            !verified,
            "tampered challenge zeta +{} must be rejected",
            delta
        );
    }

    // Boundary elements: 0, 1, p-1
    for edge in [0, 1, GOLDILOCKS_PRIME - 1] {
        let mut p = proof.clone();
        p.challenge_zeta = FieldElement::new(edge);
        if p.challenge_zeta != proof.challenge_zeta {
            assert!(
                !air.verify(&p).unwrap(),
                "edge zeta {} must be rejected",
                edge
            );
        }
    }
}

// -----------------------------------------------------------------------------
// Test 7: Quotient Polynomial Evaluation Q(zeta) Tampering (50 mutations)
// -----------------------------------------------------------------------------
#[test]
fn test_adversarial_verifier_rejects_quotient_q_tampering() {
    let air = ZkExecutionAir::new(8).expect("AIR init");
    let inputs = mock_valid_public_inputs();
    let witness = mock_honest_witness(50);
    let proof = air.prove(&inputs, &witness).expect("prove valid");

    for delta in 1..=50u64 {
        let mut p = proof.clone();
        p.quotient_q = p.quotient_q.add(FieldElement::new(delta));
        let verified = air.verify(&p).expect("verify");
        assert!(!verified, "tampered quotient Q +{} must be rejected", delta);
    }
}

// -----------------------------------------------------------------------------
// Test 8: Blinded Execution Polynomial Evaluation F(zeta) Tampering (50 mutations)
// -----------------------------------------------------------------------------
#[test]
fn test_adversarial_verifier_rejects_eval_f_zeta_tampering() {
    let air = ZkExecutionAir::new(8).expect("AIR init");
    let inputs = mock_valid_public_inputs();
    let witness = mock_honest_witness(50);
    let proof = air.prove(&inputs, &witness).expect("prove valid");

    for delta in 1..=50u64 {
        let mut p = proof.clone();
        p.eval_f_zeta = p.eval_f_zeta.add(FieldElement::new(delta));
        let verified = air.verify(&p).expect("verify");
        assert!(
            !verified,
            "tampered eval F(zeta) +{} must be rejected",
            delta
        );
    }
}

// -----------------------------------------------------------------------------
// Test 9: Commitment C_F Tampering (50 mutations)
// -----------------------------------------------------------------------------
#[test]
fn test_adversarial_verifier_rejects_commitment_c_tampering() {
    let air = ZkExecutionAir::new(8).expect("AIR init");
    let inputs = mock_valid_public_inputs();
    let witness = mock_honest_witness(50);
    let proof = air.prove(&inputs, &witness).expect("prove valid");

    for delta in 1..=50u64 {
        let mut p = proof.clone();
        p.commitment_c = p.commitment_c.add(FieldElement::new(delta));
        let verified = air.verify(&p).expect("verify");
        assert!(
            !verified,
            "tampered commitment C +{} must be rejected",
            delta
        );
    }
}

// -----------------------------------------------------------------------------
// Test 10: Public Inputs Digest & Root Bit Tampering
// -----------------------------------------------------------------------------
#[test]
fn test_adversarial_verifier_rejects_tampered_hashes() {
    let air = ZkExecutionAir::new(8).expect("AIR init");
    let inputs = mock_valid_public_inputs();
    let witness = mock_honest_witness(50);
    let proof = air.prove(&inputs, &witness).expect("prove valid");

    // All 32 bytes of patch_digest
    for byte_idx in 0..32 {
        let mut p = proof.clone();
        p.public_inputs.patch_digest[byte_idx] ^= 0x01;
        assert!(
            !air.verify(&p).unwrap(),
            "tampered patch_digest byte {} must fail",
            byte_idx
        );
    }

    // All 32 bytes of pre_state_root
    for byte_idx in 0..32 {
        let mut p = proof.clone();
        p.public_inputs.pre_state_root[byte_idx] ^= 0x01;
        assert!(
            !air.verify(&p).unwrap(),
            "tampered pre_state_root byte {} must fail",
            byte_idx
        );
    }

    // All 32 bytes of post_state_root
    for byte_idx in 0..32 {
        let mut p = proof.clone();
        p.public_inputs.post_state_root[byte_idx] ^= 0x01;
        assert!(
            !air.verify(&p).unwrap(),
            "tampered post_state_root byte {} must fail",
            byte_idx
        );
    }
}

// -----------------------------------------------------------------------------
// Test 11: Domain Size & Roots of Unity Mismatch
// -----------------------------------------------------------------------------
#[test]
fn test_adversarial_domain_size_mismatch() {
    let air8 = ZkExecutionAir::new(8).expect("AIR init 8");
    let air16 = ZkExecutionAir::new(16).expect("AIR init 16");

    let inputs = mock_valid_public_inputs();
    let witness = mock_honest_witness(50);
    let proof = air8.prove(&inputs, &witness).expect("prove valid 8");

    // Verify proof on mismatched domain air16
    if let Ok(valid) = air16.verify(&proof) {
        assert!(!valid, "air16 must reject proof generated on domain 8");
    }

    // Notice that ZkExecutionAir::verify uses self.domain_size rather than proof.domain_size.
    // Let's assert this empirical behavior:
    let mut p_tampered_domain = proof.clone();
    p_tampered_domain.domain_size = 16;
    // Verifier uses self.domain_size (8), so it succeeds unless domain_size is checked
    let verified_with_ignored_domain_field = air8.verify(&p_tampered_domain).unwrap();
    // Documenting empirical observation: proof.domain_size is not enforced by verifier
    assert!(verified_with_ignored_domain_field);
}

// -----------------------------------------------------------------------------
// Test 12: Zero-Knowledge Privacy & Blinding Statistical Independence (100 trials)
// -----------------------------------------------------------------------------
#[test]
fn test_zero_knowledge_blinding_statistical_independence() {
    let air = ZkExecutionAir::new(8).expect("AIR init");
    let inputs = mock_valid_public_inputs();

    let trials = 100;
    let mut commitments = HashSet::new();
    let mut challenges = HashSet::new();
    let mut evaluations = HashSet::new();
    let mut quotients = HashSet::new();

    for seed in 1..=trials {
        let r0 = FieldElement::new(
            0x1000_0000_0000_0000u64 ^ (seed as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15u64),
        );
        let r1 = FieldElement::new(
            0x2000_0000_0000_0000u64 ^ (seed as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4Fu64),
        );
        let witness =
            PrivateExecutionWitness::clean_pass([0x77; 32], [0x88; 32], 50, Some(vec![r0, r1]));

        let proof = air.prove(&inputs, &witness).expect("prove blinded trial");
        assert!(air.verify(&proof).expect("verify blinded trial"));

        commitments.insert(proof.commitment_c.0);
        challenges.insert(proof.challenge_zeta.0);
        evaluations.insert(proof.eval_f_zeta.0);
        quotients.insert(proof.quotient_q.0);
    }

    // Zero collision across 100 blinded instances
    assert_eq!(commitments.len(), trials, "all commitments must be unique");
    assert_eq!(challenges.len(), trials, "all challenges must be unique");
    assert_eq!(evaluations.len(), trials, "all evaluations must be unique");
    assert_eq!(quotients.len(), trials, "all quotients must be unique");
}

// -----------------------------------------------------------------------------
// Test 13: Theorem 48 Performance Bounds
// -----------------------------------------------------------------------------
#[test]
fn test_theorem_48_performance_bounds() {
    let air = ZkExecutionAir::new(8).expect("AIR init");
    let inputs = mock_valid_public_inputs();
    let witness = mock_honest_witness(50);

    // Proof size bound: strictly <= 176 bytes
    let proof = air.prove(&inputs, &witness).expect("prove valid");
    assert!(
        proof.proof_size_bytes() <= 176,
        "proof size {} exceeds 176 bytes",
        proof.proof_size_bytes()
    );

    // Prover latency benchmark (100 runs)
    let p_start = Instant::now();
    let p_trials = 100;
    for _ in 0..p_trials {
        let _ = air.prove(&inputs, &witness).unwrap();
    }
    let p_avg_us = p_start.elapsed().as_micros() as f64 / p_trials as f64;
    assert!(
        p_avg_us < 50_000.0,
        "prove latency {} us must be < 50,000 us (Theorem 48 bound)",
        p_avg_us
    );
    assert!(
        p_avg_us < 10_000.0,
        "prove latency {} us satisfies debug profile practical bound",
        p_avg_us
    );

    // Verifier latency benchmark (1,000 runs)
    let v_start = Instant::now();
    let v_trials = 1000;
    for _ in 0..v_trials {
        let valid = air.verify(&proof).unwrap();
        assert!(valid);
    }
    let v_avg_us = v_start.elapsed().as_micros() as f64 / v_trials as f64;
    assert!(
        v_avg_us < 1_000.0,
        "verify latency {} us must be < 1,000 us (Theorem 48 bound)",
        v_avg_us
    );
    assert!(
        v_avg_us < 200.0,
        "verify latency {} us satisfies debug profile practical bound",
        v_avg_us
    );
}

// -----------------------------------------------------------------------------
// Test 14: Adversarial Counterfeit Simulator Probe (Finding & Vulnerability Surface)
// -----------------------------------------------------------------------------
#[test]
fn test_adversarial_counterfeit_simulator_probe() {
    let air = ZkExecutionAir::new(8).unwrap();
    // Suppose an adversary fabricates public inputs claiming 100% test pass
    let forged_inputs = mock_valid_public_inputs();

    // Adversary calculates public polynomial C(X) without any witness
    let c_evals = air.map_public_inputs_to_evaluations(&forged_inputs);
    let c_poly = Polynomial::from_evaluations(&air.domain_points, &c_evals).unwrap();

    // Adversary chooses arbitrary commitment
    let arbitrary_commitment = FieldElement::new(0xDEAD_BEEF_CAFE_0001);

    // Computes Fiat-Shamir challenge zeta
    let mut transcript = FiatShamirTranscript::new(EXECUTION_TRANSCRIPT_DOMAIN);
    transcript.append_hash("patch_digest", &forged_inputs.patch_digest);
    transcript.append_hash("pre_state_root", &forged_inputs.pre_state_root);
    transcript.append_hash("post_state_root", &forged_inputs.post_state_root);
    transcript.append_message("memory_safety", &[0x01]);
    transcript.append_message(
        "test_count",
        &forged_inputs.holdout_test_count.to_le_bytes(),
    );
    transcript.append_message(
        "tests_passed",
        &forged_inputs.holdout_tests_passed.to_le_bytes(),
    );
    transcript.append_element("commitment_c", arbitrary_commitment);
    let challenge_zeta = transcript.challenge_element("zeta");

    // Adversary evaluates C(zeta)
    let c_zeta = c_poly.eval(challenge_zeta);

    // Adversary crafts forged proof by setting eval_f_zeta = c_zeta, quotient_q = 0
    let forged_proof = ZkExecutionProof {
        commitment_c: arbitrary_commitment,
        quotient_q: FieldElement::ZERO,
        eval_f_zeta: c_zeta,
        challenge_zeta,
        public_inputs: forged_inputs,
        domain_size: 8,
    };

    // Empirical observation of verifier behavior
    let accepted = air.verify(&forged_proof).unwrap();
    // This empirically confirms that without an opening proof binding commitment_c to F(zeta),
    // the verifier accepts synthetic proofs where eval_f_zeta = C(zeta) and quotient_q = 0.
    assert!(
        accepted,
        "simulator probe confirms algebraic acceptance of synthetic evaluation"
    );
}
