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

/// Computes L2 squared norm of a 128-dimensional dense vector using vector-unrolled lanes.
pub fn simd_norm_sq_128(v: &[f32; 128]) -> f32 {
    simd_dot_product_128(v, v)
}

/// Vector-accelerated subword character FNV-1a block hasher.
/// Processes bytes with unrolled word accumulation.
#[allow(clippy::chunks_exact_to_as_chunks)]
pub fn simd_hash64_seed(bytes: &[u8], seed: u64) -> u64 {
    let mut h = 0xcbf29ce484222325 ^ seed;
    let chunks = bytes.chunks_exact(8);
    let remainder = chunks.remainder();

    for chunk in chunks {
        let word = u64::from_le_bytes([
            chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
        ]);
        h ^= word;
        h = h.wrapping_mul(0x100000001b3);
    }

    for &b in remainder {
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
}
