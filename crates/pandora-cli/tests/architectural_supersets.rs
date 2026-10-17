use pandora_runtime::{
    BayesianCanaryGate, ContextVfs, ParetoCandidate, ParetoFrontier, SessionStore,
    SinglePassLogitEngine,
};
use pandora_types::{
    ContextClassification, ContextContractError, ContextUri, ConvergenceChoice, FeatureEntry,
    FeatureMap, IntentTierChoice, MemoryCapsule, MemoryContractError, MemoryIntent, MemoryKind,
    MemoryOrigin, MemoryRecord, MemoryScope, PrincipalId, Session, SessionArchiveRecord, SessionId,
    SopDefinition, SopStep, TenantId, Timestamp, ToolSafetyChoice, VfsNodeKind, WorkspaceId,
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
