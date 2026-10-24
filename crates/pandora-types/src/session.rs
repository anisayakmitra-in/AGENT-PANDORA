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

#[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
pub struct SessionPruneReport {
    pub older_than_seconds: u64,
    pub sessions_examined: usize,
    pub sessions_pruned: usize,
    pub events_reclaimed: usize,
    pub dry_run: bool,
    pub pruned_session_ids: Vec<String>,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq, serde::Serialize)]
pub struct StorageStats {
    pub total_pages: u64,
    pub free_pages: u64,
    pub page_size_bytes: u64,
    pub total_size_bytes: u64,
    pub reclaimable_bytes: u64,
    pub reclaimable_percent: f64,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq, serde::Serialize)]
pub struct VacuumReport {
    pub pre_size_bytes: u64,
    pub post_size_bytes: u64,
    pub reclaimed_bytes: u64,
    pub pre_reclaimable_pct: f64,
    pub post_reclaimable_pct: f64,
    pub vacuum_executed: bool,
    pub reason: String,
}
