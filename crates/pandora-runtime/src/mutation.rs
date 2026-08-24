use pandora_types::{
    EvolutionContractError, EvolutionMode, EvolutionPolicy, EvolutionSource,
    MutationPrecheckReceipt, MutationProposal, PopulationMutationRequest, ReflexionArtifact,
};
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MutationError {
    DisabledInProduction,
    WrongProposalSource,
    PrecheckMismatch,
    PrecheckRejected,
    BaseArtifactMismatch,
    CandidateArtifactMismatch,
    EvidenceMismatch,
    Contract(EvolutionContractError),
}

impl fmt::Display for MutationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DisabledInProduction => {
                formatter.write_str("mutation proposals are disabled in production")
            }
            Self::WrongProposalSource => {
                formatter.write_str("proposal source does not match the mutation path")
            }
            Self::PrecheckMismatch => {
                formatter.write_str("precheck does not match the mutation request")
            }
            Self::PrecheckRejected => formatter.write_str("mutation precheck was rejected"),
            Self::BaseArtifactMismatch => {
                formatter.write_str("proposal base artifact does not match the mutation request")
            }
            Self::CandidateArtifactMismatch => formatter
                .write_str("proposal candidate artifact does not match the mutation request"),
            Self::EvidenceMismatch => {
                formatter.write_str("proposal evidence does not match the mutation precheck")
            }
            Self::Contract(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for MutationError {}

impl From<EvolutionContractError> for MutationError {
    fn from(error: EvolutionContractError) -> Self {
        Self::Contract(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MutationEngine {
    policy: EvolutionPolicy,
}

impl MutationEngine {
    pub const fn new(policy: EvolutionPolicy) -> Self {
        Self { policy }
    }

    pub const fn policy(&self) -> EvolutionPolicy {
        self.policy
    }

    pub fn record_reflexion(
        &self,
        reflection: ReflexionArtifact,
    ) -> Result<ReflexionArtifact, MutationError> {
        Ok(reflection)
    }

    pub fn propose_gepa(
        &self,
        proposal: MutationProposal,
    ) -> Result<MutationProposal, MutationError> {
        if self.policy.mode() != EvolutionMode::Research {
            return Err(MutationError::DisabledInProduction);
        }
        if proposal.source() != EvolutionSource::Gepa {
            return Err(MutationError::WrongProposalSource);
        }
        Ok(proposal)
    }

    pub fn propose_population(
        &self,
        request: &PopulationMutationRequest,
        precheck: &MutationPrecheckReceipt,
        proposal: MutationProposal,
    ) -> Result<MutationProposal, MutationError> {
        if self.policy.mode() != EvolutionMode::Research {
            return Err(MutationError::DisabledInProduction);
        }
        if proposal.source() != EvolutionSource::Population {
            return Err(MutationError::WrongProposalSource);
        }
        if precheck.request_digest() != request.request_digest() {
            return Err(MutationError::PrecheckMismatch);
        }
        if !precheck.passed() {
            return Err(MutationError::PrecheckRejected);
        }
        if proposal.base_artifact() != request.parent_artifact() {
            return Err(MutationError::BaseArtifactMismatch);
        }
        if proposal.candidate_artifact() != request.candidate_artifact() {
            return Err(MutationError::CandidateArtifactMismatch);
        }
        if proposal.evidence_digest() != precheck.digest() {
            return Err(MutationError::EvidenceMismatch);
        }
        Ok(proposal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::{ArtifactId, ExecutionId, RequestDigest, Timestamp};

    fn proposal() -> MutationProposal {
        MutationProposal::new(
            "proposal-1",
            EvolutionSource::Gepa,
            ArtifactId::new("base-1").unwrap(),
            ArtifactId::new("candidate-1").unwrap(),
            RequestDigest::new("evidence-1").unwrap(),
            "reduce repeated verification failures",
            Timestamp::from_unix_seconds(10),
        )
        .unwrap()
    }

    #[test]
    fn production_mutation_is_disabled() {
        assert_eq!(
            MutationEngine::new(EvolutionPolicy::production(1)).propose_gepa(proposal()),
            Err(MutationError::DisabledInProduction)
        );
    }

    #[test]
    fn research_mode_emits_a_proposal_without_applying_it() {
        let proposal = MutationEngine::new(EvolutionPolicy::research(1))
            .propose_gepa(proposal())
            .unwrap();

        assert_eq!(proposal.proposal_id().as_str(), "proposal-1");
        assert_eq!(proposal.source(), EvolutionSource::Gepa);
    }

    #[test]
    fn reflexion_is_an_observation_artifact() {
        let reflection = ReflexionArtifact::new(
            ExecutionId::new("execution-1").unwrap(),
            "verification failed",
            vec!["nonzero exit".to_owned()],
            "retry only after checking the command allowlist",
            Timestamp::from_unix_seconds(10),
        )
        .unwrap();

        let recorded = MutationEngine::new(EvolutionPolicy::production(1))
            .record_reflexion(reflection.clone())
            .unwrap();

        assert_eq!(recorded, reflection);
    }
}
