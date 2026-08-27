use pandora_runtime::{MemoryEngine, PopulationStrategy, PopulationStrategyError, StrategyProfile};
use pandora_types::{
    ArtifactId, CandidateDisposition, CandidatePopulation, EvaluationKind, EvaluationReceipt,
    EvaluationResult, EvaluationStatus, ExecutionId, FailureCorpus, FailureEvidence, FailureId,
    FailurePartition, GenerationReceipt, LineageAttempt, LineageDirection, LineageLimits,
    LineageMemory, LineageQuery, MemoryApproval, MemoryKind, MemoryScope, MemoryTier,
    MutationLimits, PopulationCandidate, PopulationEvaluation, PopulationId,
    PopulationMutationRequest, PopulationPolicy, PopulationScope, RequestDigest, SessionId,
    TenantId, Timestamp, Usage, WorkspaceId,
};
use std::fs;

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

fn lineage_candidate(id: &str, parents: &[&str], generation: u32) -> PopulationCandidate {
    PopulationCandidate::new(
        ArtifactId::new(id).unwrap(),
        parents
            .iter()
            .map(|parent| ArtifactId::new(*parent).unwrap())
            .collect(),
        generation,
        80,
        true,
        0,
        RequestDigest::new(format!("evaluation-{id}")).unwrap(),
        Vec::new(),
    )
    .unwrap()
}

fn memory_scope(provider: &str) -> MemoryScope {
    let population_scope = scope();
    MemoryScope::new(
        population_scope.tenant_id().clone(),
        population_scope.workspace_id().clone(),
        population_scope.session_id().clone(),
        provider,
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

fn evaluation_result(
    kind: EvaluationKind,
    status: EvaluationStatus,
    score: u8,
) -> EvaluationResult {
    EvaluationResult::new(kind, status, score, "bounded population evaluation", false).unwrap()
}

fn evaluation_receipt(execution_id: &str, results: Vec<EvaluationResult>) -> EvaluationReceipt {
    EvaluationReceipt::new(
        SessionId::new("session-1").unwrap(),
        ExecutionId::new(execution_id).unwrap(),
        Timestamp::from_unix_seconds(20),
        results,
    )
    .unwrap()
}

fn passing_precheck_receipt() -> EvaluationReceipt {
    evaluation_receipt(
        "precheck-execution",
        vec![
            evaluation_result(EvaluationKind::Policy, EvaluationStatus::Passed, 90),
            evaluation_result(EvaluationKind::Regression, EvaluationStatus::Passed, 90),
        ],
    )
}

fn full_evaluation(execution_id: &str, status: EvaluationStatus, score: u8) -> EvaluationReceipt {
    evaluation_receipt(
        execution_id,
        vec![
            evaluation_result(EvaluationKind::Outcome, status, score),
            evaluation_result(EvaluationKind::Policy, EvaluationStatus::Passed, 90),
            evaluation_result(EvaluationKind::Regression, EvaluationStatus::Passed, 85),
            evaluation_result(EvaluationKind::Adversarial, EvaluationStatus::Passed, 80),
        ],
    )
}

fn registered_strategy(
    policy: PopulationPolicy,
    population: &CandidatePopulation,
) -> PopulationStrategy {
    let strategy = PopulationStrategy::new(StrategyProfile::Research, policy);
    strategy.register_population(population.clone()).unwrap();
    strategy
}

#[test]
fn population_state_survives_strategy_restart() {
    let path = std::env::temp_dir().join(format!(
        "pandora-population-state-{}-{}.json",
        std::process::id(),
        Timestamp::from_unix_seconds(1).as_unix_seconds()
    ));
    let _ = fs::remove_file(&path);
    let population = CandidatePopulation::new(
        scope(),
        0,
        vec![candidate("candidate-a", 90, 0, &["train-1"])],
    )
    .unwrap();
    let strategy = PopulationStrategy::open(&path, StrategyProfile::Research, policy(2)).unwrap();
    strategy.register_population(population.clone()).unwrap();
    drop(strategy);

    let reopened = PopulationStrategy::open(&path, StrategyProfile::Research, policy(2)).unwrap();
    assert_eq!(
        reopened
            .population(population.scope().population_id())
            .unwrap(),
        population
    );
    let _ = fs::remove_file(path);
}

#[test]
fn population_state_rejects_corrupt_or_oversized_schema() {
    let path = std::env::temp_dir().join(format!(
        "pandora-population-corrupt-{}.json",
        std::process::id()
    ));
    fs::write(&path, br#"{"version":99,"populations":[]}"#).unwrap();
    let result = PopulationStrategy::open(&path, StrategyProfile::Research, policy(2));
    assert!(matches!(result, Err(PopulationStrategyError::CorruptState)));
    let _ = fs::remove_file(path);
}

fn completed_lineage_run() -> (
    PopulationStrategy,
    MemoryEngine,
    MemoryScope,
    GenerationReceipt,
) {
    let limited = PopulationPolicy::new(
        2,
        1,
        4,
        1,
        MutationLimits::new(1, 128, 1).unwrap(),
        LineageLimits::new(3, 16, 2_048).unwrap(),
        70,
        5_000,
        Usage::new(10_000, 50, 300, 100_000),
    )
    .unwrap();
    let population = CandidatePopulation::new(
        scope(),
        0,
        vec![candidate("candidate-a", 90, 0, &["train-1"])],
    )
    .unwrap();
    let strategy = registered_strategy(limited, &population);
    let plan = strategy.plan(&population, &corpus()).unwrap();
    let evaluation = evaluated_candidate(
        &strategy,
        &plan,
        "candidate-a",
        "candidate-b",
        full_evaluation("full-b", EvaluationStatus::Passed, 90),
        plan.holdout_count(),
        Usage::new(100, 1, 2, 300),
    );
    let receipt = strategy
        .complete_generation(&plan, vec![evaluation], Timestamp::from_unix_seconds(30))
        .unwrap();
    (
        strategy,
        MemoryEngine::new(4),
        memory_scope("provider-a"),
        receipt,
    )
}

fn evaluated_candidate(
    strategy: &PopulationStrategy,
    plan: &pandora_runtime::PopulationPlan,
    parent: &str,
    candidate: &str,
    evaluation: EvaluationReceipt,
    holdout_count: usize,
    usage: Usage,
) -> PopulationEvaluation {
    let parent_plan = plan
        .parents()
        .iter()
        .find(|entry| entry.artifact_id().as_str() == parent)
        .unwrap();
    let request = PopulationMutationRequest::new(
        plan.population_id().clone(),
        plan.next_generation(),
        ArtifactId::new(parent).unwrap(),
        ArtifactId::new(candidate).unwrap(),
        plan.plan_digest().clone(),
        parent_plan.mutation_batch().digest().clone(),
    )
    .unwrap();
    let precheck = strategy
        .precheck(&request, &passing_precheck_receipt())
        .unwrap();
    PopulationEvaluation::evaluated(
        request,
        vec![ArtifactId::new(parent).unwrap()],
        parent_plan
            .mutation_batch()
            .failures()
            .iter()
            .map(|failure| failure.id().clone())
            .collect(),
        precheck,
        evaluation,
        plan.holdout_digest().clone(),
        holdout_count,
        usage,
    )
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
        0
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

#[test]
fn invalid_outcome_aborts_the_entire_generation() {
    let population = CandidatePopulation::new(
        scope(),
        0,
        vec![candidate("candidate-a", 90, 0, &["train-1", "train-2"])],
    )
    .unwrap();
    let strategy = registered_strategy(policy(1), &population);
    let plan = strategy.plan(&population, &corpus()).unwrap();
    let first = evaluated_candidate(
        &strategy,
        &plan,
        "candidate-a",
        "candidate-b",
        full_evaluation("full-b", EvaluationStatus::Passed, 90),
        plan.holdout_count(),
        Usage::new(100, 1, 2, 300),
    );
    let second = evaluated_candidate(
        &strategy,
        &plan,
        "candidate-a",
        "candidate-c",
        full_evaluation("full-c", EvaluationStatus::Passed, 90),
        plan.holdout_count() + 1,
        Usage::new(200, 2, 3, 400),
    );

    assert!(
        strategy
            .complete_generation(&plan, vec![first, second], Timestamp::from_unix_seconds(30))
            .is_err()
    );
    assert_eq!(
        strategy.population(plan.population_id()).unwrap(),
        population
    );
}

#[test]
fn accepted_candidates_appear_only_after_every_outcome_validates() {
    let population = CandidatePopulation::new(
        scope(),
        0,
        vec![candidate("candidate-a", 90, 0, &["train-1", "train-2"])],
    )
    .unwrap();
    let strategy = registered_strategy(policy(1), &population);
    let plan = strategy.plan(&population, &corpus()).unwrap();
    let first = evaluated_candidate(
        &strategy,
        &plan,
        "candidate-a",
        "candidate-b",
        full_evaluation("full-b", EvaluationStatus::Passed, 90),
        plan.holdout_count(),
        Usage::new(100, 1, 2, 300),
    );
    let second = evaluated_candidate(
        &strategy,
        &plan,
        "candidate-a",
        "candidate-c",
        full_evaluation("full-c", EvaluationStatus::Passed, 90),
        plan.holdout_count(),
        Usage::new(200, 2, 3, 400),
    );

    assert!(
        strategy
            .complete_generation(&plan, vec![first.clone()], Timestamp::from_unix_seconds(30))
            .is_err()
    );
    assert_eq!(
        strategy.population(plan.population_id()).unwrap(),
        population
    );

    let receipt = strategy
        .complete_generation(&plan, vec![first, second], Timestamp::from_unix_seconds(30))
        .unwrap();
    let committed = strategy.population(plan.population_id()).unwrap();

    assert_eq!(committed.generation(), 1);
    assert_eq!(committed.candidates().len(), 3);
    assert_eq!(receipt.outcomes().len(), 2);
    assert!(
        receipt
            .outcomes()
            .iter()
            .all(|outcome| outcome.disposition() == CandidateDisposition::Accepted)
    );
    assert!(!receipt.can_authorize_permit());
}

#[test]
fn failed_prechecks_are_receipted_without_full_evaluation() {
    let limited = PopulationPolicy::new(
        2,
        1,
        4,
        2,
        MutationLimits::new(1, 128, 1).unwrap(),
        LineageLimits::new(3, 16, 2_048).unwrap(),
        70,
        5_000,
        Usage::new(10_000, 50, 300, 100_000),
    )
    .unwrap();
    let population = CandidatePopulation::new(
        scope(),
        0,
        vec![candidate("candidate-a", 90, 0, &["train-1"])],
    )
    .unwrap();
    let strategy = registered_strategy(limited, &population);
    let plan = strategy.plan(&population, &corpus()).unwrap();
    let parent = &plan.parents()[0];
    let request = PopulationMutationRequest::new(
        plan.population_id().clone(),
        plan.next_generation(),
        parent.artifact_id().clone(),
        ArtifactId::new("candidate-b").unwrap(),
        plan.plan_digest().clone(),
        parent.mutation_batch().digest().clone(),
    )
    .unwrap();
    let rejected_precheck = strategy
        .precheck(
            &request,
            &evaluation_receipt(
                "rejected-precheck",
                vec![
                    evaluation_result(EvaluationKind::Policy, EvaluationStatus::Failed, 0),
                    evaluation_result(EvaluationKind::Regression, EvaluationStatus::Passed, 90),
                ],
            ),
        )
        .unwrap();
    let evaluation = PopulationEvaluation::precheck_rejected(
        request,
        rejected_precheck,
        Usage::new(50, 0, 1, 20),
    )
    .unwrap();

    let receipt = strategy
        .complete_generation(&plan, vec![evaluation], Timestamp::from_unix_seconds(30))
        .unwrap();
    let committed = strategy.population(plan.population_id()).unwrap();

    assert_eq!(committed.candidates().len(), 1);
    assert_eq!(receipt.stats().precheck_rejected(), 1);
    assert_eq!(receipt.stats().accepted(), 0);
    assert_eq!(
        receipt.outcomes()[0].disposition(),
        CandidateDisposition::RejectedPrecheck
    );
    assert!(receipt.outcomes()[0].evaluation_digest().is_none());
}

#[test]
fn empty_generation_is_successful_and_receipted() {
    let limited = PopulationPolicy::new(
        1,
        1,
        4,
        1,
        MutationLimits::new(1, 128, 1).unwrap(),
        LineageLimits::new(3, 16, 2_048).unwrap(),
        70,
        5_000,
        Usage::new(10_000, 50, 300, 100_000),
    )
    .unwrap();
    let population = CandidatePopulation::new(
        scope(),
        0,
        vec![candidate("candidate-a", 90, 0, &["train-1"])],
    )
    .unwrap();
    let strategy = registered_strategy(limited, &population);
    let plan = strategy.plan(&population, &corpus()).unwrap();

    assert_eq!(plan.planned_candidates(), 0);
    let receipt = strategy
        .complete_generation(&plan, Vec::new(), Timestamp::from_unix_seconds(30))
        .unwrap();

    assert_eq!(receipt.stats().attempted(), 0);
    assert!(receipt.outcomes().is_empty());
    assert_eq!(
        strategy
            .population(plan.population_id())
            .unwrap()
            .generation(),
        1
    );
}

#[test]
fn generation_receipt_accounts_for_all_work_and_cost() {
    let population = CandidatePopulation::new(
        scope(),
        0,
        vec![candidate("candidate-a", 90, 0, &["train-1", "train-2"])],
    )
    .unwrap();
    let strategy = registered_strategy(policy(1), &population);
    let plan = strategy.plan(&population, &corpus()).unwrap();
    let accepted = evaluated_candidate(
        &strategy,
        &plan,
        "candidate-a",
        "candidate-b",
        full_evaluation("full-b", EvaluationStatus::Passed, 90),
        plan.holdout_count(),
        Usage::new(100, 2, 3, 400),
    );
    let rejected = evaluated_candidate(
        &strategy,
        &plan,
        "candidate-a",
        "candidate-c",
        full_evaluation("full-c", EvaluationStatus::Failed, 0),
        plan.holdout_count(),
        Usage::new(200, 3, 4, 500),
    );

    let receipt = strategy
        .complete_generation(
            &plan,
            vec![accepted, rejected],
            Timestamp::from_unix_seconds(30),
        )
        .unwrap();

    assert_eq!(receipt.stats().attempted(), 2);
    assert_eq!(receipt.stats().accepted(), 1);
    assert_eq!(receipt.stats().evaluation_rejected(), 1);
    assert_eq!(receipt.stats().usage(), Usage::new(300, 5, 7, 900));
    assert_ne!(
        receipt.starting_population_digest(),
        receipt.resulting_population_digest()
    );
}

#[test]
fn lineage_queries_follow_ancestors_and_neighborhood_by_distance() {
    let population = CandidatePopulation::new(
        scope(),
        2,
        vec![
            lineage_candidate("candidate-a", &[], 0),
            lineage_candidate("candidate-b", &["candidate-a"], 1),
            lineage_candidate("candidate-c", &["candidate-b"], 2),
            lineage_candidate("candidate-d", &["candidate-b"], 2),
        ],
    )
    .unwrap();
    let strategy = registered_strategy(policy(2), &population);
    let memory = MemoryEngine::new(4);
    let scope = memory_scope("provider-a");

    let ancestors = strategy
        .lineage_view(
            &memory,
            &scope,
            &LineageQuery::new(
                PopulationId::new("population-1").unwrap(),
                ArtifactId::new("candidate-c").unwrap(),
                LineageDirection::Ancestors,
                LineageMemory::Both,
                2,
                16,
                2_048,
            )
            .unwrap(),
            Timestamp::from_unix_seconds(40),
        )
        .unwrap();
    assert_eq!(
        ancestors
            .nodes()
            .iter()
            .map(|node| (node.artifact_id().as_str(), node.distance()))
            .collect::<Vec<_>>(),
        vec![("candidate-c", 0), ("candidate-b", 1), ("candidate-a", 2)]
    );

    let neighborhood = strategy
        .lineage_view(
            &memory,
            &scope,
            &LineageQuery::new(
                PopulationId::new("population-1").unwrap(),
                ArtifactId::new("candidate-b").unwrap(),
                LineageDirection::Neighborhood,
                LineageMemory::Both,
                1,
                16,
                2_048,
            )
            .unwrap(),
            Timestamp::from_unix_seconds(40),
        )
        .unwrap();
    assert_eq!(
        neighborhood
            .nodes()
            .iter()
            .map(|node| (node.artifact_id().as_str(), node.distance()))
            .collect::<Vec<_>>(),
        vec![
            ("candidate-b", 0),
            ("candidate-a", 1),
            ("candidate-c", 1),
            ("candidate-d", 1),
        ]
    );
}

#[test]
fn lineage_records_are_receipt_bound_and_recalled_by_exact_scope_and_tier() {
    let (strategy, memory, scope, receipt) = completed_lineage_run();
    let records = strategy
        .record_lineage(
            &memory,
            scope.clone(),
            &receipt,
            vec![
                LineageAttempt::new(
                    ArtifactId::new("candidate-b").unwrap(),
                    "replace the linear scan with a bounded index",
                )
                .unwrap(),
            ],
        )
        .unwrap();

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].kind(), MemoryKind::Lineage);
    assert_eq!(records[0].tier(), MemoryTier::L1);
    assert!(records[0].summary().contains("bounded index"));
    assert!(records[0].summary().contains("accepted"));

    let lesson = memory
        .distill_l1(
            scope.clone(),
            "lesson-b",
            MemoryKind::Lesson,
            "retain the indexed lookup after regression checks",
            pandora_types::ContextClassification::Internal,
            Timestamp::from_unix_seconds(29),
            records[0].provenance(),
        )
        .unwrap();
    memory
        .distill_l1(
            scope.clone(),
            "unrelated",
            MemoryKind::Lesson,
            "unrelated evidence",
            pandora_types::ContextClassification::Internal,
            Timestamp::from_unix_seconds(28),
            "sha256:unrelated",
        )
        .unwrap();
    let other_provider = memory_scope("provider-b");
    memory
        .distill_l1(
            other_provider,
            "other-provider",
            MemoryKind::Lesson,
            "same artifact but another provider scope",
            pandora_types::ContextClassification::Internal,
            Timestamp::from_unix_seconds(27),
            records[0].provenance(),
        )
        .unwrap();

    let query = LineageQuery::new(
        PopulationId::new("population-1").unwrap(),
        ArtifactId::new("candidate-b").unwrap(),
        LineageDirection::Ancestors,
        LineageMemory::L1,
        1,
        16,
        2_048,
    )
    .unwrap();
    let view = strategy
        .lineage_view(&memory, &scope, &query, Timestamp::from_unix_seconds(40))
        .unwrap();
    assert_eq!(
        view.lessons()
            .iter()
            .map(|entry| entry.memory_id().as_str())
            .collect::<Vec<_>>(),
        vec!["lesson-b", records[0].id().as_str()]
    );
    assert!(
        view.lessons()
            .iter()
            .all(|entry| entry.artifact_id().as_str() == "candidate-b")
    );

    let approval = MemoryApproval::new("lineage-approval", "operator").unwrap();
    memory
        .promote_l2(
            &scope,
            lesson.id(),
            Some(approval),
            Timestamp::from_unix_seconds(35),
        )
        .unwrap();
    memory
        .promote_l2(
            &scope,
            records[0].id(),
            Some(MemoryApproval::new("lineage-record-approval", "operator").unwrap()),
            Timestamp::from_unix_seconds(36),
        )
        .unwrap();
    let l2_view = strategy
        .lineage_view(
            &memory,
            &scope,
            &LineageQuery::new(
                PopulationId::new("population-1").unwrap(),
                ArtifactId::new("candidate-b").unwrap(),
                LineageDirection::Ancestors,
                LineageMemory::L2,
                1,
                16,
                2_048,
            )
            .unwrap(),
            Timestamp::from_unix_seconds(40),
        )
        .unwrap();
    assert_eq!(l2_view.lessons().len(), 2);
    assert_eq!(l2_view.lessons()[0].memory_id().as_str(), "lesson-b");
    assert_eq!(l2_view.lessons()[1].memory_id(), records[0].id());
    let both_view = strategy
        .lineage_view(
            &memory,
            &scope,
            &LineageQuery::new(
                PopulationId::new("population-1").unwrap(),
                ArtifactId::new("candidate-b").unwrap(),
                LineageDirection::Ancestors,
                LineageMemory::Both,
                1,
                16,
                2_048,
            )
            .unwrap(),
            Timestamp::from_unix_seconds(40),
        )
        .unwrap();
    assert_eq!(both_view.lessons().len(), 2);
    assert!(
        both_view
            .lessons()
            .iter()
            .all(|entry| entry.tier() == MemoryTier::L2)
    );

    let wrong_tenant = MemoryScope::new(
        TenantId::new("tenant-2").unwrap(),
        scope.workspace_id().clone(),
        scope.session_id().clone(),
        scope.provider(),
    )
    .unwrap();
    assert_eq!(
        strategy.lineage_view(
            &memory,
            &wrong_tenant,
            &query,
            Timestamp::from_unix_seconds(40)
        ),
        Err(PopulationStrategyError::LineageScopeMismatch)
    );
    let wrong_workspace = MemoryScope::new(
        scope.tenant_id().clone(),
        WorkspaceId::new("workspace-2").unwrap(),
        scope.session_id().clone(),
        scope.provider(),
    )
    .unwrap();
    assert_eq!(
        strategy.lineage_view(
            &memory,
            &wrong_workspace,
            &query,
            Timestamp::from_unix_seconds(40)
        ),
        Err(PopulationStrategyError::LineageScopeMismatch)
    );
    let wrong_session = MemoryScope::new(
        scope.tenant_id().clone(),
        scope.workspace_id().clone(),
        SessionId::new("session-2").unwrap(),
        scope.provider(),
    )
    .unwrap();
    assert_eq!(
        strategy.lineage_view(
            &memory,
            &wrong_session,
            &query,
            Timestamp::from_unix_seconds(40)
        ),
        Err(PopulationStrategyError::LineageScopeMismatch)
    );
    assert!(
        strategy
            .lineage_view(
                &memory,
                &memory_scope("provider-c"),
                &query,
                Timestamp::from_unix_seconds(40),
            )
            .unwrap()
            .lessons()
            .is_empty()
    );
}

#[test]
fn lineage_rejects_a_receipt_not_committed_by_the_population() {
    let (_, _, _, receipt) = completed_lineage_run();
    let population = CandidatePopulation::new(
        scope(),
        0,
        vec![candidate("candidate-a", 90, 0, &["train-1"])],
    )
    .unwrap();
    let strategy = registered_strategy(policy(1), &population);
    let memory = MemoryEngine::new(4);

    assert_eq!(
        strategy.record_lineage(
            &memory,
            memory_scope("provider-a"),
            &receipt,
            vec![
                LineageAttempt::new(
                    ArtifactId::new("candidate-b").unwrap(),
                    "uncommitted change",
                )
                .unwrap(),
            ],
        ),
        Err(PopulationStrategyError::LineageReceiptNotCommitted)
    );
    assert!(
        memory
            .recall(
                &memory_scope("provider-a"),
                MemoryTier::L1,
                Timestamp::from_unix_seconds(40),
            )
            .is_empty()
    );
}

#[test]
fn committed_lineage_artifact_ids_cannot_be_reused_when_pruned() {
    let (strategy, _, _, _) = completed_lineage_run();
    let current = strategy
        .population(&PopulationId::new("population-1").unwrap())
        .unwrap();
    let plan = strategy.plan(&current, &corpus()).unwrap();
    assert_eq!(plan.parents()[0].artifact_id().as_str(), "candidate-a");
    let reused = evaluated_candidate(
        &strategy,
        &plan,
        "candidate-a",
        "candidate-b",
        full_evaluation("full-reused-b", EvaluationStatus::Passed, 90),
        plan.holdout_count(),
        Usage::new(100, 1, 2, 300),
    );

    assert_eq!(
        strategy.complete_generation(&plan, vec![reused], Timestamp::from_unix_seconds(40)),
        Err(PopulationStrategyError::InvalidOutcome(
            ArtifactId::new("candidate-b").unwrap(),
            "reuses a historical artifact identity",
        ))
    );
    assert_eq!(
        strategy
            .population(&PopulationId::new("population-1").unwrap())
            .unwrap(),
        current
    );
}

#[test]
fn lineage_queries_enforce_policy_record_and_byte_bounds() {
    let (strategy, memory, scope, receipt) = completed_lineage_run();
    let records = strategy
        .record_lineage(
            &memory,
            scope.clone(),
            &receipt,
            vec![
                LineageAttempt::new(ArtifactId::new("candidate-b").unwrap(), "bounded change")
                    .unwrap(),
            ],
        )
        .unwrap();
    for (id, summary, at) in [("lesson-a", "12345", 10), ("lesson-b", "67890", 11)] {
        memory
            .distill_l1(
                scope.clone(),
                id,
                MemoryKind::Lesson,
                summary,
                pandora_types::ContextClassification::Internal,
                Timestamp::from_unix_seconds(at),
                records[0].provenance(),
            )
            .unwrap();
    }

    let one_record = strategy
        .lineage_view(
            &memory,
            &scope,
            &LineageQuery::new(
                PopulationId::new("population-1").unwrap(),
                ArtifactId::new("candidate-b").unwrap(),
                LineageDirection::Ancestors,
                LineageMemory::L1,
                1,
                1,
                2_048,
            )
            .unwrap(),
            Timestamp::from_unix_seconds(40),
        )
        .unwrap();
    assert_eq!(one_record.lessons().len(), 1);
    assert_eq!(one_record.lessons()[0].memory_id().as_str(), "lesson-a");

    let byte_bounded = strategy
        .lineage_view(
            &memory,
            &scope,
            &LineageQuery::new(
                PopulationId::new("population-1").unwrap(),
                ArtifactId::new("candidate-b").unwrap(),
                LineageDirection::Ancestors,
                LineageMemory::L1,
                1,
                16,
                9,
            )
            .unwrap(),
            Timestamp::from_unix_seconds(40),
        )
        .unwrap();
    assert_eq!(byte_bounded.lessons().len(), 1);
    assert_eq!(byte_bounded.total_bytes(), 5);

    let over_policy = LineageQuery::new(
        PopulationId::new("population-1").unwrap(),
        ArtifactId::new("candidate-b").unwrap(),
        LineageDirection::Ancestors,
        LineageMemory::Both,
        4,
        16,
        2_048,
    )
    .unwrap();
    assert_eq!(
        strategy.lineage_view(
            &memory,
            &scope,
            &over_policy,
            Timestamp::from_unix_seconds(40)
        ),
        Err(PopulationStrategyError::LineageLimitExceeded(
            "lineage depth"
        ))
    );
}
