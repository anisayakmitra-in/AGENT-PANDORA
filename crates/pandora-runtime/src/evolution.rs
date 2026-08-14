use pandora_types::{
    ArtifactSignature, EvolutionContractError, EvolutionPolicy, EvolutionState, HoldoutEvaluation,
    MutationProposal, ParliamentApproval, ProposalId,
};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Mutex;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EvolutionError {
    Contract(EvolutionContractError),
    DuplicateProposal,
    NotFound,
    InvalidTransition(EvolutionState),
    ProposalMismatch,
    EvaluationRequired,
    HoldoutFailed,
    PolicyFailed,
    RegressionFailed,
    SignatureRequired,
    SignatureMismatch,
    ApprovalMismatch,
    PolicyVersionMismatch,
    StoreUnavailable,
}

impl fmt::Display for EvolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(error) => error.fmt(formatter),
            Self::DuplicateProposal => formatter.write_str("evolution proposal already exists"),
            Self::NotFound => formatter.write_str("evolution proposal was not found"),
            Self::InvalidTransition(state) => {
                write!(
                    formatter,
                    "evolution proposal is already {}",
                    state.as_str()
                )
            }
            Self::ProposalMismatch => formatter.write_str("evidence does not match the proposal"),
            Self::EvaluationRequired => formatter.write_str("a holdout evaluation is required"),
            Self::HoldoutFailed => formatter.write_str("holdout evaluation failed"),
            Self::PolicyFailed => formatter.write_str("policy evaluation failed"),
            Self::RegressionFailed => formatter.write_str("regression evaluation failed"),
            Self::SignatureRequired => formatter.write_str("signed artifact evidence is required"),
            Self::SignatureMismatch => {
                formatter.write_str("signature evidence does not match the candidate artifact")
            }
            Self::ApprovalMismatch => formatter.write_str("Parliament approval does not match"),
            Self::PolicyVersionMismatch => formatter.write_str("policy version does not match"),
            Self::StoreUnavailable => formatter.write_str("evolution store is unavailable"),
        }
    }
}

impl std::error::Error for EvolutionError {}

impl From<EvolutionContractError> for EvolutionError {
    fn from(error: EvolutionContractError) -> Self {
        Self::Contract(error)
    }
}

struct StoredProposal {
    proposal: MutationProposal,
    state: EvolutionState,
    evaluation: Option<HoldoutEvaluation>,
    approval: Option<ParliamentApproval>,
    signature: Option<ArtifactSignature>,
    canary: Option<pandora_types::CanaryResult>,
}

pub struct EvolutionEngine {
    policy: EvolutionPolicy,
    proposals: Mutex<BTreeMap<ProposalId, StoredProposal>>,
}

impl EvolutionEngine {
    pub fn new(policy: EvolutionPolicy) -> Self {
        Self {
            policy,
            proposals: Mutex::new(BTreeMap::new()),
        }
    }

    pub const fn policy(&self) -> EvolutionPolicy {
        self.policy
    }

    pub fn submit(&self, proposal: MutationProposal) -> Result<(), EvolutionError> {
        let mut proposals = self.lock()?;
        if proposals.contains_key(proposal.proposal_id()) {
            return Err(EvolutionError::DuplicateProposal);
        }
        proposals.insert(
            proposal.proposal_id().clone(),
            StoredProposal {
                proposal,
                state: EvolutionState::Proposed,
                evaluation: None,
                approval: None,
                signature: None,
                canary: None,
            },
        );
        Ok(())
    }

    pub fn record_evaluation(&self, evaluation: HoldoutEvaluation) -> Result<(), EvolutionError> {
        let mut proposals = self.lock()?;
        let record = proposals
            .get_mut(evaluation.proposal_id())
            .ok_or(EvolutionError::NotFound)?;
        if record.state != EvolutionState::Proposed {
            return Err(EvolutionError::InvalidTransition(record.state));
        }
        record.evaluation = Some(evaluation);
        record.state = EvolutionState::Evaluated;
        Ok(())
    }

    pub fn approve(
        &self,
        proposal_id: &ProposalId,
        approval: ParliamentApproval,
        signature: ArtifactSignature,
    ) -> Result<(), EvolutionError> {
        let mut proposals = self.lock()?;
        let record = proposals
            .get_mut(proposal_id)
            .ok_or(EvolutionError::NotFound)?;
        if record.state != EvolutionState::Evaluated {
            return Err(EvolutionError::InvalidTransition(record.state));
        }
        let evaluation = record
            .evaluation
            .as_ref()
            .ok_or(EvolutionError::EvaluationRequired)?;
        if self.policy.requires_holdout() && !evaluation.holdout_passed() {
            return Err(EvolutionError::HoldoutFailed);
        }
        if !evaluation.policy_passed() {
            return Err(EvolutionError::PolicyFailed);
        }
        if !evaluation.regression_passed() {
            return Err(EvolutionError::RegressionFailed);
        }
        if approval.proposal_id() != proposal_id {
            return Err(EvolutionError::ApprovalMismatch);
        }
        if approval.policy_version() != self.policy.policy_version() {
            return Err(EvolutionError::PolicyVersionMismatch);
        }
        if self.policy.requires_signature() && signature.signature().is_empty() {
            return Err(EvolutionError::SignatureRequired);
        }
        if signature.artifact_id() != record.proposal.candidate_artifact() {
            return Err(EvolutionError::SignatureMismatch);
        }
        record.approval = Some(approval);
        record.signature = Some(signature);
        record.state = EvolutionState::Approved;
        Ok(())
    }

    pub fn state(&self, proposal_id: &ProposalId) -> Result<EvolutionState, EvolutionError> {
        let proposals = self.lock()?;
        Ok(proposals
            .get(proposal_id)
            .ok_or(EvolutionError::NotFound)?
            .state)
    }

    pub fn canary(
        &self,
        proposal_id: &ProposalId,
    ) -> Result<Option<pandora_types::CanaryResult>, EvolutionError> {
        let proposals = self.lock()?;
        Ok(proposals
            .get(proposal_id)
            .ok_or(EvolutionError::NotFound)?
            .canary
            .clone())
    }

    pub(crate) fn stage(&self, proposal_id: &ProposalId) -> Result<(), EvolutionError> {
        let mut proposals = self.lock()?;
        let record = proposals
            .get_mut(proposal_id)
            .ok_or(EvolutionError::NotFound)?;
        if record.state != EvolutionState::Approved {
            return Err(EvolutionError::InvalidTransition(record.state));
        }
        record.state = EvolutionState::Staged;
        Ok(())
    }

    pub(crate) fn record_canary(
        &self,
        canary: pandora_types::CanaryResult,
    ) -> Result<(), EvolutionError> {
        let mut proposals = self.lock()?;
        let record = proposals
            .get_mut(canary.proposal_id())
            .ok_or(EvolutionError::NotFound)?;
        if record.state != EvolutionState::Staged {
            return Err(EvolutionError::InvalidTransition(record.state));
        }
        record.canary = Some(canary.clone());
        record.state = if canary.passed() {
            EvolutionState::CanaryPassed
        } else {
            EvolutionState::CanaryFailed
        };
        Ok(())
    }

    pub(crate) fn activate(
        &self,
        proposal_id: &ProposalId,
    ) -> Result<MutationProposal, EvolutionError> {
        let mut proposals = self.lock()?;
        let record = proposals
            .get_mut(proposal_id)
            .ok_or(EvolutionError::NotFound)?;
        let valid_state = if self.policy.requires_canary() {
            EvolutionState::CanaryPassed
        } else {
            EvolutionState::Approved
        };
        if record.state != valid_state {
            return Err(EvolutionError::InvalidTransition(record.state));
        }
        record.state = EvolutionState::Active;
        Ok(record.proposal.clone())
    }

    pub(crate) fn rollback(
        &self,
        proposal_id: &ProposalId,
    ) -> Result<MutationProposal, EvolutionError> {
        let mut proposals = self.lock()?;
        let record = proposals
            .get_mut(proposal_id)
            .ok_or(EvolutionError::NotFound)?;
        if record.state != EvolutionState::Active {
            return Err(EvolutionError::InvalidTransition(record.state));
        }
        record.state = EvolutionState::RolledBack;
        Ok(record.proposal.clone())
    }

    fn lock(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, BTreeMap<ProposalId, StoredProposal>>, EvolutionError>
    {
        self.proposals
            .lock()
            .map_err(|_| EvolutionError::StoreUnavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::{ArtifactId, EvolutionSource, PrincipalId, RequestDigest, Timestamp};

    fn proposal() -> MutationProposal {
        MutationProposal::new(
            "proposal-1",
            EvolutionSource::Gepa,
            ArtifactId::new("base-1").unwrap(),
            ArtifactId::new("candidate-1").unwrap(),
            RequestDigest::new("evidence-1").unwrap(),
            "improve verification reliability",
            Timestamp::from_unix_seconds(10),
        )
        .unwrap()
    }

    fn evaluation(passed: bool) -> HoldoutEvaluation {
        HoldoutEvaluation::new(
            ProposalId::new("proposal-1").unwrap(),
            90,
            95,
            passed,
            passed,
            passed,
            Timestamp::from_unix_seconds(20),
        )
    }

    #[test]
    fn approval_requires_holdout_and_policy_evidence() {
        let engine = EvolutionEngine::new(EvolutionPolicy::production(1));
        engine.submit(proposal()).unwrap();
        engine.record_evaluation(evaluation(false)).unwrap();

        let approval = ParliamentApproval::new(
            ProposalId::new("proposal-1").unwrap(),
            PrincipalId::new("parliament-1").unwrap(),
            1,
            Timestamp::from_unix_seconds(21),
        );
        let signature = ArtifactSignature::new(
            ArtifactId::new("candidate-1").unwrap(),
            PrincipalId::new("signer-1").unwrap(),
            "signed-candidate",
        )
        .unwrap();

        assert_eq!(
            engine.approve(&ProposalId::new("proposal-1").unwrap(), approval, signature),
            Err(EvolutionError::HoldoutFailed)
        );
    }

    #[test]
    fn successful_evidence_reaches_approved_state_only() {
        let engine = EvolutionEngine::new(EvolutionPolicy::production(1));
        engine.submit(proposal()).unwrap();
        engine.record_evaluation(evaluation(true)).unwrap();
        let approval = ParliamentApproval::new(
            ProposalId::new("proposal-1").unwrap(),
            PrincipalId::new("parliament-1").unwrap(),
            1,
            Timestamp::from_unix_seconds(21),
        );
        let signature = ArtifactSignature::new(
            ArtifactId::new("candidate-1").unwrap(),
            PrincipalId::new("signer-1").unwrap(),
            "signed-candidate",
        )
        .unwrap();

        engine
            .approve(&ProposalId::new("proposal-1").unwrap(), approval, signature)
            .unwrap();

        assert_eq!(
            engine
                .state(&ProposalId::new("proposal-1").unwrap())
                .unwrap(),
            EvolutionState::Approved
        );
    }
}
