use pandora_types::{
    HarnessId, MetaComposition, PackageCompatibility, PackageDependency, PackageKind,
    PackageManifest, TrustEvidence, hash_artifact,
};
use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
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
    assert_eq!(setup["provider_model"], "default");

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
    assert_eq!(response["efficiency_recorded"], true);
    assert!(
        !response["session_id"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );

    let output = fixture
        .command(&[
            "efficiency",
            "rank",
            "--task-class",
            "general",
            "--objective",
            "latency",
            "--json",
        ])
        .output()
        .expect("efficiency ranking should start");
    assert_success(&output);
    let ranking = parse_json(&output);
    assert_eq!(ranking["command"], "efficiency rank");
    assert!(!ranking["rankings"].as_array().unwrap().is_empty());
}

#[test]
fn interactive_setup_configures_a_provider_without_echoing_secrets() {
    let fixture = Fixture::new();
    let mut command = fixture.command(&["setup", "--interactive", "--json"]);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("interactive setup should start");
    let mut input = child
        .stdin
        .take()
        .expect("interactive setup should accept input");
    input
        .write_all(b"http://127.0.0.1:4317/v1\nfixture-model\nPANDORA_FIXTURE_KEY\n")
        .expect("interactive answers should be written");
    drop(input);

    let output = child
        .wait_with_output()
        .expect("interactive setup should finish");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["command"], "setup");
    assert_eq!(response["provider_configured"], true);
    assert_eq!(response["provider_model"], "fixture-model");
    assert_eq!(response["api_key_env"], "PANDORA_FIXTURE_KEY");

    let config = fs::read_to_string(&fixture.config).expect("interactive config should exist");
    assert!(config.contains("PANDORA_FIXTURE_KEY"));
    assert!(!config.contains("secret"));
}

#[test]
fn scripted_setup_persists_the_requested_credential_environment() {
    let fixture = Fixture::new();
    let output = fixture
        .command(&[
            "setup",
            "--provider-url",
            "http://127.0.0.1:4317/v1",
            "--model",
            "scripted-model",
            "--api-key-env",
            "PANDORA_SCRIPTED_KEY",
            "--json",
        ])
        .output()
        .expect("scripted setup should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["active_provider"], "openai-compatible");
    assert_eq!(response["provider_model"], "scripted-model");
    assert_eq!(response["api_key_env"], "PANDORA_SCRIPTED_KEY");

    let config = fs::read_to_string(&fixture.config).expect("scripted config should exist");
    assert!(config.contains("PANDORA_SCRIPTED_KEY"));
}

#[test]
fn scripted_setup_rejects_provider_options_without_a_provider_url() {
    let fixture = Fixture::new();
    let output = fixture
        .command(&["setup", "--api-key-env", "PANDORA_SCRIPTED_KEY", "--json"])
        .output()
        .expect("scripted setup should start");
    assert_eq!(output.status.code(), Some(2));
    let response = parse_json(&output);
    assert_eq!(response["code"], "usage_error");
    assert!(
        response["message"]
            .as_str()
            .unwrap_or_default()
            .contains("--provider-url")
    );
    assert!(!fixture.config.exists());
}

#[test]
fn chat_can_exit_without_provider_configuration() {
    let fixture = Fixture::new();
    let mut command = fixture.command(&["chat"]);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("chat should start");
    child
        .stdin
        .take()
        .expect("chat should accept input")
        .write_all(b"/exit\n")
        .expect("chat command should be written");

    let output = child.wait_with_output().expect("chat should finish");
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Chat closed"));
}

#[test]
fn chat_handles_local_commands_without_provider_configuration() {
    let fixture = Fixture::new();
    let mut command = fixture.command(&["chat"]);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("chat should start");
    child
        .stdin
        .take()
        .expect("chat should accept input")
        .write_all(b"/help\n/session\n/approve\n/deny\n/quit\n")
        .expect("chat commands should be written");

    let output = child.wait_with_output().expect("chat should finish");
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("show chat commands"));
    assert!(stdout.contains("session: not started"));
    assert!(stdout.contains("approval> no pending approval"));
    assert!(stdout.contains("Chat closed after 0 turn(s)"));
}

#[test]
fn chat_rejects_machine_readable_output() {
    let fixture = Fixture::new();
    let output = fixture
        .command(&["chat", "--json"])
        .output()
        .expect("chat should start");
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(parse_json(&output)["code"], "usage_error");
    assert_eq!(
        parse_json(&output)["message"],
        "chat does not support --json"
    );
}

#[test]
fn tui_requires_an_interactive_terminal() {
    let fixture = Fixture::new();
    let output = fixture
        .command(&["tui"])
        .output()
        .expect("tui should start");
    assert_eq!(output.status.code(), Some(10));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("error: tui requires an interactive terminal")
    );
}

#[test]
fn tui_rejects_machine_readable_output() {
    let fixture = Fixture::new();
    let output = fixture
        .command(&["tui", "--json"])
        .output()
        .expect("tui should start");
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(parse_json(&output)["code"], "usage_error");
    assert_eq!(
        parse_json(&output)["message"],
        "tui does not support --json"
    );
}

#[test]
fn search_run_returns_matching_workspace_files() {
    let fixture = Fixture::new();
    fixture.setup();
    fs::create_dir(fixture.workspace.join("src")).unwrap();
    fs::write(fixture.workspace.join("src/lib.rs"), "needle\n").unwrap();

    let output = fixture
        .command(&["run", "search:needle", "--json"])
        .output()
        .expect("search should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["status"], "completed");
    assert_eq!(response["output"], "src/lib.rs");
}

#[test]
fn skill_list_and_inspect_are_metadata_only() {
    let fixture = Fixture::new();
    let skill_root = fixture.data.join("skills").join("alpha");
    fs::create_dir_all(&skill_root).unwrap();
    fs::write(
        skill_root.join("SKILL.md"),
        "---\nid: alpha\nversion: 0.1.0\nname: Alpha Skill\ndescription: Reads project guidance\npublisher: pandora\nresources: workspace.read\n---\n# Alpha\n\nUse the read tool.\n",
    )
    .unwrap();

    let output = fixture
        .command(&["skill", "list", "--json"])
        .output()
        .expect("skill list should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["skills"][0]["id"], "alpha");
    assert_eq!(response["skills"][0]["state"], "disabled");

    let output = fixture
        .command(&["skill", "inspect", "alpha", "--json"])
        .output()
        .expect("skill inspect should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["skill"]["id"], "alpha");
    assert!(
        response["skill"]["body"]
            .as_str()
            .unwrap()
            .contains("read tool")
    );
}

#[test]
fn skill_install_admits_a_local_package_without_enabling_it() {
    let fixture = Fixture::new();
    let source = fixture.root.join("incoming").join("beta");
    fs::create_dir_all(source.join("scripts")).unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nid: beta\nversion: 0.1.0\nname: Beta Skill\ndescription: Reads project guidance\npublisher: pandora\nresources: workspace.read\n---\n# Beta\n",
    )
    .unwrap();
    fs::write(source.join("scripts/check.py"), "print('ok')").unwrap();

    let source = source.to_str().unwrap();
    let output = fixture
        .command(&["skill", "install", source, "--json"])
        .output()
        .expect("skill install should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["skill"]["id"], "beta");
    assert_eq!(response["skill"]["state"], "disabled");
    assert!(fixture.data.join("skills/beta/SKILL.md").is_file());
    assert!(fixture.root.join("incoming/beta/SKILL.md").is_file());
}

#[test]
fn tool_list_and_inspect_expose_governed_contracts_only() {
    let fixture = Fixture::new();

    let output = fixture
        .command(&["tool", "list", "--json"])
        .output()
        .expect("tool list should start");
    assert_success(&output);
    let response = parse_json(&output);
    let tools = response["tools"].as_array().unwrap();
    assert!(tools.iter().any(|tool| tool["id"] == "workspace.read"));

    let output = fixture
        .command(&["tool", "inspect", "workspace.read", "--json"])
        .output()
        .expect("tool inspect should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["tool"]["id"], "workspace.read");
    assert_eq!(response["tool"]["version"], "1.0.0");
    assert_eq!(response["tool"]["capability"], "filesystem.read");
    assert_eq!(response["tool"]["operation"], "read");
    assert_eq!(response["tool"]["input_schema"]["required"][0], "path");
    assert_eq!(
        response["tool"]["input_schema"]["additionalProperties"],
        false
    );

    let output = fixture
        .command(&["tool", "inspect", "missing", "--json"])
        .output()
        .expect("unknown tool inspect should start");
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(parse_json(&output)["code"], "usage_error");
}

#[test]
fn skill_state_transitions_persist_without_running_scripts() {
    let fixture = Fixture::new();
    let skill_root = fixture.data.join("skills").join("alpha");
    fs::create_dir_all(skill_root.join("scripts")).unwrap();
    fs::write(
        skill_root.join("SKILL.md"),
        "---\nid: alpha\nversion: 0.1.0\nname: Alpha Skill\ndescription: Reads project guidance\npublisher: pandora\nresources: workspace.read\n---\n# Alpha\n",
    )
    .unwrap();
    fs::write(skill_root.join("scripts").join("marker.txt"), "untouched").unwrap();

    let output = fixture
        .command(&["skill", "enable", "alpha", "--json"])
        .output()
        .expect("skill enable should start");
    assert_success(&output);
    assert_eq!(parse_json(&output)["skill"]["state"], "enabled");

    let output = fixture
        .command(&["skill", "list", "--json"])
        .output()
        .expect("skill list should start");
    assert_success(&output);
    assert_eq!(parse_json(&output)["skills"][0]["state"], "enabled");

    let output = fixture
        .command(&["skill", "suspend", "alpha", "--json"])
        .output()
        .expect("skill suspend should start");
    assert_success(&output);
    assert_eq!(parse_json(&output)["skill"]["state"], "suspended");

    let output = fixture
        .command(&["skill", "disable", "alpha", "--json"])
        .output()
        .expect("skill disable should start");
    assert_success(&output);
    assert_eq!(parse_json(&output)["skill"]["state"], "disabled");
    assert_eq!(
        fs::read_to_string(skill_root.join("scripts").join("marker.txt")).unwrap(),
        "untouched"
    );

    let output = fixture
        .command(&["skill", "remove", "alpha", "--json"])
        .output()
        .expect("unconfirmed skill remove should start");
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(parse_json(&output)["code"], "usage_error");
    assert!(skill_root.is_dir());

    let output = fixture
        .command(&["skill", "remove", "alpha", "--dry-run", "--json"])
        .output()
        .expect("skill remove dry-run should start");
    assert_success(&output);
    assert_eq!(parse_json(&output)["dry_run"], true);
    assert!(skill_root.is_dir());

    let output = fixture
        .command(&["skill", "remove", "alpha", "--yes", "--json"])
        .output()
        .expect("skill remove should start");
    assert_success(&output);
    assert_eq!(parse_json(&output)["skill"]["state"], "removed");
    assert!(!skill_root.exists());

    let output = fixture
        .command(&["skill", "restore", "alpha", "--json"])
        .output()
        .expect("skill restore should start");
    assert_success(&output);
    assert_eq!(parse_json(&output)["skill"]["state"], "disabled");
    assert!(skill_root.is_dir());
    assert_eq!(
        fs::read_to_string(skill_root.join("scripts").join("marker.txt")).unwrap(),
        "untouched"
    );
}

#[test]
fn orchestration_roles_are_discoverable_without_runtime_setup() {
    let fixture = Fixture::new();
    let output = fixture
        .command(&["orchestration", "roles", "--json"])
        .output()
        .expect("orchestration discovery should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["command"], "orchestration roles");
    assert_eq!(response["roles"][0], "planner");
    assert_eq!(response["roles"][3], "verifier");
}

#[test]
fn strategy_profiles_are_discoverable_without_runtime_setup() {
    let fixture = Fixture::new();
    let output = fixture
        .command(&["strategies", "list", "--json"])
        .output()
        .expect("strategy discovery should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["command"], "strategies list");
    assert_eq!(response["default"], "react");
    assert_eq!(response["available"][2]["profile"], "research");
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
fn verify_run_returns_approval_required_before_process_execution() {
    let fixture = Fixture::new();
    fixture.setup();

    let output = fixture
        .command(&["run", "verify", "--json"])
        .output()
        .expect("verify run should start");
    assert_eq!(output.status.code(), Some(40));
    let response = parse_json(&output);
    assert_eq!(response["code"], "approval_required");
    assert_eq!(response["details"]["status"], "approval_required");
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
    assert_eq!(response["agent_message_count"], 0);
    assert_eq!(response["l1_evidence_count"], 1);
}

#[test]
fn session_inspect_returns_metadata_without_event_payloads() {
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
        .command(&["session", "inspect", &session_id, "--json"])
        .output()
        .expect("inspect should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["version"], "0.1");
    assert_eq!(response["command"], "session inspect");
    assert_eq!(response["session_id"], session_id);
    assert_eq!(response["metadata"]["principal_id"], "local-user");
    assert_eq!(response["metadata"]["tenant_id"], "local-tenant");
    assert_eq!(response["metadata"]["workspace_id"], "local-workspace");
    assert_eq!(response["event_count"], 4);
    assert_eq!(response["agent_message_count"], 0);
    assert_eq!(response["last_event_type"], "effect_completed");
    assert!(response["last_event_timestamp"].as_u64().is_some());
    assert_eq!(response["observability"]["trace_count"], 1);
    assert_eq!(response["observability"]["span_count"], 4);
    assert_eq!(response["observability"]["uninstrumented_event_count"], 0);
    assert_eq!(response["observability"]["error_count"], 0);
    assert_eq!(response["observability"]["reliability_bps"], 10_000);
    assert!(response.get("events").is_none());
}

#[test]
fn session_inspect_missing_session_preserves_resume_error() {
    let fixture = Fixture::new();
    fixture.setup();

    let output = fixture
        .command(&["session", "inspect", "missing-session", "--json"])
        .output()
        .expect("inspect should start");
    assert_eq!(output.status.code(), Some(60));
    let response = parse_json(&output);
    assert_eq!(response["version"], "0.1");
    assert_eq!(response["code"], "internal_error");
    assert_eq!(response["message"], "session was not found");
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
            "--model",
            "fixture-model",
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
    assert_eq!(response["providers"][0]["default_model"], "fixture-model");
}

#[test]
fn named_provider_profiles_can_be_selected_without_exposing_credentials() {
    let fixture = Fixture::new();
    for (name, url, model, key_env) in [
        (
            "design",
            "https://design.example/v1",
            "vision-model",
            "PANDORA_DESIGN_API_KEY",
        ),
        (
            "coding",
            "https://coding.example/v1",
            "coding-model",
            "PANDORA_CODING_API_KEY",
        ),
    ] {
        let output = fixture
            .command(&[
                "provider",
                "set",
                "--name",
                name,
                "--provider-url",
                url,
                "--model",
                model,
                "--api-key-env",
                key_env,
                "--json",
            ])
            .output()
            .expect("provider profile should start");
        assert_success(&output);
    }

    let output = fixture
        .command(&[
            "provider",
            "set",
            "--name",
            "coding",
            "--provider-url",
            "https://coding.example/v1",
            "--model",
            "coding-model",
            "--api-key-env",
            "PANDORA_CODING_API_KEY",
            "--fallback-provider",
            "design",
            "--input-micros-per-million-tokens",
            "2000000",
            "--output-micros-per-million-tokens",
            "4000000",
            "--json",
        ])
        .output()
        .expect("provider fallback should be configurable");
    assert_success(&output);

    let output = fixture
        .command(&["provider", "use", "design", "--json"])
        .output()
        .expect("provider selection should start");
    assert_success(&output);

    let output = fixture
        .command(&["provider", "list", "--json"])
        .output()
        .expect("provider list should start");
    assert_success(&output);
    let response = parse_json(&output);
    let providers = response["providers"]
        .as_array()
        .expect("provider list should be an array");
    let design = providers
        .iter()
        .find(|provider| provider["id"] == "design")
        .expect("design profile should be listed");
    let coding = providers
        .iter()
        .find(|provider| provider["id"] == "coding")
        .expect("coding profile should be listed");
    assert_eq!(design["default_model"], "vision-model");
    assert_eq!(design["api_key_env"], "PANDORA_DESIGN_API_KEY");
    assert_eq!(design["active"], true);
    assert_eq!(coding["fallback_provider"], "design");
    assert_eq!(
        coding["pricing"]["input_micros_per_million_tokens"],
        2_000_000
    );
    assert_eq!(
        coding["pricing"]["output_micros_per_million_tokens"],
        4_000_000
    );
    assert!(!response.to_string().contains("test-provider-key"));
}

#[test]
fn provider_test_completes_a_configured_request_without_echoing_credentials() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("provider fixture should bind");
    let address = listener
        .local_addr()
        .expect("provider fixture should expose its address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("provider should connect");
        let mut request = Vec::new();
        loop {
            let mut chunk = [0_u8; 1_024];
            let bytes_read = stream.read(&mut chunk).expect("request should be readable");
            request.extend_from_slice(&chunk[..bytes_read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let header_end = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("request should contain headers")
            + 4;
        let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
        let content_length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .expect("provider should send a content length");
        while request.len() < header_end + content_length {
            let mut chunk = [0_u8; 1_024];
            let bytes_read = stream
                .read(&mut chunk)
                .expect("request body should be readable");
            request.extend_from_slice(&chunk[..bytes_read]);
        }
        let body = String::from_utf8_lossy(&request[header_end..header_end + content_length]);
        assert!(headers.contains("authorization: bearer test-provider-key"));
        assert!(body.contains("\"model\":\"fixture-model\""));
        let response =
            br#"{"choices":[{"message":{"content":"ready"}}],"usage":{"prompt_tokens":2,"completion_tokens":1}}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response.len()
        )
        .expect("provider response headers should be written");
        stream
            .write_all(response)
            .expect("provider response should be written");
    });

    let fixture = Fixture::new();
    let provider_url = format!("http://{address}/v1");
    let configured = fixture
        .command(&[
            "provider",
            "set",
            "--provider-url",
            &provider_url,
            "--model",
            "fixture-model",
            "--json",
        ])
        .output()
        .expect("provider set should start");
    assert_success(&configured);

    let output = fixture
        .command(&["provider", "test", "--json"])
        .env("PANDORA_PROVIDER_API_KEY", "test-provider-key")
        .output()
        .expect("provider test should start");
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("test-provider-key"));
    let response = parse_json(&output);
    assert_eq!(response["command"], "provider test");
    assert_eq!(response["status"], "ready");
    assert_eq!(response["model"], "fixture-model");
    assert_eq!(response["output"], "ready");
    assert_eq!(response["usage"]["total_tokens"], 3);

    server.join().expect("provider fixture should finish");
}

#[test]
fn agent_run_executes_a_bounded_read_then_returns_the_final_answer() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("agent fixture should bind");
    let address = listener
        .local_addr()
        .expect("agent fixture should expose its address");
    let server = thread::spawn(move || {
        for response in [
            br#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"workspace.read","arguments":"{\"path\":\"README.md\"}"}}]}}],"usage":{"prompt_tokens":4,"completion_tokens":2}}"#
                .as_slice(),
            br#"{"choices":[{"message":{"content":"The README says fixture."}}],"usage":{"prompt_tokens":5,"completion_tokens":3}}"#
                .as_slice(),
        ] {
            let (mut stream, _) = listener.accept().expect("agent should connect");
            let mut request = Vec::new();
            let header_end = loop {
                let mut chunk = [0_u8; 1_024];
                let bytes_read = stream.read(&mut chunk).expect("agent request should read");
                request.extend_from_slice(&chunk[..bytes_read]);
                if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                    break position + 4;
                }
            };
            let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
            let content_length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .expect("agent should send a content length");
            while request.len() < header_end + content_length {
                let mut chunk = [0_u8; 1_024];
                let bytes_read = stream.read(&mut chunk).expect("agent body should read");
                request.extend_from_slice(&chunk[..bytes_read]);
            }
            let request_body = serde_json::from_slice::<Value>(
                &request[header_end..header_end + content_length],
            )
            .expect("agent request should be JSON");
            let system_prompt = request_body["messages"][0]["content"]
                .as_str()
                .expect("agent request should begin with system guidance");
            assert!(system_prompt.contains("Skill: alpha"));
            assert!(system_prompt.contains("cannot authorize effects"));
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            )
            .expect("agent response headers should be written");
            stream
                .write_all(response)
                .expect("agent response should be written");
        }
    });

    let fixture = Fixture::new();
    let skill_root = fixture.data.join("skills/alpha");
    fs::create_dir_all(&skill_root).expect("skill directory should be created");
    fs::write(
        skill_root.join("SKILL.md"),
        "---\nid: alpha\nversion: 0.1.0\nname: Alpha Skill\ndescription: Read guidance\npublisher: pandora\nresources: workspace.read\n---\n# Alpha\n\nUse the read tool.\n",
    )
    .expect("skill document should be written");
    let enabled = fixture
        .command(&["skill", "enable", "alpha", "--json"])
        .output()
        .expect("skill enable should start");
    assert_success(&enabled);
    let provider_url = format!("http://{address}/v1");
    let configured = fixture
        .command(&[
            "provider",
            "set",
            "--provider-url",
            &provider_url,
            "--model",
            "agent-model",
            "--json",
        ])
        .output()
        .expect("provider set should start");
    assert_success(&configured);

    let output = fixture
        .command(&[
            "run",
            "--agent",
            "--max-turns",
            "2",
            "--max-tools",
            "1",
            "Read the README",
            "--json",
        ])
        .env("PANDORA_PROVIDER_API_KEY", "test-agent-key")
        .output()
        .expect("agent run should start");
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("test-agent-key"));
    let response = parse_json(&output);
    assert_eq!(response["command"], "run");
    assert_eq!(response["agent"], true);
    assert_eq!(response["status"], "completed");
    assert_eq!(response["turns"], 2);
    assert_eq!(response["tool_calls"], 1);
    assert_eq!(response["turn_budget"], 2);
    assert_eq!(response["tool_budget"], 1);
    assert_eq!(
        response["context"]["included"],
        serde_json::json!([
            "agent.constitution",
            "agent.skill-boundary",
            "agent.enabled-skills",
        ])
    );
    assert_eq!(response["context"]["dropped"], serde_json::json!([]));
    assert_eq!(response["context"]["cacheable"], false);
    assert!(response["context"]["token_cost"].as_u64().unwrap() > 0);
    assert_eq!(response["output"], "The README says fixture.");

    server.join().expect("agent fixture should finish");
}

#[test]
fn agent_session_reuses_persisted_conversation() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("agent fixture should bind");
    let address = listener
        .local_addr()
        .expect("agent fixture should expose its address");
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        for response in [
            br#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"workspace.read","arguments":"{\"path\":\"README.md\"}"}}]}}],"usage":{"prompt_tokens":4,"completion_tokens":2}}"#.as_slice(),
            br#"{"choices":[{"message":{"content":"first answer"}}]}"#.as_slice(),
            br#"{"choices":[{"message":{"content":"continued answer"}}]}"#.as_slice(),
        ] {
            let (mut stream, _) = listener.accept().expect("agent should connect");
            let mut request = Vec::new();
            let header_end = loop {
                let mut chunk = [0_u8; 1_024];
                let bytes_read = stream.read(&mut chunk).expect("agent request should read");
                request.extend_from_slice(&chunk[..bytes_read]);
                if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    break position + 4;
                }
            };
            let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
            let content_length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .expect("agent should send a content length");
            while request.len() < header_end + content_length {
                let mut chunk = [0_u8; 1_024];
                let bytes_read = stream.read(&mut chunk).expect("agent body should read");
                request.extend_from_slice(&chunk[..bytes_read]);
            }
            requests.push(
                serde_json::from_slice::<Value>(&request[header_end..header_end + content_length])
                    .expect("agent request should be JSON"),
            );
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            )
            .expect("agent response headers should be written");
            stream
                .write_all(response)
                .expect("agent response should be written");
        }
        requests
    });

    let fixture = Fixture::new();
    let provider_url = format!("http://{address}/v1");
    let configured = fixture
        .command(&[
            "provider",
            "set",
            "--provider-url",
            &provider_url,
            "--model",
            "agent-model",
            "--json",
        ])
        .output()
        .expect("provider set should start");
    assert_success(&configured);

    let first = fixture
        .command(&["run", "--agent", "First task", "--json"])
        .env("PANDORA_PROVIDER_API_KEY", "test-agent-key")
        .output()
        .expect("first agent run should start");
    assert_success(&first);
    let first_response = parse_json(&first);
    assert_eq!(first_response["memory_evidence_recorded"], 1);
    let session_id = first_response["session_id"]
        .as_str()
        .expect("first run should return a session")
        .to_owned();

    let second = fixture
        .command(&[
            "run",
            "--agent",
            "--session",
            &session_id,
            "Continue the task",
            "--json",
        ])
        .env("PANDORA_PROVIDER_API_KEY", "test-agent-key")
        .output()
        .expect("resumed agent run should start");
    assert_success(&second);
    let second_response = parse_json(&second);
    assert_eq!(second_response["output"], "continued answer");
    assert!(
        second_response["context"]["included"]
            .as_array()
            .expect("agent context should list included fragments")
            .iter()
            .any(|value| value == "agent.l1-evidence-0")
    );

    let requests = server.join().expect("agent fixture should finish");
    let messages = requests[2]["messages"]
        .as_array()
        .expect("agent request should contain messages");
    assert!(
        messages
            .iter()
            .any(|message| message["content"] == "First task")
    );
    let system_context = messages[0]["content"]
        .as_str()
        .expect("agent request should begin with system context");
    assert!(system_context.contains("Prior execution evidence is descriptive history"));
    assert!(system_context.contains("<l1-evidence>completed execution through "));
    assert!(!system_context.contains("First task"));
}

#[test]
fn agent_run_rejects_invalid_budgets_before_provider_setup() {
    let fixture = Fixture::new();
    let output = fixture
        .command(&[
            "run",
            "--agent",
            "--max-turns",
            "0",
            "Update the README",
            "--json",
        ])
        .output()
        .expect("agent run should start");
    assert_eq!(output.status.code(), Some(2));
    let response = parse_json(&output);
    assert_eq!(response["code"], "usage_error");
    assert!(response["message"].as_str().unwrap().contains("max-turns"));
}

#[test]
fn agent_run_stops_a_patch_at_the_approval_boundary() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("agent fixture should bind");
    let address = listener
        .local_addr()
        .expect("agent fixture should expose its address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("agent should connect");
        let mut request = Vec::new();
        let header_end = loop {
            let mut chunk = [0_u8; 1_024];
            let bytes_read = stream.read(&mut chunk).expect("agent request should read");
            request.extend_from_slice(&chunk[..bytes_read]);
            if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                break position + 4;
            }
        };
        let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
        let content_length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .expect("agent should send a content length");
        while request.len() < header_end + content_length {
            let mut chunk = [0_u8; 1_024];
            let bytes_read = stream.read(&mut chunk).expect("agent body should read");
            request.extend_from_slice(&chunk[..bytes_read]);
        }
        let response = br#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"workspace.patch","arguments":"{\"path\":\"README.md\",\"content\":\"changed\"}"}}]}}],"usage":{"prompt_tokens":4,"completion_tokens":2}}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response.len()
        )
        .expect("agent response headers should be written");
        stream
            .write_all(response)
            .expect("agent response should be written");
    });

    let fixture = Fixture::new();
    let provider_url = format!("http://{address}/v1");
    let configured = fixture
        .command(&[
            "provider",
            "set",
            "--provider-url",
            &provider_url,
            "--model",
            "agent-model",
            "--json",
        ])
        .output()
        .expect("provider set should start");
    assert_success(&configured);

    let output = fixture
        .command(&["run", "--agent", "Update the README", "--json"])
        .env("PANDORA_PROVIDER_API_KEY", "test-agent-key")
        .output()
        .expect("agent run should start");
    assert_eq!(output.status.code(), Some(40));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("test-agent-key"));
    let response = parse_json(&output);
    assert_eq!(response["code"], "approval_required");
    assert_eq!(response["details"]["agent"], true);
    assert!(
        !response["details"]["approval_id"]
            .as_str()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("README.md")).unwrap(),
        "fixture\n"
    );

    server.join().expect("agent fixture should finish");
}

#[test]
fn approved_agent_run_resumes_the_pending_tool_call_once() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("agent fixture should bind");
    let address = listener
        .local_addr()
        .expect("agent fixture should expose its address");
    let server = thread::spawn(move || {
        let first_response = br#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"workspace.patch","arguments":"{\"path\":\"README.md\",\"content\":\"approved\"}"}}]}}],"usage":{"prompt_tokens":4,"completion_tokens":2}}"#;
        let second_response = br#"{"choices":[{"message":{"content":"approved"}}],"usage":{"prompt_tokens":5,"completion_tokens":2}}"#;
        for response in [first_response.as_slice(), second_response.as_slice()] {
            let (mut stream, _) = listener.accept().expect("agent should connect");
            let mut request = Vec::new();
            let header_end = loop {
                let mut chunk = [0_u8; 1_024];
                let bytes_read = stream.read(&mut chunk).expect("agent request should read");
                request.extend_from_slice(&chunk[..bytes_read]);
                if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    break position + 4;
                }
            };
            let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
            let content_length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .expect("agent should send a content length");
            while request.len() < header_end + content_length {
                let mut chunk = [0_u8; 1_024];
                let bytes_read = stream.read(&mut chunk).expect("agent body should read");
                request.extend_from_slice(&chunk[..bytes_read]);
            }
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            )
            .expect("agent response headers should be written");
            stream
                .write_all(response)
                .expect("agent response should be written");
        }
    });

    let fixture = Fixture::new();
    let provider_url = format!("http://{address}/v1");
    let configured = fixture
        .command(&[
            "provider",
            "set",
            "--provider-url",
            &provider_url,
            "--model",
            "agent-model",
            "--json",
        ])
        .output()
        .expect("provider set should start");
    assert_success_with_context(&configured, "provider set");

    let first = fixture
        .command(&["run", "--agent", "Update the README", "--json"])
        .env("PANDORA_PROVIDER_API_KEY", "test-agent-key")
        .output()
        .expect("agent run should start");
    assert_eq!(first.status.code(), Some(40));
    let first_response = parse_json(&first);
    let approval_id = first_response["details"]["approval_id"]
        .as_str()
        .expect("approval ID should be returned")
        .to_owned();
    let session_id = first_response["details"]["session_id"]
        .as_str()
        .expect("agent approval should include its session")
        .to_owned();

    let resolved = fixture
        .command(&["approval", "resolve", &approval_id, "--allow", "--json"])
        .output()
        .expect("approval resolution should start");
    assert_success_with_context(&resolved, "approval resolve");

    let resumed = fixture
        .command(&[
            "run",
            "--agent",
            "--approval",
            &approval_id,
            "--session",
            &session_id,
            "Continue after approval",
            "--json",
        ])
        .env("PANDORA_PROVIDER_API_KEY", "test-agent-key")
        .output()
        .expect("approved agent run should start");
    assert_success_with_context(&resumed, "approved agent resume");
    assert_eq!(parse_json(&resumed)["output"], "approved");
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("README.md")).unwrap(),
        "approved"
    );

    let inspected = fixture
        .command(&["approval", "inspect", &approval_id, "--json"])
        .output()
        .expect("approval inspection should start");
    assert_success(&inspected);
    assert_eq!(parse_json(&inspected)["approval"]["status"], "consumed");
    server.join().expect("agent fixture should finish");
}

#[test]
fn chat_approves_and_resumes_the_pending_agent_task() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("agent fixture should bind");
    let address = listener
        .local_addr()
        .expect("agent fixture should expose its address");
    let server = thread::spawn(move || {
        let first_response = br#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"workspace.patch","arguments":"{\"path\":\"README.md\",\"content\":\"approved\"}"}}]}}],"usage":{"prompt_tokens":4,"completion_tokens":2}}"#;
        let second_response =
            br#"{"choices":[{"message":{"content":"approved"}}],"usage":{"prompt_tokens":5,"completion_tokens":2}}"#;
        for response in [first_response.as_slice(), second_response.as_slice()] {
            let (mut stream, _) = listener.accept().expect("agent should connect");
            let mut request = Vec::new();
            let header_end = loop {
                let mut chunk = [0_u8; 1_024];
                let bytes_read = stream.read(&mut chunk).expect("agent request should read");
                request.extend_from_slice(&chunk[..bytes_read]);
                if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    break position + 4;
                }
            };
            let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
            let content_length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .expect("agent should send a content length");
            while request.len() < header_end + content_length {
                let mut chunk = [0_u8; 1_024];
                let bytes_read = stream.read(&mut chunk).expect("agent body should read");
                request.extend_from_slice(&chunk[..bytes_read]);
            }
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            )
            .expect("agent response headers should be written");
            stream
                .write_all(response)
                .expect("agent response should be written");
        }
    });

    let fixture = Fixture::new();
    let provider_url = format!("http://{address}/v1");
    let configured = fixture
        .command(&[
            "provider",
            "set",
            "--provider-url",
            &provider_url,
            "--model",
            "agent-model",
            "--json",
        ])
        .output()
        .expect("provider set should start");
    assert_success(&configured);

    let mut command = fixture.command(&["chat"]);
    command
        .env("PANDORA_PROVIDER_API_KEY", "test-agent-key")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("chat should start");
    child
        .stdin
        .take()
        .expect("chat should accept input")
        .write_all(b"Update the README\n/approve\n/exit\n")
        .expect("chat commands should be written");

    let output = child.wait_with_output().expect("chat should finish");
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("approval:"));
    assert!(stdout.contains("approved"));
    assert!(!stdout.contains("test-agent-key"));
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("README.md")).unwrap(),
        "approved"
    );

    server.join().expect("agent fixture should finish");
}

#[test]
fn run_plan_uses_structured_provider_output_before_governed_execution() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("planner fixture should bind");
    let address = listener
        .local_addr()
        .expect("planner fixture should expose its address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("planner should connect");
        let mut request = Vec::new();
        let header_end = loop {
            let mut chunk = [0_u8; 1_024];
            let bytes_read = stream
                .read(&mut chunk)
                .expect("planner request should read");
            request.extend_from_slice(&chunk[..bytes_read]);
            if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                break position + 4;
            }
        };
        let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
        let content_length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .expect("planner should send a content length");
        while request.len() < header_end + content_length {
            let mut chunk = [0_u8; 1_024];
            let bytes_read = stream
                .read(&mut chunk)
                .expect("planner request body should read");
            request.extend_from_slice(&chunk[..bytes_read]);
        }
        let body = String::from_utf8_lossy(&request[header_end..header_end + content_length]);
        assert!(body.contains("\"model\":\"planner-model\""));
        assert!(body.contains("inspect the README"));
        assert!(!body.contains("\"tools\":[{"));
        let response =
            br#"{"choices":[{"message":{"content":"{\"task\":\"read:README.md\"}"}}],"usage":{"prompt_tokens":3,"completion_tokens":2}}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response.len()
        )
        .expect("planner response headers should be written");
        stream
            .write_all(response)
            .expect("planner response should be written");
    });

    let fixture = Fixture::new();
    let provider_url = format!("http://{address}/v1");
    let configured = fixture
        .command(&[
            "provider",
            "set",
            "--provider-url",
            &provider_url,
            "--model",
            "planner-model",
            "--json",
        ])
        .output()
        .expect("provider set should start");
    assert_success(&configured);

    let output = fixture
        .command(&["run", "--plan", "inspect the README", "--json"])
        .env("PANDORA_PROVIDER_API_KEY", "test-planner-key")
        .output()
        .expect("planned run should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["status"], "completed");
    assert_eq!(response["output"], "fixture\n");
    assert_eq!(response["planning"]["enabled"], true);
    assert_eq!(response["planning"]["model"], "planner-model");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("test-planner-key"));

    server.join().expect("planner fixture should finish");
}

#[test]
fn run_plan_rejects_malformed_provider_output_before_execution() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("planner fixture should bind");
    let address = listener
        .local_addr()
        .expect("planner fixture should expose its address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("planner should connect");
        let mut request = [0_u8; 4_096];
        let _ = stream
            .read(&mut request)
            .expect("planner request should read");
        let response =
            br#"{"choices":[{"message":{"content":"not-json"}}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response.len()
        )
        .expect("planner response headers should be written");
        stream
            .write_all(response)
            .expect("planner response should be written");
    });

    let fixture = Fixture::new();
    let provider_url = format!("http://{address}/v1");
    let configured = fixture
        .command(&[
            "provider",
            "set",
            "--provider-url",
            &provider_url,
            "--model",
            "planner-model",
            "--json",
        ])
        .output()
        .expect("provider set should start");
    assert_success(&configured);

    let output = fixture
        .command(&["run", "--plan", "inspect the README", "--json"])
        .env("PANDORA_PROVIDER_API_KEY", "test-planner-key")
        .output()
        .expect("planned run should start");
    assert_eq!(output.status.code(), Some(20));
    let response = parse_json(&output);
    assert_eq!(response["code"], "provider_error");
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("README.md")).unwrap(),
        "fixture\n"
    );

    server.join().expect("planner fixture should finish");
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
fn doctor_accepts_a_valid_local_only_setup_without_a_provider() {
    let fixture = Fixture::new();
    let mut command = fixture.command(&["setup", "--interactive", "--json"]);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("interactive setup should start");
    child
        .stdin
        .take()
        .expect("interactive setup should accept input")
        .write_all(b"\n")
        .expect("local-only setup answer should be written");
    let setup = child
        .wait_with_output()
        .expect("interactive setup should finish");
    assert_success(&setup);

    let output = fixture
        .command(&["doctor", "--json"])
        .output()
        .expect("doctor should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["healthy"], true);
    assert_eq!(response["provider"]["configured"], false);
    assert_eq!(response["provider"]["connectivity"], "not_configured");
    let storage_write = response["checks"]
        .as_array()
        .expect("doctor checks should be an array")
        .iter()
        .find(|check| check["check"] == "storage_write")
        .expect("doctor should check storage writeability");
    assert_eq!(storage_write["status"], "ok");
}

#[test]
fn doctor_rejects_a_configured_provider_without_a_credential() {
    let fixture = Fixture::new();
    fixture.setup();
    let output = fixture
        .command(&["doctor", "--json"])
        .env_remove("PANDORA_PROVIDER_API_KEY")
        .output()
        .expect("doctor should start");

    assert_eq!(output.status.code(), Some(10));
    let response = parse_json(&output);
    assert_eq!(response["code"], "configuration_error");
    assert_eq!(response["details"]["healthy"], false);
    assert_eq!(response["details"]["provider"]["credential"], "missing");
    assert!(
        response["details"]["checks"]
            .as_array()
            .expect("diagnostic checks should be an array")
            .iter()
            .any(|check| check["check"] == "credential" && check["status"] == "failed")
    );
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
    let harnesses = response["harnesses"].as_array().unwrap();
    assert!(harnesses.iter().any(|harness| {
        harness["id"] == "core-source"
            && harness["kind"] == "source"
            && harness["constitutional_service"] == "pandora-runtime"
            && harness["constitutional_service_version"] == env!("CARGO_PKG_VERSION")
            && harness["execution"]["runnable"] == false
            && harness["execution"]["mode"] == "system_augmentation"
    }));
    assert!(
        harnesses
            .iter()
            .any(|harness| harness["id"] == "coding-domain")
    );
    let meta = harnesses
        .iter()
        .find(|harness| harness["id"] == "coordination-meta")
        .expect("coordination Meta Harness should be discoverable");
    assert_eq!(meta["kind"], "meta");
    assert_eq!(meta["execution"]["runnable"], false);
    assert_eq!(meta["execution"]["mode"], "composition_only");
    assert_eq!(meta["meta_composition"]["max_handoffs"], 8);
    assert_eq!(
        meta["meta_composition"]["allowed_domains"][0],
        "coding-domain"
    );

    let output = fixture
        .command(&["harness", "inspect", "core-source", "--json"])
        .output()
        .expect("source harness inspect should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["harness"]["kind"], "source");
    assert_eq!(response["harness"]["execution"]["runnable"], false);
    assert_eq!(
        response["harness"]["constitutional_service"],
        "pandora-runtime"
    );
    assert_eq!(
        response["harness"]["constitutional_service_version"],
        env!("CARGO_PKG_VERSION")
    );
    assert!(response["harness"]["genes"].as_array().unwrap().is_empty());

    let output = fixture
        .command(&["harness", "inspect", "coordination-meta", "--json"])
        .output()
        .expect("Meta harness inspect should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["harness"]["kind"], "meta");
    assert_eq!(response["harness"]["execution"]["runnable"], false);
    assert_eq!(response["harness"]["meta_composition"]["max_handoffs"], 8);
    assert!(response["harness"]["genes"].as_array().unwrap().is_empty());

    let output = fixture
        .command(&["harness", "inspect", "coding", "--json"])
        .output()
        .expect("harness inspect should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["harness"]["kind"], "domain");
    assert_eq!(response["harness"]["execution"]["runnable"], true);
    assert_eq!(response["harness"]["execution"]["mode"], "domain_execution");
    assert!(response["harness"]["genes"].as_array().unwrap().len() >= 5);

    let output = fixture
        .command(&["harness", "inspect", "coding-domain", "--json"])
        .output()
        .expect("canonical harness inspect should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["harness"]["id"], "coding-domain");

    let output = fixture
        .command(&["harness", "run", "core-source", "--json"])
        .output()
        .expect("source harness run should start");
    assert_eq!(output.status.code(), Some(50));
    let response = parse_json(&output);
    assert_eq!(response["code"], "execution_failed");
    assert_eq!(response["message"], "harness 'core-source' is not runnable");
    assert_eq!(response["details"]["kind"], "source");

    let output = fixture
        .command(&[
            "run",
            "--harness",
            "core-source",
            "read:README.md",
            "--json",
        ])
        .output()
        .expect("direct source harness run should start");
    assert_eq!(output.status.code(), Some(50));
    let response = parse_json(&output);
    assert_eq!(response["code"], "execution_failed");
    assert_eq!(response["message"], "requested harness is not runnable");
    assert_eq!(response["details"]["harness_id"], "core-source");
    assert_eq!(response["details"]["kind"], "source");

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

    let output = fixture
        .command(&[
            "run",
            "--harness",
            "coding",
            "--gene",
            "workspace.read",
            "read:README.md",
            "--json",
        ])
        .output()
        .expect("explicit harness run should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["harness_id"], "coding-domain");
}

#[test]
fn direct_run_rejects_unclassified_tasks_without_a_phantom_harness() {
    let fixture = Fixture::new();
    fixture.setup();

    let output = fixture
        .command(&["run", "summarize the workspace", "--json"])
        .output()
        .expect("direct run should start");
    assert_eq!(output.status.code(), Some(50));
    let response = parse_json(&output);
    assert_eq!(response["code"], "execution_failed");
    assert_eq!(response["message"], "no default harness is available");
}

#[test]
fn package_meta_admission_survives_cli_restart_without_runtime_authority() {
    let fixture = Fixture::new();
    fixture.setup();
    let artifact = b"custom meta profile\n";
    let manifest = PackageManifest::new_meta(
        "example/meta",
        "1.0.0",
        "local-publisher",
        hash_artifact(artifact),
        Vec::new(),
        PackageCompatibility::new(concat!("pandora>=", env!("CARGO_PKG_VERSION"))).unwrap(),
        "Apache-2.0",
        TrustEvidence::unsigned(),
        MetaComposition::new(vec![HarnessId::new("coding-domain").unwrap()], 4).unwrap(),
    )
    .unwrap();
    let manifest_path = fixture.root.join("meta.json");
    let artifact_path = fixture.root.join("meta.artifact");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("manifest should serialize"),
    )
    .expect("manifest should be written");
    fs::write(&artifact_path, artifact).expect("artifact should be written");

    let output = fixture
        .command(&[
            "package",
            "admit",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--artifact",
            artifact_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("package admission should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["package"]["kind"], "meta_harness");
    assert_eq!(response["package"]["state"], "admitted");
    assert_eq!(response["package"]["runtime_authority"], false);

    let output = fixture
        .command(&["package", "list", "--json"])
        .output()
        .expect("package listing should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["packages"].as_array().unwrap().len(), 1);
    assert_eq!(response["packages"][0]["id"], "example/meta");
    assert_eq!(
        response["packages"][0]["meta_composition"]["max_handoffs"],
        4
    );

    let output = fixture
        .command(&["harness", "list", "--json"])
        .output()
        .expect("harness listing should start");
    assert_success(&output);
    let response = parse_json(&output);
    let admitted = response["package_records"]
        .as_array()
        .expect("harness listing should expose local package records");
    assert_eq!(admitted.len(), 1);
    assert_eq!(admitted[0]["id"], "example/meta");
    assert_eq!(admitted[0]["kind"], "meta_harness");
    assert_eq!(admitted[0]["state"], "admitted");
    assert_eq!(admitted[0]["runtime_authority"], false);
    let profiles = response["admitted_profiles"]
        .as_array()
        .expect("harness listing should expose admitted profiles");
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0]["id"], "example/meta");
    assert_eq!(profiles[0]["kind"], "meta");
    assert_eq!(profiles[0]["package_kind"], "meta_harness");
    assert_eq!(profiles[0]["execution"]["runnable"], false);
    assert_eq!(profiles[0]["execution"]["mode"], "composition_only");
    assert_eq!(profiles[0]["runtime_authority"], false);

    let output = fixture
        .command(&["package", "inspect", "example/meta", "1.0.0", "--json"])
        .output()
        .expect("package inspection should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["package"]["state"], "admitted");

    let output = fixture
        .command(&[
            "harness",
            "inspect",
            "example/meta",
            "--harness-version",
            "1.0.0",
            "--json",
        ])
        .output()
        .expect("Meta profile inspection should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["harness"]["kind"], "meta");
    assert_eq!(response["harness"]["execution"]["runnable"], false);
    assert_eq!(response["harness"]["meta_composition"]["max_handoffs"], 4);

    let output = fixture
        .command(&[
            "harness",
            "run",
            "example/meta",
            "--harness-version",
            "1.0.0",
            "--gene",
            "workspace.read",
            "--task",
            "read:README.md",
            "--json",
        ])
        .output()
        .expect("Meta profile run should start");
    assert_eq!(output.status.code(), Some(50));
    let response = parse_json(&output);
    assert_eq!(response["code"], "execution_failed");
    assert_eq!(response["message"], "requested harness is not runnable");
    assert_eq!(response["details"]["kind"], "meta");

    let output = fixture
        .command(&["session", "list", "--json"])
        .output()
        .expect("session listing should start");
    assert_success(&output);
    assert!(
        parse_json(&output)["sessions"]
            .as_array()
            .expect("sessions should be an array")
            .is_empty(),
        "a non-runnable Harness must not create a session"
    );

    let output = fixture
        .command(&["package", "remove", "example/meta", "1.0.0", "--json"])
        .output()
        .expect("unconfirmed package removal should start");
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(parse_json(&output)["code"], "usage_error");

    let output = fixture
        .command(&[
            "package",
            "remove",
            "example/meta",
            "1.0.0",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("package removal dry-run should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["dry_run"], true);
    assert_eq!(response["removed"], false);

    let output = fixture
        .command(&[
            "package",
            "remove",
            "example/meta",
            "1.0.0",
            "--yes",
            "--json",
        ])
        .output()
        .expect("package removal should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["dry_run"], false);
    assert_eq!(response["removed"], true);

    let output = fixture
        .command(&["package", "inspect", "example/meta", "1.0.0", "--json"])
        .output()
        .expect("removed package inspection should start");
    assert_eq!(output.status.code(), Some(50));
    assert_eq!(parse_json(&output)["code"], "execution_failed");
}

#[test]
fn admitted_domain_profile_runs_with_an_explicit_version() {
    let fixture = Fixture::new();
    fixture.setup();
    let artifact = b"domain profile\n";
    let manifest = PackageManifest::new(
        "example/domain",
        "1.0.0",
        PackageKind::DomainHarness,
        "local-publisher",
        hash_artifact(artifact),
        vec![PackageDependency::new("workspace.read", "0.1.0", false).unwrap()],
        PackageCompatibility::new(concat!("pandora>=", env!("CARGO_PKG_VERSION"))).unwrap(),
        "Apache-2.0",
        TrustEvidence::unsigned(),
    )
    .unwrap();
    let manifest_path = fixture.root.join("domain.json");
    let artifact_path = fixture.root.join("domain.artifact");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("manifest should be written"),
    )
    .expect("manifest should be written");
    fs::write(&artifact_path, artifact).expect("artifact should be written");

    let output = fixture
        .command(&[
            "package",
            "admit",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--artifact",
            artifact_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("domain profile admission should start");
    assert_success(&output);
    assert_eq!(parse_json(&output)["package"]["state"], "admitted");

    let output = fixture
        .command(&["harness", "list", "--json"])
        .output()
        .expect("harness listing should start");
    assert_success(&output);
    let response = parse_json(&output);
    let profiles = response["admitted_profiles"]
        .as_array()
        .expect("harness listing should expose admitted profiles");
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0]["id"], "example/domain");
    assert_eq!(profiles[0]["kind"], "domain");
    assert_eq!(profiles[0]["package_kind"], "domain_harness");
    assert_eq!(profiles[0]["execution"]["runnable"], true);
    assert_eq!(profiles[0]["execution"]["mode"], "domain_execution");
    assert_eq!(profiles[0]["runtime_authority"], false);

    let output = fixture
        .command(&[
            "harness",
            "inspect",
            "example/domain",
            "--harness-version",
            "1.0.0",
            "--json",
        ])
        .output()
        .expect("domain profile inspection should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["harness"]["id"], "example/domain");
    assert_eq!(response["harness"]["kind"], "domain");
    assert_eq!(response["harness"]["execution"]["runnable"], true);
    assert_eq!(response["harness"]["genes"][0]["id"], "workspace.read");

    let output = fixture
        .command(&[
            "run",
            "--harness",
            "example/domain",
            "--harness-version",
            "1.0.0",
            "--gene",
            "workspace.read",
            "read:README.md",
            "--json",
        ])
        .output()
        .expect("domain profile run should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["status"], "completed");
    assert_eq!(response["harness_id"], "example/domain");
    assert_eq!(response["gene_id"], "workspace.read");

    let output = fixture
        .command(&[
            "harness",
            "run",
            "example/domain",
            "--harness-version",
            "1.0.0",
            "--gene",
            "workspace.read",
            "--task",
            "read:README.md",
            "--json",
        ])
        .output()
        .expect("custom Domain Harness command should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["command"], "harness run");
    assert_eq!(response["status"], "completed");
    assert_eq!(response["harness_id"], "example/domain");
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

#[test]
fn approved_patch_resumes_once_through_the_governed_executor() {
    let fixture = Fixture::new();
    fixture.setup();
    let output = fixture
        .command(&["run", "patch:README.md:changed", "--json"])
        .output()
        .expect("run should start");
    assert_eq!(output.status.code(), Some(40));
    let approval_id = parse_json(&output)["details"]["approval_id"]
        .as_str()
        .expect("approval ID should be returned")
        .to_owned();

    let output = fixture
        .command(&["approval", "resolve", &approval_id, "--allow", "--json"])
        .output()
        .expect("approval resolution should start");
    assert_success(&output);

    let output = fixture
        .command(&[
            "run",
            "--approval",
            &approval_id,
            "patch:README.md:changed",
            "--json",
        ])
        .output()
        .expect("approved run should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["status"], "completed");
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("README.md")).unwrap(),
        "changed"
    );

    let output = fixture
        .command(&["approval", "inspect", &approval_id, "--json"])
        .output()
        .expect("approval inspection should start");
    assert_success(&output);
    assert_eq!(parse_json(&output)["approval"]["status"], "consumed");

    let output = fixture
        .command(&[
            "run",
            "--approval",
            &approval_id,
            "patch:README.md:changed-again",
            "--json",
        ])
        .output()
        .expect("replayed approval run should start");
    assert_eq!(output.status.code(), Some(40));
    assert_eq!(parse_json(&output)["code"], "approval_required");
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("README.md")).unwrap(),
        "changed"
    );
}

#[test]
fn completions_generate_a_bash_script() {
    let fixture = Fixture::new();
    let output = fixture
        .command(&["completions", "bash", "--json"])
        .output()
        .expect("completion generation should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["command"], "completions bash");
    assert!(response["script"].as_str().unwrap().contains("pandora"));
    assert!(response["script"].as_str().unwrap().contains("skill"));
}

#[test]
fn completions_include_session_inspect_for_each_shell() {
    let fixture = Fixture::new();
    let expectations = [
        (
            "powershell",
            "if ($elements.Count -gt 1 -and $elements[1] -eq 'session')",
            "'list','resume','inspect'",
        ),
        (
            "bash",
            "if [[ \"$previous\" == \"session\" ]]",
            "compgen -W 'list resume inspect'",
        ),
        (
            "zsh",
            "if [[ ${words[2]} == session ]]",
            "'2:session command:(list resume inspect)'",
        ),
        (
            "fish",
            "__fish_seen_subcommand_from session",
            "list resume inspect",
        ),
    ];
    for (shell, parent_condition, subcommands) in expectations {
        let output = fixture
            .command(&["completions", shell, "--json"])
            .output()
            .expect("completion generation should start");
        assert_success(&output);
        let response = parse_json(&output);
        let script = response["script"].as_str().unwrap();
        assert!(!script.contains("session inspect"));
        assert!(script.contains("chat"));
        assert!(script.contains("tui"));
        assert!(script.contains("package"));
        assert!(script.contains(parent_condition));
        assert!(script.contains(subcommands));
        if shell == "zsh" {
            assert!(!script.contains(
                "_arguments \\\n+    '1:command:(setup run harness session skill approval provider tool orchestration strategies completions migrate update uninstall doctor)' \\\n+    '2:session command:(list resume inspect)'"
            ));
        }
    }
}

#[test]
fn migration_converts_legacy_config_and_keeps_a_backup() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.root.join("data")).expect("legacy data should exist");
    fs::write(
        &fixture.config,
        r#"{"provider":{"url":"http://127.0.0.1:4317/v1"},"provider_model":"legacy-model","data_path":"data","workspace_path":"workspace"}"#,
    )
    .expect("legacy config should be written");

    let output = fixture
        .command(&[
            "migrate",
            "config",
            "--config",
            fixture.config.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("migration should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["command"], "migrate config");
    assert!(fixture.config.with_extension("json.bak").is_file());
    let current: Value = serde_json::from_slice(&fs::read(&fixture.config).unwrap()).unwrap();
    assert_eq!(current["format_version"], 1);
    assert_eq!(current["provider_url"], "http://127.0.0.1:4317/v1");
    assert_eq!(current["provider_model"], "legacy-model");
}

#[test]
fn migration_preserves_invalid_config() {
    let fixture = Fixture::new();
    let original = b"{not-json";
    fs::write(&fixture.config, original).expect("invalid config should be written");

    let output = fixture
        .command(&[
            "migrate",
            "config",
            "--config",
            fixture.config.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("migration should start");
    assert_eq!(output.status.code(), Some(10));
    assert_eq!(fs::read(&fixture.config).unwrap(), original);
    assert!(!fixture.config.with_extension("json.bak").exists());
}

#[test]
fn update_rejects_a_checksum_mismatch() {
    let fixture = Fixture::new();
    let artifact = fixture.root.join("pandora.bin");
    fs::write(&artifact, b"verified artifact").expect("artifact should be written");
    let output = fixture
        .command(&[
            "update",
            "--artifact",
            artifact.to_str().unwrap(),
            "--sha256",
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("update should start");
    assert_eq!(output.status.code(), Some(70));
    let response = parse_json(&output);
    assert_eq!(response["code"], "update_error");
    assert_eq!(response["details"]["reason"], "checksum_mismatch");
}

#[test]
fn update_can_rollback_the_previous_verified_artifact() {
    let fixture = Fixture::new();
    let first = fixture.root.join("first.bin");
    let second = fixture.root.join("second.bin");
    fs::write(&first, b"first").expect("first artifact should be written");
    fs::write(&second, b"second").expect("second artifact should be written");
    for (artifact, content) in [
        (&first, b"first".as_slice()),
        (&second, b"second".as_slice()),
    ] {
        let output = fixture
            .command(&[
                "update",
                "--artifact",
                artifact.to_str().unwrap(),
                "--sha256",
                hash_artifact(content).as_str(),
                "--json",
            ])
            .output()
            .expect("update should start");
        assert_success(&output);
    }
    let output = fixture
        .command(&["update", "--rollback", "--json"])
        .output()
        .expect("rollback should start");
    assert_success(&output);
    assert_eq!(
        fs::read(fixture.data.join("updates/current/pandora")).unwrap(),
        b"first"
    );
}

#[test]
fn uninstall_dry_run_preserves_user_data() {
    let fixture = Fixture::new();
    fixture.setup();
    let output = fixture
        .command(&["uninstall", "--dry-run", "--json"])
        .output()
        .expect("uninstall should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["dry_run"], true);
    assert!(fixture.config.is_file());
    assert!(fixture.data.is_dir());
    assert!(fixture.workspace.is_dir());
}

#[test]
fn uninstall_handles_a_config_file_stored_under_the_data_root() {
    let fixture = Fixture::new();
    let config = fixture.data.join("config.json");
    let config = config.to_str().unwrap();
    let output = fixture
        .command(&[
            "setup",
            "--config",
            config,
            "--provider-url",
            "http://127.0.0.1:4317/v1",
            "--json",
        ])
        .output()
        .expect("setup should start");
    assert_success(&output);

    let output = fixture
        .command(&["uninstall", "--config", config, "--yes", "--json"])
        .output()
        .expect("uninstall should start");
    assert_success(&output);
    assert!(!fixture.data.exists());
    assert!(fixture.workspace.is_dir());
}

#[test]
fn uninstall_refuses_a_data_root_that_contains_the_workspace() {
    let fixture = Fixture::new();
    let data_dir = fixture.root.to_str().unwrap();
    let workspace = fixture.workspace.to_str().unwrap();
    let output = fixture
        .command(&[
            "setup",
            "--data-dir",
            data_dir,
            "--workspace",
            workspace,
            "--provider-url",
            "http://127.0.0.1:4317/v1",
            "--json",
        ])
        .output()
        .expect("setup should start");
    assert_success(&output);

    let output = fixture
        .command(&[
            "uninstall",
            "--data-dir",
            data_dir,
            "--workspace",
            workspace,
            "--yes",
            "--json",
        ])
        .output()
        .expect("uninstall should start");
    assert_eq!(output.status.code(), Some(10));
    let response = parse_json(&output);
    assert_eq!(response["code"], "configuration_error");
    assert!(
        response["message"]
            .as_str()
            .unwrap_or_default()
            .contains("contains the workspace")
    );
    assert!(fixture.workspace.is_dir());
}

#[test]
fn doctor_reports_connectivity_state_without_exposing_credentials() {
    let fixture = Fixture::new();
    fixture.setup();
    let output = fixture
        .command(&["doctor", "--json"])
        .env("PANDORA_PROVIDER_API_KEY", "sk-live-secret")
        .output()
        .expect("doctor should start");
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("sk-live-secret"));
    let response = parse_json(&output);
    assert_eq!(response["healthy"], true);
    assert_eq!(response["provider"]["credential"], "available");
    assert_eq!(response["provider"]["connectivity"], "not_checked");
    assert_eq!(response["policy"]["mode"], "governed");
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_success_with_context(output: &Output, command: &str) {
    assert!(
        output.status.success(),
        "{command} failed: {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
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
