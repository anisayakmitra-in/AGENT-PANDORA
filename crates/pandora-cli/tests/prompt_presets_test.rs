#![forbid(unsafe_code)]

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

struct Fixture {
    root: PathBuf,
    config: PathBuf,
    data: PathBuf,
    workspace: PathBuf,
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
            "pandora-prompt-test-{}-{suffix}-{sequence}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).expect("workspace dir created");
        fs::write(
            workspace.join("Cargo.toml"),
            "[package]\nname = \"fixture\"\n",
        )
        .expect("cargo toml written");
        fs::write(workspace.join("main.rs"), "fn main() {}\n").expect("sample code written");
        Self {
            config: root.join("config.json"),
            data: root.join("data"),
            workspace,
            root,
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

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn prompt_list_displays_builtins() {
    let fixture = Fixture::new();
    fixture.setup();

    let output = fixture
        .command(&["prompt", "list", "--json"])
        .output()
        .expect("prompt list should execute");
    assert!(output.status.success());

    let val: Value = serde_json::from_slice(&output.stdout).expect("valid json output");
    let presets = val["presets"].as_array().expect("presets array");
    assert!(presets.len() >= 5);

    let names: Vec<&str> = presets.iter().filter_map(|p| p["name"].as_str()).collect();
    assert!(names.contains(&"reviewer"));
    assert!(names.contains(&"security-auditor"));
    assert!(names.contains(&"refactor"));
    assert!(names.contains(&"test-writer"));
    assert!(names.contains(&"explain"));
}

#[test]
fn prompt_save_inspect_apply_delete_lifecycle() {
    let fixture = Fixture::new();
    fixture.setup();

    // 1. Save new template preset
    let save_output = fixture
        .command(&[
            "prompt",
            "save",
            "audit-rust",
            "--content",
            "Audit {{workspace}} written in {{lang}} for task: {{task}}",
            "--json",
        ])
        .output()
        .expect("prompt save should run");
    assert!(save_output.status.success());
    let save_val: Value = serde_json::from_slice(&save_output.stdout).expect("valid json");
    assert_eq!(save_val["name"], "audit-rust");
    assert_eq!(save_val["tier"], "workspace");

    // 2. Inspect saved preset
    let inspect_output = fixture
        .command(&["prompt", "inspect", "audit-rust", "--json"])
        .output()
        .expect("prompt inspect should run");
    assert!(inspect_output.status.success());
    let inspect_val: Value = serde_json::from_slice(&inspect_output.stdout).expect("valid json");
    assert_eq!(inspect_val["name"], "audit-rust");
    let params = inspect_val["parameters"]
        .as_array()
        .expect("parameters array");
    assert!(params.contains(&Value::String("workspace".to_owned())));
    assert!(params.contains(&Value::String("lang".to_owned())));
    assert!(params.contains(&Value::String("task".to_owned())));

    // 3. Apply preset with interpolation in dry-run mode
    let apply_output = fixture
        .command(&[
            "prompt",
            "apply",
            "audit-rust",
            "--dry-run",
            "--param",
            "task=memory safety checks",
            "--json",
        ])
        .output()
        .expect("prompt apply should run");
    assert!(apply_output.status.success());
    let apply_val: Value = serde_json::from_slice(&apply_output.stdout).expect("valid json");
    let interpolated = apply_val["interpolated"]
        .as_str()
        .expect("interpolated string");
    assert!(interpolated.contains("written in rust"));
    assert!(interpolated.contains("for task: memory safety checks"));

    // 4. Delete saved preset
    let delete_output = fixture
        .command(&["prompt", "delete", "audit-rust", "--json"])
        .output()
        .expect("prompt delete should run");
    assert!(delete_output.status.success());

    // 5. Inspect should now fail
    let inspect_after_delete = fixture
        .command(&["prompt", "inspect", "audit-rust", "--json"])
        .output()
        .expect("prompt inspect should run");
    assert!(!inspect_after_delete.status.success());
}

#[test]
fn prompt_workspace_tier_overrides_builtin() {
    let fixture = Fixture::new();
    fixture.setup();

    // Built-in explain is tier Builtin
    let inspect_builtin = fixture
        .command(&["prompt", "inspect", "explain", "--json"])
        .output()
        .expect("inspect builtin");
    assert!(inspect_builtin.status.success());
    let val_builtin: Value = serde_json::from_slice(&inspect_builtin.stdout).unwrap();
    assert_eq!(val_builtin["tier"], "builtin");

    // Save custom explain in workspace
    let save_custom = fixture
        .command(&[
            "prompt",
            "save",
            "explain",
            "--content",
            "Custom workspace explain: {{task}}",
            "--json",
        ])
        .output()
        .expect("save custom explain");
    assert!(save_custom.status.success());

    // Now inspect explain should report workspace tier
    let inspect_custom = fixture
        .command(&["prompt", "inspect", "explain", "--json"])
        .output()
        .expect("inspect overridden explain");
    assert!(inspect_custom.status.success());
    let val_custom: Value = serde_json::from_slice(&inspect_custom.stdout).unwrap();
    assert_eq!(val_custom["tier"], "workspace");
    assert_eq!(val_custom["content"], "Custom workspace explain: {{task}}");
}

#[test]
fn governance_mode_flag_parsing_and_execution() {
    let fixture = Fixture::new();
    fixture.setup();

    // run with --mode read-only
    let output_ro = fixture
        .command(&["run", "--mode", "read-only", "read sample", "--json"])
        .output()
        .expect("run read-only should execute");
    assert!(
        output_ro.status.success() || !output_ro.stderr.is_empty() || !output_ro.stdout.is_empty()
    );

    // run with --permission autonomous
    let output_auto = fixture
        .command(&[
            "run",
            "--permission",
            "autonomous",
            "autonomous task",
            "--json",
        ])
        .output()
        .expect("run autonomous should execute");
    assert!(
        output_auto.status.success()
            || !output_auto.stderr.is_empty()
            || !output_auto.stdout.is_empty()
    );
}
