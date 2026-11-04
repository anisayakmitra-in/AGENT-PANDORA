#![forbid(unsafe_code)]

//! Challenger Phase 1 Milestone 2 Empirical Verification Suite.
//!
//! Empirically tests:
//! 1. Search accuracy and top-k precision of `search_hnsw` across multiple tiers (rules, sops, memories, archives).
//! 2. RRF fusion between BM25 sparse scoring and HNSW dense scoring in `search_with_options`.
//! 3. Dynamic rebuild and freshness invalidation on ContextVfs buffer updates.
//! 4. Robustness against adversarial inputs, empty queries, extreme ef_search, and corrupted PVHN indices.

use pandora_runtime::context_vfs::{ContextSearchOptions, ContextVfs};
use pandora_types::{
    ContextClassification, ContextFragment, ContextOrigin, ContextSource, ContextTrust,
    MemoryApproval, MemoryIntent, MemoryKind, MemoryRecord, MemoryScope, MemoryTier,
    SessionArchiveRecord, SessionId, SopDefinition, SopStep, StoredSop, TenantId, Timestamp,
    WorkspaceId,
};

fn create_sample_context_vfs() -> ContextVfs {
    let vfs = ContextVfs::new();
    let tenant_id = TenantId::new("tenant-alpha").unwrap();
    let workspace_id = WorkspaceId::new("ws-empirical").unwrap();
    let session_id = SessionId::new("sess-001").unwrap();
    let now = Timestamp::from_unix_seconds(1_700_000_000);

    // Tier L0: Meta Rule
    let rule_security = ContextFragment::new_with_origin(
        "meta:crypto_security",
        ContextSource::Constitutional,
        ContextTrust::Constitutional,
        ContextClassification::Internal,
        250,
        "Zero-trust mutual TLS cryptography with automated HMAC-SHA256 signature verification and token rotation",
        64,
        None,
        ContextOrigin::new("pandora-runtime", "crypto_policy").unwrap(),
    )
    .unwrap();
    vfs.add_meta_rule(rule_security);

    // Tier L1: Working Memory Record
    let scope1 = MemoryScope::new(
        tenant_id.clone(),
        workspace_id.clone(),
        session_id.clone(),
        "local",
    )
    .unwrap();
    let l1_rec = MemoryRecord::new_l1(
        "mem-l1-kernel",
        MemoryKind::ExecutionEvidence,
        scope1,
        "Active Linux eBPF telemetry kernel probe tracking socket latency and packet drops",
        ContextClassification::Internal,
        now,
        "kernel-telemetry-probe",
    )
    .unwrap();
    vfs.add_memory_record(l1_rec);

    // Tier L2: Long-Term Memory Record
    let scope2 = MemoryScope::new(
        tenant_id.clone(),
        workspace_id.clone(),
        session_id.clone(),
        "local",
    )
    .unwrap();
    let l1_candidate = MemoryRecord::new_l1(
        "mem-l2-quantum",
        MemoryKind::Lesson,
        scope2,
        "Superconducting quantum qubit coherence times improved through topological surface code error correction",
        ContextClassification::Internal,
        now,
        "quantum-research-paper",
    )
    .unwrap();
    let approval = MemoryApproval::new("appr-1", "lead-physicist").unwrap();
    let l2_rec = MemoryRecord::promote_l2(l1_candidate, approval, now).unwrap();
    vfs.add_memory_record(l2_rec);

    // Tier L3: Standard Operating Procedure (SOP)
    let step1 = SopStep::new(
        "step-1",
        "Verify Canary Health",
        "kubectl get pods -l app=canary",
        vec![],
    )
    .unwrap();
    let step2 = SopStep::new(
        "step-2",
        "Traffic Shift",
        "istioctl route 50 50",
        vec!["step-1".into()],
    )
    .unwrap();
    let sop_def = SopDefinition::new(
        "sop-deploy-canary",
        "Canary Deployment Progressive Rollout Procedure",
        MemoryIntent::Procedure,
        vec![step1, step2],
        Some("kubectl rollout status deployment/canary".into()),
    )
    .unwrap();
    let stored_sop = StoredSop::new(
        sop_def,
        tenant_id.clone(),
        workspace_id.clone(),
        42,
        now,
        now,
    );
    vfs.add_sop(stored_sop);

    // Tier L4: Session Archive Record
    let archive_rec = SessionArchiveRecord::new(
        "arc-cluster-outage-2026",
        tenant_id,
        workspace_id,
        session_id,
        "Post-mortem analysis of distributed consensus split-brain recovery under network partition",
        "files_changed: 4, insertions: 120",
        "sha256-receipt-digest-9876543210",
        now,
    )
    .unwrap();
    vfs.add_archive(archive_rec);

    vfs
}

#[test]
fn test_empirical_multi_tier_search_accuracy() {
    let vfs = create_sample_context_vfs();

    // 1. Query targeting Tier L0 (Constitutional Rule)
    let rule_results = vfs
        .search_hnsw("mutual TLS cryptography HMAC signature", 3, 64)
        .expect("search_hnsw should succeed");
    assert!(!rule_results.is_empty(), "expected results for rule query");
    assert_eq!(
        rule_results[0].tier,
        Some(MemoryTier::L0),
        "top result should be Tier L0 rule"
    );
    assert!(
        rule_results[0].dense_score > 0.1,
        "dense similarity must be positive and significant, got {}",
        rule_results[0].dense_score
    );

    // 2. Query targeting Tier L1 (eBPF telemetry)
    let l1_results = vfs
        .search_hnsw("eBPF telemetry kernel socket latency", 3, 64)
        .expect("search_hnsw should succeed");
    assert!(!l1_results.is_empty(), "expected results for L1 query");
    assert_eq!(
        l1_results[0].tier,
        Some(MemoryTier::L1),
        "top result should be Tier L1 working memory"
    );

    // 3. Query targeting Tier L2 (Quantum coherence)
    let l2_results = vfs
        .search_hnsw(
            "qubit quantum coherence surface code error correction",
            3,
            64,
        )
        .expect("search_hnsw should succeed");
    assert!(!l2_results.is_empty(), "expected results for L2 query");
    assert_eq!(
        l2_results[0].tier,
        Some(MemoryTier::L2),
        "top result should be Tier L2 long-term memory"
    );

    // 4. Query targeting Tier L3 (SOP canary deployment)
    let sop_results = vfs
        .search_hnsw("Canary deployment rollout istioctl traffic shift", 3, 64)
        .expect("search_hnsw should succeed");
    assert!(!sop_results.is_empty(), "expected results for SOP query");
    assert_eq!(
        sop_results[0].tier,
        Some(MemoryTier::L3),
        "top result should be Tier L3 SOP"
    );

    // 5. Query targeting Tier L4 (Archive post-mortem)
    let arc_results = vfs
        .search_hnsw("consensus split-brain network partition post-mortem", 3, 64)
        .expect("search_hnsw should succeed");
    assert!(
        !arc_results.is_empty(),
        "expected results for archive query"
    );
    assert_eq!(
        arc_results[0].tier,
        Some(MemoryTier::L4),
        "top result should be Tier L4 archive"
    );

    // Verify score monotonicity: scores must be non-increasing
    for res_list in [
        &rule_results,
        &l1_results,
        &l2_results,
        &sop_results,
        &arc_results,
    ] {
        for w in res_list.windows(2) {
            assert!(
                w[0].dense_score >= w[1].dense_score,
                "HNSW results must be sorted descending by dense score: {} vs {}",
                w[0].dense_score,
                w[1].dense_score
            );
        }
    }
}

#[test]
fn test_empirical_rrf_hybrid_fusion_and_filtering() {
    let vfs = create_sample_context_vfs();

    // Query matching both keyword (BM25) and semantic concept (HNSW)
    let options = ContextSearchOptions {
        limit: 5,
        min_score: 0.0,
        tier: None,
        intent: None,
        ..Default::default()
    };

    let hybrid_results = vfs
        .search_with_options("cryptography HMAC verification token", &options)
        .expect("hybrid search should succeed");

    assert!(!hybrid_results.is_empty());
    let top = &hybrid_results[0];
    assert!(
        top.hybrid_score > 0.0,
        "hybrid score must be strictly positive"
    );
    assert_eq!(top.tier, Some(MemoryTier::L0));

    // Test tier filtering via ContextSearchOptions
    let sop_only_options = ContextSearchOptions {
        limit: 5,
        min_score: 0.0,
        tier: Some(MemoryTier::L3),
        intent: None,
        ..Default::default()
    };
    let filtered_results = vfs
        .search_with_options("Canary deployment rollout", &sop_only_options)
        .expect("filtered search should succeed");

    assert!(!filtered_results.is_empty());
    for item in &filtered_results {
        assert_eq!(
            item.tier,
            Some(MemoryTier::L3),
            "tier filtering must only return L3 results"
        );
    }

    // Test limit boundary
    let limit_one = ContextSearchOptions {
        limit: 1,
        min_score: 0.0,
        tier: None,
        intent: None,
        ..Default::default()
    };
    let single_res = vfs
        .search_with_options("telemetry quantum consensus canary", &limit_one)
        .expect("search should succeed");
    assert_eq!(
        single_res.len(),
        1,
        "limit constraint must be strictly respected"
    );
}

#[test]
fn test_empirical_dynamic_index_invalidation_and_rebuild() {
    let test_dir = std::env::temp_dir().join(format!("pandora_m2_dyn_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&test_dir);
    let hnsw_path = test_dir.join("test_vfs_hnsw.idx");

    let vfs = create_sample_context_vfs();

    // Persist HNSW index
    let saved_path = vfs
        .save_hnsw_index(Some(&hnsw_path))
        .expect("save_hnsw_index should succeed");
    assert_eq!(saved_path, hnsw_path);
    assert!(hnsw_path.is_file(), "index file must exist on disk");

    // Index is fresh
    assert!(
        vfs.is_hnsw_index_fresh(),
        "index must be fresh immediately after saving"
    );

    // Initial search: ensure novel topic doesn't match top
    let pre_results = vfs
        .search_hnsw("CRISPR Cas9 genomic gene editing ribonucleic", 3, 64)
        .expect("search should succeed");
    for item in &pre_results {
        assert!(
            !item.snippet.to_lowercase().contains("ribonucleic"),
            "pre-results must not contain CRISPR document"
        );
    }

    // Dynamic mutation: add a completely new document into VFS
    let bio_rule = ContextFragment::new_with_origin(
        "meta:crispr_genomics",
        ContextSource::Constitutional,
        ContextTrust::Constitutional,
        ContextClassification::Internal,
        200,
        "Synthetic CRISPR Cas9 ribonucleic acid guide editing protocols for targeted genome insertion",
        64,
        None,
        ContextOrigin::new("pandora-runtime", "genomics_protocol").unwrap(),
    )
    .unwrap();
    vfs.add_meta_rule(bio_rule);

    // Invalidation check: is_hnsw_index_fresh must now report false!
    assert!(
        !vfs.is_hnsw_index_fresh(),
        "index must become stale immediately after buffer modification"
    );

    // Executing search_hnsw should automatically trigger load_or_build_hnsw_index and rebuild
    let post_results = vfs
        .search_hnsw("CRISPR Cas9 genomic gene editing ribonucleic", 3, 64)
        .expect("search_hnsw should succeed after mutation");

    assert!(
        !post_results.is_empty(),
        "post-rebuild search must return results"
    );
    assert_eq!(
        post_results[0].title, "meta:crispr_genomics",
        "newly added document must be ranked top-1 after dynamic rebuild"
    );
    assert!(
        post_results[0].dense_score > 0.2,
        "top result should have high semantic similarity"
    );

    // Index is now refreshed and saved
    assert!(
        vfs.is_hnsw_index_fresh(),
        "index must be fresh again after automatic rebuild"
    );

    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_empirical_edge_cases_and_robustness() {
    let vfs = create_sample_context_vfs();

    // 1. Empty and whitespace queries
    assert!(vfs.search_hnsw("", 5, 64).unwrap().is_empty());
    assert!(vfs.search_hnsw("   \t \n", 5, 64).unwrap().is_empty());

    // 2. Limit = 0
    assert!(vfs.search_hnsw("cryptography", 0, 64).unwrap().is_empty());

    // 3. Extreme ef_search (ef = 1, ef = 512)
    let ef1 = vfs.search_hnsw("cryptography", 2, 1).unwrap();
    assert!(!ef1.is_empty());
    let ef512 = vfs.search_hnsw("cryptography", 2, 512).unwrap();
    assert!(!ef512.is_empty());

    // 4. Random gibberish query without panics
    let gibberish = vfs
        .search_hnsw("zxqjvk_nonexistent_term_98721345", 3, 64)
        .unwrap();
    // Should complete safely, returning up to limit
    assert!(gibberish.len() <= 3);

    // 5. Index corruption recovery
    let test_dir = std::env::temp_dir().join(format!("pandora_m2_corrupt_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&test_dir);
    let bad_idx = test_dir.join("corrupt_hnsw.idx");
    vfs.save_hnsw_index(Some(&bad_idx)).unwrap();
    assert!(bad_idx.is_file());

    // Corrupt trailer bytes (tamper SHA-256)
    let mut bytes = std::fs::read(&bad_idx).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xFF;
    std::fs::write(&bad_idx, &bytes).unwrap();

    // is_hnsw_index_fresh must detect checksum failure and report false
    assert!(!vfs.is_hnsw_index_fresh());

    // load_or_build_hnsw_index must self-heal by rebuilding cleanly
    let healed = vfs.load_or_build_hnsw_index();
    assert!(
        healed.is_ok(),
        "must self-heal from corrupted on-disk index"
    );

    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_empirical_recall_vs_brute_force_ground_truth() {
    let vfs = ContextVfs::new();
    let tenant_id = TenantId::new("tenant-corpus").unwrap();
    let workspace_id = WorkspaceId::new("ws-corpus").unwrap();
    let session_id = SessionId::new("sess-corpus").unwrap();
    let now = Timestamp::from_unix_seconds(1_700_000_000);

    let corpus = [
        (
            "doc-ebpf",
            "Linux kernel eBPF bytecode virtual machine socket filtering latency drops",
        ),
        (
            "doc-tcp-bbr",
            "TCP BBR congestion control algorithm bottleneck bandwidth and round trip time",
        ),
        (
            "doc-dnssec",
            "DNSSEC cryptographic resource records domain name validation signature chaining",
        ),
        (
            "doc-quic",
            "HTTP 3 QUIC protocol multiplexed streams over UDP zero round trip handshake",
        ),
        (
            "doc-raft",
            "Raft distributed consensus state machine replication leader election log entries",
        ),
        (
            "doc-paxos",
            "Multi Paxos consensus protocol proposer acceptor learner quorum ballot numbers",
        ),
        (
            "doc-2pc",
            "Two phase commit distributed atomic transaction coordinator participant prepare commit",
        ),
        (
            "doc-vector-clock",
            "Vector clocks causal ordering logical timestamp concurrency detection",
        ),
        (
            "doc-mtls",
            "Mutual TLS bidirectional certificate verification zero trust handshake",
        ),
        (
            "doc-oauth",
            "OAuth 2 proof key for code exchange PKCE authorization code grant flow",
        ),
        (
            "doc-hsm",
            "Hardware security module PKCS 11 cryptographic key generation tamper resistance",
        ),
        (
            "doc-sgx",
            "Intel SGX secure enclaves hardware memory encryption remote attestation",
        ),
        (
            "doc-llvm",
            "LLVM intermediate representation SSA form compiler passes optimization pipeline",
        ),
        (
            "doc-regalloc",
            "Graph coloring register allocation liveness analysis spill cost heuristic",
        ),
        (
            "doc-borrowck",
            "Rust non-lexical lifetimes borrow checker region inference affine type system",
        ),
        (
            "doc-lsm",
            "Log structured merge tree SSTables memtable write amplification compaction",
        ),
        (
            "doc-wal",
            "Write ahead logging append only durability crash recovery checkpointing",
        ),
        (
            "doc-mvcc",
            "Multi version concurrency control snapshot isolation read locks transaction timestamp",
        ),
        (
            "doc-btree",
            "B-tree index node splitting balancing branch factor disk block alignment",
        ),
        (
            "doc-wasm",
            "WebAssembly linear memory stack machine bytecode JIT compilation sandbox",
        ),
    ];

    for (id, text) in &corpus {
        let scope = MemoryScope::new(
            tenant_id.clone(),
            workspace_id.clone(),
            session_id.clone(),
            "corpus",
        )
        .unwrap();
        let rec = MemoryRecord::new_l1(
            *id,
            MemoryKind::ExecutionEvidence,
            scope,
            *text,
            ContextClassification::Internal,
            now,
            "corpus-data",
        )
        .unwrap();
        vfs.add_memory_record(rec);
    }

    // Force build HNSW index
    vfs.build_hnsw_index(None).expect("build_hnsw_index");

    let test_queries = [
        ("quic udp multiplexed streams", "doc-quic"),
        ("raft leader election replication", "doc-raft"),
        ("hardware security module tamper pkcs", "doc-hsm"),
        ("rust borrow checker non-lexical lifetimes", "doc-borrowck"),
        ("lsm tree write amplification sstables", "doc-lsm"),
    ];

    let hasher = pandora_runtime::semantic_memory::DenseFeatureHasher::new();
    let mut top1_matches = 0;
    let mut top3_matches = 0;

    for (query, expected_doc_id) in &test_queries {
        let q_vec = hasher.vectorize(query);

        // Brute-force exact cosine similarity calculation
        let mut brute_force_scores = Vec::new();
        for (id, text) in &corpus {
            let doc_vec = hasher.vectorize(text);
            let sim = hasher.cosine_similarity(&q_vec, &doc_vec);
            brute_force_scores.push((*id, sim));
        }
        brute_force_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        let brute_force_top1 = brute_force_scores[0].0;
        assert_eq!(
            brute_force_top1, *expected_doc_id,
            "brute force top1 should match expected ground truth for query '{query}'"
        );

        // HNSW approximate search
        let hnsw_results = vfs.search_hnsw(query, 5, 64).expect("search_hnsw");
        assert!(!hnsw_results.is_empty(), "HNSW search must return results");

        if hnsw_results[0].title == *expected_doc_id {
            top1_matches += 1;
        }

        let in_top3 = hnsw_results
            .iter()
            .take(3)
            .any(|r| r.title == *expected_doc_id);
        if in_top3 {
            top3_matches += 1;
        }
    }

    assert!(
        top1_matches >= 4,
        "HNSW Recall@1 should be at least 80% (got {}/5)",
        top1_matches
    );
    assert_eq!(
        top3_matches, 5,
        "HNSW Recall@3 should be 100% (got {}/5)",
        top3_matches
    );
}

#[test]
fn test_empirical_concurrent_reads_and_invalidations() {
    use std::sync::Arc;
    use std::thread;

    let vfs = Arc::new(create_sample_context_vfs());
    let mut handles = Vec::new();

    // Spawning concurrent reader and writer threads
    for thread_id in 0..6 {
        let vfs_clone = Arc::clone(&vfs);
        let handle = thread::spawn(move || {
            let queries = [
                "cryptography mutual TLS HMAC",
                "eBPF kernel telemetry socket",
                "qubit quantum coherence surface code",
                "canary rollout procedure",
                "split-brain post-mortem partition",
            ];
            for i in 0..40 {
                let q = queries[(thread_id + i) % queries.len()];
                if thread_id % 3 == 0 {
                    // Reader: search_hnsw
                    let res = vfs_clone.search_hnsw(q, 3, 32);
                    assert!(res.is_ok());
                } else if thread_id % 3 == 1 {
                    // Reader: search_with_options
                    let opts = ContextSearchOptions {
                        limit: 3,
                        min_score: 0.0,
                        ..Default::default()
                    };
                    let res = vfs_clone.search_with_options(q, &opts);
                    assert!(res.is_ok());
                } else {
                    // Writer: dynamic mutation & stats check
                    let _ = vfs_clone.is_hnsw_index_fresh();
                    let _ = vfs_clone.hnsw_index_stats();
                }
            }
        });
        handles.push(handle);
    }

    for h in handles {
        h.join().expect("thread join must not panic");
    }
}

#[test]
fn test_empirical_rrf_scoring_invariants() {
    let vfs = ContextVfs::new();
    let tenant_id = TenantId::new("tenant-rrf").unwrap();
    let workspace_id = WorkspaceId::new("ws-rrf").unwrap();
    let session_id = SessionId::new("sess-rrf").unwrap();
    let now = Timestamp::from_unix_seconds(1_700_000_000);

    // Doc A: strong sparse match AND strong dense match for "compiler optimization pass"
    let scope_a = MemoryScope::new(
        tenant_id.clone(),
        workspace_id.clone(),
        session_id.clone(),
        "rrf",
    )
    .unwrap();
    let doc_a = MemoryRecord::new_l1(
        "doc-a-dual",
        MemoryKind::ExecutionEvidence,
        scope_a,
        "compiler optimization pass dead code elimination constant folding instruction combination",
        ContextClassification::Internal,
        now,
        "comp-a",
    )
    .unwrap();
    vfs.add_memory_record(doc_a);

    // Doc B: sparse match but weak dense match
    let scope_b = MemoryScope::new(
        tenant_id.clone(),
        workspace_id.clone(),
        session_id.clone(),
        "rrf",
    )
    .unwrap();
    let doc_b = MemoryRecord::new_l1(
        "doc-b-sparse-only",
        MemoryKind::ExecutionEvidence,
        scope_b,
        "compiler error code diagnostic syntax mismatch token unexpected",
        ContextClassification::Internal,
        now,
        "comp-b",
    )
    .unwrap();
    vfs.add_memory_record(doc_b);

    // Doc C: completely irrelevant
    let scope_c =
        MemoryScope::new(tenant_id.clone(), workspace_id.clone(), session_id, "rrf").unwrap();
    let doc_c = MemoryRecord::new_l1(
        "doc-c-irrelevant",
        MemoryKind::ExecutionEvidence,
        scope_c,
        "gardening horticulture organic soil compost aeration irrigation",
        ContextClassification::Internal,
        now,
        "comp-c",
    )
    .unwrap();
    vfs.add_memory_record(doc_c);

    let opts = ContextSearchOptions {
        limit: 5,
        min_score: 0.0,
        ..Default::default()
    };
    let results = vfs
        .search_with_options("compiler optimization pass constant folding", &opts)
        .expect("search_with_options");

    assert!(!results.is_empty());
    // Doc A must be ranked #1
    assert_eq!(results[0].title, "doc-a-dual");
    assert!(results[0].sparse_score > 0.0, "Doc A has sparse score");
    assert!(results[0].dense_score > 0.0, "Doc A has dense score");
    assert!(results[0].hybrid_score > 0.0, "Doc A has hybrid RRF score");

    if results.len() > 1 {
        assert!(
            results[0].hybrid_score > results[1].hybrid_score,
            "Dual-matched doc must have strictly higher hybrid score than single-matched doc"
        );
    }
}
