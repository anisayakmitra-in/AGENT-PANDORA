#![forbid(unsafe_code)]

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(100);

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
            "pandora-phase68-test-{}-{timestamp}-{sequence}",
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
fn test_models_route_ucb1_and_regret_bound() {
    let fixture = Fixture::new();

    let val = fixture.run_success(&[
        "models",
        "route",
        "--strategy",
        "ucb1",
        "--c",
        "1.414",
        "--rounds",
        "50",
        "--seed",
        "42",
        "--json",
    ]);

    assert_eq!(val["command"], "models route");
    assert_eq!(val["strategy"], "ucb1");
    assert_eq!(val["rounds"], 50);
    assert_eq!(val["arm_count"], 4);
    assert!(val["regret"]["bound_satisfied"].as_bool().unwrap());
    assert!(!val["selected_arm"].as_str().unwrap().is_empty());

    let empirical = val["regret"]["cumulative_empirical_regret"]
        .as_f64()
        .unwrap();
    let minimax = val["regret"]["theoretical_minimax_bound"].as_f64().unwrap();
    assert!(
        empirical <= minimax,
        "Empirical regret {empirical} must be bounded by minimax bound {minimax}"
    );
}

#[test]
fn test_models_route_thompson_sampling_with_prompt_features() {
    let fixture = Fixture::new();

    let val = fixture.run_success(&[
        "models",
        "route",
        "--strategy",
        "thompson",
        "--prompt",
        "pub fn verify_zk_snark_proof() -> bool { /* complex */ }",
        "--complexity",
        "0.85",
        "--rounds",
        "30",
        "--seed",
        "1337",
        "--json",
    ]);

    assert_eq!(val["strategy"], "thompson");
    assert_eq!(val["rounds"], 30);
    assert!((val["context"]["task_complexity"].as_f64().unwrap() - 0.85).abs() < 1e-4);
    assert!(val["regret"]["bound_satisfied"].as_bool().unwrap());

    let arms = val["arms"].as_array().unwrap();
    assert_eq!(arms.len(), 4);
    for arm in arms {
        assert!(arm["alpha"].as_f64().unwrap() >= 1.0);
        assert!(arm["beta"].as_f64().unwrap() >= 1.0);
    }
}

#[test]
fn test_models_route_epsilon_greedy_with_custom_arms() {
    let fixture = Fixture::new();

    let val = fixture.run_success(&[
        "models",
        "route",
        "--strategy",
        "epsilon-greedy",
        "--epsilon",
        "0.15",
        "--arms",
        "qwen:qwen2.5-coder:ollama,claude:claude-3-5:anthropic",
        "--rounds",
        "25",
        "--json",
    ]);

    assert_eq!(val["strategy"], "epsilon-greedy");
    assert_eq!(val["arm_count"], 2);
    let arms = val["arms"].as_array().unwrap();
    let ids: Vec<&str> = arms.iter().filter_map(|a| a["id"].as_str()).collect();
    assert!(ids.contains(&"qwen"));
    assert!(ids.contains(&"claude"));
}

#[test]
fn test_runtime_speculative_decoding_benchmark() {
    let fixture = Fixture::new();

    let val = fixture.run_success(&[
        "runtime",
        "speculative",
        "benchmark",
        "--gamma",
        "4",
        "--alpha",
        "0.85",
        "--latency-ratio",
        "0.10",
        "--tokens",
        "64",
        "--json",
    ]);

    assert_eq!(val["command"], "runtime speculative benchmark");
    assert_eq!(val["draft_window_gamma"], 4);
    assert!((val["acceptance_probability_alpha"].as_f64().unwrap() - 0.85).abs() < 1e-4);
    assert!(val["total_tokens_simulated"].as_u64().unwrap() >= 64);
    assert!(val["empirical_speedup"].as_f64().unwrap() > 1.0);

    let theo_speedup = val["theoretical_speedup"].as_f64().unwrap();
    let opt_gamma = val["optimal_draft_length"].as_u64().unwrap();
    assert!(theo_speedup > 1.5);
    assert!((1..=16).contains(&opt_gamma));
}

#[test]
fn test_runtime_radix_kv_cache_telemetry() {
    let fixture = Fixture::new();

    let val = fixture.run_success(&[
        "runtime",
        "kv-cache",
        "--sessions",
        "8",
        "--prefix-len",
        "512",
        "--suffix-len",
        "256",
        "--bytes-per-token",
        "2048",
        "--json",
    ]);

    assert_eq!(val["command"], "runtime kv-cache");
    assert_eq!(val["active_sessions"], 8);
    assert_eq!(val["prefix_length_tokens"], 512);
    assert_eq!(val["suffix_length_tokens"], 256);
    assert_eq!(val["total_tokens_requested"], 6144);
    assert_eq!(val["total_tokens_shared"], 3584);

    let emp_cr = val["empirical_compression_ratio"].as_f64().unwrap();
    let theo_cr = val["theoretical_compression_ratio"].as_f64().unwrap();
    let asym_cr = val["asymptotic_compression_ratio"].as_f64().unwrap();
    assert!((emp_cr - 2.40).abs() < 1e-4);
    assert!((theo_cr - 2.40).abs() < 1e-4);
    assert!((asym_cr - 3.00).abs() < 1e-4);

    let savings = val["memory_savings_percent"].as_f64().unwrap();
    assert!((savings - (7.0 / 12.0 * 100.0)).abs() < 1e-4);
}

#[test]
fn test_hardware_aliases_parity() {
    let fixture = Fixture::new();

    let val_hw_spec = fixture.run_success(&[
        "hardware",
        "speculative",
        "--gamma",
        "5",
        "--tokens",
        "32",
        "--json",
    ]);
    assert_eq!(val_hw_spec["command"], "hardware speculative");
    assert_eq!(val_hw_spec["draft_window_gamma"], 5);

    let val_hw_kv = fixture.run_success(&[
        "hardware",
        "kv-cache",
        "--sessions",
        "4",
        "--prefix-len",
        "256",
        "--suffix-len",
        "128",
        "--json",
    ]);
    assert_eq!(val_hw_kv["command"], "hardware kv-cache");
    assert_eq!(val_hw_kv["active_sessions"], 4);
    assert_eq!(val_hw_kv["prefix_length_tokens"], 256);
}

#[test]
fn test_backward_compatibility_preserved() {
    let fixture = Fixture::new();

    let val_models = fixture.run_success(&["models", "--json"]);
    assert_eq!(val_models["command"], "models");

    let val_runtime = fixture.run_success(&["runtime", "engines", "list", "--json"]);
    assert_eq!(val_runtime["command"], "runtime engines list");
    assert!(val_runtime["count"].as_u64().unwrap() > 0);
}
