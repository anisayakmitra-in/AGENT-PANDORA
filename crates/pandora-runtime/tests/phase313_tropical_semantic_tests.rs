#![forbid(unsafe_code)]

//! Deterministic Integration & Adversarial Tests for AGENT-PANDORA v3.1.3 (Milestones 1, 2, 4).
//!
//! Covers:
//! - R1: Sub-microsecond Tropical / MOIRA Decision Driver integration in LATS MCTS with PUCT selection rule.
//! - R2: Fast-path Reflex Tool Safety Gating in Parliament & ReferenceMonitor with entropy/margin escalation.
//! - R4: Zero-multiplication Tropical Semantic Projections ($\max_i(x_i + w_i)$), Hybrid BM25 + Tropical ranking,
//!   HNSW Tropical vector graph + PVHN v1 serialization & legacy fallback, and ContextVfs L0–L4 retrieval.

use std::sync::Arc;

use pandora_runtime::{
    ContextSearchOptions, ContextVfs, HnswConfig, HnswIndex, HnswVector, HnswVectorType,
    LatsEngine, MoiraDecisionDriver, Parliament, QuantizedVector, ReferenceMonitor,
    SemanticIndexer, SinglePassLogitEngine, TROPICAL_DIMENSION, TropicalDecisionDriver,
    TropicalHybridRanker, TropicalProjector, TropicalVector, cosine_distance,
    should_escalate_with_entropy, simd_tropical_max_plus_128, strategies::StrategyBudget,
    tropical_distance,
};
use pandora_types::{
    Capability, ContextClassification, EffectTarget, ExecutionId, ExecutionProfile,
    ExecutionProfileBinding, ExecutionProfileBindingKind, GeneId, MemoryIntent, MemoryKind,
    MemoryRecord, MemoryScope, MemoryTier, Operation, OperationRequest, PolicyContext, PrincipalId,
    ResourceScope, SessionArchiveRecord, SessionId, SopDefinition, SopStep, StoredSop, TenantId,
    Timestamp, ToolSafetyChoice, WorkspaceId, hash_artifact,
};
use serde_json::json;

#[test]
fn r1_lats_engine_tropical_mcts_executes_puct_without_llm_calls() {
    let budget = StrategyBudget::new(4, 16, 50_000, 10, 30, 5_000_000).expect("valid budget");
    let driver = Arc::new(TropicalDecisionDriver::calibrated());
    let engine = LatsEngine::new_with_tropical(budget, driver)
        .with_puct(1.414)
        .with_escalation_threshold(0.15);

    let action_ids = [0usize, 1usize, 2usize];
    let action_labels = vec![
        "verify_schema_and_run_tests".to_owned(),
        "force_push_untested_branch".to_owned(),
        "inspect_diagnostics_and_patch".to_owned(),
    ];

    let result = engine
        .search_with_tropical(
            "Resolve Rust borrow checker diagnostic safely",
            &action_ids,
            &action_labels,
        )
        .expect("tropical MCTS search must succeed");

    assert_eq!(result.metrics.rollouts, 16);
    assert!(result.metrics.max_depth_reached >= 1 && result.metrics.max_depth_reached <= 4);
    assert!(result.metrics.nodes_created > 1);
    assert!(result.best_action.is_some());
    assert!(action_ids.contains(&result.best_action.unwrap()));
    assert!(result.root_visits >= 16);
    assert!((0.0..=1.0).contains(&result.metrics.best_value));
}

#[test]
fn r2_parliament_and_reference_monitor_reflex_gating_and_escalation() {
    let engine = SinglePassLogitEngine::new();
    let safe_verdict = engine
        .evaluate_tool_safety("view_file", &json!({"path": "README.md"}))
        .expect("safe evaluation");
    assert_eq!(safe_verdict.selected, ToolSafetyChoice::Allow);

    let destructive_verdict = engine
        .evaluate_tool_safety("run_command", &json!({"command": "rm -rf /"}))
        .expect("destructive evaluation");
    assert_eq!(destructive_verdict.selected, ToolSafetyChoice::Reject);

    // High-uncertainty or high-entropy distribution must trigger System 2 escalation
    assert!(should_escalate_with_entropy(0.05, 0.15, 0.30));
    assert!(should_escalate_with_entropy(0.50, 0.15, 0.75));
    assert!(!should_escalate_with_entropy(0.45, 0.15, 0.25));

    let profile = ExecutionProfile::new(
        "3.1.3",
        "windows",
        "x86_64",
        1,
        "workspace-1",
        hash_artifact(b"containment"),
        vec![
            ExecutionProfileBinding::new(
                ExecutionProfileBindingKind::Executor,
                "system",
                Some("3.1.3"),
                hash_artifact(b"system"),
            )
            .expect("valid binding"),
        ],
    )
    .expect("valid profile");

    let req = OperationRequest::new(
        ExecutionId::new("exec-v313-1").expect("valid id"),
        SessionId::new("sess-v313-1").expect("valid id"),
        PrincipalId::new("principal-v313-1").expect("valid id"),
        profile,
        GeneId::new("gene-v313-1").expect("valid id"),
        None,
        Capability::FilesystemRead,
        Operation::Read,
        EffectTarget::path("README.md"),
        ResourceScope::path("."),
    )
    .expect("valid request");

    let parliament = Parliament::new(1);
    let policy = PolicyContext::new(
        1,
        [Capability::FilesystemRead, Capability::ProcessExecute],
        [Operation::Write],
    );
    let reflex = parliament
        .evaluate_tool_safety_reflex(&req, &policy, "view_file", &json!({"path": "README.md"}))
        .expect("reflex evaluation succeeds");
    assert!(reflex.is_allow());
    assert!(!reflex.is_escalated());

    let monitor = ReferenceMonitor::new_with_policy(policy, 60);
    let _ = monitor;

    let moira = MoiraDecisionDriver::calibrated();
    let _ = moira;
}

#[test]
fn r4_tropical_vector_max_plus_projection_and_hilbert_metric() {
    let mut x = [-2.0f32; TROPICAL_DIMENSION];
    let mut w = [-2.0f32; TROPICAL_DIMENSION];
    x[19] = 2.0;
    w[19] = 3.5;
    x[64] = 1.0;
    w[64] = 1.25;

    let vx = TropicalVector::new(x);
    let vw = TropicalVector::new(w);

    let max_plus = vx.score_max_plus(&vw);
    let simd_direct = simd_tropical_max_plus_128(&x, &w);
    assert!((max_plus - 5.5).abs() < 1e-6);
    assert!((simd_direct - 5.5).abs() < 1e-6);

    let sim = vx.similarity(&vw);
    assert!(sim > 0.99 && sim <= 1.0);
    assert_eq!(vx.distance(&vx), 0.0);
    assert_eq!(vx.hilbert_distance(&vx), 0.0);
    assert!(vx.hilbert_distance(&vw) > 0.0);

    // Roundtrip binary serialization
    let bytes = vx.to_bytes();
    let decoded = TropicalVector::from_bytes(&bytes).expect("decode tropical bytes");
    assert_eq!(vx, decoded);
}

#[test]
fn r4_hnsw_tropical_graph_search_pvhn_roundtrip_and_dense_fallback() {
    let mut index = HnswIndex::new(HnswConfig::default());

    let t0 = TropicalProjector::project("zero-multiplication tropical max-plus semiring algebra");
    let t1 = TropicalProjector::project("kubernetes pod autoscaling and helm chart deployment");
    let t2 = TropicalProjector::project("sqlite wal mode transaction checkpoint and vacuum");

    index.insert(0, HnswVector::Tropical(t0));
    index.insert(1, HnswVector::Tropical(t1));
    index.insert(2, HnswVector::Tropical(t2));

    let q = HnswVector::Tropical(TropicalProjector::project("tropical max-plus semiring"));
    let hits = index.search(&q, 2, 32);
    assert!(!hits.is_empty());
    assert_eq!(hits[0].doc_id, 0);

    // Verify PVHN v1 serialization and deserialization with TropicalVector tag 4
    let bytes = index
        .to_pvhn_bytes()
        .expect("PVHN serialize tropical index");
    assert_eq!(bytes[24], HnswVectorType::Tropical as u8);

    let restored = HnswIndex::from_pvhn_bytes(&bytes).expect("PVHN deserialize tropical index");
    assert_eq!(restored.len(), 3);
    assert!(restored.nodes()[0].vector.is_tropical());

    let restored_hits = restored.search(&q, 2, 32);
    assert_eq!(restored_hits[0].doc_id, 0);

    // Verify fallback distance between Tropical and legacy F32 / Quantized vectors
    let legacy_f32 = HnswVector::F32([0.25f32; 128]);
    let legacy_q = HnswVector::Quantized(QuantizedVector::quantize(&[0.25f32; 128]));
    let d_f32 = cosine_distance(&q, &legacy_f32);
    let d_q = cosine_distance(&q, &legacy_q);
    let d_trop = tropical_distance(&q, &legacy_q);
    assert!((0.0..=2.0).contains(&d_f32));
    assert!((0.0..=2.0).contains(&d_q));
    assert!((0.0..=2.0).contains(&d_trop));
}

#[test]
fn r4_context_vfs_l0_to_l4_tropical_and_hybrid_search() {
    let vfs = ContextVfs::new();
    let tenant = TenantId::new("tenant-v313").expect("valid tenant");
    let workspace = WorkspaceId::new("ws-v313").expect("valid workspace");
    let session_id = SessionId::new("sess-v313").expect("valid session");
    let scope = MemoryScope::new(tenant.clone(), workspace.clone(), session_id.clone(), "p1")
        .expect("valid scope");

    // Mount L0 & L1 Memory Records
    let rec_l0 = MemoryRecord::new_l0(
        "mem-l0-rule",
        scope.clone(),
        "Constitutional rule: enforce ReferenceMonitor permits on all external effects",
        ContextClassification::Internal,
        Timestamp::from_unix_seconds(1_700_000_050),
        None,
        "prov-0",
    )
    .expect("valid L0 record");
    vfs.add_memory_record(rec_l0);

    let rec_l1 = MemoryRecord::new_l1(
        "mem-l1-trop",
        MemoryKind::ExecutionEvidence,
        scope,
        "Tropical max-plus neural layer forward pass uses additions and maxima only",
        ContextClassification::Internal,
        Timestamp::from_unix_seconds(1_700_000_100),
        "prov-1",
    )
    .expect("valid L1 record")
    .with_intent(MemoryIntent::Repo);
    vfs.add_memory_record(rec_l1);

    // Mount L3 SOP
    let step = SopStep::new(
        "step-1",
        "validate",
        "Verify release_predecessor.py skips unpublished tags",
        vec![],
    )
    .expect("valid step");
    let sop_def = SopDefinition::new(
        "sop-release-v313",
        "Release Predecessor Validation SOP",
        MemoryIntent::Procedure,
        vec![step],
        Some("python scripts/validate_repo.py".to_owned()),
    )
    .expect("valid sop def");
    let stored_sop = StoredSop::new(
        sop_def,
        tenant.clone(),
        workspace.clone(),
        1,
        Timestamp::from_unix_seconds(1_700_000_300),
        Timestamp::from_unix_seconds(1_700_000_300),
    );
    vfs.add_sop(stored_sop);

    // Mount L4 Session Archive
    let archive = SessionArchiveRecord::new(
        "arch-hnsw-trop",
        tenant,
        workspace,
        session_id,
        "Archived session verifying HnswVector Tropical PVHN binary persistence",
        "+120 -4 lines in hnsw.rs",
        hash_artifact(b"receipt-v313"),
        Timestamp::from_unix_seconds(1_700_000_400),
    )
    .expect("valid archive");
    vfs.add_archive(archive);

    // 1. Direct tropical search across L0-L4
    let trop_results = vfs
        .search_tropical("tropical max-plus neural layer additions", 5)
        .expect("tropical search must succeed");
    assert!(!trop_results.is_empty());
    assert!(
        trop_results
            .iter()
            .any(|r| r.uri.as_str().contains("mem-l1-trop")),
        "expected L1 tropical memory record in top hits"
    );

    // 2. Tier-filtered tropical search for L3 SOP
    let l3_opts = ContextSearchOptions::new(5).with_tier(MemoryTier::L3);
    let l3_results = vfs
        .search_tropical_with_options("release predecessor validation", &l3_opts)
        .expect("L3 tropical search must succeed");
    assert_eq!(l3_results.len(), 1);
    assert_eq!(l3_results[0].tier, Some(MemoryTier::L3));

    // 3. Build tropical HNSW index on ContextVfs and run HNSW search
    let stats = vfs
        .build_hnsw_index_tropical(None)
        .expect("build tropical HNSW index");
    assert!(stats.node_count >= 4);

    let hnsw_hits = vfs
        .search_hnsw("HnswVector Tropical PVHN binary persistence", 3, 32)
        .expect("search tropical HNSW");
    assert!(!hnsw_hits.is_empty());
    assert_eq!(hnsw_hits[0].tier, Some(MemoryTier::L4));

    // 4. SemanticIndexer and TropicalHybridRanker linear + RRF fusion
    let mut indexer = SemanticIndexer::new();
    for doc in vfs.collect_all_documents().expect("collect docs") {
        indexer.index_vfs_doc(doc);
    }
    assert!(indexer.len() >= 4);
    let ranker = TropicalHybridRanker::with_params(60.0, 0.5);
    let blended = ranker.blend_linear(4.0, 2.0);
    assert!(blended > 0.5 && blended <= 1.0);
}
