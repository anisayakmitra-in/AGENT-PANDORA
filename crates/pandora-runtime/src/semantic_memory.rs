//! Semantic Memory & Hybrid RAG Engine.
//!
//! Provides zero-network, local, deterministic hybrid search:
//! - Pure-Rust in-memory Okapi BM25 sparse inverted index (k1 = 1.2, b = 0.75).
//! - Pure-Rust deterministic subword character n-gram / feature hashing dense vectorizer (d = 128).
//! - Hybrid Reciprocal Rank Fusion (RRF) combining sparse keyword ranking and dense similarity ranking.
//! - Search over `pandora://` ContextVfs mounted resources and session error pattern lesson retrieval (`eval-lesson-*`).
//!
//! All algorithms operate 100% locally and deterministically with zero remote network calls
//! under strict `#![forbid(unsafe_code)]`.

use pandora_types::{ContextUri, MemoryIntent, MemoryRecord, MemoryTier, Timestamp};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Dense vector dimension for feature hashing.
pub const DENSE_DIMENSION: usize = 128;

/// Okapi BM25 term frequency saturation parameter.
pub const BM25_K1: f32 = 1.2;

/// Okapi BM25 document length normalization parameter.
pub const BM25_B: f32 = 0.75;

/// Reciprocal Rank Fusion constant (Cormack et al., 2009).
pub const RRF_K: f32 = 60.0;

/// Standard non-informative English stop words.
/// Technical keywords, error codes, and compiler terms are explicitly excluded.
const STOP_WORDS: &[&str] = &[
    "a",
    "about",
    "above",
    "after",
    "again",
    "against",
    "all",
    "am",
    "an",
    "and",
    "any",
    "are",
    "aren't",
    "as",
    "at",
    "be",
    "because",
    "been",
    "before",
    "being",
    "below",
    "between",
    "both",
    "but",
    "by",
    "can",
    "cannot",
    "could",
    "couldn't",
    "did",
    "didn't",
    "do",
    "does",
    "doesn't",
    "doing",
    "don't",
    "down",
    "during",
    "each",
    "few",
    "for",
    "from",
    "further",
    "had",
    "hadn't",
    "has",
    "hasn't",
    "have",
    "haven't",
    "having",
    "he",
    "her",
    "here",
    "hers",
    "herself",
    "him",
    "himself",
    "his",
    "how",
    "i",
    "if",
    "in",
    "into",
    "is",
    "isn't",
    "it",
    "it's",
    "its",
    "itself",
    "me",
    "more",
    "most",
    "mustn't",
    "my",
    "myself",
    "no",
    "nor",
    "not",
    "of",
    "off",
    "on",
    "once",
    "only",
    "or",
    "other",
    "ought",
    "our",
    "ours",
    "ourselves",
    "out",
    "over",
    "own",
    "same",
    "she",
    "should",
    "shouldn't",
    "so",
    "some",
    "such",
    "than",
    "that",
    "the",
    "their",
    "theirs",
    "them",
    "themselves",
    "then",
    "there",
    "these",
    "they",
    "this",
    "those",
    "through",
    "to",
    "too",
    "under",
    "until",
    "up",
    "very",
    "was",
    "wasn't",
    "we",
    "were",
    "weren't",
    "what",
    "when",
    "where",
    "which",
    "while",
    "who",
    "whom",
    "why",
    "with",
    "won't",
    "would",
    "wouldn't",
    "you",
    "your",
    "yours",
    "yourself",
    "yourselves",
];

/// 64-bit FNV-1a hash with custom seed for deterministic hashing.
fn hash64_seed(bytes: &[u8], seed: u64) -> u64 {
    let mut h = 0xcbf29ce484222325 ^ seed;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Tokenizes text into lowercase alphanumeric words and sub-tokens.
/// Handles camelCase, snake_case, kebab-case, dots, colons, and punctuation.
/// Filters common non-technical stop words.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current_word = String::new();

    let mut prev_char: Option<char> = None;

    for ch in text.chars() {
        if ch.is_alphanumeric() {
            // Check for camelCase boundary (e.g. "typeMismatch" -> "type", "Mismatch")
            if let Some(prev) = prev_char
                && prev.is_lowercase()
                && ch.is_uppercase()
                && !current_word.is_empty()
            {
                let word = current_word.to_lowercase();
                if !is_stop_word(&word) {
                    tokens.push(word);
                }
                current_word.clear();
            }
            current_word.push(ch);
            prev_char = Some(ch);
        } else {
            // Delimiter encountered (whitespace, punctuation, symbols)
            if !current_word.is_empty() {
                let word = current_word.to_lowercase();
                if !is_stop_word(&word) {
                    tokens.push(word);
                }
                current_word.clear();
            }
            prev_char = None;
        }
    }

    if !current_word.is_empty() {
        let word = current_word.to_lowercase();
        if !is_stop_word(&word) {
            tokens.push(word);
        }
    }

    tokens
}

fn is_stop_word(word: &str) -> bool {
    // Preserve single/double character diagnostic terms like "e0", error codes, numbers
    if word.len() < 2 && !word.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    STOP_WORDS.binary_search(&word).is_ok() || STOP_WORDS.contains(&word)
}

/// In-memory Okapi BM25 sparse inverted index.
#[derive(Clone, Debug)]
pub struct SparseBm25Index {
    /// Postings: term -> Vec<(doc_id, term_frequency)>
    postings: HashMap<String, Vec<(usize, u32)>>,
    /// Document lengths (total token count per doc_id)
    doc_lengths: HashMap<usize, usize>,
    /// Number of indexed documents
    doc_count: usize,
    /// Total token count across all documents
    total_tokens: usize,
}

impl Default for SparseBm25Index {
    fn default() -> Self {
        Self::new()
    }
}

impl SparseBm25Index {
    pub fn new() -> Self {
        Self {
            postings: HashMap::new(),
            doc_lengths: HashMap::new(),
            doc_count: 0,
            total_tokens: 0,
        }
    }

    /// Number of documents indexed.
    pub fn doc_count(&self) -> usize {
        self.doc_count
    }

    /// Adds a document to the index.
    pub fn add_document(&mut self, doc_id: usize, text: &str) {
        let tokens = tokenize(text);
        let len = tokens.len();
        self.doc_lengths.insert(doc_id, len);
        self.total_tokens += len;
        self.doc_count += 1;

        let mut term_freqs: HashMap<String, u32> = HashMap::new();
        for token in tokens {
            *term_freqs.entry(token).or_insert(0) += 1;
        }

        for (term, tf) in term_freqs {
            self.postings.entry(term).or_default().push((doc_id, tf));
        }
    }

    /// Computes the average document length across the corpus.
    pub fn avg_doc_length(&self) -> f32 {
        if self.doc_count == 0 {
            1.0
        } else {
            self.total_tokens as f32 / self.doc_count as f32
        }
    }

    /// Scores all documents against a query string.
    /// Returns a list of `(doc_id, bm25_score)` sorted in descending score order.
    pub fn score(&self, query: &str) -> Vec<(usize, f32)> {
        let query_tokens = tokenize(query);
        if query_tokens.is_empty() || self.doc_count == 0 {
            return Vec::new();
        }

        let avgdl = self.avg_doc_length();
        let mut scores: HashMap<usize, f32> = HashMap::new();

        let mut unique_query_tokens = query_tokens;
        unique_query_tokens.sort();
        unique_query_tokens.dedup();

        let n_docs = self.doc_count as f32;

        for q in unique_query_tokens {
            if let Some(postings_list) = self.postings.get(&q) {
                let n_q = postings_list.len() as f32;
                // Robertson-Spärck Jones IDF formula with +1.0 floor smoothing
                let idf = ((n_docs - n_q + 0.5) / (n_q + 0.5) + 1.0).ln().max(0.1);

                for &(doc_id, tf) in postings_list {
                    let doc_len = *self.doc_lengths.get(&doc_id).unwrap_or(&1) as f32;
                    let tf_f = tf as f32;
                    let denom = tf_f + BM25_K1 * (1.0 - BM25_B + BM25_B * (doc_len / avgdl));
                    let term_score = idf * (tf_f * (BM25_K1 + 1.0)) / denom;

                    *scores.entry(doc_id).or_insert(0.0) += term_score;
                }
            }
        }

        let mut scored_docs: Vec<(usize, f32)> = scores.into_iter().collect();
        scored_docs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored_docs
    }
}

/// Zero-network deterministic dense vectorizer via subword character n-grams and feature hashing.
#[derive(Clone, Debug, Default)]
pub struct DenseFeatureHasher {
    dimension: usize,
}

impl DenseFeatureHasher {
    pub fn new() -> Self {
        Self {
            dimension: DENSE_DIMENSION,
        }
    }

    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// Projects arbitrary text into a normalized dense vector of dimension `DENSE_DIMENSION` ($d = 128$).
    pub fn vectorize(&self, text: &str) -> [f32; DENSE_DIMENSION] {
        let mut vector = [0.0f32; DENSE_DIMENSION];
        let tokens = tokenize(text);
        if tokens.is_empty() {
            return vector;
        }

        for token in &tokens {
            // Apply domain diagnostic weight multiplier for error / recovery concepts
            let token_weight = match token.as_str() {
                "error" | "fail" | "failed" | "failure" | "panic" | "mismatch" | "diagnostic"
                | "diagnostics" | "syntax" | "assert" | "type" | "null" | "overflow"
                | "timeout" | "abort" | "verifier" | "sop" | "rule" | "lesson" => 2.5,
                _ => 1.5,
            };

            // Full token projection with word-boundary markers
            let bounded = format!("^{token}$");
            self.project_feature(&bounded, token_weight, &mut vector);

            // Character 3-grams and 4-grams for subword / typo robustness
            let chars: Vec<char> = token.chars().collect();
            if chars.len() >= 3 {
                for window in chars.windows(3) {
                    let gram: String = window.iter().collect();
                    self.project_feature(&gram, 0.8, &mut vector);
                }
            }
            if chars.len() >= 4 {
                for window in chars.windows(4) {
                    let gram: String = window.iter().collect();
                    self.project_feature(&gram, 0.8, &mut vector);
                }
            }
        }

        // L2 normalization to unit hypersphere
        let norm_sq: f32 = crate::arch_simd::simd_norm_sq_128(&vector);
        if norm_sq > 1e-9 {
            let norm = norm_sq.sqrt();
            for val in &mut vector {
                *val /= norm;
            }
        }

        vector
    }

    /// Projects a single feature string into the vector with signed dual-hash bucketing.
    fn project_feature(&self, feature: &str, weight: f32, vector: &mut [f32; DENSE_DIMENSION]) {
        let bytes = feature.as_bytes();
        let h1 = hash64_seed(bytes, 0xcbf29ce484222325);
        let h2 = hash64_seed(bytes, 0x9e3779b97f4a7c15);

        let index = (h1 % (self.dimension as u64)) as usize;
        let sign = if (h2 & 1) == 0 { 1.0f32 } else { -1.0f32 };

        vector[index] += weight * sign;
    }

    /// Computes cosine similarity between two dense vectors.
    /// Since vectors are L2-normalized, this is the dot product clamped to `[0.0, 1.0]`.
    pub fn cosine_similarity(&self, a: &[f32; DENSE_DIMENSION], b: &[f32; DENSE_DIMENSION]) -> f32 {
        let norm_a_sq: f32 = crate::arch_simd::simd_norm_sq_128(a);
        let norm_b_sq: f32 = crate::arch_simd::simd_norm_sq_128(b);
        if norm_a_sq < 1e-9 || norm_b_sq < 1e-9 {
            return 0.0;
        }

        let dot: f32 = crate::arch_simd::simd_dot_product_128(a, b);
        dot.clamp(0.0, 1.0)
    }

    /// Scores a list of indexed document vectors against a query vector.
    /// Returns `(doc_id, similarity)` sorted in descending order.
    pub fn score(
        &self,
        query_vec: &[f32; DENSE_DIMENSION],
        doc_vecs: &[(usize, [f32; DENSE_DIMENSION])],
    ) -> Vec<(usize, f32)> {
        let mut scores: Vec<(usize, f32)> = doc_vecs
            .iter()
            .map(|(doc_id, doc_vec)| (*doc_id, self.cosine_similarity(query_vec, doc_vec)))
            .collect();
        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scores
    }
}

/// Result of hybrid ranking fusion.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HybridRankItem {
    pub doc_id: usize,
    pub sparse_score: f32,
    pub dense_score: f32,
    pub hybrid_score: f32,
}

/// Reciprocal Rank Fusion (RRF) combiner for sparse and dense retrieval streams.
#[derive(Clone, Debug)]
pub struct HybridRrfRanker {
    rrf_k: f32,
}

impl Default for HybridRrfRanker {
    fn default() -> Self {
        Self::new()
    }
}

impl HybridRrfRanker {
    pub fn new() -> Self {
        Self { rrf_k: RRF_K }
    }

    pub fn with_k(rrf_k: f32) -> Self {
        Self { rrf_k }
    }

    /// Combines sparse rankings and dense rankings into unified RRF rankings:
    ///
    /// RRF_score(d) = 1 / (k + rank_sparse(d)) + 1 / (k + rank_dense(d))
    ///
    /// Only non-zero scoring contributions are ranked.
    pub fn fuse(
        &self,
        sparse_scores: &[(usize, f32)],
        dense_scores: &[(usize, f32)],
    ) -> Vec<HybridRankItem> {
        // doc_id -> (Option<sparse_rank>, Option<dense_rank>, sparse_score, dense_score)
        let mut map: HashMap<usize, (Option<usize>, Option<usize>, f32, f32)> = HashMap::new();

        for (rank_0, &(doc_id, score)) in sparse_scores.iter().enumerate() {
            if score > 0.0 {
                let entry = map.entry(doc_id).or_insert((None, None, 0.0, 0.0));
                entry.0 = Some(rank_0 + 1);
                entry.2 = score;
            }
        }

        for (rank_0, &(doc_id, score)) in dense_scores.iter().enumerate() {
            if score > 0.0 {
                let entry = map.entry(doc_id).or_insert((None, None, 0.0, 0.0));
                entry.1 = Some(rank_0 + 1);
                entry.3 = score;
            }
        }

        let mut results = Vec::new();
        for (doc_id, (s_rank, d_rank, s_score, d_score)) in map {
            let mut rrf_score = 0.0f32;
            if let Some(r) = s_rank {
                rrf_score += 1.0 / (self.rrf_k + r as f32);
            }
            if let Some(r) = d_rank {
                rrf_score += 1.0 / (self.rrf_k + r as f32);
            }

            results.push(HybridRankItem {
                doc_id,
                sparse_score: s_score,
                dense_score: d_score,
                hybrid_score: rrf_score,
            });
        }

        results.sort_by(|a, b| {
            b.hybrid_score
                .partial_cmp(&a.hybrid_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results
    }
}

/// A search result item returned from `ContextVfs::search`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchResultItem {
    pub uri: ContextUri,
    pub title: String,
    pub snippet: String,
    pub sparse_score: f32,
    pub dense_score: f32,
    pub hybrid_score: f32,
    pub tier: Option<MemoryTier>,
    pub intent: Option<MemoryIntent>,
}

/// Alias for `SearchResultItem`.
pub type HybridSearchResult = SearchResultItem;

/// Resolved error pattern lesson retrieved from session memory (`eval-lesson-*`).
#[derive(Clone, Debug, PartialEq)]
pub struct LessonRecord {
    pub memory_id: String,
    pub execution_id: Option<String>,
    pub failed_kinds: Vec<String>,
    pub summary: String,
    pub relevance_score: f32,
    pub created_at: Timestamp,
    pub record: MemoryRecord,
}

impl LessonRecord {
    /// Formats the lesson record as a JSON value.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "memory_id": self.memory_id,
            "execution_id": self.execution_id,
            "failed_kinds": self.failed_kinds,
            "summary": self.summary,
            "relevance_score": self.relevance_score,
            "created_at": self.created_at.as_unix_seconds(),
            "tier": self.record.tier().as_str(),
            "kind": self.record.kind().as_str(),
            "classification": self.record.classification().as_str(),
            "provenance": self.record.provenance(),
        })
    }
}

/// Options to customize a `ContextVfs` search query.
#[derive(Clone, Debug, PartialEq)]
pub struct ContextSearchOptions {
    pub tier: Option<MemoryTier>,
    pub intent: Option<MemoryIntent>,
    pub limit: usize,
    pub min_score: f32,
}

impl Default for ContextSearchOptions {
    fn default() -> Self {
        Self {
            tier: None,
            intent: None,
            limit: 10,
            min_score: 0.0,
        }
    }
}

impl ContextSearchOptions {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            ..Default::default()
        }
    }

    pub fn with_tier(mut self, tier: MemoryTier) -> Self {
        self.tier = Some(tier);
        self
    }

    pub fn with_intent(mut self, intent: MemoryIntent) -> Self {
        self.intent = Some(intent);
        self
    }

    pub fn with_min_score(mut self, min_score: f32) -> Self {
        self.min_score = min_score;
        self
    }
}

/// Generates a relevant text snippet centered around query token matches.
pub fn generate_snippet(content: &str, query: &str) -> String {
    let query_tokens = tokenize(query);
    let lower_content = content.to_lowercase();
    let mut match_pos = None;

    for token in query_tokens {
        if let Some(pos) = lower_content.find(&token) {
            match_pos = Some(pos);
            break;
        }
    }

    let clean_content: String = content
        .chars()
        .map(|c| {
            if c == '\n' || c == '\r' || c == '\t' {
                ' '
            } else {
                c
            }
        })
        .collect();

    let len = clean_content.len();
    if len <= 160 {
        return clean_content.trim().to_string();
    }

    match match_pos {
        Some(pos) => {
            let raw_start = pos.saturating_sub(40);
            let raw_end = (pos + 120).min(len);

            let start = safe_boundary(&clean_content, raw_start, true);
            let end = safe_boundary(&clean_content, raw_end, false);

            let slice = clean_content[start..end].trim();
            if start > 0 && end < len {
                format!("...{slice}...")
            } else if start > 0 {
                format!("...{slice}")
            } else if end < len {
                format!("{slice}...")
            } else {
                slice.to_string()
            }
        }
        None => {
            let end = safe_boundary(&clean_content, 160.min(len), false);
            let slice = clean_content[..end].trim();
            if end < len {
                format!("{slice}...")
            } else {
                slice.to_string()
            }
        }
    }
}

fn safe_boundary(s: &str, mut idx: usize, floor: bool) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    while idx > 0 && !s.is_char_boundary(idx) {
        if floor {
            idx -= 1;
        } else {
            idx += 1;
            if idx >= s.len() {
                return s.len();
            }
        }
    }
    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenization_subwords_and_stop_words() {
        let text = "eval-lesson-101: failed evaluations: compiler diagnostics typeMismatch E0308";
        let tokens = tokenize(text);
        assert!(tokens.contains(&"eval".to_string()));
        assert!(tokens.contains(&"lesson".to_string()));
        assert!(tokens.contains(&"failed".to_string()));
        assert!(tokens.contains(&"evaluations".to_string()));
        assert!(tokens.contains(&"compiler".to_string()));
        assert!(tokens.contains(&"diagnostics".to_string()));
        assert!(tokens.contains(&"type".to_string()));
        assert!(tokens.contains(&"mismatch".to_string()));
        assert!(tokens.contains(&"e0308".to_string()));
    }

    #[test]
    fn test_sparse_bm25_index_exact_scoring() {
        let mut index = SparseBm25Index::new();
        index.add_document(0, "compiler diagnostics type mismatch error E0308 in rust");
        index.add_document(1, "runtime network timeout during fetch execution");
        index.add_document(2, "git worktree isolation squash merge procedure");

        let results = index.score("type mismatch E0308");
        assert!(!results.is_empty());
        assert_eq!(results[0].0, 0); // doc 0 must be top rank
        assert!(results[0].1 > 0.0);

        let net_results = index.score("timeout network");
        assert!(!net_results.is_empty());
        assert_eq!(net_results[0].0, 1);
    }

    #[test]
    fn test_dense_feature_hasher_deterministic_vector() {
        let hasher = DenseFeatureHasher::new();
        let vec1 = hasher.vectorize("compiler diagnostics type mismatch");
        let vec2 = hasher.vectorize("compiler diagnostics type mismatch");
        assert_eq!(vec1, vec2);

        // Vector must be normalized (norm ~ 1.0)
        let norm_sq: f32 = vec1.iter().map(|&x| x * x).sum();
        assert!((norm_sq - 1.0).abs() < 1e-4);

        // Cosine similarity with identical text is 1.0
        let sim = hasher.cosine_similarity(&vec1, &vec2);
        assert!((sim - 1.0).abs() < 1e-4);
    }

    #[test]
    fn test_dense_feature_hasher_subword_resilience() {
        let hasher = DenseFeatureHasher::new();
        // Typographical and stem variation: "mismatch" vs "mismatched", "diagnostics" vs "diagnostic"
        let vec_query = hasher.vectorize("compiler diagnostics type mismatch");
        let vec_doc_variant = hasher.vectorize("compiler diagnostic mismatched types");
        let vec_unrelated = hasher.vectorize("isolated ephemeral git branch squash merge");

        let sim_similar = hasher.cosine_similarity(&vec_query, &vec_doc_variant);
        let sim_unrelated = hasher.cosine_similarity(&vec_query, &vec_unrelated);

        assert!(
            sim_similar > sim_unrelated,
            "similar semantic variant ({sim_similar}) must score higher than unrelated ({sim_unrelated})"
        );
        assert!(sim_similar > 0.4);
    }

    #[test]
    fn test_rrf_hybrid_fusion() {
        let ranker = HybridRrfRanker::new();
        // Sparse: doc 0 is rank 1, doc 1 is rank 2
        let sparse = vec![(0, 2.5), (1, 1.2)];
        // Dense: doc 0 is rank 1, doc 2 is rank 2
        let dense = vec![(0, 0.9), (2, 0.7)];

        let fused = ranker.fuse(&sparse, &dense);
        assert_eq!(fused[0].doc_id, 0); // doc 0 ranked 1st in both -> highest RRF score
        let expected_doc0_rrf = (1.0 / (60.0 + 1.0)) + (1.0 / (60.0 + 1.0));
        assert!((fused[0].hybrid_score - expected_doc0_rrf).abs() < 1e-5);
    }

    #[test]
    fn test_generate_snippet() {
        let content = "The system constitutional rule requires architectural integrity. All code changes must be verified with tests and strict bounds.";
        let snippet = generate_snippet(content, "architectural integrity");
        assert!(snippet.contains("architectural integrity"));
    }
}
