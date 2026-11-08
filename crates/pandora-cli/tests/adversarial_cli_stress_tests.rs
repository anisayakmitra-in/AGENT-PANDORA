#![forbid(unsafe_code)]

//! Adversarial Stress & Edge Case Test Suite for Milestone 1 CLI Wiring.
//!
//! Subcommands under stress:
//! 1. `pandora models route` (MAB routing: extreme params, non-finite values, zero/large rounds, custom arms)
//! 2. `pandora audit` (Merkle-DAG: zero/single/many leaves, invalid proof index, corrupt proof, cycle detection, corrupt store)
//! 3. `pandora runtime speculative benchmark` & `kv-cache` (extreme alpha 0.0/1.0, extreme latency ratio, 1 session, 0 suffix, large tokens)
//!
//! Invariant: ZERO runtime panics under all adversarial, corrupted, and boundary inputs.

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(500);

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
            "pandora-adv-test-{}-{timestamp}-{sequence}",
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
        let mut all_args = args.to_vec();
        if !all_args.contains(&"--json") {
            all_args.push("--json");
        }
        let output = self
            .command(&all_args)
            .output()
            .expect("command should execute");
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(
            !stderr.contains("panicked at"),
            "PANIC DETECTED in command '{args:?}':\n{stderr}"
        );
        assert!(
            output.status.success(),
            "command '{args:?}' failed unexpectedly. Status: {:?}\nStderr: {stderr}\nStdout: {stdout}",
            output.status.code()
        );

        serde_json::from_str(&stdout)
            .unwrap_or_else(|e| panic!("invalid json from command {args:?}: {e}\nStdout: {stdout}"))
    }

    fn run_failure(&self, args: &[&str]) -> (Option<i32>, String, String) {
        let mut all_args = args.to_vec();
        if !all_args.contains(&"--json") {
            all_args.push("--json");
        }
        let output = self
            .command(&all_args)
            .output()
            .expect("command should execute");
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();

        assert!(
            !stderr.contains("panicked at"),
            "PANIC DETECTED on failure test for args '{args:?}':\n{stderr}"
        );
        assert!(
            !output.status.success(),
            "command '{args:?}' succeeded when expected to fail. Stdout: {stdout}"
        );

        (output.status.code(), stdout, stderr)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

// ============================================================================
// 1. MODELS ROUTE ADVERSARIAL STRESS TESTS
// ============================================================================

#[test]
fn test_adv_models_route_extreme_c_parameter() {
    let fix = Fixture::new();

    // Very small positive float
    let val = fix.run_success(&["models", "route", "--c", "0.000000001", "--rounds", "10"]);
    assert_eq!(val["command"], "models route");

    // Very large positive float
    let val2 = fix.run_success(&["models", "route", "--c", "1000000.0", "--rounds", "10"]);
    assert_eq!(val2["command"], "models route");

    // Negative c: must exit with error, zero panics
    fix.run_failure(&["models", "route", "--c", "-1.5"]);

    // Zero c: must exit with error, zero panics
    fix.run_failure(&["models", "route", "--c", "0.0"]);

    // Non-float string: must exit with error, zero panics
    fix.run_failure(&["models", "route", "--c", "not_a_float"]);
}

#[test]
fn test_adv_models_route_epsilon_boundaries_and_negatives() {
    let fix = Fixture::new();

    // Epsilon = 0.0 (pure exploitation)
    let val0 = fix.run_success(&[
        "models",
        "route",
        "--strategy",
        "epsilon-greedy",
        "--epsilon",
        "0.0",
        "--rounds",
        "20",
    ]);
    assert_eq!(val0["command"], "models route");

    // Epsilon = 1.0 (pure exploration)
    let val1 = fix.run_success(&[
        "models",
        "route",
        "--strategy",
        "epsilon-greedy",
        "--epsilon",
        "1.0",
        "--rounds",
        "20",
    ]);
    assert_eq!(val1["command"], "models route");

    // Negative epsilon must fail without panic
    fix.run_failure(&[
        "models",
        "route",
        "--strategy",
        "epsilon-greedy",
        "--epsilon",
        "-0.01",
    ]);

    // Epsilon > 1.0 must fail without panic
    fix.run_failure(&[
        "models",
        "route",
        "--strategy",
        "epsilon-greedy",
        "--epsilon",
        "1.01",
    ]);

    // Non-numeric epsilon must fail without panic
    fix.run_failure(&[
        "models",
        "route",
        "--strategy",
        "epsilon-greedy",
        "--epsilon",
        "invalid",
    ]);
}

#[test]
fn test_adv_models_route_rounds_edge_cases() {
    let fix = Fixture::new();

    // 0 rounds must fail without panic
    fix.run_failure(&["models", "route", "--rounds", "0"]);
    fix.run_failure(&["models", "route", "--pulls", "0"]);

    // Negative rounds must fail without panic
    fix.run_failure(&["models", "route", "--rounds", "-10"]);

    // 1 round must succeed
    let val1 = fix.run_success(&["models", "route", "--rounds", "1"]);
    assert_eq!(val1["rounds"], 1);

    // Large rounds (300) must succeed and satisfy regret bound
    let val300 = fix.run_success(&["models", "route", "--rounds", "300"]);
    assert_eq!(val300["rounds"], 300);
    assert_eq!(val300["regret"]["bound_satisfied"], true);
}

#[test]
fn test_adv_models_route_features_and_optimal_reward_bounds() {
    let fix = Fixture::new();

    // Optimal reward boundaries
    let v0 = fix.run_success(&[
        "models",
        "route",
        "--optimal-reward",
        "0.0",
        "--rounds",
        "5",
    ]);
    assert_eq!(v0["regret"]["optimal_reward_mean"], 0.0);

    let v1 = fix.run_success(&[
        "models",
        "route",
        "--optimal-reward",
        "1.0",
        "--rounds",
        "5",
    ]);
    assert_eq!(v1["regret"]["optimal_reward_mean"], 1.0);

    fix.run_failure(&["models", "route", "--optimal-reward", "-0.1"]);
    fix.run_failure(&["models", "route", "--optimal-reward", "1.1"]);

    // Complexity bounds
    fix.run_success(&["models", "route", "--complexity", "0.0", "--rounds", "5"]);
    fix.run_success(&["models", "route", "--complexity", "1.0", "--rounds", "5"]);
    fix.run_failure(&["models", "route", "--complexity", "-0.1"]);
    fix.run_failure(&["models", "route", "--complexity", "1.1"]);

    // Cache affinity bounds
    fix.run_success(&[
        "models",
        "route",
        "--cache-affinity",
        "0.0",
        "--rounds",
        "5",
    ]);
    fix.run_success(&[
        "models",
        "route",
        "--cache-affinity",
        "1.0",
        "--rounds",
        "5",
    ]);
    fix.run_failure(&["models", "route", "--cache-affinity", "-0.1"]);
    fix.run_failure(&["models", "route", "--cache-affinity", "1.1"]);

    // Max latency bounds
    fix.run_success(&[
        "models",
        "route",
        "--max-latency-ms",
        "1000000.0",
        "--rounds",
        "5",
    ]);
    fix.run_failure(&["models", "route", "--max-latency-ms", "0.0"]);
    fix.run_failure(&["models", "route", "--max-latency-ms", "-100.0"]);

    // Query length bounds
    fix.run_success(&["models", "route", "--len", "50000", "--rounds", "5"]);
    fix.run_failure(&["models", "route", "--query-length", "-1"]);
}

#[test]
fn test_adv_models_route_custom_arms_edge_cases() {
    let fix = Fixture::new();

    // Single solitary arm: must execute 100% of pulls on that arm
    let val_single = fix.run_success(&[
        "models",
        "route",
        "--arms",
        "solitary_model",
        "--rounds",
        "20",
    ]);
    assert_eq!(val_single["arm_count"], 1);
    assert_eq!(val_single["selected_arm"], "arm-solitary_model");
    assert_eq!(val_single["arms"][0]["pull_count"], 20);

    // Empty arms
    fix.run_failure(&["models", "route", "--arms", ""]);
    fix.run_failure(&["models", "route", "--arms", "  ,  ,,   "]);

    // Too many colons in arm definition
    fix.run_failure(&["models", "route", "--arms", "p1:p2:p3:p4"]);

    // 50 custom arms
    let arms = (0..50)
        .map(|i| format!("arm_{i}:model_{i}:prov_{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let val_50 = fix.run_success(&[
        "models",
        "route",
        "--arms",
        &arms,
        "--rounds",
        "60",
        "--strategy",
        "thompson",
    ]);
    assert_eq!(val_50["arm_count"], 50);
    assert_eq!(val_50["rounds"], 60);
    assert_eq!(val_50["regret"]["bound_satisfied"], true);
}

// ============================================================================
// 2. AUDIT MERKLE-DAG ADVERSARIAL STRESS TESTS
// ============================================================================

#[test]
fn test_adv_audit_empty_store_and_tree() {
    let fix = Fixture::new();
    let store = fix.data.join("empty_audit.json");
    let store_str = store.to_str().unwrap();

    // Proof on empty store must fail with graceful execution error, zero panics
    fix.run_failure(&["audit", "proof", "--index", "0", "--store", store_str]);

    // Tree on empty store must succeed with 0 leaves and is_acyclic true
    let tree_val = fix.run_success(&["audit", "tree", "--store", store_str]);
    assert_eq!(tree_val["leaf_count"], 0);
    assert_eq!(tree_val["tree_depth"], 0);
    assert_eq!(tree_val["is_acyclic"], true);
}

#[test]
fn test_adv_audit_single_leaf_roundtrip_and_out_of_bounds() {
    let fix = Fixture::new();
    let store = fix.data.join("single_leaf.json");
    let store_str = store.to_str().unwrap();

    // 1. Record exactly 1 leaf
    let r1 = fix.run_success(&[
        "audit",
        "record",
        "--payload",
        "isolated_genesis_event",
        "--store",
        store_str,
    ]);
    assert_eq!(r1["leaf_index"], 0);
    assert_eq!(r1["total_leaves"], 1);

    // 2. Proof for leaf 0: proof length is 0 (root == leaf), verified = true
    let proof_file = fix.data.join("proof_0.json");
    let proof_file_str = proof_file.to_str().unwrap();
    let p0 = fix.run_success(&[
        "audit",
        "proof",
        "--index",
        "0",
        "--store",
        store_str,
        "--output",
        proof_file_str,
    ]);
    assert_eq!(p0["leaf_index"], 0);
    assert_eq!(p0["proof_length"], 0);
    assert_eq!(p0["verified"], true);
    assert!(proof_file.exists());

    // 3. Verify single-leaf proof from file
    let v0 = fix.run_success(&["audit", "verify", "--proof-file", proof_file_str]);
    assert_eq!(v0["verified"], true);
    assert_eq!(v0["tamper_status"], "UNMODIFIED");

    // 4. Index out of bounds (index 1 on size 1 store)
    fix.run_failure(&["audit", "proof", "--index", "1", "--store", store_str]);

    // 5. Huge index out of bounds
    fix.run_failure(&["audit", "proof", "--index", "9999999", "--store", store_str]);

    // 6. Negative index
    fix.run_failure(&["audit", "proof", "--index", "-1", "--store", store_str]);
}

#[test]
fn test_adv_audit_corrupted_proof_verification() {
    let fix = Fixture::new();

    // Invalid JSON inline
    fix.run_failure(&["audit", "verify", "--proof", "{{{malformed_json_not_valid"]);

    // Empty JSON struct
    fix.run_failure(&["audit", "verify", "--proof", "{}"]);

    // Proof file pointing to non-existent file
    fix.run_failure(&[
        "audit",
        "verify",
        "--proof-file",
        "nonexistent_proof_999.json",
    ]);

    // Syntactically valid proof structure but cryptographically tampered:
    // Should execute successfully and report "tamper_status": "CORRUPTED_OR_TAMPERED"
    let fake_proof = serde_json::json!({
        "leaf_index": 0,
        "leaf_hash": vec![1u8; 32],
        "steps": [{
            "sibling_hash": vec![33u8; 32],
            "direction": "Right"
        }],
        "root_hash": vec![99u8; 32]
    });
    let fake_proof_str = fake_proof.to_string();

    let val = fix.run_success(&["audit", "verify", "--proof", &fake_proof_str]);
    assert_eq!(val["verified"], false);
    assert_eq!(val["tamper_status"], "CORRUPTED_OR_TAMPERED");
}

#[test]
fn test_adv_audit_corrupted_store_file_handling() {
    let fix = Fixture::new();
    let corrupt_store = fix.data.join("corrupt_audit.json");
    let corrupt_str = corrupt_store.to_str().unwrap();

    // 1. Truncated JSON in store
    fs::write(&corrupt_store, b"{\"version\": 1, \"leaves\": [").unwrap();
    fix.run_failure(&["audit", "tree", "--store", corrupt_str]);

    // 2. Invalid hex string in leaf payload
    fs::write(
        &corrupt_store,
        serde_json::json!({
            "version": 1,
            "leaves": [{
                "index": 0,
                "payload_text": "bad",
                "payload_hex": "not_hex_chars_xyz",
                "leaf_hash": "deadbeef",
                "timestamp_secs": 100
            }],
            "dag_entries": []
        })
        .to_string(),
    )
    .unwrap();
    fix.run_failure(&["audit", "tree", "--store", corrupt_str]);

    // 3. Odd-length hex string
    fs::write(
        &corrupt_store,
        serde_json::json!({
            "version": 1,
            "leaves": [{
                "index": 0,
                "payload_text": "bad",
                "payload_hex": "abc",
                "leaf_hash": "deadbeef",
                "timestamp_secs": 100
            }],
            "dag_entries": []
        })
        .to_string(),
    )
    .unwrap();
    fix.run_failure(&["audit", "tree", "--store", corrupt_str]);
}

#[test]
fn test_adv_audit_dag_nonexistent_parent_rejected() {
    let fix = Fixture::new();
    let store = fix.data.join("audit_dag.json");
    let store_str = store.to_str().unwrap();

    // Child node referencing nonexistent parent ID must fail cleanly
    fix.run_failure(&[
        "audit",
        "record",
        "--payload",
        "orphan_state",
        "--parent",
        "deadbeefdeadbeef0000000011112222",
        "--store",
        store_str,
    ]);
}

// ============================================================================
// 3. SPECULATIVE BENCHMARK & KV CACHE ADVERSARIAL STRESS TESTS
// ============================================================================

#[test]
fn test_adv_speculative_benchmark_extreme_alpha_zero_and_one() {
    let fix = Fixture::new();

    // Alpha = 0.0: Target model rejects all draft tokens; bonus tokens emitted
    let val_zero = fix.run_success(&[
        "runtime",
        "speculative",
        "benchmark",
        "--alpha",
        "0.0",
        "--tokens",
        "32",
        "--gamma",
        "4",
    ]);
    assert_eq!(val_zero["acceptance_probability_alpha"], 0.0);
    assert_eq!(val_zero["empirical_accepted_tokens"], 0);
    assert_eq!(val_zero["empirical_acceptance_rate"], 0.0);
    assert!(val_zero["total_tokens_simulated"].as_u64().unwrap() >= 32);

    // Alpha = 1.0: Target model accepts all draft tokens
    let val_one = fix.run_success(&[
        "runtime",
        "speculative",
        "benchmark",
        "--alpha",
        "1.0",
        "--tokens",
        "32",
        "--gamma",
        "4",
    ]);
    assert_eq!(val_one["acceptance_probability_alpha"], 1.0);
    assert_eq!(val_one["empirical_acceptance_rate"], 1.0);
    assert!(val_one["empirical_speedup"].as_f64().unwrap() > 2.0);

    // Alpha > 1.0 or < 0.0 must fail
    fix.run_failure(&["runtime", "speculative", "benchmark", "--alpha", "1.01"]);
    fix.run_failure(&["runtime", "speculative", "benchmark", "--alpha", "-0.01"]);
}

#[test]
fn test_adv_speculative_benchmark_extreme_latency_ratios() {
    let fix = Fixture::new();

    // Tiny ratio
    let val_fast = fix.run_success(&[
        "runtime",
        "speculative",
        "benchmark",
        "--latency-ratio",
        "0.0001",
        "--tokens",
        "32",
    ]);
    assert!(val_fast["empirical_speedup"].as_f64().unwrap() > 2.0);

    // Huge ratio (50x slower draft -> slowdown)
    let val_slow = fix.run_success(&[
        "runtime",
        "speculative",
        "benchmark",
        "--latency-ratio",
        "50.0",
        "--tokens",
        "32",
    ]);
    assert!(val_slow["empirical_speedup"].as_f64().unwrap() < 0.1);

    // Ratio <= 0 must fail
    fix.run_failure(&[
        "runtime",
        "speculative",
        "benchmark",
        "--latency-ratio",
        "0.0",
    ]);
    fix.run_failure(&[
        "runtime",
        "speculative",
        "benchmark",
        "--latency-ratio",
        "-0.1",
    ]);
}

#[test]
fn test_adv_speculative_benchmark_gamma_boundaries() {
    let fix = Fixture::new();

    // Gamma = 1 (min)
    let v1 = fix.run_success(&[
        "runtime",
        "speculative",
        "benchmark",
        "--gamma",
        "1",
        "--tokens",
        "16",
    ]);
    assert_eq!(v1["draft_window_gamma"], 1);

    // Gamma = 32 (max)
    let v32 = fix.run_success(&[
        "runtime",
        "speculative",
        "benchmark",
        "--gamma",
        "32",
        "--tokens",
        "64",
    ]);
    assert_eq!(v32["draft_window_gamma"], 32);

    // Gamma 0 and 33 must fail
    fix.run_failure(&["runtime", "speculative", "benchmark", "--gamma", "0"]);
    fix.run_failure(&["runtime", "speculative", "benchmark", "--gamma", "33"]);
}

#[test]
fn test_adv_kv_cache_sessions_boundaries() {
    let fix = Fixture::new();

    // Sessions = 0 must fail
    fix.run_failure(&["runtime", "kv-cache", "--sessions", "0"]);

    // Single session (1 session): no shared tokens, 1.0x compression, 0% savings
    let v_single = fix.run_success(&[
        "runtime",
        "kv-cache",
        "--sessions",
        "1",
        "--prefix-len",
        "512",
        "--suffix-len",
        "256",
    ]);
    assert_eq!(v_single["active_sessions"], 1);
    assert_eq!(v_single["total_tokens_shared"], 0);
    assert_eq!(v_single["empirical_compression_ratio"], 1.0);
    assert_eq!(v_single["memory_savings_percent"], 0.0);

    // 50 sessions
    let v50 = fix.run_success(&[
        "runtime",
        "kv-cache",
        "--sessions",
        "50",
        "--prefix-len",
        "1000",
        "--suffix-len",
        "200",
    ]);
    assert_eq!(v50["active_sessions"], 50);
    assert!(v50["empirical_compression_ratio"].as_f64().unwrap() > 4.0);
}

#[test]
fn test_adv_kv_cache_suffix_zero_and_high_prefix_fraction() {
    let fix = Fixture::new();

    // Suffix len = 0: 10 sessions all sharing 100% of prefix
    let v_zero_suffix = fix.run_success(&[
        "runtime",
        "kv-cache",
        "--sessions",
        "10",
        "--prefix-len",
        "500",
        "--suffix-len",
        "0",
    ]);
    assert_eq!(v_zero_suffix["suffix_length_tokens"], 0);
    assert_eq!(v_zero_suffix["empirical_compression_ratio"], 10.0);
    assert_eq!(v_zero_suffix["memory_savings_percent"], 90.0);

    // High prefix fraction: prefix 10,000, suffix 1
    let v_high_prefix = fix.run_success(&[
        "runtime",
        "kv-cache",
        "--sessions",
        "5",
        "--prefix-len",
        "10000",
        "--suffix-len",
        "1",
    ]);
    assert!(v_high_prefix["prefix_fraction_eta"].as_f64().unwrap() > 0.999);
    assert!(
        v_high_prefix["empirical_compression_ratio"]
            .as_f64()
            .unwrap()
            > 4.9
    );
}

#[test]
fn test_adv_kv_cache_invalid_arguments() {
    let fix = Fixture::new();

    // Prefix-len = 0 must fail
    fix.run_failure(&["runtime", "kv-cache", "--prefix-len", "0"]);

    // Bytes-per-token = 0 must fail
    fix.run_failure(&["runtime", "kv-cache", "--bytes-per-token", "0"]);

    // Positional arguments must fail
    fix.run_failure(&["runtime", "kv-cache", "invalid_positional"]);
}

// ============================================================================
// 4. EXTENDED ADVERSARIAL STRESS SUITE (DEEP EDGE CASES & TAMPER ORACLES)
// ============================================================================

#[test]
fn test_adv_models_route_prompt_extremes_and_seeds() {
    let fix = Fixture::new();

    // Empty prompt string
    let v_empty = fix.run_success(&["models", "route", "--prompt", "", "--rounds", "10"]);
    assert_eq!(v_empty["context"]["query_length"], 0);

    // Huge prompt string (30,000 chars)
    let huge_prompt = "fn analyze_quantum_astrophysics() { let x = 42; } ".repeat(600);
    let v_huge = fix.run_success(&[
        "models",
        "route",
        "--prompt",
        &huge_prompt,
        "--rounds",
        "10",
    ]);
    assert_eq!(v_huge["context"]["query_length"], huge_prompt.len());
    assert_eq!(v_huge["regret"]["bound_satisfied"], true);

    // Seed = 0
    let v_seed0 = fix.run_success(&["models", "route", "--seed", "0", "--rounds", "5"]);
    assert_eq!(v_seed0["seed"], 0);

    // Seed = u64::MAX (18446744073709551615)
    let v_seed_max = fix.run_success(&[
        "models",
        "route",
        "--seed",
        "18446744073709551615",
        "--rounds",
        "5",
    ]);
    assert_eq!(v_seed_max["seed"], 18446744073709551615u64);
}

#[test]
fn test_adv_models_route_duplicate_and_whitespace_arms() {
    let fix = Fixture::new();

    // Arms with duplicate ID (last should overwrite in map without panic)
    let v_dup = fix.run_success(&[
        "models",
        "route",
        "--arms",
        "same_id:model_a:local,same_id:model_b:remote",
        "--rounds",
        "10",
    ]);
    assert_eq!(v_dup["arm_count"], 1);

    // Arms with heavy whitespace around separators
    let v_space = fix.run_success(&[
        "models",
        "route",
        "--arms",
        "  arm_1 : model_1 : prov_1  ,   arm_2 : model_2 : prov_2  ",
        "--rounds",
        "10",
    ]);
    assert_eq!(v_space["arm_count"], 2);
}

#[test]
fn test_adv_audit_odd_leaf_counts_proof_coverage() {
    let fix = Fixture::new();

    // Test Merkle balanced binary tree generation across odd leaf counts: 3, 5, 7, 9
    for count in [3, 5, 7, 9] {
        let store = fix.data.join(format!("audit_odd_{count}.json"));
        let store_str = store.to_str().unwrap();

        for i in 0..count {
            fix.run_success(&[
                "audit",
                "record",
                "--payload",
                &format!("odd_event_{count}_{i}"),
                "--store",
                store_str,
            ]);
        }

        // Verify that EVERY leaf index produces a valid proof that independently verifies
        let expected_depth = (count as f64).log2().ceil() as usize;
        for i in 0..count {
            let p = fix.run_success(&[
                "audit",
                "proof",
                "--index",
                &i.to_string(),
                "--store",
                store_str,
            ]);
            assert_eq!(p["leaf_index"], i);
            assert_eq!(p["proof_length"], expected_depth);
            assert_eq!(p["verified"], true);
        }
    }
}

#[test]
fn test_adv_audit_dag_branch_merge_and_payload_tamper_detection() {
    let fix = Fixture::new();
    let store = fix.data.join("audit_dag_tamper.json");
    let store_str = store.to_str().unwrap();

    // 1. Genesis node A
    let r_a = fix.run_success(&[
        "audit",
        "record",
        "--payload",
        "state_genesis_a",
        "--parent",
        "",
        "--store",
        store_str,
    ]);
    let id_a = r_a["dag_node"]["id"].as_str().unwrap().to_string();

    // 2. Branch B from A
    let r_b = fix.run_success(&[
        "audit",
        "record",
        "--payload",
        "state_branch_b",
        "--parent",
        &id_a,
        "--store",
        store_str,
    ]);
    let id_b = r_b["dag_node"]["id"].as_str().unwrap().to_string();

    // 3. Branch C from A
    let r_c = fix.run_success(&[
        "audit",
        "record",
        "--payload",
        "state_branch_c",
        "--parent",
        &id_a,
        "--store",
        store_str,
    ]);
    let id_c = r_c["dag_node"]["id"].as_str().unwrap().to_string();

    // 4. Merge D from B and C
    let parents_bc = format!("{id_b},{id_c}");
    let r_d = fix.run_success(&[
        "audit",
        "record",
        "--payload",
        "state_merge_d",
        "--parent",
        &parents_bc,
        "--store",
        store_str,
    ]);
    let id_d = r_d["dag_node"]["id"].as_str().unwrap().to_string();

    // Prior to tampering, lineage of D is verified valid
    let v_d_before =
        fix.run_success(&["audit", "verify", "--dag-node", &id_d, "--store", store_str]);
    assert_eq!(v_d_before["verified"], true);
    assert_eq!(v_d_before["tamper_status"], "UNMODIFIED");

    // 5. Tamper: Manually modify node C's payload in the store file
    let mut store_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&store).unwrap()).unwrap();
    for entry in store_json["dag_entries"].as_array_mut().unwrap() {
        if entry["node"]["id"] == id_c {
            // Alter one character of the payload hex
            entry["payload_hex"] = serde_json::json!("ffffffffffffffff");
        }
    }
    fs::write(&store, serde_json::to_string_pretty(&store_json).unwrap()).unwrap();

    // 6. Verify tampered node or descendants: loading corrupted lineage MUST fail with execution error (zero panic)
    let (code_c, _stdout_c, stderr_c) =
        fix.run_failure(&["audit", "verify", "--dag-node", &id_c, "--store", store_str]);
    assert!(code_c.is_some());
    assert!(!stderr_c.contains("panicked at"));

    let (code_d, _stdout_d, stderr_d) =
        fix.run_failure(&["audit", "verify", "--dag-node", &id_d, "--store", store_str]);
    assert!(code_d.is_some());
    assert!(!stderr_d.contains("panicked at"));
}

#[test]
fn test_adv_audit_record_file_and_payload_conflicts() {
    let fix = Fixture::new();

    // 1. Missing file path
    fix.run_failure(&[
        "audit",
        "record",
        "--file",
        "nonexistent_event_data_file.bin",
    ]);

    // 2. Both payload and file specified simultaneously
    let dummy_file = fix.workspace.join("dummy.txt");
    fs::write(&dummy_file, b"test").unwrap();
    fix.run_failure(&[
        "audit",
        "record",
        "--payload",
        "foo",
        "--file",
        dummy_file.to_str().unwrap(),
    ]);

    // 3. Neither payload nor file specified
    fix.run_failure(&["audit", "record"]);
}

#[test]
fn test_adv_hardware_and_backup_aliases_stress() {
    let fix = Fixture::new();

    // Hardware alias for speculative benchmark
    let h_spec = fix.run_success(&[
        "hardware",
        "speculative",
        "--tokens",
        "32",
        "--gamma",
        "3",
        "--alpha",
        "0.75",
    ]);
    assert_eq!(h_spec["command"], "hardware speculative");
    assert!(h_spec["empirical_speedup"].as_f64().unwrap() > 1.0);

    // Hardware alias for kv-cache
    let h_kv = fix.run_success(&[
        "hardware",
        "kv-cache",
        "--sessions",
        "4",
        "--prefix-len",
        "256",
        "--suffix-len",
        "128",
    ]);
    assert_eq!(h_kv["command"], "hardware kv-cache");
    assert_eq!(h_kv["active_sessions"], 4);

    // Backup audit alias
    let store = fix.data.join("backup_alias_store.json");
    let store_str = store.to_str().unwrap();

    let b_rec = fix.run_success(&[
        "backup",
        "audit",
        "record",
        "--payload",
        "backup_aliased_event",
        "--store",
        store_str,
    ]);
    assert_eq!(b_rec["command"], "audit record");
    assert_eq!(b_rec["leaf_index"], 0);
}
