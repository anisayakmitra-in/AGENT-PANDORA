use crate::execution_controller::{ExecutionController, RuntimeError, WorktreeExecutionContext};
use crate::executors::{GitWorktreeExecutor, WorktreeCommand, WorktreeError};
use crate::subagent_store::{
    SubagentPreparation, SubagentRecord, SubagentScope, SubagentStore, SubagentStoreError,
};
use pandora_types::{
    ExecutionId, JobId, SessionId, SubagentId, SubagentRequest, SubagentStatus, Timestamp,
};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubagentSpawnContext {
    id: SubagentId,
    job_id: JobId,
    scope: SubagentScope,
    child_session_id: SessionId,
    child_execution_id: ExecutionId,
    parent_session_id: SessionId,
    parent_execution_id: ExecutionId,
    provider_binding_digest: Option<String>,
    harness_binding_digest: Option<String>,
}

impl SubagentSpawnContext {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: SubagentId,
        job_id: JobId,
        scope: SubagentScope,
        child_session_id: SessionId,
        child_execution_id: ExecutionId,
        parent_session_id: SessionId,
        parent_execution_id: ExecutionId,
        provider_binding_digest: Option<String>,
        harness_binding_digest: Option<String>,
    ) -> Self {
        Self {
            id,
            job_id,
            scope,
            child_session_id,
            child_execution_id,
            parent_session_id,
            parent_execution_id,
            provider_binding_digest,
            harness_binding_digest,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubagentCleanupContext {
    scope: SubagentScope,
    session_id: SessionId,
    execution_id: ExecutionId,
}

impl SubagentCleanupContext {
    pub fn new(scope: SubagentScope, session_id: SessionId, execution_id: ExecutionId) -> Self {
        Self {
            scope,
            session_id,
            execution_id,
        }
    }
}

#[derive(Debug)]
pub enum SubagentCoordinatorError {
    ParentBindingMismatch,
    UnexpectedWorktreePath,
    InvalidLifecycleState(SubagentStatus),
    Store(SubagentStoreError),
    Runtime(RuntimeError),
    Worktree(WorktreeError),
}

impl fmt::Display for SubagentCoordinatorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ParentBindingMismatch => {
                formatter.write_str("subagent parent binding does not match the request")
            }
            Self::UnexpectedWorktreePath => {
                formatter.write_str("subagent worktree path does not match its deterministic path")
            }
            Self::InvalidLifecycleState(status) => {
                write!(
                    formatter,
                    "subagent lifecycle state {status:?} is not eligible"
                )
            }
            Self::Store(error) => error.fmt(formatter),
            Self::Runtime(error) => write!(formatter, "worktree authorization failed: {error:?}"),
            Self::Worktree(error) => write!(formatter, "worktree operation failed: {error:?}"),
        }
    }
}

impl std::error::Error for SubagentCoordinatorError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Store(error) => Some(error),
            _ => None,
        }
    }
}

impl From<SubagentStoreError> for SubagentCoordinatorError {
    fn from(error: SubagentStoreError) -> Self {
        Self::Store(error)
    }
}

impl From<RuntimeError> for SubagentCoordinatorError {
    fn from(error: RuntimeError) -> Self {
        Self::Runtime(error)
    }
}

impl From<WorktreeError> for SubagentCoordinatorError {
    fn from(error: WorktreeError) -> Self {
        Self::Worktree(error)
    }
}

pub struct SubagentCoordinator<'a> {
    store: &'a SubagentStore,
    controller: &'a ExecutionController,
    executor: &'a GitWorktreeExecutor,
}

impl<'a> SubagentCoordinator<'a> {
    pub fn new(
        store: &'a SubagentStore,
        controller: &'a ExecutionController,
        executor: &'a GitWorktreeExecutor,
    ) -> Self {
        Self {
            store,
            controller,
            executor,
        }
    }

    pub fn spawn(
        &self,
        context: SubagentSpawnContext,
        request: SubagentRequest,
        now: Timestamp,
    ) -> Result<SubagentRecord, SubagentCoordinatorError> {
        if request.parent_session_id() != &context.parent_session_id
            || request.parent_execution_id() != &context.parent_execution_id
        {
            return Err(SubagentCoordinatorError::ParentBindingMismatch);
        }
        let destination = self.expected_destination(&context.id);
        let command = WorktreeCommand::create(
            self.executor.repository(),
            &destination,
            request.exact_commit(),
        )?;
        self.store.prepare(SubagentPreparation::new(
            context.id.clone(),
            context.job_id,
            context.scope.clone(),
            context.child_session_id,
            context.child_execution_id,
            request,
            command.repository().to_path_buf(),
            command.destination().to_path_buf(),
            context.provider_binding_digest,
            context.harness_binding_digest,
            now,
        ))?;
        let execution_context = WorktreeExecutionContext::new(
            context.parent_execution_id,
            context.parent_session_id,
            context.scope.principal_id().clone(),
        );
        let result =
            match self
                .controller
                .execute_worktree(self.executor, &command, execution_context, now)
            {
                Ok(result) => result,
                Err(error) => {
                    self.store.interrupt_preparing(
                        &context.id,
                        &context.scope,
                        destination_present_or_uncertain(&destination),
                        "worktree authorization ended without an effect receipt",
                        now,
                    )?;
                    return Err(error.into());
                }
            };
        let receipt = result.receipt().clone();
        match result.into_result() {
            Ok(_) => self
                .store
                .queue(&context.id, &context.scope, &receipt, now)
                .map_err(Into::into),
            Err(error) => {
                self.store.fail_preparing(
                    &context.id,
                    &context.scope,
                    &receipt,
                    destination_present_or_uncertain(&destination),
                    now,
                )?;
                Err(error.into())
            }
        }
    }

    pub fn reconcile_preparing(
        &self,
        id: &SubagentId,
        scope: &SubagentScope,
        now: Timestamp,
    ) -> Result<SubagentRecord, SubagentCoordinatorError> {
        let current = self.store.inspect(id, scope)?;
        if current.status() != SubagentStatus::Preparing {
            return Err(SubagentCoordinatorError::InvalidLifecycleState(
                current.status(),
            ));
        }
        self.ensure_expected_path(&current)?;
        self.store
            .interrupt_preparing(
                id,
                scope,
                destination_present_or_uncertain(current.worktree_path()),
                "worktree creation outcome was not durably recorded",
                now,
            )
            .map_err(Into::into)
    }

    pub fn cleanup(
        &self,
        id: &SubagentId,
        context: SubagentCleanupContext,
        now: Timestamp,
    ) -> Result<SubagentRecord, SubagentCoordinatorError> {
        let current = self.store.inspect(id, &context.scope)?;
        if !is_terminal(current.status()) {
            return Err(SubagentCoordinatorError::InvalidLifecycleState(
                current.status(),
            ));
        }
        self.ensure_expected_path(&current)?;
        let command = WorktreeCommand::remove(
            current.repository_path(),
            current.worktree_path(),
            current.request().exact_commit(),
        )?;
        self.store.claim_cleanup(id, &context.scope, now)?;
        let execution_context = WorktreeExecutionContext::new(
            context.execution_id,
            context.session_id,
            context.scope.principal_id().clone(),
        );
        let result =
            self.controller
                .execute_worktree(self.executor, &command, execution_context, now)?;
        let receipt = result.receipt().clone();
        let operation_result = result.into_result();
        let stored = self.store.record_cleanup(id, &context.scope, &receipt)?;
        match operation_result {
            Ok(_) => Ok(stored),
            Err(error) => Err(error.into()),
        }
    }

    fn expected_destination(&self, id: &SubagentId) -> PathBuf {
        let digest = Sha256::digest(id.as_str().as_bytes());
        external_process_path(self.executor.managed_root()).join(format!("{digest:x}"))
    }

    fn ensure_expected_path(
        &self,
        record: &SubagentRecord,
    ) -> Result<(), SubagentCoordinatorError> {
        if record.repository_path() != self.executor.repository()
            || record.worktree_path() != self.expected_destination(record.id())
        {
            return Err(SubagentCoordinatorError::UnexpectedWorktreePath);
        }
        Ok(())
    }
}

fn destination_present_or_uncertain(path: &std::path::Path) -> bool {
    match fs::symlink_metadata(path) {
        Ok(_) => true,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(_) => true,
    }
}

fn external_process_path(path: &std::path::Path) -> PathBuf {
    #[cfg(windows)]
    {
        let value = path.to_string_lossy();
        if let Some(remainder) = value.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{remainder}"));
        }
        if let Some(remainder) = value.strip_prefix(r"\\?\") {
            return PathBuf::from(remainder);
        }
    }
    path.to_path_buf()
}

fn is_terminal(status: SubagentStatus) -> bool {
    matches!(
        status,
        SubagentStatus::ApprovalRequired
            | SubagentStatus::Completed
            | SubagentStatus::Failed
            | SubagentStatus::Interrupted
            | SubagentStatus::Cancelled
    )
}
