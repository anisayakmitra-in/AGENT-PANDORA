use pandora_types::{
    ArtifactSignature, EvolutionContractError, EvolutionPolicy, EvolutionPromotionApproval,
    EvolutionRollout, EvolutionRolloutBinding, EvolutionScorecard, EvolutionStageLimits,
    EvolutionState, HoldoutEvaluation, MemoryIntent, MutationProposal, ParliamentApproval,
    PrincipalId, ProposalId, RequestDigest, SopDefinition, SopStep, Timestamp,
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
    RolloutAlreadyConfigured,
    RolloutRequired,
    RolloutIncomplete,
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
            Self::RolloutAlreadyConfigured => {
                formatter.write_str("governed rollout is already configured")
            }
            Self::RolloutRequired => formatter.write_str("governed rollout is not configured"),
            Self::RolloutIncomplete => {
                formatter.write_str("governed rollout has not completed every approved stage")
            }
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
    #[serde(default)]
    rollout: Option<EvolutionRollout>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvolutionRecord {
    proposal: MutationProposal,
    state: EvolutionState,
    evaluation: Option<HoldoutEvaluation>,
    approval: Option<ParliamentApproval>,
    signature: Option<ArtifactSignature>,
    canary: Option<pandora_types::CanaryResult>,
    rollout: Option<EvolutionRollout>,
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

    pub fn rollout(&self) -> Option<&EvolutionRollout> {
        self.rollout.as_ref()
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
            rollout: self.rollout.clone(),
        }
    }

    fn validate(&self) -> Result<(), EvolutionStoreError> {
        let validated = MutationProposal::new(
            self.proposal.proposal_id().as_str().to_owned(),
            self.proposal.source(),
            self.proposal.base_artifact().clone(),
            self.proposal.candidate_artifact().clone(),
            self.proposal.evidence_digest().clone(),
            self.proposal.expected_outcome().to_owned(),
            self.proposal.created_at(),
        )
        .and_then(|proposal| {
            proposal.with_memory_evidence_ids(self.proposal.memory_evidence_ids().to_vec())
        })
        .map_err(|_| EvolutionStoreError::CorruptRecord)?;
        if validated.memory_evidence_ids() != self.proposal.memory_evidence_ids() {
            return Err(EvolutionStoreError::CorruptRecord);
        }
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
        if let Some(rollout) = &self.rollout {
            rollout
                .validate()
                .map_err(|_| EvolutionStoreError::CorruptRecord)?;
            if rollout.binding().evidence_digest() != self.proposal.evidence_digest()
                || rollout
                    .pending_approval()
                    .is_some_and(|approval| approval.proposal_id() != self.proposal.proposal_id())
            {
                return Err(EvolutionStoreError::CorruptRecord);
            }
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
                rollout: None,
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

    pub fn configure_rollout(
        &self,
        proposal_id: &ProposalId,
        binding: EvolutionRolloutBinding,
        limits: Vec<EvolutionStageLimits>,
        transition_id: RequestDigest,
        actor: PrincipalId,
        now: Timestamp,
    ) -> Result<bool, EvolutionError> {
        let mut proposals = self.lock()?;
        let record = proposals.get(proposal_id).ok_or(EvolutionError::NotFound)?;
        if record.state != EvolutionState::CanaryPassed {
            return Err(EvolutionError::InvalidTransition(record.state));
        }
        if let Some(rollout) = &record.rollout {
            if rollout.configuration_replay(&binding, &limits, &transition_id, &actor, now)? {
                return Ok(false);
            }
            return Err(EvolutionError::RolloutAlreadyConfigured);
        }
        if binding.evidence_digest() != record.proposal.evidence_digest() {
            return Err(EvolutionError::ProposalMismatch);
        }
        let mut updated = record.clone();
        updated.rollout = Some(EvolutionRollout::new(
            binding,
            limits,
            transition_id,
            actor,
            now,
        )?);
        self.save_and_replace(&mut proposals, updated)?;
        Ok(true)
    }

    pub fn record_rollout_scorecard(
        &self,
        proposal_id: &ProposalId,
        scorecard: EvolutionScorecard,
        transition_id: RequestDigest,
        actor: PrincipalId,
    ) -> Result<bool, EvolutionError> {
        self.mutate_rollout(proposal_id, |rollout| {
            rollout.record_scorecard(scorecard, transition_id, actor)
        })
    }

    pub fn approve_rollout_promotion(
        &self,
        proposal_id: &ProposalId,
        approval: EvolutionPromotionApproval,
        transition_id: RequestDigest,
        now: Timestamp,
    ) -> Result<bool, EvolutionError> {
        self.mutate_rollout(proposal_id, |rollout| {
            rollout.approve(proposal_id, approval, transition_id, now)
        })
    }

    pub fn promote_rollout(
        &self,
        proposal_id: &ProposalId,
        transition_id: RequestDigest,
        actor: PrincipalId,
        now: Timestamp,
        reason: impl Into<String>,
    ) -> Result<bool, EvolutionError> {
        let reason = reason.into();
        self.mutate_rollout(proposal_id, |rollout| {
            rollout.promote(transition_id, actor, now, reason)
        })
    }

    pub fn pause_rollout(
        &self,
        proposal_id: &ProposalId,
        transition_id: RequestDigest,
        actor: PrincipalId,
        now: Timestamp,
        reason: impl Into<String>,
    ) -> Result<bool, EvolutionError> {
        let reason = reason.into();
        self.mutate_rollout(proposal_id, |rollout| {
            rollout.pause(transition_id, actor, now, reason)
        })
    }

    pub fn resume_rollout(
        &self,
        proposal_id: &ProposalId,
        transition_id: RequestDigest,
        actor: PrincipalId,
        now: Timestamp,
        reason: impl Into<String>,
    ) -> Result<bool, EvolutionError> {
        let reason = reason.into();
        self.mutate_rollout(proposal_id, |rollout| {
            rollout.resume(transition_id, actor, now, reason)
        })
    }

    pub fn reject_rollout_promotion(
        &self,
        proposal_id: &ProposalId,
        transition_id: RequestDigest,
        actor: PrincipalId,
        now: Timestamp,
        reason: impl Into<String>,
    ) -> Result<bool, EvolutionError> {
        let reason = reason.into();
        self.mutate_rollout(proposal_id, |rollout| {
            rollout.reject(transition_id, actor, now, reason)
        })
    }

    pub fn retry_rollout_stage(
        &self,
        proposal_id: &ProposalId,
        transition_id: RequestDigest,
        actor: PrincipalId,
        now: Timestamp,
        reason: impl Into<String>,
    ) -> Result<bool, EvolutionError> {
        let reason = reason.into();
        self.mutate_rollout(proposal_id, |rollout| {
            rollout.retry(transition_id, actor, now, reason)
        })
    }

    pub fn rollback_rollout(
        &self,
        proposal_id: &ProposalId,
        transition_id: RequestDigest,
        actor: PrincipalId,
        now: Timestamp,
        reason: impl Into<String>,
    ) -> Result<bool, EvolutionError> {
        let reason = reason.into();
        self.mutate_rollout(proposal_id, |rollout| {
            rollout.mark_rolled_back(transition_id, actor, now, reason)
        })
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
        if record
            .rollout
            .as_ref()
            .is_some_and(|rollout| !rollout.activation_ready())
        {
            return Err(EvolutionError::RolloutIncomplete);
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

    fn mutate_rollout<F>(
        &self,
        proposal_id: &ProposalId,
        mutation: F,
    ) -> Result<bool, EvolutionError>
    where
        F: FnOnce(&mut EvolutionRollout) -> Result<bool, EvolutionContractError>,
    {
        let mut proposals = self.lock()?;
        let record = proposals.get(proposal_id).ok_or(EvolutionError::NotFound)?;
        if record.state != EvolutionState::CanaryPassed {
            return Err(EvolutionError::InvalidTransition(record.state));
        }
        let mut updated = record.clone();
        let rollout = updated
            .rollout
            .as_mut()
            .ok_or(EvolutionError::RolloutRequired)?;
        let changed = mutation(rollout)?;
        if changed {
            self.save_and_replace(&mut proposals, updated)?;
        }
        Ok(changed)
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParetoCandidate {
    pub id: String,
    pub pass_rate: f64,
    pub token_cost: u32,
    pub latency_ms: u64,
}

impl ParetoCandidate {
    pub fn new(id: impl Into<String>, pass_rate: f64, token_cost: u32, latency_ms: u64) -> Self {
        let pr = if pass_rate.is_nan() {
            0.0
        } else {
            pass_rate.clamp(0.0, 1.0)
        };
        Self {
            id: id.into(),
            pass_rate: pr,
            token_cost,
            latency_ms,
        }
    }

    /// Returns true if `self` dominates `other`.
    /// Dominance condition: at least as good in all 3 objectives, strictly better in at least one.
    /// Objectives:
    /// - pass_rate: maximize
    /// - token_cost: minimize
    /// - latency_ms: minimize
    pub fn dominates(&self, other: &Self) -> bool {
        let no_worse = self.pass_rate >= other.pass_rate
            && self.token_cost <= other.token_cost
            && self.latency_ms <= other.latency_ms;
        let strictly_better = self.pass_rate > other.pass_rate
            || self.token_cost < other.token_cost
            || self.latency_ms < other.latency_ms;
        no_worse && strictly_better
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ParetoFrontier {
    candidates: Vec<ParetoCandidate>,
}

impl ParetoFrontier {
    pub fn new() -> Self {
        Self {
            candidates: Vec::new(),
        }
    }

    pub fn candidates(&self) -> &[ParetoCandidate] {
        &self.candidates
    }

    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }

    pub fn len(&self) -> usize {
        self.candidates.len()
    }

    pub fn is_dominated(&self, candidate: &ParetoCandidate) -> bool {
        self.candidates
            .iter()
            .any(|existing| existing.dominates(candidate))
    }

    /// Attempts to insert a candidate into the Pareto frontier.
    /// Returns true if candidate is accepted (non-dominated).
    /// If accepted, any existing candidates that are dominated by this candidate are pruned.
    pub fn try_insert(&mut self, candidate: ParetoCandidate) -> bool {
        if self.is_dominated(&candidate) {
            return false;
        }
        self.candidates
            .retain(|existing| !candidate.dominates(existing));
        if !self.candidates.iter().any(|c| c.id == candidate.id) {
            self.candidates.push(candidate);
        } else if let Some(existing) = self.candidates.iter_mut().find(|c| c.id == candidate.id) {
            *existing = candidate;
        }
        true
    }

    pub fn best_by_pass_rate(&self) -> Option<&ParetoCandidate> {
        self.candidates.iter().max_by(|a, b| {
            a.pass_rate
                .partial_cmp(&b.pass_rate)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BayesianCanaryVerdict {
    pub passes: u32,
    pub failures: u32,
    pub alpha: f64,
    pub beta: f64,
    pub lower_bound_95: f64,
    pub baseline_rate: f64,
    pub promoted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BayesianCanaryGate {
    credible_level: f64,
}

impl Default for BayesianCanaryGate {
    fn default() -> Self {
        Self::new(0.95)
    }
}

impl BayesianCanaryGate {
    pub fn new(credible_level: f64) -> Self {
        let level = if credible_level.is_nan() || credible_level <= 0.0 || credible_level >= 1.0 {
            0.95
        } else {
            credible_level
        };
        Self {
            credible_level: level,
        }
    }

    pub fn credible_level(&self) -> f64 {
        self.credible_level
    }

    /// Evaluates Beta(1 + passes, 1 + failures) distribution.
    /// Calculates the lower credible bound at credible_level (e.g. 95%).
    /// Promotes the canary only when the lower bound strictly exceeds baseline_rate.
    pub fn evaluate(
        &self,
        passes: u32,
        failures: u32,
        baseline_rate: f64,
    ) -> BayesianCanaryVerdict {
        let alpha = 1.0 + f64::from(passes);
        let beta = 1.0 + f64::from(failures);
        let target_quantile = (1.0 - self.credible_level).clamp(1e-6, 1.0 - 1e-6);
        let lower_bound_95 = beta_quantile(alpha, beta, target_quantile);
        let promoted = !baseline_rate.is_nan() && lower_bound_95 > baseline_rate;

        BayesianCanaryVerdict {
            passes,
            failures,
            alpha,
            beta,
            lower_bound_95,
            baseline_rate,
            promoted,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IslandNiche {
    QualityFirst,
    CostMinimal,
    LatencyOptimized,
}

impl IslandNiche {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::QualityFirst => "quality_first",
            Self::CostMinimal => "cost_minimal",
            Self::LatencyOptimized => "latency_optimized",
        }
    }

    pub fn standard_niches() -> [Self; 3] {
        [
            Self::QualityFirst,
            Self::CostMinimal,
            Self::LatencyOptimized,
        ]
    }

    pub fn compute_fitness(&self, pass_rate: f64, token_cost: u32, latency_ms: u64) -> f64 {
        let pr = if pass_rate.is_nan() {
            0.0
        } else {
            pass_rate.clamp(0.0, 1.0)
        };
        match self {
            Self::QualityFirst => {
                pr * 1000.0 - (token_cost as f64 * 0.01) - (latency_ms as f64 * 0.02)
            }
            Self::CostMinimal => {
                let cost_score = (10_000.0 - (token_cost as f64).min(10_000.0)).max(0.0);
                cost_score + (pr * 500.0) - (latency_ms as f64 * 0.01)
            }
            Self::LatencyOptimized => {
                let latency_score = (5_000.0 - (latency_ms as f64).min(5_000.0)).max(0.0);
                latency_score + (pr * 500.0) - (token_cost as f64 * 0.005)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IslandCandidate {
    pub id: String,
    pub generation: u32,
    pub sop: SopDefinition,
    pub pass_rate: f64,
    pub token_cost: u32,
    pub latency_ms: u64,
    pub fitness: f64,
    pub origin_island: String,
}

impl IslandCandidate {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        generation: u32,
        sop: SopDefinition,
        pass_rate: f64,
        token_cost: u32,
        latency_ms: u64,
        niche: IslandNiche,
        origin_island: impl Into<String>,
    ) -> Self {
        let fitness = niche.compute_fitness(pass_rate, token_cost, latency_ms);
        Self {
            id: id.into(),
            generation,
            sop,
            pass_rate,
            token_cost,
            latency_ms,
            fitness,
            origin_island: origin_island.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Island {
    pub id: String,
    pub niche: IslandNiche,
    pub population: Vec<IslandCandidate>,
    pub generation: u32,
}

impl Island {
    pub fn new(id: impl Into<String>, niche: IslandNiche) -> Self {
        Self {
            id: id.into(),
            niche,
            population: Vec::new(),
            generation: 0,
        }
    }

    pub fn sort_by_fitness(&mut self) {
        self.population.sort_by(|a, b| {
            b.fitness
                .partial_cmp(&a.fitness)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    pub fn best_candidate(&self) -> Option<&IslandCandidate> {
        self.population.iter().max_by(|a, b| {
            a.fitness
                .partial_cmp(&b.fitness)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }
}

pub fn evaluate_sop_metrics(sop: &SopDefinition) -> (f64, u32, u64) {
    let step_count = sop.steps.len() as f64;
    let has_verification = sop.verification_command.is_some();
    let has_audit_step = sop.steps.iter().any(|s| {
        s.command.contains("verify") || s.command.contains("check") || s.command.contains("test")
    });

    let mut quality = 0.70 + (step_count * 0.03).min(0.15);
    if has_verification {
        quality += 0.08;
    }
    if has_audit_step {
        quality += 0.06;
    }
    let pass_rate = quality.min(0.99);

    let command_tokens: usize = sop.steps.iter().map(|s| s.command.len() / 4).sum();
    let token_cost =
        (500 + (sop.steps.len() * 180) + if has_verification { 250 } else { 0 } + command_tokens)
            as u32;

    let latency_ms =
        (200 + (sop.steps.len() * 120) + if has_verification { 150 } else { 0 }) as u64;

    (pass_rate, token_cost, latency_ms)
}

pub fn tournament_selection(island: &Island, k: usize, seed: u64) -> Option<&IslandCandidate> {
    if island.population.is_empty() {
        return None;
    }
    let pop_len = island.population.len();
    let k = k.clamp(1, pop_len);
    let mut best: Option<&IslandCandidate> = None;
    for i in 0..k {
        let idx = ((seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(i as u64 + 1)) as usize)
            % pop_len;
        let cand = &island.population[idx];
        match best {
            None => best = Some(cand),
            Some(curr) if cand.fitness > curr.fitness => best = Some(cand),
            _ => {}
        }
    }
    best
}

pub fn crossover_sop_steps(
    parent_a: &SopDefinition,
    parent_b: &SopDefinition,
    child_id: &str,
    crossover_point: usize,
) -> Result<SopDefinition, pandora_types::MemoryContractError> {
    let mut child_steps: Vec<SopStep> = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();

    let split = crossover_point.min(parent_a.steps.len());
    for step in &parent_a.steps[..split] {
        if seen_ids.insert(step.id.clone()) {
            child_steps.push(step.clone());
        }
    }

    for step in &parent_b.steps {
        if !seen_ids.contains(&step.id) {
            seen_ids.insert(step.id.clone());
            let valid_deps: Vec<String> = step
                .depends_on
                .iter()
                .filter(|dep| child_steps.iter().any(|s| &s.id == *dep))
                .cloned()
                .collect();
            let mut step_clone = step.clone();
            step_clone.depends_on = valid_deps;
            child_steps.push(step_clone);
        }
    }

    if child_steps.is_empty() {
        child_steps = parent_a.steps.clone();
    }

    for i in 0..child_steps.len() {
        let valid_prior: Vec<String> = child_steps[..i].iter().map(|s| s.id.clone()).collect();
        child_steps[i]
            .depends_on
            .retain(|dep| valid_prior.contains(dep));
    }

    let verification_cmd = parent_a
        .verification_command
        .clone()
        .or_else(|| parent_b.verification_command.clone());

    SopDefinition::new(
        child_id,
        format!("Recombined SOP {child_id}"),
        parent_a.trigger_intent,
        child_steps,
        verification_cmd,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MutationKind {
    PromptCommand,
    StepReorder,
    AddVerificationCheckpoint,
}

pub fn mutate_sop(
    parent: &SopDefinition,
    child_id: &str,
    kind: MutationKind,
) -> Result<SopDefinition, pandora_types::MemoryContractError> {
    let mut steps = parent.steps.clone();
    let mut verification = parent.verification_command.clone();

    match kind {
        MutationKind::PromptCommand => {
            if let Some(first) = steps.first_mut() {
                if first.command.contains(" --cached") {
                    first.command = first.command.replace(" --cached", "");
                } else if first.command.contains(" --strict") {
                    first.command = first.command.replace(" --strict", " --cached");
                } else {
                    first.command.push_str(" --strict");
                }
            }
        }
        MutationKind::StepReorder => {
            if steps.len() >= 2 {
                for i in 0..steps.len() - 1 {
                    if !steps[i + 1].depends_on.contains(&steps[i].id) {
                        steps.swap(i, i + 1);
                        break;
                    }
                }
            }
        }
        MutationKind::AddVerificationCheckpoint => {
            let checkpoint_id = format!("verify-check-{}", steps.len() + 1);
            let prev_id = steps.last().map(|s| s.id.clone());
            let deps = match prev_id {
                Some(id) => vec![id],
                None => Vec::new(),
            };
            steps.push(SopStep::new(
                checkpoint_id,
                "Canary checkpoint audit",
                "pandora decision evaluate --question 'Verify candidate state' --binary",
                deps,
            )?);
            verification = Some("cargo test --workspace --locked".to_string());
        }
    }

    for i in 0..steps.len() {
        let valid_prior: Vec<String> = steps[..i].iter().map(|s| s.id.clone()).collect();
        steps[i].depends_on.retain(|dep| valid_prior.contains(dep));
    }

    SopDefinition::new(
        child_id,
        format!("Mutated SOP {child_id}"),
        parent.trigger_intent,
        steps,
        verification,
    )
}

pub fn default_evolution_sop() -> SopDefinition {
    SopDefinition::new(
        "sop-base-evolution",
        "Standard Operational Procedure for Autonomous Verification",
        MemoryIntent::Coding,
        vec![
            SopStep::new(
                "step-1-inspect",
                "Inspect Environment State",
                "pandora decision evaluate --question 'Verify system readiness' --binary",
                Vec::new(),
            )
            .unwrap(),
            SopStep::new(
                "step-2-execute",
                "Execute Surgical Operations",
                "cargo check --workspace --locked",
                vec!["step-1-inspect".to_string()],
            )
            .unwrap(),
        ],
        Some("cargo test --workspace --locked".to_string()),
    )
    .unwrap()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IslandSummary {
    pub island_id: String,
    pub niche: IslandNiche,
    pub generation: u32,
    pub population_size: usize,
    pub best_fitness: f64,
    pub best_pass_rate: f64,
    pub best_token_cost: u32,
    pub best_latency_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvolutionStepReport {
    pub generation: u32,
    pub islands: Vec<IslandSummary>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MigrationExchange {
    pub from_island: String,
    pub to_island: String,
    pub candidate_id: String,
    pub fitness_in_target: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MigrationReport {
    pub generation: u32,
    pub migrated_count: usize,
    pub exchanges: Vec<MigrationExchange>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IslandRunReport {
    pub total_generations: u32,
    pub islands: Vec<IslandSummary>,
    pub step_reports: Vec<EvolutionStepReport>,
    pub migration_reports: Vec<MigrationReport>,
    pub pareto_frontier_size: usize,
    pub pareto_candidates: Vec<ParetoCandidate>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IslandModelEvolver {
    pub islands: Vec<Island>,
    pub migration_interval: u32,
    pub migration_count: usize,
    pub current_generation: u32,
}

impl IslandModelEvolver {
    pub fn new(islands: Vec<Island>, migration_interval: u32, migration_count: usize) -> Self {
        Self {
            islands,
            migration_interval: migration_interval.max(1),
            migration_count: migration_count.max(1),
            current_generation: 0,
        }
    }

    pub fn new_default(base_sop: SopDefinition, population_size: usize) -> Self {
        let mut islands = Vec::new();
        for niche in IslandNiche::standard_niches() {
            let island_id = format!("island-{}", niche.as_str());
            let mut island = Island::new(&island_id, niche);
            let pop_size = population_size.max(2);
            for p in 0..pop_size {
                let cand_sop = if p == 0 {
                    base_sop.clone()
                } else {
                    let mut variant = base_sop.clone();
                    variant.id = format!("{island_id}-0-p{p}");
                    if p % 3 == 0 {
                        variant = mutate_sop(&base_sop, &variant.id, MutationKind::PromptCommand)
                            .unwrap_or_else(|_| base_sop.clone());
                    } else if p % 3 == 1 {
                        variant = mutate_sop(&base_sop, &variant.id, MutationKind::StepReorder)
                            .unwrap_or_else(|_| base_sop.clone());
                    } else {
                        variant = mutate_sop(
                            &base_sop,
                            &variant.id,
                            MutationKind::AddVerificationCheckpoint,
                        )
                        .unwrap_or_else(|_| base_sop.clone());
                    }
                    variant
                };
                let (pass_rate, token_cost, latency_ms) = evaluate_sop_metrics(&cand_sop);
                let cand = IslandCandidate::new(
                    format!("cand-{}-0-{}", niche.as_str(), p),
                    0,
                    cand_sop,
                    pass_rate,
                    token_cost,
                    latency_ms,
                    niche,
                    &island_id,
                );
                island.population.push(cand);
            }
            island.sort_by_fitness();
            islands.push(island);
        }
        Self::new(islands, 3, 1)
    }

    pub fn step(&mut self) -> EvolutionStepReport {
        self.current_generation += 1;
        let generation = self.current_generation;
        let mut summaries = Vec::new();

        for (island_idx, island) in self.islands.iter_mut().enumerate() {
            island.generation = generation;
            island.sort_by_fitness();

            let pop_len = island.population.len();
            let mut next_population = Vec::with_capacity(pop_len);

            if let Some(elite) = island.population.first() {
                next_population.push(elite.clone());
            }

            let mut seed = (generation as u64)
                .wrapping_mul(1000)
                .wrapping_add(island_idx as u64);
            while next_population.len() < pop_len {
                seed = seed.wrapping_add(17);
                let parent_a = tournament_selection(island, 3, seed)
                    .cloned()
                    .unwrap_or_else(|| island.population[0].clone());
                seed = seed.wrapping_add(31);
                let parent_b = tournament_selection(island, 3, seed)
                    .cloned()
                    .unwrap_or_else(|| island.population[0].clone());

                let child_id = format!(
                    "sop-{}-g{}-{}",
                    island.niche.as_str(),
                    generation,
                    next_population.len()
                );
                let crossover_pt = if parent_a.sop.steps.len() > 1 {
                    ((seed as usize) % (parent_a.sop.steps.len() - 1)) + 1
                } else {
                    1
                };
                let crossed =
                    crossover_sop_steps(&parent_a.sop, &parent_b.sop, &child_id, crossover_pt)
                        .unwrap_or_else(|_| parent_a.sop.clone());

                let mut child_sop = crossed;
                if next_population.len() % 2 == 1 {
                    let mut_kind = match (next_population.len() + generation as usize) % 3 {
                        0 => MutationKind::PromptCommand,
                        1 => MutationKind::StepReorder,
                        _ => MutationKind::AddVerificationCheckpoint,
                    };
                    child_sop = mutate_sop(&child_sop, &child_id, mut_kind).unwrap_or(child_sop);
                }

                let (pr, cost, lat) = evaluate_sop_metrics(&child_sop);
                let cand = IslandCandidate::new(
                    format!(
                        "cand-{}-g{}-{}",
                        island.niche.as_str(),
                        generation,
                        next_population.len()
                    ),
                    generation,
                    child_sop,
                    pr,
                    cost,
                    lat,
                    island.niche,
                    &island.id,
                );
                next_population.push(cand);
            }

            island.population = next_population;
            island.sort_by_fitness();

            let best = island.best_candidate().cloned();
            summaries.push(IslandSummary {
                island_id: island.id.clone(),
                niche: island.niche,
                generation,
                population_size: island.population.len(),
                best_fitness: best.as_ref().map(|b| b.fitness).unwrap_or(0.0),
                best_pass_rate: best.as_ref().map(|b| b.pass_rate).unwrap_or(0.0),
                best_token_cost: best.as_ref().map(|b| b.token_cost).unwrap_or(0),
                best_latency_ms: best.as_ref().map(|b| b.latency_ms).unwrap_or(0),
            });
        }

        EvolutionStepReport {
            generation,
            islands: summaries,
        }
    }

    pub fn migrate(&mut self) -> MigrationReport {
        let n = self.islands.len();
        if n <= 1 {
            return MigrationReport {
                generation: self.current_generation,
                migrated_count: 0,
                exchanges: Vec::new(),
            };
        }

        let mut elites_to_send: Vec<Vec<IslandCandidate>> = Vec::with_capacity(n);
        for island in &mut self.islands {
            island.sort_by_fitness();
            let count = self.migration_count.min(island.population.len());
            let elites: Vec<IslandCandidate> = island.population[..count].to_vec();
            elites_to_send.push(elites);
        }

        let mut exchanges = Vec::new();
        let mut migrated_total = 0;

        for (from_idx, candidates) in elites_to_send.iter().enumerate().take(n) {
            let to_idx = (from_idx + 1) % n;
            let target_niche = self.islands[to_idx].niche;
            let target_island_id = self.islands[to_idx].id.clone();
            let from_island_id = self.islands[from_idx].id.clone();

            let target_pop = &mut self.islands[to_idx].population;
            let k = candidates.len().min(target_pop.len());
            for _ in 0..k {
                target_pop.pop();
            }

            for immigrant_cand in candidates.iter().take(k) {
                let mut immigrant = immigrant_cand.clone();
                immigrant.id = format!("migrated-{}-to-{}", immigrant.id, target_island_id);
                immigrant.origin_island = from_island_id.clone();
                immigrant.fitness = target_niche.compute_fitness(
                    immigrant.pass_rate,
                    immigrant.token_cost,
                    immigrant.latency_ms,
                );
                target_pop.push(immigrant.clone());
                migrated_total += 1;
                exchanges.push(MigrationExchange {
                    from_island: from_island_id.clone(),
                    to_island: target_island_id.clone(),
                    candidate_id: immigrant.id,
                    fitness_in_target: immigrant.fitness,
                });
            }
            self.islands[to_idx].sort_by_fitness();
        }

        MigrationReport {
            generation: self.current_generation,
            migrated_count: migrated_total,
            exchanges,
        }
    }

    pub fn run(&mut self, generations: u32) -> IslandRunReport {
        let mut steps = Vec::new();
        let mut migrations = Vec::new();

        for _ in 0..generations {
            let step_report = self.step();
            if self
                .current_generation
                .is_multiple_of(self.migration_interval)
            {
                let mig_report = self.migrate();
                migrations.push(mig_report);
            }
            steps.push(step_report);
        }

        let frontier = self.frontier();
        let island_summaries = self
            .islands
            .iter()
            .map(|island| {
                let best = island.best_candidate().cloned();
                IslandSummary {
                    island_id: island.id.clone(),
                    niche: island.niche,
                    generation: self.current_generation,
                    population_size: island.population.len(),
                    best_fitness: best.as_ref().map(|b| b.fitness).unwrap_or(0.0),
                    best_pass_rate: best.as_ref().map(|b| b.pass_rate).unwrap_or(0.0),
                    best_token_cost: best.as_ref().map(|b| b.token_cost).unwrap_or(0),
                    best_latency_ms: best.as_ref().map(|b| b.latency_ms).unwrap_or(0),
                }
            })
            .collect();

        IslandRunReport {
            total_generations: self.current_generation,
            islands: island_summaries,
            step_reports: steps,
            migration_reports: migrations,
            pareto_frontier_size: frontier.len(),
            pareto_candidates: frontier.candidates().to_vec(),
        }
    }

    pub fn frontier(&self) -> ParetoFrontier {
        let mut frontier = ParetoFrontier::new();
        for island in &self.islands {
            for c in &island.population {
                let p = ParetoCandidate::new(&c.id, c.pass_rate, c.token_cost, c.latency_ms);
                frontier.try_insert(p);
            }
        }
        frontier
    }
}

fn ln_gamma(z: f64) -> f64 {
    let coef = [
        76.18009172947146,
        -86.50532032941677,
        24.01409824083091,
        -1.231739572450155,
        0.1208650973866179e-2,
        -0.5395239384953e-5,
    ];
    let mut y = z;
    let x = z;
    let mut tmp = x + 5.5;
    tmp -= (x + 0.5) * tmp.ln();
    let mut ser = 1.000000000190015;
    for c in &coef {
        y += 1.0;
        ser += c / y;
    }
    -tmp + (2.5066282746310005 * ser / x).ln()
}

fn ln_fact(n: u32) -> f64 {
    if n <= 1 {
        0.0
    } else {
        ln_gamma(f64::from(n + 1))
    }
}

fn normal_cdf(z: f64) -> f64 {
    0.5 * (1.0 + erf_approx(z / std::f64::consts::SQRT_2))
}

fn erf_approx(x: f64) -> f64 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    // Abramowitz and Stegun formula 7.1.26 (max error 1.5e-7)
    let a1 = 0.254829592;
    let a2 = -0.284496736;
    let a3 = 1.421413741;
    let a4 = -1.453152027;
    let a5 = 1.061405429;
    let p = 0.3275911;

    let t = 1.0 / (1.0 + p * x);
    let y = 1.0 - (((((a5 * t + a4) * t) + a3) * t + a2) * t + a1) * t * (-x * x).exp();
    sign * y
}

fn beta_cdf(alpha: u32, beta: u32, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }

    let n = alpha + beta - 1;
    if n > 500 {
        let a = f64::from(alpha);
        let b = f64::from(beta);
        let ab = a + b;
        let mu = a / ab;
        let var = (a * b) / (ab * ab * (ab + 1.0));
        let sigma = var.sqrt();
        if sigma <= 0.0 {
            return if x >= mu { 1.0 } else { 0.0 };
        }
        let z = (x - mu) / sigma;
        return normal_cdf(z).clamp(0.0, 1.0);
    }

    if alpha > n / 2 {
        // Sum beta terms from alpha to n
        let mut sum = 0.0;
        for k in alpha..=n {
            let log_comb = ln_fact(n) - ln_fact(k) - ln_fact(n - k);
            let log_term = log_comb + (f64::from(k) * x.ln()) + (f64::from(n - k) * (1.0 - x).ln());
            sum += log_term.exp();
        }
        sum.clamp(0.0, 1.0)
    } else {
        // Reflection identity: I_x(a, b) = 1 - I_{1-x}(b, a)
        let mut sum = 0.0;
        for k in beta..=n {
            let log_comb = ln_fact(n) - ln_fact(k) - ln_fact(n - k);
            let log_term = log_comb + (f64::from(k) * (1.0 - x).ln()) + (f64::from(n - k) * x.ln());
            sum += log_term.exp();
        }
        (1.0 - sum).clamp(0.0, 1.0)
    }
}

fn beta_quantile(alpha: f64, beta: f64, target_prob: f64) -> f64 {
    let a = (alpha.round() as u32).max(1);
    let b = (beta.round() as u32).max(1);
    let mut low = 0.0;
    let mut high = 1.0;
    for _ in 0..50 {
        let mid = (low + high) * 0.5;
        let cdf = beta_cdf(a, b, mid);
        if cdf < target_prob {
            low = mid;
        } else {
            high = mid;
        }
    }
    (low + high) * 0.5
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::{
        ArtifactId, CanaryResult, EvolutionReleaseChannel, EvolutionRolloutBinding,
        EvolutionRolloutStage, EvolutionScorecard, EvolutionSource, EvolutionStageLimits, MemoryId,
        PrincipalId, RequestDigest, Timestamp,
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
        .with_memory_evidence_ids(vec![MemoryId::new("memory-evidence-1").unwrap()])
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
            records[0].proposal().memory_evidence_ids(),
            &[MemoryId::new("memory-evidence-1").unwrap()]
        );
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

    #[test]
    fn durable_rollout_blocks_activation_until_every_stage_is_complete() {
        fn digest(value: u64) -> RequestDigest {
            RequestDigest::new(format!("sha256:{value:064x}")).unwrap()
        }
        let root = crate::test_support::new_temp_dir("pandora-evolution-rollout").unwrap();
        let path = root.join("evolution.sqlite3");
        let engine = EvolutionEngine::open(&path, EvolutionPolicy::production(1)).unwrap();
        let proposal_id = ProposalId::new("proposal-rollout").unwrap();
        engine
            .submit(
                MutationProposal::new(
                    proposal_id.as_str(),
                    EvolutionSource::Gepa,
                    ArtifactId::new("base-rollout").unwrap(),
                    ArtifactId::new("candidate-rollout").unwrap(),
                    digest(2),
                    "governed rollout",
                    Timestamp::from_unix_seconds(10),
                )
                .unwrap(),
            )
            .unwrap();
        engine
            .record_evaluation(HoldoutEvaluation::new(
                proposal_id.clone(),
                99,
                99,
                true,
                true,
                true,
                Timestamp::from_unix_seconds(11),
            ))
            .unwrap();
        engine
            .approve(
                &proposal_id,
                ParliamentApproval::new(
                    proposal_id.clone(),
                    PrincipalId::new("parliament-1").unwrap(),
                    1,
                    Timestamp::from_unix_seconds(12),
                ),
                ArtifactSignature::new(
                    ArtifactId::new("candidate-rollout").unwrap(),
                    PrincipalId::new("signer-1").unwrap(),
                    "signed-rollout-candidate",
                )
                .unwrap(),
            )
            .unwrap();
        engine.stage(&proposal_id).unwrap();
        engine
            .record_canary(
                CanaryResult::new(
                    proposal_id.clone(),
                    true,
                    0,
                    "legacy canary passed before staged rollout",
                    Timestamp::from_unix_seconds(13),
                )
                .unwrap(),
            )
            .unwrap();
        let limits = [
            EvolutionRolloutStage::Canary,
            EvolutionRolloutStage::Limited,
            EvolutionRolloutStage::Expanded,
            EvolutionRolloutStage::Complete,
        ]
        .into_iter()
        .map(|stage| EvolutionStageLimits::new(stage, 10_000, 600, 0, 90, 500, 95).unwrap())
        .collect();
        engine
            .configure_rollout(
                &proposal_id,
                EvolutionRolloutBinding::new(
                    "0123456789abcdef0123456789abcdef01234567",
                    digest(1),
                    EvolutionReleaseChannel::Beta,
                    digest(2),
                )
                .unwrap(),
                limits,
                digest(3),
                PrincipalId::new("release-manager-1").unwrap(),
                Timestamp::from_unix_seconds(14),
            )
            .unwrap();
        engine
            .record_rollout_scorecard(
                &proposal_id,
                EvolutionScorecard::new(
                    EvolutionRolloutStage::Canary,
                    99,
                    100,
                    99,
                    1_000,
                    60,
                    0,
                    digest(4),
                    digest(5),
                    PrincipalId::new("evaluator-1").unwrap(),
                    Timestamp::from_unix_seconds(15),
                )
                .unwrap(),
                digest(6),
                PrincipalId::new("evaluator-1").unwrap(),
            )
            .unwrap();

        assert_eq!(
            engine.activate(&proposal_id),
            Err(EvolutionError::RolloutIncomplete)
        );
        drop(engine);
        let reopened = EvolutionEngine::open(&path, EvolutionPolicy::production(1)).unwrap();
        let rollout = reopened
            .inspect(&proposal_id)
            .unwrap()
            .rollout()
            .cloned()
            .unwrap();
        assert_eq!(rollout.current_stage(), EvolutionRolloutStage::Canary);
        assert_eq!(
            rollout.status(),
            pandora_types::EvolutionRolloutStatus::AwaitingApproval
        );
        assert_eq!(rollout.transitions().len(), 2);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn pareto_frontier_dominance_and_pruning() {
        let mut frontier = ParetoFrontier::new();

        // Baseline candidate
        let c1 = ParetoCandidate::new("c1", 0.80, 1000, 500);
        assert!(frontier.try_insert(c1.clone()));
        assert_eq!(frontier.len(), 1);

        // Dominated candidate (lower pass rate, higher cost, higher latency)
        let dominated = ParetoCandidate::new("dominated", 0.70, 1200, 600);
        assert!(c1.dominates(&dominated));
        assert!(frontier.is_dominated(&dominated));
        assert!(!frontier.try_insert(dominated));
        assert_eq!(frontier.len(), 1);

        // Trade-off candidate (lower pass rate, but much lower cost and latency) -> non-dominated!
        let trade_off = ParetoCandidate::new("trade_off", 0.78, 400, 200);
        assert!(!c1.dominates(&trade_off));
        assert!(!trade_off.dominates(&c1));
        assert!(frontier.try_insert(trade_off));
        assert_eq!(frontier.len(), 2);

        // Dominant candidate (better pass rate, lower cost, lower latency than c1) -> should prune c1!
        let superior = ParetoCandidate::new("superior", 0.90, 800, 400);
        assert!(superior.dominates(&c1));
        assert!(frontier.try_insert(superior));
        // c1 pruned, trade_off kept, superior added -> len is 2
        assert_eq!(frontier.len(), 2);
        assert!(!frontier.candidates().iter().any(|c| c.id == "c1"));
        assert!(frontier.candidates().iter().any(|c| c.id == "superior"));
        assert!(frontier.candidates().iter().any(|c| c.id == "trade_off"));
    }

    #[test]
    fn bayesian_canary_gate_lower_bound_and_promotion() {
        let gate = BayesianCanaryGate::new(0.95);

        // Low sample count with 100% pass (e.g. 3 passes, 0 failures)
        // High empirical mean (1.0), but lower 95% bound is uncertain (< 0.60)
        let verdict_few = gate.evaluate(3, 0, 0.75);
        assert_eq!(verdict_few.passes, 3);
        assert_eq!(verdict_few.failures, 0);
        assert!(verdict_few.lower_bound_95 < 0.75);
        assert!(!verdict_few.promoted);

        // High sample count with robust performance (e.g. 95 passes, 5 failures = 95% pass rate)
        // Lower bound is high (~0.89), comfortably exceeding baseline 0.80
        let verdict_robust = gate.evaluate(95, 5, 0.80);
        assert!(verdict_robust.lower_bound_95 > 0.80);
        assert!(verdict_robust.promoted);

        // Mediocre performance (e.g. 60 passes, 40 failures = 60% pass rate)
        // Cannot exceed 0.75 baseline
        let verdict_poor = gate.evaluate(60, 40, 0.75);
        assert!(!verdict_poor.promoted);

        // Extreme sample count (e.g. 9,500 passes, 500 failures = 10,000 runs)
        // High confidence, lower bound very tight around 0.946, exceeding 0.90 baseline
        let verdict_large = gate.evaluate(9500, 500, 0.90);
        assert!(verdict_large.lower_bound_95 > 0.94);
        assert!(verdict_large.promoted);

        // NaN baseline rate fails closed
        let verdict_nan = gate.evaluate(9500, 500, f64::NAN);
        assert!(!verdict_nan.promoted);
    }
}
