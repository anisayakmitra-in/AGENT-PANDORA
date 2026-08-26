use crate::efficiency_engine::{EfficiencyEngine, EfficiencyError};
use pandora_types::{
    AdaptationCandidate, AdaptationDecision, AdaptationPolicy, AdaptationReceipt,
    AdaptationRequest, AdaptationTarget, EfficiencyObjective, EfficiencySummary, Timestamp,
};
use std::cmp::Ordering;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdaptiveError {
    InvalidPolicy(pandora_types::AdaptationContractError),
    Efficiency(EfficiencyError),
}

impl fmt::Display for AdaptiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPolicy(error) => error.fmt(formatter),
            Self::Efficiency(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for AdaptiveError {}

impl From<EfficiencyError> for AdaptiveError {
    fn from(error: EfficiencyError) -> Self {
        Self::Efficiency(error)
    }
}

pub struct AdaptiveEngine {
    policy: AdaptationPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdaptationResult {
    decision: AdaptationDecision,
    receipt: AdaptationReceipt,
}

impl AdaptationResult {
    pub fn decision(&self) -> &AdaptationDecision {
        &self.decision
    }

    pub fn receipt(&self) -> &AdaptationReceipt {
        &self.receipt
    }
}

impl AdaptiveEngine {
    pub fn new(policy: AdaptationPolicy) -> Self {
        Self { policy }
    }

    pub fn policy(&self) -> &AdaptationPolicy {
        &self.policy
    }

    pub fn select(
        &self,
        request: &AdaptationRequest,
        now: Timestamp,
    ) -> Result<AdaptationResult, AdaptiveError> {
        self.select_internal(request, now, None, accept_all)
    }

    pub fn select_recovery(
        &self,
        request: &AdaptationRequest,
        now: Timestamp,
    ) -> Result<AdaptationResult, AdaptiveError> {
        self.select_internal(request, now, None, is_recovery_candidate)
    }

    pub fn select_with_efficiency(
        &self,
        request: &AdaptationRequest,
        now: Timestamp,
        efficiency: &EfficiencyEngine,
        task_class: &str,
        objective: EfficiencyObjective,
    ) -> Result<AdaptationResult, AdaptiveError> {
        let ranking = efficiency.rank(task_class, objective)?;
        self.select_internal(request, now, Some(&ranking), accept_all)
    }

    fn select_internal(
        &self,
        request: &AdaptationRequest,
        now: Timestamp,
        efficiency_ranking: Option<&[EfficiencySummary]>,
        candidate_filter: fn(&AdaptationCandidate) -> bool,
    ) -> Result<AdaptationResult, AdaptiveError> {
        let mut eligible = request
            .candidates()
            .iter()
            .filter(|candidate| candidate_filter(candidate))
            .take(self.policy.max_candidates())
            .filter(|candidate| candidate.approved())
            .filter(|candidate| candidate.cost_micros() <= self.policy.max_cost_micros())
            .filter(|candidate| candidate.latency_ms() <= self.policy.max_latency_ms())
            .collect::<Vec<_>>();
        eligible.sort_by(|left, right| {
            compare_evidence(
                left.target().label(),
                right.target().label(),
                efficiency_ranking,
            )
            .then_with(|| right.score().cmp(&left.score()))
            .then_with(|| left.id().cmp(right.id()))
        });

        let decision = if let Some(candidate) = eligible.first() {
            let changed = request.current() != Some(candidate.target());
            AdaptationDecision::Selected {
                target: candidate.target().clone(),
                changed,
                reason: "selected approved candidate within policy ceilings".to_owned(),
            }
        } else {
            let had_approved = request
                .candidates()
                .iter()
                .filter(|candidate| candidate_filter(candidate))
                .take(self.policy.max_candidates())
                .any(|candidate| candidate.approved());
            let reason = if had_approved {
                "no approved candidate remained within policy ceilings"
            } else {
                "no approved adaptation candidate was available"
            };
            AdaptationDecision::NoChange {
                degraded: true,
                reason: reason.to_owned(),
            }
        };
        let receipt = AdaptationReceipt::new(request, self.policy.policy_version(), &decision, now);
        Ok(AdaptationResult { decision, receipt })
    }
}

fn accept_all(_: &AdaptationCandidate) -> bool {
    true
}

fn is_recovery_candidate(candidate: &AdaptationCandidate) -> bool {
    matches!(
        candidate.target(),
        AdaptationTarget::Recovery(_) | AdaptationTarget::CapabilityReduction(_)
    )
}

fn compare_evidence(
    left_target: &str,
    right_target: &str,
    ranking: Option<&[EfficiencySummary]>,
) -> Ordering {
    let Some(ranking) = ranking else {
        return Ordering::Equal;
    };
    let left_rank = ranking
        .iter()
        .position(|summary| summary.target() == left_target);
    let right_rank = ranking
        .iter()
        .position(|summary| summary.target() == right_target);
    match (left_rank, right_rank) {
        (Some(left), Some(right)) => left.cmp(&right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::{
        AdaptationCandidate, AdaptationRequest, AdaptationTarget, EfficiencyObjective,
        EfficiencySample, ExecutionId, GeneId, HarnessId, RequestDigest, SessionId,
    };

    fn candidate(
        id: &str,
        target: AdaptationTarget,
        score: i32,
        approved: bool,
    ) -> AdaptationCandidate {
        AdaptationCandidate::new(id, target, score, approved, false, 100, 10).unwrap()
    }

    fn request(candidates: Vec<AdaptationCandidate>) -> AdaptationRequest {
        AdaptationRequest::new(
            ExecutionId::new("execution-1").unwrap(),
            SessionId::new("session-1").unwrap(),
            RequestDigest::new("pandora-request-v1:sha256:test").unwrap(),
            Some(AdaptationTarget::Provider("primary".to_owned())),
            candidates,
        )
        .unwrap()
    }

    #[test]
    fn deterministic_selection_chooses_the_highest_approved_candidate() {
        let engine = AdaptiveEngine::new(AdaptationPolicy::new(7, 4, 500, 100).unwrap());
        let request = request(vec![
            candidate(
                "backup",
                AdaptationTarget::Provider("backup".to_owned()),
                80,
                true,
            ),
            candidate(
                "blocked",
                AdaptationTarget::Provider("blocked".to_owned()),
                100,
                false,
            ),
        ]);

        let first = engine
            .select(&request, Timestamp::from_unix_seconds(10))
            .unwrap();
        let second = engine
            .select(&request, Timestamp::from_unix_seconds(11))
            .unwrap();

        assert_eq!(first.decision(), second.decision());
        assert!(first.decision().changed());
        assert_eq!(first.decision().selected().unwrap().label(), "backup");
    }

    #[test]
    fn provider_failover_stays_within_the_approved_candidate_set() {
        let engine = AdaptiveEngine::new(AdaptationPolicy::new(7, 4, 500, 100).unwrap());
        let result = engine
            .select(
                &request(vec![candidate(
                    "backup",
                    AdaptationTarget::Provider("backup".to_owned()),
                    90,
                    true,
                )]),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();

        assert_eq!(result.decision().selected().unwrap().label(), "backup");
        assert!(!result.decision().degraded());
    }

    #[test]
    fn no_approved_candidate_enters_degraded_mode_without_authority_change() {
        let engine = AdaptiveEngine::new(AdaptationPolicy::new(7, 4, 500, 100).unwrap());
        let result = engine
            .select(
                &request(vec![candidate(
                    "unapproved",
                    AdaptationTarget::Harness(HarnessId::new("other").unwrap()),
                    90,
                    false,
                )]),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();

        assert!(!result.decision().changed());
        assert!(result.decision().degraded());
        assert!(result.decision().selected().is_none());
    }

    #[test]
    fn policy_ceilings_exclude_expensive_candidates() {
        let expensive = AdaptationCandidate::new(
            "expensive",
            AdaptationTarget::Gene(GeneId::new("costly").unwrap()),
            100,
            true,
            false,
            1_000,
            10,
        )
        .unwrap();
        let engine = AdaptiveEngine::new(AdaptationPolicy::new(7, 4, 500, 100).unwrap());
        let result = engine
            .select(&request(vec![expensive]), Timestamp::from_unix_seconds(10))
            .unwrap();

        assert!(result.decision().selected().is_none());
        assert!(result.receipt().reason().contains("ceiling"));
    }

    #[test]
    fn evidence_ranking_can_prefer_a_lower_cost_target() {
        let efficiency = EfficiencyEngine::new(8).unwrap();
        efficiency
            .record(
                EfficiencySample::new(
                    ExecutionId::new("efficiency-1").unwrap(),
                    "coding",
                    "cheap",
                    20,
                    10,
                    10,
                    100,
                    true,
                    Timestamp::from_unix_seconds(10),
                )
                .unwrap(),
            )
            .unwrap();
        efficiency
            .record(
                EfficiencySample::new(
                    ExecutionId::new("efficiency-2").unwrap(),
                    "coding",
                    "expensive",
                    20,
                    10,
                    100,
                    10,
                    true,
                    Timestamp::from_unix_seconds(10),
                )
                .unwrap(),
            )
            .unwrap();
        let request = request(vec![
            candidate(
                "expensive",
                AdaptationTarget::Provider("expensive".to_owned()),
                100,
                true,
            ),
            candidate(
                "cheap",
                AdaptationTarget::Provider("cheap".to_owned()),
                1,
                true,
            ),
        ]);

        let result = AdaptiveEngine::new(AdaptationPolicy::new(7, 4, 500, 100).unwrap())
            .select_with_efficiency(
                &request,
                Timestamp::from_unix_seconds(10),
                &efficiency,
                "coding",
                EfficiencyObjective::LowestCost,
            )
            .unwrap();

        assert_eq!(result.decision().selected().unwrap().label(), "cheap");
    }

    #[test]
    fn missing_efficiency_evidence_keeps_score_based_selection_for_unmatched_targets() {
        let efficiency = EfficiencyEngine::new(8).unwrap();
        let request = request(vec![
            candidate(
                "higher-score",
                AdaptationTarget::Provider("higher-score".to_owned()),
                100,
                true,
            ),
            candidate(
                "lower-score",
                AdaptationTarget::Provider("lower-score".to_owned()),
                1,
                true,
            ),
        ]);

        let result = AdaptiveEngine::new(AdaptationPolicy::new(7, 4, 500, 100).unwrap())
            .select_with_efficiency(
                &request,
                Timestamp::from_unix_seconds(10),
                &efficiency,
                "coding",
                EfficiencyObjective::HighestCertainty,
            )
            .unwrap();

        assert_eq!(
            result.decision().selected().unwrap().label(),
            "higher-score"
        );
    }
}
