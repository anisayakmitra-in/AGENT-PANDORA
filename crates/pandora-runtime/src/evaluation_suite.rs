use crate::evaluation_engine::{EvaluationTarget, EvaluationTargetKind, MAX_EVALUATION_TASK_BYTES};
use pandora_types::{ExecutionId, Timestamp, hash_artifact};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::fmt;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

pub const MAX_EVALUATION_SUITES: usize = 128;
pub const MAX_EVALUATION_SUITE_ID_BYTES: usize = 256;
pub const MAX_EVALUATION_DEFINITION_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_REGRESSION_CANDIDATES: usize = 512;
pub const MAX_REGRESSION_CANDIDATE_ID_BYTES: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationSuite {
    id: String,
    digest: String,
    definition_bytes: usize,
    created_at: Timestamp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegressionCandidateStatus {
    Proposed,
    Accepted,
    Rejected,
}

impl RegressionCandidateStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegressionCandidate {
    id: String,
    case_id: String,
    source_execution_id: ExecutionId,
    target: EvaluationTarget,
    task: String,
    failure_digest: String,
    created_at: Timestamp,
    status: RegressionCandidateStatus,
    reviewed_at: Option<Timestamp>,
}

impl RegressionCandidate {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn case_id(&self) -> &str {
        &self.case_id
    }
    pub fn source_execution_id(&self) -> &ExecutionId {
        &self.source_execution_id
    }
    pub fn target(&self) -> &EvaluationTarget {
        &self.target
    }
    pub fn task(&self) -> &str {
        &self.task
    }
    pub fn failure_digest(&self) -> &str {
        &self.failure_digest
    }
    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }
    pub const fn status(&self) -> RegressionCandidateStatus {
        self.status
    }
    pub const fn reviewed_at(&self) -> Option<Timestamp> {
        self.reviewed_at
    }
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
    InvalidRegressionCandidate,
    RegressionCandidateAlreadyExists,
    RegressionCandidateNotFound,
    RegressionCandidateAlreadyReviewed,
    RegressionCandidateNotApproved,
    TooManyRegressionCandidates,
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
            Self::InvalidRegressionCandidate => {
                formatter.write_str("regression candidate is invalid")
            }
            Self::RegressionCandidateAlreadyExists => {
                formatter.write_str("regression candidate already exists")
            }
            Self::RegressionCandidateNotFound => {
                formatter.write_str("regression candidate was not found")
            }
            Self::RegressionCandidateAlreadyReviewed => {
                formatter.write_str("regression candidate was already reviewed")
            }
            Self::RegressionCandidateNotApproved => {
                formatter.write_str("regression candidate has not been approved")
            }
            Self::TooManyRegressionCandidates => {
                formatter.write_str("regression candidate capacity is exhausted")
            }
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
             );
             CREATE TABLE IF NOT EXISTS regression_candidates (
                 id TEXT PRIMARY KEY NOT NULL,
                 case_id TEXT NOT NULL,
                 source_execution_id TEXT NOT NULL,
                 target_kind TEXT NOT NULL,
                 target_id TEXT NOT NULL,
                 task TEXT NOT NULL,
                 failure_digest TEXT NOT NULL,
                 created_at INTEGER NOT NULL CHECK (created_at >= 0),
                 status TEXT NOT NULL,
                 reviewed_at INTEGER
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

    #[allow(clippy::too_many_arguments)]
    pub fn propose_regression_candidate(
        &self,
        id: impl Into<String>,
        case_id: impl Into<String>,
        source_execution_id: ExecutionId,
        target: EvaluationTarget,
        task: impl Into<String>,
        failure_digest: impl Into<String>,
        created_at: Timestamp,
    ) -> Result<RegressionCandidate, EvaluationSuiteError> {
        let id = validate_candidate_text(id.into())?;
        let case_id = validate_candidate_text(case_id.into())?;
        let task = validate_task(task.into())?;
        let failure_digest = validate_candidate_text(failure_digest.into())?;
        let created_at_seconds = i64::try_from(created_at.as_unix_seconds())
            .map_err(|_| EvaluationSuiteError::InvalidRegressionCandidate)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let count =
            transaction.query_row("SELECT COUNT(*) FROM regression_candidates", [], |row| {
                row.get::<_, i64>(0)
            })?;
        if usize::try_from(count).map_err(|_| EvaluationSuiteError::InvalidRegressionCandidate)?
            >= MAX_REGRESSION_CANDIDATES
        {
            return Err(EvaluationSuiteError::TooManyRegressionCandidates);
        }
        let result = transaction.execute(
            "INSERT INTO regression_candidates
             (id, case_id, source_execution_id, target_kind, target_id, task, failure_digest, created_at, status, reviewed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL)",
            params![
                id,
                case_id,
                source_execution_id.as_str(),
                target.kind().as_str(),
                target.id(),
                task,
                failure_digest,
                created_at_seconds,
                RegressionCandidateStatus::Proposed.as_str(),
            ],
        );
        match result {
            Ok(_) => {
                transaction.commit()?;
                Ok(RegressionCandidate {
                    id,
                    case_id,
                    source_execution_id,
                    target,
                    task,
                    failure_digest,
                    created_at,
                    status: RegressionCandidateStatus::Proposed,
                    reviewed_at: None,
                })
            }
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(EvaluationSuiteError::RegressionCandidateAlreadyExists)
            }
            Err(error) => Err(EvaluationSuiteError::Database(error)),
        }
    }

    pub fn list_regression_candidates(
        &self,
    ) -> Result<Vec<RegressionCandidate>, EvaluationSuiteError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id, case_id, source_execution_id, target_kind, target_id, task,
                    failure_digest, created_at, status, reviewed_at
             FROM regression_candidates ORDER BY created_at ASC, id ASC",
        )?;
        let rows = statement.query_map([], decode_candidate)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn inspect_regression_candidate(
        &self,
        id: &str,
    ) -> Result<RegressionCandidate, EvaluationSuiteError> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT id, case_id, source_execution_id, target_kind, target_id, task,
                        failure_digest, created_at, status, reviewed_at
                 FROM regression_candidates WHERE id = ?1",
                params![id],
                decode_candidate,
            )
            .optional()?
            .ok_or(EvaluationSuiteError::RegressionCandidateNotFound)
    }

    pub fn review_regression_candidate(
        &self,
        id: &str,
        accepted: bool,
        reviewed_at: Timestamp,
    ) -> Result<RegressionCandidate, EvaluationSuiteError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = transaction
            .query_row(
                "SELECT id, case_id, source_execution_id, target_kind, target_id, task,
                        failure_digest, created_at, status, reviewed_at
                 FROM regression_candidates WHERE id = ?1",
                params![id],
                decode_candidate,
            )
            .optional()?
            .ok_or(EvaluationSuiteError::RegressionCandidateNotFound)?;
        if current.status != RegressionCandidateStatus::Proposed {
            return Err(EvaluationSuiteError::RegressionCandidateAlreadyReviewed);
        }
        let reviewed_at_seconds = i64::try_from(reviewed_at.as_unix_seconds())
            .map_err(|_| EvaluationSuiteError::InvalidRegressionCandidate)?;
        let status = if accepted {
            RegressionCandidateStatus::Accepted
        } else {
            RegressionCandidateStatus::Rejected
        };
        transaction.execute(
            "UPDATE regression_candidates SET status = ?1, reviewed_at = ?2 WHERE id = ?3",
            params![status.as_str(), reviewed_at_seconds, id],
        )?;
        transaction.commit()?;
        Ok(RegressionCandidate {
            status,
            reviewed_at: Some(reviewed_at),
            ..current
        })
    }

    pub fn require_approved_regression_candidate(
        &self,
        id: &str,
    ) -> Result<RegressionCandidate, EvaluationSuiteError> {
        let candidate = self.inspect_regression_candidate(id)?;
        if candidate.status != RegressionCandidateStatus::Accepted {
            return Err(EvaluationSuiteError::RegressionCandidateNotApproved);
        }
        Ok(candidate)
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

fn validate_candidate_text(value: String) -> Result<String, EvaluationSuiteError> {
    let value = value.trim().to_owned();
    if value.is_empty()
        || value.len() > MAX_REGRESSION_CANDIDATE_ID_BYTES
        || value.chars().any(char::is_control)
    {
        Err(EvaluationSuiteError::InvalidRegressionCandidate)
    } else {
        Ok(value)
    }
}

fn validate_task(value: String) -> Result<String, EvaluationSuiteError> {
    let value = value.trim().to_owned();
    if value.is_empty()
        || value.len() > MAX_EVALUATION_TASK_BYTES
        || value.chars().any(char::is_control)
    {
        Err(EvaluationSuiteError::InvalidRegressionCandidate)
    } else {
        Ok(value)
    }
}

fn decode_candidate(row: &rusqlite::Row<'_>) -> rusqlite::Result<RegressionCandidate> {
    let target_kind = row.get::<_, String>(3)?;
    let target_kind = match target_kind.as_str() {
        "prompt" => EvaluationTargetKind::Prompt,
        "skill" => EvaluationTargetKind::Skill,
        "workflow" => EvaluationTargetKind::Workflow,
        "wasm_gene" => EvaluationTargetKind::WasmGene,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let target = EvaluationTarget::new(target_kind, row.get::<_, String>(4)?)
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    let created_at = row.get::<_, i64>(7)?;
    let status = match row.get::<_, String>(8)?.as_str() {
        "proposed" => RegressionCandidateStatus::Proposed,
        "accepted" => RegressionCandidateStatus::Accepted,
        "rejected" => RegressionCandidateStatus::Rejected,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let reviewed_at = match row.get::<_, Option<i64>>(9)? {
        Some(value) => Some(Timestamp::from_unix_seconds(
            u64::try_from(value).map_err(|_| rusqlite::Error::InvalidQuery)?,
        )),
        None => None,
    };
    Ok(RegressionCandidate {
        id: row.get(0)?,
        case_id: row.get(1)?,
        source_execution_id: ExecutionId::new(row.get::<_, String>(2)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        target,
        task: validate_task(row.get(5)?).map_err(|_| rusqlite::Error::InvalidQuery)?,
        failure_digest: row.get(6)?,
        created_at: Timestamp::from_unix_seconds(
            u64::try_from(created_at).map_err(|_| rusqlite::Error::InvalidQuery)?,
        ),
        status,
        reviewed_at,
    })
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
    fn regression_candidates_require_review_and_survive_reload() {
        let (store, directory) = store();
        let candidate = store
            .propose_regression_candidate(
                "candidate-1",
                "workflow-smoke",
                pandora_types::ExecutionId::new("execution-1").unwrap(),
                crate::evaluation_engine::EvaluationTarget::new(
                    crate::evaluation_engine::EvaluationTargetKind::Workflow,
                    "workflow-1",
                )
                .unwrap(),
                "run the bounded workflow",
                "sha256:failure-evidence",
                Timestamp::from_unix_seconds(4),
            )
            .unwrap();
        assert_eq!(candidate.status(), RegressionCandidateStatus::Proposed);
        assert_eq!(
            store.list_regression_candidates().unwrap(),
            vec![candidate.clone()]
        );
        assert!(matches!(
            store.require_approved_regression_candidate("candidate-1"),
            Err(EvaluationSuiteError::RegressionCandidateNotApproved)
        ));

        let reviewed = store
            .review_regression_candidate("candidate-1", true, Timestamp::from_unix_seconds(5))
            .unwrap();
        assert_eq!(reviewed.status(), RegressionCandidateStatus::Accepted);
        assert_eq!(
            store
                .require_approved_regression_candidate("candidate-1")
                .unwrap(),
            reviewed
        );
        assert!(matches!(
            store.review_regression_candidate(
                "candidate-1",
                false,
                Timestamp::from_unix_seconds(6)
            ),
            Err(EvaluationSuiteError::RegressionCandidateAlreadyReviewed)
        ));

        let reopened = EvaluationSuiteStore::open(directory.join("suites.sqlite3")).unwrap();
        assert_eq!(
            reopened
                .require_approved_regression_candidate("candidate-1")
                .unwrap(),
            reviewed
        );
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
