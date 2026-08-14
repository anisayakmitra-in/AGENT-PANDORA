use crate::effect::SecretReference;
use crate::ids::{
    EventId, ExecutionId, GeneId, HarnessId, ReceiptId, RequestDigest, SessionId, TenantId,
    WorkspaceId,
};
use serde::{Deserialize, Serialize};

const EVENT_PROTOCOL_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    SessionStarted,
    EffectRequested,
    EffectCompleted,
    ProviderCall,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EventContext {
    tenant_id: TenantId,
    workspace_id: WorkspaceId,
    session_id: Option<SessionId>,
    execution_id: Option<ExecutionId>,
    harness_id: Option<HarnessId>,
    gene_id: Option<GeneId>,
    policy_version: Option<u32>,
    receipt_id: Option<ReceiptId>,
}

impl EventContext {
    pub fn new(tenant_id: TenantId, workspace_id: WorkspaceId) -> Self {
        Self {
            tenant_id,
            workspace_id,
            session_id: None,
            execution_id: None,
            harness_id: None,
            gene_id: None,
            policy_version: None,
            receipt_id: None,
        }
    }

    pub fn with_session(mut self, session_id: SessionId) -> Self {
        self.session_id = Some(session_id);
        self
    }

    pub fn with_execution(mut self, execution_id: ExecutionId) -> Self {
        self.execution_id = Some(execution_id);
        self
    }

    pub fn with_harness(mut self, harness_id: HarnessId) -> Self {
        self.harness_id = Some(harness_id);
        self
    }

    pub fn with_gene(mut self, gene_id: GeneId) -> Self {
        self.gene_id = Some(gene_id);
        self
    }

    pub fn with_policy_version(mut self, policy_version: u32) -> Self {
        self.policy_version = Some(policy_version);
        self
    }

    pub fn with_receipt(mut self, receipt_id: ReceiptId) -> Self {
        self.receipt_id = Some(receipt_id);
        self
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventPayload {
    Empty,
    Effect {
        capability: String,
        request_digest: RequestDigest,
    },
    ProviderCall {
        provider: String,
        credential: SecretReference,
        request_digest: RequestDigest,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RuntimeEvent {
    protocol_version: u16,
    event_id: EventId,
    event_type: EventType,
    context: EventContext,
    payload: EventPayload,
}

impl RuntimeEvent {
    pub fn new(
        event_id: EventId,
        event_type: EventType,
        context: EventContext,
        payload: EventPayload,
    ) -> Self {
        Self {
            protocol_version: EVENT_PROTOCOL_VERSION,
            event_id,
            event_type,
            context,
            payload,
        }
    }

    pub fn event_id(&self) -> &EventId {
        &self.event_id
    }

    pub fn event_type(&self) -> EventType {
        self.event_type
    }

    pub fn context(&self) -> &EventContext {
        &self.context
    }

    pub fn payload(&self) -> &EventPayload {
        &self.payload
    }
}
