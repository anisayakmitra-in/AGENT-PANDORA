use pandora_types::{JobWorkerId, PrincipalId, RunLoopId, TenantId, Timestamp, WorkspaceId};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::fmt;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

pub const MAX_SCHEDULES: usize = 128;
pub const MAX_CLAIM_BATCH: usize = 16;
pub const MAX_SCHEDULE_NAME_BYTES: usize = 128;
pub const MAX_EVALUATION_SUITE_BYTES: usize = 256;
pub const SCHEDULE_LEASE_SECONDS: u64 = 300;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationSchedule {
    id: RunLoopId,
    principal_id: PrincipalId,
    tenant_id: TenantId,
    workspace_id: WorkspaceId,
    name: String,
    suite_id: String,
    interval_seconds: u64,
    next_run_at: Timestamp,
    enabled: bool,
    created_at: Timestamp,
    last_claimed_at: Option<Timestamp>,
    run_count: u64,
}

impl EvaluationSchedule {
    pub fn id(&self) -> &RunLoopId {
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
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn suite_id(&self) -> &str {
        &self.suite_id
    }
    pub const fn interval_seconds(&self) -> u64 {
        self.interval_seconds
    }
    pub const fn next_run_at(&self) -> Timestamp {
        self.next_run_at
    }
    pub const fn enabled(&self) -> bool {
        self.enabled
    }
    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }
    pub const fn last_claimed_at(&self) -> Option<Timestamp> {
        self.last_claimed_at
    }
    pub const fn run_count(&self) -> u64 {
        self.run_count
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationScheduleRun {
    schedule_id: RunLoopId,
    suite_id: String,
    scheduled_for: Timestamp,
    status: EvaluationScheduleRunStatus,
    worker_id: Option<JobWorkerId>,
    claimed_at: Option<Timestamp>,
    lease_until: Option<Timestamp>,
    finished_at: Option<Timestamp>,
}

impl EvaluationScheduleRun {
    pub fn schedule_id(&self) -> &RunLoopId {
        &self.schedule_id
    }
    pub fn suite_id(&self) -> &str {
        &self.suite_id
    }
    pub const fn scheduled_for(&self) -> Timestamp {
        self.scheduled_for
    }
    pub const fn status(&self) -> EvaluationScheduleRunStatus {
        self.status
    }
    pub fn worker_id(&self) -> Option<&JobWorkerId> {
        self.worker_id.as_ref()
    }
    pub const fn claimed_at(&self) -> Option<Timestamp> {
        self.claimed_at
    }
    pub const fn lease_until(&self) -> Option<Timestamp> {
        self.lease_until
    }
    pub const fn finished_at(&self) -> Option<Timestamp> {
        self.finished_at
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvaluationScheduleRunStatus {
    Pending,
    Claimed,
    Completed,
    Failed,
}

impl EvaluationScheduleRunStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Claimed => "claimed",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug)]
pub enum EvaluationScheduleError {
    Database(rusqlite::Error),
    Io(std::io::Error),
    InvalidName,
    InvalidSuite,
    InvalidInterval,
    InvalidLimit,
    InvalidWorker,
    CorruptRecord,
    ScheduleAlreadyExists,
    ScheduleNotFound,
    RunNotFound,
    RunOwnedByAnotherWorker,
    InvalidTransition {
        status: EvaluationScheduleRunStatus,
        action: &'static str,
    },
    LockPoisoned,
}

impl fmt::Display for EvaluationScheduleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(_) => {
                formatter.write_str("evaluation schedule database operation failed")
            }
            Self::Io(_) => {
                formatter.write_str("evaluation schedule database directory operation failed")
            }
            Self::InvalidName => formatter.write_str("evaluation schedule name is invalid"),
            Self::InvalidSuite => formatter.write_str("evaluation suite identifier is invalid"),
            Self::InvalidInterval => formatter.write_str("evaluation schedule interval is invalid"),
            Self::InvalidLimit => formatter.write_str("evaluation schedule claim limit is invalid"),
            Self::InvalidWorker => {
                formatter.write_str("evaluation schedule worker identifier is invalid")
            }
            Self::CorruptRecord => {
                formatter.write_str("evaluation schedule database contains an invalid record")
            }
            Self::ScheduleAlreadyExists => {
                formatter.write_str("evaluation schedule already exists")
            }
            Self::ScheduleNotFound => formatter.write_str("evaluation schedule was not found"),
            Self::RunNotFound => formatter.write_str("evaluation schedule run was not found"),
            Self::RunOwnedByAnotherWorker => {
                formatter.write_str("evaluation schedule run is owned by another worker")
            }
            Self::InvalidTransition { status, action } => write!(
                formatter,
                "cannot {action} a {} evaluation schedule run",
                status.as_str()
            ),
            Self::LockPoisoned => {
                formatter.write_str("evaluation schedule database lock is unavailable")
            }
        }
    }
}

impl std::error::Error for EvaluationScheduleError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}
impl From<rusqlite::Error> for EvaluationScheduleError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}
impl From<std::io::Error> for EvaluationScheduleError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

pub struct EvaluationScheduleStore {
    connection: Mutex<Connection>,
}

impl EvaluationScheduleStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, EvaluationScheduleError> {
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
             CREATE TABLE IF NOT EXISTS evaluation_schedules (
                 id TEXT NOT NULL, principal_id TEXT NOT NULL, tenant_id TEXT NOT NULL,
                 workspace_id TEXT NOT NULL, name TEXT NOT NULL, suite_id TEXT NOT NULL,
                 interval_seconds INTEGER NOT NULL CHECK (interval_seconds > 0), next_run_at INTEGER NOT NULL,
                 enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)), created_at INTEGER NOT NULL,
                 last_claimed_at INTEGER, run_count INTEGER NOT NULL CHECK (run_count >= 0),
                 PRIMARY KEY (principal_id, tenant_id, workspace_id, id)
             );
             CREATE INDEX IF NOT EXISTS evaluation_schedules_due_idx
                 ON evaluation_schedules(principal_id, tenant_id, workspace_id, enabled, next_run_at);
             CREATE TABLE IF NOT EXISTS evaluation_schedule_runs (
                 principal_id TEXT NOT NULL, tenant_id TEXT NOT NULL, workspace_id TEXT NOT NULL,
                 schedule_id TEXT NOT NULL, scheduled_for INTEGER NOT NULL, suite_id TEXT NOT NULL,
                 status TEXT NOT NULL CHECK (status IN ('pending', 'claimed', 'completed', 'failed')),
                 worker_id TEXT, claimed_at INTEGER, lease_until INTEGER, finished_at INTEGER,
                 PRIMARY KEY (principal_id, tenant_id, workspace_id, schedule_id, scheduled_for)
             );
             CREATE INDEX IF NOT EXISTS evaluation_schedule_runs_queue_idx
                 ON evaluation_schedule_runs(principal_id, tenant_id, workspace_id, status, scheduled_for);",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create(
        &self,
        id: &RunLoopId,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
        name: impl Into<String>,
        suite_id: impl Into<String>,
        interval_seconds: u64,
        now: Timestamp,
    ) -> Result<EvaluationSchedule, EvaluationScheduleError> {
        let name = validate_text(
            name.into(),
            MAX_SCHEDULE_NAME_BYTES,
            EvaluationScheduleError::InvalidName,
        )?;
        let suite_id = validate_text(
            suite_id.into(),
            MAX_EVALUATION_SUITE_BYTES,
            EvaluationScheduleError::InvalidSuite,
        )?;
        if interval_seconds == 0 || i64::try_from(interval_seconds).is_err() {
            return Err(EvaluationScheduleError::InvalidInterval);
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = transaction.execute(
            "INSERT INTO evaluation_schedules (id, principal_id, tenant_id, workspace_id, name, suite_id, interval_seconds, next_run_at, enabled, created_at, run_count) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?8, 0)",
            params![id.as_str(), principal_id.as_str(), tenant_id.as_str(), workspace_id.as_str(), name, suite_id, i64::try_from(interval_seconds).unwrap_or(i64::MAX), to_i64(now.as_unix_seconds())?],
        );
        match result {
            Ok(_) => {
                transaction.commit()?;
                Ok(EvaluationSchedule {
                    id: id.clone(),
                    principal_id: principal_id.clone(),
                    tenant_id: tenant_id.clone(),
                    workspace_id: workspace_id.clone(),
                    name,
                    suite_id,
                    interval_seconds,
                    next_run_at: now,
                    enabled: true,
                    created_at: now,
                    last_claimed_at: None,
                    run_count: 0,
                })
            }
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(EvaluationScheduleError::ScheduleAlreadyExists)
            }
            Err(error) => Err(EvaluationScheduleError::Database(error)),
        }
    }

    pub fn list(
        &self,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<EvaluationSchedule>, EvaluationScheduleError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare("SELECT id, principal_id, tenant_id, workspace_id, name, suite_id, interval_seconds, next_run_at, enabled, created_at, last_claimed_at, run_count FROM evaluation_schedules WHERE principal_id = ?1 AND tenant_id = ?2 AND workspace_id = ?3 ORDER BY created_at ASC, id ASC")?;
        let rows = statement.query_map(
            params![
                principal_id.as_str(),
                tenant_id.as_str(),
                workspace_id.as_str()
            ],
            decode_schedule,
        )?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn disable(
        &self,
        id: &RunLoopId,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
    ) -> Result<(), EvaluationScheduleError> {
        let connection = self.lock()?;
        let changed = connection.execute("UPDATE evaluation_schedules SET enabled = 0 WHERE id = ?1 AND principal_id = ?2 AND tenant_id = ?3 AND workspace_id = ?4", params![id.as_str(), principal_id.as_str(), tenant_id.as_str(), workspace_id.as_str()])?;
        if changed == 0 {
            Err(EvaluationScheduleError::ScheduleNotFound)
        } else {
            Ok(())
        }
    }

    pub fn claim_due(
        &self,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
        worker_id: &JobWorkerId,
        now: Timestamp,
        limit: usize,
    ) -> Result<Vec<EvaluationScheduleRun>, EvaluationScheduleError> {
        self.claim_due_for(
            principal_id,
            tenant_id,
            workspace_id,
            None,
            worker_id,
            now,
            limit,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn claim_due_for(
        &self,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
        schedule_id: Option<&RunLoopId>,
        worker_id: &JobWorkerId,
        now: Timestamp,
        limit: usize,
    ) -> Result<Vec<EvaluationScheduleRun>, EvaluationScheduleError> {
        if limit == 0 || limit > MAX_CLAIM_BATCH {
            return Err(EvaluationScheduleError::InvalidLimit);
        }
        if worker_id.as_str().trim().is_empty() {
            return Err(EvaluationScheduleError::InvalidWorker);
        }
        let now_seconds = to_i64(now.as_unix_seconds())?;
        let lease_until = Timestamp::from_unix_seconds(
            now.as_unix_seconds().saturating_add(SCHEDULE_LEASE_SECONDS),
        );
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute("UPDATE evaluation_schedule_runs SET status = 'pending', worker_id = NULL, claimed_at = NULL, lease_until = NULL WHERE principal_id = ?1 AND tenant_id = ?2 AND workspace_id = ?3 AND status = 'claimed' AND lease_until <= ?4", params![principal_id.as_str(), tenant_id.as_str(), workspace_id.as_str(), now_seconds])?;
        let schedules = if let Some(schedule_id) = schedule_id {
            let mut due = transaction.prepare("SELECT id, suite_id, interval_seconds, next_run_at FROM evaluation_schedules WHERE principal_id = ?1 AND tenant_id = ?2 AND workspace_id = ?3 AND enabled = 1 AND next_run_at <= ?4 AND id = ?5 ORDER BY next_run_at ASC, id ASC LIMIT ?6")?;
            due.query_map(
                params![
                    principal_id.as_str(),
                    tenant_id.as_str(),
                    workspace_id.as_str(),
                    now_seconds,
                    schedule_id.as_str(),
                    MAX_CLAIM_BATCH as i64
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )?
            .collect::<Result<Vec<_>, _>>()?
        } else {
            let mut due = transaction.prepare("SELECT id, suite_id, interval_seconds, next_run_at FROM evaluation_schedules WHERE principal_id = ?1 AND tenant_id = ?2 AND workspace_id = ?3 AND enabled = 1 AND next_run_at <= ?4 ORDER BY next_run_at ASC, id ASC LIMIT ?5")?;
            due.query_map(
                params![
                    principal_id.as_str(),
                    tenant_id.as_str(),
                    workspace_id.as_str(),
                    now_seconds,
                    MAX_CLAIM_BATCH as i64
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )?
            .collect::<Result<Vec<_>, _>>()?
        };
        for (id, suite_id, interval_seconds, next_run_at) in schedules {
            if interval_seconds <= 0 || next_run_at < 0 {
                return Err(EvaluationScheduleError::CorruptRecord);
            }
            let next = next_run_at
                .checked_add(interval_seconds)
                .ok_or(EvaluationScheduleError::CorruptRecord)?;
            transaction.execute("INSERT OR IGNORE INTO evaluation_schedule_runs (principal_id, tenant_id, workspace_id, schedule_id, scheduled_for, suite_id, status) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending')", params![principal_id.as_str(), tenant_id.as_str(), workspace_id.as_str(), id, next_run_at, suite_id])?;
            transaction.execute("UPDATE evaluation_schedules SET next_run_at = ?1, last_claimed_at = ?2, run_count = run_count + 1 WHERE id = ?3 AND principal_id = ?4 AND tenant_id = ?5 AND workspace_id = ?6 AND next_run_at = ?7", params![next, now_seconds, id, principal_id.as_str(), tenant_id.as_str(), workspace_id.as_str(), next_run_at])?;
        }
        let candidates = {
            let mut pending = transaction.prepare("SELECT schedule_id, suite_id, scheduled_for FROM evaluation_schedule_runs WHERE principal_id = ?1 AND tenant_id = ?2 AND workspace_id = ?3 AND status = 'pending' AND (?4 IS NULL OR schedule_id = ?4) ORDER BY scheduled_for ASC, schedule_id ASC LIMIT ?5")?;
            pending
                .query_map(
                    params![
                        principal_id.as_str(),
                        tenant_id.as_str(),
                        workspace_id.as_str(),
                        schedule_id.map(RunLoopId::as_str),
                        limit as i64
                    ],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    },
                )?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut claims = Vec::with_capacity(candidates.len());
        for (schedule_id, suite_id, scheduled_for) in candidates {
            transaction.execute("UPDATE evaluation_schedule_runs SET status = 'claimed', worker_id = ?1, claimed_at = ?2, lease_until = ?3 WHERE principal_id = ?4 AND tenant_id = ?5 AND workspace_id = ?6 AND schedule_id = ?7 AND scheduled_for = ?8 AND status = 'pending'", params![worker_id.as_str(), now_seconds, to_i64(lease_until.as_unix_seconds())?, principal_id.as_str(), tenant_id.as_str(), workspace_id.as_str(), schedule_id, scheduled_for])?;
            claims.push(EvaluationScheduleRun {
                schedule_id: RunLoopId::new(schedule_id)
                    .map_err(|_| EvaluationScheduleError::CorruptRecord)?,
                suite_id,
                scheduled_for: Timestamp::from_unix_seconds(to_u64(scheduled_for)?),
                status: EvaluationScheduleRunStatus::Claimed,
                worker_id: Some(worker_id.clone()),
                claimed_at: Some(now),
                lease_until: Some(lease_until),
                finished_at: None,
            });
        }
        transaction.commit()?;
        Ok(claims)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn complete(
        &self,
        schedule_id: &RunLoopId,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
        scheduled_for: Timestamp,
        worker_id: &JobWorkerId,
        success: bool,
        finished_at: Timestamp,
    ) -> Result<(), EvaluationScheduleError> {
        let connection = self.lock()?;
        let current = connection.query_row("SELECT status, worker_id FROM evaluation_schedule_runs WHERE principal_id = ?1 AND tenant_id = ?2 AND workspace_id = ?3 AND schedule_id = ?4 AND scheduled_for = ?5", params![principal_id.as_str(), tenant_id.as_str(), workspace_id.as_str(), schedule_id.as_str(), to_i64(scheduled_for.as_unix_seconds())?], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))).optional()?;
        let Some((status, owner)) = current else {
            return Err(EvaluationScheduleError::RunNotFound);
        };
        let status = parse_status(&status)?;
        if status != EvaluationScheduleRunStatus::Claimed {
            return Err(EvaluationScheduleError::InvalidTransition {
                status,
                action: "complete",
            });
        }
        if owner.as_deref() != Some(worker_id.as_str()) {
            return Err(EvaluationScheduleError::RunOwnedByAnotherWorker);
        }
        connection.execute("UPDATE evaluation_schedule_runs SET status = ?1, finished_at = ?2, lease_until = NULL WHERE principal_id = ?3 AND tenant_id = ?4 AND workspace_id = ?5 AND schedule_id = ?6 AND scheduled_for = ?7 AND worker_id = ?8", params![if success { "completed" } else { "failed" }, to_i64(finished_at.as_unix_seconds())?, principal_id.as_str(), tenant_id.as_str(), workspace_id.as_str(), schedule_id.as_str(), to_i64(scheduled_for.as_unix_seconds())?, worker_id.as_str()])?;
        Ok(())
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, EvaluationScheduleError> {
        self.connection
            .lock()
            .map_err(|_| EvaluationScheduleError::LockPoisoned)
    }
}

fn validate_text(
    value: String,
    max_bytes: usize,
    error: EvaluationScheduleError,
) -> Result<String, EvaluationScheduleError> {
    let value = value.trim().to_owned();
    if value.is_empty() || value.len() > max_bytes || value.chars().any(char::is_control) {
        Err(error)
    } else {
        Ok(value)
    }
}
fn decode_schedule(row: &rusqlite::Row<'_>) -> Result<EvaluationSchedule, rusqlite::Error> {
    let id = RunLoopId::new(row.get::<_, String>(0)?).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let interval_seconds =
        to_u64(row.get::<_, i64>(6)?).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let next_run_at = Timestamp::from_unix_seconds(
        to_u64(row.get::<_, i64>(7)?).map_err(|_| rusqlite::Error::InvalidQuery)?,
    );
    let enabled = row.get::<_, i64>(8)?;
    let run_count = to_u64(row.get::<_, i64>(11)?).map_err(|_| rusqlite::Error::InvalidQuery)?;
    if enabled != 0 && enabled != 1 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(EvaluationSchedule {
        id,
        principal_id: PrincipalId::new(row.get::<_, String>(1)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        tenant_id: TenantId::new(row.get::<_, String>(2)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        workspace_id: WorkspaceId::new(row.get::<_, String>(3)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        name: row.get(4)?,
        suite_id: row.get(5)?,
        interval_seconds,
        next_run_at,
        enabled: enabled == 1,
        created_at: Timestamp::from_unix_seconds(
            to_u64(row.get::<_, i64>(9)?).map_err(|_| rusqlite::Error::InvalidQuery)?,
        ),
        last_claimed_at: row
            .get::<_, Option<i64>>(10)?
            .map(|value| Timestamp::from_unix_seconds(to_u64(value).unwrap_or(0))),
        run_count,
    })
}
fn parse_status(value: &str) -> Result<EvaluationScheduleRunStatus, EvaluationScheduleError> {
    match value {
        "pending" => Ok(EvaluationScheduleRunStatus::Pending),
        "claimed" => Ok(EvaluationScheduleRunStatus::Claimed),
        "completed" => Ok(EvaluationScheduleRunStatus::Completed),
        "failed" => Ok(EvaluationScheduleRunStatus::Failed),
        _ => Err(EvaluationScheduleError::CorruptRecord),
    }
}
fn to_i64(value: u64) -> Result<i64, EvaluationScheduleError> {
    i64::try_from(value).map_err(|_| EvaluationScheduleError::CorruptRecord)
}
fn to_u64(value: i64) -> Result<u64, EvaluationScheduleError> {
    u64::try_from(value).map_err(|_| EvaluationScheduleError::CorruptRecord)
}
fn set_private_permissions(path: &Path) -> Result<(), EvaluationScheduleError> {
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
    fn scope() -> (PrincipalId, TenantId, WorkspaceId) {
        (
            PrincipalId::new("principal").unwrap(),
            TenantId::new("tenant").unwrap(),
            WorkspaceId::new("workspace").unwrap(),
        )
    }
    fn store() -> (EvaluationScheduleStore, std::path::PathBuf) {
        let directory = crate::test_support::new_temp_dir("pandora-evaluation-schedule").unwrap();
        let store = EvaluationScheduleStore::open(directory.join("schedules.sqlite3")).unwrap();
        (store, directory)
    }

    #[test]
    fn creates_and_claims_a_due_occurrence_once() {
        let (store, _directory) = store();
        let (principal, tenant, workspace) = scope();
        let id = RunLoopId::new("nightly").unwrap();
        store
            .create(
                &id,
                &principal,
                &tenant,
                &workspace,
                "Nightly",
                "golden-default",
                60,
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();
        let worker = JobWorkerId::new("worker-a").unwrap();
        let claims = store
            .claim_due(
                &principal,
                &tenant,
                &workspace,
                &worker,
                Timestamp::from_unix_seconds(10),
                4,
            )
            .unwrap();
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].scheduled_for(), Timestamp::from_unix_seconds(10));
        assert!(
            store
                .claim_due(
                    &principal,
                    &tenant,
                    &workspace,
                    &worker,
                    Timestamp::from_unix_seconds(10),
                    4
                )
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store.list(&principal, &tenant, &workspace).unwrap()[0].run_count(),
            1
        );
    }

    #[test]
    fn targeted_claim_does_not_take_another_schedule() {
        let (store, _directory) = store();
        let (principal, tenant, workspace) = scope();
        let first = RunLoopId::new("first").unwrap();
        let second = RunLoopId::new("second").unwrap();
        for id in [&first, &second] {
            store
                .create(
                    id,
                    &principal,
                    &tenant,
                    &workspace,
                    id.as_str(),
                    format!("suite-{}", id.as_str()),
                    60,
                    Timestamp::from_unix_seconds(10),
                )
                .unwrap();
        }
        let worker = JobWorkerId::new("worker-a").unwrap();
        let claims = store
            .claim_due_for(
                &principal,
                &tenant,
                &workspace,
                Some(&second),
                &worker,
                Timestamp::from_unix_seconds(10),
                1,
            )
            .unwrap();
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].schedule_id(), &second);
        let remaining = store
            .claim_due(
                &principal,
                &tenant,
                &workspace,
                &worker,
                Timestamp::from_unix_seconds(10),
                1,
            )
            .unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].schedule_id(), &first);
    }

    #[test]
    fn completed_runs_are_worker_owned() {
        let (store, _directory) = store();
        let (principal, tenant, workspace) = scope();
        let id = RunLoopId::new("hourly").unwrap();
        let worker = JobWorkerId::new("worker-a").unwrap();
        let other = JobWorkerId::new("worker-b").unwrap();
        store
            .create(
                &id,
                &principal,
                &tenant,
                &workspace,
                "Hourly",
                "suite",
                3600,
                Timestamp::from_unix_seconds(20),
            )
            .unwrap();
        let run = store
            .claim_due(
                &principal,
                &tenant,
                &workspace,
                &worker,
                Timestamp::from_unix_seconds(20),
                1,
            )
            .unwrap()
            .remove(0);
        assert!(matches!(
            store.complete(
                &id,
                &principal,
                &tenant,
                &workspace,
                run.scheduled_for(),
                &other,
                true,
                Timestamp::from_unix_seconds(21)
            ),
            Err(EvaluationScheduleError::RunOwnedByAnotherWorker)
        ));
        store
            .complete(
                &id,
                &principal,
                &tenant,
                &workspace,
                run.scheduled_for(),
                &worker,
                true,
                Timestamp::from_unix_seconds(21),
            )
            .unwrap();
    }

    #[test]
    fn expired_claims_are_reclaimed() {
        let (store, _directory) = store();
        let (principal, tenant, workspace) = scope();
        let id = RunLoopId::new("reclaim").unwrap();
        let first = JobWorkerId::new("worker-a").unwrap();
        let second = JobWorkerId::new("worker-b").unwrap();
        store
            .create(
                &id,
                &principal,
                &tenant,
                &workspace,
                "Reclaim",
                "suite",
                60,
                Timestamp::from_unix_seconds(30),
            )
            .unwrap();
        let run = store
            .claim_due(
                &principal,
                &tenant,
                &workspace,
                &first,
                Timestamp::from_unix_seconds(30),
                1,
            )
            .unwrap()
            .remove(0);
        let claims = store
            .claim_due(
                &principal,
                &tenant,
                &workspace,
                &second,
                Timestamp::from_unix_seconds(30 + SCHEDULE_LEASE_SECONDS),
                1,
            )
            .unwrap();
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].scheduled_for(), run.scheduled_for());
        assert_eq!(claims[0].worker_id(), Some(&second));
    }

    #[test]
    fn invalid_schedule_text_and_limits_are_rejected() {
        let (store, _directory) = store();
        let (principal, tenant, workspace) = scope();
        let id = RunLoopId::new("invalid").unwrap();
        assert!(matches!(
            store.create(
                &id,
                &principal,
                &tenant,
                &workspace,
                "\n",
                "suite",
                60,
                Timestamp::from_unix_seconds(1)
            ),
            Err(EvaluationScheduleError::InvalidName)
        ));
        assert!(matches!(
            store.create(
                &id,
                &principal,
                &tenant,
                &workspace,
                "Name",
                "suite",
                0,
                Timestamp::from_unix_seconds(1)
            ),
            Err(EvaluationScheduleError::InvalidInterval)
        ));
        let worker = JobWorkerId::new("worker").unwrap();
        assert!(matches!(
            store.claim_due(
                &principal,
                &tenant,
                &workspace,
                &worker,
                Timestamp::from_unix_seconds(1),
                0
            ),
            Err(EvaluationScheduleError::InvalidLimit)
        ));
    }
}
