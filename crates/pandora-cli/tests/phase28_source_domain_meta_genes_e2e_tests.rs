//! Phase 28 E2E Integration Suite:
//! Unified Source, Domain & Meta Harness Tier Orchestration,
//! Constitutional Substrates, Meta Composition Genes, Domain Routing,
//! and Genomic PAC-Bayes Evolutionary Adaptation Engine (Theorems 58, 59, 60, 61).
//!
//! # Invariants
//! - Strictly `#![forbid(unsafe_code)]` workspace-wide.
//! - Deterministic genomic mutation and PAC-Bayes generalization guarantees.
//! - Isolated fixture sandboxes and ephemeral HTTP/1.1 ports.

#![forbid(unsafe_code)]

use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(2800);

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
fn test_harness_tiers_inventory_cli() {
    let fixture = TestFixture::new("harness-tiers");
    let res = fixture.run_success(&["harness", "tiers", "--json"]);

    assert_eq!(res["total_tiers"], 3);
    assert_eq!(res["total_registered_genes"], 70);
    assert_eq!(res["source_harnesses_count"], 1);
    assert_eq!(res["domain_harnesses_count"], 9);
    assert_eq!(res["meta_harnesses_count"], 3);

    let theorems = res["theorems"].as_array().expect("theorems array");
    assert!(theorems.iter().any(|v| v.as_u64() == Some(58)));
    assert!(theorems.iter().any(|v| v.as_u64() == Some(59)));
    assert!(theorems.iter().any(|v| v.as_u64() == Some(60)));
    assert!(theorems.iter().any(|v| v.as_u64() == Some(61)));
}

#[test]
fn test_harness_dispatch_multi_tier_cli() {
    let fixture = TestFixture::new("harness-dispatch");
    let res = fixture.run_success(&[
        "harness",
        "dispatch",
        "--task",
        "verify zero knowledge rollups and cryptographic proofs",
        "--meta",
        "coordination",
        "--json",
    ]);

    assert_eq!(res["dispatched_domain"], "verification-domain");
    assert_eq!(res["meta_harness"], "coordination");
    assert_eq!(res["status"], "completed");

    let tier_path = res["tier_path"].as_array().expect("tier path");
    assert_eq!(tier_path[0], "Meta");
    assert_eq!(tier_path[1], "Domain");
    assert_eq!(tier_path[2], "Source");

    assert_eq!(res["meta_coordination"]["theorem"], 58);
}

#[test]
fn test_harness_gene_list_cli() {
    let fixture = TestFixture::new("harness-genes");
    let res = fixture.run_success(&["harness", "gene", "list", "--json"]);

    assert_eq!(res["total_genes"], 70);
    assert_eq!(res["domain_genes_count"], 64);

    let source_genes = res["source_genes"].as_array().expect("source genes");
    assert_eq!(source_genes.len(), 3);
    assert!(
        source_genes
            .iter()
            .any(|g| g == "system.memory.constitutional")
    );

    let meta_genes = res["meta_genes"].as_array().expect("meta genes");
    assert_eq!(meta_genes.len(), 3);
    assert!(meta_genes.iter().any(|g| g == "meta.coordination.handoff"));
}

#[test]
fn test_harness_gene_evolve_cli() {
    let fixture = TestFixture::new("harness-evolve");
    let res = fixture.run_success(&[
        "harness",
        "gene",
        "evolve",
        "--seed",
        "thanatos-v4-supercluster",
        "--generations",
        "3",
        "--json",
    ]);

    assert_eq!(res["total_generations"], 3);
    assert_eq!(res["theorem"], 61);
    assert!(
        res["theorem_61_pac_bound_satisfied"]
            .as_bool()
            .unwrap_or(false)
    );

    let generations = res["generations"].as_array().expect("generations array");
    assert_eq!(generations.len(), 3);
    for generation in generations {
        let fitness = generation["fitness_score"].as_f64().expect("fitness");
        let pac_risk = generation["pac_bayes_risk_bound"]
            .as_f64()
            .expect("pac risk");
        assert!(fitness > 0.0);
        assert!(pac_risk > 0.0);
    }
}

#[test]
fn test_cockpit_web_harness_tiers_endpoint() {
    let controller = std::sync::Arc::new(std::sync::Mutex::new(
        pandora_runtime::cockpit::CockpitController::new(
            pandora_runtime::cockpit::CockpitLayout::TwelvePanel,
        ),
    ));
    let server =
        pandora_runtime::cockpit_web::CockpitWebServer::new(0, "127.0.0.1", controller, None)
            .expect("server initialized");

    let port = server.port();
    let shutdown_flag = server.shutdown_handle();

    let handle = thread::spawn(move || {
        let _ = server.run();
    });

    thread::sleep(Duration::from_millis(150));

    let mut stream = std::net::TcpStream::connect(format!("127.0.0.1:{port}"))
        .expect("connected to harness tiers endpoint");
    stream
        .write_all(
            b"GET /api/harness/tiers HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        )
        .expect("write request");

    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read response");

    assert!(
        response.contains("200 OK"),
        "tiers endpoint response: {response}"
    );
    assert!(response.contains("\"total_tiers\":3"));
    assert!(response.contains("\"total_registered_genes\":70"));

    shutdown_flag.store(true, Ordering::Relaxed);
    let _ = handle.join();
}
