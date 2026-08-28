use crate::effect::{RequestError, Timestamp};
use crate::events::RuntimeEvent;
use crate::ids::{
    ArtifactId, ExecutionId, GeneId, HarnessId, IdError, PrincipalId, ProposalId, RequestDigest,
    SessionId, TenantId, WorkspaceId,
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
    InvalidEvolutionConfirmation,
    InvalidEvolutionReason,
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
            Self::InvalidEvolutionConfirmation => formatter
                .write_str("evolution confirmation must match the exact proposal identifier"),
            Self::InvalidEvolutionReason => {
                formatter.write_str("evolution rollback reason is invalid")
            }
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceEvolutionEvaluation {
    trajectory_score: u8,
    outcome_score: u8,
    holdout_passed: bool,
    policy_passed: bool,
    regression_passed: bool,
    evaluated_at_unix_seconds: u64,
    holdout_digest: Option<RequestDigest>,
}

impl ServiceEvolutionEvaluation {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        trajectory_score: u8,
        outcome_score: u8,
        holdout_passed: bool,
        policy_passed: bool,
        regression_passed: bool,
        evaluated_at: Timestamp,
        holdout_digest: Option<RequestDigest>,
    ) -> Self {
        Self {
            trajectory_score,
            outcome_score,
            holdout_passed,
            policy_passed,
            regression_passed,
            evaluated_at_unix_seconds: evaluated_at.as_unix_seconds(),
            holdout_digest,
        }
    }

    pub const fn trajectory_score(&self) -> u8 {
        self.trajectory_score
    }
    pub const fn outcome_score(&self) -> u8 {
        self.outcome_score
    }
    pub const fn holdout_passed(&self) -> bool {
        self.holdout_passed
    }
    pub const fn policy_passed(&self) -> bool {
        self.policy_passed
    }
    pub const fn regression_passed(&self) -> bool {
        self.regression_passed
    }
    pub const fn evaluated_at_unix_seconds(&self) -> u64 {
        self.evaluated_at_unix_seconds
    }
    pub fn holdout_digest(&self) -> Option<&RequestDigest> {
        self.holdout_digest.as_ref()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceEvolutionApproval {
    approver_id: PrincipalId,
    policy_version: u32,
    approved_at_unix_seconds: u64,
    signer_id: PrincipalId,
    signature_present: bool,
}

impl ServiceEvolutionApproval {
    pub fn new(
        approver_id: PrincipalId,
        policy_version: u32,
        approved_at: Timestamp,
        signer_id: PrincipalId,
        signature_present: bool,
    ) -> Self {
        Self {
            approver_id,
            policy_version,
            approved_at_unix_seconds: approved_at.as_unix_seconds(),
            signer_id,
            signature_present,
        }
    }

    pub fn approver_id(&self) -> &PrincipalId {
        &self.approver_id
    }
    pub const fn policy_version(&self) -> u32 {
        self.policy_version
    }
    pub const fn approved_at_unix_seconds(&self) -> u64 {
        self.approved_at_unix_seconds
    }
    pub fn signer_id(&self) -> &PrincipalId {
        &self.signer_id
    }
    pub const fn signature_present(&self) -> bool {
        self.signature_present
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceEvolutionCanary {
    passed: bool,
    failure_count: u32,
    note: String,
    evaluated_at_unix_seconds: u64,
}

impl ServiceEvolutionCanary {
    pub fn new(
        passed: bool,
        failure_count: u32,
        note: impl Into<String>,
        evaluated_at: Timestamp,
    ) -> Self {
        Self {
            passed,
            failure_count,
            note: note.into(),
            evaluated_at_unix_seconds: evaluated_at.as_unix_seconds(),
        }
    }

    pub const fn passed(&self) -> bool {
        self.passed
    }
    pub const fn failure_count(&self) -> u32 {
        self.failure_count
    }
    pub fn note(&self) -> &str {
        &self.note
    }
    pub const fn evaluated_at_unix_seconds(&self) -> u64 {
        self.evaluated_at_unix_seconds
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceEvolutionCandidate {
    kind: String,
    target_id: String,
    provider_id: String,
    generated_at_unix_seconds: Option<u64>,
    base_bytes: u64,
    candidate_bytes: u64,
    changed_units: u64,
    added_units: u64,
    removed_units: u64,
    unit: String,
}

impl ServiceEvolutionCandidate {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kind: impl Into<String>,
        target_id: impl Into<String>,
        provider_id: impl Into<String>,
        generated_at: Option<Timestamp>,
        base_bytes: u64,
        candidate_bytes: u64,
        changed_units: u64,
        added_units: u64,
        removed_units: u64,
        unit: impl Into<String>,
    ) -> Self {
        Self {
            kind: kind.into(),
            target_id: target_id.into(),
            provider_id: provider_id.into(),
            generated_at_unix_seconds: generated_at.map(Timestamp::as_unix_seconds),
            base_bytes,
            candidate_bytes,
            changed_units,
            added_units,
            removed_units,
            unit: unit.into(),
        }
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }
    pub fn target_id(&self) -> &str {
        &self.target_id
    }
    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }
    pub const fn generated_at_unix_seconds(&self) -> Option<u64> {
        self.generated_at_unix_seconds
    }
    pub const fn base_bytes(&self) -> u64 {
        self.base_bytes
    }
    pub const fn candidate_bytes(&self) -> u64 {
        self.candidate_bytes
    }
    pub const fn changed_units(&self) -> u64 {
        self.changed_units
    }
    pub const fn added_units(&self) -> u64 {
        self.added_units
    }
    pub const fn removed_units(&self) -> u64 {
        self.removed_units
    }
    pub fn unit(&self) -> &str {
        &self.unit
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceEvolutionSummary {
    proposal_id: ProposalId,
    source: String,
    base_artifact: ArtifactId,
    candidate_artifact: ArtifactId,
    evidence_digest: RequestDigest,
    expected_outcome: String,
    created_at_unix_seconds: u64,
    state: String,
    evaluation: Option<ServiceEvolutionEvaluation>,
    approval: Option<ServiceEvolutionApproval>,
    canary: Option<ServiceEvolutionCanary>,
    #[serde(default)]
    candidate: Option<ServiceEvolutionCandidate>,
}

impl ServiceEvolutionSummary {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        proposal_id: ProposalId,
        source: impl Into<String>,
        base_artifact: ArtifactId,
        candidate_artifact: ArtifactId,
        evidence_digest: RequestDigest,
        expected_outcome: impl Into<String>,
        created_at: Timestamp,
        state: impl Into<String>,
        evaluation: Option<ServiceEvolutionEvaluation>,
        approval: Option<ServiceEvolutionApproval>,
        canary: Option<ServiceEvolutionCanary>,
    ) -> Self {
        Self {
            proposal_id,
            source: source.into(),
            base_artifact,
            candidate_artifact,
            evidence_digest,
            expected_outcome: expected_outcome.into(),
            created_at_unix_seconds: created_at.as_unix_seconds(),
            state: state.into(),
            evaluation,
            approval,
            canary,
            candidate: None,
        }
    }

    pub fn with_candidate(mut self, candidate: ServiceEvolutionCandidate) -> Self {
        self.candidate = Some(candidate);
        self
    }

    pub fn proposal_id(&self) -> &ProposalId {
        &self.proposal_id
    }
    pub fn source(&self) -> &str {
        &self.source
    }
    pub fn base_artifact(&self) -> &ArtifactId {
        &self.base_artifact
    }
    pub fn candidate_artifact(&self) -> &ArtifactId {
        &self.candidate_artifact
    }
    pub fn evidence_digest(&self) -> &RequestDigest {
        &self.evidence_digest
    }
    pub fn expected_outcome(&self) -> &str {
        &self.expected_outcome
    }
    pub const fn created_at_unix_seconds(&self) -> u64 {
        self.created_at_unix_seconds
    }
    pub fn state(&self) -> &str {
        &self.state
    }
    pub const fn evaluation(&self) -> Option<&ServiceEvolutionEvaluation> {
        self.evaluation.as_ref()
    }
    pub const fn approval(&self) -> Option<&ServiceEvolutionApproval> {
        self.approval.as_ref()
    }
    pub const fn canary(&self) -> Option<&ServiceEvolutionCanary> {
        self.canary.as_ref()
    }
    pub const fn candidate(&self) -> Option<&ServiceEvolutionCandidate> {
        self.candidate.as_ref()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceArtifactActivation {
    proposal_id: ProposalId,
    base_artifact: ArtifactId,
    candidate_artifact: ArtifactId,
    activated_at_unix_seconds: u64,
}

impl ServiceArtifactActivation {
    pub fn new(
        proposal_id: ProposalId,
        base_artifact: ArtifactId,
        candidate_artifact: ArtifactId,
        activated_at: Timestamp,
    ) -> Self {
        Self {
            proposal_id,
            base_artifact,
            candidate_artifact,
            activated_at_unix_seconds: activated_at.as_unix_seconds(),
        }
    }

    pub fn proposal_id(&self) -> &ProposalId {
        &self.proposal_id
    }
    pub fn base_artifact(&self) -> &ArtifactId {
        &self.base_artifact
    }
    pub fn candidate_artifact(&self) -> &ArtifactId {
        &self.candidate_artifact
    }
    pub const fn activated_at_unix_seconds(&self) -> u64 {
        self.activated_at_unix_seconds
    }
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
    EvolutionList {
        protocol_version: u16,
        limit: u16,
    },
    EvolutionInspect {
        protocol_version: u16,
        proposal_id: ProposalId,
    },
    EvolutionActivations {
        protocol_version: u16,
        limit: u16,
    },
    EvolutionActivate {
        protocol_version: u16,
        proposal_id: ProposalId,
        confirmation: String,
    },
    EvolutionRollback {
        protocol_version: u16,
        proposal_id: ProposalId,
        confirmation: String,
        reason: String,
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

    pub fn evolution_list(limit: u16) -> Result<Self, ServiceContractError> {
        validate_page_limit(limit, MAX_SERVICE_SESSION_PAGE)?;
        Ok(Self::EvolutionList {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            limit,
        })
    }

    pub fn evolution_inspect(proposal_id: impl Into<String>) -> Result<Self, ServiceContractError> {
        Ok(Self::EvolutionInspect {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            proposal_id: ProposalId::new(proposal_id)?,
        })
    }

    pub fn evolution_activations(limit: u16) -> Result<Self, ServiceContractError> {
        validate_page_limit(limit, MAX_SERVICE_SESSION_PAGE)?;
        Ok(Self::EvolutionActivations {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            limit,
        })
    }

    pub fn evolution_activate(
        proposal_id: impl Into<String>,
        confirmation: impl Into<String>,
    ) -> Result<Self, ServiceContractError> {
        let request = Self::EvolutionActivate {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            proposal_id: ProposalId::new(proposal_id)?,
            confirmation: confirmation.into(),
        };
        request.validate()?;
        Ok(request)
    }

    pub fn evolution_rollback(
        proposal_id: impl Into<String>,
        confirmation: impl Into<String>,
        reason: impl Into<String>,
    ) -> Result<Self, ServiceContractError> {
        let request = Self::EvolutionRollback {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            proposal_id: ProposalId::new(proposal_id)?,
            confirmation: confirmation.into(),
            reason: reason.into(),
        };
        request.validate()?;
        Ok(request)
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
            | Self::EvolutionList {
                protocol_version, ..
            }
            | Self::EvolutionInspect {
                protocol_version, ..
            }
            | Self::EvolutionActivations {
                protocol_version, ..
            }
            | Self::EvolutionActivate {
                protocol_version, ..
            }
            | Self::EvolutionRollback {
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
            Self::EvolutionList { limit, .. } => {
                validate_page_limit(*limit, MAX_SERVICE_SESSION_PAGE)
            }
            Self::EvolutionInspect { proposal_id, .. } => {
                ProposalId::new(proposal_id.as_str())?;
                Ok(())
            }
            Self::EvolutionActivations { limit, .. } => {
                validate_page_limit(*limit, MAX_SERVICE_SESSION_PAGE)
            }
            Self::EvolutionActivate {
                proposal_id,
                confirmation,
                ..
            } => validate_evolution_confirmation(proposal_id, confirmation, None),
            Self::EvolutionRollback {
                proposal_id,
                confirmation,
                reason,
                ..
            } => validate_evolution_confirmation(proposal_id, confirmation, Some(reason)),
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
    #[serde(default)]
    cached_prompt_tokens: u64,
    #[serde(default)]
    cache_write_prompt_tokens: u64,
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
        cached_prompt_tokens: u64,
        cache_write_prompt_tokens: u64,
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
            cached_prompt_tokens,
            cache_write_prompt_tokens,
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
    pub const fn cached_prompt_tokens(&self) -> u64 {
        self.cached_prompt_tokens
    }
    pub const fn cache_write_prompt_tokens(&self) -> u64 {
        self.cache_write_prompt_tokens
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
    EvolutionList {
        protocol_version: u16,
        proposals: Vec<ServiceEvolutionSummary>,
    },
    EvolutionInspect {
        protocol_version: u16,
        proposal: ServiceEvolutionSummary,
    },
    EvolutionActivations {
        protocol_version: u16,
        activations: Vec<ServiceArtifactActivation>,
    },
    EvolutionMutation {
        protocol_version: u16,
        operation: String,
        proposal_id: ProposalId,
        state: String,
        artifact: ArtifactId,
        occurred_at_unix_seconds: u64,
        backup_directory: String,
        reconciled_bindings: usize,
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

    pub fn evolution_list(proposals: Vec<ServiceEvolutionSummary>) -> Self {
        Self::EvolutionList {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            proposals,
        }
    }

    pub const fn evolution_inspect(proposal: ServiceEvolutionSummary) -> Self {
        Self::EvolutionInspect {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            proposal,
        }
    }

    pub fn evolution_activations(activations: Vec<ServiceArtifactActivation>) -> Self {
        Self::EvolutionActivations {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            activations,
        }
    }

    pub fn evolution_mutation(
        operation: impl Into<String>,
        proposal_id: ProposalId,
        state: impl Into<String>,
        artifact: ArtifactId,
        occurred_at: Timestamp,
        backup_directory: impl Into<String>,
        reconciled_bindings: usize,
    ) -> Self {
        Self::EvolutionMutation {
            protocol_version: LOCAL_SERVICE_PROTOCOL_VERSION,
            operation: operation.into(),
            proposal_id,
            state: state.into(),
            artifact,
            occurred_at_unix_seconds: occurred_at.as_unix_seconds(),
            backup_directory: backup_directory.into(),
            reconciled_bindings,
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
            | Self::EvolutionList {
                protocol_version, ..
            }
            | Self::EvolutionInspect {
                protocol_version, ..
            }
            | Self::EvolutionActivations {
                protocol_version, ..
            }
            | Self::EvolutionMutation {
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

fn validate_evolution_confirmation(
    proposal_id: &ProposalId,
    confirmation: &str,
    reason: Option<&String>,
) -> Result<(), ServiceContractError> {
    ProposalId::new(proposal_id.as_str())?;
    if confirmation != proposal_id.as_str() {
        return Err(ServiceContractError::InvalidEvolutionConfirmation);
    }
    if reason.is_some_and(|value| value.trim().is_empty() || value.len() > 4096) {
        return Err(ServiceContractError::InvalidEvolutionReason);
    }
    Ok(())
}
