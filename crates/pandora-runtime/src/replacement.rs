use crate::evolution::{EvolutionEngine, EvolutionError};
use pandora_types::{
    CanaryResult, EvolutionContractError, ExecutionId, ProposalId, ReplacementReceipt,
    RollbackReceipt,
};
use std::collections::BTreeSet;
use std::fmt;
use std::sync::Mutex;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReplacementError {
    Evolution(EvolutionError),
    ExecutionActive,
    ExecutionNotFound,
    StoreUnavailable,
    Contract(EvolutionContractError),
}

impl fmt::Display for ReplacementError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Evolution(error) => error.fmt(formatter),
            Self::ExecutionActive => formatter.write_str("replacement is blocked during execution"),
            Self::ExecutionNotFound => formatter.write_str("execution was not registered"),
            Self::StoreUnavailable => formatter.write_str("replacement store is unavailable"),
            Self::Contract(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ReplacementError {}

impl From<EvolutionError> for ReplacementError {
    fn from(error: EvolutionError) -> Self {
        Self::Evolution(error)
    }
}

impl From<EvolutionContractError> for ReplacementError {
    fn from(error: EvolutionContractError) -> Self {
        Self::Contract(error)
    }
}

pub struct ReplacementEngine {
    active_executions: Mutex<BTreeSet<ExecutionId>>,
}

impl ReplacementEngine {
    pub fn new() -> Self {
        Self {
            active_executions: Mutex::new(BTreeSet::new()),
        }
    }

    pub fn begin_execution(&self, execution_id: ExecutionId) -> Result<(), ReplacementError> {
        let mut executions = self.lock()?;
        if !executions.insert(execution_id) {
            return Err(ReplacementError::ExecutionActive);
        }
        Ok(())
    }

    pub fn end_execution(&self, execution_id: &ExecutionId) -> Result<(), ReplacementError> {
        let mut executions = self.lock()?;
        if !executions.remove(execution_id) {
            return Err(ReplacementError::ExecutionNotFound);
        }
        Ok(())
    }

    pub fn stage(
        &self,
        evolution: &EvolutionEngine,
        proposal_id: &ProposalId,
    ) -> Result<(), ReplacementError> {
        evolution.stage(proposal_id).map_err(Into::into)
    }

    pub fn record_canary(
        &self,
        evolution: &EvolutionEngine,
        canary: CanaryResult,
    ) -> Result<(), ReplacementError> {
        evolution.record_canary(canary).map_err(Into::into)
    }

    pub fn activate(
        &self,
        evolution: &EvolutionEngine,
        proposal_id: &ProposalId,
        activated_at: pandora_types::Timestamp,
    ) -> Result<ReplacementReceipt, ReplacementError> {
        if !self.lock()?.is_empty() {
            return Err(ReplacementError::ExecutionActive);
        }
        let proposal = evolution.activate(proposal_id)?;
        Ok(ReplacementReceipt::new(
            proposal.proposal_id().clone(),
            proposal.base_artifact().clone(),
            proposal.candidate_artifact().clone(),
            activated_at,
        ))
    }

    pub fn rollback(
        &self,
        evolution: &EvolutionEngine,
        proposal_id: &ProposalId,
        rolled_back_at: pandora_types::Timestamp,
        reason: impl Into<String>,
    ) -> Result<RollbackReceipt, ReplacementError> {
        if !self.lock()?.is_empty() {
            return Err(ReplacementError::ExecutionActive);
        }
        let proposal = evolution.rollback(proposal_id)?;
        Ok(RollbackReceipt::new(
            proposal.proposal_id().clone(),
            proposal.base_artifact().clone(),
            rolled_back_at,
            reason,
        )?)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, BTreeSet<ExecutionId>>, ReplacementError> {
        self.active_executions
            .lock()
            .map_err(|_| ReplacementError::StoreUnavailable)
    }
}

impl Default for ReplacementEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::{
        ArtifactId, ArtifactSignature, EvolutionPolicy, EvolutionSource, HoldoutEvaluation,
        MutationProposal, ParliamentApproval, PrincipalId, RequestDigest, Timestamp,
    };

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

    #[test]
    fn activation_is_blocked_until_execution_finishes() {
        let evolution = EvolutionEngine::new(EvolutionPolicy::production(1));
        evolution.submit(proposal()).unwrap();
        evolution
            .record_evaluation(HoldoutEvaluation::new(
                ProposalId::new("proposal-1").unwrap(),
                90,
                95,
                true,
                true,
                true,
                Timestamp::from_unix_seconds(20),
            ))
            .unwrap();
        evolution
            .approve(
                &ProposalId::new("proposal-1").unwrap(),
                ParliamentApproval::new(
                    ProposalId::new("proposal-1").unwrap(),
                    PrincipalId::new("parliament-1").unwrap(),
                    1,
                    Timestamp::from_unix_seconds(21),
                ),
                ArtifactSignature::new(
                    ArtifactId::new("candidate-1").unwrap(),
                    PrincipalId::new("signer-1").unwrap(),
                    "signed-candidate",
                )
                .unwrap(),
            )
            .unwrap();

        let replacement = ReplacementEngine::new();
        replacement
            .stage(&evolution, &ProposalId::new("proposal-1").unwrap())
            .unwrap();
        replacement
            .record_canary(
                &evolution,
                CanaryResult::new(
                    ProposalId::new("proposal-1").unwrap(),
                    true,
                    0,
                    "canary remained within the failure budget",
                    Timestamp::from_unix_seconds(22),
                )
                .unwrap(),
            )
            .unwrap();
        let execution = ExecutionId::new("execution-1").unwrap();
        replacement.begin_execution(execution.clone()).unwrap();

        assert_eq!(
            replacement.activate(
                &evolution,
                &ProposalId::new("proposal-1").unwrap(),
                Timestamp::from_unix_seconds(23),
            ),
            Err(ReplacementError::ExecutionActive)
        );

        replacement.end_execution(&execution).unwrap();
        let receipt = replacement
            .activate(
                &evolution,
                &ProposalId::new("proposal-1").unwrap(),
                Timestamp::from_unix_seconds(23),
            )
            .unwrap();
        assert_eq!(receipt.candidate_artifact().as_str(), "candidate-1");

        let rollback = replacement
            .rollback(
                &evolution,
                &ProposalId::new("proposal-1").unwrap(),
                Timestamp::from_unix_seconds(24),
                "canary regression observed after activation",
            )
            .unwrap();
        assert_eq!(rollback.restored_artifact().as_str(), "base-1");
    }
}
