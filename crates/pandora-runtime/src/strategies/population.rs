use super::StrategyProfile;
use pandora_types::{
    ArtifactId, CandidateDisposition, CandidateOutcome, CandidatePopulation, EvaluationKind,
    EvaluationReceipt, EvaluationStatus, FailureCorpus, GenerationReceipt, GenerationStats,
    MutationBatch, MutationPrecheckReceipt, POPULATION_PROTOCOL_VERSION, PopulationCandidate,
    PopulationContractError, PopulationEvaluation, PopulationId, PopulationMutationRequest,
    PopulationPolicy, PrecheckFailure, RequestDigest, Timestamp, Usage,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Mutex;

type PreparedGeneration = (CandidatePopulation, Vec<CandidateOutcome>, GenerationStats);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PopulationStrategyError {
    DisabledInProduction,
    EmptyPopulation,
    CandidateLimitExceeded,
    GenerationLimitExceeded,
    CandidateGenerationAhead(ArtifactId),
    ParentLimitExceeded(ArtifactId),
    NoViableCandidates,
    PopulationAlreadyRegistered(PopulationId),
    PopulationNotRegistered(PopulationId),
    PopulationStateChanged,
    OutcomeCountMismatch,
    InvalidOutcome(ArtifactId, &'static str),
    UsageOverflow,
    UsageLimitExceeded(&'static str),
    StateUnavailable,
    Contract(PopulationContractError),
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
            Self::PopulationAlreadyRegistered(population_id) => write!(
                formatter,
                "population {} is already registered",
                population_id.as_str()
            ),
            Self::PopulationNotRegistered(population_id) => write!(
                formatter,
                "population {} is not registered",
                population_id.as_str()
            ),
            Self::PopulationStateChanged => {
                formatter.write_str("population changed after the generation was planned")
            }
            Self::OutcomeCountMismatch => {
                formatter.write_str("generation outcome count does not match the plan")
            }
            Self::InvalidOutcome(artifact_id, reason) => {
                write!(formatter, "candidate {} {reason}", artifact_id.as_str())
            }
            Self::UsageOverflow => formatter.write_str("generation usage overflowed"),
            Self::UsageLimitExceeded(limit) => {
                write!(formatter, "generation exceeds the {limit} limit")
            }
            Self::StateUnavailable => formatter.write_str("population state is unavailable"),
            Self::Contract(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for PopulationStrategyError {}

impl From<PopulationContractError> for PopulationStrategyError {
    fn from(error: PopulationContractError) -> Self {
        Self::Contract(error)
    }
}

#[derive(Debug)]
pub struct PopulationStrategy {
    profile: StrategyProfile,
    policy: PopulationPolicy,
    populations: Mutex<BTreeMap<PopulationId, CandidatePopulation>>,
}

impl PopulationStrategy {
    pub fn new(profile: StrategyProfile, policy: PopulationPolicy) -> Self {
        Self {
            profile,
            policy,
            populations: Mutex::new(BTreeMap::new()),
        }
    }

    pub const fn profile(&self) -> StrategyProfile {
        self.profile
    }

    pub const fn policy(&self) -> PopulationPolicy {
        self.policy
    }

    pub fn register_population(
        &self,
        population: CandidatePopulation,
    ) -> Result<(), PopulationStrategyError> {
        self.validate_population(&population)?;
        let population_id = population.scope().population_id().clone();
        let mut populations = self
            .populations
            .lock()
            .map_err(|_| PopulationStrategyError::StateUnavailable)?;
        if populations.contains_key(&population_id) {
            return Err(PopulationStrategyError::PopulationAlreadyRegistered(
                population_id,
            ));
        }
        populations.insert(population_id, population);
        Ok(())
    }

    pub fn population(
        &self,
        population_id: &PopulationId,
    ) -> Result<CandidatePopulation, PopulationStrategyError> {
        self.populations
            .lock()
            .map_err(|_| PopulationStrategyError::StateUnavailable)?
            .get(population_id)
            .cloned()
            .ok_or_else(|| PopulationStrategyError::PopulationNotRegistered(population_id.clone()))
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
            .saturating_sub(ranked.len())
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
            population.scope().population_id(),
            &population_digest,
            corpus.holdout_digest(),
            corpus.holdout_count(),
            next_generation,
            planned_candidates,
            &parents,
        );
        Ok(PopulationPlan {
            population_id: population.scope().population_id().clone(),
            starting_generation: population.generation(),
            next_generation,
            population_digest,
            holdout_digest: corpus.holdout_digest().clone(),
            holdout_count: corpus.holdout_count(),
            parents,
            planned_candidates,
            plan_digest,
        })
    }

    pub fn precheck(
        &self,
        request: &PopulationMutationRequest,
        evaluation: &EvaluationReceipt,
    ) -> Result<MutationPrecheckReceipt, PopulationStrategyError> {
        if self.profile == StrategyProfile::Production {
            return Err(PopulationStrategyError::DisabledInProduction);
        }

        let mut failures = evaluation
            .results()
            .iter()
            .filter_map(|result| match result.status() {
                EvaluationStatus::Passed => None,
                EvaluationStatus::Failed => Some(PrecheckFailure::Failed(result.kind())),
                EvaluationStatus::HumanReviewRequired => {
                    Some(PrecheckFailure::HumanReviewRequired(result.kind()))
                }
            })
            .collect::<Vec<_>>();
        for required in [EvaluationKind::Policy, EvaluationKind::Regression] {
            let Some(result) = evaluation
                .results()
                .iter()
                .find(|result| result.kind() == required)
            else {
                failures.push(PrecheckFailure::Missing(required));
                continue;
            };
            if !result.passed() {
                continue;
            }
            if result.advisory() {
                failures.push(PrecheckFailure::Advisory(required));
            } else if result.score() < self.policy.min_precheck_score() {
                failures.push(PrecheckFailure::BelowMinimum(required));
            }
        }

        Ok(MutationPrecheckReceipt::new(
            request.request_digest().clone(),
            evaluation.session_id().clone(),
            evaluation.execution_id().clone(),
            digest_evaluation(evaluation),
            self.policy.min_precheck_score(),
            failures,
            evaluation.evaluated_at(),
        )?)
    }

    pub fn complete_generation(
        &self,
        plan: &PopulationPlan,
        evaluations: Vec<PopulationEvaluation>,
        completed_at: Timestamp,
    ) -> Result<GenerationReceipt, PopulationStrategyError> {
        if self.profile == StrategyProfile::Production {
            return Err(PopulationStrategyError::DisabledInProduction);
        }
        let starting = self.population(plan.population_id())?;
        self.validate_plan_state(&starting, plan)?;
        let (resulting, outcomes, stats) = self.prepare_generation(&starting, plan, evaluations)?;
        let resulting_digest = digest_population(&resulting);
        let receipt = GenerationReceipt::new(
            plan.population_id().clone(),
            plan.next_generation(),
            plan.plan_digest().clone(),
            plan.population_digest().clone(),
            resulting_digest,
            outcomes,
            stats,
            completed_at,
        )?;

        let mut populations = self
            .populations
            .lock()
            .map_err(|_| PopulationStrategyError::StateUnavailable)?;
        let current = populations.get(plan.population_id()).ok_or_else(|| {
            PopulationStrategyError::PopulationNotRegistered(plan.population_id().clone())
        })?;
        self.validate_plan_state(current, plan)?;
        populations.insert(plan.population_id().clone(), resulting);
        Ok(receipt)
    }

    fn prepare_generation(
        &self,
        starting: &CandidatePopulation,
        plan: &PopulationPlan,
        mut evaluations: Vec<PopulationEvaluation>,
    ) -> Result<PreparedGeneration, PopulationStrategyError> {
        if evaluations.len() != plan.planned_candidates() {
            return Err(PopulationStrategyError::OutcomeCountMismatch);
        }
        evaluations.sort_by(|left, right| {
            left.request()
                .candidate_artifact()
                .cmp(right.request().candidate_artifact())
        });
        for pair in evaluations.windows(2) {
            if pair[0].request().candidate_artifact() == pair[1].request().candidate_artifact() {
                return Err(PopulationStrategyError::InvalidOutcome(
                    pair[0].request().candidate_artifact().clone(),
                    "is duplicated",
                ));
            }
        }

        let parent_plans = plan
            .parents()
            .iter()
            .map(|parent| (parent.artifact_id().clone(), parent))
            .collect::<BTreeMap<_, _>>();
        let selected_parents = parent_plans.keys().cloned().collect::<BTreeSet<_>>();
        let mut parent_attempts = parent_plans
            .keys()
            .cloned()
            .map(|parent| (parent, 0usize))
            .collect::<BTreeMap<_, _>>();
        let mut accepted_candidates = Vec::new();
        let mut outcomes = Vec::with_capacity(evaluations.len());
        let mut usage = Usage::new(0, 0, 0, 0);
        let mut accepted = 0usize;
        let mut precheck_rejected = 0usize;
        let mut evaluation_rejected = 0usize;
        let mut full_evaluations = 0usize;

        for candidate_evaluation in evaluations {
            let request = candidate_evaluation.request();
            let candidate_id = request.candidate_artifact().clone();
            let parent_plan = self.validate_outcome_request(
                starting,
                plan,
                &candidate_evaluation,
                &parent_plans,
            )?;
            let attempts = parent_attempts
                .get_mut(request.parent_artifact())
                .expect("validated parent has an attempt counter");
            *attempts = attempts
                .checked_add(1)
                .ok_or(PopulationStrategyError::UsageOverflow)?;
            usage = checked_add_usage(usage, candidate_evaluation.usage())?;

            if !candidate_evaluation.precheck().passed() {
                precheck_rejected += 1;
                outcomes.push(CandidateOutcome::new(
                    candidate_id,
                    request.parent_artifact().clone(),
                    CandidateDisposition::RejectedPrecheck,
                    candidate_evaluation.precheck().digest().clone(),
                    None,
                    None,
                    candidate_evaluation.usage(),
                )?);
                continue;
            }

            let evaluation = candidate_evaluation.evaluation().ok_or_else(|| {
                PopulationStrategyError::InvalidOutcome(
                    candidate_id.clone(),
                    "is missing its full evaluation",
                )
            })?;
            if evaluation.session_id() != starting.scope().session_id() {
                return Err(PopulationStrategyError::InvalidOutcome(
                    candidate_id,
                    "uses full evaluation evidence from another session",
                ));
            }
            full_evaluations = full_evaluations
                .checked_add(1)
                .ok_or(PopulationStrategyError::UsageOverflow)?;
            self.validate_holdout(plan, &candidate_evaluation)?;
            self.validate_candidate_evidence(
                &candidate_evaluation,
                parent_plan,
                &selected_parents,
            )?;
            let (is_accepted, score) = self.evaluate_candidate(&candidate_id, evaluation)?;
            let evaluation_digest = digest_evaluation(evaluation);
            let candidate = PopulationCandidate::new(
                candidate_id.clone(),
                candidate_evaluation.parents().to_vec(),
                plan.next_generation(),
                score,
                is_accepted,
                0,
                evaluation_digest.clone(),
                candidate_evaluation.training_failures().to_vec(),
            )?;
            let disposition = if is_accepted {
                accepted += 1;
                accepted_candidates.push(candidate);
                CandidateDisposition::Accepted
            } else {
                evaluation_rejected += 1;
                CandidateDisposition::RejectedEvaluation
            };
            outcomes.push(CandidateOutcome::new(
                candidate_id,
                request.parent_artifact().clone(),
                disposition,
                candidate_evaluation.precheck().digest().clone(),
                Some(evaluation_digest),
                Some(score),
                candidate_evaluation.usage(),
            )?);
        }

        if full_evaluations > self.policy.max_evaluations() {
            return Err(PopulationStrategyError::UsageLimitExceeded(
                "evaluation count",
            ));
        }
        validate_usage_limit(usage, self.policy.max_usage())?;
        for parent in plan.parents() {
            if parent_attempts
                .get(parent.artifact_id())
                .copied()
                .unwrap_or(0)
                != parent.planned_mutations()
            {
                return Err(PopulationStrategyError::InvalidOutcome(
                    parent.artifact_id().clone(),
                    "does not have the planned number of mutations",
                ));
            }
        }

        let starting_candidates = starting
            .candidates()
            .iter()
            .map(|candidate| (candidate.artifact_id(), candidate))
            .collect::<BTreeMap<_, _>>();
        let mut next_candidates =
            Vec::with_capacity(plan.parents().len() + accepted_candidates.len());
        for parent in plan.parents() {
            let source = starting_candidates
                .get(parent.artifact_id())
                .ok_or_else(|| {
                    PopulationStrategyError::InvalidOutcome(
                        parent.artifact_id().clone(),
                        "is not present in the starting population",
                    )
                })?;
            let attempts = u32::try_from(
                parent_attempts
                    .get(parent.artifact_id())
                    .copied()
                    .unwrap_or(0),
            )
            .map_err(|_| PopulationStrategyError::UsageOverflow)?;
            let child_count = source
                .child_count()
                .checked_add(attempts)
                .ok_or(PopulationStrategyError::UsageOverflow)?;
            next_candidates.push(PopulationCandidate::new(
                source.artifact_id().clone(),
                source.parents().to_vec(),
                source.generation(),
                source.score(),
                source.viable(),
                child_count,
                source.evaluation_digest().clone(),
                source.training_failures().to_vec(),
            )?);
        }
        next_candidates.extend(accepted_candidates);
        if next_candidates.len() > self.policy.max_candidates() {
            return Err(PopulationStrategyError::CandidateLimitExceeded);
        }
        let resulting = CandidatePopulation::new(
            starting.scope().clone(),
            plan.next_generation(),
            next_candidates,
        )?;
        let stats = GenerationStats::new(
            outcomes.len(),
            accepted,
            precheck_rejected,
            evaluation_rejected,
            usage,
        );
        Ok((resulting, outcomes, stats))
    }

    fn validate_plan_state(
        &self,
        population: &CandidatePopulation,
        plan: &PopulationPlan,
    ) -> Result<(), PopulationStrategyError> {
        if population.scope().population_id() != plan.population_id()
            || population.generation() != plan.starting_generation()
            || population.generation().checked_add(1) != Some(plan.next_generation())
            || digest_population(population) != *plan.population_digest()
        {
            return Err(PopulationStrategyError::PopulationStateChanged);
        }
        Ok(())
    }

    fn validate_outcome_request<'a>(
        &self,
        starting: &CandidatePopulation,
        plan: &PopulationPlan,
        evaluation: &PopulationEvaluation,
        parents: &'a BTreeMap<ArtifactId, &PopulationParentPlan>,
    ) -> Result<&'a PopulationParentPlan, PopulationStrategyError> {
        let request = evaluation.request();
        let candidate_id = request.candidate_artifact().clone();
        if request.population_id() != plan.population_id()
            || request.generation() != plan.next_generation()
            || request.plan_digest() != plan.plan_digest()
            || request.request_digest() != evaluation.precheck().request_digest()
            || evaluation.precheck().evaluation_session_id() != starting.scope().session_id()
        {
            return Err(PopulationStrategyError::InvalidOutcome(
                candidate_id,
                "does not match the generation plan",
            ));
        }
        let parent = parents.get(request.parent_artifact()).ok_or_else(|| {
            PopulationStrategyError::InvalidOutcome(
                candidate_id.clone(),
                "uses an unselected parent",
            )
        })?;
        if request.mutation_batch_digest() != parent.mutation_batch().digest() {
            return Err(PopulationStrategyError::InvalidOutcome(
                candidate_id,
                "does not match the parent's mutation batch",
            ));
        }
        Ok(*parent)
    }

    fn validate_holdout(
        &self,
        plan: &PopulationPlan,
        evaluation: &PopulationEvaluation,
    ) -> Result<(), PopulationStrategyError> {
        if evaluation.holdout_digest() != Some(plan.holdout_digest())
            || evaluation.holdout_count() != Some(plan.holdout_count())
        {
            return Err(PopulationStrategyError::InvalidOutcome(
                evaluation.request().candidate_artifact().clone(),
                "does not match the sealed holdout evidence",
            ));
        }
        Ok(())
    }

    fn validate_candidate_evidence(
        &self,
        evaluation: &PopulationEvaluation,
        parent: &PopulationParentPlan,
        selected_parents: &BTreeSet<ArtifactId>,
    ) -> Result<(), PopulationStrategyError> {
        let candidate_id = evaluation.request().candidate_artifact().clone();
        if !evaluation
            .parents()
            .contains(evaluation.request().parent_artifact())
            || evaluation
                .parents()
                .iter()
                .any(|candidate_parent| !selected_parents.contains(candidate_parent))
        {
            return Err(PopulationStrategyError::InvalidOutcome(
                candidate_id.clone(),
                "has invalid lineage parents",
            ));
        }
        let eligible_failures = parent
            .mutation_batch()
            .failures()
            .iter()
            .map(|failure| failure.id())
            .collect::<BTreeSet<_>>();
        if evaluation
            .training_failures()
            .iter()
            .any(|failure| !eligible_failures.contains(failure))
        {
            return Err(PopulationStrategyError::InvalidOutcome(
                candidate_id,
                "uses failure evidence outside the mutation batch",
            ));
        }
        Ok(())
    }

    fn evaluate_candidate(
        &self,
        candidate_id: &ArtifactId,
        evaluation: &EvaluationReceipt,
    ) -> Result<(bool, u8), PopulationStrategyError> {
        let mut score = 100u8;
        let mut accepted = evaluation
            .results()
            .iter()
            .all(|result| result.status() == EvaluationStatus::Passed);
        for required in [
            EvaluationKind::Outcome,
            EvaluationKind::Policy,
            EvaluationKind::Regression,
            EvaluationKind::Adversarial,
        ] {
            let result = evaluation
                .results()
                .iter()
                .find(|result| result.kind() == required)
                .ok_or_else(|| {
                    PopulationStrategyError::InvalidOutcome(
                        candidate_id.clone(),
                        "is missing a required full evaluation",
                    )
                })?;
            score = score.min(result.score());
            accepted &= result.passed()
                && !result.advisory()
                && result.score() >= self.policy.min_precheck_score();
        }
        Ok((accepted, score))
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
    population_id: PopulationId,
    starting_generation: u32,
    next_generation: u32,
    population_digest: RequestDigest,
    holdout_digest: RequestDigest,
    holdout_count: usize,
    parents: Vec<PopulationParentPlan>,
    planned_candidates: usize,
    plan_digest: RequestDigest,
}

impl PopulationPlan {
    pub fn population_id(&self) -> &PopulationId {
        &self.population_id
    }

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

    pub const fn holdout_count(&self) -> usize {
        self.holdout_count
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
    population_id: &PopulationId,
    population_digest: &RequestDigest,
    holdout_digest: &RequestDigest,
    holdout_count: usize,
    next_generation: u32,
    planned_candidates: usize,
    parents: &[PopulationParentPlan],
) -> RequestDigest {
    let mut hasher = Sha256::new();
    hasher.update(POPULATION_PROTOCOL_VERSION.to_be_bytes());
    hash_text(&mut hasher, population_id.as_str());
    hash_text(&mut hasher, population_digest.as_str());
    hash_text(&mut hasher, holdout_digest.as_str());
    hasher.update((holdout_count as u64).to_be_bytes());
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

fn digest_evaluation(evaluation: &EvaluationReceipt) -> RequestDigest {
    let mut results = evaluation.results().iter().collect::<Vec<_>>();
    results.sort_by_key(|result| result.kind());
    let mut hasher = Sha256::new();
    hasher.update(POPULATION_PROTOCOL_VERSION.to_be_bytes());
    hash_text(&mut hasher, evaluation.session_id().as_str());
    hash_text(&mut hasher, evaluation.execution_id().as_str());
    hasher.update(evaluation.evaluated_at().as_unix_seconds().to_be_bytes());
    hasher.update((results.len() as u64).to_be_bytes());
    for result in results {
        hash_text(&mut hasher, result.kind().as_str());
        hash_text(&mut hasher, result.status().as_str());
        hasher.update([result.score(), u8::from(result.advisory())]);
    }
    sha256_digest(hasher)
}

fn checked_add_usage(left: Usage, right: Usage) -> Result<Usage, PopulationStrategyError> {
    Ok(Usage::new(
        left.tokens()
            .checked_add(right.tokens())
            .ok_or(PopulationStrategyError::UsageOverflow)?,
        left.tools()
            .checked_add(right.tools())
            .ok_or(PopulationStrategyError::UsageOverflow)?,
        left.duration_seconds()
            .checked_add(right.duration_seconds())
            .ok_or(PopulationStrategyError::UsageOverflow)?,
        left.cost_micros()
            .checked_add(right.cost_micros())
            .ok_or(PopulationStrategyError::UsageOverflow)?,
    ))
}

fn validate_usage_limit(usage: Usage, limit: Usage) -> Result<(), PopulationStrategyError> {
    if usage.tokens() > limit.tokens() {
        return Err(PopulationStrategyError::UsageLimitExceeded("token"));
    }
    if usage.tools() > limit.tools() {
        return Err(PopulationStrategyError::UsageLimitExceeded("tool"));
    }
    if usage.duration_seconds() > limit.duration_seconds() {
        return Err(PopulationStrategyError::UsageLimitExceeded("duration"));
    }
    if usage.cost_micros() > limit.cost_micros() {
        return Err(PopulationStrategyError::UsageLimitExceeded("cost"));
    }
    Ok(())
}

fn hash_text(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value.as_bytes());
}

fn sha256_digest(hasher: Sha256) -> RequestDigest {
    RequestDigest::new(format!("sha256:{:x}", hasher.finalize()))
        .expect("SHA-256 request digest is valid")
}
