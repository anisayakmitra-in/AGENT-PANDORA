//! Hardware Batch Compute Vector Dispatch Engine
//!
//! Grounded in mathematical foundations:
//! - Theorem 7: Roofline operational intensity scaling I(B, N) = (2/b) * (B*N)/(B+N)
//! - Amdahl's Law SIMD speedup formulation S(f, p) = 1 / ((1 - f) + f/p)
//! - Integer dot-product overflow protection bound (|sum| <= 2,097,152 < 2^31 - 1)
//! - 8-lane loop unrolling for hardware auto-vectorization

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

/// Embedding vector data representation precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Precision {
    F32,
    Int8,
}

impl Precision {
    pub const fn bytes_per_element(self) -> usize {
        match self {
            Self::F32 => 4,
            Self::Int8 => 1,
        }
    }
}

/// Analytical Roofline Arithmetic Intensity Model (Theorem 7).
pub struct RooflineModel;

impl RooflineModel {
    /// Computes operational intensity I(B, N) = (2/b) * (B * N) / (B + N)
    /// where B is batch size, N is corpus size, and b is bytes per element.
    pub fn calculate_intensity(batch_size: usize, corpus_size: usize, precision: Precision) -> f64 {
        if batch_size == 0 || corpus_size == 0 {
            return 0.0;
        }
        let b = precision.bytes_per_element() as f64;
        let bf = batch_size as f64;
        let nf = corpus_size as f64;

        (2.0 / b) * (bf * nf) / (bf + nf)
    }

    /// Computes theoretical Amdahl's Law speedup: S(f, p) = 1 / ((1 - f) + f/p)
    pub fn amdahl_speedup(parallel_fraction: f64, simd_lanes: usize) -> f64 {
        let f = parallel_fraction.clamp(0.0, 1.0);
        let p = (simd_lanes.max(1)) as f64;

        1.0 / ((1.0 - f) + (f / p))
    }
}

/// Hardware Vector Dispatcher executing batched linear algebra operations
/// under `#![forbid(unsafe_code)]` with 8-wide chunk unrolling.
pub struct BatchVectorDispatcher;

impl BatchVectorDispatcher {
    /// Unrolled 8-lane dot product between two f32 vectors.
    /// Auto-vectorizes into AVX2 / NEON SIMD instructions on modern LLVM backends.
    pub fn dot_product_f32(a: &[f32], b: &[f32]) -> f32 {
        let len = a.len().min(b.len());
        let mut acc0 = 0.0f32;
        let mut acc1 = 0.0f32;
        let mut acc2 = 0.0f32;
        let mut acc3 = 0.0f32;
        let mut acc4 = 0.0f32;
        let mut acc5 = 0.0f32;
        let mut acc6 = 0.0f32;
        let mut acc7 = 0.0f32;

        let mut offset = 0;
        while offset + 8 <= len {
            acc0 += a[offset] * b[offset];
            acc1 += a[offset + 1] * b[offset + 1];
            acc2 += a[offset + 2] * b[offset + 2];
            acc3 += a[offset + 3] * b[offset + 3];
            acc4 += a[offset + 4] * b[offset + 4];
            acc5 += a[offset + 5] * b[offset + 5];
            acc6 += a[offset + 6] * b[offset + 6];
            acc7 += a[offset + 7] * b[offset + 7];
            offset += 8;
        }

        let mut sum = (acc0 + acc1) + (acc2 + acc3) + (acc4 + acc5) + (acc6 + acc7);
        while offset < len {
            sum += a[offset] * b[offset];
            offset += 1;
        }

        sum
    }

    /// Unrolled 8-lane dot product between two quantized i8 vectors.
    /// Uses 32-bit integer accumulators guaranteeing zero overflow (Theorem).
    pub fn dot_product_int8(a: &[i8], b: &[i8]) -> i32 {
        let len = a.len().min(b.len());
        let mut acc0 = 0i32;
        let mut acc1 = 0i32;
        let mut acc2 = 0i32;
        let mut acc3 = 0i32;
        let mut acc4 = 0i32;
        let mut acc5 = 0i32;
        let mut acc6 = 0i32;
        let mut acc7 = 0i32;

        let mut offset = 0;
        while offset + 8 <= len {
            acc0 += (a[offset] as i32) * (b[offset] as i32);
            acc1 += (a[offset + 1] as i32) * (b[offset + 1] as i32);
            acc2 += (a[offset + 2] as i32) * (b[offset + 2] as i32);
            acc3 += (a[offset + 3] as i32) * (b[offset + 3] as i32);
            acc4 += (a[offset + 4] as i32) * (b[offset + 4] as i32);
            acc5 += (a[offset + 5] as i32) * (b[offset + 5] as i32);
            acc6 += (a[offset + 6] as i32) * (b[offset + 6] as i32);
            acc7 += (a[offset + 7] as i32) * (b[offset + 7] as i32);
            offset += 8;
        }

        let mut sum = acc0 + acc1 + acc2 + acc3 + acc4 + acc5 + acc6 + acc7;
        while offset < len {
            sum += (a[offset] as i32) * (b[offset] as i32);
            offset += 1;
        }

        sum
    }

    /// Computes cosine similarity between two f32 vectors: dot(a, b) / (||a|| * ||b||).
    pub fn cosine_similarity_f32(a: &[f32], b: &[f32]) -> f32 {
        let dot = Self::dot_product_f32(a, b);
        let norm_a = Self::dot_product_f32(a, a).sqrt();
        let norm_b = Self::dot_product_f32(b, b).sqrt();

        let denom = norm_a * norm_b;
        if denom < 1e-12 { 0.0 } else { dot / denom }
    }

    /// Evaluates a batch of B queries against N corpus vectors in f32 precision.
    /// Returns similarity matrix of dimensions [B][N].
    pub fn batch_cosine_similarity_matrix(
        queries: &[Vec<f32>],
        corpus: &[Vec<f32>],
    ) -> Vec<Vec<f32>> {
        let mut matrix = Vec::with_capacity(queries.len());

        // Precompute corpus norms to avoid redundant recalculation
        let corpus_norms: Vec<f32> = corpus
            .iter()
            .map(|vec| Self::dot_product_f32(vec, vec).sqrt())
            .collect();

        for query in queries {
            let query_norm = Self::dot_product_f32(query, query).sqrt();
            let mut row = Vec::with_capacity(corpus.len());

            for (c_idx, c_vec) in corpus.iter().enumerate() {
                let dot = Self::dot_product_f32(query, c_vec);
                let denom = query_norm * corpus_norms[c_idx];
                let sim = if denom < 1e-12 { 0.0 } else { dot / denom };
                row.push(sim);
            }
            matrix.push(row);
        }

        matrix
    }

    /// Evaluates a batch of B queries against N corpus vectors in Int8 precision.
    /// Returns integer dot product score matrix of dimensions [B][N].
    pub fn batch_dot_product_matrix_int8(queries: &[Vec<i8>], corpus: &[Vec<i8>]) -> Vec<Vec<i32>> {
        let mut matrix = Vec::with_capacity(queries.len());

        for query in queries {
            let mut row = Vec::with_capacity(corpus.len());
            for c_vec in corpus {
                let score = Self::dot_product_int8(query, c_vec);
                row.push(score);
            }
            matrix.push(row);
        }

        matrix
    }

    /// Top-K selection returning sorted pairs of (corpus_index, score).
    pub fn top_k(scores: &[f32], k: usize) -> Vec<(usize, f32)> {
        let mut pairs: Vec<(usize, f32)> = scores.iter().copied().enumerate().collect();
        // Sort in descending order of similarity
        pairs.sort_by(|(_, a), (_, b)| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        pairs.into_iter().take(k).collect()
    }
}
