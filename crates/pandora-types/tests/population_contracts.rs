use pandora_types::{
    ArtifactId, CandidateDisposition, CandidateOutcome, FailureCorpus, FailureEvidence, FailureId,
    FailurePartition, GenerationReceipt, GenerationStats, LineageDirection, LineageLimits,
    LineageMemory, LineageQuery, MutationLimits, PopulationCandidate, PopulationContractError,
    PopulationId, PopulationPolicy, PopulationScope, RequestDigest, SessionId, TenantId, Timestamp,
    Usage, WorkspaceId,
};

fn failure(
    id: &str,
    partition: FailurePartition,
    category: &str,
    summary: &str,
) -> FailureEvidence {
    FailureEvidence::new(
        FailureId::new(id).unwrap(),
        partition,
        category,
        summary,
        RequestDigest::new(format!("digest-{id}")).unwrap(),
        Timestamp::from_unix_seconds(10),
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

fn candidate(id: &str) -> PopulationCandidate {
    PopulationCandidate::new(
        ArtifactId::new(id).unwrap(),
        Vec::new(),
        0,
        80,
        true,
        0,
        RequestDigest::new(format!("evaluation-{id}")).unwrap(),
        vec![FailureId::new("compile-a").unwrap()],
    )
    .unwrap()
}

#[test]
fn mutation_batches_never_expose_holdout_evidence() {
    let corpus = FailureCorpus::new(vec![
        failure(
            "compile-a",
            FailurePartition::Training,
            "compile",
            "missing import",
        ),
        failure(
            "secret-holdout",
            FailurePartition::Holdout,
            "compile",
            "sealed regression",
        ),
    ])
    .unwrap();

    let batch = corpus.mutation_batch(&[FailureId::new("compile-a").unwrap()], 4, 512);

    assert_eq!(batch.failures().len(), 1);
    assert_eq!(batch.failures()[0].id().as_str(), "compile-a");
    assert!(
        batch
            .failures()
            .iter()
            .all(|evidence| evidence.partition() == FailurePartition::Training)
    );
    assert_eq!(corpus.holdout_count(), 1);
    assert_ne!(corpus.holdout_digest(), batch.digest());
}

#[test]
fn mutation_batches_are_homogeneous_and_hard_bounded() {
    let corpus = FailureCorpus::new(vec![
        failure("a", FailurePartition::Training, "compile", "one"),
        failure("b", FailurePartition::Training, "compile", "two"),
        failure("c", FailurePartition::Training, "test", "three"),
    ])
    .unwrap();
    let eligible = vec![
        FailureId::new("a").unwrap(),
        FailureId::new("b").unwrap(),
        FailureId::new("c").unwrap(),
    ];

    let count_bounded = corpus.mutation_batch(&eligible, 1, 512);
    assert_eq!(count_bounded.failures().len(), 1);
    assert_eq!(count_bounded.failures()[0].category(), "compile");

    let byte_bounded = corpus.mutation_batch(&eligible, 3, 30);
    assert_eq!(byte_bounded.failures().len(), 1);
    assert!(byte_bounded.context_bytes() <= 30);
    assert!(
        byte_bounded
            .failures()
            .iter()
            .all(|evidence| evidence.category() == byte_bounded.category())
    );
}

#[test]
fn duplicate_failures_and_candidates_are_rejected() {
    let duplicate_failure = failure("same", FailurePartition::Training, "compile", "one");
    assert!(matches!(
        FailureCorpus::new(vec![duplicate_failure.clone(), duplicate_failure]),
        Err(PopulationContractError::DuplicateFailure(_))
    ));

    assert!(matches!(
        pandora_types::CandidatePopulation::new(
            scope(),
            0,
            vec![candidate("same"), candidate("same")]
        ),
        Err(PopulationContractError::DuplicateCandidate(_))
    ));
}

#[test]
fn population_policy_rejects_zero_or_excessive_limits() {
    assert_eq!(
        MutationLimits::new(0, 128, 1),
        Err(PopulationContractError::InvalidLimit(
            "max failures per batch"
        ))
    );
    assert_eq!(
        LineageLimits::new(1, 0, 128),
        Err(PopulationContractError::InvalidLimit("max lineage records"))
    );
    assert!(matches!(
        PopulationPolicy::new(
            8,
            2,
            3,
            8,
            MutationLimits::new(2, 128, 1).unwrap(),
            LineageLimits::new(2, 8, 1024).unwrap(),
            101,
            1000,
            Usage::new(1000, 10, 60, 10_000),
        ),
        Err(PopulationContractError::InvalidScore)
    ));
}

#[test]
fn lineage_queries_reject_zero_bounds() {
    assert_eq!(
        LineageQuery::new(
            PopulationId::new("population-1").unwrap(),
            ArtifactId::new("candidate-a").unwrap(),
            LineageDirection::Ancestors,
            LineageMemory::Both,
            0,
            8,
            1024,
        ),
        Err(PopulationContractError::InvalidLimit("lineage depth"))
    );
    assert_eq!(
        LineageQuery::new(
            PopulationId::new("population-1").unwrap(),
            ArtifactId::new("candidate-a").unwrap(),
            LineageDirection::Neighborhood,
            LineageMemory::L1,
            2,
            0,
            1024,
        ),
        Err(PopulationContractError::InvalidLimit("lineage records"))
    );
    assert_eq!(
        LineageQuery::new(
            PopulationId::new("population-1").unwrap(),
            ArtifactId::new("candidate-a").unwrap(),
            LineageDirection::Neighborhood,
            LineageMemory::L2,
            2,
            8,
            0,
        ),
        Err(PopulationContractError::InvalidLimit("lineage bytes"))
    );
}

#[test]
fn generation_receipts_reject_inconsistent_statistics() {
    let outcome = CandidateOutcome::new(
        ArtifactId::new("candidate-b").unwrap(),
        ArtifactId::new("candidate-a").unwrap(),
        CandidateDisposition::Accepted,
        RequestDigest::new("precheck-digest").unwrap(),
        Some(RequestDigest::new("evaluation-digest").unwrap()),
        Some(90),
        Usage::new(100, 1, 2, 300),
    )
    .unwrap();

    assert_eq!(
        GenerationReceipt::new(
            PopulationId::new("population-1").unwrap(),
            1,
            RequestDigest::new("plan-digest").unwrap(),
            RequestDigest::new("starting-digest").unwrap(),
            RequestDigest::new("resulting-digest").unwrap(),
            vec![outcome],
            GenerationStats::new(0, 0, 0, 0, Usage::new(0, 0, 0, 0)),
            Timestamp::from_unix_seconds(30),
        ),
        Err(PopulationContractError::InvalidGenerationStats)
    );
}
