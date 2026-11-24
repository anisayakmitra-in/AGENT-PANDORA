//! Phase 25 & 26 E2E Integration Suite:
//! Zero-Knowledge Execution Attestation, Merkle Rollup, Google Colab Remote GPU Bridge,
//! Cockpit Telemetry Surfacing, and 12-Project Inference Infrastructure (Theorems 48-55).
//!
//! # Invariants
//! - Strictly `#![forbid(unsafe_code)]` workspace-wide.
//! - Non-flaky ephemeral sockets & temporary file sandboxes.
//! - Deterministic polynomial verifications and pushdown grammar parsing.

#![forbid(unsafe_code)]

use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(2500);

struct TestFixture {
    root: PathBuf,
    config: PathBuf,
    data: PathBuf,
    workspace: PathBuf,
}

impl TestFixture {
    fn new(prefix: &str) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock valid")
            .as_nanos();
        let seq = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let temp_root = std::env::temp_dir()
            .canonicalize()
            .expect("temp dir canonical path");
        let root = temp_root.join(format!(
            "pandora-{prefix}-{}-{timestamp}-{seq}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        let data = root.join("data");
        fs::create_dir_all(&workspace).expect("workspace dir created");
        fs::create_dir_all(&data).expect("data dir created");
        Self {
            config: root.join("config.json"),
            data,
            workspace,
            root,
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_pandora"));
        cmd.args(args)
            .env("PANDORA_CONFIG", &self.config)
            .env("PANDORA_DATA_DIR", &self.data)
            .env("PANDORA_WORKSPACE", &self.workspace);
        cmd
    }

    fn run_success(&self, args: &[&str]) -> Value {
        let output = self.command(args).output().expect("command executed");
        assert!(
            output.status.success(),
            "command failed: {:?}\nstderr: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).expect("stdout valid UTF-8");
        serde_json::from_str(&stdout).expect("stdout valid JSON")
    }
}

impl Drop for TestFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn test_e2e_phase25_zk_execution_attestation_and_polynomial_soundness() {
    let fixture = TestFixture::new("phase25-zk");
    let proof_file = fixture.root.join("attestation_proof.json");

    // 1. Generate Schwartz-Zippel ZK execution attestation proof (Theorem 48)
    let prove_res = fixture.run_success(&[
        "zk",
        "prove",
        "--patch-cert",
        "PANDORA_PATCH_CERT_V1:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "--holdout-count",
        "8",
        "--tests-passed",
        "8",
        "--invariants-verified",
        "true",
        "--out",
        proof_file.to_str().unwrap(),
        "--json",
    ]);

    assert_eq!(prove_res["mode"], "execution_attestation");
    assert_eq!(prove_res["verified"], true);
    assert_eq!(prove_res["memory_safety_verified"], true);
    assert_eq!(prove_res["holdout_test_count"], 8);
    assert_eq!(prove_res["holdout_tests_passed"], 8);
    assert!(proof_file.exists());

    // 2. Verify proof with polymorphic verifier
    let verify_res = fixture.run_success(&[
        "zk",
        "verify",
        "--proof-file",
        proof_file.to_str().unwrap(),
        "--json",
    ]);

    assert_eq!(verify_res["type"], "execution_attestation");
    assert_eq!(verify_res["valid"], true);
    assert_eq!(verify_res["memory_safety_verified"], true);
}

#[test]
fn test_e2e_phase25_zk_merkle_rollup_batch_and_membership_verification() {
    let fixture = TestFixture::new("phase25-rollup");
    let epoch_file = fixture.root.join("epoch_batch.json");
    let inclusion_file = fixture.root.join("leaf_inclusion.json");

    // 1. Batch rollup 8 decisions
    let batch_res = fixture.run_success(&[
        "zk",
        "rollup",
        "batch",
        "--epoch",
        "1",
        "--mock-count",
        "8",
        "--out",
        epoch_file.to_str().unwrap(),
        "--json",
    ]);

    assert_eq!(batch_res["decision_count"], 8);
    assert_eq!(batch_res["tree_height"], 3);
    assert!(batch_res["epoch_root_hex"].is_string());
    assert!(epoch_file.exists());

    // 2. Generate inclusion proof for leaf at index 3
    let inclusion_res = fixture.run_success(&[
        "zk",
        "rollup",
        "prove-inclusion",
        "--epoch-file",
        epoch_file.to_str().unwrap(),
        "--index",
        "3",
        "--out",
        inclusion_file.to_str().unwrap(),
        "--json",
    ]);

    assert_eq!(inclusion_res["leaf_index"], 3);
    assert_eq!(inclusion_res["self_verified"], true);
    assert_eq!(inclusion_res["proof_steps"], 3);
    assert!(inclusion_file.exists());

    // 3. Verify inclusion proof against Merkle root
    let verify_inc_res = fixture.run_success(&[
        "zk",
        "rollup",
        "verify-inclusion",
        "--proof-file",
        inclusion_file.to_str().unwrap(),
        "--json",
    ]);

    assert_eq!(verify_inc_res["valid"], true);
    assert_eq!(verify_inc_res["leaf_index"], 3);
}

#[test]
fn test_e2e_phase26_colab_remote_gpu_bridge_handshake_and_vram_telemetry() {
    let fixture = TestFixture::new("phase26-colab");
    let session_file = fixture.root.join("colab_session.json");
    let token = "test-secret-colab-bridge-token-xyz";

    let listener = TcpListener::bind("127.0.0.1:0").expect("binds ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let endpoint = format!("http://{addr}");

    // Mock HTTP server responding to Colab handshake & status queries
    let server_handle = thread::spawn(move || {
        for _ in 0..3 {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buffer = [0u8; 2048];
                let bytes_read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..bytes_read]);

                if request.contains("POST /api/colab/handshake") {
                    let body = r#"{"status":"connected","gpu_device":"NVIDIA Tesla T4","vram_total_mb":15109,"cuda_version":"12.2","bridge_version":"v3.1.0"}"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes());
                } else if request.contains("GET /api/colab/status") {
                    let body = r#"{"online":true,"gpu_device":"NVIDIA Tesla T4","vram_free_mb":14200,"vram_total_mb":15109,"active_models":["tropical_polyhedral"]}"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes());
                }
            }
        }
    });

    // 1. Connect Colab bridge
    let connect_res = fixture.run_success(&[
        "colab",
        "connect",
        "--url",
        &endpoint,
        "--token",
        token,
        "--session-path",
        session_file.to_str().unwrap(),
        "--json",
    ]);

    assert_eq!(connect_res["status"], "connected");
    assert_eq!(connect_res["session"]["gpu_device"], "NVIDIA Tesla T4");
    assert_eq!(connect_res["session"]["vram_total_mb"], 15109);
    assert!(session_file.exists());

    // 2. Query Colab bridge status
    let status_res = fixture.run_success(&[
        "colab",
        "status",
        "--session-path",
        session_file.to_str().unwrap(),
        "--json",
    ]);

    assert_eq!(status_res["online"], true);
    assert_eq!(status_res["vram_free_mb"], 14200);

    // 3. Ping Colab bridge
    let ping_res = fixture.run_success(&[
        "colab",
        "ping",
        "--session-path",
        session_file.to_str().unwrap(),
        "--json",
    ]);

    assert_eq!(ping_res["online"], true);
    assert_eq!(ping_res["gpu_device"], "NVIDIA Tesla T4");

    // 4. Disconnect Colab bridge
    let dc_res = fixture.run_success(&[
        "colab",
        "disconnect",
        "--session-path",
        session_file.to_str().unwrap(),
        "--json",
    ]);

    assert_eq!(dc_res["status"], "disconnected");
    assert!(!session_file.exists());

    let _ = server_handle.join();
}

#[test]
fn test_e2e_cockpit_snapshot_surfacing_zk_and_colab_telemetry() {
    let fixture = TestFixture::new("cockpit-zk-colab");

    // Query 12-panel cockpit snapshot
    let snap_res = fixture.run_success(&["cockpit", "status", "--panels", "12", "--json"]);

    let panels = snap_res["panels"].as_array().expect("panels array");
    assert_eq!(panels.len(), 12);

    // Check Panel 10: [10:Privacy-ZK]
    let panel_zk = panels
        .iter()
        .find(|p| p["panel_id"] == 10)
        .expect("Panel 10 PrivacyZk present");
    assert_eq!(panel_zk["title"], "[10:Privacy-ZK]");
    assert_eq!(panel_zk["health"], "healthy");
    let zk_metrics = &panel_zk["metrics"];
    assert_eq!(zk_metrics["rdp_epsilon"], "1.24");
    assert_eq!(zk_metrics["rdp_delta"], "1e-6");
    assert_eq!(zk_metrics["zk_proof_soundness"], "2^-49.7");
    assert_eq!(zk_metrics["zk_schwartz_zippel_domain"], "F_p_goldilocks");
    assert_eq!(zk_metrics["zk_attestation_status"], "VERIFIED");

    // Check Panel 3: [03:Genes-Evolution]
    let panel_genes = panels
        .iter()
        .find(|p| p["panel_id"] == 3)
        .expect("Panel 3 GenesEvolution present");
    assert_eq!(panel_genes["title"], "[03:Genes-Evolution]");
    let genes_metrics = &panel_genes["metrics"];
    assert_eq!(genes_metrics["colab_gpu_bridge_status"], "CONNECTED");
    assert_eq!(genes_metrics["colab_active_vram_mb"], "14200");
    assert_eq!(genes_metrics["tropical_quant_error_bound"], "0.042");
}

#[test]
fn test_e2e_inference_infra_all_12_projects_and_theorems_50_to_55() {
    let fixture = TestFixture::new("inference-infra");

    // 1. Run all 12 projects through unified CLI command
    let inf_res = fixture.run_success(&["runtime", "inference", "--mode", "all", "--json"]);

    assert_eq!(inf_res["suite"], "12_inference_infrastructure_projects");
    assert_eq!(inf_res["all_engines_passed"], true);
    assert_eq!(
        inf_res["theorems_verified"],
        serde_json::json!([50, 51, 52, 53, 54, 55])
    );

    // 2. Test individual submodes
    let grammar_res = fixture.run_success(&["runtime", "inference", "--grammar", "--json"]);
    assert_eq!(grammar_res["theorem"], 50);
    assert_eq!(grammar_res["theorem_50_grammar_automaton_verified"], true);

    let disagg_res = fixture.run_success(&["runtime", "inference", "--disaggregated", "--json"]);
    assert_eq!(disagg_res["theorem"], 51);
    assert_eq!(disagg_res["theorem_51_mfu_optimization_verified"], true);

    let moe_res = fixture.run_success(&["runtime", "inference", "--moe", "--json"]);
    assert_eq!(moe_res["theorem"], 52);
    assert_eq!(moe_res["theorem_52_expert_routing_verified"], true);

    let quant_res = fixture.run_success(&["runtime", "inference", "--quant", "--json"]);
    assert_eq!(quant_res["theorem"], 53);
    assert_eq!(quant_res["theorem_53_quant_error_bound_verified"], true);

    let finops_res = fixture.run_success(&["runtime", "inference", "--finops", "--json"]);
    assert_eq!(finops_res["theorem"], 54);
    assert_eq!(
        finops_res["theorem_54_mean_field_autoscaler_verified"],
        true
    );

    let roofline_res = fixture.run_success(&["runtime", "inference", "--roofline", "--json"]);
    assert_eq!(roofline_res["theorem"], 55);
    assert_eq!(roofline_res["theorem_55_roofline_duality_verified"], true);
}
