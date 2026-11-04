//! ContextVfs HNSW (Hierarchical Navigable Small World) Graph Indexing Engine.
//!
//! Provides logarithmic $O(\log N)$ approximate nearest neighbor search over
//! dense 128-dimensional vectors (both uncompressed `[f32; 128]` and Int8 `QuantizedVector`),
//! dynamic triangular diversity neighbor pruning, and PVHN v1 binary persistence
//! with SHA-256 integrity validation.

#![forbid(unsafe_code)]

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashSet};
use std::fmt;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::arch_simd::{simd_dot_product_128, simd_dot_product_f32_i8, simd_dot_product_i8};
use crate::semantic_memory::{QuantizedVector, DENSE_DIMENSION};

pub const PVHN_MAGIC: &[u8; 4] = b"PVHN";
pub const PVHN_VERSION: u32 = 1;
pub const PVHN_HEADER_SIZE: usize = 80;
pub const PVHN_TRAILER_SIZE: usize = 32;

pub const DEFAULT_HNSW_M: usize = 16;
pub const DEFAULT_HNSW_M_MAX_0: usize = 32;
pub const DEFAULT_HNSW_EF_CONSTRUCTION: usize = 64;
pub const DEFAULT_HNSW_EF_SEARCH: usize = 64;

/// Vector representation type for HNSW serialization headers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum HnswVectorType {
    Float32 = 1,
    QuantizedInt8 = 2,
    Mixed = 3,
}

/// Errors occurring during HNSW operations, validation, or PVHN I/O.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HnswError {
    InvalidMagic,
    UnsupportedVersion(u32),
    DimensionMismatch(u32),
    ChecksumMismatch,
    TruncatedData,
    InvalidData(String),
}

impl fmt::Display for HnswError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic => write!(f, "invalid PVHN magic bytes"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported PVHN format version: {v}"),
            Self::DimensionMismatch(d) => {
                write!(f, "vector dimension mismatch: {d} (expected {DENSE_DIMENSION})")
            }
            Self::ChecksumMismatch => {
                write!(f, "PVHN SHA-256 checksum mismatch (corrupted index)")
            }
            Self::TruncatedData => write!(f, "PVHN binary payload truncated"),
            Self::InvalidData(msg) => write!(f, "invalid PVHN data: {msg}"),
        }
    }
}

impl std::error::Error for HnswError {}

/// Configuration parameters for HNSW index construction and query routing.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct HnswConfig {
    pub m: usize,
    pub m_max_0: usize,
    pub ef_construction: usize,
    pub ef_search: usize,
    pub ml: f64,
}

impl Default for HnswConfig {
    fn default() -> Self {
        let m = DEFAULT_HNSW_M;
        Self {
            m,
            m_max_0: DEFAULT_HNSW_M_MAX_0,
            ef_construction: DEFAULT_HNSW_EF_CONSTRUCTION,
            ef_search: DEFAULT_HNSW_EF_SEARCH,
            ml: 1.0 / (m as f64).ln(),
        }
    }
}

impl HnswConfig {
    /// Constructs a new configuration with explicit graph parameters.
    pub fn new(m: usize, ef_construction: usize, ef_search: usize) -> Self {
        let m_clamped = m.max(2);
        Self {
            m: m_clamped,
            m_max_0: m_clamped * 2,
            ef_construction: ef_construction.max(m_clamped),
            ef_search: ef_search.max(1),
            ml: 1.0 / (m_clamped as f64).ln(),
        }
    }

    pub fn with_m(mut self, m: usize) -> Self {
        let m_clamped = m.max(2);
        self.m = m_clamped;
        self.m_max_0 = m_clamped * 2;
        self.ml = 1.0 / (m_clamped as f64).ln();
        self
    }

    pub fn with_m_max_0(mut self, m_max_0: usize) -> Self {
        self.m_max_0 = m_max_0.max(self.m);
        self
    }

    pub fn with_ef_construction(mut self, ef: usize) -> Self {
        self.ef_construction = ef.max(self.m);
        self
    }

    pub fn with_ef_search(mut self, ef: usize) -> Self {
        self.ef_search = ef.max(1);
        self
    }

    /// Accessor for layer 0 maximum degree $M_{\max 0}$.
    pub fn m0(&self) -> usize {
        self.m_max_0
    }
}

/// Dual vector representation supporting uncompressed `[f32; 128]` and Int8 `QuantizedVector`.
#[derive(Clone, Debug, PartialEq)]
pub enum HnswVector {
    F32([f32; 128]),
    Quantized(QuantizedVector),
}

impl HnswVector {
    /// Converts or dequantizes into an f32 array.
    pub fn as_f32(&self) -> [f32; 128] {
        match self {
            Self::F32(v) => *v,
            Self::Quantized(q) => q.dequantize(),
        }
    }

    /// Converts or quantizes into a `QuantizedVector`.
    pub fn to_quantized(&self) -> QuantizedVector {
        match self {
            Self::F32(v) => QuantizedVector::quantize(v),
            Self::Quantized(q) => *q,
        }
    }

    pub fn is_quantized(&self) -> bool {
        matches!(self, Self::Quantized(_))
    }
}

impl From<[f32; 128]> for HnswVector {
    fn from(v: [f32; 128]) -> Self {
        Self::F32(v)
    }
}

impl From<QuantizedVector> for HnswVector {
    fn from(q: QuantizedVector) -> Self {
        Self::Quantized(q)
    }
}

/// Computes the Cosine Distance $D(\mathbf{u}, \mathbf{v}) = 1.0 - \text{sim}(\mathbf{u}, \mathbf{v})$.
///
/// Bounded and clamped to `[0.0, 2.0]`. Utilizes safe SIMD dot products from `crate::arch_simd`.
pub fn cosine_distance(a: &HnswVector, b: &HnswVector) -> f32 {
    if a == b {
        return 0.0;
    }
    match (a, b) {
        (HnswVector::F32(va), HnswVector::F32(vb)) => {
            let dot = simd_dot_product_128(va, vb);
            (1.0 - dot).clamp(0.0, 2.0)
        }
        (HnswVector::Quantized(qa), HnswVector::Quantized(qb)) => {
            if qa.scale < 1e-9 || qb.scale < 1e-9 {
                return 1.0;
            }
            let int_dot = simd_dot_product_i8(&qa.data, &qb.data);
            let sim = (int_dot as f32) * qa.scale * qb.scale;
            (1.0 - sim).clamp(0.0, 2.0)
        }
        (HnswVector::F32(va), HnswVector::Quantized(qb)) => {
            let sim = simd_dot_product_f32_i8(va, &qb.data, qb.scale);
            (1.0 - sim).clamp(0.0, 2.0)
        }
        (HnswVector::Quantized(qa), HnswVector::F32(vb)) => {
            let sim = simd_dot_product_f32_i8(vb, &qa.data, qa.scale);
            (1.0 - sim).clamp(0.0, 2.0)
        }
    }
}

/// Item returned from an HNSW similarity search query.
#[derive(Clone, Debug, PartialEq)]
pub struct HnswSearchResult {
    pub doc_id: usize,
    pub distance: f32,
    pub similarity: f32,
}

impl HnswSearchResult {
    pub fn new(doc_id: usize, distance: f32, similarity: f32) -> Self {
        Self {
            doc_id,
            distance,
            similarity,
        }
    }
}

/// A node in the multi-layer HNSW graph.
#[derive(Clone, Debug, PartialEq)]
pub struct HnswNode {
    pub id: usize,
    pub doc_id: usize,
    pub level: usize,
    pub vector: HnswVector,
    /// neighbors[l] is the adjacency list of neighbor node IDs at layer l ($0 \le l \le \text{level}$).
    pub neighbors: Vec<Vec<usize>>,
}

/// Safe pseudo-random number generator for deterministic level generation.
#[derive(Clone, Debug, PartialEq)]
pub struct SafeRng {
    state: u64,
}

impl SafeRng {
    pub fn new(seed: u64) -> Self {
        let state = if seed == 0 { 0x853c49e6748fea9b } else { seed };
        Self { state }
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    /// Generates a uniform float in $(0.0, 1.0]$.
    pub fn next_f64(&mut self) -> f64 {
        let val = (self.next_u64() >> 11) as f64;
        let u = (val + 1.0) / 9007199254740992.0;
        u.clamp(1e-15, 1.0)
    }

    /// Samples node level $l = \lfloor -\ln(U) \cdot m_L \rfloor$.
    pub fn generate_level(&mut self, ml: f64) -> usize {
        let u = self.next_f64();
        let level = (-u.ln() * ml).floor();
        if level < 0.0 {
            0
        } else {
            level as usize
        }
    }
}

/// Priority queue item for min-heap (closest candidate popped first).
#[derive(Clone, Copy, Debug, PartialEq)]
struct MinCandidate {
    node_id: usize,
    distance: f32,
}

impl Eq for MinCandidate {}

impl Ord for MinCandidate {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse ordering so smaller distance has higher priority
        other
            .distance
            .partial_cmp(&self.distance)
            .unwrap_or(Ordering::Equal)
            .then_with(|| other.node_id.cmp(&self.node_id))
    }
}

impl PartialOrd for MinCandidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Priority queue item for max-heap (furthest result popped first).
#[derive(Clone, Copy, Debug, PartialEq)]
struct MaxCandidate {
    node_id: usize,
    distance: f32,
}

impl Eq for MaxCandidate {}

impl Ord for MaxCandidate {
    fn cmp(&self, other: &Self) -> Ordering {
        self.distance
            .partial_cmp(&other.distance)
            .unwrap_or(Ordering::Equal)
            .then_with(|| self.node_id.cmp(&other.node_id))
    }
}

impl PartialOrd for MaxCandidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Internal candidate item for neighbor selection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CandidateItem {
    pub node_id: usize,
    pub distance: f32,
}

/// Multi-layer Hierarchical Navigable Small World (HNSW) graph index.
#[derive(Clone, Debug, PartialEq)]
pub struct HnswIndex {
    config: HnswConfig,
    entry_point: Option<usize>,
    max_level: usize,
    nodes: Vec<HnswNode>,
    doc_ids: Vec<usize>,
    rng: SafeRng,
    corpus_hash: [u8; 32],
    created_at_unix: u64,
}

impl HnswIndex {
    /// Creates a new, empty HNSW index with the provided configuration.
    pub fn new(config: HnswConfig) -> Self {
        let created_at_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            config,
            entry_point: None,
            max_level: 0,
            nodes: Vec::new(),
            doc_ids: Vec::new(),
            rng: SafeRng::new(0x2468ace0_13579bdf),
            corpus_hash: [0u8; 32],
            created_at_unix,
        }
    }

    /// Sets the PRNG seed for deterministic graph construction.
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.rng = SafeRng::new(seed);
        self
    }

    /// Sets the corpus SHA-256 fingerprint associated with this index.
    pub fn set_corpus_hash(&mut self, hash: [u8; 32]) {
        self.corpus_hash = hash;
    }

    pub fn corpus_hash(&self) -> &[u8; 32] {
        &self.corpus_hash
    }

    pub fn config(&self) -> &HnswConfig {
        &self.config
    }

    pub fn entry_point(&self) -> Option<usize> {
        self.entry_point
    }

    pub fn max_level(&self) -> usize {
        self.max_level
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn nodes(&self) -> &[HnswNode] {
        &self.nodes
    }

    pub fn doc_ids(&self) -> &[usize] {
        &self.doc_ids
    }

    pub fn node(&self, idx: usize) -> Option<&HnswNode> {
        self.nodes.get(idx)
    }

    #[inline]
    fn distance_to_node(&self, query: &HnswVector, node_id: usize) -> f32 {
        cosine_distance(query, &self.nodes[node_id].vector)
    }

    #[inline]
    fn distance_between_nodes(&self, a: usize, b: usize) -> f32 {
        if a == b {
            0.0
        } else {
            cosine_distance(&self.nodes[a].vector, &self.nodes[b].vector)
        }
    }

    /// Greedy single-neighbor descent at layer `level`.
    fn greedy_search_level(&self, query: &HnswVector, mut curr: usize, level: usize) -> usize {
        let mut curr_dist = self.distance_to_node(query, curr);
        loop {
            let mut changed = false;
            let neighbors = &self.nodes[curr].neighbors[level];
            for &neighbor in neighbors {
                let d = self.distance_to_node(query, neighbor);
                if d < curr_dist {
                    curr_dist = d;
                    curr = neighbor;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        curr
    }

    /// Bounded beam search (`SEARCH-LAYER`) finding `ef` nearest candidates at layer `level`.
    fn search_layer(
        &self,
        query: &HnswVector,
        enter_points: &[usize],
        ef: usize,
        level: usize,
    ) -> Vec<CandidateItem> {
        let mut visited = HashSet::new();
        let mut candidates = BinaryHeap::new();
        let mut results = BinaryHeap::new();

        for &ep in enter_points {
            if ep < self.nodes.len() && visited.insert(ep) {
                let d = self.distance_to_node(query, ep);
                candidates.push(MinCandidate {
                    node_id: ep,
                    distance: d,
                });
                results.push(MaxCandidate {
                    node_id: ep,
                    distance: d,
                });
            }
        }

        while let Some(c) = candidates.pop() {
            if let Some(furthest) = results.peek() {
                if c.distance > furthest.distance && results.len() >= ef {
                    break;
                }
            }

            if c.node_id >= self.nodes.len() || level >= self.nodes[c.node_id].neighbors.len() {
                continue;
            }

            let neighbors = &self.nodes[c.node_id].neighbors[level];
            for &neighbor in neighbors {
                if neighbor < self.nodes.len() && visited.insert(neighbor) {
                    let d = self.distance_to_node(query, neighbor);
                    let should_add = if results.len() < ef {
                        true
                    } else if let Some(furthest) = results.peek() {
                        d < furthest.distance
                    } else {
                        false
                    };

                    if should_add {
                        candidates.push(MinCandidate {
                            node_id: neighbor,
                            distance: d,
                        });
                        results.push(MaxCandidate {
                            node_id: neighbor,
                            distance: d,
                        });
                        if results.len() > ef {
                            results.pop();
                        }
                    }
                }
            }
        }

        let mut out: Vec<CandidateItem> = results
            .into_iter()
            .map(|item| CandidateItem {
                node_id: item.node_id,
                distance: item.distance,
            })
            .collect();
        out.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap_or(Ordering::Equal));
        out
    }

    /// Dynamic Neighbor Selection Heuristic (`SELECT-NEIGHBORS-HEURISTIC`).
    ///
    /// Applies triangular diversity pruning: candidate $e$ is accepted only if
    /// $\forall r \in R: \text{dist}(\text{base}, e) < \text{dist}(e, r)$.
    fn select_neighbors_heuristic(
        &self,
        base_id: usize,
        candidates: &[CandidateItem],
        m_max: usize,
        _level: usize,
        keep_pruned: bool,
    ) -> Vec<usize> {
        let mut sorted = candidates.to_vec();
        sorted.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap_or(Ordering::Equal));

        let mut selected: Vec<usize> = Vec::with_capacity(m_max);
        let mut discarded: Vec<usize> = Vec::new();

        for cand in sorted {
            if cand.node_id == base_id || selected.contains(&cand.node_id) {
                continue;
            }

            let mut is_diverse = true;
            for &r in &selected {
                let dist_e_r = self.distance_between_nodes(cand.node_id, r);
                if dist_e_r < cand.distance {
                    is_diverse = false;
                    break;
                }
            }

            if is_diverse {
                selected.push(cand.node_id);
                if selected.len() >= m_max {
                    break;
                }
            } else {
                discarded.push(cand.node_id);
            }
        }

        if keep_pruned && selected.len() < m_max {
            for disc in discarded {
                if !selected.contains(&disc) {
                    selected.push(disc);
                    if selected.len() >= m_max {
                        break;
                    }
                }
            }
        }

        selected
    }

    /// Inserts a vector with corresponding document ID into the HNSW graph.
    pub fn insert(&mut self, doc_id: usize, vector: HnswVector) {
        let l_q = self.rng.generate_level(self.config.ml);
        let q_id = self.nodes.len();

        if self.nodes.is_empty() {
            let mut neighbors = Vec::with_capacity(l_q + 1);
            for _ in 0..=l_q {
                neighbors.push(Vec::new());
            }
            let node = HnswNode {
                id: q_id,
                doc_id,
                level: l_q,
                vector,
                neighbors,
            };
            self.nodes.push(node);
            self.doc_ids.push(doc_id);
            self.entry_point = Some(q_id);
            self.max_level = l_q;
            return;
        }

        let mut curr = self.entry_point.unwrap();
        let max_l = self.max_level;

        // Phase 1: Skip routing through upper layers above l_q
        if max_l > l_q {
            for l in ((l_q + 1)..=max_l).rev() {
                curr = self.greedy_search_level(&vector, curr, l);
            }
        }

        // Initialize new node
        let mut neighbors = Vec::with_capacity(l_q + 1);
        for _ in 0..=l_q {
            neighbors.push(Vec::new());
        }
        let node = HnswNode {
            id: q_id,
            doc_id,
            level: l_q,
            vector: vector.clone(),
            neighbors,
        };
        self.nodes.push(node);
        self.doc_ids.push(doc_id);

        // Phase 2: Connect into layers min(max_l, l_q) down to 0
        let target_top = std::cmp::min(max_l, l_q);
        let mut ep = vec![curr];

        for l in (0..=target_top).rev() {
            let m_l = if l == 0 { self.config.m_max_0 } else { self.config.m };
            let candidates = self.search_layer(&vector, &ep, self.config.ef_construction, l);
            let selected_neighbors =
                self.select_neighbors_heuristic(q_id, &candidates, m_l, l, true);

            self.nodes[q_id].neighbors[l] = selected_neighbors.clone();

            for &n in &selected_neighbors {
                if !self.nodes[n].neighbors[l].contains(&q_id) {
                    self.nodes[n].neighbors[l].push(q_id);
                }

                if self.nodes[n].neighbors[l].len() > m_l {
                    let cand_items: Vec<CandidateItem> = self.nodes[n].neighbors[l]
                        .iter()
                        .map(|&x| CandidateItem {
                            node_id: x,
                            distance: self.distance_between_nodes(n, x),
                        })
                        .collect();
                    self.nodes[n].neighbors[l] =
                        self.select_neighbors_heuristic(n, &cand_items, m_l, l, true);
                }
            }

            ep = candidates.into_iter().map(|c| c.node_id).collect();
            if ep.is_empty() {
                ep = vec![curr];
            }
        }

        // Phase 3: Update entry point if newly assigned level is strictly higher
        if l_q > max_l {
            self.max_level = l_q;
            self.entry_point = Some(q_id);
        }
    }

    /// Performs approximate nearest neighbor search for `top_k` results.
    pub fn search(
        &self,
        query: &HnswVector,
        top_k: usize,
        ef_search: usize,
    ) -> Vec<HnswSearchResult> {
        if self.nodes.is_empty() || top_k == 0 {
            return Vec::new();
        }

        let ef = ef_search.max(top_k);
        let mut curr = match self.entry_point {
            Some(ep) => ep,
            None => return Vec::new(),
        };

        // Skip routing through upper layers down to 1
        if self.max_level > 0 {
            for l in (1..=self.max_level).rev() {
                curr = self.greedy_search_level(query, curr, l);
            }
        }

        // Layer 0 bounded beam search
        let candidates = self.search_layer(query, &[curr], ef, 0);

        let limit = top_k.min(candidates.len());
        candidates[..limit]
            .iter()
            .map(|item| {
                let doc_id = self.nodes[item.node_id].doc_id;
                let distance = item.distance;
                let similarity = (1.0 - distance).clamp(0.0, 1.0);
                HnswSearchResult {
                    doc_id,
                    distance,
                    similarity,
                }
            })
            .collect()
    }

    /// Serializes the HNSW graph into the binary `PVHN` v1 format.
    pub fn to_pvhn_bytes(&self) -> Result<Vec<u8>, HnswError> {
        let mut buf = Vec::new();

        let vector_type = if self.nodes.is_empty() {
            HnswVectorType::QuantizedInt8 as u8
        } else {
            let mut has_f32 = false;
            let mut has_q = false;
            for n in &self.nodes {
                match n.vector {
                    HnswVector::F32(_) => has_f32 = true,
                    HnswVector::Quantized(_) => has_q = true,
                }
            }
            if has_f32 && has_q {
                HnswVectorType::Mixed as u8
            } else if has_f32 {
                HnswVectorType::Float32 as u8
            } else {
                HnswVectorType::QuantizedInt8 as u8
            }
        };

        // 1. Fixed Header (80 Bytes)
        buf.extend_from_slice(PVHN_MAGIC);
        buf.extend_from_slice(&PVHN_VERSION.to_le_bytes());
        buf.extend_from_slice(&self.created_at_unix.to_le_bytes());
        buf.extend_from_slice(&(self.nodes.len() as u32).to_le_bytes());
        buf.extend_from_slice(&(DENSE_DIMENSION as u32).to_le_bytes());
        buf.push(vector_type);
        buf.push(0u8); // flags
        buf.extend_from_slice(&(self.config.m as u16).to_le_bytes());
        buf.extend_from_slice(&(self.config.m_max_0 as u16).to_le_bytes());
        buf.extend_from_slice(&(self.config.ef_construction as u32).to_le_bytes());
        buf.extend_from_slice(&(self.config.ml as f32).to_le_bytes());
        buf.extend_from_slice(&(self.max_level as u32).to_le_bytes());
        let ep_val = match self.entry_point {
            Some(ep) => ep as u32,
            None => u32::MAX,
        };
        buf.extend_from_slice(&ep_val.to_le_bytes());
        buf.extend_from_slice(&self.corpus_hash);
        buf.extend_from_slice(&[0u8, 0u8]); // reserved

        debug_assert_eq!(buf.len(), PVHN_HEADER_SIZE);

        // 2. Node Metadata Table
        for node in &self.nodes {
            buf.extend_from_slice(&(node.doc_id as u64).to_le_bytes());
            buf.extend_from_slice(&(node.level as u32).to_le_bytes());
        }

        // 3. Dense Vector Storage Table
        for node in &self.nodes {
            match &node.vector {
                HnswVector::F32(v) => {
                    buf.push(1u8);
                    for &elem in v {
                        buf.extend_from_slice(&elem.to_le_bytes());
                    }
                }
                HnswVector::Quantized(q) => {
                    buf.push(2u8);
                    buf.extend_from_slice(&q.scale.to_le_bytes());
                    for &byte in &q.data {
                        buf.push(byte as u8);
                    }
                }
            }
        }

        // 4. Multi-Layer Adjacency Lists
        for node in &self.nodes {
            let num_layers = node.neighbors.len() as u32;
            buf.extend_from_slice(&num_layers.to_le_bytes());
            for layer_neighbors in &node.neighbors {
                let degree = layer_neighbors.len() as u32;
                buf.extend_from_slice(&degree.to_le_bytes());
                for &neighbor_id in layer_neighbors {
                    buf.extend_from_slice(&(neighbor_id as u32).to_le_bytes());
                }
            }
        }

        // 5. Trailer (32 Bytes SHA-256)
        let checksum = Sha256::digest(&buf);
        buf.extend_from_slice(&checksum);

        Ok(buf)
    }

    /// Deserializes an HNSW graph index from binary `PVHN` v1 bytes.
    pub fn from_pvhn_bytes(bytes: &[u8]) -> Result<Self, HnswError> {
        if bytes.len() < PVHN_HEADER_SIZE + PVHN_TRAILER_SIZE {
            return Err(HnswError::TruncatedData);
        }

        let payload_len = bytes.len() - PVHN_TRAILER_SIZE;
        let expected_trailer = &bytes[payload_len..];
        let computed = Sha256::digest(&bytes[..payload_len]);
        if computed.as_slice() != expected_trailer {
            return Err(HnswError::ChecksumMismatch);
        }

        // 1. Header Validation (80 Bytes)
        if &bytes[0..4] != PVHN_MAGIC {
            return Err(HnswError::InvalidMagic);
        }

        let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        if version != PVHN_VERSION {
            return Err(HnswError::UnsupportedVersion(version));
        }

        let created_at_unix = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let node_count = u32::from_le_bytes(bytes[16..20].try_into().unwrap()) as usize;
        let dense_dim = u32::from_le_bytes(bytes[20..24].try_into().unwrap());
        if dense_dim != DENSE_DIMENSION as u32 {
            return Err(HnswError::DimensionMismatch(dense_dim));
        }

        let _vector_type = bytes[24];
        let _flags = bytes[25];
        let m = u16::from_le_bytes(bytes[26..28].try_into().unwrap()) as usize;
        let m_max_0 = u16::from_le_bytes(bytes[28..30].try_into().unwrap()) as usize;
        let ef_construction = u32::from_le_bytes(bytes[30..34].try_into().unwrap()) as usize;
        let ml = f32::from_le_bytes(bytes[34..38].try_into().unwrap()) as f64;
        let max_level = u32::from_le_bytes(bytes[38..42].try_into().unwrap()) as usize;
        let ep_val = u32::from_le_bytes(bytes[42..46].try_into().unwrap());
        let entry_point = if ep_val == u32::MAX {
            None
        } else {
            Some(ep_val as usize)
        };
        let mut corpus_hash = [0u8; 32];
        corpus_hash.copy_from_slice(&bytes[46..78]);

        let config = HnswConfig {
            m,
            m_max_0,
            ef_construction,
            ef_search: DEFAULT_HNSW_EF_SEARCH,
            ml,
        };

        let mut offset = PVHN_HEADER_SIZE;

        // 2. Node Metadata Table
        struct NodeMeta {
            doc_id: usize,
            level: usize,
        }
        let mut metas = Vec::with_capacity(node_count);
        for _ in 0..node_count {
            if offset + 12 > payload_len {
                return Err(HnswError::TruncatedData);
            }
            let doc_id = u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap()) as usize;
            offset += 8;
            let level = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
            offset += 4;
            metas.push(NodeMeta { doc_id, level });
        }

        // 3. Dense Vector Storage Table
        let mut vectors = Vec::with_capacity(node_count);
        for _ in 0..node_count {
            if offset >= payload_len {
                return Err(HnswError::TruncatedData);
            }
            let tag = bytes[offset];
            offset += 1;
            match tag {
                1 => {
                    if offset + 512 > payload_len {
                        return Err(HnswError::TruncatedData);
                    }
                    let mut data = [0.0f32; 128];
                    for i in 0..128 {
                        let f_bytes = &bytes[offset..offset + 4];
                        data[i] = f32::from_le_bytes(f_bytes.try_into().unwrap());
                        offset += 4;
                    }
                    vectors.push(HnswVector::F32(data));
                }
                2 => {
                    if offset + 132 > payload_len {
                        return Err(HnswError::TruncatedData);
                    }
                    let scale = f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
                    offset += 4;
                    let mut data = [0i8; 128];
                    for i in 0..128 {
                        data[i] = bytes[offset] as i8;
                        offset += 1;
                    }
                    vectors.push(HnswVector::Quantized(QuantizedVector { scale, data }));
                }
                other => {
                    return Err(HnswError::InvalidData(format!(
                        "unknown vector tag in PVHN: {other}"
                    )));
                }
            }
        }

        // 4. Multi-Layer Adjacency Lists
        let mut all_neighbors = Vec::with_capacity(node_count);
        for _ in 0..node_count {
            if offset + 4 > payload_len {
                return Err(HnswError::TruncatedData);
            }
            let num_layers =
                u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
            offset += 4;
            let mut layers = Vec::with_capacity(num_layers);
            for _ in 0..num_layers {
                if offset + 4 > payload_len {
                    return Err(HnswError::TruncatedData);
                }
                let degree =
                    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
                offset += 4;
                if offset + degree * 4 > payload_len {
                    return Err(HnswError::TruncatedData);
                }
                let mut layer_neighbors = Vec::with_capacity(degree);
                for _ in 0..degree {
                    let nb =
                        u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
                    offset += 4;
                    layer_neighbors.push(nb);
                }
                layers.push(layer_neighbors);
            }
            all_neighbors.push(layers);
        }

        if offset != payload_len {
            return Err(HnswError::InvalidData(format!(
                "trailing payload bytes: {offset} != {payload_len}"
            )));
        }

        let mut nodes = Vec::with_capacity(node_count);
        let mut doc_ids = Vec::with_capacity(node_count);
        for i in 0..node_count {
            let meta = &metas[i];
            doc_ids.push(meta.doc_id);
            nodes.push(HnswNode {
                id: i,
                doc_id: meta.doc_id,
                level: meta.level,
                vector: vectors.remove(0),
                neighbors: all_neighbors.remove(0),
            });
        }

        Ok(Self {
            config,
            entry_point,
            max_level,
            nodes,
            doc_ids,
            rng: SafeRng::new(0x2468ace0_13579bdf),
            corpus_hash,
            created_at_unix,
        })
    }

    /// Atomically persists the HNSW index to disk at `path`.
    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<(), HnswError> {
        let bytes = self.to_pvhn_bytes()?;
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| HnswError::InvalidData(e.to_string()))?;
        }
        let mut af =
            AtomicWriteFile::open(path).map_err(|e| HnswError::InvalidData(e.to_string()))?;
        af.write_all(&bytes)
            .map_err(|e| HnswError::InvalidData(e.to_string()))?;
        af.commit()
            .map_err(|e| HnswError::InvalidData(e.to_string()))?;
        Ok(())
    }

    /// Loads and validates an HNSW index from disk with SHA-256 integrity verification.
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, HnswError> {
        let bytes = std::fs::read(path).map_err(|e| HnswError::InvalidData(e.to_string()))?;
        Self::from_pvhn_bytes(&bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generate_deterministic_vector(seed: usize) -> [f32; 128] {
        let mut vec = [0.0f32; 128];
        let mut norm_sq = 0.0f32;
        for i in 0..128 {
            let val = (((seed + 1) * 31 + (i + 1) * 17) % 1000) as f32 / 1000.0 - 0.5;
            vec[i] = val;
            norm_sq += val * val;
        }
        let norm = norm_sq.sqrt().max(1e-9);
        for elem in &mut vec {
            *elem /= norm;
        }
        vec
    }

    #[test]
    fn test_hnsw_config_defaults() {
        let config = HnswConfig::default();
        assert_eq!(config.m, 16);
        assert_eq!(config.m_max_0, 32);
        assert_eq!(config.m0(), 32);
        assert_eq!(config.ef_construction, 64);
        assert_eq!(config.ef_search, 64);
        let expected_ml = 1.0 / (16.0_f64).ln();
        assert!((config.ml - expected_ml).abs() < 1e-6);

        let custom = HnswConfig::new(8, 32, 16);
        assert_eq!(custom.m, 8);
        assert_eq!(custom.m_max_0, 16);
        assert_eq!(custom.ef_construction, 32);
        assert_eq!(custom.ef_search, 16);
    }

    #[test]
    fn test_level_generation_distribution() {
        let mut rng = SafeRng::new(42);
        let ml = 1.0 / (16.0_f64).ln();
        let total_samples = 10_000;
        let mut counts = [0usize; 6];

        for _ in 0..total_samples {
            let level = rng.generate_level(ml);
            if level < counts.len() {
                counts[level] += 1;
            }
        }

        // Expected ratio P(L >= l) / P(L >= l+1) ~ 16
        // P(L = 0) ~ 1 - 1/16 = 93.75%
        let p0 = counts[0] as f64 / total_samples as f64;
        assert!(
            p0 > 0.90 && p0 < 0.97,
            "P(L=0) expected ~0.9375, got {p0}"
        );

        let p1 = counts[1] as f64 / total_samples as f64;
        assert!(
            p1 > 0.03 && p1 < 0.09,
            "P(L=1) expected ~0.0586, got {p1}"
        );
    }

    #[test]
    fn test_empty_index_behavior() {
        let index = HnswIndex::new(HnswConfig::default());
        assert!(index.is_empty());
        assert_eq!(index.len(), 0);
        assert_eq!(index.entry_point(), None);

        let query = HnswVector::F32(generate_deterministic_vector(99));
        let results = index.search(&query, 5, 32);
        assert!(results.is_empty());

        let bytes = index.to_pvhn_bytes().expect("serialize empty index");
        let decoded = HnswIndex::from_pvhn_bytes(&bytes).expect("deserialize empty index");
        assert!(decoded.is_empty());
    }

    #[test]
    fn test_single_node_insert_and_search() {
        let mut index = HnswIndex::new(HnswConfig::default());
        let vec_data = generate_deterministic_vector(1);
        let vec = HnswVector::F32(vec_data);

        index.insert(42, vec.clone());
        assert_eq!(index.len(), 1);
        assert_eq!(index.entry_point(), Some(0));

        let results = index.search(&vec, 5, 32);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].doc_id, 42);
        assert!(results[0].distance < 1e-5);
        assert!((results[0].similarity - 1.0).abs() < 1e-4);
    }

    #[test]
    fn test_degree_bounding_invariant() {
        let config = HnswConfig {
            m: 8,
            m_max_0: 16,
            ef_construction: 32,
            ef_search: 32,
            ml: 1.0 / (8.0_f64).ln(),
        };
        let mut index = HnswIndex::new(config).with_seed(12345);

        for doc_id in 0..100 {
            let vec = HnswVector::F32(generate_deterministic_vector(doc_id));
            index.insert(doc_id, vec);
        }

        assert_eq!(index.len(), 100);

        for node in index.nodes() {
            // Layer 0 degree bound
            assert!(
                node.neighbors[0].len() <= config.m_max_0,
                "Node {} layer 0 degree {} exceeded m_max_0 {}",
                node.id,
                node.neighbors[0].len(),
                config.m_max_0
            );

            // Upper layer degree bounds
            for l in 1..=node.level {
                assert!(
                    node.neighbors[l].len() <= config.m,
                    "Node {} layer {} degree {} exceeded m {}",
                    node.id,
                    l,
                    node.neighbors[l].len(),
                    config.m
                );
            }
        }
    }

    #[test]
    fn test_layer_hierarchy_invariant() {
        let mut index = HnswIndex::new(HnswConfig::default()).with_seed(999);
        for doc_id in 0..150 {
            let vec = HnswVector::F32(generate_deterministic_vector(doc_id));
            index.insert(doc_id, vec);
        }

        // Check entry point is valid and exists at max_level
        let ep = index.entry_point().expect("entry point exists");
        assert_eq!(index.nodes()[ep].level, index.max_level());

        // Check if node exists at layer l, it must exist and have adjacency at layer l-1
        for node in index.nodes() {
            assert_eq!(node.neighbors.len(), node.level + 1);
        }
    }

    #[test]
    fn test_dual_vector_support_and_cross_search() {
        let mut index = HnswIndex::new(HnswConfig::default()).with_seed(555);

        let v0_f32 = generate_deterministic_vector(0);
        let v1_f32 = generate_deterministic_vector(1);
        let v2_f32 = generate_deterministic_vector(2);

        let _v0_q = QuantizedVector::quantize(&v0_f32);
        let v1_q = QuantizedVector::quantize(&v1_f32);

        index.insert(100, HnswVector::F32(v0_f32));
        index.insert(101, HnswVector::Quantized(v1_q));
        index.insert(102, HnswVector::F32(v2_f32));

        assert_eq!(index.len(), 3);

        // Search with Quantized query matching doc 101
        let query_q = HnswVector::Quantized(v1_q);
        let results_q = index.search(&query_q, 3, 32);
        assert_eq!(results_q[0].doc_id, 101);
        assert!(results_q[0].similarity > 0.99);

        // Search with F32 query matching doc 100
        let query_f = HnswVector::F32(v0_f32);
        let results_f = index.search(&query_f, 3, 32);
        assert_eq!(results_f[0].doc_id, 100);
        assert!(results_f[0].similarity > 0.99);
    }

    #[test]
    fn test_search_recall_vs_linear_scan() {
        let mut index = HnswIndex::new(HnswConfig::default()).with_seed(777);
        let num_docs = 80;
        let mut doc_vectors = Vec::with_capacity(num_docs);

        for doc_id in 0..num_docs {
            let raw = generate_deterministic_vector(doc_id * 3 + 1);
            let qv = QuantizedVector::quantize(&raw);
            let hvec = HnswVector::Quantized(qv);
            doc_vectors.push((doc_id, hvec.clone()));
            index.insert(doc_id, hvec);
        }

        let num_queries = 15;
        let k = 5;
        let mut total_recall = 0.0f32;

        for q in 0..num_queries {
            let query_raw = generate_deterministic_vector(5000 + q * 11);
            let query_vec = HnswVector::F32(query_raw);

            // Ground truth linear scan
            let mut ground_truth: Vec<(usize, f32)> = doc_vectors
                .iter()
                .map(|(id, vec)| (*id, cosine_distance(&query_vec, vec)))
                .collect();
            ground_truth.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal));
            let top_truth: HashSet<usize> = ground_truth.iter().take(k).map(|(id, _)| *id).collect();

            // HNSW search
            let hnsw_results = index.search(&query_vec, k, 64);
            let top_hnsw: HashSet<usize> = hnsw_results.iter().map(|r| r.doc_id).collect();

            let hits = top_truth.intersection(&top_hnsw).count();
            total_recall += hits as f32 / k as f32;
        }

        let avg_recall = total_recall / num_queries as f32;
        assert!(
            avg_recall >= 0.95,
            "Expected Recall@{k} >= 0.95, got {avg_recall:.4}"
        );
    }

    #[test]
    fn test_pvhn_serialization_roundtrip() {
        let mut index = HnswIndex::new(HnswConfig::default()).with_seed(888);
        let corpus_hash = [0x5au8; 32];
        index.set_corpus_hash(corpus_hash);

        for doc_id in 0..30 {
            let raw = generate_deterministic_vector(doc_id);
            if doc_id % 2 == 0 {
                index.insert(doc_id, HnswVector::F32(raw));
            } else {
                index.insert(doc_id, HnswVector::Quantized(QuantizedVector::quantize(&raw)));
            }
        }

        let bytes = index.to_pvhn_bytes().expect("to_pvhn_bytes");
        assert_eq!(&bytes[0..4], PVHN_MAGIC);

        let restored = HnswIndex::from_pvhn_bytes(&bytes).expect("from_pvhn_bytes");
        assert_eq!(restored.len(), index.len());
        assert_eq!(restored.entry_point(), index.entry_point());
        assert_eq!(restored.max_level(), index.max_level());
        assert_eq!(restored.corpus_hash(), index.corpus_hash());
        assert_eq!(restored.doc_ids(), index.doc_ids());

        for i in 0..index.len() {
            let n_orig = &index.nodes()[i];
            let n_rest = &restored.nodes()[i];
            assert_eq!(n_orig.id, n_rest.id);
            assert_eq!(n_orig.doc_id, n_rest.doc_id);
            assert_eq!(n_orig.level, n_rest.level);
            assert_eq!(n_orig.neighbors, n_rest.neighbors);
            assert_eq!(n_orig.vector, n_rest.vector);
        }
    }

    #[test]
    fn test_pvhn_corruption_rejection() {
        let mut index = HnswIndex::new(HnswConfig::default());
        index.insert(1, HnswVector::F32(generate_deterministic_vector(1)));
        index.insert(2, HnswVector::F32(generate_deterministic_vector(2)));

        let mut bytes = index.to_pvhn_bytes().expect("serialization");

        // 1. Bit flip inside payload triggers ChecksumMismatch
        bytes[45] ^= 0x01;
        assert_eq!(
            HnswIndex::from_pvhn_bytes(&bytes),
            Err(HnswError::ChecksumMismatch)
        );
        bytes[45] ^= 0x01; // Restore

        // 2. Corrupt magic bytes
        bytes[0] = b'X';
        // Recalculate checksum so it passes checksum check and fails magic
        let payload_len = bytes.len() - PVHN_TRAILER_SIZE;
        let new_hash = Sha256::digest(&bytes[..payload_len]);
        bytes[payload_len..].copy_from_slice(&new_hash);
        assert_eq!(
            HnswIndex::from_pvhn_bytes(&bytes),
            Err(HnswError::InvalidMagic)
        );

        // 3. Truncated bytes
        let truncated = &bytes[..40];
        assert_eq!(
            HnswIndex::from_pvhn_bytes(truncated),
            Err(HnswError::TruncatedData)
        );
    }
}
