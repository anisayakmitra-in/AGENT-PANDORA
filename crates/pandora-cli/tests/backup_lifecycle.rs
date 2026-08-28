use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

struct Fixture {
    root: PathBuf,
    config: PathBuf,
    data: PathBuf,
    workspace: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "pandora-backup-lifecycle-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        Self {
            config: root.join("config.json"),
            data: root.join("data"),
            workspace,
            root,
        }
    }

    fn command(&self, args: &[&str], key: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_pandora"))
            .args(args)
            .env("PANDORA_CONFIG", &self.config)
            .env("PANDORA_DATA_DIR", &self.data)
            .env("PANDORA_WORKSPACE", &self.workspace)
            .env("PANDORA_BACKUP_KEY", key)
            .env_remove("PANDORA_PROVIDER_URL")
            .output()
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn encrypted_backup_restores_state_and_rejects_wrong_key() {
    let fixture = Fixture::new();
    parse_success(fixture.command(&["setup", "--json"], "correct horse battery staple"));
    let marker = fixture.data.join("state.marker");
    fs::write(&marker, b"before").unwrap();
    let archive = fixture.root.join("pandora-recovery.json");

    let created = parse_success(fixture.command(
        &[
            "backup",
            "create",
            "--output",
            archive.to_str().unwrap(),
            "--json",
        ],
        "correct horse battery staple",
    ));
    assert_eq!(created["command"], "backup create");
    assert_eq!(created["encrypted"], true);
    let encoded = fs::read(&archive).unwrap();
    assert!(!encoded.windows(6).any(|window| window == b"before"));

    let rejected = fixture.command(
        &[
            "backup",
            "inspect",
            "--input",
            archive.to_str().unwrap(),
            "--json",
        ],
        "definitely the wrong passphrase",
    );
    assert!(!rejected.status.success());

    fs::write(&marker, b"after").unwrap();
    let restored = parse_success(fixture.command(
        &[
            "backup",
            "restore",
            "--input",
            archive.to_str().unwrap(),
            "--yes",
            "--json",
        ],
        "correct horse battery staple",
    ));
    assert_eq!(restored["command"], "backup restore");
    assert_eq!(restored["authenticated"], true);
    assert_eq!(fs::read(marker).unwrap(), b"before");
}

fn parse_success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "command failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
