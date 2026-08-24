use crate::{
    ArtifactId, FailureId, IdError, PopulationId, RequestDigest, SessionId, TenantId, Timestamp,
    Usage, WorkspaceId,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const POPULATION_PROTOCOL_VERSION: u16 = 1;
const MAX_FAILURES: usize = 256;
const MAX_CATEGORY_BYTES: usize = 256;
const MAX_SUMMARY_BYTES: usize = 4096;
const MAX_PARENTS: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailurePartition {
    Training,
    Holdout,
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
    ParentMatchesCandidate,
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
            Self::ParentMatchesCandidate => {
                formatter.write_str("candidate artifact cannot be its own parent")
            }
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
