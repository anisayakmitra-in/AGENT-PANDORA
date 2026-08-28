use pandora_types::{ArtifactId, ProposalId, ReplacementReceipt, RollbackReceipt, Timestamp};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

const MAX_REPLACEMENT_DEPTH: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactActivation {
    proposal_id: ProposalId,
    base_artifact: ArtifactId,
    candidate_artifact: ArtifactId,
    activated_at: Timestamp,
}

impl ArtifactActivation {
    pub fn proposal_id(&self) -> &ProposalId {
        &self.proposal_id
    }

    pub fn base_artifact(&self) -> &ArtifactId {
        &self.base_artifact
    }

    pub fn candidate_artifact(&self) -> &ArtifactId {
        &self.candidate_artifact
    }

    pub const fn activated_at(&self) -> Timestamp {
        self.activated_at
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactCatalogError {
    StoreUnavailable,
    CorruptRecord,
    BaseAlreadyReplaced,
    ProposalAlreadyActive,
    ProposalNotActive,
    ReceiptMismatch,
    ReplacementHasDependents,
    ReplacementCycle,
}

impl fmt::Display for ArtifactCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StoreUnavailable => formatter.write_str("artifact catalog is unavailable"),
            Self::CorruptRecord => {
                formatter.write_str("artifact catalog contains an invalid record")
            }
            Self::BaseAlreadyReplaced => {
                formatter.write_str("base artifact already has an active replacement")
            }
            Self::ProposalAlreadyActive => {
                formatter.write_str("evolution proposal is already active in the artifact catalog")
            }
            Self::ProposalNotActive => {
                formatter.write_str("evolution proposal is not active in the artifact catalog")
            }
            Self::ReceiptMismatch => formatter
                .write_str("replacement receipt does not match the active artifact binding"),
            Self::ReplacementHasDependents => formatter
                .write_str("replacement cannot roll back while its candidate is still replaced"),
            Self::ReplacementCycle => {
                formatter.write_str("artifact replacement would create a cycle")
            }
        }
    }
}

impl std::error::Error for ArtifactCatalogError {}

pub struct ArtifactCatalog {
    connection: Mutex<Connection>,
}

impl ArtifactCatalog {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ArtifactCatalogError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|_| ArtifactCatalogError::StoreUnavailable)?;
        }
        let connection =
            Connection::open(path).map_err(|_| ArtifactCatalogError::StoreUnavailable)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(|_| ArtifactCatalogError::StoreUnavailable)?;
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 CREATE TABLE IF NOT EXISTS artifact_activations (
                     proposal_id TEXT PRIMARY KEY NOT NULL,
                     base_artifact TEXT NOT NULL UNIQUE,
                     candidate_artifact TEXT NOT NULL,
                     activated_at INTEGER NOT NULL
                 );",
            )
            .map_err(|_| ArtifactCatalogError::StoreUnavailable)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn activate(
        &self,
        receipt: &ReplacementReceipt,
    ) -> Result<ArtifactActivation, ArtifactCatalogError> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| ArtifactCatalogError::StoreUnavailable)?;
        if transaction
            .query_row(
                "SELECT 1 FROM artifact_activations WHERE proposal_id = ?1",
                params![receipt.proposal_id().as_str()],
                |_| Ok(()),
            )
            .optional()
            .map_err(|_| ArtifactCatalogError::StoreUnavailable)?
            .is_some()
        {
            return Err(ArtifactCatalogError::ProposalAlreadyActive);
        }
        if transaction
            .query_row(
                "SELECT 1 FROM artifact_activations WHERE base_artifact = ?1",
                params![receipt.base_artifact().as_str()],
                |_| Ok(()),
            )
            .optional()
            .map_err(|_| ArtifactCatalogError::StoreUnavailable)?
            .is_some()
        {
            return Err(ArtifactCatalogError::BaseAlreadyReplaced);
        }
        ensure_no_cycle(
            &transaction,
            receipt.base_artifact(),
            receipt.candidate_artifact(),
        )?;
        let activated_at = i64::try_from(receipt.activated_at().as_unix_seconds())
            .map_err(|_| ArtifactCatalogError::CorruptRecord)?;
        transaction
            .execute(
                "INSERT INTO artifact_activations
                 (proposal_id, base_artifact, candidate_artifact, activated_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    receipt.proposal_id().as_str(),
                    receipt.base_artifact().as_str(),
                    receipt.candidate_artifact().as_str(),
                    activated_at,
                ],
            )
            .map_err(|_| ArtifactCatalogError::StoreUnavailable)?;
        transaction
            .commit()
            .map_err(|_| ArtifactCatalogError::StoreUnavailable)?;
        Ok(ArtifactActivation {
            proposal_id: receipt.proposal_id().clone(),
            base_artifact: receipt.base_artifact().clone(),
            candidate_artifact: receipt.candidate_artifact().clone(),
            activated_at: receipt.activated_at(),
        })
    }

    pub fn rollback(&self, receipt: &RollbackReceipt) -> Result<(), ArtifactCatalogError> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| ArtifactCatalogError::StoreUnavailable)?;
        let activation = load_by_proposal(&transaction, receipt.proposal_id())?
            .ok_or(ArtifactCatalogError::ProposalNotActive)?;
        if activation.base_artifact() != receipt.restored_artifact() {
            return Err(ArtifactCatalogError::ReceiptMismatch);
        }
        if transaction
            .query_row(
                "SELECT 1 FROM artifact_activations WHERE base_artifact = ?1",
                params![activation.candidate_artifact().as_str()],
                |_| Ok(()),
            )
            .optional()
            .map_err(|_| ArtifactCatalogError::StoreUnavailable)?
            .is_some()
        {
            return Err(ArtifactCatalogError::ReplacementHasDependents);
        }
        let deleted = transaction
            .execute(
                "DELETE FROM artifact_activations WHERE proposal_id = ?1",
                params![receipt.proposal_id().as_str()],
            )
            .map_err(|_| ArtifactCatalogError::StoreUnavailable)?;
        if deleted != 1 {
            return Err(ArtifactCatalogError::CorruptRecord);
        }
        transaction
            .commit()
            .map_err(|_| ArtifactCatalogError::StoreUnavailable)
    }

    pub fn resolve(&self, artifact: &ArtifactId) -> Result<ArtifactId, ArtifactCatalogError> {
        let connection = self.lock()?;
        resolve_with_connection(&connection, artifact)
    }

    pub fn inspect(
        &self,
        proposal_id: &ProposalId,
    ) -> Result<Option<ArtifactActivation>, ArtifactCatalogError> {
        let connection = self.lock()?;
        load_by_proposal(&connection, proposal_id)
    }

    pub fn list(&self, limit: usize) -> Result<Vec<ArtifactActivation>, ArtifactCatalogError> {
        let limit = i64::try_from(limit).map_err(|_| ArtifactCatalogError::StoreUnavailable)?;
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT proposal_id, base_artifact, candidate_artifact, activated_at
                 FROM artifact_activations ORDER BY activated_at, proposal_id LIMIT ?1",
            )
            .map_err(|_| ArtifactCatalogError::StoreUnavailable)?;
        let rows = statement
            .query_map(params![limit], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .map_err(|_| ArtifactCatalogError::StoreUnavailable)?;
        rows.map(|row| {
            let (proposal, base, candidate, activated_at) =
                row.map_err(|_| ArtifactCatalogError::StoreUnavailable)?;
            activation_from_parts(proposal, base, candidate, activated_at)
        })
        .collect()
    }

    pub fn references(
        &self,
        artifact: &ArtifactId,
    ) -> Result<Vec<ArtifactActivation>, ArtifactCatalogError> {
        Ok(self
            .list(i64::MAX as usize)?
            .into_iter()
            .filter(|binding| {
                binding.base_artifact() == artifact || binding.candidate_artifact() == artifact
            })
            .collect())
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, ArtifactCatalogError> {
        self.connection
            .lock()
            .map_err(|_| ArtifactCatalogError::StoreUnavailable)
    }
}

fn ensure_no_cycle(
    connection: &Connection,
    base: &ArtifactId,
    candidate: &ArtifactId,
) -> Result<(), ArtifactCatalogError> {
    let resolved = resolve_with_connection(connection, candidate)?;
    if &resolved == base {
        return Err(ArtifactCatalogError::ReplacementCycle);
    }
    Ok(())
}

fn resolve_with_connection(
    connection: &Connection,
    artifact: &ArtifactId,
) -> Result<ArtifactId, ArtifactCatalogError> {
    let mut current = artifact.clone();
    let mut visited = BTreeSet::new();
    for _ in 0..MAX_REPLACEMENT_DEPTH {
        if !visited.insert(current.clone()) {
            return Err(ArtifactCatalogError::ReplacementCycle);
        }
        let next = connection
            .query_row(
                "SELECT candidate_artifact FROM artifact_activations WHERE base_artifact = ?1",
                params![current.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|_| ArtifactCatalogError::StoreUnavailable)?;
        let Some(next) = next else {
            return Ok(current);
        };
        current = ArtifactId::new(next).map_err(|_| ArtifactCatalogError::CorruptRecord)?;
    }
    Err(ArtifactCatalogError::ReplacementCycle)
}

fn load_by_proposal(
    connection: &Connection,
    proposal_id: &ProposalId,
) -> Result<Option<ArtifactActivation>, ArtifactCatalogError> {
    connection
        .query_row(
            "SELECT proposal_id, base_artifact, candidate_artifact, activated_at
             FROM artifact_activations WHERE proposal_id = ?1",
            params![proposal_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|_| ArtifactCatalogError::StoreUnavailable)?
        .map(|(proposal, base, candidate, activated_at)| {
            activation_from_parts(proposal, base, candidate, activated_at)
        })
        .transpose()
}

fn activation_from_parts(
    proposal: String,
    base: String,
    candidate: String,
    activated_at: i64,
) -> Result<ArtifactActivation, ArtifactCatalogError> {
    Ok(ArtifactActivation {
        proposal_id: ProposalId::new(proposal).map_err(|_| ArtifactCatalogError::CorruptRecord)?,
        base_artifact: ArtifactId::new(base).map_err(|_| ArtifactCatalogError::CorruptRecord)?,
        candidate_artifact: ArtifactId::new(candidate)
            .map_err(|_| ArtifactCatalogError::CorruptRecord)?,
        activated_at: u64::try_from(activated_at)
            .map(Timestamp::from_unix_seconds)
            .map_err(|_| ArtifactCatalogError::CorruptRecord)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replacement(proposal: &str, base: &str, candidate: &str, at: u64) -> ReplacementReceipt {
        ReplacementReceipt::new(
            ProposalId::new(proposal).unwrap(),
            ArtifactId::new(base).unwrap(),
            ArtifactId::new(candidate).unwrap(),
            Timestamp::from_unix_seconds(at),
        )
    }

    #[test]
    fn durable_catalog_resolves_version_chains_and_rolls_back_the_tip() {
        let root = crate::test_support::new_temp_dir("pandora-artifact-catalog").unwrap();
        let path = root.join("catalog.sqlite3");
        let catalog = ArtifactCatalog::open(&path).unwrap();
        catalog
            .activate(&replacement("proposal-a", "artifact-a", "artifact-b", 10))
            .unwrap();
        catalog
            .activate(&replacement("proposal-b", "artifact-b", "artifact-c", 11))
            .unwrap();
        assert_eq!(
            catalog
                .resolve(&ArtifactId::new("artifact-a").unwrap())
                .unwrap()
                .as_str(),
            "artifact-c"
        );
        assert_eq!(
            catalog.rollback(
                &RollbackReceipt::new(
                    ProposalId::new("proposal-a").unwrap(),
                    ArtifactId::new("artifact-a").unwrap(),
                    Timestamp::from_unix_seconds(12),
                    "invalid out-of-order rollback",
                )
                .unwrap(),
            ),
            Err(ArtifactCatalogError::ReplacementHasDependents)
        );

        catalog
            .rollback(
                &RollbackReceipt::new(
                    ProposalId::new("proposal-b").unwrap(),
                    ArtifactId::new("artifact-b").unwrap(),
                    Timestamp::from_unix_seconds(12),
                    "canary regression",
                )
                .unwrap(),
            )
            .unwrap();
        assert_eq!(
            catalog
                .resolve(&ArtifactId::new("artifact-a").unwrap())
                .unwrap()
                .as_str(),
            "artifact-b"
        );
        drop(catalog);
        let reopened = ArtifactCatalog::open(&path).unwrap();
        assert_eq!(reopened.list(64).unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn catalog_rejects_duplicate_bases_and_cycles() {
        let root = crate::test_support::new_temp_dir("pandora-artifact-catalog-cycle").unwrap();
        let catalog = ArtifactCatalog::open(root.join("catalog.sqlite3")).unwrap();
        catalog
            .activate(&replacement("proposal-a", "artifact-a", "artifact-b", 10))
            .unwrap();
        assert_eq!(
            catalog.activate(&replacement("proposal-b", "artifact-a", "artifact-c", 11)),
            Err(ArtifactCatalogError::BaseAlreadyReplaced)
        );
        assert_eq!(
            catalog.activate(&replacement("proposal-c", "artifact-b", "artifact-a", 12)),
            Err(ArtifactCatalogError::ReplacementCycle)
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
