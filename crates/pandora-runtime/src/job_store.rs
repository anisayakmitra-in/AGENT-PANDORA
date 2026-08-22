use pandora_types::{
    JobContractError, JobId, JobRequest, JobStatus, PrincipalId, TenantId, Timestamp, WorkspaceId,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::Value;
use std::fmt;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

pub const MAX_JOB_RESULT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobRecord {
    id: JobId,
    principal_id: PrincipalId,
    tenant_id: TenantId,
    workspace_id: WorkspaceId,
    request: JobRequest,
    status: JobStatus,
    created_at: Timestamp,
    started_at: Option<Timestamp>,
    finished_at: Option<Timestamp>,
    result: Option<Value>,
}

impl JobRecord {
    pub fn id(&self) -> &JobId {
        &self.id
    }

    pub fn principal_id(&self) -> &PrincipalId {
        &self.principal_id
    }

    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    pub fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }

    pub fn request(&self) -> &JobRequest {
        &self.request
    }

    pub const fn status(&self) -> JobStatus {
        self.status
    }

    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }

    pub const fn started_at(&self) -> Option<Timestamp> {
        self.started_at
    }

    pub const fn finished_at(&self) -> Option<Timestamp> {
        self.finished_at
    }

    pub fn result(&self) -> Option<&Value> {
        self.result.as_ref()
    }
}

#[derive(Debug)]
pub enum JobStoreError {
    Database(rusqlite::Error),
    Io(std::io::Error),
    Serialization(serde_json::Error),
    Contract(JobContractError),
    CorruptRecord,
    JobAlreadyExists,
    JobNotFound,
    ResultTooLarge,
    InvalidTransition {
        status: JobStatus,
        action: &'static str,
    },
    LockPoisoned,
}

impl fmt::Display for JobStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(_) => formatter.write_str("job database operation failed"),
            Self::Io(_) => formatter.write_str("job database directory operation failed"),
            Self::Serialization(_) => formatter.write_str("job record is invalid"),
            Self::Contract(error) => error.fmt(formatter),
            Self::CorruptRecord => formatter.write_str("job database contains an invalid record"),
            Self::JobAlreadyExists => formatter.write_str("job already exists"),
            Self::JobNotFound => formatter.write_str("job was not found"),
            Self::ResultTooLarge => formatter.write_str("job result exceeds the size limit"),
            Self::InvalidTransition { status, action } => {
                write!(formatter, "cannot {action} a {} job", status.as_str())
            }
            Self::LockPoisoned => formatter.write_str("job database lock is unavailable"),
        }
    }
}

impl std::error::Error for JobStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Serialization(error) => Some(error),
            Self::Contract(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for JobStoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

impl From<std::io::Error> for JobStoreError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for JobStoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}

impl From<JobContractError> for JobStoreError {
    fn from(error: JobContractError) -> Self {
        Self::Contract(error)
    }
}

pub struct JobStore {
    connection: Mutex<Connection>,
}

impl JobStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, JobStoreError> {
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
             CREATE TABLE IF NOT EXISTS jobs (
                 id TEXT PRIMARY KEY,
                 principal_id TEXT NOT NULL,
                 tenant_id TEXT NOT NULL,
                 workspace_id TEXT NOT NULL,
                 request_json TEXT NOT NULL,
                 status TEXT NOT NULL CHECK (
                     status IN ('queued', 'running', 'completed', 'approval_required', 'failed', 'cancelled')
                 ),
                 created_at INTEGER NOT NULL,
                 started_at INTEGER,
                 finished_at INTEGER,
                 result_json TEXT
             );
             CREATE INDEX IF NOT EXISTS jobs_scope_queue_idx
                 ON jobs(principal_id, tenant_id, workspace_id, status, created_at, id);",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn submit(
        &self,
        id: &JobId,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
        request: &JobRequest,
        created_at: Timestamp,
    ) -> Result<JobRecord, JobStoreError> {
        let request_json = serde_json::to_string(request)?;
        let connection = self.lock()?;
        let result = connection.execute(
            "INSERT INTO jobs (
                 id, principal_id, tenant_id, workspace_id, request_json, status, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'queued', ?6)",
            params![
                id.as_str(),
                principal_id.as_str(),
                tenant_id.as_str(),
                workspace_id.as_str(),
                request_json,
                to_i64(created_at.as_unix_seconds())?,
            ],
        );
        match result {
            Ok(_) => Ok(JobRecord {
                id: id.clone(),
                principal_id: principal_id.clone(),
                tenant_id: tenant_id.clone(),
                workspace_id: workspace_id.clone(),
                request: request.clone(),
                status: JobStatus::Queued,
                created_at,
                started_at: None,
                finished_at: None,
                result: None,
            }),
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(JobStoreError::JobAlreadyExists)
            }
            Err(error) => Err(JobStoreError::Database(error)),
        }
    }

    pub fn list(
        &self,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<JobRecord>, JobStoreError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id, principal_id, tenant_id, workspace_id, request_json, status,
                    created_at, started_at, finished_at, result_json
             FROM jobs
             WHERE principal_id = ?1 AND tenant_id = ?2 AND workspace_id = ?3
             ORDER BY created_at DESC, id DESC",
        )?;
        let mut rows = statement.query(params![
            principal_id.as_str(),
            tenant_id.as_str(),
            workspace_id.as_str(),
        ])?;
        let mut jobs = Vec::new();
        while let Some(row) = rows.next()? {
            jobs.push(decode_job(row)?);
        }
        Ok(jobs)
    }

    pub fn inspect(
        &self,
        id: &JobId,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
    ) -> Result<JobRecord, JobStoreError> {
        let connection = self.lock()?;
        load_scoped_job(&connection, id, principal_id, tenant_id, workspace_id)?
            .ok_or(JobStoreError::JobNotFound)
    }

    pub fn claim_next(
        &self,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
        started_at: Timestamp,
    ) -> Result<Option<JobRecord>, JobStoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = transaction
            .query_row(
                "SELECT id FROM jobs
                 WHERE principal_id = ?1 AND tenant_id = ?2 AND workspace_id = ?3
                   AND status = 'queued'
                 ORDER BY created_at ASC, id ASC LIMIT 1",
                params![
                    principal_id.as_str(),
                    tenant_id.as_str(),
                    workspace_id.as_str(),
                ],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(id) = id else {
            transaction.commit()?;
            return Ok(None);
        };
        let id = JobId::new(id).map_err(|_| JobStoreError::CorruptRecord)?;
        let changed = transaction.execute(
            "UPDATE jobs SET status = 'running', started_at = ?1
             WHERE id = ?2 AND principal_id = ?3 AND tenant_id = ?4 AND workspace_id = ?5
               AND status = 'queued'",
            params![
                to_i64(started_at.as_unix_seconds())?,
                id.as_str(),
                principal_id.as_str(),
                tenant_id.as_str(),
                workspace_id.as_str(),
            ],
        )?;
        if changed != 1 {
            return Err(JobStoreError::CorruptRecord);
        }
        let mut job = load_scoped_job(&transaction, &id, principal_id, tenant_id, workspace_id)?
            .ok_or(JobStoreError::CorruptRecord)?;
        transaction.commit()?;
        job.status = JobStatus::Running;
        job.started_at = Some(started_at);
        Ok(Some(job))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn finish(
        &self,
        id: &JobId,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
        status: JobStatus,
        result: &Value,
        finished_at: Timestamp,
    ) -> Result<JobRecord, JobStoreError> {
        if !matches!(
            status,
            JobStatus::Completed | JobStatus::ApprovalRequired | JobStatus::Failed
        ) {
            return Err(JobStoreError::InvalidTransition {
                status,
                action: "finish as a non-terminal outcome",
            });
        }
        let result_json = serde_json::to_vec(result)?;
        if result_json.len() > MAX_JOB_RESULT_BYTES {
            return Err(JobStoreError::ResultTooLarge);
        }
        let result_json =
            String::from_utf8(result_json).map_err(|_| JobStoreError::CorruptRecord)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = load_scoped_job(&transaction, id, principal_id, tenant_id, workspace_id)?
            .ok_or(JobStoreError::JobNotFound)?;
        if current.status() != JobStatus::Running {
            return Err(JobStoreError::InvalidTransition {
                status: current.status(),
                action: "finish",
            });
        }
        transaction.execute(
            "UPDATE jobs SET status = ?1, finished_at = ?2, result_json = ?3
             WHERE id = ?4 AND principal_id = ?5 AND tenant_id = ?6 AND workspace_id = ?7
               AND status = 'running'",
            params![
                status.as_str(),
                to_i64(finished_at.as_unix_seconds())?,
                result_json,
                id.as_str(),
                principal_id.as_str(),
                tenant_id.as_str(),
                workspace_id.as_str(),
            ],
        )?;
        transaction.commit()?;
        Ok(JobRecord {
            status,
            finished_at: Some(finished_at),
            result: Some(result.clone()),
            ..current
        })
    }

    pub fn cancel(
        &self,
        id: &JobId,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
        finished_at: Timestamp,
    ) -> Result<JobRecord, JobStoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = load_scoped_job(&transaction, id, principal_id, tenant_id, workspace_id)?
            .ok_or(JobStoreError::JobNotFound)?;
        if current.status() != JobStatus::Queued {
            return Err(JobStoreError::InvalidTransition {
                status: current.status(),
                action: "cancel",
            });
        }
        transaction.execute(
            "UPDATE jobs SET status = 'cancelled', finished_at = ?1
             WHERE id = ?2 AND principal_id = ?3 AND tenant_id = ?4 AND workspace_id = ?5
               AND status = 'queued'",
            params![
                to_i64(finished_at.as_unix_seconds())?,
                id.as_str(),
                principal_id.as_str(),
                tenant_id.as_str(),
                workspace_id.as_str(),
            ],
        )?;
        transaction.commit()?;
        Ok(JobRecord {
            status: JobStatus::Cancelled,
            finished_at: Some(finished_at),
            ..current
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, JobStoreError> {
        self.connection
            .lock()
            .map_err(|_| JobStoreError::LockPoisoned)
    }
}

fn load_scoped_job(
    connection: &Connection,
    id: &JobId,
    principal_id: &PrincipalId,
    tenant_id: &TenantId,
    workspace_id: &WorkspaceId,
) -> Result<Option<JobRecord>, JobStoreError> {
    let mut statement = connection.prepare(
        "SELECT id, principal_id, tenant_id, workspace_id, request_json, status,
                created_at, started_at, finished_at, result_json
         FROM jobs
         WHERE id = ?1 AND principal_id = ?2 AND tenant_id = ?3 AND workspace_id = ?4",
    )?;
    let mut rows = statement.query(params![
        id.as_str(),
        principal_id.as_str(),
        tenant_id.as_str(),
        workspace_id.as_str(),
    ])?;
    rows.next()?.map(decode_job).transpose()
}

fn decode_job(row: &rusqlite::Row<'_>) -> Result<JobRecord, JobStoreError> {
    let id = JobId::new(row.get::<_, String>(0)?).map_err(|_| JobStoreError::CorruptRecord)?;
    let principal_id =
        PrincipalId::new(row.get::<_, String>(1)?).map_err(|_| JobStoreError::CorruptRecord)?;
    let tenant_id =
        TenantId::new(row.get::<_, String>(2)?).map_err(|_| JobStoreError::CorruptRecord)?;
    let workspace_id =
        WorkspaceId::new(row.get::<_, String>(3)?).map_err(|_| JobStoreError::CorruptRecord)?;
    let request = serde_json::from_str::<JobRequest>(&row.get::<_, String>(4)?)?;
    let status = decode_status(&row.get::<_, String>(5)?)?;
    let created_at = decode_timestamp(row.get::<_, i64>(6)?)?;
    let started_at = row
        .get::<_, Option<i64>>(7)?
        .map(decode_timestamp)
        .transpose()?;
    let finished_at = row
        .get::<_, Option<i64>>(8)?
        .map(decode_timestamp)
        .transpose()?;
    let result = row
        .get::<_, Option<String>>(9)?
        .map(|value| serde_json::from_str(&value))
        .transpose()?;
    validate_record(status, started_at, finished_at, result.as_ref())?;
    Ok(JobRecord {
        id,
        principal_id,
        tenant_id,
        workspace_id,
        request,
        status,
        created_at,
        started_at,
        finished_at,
        result,
    })
}

fn validate_record(
    status: JobStatus,
    started_at: Option<Timestamp>,
    finished_at: Option<Timestamp>,
    result: Option<&Value>,
) -> Result<(), JobStoreError> {
    let valid = match status {
        JobStatus::Queued => started_at.is_none() && finished_at.is_none() && result.is_none(),
        JobStatus::Running => started_at.is_some() && finished_at.is_none() && result.is_none(),
        JobStatus::Cancelled => started_at.is_none() && finished_at.is_some() && result.is_none(),
        JobStatus::Completed | JobStatus::ApprovalRequired | JobStatus::Failed => {
            started_at.is_some() && finished_at.is_some() && result.is_some()
        }
    };
    if valid {
        Ok(())
    } else {
        Err(JobStoreError::CorruptRecord)
    }
}

fn decode_status(value: &str) -> Result<JobStatus, JobStoreError> {
    match value {
        "queued" => Ok(JobStatus::Queued),
        "running" => Ok(JobStatus::Running),
        "completed" => Ok(JobStatus::Completed),
        "approval_required" => Ok(JobStatus::ApprovalRequired),
        "failed" => Ok(JobStatus::Failed),
        "cancelled" => Ok(JobStatus::Cancelled),
        _ => Err(JobStoreError::CorruptRecord),
    }
}

fn to_i64(value: u64) -> Result<i64, JobStoreError> {
    i64::try_from(value).map_err(|_| JobStoreError::CorruptRecord)
}

fn decode_timestamp(value: i64) -> Result<Timestamp, JobStoreError> {
    u64::try_from(value)
        .map(Timestamp::from_unix_seconds)
        .map_err(|_| JobStoreError::CorruptRecord)
}

fn set_private_permissions(path: &Path) -> Result<(), JobStoreError> {
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
    use pandora_types::{
        JobCommand, JobId, JobRequest, PrincipalId, TenantId, Timestamp, WorkspaceId,
    };
    use serde_json::json;

    fn scope() -> (PrincipalId, TenantId, WorkspaceId) {
        (
            PrincipalId::new("principal-1").unwrap(),
            TenantId::new("tenant-1").unwrap(),
            WorkspaceId::new("workspace-1").unwrap(),
        )
    }

    fn request(task: &str) -> JobRequest {
        JobRequest::new(JobCommand::Run, vec![task.to_owned()]).unwrap()
    }

    #[test]
    fn queued_jobs_are_scoped_and_claimed_fifo_once() {
        let root = crate::test_support::new_temp_dir("pandora-job-queue").unwrap();
        let store = JobStore::open(root.join("jobs.sqlite3")).unwrap();
        let (principal, tenant, workspace) = scope();
        let first_id = JobId::new("job-first").unwrap();
        let second_id = JobId::new("job-second").unwrap();
        store
            .submit(
                &first_id,
                &principal,
                &tenant,
                &workspace,
                &request("first task"),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();
        store
            .submit(
                &second_id,
                &principal,
                &tenant,
                &workspace,
                &request("second task"),
                Timestamp::from_unix_seconds(20),
            )
            .unwrap();

        let claimed = store
            .claim_next(
                &principal,
                &tenant,
                &workspace,
                Timestamp::from_unix_seconds(30),
            )
            .unwrap()
            .unwrap();
        assert_eq!(claimed.id(), &first_id);
        assert_eq!(claimed.status(), JobStatus::Running);
        assert_eq!(claimed.started_at(), Some(Timestamp::from_unix_seconds(30)));
        assert_eq!(
            store
                .claim_next(
                    &principal,
                    &tenant,
                    &workspace,
                    Timestamp::from_unix_seconds(31),
                )
                .unwrap()
                .unwrap()
                .id(),
            &second_id
        );
        assert!(
            store
                .claim_next(
                    &principal,
                    &tenant,
                    &workspace,
                    Timestamp::from_unix_seconds(32),
                )
                .unwrap()
                .is_none()
        );

        let foreign_workspace = WorkspaceId::new("workspace-2").unwrap();
        assert!(matches!(
            store.inspect(&first_id, &principal, &tenant, &foreign_workspace),
            Err(JobStoreError::JobNotFound)
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn running_job_finishes_once_with_a_bounded_result() {
        let root = crate::test_support::new_temp_dir("pandora-job-finish").unwrap();
        let store = JobStore::open(root.join("jobs.sqlite3")).unwrap();
        let (principal, tenant, workspace) = scope();
        let id = JobId::new("job-finish").unwrap();
        store
            .submit(
                &id,
                &principal,
                &tenant,
                &workspace,
                &request("task"),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();
        store
            .claim_next(
                &principal,
                &tenant,
                &workspace,
                Timestamp::from_unix_seconds(20),
            )
            .unwrap();

        let finished = store
            .finish(
                &id,
                &principal,
                &tenant,
                &workspace,
                JobStatus::Completed,
                &json!({"command": "run", "status": "completed"}),
                Timestamp::from_unix_seconds(30),
            )
            .unwrap();
        assert_eq!(finished.status(), JobStatus::Completed);
        assert_eq!(
            finished.finished_at(),
            Some(Timestamp::from_unix_seconds(30))
        );
        assert_eq!(
            finished.result(),
            Some(&json!({"command": "run", "status": "completed"}))
        );
        assert!(matches!(
            store.finish(
                &id,
                &principal,
                &tenant,
                &workspace,
                JobStatus::Failed,
                &json!({"code": "late"}),
                Timestamp::from_unix_seconds(31),
            ),
            Err(JobStoreError::InvalidTransition { .. })
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cancellation_is_terminal_and_only_allowed_while_queued() {
        let root = crate::test_support::new_temp_dir("pandora-job-cancel").unwrap();
        let store = JobStore::open(root.join("jobs.sqlite3")).unwrap();
        let (principal, tenant, workspace) = scope();
        let queued_id = JobId::new("job-queued").unwrap();
        let running_id = JobId::new("job-running").unwrap();
        for (id, created_at) in [(&running_id, 10), (&queued_id, 11)] {
            store
                .submit(
                    id,
                    &principal,
                    &tenant,
                    &workspace,
                    &request(id.as_str()),
                    Timestamp::from_unix_seconds(created_at),
                )
                .unwrap();
        }
        store
            .claim_next(
                &principal,
                &tenant,
                &workspace,
                Timestamp::from_unix_seconds(20),
            )
            .unwrap();

        assert!(matches!(
            store.cancel(
                &running_id,
                &principal,
                &tenant,
                &workspace,
                Timestamp::from_unix_seconds(30)
            ),
            Err(JobStoreError::InvalidTransition { .. })
        ));
        let cancelled = store
            .cancel(
                &queued_id,
                &principal,
                &tenant,
                &workspace,
                Timestamp::from_unix_seconds(30),
            )
            .unwrap();
        assert_eq!(cancelled.status(), JobStatus::Cancelled);
        assert!(cancelled.status().is_terminal());
        assert!(matches!(
            store.cancel(
                &queued_id,
                &principal,
                &tenant,
                &workspace,
                Timestamp::from_unix_seconds(31)
            ),
            Err(JobStoreError::InvalidTransition { .. })
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn malformed_persisted_state_is_reported_as_corruption() {
        let root = crate::test_support::new_temp_dir("pandora-job-corrupt").unwrap();
        let database_path = root.join("jobs.sqlite3");
        let store = JobStore::open(&database_path).unwrap();
        let (principal, tenant, workspace) = scope();
        let id = JobId::new("job-corrupt").unwrap();
        store
            .submit(
                &id,
                &principal,
                &tenant,
                &workspace,
                &request("task"),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();
        let connection = rusqlite::Connection::open(&database_path).unwrap();
        connection
            .execute(
                "UPDATE jobs SET started_at = 11 WHERE id = ?1",
                rusqlite::params![id.as_str()],
            )
            .unwrap();

        assert!(matches!(
            store.inspect(&id, &principal, &tenant, &workspace),
            Err(JobStoreError::CorruptRecord)
        ));
        let _ = std::fs::remove_dir_all(root);
    }
}
