#![forbid(unsafe_code)]

//! Deterministic verification suite for Phases 9–12 of the AGENT-PANDORA architectural roadmap:
//! - Phase 9: Autonomous in-loop `.pnn` hot-reload & subgradient self-calibration (`with_auto_calibrate`).
//! - Phase 10: Quantized 7D feature memoization (`LatsPrefixMemoCache`) for multi-turn LATS.
//! - Phase 11: Hierarchical `PVFS` v3 tier-weighted tropical HNSW routing (`L0 > L1 > L2 > L3 > L4`).

use pandora_harnesses::HarnessCatalog;
use pandora_runtime::agent_loop::{AgentLoop, LatsLoopConfig};
use pandora_runtime::context_vfs::ContextVfs;
use pandora_runtime::decision_engine::{DecisionFeatures, TropicalDecisionDriver};
use pandora_runtime::execution_controller::ExecutionController;
use pandora_runtime::executors::WorkspaceRoot;
use pandora_runtime::lats_engine::{LatsMemoEntry, LatsMemoKey, LatsPrefixMemoCache};
use pandora_runtime::strategies::StrategyBudget;
use pandora_types::{
    ContextClassification, ContextFragment, ContextOrigin, ContextSource, ContextTrust, MemoryKind,
    MemoryRecord, MemoryScope, MemoryTier, PolicyContext, PrincipalId, Session, SessionId,
    TenantId, Timestamp, WorkspaceId,
};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

struct MultiTurnEscalationProvider {
    manifest: pandora_provider::ProviderManifest,
    turn: Mutex<u32>,
    total_tool_turns: u32,
    note: &'static str,
}

impl pandora_provider::Provider for MultiTurnEscalationProvider {
    fn manifest(&self) -> &pandora_provider::ProviderManifest {
        &self.manifest
    }

    fn complete(
        &self,
        _request: pandora_provider::ModelRequest,
    ) -> Result<pandora_provider::ModelResponse, pandora_provider::ProviderError> {
        let mut t = self.turn.lock().unwrap();
        *t += 1;
        if *t <= self.total_tool_turns {
            let call = pandora_provider::ToolCall::new(
                format!("call-p9-10-{}", *t),
                "workspace.read",
                serde_json::json!({
                    "path": "README.md",
                    "note": self.note
                }),
            )?;
            Ok(pandora_provider::ModelResponse::new(
                "",
                vec![call],
                pandora_provider::TokenUsage::new(10, 5),
            ))
        } else {
            Ok(pandora_provider::ModelResponse::new(
                "Completed multi-turn inspection.",
                vec![],
                pandora_provider::TokenUsage::new(12, 6),
            ))
        }
    }
}

#[test]
fn phase9_agent_loop_hot_reloads_pnn_mid_session_with_canary_gate() {
    let temp_dir = std::env::temp_dir().join(format!(
        "pandora_phase9_hot_reload_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    std::fs::write(temp_dir.join("README.md"), "phase 9 hot reload test\n").unwrap();

    let budget = StrategyBudget::new(4, 3, 1000, 8, 10, 1000).unwrap();
    let mut base_net = TropicalDecisionDriver::calibrated_network();
    for row in 0..4 {
        base_net.layers[0].weights[row * 16 + 10] = 2.1;
    }
    let base_net = pandora_runtime::tropical_nn::TropicalNetwork::new(
        base_net.name,
        base_net.domain,
        base_net.layers,
    )
    .unwrap();
    let initial_driver = Arc::new(TropicalDecisionDriver::new(Some(base_net)));
    let initial_digest = initial_driver.network().unwrap().sha256_digest.clone();

    let lats_cfg = LatsLoopConfig::new(budget, 1.4, 0.95, initial_driver)
        .with_auto_calibrate(1)
        .with_memo(false);
    let loop_engine = AgentLoop::new(6, 6).unwrap().with_lats(lats_cfg);

    let ws_root = WorkspaceRoot::new(&temp_dir).unwrap();
    let policy = PolicyContext::read_only_workspace();
    let controller =
        ExecutionController::with_policy_and_harnesses(ws_root, policy, HarnessCatalog::builtins());

    let manifest = pandora_provider::ProviderManifest::new(
        "mock-phase9",
        "Mock Phase 9 Provider",
        "https://api.example.com/v1",
        "mock-model",
        "PANDORA_MOCK_KEY",
    )
    .unwrap();
    let provider = MultiTurnEscalationProvider {
        manifest,
        turn: Mutex::new(0),
        total_tool_turns: 2,
        note: "override read inspection",
    };
    let now = Timestamp::from_unix_seconds(1_700_000_000);
    let session = Session::new(
        SessionId::new("sess-phase9-hot-reload").unwrap(),
        PrincipalId::new("user-phase9").unwrap(),
        TenantId::new("tenant-phase9").unwrap(),
        WorkspaceId::new("ws-phase9").unwrap(),
        now,
    );

    let summary = loop_engine
        .run(&provider, &controller, session, "inspect README.md", now)
        .unwrap();

    assert_eq!(summary.dpo_pairs_emitted(), 2);
    assert!(
        summary.pnn_hot_reloads() >= 1,
        "expected at least 1 mid-session .pnn hot-reload, got {}",
        summary.pnn_hot_reloads()
    );
    let final_digest = summary
        .active_pnn_digest()
        .expect("active_pnn_digest must be populated");
    assert_ne!(
        final_digest, initial_digest,
        "hot-reloaded .pnn digest must differ from initial baseline digest"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn phase10_lats_prefix_memo_cache_quantization_eviction_and_agent_loop_hits() {
    // 1. Unit invariants: 4-bit quantization + bounded FIFO eviction
    let mut cache = LatsPrefixMemoCache::new(2);
    let f1 = DecisionFeatures::new([0.20, 0.15, 0.0, 0.125, 0.35, 0.20, 0.85]);
    let f1_perturbed = DecisionFeatures::new([0.21, 0.15, 0.0, 0.130, 0.35, 0.20, 0.85]);
    let k1 = LatsMemoKey::from_features("workspace.read", &f1, "digest-a");
    let k1_p = LatsMemoKey::from_features("workspace.read", &f1_perturbed, "digest-a");
    assert_eq!(
        k1, k1_p,
        "within-bucket perturbation must hit same 4-bit key"
    );

    let k2 = LatsMemoKey::from_features("workspace.list", &f1, "digest-a");
    let k3 = LatsMemoKey::from_features("workspace.search", &f1, "digest-a");
    let entry = LatsMemoEntry {
        best_action: Some(0),
        best_value: 0.91,
        nodes_created: 7,
        max_depth_reached: 2,
    };
    cache.insert(k1.clone(), entry.clone());
    cache.insert(k2.clone(), entry.clone());
    assert!(cache.get(&k1).is_some());
    cache.insert(k3.clone(), entry);
    assert_eq!(
        cache.len(),
        2,
        "cache capacity must be strictly bounded at 2"
    );
    assert!(cache.get(&k1).is_none(), "oldest key must be evicted");

    // 2. Multi-turn AgentLoop memoization hit on recurring tool-call pattern
    let temp_dir = std::env::temp_dir().join(format!(
        "pandora_phase10_memo_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    std::fs::write(temp_dir.join("README.md"), "phase 10 memo test\n").unwrap();

    let budget = StrategyBudget::new(4, 3, 1000, 8, 10, 1000).unwrap();
    let driver = Arc::new(TropicalDecisionDriver::calibrated());
    let lats_cfg = LatsLoopConfig::new(budget, 1.4, 0.95, driver).with_memo(true);
    let loop_engine = AgentLoop::new(24, 24).unwrap().with_lats(lats_cfg);

    let ws_root = WorkspaceRoot::new(&temp_dir).unwrap();
    let policy = PolicyContext::read_only_workspace();
    let controller =
        ExecutionController::with_policy_and_harnesses(ws_root, policy, HarnessCatalog::builtins());

    let manifest = pandora_provider::ProviderManifest::new(
        "mock-phase10",
        "Mock Phase 10 Provider",
        "https://api.example.com/v1",
        "mock-model",
        "PANDORA_MOCK_KEY",
    )
    .unwrap();
    let provider = MultiTurnEscalationProvider {
        manifest,
        turn: Mutex::new(0),
        total_tool_turns: 3,
        note: "override safe read inspection",
    };
    let now = Timestamp::from_unix_seconds(1_700_000_000);
    let session = Session::new(
        SessionId::new("sess-phase10-memo").unwrap(),
        PrincipalId::new("user-phase10").unwrap(),
        TenantId::new("tenant-phase10").unwrap(),
        WorkspaceId::new("ws-phase10").unwrap(),
        now,
    );

    let summary = loop_engine
        .run(&provider, &controller, session, "inspect README.md", now)
        .unwrap();

    assert!(
        summary.lats_memo_hits() >= 1,
        "expected >= 1 LATS memo hit across recurring turns, got {}",
        summary.lats_memo_hits()
    );
    assert!(
        summary
            .lats_telemetry()
            .iter()
            .any(|t| t.memo_hit && t.rollouts == 0),
        "expected memoized turn with rollouts == 0"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn phase11_tropical_hnsw_tier_weighted_routing_breaks_ties_l0_over_l1() {
    let vfs = ContextVfs::new();
    let now = Timestamp::from_unix_seconds(1_700_000_000);
    let tenant = TenantId::new("t-phase11").unwrap();
    let ws = WorkspaceId::new("ws-phase11").unwrap();
    let sess = SessionId::new("s-phase11").unwrap();
    let scope = MemoryScope::new(tenant, ws, sess, "local").unwrap();

    let identical_content =
        "Zero-trust capability token verification invariant for tropical policy routing";

    // Add L1 record first
    let l1 = MemoryRecord::new_l1(
        "mem-l1-tie",
        MemoryKind::Lesson,
        scope,
        identical_content,
        ContextClassification::Internal,
        now,
        "l1-source",
    )
    .unwrap();
    vfs.add_memory_record(l1);

    // Add L0 constitutional rule with identical content
    let l0 = ContextFragment::new_with_origin(
        "meta:l0-tie",
        ContextSource::Constitutional,
        ContextTrust::Constitutional,
        ContextClassification::Internal,
        250,
        identical_content,
        64,
        None,
        ContextOrigin::new("pandora-runtime", "phase11").unwrap(),
    )
    .unwrap();
    vfs.add_meta_rule(l0);

    let temp_dir = std::env::temp_dir().join(format!(
        "pandora_phase11_tier_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    vfs.set_hnsw_index_path(temp_dir.join("tier_hnsw.idx"));

    let hits = vfs
        .search_hnsw("capability token verification tropical policy", 2, 32)
        .unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(
        hits[0].tier,
        Some(MemoryTier::L0),
        "tier-weighted tropical HNSW must rank L0 above L1 on equal semantic match"
    );
    assert_eq!(hits[1].tier, Some(MemoryTier::L1));
    assert!(
        hits[0].hybrid_score > hits[1].hybrid_score,
        "L0 tier-weighted hybrid_score ({}) must exceed L1 ({})",
        hits[0].hybrid_score,
        hits[1].hybrid_score
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}
