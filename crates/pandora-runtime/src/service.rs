use crate::evaluation_engine::EvaluationEngine;
use crate::execution_controller::{ExecutionController, RunStatus, RunSummary, RuntimeError};
use crate::sessions::{SessionError, SessionStore};
use crate::tool_engine::ToolEngine;
use pandora_types::{
    EvaluationContractError, EvaluationReceipt, EvaluationRequest, EventType, IdError, PrincipalId,
    ServiceContractError, ServiceEngineSummary, ServiceEventPage, ServiceHarnessSummary,
    ServiceHealth, ServiceProviderSummary, ServiceRequest, ServiceResponse, ServiceRunRequest,
    ServiceRunResult, ServiceSessionDetail, ServiceSessionSummary, ServiceToolSummary, Session,
    SessionId, TaskIntent, TenantId, Timestamp, WorkspaceId,
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

        Ok(ServiceResponse::run(ServiceRunResult::new(
            session.id().clone(),
            summary.execution_id().clone(),
            Some(summary.selected_harness().clone()),
            Some(summary.selected_gene().clone()),
            run_status(summary.status()),
            output_text(summary.output()),
            u64::try_from(summary.receipts().len()).unwrap_or(u64::MAX),
            u64::try_from(summary.events().len()).unwrap_or(u64::MAX),
        )))
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
