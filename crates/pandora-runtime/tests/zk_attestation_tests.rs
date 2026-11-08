#![forbid(unsafe_code)]

use pandora_runtime::zk_attestation::{
    FieldElement, GOLDILOCKS_PRIME, Polynomial, StateTransitionAir, ZkAttestationError,
};

#[test]
fn test_goldilocks_field_basic_arithmetic() {
    let zero = FieldElement::ZERO;
    let one = FieldElement::ONE;
    let a = FieldElement::new(123456789);
    let b = FieldElement::new(987654321);

    // Identity tests
    assert_eq!(a.add(zero), a);
    assert_eq!(a.mul(one), a);
    assert_eq!(a.sub(a), zero);

    // Commutativity
    assert_eq!(a.add(b), b.add(a));
    assert_eq!(a.mul(b), b.mul(a));

    // Subtraction wrapping near p
    let p_minus_one = FieldElement::new(GOLDILOCKS_PRIME - 1);
    assert_eq!(zero.sub(one), p_minus_one);
    assert_eq!(p_minus_one.add(one), zero);

    // Negation
    assert_eq!(a.add(a.neg()), zero);
}

#[test]
fn test_goldilocks_multiplicative_inverses() {
    let zero = FieldElement::ZERO;
    assert_eq!(zero.inv(), None);

    let test_values = [
        1u64,
        2,
        7,
        42,
        10007,
        123456789,
        0xFFFF_FFFF,
        0x1_0000_0000,
        GOLDILOCKS_PRIME - 2,
        GOLDILOCKS_PRIME - 1,
    ];

    for &v in &test_values {
        let elem = FieldElement::new(v);
        let inv = elem.inv().expect("inversion should succeed for non-zero");
        assert_eq!(elem.mul(inv), FieldElement::ONE, "failed for v = {}", v);
    }
}

#[test]
fn test_primitive_roots_of_unity() {
    for m in 1..=10 {
        let k = 1usize << m;
        let omega = FieldElement::primitive_root_of_unity(k).expect("root of unity generation");

        // omega^k == 1
        assert_eq!(omega.pow(k as u64), FieldElement::ONE);

        // omega^(k/2) == -1 == p - 1
        let half = (k / 2) as u64;
        assert_eq!(omega.pow(half), FieldElement::new(GOLDILOCKS_PRIME - 1));

        // For all 1 <= j < k, omega^j != 1
        let mut curr = omega;
        for j in 1..k {
            assert_ne!(
                curr,
                FieldElement::ONE,
                "order mismatch at power {} for k = {}",
                j,
                k
            );
            curr = curr.mul(omega);
        }
    }
}

#[test]
fn test_polynomial_horner_evaluation() {
    // P(X) = 5 + 3X + 2X^2
    let poly = Polynomial::new(vec![
        FieldElement::new(5),
        FieldElement::new(3),
        FieldElement::new(2),
    ]);

    assert_eq!(poly.eval(FieldElement::ZERO), FieldElement::new(5));
    assert_eq!(poly.eval(FieldElement::ONE), FieldElement::new(10));
    assert_eq!(
        poly.eval(FieldElement::new(2)),
        FieldElement::new(5 + 6 + 8)
    );
}

#[test]
fn test_polynomial_div_rem_exact_and_remainder() {
    // Exact: (X^2 - 1) / (X - 1) = X + 1 with R = 0
    let dividend = Polynomial::new(vec![
        FieldElement::new(GOLDILOCKS_PRIME - 1),
        FieldElement::ZERO,
        FieldElement::ONE,
    ]);
    let divisor = Polynomial::new(vec![
        FieldElement::new(GOLDILOCKS_PRIME - 1),
        FieldElement::ONE,
    ]);

    let (q, r) = dividend.div_rem(&divisor).expect("division succeeds");
    assert!(r.is_zero(), "remainder should be zero");
    assert_eq!(
        q,
        Polynomial::new(vec![FieldElement::ONE, FieldElement::ONE])
    );

    // Non-exact: (X^3 + 2X + 5) / (X^2 + 1)
    let p_num = Polynomial::new(vec![
        FieldElement::new(5),
        FieldElement::new(2),
        FieldElement::ZERO,
        FieldElement::ONE,
    ]);
    let p_den = Polynomial::new(vec![
        FieldElement::ONE,
        FieldElement::ZERO,
        FieldElement::ONE,
    ]);

    let (q2, r2) = p_num.div_rem(&p_den).expect("division succeeds");
    // Verify P_num == Q * P_den + R
    let reconstructed = q2.mul(&p_den).add(&r2);
    assert_eq!(reconstructed, p_num);
}

#[test]
fn test_lagrange_subgroup_interpolation() {
    let k = 4;
    let omega = FieldElement::primitive_root_of_unity(k).unwrap();
    let xs = vec![FieldElement::ONE, omega, omega.pow(2), omega.pow(3)];
    let ys = vec![
        FieldElement::new(10),
        FieldElement::new(20),
        FieldElement::new(30),
        FieldElement::new(40),
    ];

    let poly = Polynomial::from_evaluations(&xs, &ys).expect("interpolation succeeds");
    assert_eq!(poly.degree(), 3);

    for i in 0..k {
        assert_eq!(poly.eval(xs[i]), ys[i], "mismatch at point {}", i);
    }
}

#[test]
fn test_succinct_state_attestation_end_to_end() {
    let domain_size = 8;
    let air = StateTransitionAir::new(domain_size).expect("AIR initialized");

    // Pre-state S_t: 8 state attributes
    let pre_state = vec![
        FieldElement::new(100), // Step counter
        FieldElement::new(500), // Memory hash lo
        FieldElement::new(800), // Memory hash hi
        FieldElement::new(1),   // Policy hash lo
        FieldElement::new(2),   // Policy hash hi
        FieldElement::new(50),  // Consumed budget
        FieldElement::new(7),   // Permission mask
        FieldElement::new(1),   // Governance status
    ];

    // Action A_t
    let action = vec![
        FieldElement::new(1),  // +1 step
        FieldElement::new(25), // +25 mem delta
        FieldElement::new(10), // +10 mem delta
        FieldElement::new(0),  // same policy
        FieldElement::new(0),  // same policy
        FieldElement::new(15), // +15 budget consumed
        FieldElement::new(0),  // same permissions
        FieldElement::new(0),  // same status
    ];

    // Post-state S_{t+1} = S_t + A_t
    let post_state: Vec<FieldElement> = pre_state
        .iter()
        .zip(action.iter())
        .map(|(&s, &a)| s.add(a))
        .collect();

    // 1. Prover generates succinct proof
    let proof = air
        .prove_transition(&pre_state, &action, &post_state)
        .expect("proof generation succeeds");

    // 2. Verify proof size is strictly O(1) <= 176 bytes
    assert!(
        proof.proof_size_bytes() <= 176,
        "proof size {} exceeds 176 bytes",
        proof.proof_size_bytes()
    );

    // 3. Verifier verifies in O(1) time
    let valid = air
        .verify_transition(&proof)
        .expect("verification executes cleanly");
    assert!(valid, "valid transition must verify true");
}

#[test]
fn test_soundness_illegal_transition_rejected() {
    let domain_size = 8;
    let air = StateTransitionAir::new(domain_size).expect("AIR initialized");

    let pre_state = vec![FieldElement::new(10); 8];
    let action = vec![FieldElement::new(5); 8];

    // Illegal post-state: coordinate 3 has corrupted value 999 instead of 15
    let mut post_state = vec![FieldElement::new(15); 8];
    post_state[3] = FieldElement::new(999);

    // Prover must detect that R(X) != 0 and reject
    let result = air.prove_transition(&pre_state, &action, &post_state);
    match result {
        Err(ZkAttestationError::InvalidTransitionConstraint { .. }) => {
            // Correct behavior: prover refuses to attest to illegal state
        }
        other => panic!("expected InvalidTransitionConstraint, got {:?}", other),
    }
}

#[test]
fn test_soundness_forgery_tampered_challenge_fails_verification() {
    let domain_size = 8;
    let air = StateTransitionAir::new(domain_size).expect("AIR initialized");

    let pre_state = vec![FieldElement::new(10); 8];
    let action = vec![FieldElement::new(5); 8];
    let post_state = vec![FieldElement::new(15); 8];

    let mut proof = air
        .prove_transition(&pre_state, &action, &post_state)
        .unwrap();

    // Malicious adversary tampers with challenge point
    proof.challenge = proof.challenge.add(FieldElement::ONE);

    let verified = air.verify_transition(&proof).unwrap();
    assert!(!verified, "tampered challenge must fail verification");
}

#[test]
fn test_soundness_forgery_tampered_quotient_eval_fails_verification() {
    let domain_size = 8;
    let air = StateTransitionAir::new(domain_size).expect("AIR initialized");

    let pre_state = vec![FieldElement::new(10); 8];
    let action = vec![FieldElement::new(5); 8];
    let post_state = vec![FieldElement::new(15); 8];

    let mut proof = air
        .prove_transition(&pre_state, &action, &post_state)
        .unwrap();

    // Adversary forges quotient evaluation
    proof.eval_quotient = proof.eval_quotient.add(FieldElement::ONE);

    let verified = air.verify_transition(&proof).unwrap();
    assert!(
        !verified,
        "tampered quotient evaluation must fail verification"
    );
}
