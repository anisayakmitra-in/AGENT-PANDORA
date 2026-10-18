use pandora_runtime::{
    BayesianCanaryGate, ContextVfs, IslandModelEvolver, IslandNiche, MeshSyncEngine,
    ParetoCandidate, ParetoFrontier, PeerIdentity, ReflexionCrystallizer, SessionJournalEngine,
    SessionStore, SignedCapsulePackage, SinglePassLogitEngine,
};
use pandora_types::{
    ContextClassification, ContextContractError, ContextUri, ConvergenceChoice, DecisionQuestion,
    DpoPair, ExecutionTopology, FeatureEntry, FeatureMap, GoldenSuiteDefinition, IntentTierChoice,
    MemoryCapsule, MemoryContractError, MemoryIntent, MemoryKind, MemoryOrigin, MemoryRecord,
    MemoryScope, OrchestrationPlanJson, OrchestrationRole, PrincipalId, Session,
    SessionArchiveRecord, SessionId, SopDefinition, SopStep, TenantId, Timestamp, ToolSafetyChoice,
    VfsNodeKind, WorkspaceId,
};
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(100);

const LOCAL_TENANT: &str = "local-tenant";
const LOCAL_WORKSPACE: &str = "local-workspace";

struct Fixture {
    root: PathBuf,
    config: PathBuf,
    data: PathBuf,
    workspace: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock available")
            .as_nanos();
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let temp_root = std::env::temp_dir()
            .canonicalize()
            .expect("canonical temp dir");
        let root = temp_root.join(format!(
            "pandora-arch-supersets-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).expect("workspace should be created");
        Self {
            config: root.join("config.json"),
            data: root.join("data"),
            workspace,
            root,
        }
    }

    fn setup(&self) {
        let output = Command::new(env!("CARGO_BIN_EXE_pandora"))
            .args(["setup", "--config"])
            .arg(&self.config)
            .args(["--data-dir"])
            .arg(&self.data)
            .env("PANDORA_CONFIG", &self.config)
            .env("PANDORA_DATA_DIR", &self.data)
            .env("PANDORA_WORKSPACE", &self.workspace)
            .output()
            .expect("setup command should start");
        assert!(
            output.status.success(),
            "setup failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(self.config.exists(), "config file should exist");
    }

    fn run(&self, args: &[&str]) -> JsonResponse {
        let mut full_args = args.to_vec();
        if !full_args.contains(&"--json") {
            full_args.push("--json");
        }
        let output = Command::new(env!("CARGO_BIN_EXE_pandora"))
            .args(&full_args)
            .env("PANDORA_CONFIG", &self.config)
            .env("PANDORA_DATA_DIR", &self.data)
            .env("PANDORA_WORKSPACE", &self.workspace)
            .env("PANDORA_MASTER_KEY", "test-master-key-1234")
            .output()
            .expect("Pandora command should execute");
        JsonResponse::from_output(output)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct JsonResponse {
    status: ExitStatus,
    text: String,
    value: Value,
}

impl JsonResponse {
    fn from_output(output: Output) -> Self {
        let text = String::from_utf8(output.stdout).expect("valid utf-8 stdout");
        let value = serde_json::from_str(&text).unwrap_or(Value::Null);
        Self {
            status: output.status,
            text,
            value,
        }
    }

    fn success(self, command: &str) -> Value {
        assert!(self.status.success(), "command failed: {}", self.text);
        assert_eq!(self.value["version"], "0.1");
        assert_eq!(self.value["command"], command);
        self.value
    }
}

#[test]
fn contract_context_vfs_cli_and_in_memory() {
    let fixture = Fixture::new();
    fixture.setup();

    let tenant = TenantId::new(LOCAL_TENANT).unwrap();
    let workspace = WorkspaceId::new(LOCAL_WORKSPACE).unwrap();
    let session_id = SessionId::new("vfs-sess-1").unwrap();
    let principal = PrincipalId::new("principal-1").unwrap();

    let sessions = SessionStore::open(fixture.data.join("sessions.sqlite3")).unwrap();
    sessions
        .create(&Session::new(
            session_id.clone(),
            principal.clone(),
            tenant.clone(),
            workspace.clone(),
            Timestamp::from_unix_seconds(100),
        ))
        .unwrap();

    let step = SopStep::new("step-init", "Initialize", "echo init", vec![]).unwrap();
    let sop = SopDefinition::new(
        "sop-vfs-1",
        "VFS Pipeline SOP",
        MemoryIntent::Procedure,
        vec![step],
        None,
    )
    .unwrap();
    let stored_sop = sessions
        .store_sop(&tenant, &workspace, &sop, Timestamp::from_unix_seconds(110))
        .unwrap();

    let archive = SessionArchiveRecord::new(
        "archive-vfs-1",
        tenant.clone(),
        workspace.clone(),
        session_id.clone(),
        "VFS Archival",
        "+10 -5",
        "sha256:vfsdigest123",
        Timestamp::from_unix_seconds(120),
    )
    .unwrap();
    sessions.store_archive(&archive).unwrap();

    let mem_rec = MemoryRecord::new_l1_with_origin(
        "rec-index-1",
        MemoryKind::Lesson,
        MemoryScope::new(
            tenant.clone(),
            workspace.clone(),
            session_id.clone(),
            "test-prov",
        )
        .unwrap(),
        "Index surgical memory insight",
        ContextClassification::Internal,
        Timestamp::from_unix_seconds(130),
        "transcript",
        MemoryOrigin::Explicit,
        Vec::new(),
    )
    .unwrap()
    .with_intent(MemoryIntent::Repo);
    sessions.record_memory(&principal, &mem_rec).unwrap();
    drop(sessions);

    // Test workspace feature map file loading
    let pandora_dir = fixture.workspace.join(".pandora");
    fs::create_dir_all(&pandora_dir).unwrap();
    let feat_map = FeatureMap::new(vec![FeatureEntry {
        name: "vfs".to_string(),
        entry_point: "pandora://".to_string(),
        description: "Context virtual filesystem from file".to_string(),
        invariants: vec!["read_only_in_memory".to_string()],
        dependencies: vec![],
    }]);
    fs::write(
        pandora_dir.join("feature_map.json"),
        serde_json::to_string(&feat_map).unwrap(),
    )
    .unwrap();

    // Test CLI: pandora memory vfs ls pandora://
    let resp = fixture.run(&["memory", "vfs", "ls", "pandora://"]);
    let val = resp.success("memory vfs ls");
    let entries = val["entries"].as_array().expect("entries array");
    let uris: Vec<&str> = entries.iter().map(|e| e["uri"].as_str().unwrap()).collect();
    assert!(uris.contains(&"pandora://sops"));
    assert!(uris.contains(&"pandora://archives"));
    assert!(uris.contains(&"pandora://features"));
    assert!(uris.contains(&"pandora://skills"));

    // Test CLI: shorthand syntax pandora memory vfs ls sops
    let resp_short = fixture.run(&["memory", "vfs", "ls", "sops"]);
    let val_short = resp_short.success("memory vfs ls");
    assert!(!val_short["entries"].as_array().unwrap().is_empty());

    // Test CLI: pandora memory vfs ls pandora://sops
    let resp_sops = fixture.run(&["memory", "vfs", "ls", "pandora://sops"]);
    let val_sops = resp_sops.success("memory vfs ls");
    let sops_uris: Vec<&str> = val_sops["entries"]
        .as_array()
        .expect("sop entries")
        .iter()
        .map(|e| e["uri"].as_str().unwrap())
        .collect();
    assert!(sops_uris.contains(&"pandora://sops/sop-vfs-1"));

    // Test CLI: pandora memory vfs cat pandora://sops/sop-vfs-1
    let resp_cat = fixture.run(&["memory", "vfs", "cat", "pandora://sops/sop-vfs-1"]);
    let val_cat = resp_cat.success("memory vfs cat");
    assert_eq!(val_cat["uri"], "pandora://sops/sop-vfs-1");
    let content = val_cat["content"].as_str().expect("content string");
    assert!(content.contains("VFS Pipeline SOP"));

    // Test CLI: SQLite memory records visible in VFS index
    let resp_idx = fixture.run(&["memory", "vfs", "ls", "pandora://index"]);
    let val_idx = resp_idx.success("memory vfs ls");
    let idx_uris: Vec<&str> = val_idx["entries"]
        .as_array()
        .expect("index entries")
        .iter()
        .map(|e| e["uri"].as_str().unwrap())
        .collect();
    assert!(idx_uris.contains(&"pandora://index/repo"));

    // Test CLI: Cat index repo item
    let resp_cat_idx = fixture.run(&["memory", "vfs", "cat", "pandora://index/repo/rec-index-1"]);
    let val_cat_idx = resp_cat_idx.success("memory vfs cat");
    assert!(
        val_cat_idx["content"]
            .as_str()
            .unwrap()
            .contains("Index surgical memory insight")
    );

    // Test CLI: Cat directory pandora://index/repo fails as IsADirectory
    let resp_cat_dir = fixture.run(&["memory", "vfs", "cat", "pandora://index/repo"]);
    assert!(!resp_cat_dir.status.success());
    assert!(resp_cat_dir.text.contains("directory"));

    // Test CLI: Cat workspace feature map loaded from file
    let resp_feat = fixture.run(&["memory", "vfs", "cat", "pandora://features/workspace"]);
    let val_feat = resp_feat.success("memory vfs cat");
    assert!(
        val_feat["content"]
            .as_str()
            .unwrap()
            .contains("Context virtual filesystem from file")
    );

    // Traversal rejection tests
    assert!(matches!(
        ContextUri::new("pandora://sops/../meta"),
        Err(ContextContractError::InvalidUri(_))
    ));
    assert!(matches!(
        ContextUri::new("pandora://sops\\meta"),
        Err(ContextContractError::InvalidUri(_))
    ));

    // Direct in-memory ContextVfs tests
    let vfs = ContextVfs::new();
    vfs.add_sop(stored_sop);
    vfs.add_archive(archive);
    vfs.add_feature_map(
        "arch_features",
        FeatureMap::new(vec![FeatureEntry {
            name: "vfs".to_string(),
            entry_point: "pandora://".to_string(),
            description: "Context virtual filesystem".to_string(),
            invariants: vec!["read_only_in_memory".to_string()],
            dependencies: vec![],
        }]),
    );

    let sop_uri = ContextUri::new("pandora://sops/sop-vfs-1").unwrap();
    let resolved = vfs.resolve(&sop_uri).expect("should resolve sop");
    assert!(resolved.content().contains("VFS Pipeline SOP"));

    let feat_uri = ContextUri::new("pandora://features/arch_features").unwrap();
    let feat_frag = vfs.resolve(&feat_uri).expect("should resolve feature map");
    assert!(feat_frag.content().contains("Context virtual filesystem"));

    let root_uri = ContextUri::new("pandora://").unwrap();
    let root_ls = vfs.list(&root_uri).expect("root ls should succeed");
    assert!(
        root_ls
            .iter()
            .any(|e| e.name == "sops" && e.kind == VfsNodeKind::Directory)
    );
    assert!(
        root_ls
            .iter()
            .any(|e| e.name == "features" && e.kind == VfsNodeKind::Directory)
    );
    assert!(
        root_ls
            .iter()
            .any(|e| e.name == "skills" && e.kind == VfsNodeKind::Directory)
    );

    let non_existent = ContextUri::new("pandora://invalid/route").unwrap();
    assert!(vfs.resolve(&non_existent).is_err());
}

#[test]
fn contract_signed_evolutionary_memory_capsule_lifecycle() {
    let fixture1 = Fixture::new();
    fixture1.setup();

    let tenant = TenantId::new(LOCAL_TENANT).unwrap();
    let workspace = WorkspaceId::new(LOCAL_WORKSPACE).unwrap();

    let sessions1 = SessionStore::open(fixture1.data.join("sessions.sqlite3")).unwrap();
    let step = SopStep::new(
        "step-optimize",
        "Optimize Latency",
        "cargo bench --bench latency",
        vec![],
    )
    .unwrap();
    let sop = SopDefinition::new(
        "sop-evo-export-1",
        "Evolutionary Benchmarking SOP",
        MemoryIntent::Procedure,
        vec![step],
        Some("cargo clean".to_string()),
    )
    .unwrap();
    sessions1
        .store_sop(&tenant, &workspace, &sop, Timestamp::from_unix_seconds(500))
        .unwrap();
    drop(sessions1);

    let capsule_file = fixture1.workspace.join("exported_sop.capsule.json");
    let export_resp = fixture1.run(&[
        "memory",
        "capsule",
        "export",
        "--sop-id",
        "sop-evo-export-1",
        "--out",
        capsule_file.to_str().unwrap(),
        "--author",
        "evolution-operator",
    ]);
    let export_val = export_resp.success("memory capsule export");
    assert_eq!(export_val["sop_id"], "sop-evo-export-1");
    assert!(capsule_file.exists(), "capsule file was created");

    // Read and verify the capsule file JSON
    let capsule_str = fs::read_to_string(&capsule_file).unwrap();
    let capsule: MemoryCapsule = serde_json::from_str(&capsule_str).unwrap();
    assert!(capsule.verify_digest(), "digest verification passes");
    assert_eq!(capsule.author, "evolution-operator");
    assert_eq!(capsule.sop.id, "sop-evo-export-1");

    // Second fixture: Import into a completely fresh environment
    let fixture2 = Fixture::new();
    fixture2.setup();

    let import_resp = fixture2.run(&[
        "memory",
        "capsule",
        "import",
        "--file",
        capsule_file.to_str().unwrap(),
    ]);
    let import_val = import_resp.success("memory capsule import");
    assert_eq!(import_val["sop_id"], "sop-evo-export-1");

    // Verify imported SOP is stored in fixture 2's session store
    let sessions2 = SessionStore::open(fixture2.data.join("sessions.sqlite3")).unwrap();
    let loaded = sessions2
        .load_sop(
            &TenantId::new(LOCAL_TENANT).unwrap(),
            &WorkspaceId::new(LOCAL_WORKSPACE).unwrap(),
            "sop-evo-export-1",
        )
        .unwrap();
    assert!(
        loaded.is_some(),
        "imported SOP should exist in session store"
    );
    drop(sessions2);

    // Tampering rejection test
    let mut tampered = capsule.clone();
    tampered.author = "tampered-author".to_string();
    let tampered_path = fixture2.workspace.join("tampered.capsule.json");
    fs::write(&tampered_path, serde_json::to_string(&tampered).unwrap()).unwrap();

    let tampered_resp = fixture2.run(&[
        "memory",
        "capsule",
        "import",
        "--file",
        tampered_path.to_str().unwrap(),
    ]);
    assert!(!tampered_resp.status.success());
    assert!(tampered_resp.text.contains("SHA-256") || tampered_resp.text.contains("tampered"));

    // Corrupted capsule: Invalid SOP step with empty command
    let mut corrupt = capsule;
    corrupt.sop.steps[0].command = "".to_string();
    corrupt.sha256_digest = MemoryCapsule::compute_digest(
        &corrupt.capsule_id,
        &corrupt.sop,
        &corrupt.author,
        corrupt.created_at,
    );
    assert_eq!(
        corrupt.validate(),
        Err(MemoryContractError::EmptyField("step command"))
    );
    let corrupt_path = fixture2.workspace.join("corrupt.capsule.json");
    fs::write(&corrupt_path, serde_json::to_string(&corrupt).unwrap()).unwrap();
    let corrupt_resp = fixture2.run(&[
        "memory",
        "capsule",
        "import",
        "--file",
        corrupt_path.to_str().unwrap(),
    ]);
    assert!(!corrupt_resp.status.success());
    assert!(
        corrupt_resp.text.contains("verification failed")
            || corrupt_resp.text.contains("step command")
    );
}

#[test]
fn contract_system_1_decision_engine_reflex_and_anti_thrash() {
    let engine = SinglePassLogitEngine::new();

    // Numerical stability under extreme logits
    let extreme_logits = [1000.0, 1002.0, 999.0];
    let probs = SinglePassLogitEngine::compute_softmax(&extreme_logits, 1.0)
        .expect("extreme logits evaluation should succeed without NaN");
    assert!((probs.iter().sum::<f64>() - 1.0).abs() < 1e-6);
    assert!(probs[1] > probs[0] && probs[0] > probs[2]);
    assert!(!probs[0].is_nan());

    // Softmax invalid input rejection (NaN and Inf)
    assert!(SinglePassLogitEngine::compute_softmax(&[f64::NAN, 1.0], 1.0).is_err());
    assert!(SinglePassLogitEngine::compute_softmax(&[f64::INFINITY, 1.0], 1.0).is_err());

    // Temperature calibration: Low temperature vs High temperature
    let cold_probs = SinglePassLogitEngine::compute_softmax(&[2.0, 1.0, 0.0], 0.2).unwrap();
    let warm_probs = SinglePassLogitEngine::compute_softmax(&[2.0, 1.0, 0.0], 2.5).unwrap();
    assert!(
        cold_probs[0] > warm_probs[0],
        "lower temperature should produce higher peak probability"
    );

    // Tool safety gating evaluation
    let safe_verdict = engine
        .evaluate_tool_safety("read_file", &json!({"path": "/workspace/test.txt"}))
        .unwrap();
    assert_eq!(safe_verdict.selected, ToolSafetyChoice::Allow);

    let mutating_verdict = engine
        .evaluate_tool_safety("write_to_file", &json!({"path": "/workspace/out.txt"}))
        .unwrap();
    assert_eq!(mutating_verdict.selected, ToolSafetyChoice::RequireConfirm);

    let destructive_verdict = engine
        .evaluate_tool_safety("run_command", &json!({"command": "rm -rf /"}))
        .unwrap();
    assert_eq!(destructive_verdict.selected, ToolSafetyChoice::Reject);

    let git_reset_verdict = engine
        .evaluate_tool_safety(
            "run_command",
            &json!({"command": "git reset --hard HEAD~1"}),
        )
        .unwrap();
    assert_eq!(git_reset_verdict.selected, ToolSafetyChoice::Reject);

    let del_verdict = engine
        .evaluate_tool_safety("run_command", &json!({"command": "del /s /f *"}))
        .unwrap();
    assert_eq!(del_verdict.selected, ToolSafetyChoice::Reject);

    // Intent routing evaluation
    let arch_verdict = engine
        .route_intent("architect and redesign the memory supersets")
        .unwrap();
    assert_eq!(arch_verdict.selected, IntentTierChoice::L2Architect);

    let surg_verdict = engine.route_intent("fix bug in test").unwrap();
    assert_eq!(surg_verdict.selected, IntentTierChoice::L1Surgical);

    let local_verdict = engine.route_intent("hello").unwrap();
    assert_eq!(local_verdict.selected, IntentTierChoice::L0Local);

    // Anti-thrash loop detection
    let stuck_verdict = engine
        .detect_anti_thrash(&["edit_file", "edit_file", "edit_file"])
        .unwrap();
    assert_eq!(stuck_verdict.selected, ConvergenceChoice::Stuck);

    let cycle_verdict = engine
        .detect_anti_thrash(&["run_test", "edit_file", "run_test", "edit_file"])
        .unwrap();
    assert_eq!(cycle_verdict.selected, ConvergenceChoice::Stuck);

    let cycle3_verdict = engine
        .detect_anti_thrash(&["step_a", "step_b", "step_c", "step_a", "step_b", "step_c"])
        .unwrap();
    assert_eq!(cycle3_verdict.selected, ConvergenceChoice::Stuck);

    let progress_verdict = engine
        .detect_anti_thrash(&["list_dir", "view_file", "edit_file"])
        .unwrap();
    assert_eq!(progress_verdict.selected, ConvergenceChoice::Progress);

    let conv_verdict = engine
        .detect_anti_thrash(&["list_dir", "view_file", "complete_task"])
        .unwrap();
    assert_eq!(conv_verdict.selected, ConvergenceChoice::Converged);
}

#[test]
fn contract_gepa_pareto_frontier_multi_objective() {
    let mut frontier = ParetoFrontier::new();

    // A dominates B on all axes (higher pass_rate, lower token_cost, lower latency_ms)
    let candidate_a = ParetoCandidate::new("candidate_A", 0.95, 120, 45);
    let candidate_b = ParetoCandidate::new("candidate_B", 0.80, 200, 75);

    // C has higher pass rate than A, but higher token cost and latency (trade-off)
    let candidate_c = ParetoCandidate::new("candidate_C", 0.99, 300, 110);

    assert!(frontier.try_insert(candidate_a));
    // B should be rejected as dominated by A
    assert!(!frontier.try_insert(candidate_b));
    // C is non-dominated, accepted into frontier
    assert!(frontier.try_insert(candidate_c));

    assert_eq!(frontier.len(), 2);
    let candidates = frontier.candidates();
    assert!(candidates.iter().any(|c| c.id == "candidate_A"));
    assert!(candidates.iter().any(|c| c.id == "candidate_C"));
    assert!(!candidates.iter().any(|c| c.id == "candidate_B"));

    // Verify ordering by pass rate
    let best = frontier.best_by_pass_rate().expect("best candidate");
    assert_eq!(best.id, "candidate_C");

    // Same-ID candidate updates existing candidate in frontier
    let candidate_a_v2 = ParetoCandidate::new("candidate_A", 0.98, 110, 40);
    assert!(frontier.try_insert(candidate_a_v2));
    assert_eq!(frontier.len(), 2);
    let best_v2 = frontier.best_by_pass_rate().unwrap();
    assert_eq!(best_v2.id, "candidate_C");
    let cand_a = frontier
        .candidates()
        .iter()
        .find(|c| c.id == "candidate_A")
        .unwrap();
    assert_eq!(cand_a.pass_rate, 0.98);

    // Pass rate clamping and NaN safety
    let cand_nan = ParetoCandidate::new("candidate_nan", f64::NAN, 100, 50);
    assert_eq!(cand_nan.pass_rate, 0.0);
    let cand_over = ParetoCandidate::new("candidate_over", 1.5, 100, 50);
    assert_eq!(cand_over.pass_rate, 1.0);
}

#[test]
fn contract_bayesian_canary_gate_bisection_and_evaluations() {
    let canary = BayesianCanaryGate::new(0.95);

    // Sufficient evidence of high success -> Promoted
    let pass_verdict = canary.evaluate(50, 2, 0.80);
    assert!(pass_verdict.promoted);
    assert!(pass_verdict.lower_bound_95 > 0.80);
    assert_eq!(pass_verdict.passes, 50);
    assert_eq!(pass_verdict.failures, 2);
    assert_eq!(pass_verdict.alpha, 51.0);
    assert_eq!(pass_verdict.beta, 3.0);

    // Clear evidence of poor performance -> Not promoted
    let fail_verdict = canary.evaluate(5, 40, 0.80);
    assert!(!fail_verdict.promoted);
    assert!(fail_verdict.lower_bound_95 < 0.80);

    // Insufficient evidence (small sample size) -> Not promoted
    let scarce_verdict = canary.evaluate(5, 0, 0.80);
    assert!(!scarce_verdict.promoted);
    assert!(scarce_verdict.lower_bound_95 < 0.80);

    // Large-scale canary evaluation (N = 10,000 runs) executes in < 1ms via Normal approx
    let large_verdict = canary.evaluate(9500, 500, 0.90);
    assert!(large_verdict.promoted);
    assert!(large_verdict.lower_bound_95 > 0.94);

    // NaN baseline rate fails closed safely
    let nan_verdict = canary.evaluate(9500, 500, f64::NAN);
    assert!(!nan_verdict.promoted);

    // Invalid credible level defaults to 0.95
    let bad_gate = BayesianCanaryGate::new(f64::NAN);
    assert_eq!(bad_gate.credible_level(), 0.95);
}

#[test]
fn contract_decision_cli_and_drivers() {
    let fixture = Fixture::new();
    fixture.setup();

    // 1. evaluate with binary flag
    let resp_eval_bin = fixture.run(&[
        "decision",
        "evaluate",
        "--question",
        "is it safe to read documentation?",
        "--binary",
    ]);
    let val_bin = resp_eval_bin.success("decision evaluate");
    assert_eq!(val_bin["selected"], "true");
    assert!(val_bin["confidence"].as_f64().unwrap() > 0.5);
    assert_eq!(val_bin["driver"], "heuristic");

    // 2. evaluate with custom choices
    let resp_eval_choices = fixture.run(&[
        "decision",
        "evaluate",
        "--question",
        "select the best operation tier",
        "--choices",
        "fast,balanced,deep",
    ]);
    let val_choices = resp_eval_choices.success("decision evaluate");
    assert!(["fast", "balanced", "deep"].contains(&val_choices["selected"].as_str().unwrap()));
    assert!(val_choices["probabilities"].is_object());

    // 3. tool-gate dangerous command
    let resp_gate_danger = fixture.run(&[
        "decision",
        "tool-gate",
        "--tool",
        "run_command",
        "--args",
        "{\"command\":\"rm -rf /\"}",
    ]);
    let val_danger = resp_gate_danger.success("decision tool-gate");
    assert_eq!(val_danger["tool"], "run_command");
    assert_eq!(val_danger["selected"], "reject");

    // 4. tool-gate safe read
    let resp_gate_safe = fixture.run(&[
        "decision",
        "tool-gate",
        "--tool",
        "read_file",
        "--args",
        "{\"path\":\"docs/ARCHITECTURE.md\"}",
    ]);
    let val_safe = resp_gate_safe.success("decision tool-gate");
    assert_eq!(val_safe["tool"], "read_file");
    assert_eq!(val_safe["selected"], "allow");

    // 5. intent-route architectural prompt
    let resp_route_arch = fixture.run(&[
        "decision",
        "intent-route",
        "--prompt",
        "architect and redesign the memory supersets",
    ]);
    let val_route_arch = resp_route_arch.success("decision intent-route");
    assert_eq!(val_route_arch["selected"], "l2_architect");

    // 6. intent-route surgical prompt
    let resp_route_surg = fixture.run(&["decision", "intent-route", "--prompt", "fix bug in test"]);
    let val_route_surg = resp_route_surg.success("decision intent-route");
    assert_eq!(val_route_surg["selected"], "l1_surgical");

    // 7. Verify runtime decision alias works identically
    let resp_alias = fixture.run(&[
        "runtime",
        "decision",
        "evaluate",
        "--question",
        "should we proceed?",
        "--choices",
        "allow,reject",
    ]);
    let val_alias = resp_alias.success("decision evaluate");
    assert!(["allow", "reject"].contains(&val_alias["selected"].as_str().unwrap()));
}

#[test]
fn contract_active_learning_uncertainty_harvester_and_golden_export() {
    let fixture = Fixture::new();
    fixture.setup();

    let golden_path = fixture.workspace.join("harvested_golden.json");
    let dpo_path = fixture.workspace.join("harvested_dpo.json");

    // 1. Run decision harvest with min-margin 0.35, export golden and dpo
    let resp = fixture.run(&[
        "decision",
        "harvest",
        "--min-margin",
        "0.35",
        "--suite-id",
        "test-harvested-suite",
        "--export-golden",
        golden_path.to_str().unwrap(),
        "--dpo-export",
        dpo_path.to_str().unwrap(),
    ]);

    let val = resp.success("decision harvest");
    assert_eq!(val["suite_id"], "test-harvested-suite");
    let harvested_count = val["harvested_count"].as_u64().expect("harvested count") as usize;
    assert!(harvested_count > 0, "should harvest uncertain decisions");
    assert!(golden_path.exists(), "golden set file should be written");
    assert!(dpo_path.exists(), "dpo pairs file should be written");

    // 2. Read and verify the golden file matches GoldenSuiteDefinition structure
    let golden_content = fs::read_to_string(&golden_path).expect("read golden set");
    let golden_suite: GoldenSuiteDefinition =
        serde_json::from_str(&golden_content).expect("parse golden suite");
    assert_eq!(
        golden_suite.suite_id.as_deref(),
        Some("test-harvested-suite")
    );
    assert_eq!(golden_suite.cases.len(), harvested_count);

    // 3. Pass the exported golden suite directly into `evaluation golden`
    let eval_resp = fixture.run(&[
        "evaluation",
        "golden",
        "--input",
        golden_path.to_str().unwrap(),
    ]);
    let eval_val = eval_resp.success("evaluation golden");
    assert_eq!(eval_val["passed"], harvested_count);
    assert_eq!(eval_val["failed"], 0);
    assert_eq!(eval_val["total"], harvested_count);

    // 4. Read and verify DPO pairs
    let dpo_content = fs::read_to_string(&dpo_path).expect("read dpo pairs");
    let dpo_pairs: Vec<DpoPair> = serde_json::from_str(&dpo_content).expect("parse dpo pairs");
    assert_eq!(dpo_pairs.len(), harvested_count);
    for pair in &dpo_pairs {
        assert!(!pair.prompt.is_empty());
        assert!(!pair.chosen.is_empty());
        assert!(!pair.rejected.is_empty());
        assert_ne!(pair.chosen, pair.rejected);
    }

    // 5. Direct in-memory engine testing for active learning and fine-tuning synthesis
    let engine = SinglePassLogitEngine::new();
    let q = DecisionQuestion::binary("q-test", "should we deploy this canary?", "true", "false");
    let v = engine.evaluate_dynamic(&q).expect("evaluate dynamic");
    let harvested = engine.harvest_uncertain_decisions_with_margin(
        std::slice::from_ref(&v),
        std::slice::from_ref(&q),
        Some(0.99),
        None,
    );
    assert_eq!(harvested.len(), 1);
    assert_eq!(harvested[0].selected_label, v.selected_label);

    let dpo_direct = engine.synthesize_dpo_pairs(&harvested);
    assert_eq!(dpo_direct.len(), 1);
    assert_eq!(dpo_direct[0].chosen, v.selected_label);
    assert_ne!(dpo_direct[0].rejected, v.selected_label);

    // Verify tight margin filtering: margin threshold lower than actual margin rejects candidate
    let strict_harvested =
        engine.harvest_uncertain_decisions_with_margin(&[v], &[q], Some(0.0001), None);
    assert!(
        strict_harvested.is_empty(),
        "tight margin threshold must filter out decisions with higher margins"
    );

    let ft_samples = engine.synthesize_fine_tuning_pairs(&harvested);
    assert_eq!(ft_samples.len(), 1);
    assert_eq!(ft_samples[0].id, "ft-q-test");
    assert!(!ft_samples[0].completion.is_empty());
}

#[test]
fn contract_darwinian_island_model_multi_population_evolution() {
    let fixture = Fixture::new();
    fixture.setup();

    let state_file = fixture.workspace.join("island_evolver.json");

    // 1. Initialize island evolver
    let init_resp = fixture.run(&[
        "evolution",
        "island",
        "init",
        "--population",
        "4",
        "--out",
        state_file.to_str().unwrap(),
    ]);
    let init_val = init_resp.success("evolution island init");
    assert_eq!(init_val["islands_count"], 3);
    assert_eq!(init_val["population_per_island"], 4);
    assert!(state_file.exists());

    // 2. Step generation by 2
    let step_resp = fixture.run(&[
        "evolution",
        "island",
        "step",
        "--state",
        state_file.to_str().unwrap(),
        "--generations",
        "2",
    ]);
    let step_val = step_resp.success("evolution island step");
    assert_eq!(step_val["current_generation"], 2);
    assert_eq!(step_val["generations_run"], 2);

    // 3. Ring elite migration
    let migrate_resp = fixture.run(&[
        "evolution",
        "island",
        "migrate",
        "--state",
        state_file.to_str().unwrap(),
        "--count",
        "1",
    ]);
    let migrate_val = migrate_resp.success("evolution island migrate");
    assert_eq!(migrate_val["migrated_count"], 3);
    let exchanges = migrate_val["exchanges"]
        .as_array()
        .expect("exchanges array");
    assert_eq!(exchanges.len(), 3);

    // 4. Run evolution for 3 more generations
    let run_resp = fixture.run(&[
        "evolution",
        "island",
        "run",
        "--state",
        state_file.to_str().unwrap(),
        "--generations",
        "3",
    ]);
    let run_val = run_resp.success("evolution island run");
    assert_eq!(run_val["total_generations"], 5);
    let frontier_size = run_val["pareto_frontier_size"].as_u64().unwrap();
    assert!(frontier_size > 0);

    // 5. In-memory IslandModelEvolver verification of multi-niche population
    let content = fs::read_to_string(&state_file).expect("read island state");
    let evolver: IslandModelEvolver = serde_json::from_str(&content).expect("parse evolver");
    assert_eq!(evolver.current_generation, 5);
    assert_eq!(evolver.islands.len(), 3);

    let quality_island = evolver
        .islands
        .iter()
        .find(|i| i.niche == IslandNiche::QualityFirst)
        .unwrap();
    let cost_island = evolver
        .islands
        .iter()
        .find(|i| i.niche == IslandNiche::CostMinimal)
        .unwrap();
    let latency_island = evolver
        .islands
        .iter()
        .find(|i| i.niche == IslandNiche::LatencyOptimized)
        .unwrap();

    assert!(!quality_island.population.is_empty());
    assert!(!cost_island.population.is_empty());
    assert!(!latency_island.population.is_empty());
}

#[test]
fn contract_dynamic_emergent_swarm_topology_synthesis() {
    let fixture = Fixture::new();
    fixture.setup();

    let plan_out = fixture.workspace.join("synthesized_diamond_plan.json");

    // 1. Synthesize plan with explicit specialist roles
    let resp = fixture.run(&[
        "orchestration",
        "synthesize",
        "--intent",
        "Autonomous security audit, cache layer tuning, and load validation",
        "--roles",
        "security_auditor,cache_specialist,load_tester",
        "--out",
        plan_out.to_str().unwrap(),
    ]);

    let val = resp.success("orchestration synthesize");
    assert_eq!(val["topology"], "diamond");
    assert_eq!(val["roles_count"], 6); // Planner + 3 specialists + Synthesizer + Canary Verifier
    assert!(val["handoffs_count"].as_u64().unwrap() >= 4);
    assert!(plan_out.exists());

    // 2. Read and parse plan JSON
    let plan_content = fs::read_to_string(&plan_out).expect("read synthesized plan");
    let plan_json: OrchestrationPlanJson =
        serde_json::from_str(&plan_content).expect("parse plan JSON");
    assert_eq!(plan_json.topology, ExecutionTopology::Diamond);
    assert_eq!(plan_json.roles.len(), 6);

    // Verify diamond DAG structure:
    // - planner has no dependencies
    let planner = plan_json
        .roles
        .iter()
        .find(|r| r.role().as_str() == "planner")
        .expect("planner exists");
    assert!(planner.depends_on().is_empty());

    // - specialists all depend on planner
    let specialists: Vec<_> = plan_json
        .roles
        .iter()
        .filter(|r| matches!(r.role(), OrchestrationRole::Custom(_)))
        .collect();
    assert_eq!(specialists.len(), 3);
    for spec in &specialists {
        assert_eq!(spec.depends_on(), &[planner.id().clone()]);
    }

    // - synthesizer depends on all 3 specialists
    let synthesizer = plan_json
        .roles
        .iter()
        .find(|r| r.role().as_str() == "maker")
        .expect("synthesizer maker exists");
    for spec in &specialists {
        assert!(synthesizer.depends_on().contains(spec.id()));
    }

    // - canary verifier depends on synthesizer
    let canary = plan_json
        .roles
        .iter()
        .find(|r| r.role().as_str() == "verifier")
        .expect("canary verifier exists");
    assert_eq!(canary.depends_on(), &[synthesizer.id().clone()]);

    // Convert to OrchestrationPlan to ensure valid contract DAG
    let plan = plan_json.to_orchestration_plan().expect("convert to plan");
    assert_eq!(plan.roles().len(), 6);

    // 3. Synthesize with default roles
    let default_resp = fixture.run(&[
        "orchestration",
        "synthesize",
        "--intent",
        "Automated code review and refactoring pipeline",
    ]);
    let default_val = default_resp.success("orchestration synthesize");
    assert_eq!(default_val["topology"], "diamond");
    assert_eq!(default_val["roles_count"], 5); // Planner + 2 default specialists + Synthesizer + Canary Verifier
}

#[test]
fn contract_island_migration_multi_candidate_exchange_and_clamping() {
    let fixture = Fixture::new();
    fixture.setup();

    let state_file = fixture.workspace.join("island_multi_mig.json");

    // 1. Initialize island evolver with population 4
    let init_resp = fixture.run(&[
        "evolution",
        "island",
        "init",
        "--population",
        "4",
        "--out",
        state_file.to_str().unwrap(),
    ]);
    init_resp.success("evolution island init");

    // 2. Perform multi-candidate migration (count = 2)
    let mig_resp = fixture.run(&[
        "evolution",
        "island",
        "migrate",
        "--state",
        state_file.to_str().unwrap(),
        "--count",
        "2",
    ]);
    let mig_val = mig_resp.success("evolution island migrate");
    assert_eq!(mig_val["migrated_count"], 6); // 2 candidates * 3 islands

    // 3. Verify each island preserved BOTH immigrants without overwriting
    let content = fs::read_to_string(&state_file).expect("read island state");
    let evolver: IslandModelEvolver = serde_json::from_str(&content).expect("parse evolver");
    for island in &evolver.islands {
        assert_eq!(island.population.len(), 4, "population size must remain 4");
        let immigrant_count = island
            .population
            .iter()
            .filter(|c| c.id.starts_with("migrated-"))
            .count();
        assert_eq!(
            immigrant_count, 2,
            "island {} must retain exactly 2 immigrants, none clobbered",
            island.id
        );
    }

    // 4. Test migration where count exceeds population (--count 10 > --population 2)
    let small_state = fixture.workspace.join("island_small_mig.json");
    let small_init = fixture.run(&[
        "evolution",
        "island",
        "init",
        "--population",
        "2",
        "--out",
        small_state.to_str().unwrap(),
    ]);
    small_init.success("evolution island init");

    let over_mig = fixture.run(&[
        "evolution",
        "island",
        "migrate",
        "--state",
        small_state.to_str().unwrap(),
        "--count",
        "10",
    ]);
    let over_val = over_mig.success("evolution island migrate");
    assert_eq!(over_val["migrated_count"], 6); // clamped to 2 per island * 3 islands

    let small_content = fs::read_to_string(&small_state).expect("read small state");
    let small_evolver: IslandModelEvolver =
        serde_json::from_str(&small_content).expect("parse small evolver");
    for island in &small_evolver.islands {
        assert_eq!(island.population.len(), 2, "population size must remain 2");
    }
}

#[test]
fn contract_swarm_topology_synthesis_duplicate_and_reserved_roles() {
    let fixture = Fixture::new();
    fixture.setup();

    // 1. Single specialist role matching fallback name "support_analyst"
    let resp1 = fixture.run(&[
        "orchestration",
        "synthesize",
        "--intent",
        "Support triage and ticket remediation workflow",
        "--roles",
        "support_analyst",
    ]);
    let val1 = resp1.success("orchestration synthesize");
    assert_eq!(val1["roles_count"], 5); // Planner + support_analyst + coordination_analyst + Synthesizer + Canary Verifier

    // 2. Duplicate roles provided by operator
    let resp2 = fixture.run(&[
        "orchestration",
        "synthesize",
        "--intent",
        "Refactoring backend microservices",
        "--roles",
        "dev,dev",
    ]);
    let val2 = resp2.success("orchestration synthesize");
    assert_eq!(val2["roles_count"], 5); // Planner + dev + support_analyst + Synthesizer + Canary Verifier

    // 3. Reserved role name "planner"
    let resp3 = fixture.run(&[
        "orchestration",
        "synthesize",
        "--intent",
        "System architecture planning and roadmapping",
        "--roles",
        "planner",
    ]);
    let val3 = resp3.success("orchestration synthesize");
    assert_eq!(val3["roles_count"], 5); // Planner + specialist_planner + support_analyst + Synthesizer + Canary Verifier

    // 4. Roles with spaces and hyphens
    let resp4 = fixture.run(&[
        "orchestration",
        "synthesize",
        "--intent",
        "Comprehensive database and security audit",
        "--roles",
        "data architect, security-tester",
    ]);
    let val4 = resp4.success("orchestration synthesize");
    assert_eq!(val4["roles_count"], 5);
}

#[test]
fn contract_reflexion_crystallize_and_evolution_loop() {
    let fixture = Fixture::new();
    fixture.setup();

    // 1. Run CLI memory crystallize --session with non-existent session ID -> must fail
    let err_resp = fixture.run(&[
        "memory",
        "crystallize",
        "--session",
        "non-existent-session-404",
    ]);
    assert!(!err_resp.status.success(), "non-existent session must fail");

    // 2. Run CLI memory crystallize --all
    let crys_resp = fixture.run(&["memory", "crystallize", "--all"]);
    let crys_val = crys_resp.success("memory crystallize");
    assert!(crys_val["results"].is_array());

    // 3. Run CLI evolution loop --once
    let evo_resp = fixture.run(&["evolution", "loop", "--once"]);
    let evo_val = evo_resp.success("evolution loop");
    assert_eq!(evo_val["status"], "completed");
    assert_eq!(evo_val["mode"], "once");
    assert_eq!(evo_val["cycles_completed"], 1);

    // 4. Test in-memory ReflexionCrystallizer with BayesianCanaryGate
    let sample_prompt = "Execute command in /tmp/work/AGENT-PANDORA with commit 0123456789abcdef0123456789abcdef01234567 session 12345678-1234-1234-1234-123456789abc and sha256:aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899";
    let (parameterized, literals) = ReflexionCrystallizer::parameterize_literals(
        sample_prompt,
        Some("/tmp/work/AGENT-PANDORA"),
    );
    assert!(parameterized.contains("{{WORKSPACE_ROOT}}"));
    assert!(parameterized.contains("{{COMMIT_SHA}}"));
    assert!(parameterized.contains("{{SESSION_ID}}"));
    assert!(parameterized.contains("{{SHA256_DIGEST}}"));
    assert_eq!(literals.len(), 4);

    // 5. Test BayesianCanaryGate evaluation
    let canary = BayesianCanaryGate::new(0.95);
    let v_pass = canary.evaluate(20, 0, 0.80);
    assert!(v_pass.promoted);
    assert!(v_pass.lower_bound_95 > 0.80);

    let v_fail = canary.evaluate(1, 10, 0.80);
    assert!(!v_fail.promoted);
}

#[test]
fn contract_session_journal_rewind_fork_diff() {
    let fixture = Fixture::new();
    fixture.setup();

    // Create a real session first in DB so session commands can operate on it
    let store =
        SessionStore::open(fixture.data.join("sessions.sqlite3")).expect("open sessions db");
    let session_id = SessionId::new("arch-journal-session-1").expect("session id");
    let principal = PrincipalId::new("principal-1").expect("principal id");
    let tenant = TenantId::new(LOCAL_TENANT).expect("tenant id");
    let workspace = WorkspaceId::new(LOCAL_WORKSPACE).expect("workspace id");
    let session = Session::new(
        session_id.clone(),
        principal.clone(),
        tenant.clone(),
        workspace.clone(),
        Timestamp::from_unix_seconds(100),
    );
    store.create(&session).expect("create session");

    // Add 3 journal steps
    let s0 = SessionJournalEngine::capture_turn_step(
        session_id.clone(),
        0,
        Some("file_write".to_string()),
        Some("call-0".to_string()),
        Some(json!({"path": "file1.txt"})),
        Some("written".to_string()),
        "+hello".to_string(),
        std::collections::HashMap::new(),
        None,
        Timestamp::from_unix_seconds(101),
    );
    store.store_journal_step(&s0).expect("store step 0");

    let s1 = SessionJournalEngine::capture_turn_step(
        session_id.clone(),
        1,
        Some("run_command".to_string()),
        Some("call-1".to_string()),
        Some(json!({"cmd": "cargo check"})),
        Some("ok".to_string()),
        "+build".to_string(),
        std::collections::HashMap::new(),
        Some(&s0.sha256_digest),
        Timestamp::from_unix_seconds(102),
    );
    store.store_journal_step(&s1).expect("store step 1");

    let s2 = SessionJournalEngine::capture_turn_step(
        session_id.clone(),
        2,
        Some("file_edit".to_string()),
        Some("call-2".to_string()),
        Some(json!({"path": "file1.txt"})),
        Some("edited".to_string()),
        "+world".to_string(),
        std::collections::HashMap::new(),
        Some(&s1.sha256_digest),
        Timestamp::from_unix_seconds(103),
    );
    store.store_journal_step(&s2).expect("store step 2");

    // Test CLI session diff
    let diff_resp = fixture.run(&[
        "session",
        "diff",
        session_id.as_str(),
        "--step-a",
        "0",
        "--step-b",
        "1",
    ]);
    let diff_val = diff_resp.success("session diff");
    assert_eq!(diff_val["step_a"], 0);
    assert_eq!(diff_val["step_b"], 1);
    assert!(diff_val["tool_diff"].as_str().unwrap().contains("Step 0"));
    assert!(
        diff_val["workspace_diff"]
            .as_str()
            .unwrap()
            .contains("--- Step 0")
    );

    // Test CLI session fork
    let fork_resp = fixture.run(&[
        "session",
        "fork",
        session_id.as_str(),
        "--step",
        "1",
        "--prompt",
        "Exploring alternative optimization path",
    ]);
    let fork_val = fork_resp.success("session fork");
    assert_eq!(fork_val["at_step"], 1);
    assert!(fork_val["forked_session_id"].as_str().is_some());

    // Test CLI session rewind
    let rewind_resp = fixture.run(&[
        "session",
        "rewind",
        session_id.as_str(),
        "--step",
        "0",
        "--yes",
    ]);
    let rewind_val = rewind_resp.success("session rewind");
    assert_eq!(rewind_val["rolled_back_to_step"], 0);
    assert_eq!(rewind_val["rolled_back_from_step"], 2);
    assert_eq!(rewind_val["confirmed"], true);

    // Verify DB truncation
    let remaining_steps = store
        .load_journal_steps(&session_id)
        .expect("load remaining steps");
    assert_eq!(remaining_steps.len(), 1);
    assert_eq!(remaining_steps[0].step_index, 0);
}

#[test]
fn contract_mesh_sync_gossip_and_3_point_validation() {
    let fixture = Fixture::new();
    fixture.setup();

    // 1. Test CLI memory mesh peers
    let peers_resp = fixture.run(&["memory", "mesh", "peers"]);
    let peers_val = peers_resp.success("memory mesh peers");
    assert!(peers_val["peers"].is_array());

    // 2. Test in-memory PeerIdentity & Handshake
    let peer = PeerIdentity::new_random();
    let handshake = peer.create_handshake("127.0.0.1:9099", vec!["gossip".to_string()]);
    assert!(handshake.verify().unwrap());
    assert_eq!(handshake.peer_id, peer.peer_id);

    // Test UDP peer announcement broadcast
    let udp_announcement =
        MeshSyncEngine::broadcast_peer_announcement(&peer, "127.0.0.1:9099", "127.0.0.1:49999");
    assert!(udp_announcement.is_ok());

    // 3. Create test SOP and MemoryCapsule
    let step = SopStep::new("step-1", "Compile Code", "cargo check", vec![]).unwrap();
    let sop = SopDefinition::new(
        "sop-mesh-test",
        "Mesh Synchronization SOP",
        MemoryIntent::Procedure,
        vec![step],
        None,
    )
    .unwrap();

    let capsule = MemoryCapsule::new(
        "capsule-mesh-alpha",
        sop.clone(),
        peer.peer_id.clone(),
        Timestamp::from_unix_seconds(200),
    )
    .unwrap();

    let author_signature = peer.sign_digest(&capsule.sha256_digest);
    let author_public_key = peer.public_key_hex();

    let package = SignedCapsulePackage {
        capsule: capsule.clone(),
        author_public_key: author_public_key.clone(),
        author_signature: author_signature.clone(),
        parent_capsule_ids: vec![],
        gossip_hops: 0,
    };

    // 4. Strict 3-point security pipeline verification
    let verdict =
        MeshSyncEngine::validate_capsule_package(&package, &std::collections::HashMap::new())
            .expect("validate signed capsule package");
    assert!(verdict.valid);
    assert!(verdict.sha256_verified);
    assert!(verdict.signature_verified);
    assert!(verdict.dag_acyclic);

    // Tampered digest check
    let mut bad_package = package.clone();
    bad_package.capsule.sha256_digest = "0".repeat(64);
    assert!(
        MeshSyncEngine::validate_capsule_package(&bad_package, &std::collections::HashMap::new())
            .is_err()
    );

    // Cyclic ancestry check
    let mut cyclic_ancestors = std::collections::HashMap::new();
    cyclic_ancestors.insert("anc-1".to_string(), vec!["anc-2".to_string()]);
    cyclic_ancestors.insert("anc-2".to_string(), vec!["anc-3".to_string()]);
    cyclic_ancestors.insert("anc-3".to_string(), vec!["anc-1".to_string()]);
    let mut cyclic_package = package.clone();
    cyclic_package.parent_capsule_ids = vec!["anc-1".to_string()];
    let cyc_res = MeshSyncEngine::validate_capsule_package(&cyclic_package, &cyclic_ancestors);
    assert!(cyc_res.is_err());

    // 5. Store capsule in DB and test CLI memory mesh sync
    let store =
        SessionStore::open(fixture.data.join("sessions.sqlite3")).expect("open sessions db");
    store
        .store_mesh_capsule(&package)
        .expect("store mesh capsule");

    let sync_resp = fixture.run(&["memory", "mesh", "sync", "--auto-promote"]);
    let sync_val = sync_resp.success("memory mesh sync");
    assert!(sync_val["admitted"].is_array());
}

#[test]
fn contract_release_manifests_generation_and_cosign_validation() {
    let fixture = Fixture::new();
    fixture.setup();

    let dist_dir = fixture.workspace.join("dist");
    fs::create_dir_all(&dist_dir).expect("create dist dir");
    let checksums_file = dist_dir.join("checksums.txt");
    let checksum_content = "\
1111111111111111111111111111111111111111111111111111111111111111  pandora-aarch64-apple-darwin\n\
2222222222222222222222222222222222222222222222222222222222222222  pandora-x86_64-apple-darwin\n\
3333333333333333333333333333333333333333333333333333333333333333  pandora-x86_64-unknown-linux-gnu\n\
4444444444444444444444444444444444444444444444444444444444444444  pandora-x86_64-pc-windows-msvc.exe\n";
    fs::write(&checksums_file, checksum_content).expect("write checksums");

    let out_dir = fixture.workspace.join("manifests_out");

    let status = Command::new("python")
        .args([
            "scripts/generate_release_manifests.py",
            "v2.0.0-beta.8",
            "--checksums",
            checksums_file.to_str().unwrap(),
            "--output-dir",
            out_dir.to_str().unwrap(),
        ])
        .current_dir(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap(),
        )
        .status()
        .expect("run generate_release_manifests.py");

    assert!(
        status.success(),
        "generate_release_manifests script must exit 0"
    );

    let formula_file = out_dir.join("Formula").join("pandora.rb");
    let scoop_file = out_dir.join("bucket").join("pandora.json");
    let winget_ver_file = out_dir.join("winget").join("manifest.version.yaml");
    let winget_inst_file = out_dir.join("winget").join("manifest.installer.yaml");
    let winget_loc_file = out_dir.join("winget").join("manifest.defaultLocale.yaml");

    assert!(formula_file.is_file());
    assert!(scoop_file.is_file());
    assert!(winget_ver_file.is_file());
    assert!(winget_inst_file.is_file());
    assert!(winget_loc_file.is_file());

    let formula = fs::read_to_string(&formula_file).unwrap();
    assert!(formula.contains("class Pandora < Formula"));
    assert!(formula.contains("1111111111111111111111111111111111111111111111111111111111111111"));

    let scoop_str = fs::read_to_string(&scoop_file).unwrap();
    let scoop_json: serde_json::Value = serde_json::from_str(&scoop_str).unwrap();
    assert_eq!(scoop_json["version"], "2.0.0-beta.8");

    let loc_str = fs::read_to_string(&winget_loc_file).unwrap();
    assert!(loc_str.contains("ManifestType: defaultLocale"));
    assert!(loc_str.contains("PackageLocale: en-US"));
}

#[test]
fn contract_curator_daemon_maintenance_and_status() {
    let fixture = Fixture::new();
    fixture.setup();

    // 1. Initial status query
    let status_resp = fixture.run(&["curator", "status"]);
    let status_val = status_resp.success("curator status");
    assert_eq!(status_val["total_sessions"], 0);
    assert_eq!(status_val["crystallized_sops"], 0);

    // 2. Run single-pass maintenance
    let run_resp = fixture.run(&["curator", "run", "--once"]);
    let run_val = run_resp.success("curator run");
    assert_eq!(run_val["sessions_evaluated"], 0);
    assert_eq!(run_val["sops_crystallized"], 0);
    assert!(run_val["wal_checkpointed"].as_bool().unwrap_or(false));

    // 3. Memory alias access
    let mem_status_resp = fixture.run(&["memory", "curator", "status"]);
    let mem_status_val = mem_status_resp.success("curator status");
    assert_eq!(mem_status_val["total_sessions"], 0);
}

#[test]
fn contract_tool_search_cli() {
    let fixture = Fixture::new();
    fixture.setup();

    // 1. Search for workspace tools
    let resp = fixture.run(&["tool", "search", "read workspace", "--limit", "3"]);
    let val = resp.success("tool search");
    assert_eq!(val["query"], "read workspace");
    let results = val["results"].as_array().unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0]["tool_id"], "workspace.read");

    // 2. Search for git diff
    let resp_diff = fixture.run(&["tool", "search", "git diff"]);
    let val_diff = resp_diff.success("tool search");
    let results_diff = val_diff["results"].as_array().unwrap();
    assert!(
        results_diff
            .iter()
            .any(|r| r["tool_id"] == "workspace.diff")
    );
}

#[test]
fn contract_secret_pool_cli() {
    let fixture = Fixture::new();
    fixture.setup();

    // 1. Add primary key to pool
    let resp = fixture.run(&[
        "secret",
        "pool",
        "add",
        "openai-pool",
        "--key",
        "OPENAI_KEY_1",
        "--priority",
        "1",
        "--strategy",
        "priority",
    ]);
    let val = resp.success("secret pool add");
    assert_eq!(val["pool_id"], "openai-pool");
    assert_eq!(val["key_id"], "OPENAI_KEY_1");
    assert_eq!(val["priority"], 1);

    // 2. Add secondary key to pool
    let resp2 = fixture.run(&[
        "secret",
        "pool",
        "add",
        "openai-pool",
        "--key",
        "OPENAI_KEY_2",
        "--priority",
        "2",
    ]);
    let val2 = resp2.success("secret pool add");
    assert_eq!(val2["total_keys"], 2);

    // 3. List pools
    let resp_list = fixture.run(&["secret", "pool", "list"]);
    let val_list = resp_list.success("secret pool list");
    assert_eq!(val_list["count"], 1);

    // 4. Inspect pool status
    let resp_status = fixture.run(&["secret", "pool", "status", "openai-pool"]);
    let val_status = resp_status.success("secret pool status");
    assert_eq!(val_status["status"]["total_keys"], 2);
    assert_eq!(val_status["status"]["active_keys"], 2);
    assert_eq!(val_status["status"]["cooldown_keys"], 0);
    assert_eq!(val_status["status"]["selected_key_id"], "OPENAI_KEY_1");

    // 5. Rotate pool (select primary)
    let resp_rot = fixture.run(&["secret", "pool", "rotate", "openai-pool"]);
    let val_rot = resp_rot.success("secret pool rotate");
    assert_eq!(val_rot["selected_key"]["key_id"], "OPENAI_KEY_1");

    // 6. Report 429 rate limit on primary key
    let resp_429 = fixture.run(&[
        "secret",
        "pool",
        "report-429",
        "openai-pool",
        "--key",
        "OPENAI_KEY_1",
        "--cooldown",
        "120",
    ]);
    let val_429 = resp_429.success("secret pool report-429");
    assert_eq!(val_429["key_id"], "OPENAI_KEY_1");
    assert!(val_429["next_reset_seconds"].as_u64().unwrap() > 0);

    // 7. Rotate pool after 429 -> must failover to OPENAI_KEY_2!
    let resp_rot2 = fixture.run(&["secret", "pool", "rotate", "openai-pool"]);
    let val_rot2 = resp_rot2.success("secret pool rotate");
    assert_eq!(val_rot2["selected_key"]["key_id"], "OPENAI_KEY_2");

    // 8. Remove key from pool
    let resp_rem_key = fixture.run(&[
        "secret",
        "pool",
        "remove",
        "openai-pool",
        "--key",
        "OPENAI_KEY_1",
        "--yes",
    ]);
    let val_rem_key = resp_rem_key.success("secret pool remove");
    assert_eq!(val_rem_key["removed"], true);

    // 9. Remove entire pool
    let resp_rem_pool = fixture.run(&["secret", "pool", "remove", "openai-pool", "--yes"]);
    let val_rem_pool = resp_rem_pool.success("secret pool remove");
    assert_eq!(val_rem_pool["removed"], true);
}

#[test]
fn contract_hardware_autopilot_cli() {
    let fixture = Fixture::new();

    // 1. Probe host hardware
    let resp_probe = fixture.run(&["hardware", "probe"]);
    let val_probe = resp_probe.success("hardware probe");
    assert!(val_probe["cpu_cores"].as_u64().unwrap() > 0);
    assert!(val_probe["total_ram_bytes"].as_u64().unwrap() > 0);
    assert!(val_probe["primary_backend"].is_string());

    // 2. Evaluate model fit
    let resp_fit = fixture.run(&[
        "hardware",
        "fit",
        "--model",
        "deepseek-r1:8b",
        "--params",
        "8.0",
        "--quant",
        "q4_k_m",
        "--context",
        "8192",
    ]);
    let val_fit = resp_fit.success("hardware fit");
    assert_eq!(val_fit["model_id"], "deepseek-r1:8b");
    assert_eq!(val_fit["parameters_billion"], 8.0);
    assert_eq!(val_fit["context_window"], 8192);
    assert!(val_fit["total_required_bytes"].as_u64().unwrap() > 0);
    assert!(val_fit["fit_score"].as_f64().unwrap() > 0.0);
    assert!(val_fit["fit_verdict"].is_string());

    // 3. Preset recommendations
    let resp_presets = fixture.run(&["hardware", "presets"]);
    let val_presets = resp_presets.success("hardware presets");
    let recs = val_presets["recommendations"]
        .as_array()
        .expect("array of presets");
    assert!(!recs.is_empty());
    assert!(recs.iter().any(|r| r["model_id"] == "qwen2.5:7b"));
    assert!(recs.iter().any(|r| r["model_id"] == "llama3.1:70b"));

    // 4. Invalid quantization error handling
    let resp_invalid = fixture.run(&["hardware", "fit", "--quant", "nonexistent_quant"]);
    assert!(!resp_invalid.status.success());
    assert!(resp_invalid.text.contains("unknown quantization"));
}

#[test]
fn contract_kanban_multi_agent_board_cli() {
    let fixture = Fixture::new();
    fixture.setup();

    // 1. Create root task
    let resp1 = fixture.run(&[
        "kanban",
        "task",
        "create",
        "--id",
        "task-1",
        "--title",
        "Design schema",
        "--desc",
        "Architecture review",
        "--priority",
        "1",
        "--board",
        "board-test",
    ]);
    let val1 = resp1.success("kanban task create");
    assert_eq!(val1["id"], "task-1");
    assert_eq!(val1["status"], "todo");
    assert_eq!(val1["priority"], 1);

    // 2. Create dependent task
    let resp2 = fixture.run(&[
        "kanban",
        "task",
        "create",
        "--id",
        "task-2",
        "--title",
        "Implement migration",
        "--deps",
        "task-1",
        "--priority",
        "2",
        "--board",
        "board-test",
    ]);
    let val2 = resp2.success("kanban task create");
    assert_eq!(val2["id"], "task-2");

    // 3. List tasks on board
    let resp_list = fixture.run(&["kanban", "task", "list", "--board", "board-test"]);
    let val_list = resp_list.success("kanban task list");
    assert_eq!(val_list["count"], 2);

    // 4. Starting task-2 before task-1 completes fails
    let resp_fail = fixture.run(&[
        "kanban",
        "task",
        "update",
        "task-2",
        "--status",
        "in_progress",
        "--worker",
        "worker-alpha",
    ]);
    assert!(!resp_fail.status.success());
    assert!(
        resp_fail
            .text
            .contains("dependency task 'task-1' is not completed")
    );

    // 5. Start and complete task-1
    let resp_t1_start = fixture.run(&[
        "kanban",
        "task",
        "update",
        "task-1",
        "--status",
        "in_progress",
        "--worker",
        "worker-alpha",
    ]);
    assert_eq!(
        resp_t1_start.success("kanban task update")["status"],
        "in_progress"
    );

    // Heartbeat on task-1
    let resp_hb = fixture.run(&[
        "kanban",
        "task",
        "heartbeat",
        "task-1",
        "--worker",
        "worker-alpha",
    ]);
    assert_eq!(resp_hb.success("kanban task heartbeat")["id"], "task-1");

    // Complete task-1
    let resp_t1_done = fixture.run(&["kanban", "task", "update", "task-1", "--status", "done"]);
    assert_eq!(resp_t1_done.success("kanban task update")["status"], "done");

    // 6. Now task-2 can start
    let resp_t2_start = fixture.run(&[
        "kanban",
        "task",
        "update",
        "task-2",
        "--status",
        "in_progress",
        "--worker",
        "worker-beta",
    ]);
    assert_eq!(
        resp_t2_start.success("kanban task update")["status"],
        "in_progress"
    );

    // 7. Zombie reclaim pass resets worker-beta's task-2 to todo
    let resp_reclaim = fixture.run(&[
        "kanban",
        "task",
        "reclaim",
        "--board",
        "board-test",
        "--timeout",
        "0",
    ]);
    let val_reclaim = resp_reclaim.success("kanban task reclaim");
    let reclaimed = val_reclaim["reclaimed_task_ids"].as_array().unwrap();
    assert!(reclaimed.iter().any(|id| id == "task-2"));

    // 8. Inspect task-2 reflects todo status
    let resp_insp = fixture.run(&["kanban", "task", "inspect", "task-2"]);
    let val_insp = resp_insp.success("kanban task inspect");
    assert_eq!(val_insp["status"], "todo");
    assert_eq!(val_insp["retry_count"], 1);

    // 9. Inspect board
    let resp_board = fixture.run(&["kanban", "board", "--board", "board-test"]);
    let val_board = resp_board.success("kanban board");
    assert_eq!(val_board["tasks"].as_array().unwrap().len(), 2);

    // 10. Delete task
    let resp_del = fixture.run(&["kanban", "task", "delete", "task-1", "--yes"]);
    let val_del = resp_del.success("kanban task delete");
    assert_eq!(val_del["deleted"], true);
}

#[test]
fn contract_society_peer_messaging_cli() {
    let fixture = Fixture::new();
    fixture.setup();

    // 1. Peer list: should contain auto-seeded default personas
    let resp_list = fixture.run(&["peer", "list"]);
    let val_list = resp_list.success("peer list");
    assert!(val_list["count"].as_u64().unwrap() >= 5);
    let personas = val_list["personas"].as_array().unwrap();
    assert!(personas.iter().any(|p| p["handle"] == "architect"));
    assert!(personas.iter().any(|p| p["handle"] == "coder"));

    // 2. Peer register: register custom persona @oracle
    let resp_reg = fixture.run(&[
        "peer",
        "register",
        "--handle",
        "oracle",
        "--role",
        "Synthesis and Strategic Projections",
        "--display",
        "Oracle",
        "--model",
        "hermes-custom",
        "--capabilities",
        "planning,synthesis,forecasting",
    ]);
    let val_reg = resp_reg.success("peer register");
    assert_eq!(val_reg["handle"], "oracle");
    assert_eq!(val_reg["display_name"], "Oracle");

    // 3. Peer inspect: inspect registered persona
    let resp_insp = fixture.run(&["peer", "inspect", "oracle"]);
    let val_insp = resp_insp.success("peer inspect");
    assert_eq!(val_insp["handle"], "oracle");
    assert_eq!(val_insp["role"], "Synthesis and Strategic Projections");

    // 4. Peer send: dispatch direct message from @oracle to @architect
    let resp_send = fixture.run(&[
        "peer",
        "send",
        "--from",
        "oracle",
        "--to",
        "architect",
        "--subject",
        "Architecture Verification Request",
        "--body",
        "Please review the micro-kernel boundary specification.",
    ]);
    let val_send = resp_send.success("peer send");
    let msg_id = val_send["message_id"].as_str().unwrap().to_string();
    let thread_id = val_send["thread_id"].as_str().unwrap().to_string();
    assert_eq!(val_send["from_handle"], "oracle");
    assert_eq!(val_send["to_handle"], "architect");
    assert_eq!(val_send["status"], "delivered");

    // 5. Peer inbox: recipient @architect checks unread inbox
    let resp_inbox = fixture.run(&["peer", "inbox", "--handle", "architect", "--unread"]);
    let val_inbox = resp_inbox.success("peer inbox");
    assert_eq!(val_inbox["count"], 1);
    let in_msg = &val_inbox["messages"].as_array().unwrap()[0];
    assert_eq!(in_msg["message_id"], msg_id);

    // 6. Peer read: message transition to read
    let resp_read = fixture.run(&["peer", "read", &msg_id]);
    let val_read = resp_read.success("peer read");
    assert_eq!(val_read["message_id"], msg_id);
    assert_eq!(val_read["status"], "read");

    // 7. Peer reply: @architect replies back to @oracle
    let resp_reply = fixture.run(&[
        "peer",
        "reply",
        &msg_id,
        "--from",
        "architect",
        "--body",
        "Micro-kernel specifications reviewed and approved.",
    ]);
    let val_reply = resp_reply.success("peer reply");
    assert_eq!(val_reply["to_handle"], "oracle");
    assert_eq!(val_reply["from_handle"], "architect");
    assert_eq!(val_reply["thread_id"], thread_id);

    // 8. Peer thread: inspect entire discussion history
    let resp_thread = fixture.run(&["peer", "thread", &thread_id]);
    let val_thread = resp_thread.success("peer thread");
    assert_eq!(val_thread["thread_id"], thread_id);
    assert_eq!(val_thread["message_count"], 2);
    let msgs = val_thread["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0]["message_id"], msg_id);
    assert_eq!(msgs[1]["message_id"], val_reply["message_id"]);

    // 9. Peer delete: remove @oracle
    let resp_del = fixture.run(&["peer", "delete", "oracle"]);
    let val_del = resp_del.success("peer delete");
    assert_eq!(val_del["deleted"], true);
}

#[test]
fn contract_session_prune_and_vacuum_cli() {
    let fixture = Fixture::new();
    fixture.setup();

    // 1. Session vacuum analyze: inspect storage stats
    let resp_analyze = fixture.run(&["session", "vacuum", "--analyze"]);
    let val_analyze = resp_analyze.success("session vacuum analyze");
    assert!(val_analyze["total_pages"].as_u64().unwrap() > 0);
    assert!(val_analyze["page_size_bytes"].as_u64().unwrap() > 0);
    assert!(val_analyze["total_size_bytes"].as_u64().unwrap() > 0);

    // 2. Session vacuum with force: run VACUUM
    let resp_vac = fixture.run(&["session", "vacuum", "--force"]);
    let val_vac = resp_vac.success("session vacuum");
    assert_eq!(val_vac["vacuum_executed"], true);

    // 3. Session prune dry-run: preview candidates
    let resp_dry = fixture.run(&["session", "prune", "--older-than", "0s", "--dry-run"]);
    let val_dry = resp_dry.success("session prune");
    assert_eq!(val_dry["dry_run"], true);
    assert_eq!(val_dry["older_than_seconds"], 0);

    // 4. Session prune live with --yes
    let resp_prune = fixture.run(&["session", "prune", "--older-than", "0s", "--yes"]);
    let val_prune = resp_prune.success("session prune");
    assert_eq!(val_prune["dry_run"], false);
    assert_eq!(val_prune["older_than_seconds"], 0);
}

#[test]
fn contract_a2a_protocol_cli() {
    let fixture = Fixture::new();
    fixture.setup();

    // 1. Peer A2A card: inspect local node agent card
    let resp_card = fixture.run(&["peer", "a2a", "card", "--node-id", "test-cluster-node"]);
    let val_card = resp_card.success("peer a2a card");
    assert_eq!(val_card["id"], "test-cluster-node");
    assert_eq!(val_card["auth_mode"], "bearer_token");
    let caps = val_card["capabilities"].as_array().unwrap();
    assert!(caps.iter().any(|c| c == "kanban_coordination"));
    assert!(caps.iter().any(|c| c == "peer_messaging"));

    // 2. Peer A2A rpc: execute JSON-RPC discover request
    let resp_rpc = fixture.run(&[
        "peer",
        "a2a",
        "rpc",
        "--request",
        r#"{"jsonrpc":"2.0","id":1,"method":"a2a.discover","params":{"node_id":"remote-node-1"}}"#,
    ]);
    let val_rpc = resp_rpc.success("peer a2a rpc");
    assert_eq!(val_rpc["jsonrpc"], "2.0");
    assert_eq!(val_rpc["result"]["id"], "remote-node-1");

    // 3. Peer A2A delegate: dispatch task intent over A2A
    let resp_del = fixture.run(&[
        "peer",
        "a2a",
        "delegate",
        "--endpoint",
        "local",
        "--from",
        "coordinator",
        "--to",
        "coder",
        "--intent",
        "Implement unit test fixtures",
    ]);
    let val_del = resp_del.success("peer a2a delegate");
    let task_id = val_del["task_id"].as_str().unwrap().to_string();
    assert_eq!(val_del["status"], "completed");
    assert!(
        val_del["evidence_digest"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );

    // 4. Peer A2A status: query delegation status
    let resp_stat = fixture.run(&["peer", "a2a", "status", "--task-id", &task_id]);
    let val_stat = resp_stat.success("peer a2a status");
    assert_eq!(val_stat["jsonrpc"], "2.0");
    assert_eq!(val_stat["result"]["task_id"], task_id);
    assert_eq!(val_stat["result"]["status"], "completed");
}

#[test]
fn contract_connector_pkce_lifecycle_cli() {
    let fixture = Fixture::new();
    fixture.setup();

    // 1. Initial list: should be empty
    let resp_list = fixture.run(&["connector", "list"]);
    let val_list = resp_list.success("connector list");
    assert_eq!(val_list["count"], 0);

    // 2. Authorize: initialize PKCE session for GitHub
    let resp_auth = fixture.run(&[
        "connector",
        "authorize",
        "--provider",
        "github",
        "--client-id",
        "gh_client_test",
        "--redirect-uri",
        "http://localhost:8080/callback",
        "--scopes",
        "repo,read:user",
    ]);
    let val_auth = resp_auth.success("connector authorize");
    let session_id = val_auth["session_id"].as_str().unwrap().to_string();
    let state = val_auth["state"].as_str().unwrap().to_string();
    let auth_url = val_auth["authorization_url"].as_str().unwrap();

    assert!(auth_url.contains("github.com/login/oauth/authorize"));
    assert!(auth_url.contains("code_challenge="));
    assert_eq!(val_auth["provider"], "github");

    // 3. Exchange: exchange code and state for authorized connector token
    let resp_exch = fixture.run(&[
        "connector",
        "exchange",
        "--session",
        &session_id,
        "--state",
        &state,
        "--code",
        "gh_auth_code_98765",
    ]);
    let val_exch = resp_exch.success("connector exchange");
    assert_eq!(val_exch["status"], "authorized");
    let connector_id = val_exch["connector"]["connector_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(connector_id.starts_with("github-"));

    // 4. Inspect: verify token record details
    let resp_insp = fixture.run(&["connector", "inspect", &connector_id]);
    let val_insp = resp_insp.success("connector inspect");
    assert_eq!(val_insp["connector"]["connector_id"], connector_id);
    assert_eq!(val_insp["connector"]["provider"], "github");
    assert!(
        val_insp["connector"]["token_masked"]
            .as_str()
            .unwrap()
            .contains("...8765")
    );

    // 5. List: now contains 1 connector
    let resp_list2 = fixture.run(&["connector", "list"]);
    let val_list2 = resp_list2.success("connector list");
    assert_eq!(val_list2["count"], 1);

    // 6. Revoke: remove connector
    let resp_rev = fixture.run(&["connector", "revoke", &connector_id]);
    let val_rev = resp_rev.success("connector revoke");
    assert_eq!(val_rev["status"], "revoked");
    assert_eq!(val_rev["connector_id"], connector_id);

    // 7. List: empty again
    let resp_list3 = fixture.run(&["connector", "list"]);
    let val_list3 = resp_list3.success("connector list");
    assert_eq!(val_list3["count"], 0);
}
