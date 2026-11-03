#![forbid(unsafe_code)]

use pandora_runtime::context_vfs::{
    ContextVfs, VfsDoc,
    compute_corpus_hash, deserialize_vfs_index, read_index_stats,
    serialize_vfs_index, VFS_INDEX_MAGIC, VFS_INDEX_VERSION_V1,
    VFS_QUANTIZED_VECTOR_SIZE,
};
use pandora_runtime::semantic_memory::{
    DenseFeatureHasher, QuantizedVector, SparseBm25Index,
};
use pandora_runtime::{simd_dot_product_f32_i8, simd_dot_product_i8};
use pandora_types::{
    ContextClassification, ContextFragment, ContextOrigin, ContextSource,
    ContextTrust, ContextUri, MemoryIntent, MemoryRecord, MemoryScope, MemoryTier,
    SessionId, TenantId, Timestamp, WorkspaceId,
};
use std::time::{SystemTime, UNIX_EPOCH};

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
        pandora_types::MemoryKind::Lesson,
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

#[test]
fn test_adversarial_zero_and_extreme_vectors() {
    // 1. All-zero vector
    let zero_vec = [0.0f32; 128];
    let q_zero = QuantizedVector::quantize(&zero_vec);
    assert_eq!(q_zero.scale, 0.0);
    assert_eq!(q_zero.data, [0i8; 128]);

    let deq_zero = q_zero.dequantize();
    assert_eq!(deq_zero, [0.0f32; 128]);

    let mut query = [0.0f32; 128];
    query[0] = 1.0;
    assert_eq!(q_zero.dot_f32(&query), 0.0);
    assert_eq!(q_zero.dot_i8(&q_zero), 0.0);

    // 2. Sub-threshold vector (< 1e-9)
    let tiny_vec = [1e-10f32; 128];
    let q_tiny = QuantizedVector::quantize(&tiny_vec);
    assert_eq!(q_tiny.scale, 0.0);
    assert_eq!(q_tiny.data, [0i8; 128]);

    // 3. One-hot vector
    for k in 0..128 {
        let mut one_hot = [0.0f32; 128];
        one_hot[k] = 1.0;
        let q = QuantizedVector::quantize(&one_hot);
        assert!((q.scale - (1.0 / 127.0)).abs() < 1e-6);
        assert_eq!(q.data[k], 127);
        for j in 0..128 {
            if j != k {
                assert_eq!(q.data[j], 0);
            }
        }
        let deq = q.dequantize();
        assert!((deq[k] - 1.0).abs() < 1e-5);
    }

    // 4. Uniform vector normalized: 1/sqrt(128)
    let val = 1.0f32 / (128.0f32).sqrt();
    let uniform_vec = [val; 128];
    let q_uni = QuantizedVector::quantize(&uniform_vec);
    assert!((q_uni.scale - (val / 127.0)).abs() < 1e-6);
    for &d in &q_uni.data {
        assert_eq!(d, 127);
    }
    let deq_uni = q_uni.dequantize();
    for &elem in &deq_uni {
        assert!((elem - val).abs() < 1e-5);
    }
}

#[test]
fn test_adversarial_simd_i8_overflow_and_extremes() {
    // Check maximum signed bounds
    let max_pos = [127i8; 128];
    let max_neg = [-127i8; 128];

    // Extreme positive: 128 * 127 * 127 = 2,064,512
    let dot_max_pos = simd_dot_product_i8(&max_pos, &max_pos);
    assert_eq!(dot_max_pos, 2_064_512);

    // Extreme negative: 128 * (-127) * 127 = -2,064,512
    let dot_max_neg = simd_dot_product_i8(&max_neg, &max_pos);
    assert_eq!(dot_max_neg, -2_064_512);

    // Zero alignment
    let zero = [0i8; 128];
    assert_eq!(simd_dot_product_i8(&zero, &max_pos), 0);

    // Asymmetric clamp check: negative raw dot product clamped to 0.0
    let query_neg = [-1.0f32; 128];
    let scale = 1.0 / 127.0;
    let dot_f32_neg = simd_dot_product_f32_i8(&query_neg, &max_pos, scale);
    assert_eq!(dot_f32_neg, 0.0);

    // Asymmetric positive alignment: 128 * (1/sqrt(128)) * 127 * (1/(127 * sqrt(128))) = 1.0
    let query_uni = [1.0f32 / (128.0f32).sqrt(); 128];
    let doc_scale = (1.0f32 / (128.0f32).sqrt()) / 127.0;
    let dot_f32_pos = simd_dot_product_f32_i8(&query_uni, &max_pos, doc_scale);
    assert!((dot_f32_pos - 1.0).abs() < 1e-4);
}

#[test]
fn test_adversarial_quantization_error_bounds_multi_query() {
    let hasher = DenseFeatureHasher::new();

    // 200 documents across various complex topics
    let base_phrases = [
        "zero-cost abstractions in rust async runtime executors",
        "concurrency safety memory models and acquire release barriers",
        "simd vector lanes 128-bit neon instruction execution throughput",
        "distributed consensus raft state machine replication log compactions",
        "int8 scalar vector quantization with symmetric scale factor bounds",
        "differential buffer hash diff caching for cold start acceleration",
        "hmac sha256 cryptographic webhook request validation and constant time auth",
        "okapi bm25 term frequency saturation and inverse document frequency",
        "tripartite governance supervisor orchestrator worker auditing rules",
        "operating system handle containment and memory limits sandboxing",
    ];

    let mut corpus = Vec::with_capacity(200);
    for i in 0..200 {
        let phrase = base_phrases[i % base_phrases.len()];
        corpus.push(format!("{phrase} benchmark evaluation iteration {i} telemetry trace"));
    }

    let mut f32_docs = Vec::with_capacity(200);
    let mut q_docs = Vec::with_capacity(200);

    for text in &corpus {
        let v = hasher.vectorize(text);
        let q = QuantizedVector::quantize(&v);
        f32_docs.push(v);
        q_docs.push(q);
    }

    // Stress test across 5 diverse queries
    let test_queries = [
        "simd vector lanes and neon instruction throughput",
        "differential buffer hash diff caching",
        "cryptographic webhook auth and constant time signature",
        "async runtime executor concurrency safety",
        "bm25 term frequency saturation",
    ];

    for query in test_queries {
        let q_vec = hasher.vectorize(query);

        let mut f32_scores = Vec::with_capacity(200);
        let mut q_scores = Vec::with_capacity(200);

        for i in 0..200 {
            let s_f32 = hasher.cosine_similarity(&q_vec, &f32_docs[i]);
            let s_q = hasher.score_quantized(&q_vec, &q_docs[i]);

            let err = (s_f32 - s_q).abs();
            // Cauchy-Schwarz bound: |E| <= 0.0156
            assert!(
                err < 0.0156,
                "Query '{query}' doc {i} error {err} exceeded 0.0156"
            );

            f32_scores.push(s_f32);
            q_scores.push(s_q);
        }

        // Compute Spearman rank correlation rho
        let mut f32_order: Vec<usize> = (0..200).collect();
        f32_order.sort_by(|&a, &b| {
            f32_scores[b]
                .partial_cmp(&f32_scores[a])
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.cmp(&b))
        });

        let mut q_order: Vec<usize> = (0..200).collect();
        q_order.sort_by(|&a, &b| {
            q_scores[b]
                .partial_cmp(&q_scores[a])
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.cmp(&b))
        });

        let mut f32_ranks = vec![0.0f64; 200];
        let mut q_ranks = vec![0.0f64; 200];
        for (rank, &idx) in f32_order.iter().enumerate() {
            f32_ranks[idx] = rank as f64;
        }
        for (rank, &idx) in q_order.iter().enumerate() {
            q_ranks[idx] = rank as f64;
        }

        let mut sum_d_sq = 0.0f64;
        for i in 0..200 {
            let d = f32_ranks[i] - q_ranks[i];
            sum_d_sq += d * d;
        }

        let n = 200.0f64;
        let rho = 1.0 - (6.0 * sum_d_sq) / (n * (n * n - 1.0));
        assert!(
            rho > 0.99,
            "Query '{query}' Spearman rho {rho} was not > 0.99"
        );

        // Pairwise ranking precision (margin >= 0.005)
        let mut total_pairs = 0;
        let mut correct_pairs = 0;
        for i in 0..200 {
            for j in (i + 1)..200 {
                let diff_true = f32_scores[i] - f32_scores[j];
                if diff_true.abs() >= 0.005 {
                    total_pairs += 1;
                    let diff_q = q_scores[i] - q_scores[j];
                    if (diff_true > 0.0 && diff_q > 0.0) || (diff_true < 0.0 && diff_q < 0.0) {
                        correct_pairs += 1;
                    }
                }
            }
        }

        let precision = (correct_pairs as f64) / (total_pairs as f64);
        assert!(
            precision > 0.99,
            "Query '{query}' pairwise ranking precision {precision} was not > 0.99"
        );
    }
}

#[test]
fn test_adversarial_v1_migration_and_storage_gain() {
    let temp_dir = std::env::temp_dir().join(format!(
        "pandora_v1_v2_migr_test_{}_{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    let _ = std::fs::create_dir_all(&temp_dir);
    let v1_path = temp_dir.join("vfs_v1.idx");
    let v2_path = temp_dir.join("vfs_v2.idx");

    let num_docs = 40;
    let mut docs = Vec::new();
    let mut raw_float_vectors = Vec::new();
    let mut bm25 = SparseBm25Index::new();
    let hasher = DenseFeatureHasher::new();

    for i in 0..num_docs {
        let content = format!("Document content iteration {i} for v1 to v2 migration verification");
        let uri = ContextUri::new(format!("pandora://meta/migr-{i}")).unwrap();
        docs.push(VfsDoc {
            uri,
            title: format!("Doc {i}"),
            content: content.clone(),
            tier: Some(MemoryTier::L1),
            intent: Some(MemoryIntent::Coding),
        });
        bm25.add_document(i, &content);
        raw_float_vectors.push((i, hasher.vectorize(&content)));
    }

    let corpus_hash = compute_corpus_hash(&docs);
    let created_at = 1700000000u64;

    // Manually craft a valid Version 1 binary index payload
    let mut v1_bytes = Vec::new();
    v1_bytes.extend_from_slice(VFS_INDEX_MAGIC);
    v1_bytes.extend_from_slice(&VFS_INDEX_VERSION_V1.to_le_bytes()); // Version 1
    v1_bytes.extend_from_slice(&created_at.to_le_bytes());
    v1_bytes.extend_from_slice(&(num_docs as u32).to_le_bytes());
    v1_bytes.extend_from_slice(&128u32.to_le_bytes());
    v1_bytes.extend_from_slice(&(bm25.total_tokens() as u64).to_le_bytes());
    v1_bytes.extend_from_slice(&(bm25.postings().len() as u32).to_le_bytes());
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

        v1_bytes.push(2); // Tier L1
        v1_bytes.push(1); // Intent Coding

        let doc_len = *bm25.doc_lengths().get(&idx).unwrap_or(&0) as u32;
        v1_bytes.extend_from_slice(&doc_len.to_le_bytes());

        // 512 bytes raw f32 vector
        let f_vec = raw_float_vectors[idx].1;
        for elem in &f_vec {
            v1_bytes.extend_from_slice(&elem.to_le_bytes());
        }
    }

    // Sparse BM25 Postings
    let mut terms: Vec<&String> = bm25.postings().keys().collect();
    terms.sort();
    for term in terms {
        let term_b = term.as_bytes();
        v1_bytes.extend_from_slice(&(term_b.len() as u16).to_le_bytes());
        v1_bytes.extend_from_slice(term_b);
        let mut postings = bm25.postings().get(term).cloned().unwrap_or_default();
        postings.sort_by_key(|&(d, _)| d);
        v1_bytes.extend_from_slice(&(postings.len() as u32).to_le_bytes());
        for (d, tf) in postings {
            v1_bytes.extend_from_slice(&(d as u32).to_le_bytes());
            v1_bytes.extend_from_slice(&tf.to_le_bytes());
        }
    }

    // SHA-256 Trailer
    use sha2::{Digest, Sha256};
    let checksum = Sha256::digest(&v1_bytes);
    v1_bytes.extend_from_slice(&checksum);

    std::fs::write(&v1_path, &v1_bytes).unwrap();

    // Verify V1 file stats
    let v1_stats = read_index_stats(&v1_path, None).unwrap();
    assert_eq!(v1_stats.version, 1);
    assert_eq!(v1_stats.vector_bytes, (num_docs * 128 * 4) as u64);
    assert_eq!(v1_stats.vector_bytes_saved, 0);

    // Deserialize V1 index
    let deser_v1 = deserialize_vfs_index(&v1_bytes).unwrap();
    assert_eq!(deser_v1.version, 1);
    assert_eq!(deser_v1.doc_vectors.len(), num_docs);

    // Reserialize as V2
    let v2_bytes = serialize_vfs_index(
        &deser_v1.docs,
        &deser_v1.bm25,
        &deser_v1.doc_vectors,
        corpus_hash,
        created_at,
    )
    .unwrap();
    std::fs::write(&v2_path, &v2_bytes).unwrap();

    // Verify V2 file stats
    let v2_stats = read_index_stats(&v2_path, None).unwrap();
    assert_eq!(v2_stats.version, 2);
    assert_eq!(v2_stats.vector_bytes, (num_docs * VFS_QUANTIZED_VECTOR_SIZE) as u64);
    assert_eq!(v2_stats.vector_bytes_saved, (num_docs * 380) as u64);

    // Verify exact file size savings
    let exact_diff = v1_bytes.len() - v2_bytes.len();
    assert_eq!(exact_diff, num_docs * (512 - 132));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_adversarial_diff_cache_mutation_and_deletion() {
    let vfs = ContextVfs::new();

    // Add 10 documents
    for i in 0..10 {
        vfs.add_memory_record(make_test_record(
            &format!("cache-test-rec-{i}"),
            MemoryTier::L1,
            MemoryIntent::Coding,
            &format!("Summary for cache test record {i}"),
        ));
    }

    let temp_dir = std::env::temp_dir().join(format!(
        "pandora_diff_mut_test_{}_{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    let _ = std::fs::create_dir_all(&temp_dir);
    let index_path = temp_dir.join("vfs.idx");
    vfs.set_index_path(&index_path);

    vfs.reset_diff_cache_stats();
    vfs.save_index_to_disk(&index_path).unwrap();

    let initial_count = vfs.collect_all_documents().unwrap().len();
    let (hits1, misses1) = vfs.diff_cache_stats();
    assert_eq!(hits1, 0);
    assert_eq!(misses1 as usize, initial_count);

    // Populate from disk verification
    vfs.clear_diff_cache();
    assert_eq!(vfs.diff_cache().len(), 0);

    let bytes = std::fs::read(&index_path).unwrap();
    let deser = deserialize_vfs_index(&bytes).unwrap();
    vfs.populate_diff_cache_from_index(&deser);
    assert_eq!(vfs.diff_cache().len(), initial_count);

    // Incremental pass without modification: 100% hits
    vfs.reset_diff_cache_stats();
    vfs.save_index_to_disk(&index_path).unwrap();
    let (hits2, misses2) = vfs.diff_cache_stats();
    assert_eq!(hits2 as usize, initial_count);
    assert_eq!(misses2, 0);

    // Mutate 1 document's content (override meta:rules)
    let custom_rule = ContextFragment::new_with_origin(
        "meta:rules",
        ContextSource::Constitutional,
        ContextTrust::Constitutional,
        ContextClassification::Internal,
        255,
        "# Mutated Constitutional Rules\n1. Strictly hardened rules.\n2. Invariant verification.",
        32,
        None,
        ContextOrigin::new("custom", "constitution").unwrap(),
    )
    .unwrap();
    vfs.add_meta_rule(custom_rule);

    // Next pass: exactly 1 miss, N-1 hits
    vfs.reset_diff_cache_stats();
    vfs.save_index_to_disk(&index_path).unwrap();
    let (hits3, misses3) = vfs.diff_cache_stats();
    assert_eq!(misses3, 1, "mutating 1 rule should register exactly 1 miss");
    assert_eq!(hits3 as usize, initial_count - 1);
    assert_eq!(hits3 + misses3, initial_count as u64);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_adversarial_malformed_and_tampered_index() {
    let vfs = ContextVfs::new();
    vfs.add_memory_record(make_test_record(
        "tamper-rec-1",
        MemoryTier::L1,
        MemoryIntent::Coding,
        "Record for tamper resilience testing",
    ));

    let docs = vfs.collect_all_documents().unwrap();
    let (bm25, doc_vectors) = vfs.build_index_differential(&docs).unwrap();
    let corpus_hash = compute_corpus_hash(&docs);
    let bytes = serialize_vfs_index(&docs, &bm25, &doc_vectors, corpus_hash, 12345).unwrap();

    // 1. Valid deserialization
    assert!(deserialize_vfs_index(&bytes).is_ok());

    // 2. Tampered byte in payload
    let mut tampered_payload = bytes.clone();
    tampered_payload[10] ^= 0xff;
    let err = deserialize_vfs_index(&tampered_payload).unwrap_err();
    assert!(err.to_string().contains("checksum mismatch"));

    // 3. Tampered checksum in trailer
    let mut tampered_trailer = bytes.clone();
    let last = tampered_trailer.len() - 1;
    tampered_trailer[last] ^= 0x01;
    let err2 = deserialize_vfs_index(&tampered_trailer).unwrap_err();
    assert!(err2.to_string().contains("checksum mismatch"));

    // 4. Truncated byte stream
    let truncated = &bytes[..40];
    assert!(deserialize_vfs_index(truncated).is_err());

    // 5. Corrupted magic bytes
    let mut bad_magic = bytes.clone();
    bad_magic[0] = b'X';
    // Recompute trailer so it reaches magic check
    use sha2::{Digest, Sha256};
    let payload_len = bad_magic.len() - 32;
    let new_cs = Sha256::digest(&bad_magic[..payload_len]);
    bad_magic[payload_len..].copy_from_slice(&new_cs);
    let err_magic = deserialize_vfs_index(&bad_magic).unwrap_err();
    assert!(err_magic.to_string().contains("invalid index magic"));

    // 6. Unsupported version (e.g. 99)
    let mut bad_ver = bytes.clone();
    bad_ver[4..8].copy_from_slice(&99u32.to_le_bytes());
    let new_cs2 = Sha256::digest(&bad_ver[..payload_len]);
    bad_ver[payload_len..].copy_from_slice(&new_cs2);
    let err_ver = deserialize_vfs_index(&bad_ver).unwrap_err();
    assert!(err_ver.to_string().contains("unsupported index version: 99"));
}
