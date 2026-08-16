use crate::harness_registry::{HarnessRegistry, HarnessRegistryError, PackageRecord};
use pandora_types::{PackageId, PackageManifest};
use rusqlite::{Connection, TransactionBehavior, params};
use std::fmt;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

pub const MAX_STORED_ARTIFACT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug)]
pub enum PackageStoreError {
    Database(rusqlite::Error),
    Io(std::io::Error),
    Serialization(serde_json::Error),
    CorruptRecord,
    LockPoisoned,
    ArtifactTooLarge,
    Admission(HarnessRegistryError),
}

impl fmt::Display for PackageStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(_) => formatter.write_str("package store operation failed"),
            Self::Io(_) => formatter.write_str("package store directory operation failed"),
            Self::Serialization(_) | Self::CorruptRecord => {
                formatter.write_str("package store contains an invalid record")
            }
            Self::LockPoisoned => formatter.write_str("package store lock is unavailable"),
            Self::ArtifactTooLarge => {
                formatter.write_str("package artifact exceeds the local limit")
            }
            Self::Admission(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for PackageStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Serialization(error) => Some(error),
            Self::Admission(error) => Some(error),
            Self::CorruptRecord | Self::LockPoisoned | Self::ArtifactTooLarge => None,
        }
    }
}

impl From<rusqlite::Error> for PackageStoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

impl From<std::io::Error> for PackageStoreError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for PackageStoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}

pub struct PackageStore {
    connection: Mutex<Connection>,
}

impl PackageStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PackageStoreError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS package_records (
                 id TEXT NOT NULL,
                 version TEXT NOT NULL,
                 manifest_json TEXT NOT NULL,
                 artifact BLOB NOT NULL,
                 state TEXT NOT NULL,
                 PRIMARY KEY (id, version)
             );",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn admit(
        &self,
        declared: &PackageManifest,
        embedded: &PackageManifest,
        artifact: &[u8],
    ) -> Result<PackageRecord, PackageStoreError> {
        if artifact.len() > MAX_STORED_ARTIFACT_BYTES {
            return Err(PackageStoreError::ArtifactTooLarge);
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut registry = load_registry(&transaction)?;
        let record = registry
            .install(declared, embedded, artifact)
            .map_err(PackageStoreError::Admission)?;
        let manifest_json = serde_json::to_string(record.manifest())?;
        transaction.execute(
            "INSERT INTO package_records (id, version, manifest_json, artifact, state)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                record.manifest().id().as_str(),
                record.manifest().version(),
                manifest_json,
                artifact,
                record.state().as_str(),
            ],
        )?;
        transaction.commit()?;
        Ok(record)
    }

    pub fn list(&self) -> Result<Vec<PackageRecord>, PackageStoreError> {
        let connection = self.lock()?;
        Ok(load_registry(&connection)?.list())
    }

    pub fn get(
        &self,
        id: &PackageId,
        version: &str,
    ) -> Result<Option<PackageRecord>, PackageStoreError> {
        let connection = self.lock()?;
        Ok(load_registry(&connection)?.get(id, version).cloned())
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, PackageStoreError> {
        self.connection
            .lock()
            .map_err(|_| PackageStoreError::LockPoisoned)
    }
}

fn load_registry(connection: &rusqlite::Connection) -> Result<HarnessRegistry, PackageStoreError> {
    let mut registry = HarnessRegistry::new();
    let mut statement =
        connection.prepare("SELECT manifest_json, artifact, state FROM package_records")?;
    let rows = statement.query_map([], |row| {
        let manifest_json = row.get::<_, String>(0)?;
        let artifact = row.get::<_, Vec<u8>>(1)?;
        let state = row.get::<_, String>(2)?;
        Ok((manifest_json, artifact, state))
    })?;
    let mut pending = Vec::new();
    for row in rows {
        let (manifest_json, artifact, state) = row?;
        pending.push((manifest_json, artifact, state));
    }
    while !pending.is_empty() {
        let mut next = Vec::new();
        let mut progress = false;
        for (manifest_json, artifact, state) in pending {
            if artifact.len() > MAX_STORED_ARTIFACT_BYTES {
                return Err(PackageStoreError::CorruptRecord);
            }
            let manifest: PackageManifest = serde_json::from_str(&manifest_json)?;
            let record = match registry.install(&manifest, &manifest, &artifact) {
                Ok(record) => record,
                Err(HarnessRegistryError::MissingDependency { .. }) => {
                    next.push((manifest_json, artifact, state));
                    continue;
                }
                Err(_) => return Err(PackageStoreError::CorruptRecord),
            };
            if record.state().as_str() != state {
                return Err(PackageStoreError::CorruptRecord);
            }
            progress = true;
        }
        if !progress {
            return Err(PackageStoreError::CorruptRecord);
        }
        pending = next;
    }
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness_registry::PackageState;
    use pandora_types::{MetaComposition, PackageCompatibility, TrustEvidence, hash_artifact};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(1);

    fn meta_manifest(artifact: &[u8]) -> PackageManifest {
        PackageManifest::new_meta(
            "example/meta",
            "1.0.0",
            "publisher",
            hash_artifact(artifact),
            Vec::new(),
            PackageCompatibility::new("pandora>=2.0.0").unwrap(),
            "Apache-2.0",
            TrustEvidence::unsigned(),
            MetaComposition::new(
                vec![pandora_types::HarnessId::new("coding-domain").unwrap()],
                4,
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn store() -> (PackageStore, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "pandora-package-store-{}-{}",
            std::process::id(),
            NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let path = root.join("packages.sqlite3");
        let store = PackageStore::open(&path).unwrap();
        (store, root)
    }

    #[test]
    fn meta_admission_survives_reopen() {
        let artifact = br#"{"kind":"meta_harness"}"#;
        let manifest = meta_manifest(artifact);
        let (store, root) = store();
        let record = store.admit(&manifest, &manifest, artifact).unwrap();
        assert_eq!(record.state(), PackageState::Admitted);
        drop(store);

        let reopened = PackageStore::open(root.join("packages.sqlite3")).unwrap();
        let records = reopened.list().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].manifest().id().as_str(), "example/meta");
        assert!(!records[0].grants_runtime_authority());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn duplicate_admission_is_atomic() {
        let artifact = b"meta";
        let manifest = meta_manifest(artifact);
        let (store, root) = store();
        store.admit(&manifest, &manifest, artifact).unwrap();
        assert!(matches!(
            store.admit(&manifest, &manifest, artifact),
            Err(PackageStoreError::Admission(
                HarnessRegistryError::DuplicateIdentity
            ))
        ));
        assert_eq!(store.list().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn oversized_artifact_is_rejected_before_opening_a_transaction() {
        let artifact = vec![0_u8; MAX_STORED_ARTIFACT_BYTES + 1];
        let manifest = meta_manifest(&artifact);
        let (store, root) = store();
        assert!(matches!(
            store.admit(&manifest, &manifest, &artifact),
            Err(PackageStoreError::ArtifactTooLarge)
        ));
        assert!(store.list().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }
}
