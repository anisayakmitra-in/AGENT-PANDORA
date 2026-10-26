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

#[test]
fn test_mcp_import_from_agent_config() {
    let fixture = Fixture::new();
    fixture.setup();

    // Create a mock Claude / Cursor MCP config file
    let mcp_config_path = fixture.workspace.join("claude_desktop_config.json");
    fs::write(
        &mcp_config_path,
        r#"{
            "mcpServers": {
                "imported-fs": {
                    "command": "node",
                    "args": ["server.js", "/data"]
                },
                "imported-remote": {
                    "url": "http://127.0.0.1:9090/sse"
                }
            }
        }"#,
    )
    .expect("mcp config written");

    // Dry-run import
    let dry_run = fixture
        .command(&[
            "mcp",
            "import",
            mcp_config_path.to_str().unwrap(),
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("mcp import dry-run should execute");
    assert!(dry_run.status.success());
    let dry_val: Value = serde_json::from_slice(&dry_run.stdout).expect("valid json");
    assert_eq!(dry_val["imported_count"], 2);
    assert_eq!(dry_val["dry_run"], true);

    // Actual import
    let import_out = fixture
        .command(&["mcp", "import", mcp_config_path.to_str().unwrap(), "--json"])
        .output()
        .expect("mcp import should execute");
    assert!(import_out.status.success());
    let val: Value = serde_json::from_slice(&import_out.stdout).expect("valid json");
    assert_eq!(val["imported_count"], 2);

    // Verify imported servers appear in `mcp list`
    let list_out = fixture
        .command(&["mcp", "list", "--json"])
        .output()
        .expect("mcp list should execute");
    assert!(list_out.status.success());
    let list_val: Value = serde_json::from_slice(&list_out.stdout).expect("valid list json");
    let servers = list_val["servers"].as_array().expect("servers array");
    let ids: Vec<&str> = servers.iter().filter_map(|s| s["id"].as_str()).collect();
    assert!(
        ids.contains(&"imported-fs"),
        "expected imported-fs in {ids:?}"
    );
    assert!(
        ids.contains(&"imported-remote"),
        "expected imported-remote in {ids:?}"
    );
}

#[test]
fn test_update_command_check_and_bare_execution() {
    let fixture = Fixture::new();
    fixture.setup();

    // Check with --check flag
    let check_out = fixture
        .command(&["update", "--check", "--json"])
        .env(
            "PANDORA_LATEST_RELEASE_OVERRIDE",
            format!("v{}", env!("CARGO_PKG_VERSION")),
        )
        .output()
        .expect("update check should execute");
    assert!(check_out.status.success());
    let val: Value = serde_json::from_slice(&check_out.stdout).expect("valid update json");
    assert_eq!(val["command"], "update check");
    assert_eq!(
        val["current_version"],
        format!("v{}", env!("CARGO_PKG_VERSION"))
    );
    assert!(val["latest_release"].is_string());

    // Check with positional "check"
    let check_pos = fixture
        .command(&["update", "check", "--json"])
        .env(
            "PANDORA_LATEST_RELEASE_OVERRIDE",
            format!("v{}", env!("CARGO_PKG_VERSION")),
        )
        .output()
        .expect("update check positional should execute");
    assert!(check_pos.status.success());
    let pos_val: Value = serde_json::from_slice(&check_pos.stdout).expect("valid pos json");
    assert_eq!(pos_val["command"], "update check");

    // Bare `pandora update --json` defaults to checking and reporting up-to-date
    let bare_out = fixture
        .command(&["update", "--json"])
        .env(
            "PANDORA_LATEST_RELEASE_OVERRIDE",
            format!("v{}", env!("CARGO_PKG_VERSION")),
        )
        .output()
        .expect("update bare should execute");
    assert!(bare_out.status.success());
    let bare_val: Value = serde_json::from_slice(&bare_out.stdout).expect("valid bare json");
    assert_eq!(bare_val["command"], "update");
    assert_eq!(bare_val["up_to_date"], true);
}

#[test]
fn test_chat_slash_update_and_uninstall_commands() {
    let fixture = Fixture::new();
    fixture.setup();

    // Test piping /help to chat shows /update and /uninstall
    let mut cmd = fixture.command(&["chat"]);
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().expect("chat spawn");
    {
        use std::io::Write;
        let mut stdin = child.stdin.take().expect("stdin");
        stdin.write_all(b"/help\n/exit\n").expect("write /help");
    }
    let output = child.wait_with_output().expect("wait chat");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("/update"), "chat help must include /update");
    assert!(
        stdout.contains("/uninstall"),
        "chat help must include /uninstall"
    );

    // Test interactive /uninstall warning when not confirmed
    let mut cmd2 = fixture.command(&["chat"]);
    cmd2.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child2 = cmd2.spawn().expect("chat spawn 2");
    {
        use std::io::Write;
        let mut stdin = child2.stdin.take().expect("stdin 2");
        stdin
            .write_all(b"/uninstall\n/exit\n")
            .expect("write /uninstall");
    }
    let output2 = child2.wait_with_output().expect("wait chat 2");
    assert!(output2.status.success());
    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    assert!(
        stdout2.contains("To confirm uninstallation, type: /uninstall --yes"),
        "expected confirmation prompt in: {stdout2}"
    );

    // Test confirmed /uninstall --yes from chat cleans up and exits
    let mut cmd3 = fixture.command(&["chat"]);
    cmd3.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child3 = cmd3.spawn().expect("chat spawn 3");
    {
        use std::io::Write;
        let mut stdin = child3.stdin.take().expect("stdin 3");
        stdin
            .write_all(b"/uninstall --yes\n")
            .expect("write /uninstall --yes");
    }
    let output3 = child3.wait_with_output().expect("wait chat 3");
    assert!(output3.status.success());
    let stdout3 = String::from_utf8_lossy(&output3.stdout);
    assert!(
        stdout3.contains("Pandora configuration and data removed"),
        "expected uninstallation message in: {stdout3}"
    );
    // Config file was removed
    assert!(
        !fixture.config.exists(),
        "config file must be removed by /uninstall"
    );
    // Workspace was preserved
    assert!(
        fixture.workspace.exists(),
        "workspace must be preserved by /uninstall"
    );
}
