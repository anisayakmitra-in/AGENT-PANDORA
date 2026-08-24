use crate::execution_profile::{ExecutionProfileAssemblyError, assemble_execution_profile};
use crate::executors::{
    FilesystemError, FilesystemExecutor, GitWorktreeExecutor, ProcessError, ProcessExecutor,
    ProviderExecutor, ProviderResult, VerificationCommand, VerificationOptions, WorkspaceRoot,
    WorktreeCommand, WorktreeResult,
};
use crate::hooks::{HookDecision, HookPoint, LifecycleHooks};
use crate::mcp::{
    McpError, McpExecutor, McpFailure, McpInvocation, McpProtocolMode, McpServer, McpStart,
    McpStartOutcome, McpStdioConfig, McpWireEra, SpawnPurpose, map_catalog_error, map_tool_error,
};
use crate::mcp_catalog::{McpCatalogRevision, McpCatalogSupervisor, McpCatalogTool};
use crate::parliament::Parliament;
use crate::reference_monitor::{AuthorizationError, ReferenceMonitor};
use crate::shadow_council::{RoutingError, ShadowCouncil};
use crate::wasm::{WasmError, WasmExecutor, WasmGeneRequest};
use crate::{ApprovalError, ApprovalStore, ConsumedPermit, PermitError, ToolContext, ToolEngine};
use pandora_harnesses::{
    CodingRequest, DesignRequest, HarnessCatalog, PlanningContext, ResearchRequest,
    canonical_harness_binding_digest, coding_static_output, design_static_output, is_design_gene,
    is_research_gene, research_static_output,
};
use pandora_provider::{ModelRequest, Provider, ProviderError, ProviderManifest};
use pandora_types::{
    Capability, EffectReceipt, EffectTarget, EventContext, EventId, EventPayload, EventType,
    ExecutionId, ExecutionProfile, ExecutionProfileBinding, ExecutionProfileBindingKind, GeneError,
    GeneId, GeneInput, GeneManifest, Harness, HarnessId, HarnessKind, Operation, OperationRequest,
    ParliamentDecision, PolicyContext, PrincipalId, RequestError, ResourceScope, RuntimeEvent,
    SecretReference, Session, SessionId, TaskIntent, Timestamp, hash_artifact,
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
    Wasm(WasmError),
    ExecutionProfile(ExecutionProfileAssemblyError),
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
    wasm: WasmExecutor,
    hooks: LifecycleHooks,
    mcp_catalogs: McpCatalogSupervisor,
    next_execution: AtomicU64,
    next_event: AtomicU64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeExecutionContext {
    execution_id: ExecutionId,
    session_id: SessionId,
    principal_id: PrincipalId,
}

impl WorktreeExecutionContext {
    pub fn new(
        execution_id: ExecutionId,
        session_id: SessionId,
        principal_id: PrincipalId,
    ) -> Self {
        Self {
            execution_id,
            session_id,
            principal_id,
        }
    }
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
        Self::with_policy_and_harnesses_and_hooks(
            workspace,
            policy,
            harnesses,
            LifecycleHooks::new(),
        )
    }

    pub fn with_hooks(workspace: WorkspaceRoot, hooks: LifecycleHooks) -> Self {
        Self::with_policy_and_harnesses_and_hooks(
            workspace,
            PolicyContext::read_only_workspace(),
            HarnessCatalog::builtins(),
            hooks,
        )
    }

    pub fn with_policy_and_harnesses_and_hooks(
        workspace: WorkspaceRoot,
        policy: PolicyContext,
        harnesses: HarnessCatalog,
        hooks: LifecycleHooks,
    ) -> Self {
        let policy_version = policy.policy_version();
        Self {
            filesystem: FilesystemExecutor::for_workspace(workspace.clone()),
            process: ProcessExecutor::new(workspace.clone()),
            provider: ProviderExecutor::new(),
            wasm: WasmExecutor::new(),
            workspace,
            shadow_council: ShadowCouncil::new(),
            parliament: Parliament::new(policy_version),
            reference_monitor: ReferenceMonitor::new_with_policy(policy.clone(), 60),
            policy,
            harnesses,
            hooks,
            mcp_catalogs: McpCatalogSupervisor::new(),
            next_execution: AtomicU64::new(1),
            next_event: AtomicU64::new(1),
        }
    }

    pub fn policy_version(&self) -> u32 {
        self.policy.policy_version()
    }

    pub fn with_wasm_executor(mut self, wasm: WasmExecutor) -> Self {
        self.wasm = wasm;
        self
    }

    fn execution_profile(
        &self,
        executor_id: &str,
        bindings: Vec<ExecutionProfileBinding>,
    ) -> Result<ExecutionProfile, RuntimeError> {
        assemble_execution_profile(
            &self.workspace,
            self.policy.policy_version(),
            executor_id,
            bindings,
        )
        .map_err(RuntimeError::ExecutionProfile)
    }

    pub(crate) fn trusted_harness_gene_ids(
        &self,
        harness_id: &HarnessId,
    ) -> Result<Vec<GeneId>, RuntimeError> {
        let harness = self.find_harness(harness_id)?;
        if !harness.is_runnable() {
            return Err(RuntimeError::NonExecutableHarness {
                id: harness.manifest().id().clone(),
                kind: harness.manifest().kind(),
            });
        }
        Ok(harness.manifest().owned_genes().to_vec())
    }

    pub fn execute_worktree(
        &self,
        executor: &GitWorktreeExecutor,
        command: &WorktreeCommand,
        context: WorktreeExecutionContext,
        now: Timestamp,
    ) -> Result<WorktreeResult, RuntimeError> {
        let gene_id = match command.operation() {
            "git_worktree_create" => "coordination.worktree.create",
            "git_worktree_remove" => "coordination.worktree.remove",
            _ => {
                return Err(RuntimeError::InvalidIntent(
                    "unsupported worktree operation",
                ));
            }
        };
        let managed_root = executor
            .managed_root()
            .to_str()
            .ok_or(RuntimeError::InvalidIntent(
                "managed worktree root must be Unicode",
            ))?;
        let execution_profile = self.execution_profile(
            "git_worktree",
            worktree_profile_bindings(gene_id, command.operation(), command.spec(), managed_root)?,
        )?;
        let request = OperationRequest::new(
            context.execution_id,
            context.session_id,
            context.principal_id,
            execution_profile,
            GeneId::new(gene_id).expect("built-in worktree Gene ID is valid"),
            None,
            Capability::ProcessExecute,
            Operation::Execute,
            EffectTarget::process(command.spec()),
            ResourceScope::path(managed_root),
        )
        .map_err(RuntimeError::Request)?;
        if let Some(reason) = self.hook_denial(&request) {
            return Err(RuntimeError::Denied(reason));
        }
        let decision = self.parliament.decide(&request, &self.policy);
        let permit = match decision {
            ParliamentDecision::Allow { .. } => self
                .reference_monitor
                .authorize(request.clone(), decision, now)
                .map_err(RuntimeError::Authorization)?,
            ParliamentDecision::Deny { reason, .. } => return Err(RuntimeError::Denied(reason)),
            ParliamentDecision::RequireApproval { reason, .. } => {
                return Err(RuntimeError::ApprovalRequired(reason));
            }
        };
        let consumed = self
            .reference_monitor
            .store()
            .consume(permit, &request, now)
            .map_err(RuntimeError::Permit)?;
        Ok(executor.execute(&consumed, command, now))
    }

    pub fn start_mcp(
        &self,
        tool_engine: &ToolEngine,
        config: McpStdioConfig,
        session: &Session,
        now: Timestamp,
    ) -> Result<McpStart, McpFailure> {
        let mut receipts = Vec::new();
        let mut events = Vec::new();
        let config_digest = config
            .catalog_config_digest()
            .map_err(|error| McpFailure::new(error, receipts.clone(), events.clone()))?;
        let catalog_reservation = self
            .mcp_catalogs
            .reserve(config.server_id(), config_digest)
            .map_err(map_catalog_error)
            .map_err(|error| McpFailure::new(error, receipts.clone(), events.clone()))?;
        let (server, selected_era, downgraded, selected_request) = match config.mode() {
            McpProtocolMode::ModernOnly => {
                let request = self
                    .mcp_spawn_request(&config, SpawnPurpose::Modern, session)
                    .map_err(|error| McpFailure::new(error, receipts.clone(), events.clone()))?;
                let consumed = self
                    .authorize_mcp_request(&request, session, now, &mut events)
                    .map_err(|error| McpFailure::new(error, receipts.clone(), events.clone()))?;
                let execution = McpExecutor::start_modern(
                    &consumed,
                    tool_engine.clone(),
                    config,
                    &catalog_reservation,
                    SpawnPurpose::Modern,
                    false,
                    now,
                );
                let (result, receipt) = execution.into_parts();
                self.record_mcp_completion(
                    &request,
                    session,
                    &receipt,
                    result.as_ref().err(),
                    &mut events,
                );
                receipts.push(receipt);
                let server = match result {
                    Ok(McpStartOutcome::Connected(server)) => *server,
                    Ok(McpStartOutcome::LegacyIdentified) => {
                        return Err(McpFailure::new(McpError::RequestRejected, receipts, events));
                    }
                    Err(error) => return Err(McpFailure::new(error, receipts, events)),
                };
                (server, McpWireEra::Modern, false, request)
            }
            McpProtocolMode::LegacyOnly => {
                let request = self
                    .mcp_spawn_request(&config, SpawnPurpose::Legacy, session)
                    .map_err(|error| McpFailure::new(error, receipts.clone(), events.clone()))?;
                let consumed = self
                    .authorize_mcp_request(&request, session, now, &mut events)
                    .map_err(|error| McpFailure::new(error, receipts.clone(), events.clone()))?;
                let execution = McpExecutor::start_legacy(
                    &consumed,
                    tool_engine.clone(),
                    config,
                    &catalog_reservation,
                    now,
                );
                let (result, receipt) = execution.into_parts();
                self.record_mcp_completion(
                    &request,
                    session,
                    &receipt,
                    result.as_ref().err(),
                    &mut events,
                );
                receipts.push(receipt);
                let server = result
                    .map_err(|error| McpFailure::new(error, receipts.clone(), events.clone()))?;
                (server, McpWireEra::Legacy, false, request)
            }
            McpProtocolMode::Auto => {
                let probe_request = self
                    .mcp_spawn_request(&config, SpawnPurpose::ModernProbe, session)
                    .map_err(|error| McpFailure::new(error, receipts.clone(), events.clone()))?;
                let consumed = self
                    .authorize_mcp_request(&probe_request, session, now, &mut events)
                    .map_err(|error| McpFailure::new(error, receipts.clone(), events.clone()))?;
                let execution = McpExecutor::start_modern(
                    &consumed,
                    tool_engine.clone(),
                    config.clone(),
                    &catalog_reservation,
                    SpawnPurpose::ModernProbe,
                    true,
                    now,
                );
                let (probe_result, receipt) = execution.into_parts();
                self.record_mcp_completion(
                    &probe_request,
                    session,
                    &receipt,
                    probe_result.as_ref().err(),
                    &mut events,
                );
                receipts.push(receipt);
                match probe_result {
                    Ok(McpStartOutcome::Connected(server)) => {
                        (*server, McpWireEra::Modern, false, probe_request)
                    }
                    Ok(McpStartOutcome::LegacyIdentified) => {
                        let legacy_request = self
                            .mcp_spawn_request(&config, SpawnPurpose::Legacy, session)
                            .map_err(|error| {
                                McpFailure::new(error, receipts.clone(), events.clone())
                            })?;
                        let consumed = self
                            .authorize_mcp_request(&legacy_request, session, now, &mut events)
                            .map_err(|error| {
                                McpFailure::new(error, receipts.clone(), events.clone())
                            })?;
                        let execution = McpExecutor::start_legacy(
                            &consumed,
                            tool_engine.clone(),
                            config,
                            &catalog_reservation,
                            now,
                        );
                        let (legacy_result, receipt) = execution.into_parts();
                        self.record_mcp_completion(
                            &legacy_request,
                            session,
                            &receipt,
                            legacy_result.as_ref().err(),
                            &mut events,
                        );
                        receipts.push(receipt);
                        let server = legacy_result.map_err(|error| {
                            McpFailure::new(error, receipts.clone(), events.clone())
                        })?;
                        (server, McpWireEra::Legacy, true, legacy_request)
                    }
                    Err(error) => return Err(McpFailure::new(error, receipts, events)),
                }
            }
        };
        events.push(self.event(
            EventType::McpEraSelected,
            self.context(
                session,
                selected_request.execution_id(),
                None,
                Some(selected_request.gene_id().clone()),
                receipts.last().cloned(),
            ),
            EventPayload::McpEra {
                server: server.server_id().to_owned(),
                era: selected_era.as_str().to_owned(),
                downgraded,
            },
        ));
        Ok(McpStart::new(
            server,
            selected_era,
            downgraded,
            receipts,
            events,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn invoke_mcp(
        &self,
        tool_engine: &ToolEngine,
        server: &mut McpServer,
        local_tool: &str,
        arguments: serde_json::Value,
        idempotency_key: &str,
        session: &Session,
        now: Timestamp,
    ) -> Result<McpInvocation, McpFailure> {
        let mut receipts = Vec::new();
        let mut events = Vec::new();
        let remote_tool = server
            .remote_tool(local_tool)
            .map(str::to_owned)
            .map_err(|error| McpFailure::new(error, receipts.clone(), events.clone()))?;
        let invocation_payload = server
            .invocation_payload(local_tool, &arguments)
            .map_err(|error| McpFailure::new(error, receipts.clone(), events.clone()))?;
        let execution_id = self
            .next_mcp_execution_id()
            .map_err(|error| McpFailure::new(error, receipts.clone(), events.clone()))?;
        let revision = server.catalog_revision();
        let tool = revision
            .tool(local_tool)
            .ok_or(McpError::UnknownTool)
            .map_err(|error| McpFailure::new(error, receipts.clone(), events.clone()))?;
        let execution_profile = self
            .execution_profile(
                "mcp_stdio",
                mcp_invocation_profile_bindings(revision, local_tool, tool).map_err(|_| {
                    McpFailure::new(McpError::RequestRejected, receipts.clone(), events.clone())
                })?,
            )
            .map_err(|_| {
                McpFailure::new(McpError::RequestRejected, receipts.clone(), events.clone())
            })?;
        let context = ToolContext::new(
            execution_id,
            session.id().clone(),
            session.principal_id().clone(),
            execution_profile,
            None,
        );
        let plan = tool_engine
            .plan_with_payload(
                local_tool,
                &context,
                arguments,
                idempotency_key,
                EffectTarget::mcp(server.server_id(), &remote_tool),
                ResourceScope::none(),
                &invocation_payload,
            )
            .map_err(map_tool_error)
            .map_err(|error| McpFailure::new(error, receipts.clone(), events.clone()))?;
        let consumed = self
            .authorize_mcp_request(plan.request(), session, now, &mut events)
            .map_err(|error| McpFailure::new(error, receipts.clone(), events.clone()))?;
        let execution = McpExecutor::invoke(&consumed, server, &plan, now);
        let (result, receipt) = execution.into_parts();
        self.record_mcp_completion(
            plan.request(),
            session,
            &receipt,
            result.as_ref().err(),
            &mut events,
        );
        receipts.push(receipt);
        let result =
            result.map_err(|error| McpFailure::new(error, receipts.clone(), events.clone()))?;
        Ok(McpInvocation::new(result, receipts, events))
    }

    fn mcp_spawn_request(
        &self,
        config: &McpStdioConfig,
        purpose: SpawnPurpose,
        session: &Session,
    ) -> Result<OperationRequest, McpError> {
        let execution_id = self.next_mcp_execution_id()?;
        let payload = config.authorization_payload(purpose)?;
        let config_digest = config.catalog_config_digest()?;
        let gene_id = format!("mcp.{}.spawn", config.server_id());
        let execution_profile = self
            .execution_profile(
                "mcp_stdio",
                vec![
                    profile_binding(
                        ExecutionProfileBindingKind::Gene,
                        &gene_id,
                        Some(env!("CARGO_PKG_VERSION")),
                        &gene_id,
                    ),
                    profile_binding(
                        ExecutionProfileBindingKind::Configuration,
                        config.server_id(),
                        Some(purpose.as_str()),
                        &config_digest,
                    ),
                ]
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| McpError::RequestRejected)?,
            )
            .map_err(|_| McpError::RequestRejected)?;
        OperationRequest::new(
            execution_id,
            session.id().clone(),
            session.principal_id().clone(),
            execution_profile,
            GeneId::new(gene_id).map_err(|_| McpError::RequestRejected)?,
            None,
            Capability::ProcessExecute,
            Operation::Execute,
            EffectTarget::process(config.program()),
            ResourceScope::none(),
        )
        .map_err(|_| McpError::RequestRejected)?
        .with_payload_digest(&payload)
        .map_err(|_| McpError::RequestRejected)
    }

    fn next_mcp_execution_id(&self) -> Result<ExecutionId, McpError> {
        ExecutionId::new(format!(
            "mcp-execution-{}",
            self.next_execution.fetch_add(1, Ordering::Relaxed)
        ))
        .map_err(|_| McpError::RequestRejected)
    }

    fn authorize_mcp_request(
        &self,
        request: &OperationRequest,
        session: &Session,
        now: Timestamp,
        events: &mut Vec<RuntimeEvent>,
    ) -> Result<ConsumedPermit, McpError> {
        let context = || {
            self.context(
                session,
                request.execution_id(),
                None,
                Some(request.gene_id().clone()),
                None,
            )
        };
        events.push(self.event(
            EventType::EffectRequested,
            context(),
            EventPayload::Effect {
                capability: request.capability().as_str().to_owned(),
                request_digest: request.request_digest().clone(),
            },
        ));
        if let Some(reason) = self.hook_denial(request) {
            events.push(self.event(
                EventType::PolicyDenied,
                context(),
                EventPayload::Policy { reason },
            ));
            return Err(McpError::PolicyDenied);
        }
        let decision = self.parliament.decide(request, &self.policy);
        let permit = match decision {
            ParliamentDecision::Allow { ref reason, .. } => {
                events.push(self.event(
                    EventType::PolicyApproved,
                    context(),
                    EventPayload::Policy {
                        reason: reason.clone(),
                    },
                ));
                self.reference_monitor
                    .authorize(request.clone(), decision, now)
                    .map_err(|_| McpError::AuthorizationFailed)?
            }
            ParliamentDecision::Deny { reason, .. } => {
                events.push(self.event(
                    EventType::PolicyDenied,
                    context(),
                    EventPayload::Policy { reason },
                ));
                return Err(McpError::PolicyDenied);
            }
            ParliamentDecision::RequireApproval { reason, .. } => {
                events.push(self.event(
                    EventType::ApprovalRequired,
                    context(),
                    EventPayload::Policy { reason },
                ));
                return Err(McpError::ApprovalRequired);
            }
        };
        self.reference_monitor
            .store()
            .consume(permit, request, now)
            .map_err(|_| McpError::PermitFailed)
    }

    fn record_mcp_completion(
        &self,
        request: &OperationRequest,
        session: &Session,
        receipt: &EffectReceipt,
        error: Option<&McpError>,
        events: &mut Vec<RuntimeEvent>,
    ) {
        let context = self.context(
            session,
            request.execution_id(),
            None,
            Some(request.gene_id().clone()),
            Some(receipt.clone()),
        );
        events.push(self.event(
            EventType::EffectCompleted,
            context.clone(),
            EventPayload::Effect {
                capability: request.capability().as_str().to_owned(),
                request_digest: request.request_digest().clone(),
            },
        ));
        if let Some(error) = error {
            events.push(self.event(
                EventType::ExecutionFailed,
                context,
                EventPayload::Failure {
                    code: error.code().to_owned(),
                },
            ));
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
        self.run_internal(intent, session, now, None, None, false)
    }

    pub fn invoke_provider(
        &self,
        provider: &dyn Provider,
        request: ModelRequest,
        session: &Session,
        now: Timestamp,
    ) -> Result<ProviderResult, RuntimeError> {
        let fallback_request = request.clone();
        let primary = self.invoke_provider_once(provider, request, session, now)?;
        let should_fallback = primary
            .result()
            .err()
            .is_some_and(ProviderError::is_retryable);
        let Some(fallback) = provider.fallback_provider().filter(|_| should_fallback) else {
            return Ok(primary);
        };
        let fallback_request = fallback_request.for_provider(
            fallback.manifest().id().clone(),
            fallback.manifest().default_model().clone(),
        );
        let Ok(mut fallback_result) =
            self.invoke_provider_once(fallback, fallback_request, session, now)
        else {
            return Ok(primary);
        };
        fallback_result.prepend_receipt(primary.receipt().clone());
        Ok(fallback_result)
    }

    fn invoke_provider_once(
        &self,
        provider: &dyn Provider,
        request: ModelRequest,
        session: &Session,
        now: Timestamp,
    ) -> Result<ProviderResult, RuntimeError> {
        let authorization_payload = request
            .authorization_payload_for(provider.manifest())
            .map_err(RuntimeError::Provider)?;
        let execution_id = ExecutionId::new(format!(
            "provider-execution-{}",
            self.next_execution.fetch_add(1, Ordering::Relaxed)
        ))
        .map_err(|_| RuntimeError::InvalidIntent("could not allocate provider execution ID"))?;
        let model_id = request.model_id().as_str();
        let execution_profile = self.execution_profile(
            "provider",
            provider_profile_bindings(provider.manifest(), model_id)?,
        )?;
        let operation_request = OperationRequest::new(
            execution_id,
            session.id().clone(),
            session.principal_id().clone(),
            execution_profile,
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
        .map_err(RuntimeError::Request)?
        .with_payload_digest(&authorization_payload)
        .map_err(RuntimeError::Request)?;
        if let Some(reason) = self.hook_denial(&operation_request) {
            return Err(RuntimeError::Denied(reason));
        }
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
        let harness_evidence = canonical_harness_binding_digest(harness.manifest());
        let mut profile_bindings = vec![
            profile_binding(
                ExecutionProfileBindingKind::Harness,
                harness.manifest().id().as_str(),
                Some(harness.manifest().version()),
                &harness_evidence,
            )?,
            gene_profile_binding(gene.manifest())?,
        ];
        if gene
            .manifest()
            .capabilities()
            .contains(&Capability::WasmExecute)
        {
            profile_bindings.push(self.wasm_artifact_binding(gene.manifest())?);
        }
        let execution_profile =
            self.execution_profile(executor_for_gene(gene.manifest()), profile_bindings)?;
        let (input, payload) = if gene
            .manifest()
            .capabilities()
            .contains(&Capability::WasmExecute)
        {
            wasm_input(&intent, &session, &execution_id, execution_profile)?
        } else if is_research_gene(&gene_id) {
            research_input(
                &intent,
                &gene_id,
                &session,
                &execution_id,
                execution_profile,
            )?
        } else if is_design_gene(&gene_id) {
            design_input(
                &intent,
                &gene_id,
                &session,
                &execution_id,
                execution_profile,
            )?
        } else {
            coding_input(
                &intent,
                &gene_id,
                &session,
                &execution_id,
                execution_profile,
            )?
        };
        let requests = gene.plan(&input).map_err(RuntimeError::Planning)?;
        let static_output = coding_static_output(&gene_id)
            .or_else(|| research_static_output(&gene_id))
            .or_else(|| design_static_output(&gene_id))
            .map(|value| value.as_bytes().to_vec());
        let mut summary = RunSummary {
            execution_id: execution_id.clone(),
            selected_harness: harness.manifest().id().clone(),
            selected_gene: gene_id,
            status: RunStatus::Completed,
            output: static_output,
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
            if let Some(reason) = self.hook_denial(&request) {
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

    fn hook_denial(&self, request: &OperationRequest) -> Option<String> {
        match self.hooks.evaluate(HookPoint::BeforeAuthorization, request) {
            HookDecision::Continue => None,
            HookDecision::Deny { hook_id, reason } => {
                Some(format!("lifecycle hook {hook_id} denied request: {reason}"))
            }
        }
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
                let gene_id = permit.request().gene_id().as_str();
                let (receipt, result) = if matches!(
                    gene_id,
                    "daedalus.audit" | "evidence.inventory" | "design.inventory"
                ) {
                    let target = self
                        .workspace
                        .path(path)
                        .map_err(RuntimeError::Filesystem)?;
                    let response = self.filesystem.inventory(permit, &target, now);
                    let receipt = response.receipt().clone();
                    let result = response
                        .into_result()
                        .map(|files| files.join("\n").into_bytes());
                    (receipt, result)
                } else if matches!(
                    gene_id,
                    "workspace.search"
                        | "ariadne.debt"
                        | "evidence.search"
                        | "citation.inventory"
                        | "design.tokens"
                        | "accessibility.evidence"
                ) {
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
                let bytes = result.map_err(RuntimeError::Filesystem)?;
                if matches!(
                    gene_id,
                    "ariadne.debt"
                        | "source.compare"
                        | "citation.inventory"
                        | "design.tokens"
                        | "design.compare"
                        | "accessibility.evidence"
                ) {
                    append_labeled_output(output, path, &bytes);
                } else {
                    output.output = Some(bytes);
                }
                Ok(())
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
            Capability::WasmExecute => {
                let payload = payload.ok_or(RuntimeError::Wasm(WasmError::InvalidInput))?;
                let response = self.wasm.execute(permit, payload, now);
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
                output.output = Some(response.into_result().map_err(RuntimeError::Wasm)?);
                Ok(())
            }
            capability => Err(RuntimeError::UnsupportedOperation(capability)),
        }
    }

    fn wasm_artifact_binding(
        &self,
        manifest: &GeneManifest,
    ) -> Result<ExecutionProfileBinding, RuntimeError> {
        let content_hash = self
            .wasm
            .content_hash(manifest.id().as_str(), manifest.version())
            .ok_or(RuntimeError::Wasm(WasmError::UnknownPackage))?;
        ExecutionProfileBinding::new(
            ExecutionProfileBindingKind::Artifact,
            manifest.id().as_str(),
            Some(manifest.version()),
            content_hash,
        )
        .map_err(|error| {
            RuntimeError::ExecutionProfile(ExecutionProfileAssemblyError::Contract(error))
        })
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

fn profile_binding(
    kind: ExecutionProfileBindingKind,
    id: &str,
    version: Option<&str>,
    canonical_evidence: &str,
) -> Result<ExecutionProfileBinding, RuntimeError> {
    ExecutionProfileBinding::new(
        kind,
        id,
        version,
        hash_artifact(canonical_evidence.as_bytes()),
    )
    .map_err(|error| RuntimeError::ExecutionProfile(ExecutionProfileAssemblyError::Contract(error)))
}

fn worktree_profile_bindings(
    gene_id: &str,
    operation: &str,
    command_spec: &str,
    managed_root: &str,
) -> Result<Vec<ExecutionProfileBinding>, RuntimeError> {
    Ok(vec![
        profile_binding(
            ExecutionProfileBindingKind::Gene,
            gene_id,
            Some(env!("CARGO_PKG_VERSION")),
            gene_id,
        )?,
        profile_binding(
            ExecutionProfileBindingKind::Configuration,
            operation,
            None,
            command_spec,
        )?,
        profile_binding(
            ExecutionProfileBindingKind::Configuration,
            "managed_worktree_root",
            None,
            managed_root,
        )?,
    ])
}

fn mcp_invocation_profile_bindings(
    revision: &McpCatalogRevision,
    local_tool: &str,
    tool: &McpCatalogTool,
) -> Result<Vec<ExecutionProfileBinding>, RuntimeError> {
    let generation = revision.generation().to_string();
    let configuration_evidence = format!(
        "mcp_configuration\0{}\0{}\0{}",
        revision.server_id(),
        revision.process_id(),
        revision.config_digest()
    );
    Ok(vec![
        profile_binding(
            ExecutionProfileBindingKind::ToolCatalog,
            revision.server_id(),
            Some(&generation),
            revision.catalog_digest(),
        )?,
        profile_binding(
            ExecutionProfileBindingKind::Gene,
            local_tool,
            Some(revision.protocol_era().as_str()),
            tool.schema_digest(),
        )?,
        profile_binding(
            ExecutionProfileBindingKind::Configuration,
            revision.server_id(),
            Some(revision.protocol_era().as_str()),
            &configuration_evidence,
        )?,
    ])
}

fn provider_profile_bindings(
    manifest: &ProviderManifest,
    model_id: &str,
) -> Result<Vec<ExecutionProfileBinding>, RuntimeError> {
    let provider_evidence = serde_json::to_string(manifest).map_err(|_| {
        RuntimeError::InvalidIntent("provider manifest could not be encoded for the profile")
    })?;
    Ok(vec![
        profile_binding(
            ExecutionProfileBindingKind::Provider,
            manifest.id().as_str(),
            None,
            &provider_evidence,
        )?,
        profile_binding(ExecutionProfileBindingKind::Model, model_id, None, model_id)?,
        profile_binding(
            ExecutionProfileBindingKind::Gene,
            "provider.invoke",
            Some(env!("CARGO_PKG_VERSION")),
            "provider.invoke",
        )?,
    ])
}

fn gene_profile_binding(manifest: &GeneManifest) -> Result<ExecutionProfileBinding, RuntimeError> {
    let mut capabilities = manifest
        .capabilities()
        .iter()
        .map(|capability| capability.as_str())
        .collect::<Vec<_>>();
    capabilities.sort_unstable();
    let evidence = format!(
        "gene\0{}\0{}\0{}\0{}",
        manifest.id(),
        manifest.version(),
        manifest.kind().as_str(),
        capabilities.join("\0")
    );
    profile_binding(
        ExecutionProfileBindingKind::Gene,
        manifest.id().as_str(),
        Some(manifest.version()),
        &evidence,
    )
}

fn executor_for_gene(manifest: &GeneManifest) -> &'static str {
    if manifest.capabilities().contains(&Capability::WasmExecute) {
        "wasm"
    } else if manifest
        .capabilities()
        .contains(&Capability::ProcessExecute)
    {
        "process"
    } else if manifest
        .capabilities()
        .contains(&Capability::ProviderInvoke)
    {
        "provider"
    } else if manifest.capabilities().contains(&Capability::McpInvoke) {
        "mcp_stdio"
    } else {
        "filesystem"
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
        RuntimeError::Wasm(_) => "wasm_failed",
        RuntimeError::ExecutionProfile(_) => "execution_profile_failed",
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
        "audit" => GeneId::new("daedalus.audit").expect("built-in Gene ID is valid"),
        "deep-review" => GeneId::new("argus.review").expect("built-in Gene ID is valid"),
        "debt" => GeneId::new("ariadne.debt").expect("built-in Gene ID is valid"),
        "measure" => GeneId::new("hephaestus.measure").expect("built-in Gene ID is valid"),
        "guide" => GeneId::new("athena.guide").expect("built-in Gene ID is valid"),
        "evidence-inventory" => {
            GeneId::new("evidence.inventory").expect("built-in Gene ID is valid")
        }
        "evidence-search" => GeneId::new("evidence.search").expect("built-in Gene ID is valid"),
        "source-read" => GeneId::new("source.read").expect("built-in Gene ID is valid"),
        "source-compare" => GeneId::new("source.compare").expect("built-in Gene ID is valid"),
        "citation-inventory" => {
            GeneId::new("citation.inventory").expect("built-in Gene ID is valid")
        }
        "research-guide" => GeneId::new("research.guide").expect("built-in Gene ID is valid"),
        "design-inventory" => GeneId::new("design.inventory").expect("built-in Gene ID is valid"),
        "design-tokens" => GeneId::new("design.tokens").expect("built-in Gene ID is valid"),
        "design-inspect" => GeneId::new("design.inspect").expect("built-in Gene ID is valid"),
        "design-compare" => GeneId::new("design.compare").expect("built-in Gene ID is valid"),
        "accessibility-evidence" => {
            GeneId::new("accessibility.evidence").expect("built-in Gene ID is valid")
        }
        "design-guide" => GeneId::new("design.guide").expect("built-in Gene ID is valid"),
        _ => GeneId::new("unknown.gene").expect("built-in fallback Gene ID is valid"),
    }
}

fn coding_input(
    intent: &TaskIntent,
    gene_id: &GeneId,
    session: &Session,
    execution_id: &ExecutionId,
    execution_profile: ExecutionProfile,
) -> Result<(GeneInput, Option<Vec<u8>>), RuntimeError> {
    let context = PlanningContext::new(
        execution_id.clone(),
        session.id().clone(),
        session.principal_id().clone(),
        session.workspace_id().clone(),
        execution_profile,
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
        "daedalus.audit" if action == "audit" && remainder.is_empty() => {
            CodingRequest::audit(context)
        }
        "argus.review" if action == "deep-review" => {
            CodingRequest::argus_review(context, remainder)
        }
        "ariadne.debt" if action == "debt" && remainder.is_empty() => CodingRequest::debt(context),
        "hephaestus.measure" if action == "measure" && remainder.is_empty() => {
            CodingRequest::measure(context)
        }
        "athena.guide" if action == "guide" && remainder.is_empty() => {
            CodingRequest::guide(context)
        }
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

fn research_input(
    intent: &TaskIntent,
    gene_id: &GeneId,
    session: &Session,
    execution_id: &ExecutionId,
    execution_profile: ExecutionProfile,
) -> Result<(GeneInput, Option<Vec<u8>>), RuntimeError> {
    let context = PlanningContext::new(
        execution_id.clone(),
        session.id().clone(),
        session.principal_id().clone(),
        session.workspace_id().clone(),
        execution_profile,
    );
    let summary = intent.summary();
    let (action, remainder) = summary.split_once(':').unwrap_or((summary, ""));
    let action = action.to_ascii_lowercase();
    let request = match gene_id.as_str() {
        "evidence.inventory" if action == "evidence-inventory" && remainder.is_empty() => {
            ResearchRequest::inventory(context)
        }
        "evidence.search" if action == "evidence-search" => {
            ResearchRequest::search(context, remainder)
        }
        "source.read" if action == "source-read" => ResearchRequest::read(context, remainder),
        "source.compare" if action == "source-compare" => {
            let (left, right) = remainder
                .split_once('|')
                .ok_or(RuntimeError::InvalidIntent(
                    "source comparison requires two paths",
                ))?;
            ResearchRequest::compare(context, left, right)
        }
        "citation.inventory" if action == "citation-inventory" && remainder.is_empty() => {
            ResearchRequest::citation_inventory(context)
        }
        "research.guide" if action == "research-guide" && remainder.is_empty() => {
            ResearchRequest::guide(context)
        }
        _ => {
            return Err(RuntimeError::InvalidIntent(
                "intent does not match the selected Gene",
            ));
        }
    };
    let input = request.into_gene_input().map_err(RuntimeError::Planning)?;
    Ok((input, None))
}

fn design_input(
    intent: &TaskIntent,
    gene_id: &GeneId,
    session: &Session,
    execution_id: &ExecutionId,
    execution_profile: ExecutionProfile,
) -> Result<(GeneInput, Option<Vec<u8>>), RuntimeError> {
    let context = PlanningContext::new(
        execution_id.clone(),
        session.id().clone(),
        session.principal_id().clone(),
        session.workspace_id().clone(),
        execution_profile,
    );
    let summary = intent.summary();
    let (action, remainder) = summary.split_once(':').unwrap_or((summary, ""));
    let action = action.to_ascii_lowercase();
    let request = match gene_id.as_str() {
        "design.inventory" if action == "design-inventory" && remainder.is_empty() => {
            DesignRequest::inventory(context)
        }
        "design.tokens" if action == "design-tokens" && remainder.is_empty() => {
            DesignRequest::tokens(context)
        }
        "design.inspect" if action == "design-inspect" => {
            DesignRequest::inspect(context, remainder)
        }
        "design.compare" if action == "design-compare" => {
            let (left, right) = remainder
                .split_once('|')
                .ok_or(RuntimeError::InvalidIntent(
                    "design comparison requires two paths",
                ))?;
            DesignRequest::compare(context, left, right)
        }
        "accessibility.evidence" if action == "accessibility-evidence" && remainder.is_empty() => {
            DesignRequest::accessibility_evidence(context)
        }
        "design.guide" if action == "design-guide" && remainder.is_empty() => {
            DesignRequest::guide(context)
        }
        _ => {
            return Err(RuntimeError::InvalidIntent(
                "intent does not match the selected Gene",
            ));
        }
    };
    let input = request.into_gene_input().map_err(RuntimeError::Planning)?;
    Ok((input, None))
}

fn wasm_input(
    intent: &TaskIntent,
    session: &Session,
    execution_id: &ExecutionId,
    execution_profile: ExecutionProfile,
) -> Result<(GeneInput, Option<Vec<u8>>), RuntimeError> {
    let request = WasmGeneRequest::new(
        execution_id.clone(),
        session.id().clone(),
        session.principal_id().clone(),
        execution_profile,
        intent.summary(),
    )
    .map_err(RuntimeError::Planning)?;
    let payload = request.payload().to_vec();
    let input = request.into_gene_input().map_err(RuntimeError::Planning)?;
    Ok((input, Some(payload)))
}

fn append_labeled_output(summary: &mut RunSummary, label: &str, bytes: &[u8]) {
    let output = summary.output.get_or_insert_with(Vec::new);
    if !output.is_empty() {
        output.push(b'\n');
    }
    output.extend_from_slice(label.as_bytes());
    output.extend_from_slice(b":\n");
    output.extend_from_slice(bytes);
}

struct ApprovalExecution<'a> {
    store: &'a ApprovalStore,
    id: &'a str,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ApprovalRequest, HookPoint, HookSelector, LifecycleHook, LifecycleHooks, WasmGene,
    };
    use pandora_harnesses::HarnessCatalog;
    use pandora_provider::{
        ChatMessage, FailoverProvider, ModelResponse, ProviderManifest, TokenUsage,
    };
    use pandora_types::{
        Capability, GeneId, HarnessId, Operation, PackageCompatibility, PackageDependency,
        PackageKind, PackageManifest, PolicyContext, PrincipalId, Session, SessionId, TaskIntent,
        TenantId, Timestamp, TrustEvidence, WorkspaceId, hash_artifact,
    };
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

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
    fn package_wasm_gene_requires_one_exact_approval_before_execution() {
        let fixture = Fixture::new();
        let artifact = wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (func (export "pandora_alloc") (param i32) (result i32) i32.const 0)
                (func (export "pandora_run") (param i32 i32) (result i64)
                    local.get 0
                    i64.extend_i32_u
                    i64.const 32
                    i64.shl
                    local.get 1
                    i64.extend_i32_u
                    i64.or))"#,
        )
        .unwrap();
        let gene_package = PackageManifest::new(
            "example/echo",
            "1.0.0",
            PackageKind::Gene,
            "publisher",
            hash_artifact(&artifact),
            Vec::new(),
            PackageCompatibility::new("pandora>=2.0.0").unwrap(),
            "Apache-2.0",
            TrustEvidence::unsigned(),
        )
        .unwrap();
        let domain_package = PackageManifest::new(
            "example/wasm-domain",
            "1.0.0",
            PackageKind::DomainHarness,
            "publisher",
            hash_artifact(b"domain profile"),
            vec![PackageDependency::new("example/echo", "1.0.0", false).unwrap()],
            PackageCompatibility::new("pandora>=2.0.0").unwrap(),
            "Apache-2.0",
            TrustEvidence::unsigned(),
        )
        .unwrap();
        let gene = WasmGene::from_package(&gene_package).unwrap();
        let harnesses = HarnessCatalog::builtins()
            .with_declarative_domain_genes(&domain_package, vec![Box::new(gene)])
            .unwrap();
        let mut wasm = WasmExecutor::new();
        wasm.register(&gene_package, &artifact).unwrap();
        let policy = PolicyContext::new(1, [Capability::WasmExecute], [Operation::Execute]);
        let controller =
            ExecutionController::with_policy_and_harnesses(fixture.root.clone(), policy, harnesses)
                .with_wasm_executor(wasm);
        let intent = TaskIntent::new(r#"{"value":42}"#)
            .unwrap()
            .with_harness(HarnessId::new("example/wasm-domain").unwrap())
            .with_gene(GeneId::new("example/echo").unwrap());
        let session = fixture.session();
        let now = Timestamp::from_unix_seconds(10);

        let pending = controller
            .run_at(intent.clone(), session.clone(), now)
            .unwrap();
        assert!(matches!(
            pending.status(),
            RunStatus::ApprovalRequired { .. }
        ));
        assert!(pending.receipts().is_empty());
        let request_digest = pending
            .events()
            .iter()
            .find_map(|event| match event.payload() {
                EventPayload::Effect { request_digest, .. } => Some(request_digest.clone()),
                _ => None,
            })
            .unwrap();
        let approval_path = fixture.path.join("approvals.sqlite3");
        let approvals = ApprovalStore::open(&approval_path).unwrap();
        approvals
            .create(
                ApprovalRequest::new(
                    "approval-wasm",
                    session.id().clone(),
                    pending.execution_id().clone(),
                    session.principal_id().clone(),
                    GeneId::new("example/echo").unwrap(),
                    request_digest,
                    "execute example/echo@1.0.0",
                    1,
                    Timestamp::from_unix_seconds(100),
                )
                .unwrap(),
            )
            .unwrap();
        approvals
            .resolve(
                "approval-wasm",
                session.principal_id(),
                &PrincipalId::new("approver-1").unwrap(),
                true,
                now,
            )
            .unwrap();

        let completed = controller
            .run_with_approval(intent, session, &approvals, "approval-wasm", now)
            .unwrap();

        assert_eq!(completed.status(), &RunStatus::Completed);
        assert_eq!(completed.output(), Some(br#"{"value":42}"#.as_slice()));
        assert_eq!(completed.receipts().len(), 1);
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
    fn research_source_comparison_is_read_only_and_receipted() {
        let fixture = Fixture::new();
        std::fs::write(fixture.path.join("first.txt"), b"first evidence\n").unwrap();
        std::fs::write(fixture.path.join("second.txt"), b"second evidence\n").unwrap();
        let controller = ExecutionController::new(fixture.root.clone());
        let intent = TaskIntent::new("source-compare:first.txt|second.txt")
            .unwrap()
            .with_harness(HarnessId::new("research-domain").unwrap())
            .with_gene(GeneId::new("source.compare").unwrap());

        let summary = controller
            .run_at(intent, fixture.session(), Timestamp::from_unix_seconds(10))
            .unwrap();

        assert_eq!(summary.status(), &RunStatus::Completed);
        assert_eq!(summary.selected_harness().as_str(), "research-domain");
        assert_eq!(summary.selected_gene().as_str(), "source.compare");
        assert_eq!(summary.receipts().len(), 2);
        let output = std::str::from_utf8(summary.output().unwrap()).unwrap();
        assert!(output.contains("first.txt:\nfirst evidence"));
        assert!(output.contains("second.txt:\nsecond evidence"));
    }

    #[test]
    fn research_citation_inventory_uses_fixed_read_only_markers() {
        let fixture = Fixture::new();
        std::fs::write(
            fixture.path.join("sources.md"),
            b"https://example.test\ndoi:10.1000/example\n",
        )
        .unwrap();
        let controller = ExecutionController::new(fixture.root.clone());
        let intent = TaskIntent::new("citation-inventory")
            .unwrap()
            .with_harness(HarnessId::new("research-domain").unwrap())
            .with_gene(GeneId::new("citation.inventory").unwrap());

        let summary = controller
            .run_at(intent, fixture.session(), Timestamp::from_unix_seconds(10))
            .unwrap();

        assert_eq!(summary.status(), &RunStatus::Completed);
        assert_eq!(summary.receipts().len(), 3);
        let output = std::str::from_utf8(summary.output().unwrap()).unwrap();
        assert!(output.contains("https://:\nsources.md"));
        assert!(output.contains("doi::\nsources.md"));
    }

    #[test]
    fn design_token_evidence_uses_fixed_read_only_markers() {
        let fixture = Fixture::new();
        std::fs::write(
            fixture.path.join("theme.css"),
            b":root { --color-brand: #123456; }\n.button { color: var(--color-brand); }\n",
        )
        .unwrap();
        let controller = ExecutionController::new(fixture.root.clone());
        let intent = TaskIntent::new("design-tokens")
            .unwrap()
            .with_harness(HarnessId::new("design-domain").unwrap())
            .with_gene(GeneId::new("design.tokens").unwrap());

        let summary = controller
            .run_at(intent, fixture.session(), Timestamp::from_unix_seconds(10))
            .unwrap();

        assert_eq!(summary.status(), &RunStatus::Completed);
        assert_eq!(summary.selected_harness().as_str(), "design-domain");
        assert_eq!(summary.receipts().len(), 4);
        let output = std::str::from_utf8(summary.output().unwrap()).unwrap();
        assert!(output.contains(":root:\ntheme.css"));
        assert!(output.contains("var(:\ntheme.css"));
        assert!(output.contains("--color:\ntheme.css"));
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
    fn lifecycle_hooks_run_in_order_and_stop_before_authorization() {
        let fixture = Fixture::new();
        let hooks = LifecycleHooks::new()
            .with_hook(LifecycleHook::deny(
                "process-only",
                HookPoint::BeforeAuthorization,
                HookSelector::Capability(Capability::ProcessExecute),
                "processes_disabled",
            ))
            .with_hook(LifecycleHook::deny(
                "maintenance",
                HookPoint::BeforeAuthorization,
                HookSelector::Any,
                "maintenance_window",
            ))
            .with_hook(LifecycleHook::deny(
                "later-rule",
                HookPoint::BeforeAuthorization,
                HookSelector::Any,
                "later_rule_should_not_win",
            ));
        let controller = ExecutionController::with_hooks(fixture.root.clone(), hooks);

        let summary = controller
            .run_at(
                TaskIntent::new("read:README.md").unwrap(),
                fixture.session(),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();

        assert_eq!(
            summary.status(),
            &RunStatus::Denied {
                reason: "lifecycle hook maintenance denied request: maintenance_window".to_owned()
            }
        );
        assert!(summary.receipts().is_empty());
        assert!(
            summary
                .events()
                .iter()
                .any(|event| event.event_type() == EventType::PolicyDenied)
        );
        assert!(
            summary
                .events()
                .iter()
                .all(|event| event.event_type() != EventType::EffectCompleted)
        );
    }

    #[test]
    fn lifecycle_hook_denies_provider_egress_before_the_provider_is_called() {
        let fixture = Fixture::new();
        let provider = PanickingProvider::new();
        let hooks = LifecycleHooks::new().with_hook(LifecycleHook::deny(
            "offline-mode",
            HookPoint::BeforeAuthorization,
            HookSelector::Capability(Capability::ProviderInvoke),
            "provider_egress_disabled",
        ));
        let controller = ExecutionController::with_hooks(fixture.root.clone(), hooks);
        let request = ModelRequest::new(
            provider.manifest().id().clone(),
            provider.manifest().default_model().clone(),
            vec![ChatMessage::user("hello").unwrap()],
        )
        .unwrap();

        let result = controller.invoke_provider(
            &provider,
            request,
            &fixture.session(),
            Timestamp::from_unix_seconds(10),
        );

        assert!(matches!(
            result,
            Err(RuntimeError::Denied(reason))
                if reason
                    == "lifecycle hook offline-mode denied request: provider_egress_disabled"
        ));
    }

    #[test]
    fn provider_profile_bindings_cover_endpoint_and_builtin_gene() {
        let first = ProviderManifest::new(
            "provider-a",
            "Provider A",
            "https://first.example.test/v1",
            "model-a",
            "PANDORA_PROVIDER_A_KEY",
        )
        .unwrap();
        let second = ProviderManifest::new(
            "provider-a",
            "Provider A",
            "https://second.example.test/v1",
            "model-a",
            "PANDORA_PROVIDER_A_KEY",
        )
        .unwrap();

        let first_bindings = provider_profile_bindings(&first, "model-a").unwrap();
        let second_bindings = provider_profile_bindings(&second, "model-a").unwrap();
        let first_provider = first_bindings
            .iter()
            .find(|binding| binding.kind() == ExecutionProfileBindingKind::Provider)
            .unwrap();
        let second_provider = second_bindings
            .iter()
            .find(|binding| binding.kind() == ExecutionProfileBindingKind::Provider)
            .unwrap();

        assert_ne!(first_provider.digest(), second_provider.digest());
        assert!(first_bindings.iter().any(|binding| {
            binding.kind() == ExecutionProfileBindingKind::Gene && binding.id() == "provider.invoke"
        }));
        let serialized = serde_json::to_string(&first_bindings).unwrap();
        assert!(!serialized.contains("first.example.test"));
        assert!(!serialized.contains("PANDORA_PROVIDER_A_KEY"));
    }

    #[test]
    fn mcp_invocation_profile_bindings_cover_process_and_configuration() {
        let first_supervisor = McpCatalogSupervisor::new();
        let first_reservation = first_supervisor.reserve("local", "config-a").unwrap();
        let first_tool = crate::mcp_catalog::McpCatalogTool::new(
            "mcp.local.echo",
            "echo",
            &serde_json::json!({"type": "object"}),
        )
        .unwrap();
        let first_revision = first_reservation
            .activate(McpWireEra::Modern, 41, vec![first_tool])
            .unwrap();
        let second_supervisor = McpCatalogSupervisor::new();
        let second_reservation = second_supervisor.reserve("local", "config-b").unwrap();
        let second_tool = crate::mcp_catalog::McpCatalogTool::new(
            "mcp.local.echo",
            "echo",
            &serde_json::json!({"type": "object"}),
        )
        .unwrap();
        let second_revision = second_reservation
            .activate(McpWireEra::Modern, 42, vec![second_tool])
            .unwrap();

        let first_bindings = mcp_invocation_profile_bindings(
            &first_revision,
            "mcp.local.echo",
            first_revision.tool("mcp.local.echo").unwrap(),
        )
        .unwrap();
        let second_bindings = mcp_invocation_profile_bindings(
            &second_revision,
            "mcp.local.echo",
            second_revision.tool("mcp.local.echo").unwrap(),
        )
        .unwrap();
        let first_configuration = first_bindings
            .iter()
            .find(|binding| binding.kind() == ExecutionProfileBindingKind::Configuration)
            .unwrap();
        let second_configuration = second_bindings
            .iter()
            .find(|binding| binding.kind() == ExecutionProfileBindingKind::Configuration)
            .unwrap();

        assert_ne!(first_configuration.digest(), second_configuration.digest());
        assert!(first_bindings.iter().any(|binding| {
            binding.kind() == ExecutionProfileBindingKind::ToolCatalog && binding.id() == "local"
        }));
    }

    #[test]
    fn worktree_profile_bindings_cover_the_managed_root_without_exposing_it() {
        let first = worktree_profile_bindings(
            "coordination.worktree.create",
            "git_worktree_create",
            r#"{"operation":"git_worktree_create"}"#,
            r"C:\work\pandora\managed-a",
        )
        .unwrap();
        let second = worktree_profile_bindings(
            "coordination.worktree.create",
            "git_worktree_create",
            r#"{"operation":"git_worktree_create"}"#,
            r"C:\work\pandora\managed-b",
        )
        .unwrap();
        let first_root = first
            .iter()
            .find(|binding| binding.id() == "managed_worktree_root")
            .unwrap();
        let second_root = second
            .iter()
            .find(|binding| binding.id() == "managed_worktree_root")
            .unwrap();

        assert_ne!(first_root.digest(), second_root.digest());
        let serialized = serde_json::to_string(&first).unwrap();
        assert!(!serialized.contains(r"C:\work\pandora\managed-a"));
    }

    #[test]
    fn provider_fallback_requires_a_fresh_permit_and_receipt() {
        let fixture = Fixture::new();
        let primary_requests = Arc::new(Mutex::new(Vec::new()));
        let fallback_requests = Arc::new(Mutex::new(Vec::new()));
        let primary = StubProvider::new(
            "provider-a",
            "model-a",
            Err(ProviderError::Transport),
            Arc::clone(&primary_requests),
        );
        let fallback = StubProvider::new(
            "provider-b",
            "model-b",
            Ok(ModelResponse::new(
                "fallback-ready",
                Vec::new(),
                TokenUsage::new(2, 1),
            )),
            Arc::clone(&fallback_requests),
        );
        let provider = FailoverProvider::new(Box::new(primary), Box::new(fallback));
        let request = ModelRequest::new(
            provider.manifest().id().clone(),
            provider.manifest().default_model().clone(),
            vec![ChatMessage::user("hello").unwrap()],
        )
        .unwrap();
        let expected_messages = request.messages().to_vec();
        let controller = ExecutionController::new(fixture.root.clone());

        let result = controller
            .invoke_provider(
                &provider,
                request,
                &fixture.session(),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();

        assert_eq!(result.result().unwrap().text(), "fallback-ready");
        assert_eq!(primary_requests.lock().unwrap().len(), 1);
        let fallback_requests = fallback_requests.lock().unwrap();
        assert_eq!(fallback_requests.len(), 1);
        assert_eq!(fallback_requests[0].provider_id().as_str(), "provider-b");
        assert_eq!(fallback_requests[0].model_id().as_str(), "model-b");
        assert_eq!(fallback_requests[0].messages(), expected_messages);
        assert_eq!(result.receipts().len(), 2);
        assert!(matches!(
            result.receipts()[0].outcome(),
            pandora_types::EffectOutcome::Failed { code } if code == "transport"
        ));
        assert!(matches!(
            result.receipts()[1].outcome(),
            pandora_types::EffectOutcome::Succeeded
        ));
        assert_ne!(
            result.receipts()[0].permit_id(),
            result.receipts()[1].permit_id()
        );
        assert_ne!(
            result.receipts()[0].request_digest(),
            result.receipts()[1].request_digest()
        );
        assert_eq!(result.receipt(), &result.receipts()[1]);
    }

    #[test]
    fn non_retryable_provider_error_does_not_invoke_fallback() {
        let fixture = Fixture::new();
        let primary_requests = Arc::new(Mutex::new(Vec::new()));
        let fallback_requests = Arc::new(Mutex::new(Vec::new()));
        let primary = StubProvider::new(
            "provider-a",
            "model-a",
            Err(ProviderError::InvalidRequest("invalid request".to_owned())),
            Arc::clone(&primary_requests),
        );
        let fallback = StubProvider::new(
            "provider-b",
            "model-b",
            Ok(ModelResponse::new(
                "must-not-run",
                Vec::new(),
                TokenUsage::default(),
            )),
            Arc::clone(&fallback_requests),
        );
        let provider = FailoverProvider::new(Box::new(primary), Box::new(fallback));
        let request = ModelRequest::new(
            provider.manifest().id().clone(),
            provider.manifest().default_model().clone(),
            vec![ChatMessage::user("hello").unwrap()],
        )
        .unwrap();
        let controller = ExecutionController::new(fixture.root.clone());

        let result = controller
            .invoke_provider(
                &provider,
                request,
                &fixture.session(),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();

        assert!(matches!(
            result.result(),
            Err(ProviderError::InvalidRequest(message)) if message == "invalid request"
        ));
        assert_eq!(primary_requests.lock().unwrap().len(), 1);
        assert!(fallback_requests.lock().unwrap().is_empty());
        assert_eq!(result.receipts().len(), 1);
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
        let harness_id = HarnessId::new("unavailable-domain").unwrap();
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

    #[test]
    fn daedalus_audit_returns_a_bounded_workspace_inventory() {
        let fixture = Fixture::new();
        std::fs::create_dir(fixture.path.join("src")).unwrap();
        std::fs::write(fixture.path.join("src/lib.rs"), b"pub fn example() {}\n").unwrap();
        let controller = ExecutionController::new(fixture.root.clone());
        let intent = TaskIntent::new("audit")
            .unwrap()
            .with_gene(GeneId::new("daedalus.audit").unwrap());

        let summary = controller
            .run_at(intent, fixture.session(), Timestamp::from_unix_seconds(10))
            .unwrap();

        assert_eq!(summary.status(), &RunStatus::Completed);
        assert_eq!(summary.receipts().len(), 1);
        assert_eq!(summary.output().unwrap(), b"README.md\nsrc/lib.rs");
    }

    #[test]
    fn ariadne_debt_aggregates_only_evidence_backed_matches() {
        let fixture = Fixture::new();
        std::fs::create_dir(fixture.path.join("src")).unwrap();
        std::fs::write(
            fixture.path.join("src/lib.rs"),
            b"// TODO: replace fixture\n",
        )
        .unwrap();
        std::fs::write(fixture.path.join("src/clean.rs"), b"pub fn clean() {}\n").unwrap();
        let controller = ExecutionController::new(fixture.root.clone());
        let intent = TaskIntent::new("debt")
            .unwrap()
            .with_gene(GeneId::new("ariadne.debt").unwrap());

        let summary = controller
            .run_at(intent, fixture.session(), Timestamp::from_unix_seconds(10))
            .unwrap();

        assert_eq!(summary.status(), &RunStatus::Completed);
        assert_eq!(summary.receipts().len(), 4);
        let output = String::from_utf8(summary.output().unwrap().to_vec()).unwrap();
        assert!(output.contains("TODO:\nsrc/lib.rs"));
        assert!(!output.contains("src/clean.rs"));
    }

    #[test]
    fn athena_guide_completes_without_requesting_an_effect() {
        let fixture = Fixture::new();
        let controller = ExecutionController::new(fixture.root.clone());
        let intent = TaskIntent::new("guide")
            .unwrap()
            .with_gene(GeneId::new("athena.guide").unwrap());

        let summary = controller
            .run_at(intent, fixture.session(), Timestamp::from_unix_seconds(10))
            .unwrap();

        assert_eq!(summary.status(), &RunStatus::Completed);
        assert!(summary.receipts().is_empty());
        assert!(String::from_utf8_lossy(summary.output().unwrap()).contains("Daedalus"));
    }

    struct Fixture {
        path: PathBuf,
        root: WorkspaceRoot,
    }

    struct PanickingProvider {
        manifest: ProviderManifest,
    }

    struct StubProvider {
        manifest: ProviderManifest,
        result: Mutex<Option<Result<ModelResponse, ProviderError>>>,
        requests: Arc<Mutex<Vec<ModelRequest>>>,
    }

    impl StubProvider {
        fn new(
            id: &str,
            model: &str,
            result: Result<ModelResponse, ProviderError>,
            requests: Arc<Mutex<Vec<ModelRequest>>>,
        ) -> Self {
            Self {
                manifest: ProviderManifest::new(
                    id,
                    id,
                    format!("https://{id}.example.test/v1"),
                    model,
                    format!("PANDORA_{}_KEY", id.replace('-', "_").to_uppercase()),
                )
                .unwrap(),
                result: Mutex::new(Some(result)),
                requests,
            }
        }
    }

    impl Provider for StubProvider {
        fn manifest(&self) -> &ProviderManifest {
            &self.manifest
        }

        fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
            self.requests.lock().unwrap().push(request);
            self.result
                .lock()
                .unwrap()
                .take()
                .expect("stub provider called more than once")
        }
    }

    impl PanickingProvider {
        fn new() -> Self {
            Self {
                manifest: ProviderManifest::new(
                    "test-provider",
                    "Test provider",
                    "http://127.0.0.1:1/v1",
                    "test-model",
                    "PANDORA_TEST_PROVIDER_KEY",
                )
                .unwrap(),
            }
        }
    }

    impl Provider for PanickingProvider {
        fn manifest(&self) -> &ProviderManifest {
            &self.manifest
        }

        fn complete(&self, _request: ModelRequest) -> Result<ModelResponse, ProviderError> {
            panic!("provider must not be called after hook denial")
        }
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
