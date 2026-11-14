#![forbid(unsafe_code)]

//! Phase 25 Zero-Knowledge Execution Attestation & Holdout Verification Tests (Theorem 48).

use pandora_runtime::zk_attestation::{FieldElement, ZkAttestationError};
use pandora_runtime::zk_execution_attestation::{
    PrivateExecutionWitness, PublicExecutionInputs, ZkExecutionAir,
};
use sha2::{Digest, Sha256};
use std::time::Instant;

fn mock_valid_inputs() -> PublicExecutionInputs {
    let patch_digest = Sha256::digest(b"patch-content-v3.0.12").into();
    let pre_state_root = Sha256::digest(b"pre-state-root").into();
    let post_state_root = Sha256::digest(b"post-state-root").into();
    PublicExecutionInputs::new(patch_digest, pre_state_root, post_state_root, true, 50, 50)
}

#[test]
fn test_valid_patch_proof_generation_and_verification() {
    let air = ZkExecutionAir::new(8).expect("AIR initialization should succeed");
    let inputs = mock_valid_inputs();
    let witness = PrivateExecutionWitness::clean_pass([0x11; 32], [0x22; 32], 50, None);

    let proof = air
        .prove(&inputs, &witness)
        .expect("valid execution should prove");
    assert!(proof.proof_size_bytes() <= 176);

    let verified = air.verify(&proof).expect("verification should execute");
    assert!(verified, "valid proof must verify successfully");
}

#[test]
fn test_rejection_on_memory_safety_violation_at_prover() {
    let air = ZkExecutionAir::new(8).unwrap();
    let inputs = mock_valid_inputs();
    // Witness contains 2 memory safety violations
    let witness = PrivateExecutionWitness::with_violations([0x11; 32], [0x22; 32], 2, 50);

    let result = air.prove(&inputs, &witness);
    assert!(
        matches!(
            result,
            Err(ZkAttestationError::InvalidTransitionConstraint { .. })
        ),
        "prover must reject execution with memory safety violations"
    );
}

#[test]
fn test_rejection_on_memory_safety_violation_flag_at_verifier() {
    let air = ZkExecutionAir::new(8).unwrap();
    let inputs = mock_valid_inputs();
    let witness = PrivateExecutionWitness::mock_valid(&inputs);
    let mut proof = air.prove(&inputs, &witness).unwrap();

    // Adversary tampers with memory_safety_verified flag
    proof.public_inputs.memory_safety_verified = false;
    let verified = air.verify(&proof).unwrap();
    assert!(
        !verified,
        "verifier must reject proof with unverified memory safety"
    );
}

#[test]
fn test_rejection_on_failing_holdout_test_at_prover() {
    let air = ZkExecutionAir::new(8).unwrap();
    let inputs = mock_valid_inputs();
    // Witness test suite passes 48/50 tests (2 failures)
    let mut executions = vec![true; 50];
    executions[12] = false;
    executions[37] = false;
    let witness = PrivateExecutionWitness::new([0x11; 32], [0x22; 32], 0, executions, Vec::new());

    let result = air.prove(&inputs, &witness);
    assert!(
        matches!(
            result,
            Err(ZkAttestationError::InvalidTransitionConstraint { .. })
        ),
        "prover must reject execution with failing holdout tests"
    );
}

#[test]
fn test_rejection_on_failing_holdout_test_at_verifier() {
    let air = ZkExecutionAir::new(8).unwrap();
    let inputs = mock_valid_inputs();
    let witness = PrivateExecutionWitness::mock_valid(&inputs);
    let mut proof = air.prove(&inputs, &witness).unwrap();

    // Public inputs modified to show failing tests
    proof.public_inputs.holdout_tests_passed = 48;
    let verified = air.verify(&proof).unwrap();
    assert!(
        !verified,
        "verifier must reject proof where not all holdout tests passed"
    );
}

#[test]
fn test_soundness_rejection_on_tampered_challenge_zeta() {
    let air = ZkExecutionAir::new(8).unwrap();
    let inputs = mock_valid_inputs();
    let witness = PrivateExecutionWitness::mock_valid(&inputs);
    let mut proof = air.prove(&inputs, &witness).unwrap();

    // Adversary tampers with challenge_zeta
    proof.challenge_zeta = proof.challenge_zeta.add(FieldElement::ONE);
    let verified = air.verify(&proof).unwrap();
    assert!(
        !verified,
        "verifier must reject forged/tampered challenge point"
    );
}

#[test]
fn test_soundness_rejection_on_tampered_eval_f_zeta() {
    let air = ZkExecutionAir::new(8).unwrap();
    let inputs = mock_valid_inputs();
    let witness = PrivateExecutionWitness::mock_valid(&inputs);
    let mut proof = air.prove(&inputs, &witness).unwrap();

    // Adversary alters evaluation claim F(zeta)
    proof.eval_f_zeta = proof.eval_f_zeta.add(FieldElement::ONE);
    let verified = air.verify(&proof).unwrap();
    assert!(
        !verified,
        "verifier must reject forged/tampered polynomial evaluation F(zeta)"
    );
}

#[test]
fn test_soundness_rejection_on_tampered_quotient_q() {
    let air = ZkExecutionAir::new(8).unwrap();
    let inputs = mock_valid_inputs();
    let witness = PrivateExecutionWitness::mock_valid(&inputs);
    let mut proof = air.prove(&inputs, &witness).unwrap();

    // Adversary alters quotient evaluation Q(zeta)
    proof.quotient_q = proof.quotient_q.add(FieldElement::ONE);
    let verified = air.verify(&proof).unwrap();
    assert!(
        !verified,
        "verifier must reject forged/tampered quotient evaluation Q(zeta)"
    );
}

#[test]
fn test_soundness_rejection_on_tampered_public_inputs() {
    let air = ZkExecutionAir::new(8).unwrap();
    let inputs = mock_valid_inputs();
    let witness = PrivateExecutionWitness::mock_valid(&inputs);
    let proof = air.prove(&inputs, &witness).unwrap();

    // Tamper with patch_digest
    let mut proof_bad_patch = proof.clone();
    proof_bad_patch.public_inputs.patch_digest[0] ^= 0xFF;
    assert!(!air.verify(&proof_bad_patch).unwrap());

    // Tamper with pre_state_root
    let mut proof_bad_pre = proof.clone();
    proof_bad_pre.public_inputs.pre_state_root[0] ^= 0xFF;
    assert!(!air.verify(&proof_bad_pre).unwrap());

    // Tamper with post_state_root
    let mut proof_bad_post = proof.clone();
    proof_bad_post.public_inputs.post_state_root[0] ^= 0xFF;
    assert!(!air.verify(&proof_bad_post).unwrap());
}

#[test]
fn test_zero_knowledge_privacy_blinding_property() {
    let air = ZkExecutionAir::new(8).unwrap();
    let inputs = mock_valid_inputs();

    // Prove identical execution with two distinct sets of random blinding factors
    let witness1 = PrivateExecutionWitness::clean_pass(
        [0x11; 32],
        [0x22; 32],
        50,
        Some(vec![
            FieldElement::new(0xAAAA_BBBB_CCCC_0001),
            FieldElement::new(0x1111_2222_3333_0002),
        ]),
    );
    let witness2 = PrivateExecutionWitness::clean_pass(
        [0x11; 32],
        [0x22; 32],
        50,
        Some(vec![
            FieldElement::new(0xDEAD_BEEF_0000_0003),
            FieldElement::new(0xCAFE_BABE_0000_0004),
        ]),
    );

    let proof1 = air.prove(&inputs, &witness1).unwrap();
    let proof2 = air.prove(&inputs, &witness2).unwrap();

    // Commitments, challenge evaluations, and quotients must differ due to random polynomial blinding
    assert_ne!(proof1.commitment_c, proof2.commitment_c);
    assert_ne!(proof1.eval_f_zeta, proof2.eval_f_zeta);
    assert_ne!(proof1.quotient_q, proof2.quotient_q);

    // Both proofs must verify completely and independently
    assert!(air.verify(&proof1).unwrap());
    assert!(air.verify(&proof2).unwrap());
}

#[test]
fn test_performance_benchmarks_proof_generation_and_verification() {
    let air = ZkExecutionAir::new(8).unwrap();
    let inputs = mock_valid_inputs();
    let witness = PrivateExecutionWitness::mock_valid(&inputs);

    // Benchmark proof generation across 100 trials
    let gen_start = Instant::now();
    let trials = 100;
    let mut last_proof = None;
    for _ in 0..trials {
        let proof = air.prove(&inputs, &witness).unwrap();
        last_proof = Some(proof);
    }
    let gen_elapsed = gen_start.elapsed();
    let avg_gen_us = gen_elapsed.as_micros() as f64 / trials as f64;
    // Average proof generation time must be < 5ms (5,000 microseconds)
    assert!(
        avg_gen_us < 5_000.0,
        "average prove time {avg_gen_us} us must be < 5000 us"
    );

    // Benchmark proof verification across 1000 trials
    let proof = last_proof.unwrap();
    let verify_start = Instant::now();
    let v_trials = 1000;
    for _ in 0..v_trials {
        let valid = air.verify(&proof).unwrap();
        assert!(valid);
    }
    let verify_elapsed = verify_start.elapsed();
    let avg_verify_us = verify_elapsed.as_micros() as f64 / v_trials as f64;
    // Average proof verification time must be < 50 microseconds
    assert!(
        avg_verify_us < 50.0,
        "average verify time {avg_verify_us} us must be < 50 us"
    );
}
