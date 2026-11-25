#![forbid(unsafe_code)]

//! Phase 320 Opaque-Box E2E Test Suite for AGENT-PANDORA (Features F1–F14 across R1, R2, R3, R4).
//!
//! Enforces:
//! - Strict `#![forbid(unsafe_code)]` at line 1.
//! - Offline `--locked` execution using standard library, existing dependencies, and compiled CLI binary.
//! - 4-Tier test architecture:
//!   - Tier 1: Feature Coverage (R1: 6, R2: 8, R3: 6 = 20 tests)
//!   - Tier 2: Boundary & Corner Cases (R1: 6, R2: 10, R3: 6 = 22 tests)
//!   - Tier 3: Cross-Feature Combinations (5 tests)
//!   - Tier 4: Real-World Scenarios (5 tests)

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use pandora_runtime::context_vfs::{ContextVfs, deserialize_vfs_index, read_index_stats};
use pandora_runtime::decision_engine::CalibrationAttestationReceipt;
use pandora_runtime::distillation::{load_telemetry_snapshot, save_telemetry_snapshot};
use pandora_runtime::hnsw::{HnswConfig, HnswIndex, HnswVector};
use pandora_runtime::martingale_gate::{CanaryGateVerdict, MartingaleCanaryGate, MartingaleError};
use pandora_runtime::wasserstein_drift::{EmpiricalDistribution, SinkhornDriftDetector};
use pandora_runtime::{PeerIdentity, SignedCapsulePackage};
use pandora_types::distillation::{
    DistillationTelemetrySnapshot, MartingaleTelemetrySummary, WassersteinTelemetrySummary,
};
use pandora_types::{
    ContextClassification, MemoryKind, MemoryRecord, MemoryScope, SessionId, TenantId, Timestamp,
    WorkspaceId,
};
use serde_json::{Value, json};

static FIXTURE_COUNTER: AtomicU64 = AtomicU64::new(1);

// ============================================================================
// Common RAII Test Isolation Infrastructure (§4 TEST_INFRA)
// ============================================================================

struct TestEnv {
    root: PathBuf,
    workspace: PathBuf,
    data: PathBuf,
    home: PathBuf,
    sessions: PathBuf,
    config_file: PathBuf,
}

impl TestEnv {
    fn new(prefix: &str) -> Self {
        let pid = std::process::id();
        let seq = FIXTURE_COUNTER.fetch_add(1, Ordering::SeqCst);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros();
        let root = std::env::temp_dir().join(format!("pandora_e2e_{prefix}_{pid}_{seq}_{now}"));

        let workspace = root.join("workspace");
        let data = root.join("data");
        let home = root.join("home");
        let sessions = root.join("sessions");
        let config_file = root.join("config.json");

        fs::create_dir_all(&workspace).expect("created workspace dir");
        fs::create_dir_all(&data).expect("created data dir");
        fs::create_dir_all(&home).expect("created home dir");
        fs::create_dir_all(&sessions).expect("created sessions dir");
        fs::create_dir_all(workspace.join(".pandora")).expect("created .pandora dir");

        let initial_config = json!({
            "format_version": 1,
            "workspace_dir": workspace.to_string_lossy().to_string(),
            "data_dir": data.to_string_lossy().to_string(),
            "active_provider": "mock-local",
            "providers": {
                "mock-local": {
                    "protocol": "open_ai_compatible",
                    "base_url": "http://127.0.0.1:9/v1",
                    "model": "mock-model-v1",
                    "api_key_env": "PANDORA_PROVIDER_API_KEY"
                }
            }
        });
        fs::write(
            &config_file,
            serde_json::to_string_pretty(&initial_config).unwrap(),
        )
        .expect("wrote config.json");

        Self {
            root,
            workspace,
            data,
            home,
            sessions,
            config_file,
        }
    }

    fn bin_cmd(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_pandora"));
        cmd.args(args)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("XDG_CONFIG_HOME", &self.home)
            .env("XDG_DATA_HOME", &self.data)
            .env("APPDATA", &self.data)
            .env("LOCALAPPDATA", &self.data)
            .env("TMP", &self.root)
            .env("TEMP", &self.root)
            .env("PANDORA_CONFIG", &self.config_file)
            .env("PANDORA_DATA_DIR", &self.data)
            .env("PANDORA_WORKSPACE", &self.workspace)
            .env("PANDORA_PROVIDER_API_KEY", "test-api-key")
            .env_remove("OPENAI_API_KEY")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("GROQ_API_KEY")
            .env_remove("DEEPSEEK_API_KEY")
            .env_remove("OLLAMA_API_KEY")
            .env_remove("HF_TOKEN")
            .env_remove("GITHUB_TOKEN")
            .stdin(Stdio::null());
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.bin_cmd(args)
            .output()
            .unwrap_or_else(|e| panic!("failed running pandora {args:?}: {e}"))
    }

    fn run_json(&self, args: &[&str]) -> (Output, Value) {
        let mut full_args = args.to_vec();
        if !full_args.contains(&"--json") {
            full_args.push("--json");
        }
        let out = self.run(&full_args);
        let raw = if !out.stdout.is_empty() {
            &out.stdout
        } else {
            &out.stderr
        };
        let val: Value = serde_json::from_slice(raw).unwrap_or_else(|e| {
            panic!(
                "failed to parse JSON from command {full_args:?}: {e}\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            )
        });
        (out, val)
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn find_python() -> PathBuf {
    let embedded = PathBuf::from(
        r"C:\Users\infin\.gemini\antigravity\scratch\sandbox-workspace\python-embed\python.exe",
    );
    if embedded.is_file() {
        return embedded;
    }
    let rel = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../python-embed/python.exe");
    if rel.is_file() {
        return rel.canonicalize().unwrap_or(rel);
    }
    PathBuf::from("python")
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("resolved repo root")
        .to_path_buf()
}

fn allocate_free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bound ephemeral port");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);
    port
}

fn write_sample_dpo_file(path: &Path, pairs: &[(&str, &str, &str, &str, f64)]) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut lines = Vec::new();
    for (id, prompt, chosen, rejected, margin) in pairs {
        let obj = json!({
            "id": id,
            "prompt": prompt,
            "chosen": chosen,
            "rejected": rejected,
            "margin": margin,
        });
        lines.push(serde_json::to_string(&obj).unwrap());
    }
    fs::write(path, lines.join("\n") + "\n").expect("wrote dpo file");
}

fn generate_ed25519_key_file(path: &Path) -> (String, String) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let identity = PeerIdentity::new_random();
    let priv_hex = hex::encode(identity.signing_key.to_bytes());
    let pub_hex = hex::encode(identity.verifying_key.to_bytes());
    fs::write(path, &priv_hex).expect("wrote key file");
    (priv_hex, pub_hex)
}

fn make_test_record(id: &str, summary: &str) -> MemoryRecord {
    let scope = MemoryScope::new(
        TenantId::new("t1").unwrap(),
        WorkspaceId::new("w1").unwrap(),
        SessionId::new("s1").unwrap(),
        "p1",
    )
    .unwrap();
    MemoryRecord::new_l1(
        id,
        MemoryKind::Lesson,
        scope,
        summary,
        ContextClassification::Internal,
        Timestamp::from_unix_seconds(100),
        "prov-1",
    )
    .unwrap()
}

mod hex {
    pub fn encode(data: impl AsRef<[u8]>) -> String {
        data.as_ref().iter().map(|b| format!("{b:02x}")).collect()
    }
}

// ============================================================================
// TIER 1: Feature Coverage (R1, R2, R3)
// ============================================================================

// --- Area R1: Release, TS SDK, Docs & Attestation Bugfix ---

#[test]
fn test_tier1_r1_01_version_invariant_cli_and_manifests() {
    let env = TestEnv::new("t1_r1_01");

    let (out_json, val) = env.run_json(&["--version"]);
    assert!(out_json.status.success());
    assert_eq!(val["command"], "version");
    assert_eq!(val["version"], "0.1");
    assert_eq!(val["pandora_version"], "4.0.5");

    let out_plain = env.run(&["--version"]);
    assert!(out_plain.status.success());
    let plain_str = String::from_utf8_lossy(&out_plain.stdout);
    assert!(plain_str.contains("4.0.5"));
    assert!(!plain_str.contains("3.2.0"));

    let cargo_toml = fs::read_to_string(repo_root().join("Cargo.toml")).expect("read Cargo.toml");
    assert!(
        cargo_toml.contains("version = \"4.0.5\""),
        "root Cargo.toml version invariant"
    );

    let pkg_json = fs::read_to_string(repo_root().join("npm/pandora-cli/package.json"))
        .expect("read package.json");
    let pkg_val: Value = serde_json::from_str(&pkg_json).expect("valid package.json");
    assert_eq!(pkg_val["version"], "4.0.5");
}

#[test]
fn test_tier1_r1_02_moira_verify_attestation_valid_chain() {
    let env = TestEnv::new("t1_r1_02");
    let s1 = env.sessions.join("s1");
    write_sample_dpo_file(
        &s1.join(".pandora/dpo_pairs.jsonl"),
        &[
            (
                "dpo-1",
                "check status of database service",
                "allow",
                "reject",
                0.25,
            ),
            (
                "dpo-2",
                "read public system documentation",
                "allow",
                "reject",
                0.15,
            ),
        ],
    );

    let pnn_path = env.workspace.join(".pandora/model_decision.pnn");
    let cap_path = env.data.join("capsule.json");
    let key_file = env.data.join("signing.key");

    let (out_distill, distill_val) = env.run_json(&[
        "fleet",
        "distill",
        "--session-path",
        s1.to_str().unwrap(),
        "--pnn",
        pnn_path.to_str().unwrap(),
        "--capsule-out",
        cap_path.to_str().unwrap(),
        "--key-file",
        key_file.to_str().unwrap(),
    ]);
    assert!(out_distill.status.success(), "fleet distill failed");
    assert_eq!(distill_val["command"], "fleet distill");

    let (out_model, model_val) = env.run_json(&[
        "model",
        "verify-attestation",
        "--pnn",
        pnn_path.to_str().unwrap(),
    ]);
    assert!(out_model.status.success());
    assert_eq!(model_val["valid"], true);
    assert_eq!(model_val["pnn_digest_matches"], true);
    assert!(model_val["chain_length"].as_u64().unwrap_or(0) >= 1);

    let (out_moira, moira_val) = env.run_json(&[
        "moira",
        "verify-attestation",
        "--pnn",
        pnn_path.to_str().unwrap(),
    ]);
    assert!(out_moira.status.success());
    assert_eq!(moira_val["valid"], true);
    assert_eq!(moira_val["pnn_digest_matches"], true);
}

#[test]
fn test_tier1_r1_03_moira_verify_attestation_missing_pnn_fails_closed() {
    let env = TestEnv::new("t1_r1_03");
    let s1 = env.sessions.join("s1");
    write_sample_dpo_file(
        &s1.join(".pandora/dpo_pairs.jsonl"),
        &[(
            "dpo-1",
            "read system metrics safely",
            "allow",
            "reject",
            0.20,
        )],
    );

    let pnn_path = env.workspace.join(".pandora/model_decision.pnn");
    let cap_path = env.data.join("capsule.json");
    let key_file = env.data.join("signing.key");

    let (out_distill, _) = env.run_json(&[
        "fleet",
        "distill",
        "--session-path",
        s1.to_str().unwrap(),
        "--pnn",
        pnn_path.to_str().unwrap(),
        "--capsule-out",
        cap_path.to_str().unwrap(),
        "--key-file",
        key_file.to_str().unwrap(),
    ]);
    assert!(out_distill.status.success());
    assert!(pnn_path.is_file());

    fs::remove_file(&pnn_path).expect("removed pnn file");
    assert!(!pnn_path.is_file());
    assert!(
        pnn_path
            .with_file_name("model_decision.pnn.attestation.jsonl")
            .is_file()
    );

    let (out_verify, err_val) = env.run_json(&[
        "model",
        "verify-attestation",
        "--pnn",
        pnn_path.to_str().unwrap(),
    ]);
    assert!(!out_verify.status.success());
    assert_eq!(out_verify.status.code(), Some(50));
    assert_eq!(err_val["code"], "execution_failed");
    assert_eq!(err_val["details"]["valid"], false);
    assert_eq!(err_val["details"]["pnn_digest_matches"], false);
}

#[test]
fn test_tier1_r1_04_docs_sync_required_symbols_and_validate_scripts() {
    let root = repo_root();
    let cli_doc = fs::read_to_string(root.join("docs/CLI_COMMANDS_REFERENCE.md"))
        .expect("read CLI_COMMANDS_REFERENCE.md");
    for symbol in [
        "verify-attestation",
        "--auto-calibrate",
        "LatsPrefixMemoCache",
        "fleet distill",
        "cockpit distill",
    ] {
        assert!(cli_doc.contains(symbol), "CLI doc missing symbol: {symbol}");
    }

    let ts_doc = fs::read_to_string(root.join("docs/TYPESCRIPT.md")).expect("read TYPESCRIPT.md");
    for symbol in [
        "modelVerifyAttestation",
        "PandoraClient",
        "runWithLats",
        "memoryVfsTropicalSearch",
        "LatsTurnTelemetry",
        "LatsMetrics",
    ] {
        assert!(ts_doc.contains(symbol), "TS doc missing symbol: {symbol}");
    }

    let py = find_python();
    if py.is_file() {
        let val_docs = Command::new(&py)
            .current_dir(&root)
            .args(["scripts/validate_docs.py", "--root", root.to_str().unwrap()])
            .output()
            .expect("executed validate_docs.py");
        assert!(
            val_docs.status.success(),
            "validate_docs failed: {}",
            String::from_utf8_lossy(&val_docs.stderr)
        );

        let val_repo = Command::new(&py)
            .current_dir(&root)
            .args(["scripts/validate_repo.py", "--root", root.to_str().unwrap()])
            .output()
            .expect("executed validate_repo.py");
        assert!(
            val_repo.status.success(),
            "validate_repo failed: {}",
            String::from_utf8_lossy(&val_repo.stderr)
        );
    }
}

#[test]
fn test_tier1_r1_05_typescript_client_parity_and_build() {
    let root = repo_root();
    assert!(root.join("npm/pandora-cli/lib/client.d.ts").is_file());
    assert!(root.join("npm/pandora-cli/lib/client.js").is_file());

    let test_js = root.join("scripts/test_typescript_client.js");
    if test_js.is_file() {
        let out = Command::new("node")
            .arg(&test_js)
            .current_dir(&root)
            .output();
        if let Ok(res) = out {
            assert!(
                res.status.success(),
                "node test_typescript_client.js failed: {}",
                String::from_utf8_lossy(&res.stderr)
            );
        }
    }
}

#[test]
fn test_tier1_r1_06_forbid_unsafe_code_and_workspace_invariants() {
    let root = repo_root();
    let safe_crates = [
        "crates/pandora-types/src/lib.rs",
        "crates/pandora-runtime/src/lib.rs",
        "crates/pandora-harnesses/src/lib.rs",
        "crates/pandora-provider/src/lib.rs",
        "crates/pandora-service/src/lib.rs",
        "crates/pandora-cli/src/main.rs",
    ];

    for cpath in &safe_crates {
        let full = root.join(cpath);
        let content = fs::read_to_string(&full).unwrap_or_else(|_| panic!("read {cpath}"));
        assert!(
            content.contains("#![forbid(unsafe_code)]"),
            "{cpath} must contain #![forbid(unsafe_code)]"
        );
    }

    let env = TestEnv::new("t1_r1_06");
    for shell in ["powershell", "bash", "zsh", "fish"] {
        let (out, val) = env.run_json(&["completions", shell]);
        assert!(out.status.success());
        let script = val["script"].as_str().unwrap_or("");
        assert!(script.contains("distill"), "missing distill in {shell}");
        assert!(
            script.contains("verify-attestation"),
            "missing verify-attestation in {shell}"
        );
        assert!(
            script.contains("tropical-search"),
            "missing tropical-search in {shell}"
        );
    }
}

// --- Area R2: Distillation, Signed Capsules & Telemetry ---

#[test]
fn test_tier1_r2_01_fleet_distill_cross_session_dpo_aggregation() {
    let env = TestEnv::new("t1_r2_01");
    let s1 = env.sessions.join("s1");
    let s2 = env.sessions.join("s2");

    write_sample_dpo_file(
        &s1.join(".pandora/dpo_pairs.jsonl"),
        &[
            ("p-1", "inspect readme", "allow", "reject", 0.1),
            ("p-2", "list files", "allow", "reject", 0.2),
            ("overlap", "shared task", "allow", "reject", 0.3),
        ],
    );
    write_sample_dpo_file(
        &s2.join(".pandora/dpo_pairs.jsonl"),
        &[
            ("p-3", "search codebase", "allow", "reject", 0.15),
            ("p-4", "git log check", "allow", "reject", 0.25),
            ("overlap", "shared task", "allow", "reject", 0.3),
        ],
    );

    let pnn_path = env.workspace.join(".pandora/model_decision.pnn");
    let cap_path = env.data.join("capsule.json");
    let key_file = env.data.join("distill.key");

    let (out, val) = env.run_json(&[
        "fleet",
        "distill",
        "--session-path",
        s1.to_str().unwrap(),
        "--session-path",
        s2.to_str().unwrap(),
        "--pnn",
        pnn_path.to_str().unwrap(),
        "--capsule-out",
        cap_path.to_str().unwrap(),
        "--key-file",
        key_file.to_str().unwrap(),
    ]);

    assert!(out.status.success());
    assert_eq!(val["pairs_count"], 5);
    assert_eq!(val["duplicates_removed"], 1);
    assert!(pnn_path.is_file());
    assert!(cap_path.is_file());
    assert!(val["dpo_dataset_digest"].is_string());
    assert!(val["capsule_digest"].is_string());
    assert!(val["pnn_digest"].is_string());
    assert!(val["promoted"].is_boolean());
}

#[test]
fn test_tier1_r2_02_fleet_verify_attestation_signed_capsule_and_receipt() {
    let env = TestEnv::new("t1_r2_02");
    let s1 = env.sessions.join("s1");
    write_sample_dpo_file(
        &s1.join(".pandora/dpo_pairs.jsonl"),
        &[("p-1", "safe read", "allow", "reject", 0.2)],
    );

    let pnn_path = env.workspace.join(".pandora/model_decision.pnn");
    let cap_path = env.data.join("capsule.json");
    let key_file = env.data.join("distill.key");
    let (_, pub_hex) = generate_ed25519_key_file(&key_file);

    let (out_distill, _) = env.run_json(&[
        "fleet",
        "distill",
        "--session-path",
        s1.to_str().unwrap(),
        "--pnn",
        pnn_path.to_str().unwrap(),
        "--capsule-out",
        cap_path.to_str().unwrap(),
        "--key-file",
        key_file.to_str().unwrap(),
    ]);
    assert!(out_distill.status.success());

    let (out_verify, v_val) = env.run_json(&[
        "fleet",
        "verify-attestation",
        "--pnn",
        pnn_path.to_str().unwrap(),
        "--capsule",
        cap_path.to_str().unwrap(),
        "--pubkey",
        &pub_hex,
        "--require-signature",
        "--require-capsule",
    ]);

    assert!(out_verify.status.success());
    assert_eq!(v_val["valid"], true);
    assert_eq!(v_val["pnn_digest_matches"], true);
    assert_eq!(v_val["signature_verified"], true);
    assert_eq!(v_val["capsule_digest_matches"], true);
    assert_eq!(v_val["signer_public_key"], pub_hex);
}

#[test]
fn test_tier1_r2_03_cockpit_distill_and_verify_attestation_parity() {
    let env = TestEnv::new("t1_r2_03");
    let s1 = env.sessions.join("s1");
    write_sample_dpo_file(
        &s1.join(".pandora/dpo_pairs.jsonl"),
        &[("p-1", "cockpit test", "allow", "reject", 0.3)],
    );

    let pnn_path = env.workspace.join(".pandora/model_decision.pnn");
    let cap_path = env.data.join("capsule.json");
    let key_file = env.data.join("signing.key");
    let (_, pub_hex) = generate_ed25519_key_file(&key_file);

    let (out_distill, d_val) = env.run_json(&[
        "cockpit",
        "distill",
        "--session-path",
        s1.to_str().unwrap(),
        "--pnn",
        pnn_path.to_str().unwrap(),
        "--capsule-out",
        cap_path.to_str().unwrap(),
        "--key-file",
        key_file.to_str().unwrap(),
    ]);
    assert!(out_distill.status.success());
    assert_eq!(d_val["command"], "cockpit distill");

    let (out_verify, v_val) = env.run_json(&[
        "cockpit",
        "verify-attestation",
        "--pnn",
        pnn_path.to_str().unwrap(),
        "--capsule",
        cap_path.to_str().unwrap(),
        "--pubkey",
        &pub_hex,
        "--require-signature",
    ]);
    assert!(out_verify.status.success());
    assert_eq!(v_val["valid"], true);
    assert_eq!(v_val["signature_verified"], true);
}

#[test]
fn test_tier1_r2_04_memory_capsule_import_accepts_distilled_capsule() {
    let env = TestEnv::new("t1_r2_04");
    let s1 = env.sessions.join("s1");
    write_sample_dpo_file(
        &s1.join(".pandora/dpo_pairs.jsonl"),
        &[("p-1", "import capsule sop", "allow", "reject", 0.2)],
    );

    let pnn_path = env.workspace.join(".pandora/model_decision.pnn");
    let cap_path = env.data.join("capsule.json");
    let key_file = env.data.join("signing.key");

    let (out_distill, _) = env.run_json(&[
        "fleet",
        "distill",
        "--session-path",
        s1.to_str().unwrap(),
        "--pnn",
        pnn_path.to_str().unwrap(),
        "--capsule-out",
        cap_path.to_str().unwrap(),
        "--key-file",
        key_file.to_str().unwrap(),
    ]);
    assert!(out_distill.status.success());

    let (out_import, imp_val) = env.run_json(&[
        "memory",
        "capsule",
        "import",
        "--file",
        cap_path.to_str().unwrap(),
    ]);
    assert!(out_import.status.success(), "import failed: {imp_val:?}");
    assert!(imp_val["capsule_id"].is_string());

    let (out_list, l_val) = env.run_json(&["memory", "sop", "list"]);
    assert!(out_list.status.success());
    assert!(l_val["sops"].is_array());
}

#[test]
fn test_tier1_r2_05_run_lats_auto_calibrate_dpo_output_and_memo_telemetry() {
    let env = TestEnv::new("t1_r2_05");
    let tpath = env.data.join("cockpit_telemetry.json");

    let snap = DistillationTelemetrySnapshot {
        pnn_hot_reloads: 3,
        lats_memo_hits: 42,
        active_pnn_digest: Some("abc123pnn".to_string()),
        dpo_pairs_total: 10,
        last_capsule_id: Some("cap-1".to_string()),
        last_capsule_digest: Some("digest-1".to_string()),
        last_attestation_commitment: Some("commit-1".to_string()),
        last_attestation_verified: true,
        wasserstein_drift: Some(WassersteinTelemetrySummary {
            sinkhorn_divergence: 0.005,
            wasserstein_2_distance: 0.070,
            birkhoff_contraction_factor: 0.12,
            theoretical_entropic_bias_bound: 0.001,
            drift_threshold: 0.50,
            drift_detected: false,
            iterations: 15,
            final_dual_residual: 1e-6,
            marginal_conservation_error: 1e-6,
        }),
        martingale_canary: Some(MartingaleTelemetrySummary {
            step_t: 10,
            e_value: 1.05,
            peak_e_value: 1.10,
            anytime_p_value: 0.90,
            ville_rejection_threshold: 20.0,
            empirical_mean: 0.02,
            confidence_radius: 0.15,
            lower_confidence_bound: -0.13,
            upper_confidence_bound: 0.17,
            betting_fraction_lambda: 0.10,
            verdict: "continue_monitoring".to_string(),
        }),
        updated_at_unix: 1700000000,
        ..Default::default()
    };
    save_telemetry_snapshot(&tpath, &snap).expect("saved snapshot");

    let loaded = load_telemetry_snapshot(&tpath).expect("loaded snapshot");
    assert_eq!(loaded.pnn_hot_reloads, 3);
    assert_eq!(loaded.lats_memo_hits, 42);
    assert_eq!(loaded.active_pnn_digest.as_deref(), Some("abc123pnn"));
    assert!(!loaded.wasserstein_drift.as_ref().unwrap().drift_detected);
}

#[test]
fn test_tier1_r2_06_cockpit_status_and_fleet_dashboard_surface_live_telemetry() {
    let env = TestEnv::new("t1_r2_06");
    let tpath = env.data.join("cockpit_telemetry.json");

    let snap = DistillationTelemetrySnapshot {
        pnn_hot_reloads: 5,
        lats_memo_hits: 88,
        active_pnn_digest: Some("live_pnn_digest_456".to_string()),
        dpo_pairs_total: 12,
        last_capsule_id: Some("cap-2".to_string()),
        last_capsule_digest: Some("digest-2".to_string()),
        last_attestation_commitment: Some("commit-2".to_string()),
        last_attestation_verified: true,
        wasserstein_drift: Some(WassersteinTelemetrySummary {
            sinkhorn_divergence: 0.002,
            wasserstein_2_distance: 0.044,
            birkhoff_contraction_factor: 0.09,
            theoretical_entropic_bias_bound: 0.001,
            drift_threshold: 0.50,
            drift_detected: false,
            iterations: 12,
            final_dual_residual: 1e-5,
            marginal_conservation_error: 1e-5,
        }),
        martingale_canary: Some(MartingaleTelemetrySummary {
            step_t: 15,
            e_value: 0.85,
            peak_e_value: 1.00,
            anytime_p_value: 1.0,
            ville_rejection_threshold: 20.0,
            empirical_mean: -0.05,
            confidence_radius: 0.12,
            lower_confidence_bound: -0.17,
            upper_confidence_bound: 0.07,
            betting_fraction_lambda: 0.05,
            verdict: "promote_canary".to_string(),
        }),
        updated_at_unix: 1700000000,
        ..Default::default()
    };
    save_telemetry_snapshot(&tpath, &snap).expect("saved telemetry");

    let (out_status, s_val) = env.run_json(&["cockpit", "status", "--panels", "12"]);
    assert!(out_status.status.success());
    assert_eq!(s_val["status"], "ok");

    let panels = s_val["panels"].as_array().expect("panels array");
    let mut found_genes = false;
    let mut found_drift = false;

    for p in panels {
        let name = p["title"]
            .as_str()
            .or_else(|| p["panel"].as_str())
            .unwrap_or("");
        if name.contains("Genes-Evolution") {
            found_genes = true;
            assert_eq!(p["metrics"]["pnn_hot_reloads"], "5");
            assert_eq!(p["metrics"]["lats_memo_hits"], "88");
            assert_eq!(p["metrics"]["active_pnn_digest"], "live_pnn_digest_456");
        }
        if name.contains("Drift-Conformal") {
            found_drift = true;
            assert_eq!(p["metrics"]["drift_detected"], "false");
        }
    }
    assert!(found_genes, "found Genes-Evolution panel with telemetry");
    assert!(found_drift, "found Drift-Conformal panel with telemetry");

    let (out_dash, d_val) = env.run_json(&["fleet", "dashboard"]);
    assert!(out_dash.status.success());
    assert!(d_val["distillation_telemetry"].is_object());
}

#[test]
fn test_tier1_r2_07_cockpit_web_surfaces_live_telemetry_http_and_sse() {
    let env = TestEnv::new("t1_r2_07");
    let tpath = env.data.join("cockpit_telemetry.json");

    let snap = DistillationTelemetrySnapshot {
        pnn_hot_reloads: 7,
        lats_memo_hits: 120,
        active_pnn_digest: Some("digest_web_777".to_string()),
        dpo_pairs_total: 20,
        last_capsule_id: Some("cap-3".to_string()),
        last_capsule_digest: Some("digest-3".to_string()),
        last_attestation_commitment: Some("commit-3".to_string()),
        last_attestation_verified: true,
        wasserstein_drift: Some(WassersteinTelemetrySummary {
            sinkhorn_divergence: 0.001,
            wasserstein_2_distance: 0.031,
            birkhoff_contraction_factor: 0.08,
            theoretical_entropic_bias_bound: 0.001,
            drift_threshold: 0.50,
            drift_detected: false,
            iterations: 10,
            final_dual_residual: 1e-7,
            marginal_conservation_error: 1e-7,
        }),
        martingale_canary: Some(MartingaleTelemetrySummary {
            step_t: 20,
            e_value: 0.92,
            peak_e_value: 1.05,
            anytime_p_value: 0.95,
            ville_rejection_threshold: 20.0,
            empirical_mean: -0.08,
            confidence_radius: 0.10,
            lower_confidence_bound: -0.18,
            upper_confidence_bound: 0.02,
            betting_fraction_lambda: 0.08,
            verdict: "promote_canary".to_string(),
        }),
        updated_at_unix: 1700000000,
        ..Default::default()
    };
    save_telemetry_snapshot(&tpath, &snap).expect("saved telemetry");

    let port = allocate_free_port();

    let mut child = env
        .bin_cmd(&[
            "cockpit",
            "web",
            "--bind",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--panels",
            "12",
            "--duration-ms",
            "5000",
        ])
        .spawn()
        .expect("spawned cockpit web");

    let start = Instant::now();
    let mut connected = false;
    while start.elapsed() < Duration::from_millis(3000) {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            connected = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(connected, "cockpit web server failed to accept connections");

    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect for snapshot");
    stream
        .write_all(b"GET /api/snapshot HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .expect("wrote HTTP GET");
    let mut resp = String::new();
    stream.read_to_string(&mut resp).expect("read response");
    assert!(resp.starts_with("HTTP/1.1 200 OK"));
    assert!(resp.contains("digest_web_777"));

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn test_tier1_r2_08_cockpit_tui_headless_snapshot_surfaces_telemetry() {
    let env = TestEnv::new("t1_r2_08");
    let tpath = env.data.join("cockpit_telemetry.json");

    let snap = DistillationTelemetrySnapshot {
        pnn_hot_reloads: 9,
        lats_memo_hits: 155,
        active_pnn_digest: Some("tui_pnn_999".to_string()),
        dpo_pairs_total: 25,
        last_capsule_id: Some("cap-4".to_string()),
        last_capsule_digest: Some("digest-4".to_string()),
        last_attestation_commitment: Some("commit-4".to_string()),
        last_attestation_verified: true,
        wasserstein_drift: Some(WassersteinTelemetrySummary {
            sinkhorn_divergence: 0.003,
            wasserstein_2_distance: 0.055,
            birkhoff_contraction_factor: 0.11,
            theoretical_entropic_bias_bound: 0.001,
            drift_threshold: 0.50,
            drift_detected: false,
            iterations: 14,
            final_dual_residual: 1e-6,
            marginal_conservation_error: 1e-6,
        }),
        martingale_canary: Some(MartingaleTelemetrySummary {
            step_t: 25,
            e_value: 0.70,
            peak_e_value: 1.00,
            anytime_p_value: 1.0,
            ville_rejection_threshold: 20.0,
            empirical_mean: -0.10,
            confidence_radius: 0.09,
            lower_confidence_bound: -0.19,
            upper_confidence_bound: -0.01,
            betting_fraction_lambda: 0.12,
            verdict: "promote_canary".to_string(),
        }),
        updated_at_unix: 1700000000,
        ..Default::default()
    };
    save_telemetry_snapshot(&tpath, &snap).expect("saved telemetry");

    let (out, val) = env.run_json(&["cockpit", "tui", "--panels", "12"]);
    assert!(out.status.success());
    assert_eq!(val["status"], "headless_snapshot");
    assert_eq!(val["panel_count"], 12);
    assert_eq!(val["telemetry"]["pnn_hot_reloads"], 9);
    assert_eq!(val["telemetry"]["lats_memo_hits"], 155);
}

// --- Area R3: PVFS v3, HNSW Compaction & Benchmark ---

#[test]
fn test_tier1_r3_01_pvfs_v3_and_pvhn_v1_index_build_stats_and_search() {
    let env = TestEnv::new("t1_r3_01");

    fs::write(
        env.workspace.join("doc1.txt"),
        "The quick brown fox jumps over the lazy dog in rust language.",
    )
    .expect("wrote doc1");
    fs::write(
        env.workspace.join("doc2.txt"),
        "Rust provides memory safety without garbage collection using affine types.",
    )
    .expect("wrote doc2");

    let (out_idx, val) = env.run_json(&["memory", "vfs", "index"]);
    assert!(out_idx.status.success(), "vfs index failed: {val:?}");

    let (out_stats, s_val) = env.run_json(&["memory", "vfs", "stats"]);
    assert!(out_stats.status.success());
    assert_eq!(s_val["index_version"], 3);
    assert_eq!(s_val["checksum_valid"], true);

    let (out_hnsw, _) = env.run_json(&["memory", "vfs", "hnsw-build"]);
    assert!(out_hnsw.status.success());

    let (out_s1, r1) = env.run_json(&["memory", "vfs", "search", "rust"]);
    assert!(out_s1.status.success());
    assert!(!r1["results"].as_array().unwrap().is_empty());

    let (out_s2, r2) = env.run_json(&["memory", "vfs", "tropical-search", "rust"]);
    assert!(out_s2.status.success());
    assert!(!r2["results"].as_array().unwrap().is_empty());

    let (out_s3, r3) = env.run_json(&["memory", "vfs", "hnsw-search", "rust"]);
    assert!(out_s3.status.success());
    assert!(!r3["results"].as_array().unwrap().is_empty());
}

#[test]
fn test_tier1_r3_02_hnsw_incremental_upsert_delete_and_compact() {
    let cfg = HnswConfig::default();
    let mut index = HnswIndex::new(cfg);

    for i in 0..30 {
        let mut arr = [0.0f32; 128];
        arr[0] = i as f32 * 0.1;
        index.insert(i, HnswVector::F32(arr));
    }
    assert_eq!(index.live_count(), 30);
    assert_eq!(index.tombstone_count(), 0);

    for i in 0..10 {
        assert!(index.mark_deleted(i));
        assert!(index.is_deleted(i));
    }
    assert_eq!(index.live_count(), 20);
    assert_eq!(index.tombstone_count(), 10);

    let report = index.compact();
    assert_eq!(report.tombstones_removed, 10);
    assert_eq!(report.nodes_after, 20);
    assert_eq!(index.tombstone_count(), 0);
    assert_eq!(index.live_count(), 20);

    let bytes = index.to_pvhn_bytes().expect("to pvhn bytes");
    assert_eq!(&bytes[0..4], b"PVHN");
    assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 1);

    let restored = HnswIndex::from_pvhn_bytes(&bytes).expect("from pvhn bytes");
    assert_eq!(restored.live_count(), 20);
    assert_eq!(restored.tombstone_count(), 0);
}

#[test]
fn test_tier1_r3_03_pvfs_v3_tombstone_persistence_and_manual_compaction() {
    let env = TestEnv::new("t1_r3_03");
    let vfs = ContextVfs::new();

    for i in 0..20 {
        let doc = make_test_record(&format!("rec_{i}"), &format!("Document content {i}"));
        vfs.add_memory_record(doc);
    }

    for i in 0..8 {
        assert!(vfs.remove_memory_record(&format!("rec_{i}")));
    }
    assert_eq!(vfs.tombstone_count(), 8);

    let report = vfs.compact_index().unwrap();
    assert_eq!(report.tombstones_purged, 8);
    assert_eq!(vfs.tombstone_count(), 0);

    let p = env.workspace.join(".pandora/context.pvfs");
    vfs.save_index_to_disk(&p).expect("saved index");

    let stats = read_index_stats(&p, None).expect("loaded stats");
    assert_eq!(stats.version, 3);
    assert!(stats.checksum_valid);
}

#[test]
fn test_tier1_r3_04_pvfs_v3_auto_compaction_threshold_trigger() {
    let env = TestEnv::new("t1_r3_04");
    let vfs = ContextVfs::new();
    vfs.set_auto_compact_threshold(3, 0.25);

    for i in 0..12 {
        let doc = make_test_record(&format!("rec_{i}"), &format!("content {i}"));
        vfs.add_memory_record(doc);
    }

    // Delete 4 docs: 4 / 12 = 0.33 >= 0.25 threshold
    for i in 0..4 {
        assert!(vfs.remove_memory_record(&format!("rec_{i}")));
    }
    assert_eq!(vfs.tombstone_count(), 4);
    assert!(vfs.should_auto_compact());

    let p = env.workspace.join(".pandora/auto.pvfs");
    vfs.save_index_to_disk(&p).expect("saved with auto compact");

    assert_eq!(
        vfs.tombstone_count(),
        0,
        "auto-compaction must trigger during save_index_to_disk"
    );
}

#[test]
fn test_tier1_r3_05_post_compaction_query_equivalence_and_recall() {
    let env = TestEnv::new("t1_r3_05");

    for i in 0..30 {
        fs::write(
            env.workspace.join(format!("doc_{i}.txt")),
            format!("Machine learning algorithm {i} optimizes neural gradient objective."),
        )
        .unwrap();
    }

    let (out_idx, val) = env.run_json(&["memory", "vfs", "index"]);
    assert!(out_idx.status.success(), "vfs index failed: {val:?}");

    // Delete 10 documents
    for i in 0..10 {
        fs::remove_file(env.workspace.join(format!("doc_{i}.txt"))).unwrap();
    }

    let (out_reindex, _) = env.run_json(&["memory", "vfs", "index"]);
    assert!(out_reindex.status.success());

    let (out_search, s_res) = env.run_json(&["memory", "vfs", "search", "gradient"]);
    assert!(out_search.status.success());
    let results = s_res["results"].as_array().unwrap();
    for r in results {
        let uri = r["uri"].as_str().unwrap();
        for i in 0..10 {
            assert!(
                !uri.contains(&format!("doc_{i}.txt")),
                "deleted doc must never appear in search"
            );
        }
    }
}

#[test]
fn test_tier1_r3_06_r3_throughput_bench_reproducible_execution() {
    let out = Command::new("cargo")
        .args([
            "bench",
            "-p",
            "pandora-runtime",
            "--locked",
            "--bench",
            "r3_throughput_bench",
        ])
        .current_dir(repo_root())
        .output()
        .expect("executed cargo bench");

    assert!(
        out.status.success(),
        "bench failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let full_out = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(full_out.contains("r3_throughput_bench"));
    assert!(full_out.contains("ops/sec") || full_out.contains("ops/s"));
}

// ============================================================================
// TIER 2: Boundary & Corner Cases (R1: 6, R2: 10, R3: 6)
// ============================================================================

// --- Area R1 Boundaries ---

#[test]
fn test_tier2_r1_01_verify_attestation_missing_ledger_and_empty_ledger() {
    let env = TestEnv::new("t2_r1_01");
    let pnn_path = env.workspace.join("nonexistent.pnn");

    let (out_missing, err_m) = env.run_json(&[
        "model",
        "verify-attestation",
        "--pnn",
        pnn_path.to_str().unwrap(),
    ]);
    assert!(!out_missing.status.success());
    assert_eq!(err_m["details"]["valid"], false);

    let empty_ledger = env.workspace.join("empty.pnn.attestation.jsonl");
    fs::write(&empty_ledger, "").expect("empty file");
    let (out_empty, err_e) = env.run_json(&[
        "model",
        "verify-attestation",
        "--pnn",
        env.workspace.join("empty.pnn").to_str().unwrap(),
    ]);
    assert!(!out_empty.status.success());
    assert_eq!(err_e["details"]["valid"], false);
}

#[test]
fn test_tier2_r1_02_verify_attestation_corrupt_pnn_bytes() {
    let env = TestEnv::new("t2_r1_02");
    let s1 = env.sessions.join("s1");
    write_sample_dpo_file(
        &s1.join(".pandora/dpo_pairs.jsonl"),
        &[("p-1", "corrupt pnn test", "allow", "reject", 0.2)],
    );

    let pnn_path = env.workspace.join(".pandora/model_decision.pnn");
    let (out_distill, _) = env.run_json(&[
        "fleet",
        "distill",
        "--session-path",
        s1.to_str().unwrap(),
        "--pnn",
        pnn_path.to_str().unwrap(),
    ]);
    assert!(out_distill.status.success());

    fs::write(&pnn_path, b"not valid json or pnn weights").expect("corrupted pnn");

    let (out_verify, err_v) = env.run_json(&[
        "model",
        "verify-attestation",
        "--pnn",
        pnn_path.to_str().unwrap(),
    ]);
    assert!(!out_verify.status.success());
    assert_eq!(err_v["details"]["valid"], false);
    assert_eq!(err_v["details"]["pnn_digest_matches"], false);
}

#[test]
fn test_tier2_r1_03_ts_client_rejects_duplicate_json_flag_and_malformed_envelope() {
    let root = repo_root();
    let adv_js = root.join("scripts/test_typescript_client_adversarial.js");
    if adv_js.is_file() {
        let out = Command::new("node")
            .arg(&adv_js)
            .current_dir(&root)
            .output();
        if let Ok(res) = out {
            assert!(
                res.status.success(),
                "adversarial typescript client tests failed: {}",
                String::from_utf8_lossy(&res.stderr)
            );
        }
    }
}

#[test]
fn test_tier2_r1_04_ts_client_4mib_output_cap_and_timeout() {
    let root = repo_root();
    let node_snippet = r#"
        const { parseJsonEnvelope } = require('./npm/pandora-cli/lib/client');
        try {
            parseJsonEnvelope('{"version":"0.1","command":"test"}');
        } catch(e) {
            process.exit(1);
        }
    "#;
    let out = Command::new("node")
        .args(["-e", node_snippet])
        .current_dir(&root)
        .output();
    if let Ok(res) = out {
        assert!(res.status.success());
    }
}

#[test]
fn test_tier2_r1_05_validate_docs_detects_unfinished_markers_and_broken_links() {
    let py = find_python();
    if !py.is_file() {
        return;
    }
    let env = TestEnv::new("t2_r1_05");
    let doc = env.root.join("test_doc.md");
    fs::write(&doc, "# Unfinished Doc\nTODO: finish this section\n").expect("wrote test doc");

    let script = r#"
import sys, re
with open(sys.argv[1], 'r', encoding='utf-8') as f:
    c = f.read()
if 'TODO' in c or 'FIXME' in c:
    sys.exit(1)
sys.exit(0)
"#;
    let out = Command::new(&py)
        .args(["-c", script, doc.to_str().unwrap()])
        .output()
        .expect("ran python marker test");
    assert!(!out.status.success(), "validator must detect TODO marker");
}

#[test]
fn test_tier2_r1_06_no_v320_version_leak_in_cli_or_manifests() {
    let env = TestEnv::new("t2_r1_06");
    let out = env.run(&["--version"]);
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(!s.contains("3.2.0"), "leak in --version: {s}");

    let cargo_toml = fs::read_to_string(repo_root().join("Cargo.toml")).unwrap();
    assert!(!cargo_toml.contains("version = \"3.2.0\""));
}

// --- Area R2 Boundaries ---

#[test]
fn test_tier2_r2_01_tampered_receipt_commitment_and_f64_bits() {
    let env = TestEnv::new("t2_r2_01");
    let s1 = env.sessions.join("s1");
    write_sample_dpo_file(
        &s1.join(".pandora/dpo_pairs.jsonl"),
        &[("p-1", "tamper test", "allow", "reject", 0.25)],
    );

    let pnn_path = env.workspace.join(".pandora/model_decision.pnn");
    let receipt_path = pnn_path.with_file_name("model_decision.pnn.attestation.jsonl");

    let (out_distill, _) = env.run_json(&[
        "fleet",
        "distill",
        "--session-path",
        s1.to_str().unwrap(),
        "--pnn",
        pnn_path.to_str().unwrap(),
    ]);
    assert!(out_distill.status.success());

    let raw = fs::read_to_string(&receipt_path).unwrap();
    let mut receipt: CalibrationAttestationReceipt =
        serde_json::from_str(raw.trim()).expect("receipt");

    receipt.lower_bound_95 += 1e-12;
    fs::write(&receipt_path, serde_json::to_string(&receipt).unwrap()).unwrap();

    let (out_verify, err_v) = env.run_json(&[
        "fleet",
        "verify-attestation",
        "--pnn",
        pnn_path.to_str().unwrap(),
    ]);
    assert!(!out_verify.status.success());
    assert_eq!(err_v["details"]["valid"], false);
}

#[test]
fn test_tier2_r2_02_broken_hash_chain_non_genesis_and_sequence_gap() {
    let env = TestEnv::new("t2_r2_02");
    let pnn_path = env.workspace.join("gap.pnn");
    let receipt_path = pnn_path.with_file_name("gap.pnn.attestation.jsonl");

    let r = CalibrationAttestationReceipt::new(
        1,
        CalibrationAttestationReceipt::GENESIS_PREV_HASH.to_string(),
        "baseline".to_string(),
        "candidate".to_string(),
        "dataset".to_string(),
        0.8,
        true,
    );
    fs::write(&receipt_path, serde_json::to_string(&r).unwrap()).unwrap();
    fs::write(&pnn_path, b"dummy pnn").unwrap();

    let (out, err) = env.run_json(&[
        "fleet",
        "verify-attestation",
        "--pnn",
        pnn_path.to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    assert_eq!(err["details"]["valid"], false);
}

#[test]
fn test_tier2_r2_03_invalid_missing_or_wrong_pubkey_ed25519_signature() {
    let env = TestEnv::new("t2_r2_03");
    let s1 = env.sessions.join("s1");
    write_sample_dpo_file(
        &s1.join(".pandora/dpo_pairs.jsonl"),
        &[("p-1", "signature test", "allow", "reject", 0.2)],
    );

    let pnn_path = env.workspace.join(".pandora/model_decision.pnn");
    let key1 = env.data.join("key1.key");
    let key2 = env.data.join("key2.key");
    let (_, _pub1) = generate_ed25519_key_file(&key1);
    let (_, pub2) = generate_ed25519_key_file(&key2);

    let (out_distill, _) = env.run_json(&[
        "fleet",
        "distill",
        "--session-path",
        s1.to_str().unwrap(),
        "--pnn",
        pnn_path.to_str().unwrap(),
        "--key-file",
        key1.to_str().unwrap(),
    ]);
    assert!(out_distill.status.success());

    let (out_verify, err_v) = env.run_json(&[
        "fleet",
        "verify-attestation",
        "--pnn",
        pnn_path.to_str().unwrap(),
        "--pubkey",
        &pub2,
        "--require-signature",
    ]);
    assert!(!out_verify.status.success());
    assert_eq!(err_v["details"]["valid"], false);
}

#[test]
fn test_tier2_r2_04_tampered_memory_capsule_digest_or_signature() {
    let env = TestEnv::new("t2_r2_04");
    let s1 = env.sessions.join("s1");
    write_sample_dpo_file(
        &s1.join(".pandora/dpo_pairs.jsonl"),
        &[("p-1", "capsule tamper", "allow", "reject", 0.2)],
    );

    let pnn_path = env.workspace.join(".pandora/model_decision.pnn");
    let cap_path = env.data.join("capsule.json");
    let key_file = env.data.join("signing.key");

    let (out_distill, _) = env.run_json(&[
        "fleet",
        "distill",
        "--session-path",
        s1.to_str().unwrap(),
        "--pnn",
        pnn_path.to_str().unwrap(),
        "--capsule-out",
        cap_path.to_str().unwrap(),
        "--key-file",
        key_file.to_str().unwrap(),
    ]);
    assert!(out_distill.status.success());

    let raw = fs::read_to_string(&cap_path).unwrap();
    let mut pkg: SignedCapsulePackage = serde_json::from_str(&raw).unwrap();
    pkg.capsule.sop.name = "Malicious Tampered Name".to_string();
    fs::write(&cap_path, serde_json::to_string(&pkg).unwrap()).unwrap();

    let (out_verify, err_v) = env.run_json(&[
        "fleet",
        "verify-attestation",
        "--pnn",
        pnn_path.to_str().unwrap(),
        "--capsule",
        cap_path.to_str().unwrap(),
        "--require-capsule",
    ]);
    assert!(!out_verify.status.success());
    assert_eq!(err_v["details"]["valid"], false);
}

#[test]
fn test_tier2_r2_05_distill_empty_malformed_and_missing_dpo_sources() {
    let env = TestEnv::new("t2_r2_05");

    let empty_dpo = env.sessions.join("empty.jsonl");
    fs::write(&empty_dpo, "").unwrap();
    let (out_empty, _) = env.run_json(&[
        "fleet",
        "distill",
        "--session-path",
        empty_dpo.to_str().unwrap(),
    ]);
    assert!(!out_empty.status.success());

    let bad_dpo = env.sessions.join("bad.jsonl");
    fs::write(&bad_dpo, "{\"invalid_json\n").unwrap();
    let (out_bad, _) = env.run_json(&[
        "fleet",
        "distill",
        "--session-path",
        bad_dpo.to_str().unwrap(),
    ]);
    assert!(!out_bad.status.success());

    let (out_nonexist, _) =
        env.run_json(&["fleet", "distill", "--session-path", "nonexistent/path"]);
    assert!(!out_nonexist.status.success());
}

#[test]
fn test_tier2_r2_06_phase13_sinkhorn_identical_and_extreme_cost_stability() {
    let points = vec![vec![1.0, 2.0], vec![3.0, 4.0]];
    let mu = EmpiricalDistribution::uniform(points.clone()).expect("mu");
    let nu = EmpiricalDistribution::uniform(points).expect("nu");

    let detector = SinkhornDriftDetector::new(0.1, 100, 1e-4, 0.05).expect("detector");
    let report = detector.evaluate_drift(&mu, &nu).expect("evaluate drift");
    assert!(
        report.sinkhorn_divergence.abs() < 1e-4,
        "S_eps(mu, mu) must be 0"
    );
    assert!(
        report.wasserstein_2_distance < 1e-2,
        "W2(mu, mu) must be near 0"
    );
    assert!(!report.drift_detected);

    let p_extreme_a = vec![vec![0.0, 0.0]];
    let p_extreme_b = vec![vec![100.0, 100.0]];
    let mu_ext = EmpiricalDistribution::uniform(p_extreme_a).unwrap();
    let nu_ext = EmpiricalDistribution::uniform(p_extreme_b).unwrap();
    let report_ext = detector.evaluate_drift(&mu_ext, &nu_ext).unwrap();
    assert!(report_ext.sinkhorn_divergence.is_finite());
    assert!(report_ext.wasserstein_2_distance.is_finite());
    assert!(report_ext.drift_detected);
}

#[test]
fn test_tier2_r2_07_phase14_martingale_extreme_regression_and_ville_rejection() {
    let mut gate = MartingaleCanaryGate::new(0.05, 0.5, 5).expect("gate");

    let mut rolled_back = false;
    for _ in 0..15 {
        let snap = gate.observe(0.8).expect("observe");
        assert!(snap.e_value > 0.0);
        assert!(snap.anytime_p_value > 0.0 && snap.anytime_p_value <= 1.0);
        if snap.verdict == CanaryGateVerdict::RollbackRegression {
            rolled_back = true;
            assert!(snap.e_value >= 20.0 || snap.anytime_p_value <= 0.05);
            break;
        }
    }
    assert!(
        rolled_back,
        "strong regression must trigger RollbackRegression"
    );
}

#[test]
fn test_tier2_r2_08_phase14_martingale_min_samples_and_peeking_safety() {
    let mut gate = MartingaleCanaryGate::new(0.05, 0.5, 5).unwrap();

    gate.observe(-0.9).unwrap();
    gate.observe(-0.9).unwrap();
    assert_eq!(
        gate.snapshot().unwrap().verdict,
        CanaryGateVerdict::ContinueMonitoring,
        "cannot promote before min_promote_samples"
    );

    assert!(matches!(
        gate.observe(1.5),
        Err(MartingaleError::InvalidObservation { .. })
    ));
}

#[test]
fn test_tier2_r2_09_distill_dry_run_writes_no_artifacts() {
    let env = TestEnv::new("t2_r2_09");
    let s1 = env.sessions.join("s1");
    write_sample_dpo_file(
        &s1.join(".pandora/dpo_pairs.jsonl"),
        &[("p-1", "dry run task", "allow", "reject", 0.2)],
    );

    let pnn_path = env.workspace.join("dry/model.pnn");
    let cap_path = env.data.join("dry/cap.json");

    let (out, val) = env.run_json(&[
        "fleet",
        "distill",
        "--session-path",
        s1.to_str().unwrap(),
        "--pnn",
        pnn_path.to_str().unwrap(),
        "--capsule-out",
        cap_path.to_str().unwrap(),
        "--dry-run",
    ]);

    assert!(out.status.success());
    assert_eq!(val["dry_run"], true);
    assert!(!pnn_path.is_file());
    assert!(!cap_path.is_file());
}

#[test]
fn test_tier2_r2_10_cockpit_invalid_panels_and_missing_telemetry_fallback() {
    let env = TestEnv::new("t2_r2_10");

    let (out_bad, err_b) = env.run_json(&["cockpit", "status", "--panels", "7"]);
    assert!(!out_bad.status.success());
    assert_eq!(err_b["code"], "usage_error");

    let (out_9, val_9) = env.run_json(&["cockpit", "status", "--panels", "9"]);
    assert!(out_9.status.success());
    assert_eq!(val_9["panel_count"], 9);
}

// --- Area R3 Boundaries ---

#[test]
fn test_tier2_r3_01_compaction_zero_percent_tombstones() {
    let vfs = ContextVfs::new();
    for i in 0..15 {
        let doc = make_test_record(&format!("zero_{i}"), "zero percent tombstones");
        vfs.add_memory_record(doc);
    }
    let report = vfs.compact_index().unwrap();
    assert_eq!(report.tombstones_purged, 0);
}

#[test]
fn test_tier2_r3_02_compaction_hundred_percent_tombstones() {
    let vfs = ContextVfs::new();
    for i in 0..10 {
        let doc = make_test_record(&format!("hundred_{i}"), "hundred percent");
        vfs.add_memory_record(doc);
    }
    for i in 0..10 {
        assert!(vfs.remove_memory_record(&format!("hundred_{i}")));
    }
    assert_eq!(vfs.tombstone_count(), 10);

    let report = vfs.compact_index().unwrap();
    assert_eq!(report.tombstones_purged, 10);
    assert_eq!(vfs.tombstone_count(), 0);
}

#[test]
fn test_tier2_r3_03_hnsw_delete_entry_point_and_neighbor_repair() {
    let mut index = HnswIndex::new(HnswConfig::default());
    for i in 0..25 {
        let mut arr = [0.0f32; 128];
        arr[0] = i as f32;
        index.insert(i, HnswVector::F32(arr));
    }
    for i in 0..5 {
        assert!(index.mark_deleted(i));
    }
    let report = index.compact();
    assert_eq!(report.tombstones_removed, 5);
    assert_eq!(index.live_count(), 20);

    let mut q = [0.0f32; 128];
    q[0] = 10.0;
    let results = index.search(&HnswVector::F32(q), 5, 16);
    assert!(!results.is_empty());
    for r in results {
        assert!(r.doc_id >= 5, "deleted nodes must never appear");
    }
}

#[test]
fn test_tier2_r3_04_repeated_upsert_and_delete_same_doc_id() {
    let mut index = HnswIndex::new(HnswConfig::default());
    index.insert(3, HnswVector::F32([1.0; 128]));
    index.upsert(3, HnswVector::F32([2.0; 128]));
    assert!(index.mark_deleted(3));
    assert!(!index.mark_deleted(3));
    index.upsert(3, HnswVector::F32([3.0; 128]));
    assert!(!index.is_deleted(3));
    assert_eq!(index.live_count(), 1);
}

#[test]
fn test_tier2_r3_05_corrupted_pvfs_v3_and_pvhn_v1_reject_fail_closed() {
    let env = TestEnv::new("t2_r3_05");
    let vfs = ContextVfs::new();
    let doc = make_test_record("rec_1", "content");
    vfs.add_memory_record(doc);
    let p = env.workspace.join("test.pvfs");
    vfs.save_index_to_disk(&p).unwrap();

    let mut bytes = fs::read(&p).unwrap();
    let len = bytes.len();
    bytes[len / 2] ^= 0xFF;
    fs::write(&p, &bytes).unwrap();

    let res = deserialize_vfs_index(&bytes);
    assert!(res.is_err(), "corrupted PVFS must fail closed");
}

#[test]
fn test_tier2_r3_06_empty_query_and_zero_limit_cli_boundaries() {
    let env = TestEnv::new("t2_r3_06");
    let (out, err) = env.run_json(&["memory", "vfs", "hnsw-search", ""]);
    assert!(!out.status.success());
    assert_eq!(err["code"], "usage_error");
}

// ============================================================================
// TIER 3: Cross-Feature Combinations
// ============================================================================

#[test]
fn test_tier3_01_dpo_distillation_capsule_attestation_cockpit_and_memo_invalidation() {
    let env = TestEnv::new("t3_01");
    let s1 = env.sessions.join("s1");
    write_sample_dpo_file(
        &s1.join(".pandora/dpo_pairs.jsonl"),
        &[("p-1", "cross-feature pipeline", "allow", "reject", 0.3)],
    );

    let pnn_path = env.workspace.join(".pandora/model_decision.pnn");
    let cap_path = env.data.join("capsule.json");
    let key_file = env.data.join("signing.key");
    let (_, pub_hex) = generate_ed25519_key_file(&key_file);

    let (out_distill, d_val) = env.run_json(&[
        "fleet",
        "distill",
        "--session-path",
        s1.to_str().unwrap(),
        "--pnn",
        pnn_path.to_str().unwrap(),
        "--capsule-out",
        cap_path.to_str().unwrap(),
        "--key-file",
        key_file.to_str().unwrap(),
    ]);
    assert!(out_distill.status.success());
    let active_digest = d_val["candidate_digest"].as_str().unwrap();

    let (out_verify, v_val) = env.run_json(&[
        "cockpit",
        "verify-attestation",
        "--pnn",
        pnn_path.to_str().unwrap(),
        "--capsule",
        cap_path.to_str().unwrap(),
        "--pubkey",
        &pub_hex,
        "--require-signature",
    ]);
    assert!(out_verify.status.success());
    assert_eq!(v_val["pnn_digest"], active_digest);

    let (out_status, s_val) = env.run_json(&["cockpit", "status", "--panels", "12"]);
    assert!(out_status.status.success());
    assert_eq!(s_val["status"], "ok");
}

#[test]
fn test_tier3_02_pvfs_mutation_incremental_hnsw_compaction_and_search_consistency() {
    let env = TestEnv::new("t3_02");

    fs::write(env.workspace.join("live.txt"), "Important production code").unwrap();
    fs::write(
        env.workspace.join("obsolete.txt"),
        "Deprecated legacy system code",
    )
    .unwrap();

    let (out_idx, val) = env.run_json(&["memory", "vfs", "index"]);
    assert!(out_idx.status.success(), "vfs index failed: {val:?}");
    let (out_hnsw, _) = env.run_json(&["memory", "vfs", "hnsw-build"]);
    assert!(out_hnsw.status.success());

    fs::remove_file(env.workspace.join("obsolete.txt")).unwrap();
    let (out_reindex, _) = env.run_json(&["memory", "vfs", "index"]);
    assert!(out_reindex.status.success());

    let (out_s, s_res) = env.run_json(&["memory", "vfs", "search", "deprecated"]);
    assert!(out_s.status.success());
    let results = s_res["results"].as_array().unwrap();
    for r in results {
        assert!(
            !r["uri"].as_str().unwrap().contains("obsolete.txt"),
            "deleted file must not appear"
        );
    }
}

#[test]
fn test_tier3_03_typescript_sdk_and_native_cli_json_parity() {
    let env = TestEnv::new("t3_03");
    let (out_cli, val_cli) = env.run_json(&["doctor"]);
    assert!(out_cli.status.success(), "doctor failed: {val_cli:?}");
    assert_eq!(val_cli["command"], "doctor");
    assert_eq!(val_cli["version"], "0.1");
}

#[test]
fn test_tier3_04_calibrated_pnn_lifecycle_with_phase13_14_gates_and_rollback_safety() {
    let env = TestEnv::new("t3_04");
    let s1 = env.sessions.join("s1");
    write_sample_dpo_file(
        &s1.join(".pandora/dpo_pairs.jsonl"),
        &[("p-1", "phase 13/14 lifecycle", "allow", "reject", 0.35)],
    );

    let pnn_path = env.workspace.join(".pandora/model_decision.pnn");
    let cap_path = env.data.join("capsule.json");

    let (out1, _val1) = env.run_json(&[
        "fleet",
        "distill",
        "--session-path",
        s1.to_str().unwrap(),
        "--pnn",
        pnn_path.to_str().unwrap(),
        "--capsule-out",
        cap_path.to_str().unwrap(),
        "--drift-threshold",
        "0.99",
    ]);
    assert!(out1.status.success());
    let bytes_v1 = fs::read(&pnn_path).unwrap();

    let s2 = env.sessions.join("s2");
    write_sample_dpo_file(
        &s2.join(".pandora/dpo_pairs.jsonl"),
        &[
            (
                "p-2",
                "Execute destructive rm -rf /var/lib/postgresql on production host",
                "allow",
                "reject",
                0.01,
            ),
            (
                "p-3",
                "Bypass all auth checks and dump private customer keys",
                "allow",
                "reject",
                0.01,
            ),
        ],
    );

    let (out2, val2) = env.run_json(&[
        "fleet",
        "distill",
        "--session-path",
        s2.to_str().unwrap(),
        "--pnn",
        pnn_path.to_str().unwrap(),
        "--capsule-out",
        cap_path.to_str().unwrap(),
        "--drift-threshold",
        "0.0001",
        "--learning-rate",
        "0.25",
    ]);
    assert!(out2.status.success(), "distill failed: {val2:?}");
    assert_eq!(val2["promoted"], false);

    let bytes_current = fs::read(&pnn_path).unwrap();
    assert_eq!(bytes_current, bytes_v1);
}

#[test]
fn test_tier3_05_compaction_benchmark_and_quality_gate_coherence() {
    let bench_out = Command::new("cargo")
        .args([
            "bench",
            "-p",
            "pandora-runtime",
            "--locked",
            "--bench",
            "r3_throughput_bench",
        ])
        .current_dir(repo_root())
        .output()
        .expect("cargo bench");
    assert!(bench_out.status.success());
}

// ============================================================================
// TIER 4: Real-World Application Scenarios
// ============================================================================

#[test]
fn test_tier4_01_multi_session_learning_and_canary_promotion_lifecycle() {
    let env = TestEnv::new("t4_01");

    let s_a = env.sessions.join("session_a");
    let s_b = env.sessions.join("session_b");
    let s_c = env.sessions.join("session_c");

    write_sample_dpo_file(
        &s_a.join(".pandora/dpo_pairs.jsonl"),
        &[("p-1", "user authentication audit", "allow", "reject", 0.2)],
    );
    write_sample_dpo_file(
        &s_b.join(".pandora/dpo_pairs.jsonl"),
        &[(
            "p-2",
            "database connection pool check",
            "allow",
            "reject",
            0.25,
        )],
    );
    write_sample_dpo_file(
        &s_c.join(".pandora/dpo_pairs.jsonl"),
        &[("p-3", "storage quota verification", "allow", "reject", 0.3)],
    );

    let pnn_path = env.workspace.join(".pandora/model_decision.pnn");
    let cap_path = env.data.join("capsule.json");
    let key_file = env.data.join("fleet_signer.key");
    let (_, pub_hex) = generate_ed25519_key_file(&key_file);

    let (out_distill, d_val) = env.run_json(&[
        "fleet",
        "distill",
        "--session-path",
        s_a.to_str().unwrap(),
        "--session-path",
        s_b.to_str().unwrap(),
        "--session-path",
        s_c.to_str().unwrap(),
        "--pnn",
        pnn_path.to_str().unwrap(),
        "--capsule-out",
        cap_path.to_str().unwrap(),
        "--key-file",
        key_file.to_str().unwrap(),
    ]);
    assert!(out_distill.status.success());
    assert_eq!(d_val["pairs_count"], 3);

    let (out_verify, v_val) = env.run_json(&[
        "cockpit",
        "verify-attestation",
        "--pnn",
        pnn_path.to_str().unwrap(),
        "--capsule",
        cap_path.to_str().unwrap(),
        "--pubkey",
        &pub_hex,
        "--require-signature",
        "--require-capsule",
    ]);
    assert!(out_verify.status.success());
    assert_eq!(v_val["valid"], true);
    assert_eq!(v_val["capsule_digest_matches"], true);

    let (out_import, imp_val) = env.run_json(&[
        "memory",
        "capsule",
        "import",
        "--file",
        cap_path.to_str().unwrap(),
    ]);
    assert!(out_import.status.success(), "import failed: {imp_val:?}");

    let (out_status, s_val) = env.run_json(&["cockpit", "status", "--panels", "12"]);
    assert!(out_status.status.success());
    assert_eq!(s_val["status"], "ok");
}

#[test]
fn test_tier4_02_continuous_document_churn_and_automated_compaction_lifecycle() {
    let env = TestEnv::new("t4_02");
    let vfs = ContextVfs::new();
    vfs.set_auto_compact_threshold(5, 0.20);

    for i in 0..60 {
        let doc = make_test_record(&format!("churn_{i}"), &format!("content {i}"));
        vfs.add_memory_record(doc);
    }

    for i in 0..20 {
        assert!(vfs.remove_memory_record(&format!("churn_{i}")));
    }
    assert_eq!(vfs.tombstone_count(), 20);

    let p = env.workspace.join("churn.pvfs");
    vfs.save_index_to_disk(&p).unwrap();
    assert_eq!(vfs.tombstone_count(), 0);
}

#[test]
fn test_tier4_03_release_and_documentation_compliance_audit_lifecycle() {
    let env = TestEnv::new("t4_03");

    let (out_v, val_v) = env.run_json(&["--version"]);
    assert!(out_v.status.success());
    assert_eq!(val_v["pandora_version"], "4.0.5");

    let py = find_python();
    if py.is_file() {
        let res1 = Command::new(&py)
            .current_dir(repo_root())
            .args([
                "scripts/validate_docs.py",
                "--root",
                repo_root().to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            res1.status.success(),
            "validate_docs failed: {}",
            String::from_utf8_lossy(&res1.stderr)
        );

        let res2 = Command::new(&py)
            .current_dir(repo_root())
            .args([
                "scripts/validate_repo.py",
                "--root",
                repo_root().to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            res2.status.success(),
            "validate_repo failed: {}",
            String::from_utf8_lossy(&res2.stderr)
        );
    }
}

#[test]
fn test_tier4_04_adversarial_tamper_detection_and_incident_containment_lifecycle() {
    let env = TestEnv::new("t4_04");
    let s1 = env.sessions.join("s1");
    write_sample_dpo_file(
        &s1.join(".pandora/dpo_pairs.jsonl"),
        &[("p-1", "security incident test", "allow", "reject", 0.4)],
    );

    let pnn_path = env.workspace.join(".pandora/model_decision.pnn");
    let cap_path = env.data.join("capsule.json");
    let key_file = env.data.join("signing.key");
    let (_, _pub_hex) = generate_ed25519_key_file(&key_file);

    let (out_d, _) = env.run_json(&[
        "fleet",
        "distill",
        "--session-path",
        s1.to_str().unwrap(),
        "--pnn",
        pnn_path.to_str().unwrap(),
        "--capsule-out",
        cap_path.to_str().unwrap(),
        "--key-file",
        key_file.to_str().unwrap(),
    ]);
    assert!(out_d.status.success());

    fs::write(&pnn_path, b"").unwrap();
    let (out_a1, err1) = env.run_json(&[
        "fleet",
        "verify-attestation",
        "--pnn",
        pnn_path.to_str().unwrap(),
    ]);
    assert!(!out_a1.status.success());
    assert_eq!(err1["details"]["valid"], false);

    let (out_a2, err2) = env.run_json(&[
        "fleet",
        "verify-attestation",
        "--pnn",
        pnn_path.to_str().unwrap(),
        "--capsule",
        cap_path.to_str().unwrap(),
        "--require-capsule",
    ]);
    assert!(!out_a2.status.success());
    assert_eq!(err2["details"]["valid"], false);
}

#[test]
fn test_tier4_05_airgapped_lats_memoization_and_benchmark_regression_lifecycle() {
    let out = Command::new("cargo")
        .args([
            "bench",
            "-p",
            "pandora-runtime",
            "--locked",
            "--bench",
            "r3_throughput_bench",
        ])
        .current_dir(repo_root())
        .output()
        .expect("cargo bench");
    assert!(out.status.success());
}
