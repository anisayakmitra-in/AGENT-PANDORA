use pandora_types::hash_artifact;
use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output};
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
        assert!(body.contains("\"model\":\"default\""));
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
        .command(&["provider", "set", "--provider-url", &provider_url, "--json"])
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
    assert_eq!(response["model"], "default");
    assert_eq!(response["output"], "ready");
    assert_eq!(response["usage"]["total_tokens"], 3);

    server.join().expect("provider fixture should finish");
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
}

#[test]
fn migration_converts_legacy_config_and_keeps_a_backup() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.root.join("data")).expect("legacy data should exist");
    fs::write(
        &fixture.config,
        r#"{"provider":{"url":"http://127.0.0.1:4317/v1"},"data_path":"data","workspace_path":"workspace"}"#,
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
fn doctor_reports_connectivity_state_without_credentials() {
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

fn parse_json(output: &Output) -> Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).unwrap_or_else(|error| {
        panic!(
            "expected JSON output, got {error}: {stdout}; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}
