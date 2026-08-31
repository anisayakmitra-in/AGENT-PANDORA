use crate::context::ContextClassification;
use crate::effect::Timestamp;
use crate::ids::{IdError, MemoryId, SessionId, TenantId, WorkspaceId};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum MemoryTier {
    L0,
    L1,
    L2,
}

pub const MAX_MEMORY_SYNTHESIS_EVIDENCE: usize = 16;
pub const MEMORY_CONSOLIDATION_POLICY_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryConsolidationBoundary {
    CrossSession,
    CrossProject,
}

impl MemoryConsolidationBoundary {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CrossSession => "cross_session",
            Self::CrossProject => "cross_project",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryConflictRule {
    Reject,
    KeepTarget,
}

impl MemoryConflictRule {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reject => "reject",
            Self::KeepTarget => "keep_target",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryOrigin {
    Explicit,
    Synthesized,
}

impl MemoryOrigin {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::Synthesized => "synthesized",
        }
    }
}

impl MemoryTier {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::L0 => "l0",
            Self::L1 => "l1",
            Self::L2 => "l2",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum MemoryKind {
    Trace,
    ExecutionEvidence,
    Decision,
    Failure,
    Benchmark,
    Lesson,
    Lineage,
    PolicyDecision,
    Replacement,
}

impl MemoryKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::ExecutionEvidence => "execution_evidence",
            Self::Decision => "decision",
            Self::Failure => "failure",
            Self::Benchmark => "benchmark",
            Self::Lesson => "lesson",
            Self::Lineage => "lineage",
            Self::PolicyDecision => "policy_decision",
            Self::Replacement => "replacement",
        }
    }

    const fn can_be_l1(self) -> bool {
        !matches!(self, Self::Trace | Self::PolicyDecision | Self::Replacement)
    }

    const fn can_be_l2(self) -> bool {
        matches!(
            self,
            Self::Lesson | Self::Lineage | Self::PolicyDecision | Self::Replacement
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MemoryContractError {
    InvalidId(IdError),
    EmptyField(&'static str),
    FieldTooLong(&'static str),
    ControlCharacter(&'static str),
    SecretContent,
    MissingEvidence,
    TooManyEvidenceItems,
    DuplicateEvidence,
    SameConsolidationSession,
    CrossTenantConsolidation,
    CrossProviderConsolidation,
    ConsolidationBoundaryMismatch,
    InvalidTier { kind: MemoryKind, tier: MemoryTier },
}

impl fmt::Display for MemoryContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId(error) => error.fmt(formatter),
            Self::EmptyField(field) => write!(formatter, "{field} cannot be empty"),
            Self::FieldTooLong(field) => write!(formatter, "{field} is too long"),
            Self::ControlCharacter(field) => {
                write!(formatter, "{field} contains a control character")
            }
            Self::SecretContent => formatter.write_str("secret content cannot be persisted"),
            Self::MissingEvidence => formatter.write_str("synthesized memory requires evidence"),
            Self::TooManyEvidenceItems => write!(
                formatter,
                "synthesized memory exceeds the {} evidence-item limit",
                MAX_MEMORY_SYNTHESIS_EVIDENCE
            ),
            Self::DuplicateEvidence => {
                formatter.write_str("synthesized memory contains duplicate evidence")
            }
            Self::SameConsolidationSession => {
                formatter.write_str("memory consolidation requires distinct sessions")
            }
            Self::CrossTenantConsolidation => {
                formatter.write_str("memory consolidation cannot cross tenants")
            }
            Self::CrossProviderConsolidation => {
                formatter.write_str("memory consolidation cannot cross providers")
            }
            Self::ConsolidationBoundaryMismatch => {
                formatter.write_str("memory consolidation boundary does not match its workspaces")
            }
            Self::InvalidTier { kind, tier } => {
                write!(
                    formatter,
                    "{} cannot be stored in {}",
                    kind.as_str(),
                    tier.as_str()
                )
            }
        }
    }
}

impl std::error::Error for MemoryContractError {}

impl From<IdError> for MemoryContractError {
    fn from(error: IdError) -> Self {
        Self::InvalidId(error)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct MemoryScope {
    tenant_id: TenantId,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    provider: String,
}

impl MemoryScope {
    pub fn new(
        tenant_id: TenantId,
        workspace_id: WorkspaceId,
        session_id: SessionId,
        provider: impl Into<String>,
    ) -> Result<Self, MemoryContractError> {
        Ok(Self {
            tenant_id,
            workspace_id,
            session_id,
            provider: validate_text("provider", provider.into())?,
        })
    }

    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    pub fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryConsolidationPolicy {
    policy_version: u32,
    boundary: MemoryConsolidationBoundary,
    conflict_rule: MemoryConflictRule,
    source_scope: MemoryScope,
    target_scope: MemoryScope,
}

impl MemoryConsolidationPolicy {
    pub fn cross_session(
        source_scope: MemoryScope,
        target_scope: MemoryScope,
        conflict_rule: MemoryConflictRule,
    ) -> Result<Self, MemoryContractError> {
        Self::new(
            MemoryConsolidationBoundary::CrossSession,
            source_scope,
            target_scope,
            conflict_rule,
        )
    }

    pub fn cross_project(
        source_scope: MemoryScope,
        target_scope: MemoryScope,
        conflict_rule: MemoryConflictRule,
    ) -> Result<Self, MemoryContractError> {
        Self::new(
            MemoryConsolidationBoundary::CrossProject,
            source_scope,
            target_scope,
            conflict_rule,
        )
    }

    fn new(
        boundary: MemoryConsolidationBoundary,
        source_scope: MemoryScope,
        target_scope: MemoryScope,
        conflict_rule: MemoryConflictRule,
    ) -> Result<Self, MemoryContractError> {
        if source_scope.tenant_id() != target_scope.tenant_id() {
            return Err(MemoryContractError::CrossTenantConsolidation);
        }
        if source_scope.provider() != target_scope.provider() {
            return Err(MemoryContractError::CrossProviderConsolidation);
        }
        if source_scope.session_id() == target_scope.session_id() {
            return Err(MemoryContractError::SameConsolidationSession);
        }
        let crosses_workspace = source_scope.workspace_id() != target_scope.workspace_id();
        if crosses_workspace != (boundary == MemoryConsolidationBoundary::CrossProject) {
            return Err(MemoryContractError::ConsolidationBoundaryMismatch);
        }
        Ok(Self {
            policy_version: MEMORY_CONSOLIDATION_POLICY_VERSION,
            boundary,
            conflict_rule,
            source_scope,
            target_scope,
        })
    }

    pub const fn policy_version(&self) -> u32 {
        self.policy_version
    }

    pub const fn boundary(&self) -> MemoryConsolidationBoundary {
        self.boundary
    }

    pub const fn conflict_rule(&self) -> MemoryConflictRule {
        self.conflict_rule
    }

    pub fn source_scope(&self) -> &MemoryScope {
        &self.source_scope
    }

    pub fn target_scope(&self) -> &MemoryScope {
        &self.target_scope
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryApproval {
    approval_id: String,
    approver: String,
}

impl MemoryApproval {
    pub fn new(
        approval_id: impl Into<String>,
        approver: impl Into<String>,
    ) -> Result<Self, MemoryContractError> {
        Ok(Self {
            approval_id: validate_text("approval ID", approval_id.into())?,
            approver: validate_text("approver", approver.into())?,
        })
    }

    pub fn approval_id(&self) -> &str {
        &self.approval_id
    }

    pub fn approver(&self) -> &str {
        &self.approver
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryRecord {
    id: MemoryId,
    tier: MemoryTier,
    kind: MemoryKind,
    scope: MemoryScope,
    summary: String,
    classification: ContextClassification,
    created_at: Timestamp,
    expires_at: Option<Timestamp>,
    provenance: String,
    approval: Option<MemoryApproval>,
    origin: MemoryOrigin,
    evidence_ids: Vec<MemoryId>,
}

impl MemoryRecord {
    #[allow(clippy::too_many_arguments)]
    pub fn new_l0(
        id: impl Into<String>,
        scope: MemoryScope,
        summary: impl Into<String>,
        classification: ContextClassification,
        created_at: Timestamp,
        expires_at: Option<Timestamp>,
        provenance: impl Into<String>,
    ) -> Result<Self, MemoryContractError> {
        Self::new(
            id,
            MemoryTier::L0,
            MemoryKind::Trace,
            scope,
            summary,
            classification,
            created_at,
            expires_at,
            provenance,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_l1(
        id: impl Into<String>,
        kind: MemoryKind,
        scope: MemoryScope,
        summary: impl Into<String>,
        classification: ContextClassification,
        created_at: Timestamp,
        provenance: impl Into<String>,
    ) -> Result<Self, MemoryContractError> {
        Self::new(
            id,
            MemoryTier::L1,
            kind,
            scope,
            summary,
            classification,
            created_at,
            None,
            provenance,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_synthesized_l1(
        id: impl Into<String>,
        kind: MemoryKind,
        scope: MemoryScope,
        summary: impl Into<String>,
        classification: ContextClassification,
        created_at: Timestamp,
        provenance: impl Into<String>,
        evidence_ids: Vec<MemoryId>,
    ) -> Result<Self, MemoryContractError> {
        Self::new_with_origin(
            id,
            MemoryTier::L1,
            kind,
            scope,
            summary,
            classification,
            created_at,
            None,
            provenance,
            None,
            MemoryOrigin::Synthesized,
            evidence_ids,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_l1_with_origin(
        id: impl Into<String>,
        kind: MemoryKind,
        scope: MemoryScope,
        summary: impl Into<String>,
        classification: ContextClassification,
        created_at: Timestamp,
        provenance: impl Into<String>,
        origin: MemoryOrigin,
        evidence_ids: Vec<MemoryId>,
    ) -> Result<Self, MemoryContractError> {
        Self::new_with_origin(
            id,
            MemoryTier::L1,
            kind,
            scope,
            summary,
            classification,
            created_at,
            None,
            provenance,
            None,
            origin,
            evidence_ids,
        )
    }

    pub fn promote_l2(
        candidate: Self,
        approval: MemoryApproval,
        promoted_at: Timestamp,
    ) -> Result<Self, MemoryContractError> {
        if candidate.tier != MemoryTier::L1 || !candidate.kind.can_be_l2() {
            return Err(MemoryContractError::InvalidTier {
                kind: candidate.kind,
                tier: MemoryTier::L2,
            });
        }
        Ok(Self {
            id: candidate.id,
            tier: MemoryTier::L2,
            kind: candidate.kind,
            scope: candidate.scope,
            summary: candidate.summary,
            classification: candidate.classification,
            created_at: promoted_at,
            expires_at: None,
            provenance: candidate.provenance,
            approval: Some(approval),
            origin: candidate.origin,
            evidence_ids: candidate.evidence_ids,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn new(
        id: impl Into<String>,
        tier: MemoryTier,
        kind: MemoryKind,
        scope: MemoryScope,
        summary: impl Into<String>,
        classification: ContextClassification,
        created_at: Timestamp,
        expires_at: Option<Timestamp>,
        provenance: impl Into<String>,
        approval: Option<MemoryApproval>,
    ) -> Result<Self, MemoryContractError> {
        Self::new_with_origin(
            id,
            tier,
            kind,
            scope,
            summary,
            classification,
            created_at,
            expires_at,
            provenance,
            approval,
            MemoryOrigin::Explicit,
            Vec::new(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_with_origin(
        id: impl Into<String>,
        tier: MemoryTier,
        kind: MemoryKind,
        scope: MemoryScope,
        summary: impl Into<String>,
        classification: ContextClassification,
        created_at: Timestamp,
        expires_at: Option<Timestamp>,
        provenance: impl Into<String>,
        approval: Option<MemoryApproval>,
        origin: MemoryOrigin,
        evidence_ids: Vec<MemoryId>,
    ) -> Result<Self, MemoryContractError> {
        let id = MemoryId::new(id.into())?;
        let summary = validate_summary(summary.into(), classification)?;
        let provenance = validate_text("provenance", provenance.into())?;
        if tier == MemoryTier::L0 && kind != MemoryKind::Trace
            || tier == MemoryTier::L1 && !kind.can_be_l1()
            || tier == MemoryTier::L2 && !kind.can_be_l2()
        {
            return Err(MemoryContractError::InvalidTier { kind, tier });
        }
        if evidence_ids.len() > MAX_MEMORY_SYNTHESIS_EVIDENCE {
            return Err(MemoryContractError::TooManyEvidenceItems);
        }
        let mut unique_evidence = evidence_ids.clone();
        unique_evidence.sort();
        unique_evidence.dedup();
        if unique_evidence.len() != evidence_ids.len() {
            return Err(MemoryContractError::DuplicateEvidence);
        }
        if origin == MemoryOrigin::Synthesized && evidence_ids.is_empty() {
            return Err(MemoryContractError::MissingEvidence);
        }
        if origin == MemoryOrigin::Explicit && !evidence_ids.is_empty() {
            return Err(MemoryContractError::InvalidTier { kind, tier });
        }
        Ok(Self {
            id,
            tier,
            kind,
            scope,
            summary,
            classification,
            created_at,
            expires_at,
            provenance,
            approval,
            origin,
            evidence_ids,
        })
    }

    pub fn id(&self) -> &MemoryId {
        &self.id
    }

    pub const fn tier(&self) -> MemoryTier {
        self.tier
    }

    pub const fn kind(&self) -> MemoryKind {
        self.kind
    }

    pub fn scope(&self) -> &MemoryScope {
        &self.scope
    }

    pub fn summary(&self) -> &str {
        &self.summary
    }

    pub const fn classification(&self) -> ContextClassification {
        self.classification
    }

    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }

    pub const fn expires_at(&self) -> Option<Timestamp> {
        self.expires_at
    }

    pub fn provenance(&self) -> &str {
        &self.provenance
    }

    pub fn approval(&self) -> Option<&MemoryApproval> {
        self.approval.as_ref()
    }

    pub const fn origin(&self) -> MemoryOrigin {
        self.origin
    }

    pub fn evidence_ids(&self) -> &[MemoryId] {
        &self.evidence_ids
    }

    pub const fn is_expired(&self, now: Timestamp) -> bool {
        match self.expires_at {
            Some(expires_at) => expires_at.as_unix_seconds() <= now.as_unix_seconds(),
            None => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryAuditAction {
    Added,
    Promoted,
    Revoked,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryAuditEntry {
    memory_id: MemoryId,
    tier: MemoryTier,
    action: MemoryAuditAction,
    scope: MemoryScope,
    at: Timestamp,
    approval_id: Option<String>,
}

impl MemoryAuditEntry {
    pub fn new(
        memory_id: MemoryId,
        tier: MemoryTier,
        action: MemoryAuditAction,
        scope: MemoryScope,
        at: Timestamp,
        approval_id: Option<String>,
    ) -> Self {
        Self {
            memory_id,
            tier,
            action,
            scope,
            at,
            approval_id,
        }
    }

    pub fn memory_id(&self) -> &MemoryId {
        &self.memory_id
    }

    pub const fn tier(&self) -> MemoryTier {
        self.tier
    }

    pub const fn action(&self) -> MemoryAuditAction {
        self.action
    }

    pub fn scope(&self) -> &MemoryScope {
        &self.scope
    }

    pub const fn at(&self) -> Timestamp {
        self.at
    }

    pub fn approval_id(&self) -> Option<&str> {
        self.approval_id.as_deref()
    }
}

fn validate_summary(
    summary: String,
    classification: ContextClassification,
) -> Result<String, MemoryContractError> {
    if classification == ContextClassification::Secret {
        return Err(MemoryContractError::SecretContent);
    }
    validate_text("summary", summary)
}

fn validate_text(field: &'static str, value: String) -> Result<String, MemoryContractError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(MemoryContractError::EmptyField(field));
    }
    if value.len() > 16_384 {
        return Err(MemoryContractError::FieldTooLong(field));
    }
    if value.chars().any(char::is_control) {
        return Err(MemoryContractError::ControlCharacter(field));
    }
    Ok(trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(workspace: &str, session: &str, provider: &str) -> MemoryScope {
        MemoryScope::new(
            TenantId::new("tenant-a").unwrap(),
            WorkspaceId::new(workspace).unwrap(),
            SessionId::new(session).unwrap(),
            provider,
        )
        .unwrap()
    }

    #[test]
    fn consolidation_policy_requires_an_exact_workspace_boundary() {
        let cross_project = MemoryConsolidationPolicy::cross_project(
            scope("project-a", "session-a", "provider-a"),
            scope("project-b", "session-b", "provider-a"),
            MemoryConflictRule::Reject,
        )
        .unwrap();
        assert_eq!(
            cross_project.policy_version(),
            MEMORY_CONSOLIDATION_POLICY_VERSION
        );
        assert_eq!(
            cross_project.boundary(),
            MemoryConsolidationBoundary::CrossProject
        );
        assert_eq!(cross_project.conflict_rule(), MemoryConflictRule::Reject);

        assert_eq!(
            MemoryConsolidationPolicy::cross_session(
                scope("project-a", "session-a", "provider-a"),
                scope("project-b", "session-b", "provider-a"),
                MemoryConflictRule::KeepTarget,
            ),
            Err(MemoryContractError::ConsolidationBoundaryMismatch)
        );
        assert_eq!(
            MemoryConsolidationPolicy::cross_project(
                scope("project-a", "session-a", "provider-a"),
                scope("project-a", "session-b", "provider-a"),
                MemoryConflictRule::KeepTarget,
            ),
            Err(MemoryContractError::ConsolidationBoundaryMismatch)
        );
    }

    #[test]
    fn consolidation_policy_denies_tenant_provider_and_session_crossings() {
        let mut other_tenant = scope("project-b", "session-b", "provider-a");
        other_tenant.tenant_id = TenantId::new("tenant-b").unwrap();
        assert_eq!(
            MemoryConsolidationPolicy::cross_project(
                scope("project-a", "session-a", "provider-a"),
                other_tenant,
                MemoryConflictRule::Reject,
            ),
            Err(MemoryContractError::CrossTenantConsolidation)
        );
        assert_eq!(
            MemoryConsolidationPolicy::cross_project(
                scope("project-a", "session-a", "provider-a"),
                scope("project-b", "session-b", "provider-b"),
                MemoryConflictRule::Reject,
            ),
            Err(MemoryContractError::CrossProviderConsolidation)
        );
        assert_eq!(
            MemoryConsolidationPolicy::cross_session(
                scope("project-a", "session-a", "provider-a"),
                scope("project-a", "session-a", "provider-a"),
                MemoryConflictRule::Reject,
            ),
            Err(MemoryContractError::SameConsolidationSession)
        );
    }
}
