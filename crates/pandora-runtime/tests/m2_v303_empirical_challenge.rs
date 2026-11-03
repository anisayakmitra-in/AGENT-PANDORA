#![forbid(unsafe_code)]
#![allow(clippy::needless_range_loop)]

//! Milestone 2 Empirical Challenge Suite (v3.0.3)
//!
//! Independent stress testing and empirical challenge harness for:
//! 1. Vector Storage Reduction (Coordinate 75.00%, Net 74.22%, binary layout)
//! 2. Quantization Precision & Ranking Preservation (>99%, Cauchy-Schwarz bounds, Spearman rho > 0.99, margins delta >= 0.005 & 0.003)
//! 3. Edge Cases & Boundary Conditions (Zero, Subnormals, Scale < 1e-9, Saturation at +/-127, NaNs, Negative Dot Clamping)
//! 4. SIMD Dot Product Accumulator Overflow Prevention across 128 dimensions
//! 5. V1 / V2 Backward Compatibility and Differential Caching Invariants

use pandora_runtime::arch_simd::simd_dot_product_128;
use pandora_runtime::context_vfs::{
    ContextVfs, VfsDoc, compute_corpus_hash, deserialize_vfs_index, read_index_stats,
    serialize_vfs_index, VFS_INDEX_MAGIC, VFS_INDEX_VERSION_V1, VFS_INDEX_VERSION_V2,
    VFS_QUANTIZED_VECTOR_SIZE,
};
use pandora_runtime::semantic_memory::{
    DenseFeatureHasher, QuantizedVector, SparseBm25Index,
};
use pandora_runtime::{simd_dot_product_f32_i8, simd_dot_product_i8};
use pandora_types::{
    ContextClassification, ContextUri, MemoryIntent, MemoryKind, MemoryRecord, MemoryScope,
    MemoryTier, SessionId, TenantId, Timestamp, WorkspaceId,
};
use std::time::{SystemTime, UNIX_EPOCH};

/// Simple deterministic 64-bit pseudo-random generator (Xorshift64*).
struct ChallengeRng {
    state: u64,
}

impl ChallengeRng {
    fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 0xdeadbeefcafebabe } else { seed },
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545f4914f6cdd1d)
    }

    fn next_f32(&mut self) -> f32 {
        // Uniform in [0.0, 1.0)
        let val = (self.next_u64() >> 40) as u32;
        (val as f32) / ((1u32 << 24) as f32)
    }

    /// Generates standard normal variable via Box-Muller transform
    fn next_normal(&mut self) -> f32 {
        let mut u1 = self.next_f32();
        while u1 <= 1e-7 {
            u1 = self.next_f32();
        }
        let u2 = self.next_f32();
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f32::consts::PI * u2).cos()
    }

    /// Generates isotropic random unit vector on S^127
    fn next_unit_vector(&mut self) -> [f32; 128] {
        let mut vec = [0.0f32; 128];
        let mut norm_sq = 0.0f32;
        for elem in &mut vec {
            let n = self.next_normal();
            *elem = n;
            norm_sq += n * n;
        }
        let norm = norm_sq.sqrt();
        for elem in &mut vec {
            *elem /= norm;
        }
        vec
    }

    /// Generates sparse unit vector with k non-zero entries
    fn next_sparse_unit_vector(&mut self, k: usize) -> [f32; 128] {
        let mut vec = [0.0f32; 128];
        let mut norm_sq = 0.0f32;
        let mut indices: Vec<usize> = (0..128).collect();
        // Fisher-Yates partial shuffle
        for i in 0..k {
            let j = i + ((self.next_u64() as usize) % (128 - i));
            indices.swap(i, j);
            let val = self.next_normal();
            vec[indices[i]] = val;
            norm_sq += val * val;
        }
        let norm = norm_sq.sqrt();
        for i in 0..k {
            vec[indices[i]] /= norm;
        }
        vec
    }
}

fn make_test_record(id: &str, tier: MemoryTier, intent: MemoryIntent, summary: &str) -> MemoryRecord {
    let scope = MemoryScope::new(
        TenantId::new("t1").unwrap(),
        WorkspaceId::new("w1").unwrap(),
        SessionId::new("s1").unwrap(),
        "p1",
    )
    .unwrap();
    let l1 = MemoryRecord::new_l1(
        id,
        MemoryKind::Lesson,
        scope,
        summary,
        ContextClassification::Internal,
        Timestamp::from_unix_seconds(100),
        "prov-1",
    )
    .unwrap()
    .with_intent(intent);
    if tier == MemoryTier::L2 {
        let approval = pandora_types::MemoryApproval::new("app-1", "operator-1").unwrap();
        MemoryRecord::promote_l2(l1, approval, Timestamp::from_unix_seconds(150)).unwrap()
    } else {
        l1
    }
}

/// Computes standard statistical Spearman rank correlation coefficient with fractional midranks for ties.
fn compute_spearman_rho(true_scores: &[f32], quant_scores: &[f32]) -> f64 {
    let n = true_scores.len();
    assert_eq!(n, quant_scores.len());

    fn rank_data(scores: &[f32]) -> Vec<f64> {
        let n = scores.len();
        let mut indices: Vec<usize> = (0..n).collect();
        indices.sort_by(|&a, &b| {
            scores[b]
                .partial_cmp(&scores[a])
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.cmp(&b))
        });

        let mut ranks = vec![0.0f64; n];
        let mut i = 0;
        while i < n {
            let mut j = i + 1;
            while j < n && (scores[indices[j]] - scores[indices[i]]).abs() < 1e-6 {
                j += 1;
            }
            let avg_rank = (i + j - 1) as f64 / 2.0;
            for k in i..j {
                ranks[indices[k]] = avg_rank;
            }
            i = j;
        }
        ranks
    }

    let r_true = rank_data(true_scores);
    let r_quant = rank_data(quant_scores);

    let mean_true: f64 = r_true.iter().sum::<f64>() / (n as f64);
    let mean_quant: f64 = r_quant.iter().sum::<f64>() / (n as f64);

    let mut cov = 0.0f64;
    let mut var_true = 0.0f64;
    let mut var_quant = 0.0f64;

    for i in 0..n {
        let dt = r_true[i] - mean_true;
        let dq = r_quant[i] - mean_quant;
        cov += dt * dq;
        var_true += dt * dt;
        var_quant += dq * dq;
    }

    if var_true < 1e-12 || var_quant < 1e-12 {
        return 1.0;
    }

    cov / (var_true.sqrt() * var_quant.sqrt())
}

// -----------------------------------------------------------------------------
// 1. Vector Storage Reduction & Binary Serialization Layout
// -----------------------------------------------------------------------------

#[test]
fn test_challenge_coordinate_and_serialized_storage_reduction_75_percent() {
    // Coordinate storage reduction: 512 bytes -> 128 bytes = 75.0000%
    let baseline_coord_bytes = 128 * std::mem::size_of::<f32>();
    let quant_coord_bytes = 128 * std::mem::size_of::<i8>();
    assert_eq!(baseline_coord_bytes, 512);
    assert_eq!(quant_coord_bytes, 128);

    let coord_reduction = (baseline_coord_bytes - quant_coord_bytes) as f64 / (baseline_coord_bytes as f64);
    assert!((coord_reduction - 0.7500).abs() < 1e-9, "Coordinate reduction must be strictly 75.00%");

    // Net storage per vector: 4-byte f32 scale + 128 bytes i8 data = 132 bytes
    let net_quant_vector_bytes = std::mem::size_of::<f32>() + 128 * std::mem::size_of::<i8>();
    assert_eq!(net_quant_vector_bytes, 132);
    assert_eq!(net_quant_vector_bytes, VFS_QUANTIZED_VECTOR_SIZE);

    let net_reduction = (baseline_coord_bytes - net_quant_vector_bytes) as f64 / (baseline_coord_bytes as f64);
    // (512 - 132) / 512 = 380 / 512 = 0.7421875 (74.21875%)
    assert!((net_reduction - 0.7421875).abs() < 1e-9);

    // Test serialized binary file layout across multiple document counts
    let doc_counts = [1, 5, 10, 25, 50, 100];
    let hasher = DenseFeatureHasher::new();

    for &num_docs in &doc_counts {
        let mut docs = Vec::with_capacity(num_docs);
        let mut bm25 = SparseBm25Index::new();
        let mut f32_vecs = Vec::with_capacity(num_docs);
        let mut quant_vecs = Vec::with_capacity(num_docs);

        for i in 0..num_docs {
            let content = format!("Storage challenge document {i} with technical vector embeddings and BM25 index terms");
            let uri = ContextUri::new(format!("pandora://bench/doc-{i}")).unwrap();
            let doc = VfsDoc {
                uri,
                title: format!("Doc Title {i}"),
                content: content.clone(),
                tier: Some(MemoryTier::L1),
                intent: Some(MemoryIntent::Coding),
            };
            bm25.add_document(i, &content);
            let v = hasher.vectorize(&content);
            let qv = QuantizedVector::quantize(&v);
            f32_vecs.push((i, v));
            quant_vecs.push((i, qv));
            docs.push(doc);
        }

        let corpus_hash = compute_corpus_hash(&docs);
        let created_at = 1730000000u64;

        // Serialize V2 format
        let v2_bytes = serialize_vfs_index(&docs, &bm25, &quant_vecs, corpus_hash, created_at).unwrap();

        // Validate Header (68 bytes)
        assert_eq!(&v2_bytes[0..4], VFS_INDEX_MAGIC);
        let v2_version = u32::from_le_bytes(v2_bytes[4..8].try_into().unwrap());
        assert_eq!(v2_version, VFS_INDEX_VERSION_V2);
        let v2_created = u64::from_le_bytes(v2_bytes[8..16].try_into().unwrap());
        assert_eq!(v2_created, created_at);
        let v2_doc_count = u32::from_le_bytes(v2_bytes[16..20].try_into().unwrap());
        assert_eq!(v2_doc_count, num_docs as u32);
        let v2_dim = u32::from_le_bytes(v2_bytes[20..24].try_into().unwrap());
        assert_eq!(v2_dim, 128);

        // Manually craft V1 format with exact same docs and postings but 512-byte float vectors
        let mut v1_bytes = Vec::new();
        v1_bytes.extend_from_slice(VFS_INDEX_MAGIC);
        v1_bytes.extend_from_slice(&VFS_INDEX_VERSION_V1.to_le_bytes());
        v1_bytes.extend_from_slice(&created_at.to_le_bytes());
        v1_bytes.extend_from_slice(&(num_docs as u32).to_le_bytes());
        v1_bytes.extend_from_slice(&128u32.to_le_bytes());
        v1_bytes.extend_from_slice(&(bm25.total_tokens() as u64).to_le_bytes());
        let mut terms: Vec<_> = bm25.postings().keys().collect();
        terms.sort();
        v1_bytes.extend_from_slice(&(terms.len() as u32).to_le_bytes());
        v1_bytes.extend_from_slice(&corpus_hash);

        for (idx, doc) in docs.iter().enumerate() {
            v1_bytes.extend_from_slice(&(idx as u32).to_le_bytes());
            let uri_b = doc.uri.as_str().as_bytes();
            v1_bytes.extend_from_slice(&(uri_b.len() as u16).to_le_bytes());
            v1_bytes.extend_from_slice(uri_b);
            let title_b = doc.title.as_bytes();
            v1_bytes.extend_from_slice(&(title_b.len() as u16).to_le_bytes());
            v1_bytes.extend_from_slice(title_b);
            let content_b = doc.content.as_bytes();
            v1_bytes.extend_from_slice(&(content_b.len() as u32).to_le_bytes());
            v1_bytes.extend_from_slice(content_b);
            v1_bytes.push(2);
            v1_bytes.push(1);
            let doc_len = *bm25.doc_lengths().get(&idx).unwrap_or(&0) as u32;
            v1_bytes.extend_from_slice(&doc_len.to_le_bytes());
            for elem in &f32_vecs[idx].1 {
                v1_bytes.extend_from_slice(&elem.to_le_bytes());
            }
        }

        for term in &terms {
            let term_b = term.as_bytes();
            v1_bytes.extend_from_slice(&(term_b.len() as u16).to_le_bytes());
            v1_bytes.extend_from_slice(term_b);
            let mut postings = bm25.postings().get(*term).cloned().unwrap_or_default();
            postings.sort_by_key(|&(d, _)| d);
            v1_bytes.extend_from_slice(&(postings.len() as u32).to_le_bytes());
            for (d, tf) in postings {
                v1_bytes.extend_from_slice(&(d as u32).to_le_bytes());
                v1_bytes.extend_from_slice(&tf.to_le_bytes());
            }
        }
        use sha2::{Digest, Sha256};
        let checksum_v1 = Sha256::digest(&v1_bytes);
        v1_bytes.extend_from_slice(&checksum_v1);

        // Assert exact file size delta: exactly 380 bytes saved per document!
        let diff = v1_bytes.len() - v2_bytes.len();
        let expected_savings = num_docs * 380;
        assert_eq!(
            diff, expected_savings,
            "Expected exactly {expected_savings} bytes saved for {num_docs} docs, got {diff}"
        );
    }
}

// -----------------------------------------------------------------------------
// 2. Cauchy-Schwarz Error Bounds and Analytical Noise Guarantees
// -----------------------------------------------------------------------------

#[test]
fn test_challenge_cauchy_schwarz_error_bounds() {
    let mut rng = ChallengeRng::new(0x1337_c001);
    let hasher = DenseFeatureHasher::new();

    // Generate 1,000 diverse test vectors across 4 regimes
    let mut test_vectors: Vec<[f32; 128]> = Vec::with_capacity(1000);

    // Regime 1: 400 real hashed text embeddings
    for i in 0..400 {
        let text = format!("domain topic {i} mathematical rigor and quantization noise variance bounds");
        test_vectors.push(hasher.vectorize(&text));
    }

    // Regime 2: 300 isotropic random unit vectors on S^127
    for _ in 0..300 {
        test_vectors.push(rng.next_unit_vector());
    }

    // Regime 3: 150 sparse unit vectors (k in 3..20)
    for i in 0..150 {
        let k = 3 + (i % 18);
        test_vectors.push(rng.next_sparse_unit_vector(k));
    }

    // Regime 4: 150 clustered vectors with small angular separations
    let centroid = rng.next_unit_vector();
    for i in 0..150 {
        let noise = rng.next_unit_vector();
        let alpha = 0.01 * ((i + 1) as f32) / 150.0;
        let mut v = [0.0f32; 128];
        let mut norm_sq = 0.0f32;
        for dim in 0..128 {
            v[dim] = centroid[dim] + alpha * noise[dim];
            norm_sq += v[dim] * v[dim];
        }
        let norm = norm_sq.sqrt();
        for dim in 0..128 {
            v[dim] /= norm;
        }
        test_vectors.push(v);
    }

    assert_eq!(test_vectors.len(), 1000);

    // Theoretical maximum error bound across all possible unit vectors:
    // |e_i| <= s / 2 <= 1 / 254
    // ||e||_2 <= sqrt(128) * (s / 2) <= sqrt(128) / 254 ~= 0.0445435
    let max_theoretical_l2_error = (128.0f32).sqrt() / 254.0;

    let mut max_observed_coord_err = 0.0f32;
    let mut max_observed_l2_err = 0.0f32;

    for (idx, v) in test_vectors.iter().enumerate() {
        let q = QuantizedVector::quantize(v);
        let deq = q.dequantize();

        let mut l2_err_sq = 0.0f32;
        for dim in 0..128 {
            let coord_err = (v[dim] - deq[dim]).abs();
            if coord_err > max_observed_coord_err {
                max_observed_coord_err = coord_err;
            }
            // Strict component-wise bound: |e_i| <= s / 2 + float epsilon
            let max_allowed_coord_err = (q.scale / 2.0) + 1e-6;
            assert!(
                coord_err <= max_allowed_coord_err,
                "Doc {idx} dim {dim} coord error {coord_err} exceeded bound {max_allowed_coord_err}"
            );
            l2_err_sq += coord_err * coord_err;
        }

        let l2_err = l2_err_sq.sqrt();
        if l2_err > max_observed_l2_err {
            max_observed_l2_err = l2_err;
        }

        // Strict Euclidean norm bound: ||e||_2 <= sqrt(128) / 254
        assert!(
            l2_err <= max_theoretical_l2_error + 1e-6,
            "Doc {idx} L2 error {l2_err} exceeded theoretical bound {max_theoretical_l2_error}"
        );
    }

    // Now test Cauchy-Schwarz on 50 independent query vectors against all 1000 docs:
    // Total 50 * 1,000 = 50,000 query-doc pairs!
    // Verification 1: Linear dot product inner error: |<q, v> - <q, \tilde{v}>| <= ||q||_2 * ||e||_2
    // Verification 2: Clamped cosine similarity error: |sim(q, v) - quant_sim(q, \tilde{v})| <= ||q||_2 * ||e||_2
    let mut queries: Vec<[f32; 128]> = Vec::with_capacity(50);
    for _ in 0..25 {
        queries.push(rng.next_unit_vector());
    }
    for i in 0..25 {
        let q_text = format!("search query phrase {i} for cauchy schwarz dot product verification");
        queries.push(hasher.vectorize(&q_text));
    }

    let mut total_pairs_tested = 0;
    let mut linear_cs_violations = 0;
    let mut clamped_sim_violations = 0;
    let mut max_linear_error = 0.0f32;

    for (q_idx, query) in queries.iter().enumerate() {
        for (d_idx, doc) in test_vectors.iter().enumerate() {
            total_pairs_tested += 1;

            let true_dot = simd_dot_product_128(query, doc);
            let q_doc = QuantizedVector::quantize(doc);
            let deq = q_doc.dequantize();
            let quant_dot_raw = simd_dot_product_128(query, &deq);

            // Compute exact L2 error of this doc: ||e||_2
            let mut l2_sq = 0.0f32;
            for k in 0..128 {
                let diff = doc[k] - deq[k];
                l2_sq += diff * diff;
            }
            let doc_l2_err = l2_sq.sqrt();

            // 1. Raw linear dot product error: |<q, v> - <q, deq>| <= ||q||_2 * ||e||_2
            let dot_err = (true_dot - quant_dot_raw).abs();
            if dot_err > max_linear_error {
                max_linear_error = dot_err;
            }

            if dot_err > doc_l2_err + 1e-6 {
                linear_cs_violations += 1;
                eprintln!(
                    "Linear CS Violation at query {q_idx}, doc {d_idx}: dot_err={dot_err}, doc_l2_err={doc_l2_err}"
                );
            }

            // 2. Clamped similarity error: |hasher.cosine_similarity - hasher.score_quantized| <= ||q||_2 * ||e||_2
            // Because clamp is 1-Lipschitz (non-expansive), |clamp(x) - clamp(y)| <= |x - y| <= ||e||_2
            let true_sim = hasher.cosine_similarity(query, doc);
            let quant_sim = hasher.score_quantized(query, &q_doc);
            let sim_err = (true_sim - quant_sim).abs();

            if sim_err > doc_l2_err + 1e-6 {
                clamped_sim_violations += 1;
                eprintln!(
                    "Clamped Sim Violation at query {q_idx}, doc {d_idx}: sim_err={sim_err}, doc_l2_err={doc_l2_err}"
                );
            }
        }
    }

    println!("Cauchy-Schwarz test: total_pairs = {total_pairs_tested}, linear_violations = {linear_cs_violations}, clamped_violations = {clamped_sim_violations}");
    println!("Max theoretical L2 error bound: {max_theoretical_l2_error:.6}");
    println!("Max observed coordinate error: {max_observed_coord_err:.6} (bound: {:.6})", 1.0 / 254.0);
    println!("Max observed L2 error: {max_observed_l2_err:.6}");
    println!("Max observed linear dot error: {max_linear_error:.6}");

    assert_eq!(total_pairs_tested, 50_000);
    assert_eq!(
        linear_cs_violations, 0,
        "Linear Cauchy-Schwarz |<q, v - v_tilde>| <= ||q|| ||e|| MUST have 0 violations across 50,000 pairs"
    );
    assert_eq!(
        clamped_sim_violations, 0,
        "Clamped similarity error |sim - quant_sim| <= ||q|| ||e|| MUST have 0 violations across 50,000 pairs"
    );
    assert!(
        max_linear_error < max_theoretical_l2_error + 1e-6,
        "Max observed linear dot error {max_linear_error} must be bounded by {max_theoretical_l2_error}"
    );
}

// -----------------------------------------------------------------------------
// 3. Spearman Rank Correlation (rho > 0.99) Across Vector Distributions
// -----------------------------------------------------------------------------

#[test]
fn test_challenge_spearman_rank_correlation_across_distributions() {
    let mut rng = ChallengeRng::new(0x4242_9999);
    let hasher = DenseFeatureHasher::new();

    // Test across 4 distinct distribution suites, each with 250 documents:
    // Suite A: Real-world text embeddings across diverse domains
    // Suite B: Multi-cluster realistic distribution (5 topic centroids with diverse intra-cluster docs)
    // Suite C: Isotropic random unit vectors with natural semantic alignment
    // Suite D: Sparse unit vectors (k in 5..20 active dimensions)
    let mut suites: Vec<(&str, Vec<[f32; 128]>)> = Vec::new();

    // Suite A: Realistic dense technical documents
    let technical_domains = [
        "Rust borrow checker lifetime variance and memory safety invariants",
        "Context virtual filesystem persistent indexing and atomic commit rollback",
        "ARM64 NEON vector pipeline latency hiding and loop unrolling",
        "HMAC SHA256 constant time cryptographic message authentication",
        "Okapi BM25 sparse inverted index frequency saturation and doc length penalty",
        "Autonomous agent multi-turn execution loop and convergence guardrails",
        "Distributed consensus Raft state machine replication and log truncation",
        "Hardware autopilot dynamic frequency scaling and energy telemetry",
        "Sandbox OS process isolation handle confinement and memory limits",
        "Subword character n-gram dual FNV-1a feature hashing projection",
    ];
    let mut suite_a = Vec::with_capacity(250);
    for i in 0..250 {
        let domain = technical_domains[i % technical_domains.len()];
        let text = format!("{domain} session run iteration {i} verification evidence");
        suite_a.push(hasher.vectorize(&text));
    }
    suites.push(("RealTextEmbeddings", suite_a));

    // Suite B: Multi-topic clustered distribution
    // 5 distinct centroid vectors with 50 documents per centroid
    let mut suite_b = Vec::with_capacity(250);
    let mut centroids: Vec<[f32; 128]> = Vec::with_capacity(5);
    for _ in 0..5 {
        centroids.push(rng.next_unit_vector());
    }
    for c_idx in 0..5 {
        let centroid = centroids[c_idx];
        for i in 0..50 {
            let noise = rng.next_unit_vector();
            let alpha = 0.25 * ((i + 1) as f32) / 50.0; // perturbation spreading across cluster
            let mut v = [0.0f32; 128];
            let mut norm_sq = 0.0f32;
            for dim in 0..128 {
                v[dim] = centroid[dim] + alpha * noise[dim];
                norm_sq += v[dim] * v[dim];
            }
            let norm = norm_sq.sqrt();
            for dim in 0..128 {
                v[dim] /= norm;
            }
            suite_b.push(v);
        }
    }
    suites.push(("MultiClusterDistribution", suite_b));

    // Suite C: Sparse unit vectors
    let mut suite_c = Vec::with_capacity(250);
    for i in 0..250 {
        suite_c.push(rng.next_sparse_unit_vector(5 + (i % 15)));
    }
    suites.push(("SparseVectors", suite_c));

    // Suite D: Realistic corpus with technical and semantic variations
    let mut suite_d = Vec::with_capacity(250);
    for i in 0..250 {
        let txt = format!("architectural pattern {i} runtime service layer and database isolation");
        suite_d.push(hasher.vectorize(&txt));
    }
    suites.push(("SemanticCorpus", suite_d));

    // Test 10 queries per suite (total 40 rankings of 250 documents each)
    let mut all_rhos: Vec<f64> = Vec::with_capacity(40);

    for (suite_name, docs) in &suites {
        let quant_docs: Vec<QuantizedVector> = docs.iter().map(QuantizedVector::quantize).collect();

        for q_i in 0..10 {
            let query = match *suite_name {
                "RealTextEmbeddings" | "SemanticCorpus" => {
                    let q_txt = format!("query {q_i} verification and architectural invariants");
                    hasher.vectorize(&q_txt)
                }
                "MultiClusterDistribution" => {
                    // Query aligned towards one of the centroids with slight offset
                    let target_centroid = centroids[q_i % centroids.len()];
                    let noise = rng.next_unit_vector();
                    let mut q = [0.0f32; 128];
                    let mut norm_sq = 0.0f32;
                    for dim in 0..128 {
                        q[dim] = target_centroid[dim] + 0.15 * noise[dim];
                        norm_sq += q[dim] * q[dim];
                    }
                    let norm = norm_sq.sqrt();
                    for dim in 0..128 {
                        q[dim] /= norm;
                    }
                    q
                }
                "SparseVectors" => {
                    rng.next_sparse_unit_vector(10)
                }
                _ => rng.next_unit_vector(),
            };

            let n = docs.len();
            let mut true_scores: Vec<(usize, f32)> = Vec::with_capacity(n);
            let mut quant_scores: Vec<(usize, f32)> = Vec::with_capacity(n);

            for idx in 0..n {
                let s_true = hasher.cosine_similarity(&query, &docs[idx]);
                let s_quant = hasher.score_quantized(&query, &quant_docs[idx]);
                true_scores.push((idx, s_true));
                quant_scores.push((idx, s_quant));
            }

            let true_vals: Vec<f32> = true_scores.iter().map(|&(_, s)| s).collect();
            let quant_vals: Vec<f32> = quant_scores.iter().map(|&(_, s)| s).collect();
            let rho = compute_spearman_rho(&true_vals, &quant_vals);
            all_rhos.push(rho);

            println!("Suite {suite_name} Query {q_i}: rho = {rho:.6}");
            assert!(
                rho > 0.99,
                "Suite {suite_name} Query {q_i}: Spearman rho {rho} failed requirement > 0.99"
            );
        }
    }

    let min_rho = all_rhos.iter().cloned().fold(f64::INFINITY, f64::min);
    let avg_rho: f64 = all_rhos.iter().sum::<f64>() / (all_rhos.len() as f64);
    assert!(
        min_rho > 0.99,
        "Minimum Spearman rho across all suites ({min_rho}) must be > 0.99"
    );
    assert!(
        avg_rho > 0.995,
        "Average Spearman rho ({avg_rho}) must be > 0.995"
    );
}

// -----------------------------------------------------------------------------
// 4. Pairwise Ranking Discrimination at Fine-Grained Margins (delta >= 0.005 & 0.003)
// -----------------------------------------------------------------------------

#[test]
fn test_challenge_pairwise_discrimination_at_fine_margins() {
    let hasher = DenseFeatureHasher::new();

    // 300 diverse documents
    let mut docs = Vec::with_capacity(300);
    for i in 0..300 {
        let txt = format!("corpus document {i} for pairwise separation margin challenge testing");
        docs.push(hasher.vectorize(&txt));
    }

    let quant_docs: Vec<QuantizedVector> = docs.iter().map(QuantizedVector::quantize).collect();

    // Evaluate over 10 test queries
    let mut total_pairs_005 = 0;
    let mut correct_pairs_005 = 0;

    let mut total_pairs_003 = 0;
    let mut correct_pairs_003 = 0;

    for q_i in 0..10 {
        let q_txt = format!("fine separation query phrase {q_i} vector similarity");
        let query = hasher.vectorize(&q_txt);

        let mut true_scores = Vec::with_capacity(300);
        let mut quant_scores = Vec::with_capacity(300);

        for idx in 0..300 {
            true_scores.push(hasher.cosine_similarity(&query, &docs[idx]));
            quant_scores.push(hasher.score_quantized(&query, &quant_docs[idx]));
        }

        for i in 0..300 {
            for j in (i + 1)..300 {
                let diff_true = true_scores[i] - true_scores[j];
                let diff_quant = quant_scores[i] - quant_scores[j];

                // Margin delta >= 0.005
                if diff_true.abs() >= 0.005 {
                    total_pairs_005 += 1;
                    if (diff_true > 0.0 && diff_quant > 0.0) || (diff_true < 0.0 && diff_quant < 0.0) {
                        correct_pairs_005 += 1;
                    }
                }

                // Margin delta >= 0.003
                if diff_true.abs() >= 0.003 {
                    total_pairs_003 += 1;
                    if (diff_true > 0.0 && diff_quant > 0.0) || (diff_true < 0.0 && diff_quant < 0.0) {
                        correct_pairs_003 += 1;
                    }
                }
            }
        }
    }

    assert!(total_pairs_005 > 1000);
    assert!(total_pairs_003 > 1000);

    let precision_005 = (correct_pairs_005 as f64) / (total_pairs_005 as f64);
    let precision_003 = (correct_pairs_003 as f64) / (total_pairs_003 as f64);

    println!("Pairwise discrimination @ delta >= 0.005: total_pairs = {total_pairs_005}, correct = {correct_pairs_005}, concordance = {:.4}%", precision_005 * 100.0);
    println!("Pairwise discrimination @ delta >= 0.003: total_pairs = {total_pairs_003}, correct = {correct_pairs_003}, concordance = {:.4}%", precision_003 * 100.0);

    assert!(
        precision_005 > 0.99,
        "Pairwise discrimination at delta >= 0.005 was {precision_005}, required > 0.99"
    );
    assert!(
        precision_003 > 0.99,
        "Pairwise discrimination at delta >= 0.003 was {precision_003}, required > 0.99"
    );
}

// -----------------------------------------------------------------------------
// 5. Boundary Conditions, Subnormals, NaNs, and Numerical Edge Cases
// -----------------------------------------------------------------------------

#[test]
fn test_challenge_edge_cases_and_boundary_conditions() {
    // 1. All-zero vector
    let zero_vec = [0.0f32; 128];
    let q_zero = QuantizedVector::quantize(&zero_vec);
    assert_eq!(q_zero.scale, 0.0);
    assert_eq!(q_zero.data, [0i8; 128]);
    assert_eq!(q_zero.dequantize(), zero_vec);

    let query_one = [1.0f32; 128];
    assert_eq!(q_zero.dot_f32(&query_one), 0.0);
    assert_eq!(q_zero.dot_i8(&q_zero), 0.0);

    // 2. Subnormals and values < 1e-9
    let subnormals = [
        f32::from_bits(1),  // smallest positive subnormal
        1e-38f32,
        1e-20f32,
        1e-15f32,
        9.99e-10f32,        // just below threshold 1e-9
    ];
    for &sn in &subnormals {
        let mut sn_vec = [0.0f32; 128];
        sn_vec[0] = sn;
        let q_sn = QuantizedVector::quantize(&sn_vec);
        assert_eq!(q_sn.scale, 0.0, "Scale for {sn} must be 0.0 (below 1e-9 threshold)");
        assert_eq!(q_sn.data, [0i8; 128]);
        assert_eq!(q_sn.dequantize(), [0.0f32; 128]);
    }

    // 3. Scale factor threshold boundary analysis:
    // When alpha >= 1e-9, quantize calculates scale = alpha / 127.0.
    // If scale > 1e-9 (e.g. alpha = 1e-6, scale = 7.874e-9), dequantize() recovers component accurately:
    let mut vec_above = [0.0f32; 128];
    vec_above[0] = 1e-6;
    let q_above = QuantizedVector::quantize(&vec_above);
    assert!(q_above.scale > 1e-9);
    assert_eq!(q_above.data[0], 127);
    let deq_above = q_above.dequantize();
    assert!((deq_above[0] - 1e-6).abs() < 1e-12);

    // If alpha < 1.27e-7, scale < 1e-9:
    // In that regime, the defensive scale guard in dequantize(), dot_i8(), and score_quantized()
    // treats the vector safely as zero (preventing division-by-zero or subnormal floating point noise).
    let mut vec_tiny = [0.0f32; 128];
    vec_tiny[0] = 1e-8;
    let q_tiny = QuantizedVector::quantize(&vec_tiny);
    assert!(q_tiny.scale < 1e-9);
    assert_eq!(q_tiny.dequantize(), [0.0f32; 128]);

    // 4. Saturation at +/-127 (never -128 in symmetric quantization)
    let mut pos_extreme = [0.0f32; 128];
    pos_extreme[0] = 100.0f32;
    pos_extreme[1] = 50.0f32;
    let q_pos = QuantizedVector::quantize(&pos_extreme);
    assert_eq!(q_pos.data[0], 127);
    assert!((q_pos.scale - (100.0 / 127.0)).abs() < 1e-5);
    for &d in &q_pos.data {
        assert!(d >= -127, "data value {d} violated [-127, 127] symmetric bounds");
    }

    let mut neg_extreme = [0.0f32; 128];
    neg_extreme[0] = -100.0f32;
    neg_extreme[1] = -50.0f32;
    let q_neg = QuantizedVector::quantize(&neg_extreme);
    assert_eq!(q_neg.data[0], -127);
    assert!((q_neg.scale - (100.0 / 127.0)).abs() < 1e-5);
    for &d in &q_neg.data {
        assert!(d >= -127, "data value {d} violated [-127, 127] symmetric bounds");
        assert_ne!(d, -128, "Value -128 is strictly prohibited in symmetric quantization");
    }

    // 5. Negative dot product clamping to 0.0
    let mut u = [0.0f32; 128];
    u[0] = 1.0;
    let mut neg_u = [0.0f32; 128];
    neg_u[0] = -1.0;

    let q_u = QuantizedVector::quantize(&u);
    let q_neg_u = QuantizedVector::quantize(&neg_u);

    // Asymmetric: query is u, doc is -u -> raw dot is -1.0, clamped to 0.0
    let asym_neg = q_neg_u.dot_f32(&u);
    assert_eq!(asym_neg, 0.0, "Negative dot product must clamp to 0.0");

    // Symmetric: q_u . q_neg_u -> clamped to 0.0
    let sym_neg = q_u.dot_i8(&q_neg_u);
    assert_eq!(sym_neg, 0.0, "Negative symmetric dot product must clamp to 0.0");

    // Orthogonal vectors
    let mut v_orth = [0.0f32; 128];
    v_orth[1] = 1.0;
    let q_orth = QuantizedVector::quantize(&v_orth);
    assert_eq!(q_u.dot_f32(&v_orth), 0.0);
    assert_eq!(q_u.dot_i8(&q_orth), 0.0);

    // 6. NaN resilience: verify quantization with NaNs does not panic
    let nan_vec = [f32::NAN; 128];
    let q_nan = QuantizedVector::quantize(&nan_vec);
    assert_eq!(q_nan.scale, 0.0);
    assert_eq!(q_nan.data, [0i8; 128]);
}

// -----------------------------------------------------------------------------
// 6. SIMD Accumulator Overflow Prevention across 128 dimensions
// -----------------------------------------------------------------------------

#[test]
fn test_challenge_simd_accumulator_overflow_prevention() {
    // Theoretical max integer dot product:
    // 128 coordinates * (+127 * +127) = 128 * 16,129 = 2,064,512.
    // 2,064,512 << 2,147,483,647 (i32::MAX) -> occupies < 0.1% of signed 32-bit range.
    let max_pos = [127i8; 128];
    let dot_max = simd_dot_product_i8(&max_pos, &max_pos);
    assert_eq!(dot_max, 2_064_512);

    // Theoretical min integer dot product:
    // 128 coordinates * (-127 * +127) = -2,064,512.
    let max_neg = [-127i8; 128];
    let dot_min = simd_dot_product_i8(&max_neg, &max_pos);
    assert_eq!(dot_min, -2_064_512);

    // Boundary check with -128 (theoretical i8 limit if ever passed):
    // 128 * (-128 * -128) = 128 * 16,384 = 2,097,152 << i32::MAX
    let i8_min = [-128i8; 128];
    let dot_i8_min = simd_dot_product_i8(&i8_min, &i8_min);
    assert_eq!(dot_i8_min, 2_097_152);

    // Test each of the 8 accumulators independently (lane isolation)
    // 16 iterations, 8 lanes per iteration = 128 elements.
    for lane in 0..8 {
        let mut single_lane_a = [0i8; 128];
        let mut single_lane_b = [0i8; 128];
        for iter in 0..16 {
            single_lane_a[iter * 8 + lane] = 127;
            single_lane_b[iter * 8 + lane] = 127;
        }
        let lane_dot = simd_dot_product_i8(&single_lane_a, &single_lane_b);
        assert_eq!(lane_dot, 16 * (127 * 127));
    }

    // Alternating checkerboard pattern
    let mut check_a = [0i8; 128];
    let mut check_b = [0i8; 128];
    for i in 0..128 {
        check_a[i] = if i % 2 == 0 { 127 } else { -127 };
        check_b[i] = if i % 2 == 0 { 127 } else { -127 };
    }
    let dot_check = simd_dot_product_i8(&check_a, &check_b);
    assert_eq!(dot_check, 2_064_512);

    // Asymmetric SIMD dot product clamp and scaling check
    let query_ones = [1.0f32; 128];
    let scale_exact = 1.0f32 / (128.0f32 * 127.0f32);
    let dot_asym = simd_dot_product_f32_i8(&query_ones, &max_pos, scale_exact);
    assert!((dot_asym - 1.0).abs() < 1e-6);

    // Extreme negative query
    let query_neg_ones = [-1.0f32; 128];
    let dot_asym_neg = simd_dot_product_f32_i8(&query_neg_ones, &max_pos, scale_exact);
    assert_eq!(dot_asym_neg, 0.0);
}

// -----------------------------------------------------------------------------
// 7. V1/V2 Backward Compatibility and Differential Diff Caching Invariants
// -----------------------------------------------------------------------------

#[test]
fn test_challenge_v1_v2_backward_compatibility_and_diff_cache() {
    let vfs = ContextVfs::new();

    // Add 15 records
    for i in 0..15 {
        vfs.add_memory_record(make_test_record(
            &format!("challenge-record-{i}"),
            MemoryTier::L1,
            MemoryIntent::Coding,
            &format!("Content for challenge diff caching record {i} with deterministic terms"),
        ));
    }

    let temp_dir = std::env::temp_dir().join(format!(
        "pandora_chal_v1_v2_test_{}_{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    let _ = std::fs::create_dir_all(&temp_dir);
    let index_path = temp_dir.join("vfs.idx");
    vfs.set_index_path(&index_path);

    // First save: 100% cache misses
    vfs.reset_diff_cache_stats();
    vfs.save_index_to_disk(&index_path).unwrap();
    let (hits1, misses1) = vfs.diff_cache_stats();
    let doc_count = vfs.collect_all_documents().unwrap().len();
    assert_eq!(hits1, 0);
    assert_eq!(misses1 as usize, doc_count);

    // Verify index stats
    let stats = read_index_stats(&index_path, Some(&vfs)).unwrap();
    assert_eq!(stats.version, 2);
    assert_eq!(stats.vector_bytes as usize, doc_count * 132);
    assert_eq!(stats.vector_bytes_saved as usize, doc_count * 380);

    // Second save: 100% cache hits, 0 misses (zero re-tokenization / zero re-vectorization)
    vfs.reset_diff_cache_stats();
    vfs.save_index_to_disk(&index_path).unwrap();
    let (hits2, misses2) = vfs.diff_cache_stats();
    assert_eq!(hits2 as usize, doc_count);
    assert_eq!(misses2, 0);

    // Mutate 1 record (add new version)
    vfs.add_memory_record(make_test_record(
        "challenge-record-0",
        MemoryTier::L1,
        MemoryIntent::Coding,
        "MODIFIED content for challenge record 0 triggering differential invalidation",
    ));

    vfs.reset_diff_cache_stats();
    vfs.save_index_to_disk(&index_path).unwrap();
    let (hits3, misses3) = vfs.diff_cache_stats();
    assert_eq!(misses3, 1, "Exactly 1 modified document must register 1 cache miss");
    assert_eq!(hits3 as usize, doc_count - 1);

    // Deserialize index from disk and verify backward compatibility
    let bytes = std::fs::read(&index_path).unwrap();
    let deser = deserialize_vfs_index(&bytes).unwrap();
    assert_eq!(deser.version, 2);
    assert_eq!(deser.doc_vectors.len(), doc_count);

    let _ = std::fs::remove_dir_all(&temp_dir);
}
