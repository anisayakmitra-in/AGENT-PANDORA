use crate::harness_registry::verify_package_signature_with_key;
use crate::{MAX_STORED_ARTIFACT_BYTES, PackageStoreError};
use pandora_types::{PackageId, PackageKind, PackageManifest, TrustLevel, hash_artifact};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

pub const MAX_DISTRIBUTION_RECORDS: usize = 1_024;
pub const MAX_DISTRIBUTION_EVENTS: usize = 8_192;
pub const MAX_DISTRIBUTION_LIST: usize = 256;
const MAX_SOURCE_TEXT_BYTES: usize = 2_048;
const MAX_SKILL_BUNDLE_FILES: usize = 256;
const MAX_SKILL_BUNDLE_PATH_BYTES: usize = 512;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SkillDistributionBundle {
    format_version: u32,
    files: Vec<SkillDistributionFile>,
}

impl SkillDistributionBundle {
    pub fn new(files: Vec<SkillDistributionFile>) -> Result<Self, PackageDistributionError> {
        let bundle = Self {
            format_version: 1,
            files,
        };
        bundle.validate()?;
        Ok(bundle)
    }

    pub const fn format_version(&self) -> u32 {
        self.format_version
    }

    pub fn files(&self) -> &[SkillDistributionFile] {
        &self.files
    }

    fn validate(&self) -> Result<(), PackageDistributionError> {
        if self.format_version != 1
            || self.files.is_empty()
            || self.files.len() > MAX_SKILL_BUNDLE_FILES
        {
            return Err(PackageDistributionError::InvalidBundle);
        }
        let mut paths = BTreeSet::new();
        for file in &self.files {
            file.validate()?;
            if !paths.insert(file.path.as_str()) {
                return Err(PackageDistributionError::InvalidBundle);
            }
        }
        if !paths.contains("SKILL.md") {
            return Err(PackageDistributionError::InvalidBundle);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SkillDistributionFile {
    path: String,
    content: String,
}

impl SkillDistributionFile {
    pub fn new(
        path: impl Into<String>,
        content: impl Into<String>,
    ) -> Result<Self, PackageDistributionError> {
        let file = Self {
            path: path.into(),
            content: content.into(),
        };
        file.validate()?;
        Ok(file)
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    fn validate(&self) -> Result<(), PackageDistributionError> {
        if self.path.is_empty()
            || self.path.len() > MAX_SKILL_BUNDLE_PATH_BYTES
            || self.path.contains('\\')
            || self.path.starts_with('/')
            || self.path.ends_with('/')
            || self.path.chars().any(char::is_control)
            || self
                .path
                .split('/')
                .any(|part| part.is_empty() || matches!(part, "." | "..") || part.contains(':'))
        {
            return Err(PackageDistributionError::InvalidBundle);
        }
        Ok(())
    }
}

pub fn materialize_skill_bundle(
    artifact: &[u8],
    staging_root: &Path,
    skill_id: &str,
) -> Result<PathBuf, PackageDistributionError> {
    let bundle: SkillDistributionBundle =
        serde_json::from_slice(artifact).map_err(|_| PackageDistributionError::InvalidBundle)?;
    bundle.validate()?;
    if skill_id.is_empty()
        || matches!(skill_id, "." | "..")
        || !skill_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(PackageDistributionError::InvalidBundle);
    }
    std::fs::create_dir(staging_root)?;
    let skill_root = staging_root.join(skill_id);
    std::fs::create_dir(&skill_root)?;
    for file in bundle.files {
        let destination = skill_root.join(&file.path);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut handle = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)?;
        handle.write_all(file.content.as_bytes())?;
        handle.sync_all()?;
    }
    Ok(skill_root)
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DistributionSourceKind {
    Registry,
    GitHub,
}

impl DistributionSourceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Registry => "registry",
            Self::GitHub => "github",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "registry" => Some(Self::Registry),
            "github" => Some(Self::GitHub),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DistributionSource {
    kind: DistributionSourceKind,
    locator: String,
    revision: String,
}

impl DistributionSource {
    pub fn new(
        kind: DistributionSourceKind,
        locator: impl Into<String>,
        revision: impl Into<String>,
    ) -> Result<Self, PackageDistributionError> {
        let source = Self {
            kind,
            locator: locator.into(),
            revision: revision.into(),
        };
        source.validate()?;
        Ok(source)
    }

    pub const fn kind(&self) -> DistributionSourceKind {
        self.kind
    }

    pub fn locator(&self) -> &str {
        &self.locator
    }

    pub fn revision(&self) -> &str {
        &self.revision
    }

    fn validate(&self) -> Result<(), PackageDistributionError> {
        for value in [&self.locator, &self.revision] {
            if value.is_empty()
                || value.len() > MAX_SOURCE_TEXT_BYTES
                || value.chars().any(char::is_control)
            {
                return Err(PackageDistributionError::InvalidSource);
            }
        }
        match self.kind {
            DistributionSourceKind::Registry => {
                Version::parse(&self.revision)
                    .map_err(|_| PackageDistributionError::InvalidSource)?;
            }
            DistributionSourceKind::GitHub => {
                if self.revision.len() != 40
                    || !self
                        .revision
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
                {
                    return Err(PackageDistributionError::InvalidSource);
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DistributionState {
    Cached,
    Admitted,
    Revoked,
}

impl DistributionState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cached => "cached",
            Self::Admitted => "admitted",
            Self::Revoked => "revoked",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "cached" => Some(Self::Cached),
            "admitted" => Some(Self::Admitted),
            "revoked" => Some(Self::Revoked),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DistributionRecord {
    manifest: PackageManifest,
    artifact: Vec<u8>,
    source: DistributionSource,
    publisher_key_id: String,
    cached_at: u64,
    admitted_at: Option<u64>,
    state: DistributionState,
}

impl DistributionRecord {
    pub fn manifest(&self) -> &PackageManifest {
        &self.manifest
    }

    pub fn artifact(&self) -> &[u8] {
        &self.artifact
    }

    pub fn source(&self) -> &DistributionSource {
        &self.source
    }

    pub fn publisher_key_id(&self) -> &str {
        &self.publisher_key_id
    }

    pub const fn cached_at(&self) -> u64 {
        self.cached_at
    }

    pub const fn admitted_at(&self) -> Option<u64> {
        self.admitted_at
    }

    pub const fn state(&self) -> DistributionState {
        self.state
    }

    pub fn manifest_digest(&self) -> Result<String, PackageDistributionError> {
        canonical_manifest_digest(&self.manifest)
    }

    pub fn artifact_digest(&self) -> String {
        hash_artifact(&self.artifact)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DistributionBinding {
    id: String,
    active_version: String,
    previous_version: Option<String>,
    generation: u64,
}

impl DistributionBinding {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn active_version(&self) -> &str {
        &self.active_version
    }

    pub fn previous_version(&self) -> Option<&str> {
        self.previous_version.as_deref()
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DistributionEventKind {
    DownloadVerified,
    OfflineVerified,
    Admitted,
    Revoked,
}

impl DistributionEventKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DownloadVerified => "download_verified",
            Self::OfflineVerified => "offline_verified",
            Self::Admitted => "admitted",
            Self::Revoked => "revoked",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "download_verified" => Some(Self::DownloadVerified),
            "offline_verified" => Some(Self::OfflineVerified),
            "admitted" => Some(Self::Admitted),
            "revoked" => Some(Self::Revoked),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DistributionEvent {
    sequence: u64,
    event_kind: DistributionEventKind,
    occurred_at: u64,
    package_id: String,
    package_version: String,
    package_kind: PackageKind,
    publisher: String,
    publisher_key_id: String,
    manifest_digest: String,
    artifact_digest: String,
    source_kind: DistributionSourceKind,
    source_locator: String,
    source_revision: String,
    previous_event_digest: Option<String>,
    event_digest: String,
}

impl DistributionEvent {
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    pub const fn event_kind(&self) -> DistributionEventKind {
        self.event_kind
    }

    pub const fn occurred_at(&self) -> u64 {
        self.occurred_at
    }

    pub fn package_id(&self) -> &str {
        &self.package_id
    }

    pub fn package_version(&self) -> &str {
        &self.package_version
    }

    pub const fn package_kind(&self) -> PackageKind {
        self.package_kind
    }

    pub fn publisher(&self) -> &str {
        &self.publisher
    }

    pub fn publisher_key_id(&self) -> &str {
        &self.publisher_key_id
    }

    pub fn manifest_digest(&self) -> &str {
        &self.manifest_digest
    }

    pub fn artifact_digest(&self) -> &str {
        &self.artifact_digest
    }

    pub const fn source_kind(&self) -> DistributionSourceKind {
        self.source_kind
    }

    pub fn source_locator(&self) -> &str {
        &self.source_locator
    }

    pub fn source_revision(&self) -> &str {
        &self.source_revision
    }

    pub fn previous_event_digest(&self) -> Option<&str> {
        self.previous_event_digest.as_deref()
    }

    pub fn event_digest(&self) -> &str {
        &self.event_digest
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DistributionMutation {
    record: DistributionRecord,
    changed: bool,
}

impl DistributionMutation {
    pub fn record(&self) -> &DistributionRecord {
        &self.record
    }

    pub const fn changed(&self) -> bool {
        self.changed
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PackageDistributionError {
    Database,
    Serialization,
    CorruptStore,
    CapacityReached,
    ArtifactTooLarge,
    InvalidSource,
    InvalidManifest,
    InvalidBundle,
    Io,
    UnsupportedKind(PackageKind),
    HashMismatch,
    IncompatibleRuntime,
    UntrustedPublisher,
    RevokedPublisher,
    InvalidSignature,
    IdentityConflict,
    NotFound,
    Revoked,
    MissingDependency { id: String, version: String },
    DowngradeDenied { active: String, requested: String },
}

impl fmt::Display for PackageDistributionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database => formatter.write_str("package distribution store operation failed"),
            Self::Serialization | Self::CorruptStore => {
                formatter.write_str("package distribution store contains invalid evidence")
            }
            Self::CapacityReached => {
                formatter.write_str("package distribution evidence capacity was reached")
            }
            Self::ArtifactTooLarge => {
                formatter.write_str("package artifact exceeds the local limit")
            }
            Self::InvalidSource => formatter.write_str("package distribution source is invalid"),
            Self::InvalidManifest => {
                formatter.write_str("package distribution manifest is invalid")
            }
            Self::InvalidBundle => {
                formatter.write_str("Skill distribution bundle is invalid or escapes its root")
            }
            Self::Io => formatter.write_str("package distribution filesystem operation failed"),
            Self::UnsupportedKind(kind) => write!(
                formatter,
                "{} packages are not supported by remote distribution",
                kind.as_str()
            ),
            Self::HashMismatch => {
                formatter.write_str("downloaded artifact hash does not match the signed manifest")
            }
            Self::IncompatibleRuntime => {
                formatter.write_str("downloaded package is incompatible with this runtime")
            }
            Self::UntrustedPublisher => {
                formatter.write_str("remote package requires an active publisher trust root")
            }
            Self::RevokedPublisher => {
                formatter.write_str("remote package publisher key is revoked")
            }
            Self::InvalidSignature => {
                formatter.write_str("remote package signature verification failed")
            }
            Self::IdentityConflict => formatter
                .write_str("package identity is already cached with different immutable evidence"),
            Self::NotFound => formatter.write_str("cached package was not found"),
            Self::Revoked => formatter.write_str("cached package is revoked"),
            Self::MissingDependency { id, version } => {
                write!(
                    formatter,
                    "required dependency {id}@{version} is not admitted"
                )
            }
            Self::DowngradeDenied { active, requested } => write!(
                formatter,
                "package downgrade from {active} to {requested} is denied by normal admission"
            ),
        }
    }
}

impl std::error::Error for PackageDistributionError {}

impl From<rusqlite::Error> for PackageDistributionError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Database
    }
}

impl From<serde_json::Error> for PackageDistributionError {
    fn from(_: serde_json::Error) -> Self {
        Self::Serialization
    }
}

impl From<std::io::Error> for PackageDistributionError {
    fn from(_: std::io::Error) -> Self {
        Self::Io
    }
}

pub struct PackageDistributionStore {
    connection: Mutex<Connection>,
}

impl PackageDistributionStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PackageDistributionError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|_| PackageDistributionError::Database)?;
        }
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS publisher_trust_roots (
                 publisher TEXT NOT NULL,
                 key_id TEXT NOT NULL,
                 public_key TEXT NOT NULL,
                 added_at INTEGER NOT NULL CHECK (added_at >= 0),
                 revoked_at INTEGER CHECK (revoked_at IS NULL OR revoked_at >= 0),
                 PRIMARY KEY (publisher, key_id)
             );
             CREATE TABLE IF NOT EXISTS package_distribution_cache (
                 id TEXT NOT NULL,
                 version TEXT NOT NULL,
                 manifest_json TEXT NOT NULL,
                 artifact BLOB NOT NULL,
                 source_kind TEXT NOT NULL,
                 source_locator TEXT NOT NULL,
                 source_revision TEXT NOT NULL,
                 publisher_key_id TEXT NOT NULL,
                 cached_at INTEGER NOT NULL CHECK (cached_at >= 0),
                 admitted_at INTEGER CHECK (admitted_at IS NULL OR admitted_at >= 0),
                 state TEXT NOT NULL CHECK (state IN ('cached', 'admitted', 'revoked')),
                 PRIMARY KEY (id, version)
             );
             CREATE TABLE IF NOT EXISTS package_distribution_bindings (
                 id TEXT PRIMARY KEY,
                 active_version TEXT NOT NULL,
                 previous_version TEXT,
                 generation INTEGER NOT NULL CHECK (generation > 0)
             );
             CREATE TABLE IF NOT EXISTS package_distribution_events (
                 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                 event_kind TEXT NOT NULL,
                 occurred_at INTEGER NOT NULL CHECK (occurred_at >= 0),
                 package_id TEXT NOT NULL,
                 package_version TEXT NOT NULL,
                 package_kind TEXT NOT NULL,
                 publisher TEXT NOT NULL,
                 publisher_key_id TEXT NOT NULL,
                 manifest_digest TEXT NOT NULL,
                 artifact_digest TEXT NOT NULL,
                 source_kind TEXT NOT NULL,
                 source_locator TEXT NOT NULL,
                 source_revision TEXT NOT NULL,
                 previous_event_digest TEXT,
                 event_digest TEXT NOT NULL UNIQUE
             );
             CREATE TRIGGER IF NOT EXISTS package_distribution_events_no_update
             BEFORE UPDATE ON package_distribution_events
             BEGIN
                 SELECT RAISE(ABORT, 'package distribution events are append-only');
             END;
             CREATE TRIGGER IF NOT EXISTS package_distribution_events_no_delete
             BEFORE DELETE ON package_distribution_events
             BEGIN
                 SELECT RAISE(ABORT, 'package distribution events are append-only');
             END;",
        )?;
        validate_event_chain(&connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn cache_verified(
        &self,
        manifest: &PackageManifest,
        artifact: &[u8],
        source: DistributionSource,
        occurred_at: u64,
    ) -> Result<DistributionMutation, PackageDistributionError> {
        source.validate()?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let key_id = verify_candidate(&transaction, manifest, artifact)?;
        let manifest_json = serde_json::to_string(manifest)?;
        let existing = load_record(&transaction, manifest.id(), manifest.version())?;
        if let Some(existing) = existing {
            if existing.manifest != *manifest
                || existing.artifact != artifact
                || existing.source != source
                || existing.publisher_key_id != key_id
            {
                return Err(PackageDistributionError::IdentityConflict);
            }
            transaction.commit()?;
            return Ok(DistributionMutation {
                record: existing,
                changed: false,
            });
        }
        let count: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM package_distribution_cache",
            [],
            |row| row.get(0),
        )?;
        if usize::try_from(count).map_err(|_| PackageDistributionError::CorruptStore)?
            >= MAX_DISTRIBUTION_RECORDS
        {
            return Err(PackageDistributionError::CapacityReached);
        }
        transaction.execute(
            "INSERT INTO package_distribution_cache
             (id, version, manifest_json, artifact, source_kind, source_locator,
              source_revision, publisher_key_id, cached_at, state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'cached')",
            params![
                manifest.id().as_str(),
                manifest.version(),
                manifest_json,
                artifact,
                source.kind().as_str(),
                source.locator(),
                source.revision(),
                key_id,
                to_i64(occurred_at)?,
            ],
        )?;
        let record = DistributionRecord {
            manifest: manifest.clone(),
            artifact: artifact.to_vec(),
            source,
            publisher_key_id: key_id,
            cached_at: occurred_at,
            admitted_at: None,
            state: DistributionState::Cached,
        };
        append_event(
            &transaction,
            DistributionEventKind::DownloadVerified,
            occurred_at,
            &record,
        )?;
        transaction.commit()?;
        Ok(DistributionMutation {
            record,
            changed: true,
        })
    }

    pub fn get(
        &self,
        id: &PackageId,
        version: &str,
    ) -> Result<Option<DistributionRecord>, PackageDistributionError> {
        let connection = self.lock()?;
        load_record(&connection, id, version)
    }

    pub fn list(&self, limit: usize) -> Result<Vec<DistributionRecord>, PackageDistributionError> {
        if limit == 0 || limit > MAX_DISTRIBUTION_LIST {
            return Err(PackageDistributionError::CorruptStore);
        }
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT manifest_json, artifact, source_kind, source_locator, source_revision,
                    publisher_key_id, cached_at, admitted_at, state
             FROM package_distribution_cache
             ORDER BY id ASC, version ASC LIMIT ?1",
        )?;
        let rows = statement.query_map(
            [i64::try_from(limit).map_err(|_| PackageDistributionError::CorruptStore)?],
            decode_record,
        )?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|_| PackageDistributionError::CorruptStore)
    }

    pub fn verify_offline(
        &self,
        id: &PackageId,
        version: &str,
        occurred_at: u64,
    ) -> Result<DistributionRecord, PackageDistributionError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record =
            load_record(&transaction, id, version)?.ok_or(PackageDistributionError::NotFound)?;
        if record.state == DistributionState::Revoked {
            return Err(PackageDistributionError::Revoked);
        }
        let key_id = verify_candidate(&transaction, &record.manifest, &record.artifact)?;
        if key_id != record.publisher_key_id {
            return Err(PackageDistributionError::IdentityConflict);
        }
        append_event(
            &transaction,
            DistributionEventKind::OfflineVerified,
            occurred_at,
            &record,
        )?;
        transaction.commit()?;
        Ok(record)
    }

    pub fn verify_current(
        &self,
        id: &PackageId,
        version: &str,
    ) -> Result<DistributionRecord, PackageDistributionError> {
        let connection = self.lock()?;
        let record =
            load_record(&connection, id, version)?.ok_or(PackageDistributionError::NotFound)?;
        if record.state == DistributionState::Revoked {
            return Err(PackageDistributionError::Revoked);
        }
        let key_id = verify_candidate(&connection, &record.manifest, &record.artifact)?;
        if key_id != record.publisher_key_id {
            return Err(PackageDistributionError::IdentityConflict);
        }
        Ok(record)
    }

    pub fn prepare_admission(
        &self,
        id: &PackageId,
        version: &str,
    ) -> Result<DistributionRecord, PackageDistributionError> {
        let connection = self.lock()?;
        let record =
            load_record(&connection, id, version)?.ok_or(PackageDistributionError::NotFound)?;
        if record.state == DistributionState::Revoked {
            return Err(PackageDistributionError::Revoked);
        }
        let key_id = verify_candidate(&connection, &record.manifest, &record.artifact)?;
        if key_id != record.publisher_key_id {
            return Err(PackageDistributionError::IdentityConflict);
        }
        verify_dependencies(&connection, &record.manifest)?;
        if let Some(binding) = load_binding(&connection, id)? {
            if binding.active_version == version {
                return Ok(record);
            }
            let active = Version::parse(&binding.active_version)
                .map_err(|_| PackageDistributionError::CorruptStore)?;
            let requested =
                Version::parse(version).map_err(|_| PackageDistributionError::InvalidManifest)?;
            if requested <= active {
                return Err(PackageDistributionError::DowngradeDenied {
                    active: binding.active_version,
                    requested: version.to_owned(),
                });
            }
        }
        Ok(record)
    }

    pub fn record_admission(
        &self,
        id: &PackageId,
        version: &str,
        artifact_digest: &str,
        occurred_at: u64,
    ) -> Result<DistributionMutation, PackageDistributionError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut record =
            load_record(&transaction, id, version)?.ok_or(PackageDistributionError::NotFound)?;
        if record.artifact_digest() != artifact_digest {
            return Err(PackageDistributionError::IdentityConflict);
        }
        let key_id = verify_candidate(&transaction, &record.manifest, &record.artifact)?;
        if key_id != record.publisher_key_id {
            return Err(PackageDistributionError::IdentityConflict);
        }
        verify_dependencies(&transaction, &record.manifest)?;
        let binding = load_binding(&transaction, id)?;
        if binding
            .as_ref()
            .is_some_and(|binding| binding.active_version == version)
            && record.state == DistributionState::Admitted
        {
            transaction.commit()?;
            return Ok(DistributionMutation {
                record,
                changed: false,
            });
        }
        if let Some(binding) = binding.as_ref() {
            let active = Version::parse(&binding.active_version)
                .map_err(|_| PackageDistributionError::CorruptStore)?;
            let requested =
                Version::parse(version).map_err(|_| PackageDistributionError::InvalidManifest)?;
            if requested <= active {
                return Err(PackageDistributionError::DowngradeDenied {
                    active: binding.active_version.clone(),
                    requested: version.to_owned(),
                });
            }
        }
        let generation = binding
            .as_ref()
            .map_or(1, |binding| binding.generation.saturating_add(1));
        let previous = binding
            .as_ref()
            .map(|binding| binding.active_version.as_str());
        transaction.execute(
            "INSERT INTO package_distribution_bindings
             (id, active_version, previous_version, generation)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
               active_version = excluded.active_version,
               previous_version = excluded.previous_version,
               generation = excluded.generation",
            params![id.as_str(), version, previous, to_i64(generation)?],
        )?;
        transaction.execute(
            "UPDATE package_distribution_cache
             SET state = 'admitted', admitted_at = ?3
             WHERE id = ?1 AND version = ?2",
            params![id.as_str(), version, to_i64(occurred_at)?],
        )?;
        record.state = DistributionState::Admitted;
        record.admitted_at = Some(occurred_at);
        append_event(
            &transaction,
            DistributionEventKind::Admitted,
            occurred_at,
            &record,
        )?;
        transaction.commit()?;
        Ok(DistributionMutation {
            record,
            changed: true,
        })
    }

    pub fn binding(
        &self,
        id: &PackageId,
    ) -> Result<Option<DistributionBinding>, PackageDistributionError> {
        let connection = self.lock()?;
        load_binding(&connection, id)
    }

    pub fn revoke_publisher_key(
        &self,
        publisher: &str,
        key_id: &str,
        occurred_at: u64,
    ) -> Result<Vec<DistributionRecord>, PackageDistributionError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut records = load_records_for_key(&transaction, publisher, key_id)?;
        for record in &mut records {
            if record.state == DistributionState::Revoked {
                continue;
            }
            record.state = DistributionState::Revoked;
            transaction.execute(
                "UPDATE package_distribution_cache SET state = 'revoked'
                 WHERE id = ?1 AND version = ?2",
                params![record.manifest.id().as_str(), record.manifest.version()],
            )?;
            transaction.execute(
                "DELETE FROM package_distribution_bindings
                 WHERE id = ?1 AND (active_version = ?2 OR previous_version = ?2)",
                params![record.manifest.id().as_str(), record.manifest.version()],
            )?;
            append_event(
                &transaction,
                DistributionEventKind::Revoked,
                occurred_at,
                record,
            )?;
        }
        transaction.commit()?;
        Ok(records)
    }

    pub fn records_for_publisher_key(
        &self,
        publisher: &str,
        key_id: &str,
    ) -> Result<Vec<DistributionRecord>, PackageDistributionError> {
        let connection = self.lock()?;
        load_records_for_key(&connection, publisher, key_id)
    }

    pub fn list_events(
        &self,
        limit: usize,
    ) -> Result<Vec<DistributionEvent>, PackageDistributionError> {
        if limit == 0 || limit > MAX_DISTRIBUTION_LIST {
            return Err(PackageDistributionError::CorruptStore);
        }
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT sequence, event_kind, occurred_at, package_id, package_version,
                    package_kind, publisher, publisher_key_id, manifest_digest,
                    artifact_digest, source_kind, source_locator, source_revision,
                    previous_event_digest, event_digest
             FROM package_distribution_events ORDER BY sequence DESC LIMIT ?1",
        )?;
        let rows = statement.query_map(
            [i64::try_from(limit).map_err(|_| PackageDistributionError::CorruptStore)?],
            decode_event,
        )?;
        let mut events = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| PackageDistributionError::CorruptStore)?;
        events.reverse();
        Ok(events)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, PackageDistributionError> {
        self.connection
            .lock()
            .map_err(|_| PackageDistributionError::Database)
    }
}

fn verify_candidate(
    connection: &Connection,
    manifest: &PackageManifest,
    artifact: &[u8],
) -> Result<String, PackageDistributionError> {
    manifest
        .validate()
        .map_err(|_| PackageDistributionError::InvalidManifest)?;
    if !matches!(
        manifest.kind(),
        PackageKind::Gene
            | PackageKind::DomainHarness
            | PackageKind::MetaHarness
            | PackageKind::Provider
            | PackageKind::Skill
    ) {
        return Err(PackageDistributionError::UnsupportedKind(manifest.kind()));
    }
    if artifact.len() > MAX_STORED_ARTIFACT_BYTES {
        return Err(PackageDistributionError::ArtifactTooLarge);
    }
    if hash_artifact(artifact) != manifest.content_hash() {
        return Err(PackageDistributionError::HashMismatch);
    }
    if !manifest
        .compatibility()
        .matches_runtime(env!("CARGO_PKG_VERSION"))
        .map_err(|_| PackageDistributionError::InvalidManifest)?
    {
        return Err(PackageDistributionError::IncompatibleRuntime);
    }
    if manifest.trust().level() != TrustLevel::Official {
        return Err(PackageDistributionError::UntrustedPublisher);
    }
    let public_key = manifest
        .trust()
        .public_key()
        .ok_or(PackageDistributionError::UntrustedPublisher)?;
    let active_key = connection
        .query_row(
            "SELECT key_id FROM publisher_trust_roots
             WHERE publisher = ?1 AND public_key = ?2 AND revoked_at IS NULL
             ORDER BY key_id ASC LIMIT 1",
            params![manifest.publisher(), public_key],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(key_id) = active_key else {
        let revoked = connection.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM publisher_trust_roots
                 WHERE publisher = ?1 AND public_key = ?2 AND revoked_at IS NOT NULL
             )",
            params![manifest.publisher(), public_key],
            |row| row.get::<_, bool>(0),
        )?;
        return Err(if revoked {
            PackageDistributionError::RevokedPublisher
        } else {
            PackageDistributionError::UntrustedPublisher
        });
    };
    verify_package_signature_with_key(manifest, public_key)
        .map_err(|_| PackageDistributionError::InvalidSignature)?;
    Ok(key_id)
}

fn verify_dependencies(
    connection: &Connection,
    manifest: &PackageManifest,
) -> Result<(), PackageDistributionError> {
    for dependency in manifest
        .dependencies()
        .iter()
        .filter(|dependency| !dependency.optional())
    {
        let admitted_distribution = connection.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM package_distribution_cache
                 WHERE id = ?1 AND version = ?2 AND state = 'admitted'
             )",
            params![dependency.id().as_str(), dependency.version()],
            |row| row.get::<_, bool>(0),
        )?;
        let admitted_local = connection.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'package_records'
             )",
            [],
            |row| row.get::<_, bool>(0),
        )? && connection.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM package_records WHERE id = ?1 AND version = ?2
             )",
            params![dependency.id().as_str(), dependency.version()],
            |row| row.get::<_, bool>(0),
        )?;
        if !admitted_distribution && !admitted_local {
            return Err(PackageDistributionError::MissingDependency {
                id: dependency.id().as_str().to_owned(),
                version: dependency.version().to_owned(),
            });
        }
    }
    Ok(())
}

fn load_record(
    connection: &Connection,
    id: &PackageId,
    version: &str,
) -> Result<Option<DistributionRecord>, PackageDistributionError> {
    connection
        .query_row(
            "SELECT manifest_json, artifact, source_kind, source_locator, source_revision,
                    publisher_key_id, cached_at, admitted_at, state
             FROM package_distribution_cache WHERE id = ?1 AND version = ?2",
            params![id.as_str(), version],
            decode_record,
        )
        .optional()
        .map_err(Into::into)
}

fn decode_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<DistributionRecord> {
    let manifest_json = row.get::<_, String>(0)?;
    let manifest = serde_json::from_str::<PackageManifest>(&manifest_json).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            manifest_json.len(),
            rusqlite::types::Type::Text,
            Box::new(PackageDistributionError::CorruptStore),
        )
    })?;
    let source_kind_text = row.get::<_, String>(2)?;
    let source_kind = DistributionSourceKind::parse(&source_kind_text).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            source_kind_text.len(),
            rusqlite::types::Type::Text,
            Box::new(PackageDistributionError::CorruptStore),
        )
    })?;
    let source = DistributionSource::new(
        source_kind,
        row.get::<_, String>(3)?,
        row.get::<_, String>(4)?,
    )
    .map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let state_text = row.get::<_, String>(8)?;
    let state = DistributionState::parse(&state_text).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            state_text.len(),
            rusqlite::types::Type::Text,
            Box::new(PackageDistributionError::CorruptStore),
        )
    })?;
    Ok(DistributionRecord {
        manifest,
        artifact: row.get(1)?,
        source,
        publisher_key_id: row.get(5)?,
        cached_at: from_i64(row.get(6)?)?,
        admitted_at: row.get::<_, Option<i64>>(7)?.map(from_i64).transpose()?,
        state,
    })
}

fn load_binding(
    connection: &Connection,
    id: &PackageId,
) -> Result<Option<DistributionBinding>, PackageDistributionError> {
    connection
        .query_row(
            "SELECT active_version, previous_version, generation
             FROM package_distribution_bindings WHERE id = ?1",
            [id.as_str()],
            |row| {
                Ok(DistributionBinding {
                    id: id.as_str().to_owned(),
                    active_version: row.get(0)?,
                    previous_version: row.get(1)?,
                    generation: from_i64(row.get(2)?)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
}

fn load_records_for_key(
    connection: &Connection,
    publisher: &str,
    key_id: &str,
) -> Result<Vec<DistributionRecord>, PackageDistributionError> {
    let mut statement = connection.prepare(
        "SELECT manifest_json, artifact, source_kind, source_locator, source_revision,
                publisher_key_id, cached_at, admitted_at, state
         FROM package_distribution_cache
         WHERE publisher_key_id = ?1 ORDER BY id ASC, version ASC",
    )?;
    let rows = statement.query_map([key_id], decode_record)?;
    let records = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| PackageDistributionError::CorruptStore)?;
    Ok(records
        .into_iter()
        .filter(|record| record.manifest.publisher() == publisher)
        .collect())
}

fn append_event(
    transaction: &Transaction<'_>,
    event_kind: DistributionEventKind,
    occurred_at: u64,
    record: &DistributionRecord,
) -> Result<(), PackageDistributionError> {
    let count: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM package_distribution_events",
        [],
        |row| row.get(0),
    )?;
    if usize::try_from(count).map_err(|_| PackageDistributionError::CorruptStore)?
        >= MAX_DISTRIBUTION_EVENTS
    {
        return Err(PackageDistributionError::CapacityReached);
    }
    let previous = transaction
        .query_row(
            "SELECT event_digest FROM package_distribution_events
             ORDER BY sequence DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let sequence = u64::try_from(count)
        .map_err(|_| PackageDistributionError::CorruptStore)?
        .saturating_add(1);
    let manifest_digest = record.manifest_digest()?;
    let artifact_digest = record.artifact_digest();
    let digest = event_digest(
        sequence,
        event_kind,
        occurred_at,
        &record.manifest,
        &record.publisher_key_id,
        &manifest_digest,
        &artifact_digest,
        &record.source,
        previous.as_deref(),
    );
    transaction.execute(
        "INSERT INTO package_distribution_events
         (event_kind, occurred_at, package_id, package_version, package_kind,
          publisher, publisher_key_id, manifest_digest, artifact_digest,
          source_kind, source_locator, source_revision, previous_event_digest,
          event_digest)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            event_kind.as_str(),
            to_i64(occurred_at)?,
            record.manifest.id().as_str(),
            record.manifest.version(),
            record.manifest.kind().as_str(),
            record.manifest.publisher(),
            record.publisher_key_id,
            manifest_digest,
            artifact_digest,
            record.source.kind().as_str(),
            record.source.locator(),
            record.source.revision(),
            previous,
            digest,
        ],
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn event_digest(
    sequence: u64,
    event_kind: DistributionEventKind,
    occurred_at: u64,
    manifest: &PackageManifest,
    key_id: &str,
    manifest_digest: &str,
    artifact_digest: &str,
    source: &DistributionSource,
    previous: Option<&str>,
) -> String {
    let mut bytes = Vec::new();
    for (label, value) in [
        ("sequence", sequence.to_string()),
        ("event_kind", event_kind.as_str().to_owned()),
        ("occurred_at", occurred_at.to_string()),
        ("package_id", manifest.id().as_str().to_owned()),
        ("package_version", manifest.version().to_owned()),
        ("package_kind", manifest.kind().as_str().to_owned()),
        ("publisher", manifest.publisher().to_owned()),
        ("publisher_key_id", key_id.to_owned()),
        ("manifest_digest", manifest_digest.to_owned()),
        ("artifact_digest", artifact_digest.to_owned()),
        ("source_kind", source.kind().as_str().to_owned()),
        ("source_locator", source.locator().to_owned()),
        ("source_revision", source.revision().to_owned()),
        ("previous", previous.unwrap_or("").to_owned()),
    ] {
        append_digest_field(&mut bytes, label, &value);
    }
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn validate_event_chain(connection: &Connection) -> Result<(), PackageDistributionError> {
    let mut statement = connection.prepare(
        "SELECT sequence, event_kind, occurred_at, package_id, package_version,
                package_kind, publisher, publisher_key_id, manifest_digest,
                artifact_digest, source_kind, source_locator, source_revision,
                previous_event_digest, event_digest
         FROM package_distribution_events ORDER BY sequence ASC",
    )?;
    let rows = statement.query_map([], decode_event)?;
    let mut expected_sequence = 1_u64;
    let mut previous: Option<String> = None;
    for row in rows {
        let event = row.map_err(|_| PackageDistributionError::CorruptStore)?;
        if event.sequence != expected_sequence
            || event.previous_event_digest != previous
            || event.event_digest
                != event_digest_from_event(&event, event.previous_event_digest.as_deref())
        {
            return Err(PackageDistributionError::CorruptStore);
        }
        expected_sequence = expected_sequence.saturating_add(1);
        previous = Some(event.event_digest);
    }
    Ok(())
}

fn decode_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<DistributionEvent> {
    let kind_text = row.get::<_, String>(1)?;
    let event_kind = DistributionEventKind::parse(&kind_text).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            kind_text.len(),
            rusqlite::types::Type::Text,
            Box::new(PackageDistributionError::CorruptStore),
        )
    })?;
    let package_kind_text = row.get::<_, String>(5)?;
    let package_kind = PackageKind::parse(&package_kind_text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            package_kind_text.len(),
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })?;
    let source_kind_text = row.get::<_, String>(10)?;
    let source_kind = DistributionSourceKind::parse(&source_kind_text).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            source_kind_text.len(),
            rusqlite::types::Type::Text,
            Box::new(PackageDistributionError::CorruptStore),
        )
    })?;
    Ok(DistributionEvent {
        sequence: from_i64(row.get(0)?)?,
        event_kind,
        occurred_at: from_i64(row.get(2)?)?,
        package_id: row.get(3)?,
        package_version: row.get(4)?,
        package_kind,
        publisher: row.get(6)?,
        publisher_key_id: row.get(7)?,
        manifest_digest: row.get(8)?,
        artifact_digest: row.get(9)?,
        source_kind,
        source_locator: row.get(11)?,
        source_revision: row.get(12)?,
        previous_event_digest: row.get(13)?,
        event_digest: row.get(14)?,
    })
}

fn event_digest_from_event(event: &DistributionEvent, previous: Option<&str>) -> String {
    let mut bytes = Vec::new();
    for (label, value) in [
        ("sequence", event.sequence.to_string()),
        ("event_kind", event.event_kind.as_str().to_owned()),
        ("occurred_at", event.occurred_at.to_string()),
        ("package_id", event.package_id.clone()),
        ("package_version", event.package_version.clone()),
        ("package_kind", event.package_kind.as_str().to_owned()),
        ("publisher", event.publisher.clone()),
        ("publisher_key_id", event.publisher_key_id.clone()),
        ("manifest_digest", event.manifest_digest.clone()),
        ("artifact_digest", event.artifact_digest.clone()),
        ("source_kind", event.source_kind.as_str().to_owned()),
        ("source_locator", event.source_locator.clone()),
        ("source_revision", event.source_revision.clone()),
        ("previous", previous.unwrap_or("").to_owned()),
    ] {
        append_digest_field(&mut bytes, label, &value);
    }
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn append_digest_field(buffer: &mut Vec<u8>, label: &str, value: &str) {
    buffer.extend_from_slice(label.len().to_string().as_bytes());
    buffer.push(b':');
    buffer.extend_from_slice(label.as_bytes());
    buffer.push(b':');
    buffer.extend_from_slice(value.len().to_string().as_bytes());
    buffer.push(b':');
    buffer.extend_from_slice(value.as_bytes());
    buffer.push(b'\n');
}

fn canonical_manifest_digest(
    manifest: &PackageManifest,
) -> Result<String, PackageDistributionError> {
    let bytes = serde_json::to_vec(manifest)?;
    Ok(hash_artifact(&bytes))
}

fn to_i64(value: u64) -> Result<i64, PackageDistributionError> {
    i64::try_from(value).map_err(|_| PackageDistributionError::CorruptStore)
}

fn from_i64(value: i64) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}

impl From<PackageStoreError> for PackageDistributionError {
    fn from(_: PackageStoreError) -> Self {
        Self::Database
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PackageStore;
    use crate::test_support::new_temp_dir;
    use ed25519_dalek::{Signer, SigningKey};
    use pandora_types::{PackageCompatibility, PackageDependency, TrustEvidence};

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn signed_manifest(
        id: &str,
        version: &str,
        kind: PackageKind,
        artifact: &[u8],
        publisher: &str,
        signing_key: &SigningKey,
        dependencies: Vec<PackageDependency>,
    ) -> PackageManifest {
        let unsigned = PackageManifest::new(
            id,
            version,
            kind,
            publisher,
            hash_artifact(artifact),
            dependencies.clone(),
            PackageCompatibility::new(concat!("pandora>=", env!("CARGO_PKG_VERSION"))).unwrap(),
            "MIT",
            TrustEvidence::unsigned(),
        )
        .unwrap();
        let signature = signing_key.sign(unsigned.signing_message().as_bytes());
        PackageManifest::new(
            id,
            version,
            kind,
            publisher,
            hash_artifact(artifact),
            dependencies,
            PackageCompatibility::new(concat!("pandora>=", env!("CARGO_PKG_VERSION"))).unwrap(),
            "MIT",
            TrustEvidence::new(
                TrustLevel::Official,
                Some(hex(&signature.to_bytes())),
                Some(hex(&signing_key.verifying_key().to_bytes())),
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn stores(
        prefix: &str,
        publisher: &str,
        key_id: &str,
        signing_key: &SigningKey,
    ) -> (PathBuf, PackageStore, PackageDistributionStore) {
        let root = new_temp_dir(prefix).unwrap();
        let database = root.join("packages.sqlite3");
        let packages = PackageStore::open(&database).unwrap();
        packages
            .add_publisher_trust_root(
                publisher,
                key_id,
                hex(&signing_key.verifying_key().to_bytes()),
                1,
            )
            .unwrap();
        let distribution = PackageDistributionStore::open(database).unwrap();
        (root, packages, distribution)
    }

    #[test]
    fn verified_download_stays_cached_and_inert_until_explicit_admission() {
        let signing_key = SigningKey::from_bytes(&[71_u8; 32]);
        let (root, packages, distribution) = stores(
            "pandora-distribution-cache",
            "publisher",
            "key-1",
            &signing_key,
        );
        let artifact = br#"{"id":"provider","name":"Provider","protocol":"open_ai_compatible","base_url":"https://provider.example/v1","default_model":"model","api_key_env":"PANDORA_PROVIDER_KEY"}"#;
        let manifest = signed_manifest(
            "publisher/provider",
            "1.0.0",
            PackageKind::Provider,
            artifact,
            "publisher",
            &signing_key,
            Vec::new(),
        );
        let source = DistributionSource::new(
            DistributionSourceKind::Registry,
            "https://registry.example",
            "1.0.0",
        )
        .unwrap();

        let cached = distribution
            .cache_verified(&manifest, artifact, source.clone(), 2)
            .unwrap();
        assert!(cached.changed());
        assert_eq!(cached.record().state(), DistributionState::Cached);
        assert!(distribution.binding(manifest.id()).unwrap().is_none());
        assert!(packages.list().unwrap().is_empty());

        let replay = distribution
            .cache_verified(&manifest, artifact, source, 3)
            .unwrap();
        assert!(!replay.changed());
        assert_eq!(distribution.list_events(16).unwrap().len(), 1);

        distribution
            .verify_offline(manifest.id(), manifest.version(), 4)
            .unwrap();
        let prepared = distribution
            .prepare_admission(manifest.id(), manifest.version())
            .unwrap();
        assert_eq!(prepared.state(), DistributionState::Cached);
        let admitted = distribution
            .record_admission(
                manifest.id(),
                manifest.version(),
                manifest.content_hash(),
                5,
            )
            .unwrap();
        assert!(admitted.changed());
        assert_eq!(admitted.record().state(), DistributionState::Admitted);
        assert_eq!(distribution.list_events(16).unwrap().len(), 3);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn untrusted_substituted_and_revoked_publishers_fail_closed() {
        let signing_key = SigningKey::from_bytes(&[73_u8; 32]);
        let substituted_key = SigningKey::from_bytes(&[79_u8; 32]);
        let (root, packages, distribution) = stores(
            "pandora-distribution-trust",
            "publisher",
            "key-1",
            &signing_key,
        );
        packages
            .add_publisher_trust_root(
                "publisher",
                "key-2",
                hex(&substituted_key.verifying_key().to_bytes()),
                2,
            )
            .unwrap();
        let artifact = b"provider manifest";
        let signed = signed_manifest(
            "publisher/provider",
            "1.0.0",
            PackageKind::Provider,
            artifact,
            "publisher",
            &signing_key,
            Vec::new(),
        );
        let untrusted = PackageManifest::new(
            "publisher/untrusted",
            "1.0.0",
            PackageKind::Skill,
            "publisher",
            hash_artifact(artifact),
            Vec::new(),
            PackageCompatibility::new(concat!("pandora>=", env!("CARGO_PKG_VERSION"))).unwrap(),
            "MIT",
            TrustEvidence::unsigned(),
        )
        .unwrap();
        let source = DistributionSource::new(
            DistributionSourceKind::Registry,
            "https://registry.example",
            "1.0.0",
        )
        .unwrap();
        assert_eq!(
            distribution.cache_verified(&untrusted, artifact, source.clone(), 3),
            Err(PackageDistributionError::UntrustedPublisher)
        );

        let signature = signing_key.sign(signed.signing_message().as_bytes());
        let substituted = PackageManifest::new(
            signed.id().as_str(),
            signed.version(),
            signed.kind(),
            signed.publisher(),
            signed.content_hash(),
            Vec::new(),
            signed.compatibility().clone(),
            signed.license(),
            TrustEvidence::new(
                TrustLevel::Official,
                Some(hex(&signature.to_bytes())),
                Some(hex(&substituted_key.verifying_key().to_bytes())),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            distribution.cache_verified(&substituted, artifact, source.clone(), 4),
            Err(PackageDistributionError::InvalidSignature)
        );

        distribution
            .cache_verified(&signed, artifact, source, 5)
            .unwrap();
        packages
            .revoke_publisher_trust_root("publisher", "key-1", 6)
            .unwrap();
        assert_eq!(
            distribution.verify_current(signed.id(), signed.version()),
            Err(PackageDistributionError::RevokedPublisher)
        );
        distribution
            .revoke_publisher_key("publisher", "key-1", 6)
            .unwrap();
        assert_eq!(
            distribution.prepare_admission(signed.id(), signed.version()),
            Err(PackageDistributionError::Revoked)
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn identity_replay_dependencies_and_downgrades_are_bounded() {
        let signing_key = SigningKey::from_bytes(&[83_u8; 32]);
        let (root, _packages, distribution) = stores(
            "pandora-distribution-replay",
            "publisher",
            "key-1",
            &signing_key,
        );
        let source = DistributionSource::new(
            DistributionSourceKind::Registry,
            "https://registry.example",
            "2.0.0",
        )
        .unwrap();
        let dependency = PackageDependency::new("publisher/base", "1.0.0", false).unwrap();
        let dependent_artifact = b"dependent";
        let dependent = signed_manifest(
            "publisher/provider",
            "2.0.0",
            PackageKind::Provider,
            dependent_artifact,
            "publisher",
            &signing_key,
            vec![dependency],
        );
        distribution
            .cache_verified(&dependent, dependent_artifact, source.clone(), 2)
            .unwrap();
        assert_eq!(
            distribution.prepare_admission(dependent.id(), dependent.version()),
            Err(PackageDistributionError::MissingDependency {
                id: "publisher/base".to_owned(),
                version: "1.0.0".to_owned(),
            })
        );
        assert_eq!(
            distribution.cache_verified(
                &dependent,
                dependent_artifact,
                DistributionSource::new(
                    DistributionSourceKind::Registry,
                    "https://mirror.example",
                    "2.0.0",
                )
                .unwrap(),
                3,
            ),
            Err(PackageDistributionError::IdentityConflict)
        );

        let current_artifact = b"current";
        let current = signed_manifest(
            "publisher/skill",
            "2.0.0",
            PackageKind::Skill,
            current_artifact,
            "publisher",
            &signing_key,
            Vec::new(),
        );
        distribution
            .cache_verified(&current, current_artifact, source, 4)
            .unwrap();
        distribution
            .record_admission(current.id(), current.version(), current.content_hash(), 5)
            .unwrap();
        let older_artifact = b"older";
        let older = signed_manifest(
            "publisher/skill",
            "1.0.0",
            PackageKind::Skill,
            older_artifact,
            "publisher",
            &signing_key,
            Vec::new(),
        );
        distribution
            .cache_verified(
                &older,
                older_artifact,
                DistributionSource::new(
                    DistributionSourceKind::Registry,
                    "https://registry.example",
                    "1.0.0",
                )
                .unwrap(),
                6,
            )
            .unwrap();
        assert_eq!(
            distribution.prepare_admission(older.id(), older.version()),
            Err(PackageDistributionError::DowngradeDenied {
                active: "2.0.0".to_owned(),
                requested: "1.0.0".to_owned(),
            })
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn skill_bundle_materialization_rejects_traversal_before_writing() {
        let root = new_temp_dir("pandora-distribution-bundle").unwrap();
        let artifact = br#"{"format_version":1,"files":[{"path":"SKILL.md","content":"safe"},{"path":"../escape","content":"unsafe"}]}"#;
        let staging = root.join("staging");

        assert_eq!(
            materialize_skill_bundle(artifact, &staging, "safe-skill"),
            Err(PackageDistributionError::InvalidBundle)
        );
        assert!(!staging.exists());
        assert!(!root.join("escape").exists());

        let unknown = br#"{"format_version":1,"files":[{"path":"SKILL.md","content":"safe","mode":"executable"}]}"#;
        assert_eq!(
            materialize_skill_bundle(unknown, &root.join("unknown"), "safe-skill"),
            Err(PackageDistributionError::InvalidBundle)
        );
        assert!(!root.join("unknown").exists());
        let _ = std::fs::remove_dir_all(root);
    }
}
