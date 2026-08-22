use crate::effect::{RequestError, Timestamp};
use crate::events::RuntimeEvent;
use crate::ids::{
    ExecutionId, GeneId, HarnessId, IdError, PrincipalId, SessionId, TenantId, WorkspaceId,
};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const LOCAL_SERVICE_PROTOCOL_VERSION: u16 = 1;
pub const MAX_SERVICE_EVENT_PAGE: u16 = 256;
pub const MAX_SERVICE_SESSION_PAGE: u16 = 256;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServiceContractError {
    InvalidTask(RequestError),
    InvalidIdentifier(IdError),
    InvalidPageLimit { limit: u16, maximum: u16 },
}

impl fmt::Display for ServiceContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTask(error) => error.fmt(formatter),
            Self::InvalidIdentifier(error) => error.fmt(formatter),
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
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServiceRequest {
    Health {
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
    Run {
        protocol_version: u16,
        request: ServiceRunRequest,
    },
}

impl ServiceRequest {
    pub const fn health() -> Self {
        Self::Health {
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

    pub const fn run(request: ServiceRunRequest) -> Self {
        Self::Run {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            request,
        }
    }

    pub const fn protocol_version(&self) -> u16 {
        match self {
            Self::Health { protocol_version }
            | Self::SessionList {
                protocol_version, ..
            }
            | Self::SessionInspect {
                protocol_version, ..
            }
            | Self::SessionEvents {
                protocol_version, ..
            }
            | Self::Run {
                protocol_version, ..
            } => *protocol_version,
        }
    }

    pub const fn is_supported_protocol(&self) -> bool {
        self.protocol_version() == LOCAL_SERVICE_PROTOCOL_VERSION
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
pub struct ServiceRunResult {
    execution_id: ExecutionId,
    selected_harness: Option<HarnessId>,
    selected_gene: Option<GeneId>,
    status: String,
    output: String,
    receipt_count: u64,
    event_count: u64,
}

impl ServiceRunResult {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        execution_id: ExecutionId,
        selected_harness: Option<HarnessId>,
        selected_gene: Option<GeneId>,
        status: impl Into<String>,
        output: impl Into<String>,
        receipt_count: u64,
        event_count: u64,
    ) -> Self {
        Self {
            execution_id,
            selected_harness,
            selected_gene,
            status: status.into(),
            output: output.into(),
            receipt_count,
            event_count,
        }
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
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServiceResponse {
    Health {
        protocol_version: u16,
        health: ServiceHealth,
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
    Run {
        protocol_version: u16,
        run: ServiceRunResult,
    },
}

impl ServiceResponse {
    pub const fn health(health: ServiceHealth) -> Self {
        Self::Health {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            health,
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

    pub const fn run(run: ServiceRunResult) -> Self {
        Self::Run {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            run,
        }
    }

    pub const fn protocol_version(&self) -> u16 {
        match self {
            Self::Health {
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
            | Self::Run {
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
