use crate::effect::Timestamp;
use crate::ids::{
    ArtifactId, ExecutionId, IdError, MemoryId, PrincipalId, ProposalId, RequestDigest,
};
use serde::{Deserialize, Serialize};
use std::fmt;

const MAX_TEXT_BYTES: usize = 4096;
const MAX_FAILURE_SIGNALS: usize = 16;
pub const MAX_EVOLUTION_MEMORY_EVIDENCE_IDS: usize = 16;

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
