#![forbid(unsafe_code)]

use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
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
            "pandora-cli-challenge-{}-{suffix}-{sequence}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).expect("workspace dir created");
        fs::write(workspace.join("test.txt"), "initial text\n").expect("test file written");
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

        let mut cfg: Value =
            serde_json::from_str(&fs::read_to_string(&self.config).expect("read config"))
                .expect("parse config json");
        if let Some(providers) = cfg.get_mut("providers").and_then(Value::as_object_mut) {
            providers.insert(
                "hot-swap-provider-1".to_string(),
                serde_json::json!({
                    "base_url": "http://127.0.0.1:4317/v1",
                    "model": "model-v1",
                    "api_key_env": "PANDORA_PROVIDER_KEY",
                    "protocol": "open_ai_compatible"
                }),
            );
            providers.insert(
                "hot-swap-provider-2".to_string(),
                serde_json::json!({
                    "base_url": "http://127.0.0.1:4317/v1",
                    "model": "model-v2",
                    "api_key_env": "PANDORA_PROVIDER_KEY",
                    "protocol": "open_ai_compatible"
                }),
            );
            fs::write(
                &self.config,
                serde_json::to_string_pretty(&cfg).expect("serialize"),
            )
            .expect("write config");
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn challenge_governance_modes_strictly_enforce_mutation_rules() {
    let fixture = Fixture::new();
    fixture.setup();

    // 1. Read-Only mode must strictly deny patch (mutating) operation with exit code 30 and policy_denied
    let out_ro = fixture
        .command(&[
            "run",
            "--mode",
            "read-only",
            "patch:test.txt:denied_text",
            "--json",
        ])
        .output()
        .expect("run read-only");
    assert_eq!(
        out_ro.status.code(),
        Some(30),
        "ReadOnly must exit with code 30"
    );
    let val_ro: Value = serde_json::from_slice(&out_ro.stdout).expect("valid json error");
    assert_eq!(val_ro["code"], "policy_denied");
    assert_eq!(val_ro["details"]["status"], "denied");
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("test.txt"))
            .unwrap()
            .trim(),
        "initial text",
        "ReadOnly must NOT modify the file"
    );

    // 2. Supervised mode must halt mutating operation with exit code 40 and approval_required
    let out_sup = fixture
        .command(&[
            "run",
            "--mode",
            "supervised",
            "patch:test.txt:supervised_text",
            "--json",
        ])
        .output()
        .expect("run supervised");
    assert_eq!(
        out_sup.status.code(),
        Some(40),
        "Supervised must exit with code 40"
    );
    let val_sup: Value = serde_json::from_slice(&out_sup.stdout).expect("valid json error");
    assert_eq!(val_sup["code"], "approval_required");
    assert_eq!(val_sup["details"]["status"], "approval_required");
    assert!(val_sup["details"]["approval_id"].is_string());
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("test.txt"))
            .unwrap()
            .trim(),
        "initial text",
        "Supervised must NOT modify the file before approval"
    );

    // 3. Autonomous mode must execute mutating operation directly with exit code 0 without approval
    let out_auto = fixture
        .command(&[
            "run",
            "--mode",
            "autonomous",
            "patch:test.txt:autonomous_applied",
            "--json",
        ])
        .output()
        .expect("run autonomous");
    assert!(
        out_auto.status.success(),
        "Autonomous must exit with code 0"
    );
    let val_auto: Value = serde_json::from_slice(&out_auto.stdout).expect("valid json");
    assert_eq!(val_auto["status"], "completed");
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("test.txt"))
            .unwrap()
            .trim(),
        "autonomous_applied",
        "Autonomous MUST modify the file directly"
    );

    // 4. Permission alias: --permission 0 -> ReadOnly, --permission 2 -> Autonomous
    let out_perm0 = fixture
        .command(&[
            "run",
            "--permission",
            "0",
            "patch:test.txt:denied_again",
            "--json",
        ])
        .output()
        .expect("run permission 0");
    assert_eq!(out_perm0.status.code(), Some(30));

    let out_perm2 = fixture
        .command(&[
            "run",
            "--permission",
            "2",
            "patch:test.txt:perm2_applied",
            "--json",
        ])
        .output()
        .expect("run permission 2");
    assert!(out_perm2.status.success());
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("test.txt"))
            .unwrap()
            .trim(),
        "perm2_applied"
    );
}

#[test]
fn challenge_chat_model_hotswap_and_session_continuity() {
    let fixture = Fixture::new();
    fixture.setup();

    // Piped input executing multiple turns with model hot-swapping
    let script = [
        "/read test.txt",
        "/session",
        "/model hot-swap-provider-1:model-v1",
        "/read test.txt",
        "/session",
        "/model hot-swap-provider-2:model-v2",
        "/read test.txt",
        "/session",
        "/exit",
    ]
    .join("\n");

    let mut child = fixture
        .command(&["chat"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("chat process spawn");

    {
        let stdin = child.stdin.as_mut().expect("stdin available");
        stdin.write_all(script.as_bytes()).expect("write script");
    }

    let output = child.wait_with_output().expect("wait chat process");
    assert!(output.status.success(), "chat session should exit cleanly");
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Find all session ID occurrences in stdout
    let session_lines: Vec<&str> = stdout
        .lines()
        .filter(|line| line.contains("session: session-"))
        .collect();

    assert_eq!(
        session_lines.len(),
        3,
        "Expected 3 /session outputs across 3 turns. Stdout: {}",
        stdout
    );
    // All 3 session IDs must be completely identical across turns and model switches
    assert_eq!(
        session_lines[0], session_lines[1],
        "Session ID changed after first /model hot-swap!"
    );
    assert_eq!(
        session_lines[1], session_lines[2],
        "Session ID changed after second /model hot-swap!"
    );

    // Verify session store persistence
    let list_out = fixture
        .command(&["session", "list", "--json"])
        .output()
        .expect("session list");
    assert!(list_out.status.success());
    let list_val: Value = serde_json::from_slice(&list_out.stdout).expect("valid json");
    let sessions = list_val["sessions"].as_array().expect("sessions array");
    assert_eq!(
        sessions.len(),
        1,
        "Exactly one continuous session must exist"
    );

    let session_id = sessions[0]["id"].as_str().expect("session id string");

    // Verify session resumption does not fail
    let resume_out = fixture
        .command(&["session", "resume", session_id, "--json"])
        .output()
        .expect("session resume");
    assert!(
        resume_out.status.success(),
        "Session resume failed after model hot-swapping!"
    );
    let resume_val: Value = serde_json::from_slice(&resume_out.stdout).expect("valid resume json");
    assert_eq!(resume_val["session_id"], session_id);
    assert_eq!(resume_val["evaluation_count"], 3);
}

#[test]
fn challenge_chat_robustness_on_edge_case_inputs() {
    let fixture = Fixture::new();
    fixture.setup();

    // Edge cases for /model, /mode, /permission: empty, trailing spaces, invalid numbers, invalid names
    let script = [
        "/model",
        "/model   ",
        "/model :",
        "/model provider-only:",
        "/model :model-only",
        "/model p:m:extra",
        "/mode",
        "/mode   ",
        "/mode invalid_mode_name",
        "/permission",
        "/permission 999",
        "/permission -1",
        "/permission autonomous",
        "/mode",
        "/exit",
    ]
    .join("\n");

    let mut child = fixture
        .command(&["chat"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("chat process spawn");

    {
        let stdin = child.stdin.as_mut().expect("stdin available");
        stdin.write_all(script.as_bytes()).expect("write script");
    }

    let output = child.wait_with_output().expect("wait chat process");
    assert!(
        output.status.success(),
        "chat process must not crash on malformed slash commands"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("governance> active mode: autonomous (level: 2)"));
}

#[test]
fn challenge_chat_mode_hotswap_governance() {
    let fixture = Fixture::new();
    fixture.setup();

    // In chat, when user sets /mode read-only, executing mutating /patch must fail closed
    // with policy_denied under the active governance policy rather than prompting for approval.
    let script = ["/mode read-only", "/patch test.txt modified_in_ro", "/exit"].join("\n");

    let mut child = fixture
        .command(&["chat"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("chat process spawn");

    {
        let stdin = child.stdin.as_mut().expect("stdin available");
        stdin.write_all(script.as_bytes()).expect("write script");
    }

    let output = child.wait_with_output().expect("wait chat process");
    assert!(output.status.success(), "chat session should exit cleanly");
    let stdout = String::from_utf8_lossy(&output.stdout);

    // ReadOnly in chat should deny mutating patch
    assert!(
        stdout.contains("error:") || stdout.contains("denied"),
        "ReadOnly in chat should deny mutating patch: stdout={stdout}"
    );
    assert!(
        !stdout.contains("operation requires explicit approval"),
        "ReadOnly in chat must not request approval"
    );
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("test.txt"))
            .unwrap()
            .trim(),
        "initial text",
        "ReadOnly must NOT modify the file"
    );
}
