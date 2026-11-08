#![forbid(unsafe_code)]

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(700);

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
            "pandora-phase1820-test-{}-{timestamp}-{sequence}",
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

    fn run_failure(&self, args: &[&str]) -> String {
        let output = self.command(args).output().expect("command should execute");
        assert!(
            !output.status.success(),
            "command '{:?}' unexpectedly succeeded: {}",
            args,
            String::from_utf8_lossy(&output.stdout)
        );
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        if !stderr.is_empty() { stderr } else { stdout }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn test_cli_mfg_execution_and_theorems() {
    let fixture = Fixture::new();

    let json = fixture.run_success(&[
        "runtime",
        "mfg",
        "--resources",
        "3",
        "--temperature",
        "1.5",
        "--json",
    ]);

    assert_eq!(json["command"], "runtime mfg");
    let data = &json;
    assert_eq!(data["num_resources"], 3);
    assert_eq!(data["temperature_tau"], 1.5);
    assert!(
        data["fixed_point"]["is_strict_contraction"]
            .as_bool()
            .unwrap()
    );
    assert!(data["poa"]["bound_satisfied"].as_bool().unwrap());

    // Price of Anarchy <= 4/3
    let poa = data["poa"]["price_of_anarchy"].as_f64().unwrap();
    assert!(poa <= 4.0 / 3.0 + 1e-4);

    // Validation error
    let err = fixture.run_failure(&["runtime", "mfg", "--resources", "1", "--json"]);
    assert!(err.contains("at least 2"));
}

#[test]
fn test_cli_pac_bayes_execution_and_theorems() {
    let fixture = Fixture::new();

    let json = fixture.run_success(&[
        "runtime",
        "pac-bayes",
        "--samples",
        "400",
        "--target-risk",
        "0.15",
        "--json",
    ]);

    assert_eq!(json["command"], "runtime pac-bayes");
    let data = &json;
    assert_eq!(data["target_risk"], 0.15);
    assert!(data["certified_risk_bound"].as_f64().unwrap() <= 0.15);
    assert_eq!(data["verdict"], "PromoteToProduction");
    assert!(data["optimal_temperature_beta"].as_f64().unwrap() > 0.0);
}

#[test]
fn test_cli_tropical_execution_and_theorems() {
    let fixture = Fixture::new();

    // Pipeline DAG test
    let json_pipe = fixture.run_success(&[
        "runtime",
        "tropical",
        "--nodes",
        "4",
        "--topology",
        "pipeline",
        "--json",
    ]);

    assert_eq!(json_pipe["command"], "runtime tropical");
    assert_eq!(json_pipe["nodes"], 4);
    assert_eq!(json_pipe["critical_path_makespan"], 60.0);
    assert!(!json_pipe["has_positive_cycle"].as_bool().unwrap());

    // Cycle recurrent test
    let json_cycle = fixture.run_success(&[
        "runtime",
        "tropical",
        "--nodes",
        "3",
        "--topology",
        "cycle",
        "--sla-threshold",
        "50.0",
        "--json",
    ]);

    assert_eq!(json_cycle["command"], "runtime tropical");
    assert!(json_cycle["sla_satisfied"].as_bool().unwrap());
    assert!(json_cycle["maximum_cycle_mean"].as_f64().unwrap() > 0.0);
}
