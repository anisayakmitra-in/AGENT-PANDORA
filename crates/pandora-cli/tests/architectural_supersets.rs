use pandora_runtime::{
    BayesianCanaryGate, ContextVfs, IslandModelEvolver, IslandNiche, ParetoCandidate,
    ParetoFrontier, SessionStore, SinglePassLogitEngine,
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
    let harvested = engine.harvest_uncertain_decisions_with_margin(&[v], &[q], Some(0.99), None);
    assert_eq!(harvested.len(), 1);

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
