#![forbid(unsafe_code)]

//! Milestone 1 Empirical Challenge Suite (v3.0.3)
//!
//! Tests:
//! 1. Benchmark JSON serialization & deserialization fidelity and robustness.
//! 2. Error handling on malformed, corrupted, and boundary JSON payloads.
//! 3. Multi-dimension statistical microbenchmark data invariants.

use pandora_runtime::{
    DimensionBenchmarkStats, SimdBenchmarkResult, run_statistical_microbenchmarks,
};

fn assert_stats_approx_eq(a: &DimensionBenchmarkStats, b: &DimensionBenchmarkStats) {
    assert_eq!(a.dimension, b.dimension);
    assert_eq!(a.iterations_per_trial, b.iterations_per_trial);
    assert_eq!(a.trials, b.trials);
    assert!((a.scalar_mean_nanos - b.scalar_mean_nanos).abs() < 1e-6);
    assert!((a.scalar_std_dev_nanos - b.scalar_std_dev_nanos).abs() < 1e-6);
    assert!((a.scalar_ci_95.0 - b.scalar_ci_95.0).abs() < 1e-6);
    assert!((a.scalar_ci_95.1 - b.scalar_ci_95.1).abs() < 1e-6);
    assert!((a.simd_mean_nanos - b.simd_mean_nanos).abs() < 1e-6);
    assert!((a.simd_std_dev_nanos - b.simd_std_dev_nanos).abs() < 1e-6);
    assert!((a.simd_ci_95.0 - b.simd_ci_95.0).abs() < 1e-6);
    assert!((a.simd_ci_95.1 - b.simd_ci_95.1).abs() < 1e-6);
    assert!((a.speedup_ratio - b.speedup_ratio).abs() < 1e-6);
    assert!((a.speedup_ci_95.0 - b.speedup_ci_95.0).abs() < 1e-6);
    assert!((a.speedup_ci_95.1 - b.speedup_ci_95.1).abs() < 1e-6);
}

#[test]
fn test_m1_benchmark_json_roundtrip_all_dimensions() {
    let dimensions = [128, 256, 512, 1536];
    let stats = run_statistical_microbenchmarks(&dimensions, 30, 200);
    assert_eq!(stats.len(), 4);

    for s in &stats {
        // Compact JSON serialization
        let json_compact = serde_json::to_string(s).expect("Compact serialization failed");
        let decoded_compact: DimensionBenchmarkStats =
            serde_json::from_str(&json_compact).expect("Compact deserialization failed");
        assert_stats_approx_eq(s, &decoded_compact);

        // Pretty JSON serialization
        let json_pretty = serde_json::to_string_pretty(s).expect("Pretty serialization failed");
        let decoded_pretty: DimensionBenchmarkStats =
            serde_json::from_str(&json_pretty).expect("Pretty deserialization failed");
        assert_stats_approx_eq(s, &decoded_pretty);

        // Verify statistical invariants
        assert!(s.dimension >= 128);
        assert_eq!(s.trials, 30);
        assert_eq!(s.iterations_per_trial, 200);
        assert!(s.scalar_mean_nanos > 0.0);
        assert!(s.simd_mean_nanos > 0.0);
        assert!(s.scalar_std_dev_nanos >= 0.0);
        assert!(s.simd_std_dev_nanos >= 0.0);
        assert!(s.scalar_ci_95.0 <= s.scalar_mean_nanos);
        assert!(s.scalar_mean_nanos <= s.scalar_ci_95.1);
        assert!(s.simd_ci_95.0 <= s.simd_mean_nanos);
        assert!(s.simd_mean_nanos <= s.simd_ci_95.1);
        assert!(s.speedup_ratio > 0.0);
        assert!(s.speedup_ci_95.0 <= s.speedup_ci_95.1);
    }
}

#[test]
fn test_m1_benchmark_json_malformed_rejection() {
    // Missing dimension
    let json_missing_field = r#"{
        "iterations_per_trial": 100,
        "trials": 30,
        "scalar_mean_nanos": 500.0,
        "scalar_std_dev_nanos": 10.0,
        "scalar_ci_95": [480.0, 520.0],
        "simd_mean_nanos": 250.0,
        "simd_std_dev_nanos": 5.0,
        "simd_ci_95": [240.0, 260.0],
        "speedup_ratio": 2.0,
        "speedup_ci_95": [1.9, 2.1]
    }"#;
    assert!(serde_json::from_str::<DimensionBenchmarkStats>(json_missing_field).is_err());

    // Type mismatch: string where float expected
    let json_bad_type = r#"{
        "dimension": 128,
        "iterations_per_trial": 100,
        "trials": 30,
        "scalar_mean_nanos": "invalid_number",
        "scalar_std_dev_nanos": 10.0,
        "scalar_ci_95": [480.0, 520.0],
        "simd_mean_nanos": 250.0,
        "simd_std_dev_nanos": 5.0,
        "simd_ci_95": [240.0, 260.0],
        "speedup_ratio": 2.0,
        "speedup_ci_95": [1.9, 2.1]
    }"#;
    assert!(serde_json::from_str::<DimensionBenchmarkStats>(json_bad_type).is_err());

    // Malformed tuple length (3 elements instead of 2)
    let json_bad_tuple = r#"{
        "dimension": 128,
        "iterations_per_trial": 100,
        "trials": 30,
        "scalar_mean_nanos": 500.0,
        "scalar_std_dev_nanos": 10.0,
        "scalar_ci_95": [480.0, 500.0, 520.0],
        "simd_mean_nanos": 250.0,
        "simd_std_dev_nanos": 5.0,
        "simd_ci_95": [240.0, 260.0],
        "speedup_ratio": 2.0,
        "speedup_ci_95": [1.9, 2.1]
    }"#;
    assert!(serde_json::from_str::<DimensionBenchmarkStats>(json_bad_tuple).is_err());

    // Empty or garbage JSON
    assert!(serde_json::from_str::<DimensionBenchmarkStats>("").is_err());
    assert!(serde_json::from_str::<DimensionBenchmarkStats>("null").is_err());
    assert!(serde_json::from_str::<DimensionBenchmarkStats>("12345").is_err());
    assert!(serde_json::from_str::<DimensionBenchmarkStats>("{ unclosed").is_err());
}

#[test]
fn test_m1_benchmark_json_boundary_values() {
    let boundary = DimensionBenchmarkStats {
        dimension: 0,
        iterations_per_trial: 0,
        trials: 0,
        scalar_mean_nanos: 0.0,
        scalar_std_dev_nanos: 0.0,
        scalar_ci_95: (0.0, 0.0),
        simd_mean_nanos: f64::MAX,
        simd_std_dev_nanos: f64::MIN_POSITIVE,
        simd_ci_95: (-0.0, 1e-300),
        speedup_ratio: 1.0,
        speedup_ci_95: (1.0, 1.0),
    };

    let serialized = serde_json::to_string(&boundary).expect("Serialization failed");
    let deserialized: DimensionBenchmarkStats =
        serde_json::from_str(&serialized).expect("Deserialization failed");
    assert_eq!(boundary.dimension, deserialized.dimension);
    assert_eq!(boundary.simd_mean_nanos, deserialized.simd_mean_nanos);
    assert_eq!(boundary.simd_std_dev_nanos, deserialized.simd_std_dev_nanos);
}

#[test]
fn test_m1_simd_benchmark_result_json() {
    let res = SimdBenchmarkResult {
        iterations: 50_000,
        scalar_duration_nanos: 9_999_999,
        simd_duration_nanos: 3_333_333,
        speedup_ratio: 3.0,
    };

    let json = serde_json::to_string(&res).expect("Serialization failed");
    let decoded: SimdBenchmarkResult = serde_json::from_str(&json).expect("Deserialization failed");
    assert_eq!(res, decoded);
}
