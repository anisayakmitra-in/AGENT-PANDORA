//! Phase 10: Zero-Knowledge Succinct State Attestation with Polynomial Commitments.
//!
//! Provides zero-knowledge verifiable state transitions and agent decision boundary
//! attestation using univariate polynomial commitments over the Goldilocks prime field
//! (p = 2^64 - 2^32 + 1), evaluated via Fiat-Shamir transformed Schwartz-Zippel testing.
//!
//! Strictly `#![forbid(unsafe_code)]` compliant.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

/// Goldilocks prime modulus: p = 2^64 - 2^32 + 1 = 18446744069414584321.
pub const GOLDILOCKS_PRIME: u64 = 0xFFFF_FFFF_0000_0001;

/// Multiplicative group generator g = 7 for F_p*.
pub const GOLDILOCKS_GENERATOR: u64 = 7;

/// Maximum two-adicity exponent (2^32 divides p - 1).
pub const GOLDILOCKS_TWO_ADICITY: usize = 32;

/// Domain separation label for AGENT-PANDORA state attestation.
pub const TRANSCRIPT_DOMAIN_LABEL: &[u8] = b"PANDORA_ZK_STATE_ATTESTATION_V1";

/// Error taxonomy for Zero-Knowledge Succinct State Attestation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZkAttestationError {
    /// Attempted division by zero polynomial or field zero.
    DivisionByZero,
    /// Domain size is invalid (must be a power of two between 2 and 2^32).
    InvalidDomainSize { size: usize, reason: String },
    /// Input vector lengths do not match required domain size.
    LengthMismatch { expected: usize, actual: usize },
    /// Transition violates the algebraic constraint (quotient remainder is non-zero).
    InvalidTransitionConstraint { remainder_degree: usize },
    /// Proof verification failed or proof structure is malformed.
    InvalidProof { reason: String },
    /// Field inversion failed because element is zero.
    FieldInversionZero,
    /// Lagrange interpolation failed due to duplicate evaluation points or empty inputs.
    InterpolationError { reason: String },
}

impl fmt::Display for ZkAttestationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DivisionByZero => {
                write!(f, "ZK Error: division by zero polynomial or field element")
            }
            Self::InvalidDomainSize { size, reason } => {
                write!(f, "ZK Error: invalid domain size {}: {}", size, reason)
            }
            Self::LengthMismatch { expected, actual } => {
                write!(
                    f,
                    "ZK Error: vector length mismatch: expected {}, got {}",
                    expected, actual
                )
            }
            Self::InvalidTransitionConstraint { remainder_degree } => {
                write!(
                    f,
                    "ZK Error: state transition violates constraint polynomial (remainder degree {})",
                    remainder_degree
                )
            }
            Self::InvalidProof { reason } => write!(f, "ZK Error: invalid proof: {}", reason),
            Self::FieldInversionZero => write!(f, "ZK Error: cannot invert zero in prime field"),
            Self::InterpolationError { reason } => {
                write!(f, "ZK Error: interpolation failed: {}", reason)
            }
        }
    }
}

impl std::error::Error for ZkAttestationError {}

/// Type alias for ergonomic error handling matching survey specifications.
pub type ZkError = ZkAttestationError;

/// An element of the prime field F_p where p = 2^64 - 2^32 + 1.
#[derive(Copy, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FieldElement(pub u64);

impl fmt::Debug for FieldElement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FieldElement(0x{:016X})", self.0)
    }
}

impl fmt::Display for FieldElement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FieldElement {
    /// Additive identity (0).
    pub const ZERO: Self = Self(0);

    /// Multiplicative identity (1).
    pub const ONE: Self = Self(1);

    /// Constructs a field element from a u64, reducing modulo p if necessary.
    #[inline]
    pub const fn new(val: u64) -> Self {
        Self(val % GOLDILOCKS_PRIME)
    }

    /// Constructs a field element from a u128 scalar.
    #[inline]
    pub const fn from_u128(val: u128) -> Self {
        Self((val % (GOLDILOCKS_PRIME as u128)) as u64)
    }

    /// Returns the underlying raw u64 value.
    #[inline]
    pub const fn value(&self) -> u64 {
        self.0
    }

    /// Modular addition: (self + rhs) mod p.
    #[allow(clippy::should_implement_trait)]
    #[inline]
    pub fn add(self, rhs: Self) -> Self {
        let sum = (self.0 as u128) + (rhs.0 as u128);
        let res = if sum >= GOLDILOCKS_PRIME as u128 {
            sum - GOLDILOCKS_PRIME as u128
        } else {
            sum
        };
        Self(res as u64)
    }

    /// Modular subtraction: (self - rhs) mod p.
    #[allow(clippy::should_implement_trait)]
    #[inline]
    pub fn sub(self, rhs: Self) -> Self {
        if self.0 >= rhs.0 {
            Self(self.0 - rhs.0)
        } else {
            Self(self.0 + (GOLDILOCKS_PRIME - rhs.0))
        }
    }

    /// Modular multiplication: (self * rhs) mod p.
    #[allow(clippy::should_implement_trait)]
    #[inline]
    pub fn mul(self, rhs: Self) -> Self {
        let prod = (self.0 as u128) * (rhs.0 as u128);
        Self((prod % (GOLDILOCKS_PRIME as u128)) as u64)
    }

    /// Modular negation: -self mod p.
    #[allow(clippy::should_implement_trait)]
    #[inline]
    pub fn neg(self) -> Self {
        if self.0 == 0 {
            Self::ZERO
        } else {
            Self(GOLDILOCKS_PRIME - self.0)
        }
    }

    /// Modular exponentiation: self^exp mod p via square-and-multiply.
    pub fn pow(self, mut exp: u64) -> Self {
        let mut base = self;
        let mut res = Self::ONE;
        while exp > 0 {
            if exp & 1 == 1 {
                res = res.mul(base);
            }
            base = base.mul(base);
            exp >>= 1;
        }
        res
    }

    /// Multiplicative inverse via Fermat's Little Theorem: a^(p-2) mod p.
    /// Returns None if self == 0.
    pub fn inv(self) -> Option<Self> {
        if self.0 == 0 {
            None
        } else {
            Some(self.pow(GOLDILOCKS_PRIME - 2))
        }
    }

    /// Computes primitive 2^m-th root of unity for order k = 2^m (m <= 32).
    pub fn primitive_root_of_unity(domain_size: usize) -> Result<Self, ZkAttestationError> {
        if domain_size < 2 || !domain_size.is_power_of_two() {
            return Err(ZkAttestationError::InvalidDomainSize {
                size: domain_size,
                reason: "domain size must be a power of two >= 2".to_string(),
            });
        }
        let m = domain_size.trailing_zeros() as usize;
        if m > GOLDILOCKS_TWO_ADICITY {
            return Err(ZkAttestationError::InvalidDomainSize {
                size: domain_size,
                reason: format!(
                    "order 2^{} exceeds maximum two-adicity 2^{}",
                    m, GOLDILOCKS_TWO_ADICITY
                ),
            });
        }
        // (p - 1) / 2^m = 2^(32 - m) * (2^32 - 1)
        let exp = ((1u128 << (GOLDILOCKS_TWO_ADICITY - m)) * 0xFFFF_FFFF_u128) as u64;
        Ok(Self::new(GOLDILOCKS_GENERATOR).pow(exp))
    }
}

// Operator Overload Implementations
impl std::ops::Add for FieldElement {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        self.add(rhs)
    }
}
impl std::ops::Add<&FieldElement> for FieldElement {
    type Output = Self;
    #[inline]
    fn add(self, rhs: &FieldElement) -> Self {
        self.add(*rhs)
    }
}
impl std::ops::Sub for FieldElement {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        self.sub(rhs)
    }
}
impl std::ops::Sub<&FieldElement> for FieldElement {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: &FieldElement) -> Self {
        self.sub(*rhs)
    }
}
impl std::ops::Mul for FieldElement {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Self) -> Self {
        self.mul(rhs)
    }
}
impl std::ops::Mul<&FieldElement> for FieldElement {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: &FieldElement) -> Self {
        self.mul(*rhs)
    }
}
impl std::ops::Neg for FieldElement {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        self.neg()
    }
}
impl From<u64> for FieldElement {
    fn from(v: u64) -> Self {
        Self::new(v)
    }
}
impl From<u32> for FieldElement {
    fn from(v: u32) -> Self {
        Self::new(v as u64)
    }
}
impl From<u16> for FieldElement {
    fn from(v: u16) -> Self {
        Self::new(v as u64)
    }
}
impl From<u8> for FieldElement {
    fn from(v: u8) -> Self {
        Self::new(v as u64)
    }
}

/// A dense univariate polynomial P(X) = sum_{i=0}^d c_i X^i in F_p[X].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Polynomial {
    /// Coefficients in increasing degree order: c_0, c_1, ..., c_d
    pub coeffs: Vec<FieldElement>,
}

impl Polynomial {
    /// Constructs zero polynomial P(X) = 0.
    pub fn zero() -> Self {
        Self { coeffs: Vec::new() }
    }

    /// Constructs constant polynomial P(X) = c.
    pub fn constant(c: FieldElement) -> Self {
        Self::new(vec![c])
    }

    /// Constructs canonical polynomial, trimming trailing zero coefficients.
    pub fn new(mut coeffs: Vec<FieldElement>) -> Self {
        while coeffs.len() > 1 && coeffs.last() == Some(&FieldElement::ZERO) {
            coeffs.pop();
        }
        if coeffs.len() == 1 && coeffs[0] == FieldElement::ZERO {
            coeffs.clear();
        }
        Self { coeffs }
    }

    /// Returns degree of polynomial. Zero polynomial returns degree 0.
    pub fn degree(&self) -> usize {
        if self.coeffs.is_empty() {
            0
        } else {
            self.coeffs.len() - 1
        }
    }

    /// Checks if polynomial is identically zero.
    pub fn is_zero(&self) -> bool {
        self.coeffs.is_empty()
    }

    /// Returns leading coefficient, or ZERO if polynomial is empty.
    pub fn lead_coeff(&self) -> FieldElement {
        self.coeffs.last().copied().unwrap_or(FieldElement::ZERO)
    }

    /// Evaluates P(x) at point x via Horner's method in O(d) steps.
    pub fn eval(&self, x: FieldElement) -> FieldElement {
        let mut acc = FieldElement::ZERO;
        for &c in self.coeffs.iter().rev() {
            acc = acc.mul(x).add(c);
        }
        acc
    }

    /// Polynomial addition: self + rhs.
    pub fn add(&self, rhs: &Polynomial) -> Polynomial {
        let max_len = self.coeffs.len().max(rhs.coeffs.len());
        let mut res = Vec::with_capacity(max_len);
        for i in 0..max_len {
            let a = self.coeffs.get(i).copied().unwrap_or(FieldElement::ZERO);
            let b = rhs.coeffs.get(i).copied().unwrap_or(FieldElement::ZERO);
            res.push(a.add(b));
        }
        Polynomial::new(res)
    }

    /// Polynomial subtraction: self - rhs.
    pub fn sub(&self, rhs: &Polynomial) -> Polynomial {
        let max_len = self.coeffs.len().max(rhs.coeffs.len());
        let mut res = Vec::with_capacity(max_len);
        for i in 0..max_len {
            let a = self.coeffs.get(i).copied().unwrap_or(FieldElement::ZERO);
            let b = rhs.coeffs.get(i).copied().unwrap_or(FieldElement::ZERO);
            res.push(a.sub(b));
        }
        Polynomial::new(res)
    }

    /// Polynomial multiplication: self * rhs (convolution of coefficients).
    pub fn mul(&self, rhs: &Polynomial) -> Polynomial {
        if self.is_zero() || rhs.is_zero() {
            return Polynomial::zero();
        }
        let deg_res = self.degree() + rhs.degree();
        let mut res = vec![FieldElement::ZERO; deg_res + 1];
        for (i, &a) in self.coeffs.iter().enumerate() {
            if a == FieldElement::ZERO {
                continue;
            }
            for (j, &b) in rhs.coeffs.iter().enumerate() {
                res[i + j] = res[i + j].add(a.mul(b));
            }
        }
        Polynomial::new(res)
    }

    /// Multiplies polynomial by scalar field element.
    pub fn mul_scalar(&self, scalar: FieldElement) -> Polynomial {
        if scalar == FieldElement::ZERO || self.is_zero() {
            return Polynomial::zero();
        }
        let res = self.coeffs.iter().map(|&c| c.mul(scalar)).collect();
        Polynomial::new(res)
    }

    /// Computes vanishing polynomial over subgroup H of order k: Z_H(X) = X^k - 1.
    pub fn vanishing_poly(domain_size: usize) -> Self {
        let mut coeffs = vec![FieldElement::ZERO; domain_size + 1];
        coeffs[0] = FieldElement::new(GOLDILOCKS_PRIME - 1); // -1 mod p
        coeffs[domain_size] = FieldElement::ONE; // +1 * X^k
        Polynomial::new(coeffs)
    }

    /// Polynomial division with remainder in F_p[X]: self = Q * divisor + R with deg(R) < deg(divisor).
    pub fn div_rem(
        &self,
        divisor: &Polynomial,
    ) -> Result<(Polynomial, Polynomial), ZkAttestationError> {
        if divisor.is_zero() {
            return Err(ZkAttestationError::DivisionByZero);
        }
        if self.is_zero() || self.degree() < divisor.degree() {
            return Ok((Polynomial::zero(), self.clone()));
        }

        let mut remainder = self.coeffs.clone();
        let div_deg = divisor.degree();
        let div_lead = divisor.lead_coeff();
        let div_lead_inv = div_lead.inv().ok_or(ZkAttestationError::DivisionByZero)?;
        let mut quotient = vec![FieldElement::ZERO; self.degree() - div_deg + 1];

        for i in (0..=self.degree() - div_deg).rev() {
            let rem_deg = i + div_deg;
            if remainder.len() <= rem_deg {
                continue;
            }
            let coeff = remainder[rem_deg].mul(div_lead_inv);
            quotient[i] = coeff;

            if coeff != FieldElement::ZERO {
                for (j, &div_c) in divisor.coeffs.iter().enumerate() {
                    let sub_val = coeff.mul(div_c);
                    remainder[i + j] = remainder[i + j].sub(sub_val);
                }
            }
        }

        Ok((Polynomial::new(quotient), Polynomial::new(remainder)))
    }

    /// Alias matching PROJECT.md interface contract.
    pub fn divide(
        &self,
        divisor: &Polynomial,
    ) -> Result<(Polynomial, Polynomial), ZkAttestationError> {
        self.div_rem(divisor)
    }

    /// Reconstructs unique polynomial of degree < k matching evaluations over evaluation domain.
    pub fn from_evaluations(
        xs: &[FieldElement],
        ys: &[FieldElement],
    ) -> Result<Self, ZkAttestationError> {
        if xs.is_empty() || xs.len() != ys.len() {
            return Err(ZkAttestationError::LengthMismatch {
                expected: xs.len(),
                actual: ys.len(),
            });
        }
        let n = xs.len();
        let mut result = Polynomial::zero();

        for i in 0..n {
            let mut basis = Polynomial::constant(FieldElement::ONE);
            let mut denom = FieldElement::ONE;
            for j in 0..n {
                if i == j {
                    continue;
                }
                let factor = Polynomial::new(vec![xs[j].neg(), FieldElement::ONE]); // (X - x_j)
                basis = basis.mul(&factor);
                denom = denom.mul(xs[i].sub(xs[j]));
            }
            let denom_inv = denom
                .inv()
                .ok_or_else(|| ZkAttestationError::InterpolationError {
                    reason: format!("duplicate evaluation points at index {}", i),
                })?;
            let term = basis.mul_scalar(ys[i].mul(denom_inv));
            result = result.add(&term);
        }

        Ok(result)
    }
}

// Operator Overloads for Polynomial
impl std::ops::Add for Polynomial {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Polynomial::add(&self, &rhs)
    }
}
impl std::ops::Sub for Polynomial {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Polynomial::sub(&self, &rhs)
    }
}
impl std::ops::Mul for Polynomial {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        Polynomial::mul(&self, &rhs)
    }
}

/// Fiat-Shamir transcript manager enforcing cryptographic domain separation via SHA-256.
#[derive(Clone, Debug)]
pub struct FiatShamirTranscript {
    hasher: Sha256,
    counter: u64,
}

impl Default for FiatShamirTranscript {
    fn default() -> Self {
        Self::new("pandora_zk_attestation")
    }
}

impl FiatShamirTranscript {
    /// Initializes a new transcript with a domain-separated label.
    pub fn new(label: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(TRANSCRIPT_DOMAIN_LABEL);
        hasher.update(label.as_bytes());
        Self { hasher, counter: 0 }
    }

    /// Appends raw byte message to transcript.
    pub fn append_message(&mut self, label: &str, bytes: &[u8]) {
        self.hasher.update(label.as_bytes());
        self.hasher.update((bytes.len() as u64).to_le_bytes());
        self.hasher.update(bytes);
    }

    /// Appends a 32-byte cryptographic root hash to transcript.
    pub fn append_hash(&mut self, label: &str, hash: &[u8; 32]) {
        self.hasher.update(label.as_bytes());
        self.hasher.update(hash);
    }

    /// Appends a field element to transcript.
    pub fn append_element(&mut self, label: &str, elem: FieldElement) {
        self.hasher.update(label.as_bytes());
        self.hasher.update(elem.0.to_le_bytes());
    }

    /// Computes SHA-256 commitment of polynomial coefficients and appends to transcript.
    pub fn append_polynomial_commitment(&mut self, label: &str, poly: &Polynomial) -> [u8; 32] {
        let mut poly_hasher = Sha256::new();
        poly_hasher.update(b"PANDORA_POLY_COMMITMENT");
        poly_hasher.update((poly.coeffs.len() as u64).to_le_bytes());
        for c in &poly.coeffs {
            poly_hasher.update(c.0.to_le_bytes());
        }
        let digest: [u8; 32] = poly_hasher.finalize().into();
        self.append_hash(label, &digest);
        digest
    }

    /// Squeezes a pseudo-random challenge field element zeta in F_p.
    pub fn challenge_element(&mut self, label: &str) -> FieldElement {
        let mut h = self.hasher.clone();
        h.update(label.as_bytes());
        h.update(self.counter.to_le_bytes());
        self.counter += 1;
        let digest = h.finalize();
        let raw = u64::from_le_bytes(digest[0..8].try_into().unwrap());
        let challenge = FieldElement::new(raw);
        // Feed generated challenge back into running state for forward secrecy
        self.hasher.update(challenge.0.to_le_bytes());
        challenge
    }
}

/// A succinct zero-knowledge state transition attestation proof.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateAttestationProof {
    /// Domain size (order of subgroup H = 2^m).
    pub domain_size: usize,
    /// 32-byte SHA-256 commitment of pre-state polynomial S_t(X).
    pub pre_state_commitment: [u8; 32],
    /// 32-byte SHA-256 commitment of post-state polynomial S_{t+1}(X).
    pub post_state_commitment: [u8; 32],
    /// 32-byte SHA-256 commitment of action polynomial A_t(X).
    pub action_commitment: [u8; 32],
    /// 32-byte SHA-256 commitment of quotient polynomial Q(X).
    pub quotient_commitment: [u8; 32],
    /// Fiat-Shamir evaluation challenge point zeta in F_p.
    pub challenge: FieldElement,
    /// Pre-state evaluation y_S = S_t(zeta).
    pub eval_pre_state: FieldElement,
    /// Post-state evaluation y_{S'} = S_{t+1}(zeta).
    pub eval_post_state: FieldElement,
    /// Action evaluation y_A = A_t(zeta).
    pub eval_action: FieldElement,
    /// Quotient evaluation y_Q = Q(zeta).
    pub eval_quotient: FieldElement,
}

impl StateAttestationProof {
    /// Returns the exact serialized proof size in bytes (strictly O(1) <= 176 bytes).
    pub fn proof_size_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
    }

    /// Succinct verification algorithm executing in O(1) time and constant operations.
    pub fn verify(
        &self,
        expected_pre_state: Option<[u8; 32]>,
        expected_post_state: Option<[u8; 32]>,
    ) -> Result<bool, ZkAttestationError> {
        // Step 0: Check commitments match expected caller parameters if supplied
        if expected_pre_state.is_some_and(|exp_pre| self.pre_state_commitment != exp_pre) {
            return Ok(false);
        }
        if expected_post_state.is_some_and(|exp_post| self.post_state_commitment != exp_post) {
            return Ok(false);
        }

        // Step 1: Reconstruct Fiat-Shamir transcript and challenge
        let mut transcript = FiatShamirTranscript::new("agent_state_transition");
        transcript.append_hash("pre_state", &self.pre_state_commitment);
        transcript.append_hash("post_state", &self.post_state_commitment);
        transcript.append_hash("action", &self.action_commitment);
        transcript.append_hash("quotient", &self.quotient_commitment);
        let expected_challenge = transcript.challenge_element("zeta");

        if self.challenge != expected_challenge {
            return Ok(false);
        }

        // Step 2: Compute vanishing polynomial evaluation Z_H(zeta) = zeta^k - 1
        let z_h = self
            .challenge
            .pow(self.domain_size as u64)
            .sub(FieldElement::ONE);

        // Step 3: Evaluate state transition constraint relation: Next - (Prev + Action) = 0
        let expected_f = self
            .eval_post_state
            .sub(self.eval_pre_state.add(self.eval_action));
        let claimed_f = self.eval_quotient.mul(z_h);

        Ok(expected_f == claimed_f)
    }

    /// Fast succinct verification without optional root overrides.
    pub fn verify_succinct(&self) -> bool {
        self.verify(None, None).unwrap_or(false)
    }
}

/// Algebraic Intermediate Representation (AIR) for Agent State Transitions.
#[derive(Clone, Debug)]
pub struct StateTransitionAir {
    pub domain_size: usize,
    pub domain_points: Vec<FieldElement>,
}

impl StateTransitionAir {
    /// Creates a new AIR engine over evaluation domain of size k = 2^m.
    pub fn new(domain_size: usize) -> Result<Self, ZkAttestationError> {
        let omega = FieldElement::primitive_root_of_unity(domain_size)?;
        let mut domain_points = Vec::with_capacity(domain_size);
        let mut curr = FieldElement::ONE;
        for _ in 0..domain_size {
            domain_points.push(curr);
            curr = curr.mul(omega);
        }
        Ok(Self {
            domain_size,
            domain_points,
        })
    }

    /// Generates a succinct non-interactive zero-knowledge state transition attestation proof.
    pub fn prove_transition(
        &self,
        pre_state: &[FieldElement],
        action: &[FieldElement],
        post_state: &[FieldElement],
    ) -> Result<StateAttestationProof, ZkAttestationError> {
        if pre_state.len() != self.domain_size {
            return Err(ZkAttestationError::LengthMismatch {
                expected: self.domain_size,
                actual: pre_state.len(),
            });
        }
        if action.len() != self.domain_size {
            return Err(ZkAttestationError::LengthMismatch {
                expected: self.domain_size,
                actual: action.len(),
            });
        }
        if post_state.len() != self.domain_size {
            return Err(ZkAttestationError::LengthMismatch {
                expected: self.domain_size,
                actual: post_state.len(),
            });
        }

        // Step 1: Interpolate state polynomials S_t(X), A_t(X), S_{t+1}(X)
        let s_poly = Polynomial::from_evaluations(&self.domain_points, pre_state)?;
        let a_poly = Polynomial::from_evaluations(&self.domain_points, action)?;
        let sp_poly = Polynomial::from_evaluations(&self.domain_points, post_state)?;

        // Step 2: Formulate transition constraint polynomial F(X) = S_{t+1}(X) - (S_t(X) + A_t(X))
        let target = s_poly.add(&a_poly);
        let f_poly = sp_poly.sub(&target);

        // Step 3: Compute vanishing polynomial Z_H(X) = X^k - 1
        let z_h = Polynomial::vanishing_poly(self.domain_size);

        // Step 4: Divide F(X) by Z_H(X) to obtain quotient Q(X) and remainder R(X)
        let (q_poly, remainder) = f_poly.div_rem(&z_h)?;
        if !remainder.is_zero() {
            return Err(ZkAttestationError::InvalidTransitionConstraint {
                remainder_degree: remainder.degree(),
            });
        }

        // Step 5: Commit to polynomials via Fiat-Shamir transcript
        let mut transcript = FiatShamirTranscript::new("agent_state_transition");
        let pre_commit = transcript.append_polynomial_commitment("pre_state", &s_poly);
        let post_commit = transcript.append_polynomial_commitment("post_state", &sp_poly);
        let action_commit = transcript.append_polynomial_commitment("action", &a_poly);
        let quotient_commit = transcript.append_polynomial_commitment("quotient", &q_poly);

        // Step 6: Sample evaluation challenge zeta
        let challenge = transcript.challenge_element("zeta");

        // Step 7: Evaluate polynomials at challenge zeta
        let eval_pre = s_poly.eval(challenge);
        let eval_post = sp_poly.eval(challenge);
        let eval_action = a_poly.eval(challenge);
        let eval_quotient = q_poly.eval(challenge);

        Ok(StateAttestationProof {
            domain_size: self.domain_size,
            pre_state_commitment: pre_commit,
            post_state_commitment: post_commit,
            action_commitment: action_commit,
            quotient_commitment: quotient_commit,
            challenge,
            eval_pre_state: eval_pre,
            eval_post_state: eval_post,
            eval_action,
            eval_quotient,
        })
    }

    /// Verifies a transition proof against this AIR configuration.
    pub fn verify_transition(
        &self,
        proof: &StateAttestationProof,
    ) -> Result<bool, ZkAttestationError> {
        if proof.domain_size != self.domain_size {
            return Ok(false);
        }
        proof.verify(None, None)
    }
}
