#![forbid(unsafe_code)]

//! Adversarial Stress & Empirical Challenge Suite: Daemon Guard & Merkle-DAG Audit Ledger
//!
//! Validates:
//! 1. Multi-iteration daemon-guard execution, hash-chain continuity, and full DAG lineage verification.
//! 2. Tampered ancestor payload detection (parent canonical hash invalidation).
//! 3. Tampered direct payload detection in DAG entries.
//! 4. Non-existent parent ID injection and rejection.
//! 5. Causal cycle injection rejection (Acyclicity preservation).
//! 6. Exhaustive bit-flip tampering rejection across Merkle inclusion proofs (leaf, root, sibling, direction).

use pandora_runtime::assurance_receipt::UnifiedAssuranceReceipt;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(1200);

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
        let sequence = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let temp_root = std::env::temp_dir()
            .canonicalize()
            .expect("temporary directory canonical path");
        let root = temp_root.join(format!(
            "pandora-adv-daemon-audit-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        let data = root.join("data");
        fs::create_dir_all(&workspace).expect("workspace created");
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
        let output = self.command(args).output().expect("command executes");
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        assert!(
            output.status.success(),
            "command '{:?}' failed with stderr: {}\nstdout: {}",
            args,
            stderr,
            stdout
        );
        serde_json::from_str(&stdout).expect("output must be valid JSON envelope")
    }

    fn run_raw(&self, args: &[&str]) -> (String, String, bool) {
        let output = self.command(args).output().expect("command executes");
        (
            String::from_utf8_lossy(&output.stdout).to_string(),
            String::from_utf8_lossy(&output.stderr).to_string(),
            output.status.success(),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn test_daemon_guard_multi_iteration_chain_integrity() {
    let fixture = Fixture::new();
    let store_path = fixture.data.join("audit_chain_test.json");
    let store_str = store_path.to_str().unwrap();

    // Execute 5 ticks of daemon-guard
    let val = fixture.run_success(&[
        "runtime",
        "daemon-guard",
        "--iterations",
        "5",
        "--interval-ms",
        "10",
        "--store",
        store_str,
        "--json",
    ]);

    assert_eq!(val["command"], "runtime daemon-guard");
    assert_eq!(val["sequence_number"], 4);
    assert_eq!(val["phases_certified"], 12);
    assert!(val["all_invariants_verified"].as_bool().unwrap());

    let tick_reports = val["tick_reports"]
        .as_array()
        .expect("tick_reports must be an array");
    assert_eq!(tick_reports.len(), 5);

    // Verify sequential integrity and hash chaining across all 5 reports
    let mut prev_digest: Option<String> = None;
    for (i, tick) in tick_reports.iter().enumerate() {
        assert_eq!(tick["sequence_number"], i);
        assert!(tick["all_invariants_verified"].as_bool().unwrap());

        let receipt_val = &tick["receipt"];
        let receipt: UnifiedAssuranceReceipt =
            serde_json::from_value(receipt_val.clone()).expect("receipt parses");
        assert_eq!(receipt.sequence_number, i as u64);
        assert_eq!(receipt.phases_certified, 12);
        assert!(receipt.verify_hash_integrity());

        if i == 0 {
            assert_eq!(receipt.prev_receipt_sha256, None);
        } else {
            assert_eq!(receipt.prev_receipt_sha256, prev_digest);
        }
        prev_digest = Some(receipt.receipt_sha256.clone());

        // Verify each DAG node independently via `pandora audit verify --dag-node <id>`
        let node_id = tick["audit_node_id"].as_str().expect("node id exists");
        let verify_val = fixture.run_success(&[
            "audit",
            "verify",
            "--dag-node",
            node_id,
            "--store",
            store_str,
            "--json",
        ]);

        assert_eq!(verify_val["command"], "audit verify");
        assert_eq!(verify_val["mode"], "dag_causal_lineage");
        assert_eq!(verify_val["node_id"], node_id);
        assert!(verify_val["verified"].as_bool().unwrap());
        assert_eq!(verify_val["tamper_status"], "UNMODIFIED");
    }

    // Verify tree properties
    let tree_val = fixture.run_success(&["audit", "tree", "--store", store_str, "--json"]);
    assert_eq!(tree_val["leaf_count"], 5);
    assert_eq!(tree_val["dag_node_count"], 5);
    assert!(tree_val["is_acyclic"].as_bool().unwrap());
}

#[test]
fn test_tampered_merkle_dag_ancestor_payload_detection() {
    let fixture = Fixture::new();
    let store_path = fixture.data.join("ancestor_tamper.json");
    let store_str = store_path.to_str().unwrap();

    // Generate 3 iterations
    let val = fixture.run_success(&[
        "runtime",
        "daemon-guard",
        "--iterations",
        "3",
        "--interval-ms",
        "10",
        "--store",
        store_str,
        "--json",
    ]);

    let final_node_id = val["audit_node_id"].as_str().unwrap();

    // Verify clean baseline
    let clean_verify = fixture.run_success(&[
        "audit",
        "verify",
        "--dag-node",
        final_node_id,
        "--store",
        store_str,
        "--json",
    ]);
    assert!(clean_verify["verified"].as_bool().unwrap());

    // Adversarially tamper with genesis node payload (dag_entries[0])
    let content = fs::read_to_string(&store_path).expect("read store");
    let mut store_json: Value = serde_json::from_str(&content).expect("parse store json");

    let original_hex = store_json["dag_entries"][0]["payload_hex"]
        .as_str()
        .unwrap()
        .to_string();
    let mut tampered_hex = original_hex.clone();
    // Flip first two hex characters
    tampered_hex.replace_range(
        0..2,
        if original_hex.starts_with("aa") {
            "bb"
        } else {
            "aa"
        },
    );
    store_json["dag_entries"][0]["payload_hex"] = Value::String(tampered_hex);

    fs::write(
        &store_path,
        serde_json::to_string_pretty(&store_json).unwrap(),
    )
    .expect("write tampered store");

    // Attempt to verify child node 2: must fail because genesis node ID changed, breaking causal lineage
    let (stdout, stderr, success) = fixture.run_raw(&[
        "audit",
        "verify",
        "--dag-node",
        final_node_id,
        "--store",
        store_str,
        "--json",
    ]);

    assert!(
        !success,
        "Tampering with ancestor payload must cause audit verify to fail!\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("parent node not found in DAG")
            || stdout.contains("parent node not found in DAG")
            || stderr.contains("causal lineage corrupted")
            || stdout.contains("causal lineage corrupted"),
        "Error must cite parent not found or corrupted lineage, got stdout: {stdout}, stderr: {stderr}"
    );
}

#[test]
fn test_tampered_merkle_dag_direct_payload_detection() {
    let fixture = Fixture::new();
    let store_path = fixture.data.join("direct_tamper.json");
    let store_str = store_path.to_str().unwrap();

    // Generate 2 iterations
    let val = fixture.run_success(&[
        "runtime",
        "daemon-guard",
        "--iterations",
        "2",
        "--interval-ms",
        "10",
        "--store",
        store_str,
        "--json",
    ]);

    let target_node_id = val["audit_node_id"].as_str().unwrap();

    // Adversarially tamper with target node payload directly (dag_entries[1])
    let content = fs::read_to_string(&store_path).expect("read store");
    let mut store_json: Value = serde_json::from_str(&content).expect("parse store json");

    let original_hex = store_json["dag_entries"][1]["payload_hex"]
        .as_str()
        .unwrap()
        .to_string();
    let mut tampered_hex = original_hex.clone();
    tampered_hex.replace_range(
        0..2,
        if original_hex.starts_with("11") {
            "22"
        } else {
            "11"
        },
    );
    store_json["dag_entries"][1]["payload_hex"] = Value::String(tampered_hex);

    fs::write(
        &store_path,
        serde_json::to_string_pretty(&store_json).unwrap(),
    )
    .expect("write tampered store");

    // Attempt to verify original target node ID: must fail because node ID is derived from canonical hash
    let (stdout, stderr, success) = fixture.run_raw(&[
        "audit",
        "verify",
        "--dag-node",
        target_node_id,
        "--store",
        store_str,
        "--json",
    ]);

    assert!(
        !success,
        "Tampering with node payload must invalidate node lookup!\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("not found in store")
            || stdout.contains("not found in store")
            || stderr.contains("not found in DAG")
            || stdout.contains("not found in DAG")
            || stderr.contains("corrupted")
            || stdout.contains("corrupted"),
        "Must report error due to hash mismatch, got stdout: '{stdout}', stderr: '{stderr}'"
    );
}

#[test]
fn test_tampered_merkle_dag_nonexistent_parent_detection() {
    let fixture = Fixture::new();
    let store_path = fixture.data.join("nonexistent_parent.json");
    let store_str = store_path.to_str().unwrap();

    let val = fixture.run_success(&[
        "runtime",
        "daemon-guard",
        "--iterations",
        "2",
        "--interval-ms",
        "10",
        "--store",
        store_str,
        "--json",
    ]);

    let node_id = val["audit_node_id"].as_str().unwrap();

    // Tamper parent_ids to point to nonexistent fake hash
    let content = fs::read_to_string(&store_path).expect("read store");
    let mut store_json: Value = serde_json::from_str(&content).expect("parse store json");
    store_json["dag_entries"][1]["parent_ids"] = Value::Array(vec![Value::String(
        "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_string(),
    )]);

    fs::write(
        &store_path,
        serde_json::to_string_pretty(&store_json).unwrap(),
    )
    .expect("write tampered store");

    let (stdout, stderr, success) = fixture.run_raw(&[
        "audit",
        "verify",
        "--dag-node",
        node_id,
        "--store",
        store_str,
        "--json",
    ]);

    assert!(!success, "Nonexistent parent must be rejected");
    assert!(
        stderr.contains("parent node not found in DAG")
            || stdout.contains("parent node not found in DAG")
    );
}

#[test]
fn test_tampered_merkle_dag_cycle_injection_detection() {
    let fixture = Fixture::new();
    let store_path = fixture.data.join("cycle_injection.json");
    let store_str = store_path.to_str().unwrap();

    let val = fixture.run_success(&[
        "runtime",
        "daemon-guard",
        "--iterations",
        "2",
        "--interval-ms",
        "10",
        "--store",
        store_str,
        "--json",
    ]);

    let tick_reports = val["tick_reports"].as_array().unwrap();
    let node0_id = tick_reports[0]["audit_node_id"].as_str().unwrap();
    let node1_id = tick_reports[1]["audit_node_id"].as_str().unwrap();

    // Adversarially inject a cycle: make node 0 list node 1 as its parent
    let content = fs::read_to_string(&store_path).expect("read store");
    let mut store_json: Value = serde_json::from_str(&content).expect("parse store json");
    store_json["dag_entries"][0]["parent_ids"] =
        Value::Array(vec![Value::String(node1_id.to_string())]);

    fs::write(
        &store_path,
        serde_json::to_string_pretty(&store_json).unwrap(),
    )
    .expect("write tampered store");

    // Loading store to verify should detect cycle or parent not found (since node 1 is defined after node 0)
    let (stdout, stderr, success) = fixture.run_raw(&[
        "audit",
        "verify",
        "--dag-node",
        node0_id,
        "--store",
        store_str,
        "--json",
    ]);

    assert!(
        !success,
        "Cyclic DAG modification must fail to load or verify"
    );
    assert!(
        stderr.contains("parent node not found in DAG")
            || stdout.contains("parent node not found in DAG")
            || stderr.contains("cycle detected")
            || stdout.contains("cycle detected")
    );
}

#[test]
fn test_tampered_merkle_inclusion_proof_bit_flip_detection() {
    let fixture = Fixture::new();
    let store_path = fixture.data.join("inclusion_proof_test.json");
    let store_str = store_path.to_str().unwrap();

    // Generate 4 iterations
    fixture.run_success(&[
        "runtime",
        "daemon-guard",
        "--iterations",
        "4",
        "--interval-ms",
        "10",
        "--store",
        store_str,
        "--json",
    ]);

    // Generate inclusion proof for leaf index 2
    let proof_val = fixture.run_success(&[
        "audit", "proof", "--index", "2", "--store", store_str, "--json",
    ]);

    assert_eq!(proof_val["command"], "audit proof");
    assert_eq!(proof_val["leaf_index"], 2);
    assert!(proof_val["verified"].as_bool().unwrap());

    // Baseline: verify honest inclusion proof inline
    // Format proof struct matching `InclusionProof` schema
    let leaf_hash_hex = proof_val["leaf_hash"].as_str().unwrap();
    let root_hash_hex = proof_val["root_hash"].as_str().unwrap();
    let steps = proof_val["steps"].as_array().unwrap();

    let hex_to_32_bytes = |hex_str: &str| -> Vec<u8> {
        (0..hex_str.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex_str[i..i + 2], 16).unwrap())
            .collect()
    };

    let honest_proof = serde_json::json!({
        "leaf_index": 2,
        "leaf_hash": hex_to_32_bytes(leaf_hash_hex),
        "steps": steps.iter().map(|s| {
            serde_json::json!({
                "sibling_hash": hex_to_32_bytes(s["sibling_hash"].as_str().unwrap()),
                "direction": s["direction"].as_str().unwrap()
            })
        }).collect::<Vec<_>>(),
        "root_hash": hex_to_32_bytes(root_hash_hex)
    });

    let honest_verify = fixture.run_success(&[
        "audit",
        "verify",
        "--proof",
        &honest_proof.to_string(),
        "--json",
    ]);
    assert!(honest_verify["verified"].as_bool().unwrap());
    assert_eq!(honest_verify["tamper_status"], "UNMODIFIED");

    // Bit-Flip Attack A: Tamper leaf_hash
    let mut tampered_leaf = honest_proof.clone();
    let mut tampered_leaf_bytes = hex_to_32_bytes(leaf_hash_hex);
    tampered_leaf_bytes[0] ^= 0x01;
    tampered_leaf["leaf_hash"] = serde_json::to_value(&tampered_leaf_bytes).unwrap();

    let leaf_tamper_verify = fixture.run_success(&[
        "audit",
        "verify",
        "--proof",
        &tampered_leaf.to_string(),
        "--json",
    ]);
    assert!(
        !leaf_tamper_verify["verified"].as_bool().unwrap(),
        "Bit-flipped leaf hash must fail verification"
    );
    assert_eq!(leaf_tamper_verify["tamper_status"], "CORRUPTED_OR_TAMPERED");

    // Bit-Flip Attack B: Tamper root_hash
    let mut tampered_root = honest_proof.clone();
    let mut tampered_root_bytes = hex_to_32_bytes(root_hash_hex);
    tampered_root_bytes[0] ^= 0x01;
    tampered_root["root_hash"] = serde_json::to_value(&tampered_root_bytes).unwrap();

    let root_tamper_verify = fixture.run_success(&[
        "audit",
        "verify",
        "--proof",
        &tampered_root.to_string(),
        "--json",
    ]);
    assert!(
        !root_tamper_verify["verified"].as_bool().unwrap(),
        "Bit-flipped root hash must fail verification"
    );
    assert_eq!(root_tamper_verify["tamper_status"], "CORRUPTED_OR_TAMPERED");

    // Bit-Flip Attack C: Tamper sibling_hash in co-path step
    let mut tampered_sibling = honest_proof.clone();
    let orig_sibling = tampered_sibling["steps"][0]["sibling_hash"]
        .as_array()
        .unwrap()
        .clone();
    let mut mutated_sibling = orig_sibling;
    let b = mutated_sibling[0].as_u64().unwrap() ^ 0x01;
    mutated_sibling[0] = serde_json::json!(b);
    tampered_sibling["steps"][0]["sibling_hash"] = Value::Array(mutated_sibling);

    let sibling_tamper_verify = fixture.run_success(&[
        "audit",
        "verify",
        "--proof",
        &tampered_sibling.to_string(),
        "--json",
    ]);
    assert!(
        !sibling_tamper_verify["verified"].as_bool().unwrap(),
        "Bit-flipped sibling hash must fail verification"
    );
    assert_eq!(
        sibling_tamper_verify["tamper_status"],
        "CORRUPTED_OR_TAMPERED"
    );

    // Direction Toggle Attack D: Invert co-path direction (Left <-> Right)
    let mut tampered_direction = honest_proof.clone();
    let cur_dir = tampered_direction["steps"][0]["direction"]
        .as_str()
        .unwrap();
    let new_dir = if cur_dir == "Left" { "Right" } else { "Left" };
    tampered_direction["steps"][0]["direction"] = serde_json::json!(new_dir);

    let dir_tamper_verify = fixture.run_success(&[
        "audit",
        "verify",
        "--proof",
        &tampered_direction.to_string(),
        "--json",
    ]);
    assert!(
        !dir_tamper_verify["verified"].as_bool().unwrap(),
        "Inverted co-path direction must fail verification"
    );
    assert_eq!(dir_tamper_verify["tamper_status"], "CORRUPTED_OR_TAMPERED");
}
