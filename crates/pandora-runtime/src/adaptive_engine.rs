use pandora_types::{
    AdaptationDecision, AdaptationPolicy, AdaptationReceipt, AdaptationRequest, Timestamp,
};
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdaptiveError {
    InvalidPolicy(pandora_types::AdaptationContractError),
}

impl fmt::Display for AdaptiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPolicy(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for AdaptiveError {}

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
        let mut eligible = request
            .candidates()
            .iter()
            .take(self.policy.max_candidates())
            .filter(|candidate| candidate.approved())
            .filter(|candidate| candidate.cost_micros() <= self.policy.max_cost_micros())
            .filter(|candidate| candidate.latency_ms() <= self.policy.max_latency_ms())
            .collect::<Vec<_>>();
        eligible.sort_by(|left, right| {
            right
                .score()
                .cmp(&left.score())
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

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::{
        AdaptationCandidate, AdaptationRequest, AdaptationTarget, ExecutionId, GeneId, HarnessId,
        RequestDigest, SessionId,
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
}
