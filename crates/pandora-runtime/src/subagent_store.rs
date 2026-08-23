use crate::job_store::{JobStoreError, open_job_connection};
use pandora_types::{
    EffectOutcome, EffectReceipt, ExecutionId, IdError, JobCommand, JobId, JobRequest, JobStatus,
    JobWorkerId, MAX_SUBAGENT_RESULT_BYTES, PermitId, PrincipalId, ReceiptId, RequestDigest,
    SessionId, SubagentId, SubagentRequest, SubagentStatus, SubagentWorktreeState, TenantId,
    Timestamp, WorkspaceId,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubagentScope {
    principal_id: PrincipalId,
    tenant_id: TenantId,
    workspace_id: WorkspaceId,
}

impl SubagentScope {
    pub fn new(principal_id: PrincipalId, tenant_id: TenantId, workspace_id: WorkspaceId) -> Self {
        Self {
            principal_id,
            tenant_id,
            workspace_id,
        }
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubagentPreparation {
    id: SubagentId,
    job_id: JobId,
    scope: SubagentScope,
    child_session_id: SessionId,
    child_execution_id: ExecutionId,
    request: SubagentRequest,
    repository_path: PathBuf,
    worktree_path: PathBuf,
    provider_binding_digest: Option<String>,
    harness_binding_digest: Option<String>,
    created_at: Timestamp,
}

impl SubagentPreparation {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: SubagentId,
        job_id: JobId,
        scope: SubagentScope,
        child_session_id: SessionId,
        child_execution_id: ExecutionId,
        request: SubagentRequest,
        repository_path: PathBuf,
        worktree_path: PathBuf,
        provider_binding_digest: Option<String>,
        harness_binding_digest: Option<String>,
        created_at: Timestamp,
    ) -> Self {
        Self {
            id,
            job_id,
            scope,
            child_session_id,
            child_execution_id,
            request,
            repository_path,
            worktree_path,
            provider_binding_digest,
            harness_binding_digest,
            created_at,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubagentRecord {
    id: SubagentId,
    job_id: JobId,
    scope: SubagentScope,
    child_session_id: SessionId,
    child_execution_id: ExecutionId,
    request: SubagentRequest,
    repository_path: PathBuf,
    worktree_path: PathBuf,
    provider_binding_digest: Option<String>,
    harness_binding_digest: Option<String>,
    status: SubagentStatus,
    worktree_state: SubagentWorktreeState,
    worker_id: Option<JobWorkerId>,
    cancel_requested_at: Option<Timestamp>,
    create_receipt: Option<EffectReceipt>,
    cleanup_claimed_at: Option<Timestamp>,
    remove_receipt: Option<EffectReceipt>,
    created_at: Timestamp,
    started_at: Option<Timestamp>,
    finished_at: Option<Timestamp>,
    result: Option<Value>,
}

impl SubagentRecord {
    pub fn id(&self) -> &SubagentId {
        &self.id
    }

    pub fn job_id(&self) -> &JobId {
        &self.job_id
    }

    pub fn scope(&self) -> &SubagentScope {
        &self.scope
    }

    pub fn child_session_id(&self) -> &SessionId {
        &self.child_session_id
    }

    pub fn child_execution_id(&self) -> &ExecutionId {
        &self.child_execution_id
    }

    pub fn request(&self) -> &SubagentRequest {
        &self.request
    }

    pub fn repository_path(&self) -> &Path {
        &self.repository_path
    }

    pub fn worktree_path(&self) -> &Path {
        &self.worktree_path
    }

    pub fn provider_binding_digest(&self) -> Option<&str> {
        self.provider_binding_digest.as_deref()
    }

    pub fn harness_binding_digest(&self) -> Option<&str> {
        self.harness_binding_digest.as_deref()
    }

    pub const fn status(&self) -> SubagentStatus {
        self.status
    }

    pub const fn worktree_state(&self) -> SubagentWorktreeState {
        self.worktree_state
    }

    pub fn worker_id(&self) -> Option<&JobWorkerId> {
        self.worker_id.as_ref()
    }

    pub const fn cancel_requested_at(&self) -> Option<Timestamp> {
        self.cancel_requested_at
    }

    pub fn create_receipt(&self) -> Option<&EffectReceipt> {
        self.create_receipt.as_ref()
    }

    pub fn remove_receipt(&self) -> Option<&EffectReceipt> {
        self.remove_receipt.as_ref()
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimedSubagent {
    record: SubagentRecord,
}

impl ClaimedSubagent {
    pub fn into_record(self) -> SubagentRecord {
        self.record
    }
}

impl Deref for ClaimedSubagent {
    type Target = SubagentRecord;

    fn deref(&self) -> &Self::Target {
        &self.record
    }
}

#[derive(Debug)]
pub enum SubagentStoreError {
    Database(rusqlite::Error),
    JobStore(JobStoreError),
    Serialization(serde_json::Error),
    InvalidIdentifier(IdError),
    CorruptRecord,
    InvalidPath,
    SubagentAlreadyExists,
    SubagentNotFound,
    JobOwnedByAnotherWorker,
    ResultTooLarge,
    WorktreeCreationNotSuccessful,
    WorktreeCreationDidNotFail,
    InvalidTransition {
        status: SubagentStatus,
        action: &'static str,
    },
    LockPoisoned,
}

impl fmt::Display for SubagentStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(_) => formatter.write_str("subagent database operation failed"),
            Self::JobStore(error) => error.fmt(formatter),
            Self::Serialization(_) => formatter.write_str("subagent record is invalid"),
            Self::InvalidIdentifier(error) => error.fmt(formatter),
            Self::CorruptRecord => {
                formatter.write_str("subagent database contains an invalid record")
            }
            Self::InvalidPath => formatter.write_str("subagent path is not valid UTF-8"),
            Self::SubagentAlreadyExists => formatter.write_str("subagent already exists"),
            Self::SubagentNotFound => formatter.write_str("subagent was not found"),
            Self::JobOwnedByAnotherWorker => {
                formatter.write_str("subagent job is owned by another worker")
            }
            Self::ResultTooLarge => formatter.write_str("subagent result exceeds the size limit"),
            Self::WorktreeCreationNotSuccessful => {
                formatter.write_str("worktree creation receipt is not successful")
            }
            Self::WorktreeCreationDidNotFail => {
                formatter.write_str("worktree creation receipt is not a failure")
            }
            Self::InvalidTransition { status, action } => {
                write!(
                    formatter,
                    "cannot {action} a {} subagent",
                    status_text(*status)
                )
            }
            Self::LockPoisoned => formatter.write_str("subagent database lock is unavailable"),
        }
    }
}

impl std::error::Error for SubagentStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            Self::JobStore(error) => Some(error),
            Self::Serialization(error) => Some(error),
            Self::InvalidIdentifier(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for SubagentStoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

impl From<JobStoreError> for SubagentStoreError {
    fn from(error: JobStoreError) -> Self {
        Self::JobStore(error)
    }
}

impl From<serde_json::Error> for SubagentStoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}

impl From<IdError> for SubagentStoreError {
    fn from(error: IdError) -> Self {
        Self::InvalidIdentifier(error)
    }
}

pub struct SubagentStore {
    connection: Mutex<Connection>,
}

impl SubagentStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, SubagentStoreError> {
        let mut connection = open_job_connection(path.as_ref())?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS subagents (
                 subagent_id TEXT PRIMARY KEY,
                 job_id TEXT NOT NULL UNIQUE,
                 principal_id TEXT NOT NULL,
                 tenant_id TEXT NOT NULL,
                 workspace_id TEXT NOT NULL,
                 child_session_id TEXT NOT NULL UNIQUE,
                 child_execution_id TEXT NOT NULL UNIQUE,
                 parent_session_id TEXT NOT NULL,
                 parent_execution_id TEXT NOT NULL,
                 repository_path TEXT NOT NULL,
                 worktree_path TEXT NOT NULL UNIQUE,
                 exact_commit TEXT NOT NULL,
                 request_json TEXT NOT NULL,
                 provider_binding_digest TEXT,
                 harness_binding_digest TEXT,
                 status TEXT NOT NULL CHECK (
                     status IN ('preparing', 'queued', 'running', 'approval_required', 'completed', 'failed', 'interrupted', 'cancelled')
                 ),
                 worktree_state TEXT NOT NULL CHECK (
                     worktree_state IN ('pending', 'ready', 'preserved', 'removed')
                 ),
                 worker_id TEXT,
                 cancel_requested_at INTEGER,
                 create_receipt_json TEXT,
                 cleanup_claimed_at INTEGER,
                 remove_receipt_json TEXT,
                 created_at INTEGER NOT NULL,
                 started_at INTEGER,
                 finished_at INTEGER,
                 result_json TEXT
             );
             CREATE INDEX IF NOT EXISTS subagents_scope_status_idx
                 ON subagents(principal_id, tenant_id, workspace_id, status);",
        )?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let cleanup_claim_exists = transaction
            .query_row(
                "SELECT 1 FROM pragma_table_info('subagents') WHERE name = 'cleanup_claimed_at'",
                [],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !cleanup_claim_exists {
            transaction.execute(
                "ALTER TABLE subagents ADD COLUMN cleanup_claimed_at INTEGER",
                [],
            )?;
        }
        transaction.commit()?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn prepare(
        &self,
        input: SubagentPreparation,
    ) -> Result<SubagentRecord, SubagentStoreError> {
        let repository_path = path_text(&input.repository_path)?;
        let worktree_path = path_text(&input.worktree_path)?;
        let request_json = serde_json::to_string(&input.request)?;
        let connection = self.lock()?;
        let result = connection.execute(
            "INSERT INTO subagents (
                 subagent_id, job_id, principal_id, tenant_id, workspace_id,
                 child_session_id, child_execution_id, parent_session_id, parent_execution_id,
                 repository_path, worktree_path, exact_commit, request_json,
                 provider_binding_digest, harness_binding_digest, status, worktree_state, created_at
             ) VALUES (
                 ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                 'preparing', 'pending', ?16
             )",
            params![
                input.id.as_str(),
                input.job_id.as_str(),
                input.scope.principal_id().as_str(),
                input.scope.tenant_id().as_str(),
                input.scope.workspace_id().as_str(),
                input.child_session_id.as_str(),
                input.child_execution_id.as_str(),
                input.request.parent_session_id().as_str(),
                input.request.parent_execution_id().as_str(),
                repository_path,
                worktree_path,
                input.request.exact_commit(),
                request_json,
                input.provider_binding_digest,
                input.harness_binding_digest,
                to_i64(input.created_at.as_unix_seconds())?,
            ],
        );
        match result {
            Ok(_) => Ok(SubagentRecord {
                id: input.id,
                job_id: input.job_id,
                scope: input.scope,
                child_session_id: input.child_session_id,
                child_execution_id: input.child_execution_id,
                request: input.request,
                repository_path: input.repository_path,
                worktree_path: input.worktree_path,
                provider_binding_digest: input.provider_binding_digest,
                harness_binding_digest: input.harness_binding_digest,
                status: SubagentStatus::Preparing,
                worktree_state: SubagentWorktreeState::Pending,
                worker_id: None,
                cancel_requested_at: None,
                create_receipt: None,
                cleanup_claimed_at: None,
                remove_receipt: None,
                created_at: input.created_at,
                started_at: None,
                finished_at: None,
                result: None,
            }),
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(SubagentStoreError::SubagentAlreadyExists)
            }
            Err(error) => Err(SubagentStoreError::Database(error)),
        }
    }

    pub fn queue(
        &self,
        id: &SubagentId,
        scope: &SubagentScope,
        receipt: &EffectReceipt,
        now: Timestamp,
    ) -> Result<SubagentRecord, SubagentStoreError> {
        if !matches!(receipt.outcome(), EffectOutcome::Succeeded) {
            return Err(SubagentStoreError::WorktreeCreationNotSuccessful);
        }
        let receipt_json = encode_receipt(receipt)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut current = load_scoped_subagent(&transaction, id, scope)?
            .ok_or(SubagentStoreError::SubagentNotFound)?;
        if current.status != SubagentStatus::Preparing
            || current.worktree_state != SubagentWorktreeState::Pending
        {
            return Err(SubagentStoreError::InvalidTransition {
                status: current.status,
                action: "queue",
            });
        }
        let job_request = JobRequest::new(JobCommand::Run, vec![current.request.task().to_owned()])
            .map_err(JobStoreError::from)?;
        let request_json = serde_json::to_string(&job_request)?;
        let submission_sequence = next_submission_sequence(&transaction)?;
        transaction.execute(
            "INSERT INTO jobs (
                 id, submission_sequence, principal_id, tenant_id, workspace_id,
                 request_json, status, created_at, job_kind
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'queued', ?7, 'subagent')",
            params![
                current.job_id.as_str(),
                submission_sequence,
                current.scope.principal_id().as_str(),
                current.scope.tenant_id().as_str(),
                current.scope.workspace_id().as_str(),
                request_json,
                to_i64(now.as_unix_seconds())?,
            ],
        )?;
        let changed = transaction.execute(
            "UPDATE subagents
             SET status = 'queued', worktree_state = 'ready', create_receipt_json = ?1
             WHERE subagent_id = ?2 AND principal_id = ?3 AND tenant_id = ?4
               AND workspace_id = ?5 AND status = 'preparing' AND worktree_state = 'pending'",
            params![
                receipt_json,
                id.as_str(),
                scope.principal_id().as_str(),
                scope.tenant_id().as_str(),
                scope.workspace_id().as_str(),
            ],
        )?;
        if changed != 1 {
            return Err(SubagentStoreError::CorruptRecord);
        }
        transaction.commit()?;
        current.status = SubagentStatus::Queued;
        current.worktree_state = SubagentWorktreeState::Ready;
        current.create_receipt = Some(receipt.clone());
        Ok(current)
    }

    pub(crate) fn fail_preparing(
        &self,
        id: &SubagentId,
        scope: &SubagentScope,
        receipt: &EffectReceipt,
        preserve_destination: bool,
        now: Timestamp,
    ) -> Result<SubagentRecord, SubagentStoreError> {
        if matches!(receipt.outcome(), EffectOutcome::Succeeded) {
            return Err(SubagentStoreError::WorktreeCreationDidNotFail);
        }
        let receipt_json = encode_receipt(receipt)?;
        let worktree_state = if preserve_destination {
            SubagentWorktreeState::Preserved
        } else {
            SubagentWorktreeState::Pending
        };
        let result = serde_json::json!({
            "code": "worktree_create_failed",
            "outcome_known": true,
        });
        let result_json = bounded_result_json(&result)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut current = load_scoped_subagent(&transaction, id, scope)?
            .ok_or(SubagentStoreError::SubagentNotFound)?;
        if current.status != SubagentStatus::Preparing
            || current.worktree_state != SubagentWorktreeState::Pending
        {
            return Err(SubagentStoreError::InvalidTransition {
                status: current.status,
                action: "record failed worktree creation for",
            });
        }
        let changed = transaction.execute(
            "UPDATE subagents
             SET status = 'failed', worktree_state = ?1, create_receipt_json = ?2,
                 finished_at = ?3, result_json = ?4
             WHERE subagent_id = ?5 AND principal_id = ?6 AND tenant_id = ?7
               AND workspace_id = ?8 AND status = 'preparing' AND worktree_state = 'pending'",
            params![
                worktree_state_text(worktree_state),
                receipt_json,
                to_i64(now.as_unix_seconds())?,
                result_json,
                id.as_str(),
                scope.principal_id().as_str(),
                scope.tenant_id().as_str(),
                scope.workspace_id().as_str(),
            ],
        )?;
        if changed != 1 {
            return Err(SubagentStoreError::CorruptRecord);
        }
        transaction.commit()?;
        current.status = SubagentStatus::Failed;
        current.worktree_state = worktree_state;
        current.create_receipt = Some(receipt.clone());
        current.finished_at = Some(now);
        current.result = Some(result);
        Ok(current)
    }

    pub(crate) fn interrupt_preparing(
        &self,
        id: &SubagentId,
        scope: &SubagentScope,
        preserve_destination: bool,
        reason: &str,
        now: Timestamp,
    ) -> Result<SubagentRecord, SubagentStoreError> {
        let worktree_state = if preserve_destination {
            SubagentWorktreeState::Preserved
        } else {
            SubagentWorktreeState::Pending
        };
        let result = serde_json::json!({
            "code": "worktree_create_interrupted",
            "outcome_known": false,
            "reason": reason,
        });
        let result_json = bounded_result_json(&result)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut current = load_scoped_subagent(&transaction, id, scope)?
            .ok_or(SubagentStoreError::SubagentNotFound)?;
        if current.status != SubagentStatus::Preparing
            || current.worktree_state != SubagentWorktreeState::Pending
        {
            return Err(SubagentStoreError::InvalidTransition {
                status: current.status,
                action: "reconcile worktree preparation for",
            });
        }
        let changed = transaction.execute(
            "UPDATE subagents
             SET status = 'interrupted', worktree_state = ?1, finished_at = ?2, result_json = ?3
             WHERE subagent_id = ?4 AND principal_id = ?5 AND tenant_id = ?6
               AND workspace_id = ?7 AND status = 'preparing' AND worktree_state = 'pending'",
            params![
                worktree_state_text(worktree_state),
                to_i64(now.as_unix_seconds())?,
                result_json,
                id.as_str(),
                scope.principal_id().as_str(),
                scope.tenant_id().as_str(),
                scope.workspace_id().as_str(),
            ],
        )?;
        if changed != 1 {
            return Err(SubagentStoreError::CorruptRecord);
        }
        transaction.commit()?;
        current.status = SubagentStatus::Interrupted;
        current.worktree_state = worktree_state;
        current.finished_at = Some(now);
        current.result = Some(result);
        Ok(current)
    }

    pub(crate) fn claim_cleanup(
        &self,
        id: &SubagentId,
        scope: &SubagentScope,
        now: Timestamp,
    ) -> Result<SubagentRecord, SubagentStoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut current = load_scoped_subagent(&transaction, id, scope)?
            .ok_or(SubagentStoreError::SubagentNotFound)?;
        if !is_terminal_status(current.status)
            || !matches!(
                current.worktree_state,
                SubagentWorktreeState::Ready | SubagentWorktreeState::Preserved
            )
            || current.cleanup_claimed_at.is_some()
            || current.remove_receipt.is_some()
        {
            return Err(SubagentStoreError::InvalidTransition {
                status: current.status,
                action: "claim worktree cleanup for",
            });
        }
        let changed = transaction.execute(
            "UPDATE subagents SET cleanup_claimed_at = ?1
             WHERE subagent_id = ?2 AND principal_id = ?3 AND tenant_id = ?4
               AND workspace_id = ?5
               AND status IN ('approval_required', 'completed', 'failed', 'interrupted', 'cancelled')
               AND worktree_state IN ('ready', 'preserved')
               AND cleanup_claimed_at IS NULL AND remove_receipt_json IS NULL",
            params![
                to_i64(now.as_unix_seconds())?,
                id.as_str(),
                scope.principal_id().as_str(),
                scope.tenant_id().as_str(),
                scope.workspace_id().as_str(),
            ],
        )?;
        if changed != 1 {
            return Err(SubagentStoreError::InvalidTransition {
                status: current.status,
                action: "claim worktree cleanup for",
            });
        }
        transaction.commit()?;
        current.cleanup_claimed_at = Some(now);
        Ok(current)
    }

    pub(crate) fn record_cleanup(
        &self,
        id: &SubagentId,
        scope: &SubagentScope,
        receipt: &EffectReceipt,
    ) -> Result<SubagentRecord, SubagentStoreError> {
        let worktree_state = if matches!(receipt.outcome(), EffectOutcome::Succeeded) {
            SubagentWorktreeState::Removed
        } else {
            SubagentWorktreeState::Preserved
        };
        let receipt_json = encode_receipt(receipt)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut current = load_scoped_subagent(&transaction, id, scope)?
            .ok_or(SubagentStoreError::SubagentNotFound)?;
        if !is_terminal_status(current.status)
            || !matches!(
                current.worktree_state,
                SubagentWorktreeState::Ready | SubagentWorktreeState::Preserved
            )
            || current.cleanup_claimed_at.is_none()
            || current.remove_receipt.is_some()
        {
            return Err(SubagentStoreError::InvalidTransition {
                status: current.status,
                action: "record worktree cleanup for",
            });
        }
        let changed = transaction.execute(
            "UPDATE subagents SET worktree_state = ?1, remove_receipt_json = ?2
             WHERE subagent_id = ?3 AND principal_id = ?4 AND tenant_id = ?5
               AND workspace_id = ?6
               AND status IN ('approval_required', 'completed', 'failed', 'interrupted', 'cancelled')
               AND worktree_state IN ('ready', 'preserved')
               AND cleanup_claimed_at IS NOT NULL AND remove_receipt_json IS NULL",
            params![
                worktree_state_text(worktree_state),
                receipt_json,
                id.as_str(),
                scope.principal_id().as_str(),
                scope.tenant_id().as_str(),
                scope.workspace_id().as_str(),
            ],
        )?;
        if changed != 1 {
            return Err(SubagentStoreError::InvalidTransition {
                status: current.status,
                action: "record worktree cleanup for",
            });
        }
        transaction.commit()?;
        current.worktree_state = worktree_state;
        current.remove_receipt = Some(receipt.clone());
        Ok(current)
    }

    pub fn claim_next(
        &self,
        scope: &SubagentScope,
        worker: &JobWorkerId,
        now: Timestamp,
    ) -> Result<Option<ClaimedSubagent>, SubagentStoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = transaction
            .query_row(
                "SELECT s.subagent_id
                 FROM subagents s
                 JOIN jobs j ON j.id = s.job_id
                 WHERE s.principal_id = ?1 AND s.tenant_id = ?2 AND s.workspace_id = ?3
                   AND s.status = 'queued' AND j.status = 'queued' AND j.job_kind = 'subagent'
                 ORDER BY j.submission_sequence ASC LIMIT 1",
                params![
                    scope.principal_id().as_str(),
                    scope.tenant_id().as_str(),
                    scope.workspace_id().as_str(),
                ],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(id) = id else {
            transaction.commit()?;
            return Ok(None);
        };
        let id = SubagentId::new(id)?;
        let mut current = load_scoped_subagent(&transaction, &id, scope)?
            .ok_or(SubagentStoreError::CorruptRecord)?;
        let changed_job = transaction.execute(
            "UPDATE jobs SET status = 'running', started_at = ?1, worker_id = ?2
             WHERE id = ?3 AND status = 'queued' AND job_kind = 'subagent'",
            params![
                to_i64(now.as_unix_seconds())?,
                worker.as_str(),
                current.job_id.as_str(),
            ],
        )?;
        let changed_subagent = transaction.execute(
            "UPDATE subagents SET status = 'running', started_at = ?1, worker_id = ?2
             WHERE subagent_id = ?3 AND status = 'queued'",
            params![to_i64(now.as_unix_seconds())?, worker.as_str(), id.as_str()],
        )?;
        if changed_job != 1 || changed_subagent != 1 {
            return Err(SubagentStoreError::CorruptRecord);
        }
        transaction.commit()?;
        current.status = SubagentStatus::Running;
        current.started_at = Some(now);
        current.worker_id = Some(worker.clone());
        Ok(Some(ClaimedSubagent { record: current }))
    }

    pub fn inspect(
        &self,
        id: &SubagentId,
        scope: &SubagentScope,
    ) -> Result<SubagentRecord, SubagentStoreError> {
        let connection = self.lock()?;
        load_scoped_subagent(&connection, id, scope)?.ok_or(SubagentStoreError::SubagentNotFound)
    }

    pub fn list(&self, scope: &SubagentScope) -> Result<Vec<SubagentRecord>, SubagentStoreError> {
        let connection = self.lock()?;
        let sql = format!(
            "{} ORDER BY created_at ASC, subagent_id ASC",
            subagent_select_sql("principal_id = ?1 AND tenant_id = ?2 AND workspace_id = ?3")
        );
        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map(
            params![
                scope.principal_id().as_str(),
                scope.tenant_id().as_str(),
                scope.workspace_id().as_str(),
            ],
            decode_subagent,
        )?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn is_cancel_requested(
        &self,
        id: &SubagentId,
        scope: &SubagentScope,
    ) -> Result<bool, SubagentStoreError> {
        Ok(self.inspect(id, scope)?.cancel_requested_at.is_some())
    }

    pub fn request_cancel(
        &self,
        id: &SubagentId,
        scope: &SubagentScope,
        now: Timestamp,
    ) -> Result<SubagentRecord, SubagentStoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut current = load_scoped_subagent(&transaction, id, scope)?
            .ok_or(SubagentStoreError::SubagentNotFound)?;
        match current.status {
            SubagentStatus::Queued => {
                let changed_job = transaction.execute(
                    "UPDATE jobs SET status = 'cancelled', finished_at = ?1
                     WHERE id = ?2 AND status = 'queued' AND job_kind = 'subagent'",
                    params![to_i64(now.as_unix_seconds())?, current.job_id.as_str()],
                )?;
                let changed_subagent = transaction.execute(
                    "UPDATE subagents
                     SET status = 'cancelled', cancel_requested_at = ?1, finished_at = ?1
                     WHERE subagent_id = ?2 AND status = 'queued'",
                    params![to_i64(now.as_unix_seconds())?, id.as_str()],
                )?;
                if changed_job != 1 || changed_subagent != 1 {
                    return Err(SubagentStoreError::CorruptRecord);
                }
                transaction.commit()?;
                current.status = SubagentStatus::Cancelled;
                current.cancel_requested_at = Some(now);
                current.finished_at = Some(now);
            }
            SubagentStatus::Running => {
                let changed = transaction.execute(
                    "UPDATE subagents SET cancel_requested_at = ?1
                     WHERE subagent_id = ?2 AND status = 'running' AND cancel_requested_at IS NULL",
                    params![to_i64(now.as_unix_seconds())?, id.as_str()],
                )?;
                if changed != 1 {
                    return Err(SubagentStoreError::InvalidTransition {
                        status: current.status,
                        action: "request cancellation for",
                    });
                }
                transaction.commit()?;
                current.cancel_requested_at = Some(now);
            }
            status => {
                return Err(SubagentStoreError::InvalidTransition {
                    status,
                    action: "request cancellation for",
                });
            }
        }
        Ok(current)
    }

    pub fn finish(
        &self,
        id: &SubagentId,
        worker: &JobWorkerId,
        status: SubagentStatus,
        result: &Value,
        now: Timestamp,
    ) -> Result<SubagentRecord, SubagentStoreError> {
        if !matches!(
            status,
            SubagentStatus::Completed
                | SubagentStatus::ApprovalRequired
                | SubagentStatus::Failed
                | SubagentStatus::Cancelled
        ) {
            return Err(SubagentStoreError::InvalidTransition {
                status,
                action: "finish as a non-terminal outcome",
            });
        }
        let result_json = bounded_result_json(result)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut current =
            load_subagent_by_id(&transaction, id)?.ok_or(SubagentStoreError::SubagentNotFound)?;
        if current.status != SubagentStatus::Running {
            return Err(SubagentStoreError::InvalidTransition {
                status: current.status,
                action: "finish",
            });
        }
        if current.worker_id.as_ref() != Some(worker) {
            return Err(SubagentStoreError::JobOwnedByAnotherWorker);
        }
        let changed_job = transaction.execute(
            "UPDATE jobs SET status = ?1, finished_at = ?2, result_json = ?3
             WHERE id = ?4 AND status = 'running' AND worker_id = ?5 AND job_kind = 'subagent'",
            params![
                job_status_text(status),
                to_i64(now.as_unix_seconds())?,
                result_json,
                current.job_id.as_str(),
                worker.as_str(),
            ],
        )?;
        let changed_subagent = transaction.execute(
            "UPDATE subagents SET status = ?1, finished_at = ?2, result_json = ?3
             WHERE subagent_id = ?4 AND status = 'running' AND worker_id = ?5",
            params![
                status_text(status),
                to_i64(now.as_unix_seconds())?,
                result_json,
                id.as_str(),
                worker.as_str(),
            ],
        )?;
        if changed_job != 1 || changed_subagent != 1 {
            return Err(SubagentStoreError::CorruptRecord);
        }
        transaction.commit()?;
        current.status = status;
        current.finished_at = Some(now);
        current.result = Some(result.clone());
        Ok(current)
    }

    pub fn mark_interrupted(
        &self,
        id: &SubagentId,
        scope: &SubagentScope,
        reason: &str,
        now: Timestamp,
    ) -> Result<SubagentRecord, SubagentStoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut current = load_scoped_subagent(&transaction, id, scope)?
            .ok_or(SubagentStoreError::SubagentNotFound)?;
        if current.status != SubagentStatus::Running {
            return Err(SubagentStoreError::InvalidTransition {
                status: current.status,
                action: "mark interrupted",
            });
        }
        let result = serde_json::json!({
            "code": "worker_interrupted",
            "outcome_known": false,
            "reason": reason,
            "worker_id": current.worker_id.as_ref().map(JobWorkerId::as_str),
        });
        let result_json = bounded_result_json(&result)?;
        let changed_job = transaction.execute(
            "UPDATE jobs SET status = 'interrupted', finished_at = ?1, result_json = ?2
             WHERE id = ?3 AND status = 'running' AND job_kind = 'subagent'",
            params![
                to_i64(now.as_unix_seconds())?,
                result_json,
                current.job_id.as_str(),
            ],
        )?;
        let changed_subagent = transaction.execute(
            "UPDATE subagents SET status = 'interrupted', finished_at = ?1, result_json = ?2
             WHERE subagent_id = ?3 AND status = 'running'",
            params![to_i64(now.as_unix_seconds())?, result_json, id.as_str()],
        )?;
        if changed_job != 1 || changed_subagent != 1 {
            return Err(SubagentStoreError::CorruptRecord);
        }
        transaction.commit()?;
        current.status = SubagentStatus::Interrupted;
        current.finished_at = Some(now);
        current.result = Some(result);
        Ok(current)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, SubagentStoreError> {
        self.connection
            .lock()
            .map_err(|_| SubagentStoreError::LockPoisoned)
    }
}

#[derive(Deserialize, Serialize)]
struct EffectReceiptWire {
    receipt_id: String,
    permit_id: String,
    request_digest: String,
    completed_at: u64,
    outcome: EffectOutcomeWire,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum EffectOutcomeWire {
    Succeeded,
    Failed { code: String },
    Denied { reason: String },
}

fn encode_receipt(receipt: &EffectReceipt) -> Result<String, SubagentStoreError> {
    let outcome = match receipt.outcome() {
        EffectOutcome::Succeeded => EffectOutcomeWire::Succeeded,
        EffectOutcome::Failed { code } => EffectOutcomeWire::Failed { code: code.clone() },
        EffectOutcome::Denied { reason } => EffectOutcomeWire::Denied {
            reason: reason.clone(),
        },
    };
    Ok(serde_json::to_string(&EffectReceiptWire {
        receipt_id: receipt.receipt_id().as_str().to_owned(),
        permit_id: receipt.permit_id().as_str().to_owned(),
        request_digest: receipt.request_digest().as_str().to_owned(),
        completed_at: receipt.completed_at().as_unix_seconds(),
        outcome,
    })?)
}

fn decode_receipt(value: String) -> Result<EffectReceipt, SubagentStoreError> {
    let wire = serde_json::from_str::<EffectReceiptWire>(&value)?;
    let outcome = match wire.outcome {
        EffectOutcomeWire::Succeeded => EffectOutcome::Succeeded,
        EffectOutcomeWire::Failed { code } => EffectOutcome::Failed { code },
        EffectOutcomeWire::Denied { reason } => EffectOutcome::Denied { reason },
    };
    Ok(EffectReceipt::new(
        ReceiptId::new(wire.receipt_id)?,
        PermitId::new(wire.permit_id)?,
        RequestDigest::new(wire.request_digest)?,
        Timestamp::from_unix_seconds(wire.completed_at),
        outcome,
    ))
}

fn load_subagent_by_id(
    connection: &Connection,
    id: &SubagentId,
) -> Result<Option<SubagentRecord>, SubagentStoreError> {
    let sql = subagent_select_sql("subagent_id = ?1");
    connection
        .query_row(&sql, params![id.as_str()], decode_subagent)
        .optional()
        .map_err(SubagentStoreError::from)
}

fn load_scoped_subagent(
    connection: &Connection,
    id: &SubagentId,
    scope: &SubagentScope,
) -> Result<Option<SubagentRecord>, SubagentStoreError> {
    let sql = subagent_select_sql(
        "subagent_id = ?1 AND principal_id = ?2 AND tenant_id = ?3 AND workspace_id = ?4",
    );
    connection
        .query_row(
            &sql,
            params![
                id.as_str(),
                scope.principal_id().as_str(),
                scope.tenant_id().as_str(),
                scope.workspace_id().as_str(),
            ],
            decode_subagent,
        )
        .optional()
        .map_err(SubagentStoreError::from)
}

fn subagent_select_sql(predicate: &str) -> String {
    format!(
        "SELECT subagent_id, job_id, principal_id, tenant_id, workspace_id,
                child_session_id, child_execution_id, parent_session_id, parent_execution_id,
                repository_path, worktree_path, exact_commit, request_json,
                provider_binding_digest, harness_binding_digest, status, worktree_state,
                worker_id, cancel_requested_at, create_receipt_json, cleanup_claimed_at,
                remove_receipt_json, created_at, started_at, finished_at, result_json
         FROM subagents WHERE {predicate}"
    )
}

fn decode_subagent(row: &rusqlite::Row<'_>) -> rusqlite::Result<SubagentRecord> {
    decode_subagent_inner(row).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })
}

fn decode_subagent_inner(row: &rusqlite::Row<'_>) -> Result<SubagentRecord, SubagentStoreError> {
    let id = SubagentId::new(row.get::<_, String>(0)?)?;
    let job_id = JobId::new(row.get::<_, String>(1)?)?;
    let scope = SubagentScope::new(
        PrincipalId::new(row.get::<_, String>(2)?)?,
        TenantId::new(row.get::<_, String>(3)?)?,
        WorkspaceId::new(row.get::<_, String>(4)?)?,
    );
    let child_session_id = SessionId::new(row.get::<_, String>(5)?)?;
    let child_execution_id = ExecutionId::new(row.get::<_, String>(6)?)?;
    let parent_session_id = row.get::<_, String>(7)?;
    let parent_execution_id = row.get::<_, String>(8)?;
    let repository_path = PathBuf::from(row.get::<_, String>(9)?);
    let worktree_path = PathBuf::from(row.get::<_, String>(10)?);
    let exact_commit = row.get::<_, String>(11)?;
    let request = serde_json::from_str::<SubagentRequest>(&row.get::<_, String>(12)?)?;
    if request.parent_session_id().as_str() != parent_session_id
        || request.parent_execution_id().as_str() != parent_execution_id
        || request.exact_commit() != exact_commit
    {
        return Err(SubagentStoreError::CorruptRecord);
    }
    let provider_binding_digest = row.get::<_, Option<String>>(13)?;
    let harness_binding_digest = row.get::<_, Option<String>>(14)?;
    let status = decode_subagent_status(&row.get::<_, String>(15)?)?;
    let worktree_state = decode_worktree_state(&row.get::<_, String>(16)?)?;
    let worker_id = row
        .get::<_, Option<String>>(17)?
        .map(JobWorkerId::new)
        .transpose()?;
    let cancel_requested_at = row
        .get::<_, Option<i64>>(18)?
        .map(decode_timestamp)
        .transpose()?;
    let create_receipt = row
        .get::<_, Option<String>>(19)?
        .map(decode_receipt)
        .transpose()?;
    let cleanup_claimed_at = row
        .get::<_, Option<i64>>(20)?
        .map(decode_timestamp)
        .transpose()?;
    let remove_receipt = row
        .get::<_, Option<String>>(21)?
        .map(decode_receipt)
        .transpose()?;
    let created_at = decode_timestamp(row.get::<_, i64>(22)?)?;
    let started_at = row
        .get::<_, Option<i64>>(23)?
        .map(decode_timestamp)
        .transpose()?;
    let finished_at = row
        .get::<_, Option<i64>>(24)?
        .map(decode_timestamp)
        .transpose()?;
    let result = row
        .get::<_, Option<String>>(25)?
        .map(|value| serde_json::from_str(&value))
        .transpose()?;
    validate_subagent_record(
        status,
        worktree_state,
        worker_id.as_ref(),
        create_receipt.as_ref(),
        started_at,
        finished_at,
        result.as_ref(),
    )?;
    validate_cleanup_record(
        status,
        worktree_state,
        cleanup_claimed_at,
        remove_receipt.as_ref(),
    )?;
    Ok(SubagentRecord {
        id,
        job_id,
        scope,
        child_session_id,
        child_execution_id,
        request,
        repository_path,
        worktree_path,
        provider_binding_digest,
        harness_binding_digest,
        status,
        worktree_state,
        worker_id,
        cancel_requested_at,
        create_receipt,
        cleanup_claimed_at,
        remove_receipt,
        created_at,
        started_at,
        finished_at,
        result,
    })
}

fn validate_subagent_record(
    status: SubagentStatus,
    worktree_state: SubagentWorktreeState,
    worker_id: Option<&JobWorkerId>,
    create_receipt: Option<&EffectReceipt>,
    started_at: Option<Timestamp>,
    finished_at: Option<Timestamp>,
    result: Option<&Value>,
) -> Result<(), SubagentStoreError> {
    let lifecycle_valid = match status {
        SubagentStatus::Preparing => {
            worktree_state == SubagentWorktreeState::Pending
                && worker_id.is_none()
                && create_receipt.is_none()
                && started_at.is_none()
                && finished_at.is_none()
                && result.is_none()
        }
        SubagentStatus::Queued => {
            worktree_state == SubagentWorktreeState::Ready
                && worker_id.is_none()
                && create_receipt.is_some()
                && started_at.is_none()
                && finished_at.is_none()
                && result.is_none()
        }
        SubagentStatus::Running => {
            worktree_state == SubagentWorktreeState::Ready
                && worker_id.is_some()
                && create_receipt.is_some()
                && started_at.is_some()
                && finished_at.is_none()
                && result.is_none()
        }
        SubagentStatus::Failed if started_at.is_none() => {
            matches!(
                worktree_state,
                SubagentWorktreeState::Pending | SubagentWorktreeState::Preserved
            ) && worker_id.is_none()
                && create_receipt
                    .is_some_and(|receipt| !matches!(receipt.outcome(), EffectOutcome::Succeeded))
                && finished_at.is_some()
                && result.is_some()
        }
        SubagentStatus::Interrupted if started_at.is_none() => {
            matches!(
                worktree_state,
                SubagentWorktreeState::Pending | SubagentWorktreeState::Preserved
            ) && worker_id.is_none()
                && create_receipt.is_none()
                && finished_at.is_some()
                && result.is_some()
        }
        SubagentStatus::Cancelled if started_at.is_none() => {
            worker_id.is_none()
                && create_receipt.is_some()
                && finished_at.is_some()
                && result.is_none()
        }
        SubagentStatus::ApprovalRequired
        | SubagentStatus::Completed
        | SubagentStatus::Failed
        | SubagentStatus::Interrupted
        | SubagentStatus::Cancelled => {
            worker_id.is_some()
                && create_receipt.is_some()
                && started_at.is_some()
                && finished_at.is_some()
                && result.is_some()
        }
    };
    if lifecycle_valid {
        Ok(())
    } else {
        Err(SubagentStoreError::CorruptRecord)
    }
}

fn validate_cleanup_record(
    status: SubagentStatus,
    worktree_state: SubagentWorktreeState,
    cleanup_claimed_at: Option<Timestamp>,
    remove_receipt: Option<&EffectReceipt>,
) -> Result<(), SubagentStoreError> {
    let valid = match (cleanup_claimed_at, remove_receipt) {
        (None, None) => worktree_state != SubagentWorktreeState::Removed,
        (Some(_), None) => {
            is_terminal_status(status)
                && matches!(
                    worktree_state,
                    SubagentWorktreeState::Ready | SubagentWorktreeState::Preserved
                )
        }
        (Some(_), Some(receipt)) if matches!(receipt.outcome(), EffectOutcome::Succeeded) => {
            is_terminal_status(status) && worktree_state == SubagentWorktreeState::Removed
        }
        (Some(_), Some(_)) => {
            is_terminal_status(status) && worktree_state == SubagentWorktreeState::Preserved
        }
        (None, Some(_)) => false,
    };
    if valid {
        Ok(())
    } else {
        Err(SubagentStoreError::CorruptRecord)
    }
}

fn next_submission_sequence(connection: &Connection) -> Result<i64, SubagentStoreError> {
    let latest = connection.query_row("SELECT MAX(submission_sequence) FROM jobs", [], |row| {
        row.get::<_, Option<i64>>(0)
    })?;
    latest
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(SubagentStoreError::CorruptRecord)
}

fn bounded_result_json(result: &Value) -> Result<String, SubagentStoreError> {
    let bytes = serde_json::to_vec(result)?;
    if bytes.len() > MAX_SUBAGENT_RESULT_BYTES {
        return Err(SubagentStoreError::ResultTooLarge);
    }
    String::from_utf8(bytes).map_err(|_| SubagentStoreError::CorruptRecord)
}

fn path_text(path: &Path) -> Result<&str, SubagentStoreError> {
    path.to_str().ok_or(SubagentStoreError::InvalidPath)
}

fn status_text(status: SubagentStatus) -> &'static str {
    match status {
        SubagentStatus::Preparing => "preparing",
        SubagentStatus::Queued => "queued",
        SubagentStatus::Running => "running",
        SubagentStatus::ApprovalRequired => "approval_required",
        SubagentStatus::Completed => "completed",
        SubagentStatus::Failed => "failed",
        SubagentStatus::Interrupted => "interrupted",
        SubagentStatus::Cancelled => "cancelled",
    }
}

fn job_status_text(status: SubagentStatus) -> &'static str {
    match status {
        SubagentStatus::ApprovalRequired => JobStatus::ApprovalRequired.as_str(),
        SubagentStatus::Completed => JobStatus::Completed.as_str(),
        SubagentStatus::Failed => JobStatus::Failed.as_str(),
        SubagentStatus::Cancelled => JobStatus::Cancelled.as_str(),
        _ => status_text(status),
    }
}

fn decode_subagent_status(value: &str) -> Result<SubagentStatus, SubagentStoreError> {
    match value {
        "preparing" => Ok(SubagentStatus::Preparing),
        "queued" => Ok(SubagentStatus::Queued),
        "running" => Ok(SubagentStatus::Running),
        "approval_required" => Ok(SubagentStatus::ApprovalRequired),
        "completed" => Ok(SubagentStatus::Completed),
        "failed" => Ok(SubagentStatus::Failed),
        "interrupted" => Ok(SubagentStatus::Interrupted),
        "cancelled" => Ok(SubagentStatus::Cancelled),
        _ => Err(SubagentStoreError::CorruptRecord),
    }
}

fn decode_worktree_state(value: &str) -> Result<SubagentWorktreeState, SubagentStoreError> {
    match value {
        "pending" => Ok(SubagentWorktreeState::Pending),
        "ready" => Ok(SubagentWorktreeState::Ready),
        "preserved" => Ok(SubagentWorktreeState::Preserved),
        "removed" => Ok(SubagentWorktreeState::Removed),
        _ => Err(SubagentStoreError::CorruptRecord),
    }
}

fn worktree_state_text(state: SubagentWorktreeState) -> &'static str {
    match state {
        SubagentWorktreeState::Pending => "pending",
        SubagentWorktreeState::Ready => "ready",
        SubagentWorktreeState::Preserved => "preserved",
        SubagentWorktreeState::Removed => "removed",
    }
}

fn is_terminal_status(status: SubagentStatus) -> bool {
    matches!(
        status,
        SubagentStatus::ApprovalRequired
            | SubagentStatus::Completed
            | SubagentStatus::Failed
            | SubagentStatus::Interrupted
            | SubagentStatus::Cancelled
    )
}

fn to_i64(value: u64) -> Result<i64, SubagentStoreError> {
    i64::try_from(value).map_err(|_| SubagentStoreError::CorruptRecord)
}

fn decode_timestamp(value: i64) -> Result<Timestamp, SubagentStoreError> {
    u64::try_from(value)
        .map(Timestamp::from_unix_seconds)
        .map_err(|_| SubagentStoreError::CorruptRecord)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::JobStore;
    use pandora_types::{
        EffectOutcome, ExecutionId, JobCommand, JobId, JobRequest, JobWorkerId, PermitId,
        PrincipalId, ReceiptId, RequestDigest, SessionId, SubagentBudgets, SubagentId,
        SubagentRequest, TenantId, Timestamp, WorkspaceId,
    };
    use rusqlite::{Connection, params};
    use serde_json::json;
    use std::path::PathBuf;

    struct StoreFixture {
        root: PathBuf,
        database: PathBuf,
        store: SubagentStore,
        id: SubagentId,
        scope: SubagentScope,
    }

    impl StoreFixture {
        fn new() -> Self {
            let root = crate::test_support::new_temp_dir("pandora-subagent-store").unwrap();
            let database = root.join("jobs.sqlite3");
            let store = SubagentStore::open(&database).unwrap();
            Self {
                root,
                database,
                store,
                id: SubagentId::new("subagent-1").unwrap(),
                scope: SubagentScope::new(
                    PrincipalId::new("principal-1").unwrap(),
                    TenantId::new("tenant-1").unwrap(),
                    WorkspaceId::new("child-workspace-1").unwrap(),
                ),
            }
        }

        fn queued() -> Self {
            let fixture = Self::new();
            let prepared = fixture.store.prepare(fixture.preparation()).unwrap();
            fixture
                .store
                .queue(
                    prepared.id(),
                    &fixture.scope,
                    &fixture.successful_worktree_receipt(),
                    fixture.now(),
                )
                .unwrap();
            fixture
        }

        fn running() -> Self {
            let fixture = Self::queued();
            fixture
                .store
                .claim_next(&fixture.scope, &fixture.worker(), fixture.now())
                .unwrap()
                .unwrap();
            fixture
        }

        fn preparation(&self) -> SubagentPreparation {
            SubagentPreparation::new(
                self.id.clone(),
                JobId::new("subagent-job-1").unwrap(),
                self.scope.clone(),
                SessionId::new("child-session-1").unwrap(),
                ExecutionId::new("child-execution-1").unwrap(),
                self.request(),
                self.root.join("repository"),
                self.root.join("managed").join("subagent-1"),
                Some("provider-sha256:abc123".to_owned()),
                Some("harness-sha256:def456".to_owned()),
                Timestamp::from_unix_seconds(10),
            )
        }

        fn request(&self) -> SubagentRequest {
            SubagentRequest::new(
                SessionId::new("parent-session-1").unwrap(),
                ExecutionId::new("parent-execution-1").unwrap(),
                1,
                "a".repeat(40),
                "fix the isolated task",
                SubagentBudgets::new(8, 16, 50_000, 900, 2, 65_536).unwrap(),
            )
            .unwrap()
        }

        fn successful_worktree_receipt(&self) -> EffectReceipt {
            EffectReceipt::new(
                ReceiptId::new("receipt-worktree-create-1").unwrap(),
                PermitId::new("permit-worktree-create-1").unwrap(),
                RequestDigest::new("request-worktree-create-1").unwrap(),
                Timestamp::from_unix_seconds(20),
                EffectOutcome::Succeeded,
            )
        }

        fn failed_worktree_receipt(&self) -> EffectReceipt {
            EffectReceipt::new(
                ReceiptId::new("receipt-worktree-create-failed").unwrap(),
                PermitId::new("permit-worktree-create-failed").unwrap(),
                RequestDigest::new("request-worktree-create-failed").unwrap(),
                Timestamp::from_unix_seconds(20),
                EffectOutcome::Failed {
                    code: "git_failed".to_owned(),
                },
            )
        }

        fn now(&self) -> Timestamp {
            Timestamp::from_unix_seconds(30)
        }

        fn worker(&self) -> JobWorkerId {
            JobWorkerId::new("worker-1").unwrap()
        }

        fn other_worker(&self) -> JobWorkerId {
            JobWorkerId::new("worker-2").unwrap()
        }
    }

    impl Drop for StoreFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn ready_worktree_and_queued_job_commit_together() {
        let fixture = StoreFixture::new();
        let prepared = fixture.store.prepare(fixture.preparation()).unwrap();
        let receipt = fixture.successful_worktree_receipt();

        let queued = fixture
            .store
            .queue(prepared.id(), &fixture.scope, &receipt, fixture.now())
            .unwrap();

        assert_eq!(queued.status(), SubagentStatus::Queued);
        assert_eq!(queued.worktree_state(), SubagentWorktreeState::Ready);
        assert_eq!(queued.create_receipt(), Some(&receipt));
        let jobs = JobStore::open(&fixture.database).unwrap();
        assert!(
            jobs.claim_next(
                fixture.scope.principal_id(),
                fixture.scope.tenant_id(),
                fixture.scope.workspace_id(),
                &fixture.worker(),
                fixture.now(),
            )
            .unwrap()
            .is_none()
        );
        assert!(
            fixture
                .store
                .claim_next(&fixture.scope, &fixture.worker(), fixture.now())
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn queue_rejects_a_scope_that_does_not_own_the_subagent() {
        let fixture = StoreFixture::new();
        let prepared = fixture.store.prepare(fixture.preparation()).unwrap();
        let wrong_scope = SubagentScope::new(
            PrincipalId::new("principal-2").unwrap(),
            TenantId::new("tenant-2").unwrap(),
            WorkspaceId::new("child-workspace-2").unwrap(),
        );

        let error = fixture
            .store
            .queue(
                prepared.id(),
                &wrong_scope,
                &fixture.successful_worktree_receipt(),
                fixture.now(),
            )
            .unwrap_err();

        assert!(matches!(error, SubagentStoreError::SubagentNotFound));
        let stored = fixture.store.inspect(&fixture.id, &fixture.scope).unwrap();
        assert_eq!(stored.status(), SubagentStatus::Preparing);
        assert_eq!(stored.worktree_state(), SubagentWorktreeState::Pending);
        assert!(stored.create_receipt().is_none());
    }

    #[test]
    fn failed_worktree_receipt_does_not_queue_a_job() {
        let fixture = StoreFixture::new();
        let prepared = fixture.store.prepare(fixture.preparation()).unwrap();

        let error = fixture
            .store
            .queue(
                prepared.id(),
                &fixture.scope,
                &fixture.failed_worktree_receipt(),
                fixture.now(),
            )
            .unwrap_err();

        assert!(matches!(
            error,
            SubagentStoreError::WorktreeCreationNotSuccessful
        ));
        let stored = fixture.store.inspect(&fixture.id, &fixture.scope).unwrap();
        assert_eq!(stored.status(), SubagentStatus::Preparing);
        assert_eq!(stored.worktree_state(), SubagentWorktreeState::Pending);
        assert!(stored.create_receipt().is_none());
    }

    #[test]
    fn linked_job_failure_rolls_back_the_queue_transition() {
        let fixture = StoreFixture::new();
        let prepared = fixture.store.prepare(fixture.preparation()).unwrap();
        let jobs = JobStore::open(&fixture.database).unwrap();
        jobs.submit(
            prepared.job_id(),
            fixture.scope.principal_id(),
            fixture.scope.tenant_id(),
            fixture.scope.workspace_id(),
            &JobRequest::new(JobCommand::Run, vec!["conflicting job".to_owned()]).unwrap(),
            fixture.now(),
        )
        .unwrap();

        assert!(
            fixture
                .store
                .queue(
                    prepared.id(),
                    &fixture.scope,
                    &fixture.successful_worktree_receipt(),
                    fixture.now(),
                )
                .is_err()
        );
        let stored = fixture.store.inspect(&fixture.id, &fixture.scope).unwrap();
        assert_eq!(stored.status(), SubagentStatus::Preparing);
        assert_eq!(stored.worktree_state(), SubagentWorktreeState::Pending);
        assert!(stored.create_receipt().is_none());
    }

    #[test]
    fn wrong_worker_cannot_finish_claimed_child() {
        let fixture = StoreFixture::running();
        let error = fixture
            .store
            .finish(
                &fixture.id,
                &fixture.other_worker(),
                SubagentStatus::Completed,
                &json!({"status":"completed"}),
                fixture.now(),
            )
            .unwrap_err();

        assert!(matches!(error, SubagentStoreError::JobOwnedByAnotherWorker));
    }

    #[test]
    fn queued_cancellation_is_terminal_but_running_cancellation_is_cooperative() {
        let queued = StoreFixture::queued();
        let cancelled = queued
            .store
            .request_cancel(&queued.id, &queued.scope, queued.now())
            .unwrap();
        assert_eq!(cancelled.status(), SubagentStatus::Cancelled);
        assert_eq!(cancelled.finished_at(), Some(queued.now()));
        assert!(
            queued
                .store
                .claim_next(&queued.scope, &queued.worker(), queued.now())
                .unwrap()
                .is_none()
        );

        let running = StoreFixture::running();
        let requested = running
            .store
            .request_cancel(&running.id, &running.scope, running.now())
            .unwrap();
        assert_eq!(requested.status(), SubagentStatus::Running);
        assert_eq!(requested.cancel_requested_at(), Some(running.now()));
        assert!(
            running
                .store
                .is_cancel_requested(&running.id, &running.scope)
                .unwrap()
        );
    }

    #[test]
    fn interruption_never_requeues_unknown_work() {
        let fixture = StoreFixture::running();
        let interrupted = fixture
            .store
            .mark_interrupted(
                &fixture.id,
                &fixture.scope,
                "worker exited after claim",
                fixture.now(),
            )
            .unwrap();

        assert_eq!(interrupted.status(), SubagentStatus::Interrupted);
        assert_eq!(interrupted.result().unwrap()["outcome_known"], false);
        assert!(
            fixture
                .store
                .claim_next(&fixture.scope, &fixture.worker(), fixture.now())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn terminal_result_is_bounded_before_any_transition() {
        let fixture = StoreFixture::running();
        let error = fixture
            .store
            .finish(
                &fixture.id,
                &fixture.worker(),
                SubagentStatus::Completed,
                &json!({"text": "x".repeat(MAX_SUBAGENT_RESULT_BYTES + 1)}),
                fixture.now(),
            )
            .unwrap_err();

        assert!(matches!(error, SubagentStoreError::ResultTooLarge));
        assert_eq!(
            fixture
                .store
                .inspect(&fixture.id, &fixture.scope)
                .unwrap()
                .status(),
            SubagentStatus::Running
        );
    }

    #[test]
    fn opening_subagent_store_migrates_legacy_jobs_without_changing_wire_data() {
        let root = crate::test_support::new_temp_dir("pandora-subagent-migration").unwrap();
        let database = root.join("jobs.sqlite3");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE jobs (
                     id TEXT PRIMARY KEY,
                     principal_id TEXT NOT NULL,
                     tenant_id TEXT NOT NULL,
                     workspace_id TEXT NOT NULL,
                     request_json TEXT NOT NULL,
                     status TEXT NOT NULL,
                     created_at INTEGER NOT NULL,
                     started_at INTEGER,
                     finished_at INTEGER,
                     result_json TEXT
                 );",
            )
            .unwrap();
        let request = JobRequest::new(JobCommand::Run, vec!["legacy task".to_owned()]).unwrap();
        connection
            .execute(
                "INSERT INTO jobs (
                     id, principal_id, tenant_id, workspace_id, request_json, status, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, 'queued', 10)",
                params![
                    "legacy-job",
                    "principal-1",
                    "tenant-1",
                    "workspace-1",
                    serde_json::to_string(&request).unwrap(),
                ],
            )
            .unwrap();
        drop(connection);

        let _subagents = SubagentStore::open(&database).unwrap();
        let jobs = JobStore::open(&database).unwrap();
        let listed = jobs
            .list(
                &PrincipalId::new("principal-1").unwrap(),
                &TenantId::new("tenant-1").unwrap(),
                &WorkspaceId::new("workspace-1").unwrap(),
            )
            .unwrap();

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id().as_str(), "legacy-job");
        assert_eq!(listed[0].request(), &request);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn opening_subagent_store_adds_cleanup_claim_to_existing_schema() {
        let root = crate::test_support::new_temp_dir("pandora-subagent-claim-migration").unwrap();
        let database = root.join("jobs.sqlite3");
        drop(SubagentStore::open(&database).unwrap());
        let connection = Connection::open(&database).unwrap();
        connection
            .execute("ALTER TABLE subagents DROP COLUMN cleanup_claimed_at", [])
            .unwrap();
        drop(connection);

        drop(SubagentStore::open(&database).unwrap());
        let connection = Connection::open(&database).unwrap();
        let mut statement = connection.prepare("PRAGMA table_info(subagents)").unwrap();
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert!(columns.iter().any(|column| column == "cleanup_claimed_at"));
        drop(statement);
        drop(connection);
        let _ = std::fs::remove_dir_all(root);
    }
}
