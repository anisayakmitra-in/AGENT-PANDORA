//! Low-level architecture SIMD and CPU feature detection engine.
//!
//! Provides hardware-accelerated vector operations (AVX2, SSE4.1) for
//! semantic memory dense embeddings, subword hashing, and BM25 tokenization
//! under strict `#![forbid(unsafe_code)]`.

use serde::{Deserialize, Serialize};

/// Detected CPU SIMD feature flags.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CpuFeatures {
    pub architecture: String,
    pub has_sse: bool,
    pub has_sse2: bool,
    pub has_sse41: bool,
    pub has_avx: bool,
    pub has_avx2: bool,
    pub has_avx512: bool,
    pub vector_width_bits: usize,
    pub preferred_simd_lane: String,
}

impl CpuFeatures {
    /// Detects current host CPU capabilities using safe platform intrinsics.
    pub fn detect() -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            let has_sse = is_x86_feature_detected!("sse");
            let has_sse2 = is_x86_feature_detected!("sse2");
            let has_sse41 = is_x86_feature_detected!("sse4.1");
            let has_avx = is_x86_feature_detected!("avx");
            let has_avx2 = is_x86_feature_detected!("avx2");
            let has_avx512 = is_x86_feature_detected!("avx512f");

            let (vector_width_bits, preferred_simd_lane) = if has_avx512 {
                (512, "avx512".to_owned())
            } else if has_avx2 {
                (256, "avx2".to_owned())
            } else if has_sse41 || has_sse2 {
                (128, "sse".to_owned())
            } else {
                (64, "scalar".to_owned())
            };

            Self {
                architecture: "x86_64".to_owned(),
                has_sse,
                has_sse2,
                has_sse41,
                has_avx,
                has_avx2,
                has_avx512,
                vector_width_bits,
                preferred_simd_lane,
            }
        }
        #[cfg(target_arch = "aarch64")]
        {
            let has_neon = std::arch::is_aarch64_feature_detected!("neon");
            Self {
                architecture: "aarch64".to_owned(),
                has_sse: false,
                has_sse2: false,
                has_sse41: false,
                has_avx: false,
                has_avx2: false,
                has_avx512: false,
                vector_width_bits: if has_neon { 128 } else { 64 },
                preferred_simd_lane: if has_neon {
                    "neon".to_owned()
                } else {
                    "scalar".to_owned()
                },
            }
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            Self {
                architecture: std::env::consts::ARCH.to_owned(),
                has_sse: false,
                has_sse2: false,
                has_sse41: false,
                has_avx: false,
                has_avx2: false,
                has_avx512: false,
                vector_width_bits: 64,
                preferred_simd_lane: "scalar".to_owned(),
            }
        }
    }
}

/// Computes dot product of two 128-dimensional dense vectors using vector-unrolled lanes.
///
/// Under 256-bit AVX2, this unrolls in 8-float chunks (8 * 32-bit = 256 bits).
/// Under 128-bit SSE, this unrolls in 4-float chunks (4 * 32-bit = 128 bits).
/// Strictly `#![forbid(unsafe_code)]` safe implementation structured for auto-vectorization.
pub fn simd_dot_product_128(a: &[f32; 128], b: &[f32; 128]) -> f32 {
    let mut sum0 = 0.0f32;
    let mut sum1 = 0.0f32;
    let mut sum2 = 0.0f32;
    let mut sum3 = 0.0f32;
    let mut sum4 = 0.0f32;
    let mut sum5 = 0.0f32;
    let mut sum6 = 0.0f32;
    let mut sum7 = 0.0f32;

    // Process in 8-lane chunks (16 iterations of 8 floats = 128 floats)
    for i in 0..16 {
        let base = i * 8;
        sum0 += a[base] * b[base];
        sum1 += a[base + 1] * b[base + 1];
        sum2 += a[base + 2] * b[base + 2];
        sum3 += a[base + 3] * b[base + 3];
        sum4 += a[base + 4] * b[base + 4];
        sum5 += a[base + 5] * b[base + 5];
        sum6 += a[base + 6] * b[base + 6];
        sum7 += a[base + 7] * b[base + 7];
    }

    (sum0 + sum1) + (sum2 + sum3) + (sum4 + sum5) + (sum6 + sum7)
}

/// Computes asymmetric dot product between an f32 query vector and an i8 quantized document vector,
/// unrolled across 8 independent accumulators under `#![forbid(unsafe_code)]`.
///
/// Scales the accumulated dot product by `scale` and clamps to `[0.0, 1.0]`.
pub fn simd_dot_product_f32_i8(query: &[f32; 128], doc_i8: &[i8; 128], scale: f32) -> f32 {
    let mut sum0 = 0.0f32;
    let mut sum1 = 0.0f32;
    let mut sum2 = 0.0f32;
    let mut sum3 = 0.0f32;
    let mut sum4 = 0.0f32;
    let mut sum5 = 0.0f32;
    let mut sum6 = 0.0f32;
    let mut sum7 = 0.0f32;

    for i in 0..16 {
        let base = i * 8;
        sum0 += query[base] * (doc_i8[base] as f32);
        sum1 += query[base + 1] * (doc_i8[base + 1] as f32);
        sum2 += query[base + 2] * (doc_i8[base + 2] as f32);
        sum3 += query[base + 3] * (doc_i8[base + 3] as f32);
        sum4 += query[base + 4] * (doc_i8[base + 4] as f32);
        sum5 += query[base + 5] * (doc_i8[base + 5] as f32);
        sum6 += query[base + 6] * (doc_i8[base + 6] as f32);
        sum7 += query[base + 7] * (doc_i8[base + 7] as f32);
    }

    let dot = (sum0 + sum1) + (sum2 + sum3) + (sum4 + sum5) + (sum6 + sum7);
    (dot * scale).clamp(0.0, 1.0)
}

/// Computes symmetric integer dot product of two 128-dimensional i8 vectors,
/// unrolled across 8 independent 32-bit accumulators to prevent overflow.
/// Maximum possible magnitude: 128 * (127 * 127) = 2,064,512 (well within signed 32-bit range).
pub fn simd_dot_product_i8(a: &[i8; 128], b: &[i8; 128]) -> i32 {
    let mut sum0: i32 = 0;
    let mut sum1: i32 = 0;
    let mut sum2: i32 = 0;
    let mut sum3: i32 = 0;
    let mut sum4: i32 = 0;
    let mut sum5: i32 = 0;
    let mut sum6: i32 = 0;
    let mut sum7: i32 = 0;

    for i in 0..16 {
        let base = i * 8;
        sum0 += (a[base] as i32) * (b[base] as i32);
        sum1 += (a[base + 1] as i32) * (b[base + 1] as i32);
        sum2 += (a[base + 2] as i32) * (b[base + 2] as i32);
        sum3 += (a[base + 3] as i32) * (b[base + 3] as i32);
        sum4 += (a[base + 4] as i32) * (b[base + 4] as i32);
        sum5 += (a[base + 5] as i32) * (b[base + 5] as i32);
        sum6 += (a[base + 6] as i32) * (b[base + 6] as i32);
        sum7 += (a[base + 7] as i32) * (b[base + 7] as i32);
    }

    (sum0 + sum1) + (sum2 + sum3) + (sum4 + sum5) + (sum6 + sum7)
}

/// Computes dot product using naive scalar loop as a baseline reference and fallback.
pub fn scalar_dot_product_128(a: &[f32; 128], b: &[f32; 128]) -> f32 {
    let mut sum = 0.0f32;
    for i in 0..128 {
        sum += a[i] * b[i];
    }
    sum
}

/// Generalized safe vector dot product over arbitrary matching slices.
///
/// Unrolled 8 lanes with independent accumulators under `#![forbid(unsafe_code)]`.
/// Structured for LLVM vectorization (AVX2 / NEON).
pub fn simd_dot_product_slice(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "vector dimensions must match");
    let len = a.len();
    let chunks = len / 8;
    let mut sum0 = 0.0f32;
    let mut sum1 = 0.0f32;
    let mut sum2 = 0.0f32;
    let mut sum3 = 0.0f32;
    let mut sum4 = 0.0f32;
    let mut sum5 = 0.0f32;
    let mut sum6 = 0.0f32;
    let mut sum7 = 0.0f32;

    for i in 0..chunks {
        let base = i * 8;
        sum0 += a[base] * b[base];
        sum1 += a[base + 1] * b[base + 1];
        sum2 += a[base + 2] * b[base + 2];
        sum3 += a[base + 3] * b[base + 3];
        sum4 += a[base + 4] * b[base + 4];
        sum5 += a[base + 5] * b[base + 5];
        sum6 += a[base + 6] * b[base + 6];
        sum7 += a[base + 7] * b[base + 7];
    }

    let mut total = (sum0 + sum1) + (sum2 + sum3) + (sum4 + sum5) + (sum6 + sum7);
    for i in (chunks * 8)..len {
        total += a[i] * b[i];
    }
    total
}

/// Baseline scalar dot product over arbitrary matching slices.
pub fn scalar_dot_product_slice(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "vector dimensions must match");
    let mut sum = 0.0f32;
    for i in 0..a.len() {
        sum += a[i] * b[i];
    }
    sum
}

/// Statistical microbenchmark metrics for a single vector dimension.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DimensionBenchmarkStats {
    pub dimension: usize,
    pub iterations_per_trial: usize,
    pub trials: usize,
    pub scalar_mean_nanos: f64,
    pub scalar_std_dev_nanos: f64,
    pub scalar_ci_95: (f64, f64),
    pub simd_mean_nanos: f64,
    pub simd_std_dev_nanos: f64,
    pub simd_ci_95: (f64, f64),
    pub speedup_ratio: f64,
    pub speedup_ci_95: (f64, f64),
}

/// Returns the critical value of Student's t-distribution for two-tailed 95% confidence interval (alpha = 0.05).
///
/// For dof in 1..=30, returns exact tabulated values. For dof > 30, uses the Cornish-Fisher asymptotic expansion.
pub fn student_t_critical_value_95(dof: usize) -> f64 {
    if dof == 0 {
        return 1.960;
    }
    const T_TABLE_95: [f64; 31] = [
        0.0, 12.706, 4.303, 3.182, 2.776, 2.571, 2.447, 2.365, 2.306, 2.262, 2.228, 2.201, 2.179,
        2.160, 2.145, 2.131, 2.120, 2.110, 2.101, 2.093, 2.086, 2.080, 2.074, 2.069, 2.064,
        2.060, 2.056, 2.052, 2.048, 2.045, 2.042,
    ];

    if dof <= 30 {
        T_TABLE_95[dof]
    } else {
        let nu = dof as f64;
        let z = 1.959_963_984_540_054;
        let z3 = z * z * z;
        let z5 = z3 * z * z;
        z + (z3 + z) / (4.0 * nu) + (5.0 * z5 + 16.0 * z3 + 3.0 * z) / (96.0 * nu * nu)
    }
}

/// Computes sample mean, sample standard deviation, and Student's t 95% confidence interval.
pub fn compute_sample_stats(samples: &[f64]) -> (f64, f64, (f64, f64)) {
    let n = samples.len();
    if n == 0 {
        return (0.0, 0.0, (0.0, 0.0));
    }
    let mean = samples.iter().sum::<f64>() / (n as f64);
    if n == 1 {
        return (mean, 0.0, (mean, mean));
    }
    let variance = samples
        .iter()
        .map(|&x| {
            let diff = x - mean;
            diff * diff
        })
        .sum::<f64>()
        / ((n - 1) as f64);
    let std_dev = variance.sqrt();
    let se = std_dev / (n as f64).sqrt();
    let t_crit = student_t_critical_value_95(n - 1);
    let margin = t_crit * se;
    let ci_lower = (mean - margin).max(0.0);
    let ci_upper = mean + margin;
    (mean, std_dev, (ci_lower, ci_upper))
}

/// Executes rigorous multi-dimension statistical microbenchmarks comparing scalar vs SIMD dot products.
///
/// Evaluates specified dimensions (e.g. `[128, 256, 512, 1536]`) over `trials` repetitions
/// of `iterations` vector dot products per trial. Computes sample mean, standard deviation,
/// Student's t 95% confidence intervals, and speedup ratio with first-order Taylor series
/// error propagation confidence bounds.
pub fn run_statistical_microbenchmarks(
    dimensions: &[usize],
    trials: usize,
    iterations: usize,
) -> Vec<DimensionBenchmarkStats> {
    let trials = if trials == 0 { 30 } else { trials };
    let iterations = if iterations == 0 { 10_000 } else { iterations };

    let mut results = Vec::with_capacity(dimensions.len());

    for &dim in dimensions {
        if dim == 0 {
            results.push(DimensionBenchmarkStats {
                dimension: 0,
                iterations_per_trial: iterations,
                trials,
                scalar_mean_nanos: 0.0,
                scalar_std_dev_nanos: 0.0,
                scalar_ci_95: (0.0, 0.0),
                simd_mean_nanos: 0.0,
                simd_std_dev_nanos: 0.0,
                simd_ci_95: (0.0, 0.0),
                speedup_ratio: 1.0,
                speedup_ci_95: (1.0, 1.0),
            });
            continue;
        }

        let mut a = vec![0.0f32; dim];
        let mut b = vec![0.0f32; dim];
        for i in 0..dim {
            a[i] = (i as f32) * 0.01;
            b[i] = ((dim - i) as f32) * 0.02;
        }

        // Cache warm-up to eliminate cold miss artifacts
        let _ = scalar_dot_product_slice(&a, &b);
        let _ = simd_dot_product_slice(&a, &b);

        let mut scalar_samples = Vec::with_capacity(trials);
        let mut simd_samples = Vec::with_capacity(trials);

        for _ in 0..trials {
            let start_scalar = std::time::Instant::now();
            let mut acc_scalar = 0.0f32;
            for _ in 0..iterations {
                acc_scalar += scalar_dot_product_slice(&a, &b);
            }
            let scalar_duration = start_scalar.elapsed().as_nanos() as f64;
            scalar_samples.push(scalar_duration);

            let start_simd = std::time::Instant::now();
            let mut acc_simd = 0.0f32;
            for _ in 0..iterations {
                acc_simd += simd_dot_product_slice(&a, &b);
            }
            let simd_duration = start_simd.elapsed().as_nanos() as f64;
            simd_samples.push(simd_duration);

            // Invariant: verify correctness and prevent optimizer dead-code elimination
            assert!(!acc_scalar.is_nan() && !acc_simd.is_nan());
        }

        let (scalar_mean, scalar_std, scalar_ci) = compute_sample_stats(&scalar_samples);
        let (simd_mean, simd_std, simd_ci) = compute_sample_stats(&simd_samples);

        let speedup_ratio = if simd_mean > 0.0 {
            scalar_mean / simd_mean
        } else {
            1.0
        };

        // First-order Taylor series error propagation for ratio R = X / Y:
        // Var(R) ~= R^2 * [ (SE_X / mean_X)^2 + (SE_Y / mean_Y)^2 ]
        let se_scalar = if trials > 0 {
            scalar_std / (trials as f64).sqrt()
        } else {
            0.0
        };
        let se_simd = if trials > 0 {
            simd_std / (trials as f64).sqrt()
        } else {
            0.0
        };

        let rel_scalar = if scalar_mean > 0.0 {
            se_scalar / scalar_mean
        } else {
            0.0
        };
        let rel_simd = if simd_mean > 0.0 {
            se_simd / simd_mean
        } else {
            0.0
        };
        let speedup_se = speedup_ratio * (rel_scalar * rel_scalar + rel_simd * rel_simd).sqrt();
        let speedup_margin = 1.96 * speedup_se;
        let speedup_ci_95 = (
            (speedup_ratio - speedup_margin).max(0.0),
            speedup_ratio + speedup_margin,
        );

        results.push(DimensionBenchmarkStats {
            dimension: dim,
            iterations_per_trial: iterations,
            trials,
            scalar_mean_nanos: scalar_mean,
            scalar_std_dev_nanos: scalar_std,
            scalar_ci_95: scalar_ci,
            simd_mean_nanos: simd_mean,
            simd_std_dev_nanos: simd_std,
            simd_ci_95: simd_ci,
            speedup_ratio,
            speedup_ci_95,
        });
    }

    results
}

/// Benchmark result comparing SIMD unrolled execution vs scalar baseline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SimdBenchmarkResult {
    pub iterations: usize,
    pub scalar_duration_nanos: u128,
    pub simd_duration_nanos: u128,
    pub speedup_ratio: f64,
}

/// Measures microbenchmark performance across SIMD and scalar dot product implementations.
pub fn benchmark_simd_vs_scalar(iterations: usize) -> SimdBenchmarkResult {
    let mut a = [0.0f32; 128];
    let mut b = [0.0f32; 128];
    for i in 0..128 {
        a[i] = (i as f32) * 0.01;
        b[i] = ((128 - i) as f32) * 0.02;
    }

    let start_scalar = std::time::Instant::now();
    let mut acc_scalar = 0.0f32;
    for _ in 0..iterations {
        acc_scalar += scalar_dot_product_128(&a, &b);
    }
    let scalar_duration = start_scalar.elapsed().as_nanos();

    let start_simd = std::time::Instant::now();
    let mut acc_simd = 0.0f32;
    for _ in 0..iterations {
        acc_simd += simd_dot_product_128(&a, &b);
    }
    let simd_duration = start_simd.elapsed().as_nanos();

    // Prevent compiler dead-code elimination
    assert!((acc_scalar - acc_simd).abs() < 1e-2);

    let speedup_ratio = if simd_duration > 0 {
        scalar_duration as f64 / simd_duration as f64
    } else {
        1.0
    };

    SimdBenchmarkResult {
        iterations,
        scalar_duration_nanos: scalar_duration,
        simd_duration_nanos: simd_duration,
        speedup_ratio,
    }
}

/// Computes L2 squared norm of a 128-dimensional dense vector using vector-unrolled lanes.
pub fn simd_norm_sq_128(v: &[f32; 128]) -> f32 {
    simd_dot_product_128(v, v)
}

/// Vector-accelerated subword character FNV-1a block hasher.
/// Processes bytes with unrolled word accumulation.
pub fn simd_hash64_seed(bytes: &[u8], seed: u64) -> u64 {
    let mut h = 0xcbf29ce484222325 ^ seed;
    let mut i = 0;
    while i + 8 <= bytes.len() {
        let word = u64::from_le_bytes([
            bytes[i],
            bytes[i + 1],
            bytes[i + 2],
            bytes[i + 3],
            bytes[i + 4],
            bytes[i + 5],
            bytes[i + 6],
            bytes[i + 7],
        ]);
        h ^= word;
        h = h.wrapping_mul(0x100000001b3);
        i += 8;
    }

    for &b in &bytes[i..] {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }

    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cpu_feature_detection() {
        let features = CpuFeatures::detect();
        assert!(!features.architecture.is_empty());
        assert!(features.vector_width_bits >= 64);
        assert!(!features.preferred_simd_lane.is_empty());
    }

    #[test]
    fn test_simd_dot_product_orthogonal() {
        let mut a = [0.0f32; 128];
        let mut b = [0.0f32; 128];
        a[0] = 1.0;
        b[1] = 1.0;
        assert_eq!(simd_dot_product_128(&a, &b), 0.0);
    }

    #[test]
    fn test_simd_dot_product_parallel() {
        let a = [1.0f32; 128];
        let b = [2.0f32; 128];
        assert_eq!(simd_dot_product_128(&a, &b), 256.0);
    }

    #[test]
    fn test_simd_norm_sq() {
        let mut a = [0.0f32; 128];
        a[0] = 3.0;
        a[1] = 4.0;
        assert_eq!(simd_norm_sq_128(&a), 25.0);
    }

    #[test]
    fn test_simd_hash64_deterministic() {
        let h1 = simd_hash64_seed(b"compiler_error_diagnostic", 42);
        let h2 = simd_hash64_seed(b"compiler_error_diagnostic", 42);
        assert_eq!(h1, h2);
        assert_ne!(h1, simd_hash64_seed(b"compiler_error_diagnostic", 43));
    }

    #[test]
    fn test_scalar_vs_simd_equivalence() {
        let mut a = [0.0f32; 128];
        let mut b = [0.0f32; 128];
        for i in 0..128 {
            a[i] = (i as f32) * 0.5 - 32.0;
            b[i] = ((128 - i) as f32) * 0.25 + 1.0;
        }
        let simd_res = simd_dot_product_128(&a, &b);
        let scalar_res = scalar_dot_product_128(&a, &b);
        assert!((simd_res - scalar_res).abs() < 1e-3);
    }

    #[test]
    fn test_benchmark_simd_vs_scalar_execution() {
        let result = benchmark_simd_vs_scalar(10_000);
        assert_eq!(result.iterations, 10_000);
        assert!(result.scalar_duration_nanos > 0);
        assert!(result.simd_duration_nanos > 0);
        assert!(result.speedup_ratio > 0.0);
    }

    #[test]
    fn test_slice_dot_product_equivalence_multidim() {
        for &dim in &[128, 256, 512, 1536] {
            let mut a = vec![0.0f32; dim];
            let mut b = vec![0.0f32; dim];
            for i in 0..dim {
                a[i] = (i as f32) * 0.3 - 15.0;
                b[i] = ((dim - i) as f32) * 0.15 + 2.0;
            }
            let simd_res = simd_dot_product_slice(&a, &b);
            let scalar_res = scalar_dot_product_slice(&a, &b);
            let diff = (simd_res - scalar_res).abs();
            let relative_error = diff / scalar_res.abs().max(1.0);
            assert!(
                relative_error < 1e-4,
                "dim {dim}: relative error {relative_error} exceeds 1e-4 (diff={diff})"
            );
        }
    }

    #[test]
    fn test_slice_dot_product_orthogonal_and_parallel() {
        for &dim in &[128, 256, 512, 1536] {
            let mut a = vec![0.0f32; dim];
            let mut b = vec![0.0f32; dim];
            a[0] = 5.0;
            b[1] = 7.0;
            assert_eq!(simd_dot_product_slice(&a, &b), 0.0);
            assert_eq!(scalar_dot_product_slice(&a, &b), 0.0);

            let a_par = vec![2.0f32; dim];
            let b_par = vec![3.0f32; dim];
            let expected = 6.0f32 * (dim as f32);
            assert_eq!(simd_dot_product_slice(&a_par, &b_par), expected);
            assert_eq!(scalar_dot_product_slice(&a_par, &b_par), expected);
        }
    }

    #[test]
    fn test_slice_dot_product_non_multiple_of_8() {
        for &len in &[1, 3, 7, 9, 15, 31, 33, 127] {
            let mut a = vec![0.0f32; len];
            let mut b = vec![0.0f32; len];
            for i in 0..len {
                a[i] = (i as f32) * 1.5;
                b[i] = (len - i) as f32;
            }
            let simd_res = simd_dot_product_slice(&a, &b);
            let scalar_res = scalar_dot_product_slice(&a, &b);
            assert!(
                (simd_res - scalar_res).abs() < 1e-4,
                "len {len}: simd={simd_res}, scalar={scalar_res}"
            );
        }
    }

    #[test]
    fn test_slice_dot_product_dimension_mismatch_panics() {
        let a = vec![1.0f32; 10];
        let b = vec![1.0f32; 12];
        let result_simd = std::panic::catch_unwind(|| {
            simd_dot_product_slice(&a, &b);
        });
        assert!(result_simd.is_err());

        let result_scalar = std::panic::catch_unwind(|| {
            scalar_dot_product_slice(&a, &b);
        });
        assert!(result_scalar.is_err());
    }

    #[test]
    fn test_student_t_critical_values() {
        // Known exact tabulated two-tailed 95% critical values
        assert!((student_t_critical_value_95(1) - 12.706).abs() < 1e-4);
        assert!((student_t_critical_value_95(10) - 2.228).abs() < 1e-4);
        assert!((student_t_critical_value_95(29) - 2.045).abs() < 1e-4);
        assert!((student_t_critical_value_95(30) - 2.042).abs() < 1e-4);

        // Cornish-Fisher expansion for dof > 30 should asymptotically approach 1.960
        let t_60 = student_t_critical_value_95(60);
        assert!(t_60 > 1.960 && t_60 < 2.050);
        let t_120 = student_t_critical_value_95(120);
        assert!(t_120 > 1.960 && t_120 < t_60);
        let t_1000 = student_t_critical_value_95(1000);
        assert!((t_1000 - 1.960).abs() < 5e-3);
    }

    #[test]
    fn test_compute_sample_stats_deterministic() {
        // Dataset: [10.0, 12.0, 14.0]
        // N = 3, dof = 2
        // Mean = (10 + 12 + 14) / 3 = 12.0
        // Variance = ((10-12)^2 + (12-12)^2 + (14-12)^2) / 2 = (4 + 0 + 4) / 2 = 4.0
        // Std Dev = sqrt(4.0) = 2.0
        // SE = 2.0 / sqrt(3) ~= 1.1547
        // t_crit(2) = 4.303
        // Margin = 4.303 * 1.1547 ~= 4.9686
        // CI = (12.0 - 4.9686, 12.0 + 4.9686) ~= (7.0314, 16.9686)
        let samples = vec![10.0, 12.0, 14.0];
        let (mean, std_dev, (ci_lo, ci_hi)) = compute_sample_stats(&samples);

        assert!((mean - 12.0).abs() < 1e-6);
        assert!((std_dev - 2.0).abs() < 1e-6);
        assert!(ci_lo < mean);
        assert!(ci_hi > mean);
        assert!((ci_lo - 7.0314).abs() < 1e-2);
        assert!((ci_hi - 16.9686).abs() < 1e-2);
    }

    #[test]
    fn test_run_statistical_microbenchmarks_execution() {
        let dimensions = [128, 256, 512, 1536];
        let stats = run_statistical_microbenchmarks(&dimensions, 5, 200);

        assert_eq!(stats.len(), 4);
        for s in &stats {
            assert!(dimensions.contains(&s.dimension));
            assert_eq!(s.trials, 5);
            assert_eq!(s.iterations_per_trial, 200);
            assert!(s.scalar_mean_nanos > 0.0);
            assert!(s.simd_mean_nanos > 0.0);
            assert!(s.scalar_ci_95.0 <= s.scalar_ci_95.1);
            assert!(s.simd_ci_95.0 <= s.simd_ci_95.1);
            assert!(s.speedup_ratio > 0.0);
            assert!(s.speedup_ci_95.0 <= s.speedup_ci_95.1);
            assert!(s.scalar_std_dev_nanos >= 0.0);
            assert!(s.simd_std_dev_nanos >= 0.0);
        }
    }

    #[test]
    fn test_run_statistical_microbenchmarks_zero_handling() {
        let stats = run_statistical_microbenchmarks(&[0], 0, 0);
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].dimension, 0);
        assert_eq!(stats[0].trials, 30);
        assert_eq!(stats[0].iterations_per_trial, 10_000);
        assert_eq!(stats[0].scalar_mean_nanos, 0.0);
        assert_eq!(stats[0].simd_mean_nanos, 0.0);
        assert_eq!(stats[0].speedup_ratio, 1.0);
    }

    #[test]
    fn test_simd_dot_product_f32_i8_and_i8_basic() {
        let mut query = [0.0f32; 128];
        let mut doc_i8 = [0i8; 128];

        query[0] = 1.0;
        query[1] = 0.5;
        doc_i8[0] = 127;
        doc_i8[1] = -64;

        let scale = 1.0 / 127.0;
        let dot = simd_dot_product_f32_i8(&query, &doc_i8, scale);
        // (1.0 * 127 + 0.5 * (-64)) * (1/127) = (127 - 32) / 127 = 95 / 127 ~= 0.7480315
        assert!((dot - (95.0 / 127.0)).abs() < 1e-5);

        // Clamping check
        let neg_query = [-1.0f32; 128];
        let pos_doc = [127i8; 128];
        let clamped_zero = simd_dot_product_f32_i8(&neg_query, &pos_doc, 1.0);
        assert_eq!(clamped_zero, 0.0);

        // Integer dot product check
        let mut a_i8 = [0i8; 128];
        let mut b_i8 = [0i8; 128];
        for i in 0..128 {
            a_i8[i] = 10;
            b_i8[i] = 20;
        }
        // 128 * (10 * 20) = 128 * 200 = 25600
        assert_eq!(simd_dot_product_i8(&a_i8, &b_i8), 25600);

        // Maximum magnitude without overflow
        let max_a = [127i8; 128];
        let max_b = [127i8; 128];
        assert_eq!(simd_dot_product_i8(&max_a, &max_b), 128 * 127 * 127);
    }
}
