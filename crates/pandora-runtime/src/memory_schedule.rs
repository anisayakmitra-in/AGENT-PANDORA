use pandora_types::{
    ContextClassification, JobWorkerId, MemoryId, MemoryKind, PrincipalId, RunLoopId, SessionId,
    TenantId, Timestamp, WorkspaceId,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::fmt;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

pub const MAX_MEMORY_SYNTHESIS_SCHEDULES: usize = 128;
pub const MAX_MEMORY_SYNTHESIS_CLAIM_BATCH: usize = 16;
pub const MAX_MEMORY_SYNTHESIS_SCHEDULE_NAME_BYTES: usize = 128;
pub const MAX_MEMORY_SYNTHESIS_SUMMARY_BYTES: usize = 8 * 1024;
pub const MEMORY_SYNTHESIS_SCHEDULE_LEASE_SECONDS: u64 = 300;
pub const MAX_MEMORY_SYNTHESIS_RUNS: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemorySynthesisSchedule {
    id: RunLoopId,
    principal_id: PrincipalId,
    tenant_id: TenantId,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    name: String,
    provider: String,
    memory_id: MemoryId,
    kind: MemoryKind,
    summary: String,
    classification: ContextClassification,
    interval_seconds: u64,
    next_run_at: Timestamp,
    enabled: bool,
    created_at: Timestamp,
    last_claimed_at: Option<Timestamp>,
    run_count: u64,
}

impl MemorySynthesisSchedule {
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

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn memory_id(&self) -> &MemoryId {
        &self.memory_id
    }

    pub const fn kind(&self) -> MemoryKind {
        self.kind
    }

    pub fn summary(&self) -> &str {
        &self.summary
    }

    pub const fn classification(&self) -> ContextClassification {
        self.classification
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
pub struct MemorySynthesisScheduleRun {
    schedule_id: RunLoopId,
    scheduled_for: Timestamp,
    status: MemorySynthesisScheduleRunStatus,
    worker_id: Option<JobWorkerId>,
    claimed_at: Option<Timestamp>,
    lease_until: Option<Timestamp>,
    finished_at: Option<Timestamp>,
    snapshot_digest: Option<String>,
    result_memory_id: Option<MemoryId>,
    failure: Option<String>,
}

impl MemorySynthesisScheduleRun {
    pub fn schedule_id(&self) -> &RunLoopId {
        &self.schedule_id
    }

    pub const fn scheduled_for(&self) -> Timestamp {
        self.scheduled_for
    }

    pub const fn status(&self) -> MemorySynthesisScheduleRunStatus {
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

    pub fn snapshot_digest(&self) -> Option<&str> {
        self.snapshot_digest.as_deref()
    }

    pub fn result_memory_id(&self) -> Option<&MemoryId> {
        self.result_memory_id.as_ref()
    }

    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemorySynthesisScheduleRunStatus {
    Pending,
    Claimed,
    Completed,
    Failed,
}

impl MemorySynthesisScheduleRunStatus {
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
pub enum MemorySynthesisScheduleError {
    Database(rusqlite::Error),
    Io(std::io::Error),
    InvalidName,
    InvalidProvider,
    InvalidMemoryId,
    InvalidKind,
    InvalidSummary,
    InvalidClassification,
    InvalidInterval,
    InvalidLimit,
    InvalidWorker,
    CorruptRecord,
    ScheduleAlreadyExists,
    ScheduleNotFound,
    RunNotFound,
    RunOwnedByAnotherWorker,
    InvalidTransition {
        status: MemorySynthesisScheduleRunStatus,
        action: &'static str,
    },
    LockPoisoned,
}

impl fmt::Display for MemorySynthesisScheduleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(_) => formatter.write_str("memory schedule database operation failed"),
            Self::Io(_) => {
                formatter.write_str("memory schedule database directory operation failed")
            }
            Self::InvalidName => formatter.write_str("memory schedule name is invalid"),
            Self::InvalidProvider => formatter.write_str("memory schedule provider is invalid"),
            Self::InvalidMemoryId => formatter.write_str("memory schedule memory ID is invalid"),
            Self::InvalidKind => formatter.write_str("memory schedule kind is not an L1 kind"),
            Self::InvalidSummary => formatter.write_str("memory schedule summary is invalid"),
            Self::InvalidClassification => {
                formatter.write_str("memory schedule classification must be public or internal")
            }
            Self::InvalidInterval => formatter.write_str("memory schedule interval is invalid"),
            Self::InvalidLimit => formatter.write_str("memory schedule claim limit is invalid"),
            Self::InvalidWorker => {
                formatter.write_str("memory schedule worker identifier is invalid")
            }
            Self::CorruptRecord => {
                formatter.write_str("memory schedule database contains an invalid record")
            }
            Self::ScheduleAlreadyExists => formatter.write_str("memory schedule already exists"),
            Self::ScheduleNotFound => formatter.write_str("memory schedule was not found"),
            Self::RunNotFound => formatter.write_str("memory schedule run was not found"),
            Self::RunOwnedByAnotherWorker => {
                formatter.write_str("memory schedule run is owned by another worker")
            }
            Self::InvalidTransition { status, action } => write!(
                formatter,
                "cannot {action} a {} memory schedule run",
                status.as_str()
            ),
            Self::LockPoisoned => {
                formatter.write_str("memory schedule database lock is unavailable")
            }
        }
    }
}

impl std::error::Error for MemorySynthesisScheduleError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for MemorySynthesisScheduleError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

impl From<std::io::Error> for MemorySynthesisScheduleError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

pub struct MemorySynthesisScheduleStore {
    connection: Mutex<Connection>,
}

impl MemorySynthesisScheduleStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, MemorySynthesisScheduleError> {
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
             CREATE TABLE IF NOT EXISTS memory_synthesis_schedules (
                 id TEXT NOT NULL, principal_id TEXT NOT NULL, tenant_id TEXT NOT NULL,
                 workspace_id TEXT NOT NULL, session_id TEXT NOT NULL, name TEXT NOT NULL, provider TEXT NOT NULL,
                 memory_id TEXT NOT NULL, kind TEXT NOT NULL, summary TEXT NOT NULL,
                 classification TEXT NOT NULL CHECK (classification IN ('public', 'internal')),
                 interval_seconds INTEGER NOT NULL CHECK (interval_seconds > 0),
                 next_run_at INTEGER NOT NULL CHECK (next_run_at >= 0),
                 enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)), created_at INTEGER NOT NULL,
                 last_claimed_at INTEGER, run_count INTEGER NOT NULL CHECK (run_count >= 0),
                 PRIMARY KEY (principal_id, tenant_id, workspace_id, id)
             );
             CREATE INDEX IF NOT EXISTS memory_synthesis_schedules_due_idx
                 ON memory_synthesis_schedules(principal_id, tenant_id, workspace_id, enabled, next_run_at);
             CREATE TABLE IF NOT EXISTS memory_synthesis_schedule_runs (
                 principal_id TEXT NOT NULL, tenant_id TEXT NOT NULL, workspace_id TEXT NOT NULL,
                 schedule_id TEXT NOT NULL, scheduled_for INTEGER NOT NULL,
                 status TEXT NOT NULL CHECK (status IN ('pending', 'claimed', 'completed', 'failed')),
                 worker_id TEXT, claimed_at INTEGER, lease_until INTEGER, finished_at INTEGER,
                 snapshot_digest TEXT, result_memory_id TEXT, failure TEXT,
                 PRIMARY KEY (principal_id, tenant_id, workspace_id, schedule_id, scheduled_for)
             );
             CREATE INDEX IF NOT EXISTS memory_synthesis_schedule_runs_queue_idx
                 ON memory_synthesis_schedule_runs(principal_id, tenant_id, workspace_id, status, scheduled_for);",
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
        session_id: &SessionId,
        name: impl Into<String>,
        provider: impl Into<String>,
        memory_id: &MemoryId,
        kind: MemoryKind,
        summary: impl Into<String>,
        classification: ContextClassification,
        interval_seconds: u64,
        now: Timestamp,
    ) -> Result<MemorySynthesisSchedule, MemorySynthesisScheduleError> {
        let name = validate_text(
            name.into(),
            MAX_MEMORY_SYNTHESIS_SCHEDULE_NAME_BYTES,
            MemorySynthesisScheduleError::InvalidName,
        )?;
        let provider = validate_text(
            provider.into(),
            256,
            MemorySynthesisScheduleError::InvalidProvider,
        )?;
        let summary = validate_text(
            summary.into(),
            MAX_MEMORY_SYNTHESIS_SUMMARY_BYTES,
            MemorySynthesisScheduleError::InvalidSummary,
        )?;
        if !matches!(
            kind,
            MemoryKind::ExecutionEvidence
                | MemoryKind::Decision
                | MemoryKind::Failure
                | MemoryKind::Benchmark
                | MemoryKind::Lesson
                | MemoryKind::Lineage
        ) {
            return Err(MemorySynthesisScheduleError::InvalidKind);
        }
        if !matches!(
            classification,
            ContextClassification::Public | ContextClassification::Internal
        ) {
            return Err(MemorySynthesisScheduleError::InvalidClassification);
        }
        if interval_seconds == 0 || i64::try_from(interval_seconds).is_err() {
            return Err(MemorySynthesisScheduleError::InvalidInterval);
        }
        let now_seconds = to_i64(now.as_unix_seconds())?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let count = transaction.query_row(
            "SELECT COUNT(*) FROM memory_synthesis_schedules WHERE principal_id = ?1 AND tenant_id = ?2 AND workspace_id = ?3",
            params![principal_id.as_str(), tenant_id.as_str(), workspace_id.as_str()],
            |row| row.get::<_, i64>(0),
        )?;
        if usize::try_from(count).map_err(|_| MemorySynthesisScheduleError::CorruptRecord)?
            >= MAX_MEMORY_SYNTHESIS_SCHEDULES
        {
            return Err(MemorySynthesisScheduleError::InvalidLimit);
        }
        let result = transaction.execute(
            "INSERT INTO memory_synthesis_schedules
             (id, principal_id, tenant_id, workspace_id, session_id, name, provider, memory_id, kind, summary,
              classification, interval_seconds, next_run_at, enabled, created_at, run_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 1, ?13, 0)",
            params![
                id.as_str(),
                principal_id.as_str(),
                tenant_id.as_str(),
                workspace_id.as_str(),
                session_id.as_str(),
                name,
                provider,
                memory_id.as_str(),
                kind.as_str(),
                summary,
                classification.as_str(),
                i64::try_from(interval_seconds).map_err(|_| MemorySynthesisScheduleError::InvalidInterval)?,
                now_seconds,
            ],
        );
        match result {
            Ok(_) => {
                transaction.commit()?;
                Ok(MemorySynthesisSchedule {
                    id: id.clone(),
                    principal_id: principal_id.clone(),
                    tenant_id: tenant_id.clone(),
                    workspace_id: workspace_id.clone(),
                    session_id: session_id.clone(),
                    name,
                    provider,
                    memory_id: memory_id.clone(),
                    kind,
                    summary,
                    classification,
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
                Err(MemorySynthesisScheduleError::ScheduleAlreadyExists)
            }
            Err(error) => Err(MemorySynthesisScheduleError::Database(error)),
        }
    }

    pub fn list(
        &self,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<MemorySynthesisSchedule>, MemorySynthesisScheduleError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id, principal_id, tenant_id, workspace_id, session_id, name, provider, memory_id,
                    kind, summary, classification, interval_seconds, next_run_at, enabled,
                    created_at, last_claimed_at, run_count
             FROM memory_synthesis_schedules
             WHERE principal_id = ?1 AND tenant_id = ?2 AND workspace_id = ?3
             ORDER BY created_at ASC, id ASC",
        )?;
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
    ) -> Result<(), MemorySynthesisScheduleError> {
        let connection = self.lock()?;
        let changed = connection.execute(
            "UPDATE memory_synthesis_schedules SET enabled = 0
             WHERE id = ?1 AND principal_id = ?2 AND tenant_id = ?3 AND workspace_id = ?4",
            params![
                id.as_str(),
                principal_id.as_str(),
                tenant_id.as_str(),
                workspace_id.as_str()
            ],
        )?;
        if changed == 0 {
            Err(MemorySynthesisScheduleError::ScheduleNotFound)
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
    ) -> Result<Vec<MemorySynthesisScheduleRun>, MemorySynthesisScheduleError> {
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
    ) -> Result<Vec<MemorySynthesisScheduleRun>, MemorySynthesisScheduleError> {
        if limit == 0 || limit > MAX_MEMORY_SYNTHESIS_CLAIM_BATCH {
            return Err(MemorySynthesisScheduleError::InvalidLimit);
        }
        if worker_id.as_str().trim().is_empty() {
            return Err(MemorySynthesisScheduleError::InvalidWorker);
        }
        let now_seconds = to_i64(now.as_unix_seconds())?;
        let lease_until = Timestamp::from_unix_seconds(
            now.as_unix_seconds()
                .saturating_add(MEMORY_SYNTHESIS_SCHEDULE_LEASE_SECONDS),
        );
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "UPDATE memory_synthesis_schedule_runs SET status = 'pending', worker_id = NULL,
                    claimed_at = NULL, lease_until = NULL
             WHERE principal_id = ?1 AND tenant_id = ?2 AND workspace_id = ?3
               AND status = 'claimed' AND lease_until <= ?4",
            params![
                principal_id.as_str(),
                tenant_id.as_str(),
                workspace_id.as_str(),
                now_seconds
            ],
        )?;
        let mut due = transaction.prepare(
            "SELECT id, interval_seconds, next_run_at
             FROM memory_synthesis_schedules
             WHERE principal_id = ?1 AND tenant_id = ?2 AND workspace_id = ?3
               AND enabled = 1 AND next_run_at <= ?4
               AND (?5 IS NULL OR id = ?5)
             ORDER BY next_run_at ASC, id ASC LIMIT ?6",
        )?;
        let schedules = due
            .query_map(
                params![
                    principal_id.as_str(),
                    tenant_id.as_str(),
                    workspace_id.as_str(),
                    now_seconds,
                    schedule_id.map(RunLoopId::as_str),
                    MAX_MEMORY_SYNTHESIS_CLAIM_BATCH as i64,
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        drop(due);
        for (id, interval_seconds, next_run_at) in schedules {
            if interval_seconds <= 0 || next_run_at < 0 {
                return Err(MemorySynthesisScheduleError::CorruptRecord);
            }
            let next = next_run_at
                .checked_add(interval_seconds)
                .ok_or(MemorySynthesisScheduleError::CorruptRecord)?;
            transaction.execute(
                "INSERT OR IGNORE INTO memory_synthesis_schedule_runs
                 (principal_id, tenant_id, workspace_id, schedule_id, scheduled_for, status)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'pending')",
                params![
                    principal_id.as_str(),
                    tenant_id.as_str(),
                    workspace_id.as_str(),
                    id,
                    next_run_at,
                ],
            )?;
            transaction.execute(
                "UPDATE memory_synthesis_schedules
                 SET next_run_at = ?1, last_claimed_at = ?2, run_count = run_count + 1
                 WHERE id = ?3 AND principal_id = ?4 AND tenant_id = ?5
                   AND workspace_id = ?6 AND next_run_at = ?7",
                params![
                    next,
                    now_seconds,
                    id,
                    principal_id.as_str(),
                    tenant_id.as_str(),
                    workspace_id.as_str(),
                    next_run_at,
                ],
            )?;
        }
        let mut pending = transaction.prepare(
            "SELECT schedule_id, scheduled_for
             FROM memory_synthesis_schedule_runs
             WHERE principal_id = ?1 AND tenant_id = ?2 AND workspace_id = ?3
               AND status = 'pending' AND (?4 IS NULL OR schedule_id = ?4)
             ORDER BY scheduled_for ASC, schedule_id ASC LIMIT ?5",
        )?;
        let candidates = pending
            .query_map(
                params![
                    principal_id.as_str(),
                    tenant_id.as_str(),
                    workspace_id.as_str(),
                    schedule_id.map(RunLoopId::as_str),
                    limit as i64,
                ],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )?
            .collect::<Result<Vec<_>, _>>()?;
        drop(pending);
        let mut claims = Vec::with_capacity(candidates.len());
        for (schedule_id, scheduled_for) in candidates {
            transaction.execute(
                "UPDATE memory_synthesis_schedule_runs
                 SET status = 'claimed', worker_id = ?1, claimed_at = ?2, lease_until = ?3
                 WHERE principal_id = ?4 AND tenant_id = ?5 AND workspace_id = ?6
                   AND schedule_id = ?7 AND scheduled_for = ?8 AND status = 'pending'",
                params![
                    worker_id.as_str(),
                    now_seconds,
                    to_i64(lease_until.as_unix_seconds())?,
                    principal_id.as_str(),
                    tenant_id.as_str(),
                    workspace_id.as_str(),
                    schedule_id,
                    scheduled_for,
                ],
            )?;
            claims.push(MemorySynthesisScheduleRun {
                schedule_id: RunLoopId::new(schedule_id)
                    .map_err(|_| MemorySynthesisScheduleError::CorruptRecord)?,
                scheduled_for: Timestamp::from_unix_seconds(to_u64(scheduled_for)?),
                status: MemorySynthesisScheduleRunStatus::Claimed,
                worker_id: Some(worker_id.clone()),
                claimed_at: Some(now),
                lease_until: Some(lease_until),
                finished_at: None,
                snapshot_digest: None,
                result_memory_id: None,
                failure: None,
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
        snapshot_digest: Option<&str>,
        result_memory_id: Option<&MemoryId>,
        failure: Option<&str>,
    ) -> Result<(), MemorySynthesisScheduleError> {
        let snapshot_digest = snapshot_digest
            .map(|value| {
                validate_text(
                    value.to_owned(),
                    256,
                    MemorySynthesisScheduleError::CorruptRecord,
                )
            })
            .transpose()?;
        let failure = failure
            .map(|value| {
                validate_text(
                    value.to_owned(),
                    4096,
                    MemorySynthesisScheduleError::CorruptRecord,
                )
            })
            .transpose()?;
        let connection = self.lock()?;
        let current = connection
            .query_row(
                "SELECT status, worker_id FROM memory_synthesis_schedule_runs
                 WHERE principal_id = ?1 AND tenant_id = ?2 AND workspace_id = ?3
                   AND schedule_id = ?4 AND scheduled_for = ?5",
                params![
                    principal_id.as_str(),
                    tenant_id.as_str(),
                    workspace_id.as_str(),
                    schedule_id.as_str(),
                    to_i64(scheduled_for.as_unix_seconds())?,
                ],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()?;
        let Some((status, owner)) = current else {
            return Err(MemorySynthesisScheduleError::RunNotFound);
        };
        let status = parse_status(&status)?;
        if status != MemorySynthesisScheduleRunStatus::Claimed {
            return Err(MemorySynthesisScheduleError::InvalidTransition {
                status,
                action: "complete",
            });
        }
        if owner.as_deref() != Some(worker_id.as_str()) {
            return Err(MemorySynthesisScheduleError::RunOwnedByAnotherWorker);
        }
        connection.execute(
            "UPDATE memory_synthesis_schedule_runs
             SET status = ?1, finished_at = ?2, lease_until = NULL,
                 snapshot_digest = ?3, result_memory_id = ?4, failure = ?5
             WHERE principal_id = ?6 AND tenant_id = ?7 AND workspace_id = ?8
               AND schedule_id = ?9 AND scheduled_for = ?10 AND worker_id = ?11",
            params![
                if success { "completed" } else { "failed" },
                to_i64(finished_at.as_unix_seconds())?,
                snapshot_digest,
                result_memory_id.map(MemoryId::as_str),
                failure,
                principal_id.as_str(),
                tenant_id.as_str(),
                workspace_id.as_str(),
                schedule_id.as_str(),
                to_i64(scheduled_for.as_unix_seconds())?,
                worker_id.as_str(),
            ],
        )?;
        Ok(())
    }

    pub fn list_runs(
        &self,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
        schedule_id: Option<&RunLoopId>,
    ) -> Result<Vec<MemorySynthesisScheduleRun>, MemorySynthesisScheduleError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT schedule_id, scheduled_for, status, worker_id, claimed_at, lease_until,
                    finished_at, snapshot_digest, result_memory_id, failure
             FROM memory_synthesis_schedule_runs
             WHERE principal_id = ?1 AND tenant_id = ?2 AND workspace_id = ?3
               AND (?4 IS NULL OR schedule_id = ?4)
             ORDER BY scheduled_for DESC, schedule_id ASC LIMIT ?5",
        )?;
        let rows = statement.query_map(
            params![
                principal_id.as_str(),
                tenant_id.as_str(),
                workspace_id.as_str(),
                schedule_id.map(RunLoopId::as_str),
                MAX_MEMORY_SYNTHESIS_RUNS as i64,
            ],
            decode_run,
        )?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, MemorySynthesisScheduleError> {
        self.connection
            .lock()
            .map_err(|_| MemorySynthesisScheduleError::LockPoisoned)
    }
}

fn validate_text(
    value: String,
    max_bytes: usize,
    error: MemorySynthesisScheduleError,
) -> Result<String, MemorySynthesisScheduleError> {
    let value = value.trim().to_owned();
    if value.is_empty() || value.len() > max_bytes || value.chars().any(char::is_control) {
        Err(error)
    } else {
        Ok(value)
    }
}

fn decode_schedule(row: &rusqlite::Row<'_>) -> Result<MemorySynthesisSchedule, rusqlite::Error> {
    let id = RunLoopId::new(row.get::<_, String>(0)?).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let principal_id =
        PrincipalId::new(row.get::<_, String>(1)?).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let tenant_id =
        TenantId::new(row.get::<_, String>(2)?).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let workspace_id =
        WorkspaceId::new(row.get::<_, String>(3)?).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let session_id =
        SessionId::new(row.get::<_, String>(4)?).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let memory_id =
        MemoryId::new(row.get::<_, String>(7)?).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let kind = parse_kind(&row.get::<_, String>(8)?).ok_or(rusqlite::Error::InvalidQuery)?;
    let classification =
        parse_classification(&row.get::<_, String>(10)?).ok_or(rusqlite::Error::InvalidQuery)?;
    let interval_seconds =
        to_u64(row.get::<_, i64>(11)?).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let next_run_at = Timestamp::from_unix_seconds(
        to_u64(row.get::<_, i64>(12)?).map_err(|_| rusqlite::Error::InvalidQuery)?,
    );
    let enabled = row.get::<_, i64>(13)?;
    let run_count = to_u64(row.get::<_, i64>(16)?).map_err(|_| rusqlite::Error::InvalidQuery)?;
    if enabled != 0 && enabled != 1 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(MemorySynthesisSchedule {
        id,
        principal_id,
        tenant_id,
        workspace_id,
        session_id,
        name: row.get(5)?,
        provider: row.get(6)?,
        memory_id,
        kind,
        summary: row.get(9)?,
        classification,
        interval_seconds,
        next_run_at,
        enabled: enabled == 1,
        created_at: Timestamp::from_unix_seconds(
            to_u64(row.get::<_, i64>(14)?).map_err(|_| rusqlite::Error::InvalidQuery)?,
        ),
        last_claimed_at: row
            .get::<_, Option<i64>>(15)?
            .map(|value| Timestamp::from_unix_seconds(to_u64(value).unwrap_or(0))),
        run_count,
    })
}

fn decode_run(row: &rusqlite::Row<'_>) -> Result<MemorySynthesisScheduleRun, rusqlite::Error> {
    Ok(MemorySynthesisScheduleRun {
        schedule_id: RunLoopId::new(row.get::<_, String>(0)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        scheduled_for: Timestamp::from_unix_seconds(
            to_u64(row.get::<_, i64>(1)?).map_err(|_| rusqlite::Error::InvalidQuery)?,
        ),
        status: parse_status(&row.get::<_, String>(2)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        worker_id: row
            .get::<_, Option<String>>(3)?
            .map(|value| JobWorkerId::new(value).map_err(|_| rusqlite::Error::InvalidQuery))
            .transpose()?,
        claimed_at: row
            .get::<_, Option<i64>>(4)?
            .map(|value| Timestamp::from_unix_seconds(to_u64(value).unwrap_or(0))),
        lease_until: row
            .get::<_, Option<i64>>(5)?
            .map(|value| Timestamp::from_unix_seconds(to_u64(value).unwrap_or(0))),
        finished_at: row
            .get::<_, Option<i64>>(6)?
            .map(|value| Timestamp::from_unix_seconds(to_u64(value).unwrap_or(0))),
        snapshot_digest: row.get(7)?,
        result_memory_id: row
            .get::<_, Option<String>>(8)?
            .map(|value| MemoryId::new(value).map_err(|_| rusqlite::Error::InvalidQuery))
            .transpose()?,
        failure: row.get(9)?,
    })
}

fn parse_kind(value: &str) -> Option<MemoryKind> {
    match value {
        "execution_evidence" => Some(MemoryKind::ExecutionEvidence),
        "decision" => Some(MemoryKind::Decision),
        "failure" => Some(MemoryKind::Failure),
        "benchmark" => Some(MemoryKind::Benchmark),
        "lesson" => Some(MemoryKind::Lesson),
        "lineage" => Some(MemoryKind::Lineage),
        _ => None,
    }
}

fn parse_classification(value: &str) -> Option<ContextClassification> {
    match value {
        "public" => Some(ContextClassification::Public),
        "internal" => Some(ContextClassification::Internal),
        _ => None,
    }
}

fn parse_status(
    value: &str,
) -> Result<MemorySynthesisScheduleRunStatus, MemorySynthesisScheduleError> {
    match value {
        "pending" => Ok(MemorySynthesisScheduleRunStatus::Pending),
        "claimed" => Ok(MemorySynthesisScheduleRunStatus::Claimed),
        "completed" => Ok(MemorySynthesisScheduleRunStatus::Completed),
        "failed" => Ok(MemorySynthesisScheduleRunStatus::Failed),
        _ => Err(MemorySynthesisScheduleError::CorruptRecord),
    }
}

fn to_i64(value: u64) -> Result<i64, MemorySynthesisScheduleError> {
    i64::try_from(value).map_err(|_| MemorySynthesisScheduleError::CorruptRecord)
}

fn to_u64(value: i64) -> Result<u64, MemorySynthesisScheduleError> {
    u64::try_from(value).map_err(|_| MemorySynthesisScheduleError::CorruptRecord)
}

fn set_private_permissions(path: &Path) -> Result<(), MemorySynthesisScheduleError> {
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

    fn scope() -> (PrincipalId, TenantId, WorkspaceId, SessionId, MemoryId) {
        (
            PrincipalId::new("principal").unwrap(),
            TenantId::new("tenant").unwrap(),
            WorkspaceId::new("workspace").unwrap(),
            SessionId::new("session").unwrap(),
            MemoryId::new("memory").unwrap(),
        )
    }

    fn store() -> (MemorySynthesisScheduleStore, std::path::PathBuf) {
        let directory = crate::test_support::new_temp_dir("pandora-memory-schedule").unwrap();
        let store =
            MemorySynthesisScheduleStore::open(directory.join("schedules.sqlite3")).unwrap();
        (store, directory)
    }

    #[test]
    fn schedules_are_scoped_and_a_due_occurrence_is_claimed_once() {
        let (store, directory) = store();
        let (principal, tenant, workspace, session, memory) = scope();
        let id = RunLoopId::new("nightly").unwrap();
        let schedule = store
            .create(
                &id,
                &principal,
                &tenant,
                &workspace,
                &session,
                "nightly lessons",
                "provider",
                &memory,
                MemoryKind::Lesson,
                "Synthesize lessons",
                ContextClassification::Internal,
                60,
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();
        assert_eq!(
            store.list(&principal, &tenant, &workspace).unwrap(),
            vec![schedule]
        );
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
        store
            .complete(
                &id,
                &principal,
                &tenant,
                &workspace,
                claims[0].scheduled_for(),
                &worker,
                true,
                Timestamp::from_unix_seconds(11),
                Some("digest"),
                Some(&memory),
                None,
            )
            .unwrap();
        let reopened =
            MemorySynthesisScheduleStore::open(directory.join("schedules.sqlite3")).unwrap();
        let runs = reopened
            .list_runs(&principal, &tenant, &workspace, Some(&id))
            .unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0].status(),
            MemorySynthesisScheduleRunStatus::Completed
        );
        assert_eq!(runs[0].snapshot_digest(), Some("digest"));
        assert_eq!(runs[0].result_memory_id(), Some(&memory));
    }

    #[test]
    fn expired_claims_can_be_reclaimed_by_another_worker() {
        let (store, _directory) = store();
        let (principal, tenant, workspace, session, memory) = scope();
        let id = RunLoopId::new("reclaim").unwrap();
        store
            .create(
                &id,
                &principal,
                &tenant,
                &workspace,
                &session,
                "nightly lessons",
                "provider",
                &memory,
                MemoryKind::Lesson,
                "Synthesize lessons",
                ContextClassification::Internal,
                60,
                Timestamp::from_unix_seconds(20),
            )
            .unwrap();
        let first = JobWorkerId::new("worker-a").unwrap();
        let second = JobWorkerId::new("worker-b").unwrap();
        let first_claim = store
            .claim_due(
                &principal,
                &tenant,
                &workspace,
                &first,
                Timestamp::from_unix_seconds(20),
                1,
            )
            .unwrap();
        assert_eq!(first_claim.len(), 1);
        let second_claim = store
            .claim_due(
                &principal,
                &tenant,
                &workspace,
                &second,
                Timestamp::from_unix_seconds(20 + MEMORY_SYNTHESIS_SCHEDULE_LEASE_SECONDS),
                1,
            )
            .unwrap();
        assert_eq!(second_claim.len(), 1);
        assert_eq!(second_claim[0].worker_id(), Some(&second));
    }

    #[test]
    fn sensitive_schedules_and_invalid_kinds_are_rejected() {
        let (store, _directory) = store();
        let (principal, tenant, workspace, session, memory) = scope();
        let id = RunLoopId::new("invalid").unwrap();
        assert!(matches!(
            store.create(
                &id,
                &principal,
                &tenant,
                &workspace,
                &session,
                "nightly lessons",
                "provider",
                &memory,
                MemoryKind::Trace,
                "summary",
                ContextClassification::Internal,
                60,
                Timestamp::from_unix_seconds(1),
            ),
            Err(MemorySynthesisScheduleError::InvalidKind)
        ));
        assert!(matches!(
            store.create(
                &id,
                &principal,
                &tenant,
                &workspace,
                &session,
                "nightly lessons",
                "provider",
                &memory,
                MemoryKind::Lesson,
                "summary",
                ContextClassification::Sensitive,
                60,
                Timestamp::from_unix_seconds(1),
            ),
            Err(MemorySynthesisScheduleError::InvalidClassification)
        ));
    }
}
