use crate::{OrchestrationError, OrchestrationRun, OrchestrationRunSnapshot};
use pandora_types::{
    GovernedOrchestrationPlan, JobWorkerId, OrchestrationRoleReceipt, OrchestrationRunId,
    PrincipalId, RoleAssignment, RoleId, TenantId, Timestamp, WorkspaceId,
    WorkspaceOrchestrationError,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrchestrationRunStatus {
    Queued,
    Running,
    Completed,
    Interrupted,
    Cancelled,
}

impl OrchestrationRunStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Interrupted => "interrupted",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrchestrationRunRecord {
    run_id: OrchestrationRunId,
    principal_id: PrincipalId,
    tenant_id: TenantId,
    coordinator_workspace_id: WorkspaceId,
    plan: GovernedOrchestrationPlan,
    snapshot: OrchestrationRunSnapshot,
    status: OrchestrationRunStatus,
    worker_id: Option<JobWorkerId>,
    role_receipts: Vec<OrchestrationRoleReceipt>,
    interruption_reason: Option<String>,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl OrchestrationRunRecord {
    pub fn run_id(&self) -> &OrchestrationRunId {
        &self.run_id
    }

    pub fn principal_id(&self) -> &PrincipalId {
        &self.principal_id
    }

    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    pub fn coordinator_workspace_id(&self) -> &WorkspaceId {
        &self.coordinator_workspace_id
    }

    pub fn plan(&self) -> &GovernedOrchestrationPlan {
        &self.plan
    }

    pub fn snapshot(&self) -> &OrchestrationRunSnapshot {
        &self.snapshot
    }

    pub const fn status(&self) -> OrchestrationRunStatus {
        self.status
    }

    pub fn worker_id(&self) -> Option<&JobWorkerId> {
        self.worker_id.as_ref()
    }

    pub fn role_receipts(&self) -> &[OrchestrationRoleReceipt] {
        &self.role_receipts
    }

    pub fn interruption_reason(&self) -> Option<&str> {
        self.interruption_reason.as_deref()
    }

    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }

    pub const fn updated_at(&self) -> Timestamp {
        self.updated_at
    }
}

#[derive(Debug)]
pub enum OrchestrationStoreError {
    Database(rusqlite::Error),
    Io(std::io::Error),
    Serialization(serde_json::Error),
    Contract(WorkspaceOrchestrationError),
    Orchestration(OrchestrationError),
    InvalidIdentifier,
    CorruptRecord,
    RunAlreadyExists,
    RunNotFound,
    RunOwnedByAnotherWorker,
    DuplicateReceipt,
    ActiveRolesRequireReconciliation,
    InvalidTransition {
        status: OrchestrationRunStatus,
        action: &'static str,
    },
    LockPoisoned,
}

impl fmt::Display for OrchestrationStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(_) => formatter.write_str("orchestration database operation failed"),
            Self::Io(_) => formatter.write_str("orchestration database directory operation failed"),
            Self::Serialization(_) => formatter.write_str("orchestration record is invalid"),
            Self::Contract(error) => error.fmt(formatter),
            Self::Orchestration(error) => error.fmt(formatter),
            Self::InvalidIdentifier => {
                formatter.write_str("orchestration record identifier is invalid")
            }
            Self::CorruptRecord => {
                formatter.write_str("orchestration database contains an invalid record")
            }
            Self::RunAlreadyExists => formatter.write_str("orchestration run already exists"),
            Self::RunNotFound => formatter.write_str("orchestration run was not found"),
            Self::RunOwnedByAnotherWorker => {
                formatter.write_str("orchestration run is owned by another worker")
            }
            Self::DuplicateReceipt => {
                formatter.write_str("orchestration role receipt is duplicated")
            }
            Self::ActiveRolesRequireReconciliation => formatter.write_str(
                "interrupted orchestration has active roles that require receipt reconciliation",
            ),
            Self::InvalidTransition { status, action } => {
                write!(
                    formatter,
                    "cannot {action} a {} orchestration run",
                    status.as_str()
                )
            }
            Self::LockPoisoned => formatter.write_str("orchestration database lock is unavailable"),
        }
    }
}

impl std::error::Error for OrchestrationStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Serialization(error) => Some(error),
            Self::Contract(error) => Some(error),
            Self::Orchestration(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for OrchestrationStoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

impl From<std::io::Error> for OrchestrationStoreError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for OrchestrationStoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}

impl From<WorkspaceOrchestrationError> for OrchestrationStoreError {
    fn from(error: WorkspaceOrchestrationError) -> Self {
        Self::Contract(error)
    }
}

impl From<OrchestrationError> for OrchestrationStoreError {
    fn from(error: OrchestrationError) -> Self {
        Self::Orchestration(error)
    }
}

pub struct OrchestrationStore {
    connection: Mutex<Connection>,
}

impl OrchestrationStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, OrchestrationStoreError> {
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
             CREATE TABLE IF NOT EXISTS orchestration_runs (
                 run_id TEXT PRIMARY KEY,
                 submission_sequence INTEGER NOT NULL UNIQUE CHECK (submission_sequence > 0),
                 principal_id TEXT NOT NULL,
                 tenant_id TEXT NOT NULL,
                 coordinator_workspace_id TEXT NOT NULL,
                 plan_json TEXT NOT NULL,
                 snapshot_json TEXT NOT NULL,
                 status TEXT NOT NULL CHECK (
                     status IN ('queued', 'running', 'completed', 'interrupted', 'cancelled')
                 ),
                 worker_id TEXT,
                 receipts_json TEXT NOT NULL,
                 interruption_reason TEXT,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS orchestration_scope_queue_idx
                 ON orchestration_runs(
                     principal_id, tenant_id, coordinator_workspace_id, status, submission_sequence
                 );",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn submit(
        &self,
        run_id: &OrchestrationRunId,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        coordinator_workspace_id: &WorkspaceId,
        plan: &GovernedOrchestrationPlan,
        created_at: Timestamp,
    ) -> Result<OrchestrationRunRecord, OrchestrationStoreError> {
        plan.validate()?;
        let snapshot = OrchestrationRun::new(plan.plan().clone()).snapshot();
        let plan_json = serde_json::to_string(plan)?;
        let snapshot_json = serde_json::to_string(&snapshot)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let sequence = transaction.query_row(
            "SELECT COALESCE(MAX(submission_sequence), 0) + 1 FROM orchestration_runs",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        let result = transaction.execute(
            "INSERT INTO orchestration_runs (
                 run_id, submission_sequence, principal_id, tenant_id,
                 coordinator_workspace_id, plan_json, snapshot_json, status,
                 receipts_json, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'queued', '[]', ?8, ?8)",
            params![
                run_id.as_str(),
                sequence,
                principal_id.as_str(),
                tenant_id.as_str(),
                coordinator_workspace_id.as_str(),
                plan_json,
                snapshot_json,
                to_i64(created_at.as_unix_seconds())?,
            ],
        );
        match result {
            Ok(_) => {
                transaction.commit()?;
                Ok(OrchestrationRunRecord {
                    run_id: run_id.clone(),
                    principal_id: principal_id.clone(),
                    tenant_id: tenant_id.clone(),
                    coordinator_workspace_id: coordinator_workspace_id.clone(),
                    plan: plan.clone(),
                    snapshot,
                    status: OrchestrationRunStatus::Queued,
                    worker_id: None,
                    role_receipts: Vec::new(),
                    interruption_reason: None,
                    created_at,
                    updated_at: created_at,
                })
            }
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(OrchestrationStoreError::RunAlreadyExists)
            }
            Err(error) => Err(OrchestrationStoreError::Database(error)),
        }
    }

    pub fn list(
        &self,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        coordinator_workspace_id: &WorkspaceId,
    ) -> Result<Vec<OrchestrationRunRecord>, OrchestrationStoreError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT run_id, principal_id, tenant_id, coordinator_workspace_id,
                    plan_json, snapshot_json, status, worker_id, receipts_json,
                    interruption_reason, created_at, updated_at
             FROM orchestration_runs
             WHERE principal_id = ?1 AND tenant_id = ?2 AND coordinator_workspace_id = ?3
             ORDER BY submission_sequence DESC",
        )?;
        let mut rows = statement.query(params![
            principal_id.as_str(),
            tenant_id.as_str(),
            coordinator_workspace_id.as_str(),
        ])?;
        let mut records = Vec::new();
        while let Some(row) = rows.next()? {
            records.push(decode_record(row)?);
        }
        Ok(records)
    }

    pub fn inspect(
        &self,
        run_id: &OrchestrationRunId,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        coordinator_workspace_id: &WorkspaceId,
    ) -> Result<OrchestrationRunRecord, OrchestrationStoreError> {
        let connection = self.lock()?;
        load_scoped(
            &connection,
            run_id,
            principal_id,
            tenant_id,
            coordinator_workspace_id,
        )?
        .ok_or(OrchestrationStoreError::RunNotFound)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn claim_next(
        &self,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        coordinator_workspace_id: &WorkspaceId,
        worker_id: &JobWorkerId,
        now: Timestamp,
    ) -> Result<Option<OrchestrationRunRecord>, OrchestrationStoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run_id = transaction
            .query_row(
                "SELECT run_id FROM orchestration_runs
                 WHERE principal_id = ?1 AND tenant_id = ?2
                   AND coordinator_workspace_id = ?3 AND status = 'queued'
                 ORDER BY submission_sequence ASC LIMIT 1",
                params![
                    principal_id.as_str(),
                    tenant_id.as_str(),
                    coordinator_workspace_id.as_str(),
                ],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(run_id) = run_id else {
            transaction.commit()?;
            return Ok(None);
        };
        let run_id = OrchestrationRunId::new(run_id)
            .map_err(|_| OrchestrationStoreError::InvalidIdentifier)?;
        let changed = transaction.execute(
            "UPDATE orchestration_runs SET status = 'running', worker_id = ?1, updated_at = ?2
             WHERE run_id = ?3 AND principal_id = ?4 AND tenant_id = ?5
               AND coordinator_workspace_id = ?6 AND status = 'queued'",
            params![
                worker_id.as_str(),
                to_i64(now.as_unix_seconds())?,
                run_id.as_str(),
                principal_id.as_str(),
                tenant_id.as_str(),
                coordinator_workspace_id.as_str(),
            ],
        )?;
        if changed != 1 {
            return Err(OrchestrationStoreError::CorruptRecord);
        }
        let record = load_scoped(
            &transaction,
            &run_id,
            principal_id,
            tenant_id,
            coordinator_workspace_id,
        )?
        .ok_or(OrchestrationStoreError::CorruptRecord)?;
        transaction.commit()?;
        Ok(Some(record))
    }

    pub fn start_ready(
        &self,
        run_id: &OrchestrationRunId,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        coordinator_workspace_id: &WorkspaceId,
        worker_id: &JobWorkerId,
        now: Timestamp,
    ) -> Result<Vec<RoleAssignment>, OrchestrationStoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = running_owned(
            &transaction,
            run_id,
            principal_id,
            tenant_id,
            coordinator_workspace_id,
            worker_id,
        )?;
        let mut run = OrchestrationRun::from_snapshot(current.snapshot.clone())?;
        let ready = run.start_ready()?;
        let snapshot_json = serde_json::to_string(&run.snapshot())?;
        transaction.execute(
            "UPDATE orchestration_runs SET snapshot_json = ?1, updated_at = ?2
             WHERE run_id = ?3 AND principal_id = ?4 AND tenant_id = ?5
               AND coordinator_workspace_id = ?6 AND status = 'running' AND worker_id = ?7",
            params![
                snapshot_json,
                to_i64(now.as_unix_seconds())?,
                run_id.as_str(),
                principal_id.as_str(),
                tenant_id.as_str(),
                coordinator_workspace_id.as_str(),
                worker_id.as_str(),
            ],
        )?;
        transaction.commit()?;
        Ok(ready)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn complete_role(
        &self,
        run_id: &OrchestrationRunId,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        coordinator_workspace_id: &WorkspaceId,
        worker_id: &JobWorkerId,
        role_id: &RoleId,
        receipt: &OrchestrationRoleReceipt,
        now: Timestamp,
    ) -> Result<OrchestrationRunRecord, OrchestrationStoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut current = running_owned(
            &transaction,
            run_id,
            principal_id,
            tenant_id,
            coordinator_workspace_id,
            worker_id,
        )?;
        current.plan.validate_receipt(run_id, role_id, receipt)?;
        if current
            .role_receipts
            .iter()
            .any(|stored| stored.receipt_id() == receipt.receipt_id())
        {
            return Err(OrchestrationStoreError::DuplicateReceipt);
        }
        let mut run = OrchestrationRun::from_snapshot(current.snapshot.clone())?;
        run.complete(role_id)?;
        current.snapshot = run.snapshot();
        current.role_receipts.push(receipt.clone());
        current.status = if run.is_complete() {
            OrchestrationRunStatus::Completed
        } else {
            OrchestrationRunStatus::Running
        };
        current.updated_at = now;
        let snapshot_json = serde_json::to_string(&current.snapshot)?;
        let receipts_json = serde_json::to_string(&current.role_receipts)?;
        let changed = transaction.execute(
            "UPDATE orchestration_runs
             SET snapshot_json = ?1, receipts_json = ?2, status = ?3, updated_at = ?4
             WHERE run_id = ?5 AND principal_id = ?6 AND tenant_id = ?7
               AND coordinator_workspace_id = ?8 AND status = 'running' AND worker_id = ?9",
            params![
                snapshot_json,
                receipts_json,
                current.status.as_str(),
                to_i64(now.as_unix_seconds())?,
                run_id.as_str(),
                principal_id.as_str(),
                tenant_id.as_str(),
                coordinator_workspace_id.as_str(),
                worker_id.as_str(),
            ],
        )?;
        if changed != 1 {
            return Err(OrchestrationStoreError::CorruptRecord);
        }
        transaction.commit()?;
        Ok(current)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn mark_interrupted(
        &self,
        run_id: &OrchestrationRunId,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        coordinator_workspace_id: &WorkspaceId,
        reason: &str,
        now: Timestamp,
    ) -> Result<OrchestrationRunRecord, OrchestrationStoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut current = load_scoped(
            &transaction,
            run_id,
            principal_id,
            tenant_id,
            coordinator_workspace_id,
        )?
        .ok_or(OrchestrationStoreError::RunNotFound)?;
        if current.status != OrchestrationRunStatus::Running {
            return Err(OrchestrationStoreError::InvalidTransition {
                status: current.status,
                action: "mark interrupted",
            });
        }
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(OrchestrationStoreError::CorruptRecord);
        }
        transaction.execute(
            "UPDATE orchestration_runs
             SET status = 'interrupted', interruption_reason = ?1, updated_at = ?2
             WHERE run_id = ?3 AND principal_id = ?4 AND tenant_id = ?5
               AND coordinator_workspace_id = ?6 AND status = 'running'",
            params![
                reason,
                to_i64(now.as_unix_seconds())?,
                run_id.as_str(),
                principal_id.as_str(),
                tenant_id.as_str(),
                coordinator_workspace_id.as_str(),
            ],
        )?;
        transaction.commit()?;
        current.status = OrchestrationRunStatus::Interrupted;
        current.interruption_reason = Some(reason.to_owned());
        current.updated_at = now;
        Ok(current)
    }

    pub fn resume(
        &self,
        run_id: &OrchestrationRunId,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        coordinator_workspace_id: &WorkspaceId,
        now: Timestamp,
    ) -> Result<OrchestrationRunRecord, OrchestrationStoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut current = load_scoped(
            &transaction,
            run_id,
            principal_id,
            tenant_id,
            coordinator_workspace_id,
        )?
        .ok_or(OrchestrationStoreError::RunNotFound)?;
        if current.status != OrchestrationRunStatus::Interrupted {
            return Err(OrchestrationStoreError::InvalidTransition {
                status: current.status,
                action: "resume",
            });
        }
        if !current.snapshot.active_roles().is_empty() {
            return Err(OrchestrationStoreError::ActiveRolesRequireReconciliation);
        }
        transaction.execute(
            "UPDATE orchestration_runs
             SET status = 'queued', worker_id = NULL, interruption_reason = NULL, updated_at = ?1
             WHERE run_id = ?2 AND principal_id = ?3 AND tenant_id = ?4
               AND coordinator_workspace_id = ?5 AND status = 'interrupted'",
            params![
                to_i64(now.as_unix_seconds())?,
                run_id.as_str(),
                principal_id.as_str(),
                tenant_id.as_str(),
                coordinator_workspace_id.as_str(),
            ],
        )?;
        transaction.commit()?;
        current.status = OrchestrationRunStatus::Queued;
        current.worker_id = None;
        current.interruption_reason = None;
        current.updated_at = now;
        Ok(current)
    }

    pub fn cancel(
        &self,
        run_id: &OrchestrationRunId,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        coordinator_workspace_id: &WorkspaceId,
        now: Timestamp,
    ) -> Result<OrchestrationRunRecord, OrchestrationStoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut current = load_scoped(
            &transaction,
            run_id,
            principal_id,
            tenant_id,
            coordinator_workspace_id,
        )?
        .ok_or(OrchestrationStoreError::RunNotFound)?;
        if current.status != OrchestrationRunStatus::Queued {
            return Err(OrchestrationStoreError::InvalidTransition {
                status: current.status,
                action: "cancel",
            });
        }
        transaction.execute(
            "UPDATE orchestration_runs SET status = 'cancelled', updated_at = ?1
             WHERE run_id = ?2 AND principal_id = ?3 AND tenant_id = ?4
               AND coordinator_workspace_id = ?5 AND status = 'queued'",
            params![
                to_i64(now.as_unix_seconds())?,
                run_id.as_str(),
                principal_id.as_str(),
                tenant_id.as_str(),
                coordinator_workspace_id.as_str(),
            ],
        )?;
        transaction.commit()?;
        current.status = OrchestrationRunStatus::Cancelled;
        current.updated_at = now;
        Ok(current)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, OrchestrationStoreError> {
        self.connection
            .lock()
            .map_err(|_| OrchestrationStoreError::LockPoisoned)
    }
}

fn running_owned(
    connection: &Connection,
    run_id: &OrchestrationRunId,
    principal_id: &PrincipalId,
    tenant_id: &TenantId,
    coordinator_workspace_id: &WorkspaceId,
    worker_id: &JobWorkerId,
) -> Result<OrchestrationRunRecord, OrchestrationStoreError> {
    let current = load_scoped(
        connection,
        run_id,
        principal_id,
        tenant_id,
        coordinator_workspace_id,
    )?
    .ok_or(OrchestrationStoreError::RunNotFound)?;
    if current.status != OrchestrationRunStatus::Running {
        return Err(OrchestrationStoreError::InvalidTransition {
            status: current.status,
            action: "update",
        });
    }
    if current.worker_id.as_ref() != Some(worker_id) {
        return Err(OrchestrationStoreError::RunOwnedByAnotherWorker);
    }
    Ok(current)
}

fn load_scoped(
    connection: &Connection,
    run_id: &OrchestrationRunId,
    principal_id: &PrincipalId,
    tenant_id: &TenantId,
    coordinator_workspace_id: &WorkspaceId,
) -> Result<Option<OrchestrationRunRecord>, OrchestrationStoreError> {
    connection
        .query_row(
            "SELECT run_id, principal_id, tenant_id, coordinator_workspace_id,
                    plan_json, snapshot_json, status, worker_id, receipts_json,
                    interruption_reason, created_at, updated_at
             FROM orchestration_runs
             WHERE run_id = ?1 AND principal_id = ?2 AND tenant_id = ?3
               AND coordinator_workspace_id = ?4",
            params![
                run_id.as_str(),
                principal_id.as_str(),
                tenant_id.as_str(),
                coordinator_workspace_id.as_str(),
            ],
            decode_record,
        )
        .optional()
        .map_err(OrchestrationStoreError::from)
}

fn decode_record(row: &rusqlite::Row<'_>) -> Result<OrchestrationRunRecord, rusqlite::Error> {
    decode_record_inner(row).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })
}

fn decode_record_inner(
    row: &rusqlite::Row<'_>,
) -> Result<OrchestrationRunRecord, OrchestrationStoreError> {
    let run_id = OrchestrationRunId::new(row.get::<_, String>(0)?)
        .map_err(|_| OrchestrationStoreError::InvalidIdentifier)?;
    let principal_id = PrincipalId::new(row.get::<_, String>(1)?)
        .map_err(|_| OrchestrationStoreError::InvalidIdentifier)?;
    let tenant_id = TenantId::new(row.get::<_, String>(2)?)
        .map_err(|_| OrchestrationStoreError::InvalidIdentifier)?;
    let coordinator_workspace_id = WorkspaceId::new(row.get::<_, String>(3)?)
        .map_err(|_| OrchestrationStoreError::InvalidIdentifier)?;
    let plan: GovernedOrchestrationPlan = serde_json::from_str(&row.get::<_, String>(4)?)?;
    let snapshot: OrchestrationRunSnapshot = serde_json::from_str(&row.get::<_, String>(5)?)?;
    let status = decode_status(&row.get::<_, String>(6)?)?;
    let worker_id = row
        .get::<_, Option<String>>(7)?
        .map(JobWorkerId::new)
        .transpose()
        .map_err(|_| OrchestrationStoreError::InvalidIdentifier)?;
    let role_receipts = serde_json::from_str(&row.get::<_, String>(8)?)?;
    let interruption_reason = row.get(9)?;
    let created_at = decode_timestamp(row.get(10)?)?;
    let updated_at = decode_timestamp(row.get(11)?)?;
    validate_record(
        &run_id,
        &plan,
        &snapshot,
        status,
        worker_id.as_ref(),
        &role_receipts,
        interruption_reason.as_deref(),
    )?;
    Ok(OrchestrationRunRecord {
        run_id,
        principal_id,
        tenant_id,
        coordinator_workspace_id,
        plan,
        snapshot,
        status,
        worker_id,
        role_receipts,
        interruption_reason,
        created_at,
        updated_at,
    })
}

fn validate_record(
    run_id: &OrchestrationRunId,
    plan: &GovernedOrchestrationPlan,
    snapshot: &OrchestrationRunSnapshot,
    status: OrchestrationRunStatus,
    worker_id: Option<&JobWorkerId>,
    receipts: &[OrchestrationRoleReceipt],
    interruption_reason: Option<&str>,
) -> Result<(), OrchestrationStoreError> {
    plan.validate()?;
    let run = OrchestrationRun::from_snapshot(snapshot.clone())?;
    if snapshot.plan() != plan.plan() {
        return Err(OrchestrationStoreError::CorruptRecord);
    }
    let status_valid = match status {
        OrchestrationRunStatus::Queued | OrchestrationRunStatus::Cancelled => {
            worker_id.is_none() && interruption_reason.is_none()
        }
        OrchestrationRunStatus::Running | OrchestrationRunStatus::Completed => {
            worker_id.is_some() && interruption_reason.is_none()
        }
        OrchestrationRunStatus::Interrupted => {
            worker_id.is_some()
                && interruption_reason
                    .map(|reason| !reason.trim().is_empty())
                    .unwrap_or(false)
        }
    };
    if !status_valid || (status == OrchestrationRunStatus::Completed && !run.is_complete()) {
        return Err(OrchestrationStoreError::CorruptRecord);
    }
    let mut receipt_ids = std::collections::BTreeSet::new();
    for receipt in receipts {
        if !receipt_ids.insert(receipt.receipt_id())
            || plan
                .validate_receipt(run_id, receipt.role_id(), receipt)
                .is_err()
        {
            return Err(OrchestrationStoreError::CorruptRecord);
        }
    }
    Ok(())
}

fn decode_status(value: &str) -> Result<OrchestrationRunStatus, OrchestrationStoreError> {
    match value {
        "queued" => Ok(OrchestrationRunStatus::Queued),
        "running" => Ok(OrchestrationRunStatus::Running),
        "completed" => Ok(OrchestrationRunStatus::Completed),
        "interrupted" => Ok(OrchestrationRunStatus::Interrupted),
        "cancelled" => Ok(OrchestrationRunStatus::Cancelled),
        _ => Err(OrchestrationStoreError::CorruptRecord),
    }
}

fn to_i64(value: u64) -> Result<i64, OrchestrationStoreError> {
    i64::try_from(value).map_err(|_| OrchestrationStoreError::CorruptRecord)
}

fn decode_timestamp(value: i64) -> Result<Timestamp, OrchestrationStoreError> {
    u64::try_from(value)
        .map(Timestamp::from_unix_seconds)
        .map_err(|_| OrchestrationStoreError::CorruptRecord)
}

fn set_private_permissions(path: &Path) -> Result<(), OrchestrationStoreError> {
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
        GovernedOrchestrationPlan, Handoff, HarnessId, MetaComposition, OrchestrationPlan,
        OrchestrationRole, PlanId, ReceiptId, RepositoryBinding, RepositoryId, RequestDigest,
        RoleRepositoryBinding,
    };

    struct Fixture {
        root: std::path::PathBuf,
        store: OrchestrationStore,
        principal: PrincipalId,
        tenant: TenantId,
        coordinator_workspace: WorkspaceId,
        worker: JobWorkerId,
        plan: GovernedOrchestrationPlan,
    }

    impl Fixture {
        fn new() -> Self {
            let root = crate::test_support::new_temp_dir("pandora-orchestration-store").unwrap();
            Self {
                store: OrchestrationStore::open(root.join("orchestration.sqlite3")).unwrap(),
                root,
                principal: PrincipalId::new("principal-1").unwrap(),
                tenant: TenantId::new("tenant-1").unwrap(),
                coordinator_workspace: WorkspaceId::new("workspace-coordinator").unwrap(),
                worker: JobWorkerId::new("orchestration-worker-1").unwrap(),
                plan: plan(),
            }
        }

        fn submit(&self, id: &str, now: u64) -> OrchestrationRunId {
            let run_id = OrchestrationRunId::new(id).unwrap();
            self.store
                .submit(
                    &run_id,
                    &self.principal,
                    &self.tenant,
                    &self.coordinator_workspace,
                    &self.plan,
                    Timestamp::from_unix_seconds(now),
                )
                .unwrap();
            run_id
        }

        fn claim(&self, now: u64) -> OrchestrationRunRecord {
            self.store
                .claim_next(
                    &self.principal,
                    &self.tenant,
                    &self.coordinator_workspace,
                    &self.worker,
                    Timestamp::from_unix_seconds(now),
                )
                .unwrap()
                .unwrap()
        }

        fn receipt(
            &self,
            run_id: &OrchestrationRunId,
            role: &str,
            repository: &str,
            workspace: &str,
            commit: &str,
        ) -> OrchestrationRoleReceipt {
            OrchestrationRoleReceipt::new(
                ReceiptId::new(format!("receipt-{role}")).unwrap(),
                run_id.clone(),
                RoleId::new(role).unwrap(),
                RepositoryId::new(repository).unwrap(),
                WorkspaceId::new(workspace).unwrap(),
                commit,
                Vec::new(),
                Some(RequestDigest::new(format!("evidence-{role}")).unwrap()),
            )
            .unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn role(id: &str, harness: &str, dependencies: &[&str]) -> RoleAssignment {
        RoleAssignment::new(
            RoleId::new(id).unwrap(),
            OrchestrationRole::Custom(id.to_owned()),
            HarnessId::new(harness).unwrap(),
            dependencies
                .iter()
                .map(|dependency| RoleId::new(*dependency).unwrap())
                .collect(),
        )
        .unwrap()
    }

    fn plan() -> GovernedOrchestrationPlan {
        GovernedOrchestrationPlan::new(
            OrchestrationPlan::new(
                PlanId::new("multi-repository").unwrap(),
                vec![
                    role("planner", "coding-domain", &[]),
                    role("maker", "design-domain", &["planner"]),
                ],
                2,
                1,
                vec![Handoff::new(
                    RoleId::new("planner").unwrap(),
                    RoleId::new("maker").unwrap(),
                    Some(HarnessId::new("coordination-meta").unwrap()),
                )],
            )
            .unwrap(),
            MetaComposition::new(
                vec![
                    HarnessId::new("coding-domain").unwrap(),
                    HarnessId::new("design-domain").unwrap(),
                ],
                1,
            )
            .unwrap(),
            vec![
                RepositoryBinding::new(
                    RepositoryId::new("api").unwrap(),
                    WorkspaceId::new("workspace-api").unwrap(),
                    "commit-api",
                )
                .unwrap(),
                RepositoryBinding::new(
                    RepositoryId::new("desktop").unwrap(),
                    WorkspaceId::new("workspace-desktop").unwrap(),
                    "commit-desktop",
                )
                .unwrap(),
            ],
            vec![
                RoleRepositoryBinding::new(
                    RoleId::new("planner").unwrap(),
                    RepositoryId::new("api").unwrap(),
                ),
                RoleRepositoryBinding::new(
                    RoleId::new("maker").unwrap(),
                    RepositoryId::new("desktop").unwrap(),
                ),
            ],
        )
        .unwrap()
    }

    #[test]
    fn durable_worker_coordinates_roles_with_per_repository_receipts() {
        let fixture = Fixture::new();
        let run_id = fixture.submit("run-1", 10);
        let claimed = fixture.claim(20);
        assert_eq!(claimed.run_id(), &run_id);
        assert_eq!(claimed.worker_id(), Some(&fixture.worker));

        let ready = fixture
            .store
            .start_ready(
                &run_id,
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
                &fixture.worker,
                Timestamp::from_unix_seconds(30),
            )
            .unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id().as_str(), "planner");

        let wrong = fixture.receipt(
            &run_id,
            "planner",
            "desktop",
            "workspace-desktop",
            "commit-desktop",
        );
        assert!(matches!(
            fixture.store.complete_role(
                &run_id,
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
                &fixture.worker,
                &RoleId::new("planner").unwrap(),
                &wrong,
                Timestamp::from_unix_seconds(40),
            ),
            Err(OrchestrationStoreError::Contract(
                WorkspaceOrchestrationError::ReceiptRepositoryMismatch(_)
            ))
        ));

        let planner_receipt =
            fixture.receipt(&run_id, "planner", "api", "workspace-api", "commit-api");
        fixture
            .store
            .complete_role(
                &run_id,
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
                &fixture.worker,
                &RoleId::new("planner").unwrap(),
                &planner_receipt,
                Timestamp::from_unix_seconds(50),
            )
            .unwrap();
        let ready = fixture
            .store
            .start_ready(
                &run_id,
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
                &fixture.worker,
                Timestamp::from_unix_seconds(60),
            )
            .unwrap();
        assert_eq!(ready[0].id().as_str(), "maker");

        let maker_receipt = fixture.receipt(
            &run_id,
            "maker",
            "desktop",
            "workspace-desktop",
            "commit-desktop",
        );
        let completed = fixture
            .store
            .complete_role(
                &run_id,
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
                &fixture.worker,
                &RoleId::new("maker").unwrap(),
                &maker_receipt,
                Timestamp::from_unix_seconds(70),
            )
            .unwrap();
        assert_eq!(completed.status(), OrchestrationRunStatus::Completed);
        assert_eq!(completed.role_receipts().len(), 2);

        let reopened =
            OrchestrationStore::open(fixture.root.join("orchestration.sqlite3")).unwrap();
        let persisted = reopened
            .inspect(
                &run_id,
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
            )
            .unwrap();
        assert_eq!(persisted.status(), OrchestrationRunStatus::Completed);
        assert_eq!(persisted.snapshot().completed_roles().len(), 2);
    }

    #[test]
    fn interrupted_runs_resume_only_before_role_execution_is_active() {
        let fixture = Fixture::new();
        let safe_run = fixture.submit("run-safe", 10);
        fixture.claim(20);
        fixture
            .store
            .mark_interrupted(
                &safe_run,
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
                "worker exited before dispatch",
                Timestamp::from_unix_seconds(30),
            )
            .unwrap();
        let resumed = fixture
            .store
            .resume(
                &safe_run,
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
                Timestamp::from_unix_seconds(40),
            )
            .unwrap();
        assert_eq!(resumed.status(), OrchestrationRunStatus::Queued);
        fixture
            .store
            .cancel(
                &safe_run,
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
                Timestamp::from_unix_seconds(45),
            )
            .unwrap();

        let active_run = fixture.submit("run-active", 50);
        fixture.claim(60);
        fixture
            .store
            .start_ready(
                &active_run,
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
                &fixture.worker,
                Timestamp::from_unix_seconds(70),
            )
            .unwrap();
        fixture
            .store
            .mark_interrupted(
                &active_run,
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
                "worker exited after dispatch",
                Timestamp::from_unix_seconds(80),
            )
            .unwrap();
        assert!(matches!(
            fixture.store.resume(
                &active_run,
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
                Timestamp::from_unix_seconds(90),
            ),
            Err(OrchestrationStoreError::ActiveRolesRequireReconciliation)
        ));
    }
}
