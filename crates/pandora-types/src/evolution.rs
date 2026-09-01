use crate::effect::Timestamp;
use crate::ids::{
    ArtifactId, ExecutionId, IdError, MemoryId, PrincipalId, ProposalId, RequestDigest,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

const MAX_TEXT_BYTES: usize = 4096;
const MAX_FAILURE_SIGNALS: usize = 16;
pub const MAX_EVOLUTION_MEMORY_EVIDENCE_IDS: usize = 16;
pub const MAX_EVOLUTION_ROLLOUT_TRANSITIONS: usize = 128;
pub const MAX_EVOLUTION_ROLLOUT_SCORECARDS: usize = 32;
pub const MAX_EVOLUTION_ROLLOUT_RETRIES: u8 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvolutionMode {
    Production,
    Research,
}

impl EvolutionMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Research => "research",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum EvolutionSource {
    Reflexion,
    Gepa,
    Population,
}

/// A non-executable material class that may be proposed only through Pandora's
/// research evolution path. The class is part of candidate provenance; it does
/// not grant a capability or select an executor.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchArtifactKind {
    Prompt,
    Skill,
    Workflow,
    WasmGene,
}

impl ResearchArtifactKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Prompt => "prompt",
            Self::Skill => "skill",
            Self::Workflow => "workflow",
            Self::WasmGene => "wasm_gene",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "prompt" => Some(Self::Prompt),
            "skill" => Some(Self::Skill),
            "workflow" => Some(Self::Workflow),
            "wasm_gene" => Some(Self::WasmGene),
            _ => None,
        }
    }
}

impl EvolutionSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reflexion => "reflexion",
            Self::Gepa => "gepa",
            Self::Population => "population",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EvolutionPolicy {
    policy_version: u32,
    mode: EvolutionMode,
    require_holdout: bool,
    require_signature: bool,
    require_canary: bool,
}

impl EvolutionPolicy {
    pub const fn production(policy_version: u32) -> Self {
        Self {
            policy_version,
            mode: EvolutionMode::Production,
            require_holdout: true,
            require_signature: true,
            require_canary: true,
        }
    }

    pub const fn research(policy_version: u32) -> Self {
        Self {
            policy_version,
            mode: EvolutionMode::Research,
            require_holdout: true,
            require_signature: true,
            require_canary: true,
        }
    }

    pub const fn policy_version(self) -> u32 {
        self.policy_version
    }

    pub const fn mode(self) -> EvolutionMode {
        self.mode
    }

    pub const fn requires_holdout(self) -> bool {
        self.require_holdout
    }

    pub const fn requires_signature(self) -> bool {
        self.require_signature
    }

    pub const fn requires_canary(self) -> bool {
        self.require_canary
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EvolutionContractError {
    InvalidId(IdError),
    EmptyField(&'static str),
    FieldTooLong(&'static str),
    ControlCharacter(&'static str),
    TooManyFailureSignals,
    TooManyMemoryEvidenceIds,
    InvalidCommit,
    InvalidDigest,
    InvalidStageLimits,
    DuplicateStageLimit,
    MissingStageLimit,
    TooManyRolloutTransitions,
    TooManyRolloutScorecards,
    TransitionIdReused,
    InvalidRolloutTransition,
    EvaluatorMismatch,
    HumanApprovalRequired,
    SelfApproval,
    ApprovalExpired,
    ApprovalMismatch,
    ApprovalAlreadyPending,
    RetryLimitExceeded,
    SameArtifact,
}

impl fmt::Display for EvolutionContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId(error) => error.fmt(formatter),
            Self::EmptyField(field) => write!(formatter, "{field} cannot be empty"),
            Self::FieldTooLong(field) => write!(formatter, "{field} is too long"),
            Self::ControlCharacter(field) => {
                write!(formatter, "{field} contains a control character")
            }
            Self::TooManyFailureSignals => formatter.write_str("too many failure signals"),
            Self::TooManyMemoryEvidenceIds => {
                formatter.write_str("too many evolution memory evidence IDs")
            }
            Self::InvalidCommit => formatter.write_str(
                "rollout commit must be an exact 40- or 64-character hexadecimal commit",
            ),
            Self::InvalidDigest => formatter
                .write_str("rollout digest must use sha256:<64 lowercase hexadecimal characters>"),
            Self::InvalidStageLimits => formatter.write_str("rollout stage limits are invalid"),
            Self::DuplicateStageLimit => {
                formatter.write_str("rollout stage limits contain a duplicate stage")
            }
            Self::MissingStageLimit => formatter.write_str(
                "rollout stage limits must cover canary, limited, expanded, and complete",
            ),
            Self::TooManyRolloutTransitions => {
                formatter.write_str("too many rollout transition records")
            }
            Self::TooManyRolloutScorecards => formatter.write_str("too many rollout scorecards"),
            Self::TransitionIdReused => {
                formatter.write_str("rollout transition ID was reused for a different transition")
            }
            Self::InvalidRolloutTransition => {
                formatter.write_str("rollout transition is not valid from the current state")
            }
            Self::EvaluatorMismatch => {
                formatter.write_str("rollout scorecard actor must match the recorded evaluator")
            }
            Self::HumanApprovalRequired => {
                formatter.write_str("rollout promotion requires human approval")
            }
            Self::SelfApproval => {
                formatter.write_str("the scorecard evaluator cannot approve its own promotion")
            }
            Self::ApprovalExpired => {
                formatter.write_str("rollout promotion approval is not currently valid")
            }
            Self::ApprovalMismatch => formatter.write_str(
                "rollout promotion approval does not match the exact rollout binding and scorecard",
            ),
            Self::ApprovalAlreadyPending => {
                formatter.write_str("a rollout promotion approval is already pending")
            }
            Self::RetryLimitExceeded => formatter.write_str("rollout retry limit was exceeded"),
            Self::SameArtifact => formatter.write_str("base and candidate artifacts must differ"),
        }
    }
}

impl std::error::Error for EvolutionContractError {}

impl From<IdError> for EvolutionContractError {
    fn from(error: IdError) -> Self {
        Self::InvalidId(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReflexionArtifact {
    execution_id: ExecutionId,
    summary: String,
    failure_signals: Vec<String>,
    lesson: String,
    created_at: Timestamp,
}

impl ReflexionArtifact {
    pub fn new(
        execution_id: ExecutionId,
        summary: impl Into<String>,
        failure_signals: Vec<String>,
        lesson: impl Into<String>,
        created_at: Timestamp,
    ) -> Result<Self, EvolutionContractError> {
        if failure_signals.len() > MAX_FAILURE_SIGNALS {
            return Err(EvolutionContractError::TooManyFailureSignals);
        }
        let failure_signals = failure_signals
            .into_iter()
            .map(|signal| validate_text("failure signal", signal, 512))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            execution_id,
            summary: validate_text("reflection summary", summary.into(), MAX_TEXT_BYTES)?,
            failure_signals,
            lesson: validate_text("reflection lesson", lesson.into(), MAX_TEXT_BYTES)?,
            created_at,
        })
    }

    pub fn execution_id(&self) -> &ExecutionId {
        &self.execution_id
    }

    pub fn summary(&self) -> &str {
        &self.summary
    }

    pub fn failure_signals(&self) -> &[String] {
        &self.failure_signals
    }

    pub fn lesson(&self) -> &str {
        &self.lesson
    }

    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MutationProposal {
    proposal_id: ProposalId,
    source: EvolutionSource,
    base_artifact: ArtifactId,
    candidate_artifact: ArtifactId,
    evidence_digest: RequestDigest,
    #[serde(default)]
    memory_evidence_ids: Vec<MemoryId>,
    expected_outcome: String,
    created_at: Timestamp,
}

impl MutationProposal {
    pub fn new(
        proposal_id: impl Into<String>,
        source: EvolutionSource,
        base_artifact: ArtifactId,
        candidate_artifact: ArtifactId,
        evidence_digest: RequestDigest,
        expected_outcome: impl Into<String>,
        created_at: Timestamp,
    ) -> Result<Self, EvolutionContractError> {
        if base_artifact == candidate_artifact {
            return Err(EvolutionContractError::SameArtifact);
        }
        Ok(Self {
            proposal_id: ProposalId::new(proposal_id)?,
            source,
            base_artifact,
            candidate_artifact,
            evidence_digest,
            memory_evidence_ids: Vec::new(),
            expected_outcome: validate_text(
                "expected outcome",
                expected_outcome.into(),
                MAX_TEXT_BYTES,
            )?,
            created_at,
        })
    }

    pub fn proposal_id(&self) -> &ProposalId {
        &self.proposal_id
    }

    pub const fn source(&self) -> EvolutionSource {
        self.source
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

    pub fn with_memory_evidence_ids(
        mut self,
        mut memory_evidence_ids: Vec<MemoryId>,
    ) -> Result<Self, EvolutionContractError> {
        if memory_evidence_ids.len() > MAX_EVOLUTION_MEMORY_EVIDENCE_IDS {
            return Err(EvolutionContractError::TooManyMemoryEvidenceIds);
        }
        for memory_id in &memory_evidence_ids {
            MemoryId::new(memory_id.as_str())?;
        }
        memory_evidence_ids.sort();
        memory_evidence_ids.dedup();
        self.memory_evidence_ids = memory_evidence_ids;
        Ok(self)
    }

    pub fn memory_evidence_ids(&self) -> &[MemoryId] {
        &self.memory_evidence_ids
    }

    pub fn expected_outcome(&self) -> &str {
        &self.expected_outcome
    }

    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HoldoutEvaluation {
    proposal_id: ProposalId,
    trajectory_score: u8,
    outcome_score: u8,
    holdout_passed: bool,
    policy_passed: bool,
    regression_passed: bool,
    evaluated_at: Timestamp,
    #[serde(default)]
    holdout_digest: Option<RequestDigest>,
}

impl HoldoutEvaluation {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        proposal_id: ProposalId,
        trajectory_score: u8,
        outcome_score: u8,
        holdout_passed: bool,
        policy_passed: bool,
        regression_passed: bool,
        evaluated_at: Timestamp,
    ) -> Self {
        Self {
            proposal_id,
            trajectory_score,
            outcome_score,
            holdout_passed,
            policy_passed,
            regression_passed,
            evaluated_at,
            holdout_digest: None,
        }
    }

    pub fn with_holdout_digest(
        mut self,
        digest: impl Into<String>,
    ) -> Result<Self, EvolutionContractError> {
        self.holdout_digest = Some(RequestDigest::new(digest.into())?);
        Ok(self)
    }

    pub fn proposal_id(&self) -> &ProposalId {
        &self.proposal_id
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

    pub const fn passed(&self, policy: EvolutionPolicy) -> bool {
        (!policy.requires_holdout() || self.holdout_passed)
            && self.policy_passed
            && self.regression_passed
    }

    pub const fn evaluated_at(&self) -> Timestamp {
        self.evaluated_at
    }

    pub fn holdout_digest(&self) -> Option<&RequestDigest> {
        self.holdout_digest.as_ref()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ArtifactSignature {
    artifact_id: ArtifactId,
    signer: PrincipalId,
    signature: String,
}

impl ArtifactSignature {
    pub fn new(
        artifact_id: ArtifactId,
        signer: PrincipalId,
        signature: impl Into<String>,
    ) -> Result<Self, EvolutionContractError> {
        Ok(Self {
            artifact_id,
            signer,
            signature: validate_text("artifact signature", signature.into(), MAX_TEXT_BYTES)?,
        })
    }

    pub fn artifact_id(&self) -> &ArtifactId {
        &self.artifact_id
    }

    pub fn signer(&self) -> &PrincipalId {
        &self.signer
    }

    pub fn signature(&self) -> &str {
        &self.signature
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ParliamentApproval {
    proposal_id: ProposalId,
    approver: PrincipalId,
    policy_version: u32,
    approved_at: Timestamp,
}

impl ParliamentApproval {
    pub const fn new(
        proposal_id: ProposalId,
        approver: PrincipalId,
        policy_version: u32,
        approved_at: Timestamp,
    ) -> Self {
        Self {
            proposal_id,
            approver,
            policy_version,
            approved_at,
        }
    }

    pub fn proposal_id(&self) -> &ProposalId {
        &self.proposal_id
    }

    pub fn approver(&self) -> &PrincipalId {
        &self.approver
    }

    pub const fn policy_version(&self) -> u32 {
        self.policy_version
    }

    pub const fn approved_at(&self) -> Timestamp {
        self.approved_at
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CanaryResult {
    proposal_id: ProposalId,
    passed: bool,
    failure_count: u32,
    note: String,
    evaluated_at: Timestamp,
}

impl CanaryResult {
    pub fn new(
        proposal_id: ProposalId,
        passed: bool,
        failure_count: u32,
        note: impl Into<String>,
        evaluated_at: Timestamp,
    ) -> Result<Self, EvolutionContractError> {
        Ok(Self {
            proposal_id,
            passed,
            failure_count,
            note: validate_text("canary note", note.into(), MAX_TEXT_BYTES)?,
            evaluated_at,
        })
    }

    pub fn proposal_id(&self) -> &ProposalId {
        &self.proposal_id
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

    pub const fn evaluated_at(&self) -> Timestamp {
        self.evaluated_at
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum EvolutionState {
    Proposed,
    Evaluated,
    Approved,
    Staged,
    CanaryPassed,
    CanaryFailed,
    Active,
    RolledBack,
}

impl EvolutionState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Evaluated => "evaluated",
            Self::Approved => "approved",
            Self::Staged => "staged",
            Self::CanaryPassed => "canary_passed",
            Self::CanaryFailed => "canary_failed",
            Self::Active => "active",
            Self::RolledBack => "rolled_back",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplacementReceipt {
    proposal_id: ProposalId,
    base_artifact: ArtifactId,
    candidate_artifact: ArtifactId,
    activated_at: Timestamp,
}

impl ReplacementReceipt {
    pub const fn new(
        proposal_id: ProposalId,
        base_artifact: ArtifactId,
        candidate_artifact: ArtifactId,
        activated_at: Timestamp,
    ) -> Self {
        Self {
            proposal_id,
            base_artifact,
            candidate_artifact,
            activated_at,
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

    pub const fn activated_at(&self) -> Timestamp {
        self.activated_at
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RollbackReceipt {
    proposal_id: ProposalId,
    restored_artifact: ArtifactId,
    rolled_back_at: Timestamp,
    reason: String,
}

impl RollbackReceipt {
    pub fn new(
        proposal_id: ProposalId,
        restored_artifact: ArtifactId,
        rolled_back_at: Timestamp,
        reason: impl Into<String>,
    ) -> Result<Self, EvolutionContractError> {
        Ok(Self {
            proposal_id,
            restored_artifact,
            rolled_back_at,
            reason: validate_text("rollback reason", reason.into(), MAX_TEXT_BYTES)?,
        })
    }

    pub fn proposal_id(&self) -> &ProposalId {
        &self.proposal_id
    }

    pub fn restored_artifact(&self) -> &ArtifactId {
        &self.restored_artifact
    }

    pub const fn rolled_back_at(&self) -> Timestamp {
        self.rolled_back_at
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }
}

fn validate_text(
    field: &'static str,
    value: String,
    max_bytes: usize,
) -> Result<String, EvolutionContractError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(EvolutionContractError::EmptyField(field));
    }
    if value.len() > max_bytes {
        return Err(EvolutionContractError::FieldTooLong(field));
    }
    if value.chars().any(char::is_control) {
        return Err(EvolutionContractError::ControlCharacter(field));
    }
    Ok(trimmed.to_owned())
}

fn validate_exact_commit(value: &str) -> Result<(), EvolutionContractError> {
    if !matches!(value.len(), 40 | 64)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(EvolutionContractError::InvalidCommit);
    }
    Ok(())
}

fn validate_sha256_digest(value: &str) -> Result<(), EvolutionContractError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(EvolutionContractError::InvalidDigest);
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(EvolutionContractError::InvalidDigest);
    }
    Ok(())
}

fn transition_request_fingerprint<T: Serialize>(
    action: &str,
    payload: &T,
) -> Result<RequestDigest, EvolutionContractError> {
    let encoded = serde_json::to_vec(payload)
        .map_err(|_| EvolutionContractError::InvalidRolloutTransition)?;
    let mut hasher = Sha256::new();
    hasher.update(b"pandora-evolution-rollout-transition-v1\0");
    hasher.update((action.len() as u64).to_be_bytes());
    hasher.update(action.as_bytes());
    hasher.update((encoded.len() as u64).to_be_bytes());
    hasher.update(encoded);
    RequestDigest::new(format!("sha256:{:x}", hasher.finalize()))
        .map_err(EvolutionContractError::InvalidId)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_policy_requires_all_release_gates() {
        let policy = EvolutionPolicy::production(7);

        assert_eq!(policy.mode(), EvolutionMode::Production);
        assert!(policy.requires_holdout());
        assert!(policy.requires_signature());
        assert!(policy.requires_canary());
    }

    #[test]
    fn reflection_is_bounded_and_does_not_store_raw_reasoning() {
        let reflection = ReflexionArtifact::new(
            ExecutionId::new("execution-1").unwrap(),
            "The verification step failed",
            vec!["exit code 1".to_owned()],
            "Run the allowlisted verification command before retrying",
            Timestamp::from_unix_seconds(10),
        )
        .unwrap();

        assert_eq!(reflection.failure_signals().len(), 1);
        assert!(!reflection.summary().contains("chain of thought"));
    }

    #[test]
    fn proposals_cannot_replace_the_same_artifact() {
        let artifact = ArtifactId::new("artifact-1").unwrap();
        let result = MutationProposal::new(
            "proposal-1",
            EvolutionSource::Gepa,
            artifact.clone(),
            artifact,
            RequestDigest::new("evidence-1").unwrap(),
            "same artifact",
            Timestamp::from_unix_seconds(10),
        );

        assert_eq!(result, Err(EvolutionContractError::SameArtifact));
    }

    #[test]
    fn holdout_digest_is_optional_for_legacy_records_and_can_be_bound() {
        let evaluation = HoldoutEvaluation::new(
            ProposalId::new("proposal-1").unwrap(),
            100,
            100,
            true,
            true,
            true,
            Timestamp::from_unix_seconds(10),
        );
        assert!(evaluation.holdout_digest().is_none());

        let bound = evaluation
            .with_holdout_digest("sha256:holdout-report")
            .unwrap();
        assert_eq!(
            bound.holdout_digest().unwrap().as_str(),
            "sha256:holdout-report"
        );
    }

    #[test]
    fn proposal_memory_evidence_is_bounded_canonical_and_backward_compatible() {
        let proposal = MutationProposal::new(
            "proposal-1",
            EvolutionSource::Gepa,
            ArtifactId::new("base-1").unwrap(),
            ArtifactId::new("candidate-1").unwrap(),
            RequestDigest::new("evidence-1").unwrap(),
            "improve verification reliability",
            Timestamp::from_unix_seconds(10),
        )
        .unwrap()
        .with_memory_evidence_ids(vec![
            MemoryId::new("memory-b").unwrap(),
            MemoryId::new("memory-a").unwrap(),
            MemoryId::new("memory-a").unwrap(),
        ])
        .unwrap();

        assert_eq!(
            proposal
                .memory_evidence_ids()
                .iter()
                .map(MemoryId::as_str)
                .collect::<Vec<_>>(),
            vec!["memory-a", "memory-b"]
        );

        let mut serialized = serde_json::to_value(&proposal).unwrap();
        serialized
            .as_object_mut()
            .unwrap()
            .remove("memory_evidence_ids");
        let legacy: MutationProposal = serde_json::from_value(serialized).unwrap();
        assert!(legacy.memory_evidence_ids().is_empty());

        let too_many = (0..=MAX_EVOLUTION_MEMORY_EVIDENCE_IDS)
            .map(|index| MemoryId::new(format!("memory-{index}")).unwrap())
            .collect();
        assert_eq!(
            proposal.with_memory_evidence_ids(too_many),
            Err(EvolutionContractError::TooManyMemoryEvidenceIds)
        );
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvolutionRolloutStage {
    Canary,
    Limited,
    Expanded,
    Complete,
}

impl EvolutionRolloutStage {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Canary => "canary",
            Self::Limited => "limited",
            Self::Expanded => "expanded",
            Self::Complete => "complete",
        }
    }

    pub const fn next(self) -> Option<Self> {
        match self {
            Self::Canary => Some(Self::Limited),
            Self::Limited => Some(Self::Expanded),
            Self::Expanded => Some(Self::Complete),
            Self::Complete => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvolutionRolloutStatus {
    Running,
    AwaitingApproval,
    Paused,
    Failed,
    Rejected,
    Complete,
    RolledBack,
}

impl EvolutionRolloutStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::AwaitingApproval => "awaiting_approval",
            Self::Paused => "paused",
            Self::Failed => "failed",
            Self::Rejected => "rejected",
            Self::Complete => "complete",
            Self::RolledBack => "rolled_back",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvolutionReleaseChannel {
    Beta,
    ReleaseCandidate,
    Stable,
}

impl EvolutionReleaseChannel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Beta => "beta",
            Self::ReleaseCandidate => "release-candidate",
            Self::Stable => "stable",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvolutionApprovalAuthority {
    Human,
    AutomatedEvaluator,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EvolutionRolloutBinding {
    exact_commit: String,
    artifact_digest: RequestDigest,
    channel: EvolutionReleaseChannel,
    evidence_digest: RequestDigest,
}

impl EvolutionRolloutBinding {
    pub fn new(
        exact_commit: impl Into<String>,
        artifact_digest: RequestDigest,
        channel: EvolutionReleaseChannel,
        evidence_digest: RequestDigest,
    ) -> Result<Self, EvolutionContractError> {
        let exact_commit = exact_commit.into();
        validate_exact_commit(&exact_commit)?;
        validate_sha256_digest(artifact_digest.as_str())?;
        validate_sha256_digest(evidence_digest.as_str())?;
        Ok(Self {
            exact_commit,
            artifact_digest,
            channel,
            evidence_digest,
        })
    }

    pub fn exact_commit(&self) -> &str {
        &self.exact_commit
    }

    pub fn artifact_digest(&self) -> &RequestDigest {
        &self.artifact_digest
    }

    pub const fn channel(&self) -> EvolutionReleaseChannel {
        self.channel
    }

    pub fn evidence_digest(&self) -> &RequestDigest {
        &self.evidence_digest
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EvolutionStageLimits {
    stage: EvolutionRolloutStage,
    max_cost_micros: u64,
    max_duration_seconds: u64,
    max_failure_count: u32,
    min_quality_score: u8,
    max_p95_latency_millis: u64,
    min_stability_score: u8,
}

impl EvolutionStageLimits {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        stage: EvolutionRolloutStage,
        max_cost_micros: u64,
        max_duration_seconds: u64,
        max_failure_count: u32,
        min_quality_score: u8,
        max_p95_latency_millis: u64,
        min_stability_score: u8,
    ) -> Result<Self, EvolutionContractError> {
        if max_cost_micros == 0
            || max_duration_seconds == 0
            || max_p95_latency_millis == 0
            || min_quality_score > 100
            || min_stability_score > 100
        {
            return Err(EvolutionContractError::InvalidStageLimits);
        }
        Ok(Self {
            stage,
            max_cost_micros,
            max_duration_seconds,
            max_failure_count,
            min_quality_score,
            max_p95_latency_millis,
            min_stability_score,
        })
    }

    pub const fn stage(&self) -> EvolutionRolloutStage {
        self.stage
    }
    pub const fn max_cost_micros(&self) -> u64 {
        self.max_cost_micros
    }
    pub const fn max_duration_seconds(&self) -> u64 {
        self.max_duration_seconds
    }
    pub const fn max_failure_count(&self) -> u32 {
        self.max_failure_count
    }
    pub const fn min_quality_score(&self) -> u8 {
        self.min_quality_score
    }
    pub const fn max_p95_latency_millis(&self) -> u64 {
        self.max_p95_latency_millis
    }
    pub const fn min_stability_score(&self) -> u8 {
        self.min_stability_score
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EvolutionScorecard {
    stage: EvolutionRolloutStage,
    quality_score: u8,
    p95_latency_millis: u64,
    stability_score: u8,
    cost_micros: u64,
    duration_seconds: u64,
    failure_count: u32,
    evidence_digest: RequestDigest,
    scorecard_digest: RequestDigest,
    evaluator: PrincipalId,
    recorded_at: Timestamp,
}

impl EvolutionScorecard {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        stage: EvolutionRolloutStage,
        quality_score: u8,
        p95_latency_millis: u64,
        stability_score: u8,
        cost_micros: u64,
        duration_seconds: u64,
        failure_count: u32,
        evidence_digest: RequestDigest,
        scorecard_digest: RequestDigest,
        evaluator: PrincipalId,
        recorded_at: Timestamp,
    ) -> Result<Self, EvolutionContractError> {
        if quality_score > 100 || stability_score > 100 {
            return Err(EvolutionContractError::InvalidStageLimits);
        }
        validate_sha256_digest(evidence_digest.as_str())?;
        validate_sha256_digest(scorecard_digest.as_str())?;
        Ok(Self {
            stage,
            quality_score,
            p95_latency_millis,
            stability_score,
            cost_micros,
            duration_seconds,
            failure_count,
            evidence_digest,
            scorecard_digest,
            evaluator,
            recorded_at,
        })
    }

    pub const fn stage(&self) -> EvolutionRolloutStage {
        self.stage
    }
    pub const fn quality_score(&self) -> u8 {
        self.quality_score
    }
    pub const fn p95_latency_millis(&self) -> u64 {
        self.p95_latency_millis
    }
    pub const fn stability_score(&self) -> u8 {
        self.stability_score
    }
    pub const fn cost_micros(&self) -> u64 {
        self.cost_micros
    }
    pub const fn duration_seconds(&self) -> u64 {
        self.duration_seconds
    }
    pub const fn failure_count(&self) -> u32 {
        self.failure_count
    }
    pub fn evidence_digest(&self) -> &RequestDigest {
        &self.evidence_digest
    }
    pub fn scorecard_digest(&self) -> &RequestDigest {
        &self.scorecard_digest
    }
    pub fn evaluator(&self) -> &PrincipalId {
        &self.evaluator
    }
    pub const fn recorded_at(&self) -> Timestamp {
        self.recorded_at
    }

    pub fn passes(&self, limits: &EvolutionStageLimits) -> bool {
        self.stage == limits.stage
            && self.quality_score >= limits.min_quality_score
            && self.p95_latency_millis <= limits.max_p95_latency_millis
            && self.stability_score >= limits.min_stability_score
            && self.cost_micros <= limits.max_cost_micros
            && self.duration_seconds <= limits.max_duration_seconds
            && self.failure_count <= limits.max_failure_count
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EvolutionPromotionApproval {
    approval_id: RequestDigest,
    proposal_id: ProposalId,
    from_stage: EvolutionRolloutStage,
    to_stage: Option<EvolutionRolloutStage>,
    binding: EvolutionRolloutBinding,
    scorecard_digest: RequestDigest,
    approver: PrincipalId,
    authority: EvolutionApprovalAuthority,
    approved_at: Timestamp,
    expires_at: Timestamp,
}

impl EvolutionPromotionApproval {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        approval_id: RequestDigest,
        proposal_id: ProposalId,
        from_stage: EvolutionRolloutStage,
        to_stage: Option<EvolutionRolloutStage>,
        binding: EvolutionRolloutBinding,
        scorecard_digest: RequestDigest,
        approver: PrincipalId,
        authority: EvolutionApprovalAuthority,
        approved_at: Timestamp,
        expires_at: Timestamp,
    ) -> Result<Self, EvolutionContractError> {
        validate_sha256_digest(approval_id.as_str())?;
        validate_sha256_digest(scorecard_digest.as_str())?;
        if expires_at.as_unix_seconds() <= approved_at.as_unix_seconds() {
            return Err(EvolutionContractError::ApprovalExpired);
        }
        Ok(Self {
            approval_id,
            proposal_id,
            from_stage,
            to_stage,
            binding,
            scorecard_digest,
            approver,
            authority,
            approved_at,
            expires_at,
        })
    }

    pub fn approval_id(&self) -> &RequestDigest {
        &self.approval_id
    }
    pub fn proposal_id(&self) -> &ProposalId {
        &self.proposal_id
    }
    pub const fn from_stage(&self) -> EvolutionRolloutStage {
        self.from_stage
    }
    pub const fn to_stage(&self) -> Option<EvolutionRolloutStage> {
        self.to_stage
    }
    pub fn binding(&self) -> &EvolutionRolloutBinding {
        &self.binding
    }
    pub fn scorecard_digest(&self) -> &RequestDigest {
        &self.scorecard_digest
    }
    pub fn approver(&self) -> &PrincipalId {
        &self.approver
    }
    pub const fn authority(&self) -> EvolutionApprovalAuthority {
        self.authority
    }
    pub const fn approved_at(&self) -> Timestamp {
        self.approved_at
    }
    pub const fn expires_at(&self) -> Timestamp {
        self.expires_at
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EvolutionTransitionEvidence {
    transition_id: RequestDigest,
    request_fingerprint: RequestDigest,
    action: String,
    from_stage: EvolutionRolloutStage,
    to_stage: EvolutionRolloutStage,
    from_status: EvolutionRolloutStatus,
    to_status: EvolutionRolloutStatus,
    actor: PrincipalId,
    evidence_digest: RequestDigest,
    occurred_at: Timestamp,
    reason: String,
}

impl EvolutionTransitionEvidence {
    #[allow(clippy::too_many_arguments)]
    fn new(
        transition_id: RequestDigest,
        request_fingerprint: RequestDigest,
        action: impl Into<String>,
        from_stage: EvolutionRolloutStage,
        to_stage: EvolutionRolloutStage,
        from_status: EvolutionRolloutStatus,
        to_status: EvolutionRolloutStatus,
        actor: PrincipalId,
        evidence_digest: RequestDigest,
        occurred_at: Timestamp,
        reason: impl Into<String>,
    ) -> Result<Self, EvolutionContractError> {
        validate_sha256_digest(transition_id.as_str())?;
        validate_sha256_digest(request_fingerprint.as_str())?;
        validate_sha256_digest(evidence_digest.as_str())?;
        Ok(Self {
            transition_id,
            request_fingerprint,
            action: validate_text("rollout transition action", action.into(), 64)?,
            from_stage,
            to_stage,
            from_status,
            to_status,
            actor,
            evidence_digest,
            occurred_at,
            reason: validate_text("rollout transition reason", reason.into(), 1024)?,
        })
    }

    pub fn transition_id(&self) -> &RequestDigest {
        &self.transition_id
    }
    pub fn request_fingerprint(&self) -> &RequestDigest {
        &self.request_fingerprint
    }
    pub fn action(&self) -> &str {
        &self.action
    }
    pub const fn from_stage(&self) -> EvolutionRolloutStage {
        self.from_stage
    }
    pub const fn to_stage(&self) -> EvolutionRolloutStage {
        self.to_stage
    }
    pub const fn from_status(&self) -> EvolutionRolloutStatus {
        self.from_status
    }
    pub const fn to_status(&self) -> EvolutionRolloutStatus {
        self.to_status
    }
    pub fn actor(&self) -> &PrincipalId {
        &self.actor
    }
    pub fn evidence_digest(&self) -> &RequestDigest {
        &self.evidence_digest
    }
    pub const fn occurred_at(&self) -> Timestamp {
        self.occurred_at
    }
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EvolutionRollout {
    binding: EvolutionRolloutBinding,
    limits: Vec<EvolutionStageLimits>,
    current_stage: EvolutionRolloutStage,
    status: EvolutionRolloutStatus,
    scorecards: Vec<EvolutionScorecard>,
    pending_approval: Option<EvolutionPromotionApproval>,
    consumed_approval_ids: Vec<RequestDigest>,
    retry_count: u8,
    transitions: Vec<EvolutionTransitionEvidence>,
}

impl EvolutionRollout {
    pub fn new(
        binding: EvolutionRolloutBinding,
        mut limits: Vec<EvolutionStageLimits>,
        transition_id: RequestDigest,
        actor: PrincipalId,
        now: Timestamp,
    ) -> Result<Self, EvolutionContractError> {
        limits.sort_by_key(EvolutionStageLimits::stage);
        let expected = [
            EvolutionRolloutStage::Canary,
            EvolutionRolloutStage::Limited,
            EvolutionRolloutStage::Expanded,
            EvolutionRolloutStage::Complete,
        ];
        if limits.len() != expected.len() {
            return Err(EvolutionContractError::MissingStageLimit);
        }
        for (index, limit) in limits.iter().enumerate() {
            if index > 0 && limits[index - 1].stage() == limit.stage() {
                return Err(EvolutionContractError::DuplicateStageLimit);
            }
            if limit.stage() != expected[index] {
                return Err(EvolutionContractError::MissingStageLimit);
            }
        }
        let request_fingerprint =
            transition_request_fingerprint("configure", &(&binding, &limits, &actor, now))?;
        let initial = EvolutionTransitionEvidence::new(
            transition_id,
            request_fingerprint,
            "configure",
            EvolutionRolloutStage::Canary,
            EvolutionRolloutStage::Canary,
            EvolutionRolloutStatus::Running,
            EvolutionRolloutStatus::Running,
            actor,
            binding.evidence_digest().clone(),
            now,
            "configured exact staged rollout binding and limits",
        )?;
        Ok(Self {
            binding,
            limits,
            current_stage: EvolutionRolloutStage::Canary,
            status: EvolutionRolloutStatus::Running,
            scorecards: Vec::new(),
            pending_approval: None,
            consumed_approval_ids: Vec::new(),
            retry_count: 0,
            transitions: vec![initial],
        })
    }

    pub fn binding(&self) -> &EvolutionRolloutBinding {
        &self.binding
    }
    pub fn limits(&self) -> &[EvolutionStageLimits] {
        &self.limits
    }
    pub const fn current_stage(&self) -> EvolutionRolloutStage {
        self.current_stage
    }
    pub const fn status(&self) -> EvolutionRolloutStatus {
        self.status
    }
    pub fn scorecards(&self) -> &[EvolutionScorecard] {
        &self.scorecards
    }
    pub fn pending_approval(&self) -> Option<&EvolutionPromotionApproval> {
        self.pending_approval.as_ref()
    }
    pub fn consumed_approval_ids(&self) -> &[RequestDigest] {
        &self.consumed_approval_ids
    }
    pub const fn retry_count(&self) -> u8 {
        self.retry_count
    }
    pub fn transitions(&self) -> &[EvolutionTransitionEvidence] {
        &self.transitions
    }
    pub fn activation_ready(&self) -> bool {
        self.current_stage == EvolutionRolloutStage::Complete
            && self.status == EvolutionRolloutStatus::Complete
    }

    pub fn configuration_replay(
        &self,
        binding: &EvolutionRolloutBinding,
        limits: &[EvolutionStageLimits],
        transition_id: &RequestDigest,
        actor: &PrincipalId,
        configured_at: Timestamp,
    ) -> Result<bool, EvolutionContractError> {
        let mut limits = limits.to_vec();
        limits.sort_by_key(EvolutionStageLimits::stage);
        let request_fingerprint =
            transition_request_fingerprint("configure", &(binding, &limits, actor, configured_at))?;
        Ok(self
            .replay(transition_id, "configure", &request_fingerprint)?
            .is_some())
    }

    pub fn validate(&self) -> Result<(), EvolutionContractError> {
        let binding = EvolutionRolloutBinding::new(
            self.binding.exact_commit().to_owned(),
            self.binding.artifact_digest().clone(),
            self.binding.channel(),
            self.binding.evidence_digest().clone(),
        )?;
        if binding != self.binding {
            return Err(EvolutionContractError::ApprovalMismatch);
        }
        let mut stages = self
            .limits
            .iter()
            .map(|limits| {
                EvolutionStageLimits::new(
                    limits.stage(),
                    limits.max_cost_micros(),
                    limits.max_duration_seconds(),
                    limits.max_failure_count(),
                    limits.min_quality_score(),
                    limits.max_p95_latency_millis(),
                    limits.min_stability_score(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        stages.sort_by_key(EvolutionStageLimits::stage);
        if stages.len() != 4
            || stages.iter().map(EvolutionStageLimits::stage).ne([
                EvolutionRolloutStage::Canary,
                EvolutionRolloutStage::Limited,
                EvolutionRolloutStage::Expanded,
                EvolutionRolloutStage::Complete,
            ])
        {
            return Err(EvolutionContractError::MissingStageLimit);
        }
        if self.scorecards.len() > MAX_EVOLUTION_ROLLOUT_SCORECARDS {
            return Err(EvolutionContractError::TooManyRolloutScorecards);
        }
        for scorecard in &self.scorecards {
            EvolutionScorecard::new(
                scorecard.stage(),
                scorecard.quality_score(),
                scorecard.p95_latency_millis(),
                scorecard.stability_score(),
                scorecard.cost_micros(),
                scorecard.duration_seconds(),
                scorecard.failure_count(),
                scorecard.evidence_digest().clone(),
                scorecard.scorecard_digest().clone(),
                scorecard.evaluator().clone(),
                scorecard.recorded_at(),
            )?;
        }
        if self.transitions.is_empty() || self.transitions.len() > MAX_EVOLUTION_ROLLOUT_TRANSITIONS
        {
            return Err(EvolutionContractError::TooManyRolloutTransitions);
        }
        let mut transition_ids = std::collections::BTreeSet::new();
        for event in &self.transitions {
            validate_sha256_digest(event.transition_id().as_str())?;
            validate_sha256_digest(event.request_fingerprint().as_str())?;
            validate_sha256_digest(event.evidence_digest().as_str())?;
            validate_text("rollout transition action", event.action().to_owned(), 64)?;
            validate_text("rollout transition reason", event.reason().to_owned(), 1024)?;
            if !transition_ids.insert(event.transition_id().clone()) {
                return Err(EvolutionContractError::TransitionIdReused);
            }
        }
        if self.retry_count > MAX_EVOLUTION_ROLLOUT_RETRIES {
            return Err(EvolutionContractError::RetryLimitExceeded);
        }
        if self.status == EvolutionRolloutStatus::Complete && !self.activation_ready() {
            return Err(EvolutionContractError::InvalidRolloutTransition);
        }
        self.validate_history()?;
        Ok(())
    }

    fn validate_history(&self) -> Result<(), EvolutionContractError> {
        let first = self
            .transitions
            .first()
            .ok_or(EvolutionContractError::InvalidRolloutTransition)?;
        let configured_fingerprint = transition_request_fingerprint(
            "configure",
            &(
                &self.binding,
                &self.limits,
                first.actor(),
                first.occurred_at(),
            ),
        )?;
        if first.action() != "configure"
            || first.request_fingerprint() != &configured_fingerprint
            || first.from_stage() != EvolutionRolloutStage::Canary
            || first.to_stage() != EvolutionRolloutStage::Canary
            || first.from_status() != EvolutionRolloutStatus::Running
            || first.to_status() != EvolutionRolloutStatus::Running
            || first.evidence_digest() != self.binding.evidence_digest()
        {
            return Err(EvolutionContractError::InvalidRolloutTransition);
        }

        let mut stage = EvolutionRolloutStage::Canary;
        let mut status = EvolutionRolloutStatus::Running;
        let mut retry_count = 0_u8;
        let mut scorecard_index = 0_usize;
        let mut approval_event_ids = std::collections::BTreeSet::new();
        let mut consumed_event_ids = std::collections::BTreeSet::new();
        let mut previous_time = first.occurred_at().as_unix_seconds();

        for event in self.transitions.iter().skip(1) {
            if event.from_stage() != stage
                || event.from_status() != status
                || event.occurred_at().as_unix_seconds() < previous_time
            {
                return Err(EvolutionContractError::InvalidRolloutTransition);
            }
            previous_time = event.occurred_at().as_unix_seconds();
            match event.action() {
                "scorecard" => {
                    let scorecard = self
                        .scorecards
                        .get(scorecard_index)
                        .ok_or(EvolutionContractError::InvalidRolloutTransition)?;
                    let limits = self
                        .limits
                        .iter()
                        .find(|limits| limits.stage() == stage)
                        .ok_or(EvolutionContractError::MissingStageLimit)?;
                    let expected_status = if scorecard.passes(limits) {
                        EvolutionRolloutStatus::AwaitingApproval
                    } else {
                        EvolutionRolloutStatus::Failed
                    };
                    let expected_fingerprint =
                        transition_request_fingerprint("scorecard", &(scorecard, event.actor()))?;
                    if status != EvolutionRolloutStatus::Running
                        || scorecard.stage() != stage
                        || scorecard.evaluator() != event.actor()
                        || event.to_stage() != stage
                        || event.to_status() != expected_status
                        || event.evidence_digest() != scorecard.scorecard_digest()
                        || event.occurred_at() != scorecard.recorded_at()
                        || event.request_fingerprint() != &expected_fingerprint
                    {
                        return Err(EvolutionContractError::InvalidRolloutTransition);
                    }
                    scorecard_index += 1;
                }
                "approve" => {
                    let scorecard = self
                        .scorecards
                        .get(scorecard_index.saturating_sub(1))
                        .ok_or(EvolutionContractError::InvalidRolloutTransition)?;
                    if status != EvolutionRolloutStatus::AwaitingApproval
                        || event.to_stage() != stage
                        || event.to_status() != status
                        || event.actor() == scorecard.evaluator()
                        || !approval_event_ids.insert(event.evidence_digest().clone())
                    {
                        return Err(EvolutionContractError::InvalidRolloutTransition);
                    }
                }
                "promote" => {
                    let expected_stage = stage.next().unwrap_or(stage);
                    let expected_status = if stage.next().is_some() {
                        EvolutionRolloutStatus::Running
                    } else {
                        EvolutionRolloutStatus::Complete
                    };
                    let expected_fingerprint = transition_request_fingerprint(
                        "promote",
                        &(event.actor(), event.reason()),
                    )?;
                    if status != EvolutionRolloutStatus::AwaitingApproval
                        || event.to_stage() != expected_stage
                        || event.to_status() != expected_status
                        || event.request_fingerprint() != &expected_fingerprint
                        || !approval_event_ids.contains(event.evidence_digest())
                        || !consumed_event_ids.insert(event.evidence_digest().clone())
                    {
                        return Err(EvolutionContractError::InvalidRolloutTransition);
                    }
                    if stage.next().is_some() {
                        retry_count = 0;
                    }
                }
                "pause" => {
                    self.validate_simple_history_event(
                        event,
                        stage,
                        EvolutionRolloutStatus::Running,
                        EvolutionRolloutStatus::Paused,
                    )?;
                }
                "resume" => {
                    self.validate_simple_history_event(
                        event,
                        stage,
                        EvolutionRolloutStatus::Paused,
                        EvolutionRolloutStatus::Running,
                    )?;
                }
                "reject" => {
                    let expected_fingerprint =
                        transition_request_fingerprint("reject", &(event.actor(), event.reason()))?;
                    let evidence_is_approval = approval_event_ids.contains(event.evidence_digest());
                    let expected_scorecard_evidence = self
                        .scorecards
                        .get(scorecard_index.saturating_sub(1))
                        .map(EvolutionScorecard::scorecard_digest);
                    if status != EvolutionRolloutStatus::AwaitingApproval
                        || event.to_stage() != stage
                        || event.to_status() != EvolutionRolloutStatus::Rejected
                        || event.request_fingerprint() != &expected_fingerprint
                        || (!evidence_is_approval
                            && expected_scorecard_evidence != Some(event.evidence_digest()))
                    {
                        return Err(EvolutionContractError::InvalidRolloutTransition);
                    }
                    if evidence_is_approval
                        && !consumed_event_ids.insert(event.evidence_digest().clone())
                    {
                        return Err(EvolutionContractError::InvalidRolloutTransition);
                    }
                }
                "retry" => {
                    let expected_fingerprint =
                        transition_request_fingerprint("retry", &(event.actor(), event.reason()))?;
                    if !matches!(
                        status,
                        EvolutionRolloutStatus::Failed | EvolutionRolloutStatus::Rejected
                    ) || event.to_stage() != stage
                        || event.to_status() != EvolutionRolloutStatus::Running
                        || event.request_fingerprint() != &expected_fingerprint
                        || event.evidence_digest() != self.binding.evidence_digest()
                        || retry_count >= MAX_EVOLUTION_ROLLOUT_RETRIES
                    {
                        return Err(EvolutionContractError::InvalidRolloutTransition);
                    }
                    retry_count = retry_count.saturating_add(1);
                }
                "rollback" => {
                    let expected_fingerprint = transition_request_fingerprint(
                        "rollback",
                        &(event.actor(), event.reason()),
                    )?;
                    let evidence_is_approval = approval_event_ids.contains(event.evidence_digest());
                    if status == EvolutionRolloutStatus::RolledBack
                        || event.to_stage() != stage
                        || event.to_status() != EvolutionRolloutStatus::RolledBack
                        || event.request_fingerprint() != &expected_fingerprint
                        || (!evidence_is_approval
                            && event.evidence_digest() != self.binding.artifact_digest())
                    {
                        return Err(EvolutionContractError::InvalidRolloutTransition);
                    }
                    if evidence_is_approval
                        && !consumed_event_ids.insert(event.evidence_digest().clone())
                    {
                        return Err(EvolutionContractError::InvalidRolloutTransition);
                    }
                }
                "activate" => {
                    let expected_fingerprint =
                        transition_request_fingerprint("activate", event.actor())?;
                    if stage != EvolutionRolloutStage::Complete
                        || status != EvolutionRolloutStatus::Complete
                        || event.to_stage() != stage
                        || event.to_status() != status
                        || event.request_fingerprint() != &expected_fingerprint
                        || event.evidence_digest() != self.binding.artifact_digest()
                    {
                        return Err(EvolutionContractError::InvalidRolloutTransition);
                    }
                }
                _ => return Err(EvolutionContractError::InvalidRolloutTransition),
            }
            stage = event.to_stage();
            status = event.to_status();
        }

        let mut consumed_ids = std::collections::BTreeSet::new();
        for approval_id in &self.consumed_approval_ids {
            validate_sha256_digest(approval_id.as_str())?;
            if !consumed_ids.insert(approval_id.clone()) {
                return Err(EvolutionContractError::ApprovalMismatch);
            }
        }
        if consumed_ids != consumed_event_ids
            || scorecard_index != self.scorecards.len()
            || retry_count != self.retry_count
            || stage != self.current_stage
            || status != self.status
        {
            return Err(EvolutionContractError::InvalidRolloutTransition);
        }

        let pending_id = if let Some(approval) = &self.pending_approval {
            let scorecard = self
                .scorecards
                .last()
                .ok_or(EvolutionContractError::ApprovalMismatch)?;
            let reconstructed = EvolutionPromotionApproval::new(
                approval.approval_id().clone(),
                approval.proposal_id().clone(),
                approval.from_stage(),
                approval.to_stage(),
                approval.binding().clone(),
                approval.scorecard_digest().clone(),
                approval.approver().clone(),
                approval.authority(),
                approval.approved_at(),
                approval.expires_at(),
            )?;
            let expected_fingerprint = transition_request_fingerprint("approve", approval)?;
            let approval_event = self.transitions.iter().find(|event| {
                event.action() == "approve" && event.evidence_digest() == approval.approval_id()
            });
            if reconstructed != *approval
                || status != EvolutionRolloutStatus::AwaitingApproval
                || approval.authority() != EvolutionApprovalAuthority::Human
                || approval.from_stage() != stage
                || approval.to_stage() != stage.next()
                || approval.binding() != &self.binding
                || approval.scorecard_digest() != scorecard.scorecard_digest()
                || approval.approver() == scorecard.evaluator()
                || approval.approved_at().as_unix_seconds()
                    < scorecard.recorded_at().as_unix_seconds()
                || consumed_ids.contains(approval.approval_id())
                || approval_event.is_none_or(|event| {
                    event.actor() != approval.approver()
                        || event.request_fingerprint() != &expected_fingerprint
                })
            {
                return Err(EvolutionContractError::ApprovalMismatch);
            }
            Some(approval.approval_id().clone())
        } else {
            None
        };
        let mut accounted_approval_ids = consumed_ids;
        if let Some(pending_id) = pending_id {
            accounted_approval_ids.insert(pending_id);
        }
        if accounted_approval_ids != approval_event_ids {
            return Err(EvolutionContractError::ApprovalMismatch);
        }
        Ok(())
    }

    fn validate_simple_history_event(
        &self,
        event: &EvolutionTransitionEvidence,
        stage: EvolutionRolloutStage,
        expected_status: EvolutionRolloutStatus,
        next_status: EvolutionRolloutStatus,
    ) -> Result<(), EvolutionContractError> {
        let expected_fingerprint =
            transition_request_fingerprint(event.action(), &(event.actor(), event.reason()))?;
        if event.from_status() != expected_status
            || event.to_stage() != stage
            || event.to_status() != next_status
            || event.request_fingerprint() != &expected_fingerprint
            || event.evidence_digest() != self.binding.evidence_digest()
        {
            return Err(EvolutionContractError::InvalidRolloutTransition);
        }
        Ok(())
    }

    pub fn record_scorecard(
        &mut self,
        scorecard: EvolutionScorecard,
        transition_id: RequestDigest,
        actor: PrincipalId,
    ) -> Result<bool, EvolutionContractError> {
        let request_fingerprint =
            transition_request_fingerprint("scorecard", &(&scorecard, &actor))?;
        if let Some(replayed) = self.replay(&transition_id, "scorecard", &request_fingerprint)? {
            return Ok(replayed);
        }
        if scorecard.evaluator() != &actor {
            return Err(EvolutionContractError::EvaluatorMismatch);
        }
        if self.status != EvolutionRolloutStatus::Running || scorecard.stage() != self.current_stage
        {
            return Err(EvolutionContractError::InvalidRolloutTransition);
        }
        if self.scorecards.len() >= MAX_EVOLUTION_ROLLOUT_SCORECARDS {
            return Err(EvolutionContractError::TooManyRolloutScorecards);
        }
        let limits = self
            .limits
            .iter()
            .find(|limits| limits.stage() == self.current_stage)
            .ok_or(EvolutionContractError::MissingStageLimit)?;
        let passed = scorecard.passes(limits);
        let from_status = self.status;
        self.status = if passed {
            EvolutionRolloutStatus::AwaitingApproval
        } else {
            EvolutionRolloutStatus::Failed
        };
        self.append_transition(EvolutionTransitionEvidence::new(
            transition_id,
            request_fingerprint,
            "scorecard",
            self.current_stage,
            self.current_stage,
            from_status,
            self.status,
            actor,
            scorecard.scorecard_digest().clone(),
            scorecard.recorded_at(),
            if passed {
                "stage scorecard passed all configured limits"
            } else {
                "stage scorecard exceeded one or more configured limits"
            },
        )?)?;
        self.scorecards.push(scorecard);
        Ok(true)
    }

    pub fn approve(
        &mut self,
        proposal_id: &ProposalId,
        approval: EvolutionPromotionApproval,
        transition_id: RequestDigest,
        now: Timestamp,
    ) -> Result<bool, EvolutionContractError> {
        let request_fingerprint = transition_request_fingerprint("approve", &approval)?;
        if let Some(replayed) = self.replay(&transition_id, "approve", &request_fingerprint)? {
            return Ok(replayed);
        }
        if self.status != EvolutionRolloutStatus::AwaitingApproval {
            return Err(EvolutionContractError::InvalidRolloutTransition);
        }
        if self.pending_approval.is_some() {
            return Err(EvolutionContractError::ApprovalAlreadyPending);
        }
        let scorecard = self
            .scorecards
            .last()
            .ok_or(EvolutionContractError::ApprovalMismatch)?;
        let expected_next = self.current_stage.next();
        if approval.authority() != EvolutionApprovalAuthority::Human {
            return Err(EvolutionContractError::HumanApprovalRequired);
        }
        if approval.approver() == scorecard.evaluator() {
            return Err(EvolutionContractError::SelfApproval);
        }
        if approval.proposal_id() != proposal_id
            || approval.from_stage() != self.current_stage
            || approval.to_stage() != expected_next
            || approval.binding() != &self.binding
            || approval.scorecard_digest() != scorecard.scorecard_digest()
        {
            return Err(EvolutionContractError::ApprovalMismatch);
        }
        if approval.approved_at().as_unix_seconds() < scorecard.recorded_at().as_unix_seconds()
            || now.as_unix_seconds() < approval.approved_at().as_unix_seconds()
            || now.as_unix_seconds() > approval.expires_at().as_unix_seconds()
        {
            return Err(EvolutionContractError::ApprovalExpired);
        }
        if self
            .consumed_approval_ids
            .iter()
            .any(|used| used == approval.approval_id())
        {
            return Err(EvolutionContractError::ApprovalMismatch);
        }
        let evidence = approval.approval_id().clone();
        let actor = approval.approver().clone();
        self.pending_approval = Some(approval);
        self.append_transition(EvolutionTransitionEvidence::new(
            transition_id,
            request_fingerprint,
            "approve",
            self.current_stage,
            self.current_stage,
            self.status,
            self.status,
            actor,
            evidence,
            now,
            "human promotion approval bound to exact commit, artifact, channel, evidence, and scorecard",
        )?)?;
        Ok(true)
    }

    pub fn promote(
        &mut self,
        transition_id: RequestDigest,
        actor: PrincipalId,
        now: Timestamp,
        reason: impl Into<String>,
    ) -> Result<bool, EvolutionContractError> {
        let reason = reason.into();
        let request_fingerprint = transition_request_fingerprint("promote", &(&actor, &reason))?;
        if let Some(replayed) = self.replay(&transition_id, "promote", &request_fingerprint)? {
            return Ok(replayed);
        }
        if self.status != EvolutionRolloutStatus::AwaitingApproval {
            return Err(EvolutionContractError::InvalidRolloutTransition);
        }
        let approval = self
            .pending_approval
            .take()
            .ok_or(EvolutionContractError::HumanApprovalRequired)?;
        if now.as_unix_seconds() > approval.expires_at().as_unix_seconds()
            || approval.from_stage() != self.current_stage
            || approval.approver() != &actor
        {
            self.pending_approval = Some(approval);
            return Err(EvolutionContractError::ApprovalMismatch);
        }
        let from_stage = self.current_stage;
        let from_status = self.status;
        let evidence = approval.approval_id().clone();
        self.consumed_approval_ids
            .push(approval.approval_id().clone());
        if let Some(next) = approval.to_stage() {
            self.current_stage = next;
            self.status = EvolutionRolloutStatus::Running;
            self.retry_count = 0;
        } else {
            self.status = EvolutionRolloutStatus::Complete;
        }
        self.append_transition(EvolutionTransitionEvidence::new(
            transition_id,
            request_fingerprint,
            "promote",
            from_stage,
            self.current_stage,
            from_status,
            self.status,
            actor,
            evidence,
            now,
            reason,
        )?)?;
        Ok(true)
    }

    pub fn pause(
        &mut self,
        transition_id: RequestDigest,
        actor: PrincipalId,
        now: Timestamp,
        reason: impl Into<String>,
    ) -> Result<bool, EvolutionContractError> {
        self.simple_transition(
            transition_id,
            "pause",
            EvolutionRolloutStatus::Running,
            EvolutionRolloutStatus::Paused,
            actor,
            now,
            reason,
        )
    }

    pub fn resume(
        &mut self,
        transition_id: RequestDigest,
        actor: PrincipalId,
        now: Timestamp,
        reason: impl Into<String>,
    ) -> Result<bool, EvolutionContractError> {
        self.simple_transition(
            transition_id,
            "resume",
            EvolutionRolloutStatus::Paused,
            EvolutionRolloutStatus::Running,
            actor,
            now,
            reason,
        )
    }

    pub fn reject(
        &mut self,
        transition_id: RequestDigest,
        actor: PrincipalId,
        now: Timestamp,
        reason: impl Into<String>,
    ) -> Result<bool, EvolutionContractError> {
        let reason = reason.into();
        let request_fingerprint = transition_request_fingerprint("reject", &(&actor, &reason))?;
        if let Some(replayed) = self.replay(&transition_id, "reject", &request_fingerprint)? {
            return Ok(replayed);
        }
        if self.status != EvolutionRolloutStatus::AwaitingApproval {
            return Err(EvolutionContractError::InvalidRolloutTransition);
        }
        let from_status = self.status;
        let evidence = if let Some(approval) = self.pending_approval.take() {
            let approval_id = approval.approval_id().clone();
            self.consumed_approval_ids.push(approval_id.clone());
            approval_id
        } else {
            self.scorecards
                .last()
                .map(|scorecard| scorecard.scorecard_digest().clone())
                .unwrap_or_else(|| self.binding.evidence_digest().clone())
        };
        self.status = EvolutionRolloutStatus::Rejected;
        self.append_transition(EvolutionTransitionEvidence::new(
            transition_id,
            request_fingerprint,
            "reject",
            self.current_stage,
            self.current_stage,
            from_status,
            self.status,
            actor,
            evidence,
            now,
            reason,
        )?)?;
        Ok(true)
    }

    pub fn retry(
        &mut self,
        transition_id: RequestDigest,
        actor: PrincipalId,
        now: Timestamp,
        reason: impl Into<String>,
    ) -> Result<bool, EvolutionContractError> {
        let reason = reason.into();
        let request_fingerprint = transition_request_fingerprint("retry", &(&actor, &reason))?;
        if let Some(replayed) = self.replay(&transition_id, "retry", &request_fingerprint)? {
            return Ok(replayed);
        }
        if !matches!(
            self.status,
            EvolutionRolloutStatus::Failed | EvolutionRolloutStatus::Rejected
        ) {
            return Err(EvolutionContractError::InvalidRolloutTransition);
        }
        if self.retry_count >= MAX_EVOLUTION_ROLLOUT_RETRIES {
            return Err(EvolutionContractError::RetryLimitExceeded);
        }
        let from_status = self.status;
        self.status = EvolutionRolloutStatus::Running;
        self.pending_approval = None;
        self.retry_count = self.retry_count.saturating_add(1);
        self.append_transition(EvolutionTransitionEvidence::new(
            transition_id,
            request_fingerprint,
            "retry",
            self.current_stage,
            self.current_stage,
            from_status,
            self.status,
            actor,
            self.binding.evidence_digest().clone(),
            now,
            reason,
        )?)?;
        Ok(true)
    }

    pub fn mark_activated(
        &mut self,
        transition_id: RequestDigest,
        actor: PrincipalId,
        now: Timestamp,
    ) -> Result<bool, EvolutionContractError> {
        let request_fingerprint = transition_request_fingerprint("activate", &actor)?;
        if !self.activation_ready() {
            return Err(EvolutionContractError::InvalidRolloutTransition);
        }
        if let Some(replayed) = self.replay(&transition_id, "activate", &request_fingerprint)? {
            return Ok(replayed);
        }
        self.append_transition(EvolutionTransitionEvidence::new(
            transition_id,
            request_fingerprint,
            "activate",
            self.current_stage,
            self.current_stage,
            self.status,
            self.status,
            actor,
            self.binding.artifact_digest().clone(),
            now,
            "used the existing artifact activation path after governed rollout completion",
        )?)?;
        Ok(true)
    }

    pub fn mark_rolled_back(
        &mut self,
        transition_id: RequestDigest,
        actor: PrincipalId,
        now: Timestamp,
        reason: impl Into<String>,
    ) -> Result<bool, EvolutionContractError> {
        let reason = reason.into();
        let request_fingerprint = transition_request_fingerprint("rollback", &(&actor, &reason))?;
        if let Some(replayed) = self.replay(&transition_id, "rollback", &request_fingerprint)? {
            return Ok(replayed);
        }
        if self.status == EvolutionRolloutStatus::RolledBack {
            return Err(EvolutionContractError::InvalidRolloutTransition);
        }
        let from_status = self.status;
        self.status = EvolutionRolloutStatus::RolledBack;
        let evidence = if let Some(approval) = self.pending_approval.take() {
            let approval_id = approval.approval_id().clone();
            self.consumed_approval_ids.push(approval_id.clone());
            approval_id
        } else {
            self.binding.artifact_digest().clone()
        };
        self.append_transition(EvolutionTransitionEvidence::new(
            transition_id,
            request_fingerprint,
            "rollback",
            self.current_stage,
            self.current_stage,
            from_status,
            self.status,
            actor,
            evidence,
            now,
            reason,
        )?)?;
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    fn simple_transition(
        &mut self,
        transition_id: RequestDigest,
        action: &'static str,
        expected: EvolutionRolloutStatus,
        next: EvolutionRolloutStatus,
        actor: PrincipalId,
        now: Timestamp,
        reason: impl Into<String>,
    ) -> Result<bool, EvolutionContractError> {
        let reason = reason.into();
        let request_fingerprint = transition_request_fingerprint(action, &(&actor, &reason))?;
        if let Some(replayed) = self.replay(&transition_id, action, &request_fingerprint)? {
            return Ok(replayed);
        }
        if self.status != expected {
            return Err(EvolutionContractError::InvalidRolloutTransition);
        }
        let from_status = self.status;
        self.status = next;
        self.append_transition(EvolutionTransitionEvidence::new(
            transition_id,
            request_fingerprint,
            action,
            self.current_stage,
            self.current_stage,
            from_status,
            next,
            actor,
            self.binding.evidence_digest().clone(),
            now,
            reason,
        )?)?;
        Ok(true)
    }

    fn replay(
        &self,
        transition_id: &RequestDigest,
        action: &str,
        request_fingerprint: &RequestDigest,
    ) -> Result<Option<bool>, EvolutionContractError> {
        match self
            .transitions
            .iter()
            .find(|event| event.transition_id() == transition_id)
        {
            Some(event)
                if event.action() == action
                    && event.request_fingerprint() == request_fingerprint =>
            {
                Ok(Some(false))
            }
            Some(_) => Err(EvolutionContractError::TransitionIdReused),
            None => Ok(None),
        }
    }

    fn append_transition(
        &mut self,
        transition: EvolutionTransitionEvidence,
    ) -> Result<(), EvolutionContractError> {
        if self.transitions.len() >= MAX_EVOLUTION_ROLLOUT_TRANSITIONS {
            return Err(EvolutionContractError::TooManyRolloutTransitions);
        }
        if self.transitions.last().is_some_and(|previous| {
            previous.occurred_at().as_unix_seconds() > transition.occurred_at().as_unix_seconds()
        }) {
            return Err(EvolutionContractError::InvalidRolloutTransition);
        }
        self.transitions.push(transition);
        Ok(())
    }
}

#[cfg(test)]
mod rollout_tests {
    use super::*;

    fn digest(value: u64) -> RequestDigest {
        RequestDigest::new(format!("sha256:{value:064x}")).unwrap()
    }

    fn limits() -> Vec<EvolutionStageLimits> {
        [
            EvolutionRolloutStage::Canary,
            EvolutionRolloutStage::Limited,
            EvolutionRolloutStage::Expanded,
            EvolutionRolloutStage::Complete,
        ]
        .into_iter()
        .map(|stage| EvolutionStageLimits::new(stage, 10_000, 600, 0, 90, 500, 95).unwrap())
        .collect()
    }

    fn binding() -> EvolutionRolloutBinding {
        EvolutionRolloutBinding::new(
            "0123456789abcdef0123456789abcdef01234567",
            digest(1),
            EvolutionReleaseChannel::Beta,
            digest(2),
        )
        .unwrap()
    }

    #[test]
    fn every_stage_requires_passing_scorecard_and_exact_human_approval() {
        let proposal_id = ProposalId::new("proposal-rollout").unwrap();
        let evaluator = PrincipalId::new("evaluator-1").unwrap();
        let approver = PrincipalId::new("release-manager-1").unwrap();
        let mut rollout = EvolutionRollout::new(
            binding(),
            limits(),
            digest(10),
            approver.clone(),
            Timestamp::from_unix_seconds(10),
        )
        .unwrap();

        for (index, stage) in [
            EvolutionRolloutStage::Canary,
            EvolutionRolloutStage::Limited,
            EvolutionRolloutStage::Expanded,
            EvolutionRolloutStage::Complete,
        ]
        .into_iter()
        .enumerate()
        {
            let scorecard_digest = digest(100 + index as u64);
            let scorecard = EvolutionScorecard::new(
                stage,
                98,
                200,
                99,
                1_000,
                60,
                0,
                digest(200 + index as u64),
                scorecard_digest.clone(),
                evaluator.clone(),
                Timestamp::from_unix_seconds(20 + index as u64 * 10),
            )
            .unwrap();
            assert!(
                rollout
                    .record_scorecard(scorecard, digest(300 + index as u64), evaluator.clone(),)
                    .unwrap()
            );
            assert_eq!(rollout.status(), EvolutionRolloutStatus::AwaitingApproval);

            let automated = EvolutionPromotionApproval::new(
                digest(400 + index as u64),
                proposal_id.clone(),
                stage,
                stage.next(),
                binding(),
                scorecard_digest.clone(),
                approver.clone(),
                EvolutionApprovalAuthority::AutomatedEvaluator,
                Timestamp::from_unix_seconds(21 + index as u64 * 10),
                Timestamp::from_unix_seconds(29 + index as u64 * 10),
            )
            .unwrap();
            assert_eq!(
                rollout.approve(
                    &proposal_id,
                    automated,
                    digest(500 + index as u64),
                    Timestamp::from_unix_seconds(22 + index as u64 * 10),
                ),
                Err(EvolutionContractError::HumanApprovalRequired)
            );

            let approval = EvolutionPromotionApproval::new(
                digest(600 + index as u64),
                proposal_id.clone(),
                stage,
                stage.next(),
                binding(),
                scorecard_digest,
                approver.clone(),
                EvolutionApprovalAuthority::Human,
                Timestamp::from_unix_seconds(21 + index as u64 * 10),
                Timestamp::from_unix_seconds(29 + index as u64 * 10),
            )
            .unwrap();
            assert!(
                rollout
                    .approve(
                        &proposal_id,
                        approval,
                        digest(700 + index as u64),
                        Timestamp::from_unix_seconds(22 + index as u64 * 10),
                    )
                    .unwrap()
            );
            let promotion_id = digest(800 + index as u64);
            assert!(
                rollout
                    .promote(
                        promotion_id.clone(),
                        approver.clone(),
                        Timestamp::from_unix_seconds(23 + index as u64 * 10),
                        "human-approved stage promotion",
                    )
                    .unwrap()
            );
            assert!(
                !rollout
                    .promote(
                        promotion_id,
                        approver.clone(),
                        Timestamp::from_unix_seconds(23 + index as u64 * 10),
                        "human-approved stage promotion",
                    )
                    .unwrap()
            );
        }

        assert!(rollout.activation_ready());
        assert_eq!(rollout.status(), EvolutionRolloutStatus::Complete);
        rollout.validate().unwrap();
    }

    #[test]
    fn failed_stage_can_retry_but_transition_ids_cannot_change_meaning() {
        let operator = PrincipalId::new("operator-1").unwrap();
        let evaluator = PrincipalId::new("evaluator-1").unwrap();
        let mut rollout = EvolutionRollout::new(
            binding(),
            limits(),
            digest(1_000),
            operator.clone(),
            Timestamp::from_unix_seconds(1),
        )
        .unwrap();
        rollout
            .record_scorecard(
                EvolutionScorecard::new(
                    EvolutionRolloutStage::Canary,
                    50,
                    900,
                    50,
                    20_000,
                    900,
                    2,
                    digest(1_001),
                    digest(1_002),
                    evaluator.clone(),
                    Timestamp::from_unix_seconds(2),
                )
                .unwrap(),
                digest(1_003),
                evaluator,
            )
            .unwrap();
        assert_eq!(rollout.status(), EvolutionRolloutStatus::Failed);
        assert!(
            rollout
                .retry(
                    digest(1_004),
                    operator.clone(),
                    Timestamp::from_unix_seconds(3),
                    "fresh bounded retry",
                )
                .unwrap()
        );
        assert_eq!(rollout.status(), EvolutionRolloutStatus::Running);
        assert_eq!(
            rollout.pause(
                digest(1_004),
                operator.clone(),
                Timestamp::from_unix_seconds(4),
                "different action",
            ),
            Err(EvolutionContractError::TransitionIdReused)
        );
        assert!(
            rollout
                .pause(
                    digest(1_005),
                    operator.clone(),
                    Timestamp::from_unix_seconds(4),
                    "operator hold",
                )
                .unwrap()
        );
        assert_eq!(
            rollout.pause(
                digest(1_005),
                operator,
                Timestamp::from_unix_seconds(5),
                "changed payload",
            ),
            Err(EvolutionContractError::TransitionIdReused)
        );
        rollout.validate().unwrap();
    }

    #[test]
    fn evaluator_identity_and_rejected_approval_reuse_fail_closed() {
        let proposal_id = ProposalId::new("proposal-one-shot-approval").unwrap();
        let operator = PrincipalId::new("operator-1").unwrap();
        let evaluator = PrincipalId::new("evaluator-1").unwrap();
        let approver = PrincipalId::new("release-manager-1").unwrap();
        let mut rollout = EvolutionRollout::new(
            binding(),
            limits(),
            digest(2_000),
            operator.clone(),
            Timestamp::from_unix_seconds(1),
        )
        .unwrap();
        let scorecard = EvolutionScorecard::new(
            EvolutionRolloutStage::Canary,
            99,
            100,
            99,
            1_000,
            60,
            0,
            digest(2_001),
            digest(2_002),
            evaluator.clone(),
            Timestamp::from_unix_seconds(2),
        )
        .unwrap();
        assert_eq!(
            rollout.record_scorecard(
                scorecard.clone(),
                digest(2_003),
                PrincipalId::new("claimed-evaluator-2").unwrap(),
            ),
            Err(EvolutionContractError::EvaluatorMismatch)
        );
        rollout
            .record_scorecard(scorecard, digest(2_004), evaluator.clone())
            .unwrap();

        let approval_id = digest(2_005);
        let approval = EvolutionPromotionApproval::new(
            approval_id.clone(),
            proposal_id.clone(),
            EvolutionRolloutStage::Canary,
            Some(EvolutionRolloutStage::Limited),
            binding(),
            digest(2_002),
            approver.clone(),
            EvolutionApprovalAuthority::Human,
            Timestamp::from_unix_seconds(3),
            Timestamp::from_unix_seconds(20),
        )
        .unwrap();
        rollout
            .approve(
                &proposal_id,
                approval,
                digest(2_006),
                Timestamp::from_unix_seconds(4),
            )
            .unwrap();
        let replacement = EvolutionPromotionApproval::new(
            digest(2_007),
            proposal_id.clone(),
            EvolutionRolloutStage::Canary,
            Some(EvolutionRolloutStage::Limited),
            binding(),
            digest(2_002),
            approver.clone(),
            EvolutionApprovalAuthority::Human,
            Timestamp::from_unix_seconds(3),
            Timestamp::from_unix_seconds(20),
        )
        .unwrap();
        assert_eq!(
            rollout.approve(
                &proposal_id,
                replacement,
                digest(2_008),
                Timestamp::from_unix_seconds(4),
            ),
            Err(EvolutionContractError::ApprovalAlreadyPending)
        );
        rollout
            .reject(
                digest(2_009),
                approver.clone(),
                Timestamp::from_unix_seconds(5),
                "human rejection",
            )
            .unwrap();
        assert_eq!(
            rollout.consumed_approval_ids(),
            std::slice::from_ref(&approval_id)
        );
        rollout
            .retry(
                digest(2_010),
                operator,
                Timestamp::from_unix_seconds(6),
                "fresh evidence",
            )
            .unwrap();
        rollout
            .record_scorecard(
                EvolutionScorecard::new(
                    EvolutionRolloutStage::Canary,
                    99,
                    90,
                    99,
                    900,
                    50,
                    0,
                    digest(2_011),
                    digest(2_012),
                    evaluator.clone(),
                    Timestamp::from_unix_seconds(7),
                )
                .unwrap(),
                digest(2_013),
                evaluator,
            )
            .unwrap();
        let reused = EvolutionPromotionApproval::new(
            approval_id,
            proposal_id.clone(),
            EvolutionRolloutStage::Canary,
            Some(EvolutionRolloutStage::Limited),
            binding(),
            digest(2_012),
            approver,
            EvolutionApprovalAuthority::Human,
            Timestamp::from_unix_seconds(8),
            Timestamp::from_unix_seconds(20),
        )
        .unwrap();
        assert_eq!(
            rollout.approve(
                &proposal_id,
                reused,
                digest(2_014),
                Timestamp::from_unix_seconds(9),
            ),
            Err(EvolutionContractError::ApprovalMismatch)
        );
        rollout.validate().unwrap();
    }
}
