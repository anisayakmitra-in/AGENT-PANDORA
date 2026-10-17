use crate::context::ContextClassification;
use crate::effect::Timestamp;
use crate::ids::{IdError, MemoryId, SessionId, TenantId, WorkspaceId};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, Ord, Hash, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryIntent {
    Repo,
    Procedure,
    Coding,
    User,
}

impl MemoryIntent {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Repo => "repo",
            Self::Procedure => "procedure",
            Self::Coding => "coding",
            Self::User => "user",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, Hash, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryTier {
    L0,
    L1,
    L2,
    L3,
    L4,
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
            Self::L3 => "l3",
            Self::L4 => "l4",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, Hash, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
    Sop,
    SessionArchive,
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
            Self::Sop => "sop",
            Self::SessionArchive => "session_archive",
        }
    }

    pub const fn can_be_l1(self) -> bool {
        !matches!(self, Self::Trace | Self::Sop | Self::SessionArchive)
    }

    pub const fn can_be_l2(self) -> bool {
        matches!(
            self,
            Self::Lesson | Self::Lineage | Self::PolicyDecision | Self::Replacement
        )
    }

    pub const fn can_be_l3(self) -> bool {
        matches!(self, Self::Sop)
    }

    pub const fn can_be_l4(self) -> bool {
        matches!(self, Self::SessionArchive)
    }

    pub const fn default_intent(self) -> MemoryIntent {
        match self {
            Self::Trace => MemoryIntent::Procedure,
            Self::ExecutionEvidence | Self::Decision | Self::Failure | Self::Replacement => {
                MemoryIntent::Coding
            }
            Self::Benchmark | Self::Lesson | Self::Sop => MemoryIntent::Procedure,
            Self::Lineage | Self::SessionArchive => MemoryIntent::Repo,
            Self::PolicyDecision => MemoryIntent::User,
        }
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
    DuplicateStep(String),
    UnknownDependency(String),
    SelfDependency(String),
    DependencyCycle,
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
            Self::DuplicateStep(step) => write!(formatter, "duplicate SOP step '{step}'"),
            Self::UnknownDependency(dep) => {
                write!(formatter, "unknown SOP step dependency '{dep}'")
            }
            Self::SelfDependency(step) => {
                write!(formatter, "SOP step '{step}' cannot depend on itself")
            }
            Self::DependencyCycle => formatter.write_str("SOP steps contain a dependency cycle"),
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
    intent: Option<MemoryIntent>,
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
            intent: candidate.intent,
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
    pub fn new_l3_sop(
        id: impl Into<String>,
        scope: MemoryScope,
        summary: impl Into<String>,
        classification: ContextClassification,
        created_at: Timestamp,
        provenance: impl Into<String>,
    ) -> Result<Self, MemoryContractError> {
        Self::new(
            id,
            MemoryTier::L3,
            MemoryKind::Sop,
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
    pub fn new_l4_archive(
        id: impl Into<String>,
        scope: MemoryScope,
        summary: impl Into<String>,
        classification: ContextClassification,
        created_at: Timestamp,
        provenance: impl Into<String>,
    ) -> Result<Self, MemoryContractError> {
        Self::new(
            id,
            MemoryTier::L4,
            MemoryKind::SessionArchive,
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
            || tier == MemoryTier::L3 && !kind.can_be_l3()
            || tier == MemoryTier::L4 && !kind.can_be_l4()
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
            intent: None,
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

    pub fn intent(&self) -> MemoryIntent {
        self.intent.unwrap_or_else(|| self.kind.default_intent())
    }

    pub const fn explicit_intent(&self) -> Option<MemoryIntent> {
        self.intent
    }

    pub fn with_intent(mut self, intent: MemoryIntent) -> Self {
        self.intent = Some(intent);
        self
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

fn validate_multiline_text(
    field: &'static str,
    value: String,
) -> Result<String, MemoryContractError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(MemoryContractError::EmptyField(field));
    }
    if value.len() > 65_536 {
        return Err(MemoryContractError::FieldTooLong(field));
    }
    if value
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(MemoryContractError::ControlCharacter(field));
    }
    Ok(trimmed.to_owned())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SopStep {
    pub id: String,
    pub name: String,
    pub command: String,
    pub depends_on: Vec<String>,
}

impl SopStep {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        command: impl Into<String>,
        depends_on: Vec<String>,
    ) -> Result<Self, MemoryContractError> {
        let id = validate_text("step id", id.into())?;
        let name = validate_text("step name", name.into())?;
        let command = validate_multiline_text("step command", command.into())?;
        let mut seen = std::collections::BTreeSet::new();
        for dep in &depends_on {
            validate_text("step dependency", dep.clone())?;
            if !seen.insert(dep) {
                return Err(MemoryContractError::DuplicateStep(dep.clone()));
            }
        }
        Ok(Self {
            id,
            name,
            command,
            depends_on,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SopDefinition {
    pub id: String,
    pub name: String,
    pub trigger_intent: MemoryIntent,
    pub steps: Vec<SopStep>,
    pub verification_command: Option<String>,
}

impl SopDefinition {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        trigger_intent: MemoryIntent,
        steps: Vec<SopStep>,
        verification_command: Option<String>,
    ) -> Result<Self, MemoryContractError> {
        let id = validate_text("SOP id", id.into())?;
        let name = validate_text("SOP name", name.into())?;
        let verification_command = verification_command
            .map(|cmd| validate_multiline_text("verification command", cmd))
            .transpose()?;

        if steps.is_empty() {
            return Err(MemoryContractError::EmptyField("steps"));
        }

        let mut step_ids = std::collections::HashSet::new();
        for step in &steps {
            if !step_ids.insert(step.id.clone()) {
                return Err(MemoryContractError::DuplicateStep(step.id.clone()));
            }
        }

        for step in &steps {
            for dep in &step.depends_on {
                if !step_ids.contains(dep) {
                    return Err(MemoryContractError::UnknownDependency(dep.clone()));
                }
                if dep == &step.id {
                    return Err(MemoryContractError::SelfDependency(step.id.clone()));
                }
            }
        }

        if has_sop_cycle(&steps) {
            return Err(MemoryContractError::DependencyCycle);
        }

        Ok(Self {
            id,
            name,
            trigger_intent,
            steps,
            verification_command,
        })
    }

    pub fn ready_steps(&self, completed: &[String]) -> Vec<SopStep> {
        let completed_set: std::collections::HashSet<&str> =
            completed.iter().map(String::as_str).collect();
        self.steps
            .iter()
            .filter(|step| {
                !completed_set.contains(step.id.as_str())
                    && step
                        .depends_on
                        .iter()
                        .all(|dep| completed_set.contains(dep.as_str()))
            })
            .cloned()
            .collect()
    }
}

fn has_sop_cycle(steps: &[SopStep]) -> bool {
    let mut in_degrees: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let mut dependents: std::collections::HashMap<&str, Vec<&str>> =
        std::collections::HashMap::new();

    for step in steps {
        in_degrees.entry(&step.id).or_insert(0);
        for dep in &step.depends_on {
            dependents.entry(dep.as_str()).or_default().push(&step.id);
            *in_degrees.entry(&step.id).or_insert(0) += 1;
        }
    }

    let mut queue: std::collections::VecDeque<&str> = in_degrees
        .iter()
        .filter(|(_, deg)| **deg == 0)
        .map(|(&id, _)| id)
        .collect();

    let mut visited = 0;
    while let Some(node) = queue.pop_front() {
        visited += 1;
        if let Some(children) = dependents.get(node) {
            for &child in children {
                if let Some(deg) = in_degrees.get_mut(child) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push_back(child);
                    }
                }
            }
        }
    }

    visited != steps.len()
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StoredSop {
    pub sop: SopDefinition,
    pub tenant_id: TenantId,
    pub workspace_id: WorkspaceId,
    pub success_count: u32,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl StoredSop {
    pub fn new(
        sop: SopDefinition,
        tenant_id: TenantId,
        workspace_id: WorkspaceId,
        success_count: u32,
        created_at: Timestamp,
        updated_at: Timestamp,
    ) -> Self {
        Self {
            sop,
            tenant_id,
            workspace_id,
            success_count,
            created_at,
            updated_at,
        }
    }

    pub fn sop_id(&self) -> &str {
        &self.sop.id
    }

    pub fn name(&self) -> &str {
        &self.sop.name
    }

    pub const fn intent(&self) -> MemoryIntent {
        self.sop.trigger_intent
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionArchiveRecord {
    pub archive_id: String,
    pub tenant_id: TenantId,
    pub workspace_id: WorkspaceId,
    pub session_id: SessionId,
    pub summary: String,
    pub diff_stats: String,
    pub receipt_digest: String,
    pub created_at: Timestamp,
}

impl SessionArchiveRecord {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        archive_id: impl Into<String>,
        tenant_id: TenantId,
        workspace_id: WorkspaceId,
        session_id: SessionId,
        summary: impl Into<String>,
        diff_stats: impl Into<String>,
        receipt_digest: impl Into<String>,
        created_at: Timestamp,
    ) -> Result<Self, MemoryContractError> {
        let archive_id = validate_text("archive ID", archive_id.into())?;
        let summary = validate_text("summary", summary.into())?;
        let diff_stats = validate_multiline_text("diff stats", diff_stats.into())?;
        let receipt_digest = validate_text("receipt digest", receipt_digest.into())?;
        Ok(Self {
            archive_id,
            tenant_id,
            workspace_id,
            session_id,
            summary,
            diff_stats,
            receipt_digest,
            created_at,
        })
    }
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

    #[test]
    fn memory_intent_defaults_and_explicit_override_work() {
        let sc = scope("project-a", "session-a", "provider-a");
        let record = MemoryRecord::new_l1(
            "mem-1",
            MemoryKind::ExecutionEvidence,
            sc.clone(),
            "completed patch verification",
            ContextClassification::Internal,
            Timestamp::from_unix_seconds(100),
            "test",
        )
        .unwrap();

        // ExecutionEvidence defaults to Coding intent
        assert_eq!(record.intent(), MemoryIntent::Coding);
        assert_eq!(record.explicit_intent(), None);

        // Explicit override works
        let repo_record = record.with_intent(MemoryIntent::Repo);
        assert_eq!(repo_record.intent(), MemoryIntent::Repo);
        assert_eq!(repo_record.explicit_intent(), Some(MemoryIntent::Repo));

        // Test Promotion carries intent on L2-eligible kind (Lesson)
        let lesson = MemoryRecord::new_l1(
            "lesson-1",
            MemoryKind::Lesson,
            sc,
            "learned to check edge verifier",
            ContextClassification::Internal,
            Timestamp::from_unix_seconds(150),
            "test",
        )
        .unwrap()
        .with_intent(MemoryIntent::Procedure);

        let approval = MemoryApproval::new("app-1", "alice").unwrap();
        let promoted =
            MemoryRecord::promote_l2(lesson, approval, Timestamp::from_unix_seconds(200)).unwrap();
        assert_eq!(promoted.intent(), MemoryIntent::Procedure);
        assert_eq!(promoted.explicit_intent(), Some(MemoryIntent::Procedure));
    }

    #[test]
    fn memory_tier_and_kind_serde_and_rules() {
        for (tier, expected_str) in [
            (MemoryTier::L0, "l0"),
            (MemoryTier::L1, "l1"),
            (MemoryTier::L2, "l2"),
            (MemoryTier::L3, "l3"),
            (MemoryTier::L4, "l4"),
        ] {
            assert_eq!(tier.as_str(), expected_str);
            let json = serde_json::to_string(&tier).unwrap();
            assert_eq!(json, format!("\"{expected_str}\""));
            let deserialized: MemoryTier = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, tier);
        }

        assert_eq!(MemoryKind::Sop.as_str(), "sop");
        assert_eq!(MemoryKind::SessionArchive.as_str(), "session_archive");
        assert!(MemoryKind::Sop.can_be_l3());
        assert!(!MemoryKind::Sop.can_be_l1());
        assert!(!MemoryKind::Sop.can_be_l2());
        assert!(!MemoryKind::Sop.can_be_l4());

        assert!(MemoryKind::SessionArchive.can_be_l4());
        assert!(!MemoryKind::SessionArchive.can_be_l1());
        assert!(!MemoryKind::SessionArchive.can_be_l2());
        assert!(!MemoryKind::SessionArchive.can_be_l3());

        assert_eq!(MemoryKind::Sop.default_intent(), MemoryIntent::Procedure);
        assert_eq!(
            MemoryKind::SessionArchive.default_intent(),
            MemoryIntent::Repo
        );
    }

    #[test]
    fn memory_record_l3_and_l4_constructors_and_tier_invariants() {
        let sc = scope("project-a", "session-a", "provider-a");
        let sop_record = MemoryRecord::new_l3_sop(
            "sop-rec-1",
            sc.clone(),
            "workflow for release build",
            ContextClassification::Internal,
            Timestamp::from_unix_seconds(300),
            "test-provenance",
        )
        .unwrap();

        assert_eq!(sop_record.tier(), MemoryTier::L3);
        assert_eq!(sop_record.kind(), MemoryKind::Sop);
        assert_eq!(sop_record.intent(), MemoryIntent::Procedure);

        let archive_record = MemoryRecord::new_l4_archive(
            "archive-rec-1",
            sc,
            "session archive summary",
            ContextClassification::Internal,
            Timestamp::from_unix_seconds(400),
            "test-provenance",
        )
        .unwrap();

        assert_eq!(archive_record.tier(), MemoryTier::L4);
        assert_eq!(archive_record.kind(), MemoryKind::SessionArchive);
        assert_eq!(archive_record.intent(), MemoryIntent::Repo);
    }

    #[test]
    fn sop_dag_validation_and_execution_order() {
        let step1 = SopStep::new(
            "step-build",
            "Build binary",
            "cargo build --release",
            vec![],
        )
        .unwrap();
        let step2 = SopStep::new(
            "step-test",
            "Run tests",
            "cargo test",
            vec!["step-build".into()],
        )
        .unwrap();
        let step3 = SopStep::new(
            "step-package",
            "Package artifact",
            "tar -czf bin.tar.gz bin",
            vec!["step-test".into()],
        )
        .unwrap();

        let sop = SopDefinition::new(
            "sop-release",
            "Release Build SOP",
            MemoryIntent::Procedure,
            vec![step1.clone(), step2.clone(), step3.clone()],
            Some("cargo check".into()),
        )
        .unwrap();

        // Check DAG execution order via ready_steps
        let initial_ready = sop.ready_steps(&[]);
        assert_eq!(initial_ready.len(), 1);
        assert_eq!(initial_ready[0].id, "step-build");

        let after_build = sop.ready_steps(&["step-build".into()]);
        assert_eq!(after_build.len(), 1);
        assert_eq!(after_build[0].id, "step-test");

        let after_test = sop.ready_steps(&["step-build".into(), "step-test".into()]);
        assert_eq!(after_test.len(), 1);
        assert_eq!(after_test[0].id, "step-package");

        let all_done = sop.ready_steps(&[
            "step-build".into(),
            "step-test".into(),
            "step-package".into(),
        ]);
        assert!(all_done.is_empty());

        // Test serde round-trip
        let json = serde_json::to_string(&sop).unwrap();
        let deserialized: SopDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, sop);

        // Negative: duplicate step IDs
        let dup_step = SopStep::new("step-build", "Build again", "cargo build", vec![]).unwrap();
        assert_eq!(
            SopDefinition::new(
                "sop-dup",
                "Dup",
                MemoryIntent::Procedure,
                vec![step1.clone(), dup_step],
                None
            ),
            Err(MemoryContractError::DuplicateStep("step-build".into()))
        );

        // Negative: unknown dependency
        let bad_dep_step =
            SopStep::new("step-orphan", "Orphan", "run", vec!["non-existent".into()]).unwrap();
        assert_eq!(
            SopDefinition::new(
                "sop-bad-dep",
                "BadDep",
                MemoryIntent::Procedure,
                vec![bad_dep_step],
                None
            ),
            Err(MemoryContractError::UnknownDependency(
                "non-existent".into()
            ))
        );

        // Negative: self dependency
        let self_dep_step =
            SopStep::new("step-loop", "Self", "run", vec!["step-loop".into()]).unwrap();
        assert_eq!(
            SopDefinition::new(
                "sop-self",
                "SelfDep",
                MemoryIntent::Procedure,
                vec![self_dep_step],
                None
            ),
            Err(MemoryContractError::SelfDependency("step-loop".into()))
        );

        // Negative: cycle detection
        let cycle_a = SopStep::new("step-a", "A", "run", vec!["step-b".into()]).unwrap();
        let cycle_b = SopStep::new("step-b", "B", "run", vec!["step-a".into()]).unwrap();
        assert_eq!(
            SopDefinition::new(
                "sop-cycle",
                "Cycle",
                MemoryIntent::Procedure,
                vec![cycle_a, cycle_b],
                None
            ),
            Err(MemoryContractError::DependencyCycle)
        );
    }

    #[test]
    fn session_archive_record_contract_and_serde() {
        let archive = SessionArchiveRecord::new(
            "arch-123",
            TenantId::new("tenant-a").unwrap(),
            WorkspaceId::new("workspace-a").unwrap(),
            SessionId::new("session-1").unwrap(),
            "Refactored memory subsystem to 5 layers",
            "+150 -20 in 3 files",
            "sha256:abcdef123456",
            Timestamp::from_unix_seconds(500),
        )
        .unwrap();

        assert_eq!(archive.archive_id, "arch-123");
        assert_eq!(archive.summary, "Refactored memory subsystem to 5 layers");
        assert_eq!(archive.diff_stats, "+150 -20 in 3 files");

        let json = serde_json::to_string(&archive).unwrap();
        let deserialized: SessionArchiveRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, archive);
    }
}
