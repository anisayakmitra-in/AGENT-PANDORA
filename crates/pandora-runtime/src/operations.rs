use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_TELEMETRY_BYTES: u64 = 4 * 1024 * 1024;
const MAX_CRASH_REPORTS: usize = 20;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationalEvent {
    CliInvocation,
    ServiceStarted,
    ServiceStopped,
    BackupCreated,
    RestoreCompleted,
    UpdateChecked,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationalStatus {
    Succeeded,
    Failed,
}

#[derive(Clone)]
pub struct OperationalRecorder {
    directory: PathBuf,
    component: &'static str,
    version: &'static str,
}

impl OperationalRecorder {
    pub fn new(
        data_directory: impl AsRef<Path>,
        component: &'static str,
        version: &'static str,
    ) -> Self {
        Self {
            directory: data_directory.as_ref().join("operations"),
            component,
            version,
        }
    }

    pub fn record(&self, event: OperationalEvent, status: OperationalStatus) {
        let _ = self.try_record(event, status);
    }

    pub fn install_crash_reporter(&self) {
        let recorder = self.clone();
        std::panic::set_hook(Box::new(move |info| {
            let _ = recorder.write_crash_report(info);
        }));
    }

    fn try_record(
        &self,
        event: OperationalEvent,
        status: OperationalStatus,
    ) -> std::io::Result<()> {
        fs::create_dir_all(&self.directory)?;
        let path = self.directory.join("telemetry.jsonl");
        if fs::metadata(&path).is_ok_and(|metadata| metadata.len() >= MAX_TELEMETRY_BYTES) {
            let rotated = self.directory.join("telemetry.previous.jsonl");
            let _ = fs::remove_file(&rotated);
            fs::rename(&path, rotated)?;
        }
        let record = TelemetryRecord {
            schema_version: 1,
            occurred_at: now(),
            component: self.component,
            version: self.version,
            event,
            status,
        };
        let mut bytes = serde_json::to_vec(&record).map_err(std::io::Error::other)?;
        bytes.push(b'\n');
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        set_private_permissions(&file)?;
        file.write_all(&bytes)
    }

    fn write_crash_report(&self, info: &PanicHookInfo<'_>) -> std::io::Result<()> {
        let crash_directory = self.directory.join("crashes");
        fs::create_dir_all(&crash_directory)?;
        prune_crash_reports(&crash_directory)?;
        let occurred_at = now();
        let location_digest = info.location().map(|location| {
            let mut hasher = Sha256::new();
            hasher.update(b"pandora-crash-location-v1\0");
            hasher.update(location.file().as_bytes());
            hasher.update(location.line().to_be_bytes());
            hasher.update(location.column().to_be_bytes());
            encode_hex(&hasher.finalize())
        });
        let report = CrashRecord {
            schema_version: 1,
            occurred_at,
            component: self.component,
            version: self.version,
            location_digest,
            panic_payload_recorded: false,
        };
        let path = crash_directory.join(format!("crash-{occurred_at}-{}.json", std::process::id()));
        let bytes = serde_json::to_vec_pretty(&report).map_err(std::io::Error::other)?;
        let mut file = atomic_write_file::AtomicWriteFile::open(path)?;
        set_private_permissions(file.as_file())?;
        file.write_all(&bytes)?;
        file.commit()
    }
}

#[derive(Serialize)]
struct TelemetryRecord {
    schema_version: u32,
    occurred_at: u64,
    component: &'static str,
    version: &'static str,
    event: OperationalEvent,
    status: OperationalStatus,
}

#[derive(Serialize)]
struct CrashRecord {
    schema_version: u32,
    occurred_at: u64,
    component: &'static str,
    version: &'static str,
    location_digest: Option<String>,
    panic_payload_recorded: bool,
}

fn prune_crash_reports(directory: &Path) -> std::io::Result<()> {
    let mut reports = fs::read_dir(directory)?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_file())
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with("crash-") && name.ends_with(".json"))
        })
        .collect::<Vec<_>>();
    reports.sort_by_key(|entry| entry.file_name());
    let remove_count = reports.len().saturating_sub(MAX_CRASH_REPORTS - 1);
    for entry in reports.into_iter().take(remove_count) {
        let _ = fs::remove_file(entry.path());
    }
    Ok(())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn set_private_permissions(file: &fs::File) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = file;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telemetry_has_no_arbitrary_sensitive_fields() {
        let root = std::env::temp_dir().join(format!(
            "pandora-operations-{}-{}",
            std::process::id(),
            now()
        ));
        let recorder = OperationalRecorder::new(&root, "test", "1");
        recorder.record(
            OperationalEvent::CliInvocation,
            OperationalStatus::Succeeded,
        );
        let contents = fs::read_to_string(root.join("operations/telemetry.jsonl")).unwrap();
        assert!(contents.contains("\"event\":\"cli_invocation\""));
        assert!(!contents.contains("token"));
        assert!(!contents.contains("prompt"));
        let _ = fs::remove_dir_all(root);
    }
}
