use serde_json::{Value, json};
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

    fn manifest(
        &self,
        name: &str,
        evidence_id: &str,
        provider: &str,
        action: &str,
        provider_fields: Value,
        digest_byte: char,
    ) -> PathBuf {
        let path = self.root.join(name);
        fs::write(
            &path,
            serde_json::to_vec_pretty(&json!({
                "policy_version": 1,
                "evidence_id": evidence_id,
                "policy_id": "retention:daily-30d",
                "provider": provider,
                "action": action,
                "resource_id": format!("resource:{evidence_id}"),
                "provider_fields": provider_fields,
                "external_evidence_digest": format!(
                    "sha256:{}",
                    digest_byte.to_string().repeat(64)
                ),
                "actor": "operator:alice",
                "performed_at": 1_788_192_000_u64
            }))
            .unwrap(),
        )
        .unwrap();
        path
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

#[test]
fn lifecycle_preview_is_non_mutating_and_record_is_idempotent_across_processes() {
    let fixture = Fixture::new();
    parse_success(fixture.command(&["setup", "--json"], "correct horse battery staple"));
    let manifest = fixture.manifest(
        "aws-backup-expired.json",
        "evidence:aws-backup-1",
        "aws_s3",
        "backup_expired",
        json!({
            "bucket": "backup-bucket",
            "deletion_marker_id": "marker-1",
            "object_key": "daily/archive.json",
            "version_id": "version-1"
        }),
        '1',
    );

    let preview = parse_success(fixture.command(
        &[
            "backup",
            "lifecycle",
            "preview",
            "--input",
            manifest.to_str().unwrap(),
            "--json",
        ],
        "correct horse battery staple",
    ));
    assert_eq!(preview["dry_run"], true);
    assert_eq!(preview["would_record"], true);
    assert_eq!(
        preview["boundary"]["external_action_performed_by_runtime"],
        false
    );
    assert!(!fixture.data.join("storage-lifecycle.sqlite3").exists());

    let recorded = parse_success(fixture.command(
        &[
            "backup",
            "lifecycle",
            "record",
            "--input",
            manifest.to_str().unwrap(),
            "--yes",
            "--json",
        ],
        "correct horse battery staple",
    ));
    assert_eq!(recorded["created"], true);
    assert_eq!(recorded["receipt"]["evidence_status"], "operator_attested");
    assert_eq!(
        recorded["receipt"]["external_action_performed_by_runtime"],
        false
    );
    assert_eq!(recorded["receipt"]["secure_erasure_guaranteed"], false);

    let retry = parse_success(fixture.command(
        &[
            "backup",
            "lifecycle",
            "record",
            "--input",
            manifest.to_str().unwrap(),
            "--yes",
            "--json",
        ],
        "correct horse battery staple",
    ));
    assert_eq!(retry["created"], false);
    assert_eq!(retry["idempotent_replay"], true);
    assert_eq!(
        retry["receipt"]["recorded_at"],
        recorded["receipt"]["recorded_at"]
    );

    let inspected = parse_success(fixture.command(
        &[
            "backup",
            "lifecycle",
            "inspect",
            "--id",
            "evidence:aws-backup-1",
            "--json",
        ],
        "correct horse battery staple",
    ));
    assert_eq!(inspected["receipt"]["manifest"]["provider"], "aws_s3");
    assert_eq!(inspected["receipt"]["manifest"]["action"], "backup_expired");

    fixture.manifest(
        "aws-backup-expired.json",
        "evidence:aws-backup-1",
        "aws_s3",
        "backup_expired",
        json!({
            "bucket": "backup-bucket",
            "deletion_marker_id": "marker-1",
            "object_key": "daily/archive.json",
            "version_id": "version-1"
        }),
        '2',
    );
    let conflict = parse_error(fixture.command(
        &[
            "backup",
            "lifecycle",
            "record",
            "--input",
            manifest.to_str().unwrap(),
            "--yes",
            "--json",
        ],
        "correct horse battery staple",
    ));
    assert_eq!(conflict["code"], "policy_denied");
    assert_eq!(
        conflict["details"]["external_action_performed_by_runtime"],
        false
    );
}

#[test]
fn lifecycle_ledger_covers_backup_snapshot_and_key_evidence_with_filters() {
    let fixture = Fixture::new();
    parse_success(fixture.command(&["setup", "--json"], "correct horse battery staple"));
    let manifests = [
        fixture.manifest(
            "aws-backup.json",
            "evidence:backup-1",
            "aws_s3",
            "backup_expired",
            json!({
                "bucket": "backup-bucket",
                "deletion_marker_id": "marker-1",
                "object_key": "daily/archive.json",
                "version_id": "version-1"
            }),
            '1',
        ),
        fixture.manifest(
            "local-snapshot.json",
            "evidence:snapshot-1",
            "local_filesystem",
            "snapshot_removed",
            json!({
                "deletion_event_id": "event-1",
                "snapshot_id": "snapshot-1"
            }),
            '2',
        ),
        fixture.manifest(
            "azure-key.json",
            "evidence:key-1",
            "azure_blob",
            "encryption_key_destroyed",
            json!({
                "key_name": "backup-key",
                "key_version": "version-1",
                "purge_event_id": "activity-1",
                "vault_uri": "https://vault.example/"
            }),
            '3',
        ),
    ];
    for manifest in &manifests {
        parse_success(fixture.command(
            &[
                "backup",
                "lifecycle",
                "record",
                "--input",
                manifest.to_str().unwrap(),
                "--yes",
                "--json",
            ],
            "correct horse battery staple",
        ));
    }

    let all = parse_success(fixture.command(
        &["backup", "lifecycle", "list", "--limit", "10", "--json"],
        "correct horse battery staple",
    ));
    assert_eq!(all["count"], 3);

    let keys = parse_success(fixture.command(
        &[
            "backup",
            "lifecycle",
            "list",
            "--action",
            "encryption_key_destroyed",
            "--storage-provider",
            "azure_blob",
            "--json",
        ],
        "correct horse battery staple",
    ));
    assert_eq!(keys["count"], 1);
    assert_eq!(
        keys["receipts"][0]["manifest"]["evidence_id"],
        "evidence:key-1"
    );
}

#[test]
fn lifecycle_manifest_rejects_provider_field_mismatch_before_opening_ledger() {
    let fixture = Fixture::new();
    parse_success(fixture.command(&["setup", "--json"], "correct horse battery staple"));
    let manifest = fixture.manifest(
        "invalid.json",
        "evidence:invalid-1",
        "gcp_cloud_storage",
        "snapshot_removed",
        json!({
            "snapshot_resource": "projects/example/global/snapshots/snapshot-1",
            "secret_material": "must-not-be-accepted"
        }),
        '1',
    );
    let rejected = parse_error(fixture.command(
        &[
            "backup",
            "lifecycle",
            "preview",
            "--input",
            manifest.to_str().unwrap(),
            "--json",
        ],
        "correct horse battery staple",
    ));
    assert_eq!(rejected["code"], "configuration_error");
    assert!(!fixture.data.join("storage-lifecycle.sqlite3").exists());
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

fn parse_error(output: Output) -> Value {
    assert!(
        !output.status.success(),
        "command unexpectedly succeeded: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
