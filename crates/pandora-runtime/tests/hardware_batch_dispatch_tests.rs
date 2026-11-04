//! Comprehensive Verification of Hardware Batch Compute Vector Dispatch
//!
//! Grounded in mathematical foundations:
//! - Theorem 7: Roofline arithmetic intensity scaling I(B, N) = (2/b) * (B*N)/(B+N)
//! - Amdahl's Law SIMD speedup bounds S(f, p) = 1 / ((1 - f) + f/p)
//! - Integer dot-product overflow protection bound (|sum| <= 2,097,152 < 2^31 - 1)
//! - Numerical equivalence between 8-wide unrolled SIMD and reference scalar calculation

use pandora_runtime::hardware_batch_dispatch::{BatchVectorDispatcher, Precision, RooflineModel};

#[test]
fn test_roofline_arithmetic_intensity_scaling_theorem() {
    // 1. Single query f32 intensity -> 0.25 FLOP/byte
    let i_single_f32 = RooflineModel::calculate_intensity(1, 1000, Precision::F32);
    // (2/4) * (1 * 1000) / (1001) = 0.5 * 1000 / 1001 ~= 0.4995 FLOP/byte for 2 vectors
    // For single query against corpus:
    assert!((i_single_f32 - 0.4995).abs() < 0.01);

    // 2. Single query Int8 intensity -> 1.0 OP/byte
    let i_single_int8 = RooflineModel::calculate_intensity(1, 1000, Precision::Int8);
    // (2/1) * (1 * 1000) / 1001 ~= 1.998 OP/byte
    assert!((i_single_int8 - 1.998).abs() < 0.01);

    // 3. Batched queries B = 32, N = 1000, Int8 -> Intensity scales to ~62.0 OPs/byte!
    let i_batch_int8 = RooflineModel::calculate_intensity(32, 1000, Precision::Int8);
    // (2/1) * (32 * 1000) / (32 + 1000) = 64000 / 1032 = 62.0155 OPs/byte
    assert!((i_batch_int8 - 62.0155).abs() < 0.01);

    // Theorem asymptotic verification: limit N -> inf I(B, N) = 2B / b
    let i_inf_batch_int8 = RooflineModel::calculate_intensity(32, 10_000_000, Precision::Int8);
    let theoretical_limit = (2.0 / 1.0) * 32.0; // 64.0
    assert!((i_inf_batch_int8 - theoretical_limit).abs() < 0.01);
}

#[test]
fn test_amdahl_law_simd_speedup_bounds() {
    let f = 0.88; // 88% vector fraction

    // AVX2 (p = 8 lanes): S = 1 / (0.12 + 0.88/8) = 1 / (0.12 + 0.11) = 1 / 0.23 ~= 4.3478x
    let s_avx2 = RooflineModel::amdahl_speedup(f, 8);
    assert!((s_avx2 - 4.3478).abs() < 0.01);

    // AVX-512 (p = 16 lanes): S = 1 / (0.12 + 0.88/16) = 1 / (0.12 + 0.055) = 1 / 0.175 ~= 5.7143x
    let s_avx512 = RooflineModel::amdahl_speedup(f, 16);
    assert!((s_avx512 - 5.7143).abs() < 0.01);
}

#[test]
fn test_integer_dot_product_extremal_overflow_bounds() {
    // 128-dimensional vector populated with worst-case extremal values: -128
    let vec_extremal_neg: Vec<i8> = vec![-128; 128];

    // Inner product (-128) * (-128) = 16384 * 128 = 2,097,152
    let dot_max = BatchVectorDispatcher::dot_product_int8(&vec_extremal_neg, &vec_extremal_neg);
    assert_eq!(dot_max, 2_097_152);

    // Headroom check: must fit comfortably in signed 32-bit integer
    assert!(dot_max < i32::MAX);
    assert!(dot_max > 0);

    // Mixed signs: -128 * 127 = -16256 * 128 = -2,080,768
    let vec_extremal_pos: Vec<i8> = vec![127; 128];
    let dot_min = BatchVectorDispatcher::dot_product_int8(&vec_extremal_neg, &vec_extremal_pos);
    assert_eq!(dot_min, -2_080_768);
    assert!(dot_min > i32::MIN);
}

#[test]
fn test_unrolled_dot_product_numerical_equivalence() {
    let dim = 128;
    let mut a = Vec::with_capacity(dim);
    let mut b = Vec::with_capacity(dim);

    for i in 0..dim {
        a.push((i as f32) * 0.1);
        b.push(((dim - i) as f32) * 0.2);
    }

    // Reference naive scalar dot product
    let reference_dot: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();

    // 8-lane unrolled dot product
    let unrolled_dot = BatchVectorDispatcher::dot_product_f32(&a, &b);

    // Floating point summation order causes negligible roundoff: relative diff < 1e-4
    let relative_diff = (reference_dot - unrolled_dot).abs() / reference_dot.abs();
    assert!(
        relative_diff < 1e-4,
        "Unrolled dot product must match scalar reference: rel diff was {relative_diff}"
    );
}

#[test]
fn test_batched_matrix_cosine_similarity_and_top_k() {
    // Batch of B = 2 queries against N = 3 corpus vectors
    let q1 = vec![1.0, 0.0, 0.0, 0.0];
    let q2 = vec![0.0, 1.0, 0.0, 0.0];
    let queries = vec![q1, q2];

    let inv_sqrt2 = std::f32::consts::FRAC_1_SQRT_2;
    let c1 = vec![1.0, 0.0, 0.0, 0.0]; // Exactly parallel to q1 (sim = 1.0)
    let c2 = vec![inv_sqrt2, inv_sqrt2, 0.0, 0.0]; // 45 deg to both q1, q2 (sim = ~0.707)
    let c3 = vec![0.0, 0.0, 1.0, 0.0]; // Orthogonal to q1, q2 (sim = 0.0)
    let corpus = vec![c1, c2, c3];

    let sim_matrix = BatchVectorDispatcher::batch_cosine_similarity_matrix(&queries, &corpus);

    assert_eq!(sim_matrix.len(), 2);
    assert_eq!(sim_matrix[0].len(), 3);
    assert_eq!(sim_matrix[1].len(), 3);

    // Row 0 (query 1):
    assert!((sim_matrix[0][0] - 1.0).abs() < 1e-3);
    assert!((sim_matrix[0][1] - inv_sqrt2).abs() < 1e-3);
    assert!((sim_matrix[0][2] - 0.0).abs() < 1e-3);

    // Top-K for Query 1
    let top2_q1 = BatchVectorDispatcher::top_k(&sim_matrix[0], 2);
    assert_eq!(top2_q1.len(), 2);
    assert_eq!(top2_q1[0].0, 0); // c1 ranked first
    assert_eq!(top2_q1[1].0, 1); // c2 ranked second
}
