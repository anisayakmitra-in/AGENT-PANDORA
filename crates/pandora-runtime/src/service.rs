use crate::evaluation_engine::EvaluationEngine;
use crate::execution_controller::{ExecutionController, RunStatus, RunSummary, RuntimeError};
use crate::sessions::{SessionError, SessionStore};
use crate::tool_engine::ToolEngine;
use pandora_types::{
    EvaluationContractError, EvaluationReceipt, EvaluationRequest, EventType, IdError, MemoryTier,
    PrincipalId, ServiceContractError, ServiceEngineSummary, ServiceEventPage,
    ServiceHarnessSummary, ServiceHealth, ServiceMemoryPage, ServiceMemoryRecord,
    ServiceProviderSummary, ServiceRequest, ServiceResponse, ServiceRunRequest, ServiceRunResult,
    ServiceSessionDetail, ServiceSessionSummary, ServiceToolSummary, Session, SessionId,
    TaskIntent, TenantId, Timestamp, WorkspaceId,
};
use std::fmt;
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
    Session(SessionError),
}

impl RuntimeServiceError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedProtocol { .. } => "unsupported_protocol_version",
            Self::Contract(_) => "invalid_service_request",
            Self::Evaluation(_) => "invalid_evaluation",
            Self::Identifier(_) => "invalid_session_identifier",
            Self::Runtime(_) => "runtime_execution_failed",
            Self::Session(SessionError::SessionNotFound) => "session_not_found",
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
            Self::Session(error) => error.fmt(formatter),
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

impl From<SessionError> for RuntimeServiceError {
    fn from(error: SessionError) -> Self {
        Self::Session(error)
    }
}

pub struct RuntimeService {
    controller: ExecutionController,
    sessions: SessionStore,
    scope: RuntimeServiceScope,
    providers: Vec<ServiceProviderSummary>,
    next_session: AtomicU64,
}

impl RuntimeService {
    pub fn new(
        controller: ExecutionController,
        sessions: SessionStore,
        scope: RuntimeServiceScope,
    ) -> Self {
        Self::new_with_providers(controller, sessions, scope, Vec::new())
    }

    pub fn new_with_providers(
        controller: ExecutionController,
        sessions: SessionStore,
        scope: RuntimeServiceScope,
        providers: Vec<ServiceProviderSummary>,
    ) -> Self {
        Self {
            controller,
            sessions,
            scope,
            providers,
            next_session: AtomicU64::new(1),
        }
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
            ServiceRequest::Run { request, .. } => self.run(request, now),
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

    fn run(
        &self,
        request: &ServiceRunRequest,
        now: Timestamp,
    ) -> Result<ServiceResponse, RuntimeServiceError> {
        let session = self.allocate_session(now)?;
        let mut intent = TaskIntent::new(request.task()).map_err(ServiceContractError::from)?;
        if let Some(harness_id) = request.requested_harness() {
            intent = intent.with_harness(harness_id.clone());
        }
        if let Some(gene_id) = request.requested_gene() {
            intent = intent.with_gene(gene_id.clone());
        }

        let summary = self.controller.run_at(intent, session.clone(), now)?;
        self.sessions.create(&session)?;
        self.persist_execution(&session, &summary, now)?;

        let mut result = ServiceRunResult::new(
            session.id().clone(),
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
        Ok(ServiceResponse::run(result))
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
            "local",
            &receipt,
        );
        Ok(receipt)
    }
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
    use pandora_types::{
        ContextClassification, MemoryApproval, MemoryKind, MemoryRecord, MemoryScope,
    };

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
