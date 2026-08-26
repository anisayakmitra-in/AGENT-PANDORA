use crate::adaptive_engine::{AdaptationResult, AdaptiveEngine, AdaptiveError};
use pandora_types::{AdaptationPolicy, AdaptationRequest, AdaptationTarget, Timestamp};
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SelfHealingError {
    Adaptation(AdaptiveError),
}

impl fmt::Display for SelfHealingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Adaptation(error) => {
                write!(formatter, "self-healing adaptation failed: {error}")
            }
        }
    }
}

impl std::error::Error for SelfHealingError {}

impl From<AdaptiveError> for SelfHealingError {
    fn from(error: AdaptiveError) -> Self {
        Self::Adaptation(error)
    }
}

pub struct SelfHealingEngine {
    adaptive: AdaptiveEngine,
}

impl SelfHealingEngine {
    pub fn new(policy: AdaptationPolicy) -> Self {
        Self {
            adaptive: AdaptiveEngine::new(policy),
        }
    }

    pub fn policy(&self) -> &AdaptationPolicy {
        self.adaptive.policy()
    }

    pub fn can_handle(&self, request: &AdaptationRequest) -> bool {
        request.candidates().iter().any(|candidate| {
            matches!(
                candidate.target(),
                AdaptationTarget::Recovery(_) | AdaptationTarget::CapabilityReduction(_)
            )
        })
    }

    pub fn recover(
        &self,
        request: &AdaptationRequest,
        now: Timestamp,
    ) -> Result<AdaptationResult, SelfHealingError> {
        Ok(self.adaptive.select_recovery(request, now)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::{
        AdaptationCandidate, AdaptationRequest, ExecutionId, RequestDigest, SessionId,
    };

    fn request(candidates: Vec<AdaptationCandidate>) -> AdaptationRequest {
        AdaptationRequest::new(
            ExecutionId::new("execution-1").unwrap(),
            SessionId::new("session-1").unwrap(),
            RequestDigest::new("pandora-request-v1:sha256:self-healing").unwrap(),
            Some(AdaptationTarget::Provider("primary".to_owned())),
            candidates,
        )
        .unwrap()
    }

    fn candidate(
        id: &str,
        target: AdaptationTarget,
        score: i32,
        approved: bool,
    ) -> AdaptationCandidate {
        AdaptationCandidate::new(id, target, score, approved, false, 100, 10).unwrap()
    }

    #[test]
    fn recovery_selects_only_bounded_recovery_targets() {
        let engine = SelfHealingEngine::new(AdaptationPolicy::new(1, 4, 500, 100).unwrap());
        let request = request(vec![
            candidate(
                "workflow",
                AdaptationTarget::Workflow("replan".to_owned()),
                100,
                true,
            ),
            candidate(
                "restart",
                AdaptationTarget::Recovery("restart-worker".to_owned()),
                10,
                true,
            ),
        ]);

        assert!(engine.can_handle(&request));
        let result = engine
            .recover(&request, Timestamp::from_unix_seconds(10))
            .unwrap();

        assert_eq!(
            result.decision().selected().unwrap().label(),
            "restart-worker"
        );
    }

    #[test]
    fn no_approved_recovery_target_enters_degraded_mode() {
        let engine = SelfHealingEngine::new(AdaptationPolicy::new(1, 4, 500, 100).unwrap());
        let request = request(vec![candidate(
            "restart",
            AdaptationTarget::Recovery("restart-worker".to_owned()),
            100,
            false,
        )]);

        let result = engine
            .recover(&request, Timestamp::from_unix_seconds(10))
            .unwrap();

        assert!(result.decision().selected().is_none());
        assert!(result.decision().degraded());
    }
}
