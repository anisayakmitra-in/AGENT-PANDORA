#![forbid(unsafe_code)]

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_AUDIT_FIXTURE: AtomicU64 = AtomicU64::new(500);

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
            .expect("system clock should be available")
            .as_nanos();
        let sequence = NEXT_AUDIT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let temp_root = std::env::temp_dir()
            .canonicalize()
            .expect("temporary directory should have a canonical path");
        let root = temp_root.join(format!(
            "pandora-audit-test-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        let data = root.join("data");
        fs::create_dir_all(&workspace).expect("workspace should be created");
        fs::create_dir_all(&data).expect("data dir should be created");
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
        let output = self.command(args).output().expect("command should execute");
        assert!(
            output.status.success(),
            "command '{:?}' failed with stderr: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str(&stdout).expect("output should be valid json envelope")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn test_audit_record_proof_and_verify_e2e() {
    let fixture = Fixture::new();

    // 1. Record first event
    let val_rec1 = fixture.run_success(&[
        "audit",
        "record",
        "--payload",
        "agent_decision_step_1:initiate_plan",
        "--json",
    ]);
    assert_eq!(val_rec1["command"], "audit record");
    assert_eq!(val_rec1["leaf_index"], 0);
    assert_eq!(val_rec1["total_leaves"], 1);

    // 2. Record second event from a file
    let payload_file = fixture.workspace.join("event_payload.txt");
    fs::write(&payload_file, "agent_decision_step_2:execute_patch").unwrap();
    let val_rec2 = fixture.run_success(&[
        "audit",
        "record",
        "--file",
        payload_file.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(val_rec2["leaf_index"], 1);
    assert_eq!(val_rec2["total_leaves"], 2);
    assert_eq!(val_rec2["tree_depth"], 1);

    // 3. Generate inclusion proof for leaf 0 and export to file
    let proof_file = fixture.workspace.join("proof_0.json");
    let val_proof = fixture.run_success(&[
        "audit",
        "proof",
        "--index",
        "0",
        "--output",
        proof_file.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(val_proof["command"], "audit proof");
    assert_eq!(val_proof["verified"], true);
    assert_eq!(val_proof["proof_length"], 1);
    assert!(proof_file.exists());

    // 4. Verify inclusion proof from file
    let val_verify = fixture.run_success(&[
        "audit",
        "verify",
        "--proof-file",
        proof_file.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(val_verify["command"], "audit verify");
    assert_eq!(val_verify["verified"], true);
    assert_eq!(val_verify["tamper_status"], "UNMODIFIED");

    // 5. Tamper detection: corrupt the leaf hash in the proof file and verify failure
    let mut proof_content: Value =
        serde_json::from_str(&fs::read_to_string(&proof_file).unwrap()).unwrap();
    if let Some(arr) = proof_content["leaf_hash"].as_array_mut() {
        arr[0] = serde_json::json!(255); // Flip byte
    }
    fs::write(&proof_file, serde_json::to_string(&proof_content).unwrap()).unwrap();

    let val_tampered = fixture.run_success(&[
        "audit",
        "verify",
        "--proof-file",
        proof_file.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(val_tampered["verified"], false);
    assert_eq!(val_tampered["tamper_status"], "CORRUPTED_OR_TAMPERED");
}

#[test]
fn test_audit_tree_inspection() {
    let fixture = Fixture::new();

    for i in 0..4 {
        fixture.run_success(&[
            "audit",
            "record",
            "--payload",
            &format!("state_checkpoint_{i}"),
            "--json",
        ]);
    }

    let val_tree = fixture.run_success(&["audit", "tree", "--json"]);
    assert_eq!(val_tree["command"], "audit tree");
    assert_eq!(val_tree["leaf_count"], 4);
    assert_eq!(val_tree["tree_depth"], 2); // ceil(log2(4)) = 2
    assert_eq!(val_tree["is_acyclic"], true);
}

#[test]
fn test_audit_simulate_benchmark_and_tamper_suite() {
    let fixture = Fixture::new();

    let val_sim = fixture.run_success(&[
        "audit",
        "simulate",
        "--events",
        "100",
        "--samples",
        "25",
        "--seed",
        "424242",
        "--json",
    ]);

    assert_eq!(val_sim["command"], "audit simulate");
    assert_eq!(val_sim["events_count"], 100);
    assert_eq!(val_sim["tree_depth"], 7); // ceil(log2(100)) = 7
    assert_eq!(val_sim["all_sample_proofs_valid"], true);

    let tamper = &val_sim["tamper_detection"];
    assert_eq!(tamper["leaf_bit_flip_rejected"], true);
    assert_eq!(tamper["sibling_bit_flip_rejected"], true);
    assert_eq!(tamper["direction_inversion_rejected"], true);
    assert_eq!(tamper["root_tamper_rejected"], true);
    assert_eq!(tamper["tamper_resistance_verified"], true);

    let dag_sim = &val_sim["dag_simulation"];
    assert_eq!(dag_sim["lineages_verified"], true);
    assert_eq!(dag_sim["acyclic"], true);
}

#[test]
fn test_backup_audit_alias_parity() {
    let fixture = Fixture::new();

    let val_backup_rec = fixture.run_success(&[
        "backup",
        "audit",
        "record",
        "--payload",
        "backup_snapshot_event",
        "--json",
    ]);
    assert_eq!(val_backup_rec["command"], "audit record");
    assert_eq!(val_backup_rec["leaf_index"], 0);

    let val_backup_tree = fixture.run_success(&["backup", "audit", "tree", "--json"]);
    assert_eq!(val_backup_tree["command"], "audit tree");
    assert_eq!(val_backup_tree["leaf_count"], 1);
}

#[test]
fn test_audit_dag_causal_lineage_verification() {
    let fixture = Fixture::new();

    // 1. Genesis node
    let val_gen = fixture.run_success(&[
        "audit",
        "record",
        "--payload",
        "genesis_system_prompt_v1",
        "--parent",
        "",
        "--json",
    ]);
    let gen_id = val_gen["dag_node"]["id"].as_str().unwrap().to_string();

    // 2. Transition 1 child of genesis
    let val_child = fixture.run_success(&[
        "audit",
        "record",
        "--payload",
        "transition_to_code_generation",
        "--parent",
        &gen_id,
        "--json",
    ]);
    let child_id = val_child["dag_node"]["id"].as_str().unwrap().to_string();

    // 3. Verify causal lineage back to genesis
    let val_verify = fixture.run_success(&["audit", "verify", "--dag-node", &child_id, "--json"]);
    assert_eq!(val_verify["command"], "audit verify");
    assert_eq!(val_verify["mode"], "dag_causal_lineage");
    assert_eq!(val_verify["verified"], true);
    assert_eq!(val_verify["tamper_status"], "UNMODIFIED");
    assert_eq!(val_verify["parents_count"], 1);
}
