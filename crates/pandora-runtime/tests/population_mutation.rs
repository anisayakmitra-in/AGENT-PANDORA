use pandora_runtime::{
    MutationEngine, MutationError, PopulationStrategy, PopulationStrategyError, StrategyProfile,
};
use pandora_types::{
    ArtifactId, EvaluationKind, EvaluationReceipt, EvaluationResult, EvaluationStatus,
    EvolutionPolicy, EvolutionSource, ExecutionId, LineageLimits, MutationLimits, MutationProposal,
    PopulationId, PopulationMutationRequest, PopulationPolicy, PrecheckDisposition, RequestDigest,
    SessionId, Timestamp, Usage,
};

fn policy() -> PopulationPolicy {
    PopulationPolicy::new(
        8,
        2,
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

fn request() -> PopulationMutationRequest {
    PopulationMutationRequest::new(
        PopulationId::new("population-1").unwrap(),
        1,
        ArtifactId::new("parent-1").unwrap(),
        ArtifactId::new("candidate-1").unwrap(),
        RequestDigest::new("plan-1").unwrap(),
        RequestDigest::new("batch-1").unwrap(),
    )
    .unwrap()
}

fn result(
    kind: EvaluationKind,
    status: EvaluationStatus,
    score: u8,
    advisory: bool,
) -> EvaluationResult {
    EvaluationResult::new(kind, status, score, "bounded precheck", advisory).unwrap()
}

fn receipt(results: Vec<EvaluationResult>) -> EvaluationReceipt {
    EvaluationReceipt::new(
        SessionId::new("session-1").unwrap(),
        ExecutionId::new("execution-1").unwrap(),
        Timestamp::from_unix_seconds(10),
        results,
    )
    .unwrap()
}

fn passing_receipt() -> EvaluationReceipt {
    receipt(vec![
        result(EvaluationKind::Policy, EvaluationStatus::Passed, 90, false),
        result(
            EvaluationKind::Regression,
            EvaluationStatus::Passed,
            85,
            false,
        ),
    ])
}

fn proposal(
    request: &PopulationMutationRequest,
    evidence_digest: RequestDigest,
) -> MutationProposal {
    MutationProposal::new(
        "proposal-1",
        EvolutionSource::Population,
        request.parent_artifact().clone(),
        request.candidate_artifact().clone(),
        evidence_digest,
        "reduce verified failures",
        Timestamp::from_unix_seconds(11),
    )
    .unwrap()
}

#[test]
fn passing_precheck_admits_a_digest_bound_population_proposal() {
    let request = request();
    let precheck = PopulationStrategy::new(StrategyProfile::Research, policy())
        .precheck(&request, &passing_receipt())
        .unwrap();

    assert_eq!(precheck.disposition(), PrecheckDisposition::Passed);
    assert!(!precheck.can_authorize_permit());

    let proposal = proposal(&request, precheck.digest().clone());
    let admitted = MutationEngine::new(EvolutionPolicy::research(1))
        .propose_population(&request, &precheck, proposal.clone())
        .unwrap();

    assert_eq!(admitted, proposal);
}

#[test]
fn precheck_rejects_missing_advisory_failed_and_low_scoring_results() {
    let strategy = PopulationStrategy::new(StrategyProfile::Research, policy());
    let request = request();
    let cases = [
        receipt(vec![result(
            EvaluationKind::Policy,
            EvaluationStatus::Passed,
            90,
            false,
        )]),
        receipt(vec![
            result(EvaluationKind::Policy, EvaluationStatus::Passed, 90, true),
            result(
                EvaluationKind::Regression,
                EvaluationStatus::Passed,
                90,
                false,
            ),
        ]),
        receipt(vec![
            result(EvaluationKind::Policy, EvaluationStatus::Failed, 90, false),
            result(
                EvaluationKind::Regression,
                EvaluationStatus::Passed,
                90,
                false,
            ),
        ]),
        receipt(vec![
            result(EvaluationKind::Policy, EvaluationStatus::Passed, 69, false),
            result(
                EvaluationKind::Regression,
                EvaluationStatus::Passed,
                90,
                false,
            ),
        ]),
        receipt(vec![
            result(EvaluationKind::Policy, EvaluationStatus::Passed, 90, false),
            result(
                EvaluationKind::Regression,
                EvaluationStatus::HumanReviewRequired,
                90,
                false,
            ),
        ]),
    ];

    for evaluation in cases {
        assert_eq!(
            strategy
                .precheck(&request, &evaluation)
                .unwrap()
                .disposition(),
            PrecheckDisposition::Rejected
        );
    }
}

#[test]
fn mutation_admission_rejects_mismatched_identity_and_evidence() {
    let request = request();
    let precheck = PopulationStrategy::new(StrategyProfile::Research, policy())
        .precheck(&request, &passing_receipt())
        .unwrap();
    let engine = MutationEngine::new(EvolutionPolicy::research(1));

    assert_eq!(
        engine.propose_population(
            &request,
            &precheck,
            MutationProposal::new(
                "proposal-1",
                EvolutionSource::Population,
                ArtifactId::new("different-parent").unwrap(),
                request.candidate_artifact().clone(),
                precheck.digest().clone(),
                "reduce verified failures",
                Timestamp::from_unix_seconds(11),
            )
            .unwrap(),
        ),
        Err(MutationError::BaseArtifactMismatch)
    );
    assert_eq!(
        engine.propose_population(
            &request,
            &precheck,
            proposal(&request, RequestDigest::new("different-evidence").unwrap()),
        ),
        Err(MutationError::EvidenceMismatch)
    );

    let mismatched_request = PopulationMutationRequest::new(
        PopulationId::new("population-1").unwrap(),
        1,
        ArtifactId::new("parent-1").unwrap(),
        ArtifactId::new("candidate-2").unwrap(),
        RequestDigest::new("plan-1").unwrap(),
        RequestDigest::new("batch-1").unwrap(),
    )
    .unwrap();
    assert_eq!(
        engine.propose_population(
            &mismatched_request,
            &precheck,
            proposal(&request, precheck.digest().clone()),
        ),
        Err(MutationError::PrecheckMismatch)
    );

    let rejected = PopulationStrategy::new(StrategyProfile::Research, policy())
        .precheck(
            &request,
            &receipt(vec![
                result(EvaluationKind::Policy, EvaluationStatus::Failed, 0, false),
                result(
                    EvaluationKind::Regression,
                    EvaluationStatus::Passed,
                    90,
                    false,
                ),
            ]),
        )
        .unwrap();
    assert_eq!(
        engine.propose_population(
            &request,
            &rejected,
            proposal(&request, rejected.digest().clone()),
        ),
        Err(MutationError::PrecheckRejected)
    );
}

#[test]
fn production_mode_cannot_precheck_or_admit_population_mutations() {
    let request = request();
    assert_eq!(
        PopulationStrategy::new(StrategyProfile::Production, policy())
            .precheck(&request, &passing_receipt()),
        Err(PopulationStrategyError::DisabledInProduction)
    );

    let research = PopulationStrategy::new(StrategyProfile::Research, policy())
        .precheck(&request, &passing_receipt())
        .unwrap();
    assert_eq!(
        MutationEngine::new(EvolutionPolicy::production(1)).propose_population(
            &request,
            &research,
            proposal(&request, research.digest().clone()),
        ),
        Err(MutationError::DisabledInProduction)
    );
}
