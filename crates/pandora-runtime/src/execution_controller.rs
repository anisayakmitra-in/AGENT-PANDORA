use crate::executors::{
    FilesystemError, FilesystemExecutor, ProcessError, ProcessExecutor, ProviderExecutor,
    ProviderResult, VerificationCommand, VerificationOptions, WorkspaceRoot,
};
use crate::parliament::Parliament;
use crate::reference_monitor::{AuthorizationError, ReferenceMonitor};
use crate::shadow_council::{RoutingError, ShadowCouncil};
use crate::{ApprovalError, ApprovalStore, ConsumedPermit, PermitError};
use pandora_harnesses::{CodingRequest, HarnessCatalog, PlanningContext};
use pandora_provider::{ModelRequest, Provider, ProviderError};
use pandora_types::{
    Capability, EffectReceipt, EffectTarget, EventContext, EventId, EventPayload, EventType,
    ExecutionId, GeneError, GeneId, GeneInput, Harness, HarnessId, HarnessKind, Operation,
    OperationRequest, ParliamentDecision, PolicyContext, RequestError, ResourceScope, RuntimeEvent,
    SecretReference, Session, TaskIntent, Timestamp,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeError {
    InvalidIntent(&'static str),
    NoDefaultHarness,
    UnsupportedHarness(HarnessId),
    NonExecutableHarness { id: HarnessId, kind: HarnessKind },
    UnknownGene,
    Planning(GeneError),
    Denied(String),
    ApprovalRequired(String),
    Approval(ApprovalError),
    ApprovalNotRequired,
    Authorization(AuthorizationError),
    Permit(PermitError),
    Provider(ProviderError),
    Request(RequestError),
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
    harnesses: HarnessCatalog,
    filesystem: FilesystemExecutor,
    process: ProcessExecutor,
    provider: ProviderExecutor,
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
        Self::with_policy_and_harnesses(workspace, policy, HarnessCatalog::builtins())
    }

    pub fn with_policy_and_harnesses(
        workspace: WorkspaceRoot,
        policy: PolicyContext,
        harnesses: HarnessCatalog,
    ) -> Self {
        let policy_version = policy.policy_version();
        Self {
            filesystem: FilesystemExecutor::new(),
            process: ProcessExecutor::new(workspace.clone()),
            provider: ProviderExecutor::new(),
            workspace,
            shadow_council: ShadowCouncil::new(),
            parliament: Parliament::new(policy_version),
            reference_monitor: ReferenceMonitor::new_with_policy(policy.clone(), 60),
            policy,
            harnesses,
            next_execution: AtomicU64::new(1),
            next_event: AtomicU64::new(1),
        }
    }

    pub fn policy_version(&self) -> u32 {
        self.policy.policy_version()
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
        self.run_internal(intent, session, now, None, None, false)
    }

    pub fn invoke_provider(
        &self,
        provider: &dyn Provider,
        request: ModelRequest,
        session: &Session,
        now: Timestamp,
    ) -> Result<ProviderResult, RuntimeError> {
        let execution_id = ExecutionId::new(format!(
            "provider-execution-{}",
            self.next_execution.fetch_add(1, Ordering::Relaxed)
        ))
        .map_err(|_| RuntimeError::InvalidIntent("could not allocate provider execution ID"))?;
        let operation_request = OperationRequest::new(
            execution_id,
            session.id().clone(),
            session.principal_id().clone(),
            GeneId::new("provider.invoke").expect("built-in provider Gene ID is valid"),
            None,
            Capability::ProviderInvoke,
            Operation::Invoke,
            EffectTarget::provider(
                provider.manifest().id().as_str(),
                SecretReference::new(provider.manifest().api_key_env().to_owned())
                    .map_err(RuntimeError::Request)?,
            ),
            ResourceScope::none(),
        )
        .map_err(RuntimeError::Request)?;
        let decision = self.parliament.decide(&operation_request, &self.policy);
        let permit = match decision {
            ParliamentDecision::Allow { .. } => self
                .reference_monitor
                .authorize(operation_request.clone(), decision, now)
                .map_err(RuntimeError::Authorization)?,
            ParliamentDecision::Deny { reason, .. } => return Err(RuntimeError::Denied(reason)),
            ParliamentDecision::RequireApproval { reason, .. } => {
                return Err(RuntimeError::ApprovalRequired(reason));
            }
        };
        let consumed = self
            .reference_monitor
            .store()
            .consume(permit, &operation_request, now)
            .map_err(RuntimeError::Permit)?;
        Ok(self.provider.complete(&consumed, provider, request, now))
    }

    pub fn run_with_approval(
        &self,
        intent: TaskIntent,
        session: Session,
        approval_store: &ApprovalStore,
        approval_id: &str,
        now: Timestamp,
    ) -> Result<RunSummary, RuntimeError> {
        let approval = approval_store
            .inspect(approval_id, session.principal_id())
            .map_err(RuntimeError::Approval)?;
        if approval.session_id() != session.id() {
            return Err(RuntimeError::Approval(ApprovalError::ScopeMismatch));
        }
        self.run_internal(
            intent,
            session,
            now,
            Some(approval.execution_id().clone()),
            Some(ApprovalExecution {
                store: approval_store,
                id: approval_id,
            }),
            false,
        )
    }

    pub(crate) fn run_agent_with_approval(
        &self,
        intent: TaskIntent,
        session: Session,
        approval_store: &ApprovalStore,
        approval_id: &str,
        now: Timestamp,
    ) -> Result<RunSummary, RuntimeError> {
        let approval = approval_store
            .inspect(approval_id, session.principal_id())
            .map_err(RuntimeError::Approval)?;
        if approval.session_id() != session.id() {
            return Err(RuntimeError::Approval(ApprovalError::ScopeMismatch));
        }
        self.run_internal(
            intent,
            session,
            now,
            Some(approval.execution_id().clone()),
            Some(ApprovalExecution {
                store: approval_store,
                id: approval_id,
            }),
            true,
        )
    }

    fn run_internal(
        &self,
        intent: TaskIntent,
        session: Session,
        now: Timestamp,
        execution_id_override: Option<ExecutionId>,
        approval: Option<ApprovalExecution<'_>>,
        allow_unrequired_approval: bool,
    ) -> Result<RunSummary, RuntimeError> {
        let execution_id = ExecutionId::new(format!(
            "execution-{}",
            self.next_execution.fetch_add(1, Ordering::Relaxed)
        ))
        .map_err(|_| RuntimeError::InvalidIntent("could not allocate execution ID"))?;
        let execution_id = execution_id_override.unwrap_or(execution_id);
        let selection = self
            .shadow_council
            .select(&intent)
            .map_err(|error| match error {
                RoutingError::NoDefaultHarness => RuntimeError::NoDefaultHarness,
            })?;
        let harness = self.find_harness(selection.harness_id())?;
        if !harness.is_runnable() {
            return Err(RuntimeError::NonExecutableHarness {
                id: harness.manifest().id().clone(),
                kind: harness.manifest().kind(),
            });
        }
        let gene_id = selection
            .gene_id()
            .cloned()
            .unwrap_or_else(|| default_gene_id(&intent));
        let gene = harness
            .genes()
            .iter()
            .find(|gene| gene.manifest().id() == &gene_id)
            .ok_or(RuntimeError::UnknownGene)?;
        let (input, payload) = coding_input(&intent, &gene_id, &session, &execution_id)?;
        let requests = gene.plan(&input).map_err(RuntimeError::Planning)?;
        let mut summary = RunSummary {
            execution_id: execution_id.clone(),
            selected_harness: harness.manifest().id().clone(),
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
            let decision_for_authorization = decision.clone();
            let permit = match decision {
                ParliamentDecision::Allow { ref reason, .. } => {
                    if approval.is_some() && !allow_unrequired_approval {
                        self.record_failure(
                            &session,
                            &execution_id,
                            &mut summary,
                            &RuntimeError::ApprovalNotRequired,
                        );
                        return Ok(summary);
                    }
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
                    match self.reference_monitor.authorize(
                        request.clone(),
                        decision_for_authorization,
                        now,
                    ) {
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
                    if let Some(approval) = approval.as_ref() {
                        let grant = approval
                            .store
                            .consume_grant(
                                approval.id,
                                request.principal_id(),
                                request.session_id(),
                                request.execution_id(),
                                request.gene_id(),
                                request.request_digest(),
                                now,
                            )
                            .map_err(RuntimeError::Approval)?;
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
                                reason: format!("explicit approval consumed: {reason}"),
                            },
                        ));
                        match self.reference_monitor.authorize_after_approval_with_grant(
                            request.clone(),
                            decision_for_authorization,
                            &grant,
                            now,
                        ) {
                            Ok(permit) => permit,
                            Err(error) => {
                                let failure = RuntimeError::Authorization(error);
                                self.record_failure(
                                    &session,
                                    &execution_id,
                                    &mut summary,
                                    &failure,
                                );
                                return Ok(summary);
                            }
                        }
                    } else {
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
            if let Err(error) = self.execute_request(
                &session,
                &execution_id,
                &consumed,
                payload.as_deref(),
                now,
                &mut summary,
            ) {
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

    fn find_harness(&self, harness_id: &HarnessId) -> Result<&dyn Harness, RuntimeError> {
        self.harnesses
            .find(harness_id)
            .ok_or_else(|| RuntimeError::UnsupportedHarness(harness_id.clone()))
    }

    fn execute_request(
        &self,
        session: &Session,
        execution_id: &ExecutionId,
        permit: &ConsumedPermit,
        payload: Option<&[u8]>,
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
                let (receipt, result) = if permit.request().gene_id().as_str() == "workspace.search"
                {
                    let target = self.workspace.path(".").map_err(RuntimeError::Filesystem)?;
                    let response = self.filesystem.search(permit, &target, path, now);
                    let receipt = response.receipt().clone();
                    let result = response
                        .into_result()
                        .map(|matches| matches.join("\n").into_bytes());
                    (receipt, result)
                } else {
                    let target = self
                        .workspace
                        .path(path)
                        .map_err(RuntimeError::Filesystem)?;
                    let response = self.filesystem.read(permit, &target, now);
                    let receipt = response.receipt().clone();
                    let result = response.into_result();
                    (receipt, result)
                };
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
                result
                    .map(|bytes| output.output = Some(bytes))
                    .map_err(RuntimeError::Filesystem)
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
            Capability::FilesystemWrite => {
                let path = match permit.request().target() {
                    EffectTarget::Path { path } => path,
                    _ => {
                        return Err(RuntimeError::UnsupportedOperation(
                            Capability::FilesystemWrite,
                        ));
                    }
                };
                let target = self
                    .workspace
                    .path(path)
                    .map_err(RuntimeError::Filesystem)?;
                let content =
                    payload.ok_or(RuntimeError::Filesystem(FilesystemError::PermissionDenied))?;
                let response = self.filesystem.write_patch(permit, &target, content, now);
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
                        Some(receipt),
                    ),
                    EventPayload::Effect {
                        capability: permit.request().capability().as_str().to_owned(),
                        request_digest: permit.request().request_digest().clone(),
                    },
                ));
                result
                    .map(|_| ())
                    .map_err(|error| RuntimeError::Filesystem(error.clone()))
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
        RuntimeError::NoDefaultHarness => "no_default_harness",
        RuntimeError::UnsupportedHarness(_) => "unsupported_harness",
        RuntimeError::NonExecutableHarness { .. } => "non_executable_harness",
        RuntimeError::UnknownGene => "unknown_gene",
        RuntimeError::Planning(_) => "planning_failed",
        RuntimeError::Denied(_) => "denied",
        RuntimeError::ApprovalRequired(_) => "approval_required",
        RuntimeError::Approval(_) => "approval_failed",
        RuntimeError::ApprovalNotRequired => "approval_not_required",
        RuntimeError::Authorization(_) => "authorization_failed",
        RuntimeError::Permit(_) => "permit_failed",
        RuntimeError::Provider(_) => "provider_failed",
        RuntimeError::Request(_) => "request_failed",
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
) -> Result<(GeneInput, Option<Vec<u8>>), RuntimeError> {
    let context = PlanningContext::new(
        execution_id.clone(),
        session.id().clone(),
        session.principal_id().clone(),
        session.workspace_id().clone(),
    );
    let summary = intent.summary();
    let (action, remainder) = summary.split_once(':').unwrap_or((summary, ""));
    let action = action.to_ascii_lowercase();
    let payload = if gene_id.as_str() == "patch.apply" {
        remainder
            .split_once(':')
            .map(|(_, content)| content.as_bytes().to_vec())
    } else {
        None
    };
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
    let input = request.into_gene_input().map_err(RuntimeError::Planning)?;
    Ok((input, payload))
}

struct ApprovalExecution<'a> {
    store: &'a ApprovalStore,
    id: &'a str,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_harnesses::HarnessCatalog;
    use pandora_types::{
        Capability, GeneId, HarnessId, Operation, PackageCompatibility, PackageDependency,
        PackageKind, PackageManifest, PolicyContext, PrincipalId, Session, SessionId, TaskIntent,
        TenantId, Timestamp, TrustEvidence, WorkspaceId, hash_artifact,
    };
    use std::path::PathBuf;

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
    fn an_admitted_domain_profile_runs_through_the_existing_controller() {
        let fixture = Fixture::new();
        let artifact = b"domain profile";
        let package = PackageManifest::new(
            "example/domain",
            "1.0.0",
            PackageKind::DomainHarness,
            "publisher",
            hash_artifact(artifact),
            vec![PackageDependency::new("workspace.read", "0.1.0", false).unwrap()],
            PackageCompatibility::new("pandora>=2.0.0").unwrap(),
            "Apache-2.0",
            TrustEvidence::unsigned(),
        )
        .unwrap();
        let harnesses = HarnessCatalog::builtins()
            .with_declarative_domain(&package)
            .unwrap();
        let controller = ExecutionController::with_policy_and_harnesses(
            fixture.root.clone(),
            PolicyContext::read_only_workspace(),
            harnesses,
        );
        let intent = TaskIntent::new("read:README.md")
            .unwrap()
            .with_harness(HarnessId::new("example/domain").unwrap())
            .with_gene(GeneId::new("workspace.read").unwrap());

        let summary = controller
            .run_at(intent, fixture.session(), Timestamp::from_unix_seconds(10))
            .unwrap();

        assert_eq!(summary.status(), &RunStatus::Completed);
        assert_eq!(summary.output().unwrap(), b"fixture\n");
        assert_eq!(summary.selected_harness().as_str(), "example/domain");
        assert_eq!(summary.selected_gene().as_str(), "workspace.read");
    }

    #[test]
    fn search_returns_matching_workspace_files() {
        let fixture = Fixture::new();
        std::fs::create_dir(fixture.path.join("src")).unwrap();
        std::fs::write(fixture.path.join("src/lib.rs"), b"needle\n").unwrap();
        let controller = ExecutionController::new(fixture.root.clone());

        let summary = controller
            .run_at(
                TaskIntent::new("search:needle").unwrap(),
                fixture.session(),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();

        assert_eq!(summary.status(), &RunStatus::Completed);
        assert_eq!(summary.output().unwrap(), b"src/lib.rs");
        assert_eq!(summary.selected_gene().as_str(), "workspace.search");
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

    #[test]
    fn unsupported_harness_is_rejected_before_gene_planning() {
        let fixture = Fixture::new();
        let controller = ExecutionController::new(fixture.root.clone());
        let harness_id = HarnessId::new("design-domain").unwrap();
        let intent = TaskIntent::new("read:README.md")
            .unwrap()
            .with_harness(harness_id.clone());

        assert_eq!(
            controller.run_at(intent, fixture.session(), Timestamp::from_unix_seconds(10)),
            Err(RuntimeError::UnsupportedHarness(harness_id))
        );
    }

    #[test]
    fn source_and_meta_harnesses_are_not_execution_targets() {
        let fixture = Fixture::new();
        let controller = ExecutionController::new(fixture.root.clone());

        for harness_id in ["core-source", "coordination-meta"] {
            let harness_id = HarnessId::new(harness_id).unwrap();
            let intent = TaskIntent::new("read:README.md")
                .unwrap()
                .with_harness(harness_id.clone());
            let kind = if harness_id.as_str() == "core-source" {
                HarnessKind::Source
            } else {
                HarnessKind::Meta
            };

            assert_eq!(
                controller.run_at(intent, fixture.session(), Timestamp::from_unix_seconds(10),),
                Err(RuntimeError::NonExecutableHarness {
                    id: harness_id,
                    kind,
                })
            );
        }
    }

    struct Fixture {
        path: PathBuf,
        root: WorkspaceRoot,
    }

    impl Fixture {
        fn new() -> Self {
            let path = crate::test_support::new_temp_dir("pandora-controller-test").unwrap();
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
}
