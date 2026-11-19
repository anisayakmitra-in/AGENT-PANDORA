//! Tropical Semantic Projections & Hybrid BM25 + Max-Plus Memory Indexer.
//!
//! Implements ultra-fast, zero-multiplication semantic similarity across L0–L4 memory
//! records and virtual `pandora://` files using the Max-Plus tropical semiring
//! $(\mathbb{R} \cup \{-\infty\}, \oplus, \otimes)$ where:
//! - $a \oplus b = \max(a, b)$
//! - $a \otimes b = a + b$
//! - Tropical inner product: $\langle \mathbf{x}, \mathbf{w} \rangle_{\mathbb{T}} = \max_{i=0}^{127} (x_i + w_i)$
//!
//! Strictly adheres to `#![forbid(unsafe_code)]`.

use crate::arch_simd::{simd_hash64_seed, simd_tropical_max_plus_128};
use crate::context_vfs::{ContextSearchOptions, SearchResultItem, VfsDoc};
use crate::semantic_memory::{
    DENSE_DIMENSION, DenseFeatureHasher, HybridRankItem, HybridRrfRanker, QuantizedVector, RRF_K,
    SparseBm25Index, extract_term_frequencies, tokenize,
};
use pandora_types::{ContextUri, MemoryIntent, MemoryTier};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Dimension of tropical semantic projection vectors (aligned with 128-d HNSW & dense vectors).
pub const TROPICAL_DIMENSION: usize = DENSE_DIMENSION;

/// Default calibration temperature $\gamma$ for sigmoid squashing of tropical max-plus scores.
pub const DEFAULT_TROPICAL_GAMMA: f32 = 1.0;

/// Default convex combination weight $\alpha$ for linear BM25 + tropical blending.
pub const DEFAULT_HYBRID_ALPHA: f32 = 0.5;

/// A 128-dimensional vector in the Max-Plus tropical semiring $(\mathbb{R} \cup \{-\infty\}, \max, +)$.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TropicalVector(pub [f32; TROPICAL_DIMENSION]);

impl Default for TropicalVector {
    fn default() -> Self {
        Self::zeros()
    }
}

impl Serialize for TropicalVector {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.as_slice().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TropicalVector {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let vec = Vec::<f32>::deserialize(deserializer)?;
        if vec.len() != TROPICAL_DIMENSION {
            return Err(serde::de::Error::custom(format!(
                "expected {} tropical coordinates, got {}",
                TROPICAL_DIMENSION,
                vec.len()
            )));
        }
        let mut arr = [0.0f32; TROPICAL_DIMENSION];
        arr.copy_from_slice(&vec);
        Ok(Self(arr))
    }
}

impl TropicalVector {
    /// Creates a new `TropicalVector` from a 128-float array.
    pub const fn new(data: [f32; TROPICAL_DIMENSION]) -> Self {
        Self(data)
    }

    /// Returns a zero-initialized `TropicalVector` (baseline neutral coordinates).
    pub const fn zeros() -> Self {
        Self([0.0f32; TROPICAL_DIMENSION])
    }

    /// Returns the additive identity in the max-plus semiring ($-\infty$ across all coordinates).
    pub const fn neg_infinity() -> Self {
        Self([f32::NEG_INFINITY; TROPICAL_DIMENSION])
    }

    /// Constructs a `TropicalVector` from a float slice of length 128.
    pub fn from_slice(slice: &[f32]) -> Result<Self, String> {
        if slice.len() != TROPICAL_DIMENSION {
            return Err(format!(
                "invalid tropical vector length: expected {}, got {}",
                TROPICAL_DIMENSION,
                slice.len()
            ));
        }
        let mut data = [0.0f32; TROPICAL_DIMENSION];
        data.copy_from_slice(slice);
        Ok(Self(data))
    }

    /// Returns a reference to the underlying 128-element array.
    pub const fn as_array(&self) -> &[f32; TROPICAL_DIMENSION] {
        &self.0
    }

    /// Returns true if all coordinates are $0.0$ or $-\infty$ (no active tropical features).
    pub fn is_neutral(&self) -> bool {
        self.0.iter().all(|&x| x == 0.0 || x == f32::NEG_INFINITY)
    }

    /// Computes the zero-multiplication max-plus inner product:
    /// $$\langle \mathbf{x}, \mathbf{w} \rangle_{\mathbb{T}} = \max_{i=0}^{127} (x_i + w_i)$$
    pub fn score_max_plus(&self, other: &Self) -> f32 {
        let raw = simd_tropical_max_plus_128(&self.0, &other.0);
        if raw.is_nan() { f32::NEG_INFINITY } else { raw }
    }

    /// Computes Hilbert's tropical projective metric distance with zero multiplications:
    /// $$d_{\text{Hilbert}}(\mathbf{x}, \mathbf{w}) = \max_i(x_i - w_i) + \max_i(w_i - x_i)$$
    pub fn hilbert_distance(&self, other: &Self) -> f32 {
        if self == other {
            return 0.0;
        }
        let mut max_diff_ab = f32::NEG_INFINITY;
        let mut max_diff_ba = f32::NEG_INFINITY;
        for i in 0..TROPICAL_DIMENSION {
            let a = self.0[i];
            let b = other.0[i];
            if a.is_finite() && b.is_finite() {
                let d_ab = a - b;
                let d_ba = b - a;
                if d_ab > max_diff_ab {
                    max_diff_ab = d_ab;
                }
                if d_ba > max_diff_ba {
                    max_diff_ba = d_ba;
                }
            }
        }
        if !max_diff_ab.is_finite() || !max_diff_ba.is_finite() {
            return 0.0;
        }
        (max_diff_ab + max_diff_ba).max(0.0)
    }

    /// Maps the max-plus inner product and positive multi-coordinate tropical overlap
    /// into a calibrated similarity score in $[0.0, 1.0]$ via logistic sigmoid
    /// $\sigma(z) = \frac{1}{1 + e^{-z}}$, using zero multiplications in the inner loop.
    pub fn similarity(&self, other: &Self) -> f32 {
        let raw_max = self.score_max_plus(other);
        if raw_max == f32::NEG_INFINITY {
            return 0.0;
        }
        // Accumulate coordinate alignments where both vectors have whole-token salience (> 0.5)
        // so multi-token matches strictly dominate n-gram noise (0.45) and single collisions.
        let mut pos_sum = 0.0f32;
        for i in 0..TROPICAL_DIMENSION {
            let a = self.0[i];
            let b = other.0[i];
            if a.is_finite() && b.is_finite() && a > 0.5 && b > 0.5 {
                pos_sum += a + b;
            }
        }
        let z = if pos_sum > 0.0 { pos_sum } else { raw_max };
        let clamped = z.clamp(-20.0, 20.0);
        (1.0 / (1.0 + (-clamped).exp())).clamp(0.0, 1.0)
    }

    /// Computes a bounded metric distance in $[0.0, 2.0]$ suitable for HNSW graph search.
    pub fn distance(&self, other: &Self) -> f32 {
        if self == other {
            return 0.0;
        }
        (1.0 - self.similarity(other)).clamp(0.0, 2.0)
    }

    /// Converts a legacy `QuantizedVector` into a `TropicalVector` for fallback compatibility.
    pub fn from_quantized_fallback(q: &QuantizedVector) -> Self {
        let dequantized = q.dequantize();
        Self::from_dense_fallback(&dequantized)
    }

    /// Converts a legacy dense L2-normalized `[f32; 128]` vector into a `TropicalVector`
    /// using log-magnitude tropical lifting with zero multiplications.
    pub fn from_dense_fallback(dense: &[f32; TROPICAL_DIMENSION]) -> Self {
        let mut out = [0.0f32; TROPICAL_DIMENSION];
        for i in 0..TROPICAL_DIMENSION {
            let v = dense[i];
            if v > 0.0 {
                // Additive lift of positive activation features into the max-plus cone
                out[i] = v + v;
            } else if v < 0.0 {
                out[i] = v;
            } else {
                out[i] = -1.0;
            }
        }
        Self(out)
    }

    /// Serializes this `TropicalVector` to 512 little-endian bytes.
    pub fn to_bytes(&self) -> [u8; TROPICAL_DIMENSION * 4] {
        let mut buf = [0u8; TROPICAL_DIMENSION * 4];
        for (i, &val) in self.0.iter().enumerate() {
            let offset = i * 4;
            buf[offset..offset + 4].copy_from_slice(&val.to_le_bytes());
        }
        buf
    }

    /// Deserializes a `TropicalVector` from 512 little-endian bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < TROPICAL_DIMENSION * 4 {
            return Err(format!(
                "insufficient bytes for TropicalVector: expected {}, got {}",
                TROPICAL_DIMENSION * 4,
                bytes.len()
            ));
        }
        let mut data = [0.0f32; TROPICAL_DIMENSION];
        for (i, slot) in data.iter_mut().enumerate() {
            let offset = i * 4;
            let b: [u8; 4] = bytes[offset..offset + 4]
                .try_into()
                .map_err(|_| "slice conversion failed".to_owned())?;
            let val = f32::from_le_bytes(b);
            *slot = if val.is_nan() { f32::NEG_INFINITY } else { val };
        }
        Ok(Self(data))
    }
}

/// Projects text into 128-dimensional `TropicalVector` representations using subword n-grams
/// and token max-plus feature hashing with zero multiplications.
#[derive(Clone, Debug, Default)]
pub struct TropicalProjector;

/// Alias for `TropicalProjector` emphasizing semantic projection across memory tiers.
pub type TropicalSemanticProjector = TropicalProjector;

impl TropicalProjector {
    const SEED_PRIMARY: u64 = 0xcbf2_9ce4_8422_2325;
    const SEED_SECONDARY: u64 = 0x8422_2325_cbf2_9ce4;

    /// Projects text into a 128-dimensional `TropicalVector`.
    ///
    /// Empty or stop-word-only strings return `TropicalVector::zeros()`.
    /// Active tokens and character 3-grams/4-grams are accumulated via tropical addition
    /// ($\oplus = \max$) and additive log-frequency increments ($\otimes = +$).
    pub fn project(text: &str) -> TropicalVector {
        let tokens = tokenize(text);
        if tokens.is_empty() {
            return TropicalVector::zeros();
        }

        // Start inactive slots at -2.0 so unmatched dimensions do not dominate matched ones
        let mut coords = [-2.0f32; TROPICAL_DIMENSION];
        let mut touched = [false; TROPICAL_DIMENSION];

        for token in &tokens {
            let token_bytes = token.as_bytes();
            let base_salience = Self::token_salience(token);

            // 1. Primary whole-token projection
            let h1 = simd_hash64_seed(token_bytes, Self::SEED_PRIMARY);
            let idx1 = (h1 as usize) & (TROPICAL_DIMENSION - 1);
            Self::accumulate_tropical(&mut coords, &mut touched, idx1, base_salience);

            // 2. Secondary whole-token projection for collision resilience
            let h2 = simd_hash64_seed(token_bytes, Self::SEED_SECONDARY);
            let idx2 = (h2 as usize) & (TROPICAL_DIMENSION - 1);
            Self::accumulate_tropical(&mut coords, &mut touched, idx2, base_salience - 0.25);

            // 3. Subword character 3-grams and 4-grams
            let chars: Vec<char> = token.chars().collect();
            for n in [3usize, 4usize] {
                if chars.len() >= n {
                    for window in chars.windows(n) {
                        let ngram: String = window.iter().collect();
                        let hn =
                            simd_hash64_seed(ngram.as_bytes(), Self::SEED_PRIMARY ^ (n as u64));
                        let idx_n = (hn as usize) & (TROPICAL_DIMENSION - 1);
                        Self::accumulate_tropical(&mut coords, &mut touched, idx_n, 0.45);
                    }
                }
            }
        }

        TropicalVector(coords)
    }

    #[inline]
    fn accumulate_tropical(
        coords: &mut [f32; TROPICAL_DIMENSION],
        touched: &mut [bool; TROPICAL_DIMENSION],
        idx: usize,
        weight: f32,
    ) {
        if !touched[idx] {
            coords[idx] = weight;
            touched[idx] = true;
        } else {
            // Idempotent tropical addition (\oplus = max): prevents n-gram hash collisions
            // from inflating buckets above whole-token salience (0.85..1.35).
            coords[idx] = coords[idx].max(weight);
        }
    }

    #[inline]
    fn token_salience(token: &str) -> f32 {
        let len = token.len();
        if len >= 8 {
            1.35
        } else if len >= 5 {
            1.10
        } else {
            0.85
        }
    }

    /// Computes zero-multiplication max-plus score $\max_i(q_i + d_i)$ across a slice of indexed documents.
    ///
    /// Returns `(doc_id, similarity_score)` pairs sorted in descending order of score.
    pub fn score_docs(
        query: &TropicalVector,
        docs: &[(usize, TropicalVector)],
    ) -> Vec<(usize, f32)> {
        if query.is_neutral() {
            return Vec::new();
        }
        let mut results: Vec<(usize, f32)> = docs
            .iter()
            .filter_map(|(doc_id, doc_vec)| {
                if doc_vec.is_neutral() {
                    return None;
                }
                let raw = query.score_max_plus(doc_vec);
                // Require positive max-plus alignment (matching active buckets sum > 0)
                if raw > 0.0 {
                    let sim = query.similarity(doc_vec);
                    Some((*doc_id, sim))
                } else {
                    None
                }
            })
            .collect();

        results.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        results
    }
}

/// Hybrid ranker combining Okapi BM25 sparse keyword rankings with Tropical Max-Plus metric projections.
#[derive(Clone, Debug)]
pub struct TropicalHybridRanker {
    pub rrf_k: f32,
    pub alpha: f32,
}

impl Default for TropicalHybridRanker {
    fn default() -> Self {
        Self {
            rrf_k: RRF_K,
            alpha: DEFAULT_HYBRID_ALPHA,
        }
    }
}

impl TropicalHybridRanker {
    /// Creates a new `TropicalHybridRanker` with default RRF constant ($k = 60.0$) and $\alpha = 0.5$.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a `TropicalHybridRanker` with custom RRF constant and convex weight $\alpha \in [0.0, 1.0]$.
    pub fn with_params(rrf_k: f32, alpha: f32) -> Self {
        Self {
            rrf_k: rrf_k.max(1.0),
            alpha: alpha.clamp(0.0, 1.0),
        }
    }

    /// Computes linear convex combination of normalized BM25 score and tropical score:
    /// $$S_{\text{linear}} = \alpha \cdot S_{\text{bm25\_norm}} + (1 - \alpha) \cdot \sigma(S_{\text{tropical}})$$
    pub fn blend_linear(&self, bm25_score: f32, tropical_raw_or_sim: f32) -> f32 {
        let bm25_norm = if bm25_score <= 0.0 {
            0.0
        } else {
            (bm25_score / (bm25_score + 2.0)).clamp(0.0, 1.0)
        };
        let trop_norm = if (0.0..=1.0).contains(&tropical_raw_or_sim) {
            tropical_raw_or_sim
        } else {
            let clamped = tropical_raw_or_sim.clamp(-20.0, 20.0);
            (1.0 / (1.0 + (-clamped).exp())).clamp(0.0, 1.0)
        };
        (self.alpha * bm25_norm + (1.0 - self.alpha) * trop_norm).clamp(0.0, 1.0)
    }

    /// Fuses sparse BM25 rankings with tropical projection rankings via Reciprocal Rank Fusion (RRF).
    pub fn fuse(
        &self,
        sparse_scores: &[(usize, f32)],
        tropical_scores: &[(usize, f32)],
    ) -> Vec<HybridRankItem> {
        HybridRrfRanker::with_k(self.rrf_k).fuse(sparse_scores, tropical_scores)
    }

    /// Fuses sparse BM25 rankings with tropical rankings, automatically falling back to legacy
    /// dense embeddings when tropical rankings are unavailable.
    pub fn fuse_with_fallback(
        &self,
        sparse_scores: &[(usize, f32)],
        tropical_scores: &[(usize, f32)],
        dense_fallback_scores: &[(usize, f32)],
    ) -> Vec<HybridRankItem> {
        let semantic_stream = if !tropical_scores.is_empty() {
            tropical_scores
        } else {
            dense_fallback_scores
        };
        self.fuse(sparse_scores, semantic_stream)
    }
}

/// High-level unified semantic indexer across L0–L4 memory records and virtual `pandora://` files.
///
/// Maintains:
/// 1. Okapi BM25 inverted index (`SparseBm25Index`)
/// 2. Tropical Max-Plus metric projections (`TropicalVector`)
/// 3. Legacy dense quantized embeddings (`QuantizedVector`) for fallback compatibility
#[derive(Clone, Debug, Default)]
pub struct SemanticIndexer {
    docs: Vec<VfsDoc>,
    bm25: SparseBm25Index,
    tropical_vectors: Vec<(usize, TropicalVector)>,
    dense_fallback_vectors: Vec<(usize, QuantizedVector)>,
    ranker: TropicalHybridRanker,
}

impl SemanticIndexer {
    /// Creates an empty `SemanticIndexer`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the number of indexed documents.
    pub fn len(&self) -> usize {
        self.docs.len()
    }

    /// Returns true if no documents have been indexed.
    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    /// Returns a slice of all indexed `VfsDoc` items.
    pub fn documents(&self) -> &[VfsDoc] {
        &self.docs
    }

    /// Returns a slice of `(doc_id, TropicalVector)` entries.
    pub fn tropical_vectors(&self) -> &[(usize, TropicalVector)] {
        &self.tropical_vectors
    }

    /// Returns a slice of legacy `(doc_id, QuantizedVector)` fallback entries.
    pub fn dense_fallback_vectors(&self) -> &[(usize, QuantizedVector)] {
        &self.dense_fallback_vectors
    }

    /// Indexes a single `VfsDoc` across BM25, tropical max-plus projection, and dense fallback stores.
    pub fn index_vfs_doc(&mut self, doc: VfsDoc) -> usize {
        let doc_id = self.docs.len();
        let combined = format!("{} {}", doc.title, doc.content);
        let (doc_len, tfs) = extract_term_frequencies(&combined);
        self.bm25.add_document_terms(doc_id, doc_len, &tfs);

        let tropical = TropicalProjector::project(&combined);
        self.tropical_vectors.push((doc_id, tropical));

        let hasher = DenseFeatureHasher::new();
        let dense = hasher.vectorize(&combined);
        let quantized = QuantizedVector::quantize(&dense);
        self.dense_fallback_vectors.push((doc_id, quantized));

        self.docs.push(doc);
        doc_id
    }

    /// Indexes a memory record or virtual `pandora://` resource directly from fields.
    pub fn index_record(
        &mut self,
        uri: ContextUri,
        title: impl Into<String>,
        content: impl Into<String>,
        tier: Option<MemoryTier>,
        intent: Option<MemoryIntent>,
    ) -> usize {
        self.index_vfs_doc(VfsDoc {
            uri,
            title: title.into(),
            content: content.into(),
            tier,
            intent,
        })
    }

    /// Performs pure zero-multiplication tropical semantic projection search.
    pub fn search_tropical(&self, query: &str, limit: usize) -> Vec<SearchResultItem> {
        let trimmed = query.trim();
        if trimmed.is_empty() || self.docs.is_empty() {
            return Vec::new();
        }

        let query_trop = TropicalProjector::project(trimmed);
        let mut scored = TropicalProjector::score_docs(&query_trop, &self.tropical_vectors);
        if scored.is_empty() {
            // Fallback to legacy dense embeddings if tropical stream yields no matches
            let hasher = DenseFeatureHasher::new();
            let query_dense = hasher.vectorize(trimmed);
            scored = hasher.score_quantized_docs(&query_dense, &self.dense_fallback_vectors);
        }

        let effective_limit = limit.max(1);
        scored
            .into_iter()
            .take(effective_limit)
            .filter_map(|(doc_id, score)| {
                let doc = self.docs.get(doc_id)?;
                Some(SearchResultItem {
                    uri: doc.uri.clone(),
                    title: doc.title.clone(),
                    snippet: Self::make_snippet(&doc.content, 160),
                    sparse_score: 0.0,
                    dense_score: score,
                    hybrid_score: score,
                    tier: doc.tier,
                    intent: doc.intent,
                })
            })
            .collect()
    }

    /// Performs hybrid BM25 + tropical max-plus projection search with tier/intent filtering
    /// and automatic fallback to legacy dense embeddings.
    pub fn search_hybrid(
        &self,
        query: &str,
        options: &ContextSearchOptions,
    ) -> Vec<SearchResultItem> {
        let trimmed = query.trim();
        if trimmed.is_empty() || self.docs.is_empty() {
            return Vec::new();
        }

        let sparse_scores = self.bm25.score(trimmed);
        let query_trop = TropicalProjector::project(trimmed);
        let tropical_scores = TropicalProjector::score_docs(&query_trop, &self.tropical_vectors);

        let dense_fallback = if tropical_scores.is_empty() {
            let hasher = DenseFeatureHasher::new();
            let query_dense = hasher.vectorize(trimmed);
            hasher.score_quantized_docs(&query_dense, &self.dense_fallback_vectors)
        } else {
            Vec::new()
        };

        let fused =
            self.ranker
                .fuse_with_fallback(&sparse_scores, &tropical_scores, &dense_fallback);

        let limit = options.limit.max(1);
        let mut results = Vec::new();
        for item in fused {
            if item.hybrid_score < options.min_score {
                continue;
            }
            let Some(doc) = self.docs.get(item.doc_id) else {
                continue;
            };
            if let Some(required_tier) = options.tier
                && doc.tier != Some(required_tier)
            {
                continue;
            }
            if let Some(required_intent) = options.intent
                && doc.intent != Some(required_intent)
            {
                continue;
            }
            results.push(SearchResultItem {
                uri: doc.uri.clone(),
                title: doc.title.clone(),
                snippet: Self::make_snippet(&doc.content, 160),
                sparse_score: item.sparse_score,
                dense_score: item.dense_score,
                hybrid_score: item.hybrid_score,
                tier: doc.tier,
                intent: doc.intent,
            });
            if results.len() >= limit {
                break;
            }
        }
        results
    }

    fn make_snippet(content: &str, max_chars: usize) -> String {
        let single_line = content.split_whitespace().collect::<Vec<_>>().join(" ");
        if single_line.chars().count() <= max_chars {
            single_line
        } else {
            let truncated: String = single_line.chars().take(max_chars).collect();
            format!("{truncated}...")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tropical_vector_max_plus_zero_multiplication_identity_and_bounds() {
        let mut a_arr = [-1.0f32; TROPICAL_DIMENSION];
        let mut b_arr = [-1.0f32; TROPICAL_DIMENSION];
        a_arr[7] = 1.5;
        b_arr[7] = 2.25;
        a_arr[42] = 0.5;
        b_arr[42] = 1.0;

        let va = TropicalVector::new(a_arr);
        let vb = TropicalVector::new(b_arr);

        let score = va.score_max_plus(&vb);
        assert!(
            (score - 3.75).abs() < 1e-6,
            "expected max_i(a_i + b_i) = 3.75, got {score}"
        );

        let sim = va.similarity(&vb);
        assert!(sim > 0.95 && sim <= 1.0);
        assert_eq!(va.distance(&va), 0.0);
        assert!(va.distance(&vb) < 0.05);
    }

    #[test]
    fn tropical_projector_and_hybrid_indexer_rank_l0_to_l4_records() {
        let mut indexer = SemanticIndexer::new();
        indexer.index_record(
            ContextUri::new("pandora://meta/constitution").expect("valid uri"),
            "Safety Constitution",
            "Enforce strict zero-trust reference monitor and capability token verification",
            Some(MemoryTier::L0),
            None,
        );
        indexer.index_record(
            ContextUri::new("pandora://sops/database-migration").expect("valid uri"),
            "Database Migration SOP",
            "Run Liquibase schema migration and verify PostgreSQL foreign keys",
            Some(MemoryTier::L3),
            None,
        );
        indexer.index_record(
            ContextUri::new("pandora://archives/tropical-mcts-notes").expect("valid uri"),
            "Tropical Semiring MCTS Notes",
            "Max-plus tropical algebra eliminates floating-point multiplications in PUCT search",
            Some(MemoryTier::L4),
            None,
        );

        let trop_hits = indexer.search_tropical("tropical max-plus algebra", 3);
        assert!(!trop_hits.is_empty());
        assert_eq!(
            trop_hits[0].uri.as_str(),
            "pandora://archives/tropical-mcts-notes"
        );

        let hybrid_l0 = indexer.search_hybrid(
            "reference monitor capability verification",
            &ContextSearchOptions::new(10).with_tier(MemoryTier::L0),
        );
        assert_eq!(hybrid_l0.len(), 1);
        assert_eq!(hybrid_l0[0].tier, Some(MemoryTier::L0));
    }
}
