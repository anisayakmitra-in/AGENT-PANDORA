use pandora_runtime::{PopulationStrategy, PopulationStrategyError, StrategyProfile};
use pandora_types::{
    ArtifactId, CandidatePopulation, FailureCorpus, FailureEvidence, FailureId, FailurePartition,
    LineageLimits, MutationLimits, PopulationCandidate, PopulationId, PopulationPolicy,
    PopulationScope, RequestDigest, SessionId, TenantId, Timestamp, Usage, WorkspaceId,
};

fn policy(max_parents: usize) -> PopulationPolicy {
    PopulationPolicy::new(
        8,
        max_parents,
        4,
        8,
        MutationLimits::new(2, 128, 2).unwrap(),
        LineageLimits::new(3, 16, 2_048).unwrap(),
        70,
        5_000,
        Usage::new(10_000, 50, 300, 100_000),
    )
    .unwrap()
}

fn scope() -> PopulationScope {
    PopulationScope::new(
        PopulationId::new("population-1").unwrap(),
        TenantId::new("tenant-1").unwrap(),
        WorkspaceId::new("workspace-1").unwrap(),
        SessionId::new("session-1").unwrap(),
    )
}

fn candidate(id: &str, score: u8, child_count: u32, failures: &[&str]) -> PopulationCandidate {
    PopulationCandidate::new(
        ArtifactId::new(id).unwrap(),
        Vec::new(),
        0,
        score,
        true,
        child_count,
        RequestDigest::new(format!("evaluation-{id}")).unwrap(),
        failures
            .iter()
            .map(|failure| FailureId::new(*failure).unwrap())
            .collect(),
    )
    .unwrap()
}

fn failure(id: &str, partition: FailurePartition, summary: &str) -> FailureEvidence {
    FailureEvidence::new(
        FailureId::new(id).unwrap(),
        partition,
        "verification",
        summary,
        RequestDigest::new(format!("evidence-{id}")).unwrap(),
        Timestamp::from_unix_seconds(10),
    )
    .unwrap()
}

fn corpus() -> FailureCorpus {
    FailureCorpus::new(vec![
        failure("train-1", FailurePartition::Training, "test failed"),
        failure("train-2", FailurePartition::Training, "lint failed"),
        failure("train-3", FailurePartition::Training, "build failed"),
        failure("holdout-1", FailurePartition::Holdout, "hidden regression"),
    ])
    .unwrap()
}

#[test]
fn population_planning_is_research_only() {
    let population = CandidatePopulation::new(
        scope(),
        0,
        vec![candidate("candidate-a", 90, 0, &["train-1"])],
    )
    .unwrap();

    assert_eq!(
        PopulationStrategy::new(StrategyProfile::Production, policy(1))
            .plan(&population, &corpus()),
        Err(PopulationStrategyError::DisabledInProduction)
    );
}

#[test]
fn planning_replays_deterministically_and_binds_the_starting_population() {
    let strategy = PopulationStrategy::new(StrategyProfile::Research, policy(2));
    let population = CandidatePopulation::new(
        scope(),
        0,
        vec![
            candidate("candidate-a", 90, 2, &["train-1", "train-2"]),
            candidate("candidate-b", 85, 0, &["train-3"]),
        ],
    )
    .unwrap();

    let first = strategy.plan(&population, &corpus()).unwrap();
    let second = strategy.plan(&population, &corpus()).unwrap();

    assert_eq!(first, second);
    assert_eq!(first.starting_generation(), 0);
    assert_eq!(first.next_generation(), 1);
    assert_ne!(first.population_digest(), first.plan_digest());

    let changed = CandidatePopulation::new(
        scope(),
        0,
        vec![
            candidate("candidate-a", 91, 2, &["train-1", "train-2"]),
            candidate("candidate-b", 85, 0, &["train-3"]),
        ],
    )
    .unwrap();
    let changed_plan = strategy.plan(&changed, &corpus()).unwrap();

    assert_ne!(first.population_digest(), changed_plan.population_digest());
    assert_ne!(first.plan_digest(), changed_plan.plan_digest());
}

#[test]
fn novelty_prefers_the_less_explored_viable_candidate() {
    let population = CandidatePopulation::new(
        scope(),
        0,
        vec![
            candidate("candidate-explored", 90, 8, &["train-1"]),
            candidate("candidate-novel", 90, 0, &["train-2"]),
        ],
    )
    .unwrap();

    let plan = PopulationStrategy::new(StrategyProfile::Research, policy(1))
        .plan(&population, &corpus())
        .unwrap();

    assert_eq!(plan.parents().len(), 1);
    assert_eq!(plan.parents()[0].artifact_id().as_str(), "candidate-novel");
    assert!(plan.parents()[0].novelty_score() > 0);
}

#[test]
fn plans_use_only_bounded_training_evidence() {
    let population = CandidatePopulation::new(
        scope(),
        0,
        vec![candidate(
            "candidate-a",
            90,
            0,
            &["train-1", "train-2", "train-3", "holdout-1"],
        )],
    )
    .unwrap();
    let corpus = corpus();

    let plan = PopulationStrategy::new(StrategyProfile::Research, policy(1))
        .plan(&population, &corpus)
        .unwrap();
    let parent = &plan.parents()[0];

    assert_eq!(plan.holdout_digest(), corpus.holdout_digest());
    assert_eq!(parent.mutation_batch().failures().len(), 2);
    assert!(
        parent
            .mutation_batch()
            .failures()
            .iter()
            .all(|failure| failure.partition() == FailurePartition::Training)
    );
    assert!(parent.mutation_batch().context_bytes() <= 128);
    assert_eq!(parent.planned_mutations(), 2);
    assert_eq!(plan.planned_candidates(), 2);
}

#[test]
fn planning_enforces_generation_and_evaluation_limits() {
    let limited = PopulationPolicy::new(
        1,
        1,
        1,
        1,
        MutationLimits::new(2, 128, 2).unwrap(),
        LineageLimits::new(3, 16, 2_048).unwrap(),
        70,
        5_000,
        Usage::new(10_000, 50, 300, 100_000),
    )
    .unwrap();
    let strategy = PopulationStrategy::new(StrategyProfile::Research, limited);
    let population = CandidatePopulation::new(
        scope(),
        0,
        vec![candidate("candidate-a", 90, 0, &["train-1", "train-2"])],
    )
    .unwrap();

    assert_eq!(
        strategy
            .plan(&population, &corpus())
            .unwrap()
            .planned_candidates(),
        1
    );

    let exhausted = CandidatePopulation::new(
        scope(),
        1,
        vec![
            PopulationCandidate::new(
                ArtifactId::new("candidate-a").unwrap(),
                Vec::new(),
                1,
                90,
                true,
                0,
                RequestDigest::new("evaluation-candidate-a").unwrap(),
                vec![FailureId::new("train-1").unwrap()],
            )
            .unwrap(),
        ],
    )
    .unwrap();
    assert_eq!(
        strategy.plan(&exhausted, &corpus()),
        Err(PopulationStrategyError::GenerationLimitExceeded)
    );
}
