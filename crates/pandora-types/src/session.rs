use crate::effect::{RequestError, Timestamp};
use crate::ids::{GeneId, HarnessId, PrincipalId, SessionId, TenantId, WorkspaceId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Session {
    id: SessionId,
    principal_id: PrincipalId,
    tenant_id: TenantId,
    workspace_id: WorkspaceId,
    created_at: Timestamp,
}

impl Session {
    pub fn new(
        id: SessionId,
        principal_id: PrincipalId,
        tenant_id: TenantId,
        workspace_id: WorkspaceId,
        created_at: Timestamp,
    ) -> Self {
        Self {
            id,
            principal_id,
            tenant_id,
            workspace_id,
            created_at,
        }
    }

    pub fn id(&self) -> &SessionId {
        &self.id
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

    pub fn created_at(&self) -> Timestamp {
        self.created_at
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskIntent {
    summary: String,
    requested_harness: Option<HarnessId>,
    requested_gene: Option<GeneId>,
}

impl TaskIntent {
    pub fn new(summary: impl Into<String>) -> Result<Self, RequestError> {
        let summary = summary.into();
        if summary.trim().is_empty() {
            return Err(RequestError::EmptyField("task summary"));
        }
        Ok(Self {
            summary,
            requested_harness: None,
            requested_gene: None,
        })
    }

    pub fn with_harness(mut self, harness_id: HarnessId) -> Self {
        self.requested_harness = Some(harness_id);
        self
    }

    pub fn with_gene(mut self, gene_id: GeneId) -> Self {
        self.requested_gene = Some(gene_id);
        self
    }

    pub fn summary(&self) -> &str {
        &self.summary
    }

    pub fn requested_harness(&self) -> Option<&HarnessId> {
        self.requested_harness.as_ref()
    }

    pub fn requested_gene(&self) -> Option<&GeneId> {
        self.requested_gene.as_ref()
    }
}
