use pandora_types::{
    ArtifactSignature, EvolutionContractError, EvolutionPolicy, EvolutionState, HoldoutEvaluation,
    MutationProposal, ParliamentApproval, ProposalId,
};
use rusqlite::{Connection, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

const MAX_EVOLUTION_RECORD_BYTES: usize = 256 * 1024;

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

#[derive(Clone, Debug, Deserialize, Serialize)]
struct StoredProposal {
    proposal: MutationProposal,
    state: EvolutionState,
    evaluation: Option<HoldoutEvaluation>,
    approval: Option<ParliamentApproval>,
    signature: Option<ArtifactSignature>,
    canary: Option<pandora_types::CanaryResult>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvolutionRecord {
    proposal: MutationProposal,
    state: EvolutionState,
    evaluation: Option<HoldoutEvaluation>,
    approval: Option<ParliamentApproval>,
    signature: Option<ArtifactSignature>,
    canary: Option<pandora_types::CanaryResult>,
}

impl EvolutionRecord {
    pub fn proposal(&self) -> &MutationProposal {
        &self.proposal
    }

    pub const fn state(&self) -> EvolutionState {
        self.state
    }

    pub fn evaluation(&self) -> Option<&HoldoutEvaluation> {
        self.evaluation.as_ref()
    }

    pub fn approval(&self) -> Option<&ParliamentApproval> {
        self.approval.as_ref()
    }

    pub fn signature(&self) -> Option<&ArtifactSignature> {
        self.signature.as_ref()
    }

    pub fn canary(&self) -> Option<&pandora_types::CanaryResult> {
        self.canary.as_ref()
    }
}

impl StoredProposal {
    fn record(&self) -> EvolutionRecord {
        EvolutionRecord {
            proposal: self.proposal.clone(),
            state: self.state,
            evaluation: self.evaluation.clone(),
            approval: self.approval.clone(),
            signature: self.signature.clone(),
            canary: self.canary.clone(),
        }
    }

    fn validate(&self) -> Result<(), EvolutionStoreError> {
        MutationProposal::new(
            self.proposal.proposal_id().as_str().to_owned(),
            self.proposal.source(),
            self.proposal.base_artifact().clone(),
            self.proposal.candidate_artifact().clone(),
            self.proposal.evidence_digest().clone(),
            self.proposal.expected_outcome().to_owned(),
            self.proposal.created_at(),
        )
        .map_err(|_| EvolutionStoreError::CorruptRecord)?;
        if let Some(evaluation) = &self.evaluation
            && evaluation.proposal_id() != self.proposal.proposal_id()
        {
            return Err(EvolutionStoreError::CorruptRecord);
        }
        if let Some(approval) = &self.approval
            && approval.proposal_id() != self.proposal.proposal_id()
        {
            return Err(EvolutionStoreError::CorruptRecord);
        }
        if let Some(signature) = &self.signature {
            ArtifactSignature::new(
                signature.artifact_id().clone(),
                signature.signer().clone(),
                signature.signature().to_owned(),
            )
            .map_err(|_| EvolutionStoreError::CorruptRecord)?;
        }
        if let Some(canary) = &self.canary
            && canary.proposal_id() != self.proposal.proposal_id()
        {
            return Err(EvolutionStoreError::CorruptRecord);
        }
        Ok(())
    }
}

struct EvolutionStore {
    connection: Mutex<Connection>,
}

#[derive(Debug)]
enum EvolutionStoreError {
    Database,
    Io,
    Serialization,
    CorruptRecord,
    RecordTooLarge,
    LockPoisoned,
}

impl From<rusqlite::Error> for EvolutionStoreError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Database
    }
}

impl From<std::io::Error> for EvolutionStoreError {
    fn from(_: std::io::Error) -> Self {
        Self::Io
    }
}

impl From<serde_json::Error> for EvolutionStoreError {
    fn from(_: serde_json::Error) -> Self {
        Self::Serialization
    }
}

impl EvolutionStore {
    fn open(path: impl AsRef<Path>) -> Result<Self, EvolutionStoreError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        set_private_permissions(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS evolution_proposals (
                 proposal_id TEXT PRIMARY KEY NOT NULL,
                 record_json TEXT NOT NULL
             );",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn load(&self) -> Result<BTreeMap<ProposalId, StoredProposal>, EvolutionStoreError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| EvolutionStoreError::LockPoisoned)?;
        let mut statement =
            connection.prepare("SELECT proposal_id, record_json FROM evolution_proposals")?;
        let mut rows = statement.query([])?;
        let mut proposals = BTreeMap::new();
        while let Some(row) = rows.next()? {
            let proposal_id = ProposalId::new(row.get::<_, String>(0)?)
                .map_err(|_| EvolutionStoreError::CorruptRecord)?;
            let record_json = row.get::<_, String>(1)?;
            if record_json.len() > MAX_EVOLUTION_RECORD_BYTES {
                return Err(EvolutionStoreError::RecordTooLarge);
            }
            let record: StoredProposal = serde_json::from_str(&record_json)?;
            if record.proposal.proposal_id() != &proposal_id {
                return Err(EvolutionStoreError::CorruptRecord);
            }
            record.validate()?;
            if proposals.insert(proposal_id, record).is_some() {
                return Err(EvolutionStoreError::CorruptRecord);
            }
        }
        Ok(proposals)
    }

    fn save(&self, record: &StoredProposal) -> Result<(), EvolutionStoreError> {
        record.validate()?;
        let record_json = serde_json::to_string(record)?;
        if record_json.len() > MAX_EVOLUTION_RECORD_BYTES {
            return Err(EvolutionStoreError::RecordTooLarge);
        }
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| EvolutionStoreError::LockPoisoned)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO evolution_proposals (proposal_id, record_json)
             VALUES (?1, ?2)
             ON CONFLICT (proposal_id) DO UPDATE SET record_json = excluded.record_json",
            params![record.proposal.proposal_id().as_str(), record_json],
        )?;
        transaction.commit()?;
        Ok(())
    }
}

fn set_private_permissions(path: &Path) -> Result<(), EvolutionStoreError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

pub struct EvolutionEngine {
    policy: EvolutionPolicy,
    proposals: Mutex<BTreeMap<ProposalId, StoredProposal>>,
    durable: Option<EvolutionStore>,
}

impl EvolutionEngine {
    pub fn new(policy: EvolutionPolicy) -> Self {
        Self {
            policy,
            proposals: Mutex::new(BTreeMap::new()),
            durable: None,
        }
    }

    pub fn open(path: impl AsRef<Path>, policy: EvolutionPolicy) -> Result<Self, EvolutionError> {
        let durable = EvolutionStore::open(path).map_err(|_| EvolutionError::StoreUnavailable)?;
        let proposals = durable
            .load()
            .map_err(|_| EvolutionError::StoreUnavailable)?;
        Ok(Self {
            policy,
            proposals: Mutex::new(proposals),
            durable: Some(durable),
        })
    }

    pub const fn policy(&self) -> EvolutionPolicy {
        self.policy
    }

    pub fn submit(&self, proposal: MutationProposal) -> Result<(), EvolutionError> {
        let mut proposals = self.lock()?;
        if proposals.contains_key(proposal.proposal_id()) {
            return Err(EvolutionError::DuplicateProposal);
        }
        self.save_and_replace(
            &mut proposals,
            StoredProposal {
                proposal,
                state: EvolutionState::Proposed,
                evaluation: None,
                approval: None,
                signature: None,
                canary: None,
            },
        )?;
        Ok(())
    }

    pub fn record_evaluation(&self, evaluation: HoldoutEvaluation) -> Result<(), EvolutionError> {
        let mut proposals = self.lock()?;
        let record = proposals
            .get(evaluation.proposal_id())
            .ok_or(EvolutionError::NotFound)?;
        if record.state != EvolutionState::Proposed {
            return Err(EvolutionError::InvalidTransition(record.state));
        }
        let mut updated = record.clone();
        updated.evaluation = Some(evaluation);
        updated.state = EvolutionState::Evaluated;
        self.save_and_replace(&mut proposals, updated)?;
        Ok(())
    }

    pub fn approve(
        &self,
        proposal_id: &ProposalId,
        approval: ParliamentApproval,
        signature: ArtifactSignature,
    ) -> Result<(), EvolutionError> {
        let mut proposals = self.lock()?;
        let record = proposals.get(proposal_id).ok_or(EvolutionError::NotFound)?;
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
        let mut updated = record.clone();
        updated.approval = Some(approval);
        updated.signature = Some(signature);
        updated.state = EvolutionState::Approved;
        self.save_and_replace(&mut proposals, updated)?;
        Ok(())
    }

    pub fn inspect(&self, proposal_id: &ProposalId) -> Result<EvolutionRecord, EvolutionError> {
        let proposals = self.lock()?;
        Ok(proposals
            .get(proposal_id)
            .ok_or(EvolutionError::NotFound)?
            .record())
    }

    pub fn list(&self) -> Result<Vec<EvolutionRecord>, EvolutionError> {
        let proposals = self.lock()?;
        Ok(proposals.values().map(StoredProposal::record).collect())
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
        let record = proposals.get(proposal_id).ok_or(EvolutionError::NotFound)?;
        if record.state != EvolutionState::Approved {
            return Err(EvolutionError::InvalidTransition(record.state));
        }
        let mut updated = record.clone();
        updated.state = EvolutionState::Staged;
        self.save_and_replace(&mut proposals, updated)?;
        Ok(())
    }

    pub(crate) fn record_canary(
        &self,
        canary: pandora_types::CanaryResult,
    ) -> Result<(), EvolutionError> {
        let mut proposals = self.lock()?;
        let record = proposals
            .get(canary.proposal_id())
            .ok_or(EvolutionError::NotFound)?;
        if matches!(
            record.state,
            EvolutionState::CanaryPassed | EvolutionState::CanaryFailed
        ) && record.canary.as_ref() == Some(&canary)
        {
            return Ok(());
        }
        if record.state != EvolutionState::Staged {
            return Err(EvolutionError::InvalidTransition(record.state));
        }
        let mut updated = record.clone();
        updated.canary = Some(canary.clone());
        updated.state = if canary.passed() {
            EvolutionState::CanaryPassed
        } else {
            EvolutionState::CanaryFailed
        };
        self.save_and_replace(&mut proposals, updated)?;
        Ok(())
    }

    pub(crate) fn activate(
        &self,
        proposal_id: &ProposalId,
    ) -> Result<MutationProposal, EvolutionError> {
        let mut proposals = self.lock()?;
        let record = proposals.get(proposal_id).ok_or(EvolutionError::NotFound)?;
        let valid_state = if self.policy.requires_canary() {
            EvolutionState::CanaryPassed
        } else {
            EvolutionState::Approved
        };
        if record.state != valid_state {
            return Err(EvolutionError::InvalidTransition(record.state));
        }
        let mut updated = record.clone();
        updated.state = EvolutionState::Active;
        let proposal = updated.proposal.clone();
        self.save_and_replace(&mut proposals, updated)?;
        Ok(proposal)
    }

    pub(crate) fn rollback(
        &self,
        proposal_id: &ProposalId,
    ) -> Result<MutationProposal, EvolutionError> {
        let mut proposals = self.lock()?;
        let record = proposals.get(proposal_id).ok_or(EvolutionError::NotFound)?;
        if record.state != EvolutionState::Active {
            return Err(EvolutionError::InvalidTransition(record.state));
        }
        let mut updated = record.clone();
        updated.state = EvolutionState::RolledBack;
        let proposal = updated.proposal.clone();
        self.save_and_replace(&mut proposals, updated)?;
        Ok(proposal)
    }

    fn save_and_replace(
        &self,
        proposals: &mut BTreeMap<ProposalId, StoredProposal>,
        record: StoredProposal,
    ) -> Result<(), EvolutionError> {
        if let Some(durable) = &self.durable {
            durable
                .save(&record)
                .map_err(|_| EvolutionError::StoreUnavailable)?;
        }
        proposals.insert(record.proposal.proposal_id().clone(), record);
        Ok(())
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
    use pandora_types::{
        ArtifactId, CanaryResult, EvolutionSource, PrincipalId, RequestDigest, Timestamp,
    };
    use rusqlite::{Connection, params};

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

    #[test]
    fn exact_canary_retry_is_idempotent_but_conflicting_evidence_is_rejected() {
        let engine = EvolutionEngine::new(EvolutionPolicy::production(1));
        let proposal_id = ProposalId::new("proposal-1").unwrap();
        engine.submit(proposal()).unwrap();
        engine.record_evaluation(evaluation(true)).unwrap();
        engine
            .approve(
                &proposal_id,
                ParliamentApproval::new(
                    proposal_id.clone(),
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
        engine.stage(&proposal_id).unwrap();
        let canary = CanaryResult::new(
            proposal_id.clone(),
            true,
            0,
            "scheduled suite report sha256:exact",
            Timestamp::from_unix_seconds(22),
        )
        .unwrap();
        engine.record_canary(canary.clone()).unwrap();
        engine.record_canary(canary).unwrap();

        let conflicting = CanaryResult::new(
            proposal_id.clone(),
            false,
            1,
            "scheduled suite report sha256:changed",
            Timestamp::from_unix_seconds(22),
        )
        .unwrap();
        assert_eq!(
            engine.record_canary(conflicting),
            Err(EvolutionError::InvalidTransition(
                EvolutionState::CanaryPassed
            ))
        );
    }

    #[test]
    fn durable_engine_reopens_records_and_evidence() {
        let root = crate::test_support::new_temp_dir("pandora-evolution-store").unwrap();
        let path = root.join("evolution.sqlite3");
        let engine = EvolutionEngine::open(&path, EvolutionPolicy::production(1)).unwrap();
        engine.submit(proposal()).unwrap();
        engine.record_evaluation(evaluation(true)).unwrap();
        drop(engine);

        let reopened = EvolutionEngine::open(&path, EvolutionPolicy::production(1)).unwrap();
        let records = reopened.list().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].state(), EvolutionState::Evaluated);
        assert_eq!(
            records[0].evaluation().unwrap().outcome_score(),
            evaluation(true).outcome_score()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn durable_engine_fails_closed_on_corrupt_records() {
        let root = crate::test_support::new_temp_dir("pandora-evolution-corrupt").unwrap();
        let path = root.join("evolution.sqlite3");
        let engine = EvolutionEngine::open(&path, EvolutionPolicy::production(1)).unwrap();
        drop(engine);
        let connection = Connection::open(&path).unwrap();
        connection
            .execute(
                "INSERT INTO evolution_proposals (proposal_id, record_json) VALUES (?1, ?2)",
                params!["corrupt", "not-json"],
            )
            .unwrap();
        drop(connection);

        assert!(matches!(
            EvolutionEngine::open(&path, EvolutionPolicy::production(1)),
            Err(EvolutionError::StoreUnavailable)
        ));
        let _ = std::fs::remove_dir_all(root);
    }
}
