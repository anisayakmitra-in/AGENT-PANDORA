use crate::executors::{
    FilesystemError, FilesystemExecutor, ProcessError, ProcessExecutor, VerificationCommand,
    VerificationOptions, WorkspaceRoot,
};
use crate::parliament::Parliament;
use crate::reference_monitor::{AuthorizationError, ReferenceMonitor};
use crate::shadow_council::{Selection, ShadowCouncil};
use crate::{ConsumedPermit, PermitError};
use pandora_harnesses::{CodingHarness, CodingRequest, PlanningContext};
use pandora_types::{
    Capability, EffectReceipt, EffectTarget, EventContext, EventId, EventPayload, EventType,
    ExecutionId, GeneError, GeneId, GeneInput, Harness, HarnessId, ParliamentDecision,
    PolicyContext, RuntimeEvent, Session, TaskIntent, Timestamp,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeError {
    InvalidIntent(&'static str),
    UnsupportedHarness(HarnessId),
    UnknownGene,
    Planning(GeneError),
    Denied(String),
    ApprovalRequired(String),
    Authorization(AuthorizationError),
    Permit(PermitError),
    Filesystem(FilesystemError),
    Process(ProcessError),
    UnsupportedOperation(Capability),
}

pub struct ExecutionController {
    workspace: WorkspaceRoot,
    shadow_council: ShadowCouncil,
    parliament: Parliament,
    policy: PolicyContext,
    reference_monitor: ReferenceMonitor,
    coding_harness: CodingHarness,
    filesystem: FilesystemExecutor,
    process: ProcessExecutor,
    next_execution: AtomicU64,
    next_event: AtomicU64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunSummary {
    execution_id: ExecutionId,
    selected_harness: HarnessId,
    selected_gene: GeneId,
    status: RunStatus,
    output: Option<Vec<u8>>,
    receipts: Vec<EffectReceipt>,
    events: Vec<RuntimeEvent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunStatus {
    Completed,
    Denied { reason: String },
    ApprovalRequired { reason: String },
    Failed { code: String },
}

impl RunSummary {
    pub fn execution_id(&self) -> &ExecutionId {
        &self.execution_id
    }

    pub fn selected_harness(&self) -> &HarnessId {
        &self.selected_harness
    }

    pub fn selected_gene(&self) -> &GeneId {
        &self.selected_gene
    }

    pub fn status(&self) -> &RunStatus {
        &self.status
    }

    pub fn output(&self) -> Option<&[u8]> {
        self.output.as_deref()
    }

    pub fn receipts(&self) -> &[EffectReceipt] {
        &self.receipts
    }

    pub fn events(&self) -> &[RuntimeEvent] {
        &self.events
    }
}

impl ExecutionController {
    pub fn new(workspace: WorkspaceRoot) -> Self {
        Self::with_policy(workspace, PolicyContext::read_only_workspace())
    }

    pub fn with_policy(workspace: WorkspaceRoot, policy: PolicyContext) -> Self {
        let policy_version = policy.policy_version();
        Self {
            filesystem: FilesystemExecutor::new(),
            process: ProcessExecutor::new(workspace.clone()),
            workspace,
            shadow_council: ShadowCouncil::new(),
            parliament: Parliament::new(policy_version),
            reference_monitor: ReferenceMonitor::new(policy_version, 60),
            policy,
            coding_harness: CodingHarness::new(),
            next_execution: AtomicU64::new(1),
            next_event: AtomicU64::new(1),
        }
    }

    pub fn run(&self, intent: TaskIntent, session: Session) -> Result<RunSummary, RuntimeError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| Timestamp::from_unix_seconds(duration.as_secs()))
            .unwrap_or_else(|_| Timestamp::from_unix_seconds(0));
        self.run_at(intent, session, now)
    }

    pub fn run_at(
        &self,
        intent: TaskIntent,
        session: Session,
        now: Timestamp,
    ) -> Result<RunSummary, RuntimeError> {
        let execution_id = ExecutionId::new(format!(
            "execution-{}",
            self.next_execution.fetch_add(1, Ordering::Relaxed)
        ))
        .map_err(|_| RuntimeError::InvalidIntent("could not allocate execution ID"))?;
        let selection = self.shadow_council.select(&intent);
        self.ensure_coding_selection(&selection)?;
        let gene_id = selection
            .gene_id()
            .cloned()
            .unwrap_or_else(|| default_gene_id(&intent));
        let gene = self
            .coding_harness
            .genes()
            .iter()
            .find(|gene| gene.manifest().id() == &gene_id)
            .ok_or(RuntimeError::UnknownGene)?;
        let input = coding_input(&intent, &gene_id, &session, &execution_id)?;
        let requests = gene.plan(&input).map_err(RuntimeError::Planning)?;
        let mut summary = RunSummary {
            execution_id: execution_id.clone(),
            selected_harness: self.coding_harness.manifest().id().clone(),
            selected_gene: gene_id,
            status: RunStatus::Completed,
            output: None,
            receipts: Vec::new(),
            events: vec![self.event(
                EventType::SessionStarted,
                self.context(&session, &execution_id, None, None, None),
                EventPayload::Empty,
            )],
        };

        for request in requests {
            summary.events.push(self.event(
                EventType::EffectRequested,
                self.context(
                    &session,
                    &execution_id,
                    Some(summary.selected_harness.clone()),
                    Some(summary.selected_gene.clone()),
                    None,
                ),
                EventPayload::Effect {
                    capability: request.capability().as_str().to_owned(),
                    request_digest: request.request_digest().clone(),
                },
            ));
            let decision = self.parliament.decide(&request, &self.policy);
            let permit = match decision {
                ParliamentDecision::Allow { ref reason, .. } => {
                    summary.events.push(self.event(
                        EventType::PolicyApproved,
                        self.context(
                            &session,
                            &execution_id,
                            Some(summary.selected_harness.clone()),
                            Some(summary.selected_gene.clone()),
                            None,
                        ),
                        EventPayload::Policy {
                            reason: reason.clone(),
                        },
                    ));
                    match self
                        .reference_monitor
                        .authorize(request.clone(), decision, now)
                    {
                        Ok(permit) => permit,
                        Err(error) => {
                            let failure = RuntimeError::Authorization(error);
                            self.record_failure(&session, &execution_id, &mut summary, &failure);
                            return Ok(summary);
                        }
                    }
                }
                ParliamentDecision::Deny { reason, .. } => {
                    summary.events.push(self.event(
                        EventType::PolicyDenied,
                        self.context(
                            &session,
                            &execution_id,
                            Some(summary.selected_harness.clone()),
                            Some(summary.selected_gene.clone()),
                            None,
                        ),
                        EventPayload::Policy {
                            reason: reason.clone(),
                        },
                    ));
                    summary.status = RunStatus::Denied { reason };
                    return Ok(summary);
                }
                ParliamentDecision::RequireApproval { reason, .. } => {
                    summary.events.push(self.event(
                        EventType::ApprovalRequired,
                        self.context(
                            &session,
                            &execution_id,
                            Some(summary.selected_harness.clone()),
                            Some(summary.selected_gene.clone()),
                            None,
                        ),
                        EventPayload::Policy {
                            reason: reason.clone(),
                        },
                    ));
                    summary.status = RunStatus::ApprovalRequired { reason };
                    return Ok(summary);
                }
            };
            let consumed = match self
                .reference_monitor
                .store()
                .consume(permit, &request, now)
            {
                Ok(consumed) => consumed,
                Err(error) => {
                    self.record_failure(
                        &session,
                        &execution_id,
                        &mut summary,
                        &RuntimeError::Permit(error),
                    );
                    return Ok(summary);
                }
            };
            if let Err(error) =
                self.execute_request(&session, &execution_id, &consumed, now, &mut summary)
            {
                self.record_failure(&session, &execution_id, &mut summary, &error);
                return Ok(summary);
            }
        }
        Ok(summary)
    }

    fn record_failure(
        &self,
        session: &Session,
        execution_id: &ExecutionId,
        summary: &mut RunSummary,
        error: &RuntimeError,
    ) {
        summary.status = RunStatus::Failed {
            code: runtime_error_code(error).to_owned(),
        };
        summary.events.push(self.event(
            EventType::ExecutionFailed,
            self.context(
                session,
                execution_id,
                Some(summary.selected_harness.clone()),
                Some(summary.selected_gene.clone()),
                summary.receipts.last().cloned(),
            ),
            EventPayload::Failure {
                code: runtime_error_code(error).to_owned(),
            },
        ));
    }

    fn ensure_coding_selection(&self, selection: &Selection) -> Result<(), RuntimeError> {
        if selection.harness_id() != self.coding_harness.manifest().id() {
            return Err(RuntimeError::UnsupportedHarness(
                selection.harness_id().clone(),
            ));
        }
        Ok(())
    }

    fn execute_request(
        &self,
        session: &Session,
        execution_id: &ExecutionId,
        permit: &ConsumedPermit,
        now: Timestamp,
        output: &mut RunSummary,
    ) -> Result<(), RuntimeError> {
        match permit.request().capability() {
            Capability::FilesystemRead => {
                let path = match permit.request().target() {
                    EffectTarget::Path { path } => path,
                    _ => {
                        return Err(RuntimeError::UnsupportedOperation(
                            Capability::FilesystemRead,
                        ));
                    }
                };
                let target = self
                    .workspace
                    .path(path)
                    .map_err(RuntimeError::Filesystem)?;
                let response = self.filesystem.read(permit, &target, now);
                let receipt = response.receipt().clone();
                let result = response.result();
                output.receipts.push(receipt.clone());
                output.events.push(self.event(
                    EventType::EffectCompleted,
                    self.context(
                        session,
                        execution_id,
                        Some(output.selected_harness.clone()),
                        Some(output.selected_gene.clone()),
                        Some(receipt.clone()),
                    ),
                    EventPayload::Effect {
                        capability: permit.request().capability().as_str().to_owned(),
                        request_digest: permit.request().request_digest().clone(),
                    },
                ));
                match result {
                    Ok(bytes) => {
                        output.output = Some(bytes.clone());
                        Ok(())
                    }
                    Err(error) => Err(RuntimeError::Filesystem(error.clone())),
                }
            }
            Capability::ProcessExecute => {
                let command = VerificationCommand::cargo_check_locked(self.workspace.clone());
                let response = self.process.run_verification(
                    permit,
                    &command,
                    &VerificationOptions::default(),
                    now,
                );
                let receipt = response.receipt().clone();
                output.receipts.push(receipt.clone());
                output.events.push(self.event(
                    EventType::EffectCompleted,
                    self.context(
                        session,
                        execution_id,
                        Some(output.selected_harness.clone()),
                        Some(output.selected_gene.clone()),
                        Some(receipt),
                    ),
                    EventPayload::Effect {
                        capability: permit.request().capability().as_str().to_owned(),
                        request_digest: permit.request().request_digest().clone(),
                    },
                ));
                response
                    .result()
                    .map(|_| ())
                    .map_err(|error| RuntimeError::Process(error.clone()))
            }
            capability => Err(RuntimeError::UnsupportedOperation(capability)),
        }
    }

    fn context(
        &self,
        session: &Session,
        execution_id: &ExecutionId,
        harness_id: Option<HarnessId>,
        gene_id: Option<GeneId>,
        receipt: Option<EffectReceipt>,
    ) -> EventContext {
        let mut context =
            EventContext::new(session.tenant_id().clone(), session.workspace_id().clone())
                .with_session(session.id().clone())
                .with_execution(execution_id.clone())
                .with_policy_version(self.policy.policy_version());
        if let Some(harness_id) = harness_id {
            context = context.with_harness(harness_id);
        }
        if let Some(gene_id) = gene_id {
            context = context.with_gene(gene_id);
        }
        if let Some(receipt) = receipt {
            context = context.with_receipt(receipt.receipt_id().clone());
        }
        context
    }

    fn event(
        &self,
        event_type: EventType,
        context: EventContext,
        payload: EventPayload,
    ) -> RuntimeEvent {
        let event_id = EventId::new(format!(
            "event-{}",
            self.next_event.fetch_add(1, Ordering::Relaxed)
        ))
        .expect("generated event ID is valid");
        RuntimeEvent::new(event_id, event_type, context, payload)
    }
}

fn runtime_error_code(error: &RuntimeError) -> &'static str {
    match error {
        RuntimeError::InvalidIntent(_) => "invalid_intent",
        RuntimeError::UnsupportedHarness(_) => "unsupported_harness",
        RuntimeError::UnknownGene => "unknown_gene",
        RuntimeError::Planning(_) => "planning_failed",
        RuntimeError::Denied(_) => "denied",
        RuntimeError::ApprovalRequired(_) => "approval_required",
        RuntimeError::Authorization(_) => "authorization_failed",
        RuntimeError::Permit(_) => "permit_failed",
        RuntimeError::Filesystem(_) => "filesystem_failed",
        RuntimeError::Process(_) => "process_failed",
        RuntimeError::UnsupportedOperation(_) => "unsupported_operation",
    }
}

fn default_gene_id(intent: &TaskIntent) -> GeneId {
    match intent
        .summary()
        .split_once(':')
        .map(|(action, _)| action)
        .unwrap_or(intent.summary())
        .to_ascii_lowercase()
        .as_str()
    {
        "read" => GeneId::new("workspace.read").expect("built-in Gene ID is valid"),
        "search" => GeneId::new("workspace.search").expect("built-in Gene ID is valid"),
        "patch" => GeneId::new("patch.apply").expect("built-in Gene ID is valid"),
        "verify" => GeneId::new("verification.run").expect("built-in Gene ID is valid"),
        "review" => GeneId::new("change.review").expect("built-in Gene ID is valid"),
        _ => GeneId::new("unknown.gene").expect("built-in fallback Gene ID is valid"),
    }
}

fn coding_input(
    intent: &TaskIntent,
    gene_id: &GeneId,
    session: &Session,
    execution_id: &ExecutionId,
) -> Result<GeneInput, RuntimeError> {
    let context = PlanningContext::new(
        execution_id.clone(),
        session.id().clone(),
        session.principal_id().clone(),
        session.workspace_id().clone(),
    );
    let summary = intent.summary();
    let (action, remainder) = summary.split_once(':').unwrap_or((summary, ""));
    let action = action.to_ascii_lowercase();
    let request = match gene_id.as_str() {
        "workspace.read" if action == "read" => CodingRequest::read(context, remainder),
        "workspace.search" if action == "search" => CodingRequest::search(context, remainder),
        "change.review" if action == "review" => CodingRequest::review(context, remainder),
        "patch.apply" if action == "patch" => {
            let (path, content) = remainder
                .split_once(':')
                .ok_or(RuntimeError::InvalidIntent(
                    "patch requires path and content",
                ))?;
            CodingRequest::patch(context, path, content)
        }
        "verification.run" if action == "verify" && remainder.is_empty() => {
            CodingRequest::verify(context)
        }
        _ => {
            return Err(RuntimeError::InvalidIntent(
                "intent does not match the selected Gene",
            ));
        }
    };
    request.into_gene_input().map_err(RuntimeError::Planning)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::{
        Capability, GeneId, HarnessId, Operation, PolicyContext, PrincipalId, Session, SessionId,
        TaskIntent, TenantId, Timestamp, WorkspaceId,
    };
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn read_only_coding_task_completes_with_receipt_and_events() {
        let fixture = Fixture::new();
        let controller = ExecutionController::new(fixture.root.clone());

        let summary = controller
            .run_at(
                TaskIntent::new("read:README.md").unwrap(),
                fixture.session(),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();

        assert_eq!(summary.output().unwrap(), b"fixture\n");
        assert_eq!(summary.status(), &RunStatus::Completed);
        assert_eq!(summary.receipts().len(), 1);
        assert_eq!(summary.events().len(), 4);
        assert_eq!(summary.selected_harness().as_str(), "coding-domain");
        assert_eq!(summary.selected_gene().as_str(), "workspace.read");
    }

    #[test]
    fn patch_is_stopped_at_the_approval_boundary() {
        let fixture = Fixture::new();
        let policy = PolicyContext::new(
            1,
            [Capability::FilesystemRead, Capability::FilesystemWrite],
            [Operation::Write],
        );
        let controller = ExecutionController::with_policy(fixture.root.clone(), policy);

        let summary = controller
            .run_at(
                TaskIntent::new("patch:README.md:changed").unwrap(),
                fixture.session(),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();

        assert_eq!(
            summary.status(),
            &RunStatus::ApprovalRequired {
                reason: "operation requires explicit approval".to_owned()
            }
        );
        assert!(
            summary
                .events()
                .iter()
                .any(|event| event.event_type() == EventType::ApprovalRequired)
        );
    }

    #[test]
    fn path_escape_is_rejected_before_effect_authorization() {
        let fixture = Fixture::new();
        let controller = ExecutionController::new(fixture.root.clone());

        assert!(matches!(
            controller.run_at(
                TaskIntent::new("read:../outside.txt").unwrap(),
                fixture.session(),
                Timestamp::from_unix_seconds(10),
            ),
            Err(RuntimeError::Planning(_))
        ));
    }

    #[test]
    fn unknown_gene_is_not_silently_replaced() {
        let fixture = Fixture::new();
        let controller = ExecutionController::new(fixture.root.clone());
        let intent = TaskIntent::new("read:README.md")
            .unwrap()
            .with_harness(HarnessId::new("coding-domain").unwrap())
            .with_gene(GeneId::new("unknown.gene").unwrap());

        assert_eq!(
            controller.run_at(intent, fixture.session(), Timestamp::from_unix_seconds(10)),
            Err(RuntimeError::UnknownGene)
        );
    }

    struct Fixture {
        path: PathBuf,
        root: WorkspaceRoot,
    }

    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "pandora-controller-test-{}-{}",
                std::process::id(),
                NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("README.md"), b"fixture\n").unwrap();
            let root = WorkspaceRoot::new(&path).unwrap();
            Self { path, root }
        }

        fn session(&self) -> Session {
            Session::new(
                SessionId::new("session-1").unwrap(),
                PrincipalId::new("principal-1").unwrap(),
                TenantId::new("tenant-1").unwrap(),
                WorkspaceId::new("workspace-1").unwrap(),
                Timestamp::from_unix_seconds(1),
            )
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(1);
}
