use crate::effect::{RequestError, Timestamp};
use crate::events::RuntimeEvent;
use crate::ids::{
    ExecutionId, GeneId, HarnessId, IdError, PrincipalId, RequestDigest, SessionId, TenantId,
    WorkspaceId,
};
use crate::memory::{MemoryKind, MemoryTier};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const LOCAL_SERVICE_PROTOCOL_VERSION: u16 = 1;
pub const MAX_SERVICE_EVENT_PAGE: u16 = 256;
pub const MAX_SERVICE_SESSION_PAGE: u16 = 256;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServiceContractError {
    InvalidTask(RequestError),
    InvalidIdentifier(IdError),
    InvalidApprovalIdentifier,
    InvalidApprovalSummary,
    InvalidPageLimit { limit: u16, maximum: u16 },
}

impl fmt::Display for ServiceContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTask(error) => error.fmt(formatter),
            Self::InvalidIdentifier(error) => error.fmt(formatter),
            Self::InvalidApprovalIdentifier => {
                formatter.write_str("approval identifier is invalid")
            }
            Self::InvalidApprovalSummary => formatter.write_str("approval summary is invalid"),
            Self::InvalidPageLimit { limit, maximum } => {
                write!(
                    formatter,
                    "page limit {limit} must be between 1 and {maximum}"
                )
            }
        }
    }
}

impl std::error::Error for ServiceContractError {}

impl From<RequestError> for ServiceContractError {
    fn from(error: RequestError) -> Self {
        Self::InvalidTask(error)
    }
}

impl From<IdError> for ServiceContractError {
    fn from(error: IdError) -> Self {
        Self::InvalidIdentifier(error)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceRunRequest {
    task: String,
    requested_harness: Option<HarnessId>,
    requested_gene: Option<GeneId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceRunResumeRequest {
    approval_id: String,
    request: ServiceRunRequest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceAgentRunRequest {
    task: String,
    session_id: Option<SessionId>,
    requested_harness: Option<HarnessId>,
}

impl ServiceAgentRunRequest {
    pub fn new(
        task: impl Into<String>,
        session_id: Option<SessionId>,
        requested_harness: Option<HarnessId>,
    ) -> Result<Self, ServiceContractError> {
        let task = crate::session::TaskIntent::new(task.into())?
            .summary()
            .to_owned();
        let request = Self {
            task,
            session_id,
            requested_harness,
        };
        request.validate()?;
        Ok(request)
    }

    pub fn task(&self) -> &str {
        &self.task
    }

    pub fn session_id(&self) -> Option<&SessionId> {
        self.session_id.as_ref()
    }

    pub fn requested_harness(&self) -> Option<&HarnessId> {
        self.requested_harness.as_ref()
    }

    pub fn validate(&self) -> Result<(), ServiceContractError> {
        crate::session::TaskIntent::new(self.task.clone())?;
        if let Some(session_id) = self.session_id() {
            SessionId::new(session_id.as_str())?;
        }
        if let Some(harness_id) = self.requested_harness() {
            HarnessId::new(harness_id.as_str())?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceAgentResumeRequest {
    approval_id: String,
}

impl ServiceAgentResumeRequest {
    pub fn new(approval_id: impl Into<String>) -> Result<Self, ServiceContractError> {
        let approval_id = approval_id.into();
        validate_approval_id(&approval_id)?;
        Ok(Self { approval_id })
    }

    pub fn approval_id(&self) -> &str {
        &self.approval_id
    }

    pub fn validate(&self) -> Result<(), ServiceContractError> {
        validate_approval_id(&self.approval_id)
    }
}

impl ServiceRunResumeRequest {
    pub fn new(
        approval_id: impl Into<String>,
        request: ServiceRunRequest,
    ) -> Result<Self, ServiceContractError> {
        let approval_id = approval_id.into();
        validate_approval_id(&approval_id)?;
        request.validate()?;
        Ok(Self {
            approval_id,
            request,
        })
    }

    pub fn approval_id(&self) -> &str {
        &self.approval_id
    }

    pub const fn request(&self) -> &ServiceRunRequest {
        &self.request
    }

    pub fn validate(&self) -> Result<(), ServiceContractError> {
        validate_approval_id(&self.approval_id)?;
        self.request.validate()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceHarnessSummary {
    id: HarnessId,
    version: String,
    name: String,
    kind: String,
    gene_count: u32,
    runnable: bool,
    #[serde(default)]
    gene_ids: Vec<GeneId>,
}

impl ServiceHarnessSummary {
    pub fn new(
        id: HarnessId,
        version: impl Into<String>,
        name: impl Into<String>,
        kind: impl Into<String>,
        gene_count: u32,
        runnable: bool,
    ) -> Self {
        Self {
            id,
            version: version.into(),
            name: name.into(),
            kind: kind.into(),
            gene_count,
            runnable,
            gene_ids: Vec::new(),
        }
    }

    pub fn with_gene_ids(mut self, gene_ids: Vec<GeneId>) -> Self {
        self.gene_ids = gene_ids;
        self
    }

    pub fn id(&self) -> &HarnessId {
        &self.id
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }

    pub const fn gene_count(&self) -> u32 {
        self.gene_count
    }

    pub const fn runnable(&self) -> bool {
        self.runnable
    }

    pub fn gene_ids(&self) -> &[GeneId] {
        &self.gene_ids
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceProviderSummary {
    name: String,
    model: String,
    protocol: String,
    active: bool,
    credential_configured: bool,
    fallback_provider: Option<String>,
}

impl ServiceProviderSummary {
    pub fn new(
        name: impl Into<String>,
        model: impl Into<String>,
        protocol: impl Into<String>,
        active: bool,
        credential_configured: bool,
        fallback_provider: Option<String>,
    ) -> Self {
        Self {
            name: name.into(),
            model: model.into(),
            protocol: protocol.into(),
            active,
            credential_configured,
            fallback_provider,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn protocol(&self) -> &str {
        &self.protocol
    }

    pub const fn active(&self) -> bool {
        self.active
    }

    pub const fn credential_configured(&self) -> bool {
        self.credential_configured
    }

    pub fn fallback_provider(&self) -> Option<&str> {
        self.fallback_provider.as_deref()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceEngineSummary {
    id: String,
    name: String,
    role: String,
    authority: String,
}

impl ServiceEngineSummary {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        role: impl Into<String>,
        authority: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            role: role.into(),
            authority: authority.into(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn role(&self) -> &str {
        &self.role
    }

    pub fn authority(&self) -> &str {
        &self.authority
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceToolSummary {
    id: GeneId,
    version: String,
    name: String,
    capability: String,
    operation: String,
}

impl ServiceToolSummary {
    pub fn new(
        id: GeneId,
        version: impl Into<String>,
        name: impl Into<String>,
        capability: impl Into<String>,
        operation: impl Into<String>,
    ) -> Self {
        Self {
            id,
            version: version.into(),
            name: name.into(),
            capability: capability.into(),
            operation: operation.into(),
        }
    }

    pub fn id(&self) -> &GeneId {
        &self.id
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn capability(&self) -> &str {
        &self.capability
    }

    pub fn operation(&self) -> &str {
        &self.operation
    }
}

impl ServiceRunRequest {
    pub fn new(
        task: impl Into<String>,
        requested_harness: Option<HarnessId>,
        requested_gene: Option<GeneId>,
    ) -> Result<Self, ServiceContractError> {
        let task = task.into();
        let task = crate::session::TaskIntent::new(task)?.summary().to_owned();

        Ok(Self {
            task,
            requested_harness,
            requested_gene,
        })
    }

    pub fn task(&self) -> &str {
        &self.task
    }

    pub fn requested_harness(&self) -> Option<&HarnessId> {
        self.requested_harness.as_ref()
    }

    pub fn requested_gene(&self) -> Option<&GeneId> {
        self.requested_gene.as_ref()
    }

    pub fn validate(&self) -> Result<(), ServiceContractError> {
        crate::session::TaskIntent::new(self.task.clone())?;
        if let Some(harness_id) = self.requested_harness() {
            HarnessId::new(harness_id.as_str())?;
        }
        if let Some(gene_id) = self.requested_gene() {
            GeneId::new(gene_id.as_str())?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceEventPageRequest {
    session_id: SessionId,
    after_sequence: Option<u64>,
    limit: u16,
}

impl ServiceEventPageRequest {
    pub fn new(
        session_id: impl Into<String>,
        after_sequence: Option<u64>,
        limit: u16,
    ) -> Result<Self, ServiceContractError> {
        validate_page_limit(limit, MAX_SERVICE_EVENT_PAGE)?;

        Ok(Self {
            session_id: SessionId::new(session_id)?,
            after_sequence,
            limit,
        })
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub const fn after_sequence(&self) -> Option<u64> {
        self.after_sequence
    }

    pub const fn limit(&self) -> u16 {
        self.limit
    }

    pub fn validate(&self) -> Result<(), ServiceContractError> {
        SessionId::new(self.session_id.as_str())?;
        validate_page_limit(self.limit, MAX_SERVICE_EVENT_PAGE)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServiceRequest {
    Health {
        protocol_version: u16,
    },
    Capabilities {
        protocol_version: u16,
    },
    Providers {
        protocol_version: u16,
    },
    Engines {
        protocol_version: u16,
    },
    Tools {
        protocol_version: u16,
    },
    SessionList {
        protocol_version: u16,
        limit: u16,
    },
    SessionInspect {
        protocol_version: u16,
        session_id: SessionId,
    },
    SessionEvents {
        protocol_version: u16,
        request: ServiceEventPageRequest,
    },
    SessionMemory {
        protocol_version: u16,
        session_id: SessionId,
        limit: u16,
    },
    ApprovalList {
        protocol_version: u16,
        limit: u16,
    },
    ApprovalInspect {
        protocol_version: u16,
        approval_id: String,
    },
    ApprovalResolve {
        protocol_version: u16,
        approval_id: String,
        allow: bool,
    },
    Run {
        protocol_version: u16,
        request: ServiceRunRequest,
    },
    RunResume {
        protocol_version: u16,
        request: ServiceRunResumeRequest,
    },
    AgentRun {
        protocol_version: u16,
        request: ServiceAgentRunRequest,
    },
    AgentResume {
        protocol_version: u16,
        request: ServiceAgentResumeRequest,
    },
}

impl ServiceRequest {
    pub const fn health() -> Self {
        Self::Health {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
        }
    }

    pub const fn capabilities() -> Self {
        Self::Capabilities {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
        }
    }

    pub const fn providers() -> Self {
        Self::Providers {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
        }
    }

    pub const fn engines() -> Self {
        Self::Engines {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
        }
    }

    pub const fn tools() -> Self {
        Self::Tools {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
        }
    }

    pub fn session_list(limit: u16) -> Result<Self, ServiceContractError> {
        validate_page_limit(limit, MAX_SERVICE_SESSION_PAGE)?;
        Ok(Self::SessionList {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            limit,
        })
    }

    pub fn session_inspect(session_id: impl Into<String>) -> Result<Self, ServiceContractError> {
        Ok(Self::SessionInspect {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            session_id: SessionId::new(session_id)?,
        })
    }

    pub const fn session_events(request: ServiceEventPageRequest) -> Self {
        Self::SessionEvents {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            request,
        }
    }

    pub fn session_memory(
        session_id: impl Into<String>,
        limit: u16,
    ) -> Result<Self, ServiceContractError> {
        validate_page_limit(limit, MAX_SERVICE_SESSION_PAGE)?;
        Ok(Self::SessionMemory {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            session_id: SessionId::new(session_id.into())?,
            limit,
        })
    }

    pub const fn run(request: ServiceRunRequest) -> Self {
        Self::Run {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            request,
        }
    }

    pub fn approval_list(limit: u16) -> Result<Self, ServiceContractError> {
        validate_page_limit(limit, MAX_SERVICE_SESSION_PAGE)?;
        Ok(Self::ApprovalList {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            limit,
        })
    }

    pub fn approval_inspect(approval_id: impl Into<String>) -> Result<Self, ServiceContractError> {
        let approval_id = approval_id.into();
        validate_approval_id(&approval_id)?;
        Ok(Self::ApprovalInspect {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            approval_id,
        })
    }

    pub fn approval_resolve(
        approval_id: impl Into<String>,
        allow: bool,
    ) -> Result<Self, ServiceContractError> {
        let approval_id = approval_id.into();
        validate_approval_id(&approval_id)?;
        Ok(Self::ApprovalResolve {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            approval_id,
            allow,
        })
    }

    pub const fn run_resume(request: ServiceRunResumeRequest) -> Self {
        Self::RunResume {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            request,
        }
    }

    pub const fn agent_run(request: ServiceAgentRunRequest) -> Self {
        Self::AgentRun {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            request,
        }
    }

    pub const fn agent_resume(request: ServiceAgentResumeRequest) -> Self {
        Self::AgentResume {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            request,
        }
    }

    pub const fn protocol_version(&self) -> u16 {
        match self {
            Self::Health { protocol_version }
            | Self::Capabilities { protocol_version }
            | Self::Providers { protocol_version }
            | Self::Engines { protocol_version }
            | Self::Tools { protocol_version }
            | Self::SessionList {
                protocol_version, ..
            }
            | Self::SessionInspect {
                protocol_version, ..
            }
            | Self::SessionEvents {
                protocol_version, ..
            }
            | Self::SessionMemory {
                protocol_version, ..
            }
            | Self::ApprovalList {
                protocol_version, ..
            }
            | Self::ApprovalInspect {
                protocol_version, ..
            }
            | Self::ApprovalResolve {
                protocol_version, ..
            }
            | Self::Run {
                protocol_version, ..
            }
            | Self::RunResume {
                protocol_version, ..
            }
            | Self::AgentRun {
                protocol_version, ..
            }
            | Self::AgentResume {
                protocol_version, ..
            } => *protocol_version,
        }
    }

    pub const fn is_supported_protocol(&self) -> bool {
        self.protocol_version() == LOCAL_SERVICE_PROTOCOL_VERSION
    }

    pub fn validate(&self) -> Result<(), ServiceContractError> {
        match self {
            Self::Health { .. }
            | Self::Capabilities { .. }
            | Self::Providers { .. }
            | Self::Engines { .. } => Ok(()),
            Self::Tools { .. } => Ok(()),
            Self::SessionList { limit, .. } => {
                validate_page_limit(*limit, MAX_SERVICE_SESSION_PAGE)
            }
            Self::SessionInspect { session_id, .. } => {
                SessionId::new(session_id.as_str())?;
                Ok(())
            }
            Self::SessionEvents { request, .. } => request.validate(),
            Self::SessionMemory {
                session_id, limit, ..
            } => {
                SessionId::new(session_id.as_str())?;
                validate_page_limit(*limit, MAX_SERVICE_SESSION_PAGE)
            }
            Self::ApprovalList { limit, .. } => {
                validate_page_limit(*limit, MAX_SERVICE_SESSION_PAGE)
            }
            Self::ApprovalInspect { approval_id, .. }
            | Self::ApprovalResolve { approval_id, .. } => validate_approval_id(approval_id),
            Self::Run { request, .. } => request.validate(),
            Self::RunResume { request, .. } => request.validate(),
            Self::AgentRun { request, .. } => request.validate(),
            Self::AgentResume { request, .. } => request.validate(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceHealth {
    status: String,
}

impl ServiceHealth {
    pub fn ready() -> Self {
        Self {
            status: "ready".to_owned(),
        }
    }

    pub fn status(&self) -> &str {
        &self.status
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceSessionSummary {
    session_id: SessionId,
    principal_id: PrincipalId,
    tenant_id: TenantId,
    workspace_id: WorkspaceId,
    created_at_unix_seconds: u64,
}

impl ServiceSessionSummary {
    pub fn new(
        session_id: SessionId,
        principal_id: PrincipalId,
        tenant_id: TenantId,
        workspace_id: WorkspaceId,
        created_at: Timestamp,
    ) -> Self {
        Self {
            session_id,
            principal_id,
            tenant_id,
            workspace_id,
            created_at_unix_seconds: created_at.as_unix_seconds(),
        }
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
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

    pub const fn created_at_unix_seconds(&self) -> u64 {
        self.created_at_unix_seconds
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceSessionDetail {
    session: ServiceSessionSummary,
    event_count: u64,
}

impl ServiceSessionDetail {
    pub fn new(session: ServiceSessionSummary, event_count: u64) -> Self {
        Self {
            session,
            event_count,
        }
    }

    pub fn session(&self) -> &ServiceSessionSummary {
        &self.session
    }

    pub const fn event_count(&self) -> u64 {
        self.event_count
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceEventPage {
    session_id: SessionId,
    events: Vec<RuntimeEvent>,
    next_sequence: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceMemoryRecord {
    memory_id: String,
    tier: String,
    kind: String,
    summary: String,
    classification: String,
    created_at_unix_seconds: u64,
    provenance: String,
    origin: String,
    evidence_count: u16,
}

impl ServiceMemoryRecord {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        memory_id: impl Into<String>,
        tier: MemoryTier,
        kind: MemoryKind,
        summary: impl Into<String>,
        classification: impl Into<String>,
        created_at_unix_seconds: u64,
        provenance: impl Into<String>,
        origin: impl Into<String>,
        evidence_count: u16,
    ) -> Self {
        Self {
            memory_id: memory_id.into(),
            tier: tier.as_str().to_owned(),
            kind: kind.as_str().to_owned(),
            summary: summary.into(),
            classification: classification.into(),
            created_at_unix_seconds,
            provenance: provenance.into(),
            origin: origin.into(),
            evidence_count,
        }
    }

    pub fn memory_id(&self) -> &str {
        &self.memory_id
    }

    pub fn tier(&self) -> &str {
        &self.tier
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }

    pub fn summary(&self) -> &str {
        &self.summary
    }

    pub fn classification(&self) -> &str {
        &self.classification
    }

    pub const fn created_at_unix_seconds(&self) -> u64 {
        self.created_at_unix_seconds
    }

    pub fn provenance(&self) -> &str {
        &self.provenance
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    pub const fn evidence_count(&self) -> u16 {
        self.evidence_count
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceMemoryPage {
    session_id: SessionId,
    records: Vec<ServiceMemoryRecord>,
}

impl ServiceMemoryPage {
    pub fn new(session_id: SessionId, records: Vec<ServiceMemoryRecord>) -> Self {
        Self {
            session_id,
            records,
        }
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub fn records(&self) -> &[ServiceMemoryRecord] {
        &self.records
    }
}

impl ServiceEventPage {
    pub fn new(
        session_id: SessionId,
        events: Vec<RuntimeEvent>,
        next_sequence: Option<u64>,
    ) -> Self {
        Self {
            session_id,
            events,
            next_sequence,
        }
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub fn events(&self) -> &[RuntimeEvent] {
        &self.events
    }

    pub const fn next_sequence(&self) -> Option<u64> {
        self.next_sequence
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceApprovalSummary {
    approval_id: String,
    session_id: SessionId,
    execution_id: ExecutionId,
    gene_id: GeneId,
    request_digest: RequestDigest,
    request_summary: String,
    policy_version: u32,
    expires_at_unix_seconds: u64,
    status: String,
    approver_id: Option<PrincipalId>,
    created_at_unix_seconds: u64,
}

impl ServiceApprovalSummary {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        approval_id: impl Into<String>,
        session_id: SessionId,
        execution_id: ExecutionId,
        gene_id: GeneId,
        request_digest: RequestDigest,
        request_summary: impl Into<String>,
        policy_version: u32,
        expires_at_unix_seconds: u64,
        status: impl Into<String>,
        approver_id: Option<PrincipalId>,
        created_at_unix_seconds: u64,
    ) -> Result<Self, ServiceContractError> {
        let approval_id = approval_id.into();
        validate_approval_id(&approval_id)?;
        let request_summary = request_summary.into();
        if request_summary.trim().is_empty() {
            return Err(ServiceContractError::InvalidApprovalSummary);
        }
        Ok(Self {
            approval_id,
            session_id,
            execution_id,
            gene_id,
            request_digest,
            request_summary,
            policy_version,
            expires_at_unix_seconds,
            status: status.into(),
            approver_id,
            created_at_unix_seconds,
        })
    }

    pub fn approval_id(&self) -> &str {
        &self.approval_id
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub fn execution_id(&self) -> &ExecutionId {
        &self.execution_id
    }

    pub fn gene_id(&self) -> &GeneId {
        &self.gene_id
    }

    pub fn request_digest(&self) -> &RequestDigest {
        &self.request_digest
    }

    pub fn request_summary(&self) -> &str {
        &self.request_summary
    }

    pub const fn policy_version(&self) -> u32 {
        self.policy_version
    }

    pub const fn expires_at_unix_seconds(&self) -> u64 {
        self.expires_at_unix_seconds
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn approver_id(&self) -> Option<&PrincipalId> {
        self.approver_id.as_ref()
    }

    pub const fn created_at_unix_seconds(&self) -> u64 {
        self.created_at_unix_seconds
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceRunResult {
    session_id: SessionId,
    execution_id: ExecutionId,
    selected_harness: Option<HarnessId>,
    selected_gene: Option<GeneId>,
    status: String,
    output: String,
    receipt_count: u64,
    event_count: u64,
    #[serde(default)]
    status_detail: Option<String>,
    #[serde(default)]
    approval: Option<ServiceApprovalSummary>,
}

impl ServiceRunResult {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_id: SessionId,
        execution_id: ExecutionId,
        selected_harness: Option<HarnessId>,
        selected_gene: Option<GeneId>,
        status: impl Into<String>,
        output: impl Into<String>,
        receipt_count: u64,
        event_count: u64,
    ) -> Self {
        Self {
            session_id,
            execution_id,
            selected_harness,
            selected_gene,
            status: status.into(),
            output: output.into(),
            receipt_count,
            event_count,
            status_detail: None,
            approval: None,
        }
    }

    pub fn with_status_detail(mut self, detail: impl Into<String>) -> Self {
        self.status_detail = Some(detail.into());
        self
    }

    pub fn with_approval(mut self, approval: ServiceApprovalSummary) -> Self {
        self.approval = Some(approval);
        self
    }

    pub const fn approval(&self) -> Option<&ServiceApprovalSummary> {
        self.approval.as_ref()
    }

    pub fn status_detail(&self) -> Option<&str> {
        self.status_detail.as_deref()
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub fn execution_id(&self) -> &ExecutionId {
        &self.execution_id
    }

    pub fn selected_harness(&self) -> Option<&HarnessId> {
        self.selected_harness.as_ref()
    }

    pub fn selected_gene(&self) -> Option<&GeneId> {
        self.selected_gene.as_ref()
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn output(&self) -> &str {
        &self.output
    }

    pub const fn receipt_count(&self) -> u64 {
        self.receipt_count
    }

    pub const fn event_count(&self) -> u64 {
        self.event_count
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceAgentRunResult {
    session_id: SessionId,
    execution_id: Option<ExecutionId>,
    selected_harness: Option<HarnessId>,
    selected_gene: Option<GeneId>,
    status: String,
    output: String,
    turns: u32,
    tool_calls: u32,
    provider_calls: u32,
    prompt_tokens: u64,
    completion_tokens: u64,
    run_count: u32,
    receipt_count: u64,
    event_count: u64,
    #[serde(default)]
    status_detail: Option<String>,
    #[serde(default)]
    approval: Option<ServiceApprovalSummary>,
}

impl ServiceAgentRunResult {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_id: SessionId,
        execution_id: Option<ExecutionId>,
        selected_harness: Option<HarnessId>,
        selected_gene: Option<GeneId>,
        status: impl Into<String>,
        output: impl Into<String>,
        turns: u32,
        tool_calls: u32,
        provider_calls: u32,
        prompt_tokens: u64,
        completion_tokens: u64,
        run_count: u32,
        receipt_count: u64,
        event_count: u64,
    ) -> Self {
        Self {
            session_id,
            execution_id,
            selected_harness,
            selected_gene,
            status: status.into(),
            output: output.into(),
            turns,
            tool_calls,
            provider_calls,
            prompt_tokens,
            completion_tokens,
            run_count,
            receipt_count,
            event_count,
            status_detail: None,
            approval: None,
        }
    }

    pub fn with_status_detail(mut self, detail: impl Into<String>) -> Self {
        self.status_detail = Some(detail.into());
        self
    }

    pub fn with_approval(mut self, approval: ServiceApprovalSummary) -> Self {
        self.approval = Some(approval);
        self
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }
    pub fn execution_id(&self) -> Option<&ExecutionId> {
        self.execution_id.as_ref()
    }
    pub fn selected_harness(&self) -> Option<&HarnessId> {
        self.selected_harness.as_ref()
    }
    pub fn selected_gene(&self) -> Option<&GeneId> {
        self.selected_gene.as_ref()
    }
    pub fn status(&self) -> &str {
        &self.status
    }
    pub fn output(&self) -> &str {
        &self.output
    }
    pub const fn turns(&self) -> u32 {
        self.turns
    }
    pub const fn tool_calls(&self) -> u32 {
        self.tool_calls
    }
    pub const fn provider_calls(&self) -> u32 {
        self.provider_calls
    }
    pub const fn prompt_tokens(&self) -> u64 {
        self.prompt_tokens
    }
    pub const fn completion_tokens(&self) -> u64 {
        self.completion_tokens
    }
    pub const fn run_count(&self) -> u32 {
        self.run_count
    }
    pub const fn receipt_count(&self) -> u64 {
        self.receipt_count
    }
    pub const fn event_count(&self) -> u64 {
        self.event_count
    }
    pub fn status_detail(&self) -> Option<&str> {
        self.status_detail.as_deref()
    }
    pub const fn approval(&self) -> Option<&ServiceApprovalSummary> {
        self.approval.as_ref()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServiceResponse {
    Health {
        protocol_version: u16,
        health: ServiceHealth,
    },
    Capabilities {
        protocol_version: u16,
        harnesses: Vec<ServiceHarnessSummary>,
    },
    Providers {
        protocol_version: u16,
        providers: Vec<ServiceProviderSummary>,
    },
    Engines {
        protocol_version: u16,
        engines: Vec<ServiceEngineSummary>,
    },
    Tools {
        protocol_version: u16,
        tools: Vec<ServiceToolSummary>,
    },
    SessionList {
        protocol_version: u16,
        sessions: Vec<ServiceSessionSummary>,
    },
    SessionInspect {
        protocol_version: u16,
        session: ServiceSessionDetail,
    },
    SessionEvents {
        protocol_version: u16,
        events: ServiceEventPage,
    },
    SessionMemory {
        protocol_version: u16,
        memory: ServiceMemoryPage,
    },
    ApprovalList {
        protocol_version: u16,
        approvals: Vec<ServiceApprovalSummary>,
    },
    ApprovalInspect {
        protocol_version: u16,
        approval: ServiceApprovalSummary,
    },
    ApprovalResolve {
        protocol_version: u16,
        approval: ServiceApprovalSummary,
    },
    Run {
        protocol_version: u16,
        run: ServiceRunResult,
    },
    AgentRun {
        protocol_version: u16,
        run: ServiceAgentRunResult,
    },
}

impl ServiceResponse {
    pub const fn health(health: ServiceHealth) -> Self {
        Self::Health {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            health,
        }
    }

    pub fn capabilities(harnesses: Vec<ServiceHarnessSummary>) -> Self {
        Self::Capabilities {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            harnesses,
        }
    }

    pub fn providers(providers: Vec<ServiceProviderSummary>) -> Self {
        Self::Providers {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            providers,
        }
    }

    pub fn engines(engines: Vec<ServiceEngineSummary>) -> Self {
        Self::Engines {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            engines,
        }
    }

    pub fn tools(tools: Vec<ServiceToolSummary>) -> Self {
        Self::Tools {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            tools,
        }
    }

    pub fn session_list(sessions: Vec<ServiceSessionSummary>) -> Self {
        Self::SessionList {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            sessions,
        }
    }

    pub const fn session_inspect(session: ServiceSessionDetail) -> Self {
        Self::SessionInspect {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            session,
        }
    }

    pub const fn session_events(events: ServiceEventPage) -> Self {
        Self::SessionEvents {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            events,
        }
    }

    pub const fn session_memory(memory: ServiceMemoryPage) -> Self {
        Self::SessionMemory {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            memory,
        }
    }

    pub fn approval_list(approvals: Vec<ServiceApprovalSummary>) -> Self {
        Self::ApprovalList {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            approvals,
        }
    }

    pub const fn approval_inspect(approval: ServiceApprovalSummary) -> Self {
        Self::ApprovalInspect {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            approval,
        }
    }

    pub const fn approval_resolve(approval: ServiceApprovalSummary) -> Self {
        Self::ApprovalResolve {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            approval,
        }
    }

    pub const fn run(run: ServiceRunResult) -> Self {
        Self::Run {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            run,
        }
    }

    pub const fn agent_run(run: ServiceAgentRunResult) -> Self {
        Self::AgentRun {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            run,
        }
    }

    pub const fn protocol_version(&self) -> u16 {
        match self {
            Self::Health {
                protocol_version, ..
            }
            | Self::Capabilities {
                protocol_version, ..
            }
            | Self::Providers {
                protocol_version, ..
            }
            | Self::Engines {
                protocol_version, ..
            }
            | Self::Tools {
                protocol_version, ..
            }
            | Self::SessionList {
                protocol_version, ..
            }
            | Self::SessionInspect {
                protocol_version, ..
            }
            | Self::SessionEvents {
                protocol_version, ..
            }
            | Self::SessionMemory {
                protocol_version, ..
            }
            | Self::ApprovalList {
                protocol_version, ..
            }
            | Self::ApprovalInspect {
                protocol_version, ..
            }
            | Self::ApprovalResolve {
                protocol_version, ..
            }
            | Self::Run {
                protocol_version, ..
            }
            | Self::AgentRun {
                protocol_version, ..
            } => *protocol_version,
        }
    }
}

fn validate_page_limit(limit: u16, maximum: u16) -> Result<(), ServiceContractError> {
    if limit == 0 || limit > maximum {
        return Err(ServiceContractError::InvalidPageLimit { limit, maximum });
    }
    Ok(())
}

fn validate_approval_id(approval_id: &str) -> Result<(), ServiceContractError> {
    if approval_id.trim().is_empty() || approval_id.len() > 256 {
        Err(ServiceContractError::InvalidApprovalIdentifier)
    } else {
        Ok(())
    }
}
