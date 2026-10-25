#![forbid(unsafe_code)]

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(100);

struct Fixture {
    config: PathBuf,
    data: PathBuf,
    workspace: PathBuf,
    _root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock available")
            .as_nanos();
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let temp_root = std::env::temp_dir()
            .canonicalize()
            .expect("temp dir canonical path");
        let root = temp_root.join(format!(
            "pandora-gap-closure-{}-{suffix}-{sequence}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).expect("workspace dir created");
        fs::write(workspace.join("test.txt"), "hello world\n").expect("test file written");
        Self {
            config: root.join("config.json"),
            data: root.join("data"),
            workspace,
            _root: root,
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_pandora"));
        cmd.args(args)
            .env("PANDORA_CONFIG", &self.config)
            .env("PANDORA_DATA_DIR", &self.data)
            .env("PANDORA_WORKSPACE", &self.workspace)
            .env_remove("PANDORA_PROVIDER_URL");
        cmd
    }

    fn setup(&self) {
        let output = self
            .command(&[
                "setup",
                "--provider-url",
                "http://127.0.0.1:4317/v1",
                "--json",
            ])
            .output()
            .expect("setup command should succeed");
        assert!(output.status.success(), "setup must succeed");
    }
}

#[test]
fn test_tool_profile_options_accepted_by_cli() {
    let fixture = Fixture::new();
    fixture.setup();

    // Verify invalid tool profile is rejected with clean usage error
    let out_bad = fixture
        .command(&[
            "run",
            "--agent",
            "--tool-profile",
            "invalid_profile",
            "test task",
            "--json",
        ])
        .output()
        .expect("run command should execute");
    assert!(!out_bad.status.success());
    let raw = if !out_bad.stdout.is_empty() {
        &out_bad.stdout
    } else {
        &out_bad.stderr
    };
    let err_val: Value = serde_json::from_slice(raw).unwrap_or_default();
    assert!(
        err_val["message"]
            .as_str()
            .unwrap_or_default()
            .contains("unknown tool profile")
    );

    // Verify valid tool profile options ("minimal", "coding", "none", "full") parse cleanly
    for profile in ["minimal", "coding", "none", "full"] {
        let out = fixture
            .command(&[
                "run",
                "--agent",
                "--tool-profile",
                profile,
                "--tools",
                "workspace.read,workspace.status",
                "inspect workspace",
                "--json",
            ])
            .output()
            .expect("run command should execute");
        // Even if mock provider fails connecting, the argument parsing must succeed
        let stderr_str = String::from_utf8_lossy(&out.stderr);
        assert!(
            !stderr_str.contains("unexpected option")
                && !stderr_str.contains("unknown option")
                && !stderr_str.contains("unknown tool profile"),
            "tool profile '{profile}' must be recognized by CLI options parser, got: {stderr_str}"
        );
    }
}

#[test]
fn test_cron_positional_add_and_execution() {
    let fixture = Fixture::new();
    fixture.setup();

    // Positional short form: cron add "<schedule>" "<command>"
    let add_out = fixture
        .command(&[
            "cron",
            "add",
            "every 1h",
            "verify repository state",
            "--json",
        ])
        .output()
        .expect("cron add should execute");
    assert!(
        add_out.status.success(),
        "cron add must succeed: {}",
        String::from_utf8_lossy(&add_out.stderr)
    );

    let add_val: Value = serde_json::from_slice(&add_out.stdout).expect("valid json output");
    assert_eq!(add_val["job"]["schedule_expr"], "every 1h");
    assert_eq!(add_val["job"]["command"], "verify repository state");
    let job_id = add_val["job"]["id"].as_str().expect("job id");

    // Run job explicitly
    let run_out = fixture
        .command(&["cron", "run", job_id, "--json"])
        .output()
        .expect("cron run should execute");
    assert!(run_out.status.success());
    let run_val: Value = serde_json::from_slice(&run_out.stdout).expect("valid run json");
    assert_eq!(run_val["job"]["runs_count"], 1);
    assert_eq!(run_val["result"]["success"], true);
}

#[test]
fn test_setup_bare_invocation_help_or_json() {
    let fixture = Fixture::new();

    // Invoking setup with --json non-interactively succeeds
    let out = fixture
        .command(&["setup", "--preset", "ollama", "--json"])
        .output()
        .expect("setup command should execute");
    assert!(out.status.success());
    let val: Value = serde_json::from_slice(&out.stdout).expect("valid json output");
    assert_eq!(val["provider_model"], "qwen2.5-coder:32b");
}
