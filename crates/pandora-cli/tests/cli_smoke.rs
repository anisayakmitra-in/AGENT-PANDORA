use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
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
            .expect("system clock should be available")
            .as_nanos();
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "pandora-cli-smoke-{}-{suffix}-{sequence}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).expect("workspace should be created");
        fs::write(workspace.join("README.md"), "fixture\n").expect("fixture should be written");
        Self {
            config: root.join("config.json"),
            data: root.join("data"),
            workspace,
            root,
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_pandora"));
        command
            .args(args)
            .env("PANDORA_CONFIG", &self.config)
            .env("PANDORA_DATA_DIR", &self.data)
            .env("PANDORA_WORKSPACE", &self.workspace)
            .env_remove("PANDORA_PROVIDER_URL");
        command
    }

    fn setup(&self) -> Value {
        let mut output = self.command(&[
            "setup",
            "--provider-url",
            "http://127.0.0.1:4317/v1",
            "--json",
        ]);
        let output = output.output().expect("setup should start");
        assert_success(&output);
        parse_json(&output)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn setup_and_read_only_run_return_versioned_json() {
    let fixture = Fixture::new();
    let setup = fixture.setup();
    assert_eq!(setup["version"], "0.1");
    assert_eq!(setup["command"], "setup");

    let output = fixture
        .command(&["run", "read:README.md", "--json"])
        .output()
        .expect("run should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["version"], "0.1");
    assert_eq!(response["command"], "run");
    assert_eq!(response["status"], "completed");
    assert_eq!(response["output"], "fixture\n");
    assert!(
        !response["session_id"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );
}

#[test]
fn patch_run_returns_approval_required_without_writing() {
    let fixture = Fixture::new();
    fixture.setup();

    let output = fixture
        .command(&["run", "patch:README.md:changed", "--json"])
        .output()
        .expect("run should start");
    assert_eq!(output.status.code(), Some(40));
    let response = parse_json(&output);
    assert_eq!(response["version"], "0.1");
    assert_eq!(response["code"], "approval_required");
    assert_eq!(response["details"]["status"], "approval_required");
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("README.md")).unwrap(),
        "fixture\n"
    );
}

#[test]
fn session_resume_returns_persisted_events() {
    let fixture = Fixture::new();
    fixture.setup();
    let run = fixture
        .command(&["run", "read:README.md", "--json"])
        .output()
        .expect("run should start");
    assert_success(&run);
    let session_id = parse_json(&run)["session_id"]
        .as_str()
        .expect("run should return a session")
        .to_owned();

    let output = fixture
        .command(&["session", "resume", &session_id, "--json"])
        .output()
        .expect("resume should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["version"], "0.1");
    assert_eq!(response["command"], "session resume");
    assert_eq!(response["session_id"], session_id);
    assert_eq!(response["event_count"], 4);
}

#[test]
fn provider_set_and_list_use_the_public_configuration_api() {
    let fixture = Fixture::new();
    let output = fixture
        .command(&[
            "provider",
            "set",
            "--provider-url",
            "http://127.0.0.1:4317/v1",
            "--json",
        ])
        .output()
        .expect("provider set should start");
    assert_success(&output);

    let output = fixture
        .command(&["provider", "list", "--json"])
        .output()
        .expect("provider list should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["version"], "0.1");
    assert_eq!(response["providers"][0]["id"], "openai-compatible");
    assert_eq!(
        response["providers"][0]["base_url"],
        "http://127.0.0.1:4317/v1"
    );
}

#[test]
fn doctor_reports_missing_configuration_with_stable_error() {
    let fixture = Fixture::new();
    let output = fixture
        .command(&["doctor", "--json"])
        .output()
        .expect("doctor should start");
    assert_eq!(output.status.code(), Some(10));
    let response = parse_json(&output);
    assert_eq!(response["version"], "0.1");
    assert_eq!(response["code"], "configuration_error");
    assert!(response["message"].as_str().is_some());
    assert!(response.get("details").is_some());
}

#[test]
fn harness_discovery_exposes_the_coding_domain_without_runtime_internals() {
    let fixture = Fixture::new();
    fixture.setup();

    let output = fixture
        .command(&["harness", "list", "--json"])
        .output()
        .expect("harness list should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["version"], "0.1");
    assert_eq!(response["harnesses"][0]["id"], "coding-domain");

    let output = fixture
        .command(&["harness", "inspect", "coding", "--json"])
        .output()
        .expect("harness inspect should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["harness"]["kind"], "domain");
    assert!(response["harness"]["genes"].as_array().unwrap().len() >= 5);

    let output = fixture
        .command(&[
            "harness",
            "run",
            "coding",
            "--gene",
            "workspace.read",
            "--task",
            "read:README.md",
            "--json",
        ])
        .output()
        .expect("harness run should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["command"], "harness run");
    assert_eq!(response["status"], "completed");
}

#[test]
fn approval_can_be_inspected_and_resolved_without_exposing_patch_content() {
    let fixture = Fixture::new();
    fixture.setup();
    let output = fixture
        .command(&["run", "patch:README.md:sk-live-secret", "--json"])
        .output()
        .expect("run should start");
    assert_eq!(output.status.code(), Some(40));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("sk-live-secret"));
    let response = parse_json(&output);
    let approval_id = response["details"]["approval_id"]
        .as_str()
        .expect("approval ID should be returned")
        .to_owned();

    let output = fixture
        .command(&["approval", "inspect", &approval_id, "--json"])
        .output()
        .expect("approval inspect should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["approval"]["status"], "pending");
    assert_eq!(response["approval"]["gene_id"], "patch.apply");
    assert!(!response.to_string().contains("sk-live-secret"));

    let output = fixture
        .command(&["approval", "resolve", &approval_id, "--allow", "--json"])
        .output()
        .expect("approval resolve should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["approval"]["status"], "approved");
    assert_eq!(response["approval"]["approver_id"], "local-user");

    let output = fixture
        .command(&["approval", "list", "--json"])
        .output()
        .expect("approval list should start");
    assert_success(&output);
    assert_eq!(
        parse_json(&output)["approvals"].as_array().unwrap().len(),
        1
    );
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn parse_json(output: &Output) -> Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).unwrap_or_else(|error| {
        panic!(
            "expected JSON output, got {error}: {stdout}; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}
