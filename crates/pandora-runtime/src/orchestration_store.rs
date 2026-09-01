use crate::{OrchestrationError, OrchestrationRun, OrchestrationRunSnapshot};
use pandora_types::{
    GovernedOrchestrationPlan, JobWorkerId, OrchestrationBudgetAmount, OrchestrationRoleReceipt,
    OrchestrationRunId, OrchestrationUsage, PrincipalId, RequestDigest, RoleAssignment, RoleId,
    TenantId, Timestamp, WorkspaceId, WorkspaceOrchestrationError,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

const ORCHESTRATION_SCHEMA_COMPONENT: &str = "orchestration_store";
const ORCHESTRATION_SCHEMA_VERSION: i64 = 1;

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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrchestrationBudgetReservationState {
    Active,
    Settled,
    Released,
}

impl OrchestrationBudgetReservationState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Settled => "settled",
            Self::Released => "released",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OrchestrationBudgetReservationRecord {
    role_id: RoleId,
    reservation: OrchestrationBudgetAmount,
    state: OrchestrationBudgetReservationState,
    completion_receipt_id: Option<String>,
    reconciliation_evidence_digest: Option<RequestDigest>,
    usage: Option<OrchestrationUsage>,
    reserved_at: Timestamp,
    updated_at: Timestamp,
}

impl OrchestrationBudgetReservationRecord {
    pub fn role_id(&self) -> &RoleId {
        &self.role_id
    }

    pub const fn reservation(&self) -> OrchestrationBudgetAmount {
        self.reservation
    }

    pub const fn state(&self) -> OrchestrationBudgetReservationState {
        self.state
    }

    pub fn completion_receipt_id(&self) -> Option<&str> {
        self.completion_receipt_id.as_deref()
    }

    pub fn reconciliation_evidence_digest(&self) -> Option<&RequestDigest> {
        self.reconciliation_evidence_digest.as_ref()
    }

    pub fn usage(&self) -> Option<&OrchestrationUsage> {
        self.usage.as_ref()
    }

    pub const fn reserved_at(&self) -> Timestamp {
        self.reserved_at
    }

    pub const fn updated_at(&self) -> Timestamp {
        self.updated_at
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OrchestrationBudgetSnapshot {
    run_id: OrchestrationRunId,
    principal_id: PrincipalId,
    tenant_id: TenantId,
    coordinator_workspace_id: WorkspaceId,
    ceiling: OrchestrationBudgetAmount,
    reserved: OrchestrationBudgetAmount,
    enforced_consumed: OrchestrationBudgetAmount,
    enforced_remaining: OrchestrationBudgetAmount,
    actual_cost_micros: Option<u64>,
    known_cost_micros: u64,
    unknown_cost_receipts: u64,
    reservations: Vec<OrchestrationBudgetReservationRecord>,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl OrchestrationBudgetSnapshot {
    pub fn run_id(&self) -> &OrchestrationRunId {
        &self.run_id
    }

    pub const fn ceiling(&self) -> OrchestrationBudgetAmount {
        self.ceiling
    }

    pub const fn reserved(&self) -> OrchestrationBudgetAmount {
        self.reserved
    }

    pub const fn enforced_consumed(&self) -> OrchestrationBudgetAmount {
        self.enforced_consumed
    }

    pub const fn enforced_remaining(&self) -> OrchestrationBudgetAmount {
        self.enforced_remaining
    }

    pub const fn actual_cost_micros(&self) -> Option<u64> {
        self.actual_cost_micros
    }

    pub const fn known_cost_micros(&self) -> u64 {
        self.known_cost_micros
    }

    pub const fn unknown_cost_receipts(&self) -> u64 {
        self.unknown_cost_receipts
    }

    pub fn reservations(&self) -> &[OrchestrationBudgetReservationRecord] {
        &self.reservations
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
struct OrchestrationBudgetLedger {
    enforced_consumed: OrchestrationBudgetAmount,
    known_cost_micros: u64,
    unknown_cost_receipts: u64,
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
    AggregateBudgetRequired,
    AggregateBudgetNotFound,
    BudgetReservationNotFound,
    BudgetReservationNotActive,
    UsageRequired,
    UsageExceedsReservation(&'static str),
    AggregateBudgetExceeded(&'static str),
    AggregateBudgetOverflow,
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
            Self::AggregateBudgetRequired => {
                formatter.write_str("orchestration requires an aggregate budget")
            }
            Self::AggregateBudgetNotFound => {
                formatter.write_str("orchestration aggregate budget was not found")
            }
            Self::BudgetReservationNotFound => {
                formatter.write_str("orchestration role budget reservation was not found")
            }
            Self::BudgetReservationNotActive => {
                formatter.write_str("orchestration role budget reservation is not active")
            }
            Self::UsageRequired => {
                formatter.write_str("budgeted orchestration completion requires measured usage")
            }
            Self::UsageExceedsReservation(resource) => {
                write!(
                    formatter,
                    "measured {resource} usage exceeds the role reservation"
                )
            }
            Self::AggregateBudgetExceeded(resource) => {
                write!(formatter, "aggregate {resource} budget is exhausted")
            }
            Self::AggregateBudgetOverflow => {
                formatter.write_str("aggregate orchestration budget overflowed")
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
        let mut connection = Connection::open(path)?;
        set_private_permissions(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        let journal_mode =
            connection.query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            connection.execute_batch("PRAGMA journal_mode = WAL;")?;
        }
        if current_orchestration_schema_version(&connection)? == Some(ORCHESTRATION_SCHEMA_VERSION)
        {
            return Ok(Self {
                connection: Mutex::new(connection),
            });
        }
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS orchestration_runs (
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
                 );
             CREATE TABLE IF NOT EXISTS orchestration_budgets (
                 run_id TEXT PRIMARY KEY,
                 principal_id TEXT NOT NULL,
                 tenant_id TEXT NOT NULL,
                 coordinator_workspace_id TEXT NOT NULL,
                 ceiling_json TEXT NOT NULL,
                 ledger_json TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS orchestration_budget_scope_idx
                 ON orchestration_budgets(
                     principal_id, tenant_id, coordinator_workspace_id, updated_at
                 );
             CREATE TABLE IF NOT EXISTS orchestration_budget_reservations (
                 run_id TEXT NOT NULL,
                 principal_id TEXT NOT NULL,
                 tenant_id TEXT NOT NULL,
                 coordinator_workspace_id TEXT NOT NULL,
                 role_id TEXT NOT NULL,
                 reservation_json TEXT NOT NULL,
                 state TEXT NOT NULL CHECK (state IN ('active', 'settled', 'released')),
                 completion_receipt_id TEXT,
                 reconciliation_evidence_digest TEXT,
                 usage_json TEXT,
                 reserved_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 PRIMARY KEY (run_id, role_id)
             );
             CREATE UNIQUE INDEX IF NOT EXISTS orchestration_budget_completion_receipt_idx
                 ON orchestration_budget_reservations(completion_receipt_id)
                 WHERE completion_receipt_id IS NOT NULL;
             CREATE TABLE IF NOT EXISTS orchestration_budget_reconciliations (
                 run_id TEXT NOT NULL,
                 principal_id TEXT NOT NULL,
                 tenant_id TEXT NOT NULL,
                 coordinator_workspace_id TEXT NOT NULL,
                 role_id TEXT NOT NULL,
                 evidence_digest TEXT NOT NULL,
                 usage_json TEXT NOT NULL,
                 recorded_at INTEGER NOT NULL,
                 PRIMARY KEY (run_id, role_id, evidence_digest)
             );",
        )?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(
            "CREATE TABLE IF NOT EXISTS pandora_schema_versions (
                 component TEXT PRIMARY KEY,
                 version INTEGER NOT NULL CHECK (version > 0)
             );",
        )?;
        transaction.execute(
            "INSERT INTO pandora_schema_versions (component, version)
             VALUES (?1, ?2)
             ON CONFLICT(component) DO UPDATE SET version = excluded.version",
            params![ORCHESTRATION_SCHEMA_COMPONENT, ORCHESTRATION_SCHEMA_VERSION],
        )?;
        transaction.commit()?;
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
        let aggregate_budget = plan
            .aggregate_budget()
            .ok_or(OrchestrationStoreError::AggregateBudgetRequired)?;
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
                transaction.execute(
                    "INSERT INTO orchestration_budgets (
                         run_id, principal_id, tenant_id, coordinator_workspace_id,
                         ceiling_json, ledger_json, created_at, updated_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                    params![
                        run_id.as_str(),
                        principal_id.as_str(),
                        tenant_id.as_str(),
                        coordinator_workspace_id.as_str(),
                        serde_json::to_string(&aggregate_budget.ceiling())?,
                        serde_json::to_string(&OrchestrationBudgetLedger::default())?,
                        to_i64(created_at.as_unix_seconds())?,
                    ],
                )?;
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

    pub fn inspect_budget(
        &self,
        run_id: &OrchestrationRunId,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        coordinator_workspace_id: &WorkspaceId,
    ) -> Result<OrchestrationBudgetSnapshot, OrchestrationStoreError> {
        let connection = self.lock()?;
        load_budget_snapshot(
            &connection,
            run_id,
            principal_id,
            tenant_id,
            coordinator_workspace_id,
        )
    }

    pub fn list_budgets(
        &self,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        coordinator_workspace_id: &WorkspaceId,
    ) -> Result<Vec<OrchestrationBudgetSnapshot>, OrchestrationStoreError> {
        let connection = self.lock()?;
        let run_ids = {
            let mut statement = connection.prepare(
                "SELECT run_id FROM orchestration_budgets
                 WHERE principal_id = ?1 AND tenant_id = ?2
                   AND coordinator_workspace_id = ?3
                 ORDER BY updated_at DESC, run_id ASC",
            )?;
            let rows = statement.query_map(
                params![
                    principal_id.as_str(),
                    tenant_id.as_str(),
                    coordinator_workspace_id.as_str(),
                ],
                |row| row.get::<_, String>(0),
            )?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        run_ids
            .into_iter()
            .map(|run_id| {
                let run_id = OrchestrationRunId::new(run_id)
                    .map_err(|_| OrchestrationStoreError::InvalidIdentifier)?;
                load_budget_snapshot(
                    &connection,
                    &run_id,
                    principal_id,
                    tenant_id,
                    coordinator_workspace_id,
                )
            })
            .collect()
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
        let aggregate_budget = current
            .plan
            .aggregate_budget()
            .ok_or(OrchestrationStoreError::AggregateBudgetRequired)?;
        for assignment in &ready {
            let reservation = aggregate_budget
                .reservation_for_role(assignment.id())
                .ok_or(OrchestrationStoreError::CorruptRecord)?;
            reserve_role_budget(
                &transaction,
                run_id,
                principal_id,
                tenant_id,
                coordinator_workspace_id,
                assignment.id(),
                reservation,
                now,
            )?;
        }
        let snapshot_json = serde_json::to_string(&run.snapshot())?;
        let changed = transaction.execute(
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
        if changed != 1 {
            return Err(OrchestrationStoreError::CorruptRecord);
        }
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
        settle_role_budget(
            &transaction,
            run_id,
            principal_id,
            tenant_id,
            coordinator_workspace_id,
            role_id,
            receipt,
            now,
        )?;
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
        let changed = transaction.execute(
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
        if changed != 1 {
            return Err(OrchestrationStoreError::CorruptRecord);
        }
        transaction.commit()?;
        current.status = OrchestrationRunStatus::Interrupted;
        current.interruption_reason = Some(reason.to_owned());
        current.updated_at = now;
        Ok(current)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn reconcile_interrupted_role(
        &self,
        run_id: &OrchestrationRunId,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        coordinator_workspace_id: &WorkspaceId,
        role_id: &RoleId,
        usage: &OrchestrationUsage,
        evidence_digest: &RequestDigest,
        now: Timestamp,
    ) -> Result<OrchestrationRunRecord, OrchestrationStoreError> {
        usage.validate()?;
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
                action: "reconcile a role in",
            });
        }
        let mut run = OrchestrationRun::from_snapshot(current.snapshot.clone())?;
        run.release_active(role_id)?;
        reconcile_failed_role_budget(
            &transaction,
            run_id,
            principal_id,
            tenant_id,
            coordinator_workspace_id,
            role_id,
            usage,
            evidence_digest,
            now,
        )?;
        current.snapshot = run.snapshot();
        current.updated_at = now;
        let changed = transaction.execute(
            "UPDATE orchestration_runs SET snapshot_json = ?1, updated_at = ?2
             WHERE run_id = ?3 AND principal_id = ?4 AND tenant_id = ?5
               AND coordinator_workspace_id = ?6 AND status = 'interrupted'",
            params![
                serde_json::to_string(&current.snapshot)?,
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
        transaction.commit()?;
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
        let changed = transaction.execute(
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
        if changed != 1 {
            return Err(OrchestrationStoreError::CorruptRecord);
        }
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
        let changed = transaction.execute(
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
        if changed != 1 {
            return Err(OrchestrationStoreError::CorruptRecord);
        }
        release_active_budget_reservations(
            &transaction,
            run_id,
            principal_id,
            tenant_id,
            coordinator_workspace_id,
            now,
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

fn current_orchestration_schema_version(
    connection: &Connection,
) -> Result<Option<i64>, OrchestrationStoreError> {
    let marker_exists = connection.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM sqlite_master
             WHERE type = 'table' AND name = 'pandora_schema_versions'
         )",
        [],
        |row| row.get::<_, bool>(0),
    )?;
    if !marker_exists {
        return Ok(None);
    }
    let version = connection
        .query_row(
            "SELECT version FROM pandora_schema_versions WHERE component = ?1",
            params![ORCHESTRATION_SCHEMA_COMPONENT],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    if let Some(version) = version
        && version != ORCHESTRATION_SCHEMA_VERSION
    {
        return Err(OrchestrationStoreError::CorruptRecord);
    }
    if version.is_some() {
        for table in [
            "orchestration_runs",
            "orchestration_budgets",
            "orchestration_budget_reservations",
            "orchestration_budget_reconciliations",
        ] {
            let exists = connection.query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
                 )",
                params![table],
                |row| row.get::<_, bool>(0),
            )?;
            if !exists {
                return Err(OrchestrationStoreError::CorruptRecord);
            }
        }
    }
    Ok(version)
}

fn load_budget_snapshot(
    connection: &Connection,
    run_id: &OrchestrationRunId,
    principal_id: &PrincipalId,
    tenant_id: &TenantId,
    coordinator_workspace_id: &WorkspaceId,
) -> Result<OrchestrationBudgetSnapshot, OrchestrationStoreError> {
    let (ceiling, ledger, created_at, updated_at) = load_budget_state(
        connection,
        run_id,
        principal_id,
        tenant_id,
        coordinator_workspace_id,
    )?;
    let reservations = load_budget_reservations(
        connection,
        run_id,
        principal_id,
        tenant_id,
        coordinator_workspace_id,
    )?;
    let reserved = sum_active_reservations(&reservations, None)?;
    let committed = ledger
        .enforced_consumed
        .checked_add(reserved)
        .ok_or(OrchestrationStoreError::AggregateBudgetOverflow)?;
    ensure_aggregate_within(committed, ceiling)?;
    let enforced_remaining = subtract_budget(ceiling, committed);
    Ok(OrchestrationBudgetSnapshot {
        run_id: run_id.clone(),
        principal_id: principal_id.clone(),
        tenant_id: tenant_id.clone(),
        coordinator_workspace_id: coordinator_workspace_id.clone(),
        ceiling,
        reserved,
        enforced_consumed: ledger.enforced_consumed,
        enforced_remaining,
        actual_cost_micros: (ledger.unknown_cost_receipts == 0).then_some(ledger.known_cost_micros),
        known_cost_micros: ledger.known_cost_micros,
        unknown_cost_receipts: ledger.unknown_cost_receipts,
        reservations,
        created_at,
        updated_at,
    })
}

fn load_budget_state(
    connection: &Connection,
    run_id: &OrchestrationRunId,
    principal_id: &PrincipalId,
    tenant_id: &TenantId,
    coordinator_workspace_id: &WorkspaceId,
) -> Result<
    (
        OrchestrationBudgetAmount,
        OrchestrationBudgetLedger,
        Timestamp,
        Timestamp,
    ),
    OrchestrationStoreError,
> {
    let row = connection
        .query_row(
            "SELECT ceiling_json, ledger_json, created_at, updated_at
             FROM orchestration_budgets
             WHERE run_id = ?1 AND principal_id = ?2 AND tenant_id = ?3
               AND coordinator_workspace_id = ?4",
            params![
                run_id.as_str(),
                principal_id.as_str(),
                tenant_id.as_str(),
                coordinator_workspace_id.as_str(),
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()?
        .ok_or(OrchestrationStoreError::AggregateBudgetNotFound)?;
    Ok((
        serde_json::from_str(&row.0)?,
        serde_json::from_str(&row.1)?,
        decode_timestamp(row.2)?,
        decode_timestamp(row.3)?,
    ))
}

fn load_budget_reservations(
    connection: &Connection,
    run_id: &OrchestrationRunId,
    principal_id: &PrincipalId,
    tenant_id: &TenantId,
    coordinator_workspace_id: &WorkspaceId,
) -> Result<Vec<OrchestrationBudgetReservationRecord>, OrchestrationStoreError> {
    let mut statement = connection.prepare(
        "SELECT role_id, reservation_json, state, completion_receipt_id,
                reconciliation_evidence_digest, usage_json, reserved_at, updated_at
         FROM orchestration_budget_reservations
         WHERE run_id = ?1 AND principal_id = ?2 AND tenant_id = ?3
           AND coordinator_workspace_id = ?4
         ORDER BY role_id ASC",
    )?;
    let mut rows = statement.query(params![
        run_id.as_str(),
        principal_id.as_str(),
        tenant_id.as_str(),
        coordinator_workspace_id.as_str(),
    ])?;
    let mut reservations = Vec::new();
    while let Some(row) = rows.next()? {
        let reconciliation_evidence_digest = row
            .get::<_, Option<String>>(4)?
            .map(RequestDigest::new)
            .transpose()
            .map_err(|_| OrchestrationStoreError::InvalidIdentifier)?;
        let usage_json = row.get::<_, Option<String>>(5)?;
        reservations.push(OrchestrationBudgetReservationRecord {
            role_id: RoleId::new(row.get::<_, String>(0)?)
                .map_err(|_| OrchestrationStoreError::InvalidIdentifier)?,
            reservation: serde_json::from_str(&row.get::<_, String>(1)?)?,
            state: decode_budget_reservation_state(&row.get::<_, String>(2)?)?,
            completion_receipt_id: row.get(3)?,
            reconciliation_evidence_digest,
            usage: usage_json
                .as_deref()
                .map(serde_json::from_str)
                .transpose()?,
            reserved_at: decode_timestamp(row.get(6)?)?,
            updated_at: decode_timestamp(row.get(7)?)?,
        });
    }
    Ok(reservations)
}

#[allow(clippy::too_many_arguments)]
fn reserve_role_budget(
    transaction: &rusqlite::Transaction<'_>,
    run_id: &OrchestrationRunId,
    principal_id: &PrincipalId,
    tenant_id: &TenantId,
    coordinator_workspace_id: &WorkspaceId,
    role_id: &RoleId,
    reservation: OrchestrationBudgetAmount,
    now: Timestamp,
) -> Result<(), OrchestrationStoreError> {
    let (ceiling, ledger, _, _) = load_budget_state(
        transaction,
        run_id,
        principal_id,
        tenant_id,
        coordinator_workspace_id,
    )?;
    let reservations = load_budget_reservations(
        transaction,
        run_id,
        principal_id,
        tenant_id,
        coordinator_workspace_id,
    )?;
    let existing = reservations.iter().find(|item| item.role_id() == role_id);
    if existing.is_some_and(|item| item.state() != OrchestrationBudgetReservationState::Released) {
        return Err(OrchestrationStoreError::BudgetReservationNotActive);
    }
    let active = sum_active_reservations(&reservations, None)?;
    let committed = ledger
        .enforced_consumed
        .checked_add(active)
        .and_then(|amount| amount.checked_add(reservation))
        .ok_or(OrchestrationStoreError::AggregateBudgetOverflow)?;
    ensure_aggregate_within(committed, ceiling)?;
    if existing.is_some() {
        let changed = transaction.execute(
            "UPDATE orchestration_budget_reservations
             SET reservation_json = ?1, state = 'active', completion_receipt_id = NULL,
                 reconciliation_evidence_digest = NULL, usage_json = NULL,
                 reserved_at = ?2, updated_at = ?2
             WHERE run_id = ?3 AND principal_id = ?4 AND tenant_id = ?5
               AND coordinator_workspace_id = ?6 AND role_id = ?7 AND state = 'released'",
            params![
                serde_json::to_string(&reservation)?,
                to_i64(now.as_unix_seconds())?,
                run_id.as_str(),
                principal_id.as_str(),
                tenant_id.as_str(),
                coordinator_workspace_id.as_str(),
                role_id.as_str(),
            ],
        )?;
        if changed != 1 {
            return Err(OrchestrationStoreError::BudgetReservationNotActive);
        }
    } else {
        transaction.execute(
            "INSERT INTO orchestration_budget_reservations (
                 run_id, principal_id, tenant_id, coordinator_workspace_id, role_id,
                 reservation_json, state, reserved_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7, ?7)",
            params![
                run_id.as_str(),
                principal_id.as_str(),
                tenant_id.as_str(),
                coordinator_workspace_id.as_str(),
                role_id.as_str(),
                serde_json::to_string(&reservation)?,
                to_i64(now.as_unix_seconds())?,
            ],
        )?;
    }
    transaction.execute(
        "UPDATE orchestration_budgets SET updated_at = ?1
         WHERE run_id = ?2 AND principal_id = ?3 AND tenant_id = ?4
           AND coordinator_workspace_id = ?5",
        params![
            to_i64(now.as_unix_seconds())?,
            run_id.as_str(),
            principal_id.as_str(),
            tenant_id.as_str(),
            coordinator_workspace_id.as_str(),
        ],
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn settle_role_budget(
    transaction: &rusqlite::Transaction<'_>,
    run_id: &OrchestrationRunId,
    principal_id: &PrincipalId,
    tenant_id: &TenantId,
    coordinator_workspace_id: &WorkspaceId,
    role_id: &RoleId,
    receipt: &OrchestrationRoleReceipt,
    now: Timestamp,
) -> Result<(), OrchestrationStoreError> {
    let usage = receipt
        .usage()
        .ok_or(OrchestrationStoreError::UsageRequired)?;
    let reservations = load_budget_reservations(
        transaction,
        run_id,
        principal_id,
        tenant_id,
        coordinator_workspace_id,
    )?;
    let current = reservations
        .iter()
        .find(|item| item.role_id() == role_id)
        .ok_or(OrchestrationStoreError::BudgetReservationNotFound)?;
    if current.state() != OrchestrationBudgetReservationState::Active {
        return Err(OrchestrationStoreError::BudgetReservationNotActive);
    }
    let reservation = current.reservation();
    let enforced_usage = OrchestrationBudgetAmount::new(
        usage.tokens(),
        usage.tools(),
        usage.elapsed_ms(),
        usage.cost_micros().unwrap_or(reservation.cost_micros()),
    );
    ensure_usage_within(enforced_usage, reservation)?;
    let (ceiling, mut ledger, _, _) = load_budget_state(
        transaction,
        run_id,
        principal_id,
        tenant_id,
        coordinator_workspace_id,
    )?;
    let other_active = sum_active_reservations(&reservations, Some(role_id))?;
    ledger.enforced_consumed = ledger
        .enforced_consumed
        .checked_add(enforced_usage)
        .ok_or(OrchestrationStoreError::AggregateBudgetOverflow)?;
    match usage.cost_micros() {
        Some(cost) => {
            ledger.known_cost_micros = ledger
                .known_cost_micros
                .checked_add(cost)
                .ok_or(OrchestrationStoreError::AggregateBudgetOverflow)?;
        }
        None => {
            ledger.unknown_cost_receipts = ledger
                .unknown_cost_receipts
                .checked_add(1)
                .ok_or(OrchestrationStoreError::AggregateBudgetOverflow)?;
        }
    }
    let committed = ledger
        .enforced_consumed
        .checked_add(other_active)
        .ok_or(OrchestrationStoreError::AggregateBudgetOverflow)?;
    ensure_aggregate_within(committed, ceiling)?;
    let changed = transaction.execute(
        "UPDATE orchestration_budget_reservations
         SET state = 'settled', completion_receipt_id = ?1, usage_json = ?2, updated_at = ?3
         WHERE run_id = ?4 AND principal_id = ?5 AND tenant_id = ?6
           AND coordinator_workspace_id = ?7 AND role_id = ?8 AND state = 'active'",
        params![
            receipt.receipt_id().as_str(),
            serde_json::to_string(usage)?,
            to_i64(now.as_unix_seconds())?,
            run_id.as_str(),
            principal_id.as_str(),
            tenant_id.as_str(),
            coordinator_workspace_id.as_str(),
            role_id.as_str(),
        ],
    )?;
    if changed != 1 {
        return Err(OrchestrationStoreError::BudgetReservationNotActive);
    }
    let changed = transaction.execute(
        "UPDATE orchestration_budgets SET ledger_json = ?1, updated_at = ?2
         WHERE run_id = ?3 AND principal_id = ?4 AND tenant_id = ?5
           AND coordinator_workspace_id = ?6",
        params![
            serde_json::to_string(&ledger)?,
            to_i64(now.as_unix_seconds())?,
            run_id.as_str(),
            principal_id.as_str(),
            tenant_id.as_str(),
            coordinator_workspace_id.as_str(),
        ],
    )?;
    if changed != 1 {
        return Err(OrchestrationStoreError::AggregateBudgetNotFound);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn reconcile_failed_role_budget(
    transaction: &rusqlite::Transaction<'_>,
    run_id: &OrchestrationRunId,
    principal_id: &PrincipalId,
    tenant_id: &TenantId,
    coordinator_workspace_id: &WorkspaceId,
    role_id: &RoleId,
    usage: &OrchestrationUsage,
    evidence_digest: &RequestDigest,
    now: Timestamp,
) -> Result<(), OrchestrationStoreError> {
    let reservations = load_budget_reservations(
        transaction,
        run_id,
        principal_id,
        tenant_id,
        coordinator_workspace_id,
    )?;
    let current = reservations
        .iter()
        .find(|item| item.role_id() == role_id)
        .ok_or(OrchestrationStoreError::BudgetReservationNotFound)?;
    if current.state() != OrchestrationBudgetReservationState::Active {
        return Err(OrchestrationStoreError::BudgetReservationNotActive);
    }
    let reservation = current.reservation();
    let enforced_usage = OrchestrationBudgetAmount::new(
        usage.tokens(),
        usage.tools(),
        usage.elapsed_ms(),
        usage.cost_micros().unwrap_or(reservation.cost_micros()),
    );
    ensure_usage_within(enforced_usage, reservation)?;
    let (ceiling, mut ledger, _, _) = load_budget_state(
        transaction,
        run_id,
        principal_id,
        tenant_id,
        coordinator_workspace_id,
    )?;
    let other_active = sum_active_reservations(&reservations, Some(role_id))?;
    ledger.enforced_consumed = ledger
        .enforced_consumed
        .checked_add(enforced_usage)
        .ok_or(OrchestrationStoreError::AggregateBudgetOverflow)?;
    match usage.cost_micros() {
        Some(cost) => {
            ledger.known_cost_micros = ledger
                .known_cost_micros
                .checked_add(cost)
                .ok_or(OrchestrationStoreError::AggregateBudgetOverflow)?;
        }
        None => {
            ledger.unknown_cost_receipts = ledger
                .unknown_cost_receipts
                .checked_add(1)
                .ok_or(OrchestrationStoreError::AggregateBudgetOverflow)?;
        }
    }
    let committed = ledger
        .enforced_consumed
        .checked_add(other_active)
        .ok_or(OrchestrationStoreError::AggregateBudgetOverflow)?;
    ensure_aggregate_within(committed, ceiling)?;
    transaction.execute(
        "INSERT INTO orchestration_budget_reconciliations (
             run_id, principal_id, tenant_id, coordinator_workspace_id, role_id,
             evidence_digest, usage_json, recorded_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            run_id.as_str(),
            principal_id.as_str(),
            tenant_id.as_str(),
            coordinator_workspace_id.as_str(),
            role_id.as_str(),
            evidence_digest.as_str(),
            serde_json::to_string(usage)?,
            to_i64(now.as_unix_seconds())?,
        ],
    )?;
    let changed = transaction.execute(
        "UPDATE orchestration_budget_reservations
         SET state = 'released', reconciliation_evidence_digest = ?1,
             usage_json = ?2, updated_at = ?3
         WHERE run_id = ?4 AND principal_id = ?5 AND tenant_id = ?6
           AND coordinator_workspace_id = ?7 AND role_id = ?8 AND state = 'active'",
        params![
            evidence_digest.as_str(),
            serde_json::to_string(usage)?,
            to_i64(now.as_unix_seconds())?,
            run_id.as_str(),
            principal_id.as_str(),
            tenant_id.as_str(),
            coordinator_workspace_id.as_str(),
            role_id.as_str(),
        ],
    )?;
    if changed != 1 {
        return Err(OrchestrationStoreError::BudgetReservationNotActive);
    }
    let changed = transaction.execute(
        "UPDATE orchestration_budgets SET ledger_json = ?1, updated_at = ?2
         WHERE run_id = ?3 AND principal_id = ?4 AND tenant_id = ?5
           AND coordinator_workspace_id = ?6",
        params![
            serde_json::to_string(&ledger)?,
            to_i64(now.as_unix_seconds())?,
            run_id.as_str(),
            principal_id.as_str(),
            tenant_id.as_str(),
            coordinator_workspace_id.as_str(),
        ],
    )?;
    if changed != 1 {
        return Err(OrchestrationStoreError::AggregateBudgetNotFound);
    }
    Ok(())
}

fn release_active_budget_reservations(
    transaction: &rusqlite::Transaction<'_>,
    run_id: &OrchestrationRunId,
    principal_id: &PrincipalId,
    tenant_id: &TenantId,
    coordinator_workspace_id: &WorkspaceId,
    now: Timestamp,
) -> Result<(), OrchestrationStoreError> {
    transaction.execute(
        "UPDATE orchestration_budget_reservations
         SET state = 'released', updated_at = ?1
         WHERE run_id = ?2 AND principal_id = ?3 AND tenant_id = ?4
           AND coordinator_workspace_id = ?5 AND state = 'active'",
        params![
            to_i64(now.as_unix_seconds())?,
            run_id.as_str(),
            principal_id.as_str(),
            tenant_id.as_str(),
            coordinator_workspace_id.as_str(),
        ],
    )?;
    transaction.execute(
        "UPDATE orchestration_budgets SET updated_at = ?1
         WHERE run_id = ?2 AND principal_id = ?3 AND tenant_id = ?4
           AND coordinator_workspace_id = ?5",
        params![
            to_i64(now.as_unix_seconds())?,
            run_id.as_str(),
            principal_id.as_str(),
            tenant_id.as_str(),
            coordinator_workspace_id.as_str(),
        ],
    )?;
    Ok(())
}

fn sum_active_reservations(
    reservations: &[OrchestrationBudgetReservationRecord],
    excluded_role: Option<&RoleId>,
) -> Result<OrchestrationBudgetAmount, OrchestrationStoreError> {
    reservations
        .iter()
        .filter(|item| {
            item.state() == OrchestrationBudgetReservationState::Active
                && excluded_role != Some(item.role_id())
        })
        .try_fold(OrchestrationBudgetAmount::default(), |total, item| {
            total
                .checked_add(item.reservation())
                .ok_or(OrchestrationStoreError::AggregateBudgetOverflow)
        })
}

fn ensure_usage_within(
    usage: OrchestrationBudgetAmount,
    reservation: OrchestrationBudgetAmount,
) -> Result<(), OrchestrationStoreError> {
    if let Some(resource) = exceeded_resource(usage, reservation) {
        return Err(OrchestrationStoreError::UsageExceedsReservation(resource));
    }
    Ok(())
}

fn ensure_aggregate_within(
    committed: OrchestrationBudgetAmount,
    ceiling: OrchestrationBudgetAmount,
) -> Result<(), OrchestrationStoreError> {
    if let Some(resource) = exceeded_resource(committed, ceiling) {
        return Err(OrchestrationStoreError::AggregateBudgetExceeded(resource));
    }
    Ok(())
}

fn exceeded_resource(
    amount: OrchestrationBudgetAmount,
    ceiling: OrchestrationBudgetAmount,
) -> Option<&'static str> {
    if amount.tokens() > ceiling.tokens() {
        Some("token")
    } else if amount.tools() > ceiling.tools() {
        Some("tool")
    } else if amount.elapsed_ms() > ceiling.elapsed_ms() {
        Some("elapsed-time")
    } else if amount.cost_micros() > ceiling.cost_micros() {
        Some("cost")
    } else {
        None
    }
}

fn subtract_budget(
    ceiling: OrchestrationBudgetAmount,
    amount: OrchestrationBudgetAmount,
) -> OrchestrationBudgetAmount {
    OrchestrationBudgetAmount::new(
        ceiling.tokens() - amount.tokens(),
        ceiling.tools() - amount.tools(),
        ceiling.elapsed_ms() - amount.elapsed_ms(),
        ceiling.cost_micros() - amount.cost_micros(),
    )
}

fn decode_budget_reservation_state(
    value: &str,
) -> Result<OrchestrationBudgetReservationState, OrchestrationStoreError> {
    match value {
        "active" => Ok(OrchestrationBudgetReservationState::Active),
        "settled" => Ok(OrchestrationBudgetReservationState::Settled),
        "released" => Ok(OrchestrationBudgetReservationState::Released),
        _ => Err(OrchestrationStoreError::CorruptRecord),
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
    let role_receipts: Vec<OrchestrationRoleReceipt> =
        serde_json::from_str(&row.get::<_, String>(8)?)?;
    let interruption_reason: Option<String> = row.get(9)?;
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
        GovernedOrchestrationPlan, Handoff, HarnessId, MetaComposition,
        OrchestrationAggregateBudget, OrchestrationBudgetAmount, OrchestrationPlan,
        OrchestrationRole, OrchestrationRoleBudget, OrchestrationUsage, PlanId, ReceiptId,
        RepositoryBinding, RepositoryId, RequestDigest, RoleRepositoryBinding,
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
            self.receipt_with_cost(run_id, role, repository, workspace, commit, Some(10))
        }

        fn receipt_with_cost(
            &self,
            run_id: &OrchestrationRunId,
            role: &str,
            repository: &str,
            workspace: &str,
            commit: &str,
            cost_micros: Option<u64>,
        ) -> OrchestrationRoleReceipt {
            let effect_receipt = ReceiptId::new(format!("effect-{role}")).unwrap();
            OrchestrationRoleReceipt::new(
                ReceiptId::new(format!("receipt-{role}")).unwrap(),
                run_id.clone(),
                RoleId::new(role).unwrap(),
                RepositoryId::new(repository).unwrap(),
                WorkspaceId::new(workspace).unwrap(),
                commit,
                vec![effect_receipt.clone()],
                Some(RequestDigest::new(format!("evidence-{role}")).unwrap()),
            )
            .unwrap()
            .with_usage(
                OrchestrationUsage::new(25, 2, 200, cost_micros, vec![effect_receipt]).unwrap(),
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
        .with_aggregate_budget(
            OrchestrationAggregateBudget::new(
                OrchestrationBudgetAmount::new(200, 20, 2_000, 200),
                vec![
                    OrchestrationRoleBudget::new(
                        RoleId::new("planner").unwrap(),
                        OrchestrationBudgetAmount::new(100, 10, 1_000, 100),
                    ),
                    OrchestrationRoleBudget::new(
                        RoleId::new("maker").unwrap(),
                        OrchestrationBudgetAmount::new(100, 10, 1_000, 100),
                    ),
                ],
            )
            .unwrap(),
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
        let budget = fixture
            .store
            .inspect_budget(
                &run_id,
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
            )
            .unwrap();
        assert_eq!(budget.reserved().tokens(), 100);
        assert_eq!(budget.enforced_consumed().tokens(), 0);

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
        let budget = fixture
            .store
            .inspect_budget(
                &run_id,
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
            )
            .unwrap();
        assert_eq!(budget.reserved(), OrchestrationBudgetAmount::default());
        assert_eq!(
            budget.enforced_consumed(),
            OrchestrationBudgetAmount::new(50, 4, 400, 20)
        );
        assert_eq!(budget.actual_cost_micros(), Some(20));
        assert_eq!(budget.unknown_cost_receipts(), 0);

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
        assert_eq!(
            reopened
                .inspect_budget(
                    &run_id,
                    &fixture.principal,
                    &fixture.tenant,
                    &fixture.coordinator_workspace,
                )
                .unwrap()
                .enforced_remaining(),
            OrchestrationBudgetAmount::new(150, 16, 1_600, 180)
        );
    }

    #[test]
    fn partial_multi_repository_failure_preserves_receipts_and_requires_reconciliation() {
        let fixture = Fixture::new();
        let run_id = fixture.submit("run-partial", 10);
        fixture.claim(20);
        fixture
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
                Timestamp::from_unix_seconds(40),
            )
            .unwrap();
        fixture
            .store
            .start_ready(
                &run_id,
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
                &fixture.worker,
                Timestamp::from_unix_seconds(50),
            )
            .unwrap();

        let wrong_maker = fixture.receipt(&run_id, "maker", "api", "workspace-api", "commit-api");
        assert!(matches!(
            fixture.store.complete_role(
                &run_id,
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
                &fixture.worker,
                &RoleId::new("maker").unwrap(),
                &wrong_maker,
                Timestamp::from_unix_seconds(60),
            ),
            Err(OrchestrationStoreError::Contract(
                WorkspaceOrchestrationError::ReceiptRepositoryMismatch(_)
            ))
        ));

        let interrupted = fixture
            .store
            .mark_interrupted(
                &run_id,
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
                "maker repository failed after planner completed",
                Timestamp::from_unix_seconds(70),
            )
            .unwrap();
        assert_eq!(interrupted.status(), OrchestrationRunStatus::Interrupted);
        assert_eq!(interrupted.role_receipts().len(), 1);
        assert_eq!(interrupted.snapshot().completed_roles().len(), 1);
        assert_eq!(interrupted.snapshot().active_roles().len(), 1);

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
        assert_eq!(persisted.status(), OrchestrationRunStatus::Interrupted);
        assert_eq!(persisted.role_receipts().len(), 1);
        assert_eq!(persisted.snapshot().completed_roles().len(), 1);
        assert_eq!(persisted.snapshot().active_roles().len(), 1);
        let budget = reopened
            .inspect_budget(
                &run_id,
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
            )
            .unwrap();
        assert_eq!(budget.enforced_consumed().tokens(), 25);
        assert_eq!(budget.reserved().tokens(), 100);
        assert!(
            budget
                .enforced_consumed()
                .checked_add(budget.reserved())
                .unwrap()
                .fits_within(budget.ceiling())
        );
        assert!(matches!(
            reopened.resume(
                &run_id,
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
                Timestamp::from_unix_seconds(80),
            ),
            Err(OrchestrationStoreError::ActiveRolesRequireReconciliation)
        ));

        let failed_usage = OrchestrationUsage::new(
            5,
            1,
            50,
            Some(1),
            vec![ReceiptId::new("maker-failure-provider-receipt").unwrap()],
        )
        .unwrap();
        reopened
            .reconcile_interrupted_role(
                &run_id,
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
                &RoleId::new("maker").unwrap(),
                &failed_usage,
                &RequestDigest::new("maker-failure-reconciliation").unwrap(),
                Timestamp::from_unix_seconds(90),
            )
            .unwrap();
        let reconciled_budget = reopened
            .inspect_budget(
                &run_id,
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
            )
            .unwrap();
        assert_eq!(
            reconciled_budget.reserved(),
            OrchestrationBudgetAmount::default()
        );
        assert_eq!(reconciled_budget.enforced_consumed().tokens(), 30);
        assert_eq!(
            reconciled_budget
                .reservations()
                .iter()
                .find(|reservation| reservation.role_id().as_str() == "maker")
                .unwrap()
                .state(),
            OrchestrationBudgetReservationState::Released
        );
        assert!(matches!(
            reopened.reconcile_interrupted_role(
                &run_id,
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
                &RoleId::new("maker").unwrap(),
                &failed_usage,
                &RequestDigest::new("maker-failure-reconciliation").unwrap(),
                Timestamp::from_unix_seconds(91),
            ),
            Err(OrchestrationStoreError::Orchestration(
                OrchestrationError::RoleNotActive(_)
            ))
        ));
        assert_eq!(
            reopened
                .inspect_budget(
                    &run_id,
                    &fixture.principal,
                    &fixture.tenant,
                    &fixture.coordinator_workspace,
                )
                .unwrap()
                .enforced_consumed()
                .tokens(),
            30
        );
        assert_eq!(
            reopened
                .resume(
                    &run_id,
                    &fixture.principal,
                    &fixture.tenant,
                    &fixture.coordinator_workspace,
                    Timestamp::from_unix_seconds(100),
                )
                .unwrap()
                .status(),
            OrchestrationRunStatus::Queued
        );
        reopened
            .claim_next(
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
                &fixture.worker,
                Timestamp::from_unix_seconds(110),
            )
            .unwrap()
            .unwrap();
        let retried = reopened
            .start_ready(
                &run_id,
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
                &fixture.worker,
                Timestamp::from_unix_seconds(120),
            )
            .unwrap();
        assert_eq!(retried.len(), 1);
        assert_eq!(retried[0].id().as_str(), "maker");
        let retry_budget = reopened
            .inspect_budget(
                &run_id,
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
            )
            .unwrap();
        assert_eq!(retry_budget.enforced_consumed().tokens(), 30);
        assert_eq!(retry_budget.reserved().tokens(), 100);
        let reconciliation_count = reopened
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM orchestration_budget_reconciliations WHERE run_id = ?1",
                params![run_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(reconciliation_count, 1);
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

    #[test]
    fn unknown_cost_remains_unknown_and_consumes_the_full_cost_reservation() {
        let fixture = Fixture::new();
        let run_id = fixture.submit("run-unknown-cost", 10);
        fixture.claim(20);
        fixture
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
        let receipt = fixture.receipt_with_cost(
            &run_id,
            "planner",
            "api",
            "workspace-api",
            "commit-api",
            None,
        );
        fixture
            .store
            .complete_role(
                &run_id,
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
                &fixture.worker,
                &RoleId::new("planner").unwrap(),
                &receipt,
                Timestamp::from_unix_seconds(40),
            )
            .unwrap();

        let budget = fixture
            .store
            .inspect_budget(
                &run_id,
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
            )
            .unwrap();
        assert_eq!(budget.actual_cost_micros(), None);
        assert_eq!(budget.known_cost_micros(), 0);
        assert_eq!(budget.unknown_cost_receipts(), 1);
        assert_eq!(budget.enforced_consumed().cost_micros(), 100);
        assert_eq!(budget.enforced_remaining().cost_micros(), 100);
    }

    #[test]
    fn concurrent_workers_cannot_duplicate_or_oversubscribe_a_role_reservation() {
        let fixture = Fixture::new();
        let run_id = fixture.submit("run-concurrent-budget", 10);
        fixture.claim(20);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let database = fixture.root.join("orchestration.sqlite3");
        let principal = fixture.principal.clone();
        let tenant = fixture.tenant.clone();
        let workspace = fixture.coordinator_workspace.clone();
        let worker = fixture.worker.clone();
        let results = std::thread::scope(|scope| {
            let handles = (0..2)
                .map(|_| {
                    let barrier = std::sync::Arc::clone(&barrier);
                    let database = database.clone();
                    let principal = principal.clone();
                    let tenant = tenant.clone();
                    let workspace = workspace.clone();
                    let worker = worker.clone();
                    let run_id = run_id.clone();
                    scope.spawn(move || {
                        let store = OrchestrationStore::open(database).unwrap();
                        barrier.wait();
                        store
                            .start_ready(
                                &run_id,
                                &principal,
                                &tenant,
                                &workspace,
                                &worker,
                                Timestamp::from_unix_seconds(30),
                            )
                            .unwrap()
                            .len()
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(results.into_iter().sum::<usize>(), 1);
        let budget = fixture
            .store
            .inspect_budget(
                &run_id,
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
            )
            .unwrap();
        assert_eq!(budget.reserved().tokens(), 100);
        assert_eq!(
            budget
                .reservations()
                .iter()
                .filter(|reservation| {
                    reservation.state() == OrchestrationBudgetReservationState::Active
                })
                .count(),
            1
        );
        assert!(
            budget
                .enforced_consumed()
                .checked_add(budget.reserved())
                .unwrap()
                .fits_within(budget.ceiling())
        );
    }

    #[test]
    fn concurrent_duplicate_completions_settle_usage_exactly_once() {
        let fixture = Fixture::new();
        let run_id = fixture.submit("run-concurrent-completion", 10);
        fixture.claim(20);
        fixture
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
        let receipt = fixture.receipt(&run_id, "planner", "api", "workspace-api", "commit-api");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let database = fixture.root.join("orchestration.sqlite3");
        let results = std::thread::scope(|scope| {
            let handles = (0..2)
                .map(|_| {
                    let barrier = std::sync::Arc::clone(&barrier);
                    let database = database.clone();
                    let principal = fixture.principal.clone();
                    let tenant = fixture.tenant.clone();
                    let workspace = fixture.coordinator_workspace.clone();
                    let worker = fixture.worker.clone();
                    let run_id = run_id.clone();
                    let receipt = receipt.clone();
                    scope.spawn(move || {
                        let store = OrchestrationStore::open(database).unwrap();
                        barrier.wait();
                        store
                            .complete_role(
                                &run_id,
                                &principal,
                                &tenant,
                                &workspace,
                                &worker,
                                &RoleId::new("planner").unwrap(),
                                &receipt,
                                Timestamp::from_unix_seconds(40),
                            )
                            .is_ok()
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(
            results.into_iter().filter(|succeeded| *succeeded).count(),
            1
        );

        let persisted = fixture
            .store
            .inspect(
                &run_id,
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
            )
            .unwrap();
        assert_eq!(persisted.role_receipts().len(), 1);
        let budget = fixture
            .store
            .inspect_budget(
                &run_id,
                &fixture.principal,
                &fixture.tenant,
                &fixture.coordinator_workspace,
            )
            .unwrap();
        assert_eq!(
            budget.enforced_consumed(),
            OrchestrationBudgetAmount::new(25, 2, 200, 10)
        );
        assert_eq!(budget.known_cost_micros(), 10);
        assert_eq!(
            budget
                .reservations()
                .iter()
                .filter(|reservation| {
                    reservation.state() == OrchestrationBudgetReservationState::Settled
                })
                .count(),
            1
        );
    }
}
