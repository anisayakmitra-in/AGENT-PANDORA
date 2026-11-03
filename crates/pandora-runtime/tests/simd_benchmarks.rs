//! Integration tests for SIMD vector acceleration and statistical microbenchmarks.

#![forbid(unsafe_code)]

use pandora_runtime::{
    DimensionBenchmarkStats, compute_sample_stats, run_statistical_microbenchmarks,
    scalar_dot_product_slice, simd_dot_product_slice, student_t_critical_value_95,
};

#[test]
fn test_multidim_dot_product_exact_equivalence() {
    let test_dimensions = [128, 256, 512, 1536];

    for &dim in &test_dimensions {
        let mut a = vec![0.0f32; dim];
        let mut b = vec![0.0f32; dim];

        // Seed with varied positive and negative floating point values
        for i in 0..dim {
            a[i] = ((i % 17) as f32) * 0.25 - 2.0;
            b[i] = (((dim - i) % 13) as f32) * 0.5 - 3.0;
        }

        let simd_val = simd_dot_product_slice(&a, &b);
        let scalar_val = scalar_dot_product_slice(&a, &b);

        let delta = (simd_val - scalar_val).abs();
        let scale = scalar_val.abs().max(1.0);
        let rel_err = delta / scale;

        assert!(
            rel_err < 1e-4,
            "Dimension {dim}: relative error {rel_err} exceeded 1e-4 (simd={simd_val}, scalar={scalar_val})"
        );
    }
}

#[test]
fn test_arbitrary_unaligned_slice_lengths() {
    let unaligned_lengths = [1, 2, 3, 5, 7, 11, 13, 17, 31, 63, 127, 255, 511, 1023, 1535];

    for &len in &unaligned_lengths {
        let a: Vec<f32> = (0..len).map(|i| (i as f32) * 0.1).collect();
        let b: Vec<f32> = (0..len).map(|i| ((len - i) as f32) * 0.2).collect();

        let simd_val = simd_dot_product_slice(&a, &b);
        let scalar_val = scalar_dot_product_slice(&a, &b);

        let delta = (simd_val - scalar_val).abs();
        let scale = scalar_val.abs().max(1.0);
        assert!(
            delta / scale < 1e-4,
            "Length {len}: simd={simd_val}, scalar={scalar_val}"
        );
    }
}

#[test]
fn test_statistical_microbenchmarks_suite_30_trials() {
    let dimensions = [128, 256, 512, 1536];
    let trials = 30;
    let iterations = 500;

    let stats = run_statistical_microbenchmarks(&dimensions, trials, iterations);

    assert_eq!(stats.len(), 4);

    for stat in &stats {
        assert!(dimensions.contains(&stat.dimension));
        assert_eq!(stat.trials, 30);
        assert_eq!(stat.iterations_per_trial, 500);

        // Sample means must be positive non-zero
        assert!(stat.scalar_mean_nanos > 0.0);
        assert!(stat.simd_mean_nanos > 0.0);

        // Standard deviations must be non-negative
        assert!(stat.scalar_std_dev_nanos >= 0.0);
        assert!(stat.simd_std_dev_nanos >= 0.0);

        // 95% Confidence interval for mean must contain or bracket the mean
        assert!(stat.scalar_ci_95.0 <= stat.scalar_mean_nanos);
        assert!(stat.scalar_mean_nanos <= stat.scalar_ci_95.1);

        assert!(stat.simd_ci_95.0 <= stat.simd_mean_nanos);
        assert!(stat.simd_mean_nanos <= stat.simd_ci_95.1);

        // Speedup ratio and Taylor-series bounds must be strictly positive and properly ordered
        assert!(stat.speedup_ratio > 0.0);
        assert!(stat.speedup_ci_95.0 <= stat.speedup_ci_95.1);

        // Test serde serialization / deserialization roundtrip
        let json = serde_json::to_string(&stat).expect("Serialization failed");
        let decoded: DimensionBenchmarkStats =
            serde_json::from_str(&json).expect("Deserialization failed");
        assert_eq!(stat.dimension, decoded.dimension);
        assert_eq!(stat.trials, decoded.trials);
        assert_eq!(stat.iterations_per_trial, decoded.iterations_per_trial);
        assert!((stat.scalar_mean_nanos - decoded.scalar_mean_nanos).abs() < 1e-3);
        assert!((stat.simd_mean_nanos - decoded.simd_mean_nanos).abs() < 1e-3);
        assert!((stat.speedup_ratio - decoded.speedup_ratio).abs() < 1e-4);
    }
}

#[test]
fn test_student_t_and_taylor_propagation_mathematical_properties() {
    // Check known values for Student's t critical values
    let t_29 = student_t_critical_value_95(29);
    assert!((t_29 - 2.045).abs() < 1e-3);

    // Verify monotonicity of Student's t for small dof
    for nu in 1..29 {
        assert!(
            student_t_critical_value_95(nu) > student_t_critical_value_95(nu + 1),
            "Expected t({nu}) > t({})",
            nu + 1
        );
    }

    // Verify statistical sample stats helper with known analytical dataset
    let data = vec![2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
    // N = 8, mean = 40 / 8 = 5.0
    // Var = (9 + 1 + 1 + 1 + 0 + 0 + 4 + 16) / 7 = 32 / 7 ~= 4.571428
    // Std dev = sqrt(32 / 7) ~= 2.13809
    let (mean, std_dev, (ci_lo, ci_hi)) = compute_sample_stats(&data);
    assert!((mean - 5.0).abs() < 1e-6);
    assert!((std_dev - (32.0f64 / 7.0).sqrt()).abs() < 1e-4);
    assert!(ci_lo < mean && mean < ci_hi);
}
