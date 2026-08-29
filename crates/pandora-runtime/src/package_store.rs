use crate::harness_registry::{
    HarnessRegistry, HarnessRegistryError, PackageRecord, PublisherTrustRoot, PublisherTrustRoots,
};
use pandora_harnesses::{builtin_genes, builtin_harnesses};
use pandora_types::{
    ArtifactId, PackageId, PackageKind, PackageLock, PackageManifest, hash_artifact,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

pub const MAX_STORED_ARTIFACT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_PUBLISHER_TRUST_ROOTS: usize = 64;
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
    PackageNotFound {
        id: String,
        version: String,
    },
    MissingEnabledDependency {
        id: String,
        version: String,
    },
    MissingEnabledDomain {
        id: String,
    },
    HasEnabledDependents {
        id: String,
        version: String,
        dependents: Vec<String>,
    },
    PackageNotEnabled {
        id: String,
        version: String,
    },
    NoRollbackBinding {
        id: String,
    },
    PackageBound {
        id: String,
        version: String,
        role: &'static str,
    },
    TrustRootAlreadyExists,
    TrustRootNotFound,
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
            Self::PackageNotFound { id, version } => {
                write!(formatter, "package {id}@{version} is not installed")
            }
            Self::MissingEnabledDependency { id, version } => write!(
                formatter,
                "required package dependency {id}@{version} is not enabled"
            ),
            Self::MissingEnabledDomain { id } => {
                write!(formatter, "required Domain Harness {id} is not enabled")
            }
            Self::HasEnabledDependents {
                id,
                version,
                dependents,
            } => write!(
                formatter,
                "package {id}@{version} is required by enabled package(s): {}",
                dependents.join(", ")
            ),
            Self::PackageNotEnabled { id, version } => {
                write!(formatter, "package {id}@{version} is not enabled")
            }
            Self::NoRollbackBinding { id } => {
                write!(formatter, "package {id} has no previous enabled version")
            }
            Self::PackageBound { id, version, role } => write!(
                formatter,
                "package {id}@{version} is retained as the {role} lifecycle binding"
            ),
            Self::TrustRootAlreadyExists => {
                formatter.write_str("publisher trust root is already configured")
            }
            Self::TrustRootNotFound => formatter.write_str("publisher trust root was not found"),
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
            | Self::HasDependents { .. }
            | Self::PackageNotFound { .. }
            | Self::MissingEnabledDependency { .. }
            | Self::MissingEnabledDomain { .. }
            | Self::HasEnabledDependents { .. }
            | Self::PackageNotEnabled { .. }
            | Self::NoRollbackBinding { .. }
            | Self::PackageBound { .. }
            | Self::TrustRootAlreadyExists
            | Self::TrustRootNotFound => None,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackageBinding {
    id: String,
    active_version: Option<String>,
    previous_version: Option<String>,
    generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublisherTrustRootRecord {
    root: PublisherTrustRoot,
    added_at: u64,
    revoked_at: Option<u64>,
}

impl PublisherTrustRootRecord {
    pub fn publisher(&self) -> &str {
        self.root.publisher()
    }

    pub fn key_id(&self) -> &str {
        self.root.key_id()
    }

    pub fn public_key(&self) -> &str {
        self.root.public_key()
    }

    pub const fn added_at(&self) -> u64 {
        self.added_at
    }

    pub const fn revoked_at(&self) -> Option<u64> {
        self.revoked_at
    }

    pub const fn active(&self) -> bool {
        self.revoked_at.is_none()
    }
}

impl PackageBinding {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn active_version(&self) -> Option<&str> {
        self.active_version.as_deref()
    }

    pub fn previous_version(&self) -> Option<&str> {
        self.previous_version.as_deref()
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub fn enables(&self, version: &str) -> bool {
        self.active_version.as_deref() == Some(version)
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
             );
             CREATE TABLE IF NOT EXISTS package_bindings (
                 id TEXT PRIMARY KEY,
                 active_version TEXT,
                 previous_version TEXT,
                 generation INTEGER NOT NULL,
                 CHECK (active_version IS NOT NULL OR previous_version IS NOT NULL)
             );
             CREATE TABLE IF NOT EXISTS publisher_trust_roots (
                 publisher TEXT NOT NULL,
                 key_id TEXT NOT NULL,
                 public_key TEXT NOT NULL,
                 added_at INTEGER NOT NULL CHECK (added_at >= 0),
                 revoked_at INTEGER CHECK (revoked_at IS NULL OR revoked_at >= 0),
                 PRIMARY KEY (publisher, key_id)
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
        let trust_roots = load_publisher_trust_roots(&transaction)?;
        let record = registry
            .install_with_trust_roots(declared, embedded, artifact, &trust_roots)
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

    pub fn add_publisher_trust_root(
        &self,
        publisher: impl Into<String>,
        key_id: impl Into<String>,
        public_key: impl Into<String>,
        added_at: u64,
    ) -> Result<PublisherTrustRootRecord, PackageStoreError> {
        let root = PublisherTrustRoot::new(publisher, key_id, public_key)
            .map_err(PackageStoreError::Admission)?;
        let added_at_i64 = i64::try_from(added_at).map_err(|_| PackageStoreError::CorruptRecord)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let count =
            transaction.query_row("SELECT COUNT(*) FROM publisher_trust_roots", [], |row| {
                row.get::<_, i64>(0)
            })?;
        if usize::try_from(count).map_err(|_| PackageStoreError::CorruptRecord)?
            >= MAX_PUBLISHER_TRUST_ROOTS
        {
            return Err(PackageStoreError::CorruptRecord);
        }
        let inserted = transaction.execute(
            "INSERT INTO publisher_trust_roots (publisher, key_id, public_key, added_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                root.publisher(),
                root.key_id(),
                root.public_key(),
                added_at_i64
            ],
        );
        match inserted {
            Ok(_) => {
                transaction.commit()?;
                Ok(PublisherTrustRootRecord {
                    root,
                    added_at,
                    revoked_at: None,
                })
            }
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(PackageStoreError::TrustRootAlreadyExists)
            }
            Err(error) => Err(PackageStoreError::Database(error)),
        }
    }

    pub fn list_publisher_trust_roots(
        &self,
    ) -> Result<Vec<PublisherTrustRootRecord>, PackageStoreError> {
        let connection = self.lock()?;
        load_publisher_trust_root_records(&connection)
    }

    pub fn revoke_publisher_trust_root(
        &self,
        publisher: &str,
        key_id: &str,
        revoked_at: u64,
    ) -> Result<PublisherTrustRootRecord, PackageStoreError> {
        let revoked_at_i64 =
            i64::try_from(revoked_at).map_err(|_| PackageStoreError::CorruptRecord)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = transaction
            .query_row(
                "SELECT publisher, key_id, public_key, added_at, revoked_at
                 FROM publisher_trust_roots WHERE publisher = ?1 AND key_id = ?2",
                params![publisher, key_id],
                decode_trust_root,
            )
            .optional()?;
        let Some(mut record) = row else {
            return Err(PackageStoreError::TrustRootNotFound);
        };
        if record.revoked_at.is_some() {
            transaction.commit()?;
            return Ok(record);
        }
        transaction.execute(
            "UPDATE publisher_trust_roots SET revoked_at = ?1
             WHERE publisher = ?2 AND key_id = ?3 AND revoked_at IS NULL",
            params![revoked_at_i64, publisher, key_id],
        )?;
        record.revoked_at = Some(revoked_at);
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

    pub fn binding(&self, id: &PackageId) -> Result<Option<PackageBinding>, PackageStoreError> {
        let connection = self.lock()?;
        load_binding(&connection, id.as_str())
    }

    pub fn bindings(&self) -> Result<Vec<PackageBinding>, PackageStoreError> {
        let connection = self.lock()?;
        Ok(load_bindings(&connection)?.into_values().collect())
    }

    pub fn is_enabled(&self, id: &PackageId, version: &str) -> Result<bool, PackageStoreError> {
        Ok(self
            .binding(id)?
            .is_some_and(|binding| binding.enables(version)))
    }

    pub fn enabled_dependents(
        &self,
        id: &PackageId,
        version: &str,
    ) -> Result<Vec<String>, PackageStoreError> {
        let connection = self.lock()?;
        let registry = load_registry(&connection)?;
        let bindings = active_bindings(&connection)?;
        Ok(enabled_dependents(
            &registry,
            &bindings,
            id.as_str(),
            version,
        ))
    }

    pub fn enable(
        &self,
        id: &PackageId,
        version: &str,
    ) -> Result<PackageBinding, PackageStoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let registry = load_registry(&transaction)?;
        let record = registry.get(id, version).cloned().ok_or_else(|| {
            PackageStoreError::PackageNotFound {
                id: id.as_str().to_owned(),
                version: version.to_owned(),
            }
        })?;
        let existing = load_binding(&transaction, id.as_str())?;
        if existing
            .as_ref()
            .is_some_and(|binding| binding.enables(version))
        {
            transaction.commit()?;
            return Ok(existing.expect("enabled binding exists"));
        }
        let active = active_bindings(&transaction)?;
        if let Some(current) = existing
            .as_ref()
            .and_then(|binding| binding.active_version())
        {
            reject_enabled_dependents(&registry, &active, id.as_str(), current)?;
        }
        ensure_enableable(&registry, &active, &record)?;
        let previous_version = existing.as_ref().and_then(|binding| {
            binding
                .active_version()
                .or_else(|| binding.previous_version())
                .filter(|previous| *previous != version)
                .map(str::to_owned)
        });
        let binding = PackageBinding {
            id: id.as_str().to_owned(),
            active_version: Some(version.to_owned()),
            previous_version,
            generation: existing
                .as_ref()
                .map_or(1, |binding| binding.generation().saturating_add(1)),
        };
        persist_binding(&transaction, &binding)?;
        transaction.commit()?;
        Ok(binding)
    }

    pub fn disable(
        &self,
        id: &PackageId,
        version: &str,
    ) -> Result<PackageBinding, PackageStoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let registry = load_registry(&transaction)?;
        if registry.get(id, version).is_none() {
            return Err(PackageStoreError::PackageNotFound {
                id: id.as_str().to_owned(),
                version: version.to_owned(),
            });
        }
        let existing = load_binding(&transaction, id.as_str())?.ok_or_else(|| {
            PackageStoreError::PackageNotEnabled {
                id: id.as_str().to_owned(),
                version: version.to_owned(),
            }
        })?;
        if !existing.enables(version) {
            return Err(PackageStoreError::PackageNotEnabled {
                id: id.as_str().to_owned(),
                version: version.to_owned(),
            });
        }
        let active = active_bindings(&transaction)?;
        reject_enabled_dependents(&registry, &active, id.as_str(), version)?;
        let binding = PackageBinding {
            id: id.as_str().to_owned(),
            active_version: None,
            previous_version: Some(version.to_owned()),
            generation: existing.generation().saturating_add(1),
        };
        persist_binding(&transaction, &binding)?;
        transaction.commit()?;
        Ok(binding)
    }

    pub fn rollback(&self, id: &PackageId) -> Result<PackageBinding, PackageStoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let registry = load_registry(&transaction)?;
        let existing = load_binding(&transaction, id.as_str())?.ok_or_else(|| {
            PackageStoreError::NoRollbackBinding {
                id: id.as_str().to_owned(),
            }
        })?;
        let target =
            existing
                .previous_version()
                .ok_or_else(|| PackageStoreError::NoRollbackBinding {
                    id: id.as_str().to_owned(),
                })?;
        let target_record = registry.get(id, target).cloned().ok_or_else(|| {
            PackageStoreError::PackageNotFound {
                id: id.as_str().to_owned(),
                version: target.to_owned(),
            }
        })?;
        let active = active_bindings(&transaction)?;
        if let Some(current) = existing.active_version() {
            reject_enabled_dependents(&registry, &active, id.as_str(), current)?;
        }
        let mut target_bindings = active;
        target_bindings.insert(id.as_str().to_owned(), target.to_owned());
        ensure_enableable(&registry, &target_bindings, &target_record)?;
        let binding = PackageBinding {
            id: id.as_str().to_owned(),
            active_version: Some(target.to_owned()),
            previous_version: existing.active_version().map(str::to_owned),
            generation: existing.generation().saturating_add(1),
        };
        persist_binding(&transaction, &binding)?;
        transaction.commit()?;
        Ok(binding)
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

    pub fn contains_artifact(&self, artifact_id: &ArtifactId) -> Result<bool, PackageStoreError> {
        Ok(self
            .list()?
            .into_iter()
            .any(|record| record.manifest().content_hash() == artifact_id.as_str()))
    }

    pub fn load_artifact_by_id(
        &self,
        artifact_id: &ArtifactId,
    ) -> Result<Option<(PackageRecord, Vec<u8>)>, PackageStoreError> {
        let connection = self.lock()?;
        let registry = load_registry(&connection)?;
        let Some(record) = registry
            .list()
            .into_iter()
            .find(|record| record.manifest().content_hash() == artifact_id.as_str())
        else {
            return Ok(None);
        };
        let artifact = connection
            .query_row(
                "SELECT artifact FROM package_records WHERE id = ?1 AND version = ?2",
                params![record.manifest().id().as_str(), record.manifest().version()],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?
            .ok_or(PackageStoreError::CorruptRecord)?;
        if artifact.len() > MAX_STORED_ARTIFACT_BYTES
            || hash_artifact(&artifact) != artifact_id.as_str()
        {
            return Err(PackageStoreError::CorruptRecord);
        }
        Ok(Some((record, artifact)))
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

        if let Some(binding) = load_binding(&transaction, id.as_str())? {
            if binding.active_version() == Some(version) {
                return Err(PackageStoreError::PackageBound {
                    id: id.as_str().to_owned(),
                    version: version.to_owned(),
                    role: "active",
                });
            }
            if binding.previous_version() == Some(version) {
                if binding.active_version().is_some() {
                    return Err(PackageStoreError::PackageBound {
                        id: id.as_str().to_owned(),
                        version: version.to_owned(),
                        role: "rollback",
                    });
                }
                transaction.execute("DELETE FROM package_bindings WHERE id = ?1", [id.as_str()])?;
            }
        }

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

fn load_binding(
    connection: &Connection,
    id: &str,
) -> Result<Option<PackageBinding>, PackageStoreError> {
    connection
        .query_row(
            "SELECT active_version, previous_version, generation
             FROM package_bindings WHERE id = ?1",
            [id],
            |row| {
                let generation = row.get::<_, i64>(2)?;
                if generation < 0 {
                    return Err(rusqlite::Error::IntegralValueOutOfRange(2, generation));
                }
                Ok(PackageBinding {
                    id: id.to_owned(),
                    active_version: row.get(0)?,
                    previous_version: row.get(1)?,
                    generation: generation as u64,
                })
            },
        )
        .optional()
        .map_err(PackageStoreError::from)
}

fn load_bindings(
    connection: &Connection,
) -> Result<BTreeMap<String, PackageBinding>, PackageStoreError> {
    let mut statement = connection.prepare(
        "SELECT id, active_version, previous_version, generation
         FROM package_bindings ORDER BY id",
    )?;
    let rows = statement.query_map([], |row| {
        let generation = row.get::<_, i64>(3)?;
        if generation < 0 {
            return Err(rusqlite::Error::IntegralValueOutOfRange(3, generation));
        }
        Ok(PackageBinding {
            id: row.get(0)?,
            active_version: row.get(1)?,
            previous_version: row.get(2)?,
            generation: generation as u64,
        })
    })?;
    let mut bindings = BTreeMap::new();
    for row in rows {
        let binding = row?;
        if binding.active_version.is_none() && binding.previous_version.is_none() {
            return Err(PackageStoreError::CorruptRecord);
        }
        bindings.insert(binding.id.clone(), binding);
    }
    Ok(bindings)
}

fn active_bindings(connection: &Connection) -> Result<BTreeMap<String, String>, PackageStoreError> {
    Ok(load_bindings(connection)?
        .into_iter()
        .filter_map(|(id, binding)| binding.active_version.map(|version| (id, version)))
        .collect())
}

fn persist_binding(
    connection: &Connection,
    binding: &PackageBinding,
) -> Result<(), PackageStoreError> {
    let generation =
        i64::try_from(binding.generation).map_err(|_| PackageStoreError::CorruptRecord)?;
    connection.execute(
        "INSERT INTO package_bindings (id, active_version, previous_version, generation)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET
             active_version = excluded.active_version,
             previous_version = excluded.previous_version,
             generation = excluded.generation",
        params![
            binding.id,
            binding.active_version,
            binding.previous_version,
            generation,
        ],
    )?;
    Ok(())
}

fn ensure_enableable(
    registry: &HarnessRegistry,
    bindings: &BTreeMap<String, String>,
    record: &PackageRecord,
) -> Result<(), PackageStoreError> {
    for dependency in record
        .manifest()
        .dependencies()
        .iter()
        .filter(|dependency| !dependency.optional())
    {
        let built_in = builtin_genes().into_iter().any(|gene| {
            gene.manifest().id().as_str() == dependency.id().as_str()
                && gene.manifest().version() == dependency.version()
        });
        if !built_in
            && bindings.get(dependency.id().as_str()).map(String::as_str)
                != Some(dependency.version())
        {
            return Err(PackageStoreError::MissingEnabledDependency {
                id: dependency.id().as_str().to_owned(),
                version: dependency.version().to_owned(),
            });
        }
    }

    if record.manifest().kind() == PackageKind::MetaHarness {
        for domain in record
            .manifest()
            .meta_composition()
            .expect("admitted Meta Harness has a composition")
            .allowed_domains()
        {
            let built_in = builtin_harnesses().into_iter().any(|harness| {
                harness.manifest().id() == domain
                    && harness.manifest().kind() == pandora_types::HarnessKind::Domain
            });
            if built_in {
                continue;
            }
            let Some(version) = bindings.get(domain.as_str()) else {
                return Err(PackageStoreError::MissingEnabledDomain {
                    id: domain.as_str().to_owned(),
                });
            };
            let package_id = PackageId::new(domain.as_str().to_owned())
                .map_err(|_| PackageStoreError::CorruptRecord)?;
            if !registry
                .get(&package_id, version)
                .is_some_and(|candidate| candidate.manifest().kind() == PackageKind::DomainHarness)
            {
                return Err(PackageStoreError::MissingEnabledDomain {
                    id: domain.as_str().to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn enabled_dependents(
    registry: &HarnessRegistry,
    bindings: &BTreeMap<String, String>,
    id: &str,
    version: &str,
) -> Vec<String> {
    let mut dependents = BTreeSet::new();
    for candidate in registry.list() {
        let manifest = candidate.manifest();
        if bindings.get(manifest.id().as_str()).map(String::as_str) != Some(manifest.version())
            || (manifest.id().as_str() == id && manifest.version() == version)
        {
            continue;
        }
        let dependency_match = manifest.dependencies().iter().any(|dependency| {
            !dependency.optional()
                && dependency.id().as_str() == id
                && dependency.version() == version
        });
        let composition_match = manifest.kind() == PackageKind::MetaHarness
            && manifest.meta_composition().is_some_and(|composition| {
                composition
                    .allowed_domains()
                    .iter()
                    .any(|domain| domain.as_str() == id)
            });
        if dependency_match || composition_match {
            dependents.insert(format!("{}@{}", manifest.id().as_str(), manifest.version()));
        }
    }
    dependents.into_iter().collect()
}

fn reject_enabled_dependents(
    registry: &HarnessRegistry,
    bindings: &BTreeMap<String, String>,
    id: &str,
    version: &str,
) -> Result<(), PackageStoreError> {
    let dependents = enabled_dependents(registry, bindings, id, version);
    if dependents.is_empty() {
        Ok(())
    } else {
        Err(PackageStoreError::HasEnabledDependents {
            id: id.to_owned(),
            version: version.to_owned(),
            dependents,
        })
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

fn load_publisher_trust_roots(
    connection: &rusqlite::Connection,
) -> Result<PublisherTrustRoots, PackageStoreError> {
    let records = load_publisher_trust_root_records(connection)?;
    let mut roots = PublisherTrustRoots::new();
    for record in records.into_iter().filter(PublisherTrustRootRecord::active) {
        roots
            .insert(record.root)
            .map_err(|_| PackageStoreError::CorruptRecord)?;
    }
    Ok(roots)
}

fn load_publisher_trust_root_records(
    connection: &rusqlite::Connection,
) -> Result<Vec<PublisherTrustRootRecord>, PackageStoreError> {
    let mut statement = connection.prepare(
        "SELECT publisher, key_id, public_key, added_at, revoked_at
         FROM publisher_trust_roots ORDER BY publisher ASC, key_id ASC",
    )?;
    let rows = statement.query_map([], decode_trust_root)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn decode_trust_root(row: &rusqlite::Row<'_>) -> rusqlite::Result<PublisherTrustRootRecord> {
    let publisher = row.get::<_, String>(0)?;
    let key_id = row.get::<_, String>(1)?;
    let public_key = row.get::<_, String>(2)?;
    let added_at = row.get::<_, i64>(3)?;
    let revoked_at = row.get::<_, Option<i64>>(4)?;
    let root = PublisherTrustRoot::new(publisher, key_id, public_key)
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    Ok(PublisherTrustRootRecord {
        root,
        added_at: u64::try_from(added_at).map_err(|_| rusqlite::Error::InvalidQuery)?,
        revoked_at: revoked_at
            .map(|value| u64::try_from(value).map_err(|_| rusqlite::Error::InvalidQuery))
            .transpose()?,
    })
}

fn load_registry(connection: &rusqlite::Connection) -> Result<HarnessRegistry, PackageStoreError> {
    let mut registry = HarnessRegistry::new();
    let trust_roots = load_publisher_trust_roots(connection)?;
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
            let record = match registry.install_with_trust_roots(
                &manifest,
                &manifest,
                &artifact,
                &trust_roots,
            ) {
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
    use ed25519_dalek::{Signer, SigningKey};
    use pandora_types::{
        MetaComposition, PackageCompatibility, PackageDependency, PackageKind, PackageManifest,
        TrustEvidence, TrustLevel, hash_artifact,
    };

    const CURRENT_RUNTIME_REQUIREMENT: &str = concat!("pandora>=", env!("CARGO_PKG_VERSION"));

    fn meta_manifest(artifact: &[u8]) -> PackageManifest {
        custom_meta_manifest(artifact, &["coding-domain"])
    }

    fn hex_bytes(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn hex_key(key: &SigningKey) -> String {
        hex_bytes(&key.verifying_key().to_bytes())
    }

    fn custom_meta_manifest(artifact: &[u8], domains: &[&str]) -> PackageManifest {
        PackageManifest::new_meta(
            "example/meta",
            "1.0.0",
            "publisher",
            hash_artifact(artifact),
            Vec::new(),
            PackageCompatibility::new(CURRENT_RUNTIME_REQUIREMENT).unwrap(),
            "MIT",
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
        versioned_gene_manifest(id, "1.0.0", artifact)
    }

    fn versioned_gene_manifest(id: &str, version: &str, artifact: &[u8]) -> PackageManifest {
        PackageManifest::new(
            id,
            version,
            PackageKind::Gene,
            "publisher",
            hash_artifact(artifact),
            Vec::new(),
            PackageCompatibility::new(CURRENT_RUNTIME_REQUIREMENT).unwrap(),
            "MIT",
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
            "MIT",
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
    fn admitted_package_stays_inert_until_explicitly_enabled() {
        let artifact = b"inert gene";
        let manifest = gene_manifest("example/inert", artifact);
        let (store, root) = store();

        store.admit(&manifest, &manifest, artifact).unwrap();
        assert!(!store.is_enabled(manifest.id(), manifest.version()).unwrap());

        let binding = store.enable(manifest.id(), manifest.version()).unwrap();
        assert_eq!(binding.active_version(), Some("1.0.0"));
        assert_eq!(binding.previous_version(), None);
        assert_eq!(binding.generation(), 1);
        assert!(store.is_enabled(manifest.id(), manifest.version()).unwrap());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn enabled_dependencies_fail_closed_during_activation_and_disable() {
        let gene_artifact = b"bounded gene";
        let gene = gene_manifest("example/bounded", gene_artifact);
        let domain_artifact = b"bounded domain";
        let domain = domain_manifest(
            "example/bounded-domain",
            vec![PackageDependency::new("example/bounded", "1.0.0", false).unwrap()],
            domain_artifact,
        );
        let (store, root) = store();
        store.admit(&gene, &gene, gene_artifact).unwrap();
        store.admit(&domain, &domain, domain_artifact).unwrap();

        assert!(matches!(
            store.enable(domain.id(), domain.version()),
            Err(PackageStoreError::MissingEnabledDependency { .. })
        ));
        store.enable(gene.id(), gene.version()).unwrap();
        store.enable(domain.id(), domain.version()).unwrap();
        assert!(matches!(
            store.disable(gene.id(), gene.version()),
            Err(PackageStoreError::HasEnabledDependents { .. })
        ));
        store.disable(domain.id(), domain.version()).unwrap();
        store.disable(gene.id(), gene.version()).unwrap();
        assert!(!store.is_enabled(gene.id(), gene.version()).unwrap());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn exact_version_switches_retain_a_reversible_binding() {
        let first_artifact = b"gene v1";
        let first = versioned_gene_manifest("example/versioned", "1.0.0", first_artifact);
        let second_artifact = b"gene v2";
        let second = versioned_gene_manifest("example/versioned", "2.0.0", second_artifact);
        let (store, root) = store();
        store.admit(&first, &first, first_artifact).unwrap();
        store.admit(&second, &second, second_artifact).unwrap();

        store.enable(first.id(), first.version()).unwrap();
        let updated = store.enable(second.id(), second.version()).unwrap();
        assert_eq!(updated.active_version(), Some("2.0.0"));
        assert_eq!(updated.previous_version(), Some("1.0.0"));

        let rolled_back = store.rollback(first.id()).unwrap();
        assert_eq!(rolled_back.active_version(), Some("1.0.0"));
        assert_eq!(rolled_back.previous_version(), Some("2.0.0"));
        assert_eq!(rolled_back.generation(), 3);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn active_and_rollback_versions_cannot_be_removed() {
        let first_artifact = b"retained v1";
        let first = versioned_gene_manifest("example/retained", "1.0.0", first_artifact);
        let second_artifact = b"retained v2";
        let second = versioned_gene_manifest("example/retained", "2.0.0", second_artifact);
        let (store, root) = store();
        store.admit(&first, &first, first_artifact).unwrap();
        store.admit(&second, &second, second_artifact).unwrap();
        store.enable(first.id(), first.version()).unwrap();
        store.enable(second.id(), second.version()).unwrap();

        assert!(matches!(
            store.remove(second.id(), second.version()),
            Err(PackageStoreError::PackageBound { role: "active", .. })
        ));
        assert!(matches!(
            store.remove(first.id(), first.version()),
            Err(PackageStoreError::PackageBound {
                role: "rollback",
                ..
            })
        ));
        let _ = std::fs::remove_dir_all(root);
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
            "MIT",
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
            "MIT",
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
    fn admitted_artifacts_can_be_loaded_by_content_identity() {
        let artifact = b"content-addressed gene";
        let manifest = gene_manifest("example/content-addressed", artifact);
        let artifact_id = ArtifactId::new(manifest.content_hash()).unwrap();
        let (store, root) = store();
        store.admit(&manifest, &manifest, artifact).unwrap();

        let (record, loaded) = store
            .load_artifact_by_id(&artifact_id)
            .unwrap()
            .expect("admitted artifact should resolve by content identity");

        assert_eq!(record.manifest(), &manifest);
        assert_eq!(record.state(), PackageState::Installed);
        assert_eq!(loaded, artifact);
        assert!(
            store
                .load_artifact_by_id(&ArtifactId::new("missing-artifact").unwrap())
                .unwrap()
                .is_none()
        );
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
            "MIT",
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
    fn publisher_roots_rotate_and_revoke_durably() {
        let (store, root) = store();
        let first_key = SigningKey::from_bytes(&[23_u8; 32]);
        let second_key = SigningKey::from_bytes(&[29_u8; 32]);
        let first = store
            .add_publisher_trust_root("publisher", "publisher-key-1", hex_key(&first_key), 10)
            .unwrap();
        assert!(first.active());
        store
            .add_publisher_trust_root("publisher", "publisher-key-2", hex_key(&second_key), 11)
            .unwrap();
        let revoked = store
            .revoke_publisher_trust_root("publisher", "publisher-key-1", 12)
            .unwrap();
        assert_eq!(revoked.revoked_at(), Some(12));
        assert!(!revoked.active());
        let roots = store.list_publisher_trust_roots().unwrap();
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0].key_id(), "publisher-key-1");
        assert_eq!(roots[1].key_id(), "publisher-key-2");
        assert!(roots[1].active());
        drop(store);
        let reopened = PackageStore::open(root.join("packages.sqlite3")).unwrap();
        assert_eq!(reopened.list_publisher_trust_roots().unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn official_package_admission_uses_an_active_publisher_root() {
        let artifact = b"official gene";
        let signing_key = SigningKey::from_bytes(&[31_u8; 32]);
        let content_hash = hash_artifact(artifact);
        let unsigned = PackageManifest::new(
            "publisher/gene",
            "1.0.0",
            PackageKind::Gene,
            "publisher",
            content_hash.clone(),
            Vec::new(),
            PackageCompatibility::new(CURRENT_RUNTIME_REQUIREMENT).unwrap(),
            "MIT",
            TrustEvidence::unsigned(),
        )
        .unwrap();
        let signature = signing_key.sign(unsigned.signing_message().as_bytes());
        let package = PackageManifest::new(
            "publisher/gene",
            "1.0.0",
            PackageKind::Gene,
            "publisher",
            content_hash,
            Vec::new(),
            PackageCompatibility::new(CURRENT_RUNTIME_REQUIREMENT).unwrap(),
            "MIT",
            TrustEvidence::new(
                TrustLevel::Official,
                Some(hex_bytes(&signature.to_bytes())),
                Some(hex_key(&signing_key)),
            )
            .unwrap(),
        )
        .unwrap();
        let (store, root) = store();
        store
            .add_publisher_trust_root("publisher", "publisher-key", hex_key(&signing_key), 10)
            .unwrap();
        store.admit(&package, &package, artifact).unwrap();
        assert_eq!(store.list().unwrap().len(), 1);
        store
            .revoke_publisher_trust_root("publisher", "publisher-key", 20)
            .unwrap();
        assert!(matches!(
            store.list(),
            Err(PackageStoreError::CorruptRecord)
        ));
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
            "MIT",
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
