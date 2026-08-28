use pandora_types::{
    ArtifactId, EvolutionSource, MutationProposal, ProposalId, RequestDigest, ResearchArtifactKind,
    Timestamp, hash_artifact,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::fmt;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;
use wasmi::{Engine, Module};

pub const MAX_RESEARCH_ARTIFACT_BYTES: usize = 16 * 1024 * 1024;
const MAX_TEXT_ARTIFACT_BYTES: usize = 64 * 1024;
const MAX_WORKFLOW_ARTIFACT_BYTES: usize = 256 * 1024;
const MAX_TARGET_ID_BYTES: usize = 256;
const MAX_PROVIDER_ID_BYTES: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResearchArtifactRecord {
    artifact_id: ArtifactId,
    kind: ResearchArtifactKind,
    target_id: String,
    admitted_at: Timestamp,
}

impl ResearchArtifactRecord {
    pub fn artifact_id(&self) -> &ArtifactId {
        &self.artifact_id
    }

    pub const fn kind(&self) -> ResearchArtifactKind {
        self.kind
    }

    pub fn target_id(&self) -> &str {
        &self.target_id
    }

    pub const fn admitted_at(&self) -> Timestamp {
        self.admitted_at
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResearchCandidateRecord {
    proposal_id: ProposalId,
    kind: ResearchArtifactKind,
    target_id: String,
    base_artifact: ArtifactId,
    candidate_artifact: ArtifactId,
    evidence_digest: RequestDigest,
    provider_id: String,
    generated_at: Timestamp,
}

impl ResearchCandidateRecord {
    pub fn proposal_id(&self) -> &ProposalId {
        &self.proposal_id
    }

    pub const fn kind(&self) -> ResearchArtifactKind {
        self.kind
    }

    pub fn target_id(&self) -> &str {
        &self.target_id
    }

    pub fn base_artifact(&self) -> &ArtifactId {
        &self.base_artifact
    }

    pub fn candidate_artifact(&self) -> &ArtifactId {
        &self.candidate_artifact
    }

    pub fn evidence_digest(&self) -> &RequestDigest {
        &self.evidence_digest
    }

    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    pub const fn generated_at(&self) -> Timestamp {
        self.generated_at
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResearchArtifactError {
    StoreUnavailable,
    CorruptRecord,
    ArtifactTooLarge,
    InvalidArtifact,
    InvalidTarget,
    InvalidProvider,
    DuplicateProposal,
    ProposalMismatch,
    ProposalNotFound,
}

impl fmt::Display for ResearchArtifactError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StoreUnavailable => formatter.write_str("research artifact store is unavailable"),
            Self::CorruptRecord => {
                formatter.write_str("research artifact store contains an invalid record")
            }
            Self::ArtifactTooLarge => {
                formatter.write_str("research artifact exceeds the configured limit")
            }
            Self::InvalidArtifact => {
                formatter.write_str("research artifact does not match its declared kind")
            }
            Self::InvalidTarget => formatter.write_str("research artifact target is invalid"),
            Self::InvalidProvider => {
                formatter.write_str("research proposal provider identity is invalid")
            }
            Self::DuplicateProposal => {
                formatter.write_str("research proposal already has staged artifacts")
            }
            Self::ProposalMismatch => {
                formatter.write_str("research artifacts do not match the evolution proposal")
            }
            Self::ProposalNotFound => {
                formatter.write_str("research proposal artifacts were not found")
            }
        }
    }
}

impl std::error::Error for ResearchArtifactError {}

/// Durable, non-executable candidate material for research evolution.
///
/// This store intentionally has no execution or activation authority. It only
/// verifies exact bytes, target class, and provenance before a candidate enters
/// the separate EvolutionEngine lifecycle.
pub struct ResearchArtifactStore {
    connection: Mutex<Connection>,
}

impl ResearchArtifactStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ResearchArtifactError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|_| ResearchArtifactError::StoreUnavailable)?;
        }
        let connection =
            Connection::open(path).map_err(|_| ResearchArtifactError::StoreUnavailable)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(|_| ResearchArtifactError::StoreUnavailable)?;
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 CREATE TABLE IF NOT EXISTS research_artifacts (
                     artifact_id TEXT NOT NULL,
                     kind TEXT NOT NULL,
                     target_id TEXT NOT NULL,
                     artifact BLOB NOT NULL,
                     admitted_at INTEGER NOT NULL,
                     PRIMARY KEY (artifact_id, kind, target_id)
                 );
                 CREATE TABLE IF NOT EXISTS research_candidates (
                     proposal_id TEXT PRIMARY KEY NOT NULL,
                     kind TEXT NOT NULL,
                     target_id TEXT NOT NULL,
                     base_artifact TEXT NOT NULL,
                     candidate_artifact TEXT NOT NULL,
                     evidence_digest TEXT NOT NULL,
                     provider_id TEXT NOT NULL,
                     generated_at INTEGER NOT NULL
                 );",
            )
            .map_err(|_| ResearchArtifactError::StoreUnavailable)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn stage_generated(
        &self,
        proposal: &MutationProposal,
        kind: ResearchArtifactKind,
        target_id: &str,
        base: &[u8],
        candidate: &[u8],
        provider_id: &str,
        generated_at: Timestamp,
    ) -> Result<ResearchCandidateRecord, ResearchArtifactError> {
        if proposal.source() != EvolutionSource::Gepa
            || hash_artifact(base) != proposal.base_artifact().as_str()
            || hash_artifact(candidate) != proposal.candidate_artifact().as_str()
            || proposal.base_artifact() == proposal.candidate_artifact()
        {
            return Err(ResearchArtifactError::ProposalMismatch);
        }
        validate_target(target_id)?;
        validate_provider(provider_id)?;
        validate_artifact(kind, base)?;
        validate_artifact(kind, candidate)?;
        let generated_at = i64::try_from(generated_at.as_unix_seconds())
            .map_err(|_| ResearchArtifactError::CorruptRecord)?;
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| ResearchArtifactError::StoreUnavailable)?;
        if transaction
            .query_row(
                "SELECT 1 FROM research_candidates WHERE proposal_id = ?1",
                params![proposal.proposal_id().as_str()],
                |_| Ok(()),
            )
            .optional()
            .map_err(|_| ResearchArtifactError::StoreUnavailable)?
            .is_some()
        {
            return Err(ResearchArtifactError::DuplicateProposal);
        }
        insert_artifact(
            &transaction,
            proposal.base_artifact(),
            kind,
            target_id,
            base,
            generated_at,
        )?;
        insert_artifact(
            &transaction,
            proposal.candidate_artifact(),
            kind,
            target_id,
            candidate,
            generated_at,
        )?;
        transaction
            .execute(
                "INSERT INTO research_candidates
                 (proposal_id, kind, target_id, base_artifact, candidate_artifact, evidence_digest, provider_id, generated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    proposal.proposal_id().as_str(),
                    kind.as_str(),
                    target_id,
                    proposal.base_artifact().as_str(),
                    proposal.candidate_artifact().as_str(),
                    proposal.evidence_digest().as_str(),
                    provider_id,
                    generated_at,
                ],
            )
            .map_err(|_| ResearchArtifactError::StoreUnavailable)?;
        transaction
            .commit()
            .map_err(|_| ResearchArtifactError::StoreUnavailable)?;
        Ok(ResearchCandidateRecord {
            proposal_id: proposal.proposal_id().clone(),
            kind,
            target_id: target_id.to_owned(),
            base_artifact: proposal.base_artifact().clone(),
            candidate_artifact: proposal.candidate_artifact().clone(),
            evidence_digest: proposal.evidence_digest().clone(),
            provider_id: provider_id.to_owned(),
            generated_at: Timestamp::from_unix_seconds(
                u64::try_from(generated_at).map_err(|_| ResearchArtifactError::CorruptRecord)?,
            ),
        })
    }

    pub fn inspect(
        &self,
        proposal_id: &ProposalId,
    ) -> Result<Option<ResearchCandidateRecord>, ResearchArtifactError> {
        let connection = self.lock()?;
        load_candidate(&connection, proposal_id)
    }

    pub fn validate_proposal(
        &self,
        proposal: &MutationProposal,
    ) -> Result<ResearchCandidateRecord, ResearchArtifactError> {
        let candidate = self
            .inspect(proposal.proposal_id())?
            .ok_or(ResearchArtifactError::ProposalNotFound)?;
        if candidate.base_artifact() != proposal.base_artifact()
            || candidate.candidate_artifact() != proposal.candidate_artifact()
            || candidate.evidence_digest() != proposal.evidence_digest()
        {
            return Err(ResearchArtifactError::ProposalMismatch);
        }
        let connection = self.lock()?;
        for artifact in [candidate.base_artifact(), candidate.candidate_artifact()] {
            let found = connection
                .query_row(
                    "SELECT artifact FROM research_artifacts WHERE artifact_id = ?1 AND kind = ?2 AND target_id = ?3",
                    params![artifact.as_str(), candidate.kind().as_str(), candidate.target_id()],
                    |row| row.get::<_, Vec<u8>>(0),
                )
                .optional()
                .map_err(|_| ResearchArtifactError::StoreUnavailable)?
                .ok_or(ResearchArtifactError::ProposalNotFound)?;
            if hash_artifact(&found) != artifact.as_str() {
                return Err(ResearchArtifactError::CorruptRecord);
            }
            validate_artifact(candidate.kind(), &found)?;
        }
        Ok(candidate)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, ResearchArtifactError> {
        self.connection
            .lock()
            .map_err(|_| ResearchArtifactError::StoreUnavailable)
    }
}

fn insert_artifact(
    transaction: &rusqlite::Transaction<'_>,
    artifact_id: &ArtifactId,
    kind: ResearchArtifactKind,
    target_id: &str,
    artifact: &[u8],
    admitted_at: i64,
) -> Result<(), ResearchArtifactError> {
    transaction
        .execute(
            "INSERT INTO research_artifacts (artifact_id, kind, target_id, artifact, admitted_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(artifact_id, kind, target_id) DO NOTHING",
            params![
                artifact_id.as_str(),
                kind.as_str(),
                target_id,
                artifact,
                admitted_at
            ],
        )
        .map_err(|_| ResearchArtifactError::StoreUnavailable)?;
    let stored = transaction
        .query_row(
            "SELECT artifact FROM research_artifacts WHERE artifact_id = ?1 AND kind = ?2 AND target_id = ?3",
            params![artifact_id.as_str(), kind.as_str(), target_id],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .map_err(|_| ResearchArtifactError::StoreUnavailable)?;
    if stored != artifact || hash_artifact(&stored) != artifact_id.as_str() {
        return Err(ResearchArtifactError::CorruptRecord);
    }
    Ok(())
}

fn load_candidate(
    connection: &Connection,
    proposal_id: &ProposalId,
) -> Result<Option<ResearchCandidateRecord>, ResearchArtifactError> {
    connection
        .query_row(
            "SELECT proposal_id, kind, target_id, base_artifact, candidate_artifact, evidence_digest, provider_id, generated_at
             FROM research_candidates WHERE proposal_id = ?1",
            params![proposal_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            },
        )
        .optional()
        .map_err(|_| ResearchArtifactError::StoreUnavailable)?
        .map(candidate_from_parts)
        .transpose()
}

#[allow(clippy::type_complexity)]
fn candidate_from_parts(
    parts: (String, String, String, String, String, String, String, i64),
) -> Result<ResearchCandidateRecord, ResearchArtifactError> {
    let (
        proposal_id,
        kind,
        target_id,
        base_artifact,
        candidate_artifact,
        evidence_digest,
        provider_id,
        generated_at,
    ) = parts;
    Ok(ResearchCandidateRecord {
        proposal_id: ProposalId::new(proposal_id)
            .map_err(|_| ResearchArtifactError::CorruptRecord)?,
        kind: ResearchArtifactKind::parse(&kind).ok_or(ResearchArtifactError::CorruptRecord)?,
        target_id: validate_target(&target_id).map(|_| target_id.clone())?,
        base_artifact: ArtifactId::new(base_artifact)
            .map_err(|_| ResearchArtifactError::CorruptRecord)?,
        candidate_artifact: ArtifactId::new(candidate_artifact)
            .map_err(|_| ResearchArtifactError::CorruptRecord)?,
        evidence_digest: RequestDigest::new(evidence_digest)
            .map_err(|_| ResearchArtifactError::CorruptRecord)?,
        provider_id: validate_provider(&provider_id).map(|_| provider_id.clone())?,
        generated_at: u64::try_from(generated_at)
            .map(Timestamp::from_unix_seconds)
            .map_err(|_| ResearchArtifactError::CorruptRecord)?,
    })
}

fn validate_target(target_id: &str) -> Result<(), ResearchArtifactError> {
    if target_id.is_empty()
        || target_id.len() > MAX_TARGET_ID_BYTES
        || target_id.chars().any(char::is_control)
    {
        return Err(ResearchArtifactError::InvalidTarget);
    }
    Ok(())
}

fn validate_provider(provider_id: &str) -> Result<(), ResearchArtifactError> {
    if provider_id.is_empty()
        || provider_id.len() > MAX_PROVIDER_ID_BYTES
        || provider_id.chars().any(char::is_control)
    {
        return Err(ResearchArtifactError::InvalidProvider);
    }
    Ok(())
}

fn validate_artifact(
    kind: ResearchArtifactKind,
    artifact: &[u8],
) -> Result<(), ResearchArtifactError> {
    let maximum = match kind {
        ResearchArtifactKind::Prompt | ResearchArtifactKind::Skill => MAX_TEXT_ARTIFACT_BYTES,
        ResearchArtifactKind::Workflow => MAX_WORKFLOW_ARTIFACT_BYTES,
        ResearchArtifactKind::WasmGene => MAX_RESEARCH_ARTIFACT_BYTES,
    };
    if artifact.is_empty() {
        return Err(ResearchArtifactError::InvalidArtifact);
    }
    if artifact.len() > maximum {
        return Err(ResearchArtifactError::ArtifactTooLarge);
    }
    match kind {
        ResearchArtifactKind::Prompt | ResearchArtifactKind::Skill => {
            let text = std::str::from_utf8(artifact)
                .map_err(|_| ResearchArtifactError::InvalidArtifact)?;
            if text.chars().any(disallowed_text_control) {
                return Err(ResearchArtifactError::InvalidArtifact);
            }
        }
        ResearchArtifactKind::Workflow => {
            let value: serde_json::Value = serde_json::from_slice(artifact)
                .map_err(|_| ResearchArtifactError::InvalidArtifact)?;
            if !value.is_object() {
                return Err(ResearchArtifactError::InvalidArtifact);
            }
        }
        ResearchArtifactKind::WasmGene => {
            if !artifact.starts_with(b"\0asm") || Module::new(&Engine::default(), artifact).is_err()
            {
                return Err(ResearchArtifactError::InvalidArtifact);
            }
        }
    }
    Ok(())
}

fn disallowed_text_control(character: char) -> bool {
    character.is_control() && !matches!(character, '\n' | '\r' | '\t')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proposal(base: &[u8], candidate: &[u8]) -> MutationProposal {
        MutationProposal::new(
            "research-proposal",
            EvolutionSource::Gepa,
            ArtifactId::new(hash_artifact(base)).unwrap(),
            ArtifactId::new(hash_artifact(candidate)).unwrap(),
            RequestDigest::new("evidence-research").unwrap(),
            "reduce verified workflow failures",
            Timestamp::from_unix_seconds(10),
        )
        .unwrap()
    }

    #[test]
    fn stages_exact_non_executable_research_artifacts() {
        let root = crate::test_support::new_temp_dir("pandora-research-artifacts").unwrap();
        let store = ResearchArtifactStore::open(root.join("research.sqlite3")).unwrap();
        let base = br#"{"steps":["verify"]}"#;
        let candidate = br#"{"steps":["verify","test"]}"#;
        let proposal = proposal(base, candidate);

        let record = store
            .stage_generated(
                &proposal,
                ResearchArtifactKind::Workflow,
                "coding.verification",
                base,
                candidate,
                "research-provider",
                Timestamp::from_unix_seconds(11),
            )
            .unwrap();

        assert_eq!(record.kind(), ResearchArtifactKind::Workflow);
        assert_eq!(record.target_id(), "coding.verification");
        assert_eq!(store.validate_proposal(&proposal).unwrap(), record);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_kind_mismatch_and_proposal_substitution() {
        let root = crate::test_support::new_temp_dir("pandora-research-artifact-reject").unwrap();
        let store = ResearchArtifactStore::open(root.join("research.sqlite3")).unwrap();
        let base = b"base prompt";
        let candidate = b"candidate prompt";
        let proposal = proposal(base, candidate);

        assert_eq!(
            store.stage_generated(
                &proposal,
                ResearchArtifactKind::WasmGene,
                "owner/gene",
                base,
                candidate,
                "research-provider",
                Timestamp::from_unix_seconds(11),
            ),
            Err(ResearchArtifactError::InvalidArtifact)
        );
        assert_eq!(
            store.stage_generated(
                &proposal,
                ResearchArtifactKind::Prompt,
                "planner",
                b"different base",
                candidate,
                "research-provider",
                Timestamp::from_unix_seconds(11),
            ),
            Err(ResearchArtifactError::ProposalMismatch)
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn accepts_each_research_candidate_class_without_execution() {
        assert!(
            validate_artifact(ResearchArtifactKind::Prompt, b"follow the holdout plan\n").is_ok()
        );
        assert!(
            validate_artifact(
                ResearchArtifactKind::Skill,
                b"# Verification\nRun holdouts.\n"
            )
            .is_ok()
        );
        assert!(
            validate_artifact(ResearchArtifactKind::Workflow, br#"{"steps":["holdout"]}"#).is_ok()
        );
        assert!(validate_artifact(ResearchArtifactKind::WasmGene, b"\0asm\x01\0\0\0",).is_ok());
        assert_eq!(
            validate_artifact(ResearchArtifactKind::WasmGene, b"\0asm"),
            Err(ResearchArtifactError::InvalidArtifact)
        );
    }
}
