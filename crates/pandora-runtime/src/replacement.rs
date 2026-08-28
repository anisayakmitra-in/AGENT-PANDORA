use crate::artifact_catalog::{ArtifactCatalog, ArtifactCatalogError};
use crate::evolution::{EvolutionEngine, EvolutionError};
use crate::package_store::{PackageStore, PackageStoreError};
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
    PackageStoreUnavailable,
    ArtifactCatalog(ArtifactCatalogError),
    BaseArtifactNotAdmitted,
    CandidateArtifactNotAdmitted,
}

impl fmt::Display for ReplacementError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Evolution(error) => error.fmt(formatter),
            Self::ExecutionActive => formatter.write_str("replacement is blocked during execution"),
            Self::ExecutionNotFound => formatter.write_str("execution was not registered"),
            Self::StoreUnavailable => formatter.write_str("replacement store is unavailable"),
            Self::Contract(error) => error.fmt(formatter),
            Self::PackageStoreUnavailable => formatter.write_str("package store is unavailable"),
            Self::ArtifactCatalog(error) => error.fmt(formatter),
            Self::BaseArtifactNotAdmitted => {
                formatter.write_str("base artifact is not present in the admitted package store")
            }
            Self::CandidateArtifactNotAdmitted => formatter
                .write_str("candidate artifact is not present in the admitted package store"),
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

impl From<PackageStoreError> for ReplacementError {
    fn from(_: PackageStoreError) -> Self {
        Self::PackageStoreUnavailable
    }
}

impl From<ArtifactCatalogError> for ReplacementError {
    fn from(error: ArtifactCatalogError) -> Self {
        Self::ArtifactCatalog(error)
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
        let _executions = self.quiescent()?;
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
        let _executions = self.quiescent()?;
        let record = evolution.inspect(proposal_id)?;
        let receipt = RollbackReceipt::new(
            record.proposal().proposal_id().clone(),
            record.proposal().base_artifact().clone(),
            rolled_back_at,
            reason,
        )?;
        evolution.rollback(proposal_id)?;
        Ok(receipt)
    }

    pub fn activate_admitted(
        &self,
        evolution: &EvolutionEngine,
        packages: &PackageStore,
        catalog: &ArtifactCatalog,
        proposal_id: &ProposalId,
        activated_at: pandora_types::Timestamp,
    ) -> Result<ReplacementReceipt, ReplacementError> {
        let record = evolution.inspect(proposal_id)?;
        if !packages.contains_artifact(record.proposal().base_artifact())? {
            return Err(ReplacementError::BaseArtifactNotAdmitted);
        }
        if !packages.contains_artifact(record.proposal().candidate_artifact())? {
            return Err(ReplacementError::CandidateArtifactNotAdmitted);
        }
        let _executions = self.quiescent()?;
        let proposal = evolution.activate(proposal_id)?;
        let receipt = ReplacementReceipt::new(
            proposal.proposal_id().clone(),
            proposal.base_artifact().clone(),
            proposal.candidate_artifact().clone(),
            activated_at,
        );
        if let Err(error) = catalog.activate(&receipt) {
            let _ = evolution.rollback(proposal_id);
            return Err(error.into());
        }
        Ok(receipt)
    }

    pub fn rollback_admitted(
        &self,
        evolution: &EvolutionEngine,
        catalog: &ArtifactCatalog,
        proposal_id: &ProposalId,
        rolled_back_at: pandora_types::Timestamp,
        reason: impl Into<String>,
    ) -> Result<RollbackReceipt, ReplacementError> {
        let _executions = self.quiescent()?;
        let reason = reason.into();
        let activation = catalog
            .inspect(proposal_id)?
            .ok_or(ArtifactCatalogError::ProposalNotActive)?;
        let catalog_receipt = RollbackReceipt::new(
            proposal_id.clone(),
            activation.base_artifact().clone(),
            rolled_back_at,
            reason.clone(),
        )?;
        catalog.rollback(&catalog_receipt)?;
        match evolution.rollback(proposal_id) {
            Ok(proposal) => Ok(RollbackReceipt::new(
                proposal.proposal_id().clone(),
                proposal.base_artifact().clone(),
                rolled_back_at,
                reason,
            )?),
            Err(error) => {
                let _ = catalog.activate(&ReplacementReceipt::new(
                    activation.proposal_id().clone(),
                    activation.base_artifact().clone(),
                    activation.candidate_artifact().clone(),
                    activation.activated_at(),
                ));
                Err(error.into())
            }
        }
    }

    fn quiescent(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, BTreeSet<ExecutionId>>, ReplacementError> {
        let executions = self.lock()?;
        if executions.is_empty() {
            Ok(executions)
        } else {
            Err(ReplacementError::ExecutionActive)
        }
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
        MutationProposal, PackageCompatibility, PackageKind, PackageManifest, ParliamentApproval,
        PrincipalId, RequestDigest, Timestamp, TrustEvidence, hash_artifact,
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

    fn admitted_manifest(id: &str, artifact: &[u8]) -> PackageManifest {
        PackageManifest::new(
            id,
            "1.0.0",
            PackageKind::Gene,
            "publisher",
            hash_artifact(artifact),
            Vec::new(),
            PackageCompatibility::new(concat!("pandora>=", env!("CARGO_PKG_VERSION"))).unwrap(),
            "MIT",
            TrustEvidence::unsigned(),
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

        assert!(
            replacement
                .rollback(
                    &evolution,
                    &ProposalId::new("proposal-1").unwrap(),
                    Timestamp::from_unix_seconds(24),
                    "",
                )
                .is_err()
        );
        assert_eq!(
            evolution
                .state(&ProposalId::new("proposal-1").unwrap())
                .unwrap(),
            pandora_types::EvolutionState::Active
        );

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

    #[test]
    fn admitted_activation_changes_the_durable_artifact_resolver_and_rolls_back() {
        let root = crate::test_support::new_temp_dir("pandora-admitted-replacement").unwrap();
        let packages = PackageStore::open(root.join("packages.sqlite3")).unwrap();
        let catalog = ArtifactCatalog::open(root.join("artifact-catalog.sqlite3")).unwrap();
        let base_bytes = b"base gene artifact";
        let candidate_bytes = b"candidate gene artifact";
        let base_manifest = admitted_manifest("publisher/base", base_bytes);
        let candidate_manifest = admitted_manifest("publisher/candidate", candidate_bytes);
        packages
            .admit(&base_manifest, &base_manifest, base_bytes)
            .unwrap();
        let base = ArtifactId::new(base_manifest.content_hash()).unwrap();
        let candidate = ArtifactId::new(candidate_manifest.content_hash()).unwrap();
        let proposal_id = ProposalId::new("proposal-admitted").unwrap();
        let evolution = EvolutionEngine::new(EvolutionPolicy::production(1));
        evolution
            .submit(
                MutationProposal::new(
                    proposal_id.as_str(),
                    EvolutionSource::Gepa,
                    base.clone(),
                    candidate.clone(),
                    RequestDigest::new("evidence-admitted").unwrap(),
                    "replace the admitted gene artifact",
                    Timestamp::from_unix_seconds(10),
                )
                .unwrap(),
            )
            .unwrap();
        evolution
            .record_evaluation(HoldoutEvaluation::new(
                proposal_id.clone(),
                100,
                100,
                true,
                true,
                true,
                Timestamp::from_unix_seconds(11),
            ))
            .unwrap();
        evolution
            .approve(
                &proposal_id,
                ParliamentApproval::new(
                    proposal_id.clone(),
                    PrincipalId::new("parliament-1").unwrap(),
                    1,
                    Timestamp::from_unix_seconds(12),
                ),
                ArtifactSignature::new(
                    candidate.clone(),
                    PrincipalId::new("signer-1").unwrap(),
                    "signed-candidate",
                )
                .unwrap(),
            )
            .unwrap();
        let replacement = ReplacementEngine::new();
        replacement.stage(&evolution, &proposal_id).unwrap();
        replacement
            .record_canary(
                &evolution,
                CanaryResult::new(
                    proposal_id.clone(),
                    true,
                    0,
                    "canary passed",
                    Timestamp::from_unix_seconds(13),
                )
                .unwrap(),
            )
            .unwrap();

        assert_eq!(
            replacement.activate_admitted(
                &evolution,
                &packages,
                &catalog,
                &proposal_id,
                Timestamp::from_unix_seconds(14),
            ),
            Err(ReplacementError::CandidateArtifactNotAdmitted)
        );
        assert_eq!(catalog.resolve(&base).unwrap(), base);
        assert_eq!(
            evolution.state(&proposal_id).unwrap(),
            pandora_types::EvolutionState::CanaryPassed
        );

        packages
            .admit(&candidate_manifest, &candidate_manifest, candidate_bytes)
            .unwrap();
        replacement
            .activate_admitted(
                &evolution,
                &packages,
                &catalog,
                &proposal_id,
                Timestamp::from_unix_seconds(14),
            )
            .unwrap();
        assert_eq!(catalog.resolve(&base).unwrap(), candidate);
        replacement
            .rollback_admitted(
                &evolution,
                &catalog,
                &proposal_id,
                Timestamp::from_unix_seconds(15),
                "post-activation regression",
            )
            .unwrap();
        assert_eq!(catalog.resolve(&base).unwrap(), base);
        assert_eq!(
            evolution.state(&proposal_id).unwrap(),
            pandora_types::EvolutionState::RolledBack
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
