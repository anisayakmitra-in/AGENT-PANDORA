use pandora_types::hash_artifact;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const CREDENTIAL_ENV: &str = "PANDORA_JSON_CONTRACT_KEY";
const CREDENTIAL_VALUE: &str = "json-contract-secret-value";
static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

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
        let root = std::env::temp_dir().join(format!(
            "pandora-json-contract-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).expect("workspace should be created");
        Self {
            config: root.join("config.json"),
            data: root.join("data"),
            workspace,
            root,
        }
    }

    fn run(&self, args: &[&str]) -> JsonResponse {
        let output = Command::new(env!("CARGO_BIN_EXE_pandora"))
            .args(args)
            .env("PANDORA_CONFIG", &self.config)
            .env("PANDORA_DATA_DIR", &self.data)
            .env("PANDORA_WORKSPACE", &self.workspace)
            .env(CREDENTIAL_ENV, CREDENTIAL_VALUE)
            .env_remove("PANDORA_PROVIDER_URL")
            .output()
            .expect("Pandora command should start");
        JsonResponse::from_output(output)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct JsonResponse {
    status: ExitStatus,
    text: String,
    value: Value,
}

impl JsonResponse {
    fn from_output(output: Output) -> Self {
        let text = String::from_utf8(output.stdout).expect("JSON output must be valid UTF-8");
        assert!(
            !text.contains(CREDENTIAL_VALUE),
            "JSON output must not contain credential values"
        );
        let value = serde_json::from_str(&text).expect("command output must be one JSON value");
        Self {
            status: output.status,
            text,
            value,
        }
    }

    fn success(self, command: &str) -> Value {
        assert!(self.status.success(), "command failed: {}", self.text);
        assert_eq!(self.value["version"], "0.1");
        assert_eq!(self.value["command"], command);
        self.value
    }

    fn error(self, code: &str, exit_code: i32) -> Value {
        assert_eq!(
            self.status.code(),
            Some(exit_code),
            "response: {}",
            self.text
        );
        assert_eq!(self.value["version"], "0.1");
        assert_eq!(self.value["code"], code);
        assert!(self.value["message"].is_string());
        assert!(self.value["details"].is_object());
        assert!(self.value.get("command").is_none());
        self.value
    }
}

#[test]
fn release_critical_success_envelopes_are_stable() {
    let fixture = Fixture::new();

    let version = fixture.run(&["--version", "--json"]).success("version");
    assert!(version["pandora_version"].is_string());

    let setup = fixture
        .run(&[
            "setup",
            "--provider-url",
            "https://provider.example/v1",
            "--model",
            "contract-model",
            "--api-key-env",
            CREDENTIAL_ENV,
            "--json",
        ])
        .success("setup");
    assert_eq!(setup["config_path"], path_value(&fixture.config));
    assert_eq!(setup["data_dir"], path_value(&fixture.data));
    assert_eq!(setup["workspace"], path_value(&fixture.workspace));
    assert_eq!(setup["provider_configured"], true);
    assert_eq!(setup["provider_model"], "contract-model");
    assert_eq!(setup["api_key_env"], CREDENTIAL_ENV);
    assert_eq!(setup["interactive"], false);

    let doctor = fixture.run(&["doctor", "--json"]).success("doctor");
    assert_eq!(doctor["healthy"], true);
    assert!(doctor["platform"].is_object());
    assert_eq!(doctor["config_path"], path_value(&fixture.config));
    assert_eq!(doctor["storage_path"], path_value(&fixture.data));
    assert_eq!(doctor["workspace_path"], path_value(&fixture.workspace));
    assert_eq!(doctor["provider"]["configured"], true);
    assert_eq!(doctor["provider"]["credential"], "available");
    assert_eq!(doctor["policy"]["effect_boundary"], "reference_monitor");
    assert!(doctor["containment"].is_object());
    assert!(doctor["checks"].is_array());

    let target = fixture.root.join("installed-pandora");
    let artifact = fixture.root.join("candidate.bin");
    let previous = b"previous release bytes";
    let candidate = b"verified candidate bytes";
    fs::write(&target, previous).expect("previous target should be written");
    fs::write(&artifact, candidate).expect("candidate should be written");
    let checksum = hash_artifact(candidate);
    let update = fixture
        .run(&[
            "update",
            "--artifact",
            artifact.to_str().unwrap(),
            "--sha256",
            &checksum,
            "--target",
            target.to_str().unwrap(),
            "--json",
        ])
        .success("update");
    assert_eq!(update["verified"], true);
    assert_eq!(update["artifact"], path_value(&artifact));
    assert_eq!(update["target"], path_value(&target));
    assert_eq!(update["signature_verified"], false);
    assert_eq!(update["dry_run"], false);

    let rollback = fixture
        .run(&[
            "update",
            "--rollback",
            "--target",
            target.to_str().unwrap(),
            "--json",
        ])
        .success("update rollback");
    assert_eq!(rollback["target"], path_value(&target));
    assert_eq!(rollback["restored"], true);
    assert_eq!(rollback["dry_run"], false);
    assert_eq!(fs::read(&target).unwrap(), previous);

    let preview = fixture
        .run(&["uninstall", "--dry-run", "--json"])
        .success("uninstall");
    assert_eq!(preview["dry_run"], true);
    assert!(preview["would_remove"].is_array());
    assert_eq!(preview["preserved"][0], path_value(&fixture.workspace));

    let uninstall = fixture
        .run(&["uninstall", "--yes", "--json"])
        .success("uninstall");
    assert_eq!(uninstall["dry_run"], false);
    assert!(uninstall["removed"].is_array());
    assert_eq!(uninstall["preserved"][0], path_value(&fixture.workspace));
}

#[test]
fn release_critical_error_envelopes_match_process_exit_codes() {
    let fixture = Fixture::new();

    let usage = fixture.run(&["update", "--json"]).error("usage_error", 2);
    assert_eq!(usage["details"], serde_json::json!({}));

    let configuration = fixture
        .run(&["doctor", "--json"])
        .error("configuration_error", 10);
    assert_eq!(configuration["details"]["healthy"], false);

    let artifact = fixture.root.join("candidate.bin");
    fs::write(&artifact, b"candidate").expect("candidate should be written");
    let target = fixture.root.join("target");
    let update = fixture
        .run(&[
            "update",
            "--artifact",
            artifact.to_str().unwrap(),
            "--sha256",
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "--target",
            target.to_str().unwrap(),
            "--json",
        ])
        .error("update_error", 70);
    assert_eq!(update["details"]["reason"], "checksum_mismatch");
    assert_eq!(update["details"]["path"], path_value(&artifact));
}

fn path_value(path: &Path) -> Value {
    Value::String(path.to_string_lossy().into_owned())
}
