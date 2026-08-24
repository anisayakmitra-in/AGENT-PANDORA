use pandora_types::hash_artifact;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
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
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be available")
            .as_nanos();
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "pandora-release-lifecycle-{}-{timestamp}-{sequence}",
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

    fn command(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_pandora"))
            .args(args)
            .env("PANDORA_CONFIG", &self.config)
            .env("PANDORA_DATA_DIR", &self.data)
            .env("PANDORA_WORKSPACE", &self.workspace)
            .env_remove("PANDORA_PROVIDER_URL")
            .output()
            .expect("Pandora command should start")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn clean_state_lifecycle() {
    let fixture = Fixture::new();
    let sentinel = fixture.workspace.join("sentinel.txt");
    fs::write(&sentinel, b"preserve me").expect("sentinel should be written");

    let version = parse_success(fixture.command(&["--version", "--json"]));
    assert_eq!(version["version"], "0.1");
    assert_eq!(version["command"], "version");

    let setup = parse_success(fixture.command(&["setup", "--json"]));
    assert_eq!(setup["version"], "0.1");
    assert_eq!(setup["command"], "setup");
    assert_eq!(setup["config_path"], path_value(&fixture.config));
    assert_eq!(setup["data_dir"], path_value(&fixture.data));
    assert_eq!(setup["workspace"], path_value(&fixture.workspace));

    let doctor = parse_success(fixture.command(&["doctor", "--json"]));
    assert_eq!(doctor["version"], "0.1");
    assert_eq!(doctor["command"], "doctor");
    assert_eq!(doctor["healthy"], true);
    assert_eq!(doctor["provider"]["configured"], false);

    let preview = parse_success(fixture.command(&["uninstall", "--dry-run", "--json"]));
    assert_eq!(preview["version"], "0.1");
    assert_eq!(preview["command"], "uninstall");
    assert_eq!(preview["dry_run"], true);
    assert_eq!(
        preview["would_remove"],
        Value::Array(vec![path_value(&fixture.config), path_value(&fixture.data)])
    );
    assert_eq!(
        preview["preserved"],
        Value::Array(vec![path_value(&fixture.workspace)])
    );
    assert!(fixture.config.is_file());
    assert!(fixture.data.is_dir());
    assert_eq!(fs::read(&sentinel).unwrap(), b"preserve me");

    let uninstall = parse_success(fixture.command(&["uninstall", "--yes", "--json"]));
    assert_eq!(uninstall["version"], "0.1");
    assert_eq!(uninstall["command"], "uninstall");
    assert_eq!(uninstall["dry_run"], false);
    assert!(!fixture.config.exists());
    assert!(!fixture.data.exists());
    assert_eq!(fs::read(&sentinel).unwrap(), b"preserve me");
}

#[test]
fn verified_update_and_rollback() {
    let fixture = Fixture::new();
    let target = fixture.root.join("installed-pandora");
    let first = fixture.root.join("candidate-one.bin");
    let second = fixture.root.join("candidate-two.bin");
    let original = fs::read(env!("CARGO_BIN_EXE_pandora")).expect("CLI bytes should be readable");
    let first_bytes = b"verified candidate one";
    let second_bytes = b"verified candidate two";
    fs::write(&target, &original).expect("initial target should be written");
    make_executable(&target);
    fs::write(&first, first_bytes).expect("first candidate should be written");
    fs::write(&second, second_bytes).expect("second candidate should be written");

    install(&fixture, &first, first_bytes, &target);
    assert_eq!(fs::read(&target).unwrap(), first_bytes);
    assert_executable(&target);
    install(&fixture, &second, second_bytes, &target);
    assert_eq!(fs::read(&target).unwrap(), second_bytes);
    assert_executable(&target);

    let rollback = parse_success(fixture.command(&[
        "update",
        "--rollback",
        "--target",
        target.to_str().unwrap(),
        "--json",
    ]));
    assert_eq!(rollback["version"], "0.1");
    assert_eq!(rollback["command"], "update rollback");
    assert_eq!(rollback["restored"], true);
    assert_eq!(fs::read(&target).unwrap(), first_bytes);
    assert_executable(&target);
    assert!(!previous_path(&target).exists());
    assert!(fs::read_dir(&fixture.root).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".new-")
    }));
}

fn install(fixture: &Fixture, artifact: &Path, bytes: &[u8], target: &Path) {
    let checksum = hash_artifact(bytes);
    let output = fixture.command(&[
        "update",
        "--artifact",
        artifact.to_str().unwrap(),
        "--sha256",
        &checksum,
        "--target",
        target.to_str().unwrap(),
        "--json",
    ]);
    let response = parse_success(output);
    assert_eq!(response["version"], "0.1");
    assert_eq!(response["command"], "update");
    assert_eq!(response["verified"], true);
    assert_eq!(response["target"], path_value(target));
}

fn previous_path(target: &Path) -> PathBuf {
    let name = target.file_name().unwrap().to_string_lossy();
    target.parent().unwrap().join(format!(".{name}.previous"))
}

fn parse_success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "command failed with {}: stdout={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("command output should be JSON")
}

fn path_value(path: &Path) -> Value {
    Value::String(path.to_string_lossy().into_owned())
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
        .expect("initial target should be executable");
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}

#[cfg(unix)]
fn assert_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let mode = fs::metadata(path).unwrap().permissions().mode();
    assert_ne!(mode & 0o111, 0, "updated CLI should remain executable");
}

#[cfg(not(unix))]
fn assert_executable(_path: &Path) {}
