use pandora_types::{
    StorageLifecycleAction, StorageLifecycleContractError, StorageLifecycleManifest,
    StorageLifecycleProvider, Timestamp,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::fmt;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

pub const MAX_STORAGE_LIFECYCLE_RECEIPTS: usize = 4_096;
pub const MAX_STORAGE_LIFECYCLE_LIST: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageLifecycleReceipt {
    manifest: StorageLifecycleManifest,
    manifest_digest: String,
    recorded_at: Timestamp,
}

impl StorageLifecycleReceipt {
    pub fn manifest(&self) -> &StorageLifecycleManifest {
        &self.manifest
    }

    pub fn manifest_digest(&self) -> &str {
        &self.manifest_digest
    }

    pub const fn recorded_at(&self) -> Timestamp {
        self.recorded_at
    }

    pub const fn evidence_status(&self) -> &'static str {
        "operator_attested"
    }

    pub const fn external_action_performed_by_runtime(&self) -> bool {
        false
    }

    pub const fn secure_erasure_guaranteed(&self) -> bool {
        false
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageLifecycleRecordResult {
    receipt: StorageLifecycleReceipt,
    created: bool,
}

impl StorageLifecycleRecordResult {
    pub fn receipt(&self) -> &StorageLifecycleReceipt {
        &self.receipt
    }

    pub const fn created(&self) -> bool {
        self.created
    }
}

#[derive(Debug)]
pub enum StorageLifecycleStoreError {
    Database(rusqlite::Error),
    Io(std::io::Error),
    Contract(StorageLifecycleContractError),
    Serialization(serde_json::Error),
    EvidenceConflict,
    EvidenceNotFound,
    ReceiptLimitReached,
    PerformedAfterRecord,
    CorruptRecord,
    LockPoisoned,
}

impl fmt::Display for StorageLifecycleStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(error) => {
                write!(formatter, "storage lifecycle database failed: {error}")
            }
            Self::Io(error) => write!(formatter, "storage lifecycle storage failed: {error}"),
            Self::Contract(error) => error.fmt(formatter),
            Self::Serialization(error) => {
                write!(
                    formatter,
                    "storage lifecycle evidence could not be encoded: {error}"
                )
            }
            Self::EvidenceConflict => formatter
                .write_str("storage lifecycle evidence ID already exists with different content"),
            Self::EvidenceNotFound => {
                formatter.write_str("storage lifecycle evidence was not found")
            }
            Self::ReceiptLimitReached => formatter
                .write_str("storage lifecycle evidence ledger reached its bounded receipt limit"),
            Self::PerformedAfterRecord => formatter
                .write_str("storage lifecycle action time cannot be later than its receipt time"),
            Self::CorruptRecord => {
                formatter.write_str("storage lifecycle evidence ledger contains a corrupt record")
            }
            Self::LockPoisoned => {
                formatter.write_str("storage lifecycle evidence ledger lock is unavailable")
            }
        }
    }
}

impl std::error::Error for StorageLifecycleStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Contract(error) => Some(error),
            Self::Serialization(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for StorageLifecycleStoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

impl From<std::io::Error> for StorageLifecycleStoreError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<StorageLifecycleContractError> for StorageLifecycleStoreError {
    fn from(error: StorageLifecycleContractError) -> Self {
        Self::Contract(error)
    }
}

impl From<serde_json::Error> for StorageLifecycleStoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}

pub struct StorageLifecycleStore {
    connection: Mutex<Connection>,
}

impl StorageLifecycleStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageLifecycleStoreError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        set_private_permissions(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS storage_lifecycle_receipts (
                 evidence_id TEXT PRIMARY KEY NOT NULL,
                 policy_version INTEGER NOT NULL,
                 provider TEXT NOT NULL,
                 action TEXT NOT NULL,
                 resource_id TEXT NOT NULL,
                 manifest_digest TEXT NOT NULL,
                 manifest_json TEXT NOT NULL,
                 recorded_at INTEGER NOT NULL CHECK (recorded_at > 0)
             );
             CREATE INDEX IF NOT EXISTS storage_lifecycle_receipts_recorded_idx
                 ON storage_lifecycle_receipts(recorded_at DESC, evidence_id ASC);
             CREATE TRIGGER IF NOT EXISTS storage_lifecycle_receipts_no_update
                 BEFORE UPDATE ON storage_lifecycle_receipts
                 BEGIN SELECT RAISE(ABORT, 'storage lifecycle receipts are append-only'); END;
             CREATE TRIGGER IF NOT EXISTS storage_lifecycle_receipts_no_delete
                 BEFORE DELETE ON storage_lifecycle_receipts
                 BEGIN SELECT RAISE(ABORT, 'storage lifecycle receipts are append-only'); END;",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn record(
        &self,
        manifest: StorageLifecycleManifest,
        recorded_at: Timestamp,
    ) -> Result<StorageLifecycleRecordResult, StorageLifecycleStoreError> {
        manifest.validate()?;
        if recorded_at.as_unix_seconds() == 0
            || i64::try_from(recorded_at.as_unix_seconds()).is_err()
        {
            return Err(StorageLifecycleStoreError::CorruptRecord);
        }
        if manifest.performed_at() > recorded_at.as_unix_seconds() {
            return Err(StorageLifecycleStoreError::PerformedAfterRecord);
        }
        let manifest_digest = manifest.manifest_digest();
        let manifest_json = serde_json::to_string(&manifest)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = load_receipt(&transaction, manifest.evidence_id())? {
            if existing.manifest_digest == manifest_digest && existing.manifest == manifest {
                transaction.commit()?;
                return Ok(StorageLifecycleRecordResult {
                    receipt: existing,
                    created: false,
                });
            }
            return Err(StorageLifecycleStoreError::EvidenceConflict);
        }
        let count = transaction.query_row(
            "SELECT COUNT(*) FROM storage_lifecycle_receipts",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        if usize::try_from(count).unwrap_or(usize::MAX) >= MAX_STORAGE_LIFECYCLE_RECEIPTS {
            return Err(StorageLifecycleStoreError::ReceiptLimitReached);
        }
        transaction.execute(
            "INSERT INTO storage_lifecycle_receipts (
                 evidence_id, policy_version, provider, action, resource_id,
                 manifest_digest, manifest_json, recorded_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                manifest.evidence_id(),
                i64::from(manifest.policy_version()),
                manifest.provider().as_str(),
                manifest.action().as_str(),
                manifest.resource_id(),
                manifest_digest,
                manifest_json,
                to_i64(recorded_at.as_unix_seconds())?,
            ],
        )?;
        transaction.commit()?;
        Ok(StorageLifecycleRecordResult {
            receipt: StorageLifecycleReceipt {
                manifest,
                manifest_digest,
                recorded_at,
            },
            created: true,
        })
    }

    pub fn inspect(
        &self,
        evidence_id: &str,
    ) -> Result<StorageLifecycleReceipt, StorageLifecycleStoreError> {
        let connection = self.lock()?;
        load_receipt(&connection, evidence_id)?.ok_or(StorageLifecycleStoreError::EvidenceNotFound)
    }

    pub fn list(
        &self,
        provider: Option<StorageLifecycleProvider>,
        action: Option<StorageLifecycleAction>,
        limit: usize,
    ) -> Result<Vec<StorageLifecycleReceipt>, StorageLifecycleStoreError> {
        if limit == 0 || limit > MAX_STORAGE_LIFECYCLE_LIST {
            return Err(StorageLifecycleStoreError::CorruptRecord);
        }
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT manifest_digest, manifest_json, recorded_at
             FROM storage_lifecycle_receipts
             ORDER BY recorded_at DESC, evidence_id ASC",
        )?;
        let rows = statement.query_map([], decode_receipt)?;
        let mut receipts = Vec::new();
        for row in rows {
            let receipt = row?;
            if provider.is_some_and(|candidate| receipt.manifest.provider() != candidate)
                || action.is_some_and(|candidate| receipt.manifest.action() != candidate)
            {
                continue;
            }
            receipts.push(receipt);
            if receipts.len() == limit {
                break;
            }
        }
        Ok(receipts)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, StorageLifecycleStoreError> {
        self.connection
            .lock()
            .map_err(|_| StorageLifecycleStoreError::LockPoisoned)
    }
}

fn load_receipt(
    connection: &Connection,
    evidence_id: &str,
) -> Result<Option<StorageLifecycleReceipt>, StorageLifecycleStoreError> {
    connection
        .query_row(
            "SELECT manifest_digest, manifest_json, recorded_at
             FROM storage_lifecycle_receipts WHERE evidence_id = ?1",
            params![evidence_id],
            decode_receipt,
        )
        .optional()
        .map_err(Into::into)
}

fn decode_receipt(row: &rusqlite::Row<'_>) -> Result<StorageLifecycleReceipt, rusqlite::Error> {
    let manifest_digest = row.get::<_, String>(0)?;
    let manifest_json = row.get::<_, String>(1)?;
    let recorded_at = row.get::<_, i64>(2)?;
    let manifest: StorageLifecycleManifest =
        serde_json::from_str(&manifest_json).map_err(|_| rusqlite::Error::InvalidQuery)?;
    manifest
        .validate()
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    if manifest.manifest_digest() != manifest_digest {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(StorageLifecycleReceipt {
        manifest,
        manifest_digest,
        recorded_at: Timestamp::from_unix_seconds(
            to_u64(recorded_at).map_err(|_| rusqlite::Error::InvalidQuery)?,
        ),
    })
}

fn to_i64(value: u64) -> Result<i64, StorageLifecycleStoreError> {
    i64::try_from(value).map_err(|_| StorageLifecycleStoreError::CorruptRecord)
}

fn to_u64(value: i64) -> Result<u64, StorageLifecycleStoreError> {
    u64::try_from(value).map_err(|_| StorageLifecycleStoreError::CorruptRecord)
}

fn set_private_permissions(path: &Path) -> Result<(), StorageLifecycleStoreError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn database_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "pandora-storage-lifecycle-{}-{}.sqlite3",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn manifest(evidence_id: &str, digest_byte: char) -> StorageLifecycleManifest {
        StorageLifecycleManifest::new(
            evidence_id,
            "policy:daily-30d",
            StorageLifecycleProvider::AwsS3,
            StorageLifecycleAction::BackupExpired,
            "resource:daily-1",
            BTreeMap::from([
                ("bucket".to_owned(), "backup-bucket".to_owned()),
                ("deletion_marker_id".to_owned(), "marker-1".to_owned()),
                ("object_key".to_owned(), "daily/archive.json".to_owned()),
                ("version_id".to_owned(), "version-1".to_owned()),
            ]),
            format!("sha256:{}", digest_byte.to_string().repeat(64)),
            "operator:alice",
            1_788_192_000,
        )
        .unwrap()
    }

    #[test]
    fn exact_retry_is_idempotent_and_conflict_fails_closed() {
        let path = database_path();
        let store = StorageLifecycleStore::open(&path).unwrap();
        let first = store
            .record(
                manifest("evidence:1", '1'),
                Timestamp::from_unix_seconds(1_788_192_010),
            )
            .unwrap();
        assert!(first.created());
        let retry = store
            .record(
                manifest("evidence:1", '1'),
                Timestamp::from_unix_seconds(1_788_192_011),
            )
            .unwrap();
        assert!(!retry.created());
        assert_eq!(
            retry.receipt().recorded_at().as_unix_seconds(),
            1_788_192_010
        );
        assert!(matches!(
            store.record(
                manifest("evidence:1", '2'),
                Timestamp::from_unix_seconds(1_788_192_012)
            ),
            Err(StorageLifecycleStoreError::EvidenceConflict)
        ));
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn receipts_survive_reopen_and_filter_without_claiming_external_action() {
        let path = database_path();
        let store = StorageLifecycleStore::open(&path).unwrap();
        store
            .record(
                manifest("evidence:1", '1'),
                Timestamp::from_unix_seconds(1_788_192_010),
            )
            .unwrap();
        drop(store);

        let reopened = StorageLifecycleStore::open(&path).unwrap();
        let receipt = reopened.inspect("evidence:1").unwrap();
        assert_eq!(receipt.evidence_status(), "operator_attested");
        assert!(!receipt.external_action_performed_by_runtime());
        assert!(!receipt.secure_erasure_guaranteed());
        assert_eq!(
            reopened
                .list(
                    Some(StorageLifecycleProvider::AwsS3),
                    Some(StorageLifecycleAction::BackupExpired),
                    10,
                )
                .unwrap()
                .len(),
            1
        );
        drop(reopened);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn sqlite_guards_receipts_against_update_and_delete() {
        let path = database_path();
        let store = StorageLifecycleStore::open(&path).unwrap();
        store
            .record(
                manifest("evidence:1", '1'),
                Timestamp::from_unix_seconds(1_788_192_010),
            )
            .unwrap();
        drop(store);
        let connection = Connection::open(&path).unwrap();
        assert!(
            connection
                .execute(
                    "UPDATE storage_lifecycle_receipts SET resource_id = 'changed'",
                    [],
                )
                .is_err()
        );
        assert!(
            connection
                .execute("DELETE FROM storage_lifecycle_receipts", [])
                .is_err()
        );
        drop(connection);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn receipt_rejects_an_action_time_from_the_future() {
        let path = database_path();
        let store = StorageLifecycleStore::open(&path).unwrap();
        assert!(matches!(
            store.record(
                manifest("evidence:future", '1'),
                Timestamp::from_unix_seconds(1_788_191_999)
            ),
            Err(StorageLifecycleStoreError::PerformedAfterRecord)
        ));
        assert!(store.list(None, None, 10).unwrap().is_empty());
        drop(store);
        let _ = std::fs::remove_file(path);
    }
}
