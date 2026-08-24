use crate::harness_registry::{HarnessRegistry, HarnessRegistryError, PackageRecord};
use pandora_types::{PackageId, PackageKind, PackageLock, PackageManifest, hash_artifact};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

pub const MAX_STORED_ARTIFACT_BYTES: usize = 16 * 1024 * 1024;
const MAX_PACKAGE_LOCK_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub enum PackageStoreError {
    Database(rusqlite::Error),
    Io(std::io::Error),
    Serialization(serde_json::Error),
    CorruptRecord,
    LockPoisoned,
    ArtifactTooLarge,
    InvalidLockfile,
    LockfileTooLarge,
    LockfileMismatch,
    HasDependents {
        id: String,
        version: String,
        dependents: Vec<String>,
    },
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
            Self::InvalidLockfile => formatter.write_str("package lock is invalid"),
            Self::LockfileTooLarge => formatter.write_str("package lock exceeds the local limit"),
            Self::LockfileMismatch => {
                formatter.write_str("package lock does not match the admitted package set")
            }
            Self::HasDependents {
                id,
                version,
                dependents,
            } => write!(
                formatter,
                "cannot remove {id}@{version}; required by {}",
                dependents.join(", ")
            ),
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
            Self::CorruptRecord
            | Self::LockPoisoned
            | Self::ArtifactTooLarge
            | Self::InvalidLockfile
            | Self::LockfileTooLarge
            | Self::LockfileMismatch
            | Self::HasDependents { .. } => None,
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

    pub fn load_artifact(
        &self,
        id: &PackageId,
        version: &str,
    ) -> Result<Option<Vec<u8>>, PackageStoreError> {
        let connection = self.lock()?;
        let registry = load_registry(&connection)?;
        let Some(record) = registry.get(id, version) else {
            return Ok(None);
        };
        let artifact = connection
            .query_row(
                "SELECT artifact FROM package_records WHERE id = ?1 AND version = ?2",
                params![id.as_str(), version],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?
            .ok_or(PackageStoreError::CorruptRecord)?;
        if artifact.len() > MAX_STORED_ARTIFACT_BYTES
            || hash_artifact(&artifact) != record.manifest().content_hash()
        {
            return Err(PackageStoreError::CorruptRecord);
        }
        Ok(Some(artifact))
    }

    pub fn lockfile(&self) -> Result<PackageLock, PackageStoreError> {
        let manifests = self
            .list()?
            .into_iter()
            .map(|record| record.manifest().clone())
            .collect();
        PackageLock::new(manifests).map_err(|_| PackageStoreError::CorruptRecord)
    }

    pub fn write_lockfile(&self, path: impl AsRef<Path>) -> Result<PackageLock, PackageStoreError> {
        let path = path.as_ref();
        if path.is_dir() {
            return Err(PackageStoreError::Io(std::io::Error::new(
                std::io::ErrorKind::IsADirectory,
                "package lock path is a directory",
            )));
        }
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let lock = self.lockfile()?;
        let data = serialize_lockfile(&lock)?;
        let mut file = atomic_write_file::AtomicWriteFile::open(path)?;
        file.write_all(&data)?;
        file.commit()?;
        Ok(lock)
    }

    pub fn verify_lockfile(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<PackageLock, PackageStoreError> {
        let actual = read_lockfile(path.as_ref())?;
        let expected = self.lockfile()?;
        if actual != expected {
            return Err(PackageStoreError::LockfileMismatch);
        }
        Ok(actual)
    }

    pub fn remove(
        &self,
        id: &PackageId,
        version: &str,
    ) -> Result<Option<PackageRecord>, PackageStoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let registry = load_registry(&transaction)?;
        let Some(record) = registry.get(id, version).cloned() else {
            transaction.commit()?;
            return Ok(None);
        };

        let dependents = registry
            .list()
            .into_iter()
            .filter(|candidate| {
                candidate.manifest().id() != id || candidate.manifest().version() != version
            })
            .filter(|candidate| {
                candidate
                    .manifest()
                    .dependencies()
                    .iter()
                    .any(|dependency| {
                        !dependency.optional()
                            && dependency.id() == id
                            && dependency.version() == version
                    })
                    || (record.manifest().kind() == PackageKind::DomainHarness
                        && candidate
                            .manifest()
                            .meta_composition()
                            .is_some_and(|composition| {
                                composition
                                    .allowed_domains()
                                    .iter()
                                    .any(|domain| domain.as_str() == id.as_str())
                            }))
            })
            .map(|candidate| {
                format!(
                    "{}@{}",
                    candidate.manifest().id().as_str(),
                    candidate.manifest().version()
                )
            })
            .collect::<Vec<_>>();
        if !dependents.is_empty() {
            return Err(PackageStoreError::HasDependents {
                id: id.as_str().to_owned(),
                version: version.to_owned(),
                dependents,
            });
        }

        let deleted = transaction.execute(
            "DELETE FROM package_records WHERE id = ?1 AND version = ?2",
            params![id.as_str(), version],
        )?;
        if deleted != 1 {
            return Err(PackageStoreError::CorruptRecord);
        }
        transaction.commit()?;
        Ok(Some(record))
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, PackageStoreError> {
        self.connection
            .lock()
            .map_err(|_| PackageStoreError::LockPoisoned)
    }
}

fn read_lockfile(path: &Path) -> Result<PackageLock, PackageStoreError> {
    let file = fs::File::open(path)?;
    let mut data = Vec::new();
    file.take(MAX_PACKAGE_LOCK_BYTES as u64 + 1)
        .read_to_end(&mut data)?;
    if data.len() > MAX_PACKAGE_LOCK_BYTES {
        return Err(PackageStoreError::LockfileTooLarge);
    }
    let lock: PackageLock =
        serde_json::from_slice(&data).map_err(|_| PackageStoreError::InvalidLockfile)?;
    lock.validate()
        .map_err(|_| PackageStoreError::InvalidLockfile)?;
    if serialize_lockfile(&lock)? != data {
        return Err(PackageStoreError::InvalidLockfile);
    }
    Ok(lock)
}

fn serialize_lockfile(lock: &PackageLock) -> Result<Vec<u8>, PackageStoreError> {
    let mut data = serde_json::to_vec_pretty(lock)?;
    data.push(b'\n');
    if data.len() > MAX_PACKAGE_LOCK_BYTES {
        return Err(PackageStoreError::LockfileTooLarge);
    }
    Ok(data)
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
                Err(
                    HarnessRegistryError::MissingDependency { .. }
                    | HarnessRegistryError::MetaDomainMissing { .. },
                ) => {
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
    use pandora_types::{
        MetaComposition, PackageCompatibility, PackageDependency, PackageKind, PackageManifest,
        TrustEvidence, TrustLevel, hash_artifact,
    };

    const CURRENT_RUNTIME_REQUIREMENT: &str = concat!("pandora>=", env!("CARGO_PKG_VERSION"));

    fn meta_manifest(artifact: &[u8]) -> PackageManifest {
        custom_meta_manifest(artifact, &["coding-domain"])
    }

    fn custom_meta_manifest(artifact: &[u8], domains: &[&str]) -> PackageManifest {
        PackageManifest::new_meta(
            "example/meta",
            "1.0.0",
            "publisher",
            hash_artifact(artifact),
            Vec::new(),
            PackageCompatibility::new(CURRENT_RUNTIME_REQUIREMENT).unwrap(),
            "Apache-2.0",
            TrustEvidence::unsigned(),
            MetaComposition::new(
                domains
                    .iter()
                    .map(|domain| pandora_types::HarnessId::new(*domain).unwrap())
                    .collect(),
                4,
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn gene_manifest(id: &str, artifact: &[u8]) -> PackageManifest {
        PackageManifest::new(
            id,
            "1.0.0",
            PackageKind::Gene,
            "publisher",
            hash_artifact(artifact),
            Vec::new(),
            PackageCompatibility::new(CURRENT_RUNTIME_REQUIREMENT).unwrap(),
            "Apache-2.0",
            TrustEvidence::unsigned(),
        )
        .unwrap()
    }

    fn domain_manifest(
        id: &str,
        dependencies: Vec<PackageDependency>,
        artifact: &[u8],
    ) -> PackageManifest {
        PackageManifest::new(
            id,
            "1.0.0",
            PackageKind::DomainHarness,
            "publisher",
            hash_artifact(artifact),
            dependencies,
            PackageCompatibility::new(CURRENT_RUNTIME_REQUIREMENT).unwrap(),
            "Apache-2.0",
            TrustEvidence::unsigned(),
        )
        .unwrap()
    }

    fn store() -> (PackageStore, std::path::PathBuf) {
        let root = crate::test_support::new_temp_dir("pandora-package-store").unwrap();
        let path = root.join("packages.sqlite3");
        let store = PackageStore::open(&path).unwrap();
        (store, root)
    }

    fn insert_record(
        store: &PackageStore,
        manifest: &PackageManifest,
        artifact: &[u8],
        state: PackageState,
    ) {
        let connection = store.lock().unwrap();
        connection
            .execute(
                "INSERT INTO package_records (id, version, manifest_json, artifact, state)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    manifest.id().as_str(),
                    manifest.version(),
                    serde_json::to_string(manifest).unwrap(),
                    artifact,
                    state.as_str(),
                ],
            )
            .unwrap();
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
    fn domain_profile_admission_survives_reopen() {
        let gene_artifact = b"gene";
        let gene = PackageManifest::new(
            "example/gene",
            "1.0.0",
            PackageKind::Gene,
            "publisher",
            hash_artifact(gene_artifact),
            Vec::new(),
            PackageCompatibility::new(CURRENT_RUNTIME_REQUIREMENT).unwrap(),
            "Apache-2.0",
            TrustEvidence::unsigned(),
        )
        .unwrap();
        let profile_artifact = b"domain profile";
        let profile = PackageManifest::new(
            "example/domain",
            "1.0.0",
            PackageKind::DomainHarness,
            "publisher",
            hash_artifact(profile_artifact),
            vec![PackageDependency::new("example/gene", "1.0.0", false).unwrap()],
            PackageCompatibility::new(CURRENT_RUNTIME_REQUIREMENT).unwrap(),
            "Apache-2.0",
            TrustEvidence::unsigned(),
        )
        .unwrap();
        let (store, root) = store();
        store.admit(&gene, &gene, gene_artifact).unwrap();
        let record = store.admit(&profile, &profile, profile_artifact).unwrap();
        assert_eq!(record.state(), PackageState::Admitted);
        drop(store);

        let reopened = PackageStore::open(root.join("packages.sqlite3")).unwrap();
        let records = reopened.list().unwrap();
        let profile_record = records
            .iter()
            .find(|record| record.manifest().kind() == PackageKind::DomainHarness)
            .expect("domain profile should survive reopen");
        assert_eq!(profile_record.state(), PackageState::Admitted);
        assert!(!profile_record.grants_runtime_authority());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn stored_artifacts_are_revalidated_when_loaded() {
        let artifact = b"stored gene";
        let manifest = gene_manifest("example/gene", artifact);
        let (store, root) = store();
        store.admit(&manifest, &manifest, artifact).unwrap();

        assert_eq!(
            store
                .load_artifact(manifest.id(), manifest.version())
                .unwrap(),
            Some(artifact.to_vec())
        );

        store
            .lock()
            .unwrap()
            .execute(
                "UPDATE package_records SET artifact = ?1 WHERE id = ?2 AND version = ?3",
                params![b"tampered", manifest.id().as_str(), manifest.version()],
            )
            .unwrap();
        assert!(matches!(
            store.load_artifact(manifest.id(), manifest.version()),
            Err(PackageStoreError::CorruptRecord)
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn custom_meta_profile_with_custom_domain_survives_reopen() {
        let gene_artifact = b"gene";
        let gene = gene_manifest("example/gene", gene_artifact);
        let domain_artifact = b"domain";
        let domain = domain_manifest(
            "example/domain",
            vec![PackageDependency::new("example/gene", "1.0.0", false).unwrap()],
            domain_artifact,
        );
        let meta_artifact = b"meta";
        let meta = custom_meta_manifest(meta_artifact, &["example/domain"]);
        let (store, root) = store();
        store.admit(&gene, &gene, gene_artifact).unwrap();
        store.admit(&domain, &domain, domain_artifact).unwrap();
        store.admit(&meta, &meta, meta_artifact).unwrap();
        drop(store);

        let reopened = PackageStore::open(root.join("packages.sqlite3")).unwrap();
        let records = reopened.list().unwrap();
        assert_eq!(records.len(), 3);
        assert!(records.iter().any(|record| {
            record.manifest().kind() == PackageKind::MetaHarness
                && record.manifest().id().as_str() == "example/meta"
        }));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn replay_defers_meta_composition_until_its_custom_domain_is_loaded() {
        let gene_artifact = b"gene";
        let gene = gene_manifest("example/gene", gene_artifact);
        let domain_artifact = b"domain";
        let domain = domain_manifest(
            "example/domain",
            vec![PackageDependency::new("example/gene", "1.0.0", false).unwrap()],
            domain_artifact,
        );
        let meta_artifact = b"meta";
        let meta = custom_meta_manifest(meta_artifact, &["example/domain"]);
        let (store, root) = store();

        insert_record(&store, &meta, meta_artifact, PackageState::Admitted);
        insert_record(&store, &domain, domain_artifact, PackageState::Admitted);
        insert_record(&store, &gene, gene_artifact, PackageState::Installed);
        drop(store);

        let reopened = PackageStore::open(root.join("packages.sqlite3")).unwrap();
        assert_eq!(reopened.list().unwrap().len(), 3);
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
    fn incompatible_runtime_does_not_persist() {
        let artifact = b"gene";
        let manifest = PackageManifest::new(
            "example/gene",
            "1.0.0",
            PackageKind::Gene,
            "publisher",
            hash_artifact(artifact),
            Vec::new(),
            PackageCompatibility::new("pandora>=3.0.0").unwrap(),
            "Apache-2.0",
            TrustEvidence::unsigned(),
        )
        .unwrap();
        let (store, root) = store();

        assert!(matches!(
            store.admit(&manifest, &manifest, artifact),
            Err(PackageStoreError::Admission(
                HarnessRegistryError::IncompatibleRuntime { .. }
            ))
        ));
        assert!(store.list().unwrap().is_empty());
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn official_trust_does_not_persist_without_a_publisher_root() {
        let artifact = b"gene";
        let manifest = PackageManifest::new(
            "example/gene",
            "1.0.0",
            PackageKind::Gene,
            "publisher",
            hash_artifact(artifact),
            Vec::new(),
            PackageCompatibility::new(CURRENT_RUNTIME_REQUIREMENT).unwrap(),
            "Apache-2.0",
            TrustEvidence::new(
                TrustLevel::Official,
                Some("signature".to_owned()),
                Some("public-key".to_owned()),
            )
            .unwrap(),
        )
        .unwrap();
        let (store, root) = store();

        assert!(matches!(
            store.admit(&manifest, &manifest, artifact),
            Err(PackageStoreError::Admission(
                HarnessRegistryError::OfficialTrustUnsupported
            ))
        ));
        assert!(store.list().unwrap().is_empty());
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn corrupted_deserialized_manifest_is_rejected_on_read() {
        let artifact = b"gene";
        let manifest = gene_manifest("example/gene", artifact);
        let mut encoded = serde_json::to_value(&manifest).unwrap();
        encoded["version"] = serde_json::Value::String("release-1".to_owned());
        let (store, root) = store();

        let connection = store.lock().unwrap();
        connection
            .execute(
                "INSERT INTO package_records (id, version, manifest_json, artifact, state)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    "example/gene",
                    "release-1",
                    serde_json::to_string(&encoded).unwrap(),
                    artifact,
                    PackageState::Installed.as_str(),
                ],
            )
            .unwrap();
        drop(connection);

        assert!(matches!(
            store.list(),
            Err(PackageStoreError::CorruptRecord)
        ));
        drop(store);
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

    #[test]
    fn removing_a_package_is_persistent() {
        let artifact = b"meta";
        let manifest = meta_manifest(artifact);
        let (store, root) = store();
        store.admit(&manifest, &manifest, artifact).unwrap();

        let removed = store
            .remove(manifest.id(), manifest.version())
            .unwrap()
            .expect("admitted package should be removed");
        assert_eq!(removed.manifest().id(), manifest.id());
        assert!(store.list().unwrap().is_empty());
        drop(store);

        let reopened = PackageStore::open(root.join("packages.sqlite3")).unwrap();
        assert!(reopened.list().unwrap().is_empty());
        assert!(
            reopened
                .remove(manifest.id(), manifest.version())
                .unwrap()
                .is_none()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn required_dependents_block_removal() {
        let gene_artifact = b"gene";
        let gene = gene_manifest("example/gene", gene_artifact);
        let domain_artifact = b"domain";
        let domain = domain_manifest(
            "example/domain",
            vec![PackageDependency::new("example/gene", "1.0.0", false).unwrap()],
            domain_artifact,
        );
        let (store, root) = store();
        store.admit(&gene, &gene, gene_artifact).unwrap();
        store.admit(&domain, &domain, domain_artifact).unwrap();

        let error = store
            .remove(gene.id(), gene.version())
            .expect_err("required dependency must block removal");
        match error {
            PackageStoreError::HasDependents {
                id,
                version,
                dependents,
            } => {
                assert_eq!(id, "example/gene");
                assert_eq!(version, "1.0.0");
                assert_eq!(dependents, vec!["example/domain@1.0.0"]);
            }
            other => panic!("unexpected removal error: {other:?}"),
        }
        assert!(store.get(gene.id(), gene.version()).unwrap().is_some());
        assert!(store.get(domain.id(), domain.version()).unwrap().is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn meta_composition_blocks_domain_removal() {
        let gene_artifact = b"gene";
        let gene = gene_manifest("example/gene", gene_artifact);
        let domain_artifact = b"domain";
        let domain = domain_manifest(
            "example/domain",
            vec![PackageDependency::new("example/gene", "1.0.0", false).unwrap()],
            domain_artifact,
        );
        let meta_artifact = b"meta";
        let meta = custom_meta_manifest(meta_artifact, &["example/domain"]);
        let (store, root) = store();
        store.admit(&gene, &gene, gene_artifact).unwrap();
        store.admit(&domain, &domain, domain_artifact).unwrap();
        store.admit(&meta, &meta, meta_artifact).unwrap();

        let error = store
            .remove(domain.id(), domain.version())
            .expect_err("Meta composition must block Domain removal");
        match error {
            PackageStoreError::HasDependents {
                id,
                version,
                dependents,
            } => {
                assert_eq!(id, "example/domain");
                assert_eq!(version, "1.0.0");
                assert_eq!(dependents, vec!["example/meta@1.0.0"]);
            }
            other => panic!("unexpected removal error: {other:?}"),
        }
        assert!(store.get(domain.id(), domain.version()).unwrap().is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn optional_dependents_do_not_block_removal() {
        let gene_artifact = b"gene";
        let gene = gene_manifest("example/gene", gene_artifact);
        let domain_artifact = b"domain";
        let domain = domain_manifest(
            "example/domain",
            vec![
                PackageDependency::new("workspace.read", "0.1.0", false).unwrap(),
                PackageDependency::new("example/gene", "1.0.0", true).unwrap(),
            ],
            domain_artifact,
        );
        let (store, root) = store();
        store.admit(&gene, &gene, gene_artifact).unwrap();
        store.admit(&domain, &domain, domain_artifact).unwrap();

        assert!(store.remove(gene.id(), gene.version()).unwrap().is_some());
        assert!(store.get(gene.id(), gene.version()).unwrap().is_none());
        assert!(store.get(domain.id(), domain.version()).unwrap().is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn package_lock_round_trips_and_detects_store_drift() {
        let artifact = b"gene";
        let manifest = gene_manifest("example/gene", artifact);
        let (store, root) = store();
        store.admit(&manifest, &manifest, artifact).unwrap();
        let path = root.join("pandora.lock");

        let written = store.write_lockfile(&path).unwrap();
        assert_eq!(written.packages().len(), 1);
        assert_eq!(
            written.packages()[0].content_hash(),
            hash_artifact(artifact)
        );
        assert_eq!(store.verify_lockfile(&path).unwrap(), written);

        store.remove(manifest.id(), manifest.version()).unwrap();
        assert!(matches!(
            store.verify_lockfile(&path),
            Err(PackageStoreError::LockfileMismatch)
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn invalid_existing_lockfile_fails_closed() {
        let (store, root) = store();
        let path = root.join("pandora.lock");
        std::fs::write(&path, b"{not-json}\n").unwrap();

        assert!(matches!(
            store.verify_lockfile(&path),
            Err(PackageStoreError::InvalidLockfile)
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn noncanonical_existing_lockfile_fails_closed() {
        let artifact = b"gene";
        let manifest = gene_manifest("example/gene", artifact);
        let (store, root) = store();
        store.admit(&manifest, &manifest, artifact).unwrap();
        let path = root.join("pandora.lock");
        let lock = store.lockfile().unwrap();
        let mut encoded = serde_json::to_value(lock).unwrap();
        encoded["unknown"] = serde_json::json!(true);
        std::fs::write(&path, serde_json::to_vec_pretty(&encoded).unwrap()).unwrap();

        assert!(matches!(
            store.verify_lockfile(&path),
            Err(PackageStoreError::InvalidLockfile)
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn writer_refuses_a_lock_larger_than_the_reader_limit() {
        let manifests = (0..300)
            .map(|index| {
                PackageManifest::new(
                    format!("example/gene-{index}"),
                    "1.0.0",
                    PackageKind::Gene,
                    "publisher",
                    hash_artifact(format!("gene-{index}").as_bytes()),
                    Vec::new(),
                    PackageCompatibility::new(CURRENT_RUNTIME_REQUIREMENT).unwrap(),
                    "x".repeat(4096),
                    TrustEvidence::unsigned(),
                )
                .unwrap()
            })
            .collect();
        let lock = PackageLock::new(manifests).unwrap();

        assert!(matches!(
            serialize_lockfile(&lock),
            Err(PackageStoreError::LockfileTooLarge)
        ));
    }
}
