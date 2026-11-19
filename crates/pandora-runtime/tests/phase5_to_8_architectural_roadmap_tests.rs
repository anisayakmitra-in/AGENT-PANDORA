#![forbid(unsafe_code)]

//! Deterministic verification suite for Phases 5–8 of the AGENT-PANDORA architectural roadmap:
//! - Phase 5: Online System-2 -> System-1 DPO distillation inside `AgentLoop` (with `.pandora/dpo_pairs.jsonl` persistence).
//! - Phase 6: Direct `PVFS` v3 -> Tropical HNSW reuse in `save_hnsw_index` & projective Hilbert tie-breaking.
//! - Phase 7: Multi-step 7D `DecisionFeatures` transition rollouts ($T: [0,1]^7 \times \{0,1,2,3\} \to [0,1]^7$) in `LatsEngine` via calibrated `.pnn`.
//! - Phase 8: Hash-chained `.pnn` calibration attestation receipts ($C_{\text{cal}} = \operatorname{SHA256}(H(W_{\text{base}}) \,\|\, H(W_{\text{cand}}) \,\|\, H(\mathcal{D}_{\text{DPO}}) \,\|\, \operatorname{le64}(q_{0.05}))$).

use pandora_runtime::context_vfs::ContextVfs;
use pandora_runtime::decision_engine::{
    CalibrationAttestationReceipt, DecisionFeatures, TropicalDecisionDriver,
    compute_calibration_commitment, compute_dpo_dataset_digest,
};
use pandora_runtime::hnsw::HnswIndex;
use pandora_runtime::lats_engine::LatsEngine;
use pandora_runtime::semantic_indexer::{TropicalProjector, TropicalVector};
use pandora_runtime::strategies::StrategyBudget;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn phase6_save_hnsw_index_with_empty_cache_builds_tropical_hnsw() {
    let vfs = ContextVfs::new();
    let temp_dir = std::env::temp_dir().join(format!(
        "pandora_phase6_save_hnsw_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let hnsw_path = temp_dir.join("direct_save_tropical.hnsw");

    // Calling save_hnsw_index directly with an empty cache must build a Tropical HNSW index
    let saved_path = vfs.save_hnsw_index(Some(&hnsw_path)).unwrap();
    assert_eq!(saved_path, hnsw_path);
    assert!(hnsw_path.is_file());

    let bytes = std::fs::read(&hnsw_path).unwrap();
    let loaded = HnswIndex::from_pvhn_bytes(&bytes).unwrap();
    assert!(!loaded.is_empty());
    assert!(
        loaded.nodes().iter().all(|n| n.vector.is_tropical()),
        "save_hnsw_index with empty cache must build Tropical HNSW vectors, not Quantized"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn phase6_hilbert_projective_dispersion_breaks_specificity_ties_and_preserves_bounds() {
    let query = TropicalProjector::project("quantum surface code coherence");
    let exact_doc = TropicalProjector::project("quantum surface code coherence");
    let diluted_doc = TropicalProjector::project(
        "quantum surface code coherence with unrelated kubernetes ingress controller deployment yaml pipeline",
    );

    let d_exact = query.active_hilbert_distance(&exact_doc);
    let d_diluted = query.active_hilbert_distance(&diluted_doc);
    assert_eq!(
        d_exact, 0.0,
        "identical active support must have 0 Hilbert distance"
    );
    assert!(
        d_diluted > d_exact,
        "diluted document with extra unmatched active tokens must have strictly larger active Hilbert distance ({d_diluted} vs {d_exact})"
    );

    let sim_exact = query.similarity(&exact_doc);
    let sim_diluted = query.similarity(&diluted_doc);
    assert!(
        sim_exact > sim_diluted,
        "exact document similarity ({sim_exact}) must strictly exceed diluted document similarity ({sim_diluted})"
    );
    assert!(sim_exact <= 1.0 && sim_diluted >= 0.0);

    // Verify existing similarity bounds on synthetic vectors
    let mut a = [-1.0f32; 128];
    let mut b = [-1.0f32; 128];
    a[7] = 1.5;
    b[7] = 2.25;
    a[42] = 0.5;
    b[42] = 1.0;
    let va = TropicalVector::new(a);
    let vb = TropicalVector::new(b);
    let sim = va.similarity(&vb);
    assert!(
        sim > 0.95 && sim <= 1.0,
        "expected sim in (0.95, 1.0], got {sim}"
    );
}

#[test]
fn phase7_decision_features_7d_transition_invariants_and_multistep_recovery_rollout() {
    // 1. Verify T: [0,1]^7 x {0,1,2,3} -> [0,1]^7 bounds and action semantics
    let high_fail = DecisionFeatures::new([0.20, 0.55, 0.30, 0.10, 0.20, 0.20, 0.50]);
    assert!(high_fail.is_valid());

    for action in 0..=4 {
        let next = high_fail.transition(action);
        assert!(
            next.is_valid(),
            "transition for action {action} must stay in [0,1]^7: {:?}",
            next
        );
    }

    // Premature Proceed (a=0) when failure > 0.35 increases failure; Repair (a=1) contracts failure
    let after_proceed = high_fail.transition(0);
    let after_repair = high_fail.transition(1);
    assert!(
        after_proceed.failure > high_fail.failure,
        "myopic Proceed in unrepaired state must increase failure"
    );
    assert!(
        after_repair.failure < high_fail.failure && after_repair.failure <= 0.35,
        "Repair must contract failure into the safe regime (<= 0.35), got {}",
        after_repair.failure
    );

    // 2. Verify two-step trajectory Repair (1) -> Proceed (0) strictly beats Proceed (0) -> Proceed (0)
    let driver = std::sync::Arc::new(TropicalDecisionDriver::calibrated());
    let labels = ["allow", "require_confirm", "reject"];
    let prompt = "workspace.patch src/lib.rs after test failure";

    let v_greedy_fail =
        driver.evaluate_multistep_return(prompt, &high_fail, &[0, 0], &labels, 0.90);
    let v_repair_then_proceed =
        driver.evaluate_multistep_return(prompt, &high_fail, &[1, 0], &labels, 0.90);
    assert!(
        v_repair_then_proceed > v_greedy_fail + 0.25,
        "2-step Repair->Proceed return ({v_repair_then_proceed}) must decisively exceed Proceed->Proceed ({v_greedy_fail})"
    );

    // 3. Verify depth D >= 2 LatsEngine::search_with_feature_rollouts selects Repair (Some(1))
    let budget = StrategyBudget::new(8, 3, 1000, 8, 10, 1000).unwrap();
    let engine = LatsEngine::new_with_tropical(budget, driver);
    let actions = [0usize, 1, 2];
    let search_res = engine
        .search_with_feature_rollouts(prompt, high_fail, &actions, &labels, 0.90)
        .unwrap();
    assert_eq!(
        search_res.best_action,
        Some(1),
        "depth D >= 2 MCTS rollout on recoverable failure state must select Repair (1)"
    );
    assert!(search_res.metrics.max_depth_reached >= 2);
}

#[test]
fn phase8_calibration_attestation_receipt_commitment_and_tamper_detection() {
    let pairs = vec![
        TropicalDecisionDriver::emit_dpo_pair_from_escalation(
            "dpo-1",
            "run_command rm -rf /",
            "reject",
            "allow",
            0.05,
        ),
        TropicalDecisionDriver::emit_dpo_pair_from_escalation(
            "dpo-2",
            "view_file Cargo.toml",
            "allow",
            "require_confirm",
            0.09,
        ),
    ];
    let dpo_digest = compute_dpo_dataset_digest(&pairs);
    let base_net = TropicalDecisionDriver::calibrated_network();
    let cand_net =
        TropicalDecisionDriver::calibrate_network_from_pairs(&base_net, &pairs, 0.05).unwrap();
    let q05 = 0.794_183_375_585_332_7_f64;

    let expected_c = compute_calibration_commitment(
        &base_net.sha256_digest,
        &cand_net.sha256_digest,
        &dpo_digest,
        q05,
    );

    let r0 = CalibrationAttestationReceipt::new(
        0,
        CalibrationAttestationReceipt::GENESIS_PREV_HASH,
        base_net.sha256_digest.clone(),
        cand_net.sha256_digest.clone(),
        dpo_digest.clone(),
        q05,
        true,
    );
    assert_eq!(r0.commitment, expected_c);
    assert!(r0.verify());

    let r1 = CalibrationAttestationReceipt::new(
        1,
        r0.chain_hash.clone(),
        cand_net.sha256_digest.clone(),
        cand_net.sha256_digest.clone(),
        dpo_digest.clone(),
        q05,
        true,
    );
    assert!(r1.verify());
    assert!(CalibrationAttestationReceipt::verify_chain(&[
        r0.clone(),
        r1.clone()
    ]));

    // Tampering with baseline_digest, candidate_digest, dpo_dataset_digest, lower_bound_95, or chain link must fail
    let mut bad_base = r0.clone();
    bad_base.baseline_digest = "f".repeat(64);
    assert!(!bad_base.verify());

    let mut bad_q = r0.clone();
    bad_q.lower_bound_95 = 0.99;
    assert!(!bad_q.verify());

    let mut bad_chain = r1.clone();
    bad_chain.prev_receipt_hash = "a".repeat(64);
    assert!(!CalibrationAttestationReceipt::verify_chain(&[
        r0, bad_chain
    ]));
}

struct ScriptedPhase5Provider {
    manifest: pandora_provider::ProviderManifest,
    turn: std::sync::Mutex<u32>,
}

impl pandora_provider::Provider for ScriptedPhase5Provider {
    fn manifest(&self) -> &pandora_provider::ProviderManifest {
        &self.manifest
    }

    fn complete(
        &self,
        _request: pandora_provider::ModelRequest,
    ) -> Result<pandora_provider::ModelResponse, pandora_provider::ProviderError> {
        let mut t = self.turn.lock().unwrap();
        *t += 1;
        if *t == 1 {
            let call = pandora_provider::ToolCall::new(
                "call-phase5-1",
                "workspace.read",
                serde_json::json!({
                    "path": "README.md",
                    "note": "override safe read inspection"
                }),
            )?;
            Ok(pandora_provider::ModelResponse::new(
                "",
                vec![call],
                pandora_provider::TokenUsage::new(10, 5),
            ))
        } else {
            Ok(pandora_provider::ModelResponse::new(
                "Completed inspection.",
                vec![],
                pandora_provider::TokenUsage::new(12, 6),
            ))
        }
    }
}

#[test]
fn phase5_agent_loop_emits_and_persists_dpo_pairs_on_lats_escalation_override() {
    use pandora_harnesses::HarnessCatalog;
    use pandora_runtime::agent_loop::{AgentLoop, LatsLoopConfig};
    use pandora_runtime::execution_controller::ExecutionController;
    use pandora_runtime::executors::WorkspaceRoot;
    use pandora_types::{
        DpoPair, PolicyContext, PrincipalId, Session, SessionId, TenantId, Timestamp, WorkspaceId,
    };
    use std::sync::{Arc, Mutex};

    let temp_dir = std::env::temp_dir().join(format!(
        "pandora_phase5_dpo_loop_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    std::fs::write(temp_dir.join("README.md"), "phase 5 test content\n").unwrap();
    let dpo_jsonl_path = temp_dir.join(".pandora/dpo_pairs.jsonl");

    let budget = StrategyBudget::new(4, 3, 1000, 8, 10, 1000).unwrap();
    let driver = Arc::new(TropicalDecisionDriver::calibrated());
    let lats_cfg =
        LatsLoopConfig::new(budget, 1.4, 0.95, driver).with_dpo_output_path(&dpo_jsonl_path);

    let loop_engine = AgentLoop::new(4, 4).unwrap().with_lats(lats_cfg);
    let ws_root = WorkspaceRoot::new(&temp_dir).unwrap();
    let policy = PolicyContext::read_only_workspace();
    let controller =
        ExecutionController::with_policy_and_harnesses(ws_root, policy, HarnessCatalog::builtins());

    let manifest = pandora_provider::ProviderManifest::new(
        "mock-phase5",
        "Mock Phase 5 Provider",
        "https://api.example.com/v1",
        "mock-model",
        "PANDORA_MOCK_KEY",
    )
    .unwrap();
    let provider = ScriptedPhase5Provider {
        manifest,
        turn: Mutex::new(0),
    };
    let now = Timestamp::from_unix_seconds(1_700_000_000);
    let session = Session::new(
        SessionId::new("sess-phase5-dpo").unwrap(),
        PrincipalId::new("user-phase5").unwrap(),
        TenantId::new("tenant-phase5").unwrap(),
        WorkspaceId::new("ws-phase5").unwrap(),
        now,
    );

    let summary = loop_engine
        .run(&provider, &controller, session, "inspect README.md", now)
        .unwrap();

    assert_eq!(summary.dpo_pairs_emitted(), 1);
    assert_eq!(summary.dpo_pairs().len(), 1);
    assert_eq!(summary.lats_telemetry().len(), 1);
    let t0 = &summary.lats_telemetry()[0];
    assert!(t0.escalated);
    assert!(t0.dpo_pair_emitted);
    assert_eq!(t0.reflex_action, Some(1)); // System 1 ambiguous ("force") -> RequireConfirm (1)
    assert_eq!(t0.best_action, Some(0)); // System 2 LATS MCTS ("safe read") -> Allow (0)

    assert!(
        dpo_jsonl_path.is_file(),
        "expected .pandora/dpo_pairs.jsonl to be written"
    );
    let raw_jsonl = std::fs::read_to_string(&dpo_jsonl_path).unwrap();
    let lines: Vec<&str> = raw_jsonl.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 1);
    let persisted_pair: DpoPair = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(persisted_pair.chosen, "allow");
    assert_eq!(persisted_pair.rejected, "require_confirm");
    assert_eq!(persisted_pair, summary.dpo_pairs()[0]);

    let _ = std::fs::remove_dir_all(&temp_dir);
}
