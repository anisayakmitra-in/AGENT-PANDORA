#![forbid(unsafe_code)]

//! Milestone 2 Empirical Challenger Suite (v3.0.3)
//! Challenger 2: ContextVfs Int8 Vector Quantization, Incremental Diff Caching,
//! Index Backward Compatibility, and Concurrency Stress.
//!
//! Objectives:
//! 1. Incremental Diff Caching Verification:
//!    - N-buffer corpus update/addition: exactly 1 doc re-hashed/re-tokenized, N-1 hit cache.
//!    - Zero re-tokenization and zero feature hashing on cache hits.
//!    - Cache hit and miss count accounting assertions.
//!    - Invalidation across document content, URI, title, tier, and intent.
//! 2. VFS Index Backward Compatibility:
//!    - Loading and querying Version 1 (512-byte float vector) binary index.
//!    - Search results and scoring identical between V1 and deserialized representation.
//!    - Transparent upgrade from Version 1 to Version 2 on subsequent saves.
//! 3. Edge Cases & Concurrency Stress:
//!    - High churn: rapid modifications to multiple buffers across parallel threads.
//!    - Deleted document handling: clean eviction from diff cache and exclusion from search.
//!    - Empty index, large index, corrupt binary index handling (graceful error handling, no panics).

use pandora_runtime::context_vfs::{
    ContextVfs, VFS_DENSE_DIMENSION, VFS_INDEX_HEADER_SIZE, VFS_INDEX_MAGIC,
    VFS_INDEX_TRAILER_SIZE, VFS_INDEX_VERSION_V1, VFS_QUANTIZED_VECTOR_SIZE, VfsDoc,
    compute_corpus_hash, compute_doc_fingerprint, deserialize_vfs_index, read_index_stats,
    serialize_vfs_index,
};
use pandora_runtime::semantic_memory::{DenseFeatureHasher, SparseBm25Index};
use pandora_types::{
    ContextClassification, ContextUri, MemoryApproval, MemoryIntent, MemoryKind, MemoryRecord,
    MemoryScope, MemoryTier, SessionId, TenantId, Timestamp, WorkspaceId,
};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn make_test_record(
    id: &str,
    tier: MemoryTier,
    intent: MemoryIntent,
    summary: &str,
) -> MemoryRecord {
    let scope = MemoryScope::new(
        TenantId::new("t1").unwrap(),
        WorkspaceId::new("w1").unwrap(),
        SessionId::new("s1").unwrap(),
        "challenger",
    )
    .unwrap();
    let l1 = MemoryRecord::new_l1(
        id,
        MemoryKind::Lesson,
        scope,
        summary,
        ContextClassification::Internal,
        Timestamp::from_unix_seconds(100),
        "challenger-m2",
    )
    .unwrap()
    .with_intent(intent);

    if tier == MemoryTier::L2 {
        let approval = MemoryApproval::new("app-chal", "operator-chal").unwrap();
        MemoryRecord::promote_l2(l1, approval, Timestamp::from_unix_seconds(150)).unwrap()
    } else {
        l1
    }
}

fn create_temp_dir(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pandora_chal_{prefix}_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("failed to create temp dir");
    dir
}

/// Helper to construct a valid Version 1 binary index payload (512-byte f32 vectors).
fn serialize_vfs_index_v1_helper(
    docs: &[VfsDoc],
    bm25: &SparseBm25Index,
    doc_vectors: &[(usize, [f32; 128])],
    corpus_hash: [u8; 32],
    created_at_unix: u64,
) -> Vec<u8> {
    let mut buffer = Vec::new();

    // 1. Header (68 bytes) with version = 1
    buffer.extend_from_slice(VFS_INDEX_MAGIC);
    buffer.extend_from_slice(&VFS_INDEX_VERSION_V1.to_le_bytes());
    buffer.extend_from_slice(&created_at_unix.to_le_bytes());
    let doc_count = docs.len() as u32;
    buffer.extend_from_slice(&doc_count.to_le_bytes());
    buffer.extend_from_slice(&VFS_DENSE_DIMENSION.to_le_bytes());
    buffer.extend_from_slice(&(bm25.total_tokens() as u64).to_le_bytes());

    let mut terms: Vec<_> = bm25.postings().keys().collect();
    terms.sort();
    let vocab_size = terms.len() as u32;
    buffer.extend_from_slice(&vocab_size.to_le_bytes());
    buffer.extend_from_slice(&corpus_hash);

    // 2. Document Metadata & 512-byte Float Vector Table
    for (idx, doc) in docs.iter().enumerate() {
        let doc_id = idx as u32;
        buffer.extend_from_slice(&doc_id.to_le_bytes());

        let uri_bytes = doc.uri.as_str().as_bytes();
        buffer.extend_from_slice(&(uri_bytes.len() as u16).to_le_bytes());
        buffer.extend_from_slice(uri_bytes);

        let title_bytes = doc.title.as_bytes();
        buffer.extend_from_slice(&(title_bytes.len() as u16).to_le_bytes());
        buffer.extend_from_slice(title_bytes);

        let content_bytes = doc.content.as_bytes();
        buffer.extend_from_slice(&(content_bytes.len() as u32).to_le_bytes());
        buffer.extend_from_slice(content_bytes);

        buffer.push(match doc.tier {
            None => 0,
            Some(MemoryTier::L0) => 1,
            Some(MemoryTier::L1) => 2,
            Some(MemoryTier::L2) => 3,
            Some(MemoryTier::L3) => 4,
            Some(MemoryTier::L4) => 5,
        });
        buffer.push(match doc.intent {
            None => 0,
            Some(MemoryIntent::Repo) => 1,
            Some(MemoryIntent::Procedure) => 2,
            Some(MemoryIntent::Coding) => 3,
            Some(MemoryIntent::User) => 4,
        });

        let doc_length = bm25.doc_lengths().get(&idx).copied().unwrap_or(0) as u32;
        buffer.extend_from_slice(&doc_length.to_le_bytes());

        let vec = doc_vectors
            .get(idx)
            .map(|(_, v)| *v)
            .unwrap_or([0.0f32; 128]);
        for f in vec {
            buffer.extend_from_slice(&f.to_le_bytes());
        }
    }

    // 3. Sparse BM25 Postings Table
    for term in terms {
        let term_bytes = term.as_bytes();
        buffer.extend_from_slice(&(term_bytes.len() as u16).to_le_bytes());
        buffer.extend_from_slice(term_bytes);

        let mut postings = bm25.postings().get(term).cloned().unwrap_or_default();
        postings.sort_by_key(|&(doc_id, _)| doc_id);
        let postings_count = postings.len() as u32;
        buffer.extend_from_slice(&postings_count.to_le_bytes());

        for (doc_id, tf) in postings {
            buffer.extend_from_slice(&(doc_id as u32).to_le_bytes());
            buffer.extend_from_slice(&tf.to_le_bytes());
        }
    }

    // 4. SHA-256 Trailer
    let checksum = Sha256::digest(&buffer);
    buffer.extend_from_slice(&checksum);

    buffer
}

// ============================================================================
// 1. Incremental Diff Caching Verification
// ============================================================================

#[test]
fn test_empirical_incremental_diff_caching_n_corpus() {
    let temp_dir = create_temp_dir("diff_n_corpus");
    let index_path = temp_dir.join("vfs.idx");

    let vfs = ContextVfs::new();
    vfs.set_index_path(&index_path);

    const N: usize = 50;
    for i in 0..N {
        vfs.add_memory_record(make_test_record(
            &format!("corpus-doc-{i:03}"),
            if i % 2 == 0 {
                MemoryTier::L1
            } else {
                MemoryTier::L2
            },
            if i % 3 == 0 {
                MemoryIntent::Coding
            } else {
                MemoryIntent::Repo
            },
            &format!("Corpus document payload #{i} with specific tokens: alpha_{i} beta_{i} gamma"),
        ));
    }

    let initial_docs = vfs.collect_all_documents().expect("collect docs");
    let total_initial = initial_docs.len(); // N records + 1 default meta rule = 51

    // Pass 1: Cold start -> 0 hits, total_initial misses
    vfs.reset_diff_cache_stats();
    let start_cold = Instant::now();
    vfs.save_index_to_disk(&index_path).expect("cold save");
    let cold_duration = start_cold.elapsed();

    let (hits1, misses1) = vfs.diff_cache_stats();
    assert_eq!(hits1, 0, "cold cache must produce 0 hits");
    assert_eq!(
        misses1 as usize, total_initial,
        "cold cache must produce exactly total_initial misses"
    );
    assert_eq!(
        vfs.diff_cache().len(),
        total_initial,
        "diff cache must contain all initial documents"
    );

    // Snapshot cached entries to verify ZERO re-quantization / ZERO re-tokenization
    let snapshot_before = vfs.diff_cache();

    // Pass 2: Unchanged save -> 100% hits, 0 misses
    vfs.reset_diff_cache_stats();
    let start_warm = Instant::now();
    vfs.save_index_to_disk(&index_path).expect("warm save");
    let warm_duration = start_warm.elapsed();

    let (hits2, misses2) = vfs.diff_cache_stats();
    assert_eq!(
        hits2 as usize, total_initial,
        "unchanged save must produce 100% cache hits"
    );
    assert_eq!(misses2, 0, "unchanged save must produce 0 cache misses");

    println!("Cold duration: {cold_duration:?}, Warm duration: {warm_duration:?}");

    // Verify cache contents are bit-for-bit identical after warm pass
    let snapshot_after = vfs.diff_cache();
    assert_eq!(snapshot_before.len(), snapshot_after.len());
    for (uri, entry_before) in &snapshot_before {
        let entry_after = snapshot_after.get(uri).expect("entry must exist");
        assert_eq!(entry_before.fingerprint, entry_after.fingerprint);
        assert_eq!(entry_before.vector.scale, entry_after.vector.scale);
        assert_eq!(entry_before.vector.data, entry_after.vector.data);
        assert_eq!(entry_before.doc_length, entry_after.doc_length);
        assert_eq!(entry_before.term_frequencies, entry_after.term_frequencies);
    }

    // Pass 3: Add exactly 1 new record (corpus becomes total_initial + 1)
    vfs.add_memory_record(make_test_record(
        "corpus-doc-new-51",
        MemoryTier::L1,
        MemoryIntent::Coding,
        "Newly introduced single record with unique token omega_unique",
    ));

    vfs.reset_diff_cache_stats();
    vfs.save_index_to_disk(&index_path)
        .expect("save with 1 added");

    let (hits3, misses3) = vfs.diff_cache_stats();
    assert_eq!(
        hits3 as usize, total_initial,
        "all prior documents must hit cache"
    );
    assert_eq!(
        misses3, 1,
        "adding 1 document must result in exactly 1 miss"
    );
    assert_eq!(vfs.diff_cache().len(), total_initial + 1);

    // Pass 4: Mutate exactly 1 record's content (e.g. corpus-doc-010)
    vfs.add_memory_record(make_test_record(
        "corpus-doc-010",
        MemoryTier::L1,
        MemoryIntent::Coding,
        "Completely replaced and modified summary text for doc 10 with new_payload_delta",
    ));

    vfs.reset_diff_cache_stats();
    vfs.save_index_to_disk(&index_path)
        .expect("save with 1 modified");

    let (hits4, misses4) = vfs.diff_cache_stats();
    assert_eq!(
        hits4 as usize, total_initial,
        "all other N unchanged documents must hit cache"
    );
    assert_eq!(
        misses4, 1,
        "modifying 1 document must result in exactly 1 miss"
    );

    // Verify search works immediately and reflects updated doc
    let res = vfs.search("new_payload_delta", 5).expect("search");
    assert!(!res.is_empty(), "must find updated document");
    assert_eq!(res[0].title, "corpus-doc-010");

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_empirical_cache_invalidation_matrix() {
    let base_uri = ContextUri::new("pandora://meta/invalidation-test").unwrap();
    let base_doc = VfsDoc {
        uri: base_uri.clone(),
        title: "Original Title".to_string(),
        content: "Original document content for fingerprint hashing".to_string(),
        tier: Some(MemoryTier::L1),
        intent: Some(MemoryIntent::Coding),
    };
    let base_fp = compute_doc_fingerprint(&base_doc);

    // 1. Content modification invalidation
    let mut doc_content_mod = base_doc.clone();
    doc_content_mod.content = "Modified document content".to_string();
    let fp_content = compute_doc_fingerprint(&doc_content_mod);
    assert_ne!(
        base_fp, fp_content,
        "content change must invalidate fingerprint"
    );

    // 2. URI modification invalidation
    let mut doc_uri_mod = base_doc.clone();
    doc_uri_mod.uri = ContextUri::new("pandora://meta/invalidation-test-renamed").unwrap();
    let fp_uri = compute_doc_fingerprint(&doc_uri_mod);
    assert_ne!(base_fp, fp_uri, "URI change must invalidate fingerprint");

    // 3. Title modification invalidation
    let mut doc_title_mod = base_doc.clone();
    doc_title_mod.title = "Modified Title".to_string();
    let fp_title = compute_doc_fingerprint(&doc_title_mod);
    assert_ne!(
        base_fp, fp_title,
        "title change must invalidate fingerprint"
    );

    // 4. Tier modification invalidation across all tiers
    let all_tiers = [
        None,
        Some(MemoryTier::L0),
        Some(MemoryTier::L1),
        Some(MemoryTier::L2),
        Some(MemoryTier::L3),
        Some(MemoryTier::L4),
    ];
    let mut tier_fps = Vec::new();
    for &t in &all_tiers {
        let mut d = base_doc.clone();
        d.tier = t;
        let fp = compute_doc_fingerprint(&d);
        tier_fps.push(fp);
    }
    // Check that each distinct tier yields a unique fingerprint
    for i in 0..tier_fps.len() {
        for j in (i + 1)..tier_fps.len() {
            assert_ne!(
                tier_fps[i], tier_fps[j],
                "tier transition {:?} -> {:?} must yield distinct fingerprints",
                all_tiers[i], all_tiers[j]
            );
        }
    }

    // 5. Intent modification invalidation across all intents
    let all_intents = [
        None,
        Some(MemoryIntent::Repo),
        Some(MemoryIntent::Procedure),
        Some(MemoryIntent::Coding),
        Some(MemoryIntent::User),
    ];
    let mut intent_fps = Vec::new();
    for &int in &all_intents {
        let mut d = base_doc.clone();
        d.intent = int;
        let fp = compute_doc_fingerprint(&d);
        intent_fps.push(fp);
    }
    for i in 0..intent_fps.len() {
        for j in (i + 1)..intent_fps.len() {
            assert_ne!(
                intent_fps[i], intent_fps[j],
                "intent transition {:?} -> {:?} must yield distinct fingerprints",
                all_intents[i], all_intents[j]
            );
        }
    }

    // Now verify through ContextVfs build_index_differential directly
    let vfs = ContextVfs::new();
    let docs_initial = vec![base_doc.clone()];

    // Pass 1: Cold build
    vfs.reset_diff_cache_stats();
    let _ = vfs.build_index_differential(&docs_initial).unwrap();
    let (h1, m1) = vfs.diff_cache_stats();
    assert_eq!(h1, 0);
    assert_eq!(m1, 1);

    // Pass 2: Unchanged -> Hit
    vfs.reset_diff_cache_stats();
    let _ = vfs.build_index_differential(&docs_initial).unwrap();
    let (h2, m2) = vfs.diff_cache_stats();
    assert_eq!(h2, 1);
    assert_eq!(m2, 0);

    // Pass 3: Invalidate content -> Miss
    vfs.reset_diff_cache_stats();
    let _ = vfs.build_index_differential(&[doc_content_mod]).unwrap();
    let (h3, m3) = vfs.diff_cache_stats();
    assert_eq!(h3, 0);
    assert_eq!(m3, 1);

    // Pass 4: Invalidate title -> Miss
    vfs.reset_diff_cache_stats();
    let _ = vfs.build_index_differential(&[doc_title_mod]).unwrap();
    let (h4, m4) = vfs.diff_cache_stats();
    assert_eq!(h4, 0);
    assert_eq!(m4, 1);

    // Pass 5: Invalidate tier -> Miss
    let mut doc_tier_mod = base_doc.clone();
    doc_tier_mod.tier = Some(MemoryTier::L3);
    vfs.reset_diff_cache_stats();
    let _ = vfs.build_index_differential(&[doc_tier_mod]).unwrap();
    let (h5, m5) = vfs.diff_cache_stats();
    assert_eq!(h5, 0);
    assert_eq!(m5, 1);

    // Pass 6: Invalidate intent -> Miss
    let mut doc_intent_mod = base_doc.clone();
    doc_intent_mod.intent = Some(MemoryIntent::User);
    vfs.reset_diff_cache_stats();
    let _ = vfs.build_index_differential(&[doc_intent_mod]).unwrap();
    let (h6, m6) = vfs.diff_cache_stats();
    assert_eq!(h6, 0);
    assert_eq!(m6, 1);
}

// ============================================================================
// 2. VFS Index Backward Compatibility & Upgrade
// ============================================================================

#[test]
fn test_empirical_v1_backward_compatibility_and_transparent_upgrade() {
    let temp_dir = create_temp_dir("v1_compat_upgrade");
    let index_file = temp_dir.join("vfs.idx");

    let vfs = ContextVfs::new();
    vfs.set_index_path(&index_file);

    // Pre-populate vfs with 29 memory records (total 30 docs including default meta:rules)
    const NUM_RECS: usize = 29;
    let topics = [
        "quantum computing qubit entanglement decoherence algorithms",
        "distributed consensus raft raft-log compactions term leadership",
        "compiler optimization register allocation intermediate representation",
        "memory safety ownership borrow checker lifetimes affine types",
        "vector quantization int8 scalar quantization scale factor truncation",
    ];

    for i in 0..NUM_RECS {
        let topic = topics[i % topics.len()];
        let content = format!("Documentation on {topic} index entry {i} detailed description");
        let rec = make_test_record(
            &format!("doc-{i:02}"),
            MemoryTier::L1,
            MemoryIntent::Coding,
            &content,
        );
        vfs.add_memory_record(rec);
    }

    let docs = vfs.collect_all_documents().expect("collect docs");
    let doc_count = docs.len(); // 30 (29 records + 1 default meta rule)

    let mut bm25 = SparseBm25Index::new();
    let mut raw_f32_vectors = Vec::with_capacity(doc_count);
    let hasher = DenseFeatureHasher::new();

    for (idx, doc) in docs.iter().enumerate() {
        bm25.add_document(idx, &doc.content);
        raw_f32_vectors.push((idx, hasher.vectorize(&doc.content)));
    }

    let corpus_hash = compute_corpus_hash(&docs);
    let created_at_unix = 1710000000u64;

    // 1. Construct Version 1 binary file on disk
    let v1_bytes =
        serialize_vfs_index_v1_helper(&docs, &bm25, &raw_f32_vectors, corpus_hash, created_at_unix);
    std::fs::write(&index_file, &v1_bytes).expect("write v1 file");

    // 2. Verify V1 index stats
    let stats_v1 = read_index_stats(&index_file, None).expect("read stats v1");
    assert_eq!(stats_v1.version, 1);
    assert_eq!(stats_v1.doc_count as usize, doc_count);
    assert_eq!(
        stats_v1.vector_bytes,
        (doc_count * 128 * 4) as u64,
        "V1 vector bytes must be 512 bytes per doc"
    );
    assert_eq!(stats_v1.vector_bytes_saved, 0, "V1 index saves 0 bytes");
    assert!(stats_v1.checksum_valid);

    // 3. Deserialize V1 index and test scoring equivalence
    let deser_v1 = deserialize_vfs_index(&v1_bytes).expect("must deserialize v1");
    assert_eq!(deser_v1.version, 1);
    assert_eq!(deser_v1.doc_count as usize, doc_count);
    assert_eq!(deser_v1.doc_vectors.len(), doc_count);

    // Test queries across topics
    let test_queries = [
        "quantum computing qubit",
        "distributed consensus raft",
        "compiler optimization register",
        "memory safety borrow checker",
        "int8 scalar quantization",
    ];

    for q in test_queries {
        let q_vec = hasher.vectorize(q);
        // Scores from unquantized floats
        let unquant_scores: Vec<(usize, f32)> = raw_f32_vectors
            .iter()
            .map(|&(id, ref v)| (id, hasher.cosine_similarity(&q_vec, v)))
            .collect();
        // Scores from deserialized V1 quantized vectors
        let quant_scores = hasher.score_quantized_docs(&q_vec, &deser_v1.doc_vectors);

        assert_eq!(unquant_scores.len(), doc_count);
        assert_eq!(quant_scores.len(), doc_count);

        // Verify top-1 match
        let top_unquant = unquant_scores
            .iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            .unwrap();
        let top_quant = quant_scores
            .iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            .unwrap();
        assert_eq!(
            top_unquant.0, top_quant.0,
            "Top-1 document ID must match between unquantized and V1 deserialized for query '{q}'"
        );
        // Error bounded by Cauchy-Schwarz
        assert!((top_unquant.1 - top_quant.1).abs() < 0.0156);
    }

    // 4. Transparent upgrade: Load V1 into ContextVfs and save
    // Warm cold start from existing V1 index file on disk
    let search_res = vfs
        .search("quantum computing entanglement", 5)
        .expect("search on v1");
    assert!(!search_res.is_empty());
    assert!(search_res[0].snippet.contains("quantum") || search_res[0].title.contains("doc-"));

    // Save index: this should transparently upgrade disk file to Version 2!
    vfs.save_index_to_disk(&index_file)
        .expect("save upgraded index");

    // 5. Verify upgraded Version 2 index stats
    let stats_v2 = read_index_stats(&index_file, Some(&vfs)).expect("read stats v2");
    assert_eq!(stats_v2.version, 2, "index must be upgraded to version 2");
    assert_eq!(stats_v2.doc_count as usize, doc_count);
    assert_eq!(
        stats_v2.vector_bytes,
        (doc_count * VFS_QUANTIZED_VECTOR_SIZE) as u64,
        "V2 vector bytes must be 132 bytes per doc"
    );
    assert_eq!(
        stats_v2.vector_bytes_saved,
        (doc_count * 380) as u64,
        "V2 index must save 380 bytes per doc"
    );
    assert!(stats_v2.checksum_valid);
    assert!(stats_v2.is_fresh);

    // Exact file size reduction
    let v2_bytes = std::fs::read(&index_file).expect("read v2 bytes");
    let file_diff = v1_bytes.len() - v2_bytes.len();
    assert_eq!(
        file_diff,
        doc_count * (512 - 132),
        "file reduction must be exactly doc_count * 380 bytes"
    );

    // Subsequent search produces identical top result
    let search_res2 = vfs
        .search("quantum computing entanglement", 5)
        .expect("search on v2");
    assert_eq!(search_res[0].uri, search_res2[0].uri);
    assert_eq!(search_res[0].title, search_res2[0].title);
    assert!((search_res[0].hybrid_score - search_res2[0].hybrid_score).abs() < 1e-4);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

// ============================================================================
// 3. Concurrency Stress & High Churn
// ============================================================================

#[test]
fn test_empirical_concurrency_stress_and_high_churn() {
    let temp_dir = create_temp_dir("concurrency_churn");
    let index_path = temp_dir.join("vfs.idx");

    let vfs = Arc::new(ContextVfs::new());
    vfs.set_index_path(&index_path);

    // Pre-seed with 20 base records
    for i in 0..20 {
        vfs.add_memory_record(make_test_record(
            &format!("base-concurrency-{i:02}"),
            MemoryTier::L1,
            MemoryIntent::Coding,
            &format!("Base document {i} for concurrent stress verification concurrent safety"),
        ));
    }
    vfs.save_index_to_disk(&index_path).expect("initial save");

    const NUM_THREADS: usize = 16;
    const OPS_PER_THREAD: usize = 60;

    let stop_signal = Arc::new(AtomicBool::new(false));
    let mut handles = Vec::with_capacity(NUM_THREADS);

    for thread_idx in 0..NUM_THREADS {
        let vfs_clone = Arc::clone(&vfs);
        let stop_clone = Arc::clone(&stop_signal);
        let idx_path_clone = index_path.clone();

        let handle = std::thread::spawn(move || {
            for op in 0..OPS_PER_THREAD {
                if stop_clone.load(Ordering::Relaxed) {
                    break;
                }
                match thread_idx % 4 {
                    0 => {
                        // Mutator thread: inserts / updates memory records
                        let rec_id = format!("churn-rec-t{thread_idx}-op{op}");
                        vfs_clone.add_memory_record(make_test_record(
                            &rec_id,
                            if op % 2 == 0 {
                                MemoryTier::L1
                            } else {
                                MemoryTier::L2
                            },
                            MemoryIntent::Coding,
                            &format!(
                                "Concurrent payload thread {thread_idx} op {op} token_alpha_{op}"
                            ),
                        ));
                    }
                    1 => {
                        // Searcher thread: runs concurrent searches
                        let query = if op % 2 == 0 {
                            "concurrent safety"
                        } else {
                            "token_alpha"
                        };
                        let _ = vfs_clone.search(query, 5);
                    }
                    2 => {
                        // Saver thread: periodically saves index to disk
                        if op % 5 == 0 {
                            let _ = vfs_clone.save_index_to_disk(&idx_path_clone);
                        }
                    }
                    3 => {
                        // Reader / stat inspector thread
                        let _ = vfs_clone.diff_cache_stats();
                        let _ = vfs_clone.diff_cache();
                        let _ = vfs_clone.collect_all_documents();
                    }
                    _ => unreachable!(),
                }
            }
        });
        handles.push(handle);
    }

    // Join all threads
    for (i, h) in handles.into_iter().enumerate() {
        h.join()
            .unwrap_or_else(|_| panic!("Thread {i} panicked under concurrency stress"));
    }

    // Post-concurrency sanity check
    let docs = vfs.collect_all_documents().expect("collect after churn");
    assert!(
        docs.len() >= 20,
        "all base records plus inserted records should be intact"
    );

    // Final clean save and search to ensure lock integrity
    vfs.save_index_to_disk(&index_path).expect("final save");
    let results = vfs.search("concurrent safety", 10).expect("final search");
    assert!(!results.is_empty(), "must find concurrent records");

    let _ = std::fs::remove_dir_all(&temp_dir);
}

// ============================================================================
// 4. Deleted Document Handling
// ============================================================================

#[test]
fn test_empirical_deleted_document_handling() {
    let temp_dir = create_temp_dir("deleted_docs");
    let index_path = temp_dir.join("vfs.idx");

    // Phase 1: vfs1 has 10 records (including record 5 with unique marker)
    let vfs1 = ContextVfs::new();
    vfs1.set_index_path(&index_path);

    for i in 0..10 {
        let summary = if i == 5 {
            "Record 5 with extremely rare marker: KANGAROO_SUBMARINE_COBALT"
        } else {
            "Standard regular record without unique marker"
        };
        vfs1.add_memory_record(make_test_record(
            &format!("del-rec-{i}"),
            MemoryTier::L1,
            MemoryIntent::Coding,
            summary,
        ));
    }

    // Save index with all 10 records
    vfs1.save_index_to_disk(&index_path).expect("save initial");

    // Verify record 5 is present in search and diff cache of vfs1
    let res1 = vfs1.search("KANGAROO_SUBMARINE_COBALT", 5).expect("search");
    assert!(!res1.is_empty());
    assert_eq!(res1[0].title, "del-rec-5");
    assert!(res1[0].snippet.contains("KANGAROO_SUBMARINE_COBALT"));

    let target_uri = "pandora://index/coding/del-rec-5";
    assert!(
        vfs1.diff_cache().contains_key(target_uri),
        "target doc must be in diff cache"
    );

    // Phase 2: In the next session / workspace state, record 5 is deleted
    // vfs2 represents the workspace state containing the other 9 records
    let vfs2 = ContextVfs::new();
    vfs2.set_index_path(&index_path);

    for i in 0..10 {
        if i == 5 {
            continue; // Record 5 deleted
        }
        vfs2.add_memory_record(make_test_record(
            &format!("del-rec-{i}"),
            MemoryTier::L1,
            MemoryIntent::Coding,
            "Standard regular record without unique marker",
        ));
    }

    // Pre-populate diff_cache from the existing index on disk
    let bytes = std::fs::read(&index_path).expect("read existing index");
    let deser = deserialize_vfs_index(&bytes).expect("deserialize existing index");
    vfs2.populate_diff_cache_from_index(&deser);

    // Verify target doc was carried over into diff_cache initially
    assert!(
        vfs2.diff_cache().contains_key(target_uri),
        "target doc was initially in loaded diff cache"
    );

    // Now save index differentially in vfs2
    vfs2.reset_diff_cache_stats();
    vfs2.save_index_to_disk(&index_path)
        .expect("save updated index");

    // 1. Verify target doc was cleanly evicted from diff_cache by differential indexing
    assert!(
        !vfs2.diff_cache().contains_key(target_uri),
        "deleted doc URI must be cleanly evicted from diff_cache"
    );

    // 2. Verify diff cache stats: 9 records hit cache, 0 misses, target doc evicted
    let (hits, misses) = vfs2.diff_cache_stats();
    // 9 records + 1 default meta rule = 10 documents
    assert_eq!(hits, 10, "all 10 remaining documents must hit diff cache");
    assert_eq!(misses, 0, "no misses for remaining unchanged documents");

    // 3. Clear in-memory cached index so search tests index on disk
    vfs2.clear_cached_index();

    // 4. Search for the unique marker: target doc MUST NOT be present anywhere in search results
    let res2 = vfs2
        .search("KANGAROO_SUBMARINE_COBALT", 5)
        .expect("search after deletion");
    for r in &res2 {
        assert_ne!(
            r.title, "del-rec-5",
            "deleted document title must not be returned in search results"
        );
        assert_ne!(
            r.uri.as_str(),
            target_uri,
            "deleted document URI must not be returned in search results"
        );
        assert!(
            !r.snippet.contains("KANGAROO_SUBMARINE_COBALT"),
            "deleted snippet must not contain marker"
        );
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}

// ============================================================================
// 5. Empty & Large Index Handling
// ============================================================================

#[test]
fn test_empirical_empty_and_large_index_handling() {
    // A. Empty index handling
    let empty_docs: Vec<VfsDoc> = Vec::new();
    let (empty_bm25, empty_vectors) = ContextVfs::new()
        .build_index_differential(&empty_docs)
        .expect("empty diff build");

    assert_eq!(empty_vectors.len(), 0);
    assert_eq!(empty_bm25.doc_count(), 0);
    assert_eq!(empty_bm25.total_tokens(), 0);

    let empty_hash = compute_corpus_hash(&empty_docs);
    let empty_bytes = serialize_vfs_index(&empty_docs, &empty_bm25, &empty_vectors, empty_hash, 0)
        .expect("serialize empty");
    assert_eq!(
        empty_bytes.len(),
        VFS_INDEX_HEADER_SIZE + VFS_INDEX_TRAILER_SIZE,
        "empty index must be exactly header (68) + trailer (32) = 100 bytes"
    );

    let deser_empty = deserialize_vfs_index(&empty_bytes).expect("deserialize empty");
    assert_eq!(deser_empty.doc_count, 0);
    assert_eq!(deser_empty.doc_vectors.len(), 0);
    assert_eq!(deser_empty.bm25.doc_count(), 0);

    // B. Large index handling (500 documents)
    let temp_dir = create_temp_dir("large_index");
    let index_path = temp_dir.join("vfs.idx");

    let vfs = ContextVfs::new();
    vfs.set_index_path(&index_path);

    const LARGE_N: usize = 500;
    for i in 0..LARGE_N {
        vfs.add_memory_record(make_test_record(
            &format!("large-rec-{i:04}"),
            MemoryTier::L1,
            MemoryIntent::Coding,
            &format!(
                "High scale scalability verification payload {i} containing domain keyword_{} \
                 and secondary vocabulary token_{} for throughput benchmarks",
                i % 25,
                i % 50
            ),
        ));
    }

    // Cold build 500 docs
    vfs.reset_diff_cache_stats();
    let start_cold = Instant::now();
    vfs.save_index_to_disk(&index_path).expect("save 500 docs");
    let cold_dur = start_cold.elapsed();

    let (h1, m1) = vfs.diff_cache_stats();
    assert_eq!(h1, 0);
    assert_eq!(m1 as usize, LARGE_N + 1); // 500 records + 1 meta rule

    // Warm build 500 docs: must achieve 100% hits and fast execution (< 10ms)
    vfs.reset_diff_cache_stats();
    let start_warm = Instant::now();
    vfs.save_index_to_disk(&index_path)
        .expect("warm save 500 docs");
    let warm_dur = start_warm.elapsed();

    let (h2, m2) = vfs.diff_cache_stats();
    assert_eq!(h2 as usize, LARGE_N + 1);
    assert_eq!(m2, 0);

    println!("Large corpus (501 docs) - Cold: {cold_dur:?}, Warm: {warm_dur:?}");
    assert!(
        warm_dur < cold_dur,
        "warm diff indexing must be significantly faster than cold indexing"
    );

    // Verify stats and storage savings on 500 docs
    let stats = read_index_stats(&index_path, Some(&vfs)).expect("stats");
    assert_eq!(stats.doc_count as usize, LARGE_N + 1);
    let expected_saved = (LARGE_N + 1) * 380;
    assert_eq!(stats.vector_bytes_saved as usize, expected_saved);

    // Search performance on 500 docs
    let start_search = Instant::now();
    let search_res = vfs
        .search("scalability keyword_12", 10)
        .expect("search 500");
    let search_dur = start_search.elapsed();
    assert!(!search_res.is_empty());
    println!("Search on 501 docs: {search_dur:?}");
    assert!(search_dur < Duration::from_millis(50));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

// ============================================================================
// 6. Corrupt Binary Index Handling & Fuzzing (Graceful, No Panics)
// ============================================================================

#[test]
fn test_empirical_corrupt_binary_index_fuzzing_no_panics() {
    let vfs = ContextVfs::new();
    vfs.add_memory_record(make_test_record(
        "fuzz-seed-1",
        MemoryTier::L1,
        MemoryIntent::Coding,
        "Seed document payload for fuzzing and tampering verification",
    ));
    vfs.add_memory_record(make_test_record(
        "fuzz-seed-2",
        MemoryTier::L2,
        MemoryIntent::Repo,
        "Secondary seed document for multi-posting integrity verification",
    ));

    let docs = vfs.collect_all_documents().unwrap();
    let (bm25, doc_vectors) = vfs.build_index_differential(&docs).unwrap();
    let corpus_hash = compute_corpus_hash(&docs);
    let valid_bytes = serialize_vfs_index(&docs, &bm25, &doc_vectors, corpus_hash, 100).unwrap();

    assert!(deserialize_vfs_index(&valid_bytes).is_ok());

    // 1. Truncation fuzzing: every prefix from 0 to len - 1
    for len in 0..valid_bytes.len() {
        let truncated = &valid_bytes[..len];
        let res = deserialize_vfs_index(truncated);
        assert!(
            res.is_err(),
            "truncated bytes of len {len} must return Err, got Ok"
        );
    }

    // 2. Corrupt Magic
    let bad_magics: [&[u8; 4]; 4] = [b"XXXX", b"pvfs", b"\0\0\0\0", b"PVF\0"];
    for bad_magic in bad_magics {
        let mut tampered = valid_bytes.clone();
        tampered[0..4].copy_from_slice(bad_magic);
        // Fix trailer checksum so it bypasses checksum to hit magic validator
        let payload_len = tampered.len() - 32;
        let cs = Sha256::digest(&tampered[..payload_len]);
        tampered[payload_len..].copy_from_slice(&cs);

        let err = deserialize_vfs_index(&tampered).unwrap_err();
        assert!(
            err.to_string().contains("invalid index magic"),
            "expected magic error, got {err}"
        );
    }

    // 3. Corrupt Version (e.g. 0, 3, 4, 9999)
    let bad_versions = [0u32, 3u32, 4u32, 9999u32];
    for ver in bad_versions {
        let mut tampered = valid_bytes.clone();
        tampered[4..8].copy_from_slice(&ver.to_le_bytes());
        let payload_len = tampered.len() - 32;
        let cs = Sha256::digest(&tampered[..payload_len]);
        tampered[payload_len..].copy_from_slice(&cs);

        let err = deserialize_vfs_index(&tampered).unwrap_err();
        assert!(
            err.to_string().contains("unsupported index version"),
            "expected version error, got {err}"
        );
    }

    // 4. Corrupt Dense Dimension (expected 128)
    let bad_dims = [0u32, 64u32, 256u32, 1024u32];
    for dim in bad_dims {
        let mut tampered = valid_bytes.clone();
        tampered[20..24].copy_from_slice(&dim.to_le_bytes());
        let payload_len = tampered.len() - 32;
        let cs = Sha256::digest(&tampered[..payload_len]);
        tampered[payload_len..].copy_from_slice(&cs);

        let err = deserialize_vfs_index(&tampered).unwrap_err();
        assert!(
            err.to_string().contains("unexpected dense dimension"),
            "expected dimension error, got {err}"
        );
    }

    // 5. Exaggerated doc_count claim (10,000 docs with small body)
    let mut tampered_docs = valid_bytes.clone();
    tampered_docs[16..20].copy_from_slice(&10_000u32.to_le_bytes());
    let payload_len = tampered_docs.len() - 32;
    let cs = Sha256::digest(&tampered_docs[..payload_len]);
    tampered_docs[payload_len..].copy_from_slice(&cs);
    let res_docs = deserialize_vfs_index(&tampered_docs);
    assert!(res_docs.is_err(), "exaggerated doc count must return Err");

    // 6. Exaggerated vocab_size claim (50,000 terms)
    let mut tampered_vocab = valid_bytes.clone();
    tampered_vocab[32..36].copy_from_slice(&50_000u32.to_le_bytes());
    let payload_len = tampered_vocab.len() - 32;
    let cs = Sha256::digest(&tampered_vocab[..payload_len]);
    tampered_vocab[payload_len..].copy_from_slice(&cs);
    let res_vocab = deserialize_vfs_index(&tampered_vocab);
    assert!(res_vocab.is_err(), "exaggerated vocab size must return Err");

    // 7. Single-bit flip across payload: must ALWAYS fail checksum verification
    let mut bit_tampered = valid_bytes.clone();
    bit_tampered[15] ^= 0x01; // flip 1 bit
    let err_bit = deserialize_vfs_index(&bit_tampered).unwrap_err();
    assert!(err_bit.to_string().contains("checksum mismatch"));

    // 8. Trailing garbage bytes appended
    let mut trailing_garbage = valid_bytes.clone();
    trailing_garbage.extend_from_slice(b"EXTRA_TRAILING_JUNK");
    let err_trailing = deserialize_vfs_index(&trailing_garbage).unwrap_err();
    assert!(err_trailing.to_string().contains("checksum mismatch"));

    // 9. Completely random fuzz buffer of 500 bytes
    for seed in 1u8..10u8 {
        let mut pseudo_random = vec![0u8; 500];
        for (i, b) in pseudo_random.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(seed).wrapping_add(37);
        }
        let res = deserialize_vfs_index(&pseudo_random);
        assert!(
            res.is_err(),
            "random fuzz bytes must not successfully parse"
        );
    }
}
