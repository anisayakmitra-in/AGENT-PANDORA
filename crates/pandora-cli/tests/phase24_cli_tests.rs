//! Phase 24 CLI Integration Tests: Distributed Swarm Mesh & Cross-Node Consensus (Theorem 47)
//!
//! # Safety Invariant
//! Strictly `#![forbid(unsafe_code)]` enforced workspace-wide.

#![forbid(unsafe_code)]

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(2400);

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
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let temp_root = std::env::temp_dir()
            .canonicalize()
            .expect("temporary directory should have a canonical path");
        let root = temp_root.join(format!(
            "pandora-phase24-test-{}-{timestamp}-{sequence}",
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
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON")
    }

    fn run_failure(&self, args: &[&str]) -> (i32, String) {
        let output = self.command(args).output().expect("command should execute");
        assert!(
            !output.status.success(),
            "command should have failed: {:?}",
            args
        );
        let combined = format!(
            "{} {}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        (output.status.code().unwrap_or(-1), combined)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn test_swarm_connect_and_status() {
    let fixture = Fixture::new();

    // 1. Initial status before connections
    let init_status = fixture.run_success(&["swarm", "status", "--json"]);
    assert_eq!(init_status["command"], "swarm status");
    assert_eq!(init_status["epoch"], 1);
    assert_eq!(init_status["connected_peers_count"], 0);
    assert_eq!(init_status["bft_parameters"]["total_nodes"], 4);
    assert_eq!(init_status["bft_parameters"]["byzantine_tolerance"], 1);
    assert_eq!(init_status["bft_parameters"]["quorum_threshold"], 3);
    assert!(init_status["local_peer_id"].is_string());
    assert!(init_status["public_key"].is_string());

    // 2. Connect to peer
    let connect_res = fixture.run_success(&[
        "swarm",
        "connect",
        "--peer",
        "127.0.0.1:4001",
        "--node-id",
        "node-alpha-4001",
        "--json",
    ]);
    assert_eq!(connect_res["command"], "swarm connect");
    assert_eq!(connect_res["connected"], true);
    assert_eq!(connect_res["peer_address"], "127.0.0.1:4001");
    assert_eq!(connect_res["remote_node_id"], "node-alpha-4001");
    assert_eq!(connect_res["active_peers"], 1);

    // 3. Connect to second peer
    let connect_res2 = fixture.run_success(&[
        "swarm",
        "connect",
        "--peer",
        "127.0.0.1:4002",
        "--node-id",
        "node-beta-4002",
        "--json",
    ]);
    assert_eq!(connect_res2["active_peers"], 2);

    // 4. Verify updated status
    let status_after = fixture.run_success(&["swarm", "status", "--json"]);
    assert_eq!(status_after["connected_peers_count"], 2);
    let peers = status_after["connected_peers"]
        .as_array()
        .expect("peers array");
    assert_eq!(peers.len(), 2);
    assert_eq!(peers[0]["peer_id"], "node-alpha-4001");
    assert_eq!(peers[1]["peer_id"], "node-beta-4002");
}

#[test]
fn test_swarm_propose_patch_lifecycle_and_duplicate_rejection() {
    let fixture = Fixture::new();

    // 1. Propose candidate patch
    let prop_res = fixture.run_success(&[
        "swarm",
        "propose",
        "--goal",
        "Harden buffer boundary checks against overflow",
        "--title",
        "Memory Safety Patch",
        "--author",
        "claude-code",
        "--proposal-id",
        "prop-phase24-test-1",
        "--json",
    ]);
    assert_eq!(prop_res["command"], "swarm propose");
    assert_eq!(prop_res["proposal_id"], "prop-phase24-test-1");
    assert_eq!(prop_res["agent"], "claude");
    assert_eq!(prop_res["status"], "pending_quorum");
    assert_eq!(prop_res["quorum_needed"], 3);
    assert!(prop_res["fingerprint"].is_string());

    // 2. Duplicate proposal rejection
    let (code, combined) = fixture.run_failure(&[
        "swarm",
        "propose",
        "--goal",
        "Duplicate proposal attempt",
        "--proposal-id",
        "prop-phase24-test-1",
        "--json",
    ]);
    assert_ne!(code, 0);
    assert!(
        combined.contains("already registered"),
        "output should mention already registered: {combined}"
    );

    // 3. Status reflects the registered proposal
    let status = fixture.run_success(&["swarm", "status", "--json"]);
    assert_eq!(status["proposals_count"], 1);
    assert_eq!(status["decisions_count"], 1);
}

#[test]
fn test_swarm_byzantine_vote_quorum_and_certification() {
    let fixture = Fixture::new();

    // 1. Propose patch
    let _ = fixture.run_success(&[
        "swarm",
        "propose",
        "--goal",
        "Fix concurrency race condition in scheduler",
        "--title",
        "Race condition patch",
        "--author",
        "gemini-cli",
        "--proposal-id",
        "prop-consensus-47",
        "--json",
    ]);

    // 2. Vote 1: node-alpha approves (1/3)
    let vote1 = fixture.run_success(&[
        "swarm",
        "vote",
        "--proposal",
        "prop-consensus-47",
        "--voter",
        "node-alpha",
        "--approve",
        "--pass",
        "0.98",
        "--confidence",
        "0.96",
        "--compactness",
        "0.91",
        "--invariant",
        "0.99",
        "--json",
    ]);
    assert_eq!(vote1["command"], "swarm vote");
    assert_eq!(vote1["proposal_id"], "prop-consensus-47");
    assert_eq!(vote1["voter"], "node-alpha");
    assert_eq!(vote1["approve"], true);
    assert!(vote1["decision"]["Pending"].is_object());
    assert_eq!(vote1["decision"]["Pending"]["current_approvals"], 1);
    assert_eq!(vote1["decision"]["Pending"]["quorum_needed"], 3);

    // 3. Vote 2: node-beta approves (2/3)
    let vote2 = fixture.run_success(&[
        "swarm",
        "vote",
        "--proposal",
        "prop-consensus-47",
        "--voter",
        "node-beta",
        "--approve",
        "--pass",
        "0.95",
        "--confidence",
        "0.93",
        "--compactness",
        "0.89",
        "--invariant",
        "0.97",
        "--json",
    ]);
    assert_eq!(vote2["decision"]["Pending"]["current_approvals"], 2);

    // 4. Vote 3: node-gamma approves (3/3) -> Quorum reached (Theorem 47)
    let vote3 = fixture.run_success(&[
        "swarm",
        "vote",
        "--proposal",
        "prop-consensus-47",
        "--voter",
        "node-gamma",
        "--approve",
        "--pass",
        "0.97",
        "--confidence",
        "0.95",
        "--compactness",
        "0.92",
        "--invariant",
        "0.98",
        "--json",
    ]);
    assert!(vote3["decision"]["Approved"].is_object());
    let approved = &vote3["decision"]["Approved"];
    assert_eq!(approved["winning_proposal_id"], "prop-consensus-47");
    assert_eq!(approved["winning_agent"], "gemini_cli");
    assert!(
        approved["certificate_digest"]
            .as_str()
            .unwrap()
            .starts_with("PANDORA_PATCH_CERT_V1:")
    );
    assert!(approved["scores"].is_object());

    // 5. Status shows approved decision
    let status = fixture.run_success(&["swarm", "status", "--json"]);
    assert_eq!(status["decisions_count"], 1);
}

#[test]
fn test_swarm_vote_equivocation_detection_theorem_47() {
    let fixture = Fixture::new();

    // 1. Propose patch
    let _ = fixture.run_success(&[
        "swarm",
        "propose",
        "--goal",
        "Equivocation resilience test",
        "--proposal-id",
        "prop-equivocation-1",
        "--json",
    ]);

    // 2. Node casts an approval vote
    let _ = fixture.run_success(&[
        "swarm",
        "vote",
        "--proposal",
        "prop-equivocation-1",
        "--voter",
        "adversarial-node",
        "--approve",
        "--pass",
        "0.95",
        "--json",
    ]);

    // 3. Same node attempts to equivocate by casting a rejection vote
    let (code, combined) = fixture.run_failure(&[
        "swarm",
        "vote",
        "--proposal",
        "prop-equivocation-1",
        "--voter",
        "adversarial-node",
        "--reject",
        "--json",
    ]);
    assert_ne!(code, 0);
    assert!(
        combined.contains("equivocation detected"),
        "output should detect equivocation under Theorem 47: {combined}"
    );
}

#[test]
fn test_swarm_sync_gradients_and_policy_evolution() {
    let fixture = Fixture::new();

    // 1. Initial status policy parameters
    let status_init = fixture.run_success(&["swarm", "status", "--json"]);
    let init_policy = status_init["federated_policy"]
        .as_array()
        .expect("policy array");
    assert_eq!(init_policy.len(), 6);
    assert_eq!(status_init["epoch"], 1);

    // 2. Synchronize federated policy gradients (Theorem 47 Lemma 47.4)
    let sync_res = fixture.run_success(&[
        "swarm",
        "sync-gradients",
        "--epoch",
        "1",
        "--loss",
        "0.115",
        "--samples",
        "128",
        "--json",
    ]);
    assert_eq!(sync_res["command"], "swarm sync-gradients");
    assert_eq!(sync_res["epoch"], 1);
    assert_eq!(sync_res["participating_agents"], 4);
    assert!(sync_res["aggregate_norm"].as_f64().unwrap() > 0.0);
    assert_eq!(sync_res["update_hash"].as_str().unwrap().len(), 64);

    let updated_policy = sync_res["updated_policy"]
        .as_array()
        .expect("updated policy");
    assert_eq!(updated_policy.len(), 6);

    // Check that policy parameters shifted due to gradient descent
    let mut any_diff = false;
    for i in 0..6 {
        let p_init = init_policy[i].as_f64().unwrap();
        let p_new = updated_policy[i].as_f64().unwrap();
        if (p_init - p_new).abs() > 1e-6 {
            any_diff = true;
        }
    }
    assert!(
        any_diff,
        "policy parameters must update after gradient sync"
    );

    // 3. Status reflects advanced epoch and new policy
    let status_after = fixture.run_success(&["swarm", "status", "--json"]);
    assert_eq!(status_after["epoch"], 2);
}

#[test]
fn test_swarm_cli_usage_validation_errors() {
    let fixture = Fixture::new();

    // Bare swarm command without subcommand
    let (code1, combined1) = fixture.run_failure(&["swarm"]);
    assert_ne!(code1, 0);
    assert!(combined1.contains("swarm requires"));

    // Connect without peer
    let (code2, combined2) = fixture.run_failure(&["swarm", "connect"]);
    assert_ne!(code2, 0);
    assert!(combined2.contains("swarm connect requires"));

    // Propose without goal
    let (code3, combined3) = fixture.run_failure(&["swarm", "propose"]);
    assert_ne!(code3, 0);
    assert!(combined3.contains("swarm propose requires"));

    // Vote without proposal
    let (code4, combined4) = fixture.run_failure(&["swarm", "vote"]);
    assert_ne!(code4, 0);
    assert!(combined4.contains("swarm vote requires"));

    // Unknown subcommand
    let (code5, combined5) = fixture.run_failure(&["swarm", "nonexistent"]);
    assert_ne!(code5, 0);
    assert!(combined5.contains("unknown swarm command"));
}
