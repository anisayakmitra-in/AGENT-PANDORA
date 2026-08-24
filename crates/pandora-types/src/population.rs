use crate::{
    ArtifactId, EvaluationKind, EvaluationReceipt, ExecutionId, FailureId, IdError, MemoryId,
    MemoryKind, MemoryTier, PopulationId, RequestDigest, SessionId, TenantId, Timestamp, Usage,
    WorkspaceId,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const POPULATION_PROTOCOL_VERSION: u16 = 1;
const MAX_FAILURES: usize = 256;
const MAX_CATEGORY_BYTES: usize = 256;
const MAX_SUMMARY_BYTES: usize = 4096;
const MAX_LINEAGE_SUMMARY_BYTES: usize = 16_384;
const MAX_PARENTS: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailurePartition {
    Training,
    Holdout,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum PrecheckDisposition {
    Passed,
    Rejected,
}

impl PrecheckDisposition {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Rejected => "rejected",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum PrecheckFailure {
    Missing(EvaluationKind),
    Advisory(EvaluationKind),
    Failed(EvaluationKind),
    HumanReviewRequired(EvaluationKind),
    BelowMinimum(EvaluationKind),
}

impl PrecheckFailure {
    pub const fn kind(self) -> EvaluationKind {
        match self {
            Self::Missing(kind)
            | Self::Advisory(kind)
            | Self::Failed(kind)
            | Self::HumanReviewRequired(kind)
            | Self::BelowMinimum(kind) => kind,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Missing(_) => "missing",
            Self::Advisory(_) => "advisory",
            Self::Failed(_) => "failed",
            Self::HumanReviewRequired(_) => "human_review_required",
            Self::BelowMinimum(_) => "below_minimum",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PopulationContractError {
    InvalidId(IdError),
    EmptyField(&'static str),
    FieldTooLong(&'static str),
    ControlCharacter(&'static str),
    InvalidLimit(&'static str),
    InvalidScore,
    TooManyFailures,
    TooManyParents,
    DuplicateFailure(FailureId),
    DuplicateCandidate(ArtifactId),
    DuplicateParent(ArtifactId),
    DuplicateTrainingFailure(FailureId),
    DuplicateLineageAttempt(ArtifactId),
    ParentMatchesCandidate,
    PrecheckRequestMismatch,
    InvalidPrecheckDisposition,
    InvalidOutcomeShape,
    InvalidGenerationStats,
    UsageOverflow,
}

impl fmt::Display for PopulationContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId(error) => error.fmt(formatter),
            Self::EmptyField(field) => write!(formatter, "{field} cannot be empty"),
            Self::FieldTooLong(field) => write!(formatter, "{field} is too long"),
            Self::ControlCharacter(field) => {
                write!(formatter, "{field} contains a control character")
            }
            Self::InvalidLimit(limit) => write!(formatter, "{limit} must be positive and bounded"),
            Self::InvalidScore => formatter.write_str("population score exceeds 100"),
            Self::TooManyFailures => formatter.write_str("failure corpus exceeds 256 entries"),
            Self::TooManyParents => formatter.write_str("candidate exceeds 16 parent artifacts"),
            Self::DuplicateFailure(id) => write!(formatter, "failure {id} is duplicated"),
            Self::DuplicateCandidate(id) => write!(formatter, "candidate {id} is duplicated"),
            Self::DuplicateParent(id) => write!(formatter, "parent {id} is duplicated"),
            Self::DuplicateTrainingFailure(id) => {
                write!(formatter, "training failure {id} is duplicated")
            }
            Self::DuplicateLineageAttempt(id) => {
                write!(formatter, "lineage attempt {id} is duplicated")
            }
            Self::ParentMatchesCandidate => {
                formatter.write_str("candidate artifact cannot be its own parent")
            }
            Self::PrecheckRequestMismatch => {
                formatter.write_str("mutation precheck does not match the request")
            }
            Self::InvalidPrecheckDisposition => {
                formatter.write_str("mutation precheck has the wrong disposition")
            }
            Self::InvalidOutcomeShape => {
                formatter.write_str("candidate outcome has inconsistent evaluation evidence")
            }
            Self::InvalidGenerationStats => {
                formatter.write_str("generation statistics do not match candidate outcomes")
            }
            Self::UsageOverflow => formatter.write_str("generation usage overflowed"),
        }
    }
}

impl std::error::Error for PopulationContractError {}

impl From<IdError> for PopulationContractError {
    fn from(error: IdError) -> Self {
        Self::InvalidId(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PopulationMutationRequest {
    population_id: PopulationId,
    generation: u32,
    parent_artifact: ArtifactId,
    candidate_artifact: ArtifactId,
    plan_digest: RequestDigest,
    mutation_batch_digest: RequestDigest,
    request_digest: RequestDigest,
}

impl PopulationMutationRequest {
    pub fn new(
        population_id: PopulationId,
        generation: u32,
        parent_artifact: ArtifactId,
        candidate_artifact: ArtifactId,
        plan_digest: RequestDigest,
        mutation_batch_digest: RequestDigest,
    ) -> Result<Self, PopulationContractError> {
        if parent_artifact == candidate_artifact {
            return Err(PopulationContractError::ParentMatchesCandidate);
        }
        let request_digest = digest_mutation_request(
            &population_id,
            generation,
            &parent_artifact,
            &candidate_artifact,
            &plan_digest,
            &mutation_batch_digest,
        );
        Ok(Self {
            population_id,
            generation,
            parent_artifact,
            candidate_artifact,
            plan_digest,
            mutation_batch_digest,
            request_digest,
        })
    }

    pub fn population_id(&self) -> &PopulationId {
        &self.population_id
    }

    pub const fn generation(&self) -> u32 {
        self.generation
    }

    pub fn parent_artifact(&self) -> &ArtifactId {
        &self.parent_artifact
    }

    pub fn candidate_artifact(&self) -> &ArtifactId {
        &self.candidate_artifact
    }

    pub fn plan_digest(&self) -> &RequestDigest {
        &self.plan_digest
    }

    pub fn mutation_batch_digest(&self) -> &RequestDigest {
        &self.mutation_batch_digest
    }

    pub fn request_digest(&self) -> &RequestDigest {
        &self.request_digest
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MutationPrecheckReceipt {
    request_digest: RequestDigest,
    evaluation_session_id: SessionId,
    evaluation_execution_id: ExecutionId,
    evaluation_digest: RequestDigest,
    minimum_score: u8,
    disposition: PrecheckDisposition,
    failures: Vec<PrecheckFailure>,
    evaluated_at: Timestamp,
    digest: RequestDigest,
}

impl MutationPrecheckReceipt {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        request_digest: RequestDigest,
        evaluation_session_id: SessionId,
        evaluation_execution_id: ExecutionId,
        evaluation_digest: RequestDigest,
        minimum_score: u8,
        mut failures: Vec<PrecheckFailure>,
        evaluated_at: Timestamp,
    ) -> Result<Self, PopulationContractError> {
        if minimum_score > 100 {
            return Err(PopulationContractError::InvalidScore);
        }
        failures.sort();
        failures.dedup();
        let disposition = if failures.is_empty() {
            PrecheckDisposition::Passed
        } else {
            PrecheckDisposition::Rejected
        };
        let digest = digest_precheck(
            &request_digest,
            &evaluation_session_id,
            &evaluation_execution_id,
            &evaluation_digest,
            minimum_score,
            disposition,
            &failures,
            evaluated_at,
        );
        Ok(Self {
            request_digest,
            evaluation_session_id,
            evaluation_execution_id,
            evaluation_digest,
            minimum_score,
            disposition,
            failures,
            evaluated_at,
            digest,
        })
    }

    pub fn request_digest(&self) -> &RequestDigest {
        &self.request_digest
    }

    pub fn evaluation_session_id(&self) -> &SessionId {
        &self.evaluation_session_id
    }

    pub fn evaluation_execution_id(&self) -> &ExecutionId {
        &self.evaluation_execution_id
    }

    pub fn evaluation_digest(&self) -> &RequestDigest {
        &self.evaluation_digest
    }

    pub const fn minimum_score(&self) -> u8 {
        self.minimum_score
    }

    pub const fn disposition(&self) -> PrecheckDisposition {
        self.disposition
    }

    pub fn failures(&self) -> &[PrecheckFailure] {
        &self.failures
    }

    pub const fn evaluated_at(&self) -> Timestamp {
        self.evaluated_at
    }

    pub fn digest(&self) -> &RequestDigest {
        &self.digest
    }

    pub const fn passed(&self) -> bool {
        matches!(self.disposition, PrecheckDisposition::Passed)
    }

    pub const fn can_authorize_permit(&self) -> bool {
        false
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PopulationEvaluation {
    request: PopulationMutationRequest,
    parents: Vec<ArtifactId>,
    training_failures: Vec<FailureId>,
    precheck: MutationPrecheckReceipt,
    evaluation: Option<EvaluationReceipt>,
    holdout_digest: Option<RequestDigest>,
    holdout_count: Option<usize>,
    usage: Usage,
}

impl PopulationEvaluation {
    #[allow(clippy::too_many_arguments)]
    pub fn evaluated(
        request: PopulationMutationRequest,
        mut parents: Vec<ArtifactId>,
        mut training_failures: Vec<FailureId>,
        precheck: MutationPrecheckReceipt,
        evaluation: EvaluationReceipt,
        holdout_digest: RequestDigest,
        holdout_count: usize,
        usage: Usage,
    ) -> Result<Self, PopulationContractError> {
        validate_precheck(&request, &precheck, true)?;
        validate_candidate_links(&request, &mut parents, &mut training_failures)?;
        Ok(Self {
            request,
            parents,
            training_failures,
            precheck,
            evaluation: Some(evaluation),
            holdout_digest: Some(holdout_digest),
            holdout_count: Some(holdout_count),
            usage,
        })
    }

    pub fn precheck_rejected(
        request: PopulationMutationRequest,
        precheck: MutationPrecheckReceipt,
        usage: Usage,
    ) -> Result<Self, PopulationContractError> {
        validate_precheck(&request, &precheck, false)?;
        Ok(Self {
            request,
            parents: Vec::new(),
            training_failures: Vec::new(),
            precheck,
            evaluation: None,
            holdout_digest: None,
            holdout_count: None,
            usage,
        })
    }

    pub fn request(&self) -> &PopulationMutationRequest {
        &self.request
    }

    pub fn parents(&self) -> &[ArtifactId] {
        &self.parents
    }

    pub fn training_failures(&self) -> &[FailureId] {
        &self.training_failures
    }

    pub fn precheck(&self) -> &MutationPrecheckReceipt {
        &self.precheck
    }

    pub fn evaluation(&self) -> Option<&EvaluationReceipt> {
        self.evaluation.as_ref()
    }

    pub fn holdout_digest(&self) -> Option<&RequestDigest> {
        self.holdout_digest.as_ref()
    }

    pub const fn holdout_count(&self) -> Option<usize> {
        self.holdout_count
    }

    pub const fn usage(&self) -> Usage {
        self.usage
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum CandidateDisposition {
    Accepted,
    RejectedPrecheck,
    RejectedEvaluation,
}

impl CandidateDisposition {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::RejectedPrecheck => "rejected_precheck",
            Self::RejectedEvaluation => "rejected_evaluation",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandidateOutcome {
    candidate_artifact: ArtifactId,
    parent_artifact: ArtifactId,
    disposition: CandidateDisposition,
    precheck_digest: RequestDigest,
    evaluation_digest: Option<RequestDigest>,
    score: Option<u8>,
    usage: Usage,
}

impl CandidateOutcome {
    pub fn new(
        candidate_artifact: ArtifactId,
        parent_artifact: ArtifactId,
        disposition: CandidateDisposition,
        precheck_digest: RequestDigest,
        evaluation_digest: Option<RequestDigest>,
        score: Option<u8>,
        usage: Usage,
    ) -> Result<Self, PopulationContractError> {
        if score.is_some_and(|score| score > 100) {
            return Err(PopulationContractError::InvalidScore);
        }
        let has_evaluation = evaluation_digest.is_some() && score.is_some();
        let valid = match disposition {
            CandidateDisposition::Accepted | CandidateDisposition::RejectedEvaluation => {
                has_evaluation
            }
            CandidateDisposition::RejectedPrecheck => {
                evaluation_digest.is_none() && score.is_none()
            }
        };
        if !valid {
            return Err(PopulationContractError::InvalidOutcomeShape);
        }
        Ok(Self {
            candidate_artifact,
            parent_artifact,
            disposition,
            precheck_digest,
            evaluation_digest,
            score,
            usage,
        })
    }

    pub fn candidate_artifact(&self) -> &ArtifactId {
        &self.candidate_artifact
    }

    pub fn parent_artifact(&self) -> &ArtifactId {
        &self.parent_artifact
    }

    pub const fn disposition(&self) -> CandidateDisposition {
        self.disposition
    }

    pub fn precheck_digest(&self) -> &RequestDigest {
        &self.precheck_digest
    }

    pub fn evaluation_digest(&self) -> Option<&RequestDigest> {
        self.evaluation_digest.as_ref()
    }

    pub const fn score(&self) -> Option<u8> {
        self.score
    }

    pub const fn usage(&self) -> Usage {
        self.usage
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GenerationStats {
    attempted: usize,
    accepted: usize,
    precheck_rejected: usize,
    evaluation_rejected: usize,
    usage: Usage,
}

impl GenerationStats {
    pub const fn new(
        attempted: usize,
        accepted: usize,
        precheck_rejected: usize,
        evaluation_rejected: usize,
        usage: Usage,
    ) -> Self {
        Self {
            attempted,
            accepted,
            precheck_rejected,
            evaluation_rejected,
            usage,
        }
    }

    pub const fn attempted(self) -> usize {
        self.attempted
    }

    pub const fn accepted(self) -> usize {
        self.accepted
    }

    pub const fn precheck_rejected(self) -> usize {
        self.precheck_rejected
    }

    pub const fn evaluation_rejected(self) -> usize {
        self.evaluation_rejected
    }

    pub const fn usage(self) -> Usage {
        self.usage
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GenerationReceipt {
    protocol_version: u16,
    population_id: PopulationId,
    generation: u32,
    plan_digest: RequestDigest,
    starting_population_digest: RequestDigest,
    resulting_population_digest: RequestDigest,
    outcomes: Vec<CandidateOutcome>,
    stats: GenerationStats,
    completed_at: Timestamp,
    digest: RequestDigest,
}

impl GenerationReceipt {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        population_id: PopulationId,
        generation: u32,
        plan_digest: RequestDigest,
        starting_population_digest: RequestDigest,
        resulting_population_digest: RequestDigest,
        mut outcomes: Vec<CandidateOutcome>,
        stats: GenerationStats,
        completed_at: Timestamp,
    ) -> Result<Self, PopulationContractError> {
        outcomes.sort_by(|left, right| left.candidate_artifact().cmp(right.candidate_artifact()));
        for pair in outcomes.windows(2) {
            if pair[0].candidate_artifact() == pair[1].candidate_artifact() {
                return Err(PopulationContractError::DuplicateCandidate(
                    pair[0].candidate_artifact().clone(),
                ));
            }
        }
        if stats != generation_stats(&outcomes)? {
            return Err(PopulationContractError::InvalidGenerationStats);
        }
        let digest = digest_generation_receipt(
            &population_id,
            generation,
            &plan_digest,
            &starting_population_digest,
            &resulting_population_digest,
            &outcomes,
            stats,
            completed_at,
        );
        Ok(Self {
            protocol_version: POPULATION_PROTOCOL_VERSION,
            population_id,
            generation,
            plan_digest,
            starting_population_digest,
            resulting_population_digest,
            outcomes,
            stats,
            completed_at,
            digest,
        })
    }

    pub const fn protocol_version(&self) -> u16 {
        self.protocol_version
    }

    pub fn population_id(&self) -> &PopulationId {
        &self.population_id
    }

    pub const fn generation(&self) -> u32 {
        self.generation
    }

    pub fn plan_digest(&self) -> &RequestDigest {
        &self.plan_digest
    }

    pub fn starting_population_digest(&self) -> &RequestDigest {
        &self.starting_population_digest
    }

    pub fn resulting_population_digest(&self) -> &RequestDigest {
        &self.resulting_population_digest
    }

    pub fn outcomes(&self) -> &[CandidateOutcome] {
        &self.outcomes
    }

    pub const fn stats(&self) -> GenerationStats {
        self.stats
    }

    pub const fn completed_at(&self) -> Timestamp {
        self.completed_at
    }

    pub fn digest(&self) -> &RequestDigest {
        &self.digest
    }

    pub const fn can_authorize_permit(&self) -> bool {
        false
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FailureEvidence {
    id: FailureId,
    partition: FailurePartition,
    category: String,
    redacted_summary: String,
    evidence_digest: RequestDigest,
    observed_at: Timestamp,
}

impl FailureEvidence {
    pub fn new(
        id: FailureId,
        partition: FailurePartition,
        category: impl Into<String>,
        redacted_summary: impl Into<String>,
        evidence_digest: RequestDigest,
        observed_at: Timestamp,
    ) -> Result<Self, PopulationContractError> {
        Ok(Self {
            id,
            partition,
            category: validate_text("failure category", category.into(), MAX_CATEGORY_BYTES)?,
            redacted_summary: validate_text(
                "redacted failure summary",
                redacted_summary.into(),
                MAX_SUMMARY_BYTES,
            )?,
            evidence_digest,
            observed_at,
        })
    }

    pub fn id(&self) -> &FailureId {
        &self.id
    }

    pub const fn partition(&self) -> FailurePartition {
        self.partition
    }

    pub fn category(&self) -> &str {
        &self.category
    }

    pub fn redacted_summary(&self) -> &str {
        &self.redacted_summary
    }

    pub fn evidence_digest(&self) -> &RequestDigest {
        &self.evidence_digest
    }

    pub const fn observed_at(&self) -> Timestamp {
        self.observed_at
    }

    fn context_bytes(&self) -> usize {
        self.id.as_str().len()
            + self.category.len()
            + self.redacted_summary.len()
            + self.evidence_digest.as_str().len()
            + 4
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FailureCorpus {
    failures: Vec<FailureEvidence>,
    holdout_digest: RequestDigest,
    holdout_count: usize,
}

impl FailureCorpus {
    pub fn new(mut failures: Vec<FailureEvidence>) -> Result<Self, PopulationContractError> {
        if failures.len() > MAX_FAILURES {
            return Err(PopulationContractError::TooManyFailures);
        }
        failures.sort_by(|left, right| left.id().cmp(right.id()));
        for pair in failures.windows(2) {
            if pair[0].id() == pair[1].id() {
                return Err(PopulationContractError::DuplicateFailure(
                    pair[0].id().clone(),
                ));
            }
        }
        let holdout = failures
            .iter()
            .filter(|failure| failure.partition() == FailurePartition::Holdout)
            .collect::<Vec<_>>();
        let holdout_digest = digest_failures(&holdout)?;
        Ok(Self {
            holdout_count: holdout.len(),
            failures,
            holdout_digest,
        })
    }

    pub fn mutation_batch(
        &self,
        eligible: &[FailureId],
        max_failures: usize,
        max_bytes: usize,
    ) -> MutationBatch {
        let eligible = eligible.iter().collect::<BTreeSet<_>>();
        let mut groups: BTreeMap<&str, Vec<&FailureEvidence>> = BTreeMap::new();
        for failure in self.failures.iter().filter(|failure| {
            failure.partition() == FailurePartition::Training && eligible.contains(failure.id())
        }) {
            groups.entry(failure.category()).or_default().push(failure);
        }
        let selected_category = groups
            .iter()
            .max_by(|(left_name, left), (right_name, right)| {
                left.len()
                    .cmp(&right.len())
                    .then_with(|| right_name.cmp(left_name))
            })
            .map(|(category, _)| *category)
            .unwrap_or("");
        let mut context_bytes = 0usize;
        let mut failures = Vec::new();
        if let Some(group) = groups.get(selected_category) {
            for failure in group {
                if failures.len() >= max_failures {
                    break;
                }
                let Some(next_bytes) = context_bytes.checked_add(failure.context_bytes()) else {
                    break;
                };
                if next_bytes > max_bytes {
                    break;
                }
                context_bytes = next_bytes;
                failures.push((*failure).clone());
            }
        }
        let refs = failures.iter().collect::<Vec<_>>();
        MutationBatch {
            category: selected_category.to_owned(),
            digest: digest_failures(&refs).expect("bounded failure digest is valid"),
            failures,
            context_bytes,
        }
    }

    pub const fn holdout_count(&self) -> usize {
        self.holdout_count
    }

    pub fn holdout_digest(&self) -> &RequestDigest {
        &self.holdout_digest
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MutationBatch {
    category: String,
    failures: Vec<FailureEvidence>,
    context_bytes: usize,
    digest: RequestDigest,
}

impl MutationBatch {
    pub fn category(&self) -> &str {
        &self.category
    }

    pub fn failures(&self) -> &[FailureEvidence] {
        &self.failures
    }

    pub const fn context_bytes(&self) -> usize {
        self.context_bytes
    }

    pub fn digest(&self) -> &RequestDigest {
        &self.digest
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MutationLimits {
    max_failures_per_batch: usize,
    max_batch_bytes: usize,
    max_mutations_per_parent: usize,
}

impl MutationLimits {
    pub const fn new(
        max_failures_per_batch: usize,
        max_batch_bytes: usize,
        max_mutations_per_parent: usize,
    ) -> Result<Self, PopulationContractError> {
        if max_failures_per_batch == 0 {
            return Err(PopulationContractError::InvalidLimit(
                "max failures per batch",
            ));
        }
        if max_batch_bytes == 0 {
            return Err(PopulationContractError::InvalidLimit("max batch bytes"));
        }
        if max_mutations_per_parent == 0 {
            return Err(PopulationContractError::InvalidLimit(
                "max mutations per parent",
            ));
        }
        Ok(Self {
            max_failures_per_batch,
            max_batch_bytes,
            max_mutations_per_parent,
        })
    }

    pub const fn max_failures_per_batch(self) -> usize {
        self.max_failures_per_batch
    }

    pub const fn max_batch_bytes(self) -> usize {
        self.max_batch_bytes
    }

    pub const fn max_mutations_per_parent(self) -> usize {
        self.max_mutations_per_parent
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LineageLimits {
    max_depth: u32,
    max_records: usize,
    max_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineageDirection {
    Ancestors,
    Neighborhood,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineageMemory {
    L1,
    L2,
    Both,
}

impl LineageMemory {
    pub const fn includes(self, tier: MemoryTier) -> bool {
        matches!(
            (self, tier),
            (Self::L1, MemoryTier::L1)
                | (Self::L2, MemoryTier::L2)
                | (Self::Both, MemoryTier::L1 | MemoryTier::L2)
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LineageQuery {
    population_id: PopulationId,
    artifact_id: ArtifactId,
    direction: LineageDirection,
    memory: LineageMemory,
    max_depth: u32,
    max_records: usize,
    max_bytes: usize,
}

impl LineageQuery {
    pub fn new(
        population_id: PopulationId,
        artifact_id: ArtifactId,
        direction: LineageDirection,
        memory: LineageMemory,
        max_depth: u32,
        max_records: usize,
        max_bytes: usize,
    ) -> Result<Self, PopulationContractError> {
        if max_depth == 0 {
            return Err(PopulationContractError::InvalidLimit("lineage depth"));
        }
        if max_records == 0 {
            return Err(PopulationContractError::InvalidLimit("lineage records"));
        }
        if max_bytes == 0 {
            return Err(PopulationContractError::InvalidLimit("lineage bytes"));
        }
        Ok(Self {
            population_id,
            artifact_id,
            direction,
            memory,
            max_depth,
            max_records,
            max_bytes,
        })
    }

    pub fn population_id(&self) -> &PopulationId {
        &self.population_id
    }

    pub fn artifact_id(&self) -> &ArtifactId {
        &self.artifact_id
    }

    pub const fn direction(&self) -> LineageDirection {
        self.direction
    }

    pub const fn memory(&self) -> LineageMemory {
        self.memory
    }

    pub const fn max_depth(&self) -> u32 {
        self.max_depth
    }

    pub const fn max_records(&self) -> usize {
        self.max_records
    }

    pub const fn max_bytes(&self) -> usize {
        self.max_bytes
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LineageAttempt {
    artifact_id: ArtifactId,
    redacted_change: String,
}

impl LineageAttempt {
    pub fn new(
        artifact_id: ArtifactId,
        redacted_change: impl Into<String>,
    ) -> Result<Self, PopulationContractError> {
        Ok(Self {
            artifact_id,
            redacted_change: validate_text(
                "redacted lineage change",
                redacted_change.into(),
                MAX_SUMMARY_BYTES,
            )?,
        })
    }

    pub fn artifact_id(&self) -> &ArtifactId {
        &self.artifact_id
    }

    pub fn redacted_change(&self) -> &str {
        &self.redacted_change
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LineageNode {
    artifact_id: ArtifactId,
    distance: u32,
}

impl LineageNode {
    pub const fn new(artifact_id: ArtifactId, distance: u32) -> Self {
        Self {
            artifact_id,
            distance,
        }
    }

    pub fn artifact_id(&self) -> &ArtifactId {
        &self.artifact_id
    }

    pub const fn distance(&self) -> u32 {
        self.distance
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LineageLesson {
    memory_id: MemoryId,
    artifact_id: ArtifactId,
    distance: u32,
    tier: MemoryTier,
    kind: MemoryKind,
    summary: String,
    created_at: Timestamp,
    provenance: RequestDigest,
}

impl LineageLesson {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        memory_id: MemoryId,
        artifact_id: ArtifactId,
        distance: u32,
        tier: MemoryTier,
        kind: MemoryKind,
        summary: impl Into<String>,
        created_at: Timestamp,
        provenance: RequestDigest,
    ) -> Result<Self, PopulationContractError> {
        Ok(Self {
            memory_id,
            artifact_id,
            distance,
            tier,
            kind,
            summary: validate_text("lineage summary", summary.into(), MAX_LINEAGE_SUMMARY_BYTES)?,
            created_at,
            provenance,
        })
    }

    pub fn memory_id(&self) -> &MemoryId {
        &self.memory_id
    }

    pub fn artifact_id(&self) -> &ArtifactId {
        &self.artifact_id
    }

    pub const fn distance(&self) -> u32 {
        self.distance
    }

    pub const fn tier(&self) -> MemoryTier {
        self.tier
    }

    pub const fn kind(&self) -> MemoryKind {
        self.kind
    }

    pub fn summary(&self) -> &str {
        &self.summary
    }

    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }

    pub fn provenance(&self) -> &RequestDigest {
        &self.provenance
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LineageView {
    population_id: PopulationId,
    root_artifact: ArtifactId,
    direction: LineageDirection,
    nodes: Vec<LineageNode>,
    lessons: Vec<LineageLesson>,
    total_bytes: usize,
    generated_at: Timestamp,
}

impl LineageView {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        population_id: PopulationId,
        root_artifact: ArtifactId,
        direction: LineageDirection,
        nodes: Vec<LineageNode>,
        lessons: Vec<LineageLesson>,
        total_bytes: usize,
        generated_at: Timestamp,
    ) -> Self {
        Self {
            population_id,
            root_artifact,
            direction,
            nodes,
            lessons,
            total_bytes,
            generated_at,
        }
    }

    pub fn population_id(&self) -> &PopulationId {
        &self.population_id
    }

    pub fn root_artifact(&self) -> &ArtifactId {
        &self.root_artifact
    }

    pub const fn direction(&self) -> LineageDirection {
        self.direction
    }

    pub fn nodes(&self) -> &[LineageNode] {
        &self.nodes
    }

    pub fn lessons(&self) -> &[LineageLesson] {
        &self.lessons
    }

    pub const fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    pub const fn generated_at(&self) -> Timestamp {
        self.generated_at
    }
}

impl LineageLimits {
    pub const fn new(
        max_depth: u32,
        max_records: usize,
        max_bytes: usize,
    ) -> Result<Self, PopulationContractError> {
        if max_depth == 0 {
            return Err(PopulationContractError::InvalidLimit("max lineage depth"));
        }
        if max_records == 0 {
            return Err(PopulationContractError::InvalidLimit("max lineage records"));
        }
        if max_bytes == 0 {
            return Err(PopulationContractError::InvalidLimit("max lineage bytes"));
        }
        Ok(Self {
            max_depth,
            max_records,
            max_bytes,
        })
    }

    pub const fn max_depth(self) -> u32 {
        self.max_depth
    }

    pub const fn max_records(self) -> usize {
        self.max_records
    }

    pub const fn max_bytes(self) -> usize {
        self.max_bytes
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PopulationPolicy {
    max_candidates: usize,
    max_parents: usize,
    max_generations: u32,
    max_evaluations: usize,
    mutation: MutationLimits,
    lineage: LineageLimits,
    min_precheck_score: u8,
    novelty_weight_basis_points: u32,
    max_usage: Usage,
}

impl PopulationPolicy {
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        max_candidates: usize,
        max_parents: usize,
        max_generations: u32,
        max_evaluations: usize,
        mutation: MutationLimits,
        lineage: LineageLimits,
        min_precheck_score: u8,
        novelty_weight_basis_points: u32,
        max_usage: Usage,
    ) -> Result<Self, PopulationContractError> {
        if max_candidates == 0 {
            return Err(PopulationContractError::InvalidLimit("max candidates"));
        }
        if max_parents == 0 || max_parents > max_candidates {
            return Err(PopulationContractError::InvalidLimit("max parents"));
        }
        if max_generations == 0 {
            return Err(PopulationContractError::InvalidLimit("max generations"));
        }
        if max_evaluations == 0 {
            return Err(PopulationContractError::InvalidLimit("max evaluations"));
        }
        if min_precheck_score > 100 {
            return Err(PopulationContractError::InvalidScore);
        }
        if novelty_weight_basis_points == 0 || novelty_weight_basis_points > 10_000 {
            return Err(PopulationContractError::InvalidLimit(
                "novelty weight basis points",
            ));
        }
        if max_usage.tokens() == 0
            || max_usage.tools() == 0
            || max_usage.duration_seconds() == 0
            || max_usage.cost_micros() == 0
        {
            return Err(PopulationContractError::InvalidLimit("max usage"));
        }
        Ok(Self {
            max_candidates,
            max_parents,
            max_generations,
            max_evaluations,
            mutation,
            lineage,
            min_precheck_score,
            novelty_weight_basis_points,
            max_usage,
        })
    }

    pub const fn max_candidates(self) -> usize {
        self.max_candidates
    }

    pub const fn max_parents(self) -> usize {
        self.max_parents
    }

    pub const fn max_generations(self) -> u32 {
        self.max_generations
    }

    pub const fn max_evaluations(self) -> usize {
        self.max_evaluations
    }

    pub const fn mutation(self) -> MutationLimits {
        self.mutation
    }

    pub const fn lineage(self) -> LineageLimits {
        self.lineage
    }

    pub const fn min_precheck_score(self) -> u8 {
        self.min_precheck_score
    }

    pub const fn novelty_weight_basis_points(self) -> u32 {
        self.novelty_weight_basis_points
    }

    pub const fn max_usage(self) -> Usage {
        self.max_usage
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PopulationScope {
    population_id: PopulationId,
    tenant_id: TenantId,
    workspace_id: WorkspaceId,
    session_id: SessionId,
}

impl PopulationScope {
    pub const fn new(
        population_id: PopulationId,
        tenant_id: TenantId,
        workspace_id: WorkspaceId,
        session_id: SessionId,
    ) -> Self {
        Self {
            population_id,
            tenant_id,
            workspace_id,
            session_id,
        }
    }

    pub fn population_id(&self) -> &PopulationId {
        &self.population_id
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PopulationCandidate {
    artifact_id: ArtifactId,
    parents: Vec<ArtifactId>,
    generation: u32,
    score: u8,
    viable: bool,
    child_count: u32,
    evaluation_digest: RequestDigest,
    training_failures: Vec<FailureId>,
}

impl PopulationCandidate {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        artifact_id: ArtifactId,
        mut parents: Vec<ArtifactId>,
        generation: u32,
        score: u8,
        viable: bool,
        child_count: u32,
        evaluation_digest: RequestDigest,
        mut training_failures: Vec<FailureId>,
    ) -> Result<Self, PopulationContractError> {
        if score > 100 {
            return Err(PopulationContractError::InvalidScore);
        }
        if parents.len() > MAX_PARENTS {
            return Err(PopulationContractError::TooManyParents);
        }
        parents.sort();
        for parent in &parents {
            if parent == &artifact_id {
                return Err(PopulationContractError::ParentMatchesCandidate);
            }
        }
        for pair in parents.windows(2) {
            if pair[0] == pair[1] {
                return Err(PopulationContractError::DuplicateParent(pair[0].clone()));
            }
        }
        training_failures.sort();
        for pair in training_failures.windows(2) {
            if pair[0] == pair[1] {
                return Err(PopulationContractError::DuplicateTrainingFailure(
                    pair[0].clone(),
                ));
            }
        }
        Ok(Self {
            artifact_id,
            parents,
            generation,
            score,
            viable,
            child_count,
            evaluation_digest,
            training_failures,
        })
    }

    pub fn artifact_id(&self) -> &ArtifactId {
        &self.artifact_id
    }

    pub fn parents(&self) -> &[ArtifactId] {
        &self.parents
    }

    pub const fn generation(&self) -> u32 {
        self.generation
    }

    pub const fn score(&self) -> u8 {
        self.score
    }

    pub const fn viable(&self) -> bool {
        self.viable
    }

    pub const fn child_count(&self) -> u32 {
        self.child_count
    }

    pub fn evaluation_digest(&self) -> &RequestDigest {
        &self.evaluation_digest
    }

    pub fn training_failures(&self) -> &[FailureId] {
        &self.training_failures
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandidatePopulation {
    scope: PopulationScope,
    generation: u32,
    candidates: Vec<PopulationCandidate>,
}

impl CandidatePopulation {
    pub fn new(
        scope: PopulationScope,
        generation: u32,
        mut candidates: Vec<PopulationCandidate>,
    ) -> Result<Self, PopulationContractError> {
        candidates.sort_by(|left, right| left.artifact_id().cmp(right.artifact_id()));
        for pair in candidates.windows(2) {
            if pair[0].artifact_id() == pair[1].artifact_id() {
                return Err(PopulationContractError::DuplicateCandidate(
                    pair[0].artifact_id().clone(),
                ));
            }
        }
        Ok(Self {
            scope,
            generation,
            candidates,
        })
    }

    pub fn scope(&self) -> &PopulationScope {
        &self.scope
    }

    pub const fn generation(&self) -> u32 {
        self.generation
    }

    pub fn candidates(&self) -> &[PopulationCandidate] {
        &self.candidates
    }
}

fn validate_precheck(
    request: &PopulationMutationRequest,
    precheck: &MutationPrecheckReceipt,
    expected_passed: bool,
) -> Result<(), PopulationContractError> {
    if request.request_digest() != precheck.request_digest() {
        return Err(PopulationContractError::PrecheckRequestMismatch);
    }
    if precheck.passed() != expected_passed {
        return Err(PopulationContractError::InvalidPrecheckDisposition);
    }
    Ok(())
}

fn validate_candidate_links(
    request: &PopulationMutationRequest,
    parents: &mut [ArtifactId],
    training_failures: &mut [FailureId],
) -> Result<(), PopulationContractError> {
    if parents.len() > MAX_PARENTS {
        return Err(PopulationContractError::TooManyParents);
    }
    parents.sort();
    for parent in parents.iter() {
        if parent == request.candidate_artifact() {
            return Err(PopulationContractError::ParentMatchesCandidate);
        }
    }
    for pair in parents.windows(2) {
        if pair[0] == pair[1] {
            return Err(PopulationContractError::DuplicateParent(pair[0].clone()));
        }
    }
    training_failures.sort();
    for pair in training_failures.windows(2) {
        if pair[0] == pair[1] {
            return Err(PopulationContractError::DuplicateTrainingFailure(
                pair[0].clone(),
            ));
        }
    }
    Ok(())
}

fn digest_failures(
    failures: &[&FailureEvidence],
) -> Result<RequestDigest, PopulationContractError> {
    let mut hasher = Sha256::new();
    hasher.update(POPULATION_PROTOCOL_VERSION.to_be_bytes());
    for failure in failures {
        hasher.update(failure.id().as_str().as_bytes());
        hasher.update([0]);
        hasher.update(failure.category().as_bytes());
        hasher.update([0]);
        hasher.update(failure.redacted_summary().as_bytes());
        hasher.update([0]);
        hasher.update(failure.evidence_digest().as_str().as_bytes());
        hasher.update([failure.partition() as u8]);
    }
    Ok(RequestDigest::new(format!(
        "sha256:{:x}",
        hasher.finalize()
    ))?)
}

fn digest_mutation_request(
    population_id: &PopulationId,
    generation: u32,
    parent_artifact: &ArtifactId,
    candidate_artifact: &ArtifactId,
    plan_digest: &RequestDigest,
    mutation_batch_digest: &RequestDigest,
) -> RequestDigest {
    let mut hasher = Sha256::new();
    hasher.update(POPULATION_PROTOCOL_VERSION.to_be_bytes());
    update_text(&mut hasher, population_id.as_str());
    hasher.update(generation.to_be_bytes());
    update_text(&mut hasher, parent_artifact.as_str());
    update_text(&mut hasher, candidate_artifact.as_str());
    update_text(&mut hasher, plan_digest.as_str());
    update_text(&mut hasher, mutation_batch_digest.as_str());
    sha256_request_digest(hasher)
}

#[allow(clippy::too_many_arguments)]
fn digest_precheck(
    request_digest: &RequestDigest,
    session_id: &SessionId,
    execution_id: &ExecutionId,
    evaluation_digest: &RequestDigest,
    minimum_score: u8,
    disposition: PrecheckDisposition,
    failures: &[PrecheckFailure],
    evaluated_at: Timestamp,
) -> RequestDigest {
    let mut hasher = Sha256::new();
    hasher.update(POPULATION_PROTOCOL_VERSION.to_be_bytes());
    update_text(&mut hasher, request_digest.as_str());
    update_text(&mut hasher, session_id.as_str());
    update_text(&mut hasher, execution_id.as_str());
    update_text(&mut hasher, evaluation_digest.as_str());
    hasher.update([minimum_score]);
    update_text(&mut hasher, disposition.as_str());
    hasher.update((failures.len() as u64).to_be_bytes());
    for failure in failures {
        update_text(&mut hasher, failure.as_str());
        update_text(&mut hasher, failure.kind().as_str());
    }
    hasher.update(evaluated_at.as_unix_seconds().to_be_bytes());
    sha256_request_digest(hasher)
}

#[allow(clippy::too_many_arguments)]
fn digest_generation_receipt(
    population_id: &PopulationId,
    generation: u32,
    plan_digest: &RequestDigest,
    starting_population_digest: &RequestDigest,
    resulting_population_digest: &RequestDigest,
    outcomes: &[CandidateOutcome],
    stats: GenerationStats,
    completed_at: Timestamp,
) -> RequestDigest {
    let mut hasher = Sha256::new();
    hasher.update(POPULATION_PROTOCOL_VERSION.to_be_bytes());
    update_text(&mut hasher, population_id.as_str());
    hasher.update(generation.to_be_bytes());
    update_text(&mut hasher, plan_digest.as_str());
    update_text(&mut hasher, starting_population_digest.as_str());
    update_text(&mut hasher, resulting_population_digest.as_str());
    hasher.update((outcomes.len() as u64).to_be_bytes());
    for outcome in outcomes {
        update_text(&mut hasher, outcome.candidate_artifact().as_str());
        update_text(&mut hasher, outcome.parent_artifact().as_str());
        update_text(&mut hasher, outcome.disposition().as_str());
        update_text(&mut hasher, outcome.precheck_digest().as_str());
        hash_optional_digest(&mut hasher, outcome.evaluation_digest());
        match outcome.score() {
            Some(score) => hasher.update([1, score]),
            None => hasher.update([0, 0]),
        }
        hash_usage(&mut hasher, outcome.usage());
    }
    hasher.update((stats.attempted() as u64).to_be_bytes());
    hasher.update((stats.accepted() as u64).to_be_bytes());
    hasher.update((stats.precheck_rejected() as u64).to_be_bytes());
    hasher.update((stats.evaluation_rejected() as u64).to_be_bytes());
    hash_usage(&mut hasher, stats.usage());
    hasher.update(completed_at.as_unix_seconds().to_be_bytes());
    sha256_request_digest(hasher)
}

fn generation_stats(
    outcomes: &[CandidateOutcome],
) -> Result<GenerationStats, PopulationContractError> {
    let mut accepted = 0usize;
    let mut precheck_rejected = 0usize;
    let mut evaluation_rejected = 0usize;
    let mut usage = Usage::new(0, 0, 0, 0);
    for outcome in outcomes {
        match outcome.disposition() {
            CandidateDisposition::Accepted => accepted += 1,
            CandidateDisposition::RejectedPrecheck => precheck_rejected += 1,
            CandidateDisposition::RejectedEvaluation => evaluation_rejected += 1,
        }
        usage = Usage::new(
            usage
                .tokens()
                .checked_add(outcome.usage().tokens())
                .ok_or(PopulationContractError::UsageOverflow)?,
            usage
                .tools()
                .checked_add(outcome.usage().tools())
                .ok_or(PopulationContractError::UsageOverflow)?,
            usage
                .duration_seconds()
                .checked_add(outcome.usage().duration_seconds())
                .ok_or(PopulationContractError::UsageOverflow)?,
            usage
                .cost_micros()
                .checked_add(outcome.usage().cost_micros())
                .ok_or(PopulationContractError::UsageOverflow)?,
        );
    }
    Ok(GenerationStats::new(
        outcomes.len(),
        accepted,
        precheck_rejected,
        evaluation_rejected,
        usage,
    ))
}

fn hash_optional_digest(hasher: &mut Sha256, digest: Option<&RequestDigest>) {
    match digest {
        Some(digest) => {
            hasher.update([1]);
            update_text(hasher, digest.as_str());
        }
        None => hasher.update([0]),
    }
}

fn hash_usage(hasher: &mut Sha256, usage: Usage) {
    hasher.update(usage.tokens().to_be_bytes());
    hasher.update(usage.tools().to_be_bytes());
    hasher.update(usage.duration_seconds().to_be_bytes());
    hasher.update(usage.cost_micros().to_be_bytes());
}

fn update_text(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value.as_bytes());
}

fn sha256_request_digest(hasher: Sha256) -> RequestDigest {
    RequestDigest::new(format!("sha256:{:x}", hasher.finalize()))
        .expect("SHA-256 request digest is valid")
}

fn validate_text(
    field: &'static str,
    value: String,
    max_bytes: usize,
) -> Result<String, PopulationContractError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(PopulationContractError::EmptyField(field));
    }
    if value.len() > max_bytes {
        return Err(PopulationContractError::FieldTooLong(field));
    }
    if value.chars().any(char::is_control) {
        return Err(PopulationContractError::ControlCharacter(field));
    }
    Ok(trimmed.to_owned())
}
