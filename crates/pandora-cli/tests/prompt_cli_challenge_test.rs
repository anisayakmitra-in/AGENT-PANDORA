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
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock available")
            .as_nanos();
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let temp_root = std::env::temp_dir()
            .canonicalize()
            .expect("temp dir canonical path");
        let root = temp_root.join(format!(
            "pandora-prompt-challenge-{}-{suffix}-{sequence}",
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

// ---------------------------------------------------------------------------
// Challenge 1: --global flag behavior in save, delete, list
// ---------------------------------------------------------------------------

#[test]
fn test_cli_save_with_global_flag_behavior() {
    let fixture = Fixture::new();
    fixture.setup();

    let output = fixture
        .command(&[
            "prompt",
            "save",
            "test-global",
            "--content",
            "my content",
            "--global",
            "--json",
        ])
        .output()
        .expect("command runs");

    assert!(
        output.status.success(),
        "save --global must succeed cleanly"
    );
    let val: Value = serde_json::from_slice(&output.stdout).expect("valid json output");
    assert_eq!(val["name"], "test-global");
    assert_eq!(val["tier"], "user");
}

#[test]
fn test_cli_delete_with_global_flag_behavior() {
    let fixture = Fixture::new();
    fixture.setup();

    // Pre-save to global tier
    let save_output = fixture
        .command(&[
            "prompt",
            "save",
            "test-global",
            "--content",
            "my content",
            "--global",
            "--json",
        ])
        .output()
        .expect("save runs");
    assert!(save_output.status.success());

    let output = fixture
        .command(&["prompt", "delete", "test-global", "--global", "--json"])
        .output()
        .expect("command runs");

    assert!(
        output.status.success(),
        "delete --global must succeed cleanly"
    );
    let val: Value = serde_json::from_slice(&output.stdout).expect("valid json output");
    assert_eq!(val["name"], "test-global");
    assert_eq!(val["deleted"], true);
}

#[test]
fn test_cli_list_with_global_option_behavior() {
    let fixture = Fixture::new();
    fixture.setup();

    let output = fixture
        .command(&["prompt", "list", "--global", "--json"])
        .output()
        .expect("command runs");

    assert!(
        output.status.success(),
        "prompt list --global must succeed cleanly"
    );
}

// ---------------------------------------------------------------------------
// Challenge 2: Arbitrary file deletion via path traversal in `prompt delete`
// ---------------------------------------------------------------------------

#[test]
fn test_cli_delete_path_traversal_vulnerability() {
    let fixture = Fixture::new();
    fixture.setup();

    // Create a target json file outside .pandora/prompts/
    // e.g. in workspace/.pandora/victim.json
    let pandora_dir = fixture.workspace.join(".pandora");
    fs::create_dir_all(pandora_dir.join("prompts")).expect("created prompts dir");
    let victim = pandora_dir.join("victim.json");
    fs::write(&victim, b"{\"secret_key\": \"DO_NOT_DELETE\"}").expect("wrote victim");
    assert!(victim.exists(), "victim file must exist before delete");

    // Call CLI delete with path traversal: "../victim"
    let output = fixture
        .command(&["prompt", "delete", "../victim"])
        .output()
        .expect("command runs");

    assert!(!output.status.success(), "delete with traversal must fail");
    let victim_exists_after = victim.exists();
    assert!(
        victim_exists_after,
        "SECURITY INVARIANT VERIFIED: victim.json was NOT deleted outside prompts directory!"
    );
}

// ---------------------------------------------------------------------------
// Challenge 3: Parameter interpolation in CLI apply
// ---------------------------------------------------------------------------

#[test]
fn test_cli_apply_repeated_param_succeeds() {
    let fixture = Fixture::new();
    fixture.setup();

    // Builtin refactor requires workspace, target, task, lang
    // Supplying target and lang via multiple --param flags
    let output = fixture
        .command(&[
            "prompt",
            "apply",
            "refactor",
            "do task",
            "--dry-run",
            "--param",
            "target=crates/foo",
            "--param",
            "lang=rust",
            "--json",
        ])
        .output()
        .expect("command runs");

    assert!(
        output.status.success(),
        "apply with multiple --param flags must succeed"
    );
    let val: Value = serde_json::from_slice(&output.stdout).expect("valid json output");
    let interpolated = val["interpolated"].as_str().expect("interpolated string");
    assert!(interpolated.contains("crates/foo"));
    assert!(interpolated.contains("rust"));
    assert!(interpolated.contains("do task"));
}

#[test]
fn test_cli_apply_param_value_with_commas() {
    let fixture = Fixture::new();
    fixture.setup();

    let output = fixture
        .command(&[
            "prompt",
            "apply",
            "refactor",
            "do task",
            "--dry-run",
            "--param",
            "target=crates/a,crates/b",
            "--param",
            "lang=rust",
            "--json",
        ])
        .output()
        .expect("command runs");

    assert!(
        output.status.success(),
        "apply with comma in param value must succeed"
    );
    let val: Value = serde_json::from_slice(&output.stdout).expect("valid json output");
    let interpolated = val["interpolated"].as_str().expect("interpolated string");
    assert!(interpolated.contains("crates/a,crates/b"));
}

#[test]
fn test_cli_apply_missing_required_param_fails_cleanly() {
    let fixture = Fixture::new();
    fixture.setup();

    let output = fixture
        .command(&["prompt", "apply", "refactor", "do task", "--dry-run"])
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("missing required template parameter '{{target}}'"));
}

#[test]
fn test_cli_inspect_nonexistent_returns_error() {
    let fixture = Fixture::new();
    fixture.setup();

    let output = fixture
        .command(&["prompt", "inspect", "does-not-exist", "--json"])
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    let val: Value = serde_json::from_slice(&output.stdout).expect("valid json error");
    assert_eq!(val["code"], "configuration_error");
    assert!(val["message"].as_str().unwrap().contains("not found"));
}
