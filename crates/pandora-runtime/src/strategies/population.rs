use super::StrategyProfile;
use pandora_types::{
    ArtifactId, CandidatePopulation, FailureCorpus, MutationBatch, POPULATION_PROTOCOL_VERSION,
    PopulationPolicy, RequestDigest,
};
use sha2::{Digest, Sha256};
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PopulationStrategyError {
    DisabledInProduction,
    EmptyPopulation,
    CandidateLimitExceeded,
    GenerationLimitExceeded,
    CandidateGenerationAhead(ArtifactId),
    ParentLimitExceeded(ArtifactId),
    NoViableCandidates,
}

impl fmt::Display for PopulationStrategyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DisabledInProduction => {
                formatter.write_str("population evolution is disabled in production")
            }
            Self::EmptyPopulation => formatter.write_str("candidate population is empty"),
            Self::CandidateLimitExceeded => {
                formatter.write_str("candidate population exceeds its policy limit")
            }
            Self::GenerationLimitExceeded => {
                formatter.write_str("population generation limit is exhausted")
            }
            Self::CandidateGenerationAhead(artifact_id) => write!(
                formatter,
                "candidate {} is ahead of the population generation",
                artifact_id.as_str()
            ),
            Self::ParentLimitExceeded(artifact_id) => write!(
                formatter,
                "candidate {} exceeds the parent limit",
                artifact_id.as_str()
            ),
            Self::NoViableCandidates => formatter.write_str("population has no viable candidates"),
        }
    }
}

impl std::error::Error for PopulationStrategyError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PopulationStrategy {
    profile: StrategyProfile,
    policy: PopulationPolicy,
}

impl PopulationStrategy {
    pub const fn new(profile: StrategyProfile, policy: PopulationPolicy) -> Self {
        Self { profile, policy }
    }

    pub const fn profile(self) -> StrategyProfile {
        self.profile
    }

    pub const fn policy(self) -> PopulationPolicy {
        self.policy
    }

    pub fn plan(
        &self,
        population: &CandidatePopulation,
        corpus: &FailureCorpus,
    ) -> Result<PopulationPlan, PopulationStrategyError> {
        self.validate_population(population)?;

        let mut ranked = population
            .candidates()
            .iter()
            .filter(|candidate| candidate.viable())
            .map(|candidate| {
                let novelty_score = u64::from(self.policy.novelty_weight_basis_points())
                    / (u64::from(candidate.child_count()) + 1);
                let selection_score = u64::from(candidate.score()) * 10_000 + novelty_score;
                (candidate, selection_score, novelty_score)
            })
            .collect::<Vec<_>>();
        if ranked.is_empty() {
            return Err(PopulationStrategyError::NoViableCandidates);
        }
        ranked.sort_by(|left, right| {
            right
                .1
                .cmp(&left.1)
                .then_with(|| left.0.artifact_id().cmp(right.0.artifact_id()))
        });
        ranked.truncate(self.policy.max_parents());

        let population_digest = digest_population(population);
        let mut remaining_candidates = self
            .policy
            .max_candidates()
            .min(self.policy.max_evaluations());
        let mut parents = Vec::with_capacity(ranked.len());
        let mut planned_candidates = 0usize;
        for (candidate, selection_score, novelty_score) in ranked {
            let batch = corpus.mutation_batch(
                candidate.training_failures(),
                self.policy.mutation().max_failures_per_batch(),
                self.policy.mutation().max_batch_bytes(),
            );
            let planned_mutations = batch
                .failures()
                .len()
                .min(self.policy.mutation().max_mutations_per_parent())
                .min(remaining_candidates);
            remaining_candidates -= planned_mutations;
            planned_candidates += planned_mutations;
            parents.push(PopulationParentPlan {
                artifact_id: candidate.artifact_id().clone(),
                selection_score,
                novelty_score,
                mutation_batch: batch,
                planned_mutations,
            });
        }

        let next_generation = population.generation() + 1;
        let plan_digest = digest_plan(
            &population_digest,
            corpus.holdout_digest(),
            next_generation,
            planned_candidates,
            &parents,
        );
        Ok(PopulationPlan {
            starting_generation: population.generation(),
            next_generation,
            population_digest,
            holdout_digest: corpus.holdout_digest().clone(),
            parents,
            planned_candidates,
            plan_digest,
        })
    }

    fn validate_population(
        &self,
        population: &CandidatePopulation,
    ) -> Result<(), PopulationStrategyError> {
        if self.profile == StrategyProfile::Production {
            return Err(PopulationStrategyError::DisabledInProduction);
        }
        if population.candidates().is_empty() {
            return Err(PopulationStrategyError::EmptyPopulation);
        }
        if population.candidates().len() > self.policy.max_candidates() {
            return Err(PopulationStrategyError::CandidateLimitExceeded);
        }
        if population.generation() >= self.policy.max_generations() {
            return Err(PopulationStrategyError::GenerationLimitExceeded);
        }
        for candidate in population.candidates() {
            if candidate.generation() > population.generation() {
                return Err(PopulationStrategyError::CandidateGenerationAhead(
                    candidate.artifact_id().clone(),
                ));
            }
            if candidate.parents().len() > self.policy.max_parents() {
                return Err(PopulationStrategyError::ParentLimitExceeded(
                    candidate.artifact_id().clone(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PopulationParentPlan {
    artifact_id: ArtifactId,
    selection_score: u64,
    novelty_score: u64,
    mutation_batch: MutationBatch,
    planned_mutations: usize,
}

impl PopulationParentPlan {
    pub fn artifact_id(&self) -> &ArtifactId {
        &self.artifact_id
    }

    pub const fn selection_score(&self) -> u64 {
        self.selection_score
    }

    pub const fn novelty_score(&self) -> u64 {
        self.novelty_score
    }

    pub fn mutation_batch(&self) -> &MutationBatch {
        &self.mutation_batch
    }

    pub const fn planned_mutations(&self) -> usize {
        self.planned_mutations
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PopulationPlan {
    starting_generation: u32,
    next_generation: u32,
    population_digest: RequestDigest,
    holdout_digest: RequestDigest,
    parents: Vec<PopulationParentPlan>,
    planned_candidates: usize,
    plan_digest: RequestDigest,
}

impl PopulationPlan {
    pub const fn starting_generation(&self) -> u32 {
        self.starting_generation
    }

    pub const fn next_generation(&self) -> u32 {
        self.next_generation
    }

    pub fn population_digest(&self) -> &RequestDigest {
        &self.population_digest
    }

    pub fn holdout_digest(&self) -> &RequestDigest {
        &self.holdout_digest
    }

    pub fn parents(&self) -> &[PopulationParentPlan] {
        &self.parents
    }

    pub const fn planned_candidates(&self) -> usize {
        self.planned_candidates
    }

    pub fn plan_digest(&self) -> &RequestDigest {
        &self.plan_digest
    }
}

fn digest_population(population: &CandidatePopulation) -> RequestDigest {
    let mut hasher = Sha256::new();
    hasher.update(POPULATION_PROTOCOL_VERSION.to_be_bytes());
    hash_text(&mut hasher, population.scope().population_id().as_str());
    hash_text(&mut hasher, population.scope().tenant_id().as_str());
    hash_text(&mut hasher, population.scope().workspace_id().as_str());
    hash_text(&mut hasher, population.scope().session_id().as_str());
    hasher.update(population.generation().to_be_bytes());
    hasher.update((population.candidates().len() as u64).to_be_bytes());
    for candidate in population.candidates() {
        hash_text(&mut hasher, candidate.artifact_id().as_str());
        hasher.update(candidate.generation().to_be_bytes());
        hasher.update([candidate.score(), u8::from(candidate.viable())]);
        hasher.update(candidate.child_count().to_be_bytes());
        hash_text(&mut hasher, candidate.evaluation_digest().as_str());
        hasher.update((candidate.parents().len() as u64).to_be_bytes());
        for parent in candidate.parents() {
            hash_text(&mut hasher, parent.as_str());
        }
        hasher.update((candidate.training_failures().len() as u64).to_be_bytes());
        for failure in candidate.training_failures() {
            hash_text(&mut hasher, failure.as_str());
        }
    }
    sha256_digest(hasher)
}

fn digest_plan(
    population_digest: &RequestDigest,
    holdout_digest: &RequestDigest,
    next_generation: u32,
    planned_candidates: usize,
    parents: &[PopulationParentPlan],
) -> RequestDigest {
    let mut hasher = Sha256::new();
    hasher.update(POPULATION_PROTOCOL_VERSION.to_be_bytes());
    hash_text(&mut hasher, population_digest.as_str());
    hash_text(&mut hasher, holdout_digest.as_str());
    hasher.update(next_generation.to_be_bytes());
    hasher.update((planned_candidates as u64).to_be_bytes());
    hasher.update((parents.len() as u64).to_be_bytes());
    for parent in parents {
        hash_text(&mut hasher, parent.artifact_id().as_str());
        hasher.update(parent.selection_score().to_be_bytes());
        hasher.update(parent.novelty_score().to_be_bytes());
        hash_text(&mut hasher, parent.mutation_batch().digest().as_str());
        hasher.update((parent.planned_mutations() as u64).to_be_bytes());
    }
    sha256_digest(hasher)
}

fn hash_text(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value.as_bytes());
}

fn sha256_digest(hasher: Sha256) -> RequestDigest {
    RequestDigest::new(format!("sha256:{:x}", hasher.finalize()))
        .expect("SHA-256 request digest is valid")
}
