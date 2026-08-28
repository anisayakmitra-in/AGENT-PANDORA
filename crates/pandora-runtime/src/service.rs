use crate::agent_loop::{
    AgentApprovalContext, AgentLoop, AgentLoopError, AgentRunRequest, AgentRunSummary,
};
use crate::approvals::{ApprovalError, ApprovalRequest, ApprovalStore, PendingApproval};
use crate::artifact_catalog::{ArtifactCatalog, ArtifactCatalogError};
use crate::evaluation_engine::EvaluationEngine;
use crate::evolution::{EvolutionEngine, EvolutionError, EvolutionRecord};
use crate::execution_controller::{ExecutionController, RunStatus, RunSummary, RuntimeError};
use crate::fleet::{FleetBudget, FleetEngine, FleetError, FleetNode};
use crate::sessions::{SessionError, SessionStore};
use crate::tool_engine::ToolEngine;
use pandora_provider::Provider;
use pandora_types::{
    EvaluationContractError, EvaluationReceipt, EvaluationRequest, EventPayload, EventType,
    IdError, MemoryTier, PrincipalId, ProposalId, ServiceAgentResumeRequest,
    ServiceAgentRunRequest, ServiceAgentRunResult, ServiceApprovalSummary,
    ServiceArtifactActivation, ServiceContractError, ServiceEngineSummary, ServiceEventPage,
    ServiceEvolutionApproval, ServiceEvolutionCanary, ServiceEvolutionEvaluation,
    ServiceEvolutionSummary, ServiceHarnessSummary, ServiceHealth, ServiceMemoryPage,
    ServiceMemoryRecord, ServiceProviderSummary, ServiceRequest, ServiceResponse,
    ServiceRunRequest, ServiceRunResult, ServiceRunResumeRequest, ServiceSessionDetail,
    ServiceSessionSummary, ServiceToolSummary, Session, SessionId, TaskIntent, TenantId, Timestamp,
    WorkspaceId,
};
use std::fmt;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeServiceScope {
    principal_id: PrincipalId,
    tenant_id: TenantId,
    workspace_id: WorkspaceId,
}

impl RuntimeServiceScope {
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

#[derive(Debug)]
pub enum RuntimeServiceError {
    UnsupportedProtocol { actual: u16 },
    Contract(ServiceContractError),
    Evaluation(EvaluationContractError),
    Identifier(IdError),
    Runtime(RuntimeError),
    Agent(AgentLoopError),
    AgentUnavailable,
    Evolution(EvolutionError),
    EvolutionUnavailable,
    ArtifactCatalog(ArtifactCatalogError),
    ArtifactCatalogUnavailable,
    Approval(ApprovalError),
    Session(SessionError),
    Fleet(FleetError),
}

impl RuntimeServiceError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedProtocol { .. } => "unsupported_protocol_version",
            Self::Contract(_) => "invalid_service_request",
            Self::Evaluation(_) => "invalid_evaluation",
            Self::Identifier(_) => "invalid_session_identifier",
            Self::Runtime(RuntimeError::Approval(ApprovalError::NotFound)) => "approval_not_found",
            Self::Runtime(RuntimeError::Approval(ApprovalError::Expired)) => "approval_expired",
            Self::Runtime(RuntimeError::Approval(ApprovalError::Terminal)) => "approval_terminal",
            Self::Runtime(RuntimeError::Approval(
                ApprovalError::ScopeMismatch | ApprovalError::DigestMismatch,
            )) => "approval_scope_mismatch",
            Self::Runtime(_) => "runtime_execution_failed",
            Self::Agent(_) => "agent_execution_failed",
            Self::AgentUnavailable => "agent_unavailable",
            Self::Evolution(EvolutionError::NotFound) => "evolution_proposal_not_found",
            Self::Evolution(_) => "evolution_store_failed",
            Self::EvolutionUnavailable => "evolution_unavailable",
            Self::ArtifactCatalog(_) => "artifact_catalog_failed",
            Self::ArtifactCatalogUnavailable => "artifact_catalog_unavailable",
            Self::Approval(ApprovalError::NotFound) => "approval_not_found",
            Self::Approval(ApprovalError::Expired) => "approval_expired",
            Self::Approval(ApprovalError::Terminal) => "approval_terminal",
            Self::Approval(ApprovalError::ScopeMismatch | ApprovalError::DigestMismatch) => {
                "approval_scope_mismatch"
            }
            Self::Approval(_) => "approval_store_failed",
            Self::Session(SessionError::SessionNotFound) => "session_not_found",
            Self::Fleet(_) => "fleet_registry_failed",
            Self::Session(SessionError::ScopeViolation) => "session_scope_violation",
            Self::Session(_) => "session_store_failed",
        }
    }
}

impl fmt::Display for RuntimeServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedProtocol { actual } => {
                write!(
                    formatter,
                    "service protocol version {actual} is unsupported"
                )
            }
            Self::Contract(_) => formatter.write_str("service request is invalid"),
            Self::Evaluation(_) => formatter.write_str("execution evaluation is invalid"),
            Self::Identifier(_) => formatter.write_str("service session identifier is invalid"),
            Self::Runtime(_) => formatter.write_str("governed runtime execution failed"),
            Self::Agent(_) => formatter.write_str("governed agent execution failed"),
            Self::AgentUnavailable => formatter.write_str("agent provider is not configured"),
            Self::Evolution(error) => error.fmt(formatter),
            Self::EvolutionUnavailable => {
                formatter.write_str("evolution records are not configured")
            }
            Self::ArtifactCatalog(error) => error.fmt(formatter),
            Self::ArtifactCatalogUnavailable => {
                formatter.write_str("artifact activation catalog is not configured")
            }
            Self::Approval(error) => error.fmt(formatter),
            Self::Session(error) => error.fmt(formatter),
            Self::Fleet(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for RuntimeServiceError {}

impl From<ServiceContractError> for RuntimeServiceError {
    fn from(error: ServiceContractError) -> Self {
        Self::Contract(error)
    }
}

impl From<EvaluationContractError> for RuntimeServiceError {
    fn from(error: EvaluationContractError) -> Self {
        Self::Evaluation(error)
    }
}

impl From<IdError> for RuntimeServiceError {
    fn from(error: IdError) -> Self {
        Self::Identifier(error)
    }
}

impl From<RuntimeError> for RuntimeServiceError {
    fn from(error: RuntimeError) -> Self {
        Self::Runtime(error)
    }
}

impl From<AgentLoopError> for RuntimeServiceError {
    fn from(error: AgentLoopError) -> Self {
        Self::Agent(error)
    }
}

impl From<EvolutionError> for RuntimeServiceError {
    fn from(error: EvolutionError) -> Self {
        Self::Evolution(error)
    }
}

impl From<ArtifactCatalogError> for RuntimeServiceError {
    fn from(error: ArtifactCatalogError) -> Self {
        Self::ArtifactCatalog(error)
    }
}

impl From<ApprovalError> for RuntimeServiceError {
    fn from(error: ApprovalError) -> Self {
        Self::Approval(error)
    }
}

impl From<SessionError> for RuntimeServiceError {
    fn from(error: SessionError) -> Self {
        Self::Session(error)
    }
}

impl From<FleetError> for RuntimeServiceError {
    fn from(error: FleetError) -> Self {
        Self::Fleet(error)
    }
}

pub struct RuntimeService {
    controller: ExecutionController,
    sessions: SessionStore,
    approvals: ApprovalStore,
    scope: RuntimeServiceScope,
    providers: Vec<ServiceProviderSummary>,
    agent: Option<RuntimeServiceAgent>,
    evolution: Option<Arc<EvolutionEngine>>,
    artifact_catalog: Option<Arc<ArtifactCatalog>>,
    fleet: Option<RuntimeServiceFleet>,
    next_session: AtomicU64,
    next_lease: AtomicU64,
}

struct RuntimeServiceFleet {
    engine: Arc<FleetEngine>,
    node_id: String,
}

struct ActiveServiceLease {
    engine: Arc<FleetEngine>,
    lease_id: String,
}

impl Drop for ActiveServiceLease {
    fn drop(&mut self) {
        let _ = self.engine.release_lease(&self.lease_id);
    }
}

struct RuntimeServiceAgent {
    provider: Arc<dyn Provider>,
    loop_engine: AgentLoop,
    max_tool_calls: u32,
    skill_context: Option<String>,
}

impl RuntimeService {
    pub fn new(
        controller: ExecutionController,
        sessions: SessionStore,
        approvals: ApprovalStore,
        scope: RuntimeServiceScope,
    ) -> Self {
        Self::new_with_providers(controller, sessions, approvals, scope, Vec::new())
    }

    pub fn new_with_providers(
        controller: ExecutionController,
        sessions: SessionStore,
        approvals: ApprovalStore,
        scope: RuntimeServiceScope,
        providers: Vec<ServiceProviderSummary>,
    ) -> Self {
        Self {
            controller,
            sessions,
            approvals,
            scope,
            providers,
            agent: None,
            evolution: None,
            artifact_catalog: None,
            fleet: None,
            next_session: AtomicU64::new(1),
            next_lease: AtomicU64::new(1),
        }
    }

    pub fn with_fleet(
        mut self,
        engine: FleetEngine,
        node_id: impl Into<String>,
    ) -> Result<Self, RuntimeServiceError> {
        let node_id = node_id.into();
        let now = current_timestamp().as_unix_seconds();
        engine.expire_leases(now)?;
        let node = FleetNode::new(
            node_id.clone(),
            env!("CARGO_PKG_VERSION"),
            "service",
            vec!["agent.execute".to_owned(), "runtime.execute".to_owned()],
            now,
        )?;
        match engine.register_node(&node) {
            Ok(_) | Err(FleetError::NodeAlreadyRegistered) => {}
            Err(error) => return Err(error.into()),
        }
        self.fleet = Some(RuntimeServiceFleet {
            engine: Arc::new(engine),
            node_id,
        });
        Ok(self)
    }

    pub fn with_agent(
        mut self,
        provider: Arc<dyn Provider>,
        max_turns: u32,
        max_tool_calls: u32,
        context_cache_path: impl AsRef<Path>,
        skill_context: Option<String>,
    ) -> Result<Self, RuntimeServiceError> {
        let loop_engine =
            AgentLoop::new(max_turns, max_tool_calls)?.with_context_cache(context_cache_path)?;
        self.agent = Some(RuntimeServiceAgent {
            provider,
            loop_engine,
            max_tool_calls,
            skill_context,
        });
        Ok(self)
    }

    pub fn with_evolution(mut self, evolution: Arc<EvolutionEngine>) -> Self {
        self.evolution = Some(evolution);
        self
    }

    pub fn with_artifact_catalog(mut self, catalog: Arc<ArtifactCatalog>) -> Self {
        self.artifact_catalog = Some(catalog);
        self
    }

    pub fn scope(&self) -> &RuntimeServiceScope {
        &self.scope
    }

    pub fn handle(
        &self,
        request: &ServiceRequest,
        now: Timestamp,
    ) -> Result<ServiceResponse, RuntimeServiceError> {
        if !request.is_supported_protocol() {
            return Err(RuntimeServiceError::UnsupportedProtocol {
                actual: request.protocol_version(),
            });
        }
        request.validate()?;

        match request {
            ServiceRequest::Health { .. } => Ok(ServiceResponse::health(ServiceHealth::ready())),
            ServiceRequest::Capabilities { .. } => self.capabilities(),
            ServiceRequest::Providers { .. } => {
                Ok(ServiceResponse::providers(self.providers.clone()))
            }
            ServiceRequest::Engines { .. } => self.engines(),
            ServiceRequest::Tools { .. } => self.tools(),
            ServiceRequest::SessionList { limit, .. } => self.list_sessions(*limit),
            ServiceRequest::SessionInspect { session_id, .. } => self.inspect_session(session_id),
            ServiceRequest::SessionEvents { request, .. } => self.session_events(request),
            ServiceRequest::SessionMemory {
                session_id, limit, ..
            } => self.session_memory(session_id, *limit),
            ServiceRequest::ApprovalList { limit, .. } => self.list_approvals(*limit, now),
            ServiceRequest::ApprovalInspect { approval_id, .. } => {
                self.inspect_approval(approval_id, now)
            }
            ServiceRequest::ApprovalResolve {
                approval_id, allow, ..
            } => self.resolve_approval(approval_id, *allow, now),
            ServiceRequest::EvolutionList { limit, .. } => self.list_evolution(*limit),
            ServiceRequest::EvolutionInspect { proposal_id, .. } => {
                self.inspect_evolution(proposal_id)
            }
            ServiceRequest::EvolutionActivations { limit, .. } => {
                self.list_artifact_activations(*limit)
            }
            ServiceRequest::Run { request, .. } => self.run(request, now),
            ServiceRequest::RunResume { request, .. } => self.resume_run(request, now),
            ServiceRequest::AgentRun { request, .. } => self.run_agent(request, now),
            ServiceRequest::AgentResume { request, .. } => self.resume_agent(request, now),
        }
    }

    fn capabilities(&self) -> Result<ServiceResponse, RuntimeServiceError> {
        let harnesses = self
            .controller
            .harnesses()
            .map(|harness| {
                ServiceHarnessSummary::new(
                    harness.manifest().id().clone(),
                    harness.manifest().version(),
                    harness.manifest().name(),
                    harness.manifest().kind().as_str(),
                    u32::try_from(harness.genes().len()).unwrap_or(u32::MAX),
                    harness.is_runnable(),
                )
                .with_gene_ids(
                    harness
                        .genes()
                        .iter()
                        .map(|gene| gene.manifest().id().clone())
                        .collect(),
                )
            })
            .collect();
        Ok(ServiceResponse::capabilities(harnesses))
    }

    fn engines(&self) -> Result<ServiceResponse, RuntimeServiceError> {
        Ok(ServiceResponse::engines(vec![
            ServiceEngineSummary::new(
                "execution-controller",
                "ExecutionController",
                "Fixed runtime pipeline",
                "Runtime authority",
            ),
            ServiceEngineSummary::new(
                "reference-monitor",
                "ReferenceMonitor",
                "Authorization",
                "Sole permit issuer",
            ),
            ServiceEngineSummary::new(
                "tool-engine",
                "ToolEngine",
                "Tool contracts",
                "Request boundary",
            ),
            ServiceEngineSummary::new(
                "context-engine",
                "ContextEngine",
                "Context assembly",
                "Scoped evidence",
            ),
            ServiceEngineSummary::new(
                "memory-engine",
                "MemoryEngine",
                "Evidence lifecycle",
                "Scoped persistence",
            ),
            ServiceEngineSummary::new(
                "evaluation-engine",
                "EvaluationEngine",
                "Evaluation evidence",
                "Policy and outcome checks",
            ),
            ServiceEngineSummary::new(
                "evolution-engine",
                "EvolutionEngine",
                "Governed improvement",
                "Proposal only",
            ),
            ServiceEngineSummary::new(
                "mcp-adapter",
                "MCP adapter",
                "Local tool bridge",
                "Configured stdio boundary",
            ),
            ServiceEngineSummary::new(
                "adaptive-engine",
                "AdaptiveEngine",
                "Bounded selection",
                "Approved options only",
            ),
            ServiceEngineSummary::new(
                "coding-feedback-loop",
                "CodingFeedbackLoop",
                "Coding verification",
                "Evidence-driven iteration",
            ),
            ServiceEngineSummary::new(
                "efficiency-engine",
                "EfficiencyEngine",
                "Cost and latency evidence",
                "Selection guidance",
            ),
            ServiceEngineSummary::new(
                "graph-intelligence-engine",
                "GraphIntelligenceEngine",
                "Code and knowledge graphs",
                "Provenance-aware evidence",
            ),
            ServiceEngineSummary::new(
                "orchestration-engine",
                "OrchestrationEngine",
                "Role composition",
                "Governed coordination",
            ),
            ServiceEngineSummary::new(
                "self-healing-engine",
                "SelfHealingEngine",
                "Safe recovery",
                "Allowlisted reductions",
            ),
            ServiceEngineSummary::new(
                "skill-engine",
                "SkillEngine",
                "Skill admission",
                "Provenance and activation",
            ),
            ServiceEngineSummary::new(
                "observability-engine",
                "ObservabilityEngine",
                "Trace projection",
                "Canonical runtime events",
            ),
            ServiceEngineSummary::new(
                "fleet-engine",
                "FleetEngine",
                "Worker coordination",
                "Leases and quarantine",
            ),
            ServiceEngineSummary::new(
                "mutation-engine",
                "MutationEngine",
                "Improvement proposals",
                "Research-scoped generation",
            ),
            ServiceEngineSummary::new(
                "replacement-engine",
                "ReplacementEngine",
                "Staged replacement",
                "Canary and rollback",
            ),
            ServiceEngineSummary::new(
                "population-strategy",
                "PopulationStrategy",
                "Research candidate populations",
                "Proposal only",
            ),
        ]))
    }

    fn tools(&self) -> Result<ServiceResponse, RuntimeServiceError> {
        Ok(ServiceResponse::tools(
            ToolEngine::with_builtins()
                .list()
                .into_iter()
                .map(|tool| {
                    ServiceToolSummary::new(
                        tool.id().clone(),
                        tool.version(),
                        tool.name(),
                        tool.capability().as_str(),
                        tool.operation().as_str(),
                    )
                })
                .collect(),
        ))
    }

    fn list_sessions(&self, limit: u16) -> Result<ServiceResponse, RuntimeServiceError> {
        let sessions = self
            .sessions
            .list(
                self.scope.principal_id(),
                self.scope.tenant_id(),
                self.scope.workspace_id(),
            )?
            .into_iter()
            .take(usize::from(limit))
            .map(service_session_summary)
            .collect();
        Ok(ServiceResponse::session_list(sessions))
    }

    fn inspect_session(
        &self,
        session_id: &SessionId,
    ) -> Result<ServiceResponse, RuntimeServiceError> {
        let snapshot = self.sessions.resume(
            session_id,
            self.scope.principal_id(),
            self.scope.tenant_id(),
            self.scope.workspace_id(),
        )?;
        Ok(ServiceResponse::session_inspect(ServiceSessionDetail::new(
            service_session_summary(snapshot.session().clone()),
            u64::try_from(snapshot.events().len()).unwrap_or(u64::MAX),
        )))
    }

    fn session_events(
        &self,
        request: &pandora_types::ServiceEventPageRequest,
    ) -> Result<ServiceResponse, RuntimeServiceError> {
        let page = self.sessions.event_page(
            request.session_id(),
            self.scope.principal_id(),
            self.scope.tenant_id(),
            self.scope.workspace_id(),
            request.after_sequence(),
            request.limit(),
        )?;

        Ok(ServiceResponse::session_events(ServiceEventPage::new(
            request.session_id().clone(),
            page.events().to_vec(),
            page.next_sequence(),
        )))
    }

    fn session_memory(
        &self,
        session_id: &SessionId,
        limit: u16,
    ) -> Result<ServiceResponse, RuntimeServiceError> {
        let mut records = Vec::new();
        for tier in [MemoryTier::L1, MemoryTier::L2] {
            let remaining = limit.saturating_sub(u16::try_from(records.len()).unwrap_or(u16::MAX));
            if remaining == 0 {
                break;
            }
            records.extend(
                self.sessions
                    .recall_memory(
                        session_id,
                        self.scope.principal_id(),
                        self.scope.tenant_id(),
                        self.scope.workspace_id(),
                        "local",
                        tier,
                        remaining,
                    )?
                    .into_iter()
                    .map(|record| {
                        ServiceMemoryRecord::new(
                            record.id().as_str(),
                            record.tier(),
                            record.kind(),
                            record.summary(),
                            record.classification().as_str(),
                            record.created_at().as_unix_seconds(),
                            record.provenance(),
                            record.origin().as_str(),
                            u16::try_from(record.evidence_ids().len()).unwrap_or(u16::MAX),
                        )
                    }),
            );
        }
        Ok(ServiceResponse::session_memory(ServiceMemoryPage::new(
            session_id.clone(),
            records,
        )))
    }

    fn list_approvals(
        &self,
        limit: u16,
        now: Timestamp,
    ) -> Result<ServiceResponse, RuntimeServiceError> {
        let mut approvals = Vec::new();
        let mut available = self.approvals.list(self.scope.principal_id())?;
        available.reverse();
        for approval in available {
            match self.sessions.resume(
                approval.session_id(),
                self.scope.principal_id(),
                self.scope.tenant_id(),
                self.scope.workspace_id(),
            ) {
                Ok(_) => approvals.push(service_approval_summary(&approval, now)?),
                Err(SessionError::SessionNotFound | SessionError::ScopeViolation) => {}
                Err(error) => return Err(error.into()),
            }
            if approvals.len() >= usize::from(limit) {
                break;
            }
        }
        Ok(ServiceResponse::approval_list(approvals))
    }

    fn inspect_approval(
        &self,
        approval_id: &str,
        now: Timestamp,
    ) -> Result<ServiceResponse, RuntimeServiceError> {
        let approval = self.scoped_approval(approval_id)?;
        Ok(ServiceResponse::approval_inspect(service_approval_summary(
            &approval, now,
        )?))
    }

    fn resolve_approval(
        &self,
        approval_id: &str,
        allow: bool,
        now: Timestamp,
    ) -> Result<ServiceResponse, RuntimeServiceError> {
        self.scoped_approval(approval_id)?;
        let approval = self.approvals.resolve(
            approval_id,
            self.scope.principal_id(),
            self.scope.principal_id(),
            allow,
            now,
        )?;
        Ok(ServiceResponse::approval_resolve(service_approval_summary(
            &approval, now,
        )?))
    }

    fn scoped_approval(&self, approval_id: &str) -> Result<PendingApproval, RuntimeServiceError> {
        let approval = self
            .approvals
            .inspect(approval_id, self.scope.principal_id())?;
        self.sessions.resume(
            approval.session_id(),
            self.scope.principal_id(),
            self.scope.tenant_id(),
            self.scope.workspace_id(),
        )?;
        Ok(approval)
    }

    fn list_evolution(&self, limit: u16) -> Result<ServiceResponse, RuntimeServiceError> {
        let engine = self
            .evolution
            .as_ref()
            .ok_or(RuntimeServiceError::EvolutionUnavailable)?;
        let proposals = engine
            .list()?
            .into_iter()
            .take(usize::from(limit))
            .map(service_evolution_summary)
            .collect();
        Ok(ServiceResponse::evolution_list(proposals))
    }

    fn inspect_evolution(
        &self,
        proposal_id: &ProposalId,
    ) -> Result<ServiceResponse, RuntimeServiceError> {
        let engine = self
            .evolution
            .as_ref()
            .ok_or(RuntimeServiceError::EvolutionUnavailable)?;
        Ok(ServiceResponse::evolution_inspect(
            service_evolution_summary(engine.inspect(proposal_id)?),
        ))
    }

    fn list_artifact_activations(
        &self,
        limit: u16,
    ) -> Result<ServiceResponse, RuntimeServiceError> {
        let catalog = self
            .artifact_catalog
            .as_ref()
            .ok_or(RuntimeServiceError::ArtifactCatalogUnavailable)?;
        let activations = catalog
            .list(usize::from(limit))?
            .into_iter()
            .map(|activation| {
                ServiceArtifactActivation::new(
                    activation.proposal_id().clone(),
                    activation.base_artifact().clone(),
                    activation.candidate_artifact().clone(),
                    activation.activated_at(),
                )
            })
            .collect();
        Ok(ServiceResponse::evolution_activations(activations))
    }

    fn run(
        &self,
        request: &ServiceRunRequest,
        now: Timestamp,
    ) -> Result<ServiceResponse, RuntimeServiceError> {
        let session = self.allocate_session(now)?;
        let intent = service_task_intent(request)?;

        let _active_lease = self.acquire_execution_lease(&session, now, 0)?;
        let summary = self.controller.run_at(intent, session.clone(), now)?;
        self.sessions.create(&session)?;
        self.persist_execution(&session, &summary, "local", now)?;

        let approval = if matches!(summary.status(), RunStatus::ApprovalRequired { .. }) {
            Some(self.create_approval(request.task(), &session, &summary, now)?)
        } else {
            None
        };

        Ok(ServiceResponse::run(service_run_result(
            &summary,
            session.id().clone(),
            approval.as_ref(),
            now,
        )?))
    }

    fn resume_run(
        &self,
        request: &ServiceRunResumeRequest,
        now: Timestamp,
    ) -> Result<ServiceResponse, RuntimeServiceError> {
        let approval = self.scoped_approval(request.approval_id())?;
        let snapshot = self.sessions.resume(
            approval.session_id(),
            self.scope.principal_id(),
            self.scope.tenant_id(),
            self.scope.workspace_id(),
        )?;
        let session = snapshot.session().clone();
        let intent = service_task_intent(request.request())?;
        let _active_lease = self.acquire_execution_lease(&session, now, 0)?;
        let summary = self.controller.run_with_approval(
            intent,
            session.clone(),
            &self.approvals,
            request.approval_id(),
            now,
        )?;
        self.persist_execution(&session, &summary, "local", now)?;

        Ok(ServiceResponse::run(service_run_result(
            &summary,
            session.id().clone(),
            None,
            now,
        )?))
    }

    fn run_agent(
        &self,
        request: &ServiceAgentRunRequest,
        now: Timestamp,
    ) -> Result<ServiceResponse, RuntimeServiceError> {
        let agent = self
            .agent
            .as_ref()
            .ok_or(RuntimeServiceError::AgentUnavailable)?;
        let (session, history) = match request.session_id() {
            Some(session_id) => {
                let snapshot = self.sessions.resume(
                    session_id,
                    self.scope.principal_id(),
                    self.scope.tenant_id(),
                    self.scope.workspace_id(),
                )?;
                (
                    snapshot.session().clone(),
                    snapshot.agent_messages().to_vec(),
                )
            }
            None => {
                let session = self.allocate_session(now)?;
                self.sessions.create(&session)?;
                (session, Vec::new())
            }
        };
        let _active_lease =
            self.acquire_execution_lease(&session, now, u64::from(agent.max_tool_calls))?;
        let provider_id = agent.provider.manifest().id().as_str();
        let l1_evidence = self.sessions.l1_evidence_context(
            session.id(),
            session.principal_id(),
            session.tenant_id(),
            session.workspace_id(),
            provider_id,
        )?;
        let mut agent_request = AgentRunRequest::new(session.clone(), history, request.task(), now)
            .with_skill_context(agent.skill_context.as_deref())
            .with_l1_evidence(Some(&l1_evidence));
        if let Some(harness) = request.requested_harness() {
            agent_request = agent_request.with_trusted_harness(harness.clone());
        }

        match agent.loop_engine.run_with_request(
            agent.provider.as_ref(),
            &self.controller,
            agent_request,
        ) {
            Ok(summary) => self.finish_agent_run(
                &session,
                &summary,
                provider_id,
                "completed",
                None,
                None,
                now,
            ),
            Err(AgentLoopError::ApprovalRequired { reason, summary }) => {
                let run = summary.runs().last().ok_or(ApprovalError::InvalidSummary)?;
                let approval = self.create_approval(request.task(), &session, run, now)?;
                self.finish_agent_run(
                    &session,
                    &summary,
                    provider_id,
                    "approval_required",
                    Some(reason),
                    Some(&approval),
                    now,
                )
            }
            Err(error) => Err(error.into()),
        }
    }

    fn resume_agent(
        &self,
        request: &ServiceAgentResumeRequest,
        now: Timestamp,
    ) -> Result<ServiceResponse, RuntimeServiceError> {
        let agent = self
            .agent
            .as_ref()
            .ok_or(RuntimeServiceError::AgentUnavailable)?;
        let approval = self.scoped_approval(request.approval_id())?;
        let snapshot = self.sessions.resume(
            approval.session_id(),
            self.scope.principal_id(),
            self.scope.tenant_id(),
            self.scope.workspace_id(),
        )?;
        let session = snapshot.session().clone();
        let _active_lease =
            self.acquire_execution_lease(&session, now, u64::from(agent.max_tool_calls))?;
        let provider_id = agent.provider.manifest().id().as_str();
        let l1_evidence = self.sessions.l1_evidence_context(
            session.id(),
            session.principal_id(),
            session.tenant_id(),
            session.workspace_id(),
            provider_id,
        )?;
        let trusted_harness = snapshot
            .events()
            .iter()
            .rev()
            .find(|event| {
                event.context().execution_id() == Some(approval.execution_id())
                    && event.context().harness_id().is_some()
            })
            .and_then(|event| event.context().harness_id())
            .cloned();
        let mut approval_context =
            AgentApprovalContext::new(session.clone(), &self.approvals, request.approval_id(), now)
                .with_l1_evidence(Some(&l1_evidence));
        if let Some(harness) = trusted_harness {
            approval_context = approval_context.with_trusted_harness(harness);
        }

        match agent
            .loop_engine
            .run_with_history_and_approval_and_skill_context(
                agent.provider.as_ref(),
                &self.controller,
                snapshot.agent_messages().to_vec(),
                approval_context,
                agent.skill_context.as_deref(),
                "resume approved operation",
            ) {
            Ok(summary) => self.finish_agent_run(
                &session,
                &summary,
                provider_id,
                "completed",
                None,
                None,
                now,
            ),
            Err(AgentLoopError::ApprovalRequired { reason, summary }) => {
                let run = summary.runs().last().ok_or(ApprovalError::InvalidSummary)?;
                let next_approval =
                    self.create_approval("agent continuation", &session, run, now)?;
                self.finish_agent_run(
                    &session,
                    &summary,
                    provider_id,
                    "approval_required",
                    Some(reason),
                    Some(&next_approval),
                    now,
                )
            }
            Err(error) => Err(error.into()),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_agent_run(
        &self,
        session: &Session,
        summary: &AgentRunSummary,
        provider: &str,
        status: &str,
        status_detail: Option<String>,
        approval: Option<&PendingApproval>,
        now: Timestamp,
    ) -> Result<ServiceResponse, RuntimeServiceError> {
        for run in summary.runs() {
            self.persist_execution(session, run, provider, now)?;
        }
        self.sessions.save_agent_transcript(
            session.id(),
            session.principal_id(),
            session.tenant_id(),
            session.workspace_id(),
            summary.messages(),
        )?;
        let mut result = service_agent_run_result(summary, session.id().clone(), status);
        if let Some(detail) = status_detail {
            result = result.with_status_detail(detail);
        }
        if let Some(approval) = approval {
            result = result.with_approval(service_approval_summary(approval, now)?);
        }
        Ok(ServiceResponse::agent_run(result))
    }

    fn create_approval(
        &self,
        task: &str,
        session: &Session,
        summary: &RunSummary,
        now: Timestamp,
    ) -> Result<PendingApproval, RuntimeServiceError> {
        let (capability, request_digest) = summary
            .events()
            .iter()
            .find_map(|event| match event.payload() {
                EventPayload::Effect {
                    capability,
                    request_digest,
                } => Some((capability.as_str(), request_digest.clone())),
                _ => None,
            })
            .ok_or(ApprovalError::InvalidSummary)?;
        let expires_at = Timestamp::from_unix_seconds(now.as_unix_seconds().saturating_add(900));
        let approval = ApprovalRequest::new(
            format!("approval-{}-{}", session.id(), summary.execution_id()),
            session.id().clone(),
            summary.execution_id().clone(),
            session.principal_id().clone(),
            summary.selected_gene().clone(),
            request_digest,
            service_approval_request_summary(task, capability),
            self.controller.policy_version(),
            expires_at,
        )?;
        Ok(self.approvals.create(approval)?)
    }

    fn acquire_execution_lease(
        &self,
        session: &Session,
        now: Timestamp,
        max_tools: u64,
    ) -> Result<Option<ActiveServiceLease>, RuntimeServiceError> {
        let Some(fleet) = &self.fleet else {
            return Ok(None);
        };
        let now = now.as_unix_seconds();
        fleet.engine.expire_leases(now)?;
        let sequence = self.next_lease.fetch_add(1, Ordering::Relaxed);
        let lease_id = format!("service-{}-{}-{}", session.id(), now, sequence);
        fleet.engine.acquire_lease(
            lease_id.clone(),
            fleet.node_id.clone(),
            format!("service:{}", session.id()),
            FleetBudget::new(0, max_tools, 60 * 60, 0),
            now,
            60 * 60,
        )?;
        Ok(Some(ActiveServiceLease {
            engine: Arc::clone(&fleet.engine),
            lease_id,
        }))
    }

    fn allocate_session(&self, now: Timestamp) -> Result<Session, RuntimeServiceError> {
        let sequence = self.next_session.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let session_id = SessionId::new(format!("service-session-{timestamp}-{sequence}"))?;
        Ok(Session::new(
            session_id,
            self.scope.principal_id().clone(),
            self.scope.tenant_id().clone(),
            self.scope.workspace_id().clone(),
            now,
        ))
    }

    fn persist_execution(
        &self,
        session: &Session,
        summary: &RunSummary,
        provider: &str,
        now: Timestamp,
    ) -> Result<EvaluationReceipt, RuntimeServiceError> {
        let mut evaluation_request = EvaluationRequest::new(
            summary.execution_id().clone(),
            summary.receipts().to_vec(),
            run_status(summary.status()),
            summary
                .events()
                .iter()
                .filter(|event| event.event_type() == EventType::PolicyDenied)
                .map(|_| "policy_denied".to_owned())
                .collect(),
        )?;
        if let RunStatus::Failed { code } = summary.status() {
            evaluation_request = evaluation_request.with_terminal_failure(code)?;
        }

        let engine = EvaluationEngine::new();
        let mut results = vec![
            engine.evaluate_trajectory(&evaluation_request, 0),
            engine.evaluate_policy(&evaluation_request),
        ];
        if matches!(summary.status(), RunStatus::ApprovalRequired { .. }) {
            results.push(
                engine.require_human_review(&evaluation_request, "explicit approval is required"),
            );
        }
        let receipt = EvaluationReceipt::new(
            session.id().clone(),
            summary.execution_id().clone(),
            now,
            results,
        )?;
        self.sessions.append_execution(
            session.id(),
            session.principal_id(),
            session.tenant_id(),
            session.workspace_id(),
            summary.events(),
            &receipt,
            now,
        )?;
        let _ = self.sessions.record_evaluation_feedback(
            session,
            session.principal_id(),
            provider,
            &receipt,
        );
        Ok(receipt)
    }
}

fn current_timestamp() -> Timestamp {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    Timestamp::from_unix_seconds(seconds)
}

fn service_task_intent(request: &ServiceRunRequest) -> Result<TaskIntent, RuntimeServiceError> {
    let mut intent = TaskIntent::new(request.task()).map_err(ServiceContractError::from)?;
    if let Some(harness_id) = request.requested_harness() {
        intent = intent.with_harness(harness_id.clone());
    }
    if let Some(gene_id) = request.requested_gene() {
        intent = intent.with_gene(gene_id.clone());
    }
    Ok(intent)
}

fn service_run_result(
    summary: &RunSummary,
    session_id: SessionId,
    approval: Option<&PendingApproval>,
    now: Timestamp,
) -> Result<ServiceRunResult, RuntimeServiceError> {
    let mut result = ServiceRunResult::new(
        session_id,
        summary.execution_id().clone(),
        Some(summary.selected_harness().clone()),
        Some(summary.selected_gene().clone()),
        run_status(summary.status()),
        output_text(summary.output()),
        u64::try_from(summary.receipts().len()).unwrap_or(u64::MAX),
        u64::try_from(summary.events().len()).unwrap_or(u64::MAX),
    );
    if let RunStatus::ApprovalRequired { reason } = summary.status() {
        result = result.with_status_detail(reason.clone());
    }
    if let Some(approval) = approval {
        result = result.with_approval(service_approval_summary(approval, now)?);
    }
    Ok(result)
}

fn service_agent_run_result(
    summary: &AgentRunSummary,
    session_id: SessionId,
    status: &str,
) -> ServiceAgentRunResult {
    let last_run = summary.runs().last();
    let receipt_count = summary
        .runs()
        .iter()
        .map(|run| u64::try_from(run.receipts().len()).unwrap_or(u64::MAX))
        .fold(
            u64::try_from(summary.provider_receipts().len()).unwrap_or(u64::MAX),
            u64::saturating_add,
        );
    let event_count = summary
        .runs()
        .iter()
        .map(|run| u64::try_from(run.events().len()).unwrap_or(u64::MAX))
        .fold(0_u64, u64::saturating_add);
    ServiceAgentRunResult::new(
        session_id,
        last_run.map(|run| run.execution_id().clone()),
        last_run.map(|run| run.selected_harness().clone()),
        last_run.map(|run| run.selected_gene().clone()),
        status,
        summary.final_text(),
        summary.turns(),
        summary.tool_calls(),
        u32::try_from(summary.provider_receipts().len()).unwrap_or(u32::MAX),
        u64::from(summary.usage().prompt_tokens()),
        u64::from(summary.usage().completion_tokens()),
        u32::try_from(summary.runs().len()).unwrap_or(u32::MAX),
        receipt_count,
        event_count,
    )
}

fn service_approval_summary(
    approval: &PendingApproval,
    now: Timestamp,
) -> Result<ServiceApprovalSummary, RuntimeServiceError> {
    Ok(ServiceApprovalSummary::new(
        approval.id(),
        approval.session_id().clone(),
        approval.execution_id().clone(),
        approval.gene_id().clone(),
        approval.request_digest().clone(),
        approval.request_summary(),
        approval.policy_version(),
        approval.expires_at().as_unix_seconds(),
        approval.status_at(now).as_str(),
        approval.approver_id().cloned(),
        approval.created_at().as_unix_seconds(),
    )?)
}

fn service_approval_request_summary(task: &str, capability: &str) -> String {
    let mut parts = task.splitn(3, ':');
    let action = parts.next().unwrap_or("task");
    let target = parts.next().unwrap_or("workspace");
    format!("{capability} for {action} on {target}")
}

fn service_evolution_summary(record: EvolutionRecord) -> ServiceEvolutionSummary {
    let proposal = record.proposal();
    let evaluation = record.evaluation().map(|evaluation| {
        ServiceEvolutionEvaluation::new(
            evaluation.trajectory_score(),
            evaluation.outcome_score(),
            evaluation.holdout_passed(),
            evaluation.policy_passed(),
            evaluation.regression_passed(),
            evaluation.evaluated_at(),
            evaluation.holdout_digest().cloned(),
        )
    });
    let approval = record
        .approval()
        .zip(record.signature())
        .map(|(approval, signature)| {
            ServiceEvolutionApproval::new(
                approval.approver().clone(),
                approval.policy_version(),
                approval.approved_at(),
                signature.signer().clone(),
                !signature.signature().is_empty(),
            )
        });
    let canary = record.canary().map(|canary| {
        ServiceEvolutionCanary::new(
            canary.passed(),
            canary.failure_count(),
            canary.note(),
            canary.evaluated_at(),
        )
    });
    ServiceEvolutionSummary::new(
        proposal.proposal_id().clone(),
        proposal.source().as_str(),
        proposal.base_artifact().clone(),
        proposal.candidate_artifact().clone(),
        proposal.evidence_digest().clone(),
        proposal.expected_outcome(),
        proposal.created_at(),
        record.state().as_str(),
        evaluation,
        approval,
        canary,
    )
}

fn service_session_summary(session: Session) -> ServiceSessionSummary {
    ServiceSessionSummary::new(
        session.id().clone(),
        session.principal_id().clone(),
        session.tenant_id().clone(),
        session.workspace_id().clone(),
        session.created_at(),
    )
}

fn run_status(status: &RunStatus) -> &'static str {
    match status {
        RunStatus::Completed => "completed",
        RunStatus::Denied { .. } => "denied",
        RunStatus::ApprovalRequired { .. } => "approval_required",
        RunStatus::Failed { .. } => "failed",
    }
}

fn output_text(output: Option<&[u8]>) -> String {
    output
        .map(String::from_utf8_lossy)
        .unwrap_or_default()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executors::WorkspaceRoot;
    use pandora_harnesses::HarnessCatalog;
    use pandora_provider::{
        MessageRole, ModelRequest, ModelResponse, ProviderError, ProviderManifest, TokenUsage,
        ToolCall,
    };
    use pandora_types::{
        ArtifactId, ArtifactSignature, Capability, ContextClassification, EvolutionPolicy,
        EvolutionSource, HoldoutEvaluation, MemoryApproval, MemoryKind, MemoryRecord, MemoryScope,
        MutationProposal, Operation, ParliamentApproval, PolicyContext, ReplacementReceipt,
        RequestDigest,
    };
    use std::sync::Mutex;

    struct SequenceProvider {
        manifest: ProviderManifest,
        responses: Mutex<Vec<ModelResponse>>,
        requests: Mutex<Vec<Vec<pandora_provider::ChatMessage>>>,
    }

    impl SequenceProvider {
        fn new(responses: Vec<ModelResponse>) -> Self {
            Self {
                manifest: ProviderManifest::new(
                    "service-provider",
                    "Service provider",
                    "http://127.0.0.1:1/v1",
                    "model-a",
                    "PANDORA_SERVICE_PROVIDER_KEY",
                )
                .unwrap(),
                responses: Mutex::new(responses),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn requests(&self) -> Vec<Vec<pandora_provider::ChatMessage>> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl Provider for SequenceProvider {
        fn manifest(&self) -> &ProviderManifest {
            &self.manifest
        }

        fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
            self.requests
                .lock()
                .unwrap()
                .push(request.messages().to_vec());
            self.responses
                .lock()
                .unwrap()
                .pop()
                .ok_or(ProviderError::InvalidResponse)
        }
    }

    #[test]
    fn evolution_inventory_is_real_and_redacts_signature_material() {
        let root = crate::test_support::new_temp_dir("pandora-runtime-service-evolution").unwrap();
        let scope = RuntimeServiceScope::new(
            PrincipalId::new("principal-a").unwrap(),
            TenantId::new("tenant-a").unwrap(),
            WorkspaceId::new("workspace-a").unwrap(),
        );
        let proposal_id = ProposalId::new("proposal-a").unwrap();
        let evolution = Arc::new(
            EvolutionEngine::open(
                root.join("evolution.sqlite3"),
                EvolutionPolicy::production(1),
            )
            .unwrap(),
        );
        evolution
            .submit(
                MutationProposal::new(
                    proposal_id.as_str(),
                    EvolutionSource::Gepa,
                    ArtifactId::new("base-a").unwrap(),
                    ArtifactId::new("candidate-a").unwrap(),
                    RequestDigest::new("evidence-a").unwrap(),
                    "improve verification reliability",
                    Timestamp::from_unix_seconds(10),
                )
                .unwrap(),
            )
            .unwrap();
        evolution
            .record_evaluation(
                HoldoutEvaluation::new(
                    proposal_id.clone(),
                    95,
                    96,
                    true,
                    true,
                    true,
                    Timestamp::from_unix_seconds(11),
                )
                .with_holdout_digest("holdout-a")
                .unwrap(),
            )
            .unwrap();
        evolution
            .approve(
                &proposal_id,
                ParliamentApproval::new(
                    proposal_id.clone(),
                    PrincipalId::new("parliament-a").unwrap(),
                    1,
                    Timestamp::from_unix_seconds(12),
                ),
                ArtifactSignature::new(
                    ArtifactId::new("candidate-a").unwrap(),
                    PrincipalId::new("signer-a").unwrap(),
                    "secret-signature-material",
                )
                .unwrap(),
            )
            .unwrap();
        let catalog =
            Arc::new(ArtifactCatalog::open(root.join("artifact-catalog.sqlite3")).unwrap());
        catalog
            .activate(&ReplacementReceipt::new(
                proposal_id.clone(),
                ArtifactId::new("base-a").unwrap(),
                ArtifactId::new("candidate-a").unwrap(),
                Timestamp::from_unix_seconds(13),
            ))
            .unwrap();
        let service = RuntimeService::new(
            ExecutionController::new(WorkspaceRoot::new(&root).unwrap()),
            SessionStore::open(root.join("sessions.sqlite3")).unwrap(),
            ApprovalStore::open(root.join("sessions.sqlite3")).unwrap(),
            scope,
        )
        .with_evolution(evolution)
        .with_artifact_catalog(catalog);

        let response = service
            .handle(
                &ServiceRequest::evolution_list(16).unwrap(),
                Timestamp::from_unix_seconds(13),
            )
            .unwrap();
        let ServiceResponse::EvolutionList { proposals, .. } = &response else {
            panic!("expected an evolution list response");
        };
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].state(), "approved");
        assert_eq!(
            proposals[0].approval().unwrap().signer_id().as_str(),
            "signer-a"
        );
        assert!(proposals[0].approval().unwrap().signature_present());
        let serialized = serde_json::to_string(&response).unwrap();
        assert!(!serialized.contains("secret-signature-material"));

        let response = service
            .handle(
                &ServiceRequest::evolution_activations(16).unwrap(),
                Timestamp::from_unix_seconds(14),
            )
            .unwrap();
        let ServiceResponse::EvolutionActivations { activations, .. } = response else {
            panic!("expected an evolution activation response");
        };
        assert_eq!(activations.len(), 1);
        assert_eq!(activations[0].proposal_id(), &proposal_id);
        assert_eq!(activations[0].base_artifact().as_str(), "base-a");
        assert_eq!(activations[0].candidate_artifact().as_str(), "candidate-a");

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn agent_write_resumes_only_the_exact_governed_pending_call() {
        let root = crate::test_support::new_temp_dir("pandora-runtime-service-agent").unwrap();
        std::fs::write(root.join("README.md"), b"fixture\n").unwrap();
        let scope = RuntimeServiceScope::new(
            PrincipalId::new("principal-a").unwrap(),
            TenantId::new("tenant-a").unwrap(),
            WorkspaceId::new("workspace-a").unwrap(),
        );
        let provider = Arc::new(SequenceProvider::new(vec![
            ModelResponse::new("README updated", Vec::new(), TokenUsage::new(4, 2)),
            ModelResponse::new(
                "",
                vec![
                    ToolCall::new(
                        "call-patch",
                        "workspace.patch",
                        serde_json::json!({"path": "README.md", "content": "changed"}),
                    )
                    .unwrap(),
                ],
                TokenUsage::new(8, 3),
            ),
        ]));
        let policy = PolicyContext::new(
            1,
            [
                Capability::FilesystemRead,
                Capability::FilesystemWrite,
                Capability::ProviderInvoke,
            ],
            [Operation::Write],
        );
        let service = RuntimeService::new(
            ExecutionController::with_policy_and_harnesses(
                WorkspaceRoot::new(&root).unwrap(),
                policy,
                HarnessCatalog::builtins(),
            ),
            SessionStore::open(root.join("sessions.sqlite3")).unwrap(),
            ApprovalStore::open(root.join("sessions.sqlite3")).unwrap(),
            scope,
        )
        .with_agent(
            provider.clone(),
            4,
            4,
            root.join("context-cache.json"),
            None,
        )
        .unwrap();

        let first = service
            .handle(
                &ServiceRequest::agent_run(
                    ServiceAgentRunRequest::new("Update the README", None, None).unwrap(),
                ),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();
        let ServiceResponse::AgentRun { run, .. } = first else {
            panic!("expected an agent run response");
        };
        assert_eq!(run.status(), "approval_required");
        let approval_id = run.approval().unwrap().approval_id().to_owned();
        assert_eq!(std::fs::read(root.join("README.md")).unwrap(), b"fixture\n");

        service
            .handle(
                &ServiceRequest::approval_resolve(&approval_id, true).unwrap(),
                Timestamp::from_unix_seconds(11),
            )
            .unwrap();
        let resumed = service
            .handle(
                &ServiceRequest::agent_resume(
                    ServiceAgentResumeRequest::new(&approval_id).unwrap(),
                ),
                Timestamp::from_unix_seconds(12),
            )
            .unwrap();
        let ServiceResponse::AgentRun { run, .. } = resumed else {
            panic!("expected a resumed agent run response");
        };
        assert_eq!(run.status(), "completed");
        assert_eq!(run.output(), "README updated");
        assert_eq!(std::fs::read(root.join("README.md")).unwrap(), b"changed");

        let requests = provider.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1][1].role(), MessageRole::User);
        assert_eq!(requests[1][2].role(), MessageRole::Assistant);
        assert_eq!(requests[1][3].role(), MessageRole::Tool);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn session_memory_returns_scoped_l1_and_l2_records() {
        let root = crate::test_support::new_temp_dir("pandora-runtime-service-memory").unwrap();
        let scope = RuntimeServiceScope::new(
            PrincipalId::new("principal-a").unwrap(),
            TenantId::new("tenant-a").unwrap(),
            WorkspaceId::new("workspace-a").unwrap(),
        );
        let service = RuntimeService::new(
            ExecutionController::new(WorkspaceRoot::new(&root).unwrap()),
            SessionStore::open(root.join("sessions.sqlite3")).unwrap(),
            ApprovalStore::open(root.join("sessions.sqlite3")).unwrap(),
            scope,
        );
        let session = Session::new(
            SessionId::new("session-a").unwrap(),
            service.scope.principal_id().clone(),
            service.scope.tenant_id().clone(),
            service.scope.workspace_id().clone(),
            Timestamp::from_unix_seconds(1),
        );
        service.sessions.create(&session).unwrap();
        let memory_scope = MemoryScope::new(
            session.tenant_id().clone(),
            session.workspace_id().clone(),
            session.id().clone(),
            "local",
        )
        .unwrap();
        let record = MemoryRecord::new_l1(
            "lesson-1",
            MemoryKind::Lesson,
            memory_scope.clone(),
            "retry after fresh verification",
            ContextClassification::Internal,
            Timestamp::from_unix_seconds(2),
            "evaluation:execution-1",
        )
        .unwrap();
        service
            .sessions
            .record_memory(session.principal_id(), &record)
            .unwrap();
        service
            .sessions
            .promote_memory(
                session.principal_id(),
                &memory_scope,
                record.id(),
                MemoryApproval::new("approval-1", "owner").unwrap(),
                Timestamp::from_unix_seconds(3),
            )
            .unwrap();

        let response = service
            .handle(
                &ServiceRequest::session_memory(session.id().as_str(), 16).unwrap(),
                Timestamp::from_unix_seconds(4),
            )
            .unwrap();
        let ServiceResponse::SessionMemory { memory, .. } = response else {
            panic!("expected a session memory response");
        };

        assert_eq!(memory.session_id(), session.id());
        assert_eq!(memory.records().len(), 2);
        assert!(memory.records().iter().any(|item| item.tier() == "l1"));
        assert!(memory.records().iter().any(|item| item.tier() == "l2"));
        assert!(
            memory
                .records()
                .iter()
                .all(|item| item.memory_id() == "lesson-1")
        );
        assert!(
            memory
                .records()
                .iter()
                .all(|item| item.provenance() == "evaluation:execution-1")
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn engine_inventory_exposes_research_population_strategy() {
        let root = crate::test_support::new_temp_dir("pandora-runtime-service-engines").unwrap();
        let scope = RuntimeServiceScope::new(
            PrincipalId::new("principal-a").unwrap(),
            TenantId::new("tenant-a").unwrap(),
            WorkspaceId::new("workspace-a").unwrap(),
        );
        let service = RuntimeService::new(
            ExecutionController::new(WorkspaceRoot::new(&root).unwrap()),
            SessionStore::open(root.join("sessions.sqlite3")).unwrap(),
            ApprovalStore::open(root.join("sessions.sqlite3")).unwrap(),
            scope,
        );

        let response = service
            .handle(&ServiceRequest::engines(), Timestamp::from_unix_seconds(1))
            .unwrap();
        let ServiceResponse::Engines { engines, .. } = response else {
            panic!("expected an engine response");
        };

        let population = engines
            .iter()
            .find(|engine| engine.id() == "population-strategy")
            .expect("population strategy should be discoverable");
        assert_eq!(population.name(), "PopulationStrategy");
        assert_eq!(population.authority(), "Proposal only");

        let _ = std::fs::remove_dir_all(root);
    }
}
