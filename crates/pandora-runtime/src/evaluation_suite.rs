use pandora_types::{Timestamp, hash_artifact};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::fmt;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

pub const MAX_EVALUATION_SUITES: usize = 128;
pub const MAX_EVALUATION_SUITE_ID_BYTES: usize = 256;
pub const MAX_EVALUATION_DEFINITION_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationSuite {
    id: String,
    digest: String,
    definition_bytes: usize,
    created_at: Timestamp,
}

impl EvaluationSuite {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }

    pub const fn definition_bytes(&self) -> usize {
        self.definition_bytes
    }

    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }
}

#[derive(Debug)]
pub enum EvaluationSuiteError {
    Database(rusqlite::Error),
    Io(std::io::Error),
    InvalidId,
    InvalidDefinition,
    SuiteAlreadyExists,
    SuiteNotFound,
    TooManySuites,
    CorruptRecord,
    LockPoisoned,
}

impl fmt::Display for EvaluationSuiteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(_) => formatter.write_str("evaluation suite database operation failed"),
            Self::Io(_) => {
                formatter.write_str("evaluation suite database directory operation failed")
            }
            Self::InvalidId => formatter.write_str("evaluation suite identifier is invalid"),
            Self::InvalidDefinition => {
                formatter.write_str("evaluation suite definition is invalid")
            }
            Self::SuiteAlreadyExists => formatter.write_str("evaluation suite already exists"),
            Self::SuiteNotFound => formatter.write_str("evaluation suite was not found"),
            Self::TooManySuites => formatter.write_str("evaluation suite capacity is exhausted"),
            Self::CorruptRecord => {
                formatter.write_str("evaluation suite database contains an invalid record")
            }
            Self::LockPoisoned => {
                formatter.write_str("evaluation suite database lock is unavailable")
            }
        }
    }
}

impl std::error::Error for EvaluationSuiteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for EvaluationSuiteError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

impl From<std::io::Error> for EvaluationSuiteError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

pub struct EvaluationSuiteStore {
    connection: Mutex<Connection>,
}

impl EvaluationSuiteStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, EvaluationSuiteError> {
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
             CREATE TABLE IF NOT EXISTS evaluation_suites (
                 id TEXT PRIMARY KEY NOT NULL,
                 digest TEXT NOT NULL,
                 definition BLOB NOT NULL,
                 created_at INTEGER NOT NULL CHECK (created_at >= 0)
             );",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn register(
        &self,
        id: impl Into<String>,
        definition: impl AsRef<[u8]>,
        created_at: Timestamp,
    ) -> Result<EvaluationSuite, EvaluationSuiteError> {
        let id = validate_id(id.into())?;
        let definition = definition.as_ref();
        if definition.is_empty() || definition.len() > MAX_EVALUATION_DEFINITION_BYTES {
            return Err(EvaluationSuiteError::InvalidDefinition);
        }
        let digest = hash_artifact(definition);
        let created_at_seconds = i64::try_from(created_at.as_unix_seconds())
            .map_err(|_| EvaluationSuiteError::CorruptRecord)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let count = transaction.query_row("SELECT COUNT(*) FROM evaluation_suites", [], |row| {
            row.get::<_, i64>(0)
        })?;
        if usize::try_from(count).map_err(|_| EvaluationSuiteError::CorruptRecord)?
            >= MAX_EVALUATION_SUITES
        {
            return Err(EvaluationSuiteError::TooManySuites);
        }
        let result = transaction.execute(
            "INSERT INTO evaluation_suites (id, digest, definition, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![id, digest, definition, created_at_seconds],
        );
        match result {
            Ok(_) => {
                transaction.commit()?;
                Ok(EvaluationSuite {
                    id,
                    digest,
                    definition_bytes: definition.len(),
                    created_at,
                })
            }
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(EvaluationSuiteError::SuiteAlreadyExists)
            }
            Err(error) => Err(EvaluationSuiteError::Database(error)),
        }
    }

    pub fn list(&self) -> Result<Vec<EvaluationSuite>, EvaluationSuiteError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id, digest, length(definition), created_at
             FROM evaluation_suites ORDER BY created_at ASC, id ASC",
        )?;
        let rows = statement.query_map([], decode_suite)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn inspect(&self, id: &str) -> Result<EvaluationSuite, EvaluationSuiteError> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT id, digest, length(definition), created_at
                 FROM evaluation_suites WHERE id = ?1",
                params![id],
                decode_suite,
            )
            .optional()?
            .ok_or(EvaluationSuiteError::SuiteNotFound)
    }

    pub fn load(&self, id: &str) -> Result<Vec<u8>, EvaluationSuiteError> {
        let connection = self.lock()?;
        let (digest, definition) = connection
            .query_row(
                "SELECT digest, definition FROM evaluation_suites WHERE id = ?1",
                params![id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)),
            )
            .optional()?
            .ok_or(EvaluationSuiteError::SuiteNotFound)?;
        if definition.is_empty() || definition.len() > MAX_EVALUATION_DEFINITION_BYTES {
            return Err(EvaluationSuiteError::CorruptRecord);
        }
        if hash_artifact(&definition) != digest {
            return Err(EvaluationSuiteError::CorruptRecord);
        }
        Ok(definition)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, EvaluationSuiteError> {
        self.connection
            .lock()
            .map_err(|_| EvaluationSuiteError::LockPoisoned)
    }
}

fn validate_id(value: String) -> Result<String, EvaluationSuiteError> {
    let value = value.trim().to_owned();
    if value.is_empty()
        || value.len() > MAX_EVALUATION_SUITE_ID_BYTES
        || value.chars().any(char::is_control)
    {
        Err(EvaluationSuiteError::InvalidId)
    } else {
        Ok(value)
    }
}

fn decode_suite(row: &rusqlite::Row<'_>) -> rusqlite::Result<EvaluationSuite> {
    let definition_bytes = row.get::<_, i64>(2)?;
    let created_at = row.get::<_, i64>(3)?;
    Ok(EvaluationSuite {
        id: row.get(0)?,
        digest: row.get(1)?,
        definition_bytes: usize::try_from(definition_bytes)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        created_at: Timestamp::from_unix_seconds(
            u64::try_from(created_at).map_err(|_| rusqlite::Error::InvalidQuery)?,
        ),
    })
}

fn set_private_permissions(path: &Path) -> Result<(), EvaluationSuiteError> {
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

    fn store() -> (EvaluationSuiteStore, std::path::PathBuf) {
        let directory = crate::test_support::new_temp_dir("pandora-evaluation-suite").unwrap();
        let store = EvaluationSuiteStore::open(directory.join("suites.sqlite3")).unwrap();
        (store, directory)
    }

    #[test]
    fn registers_lists_loads_and_verifies_a_suite() {
        let (store, _directory) = store();
        let suite = store
            .register(
                "golden-default",
                br#"{"suite_id":"golden-default","cases":[]}"#,
                Timestamp::from_unix_seconds(4),
            )
            .unwrap();
        assert_eq!(suite.id(), "golden-default");
        assert_eq!(store.list().unwrap(), vec![suite.clone()]);
        assert_eq!(
            store.load("golden-default").unwrap(),
            br#"{"suite_id":"golden-default","cases":[]}"#
        );
        assert_eq!(store.inspect("golden-default").unwrap(), suite);
    }

    #[test]
    fn rejects_duplicates_invalid_definitions_and_missing_suites() {
        let (store, _directory) = store();
        assert!(matches!(
            store.register("", b"{}", Timestamp::from_unix_seconds(1)),
            Err(EvaluationSuiteError::InvalidId)
        ));
        assert!(matches!(
            store.register("suite", b"", Timestamp::from_unix_seconds(1)),
            Err(EvaluationSuiteError::InvalidDefinition)
        ));
        store
            .register("suite", b"{}", Timestamp::from_unix_seconds(1))
            .unwrap();
        assert!(matches!(
            store.register("suite", b"{}", Timestamp::from_unix_seconds(1)),
            Err(EvaluationSuiteError::SuiteAlreadyExists)
        ));
        assert!(matches!(
            store.load("missing"),
            Err(EvaluationSuiteError::SuiteNotFound)
        ));
    }
}
