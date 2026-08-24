use crate::sessions::{MAX_MEMORY_RECALL_RECORDS, SessionError, SessionStore};
use pandora_types::{
    ContextClassification, MemoryApproval, MemoryAuditAction, MemoryAuditEntry,
    MemoryContractError, MemoryId, MemoryKind, MemoryRecord, MemoryScope, MemoryTier, PrincipalId,
    Timestamp,
};
use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::sync::Mutex;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MemoryError {
    NotFound,
    ApprovalRequired,
    AlreadyPromoted,
    InvalidCapacity,
    InvalidRecord,
    SecretContent,
    AlreadyExists,
    CapacityExceeded,
    Revoked,
    ScopeViolation,
    Contract(MemoryContractError),
    StoreUnavailable,
}

impl From<MemoryContractError> for MemoryError {
    fn from(error: MemoryContractError) -> Self {
        if error == MemoryContractError::SecretContent {
            Self::SecretContent
        } else {
            Self::Contract(error)
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct MemoryKey {
    scope: MemoryScope,
    id: MemoryId,
}

struct MemoryStore {
    l0: VecDeque<MemoryRecord>,
    l1: Vec<MemoryRecord>,
    l2: Vec<MemoryRecord>,
    revoked: HashSet<MemoryKey>,
    audit: Vec<MemoryAuditEntry>,
}

pub struct MemoryEngine {
    max_l0_entries: usize,
    store: Mutex<MemoryStore>,
    durable: Option<DurableMemoryBackend>,
}

struct DurableMemoryBackend {
    store: SessionStore,
    principal_id: PrincipalId,
}

impl MemoryEngine {
    pub fn new(max_l0_entries: usize) -> Self {
        Self {
            max_l0_entries,
            store: Mutex::new(MemoryStore {
                l0: VecDeque::new(),
                l1: Vec::new(),
                l2: Vec::new(),
                revoked: HashSet::new(),
                audit: Vec::new(),
            }),
            durable: None,
        }
    }

    pub fn open(
        path: impl AsRef<Path>,
        max_l0_entries: usize,
        principal_id: PrincipalId,
    ) -> Result<Self, MemoryError> {
        if max_l0_entries == 0 {
            return Err(MemoryError::InvalidCapacity);
        }
        let mut engine = Self::new(max_l0_entries);
        engine.durable = Some(DurableMemoryBackend {
            store: SessionStore::open(path).map_err(memory_store_error)?,
            principal_id,
        });
        Ok(engine)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn remember_l0(
        &self,
        scope: MemoryScope,
        id: impl Into<String>,
        summary: impl Into<String>,
        classification: ContextClassification,
        created_at: Timestamp,
        expires_at: Option<Timestamp>,
        provenance: impl Into<String>,
    ) -> Result<MemoryRecord, MemoryError> {
        let record = MemoryRecord::new_l0(
            id,
            scope,
            summary,
            classification,
            created_at,
            expires_at,
            provenance,
        )?;
        let mut store = self
            .store
            .lock()
            .map_err(|_| MemoryError::StoreUnavailable)?;
        prune_l0(&mut store.l0, created_at);
        store.l0.push_back(record.clone());
        while store.l0.len() > self.max_l0_entries {
            store.l0.pop_front();
        }
        record_added(
            &mut store,
            &record,
            MemoryAuditAction::Added,
            created_at,
            None,
        );
        Ok(record)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn distill_l1(
        &self,
        scope: MemoryScope,
        id: impl Into<String>,
        kind: MemoryKind,
        summary: impl Into<String>,
        classification: ContextClassification,
        created_at: Timestamp,
        provenance: impl Into<String>,
    ) -> Result<MemoryRecord, MemoryError> {
        let record = MemoryRecord::new_l1(
            id,
            kind,
            scope,
            summary,
            classification,
            created_at,
            provenance,
        )?;
        if let Some(durable) = &self.durable {
            durable
                .store
                .record_memory(&durable.principal_id, &record)
                .map_err(memory_store_error)?;
            return Ok(record);
        }
        let mut store = self
            .store
            .lock()
            .map_err(|_| MemoryError::StoreUnavailable)?;
        store.l1.push(record.clone());
        record_added(
            &mut store,
            &record,
            MemoryAuditAction::Added,
            created_at,
            None,
        );
        Ok(record)
    }

    pub fn promote_l2(
        &self,
        scope: &MemoryScope,
        id: &MemoryId,
        approval: Option<MemoryApproval>,
        promoted_at: Timestamp,
    ) -> Result<MemoryRecord, MemoryError> {
        let approval = approval.ok_or(MemoryError::ApprovalRequired)?;
        if let Some(durable) = &self.durable {
            return durable
                .store
                .promote_memory(&durable.principal_id, scope, id, approval, promoted_at)
                .map_err(memory_store_error);
        }
        let mut store = self
            .store
            .lock()
            .map_err(|_| MemoryError::StoreUnavailable)?;
        if store
            .l2
            .iter()
            .any(|record| record.scope() == scope && record.id() == id)
        {
            return Err(MemoryError::AlreadyPromoted);
        }
        let candidate = store
            .l1
            .iter()
            .find(|record| {
                record.scope() == scope && record.id() == id && !is_revoked(&store.revoked, record)
            })
            .cloned()
            .ok_or(MemoryError::NotFound)?;
        let record = MemoryRecord::promote_l2(candidate, approval, promoted_at)?;
        let approval_id = record
            .approval()
            .map(|value| value.approval_id().to_owned());
        store.l2.push(record.clone());
        record_added(
            &mut store,
            &record,
            MemoryAuditAction::Promoted,
            promoted_at,
            approval_id,
        );
        Ok(record)
    }

    pub fn recall(
        &self,
        scope: &MemoryScope,
        tier: MemoryTier,
        now: Timestamp,
    ) -> Vec<MemoryRecord> {
        self.try_recall(scope, tier, now).unwrap_or_default()
    }

    pub fn try_recall(
        &self,
        scope: &MemoryScope,
        tier: MemoryTier,
        now: Timestamp,
    ) -> Result<Vec<MemoryRecord>, MemoryError> {
        if tier != MemoryTier::L0
            && let Some(durable) = &self.durable
        {
            return durable
                .store
                .recall_memory(
                    scope.session_id(),
                    &durable.principal_id,
                    scope.tenant_id(),
                    scope.workspace_id(),
                    scope.provider(),
                    tier,
                    MAX_MEMORY_RECALL_RECORDS,
                )
                .map_err(memory_store_error);
        }
        let Ok(mut store) = self.store.lock() else {
            return Err(MemoryError::StoreUnavailable);
        };
        if tier == MemoryTier::L0 {
            prune_l0(&mut store.l0, now);
        }
        let records = match tier {
            MemoryTier::L0 => store.l0.iter().collect::<Vec<_>>(),
            MemoryTier::L1 => store.l1.iter().collect::<Vec<_>>(),
            MemoryTier::L2 => store.l2.iter().collect::<Vec<_>>(),
        };
        Ok(records
            .into_iter()
            .filter(|record| {
                record.scope() == scope
                    && !is_revoked(&store.revoked, record)
                    && !record.is_expired(now)
            })
            .cloned()
            .collect())
    }

    pub fn forget(
        &self,
        scope: &MemoryScope,
        id: &MemoryId,
        at: Timestamp,
    ) -> Result<(), MemoryError> {
        let (durable_revoked, durable_already_revoked) = if let Some(durable) = &self.durable {
            match durable
                .store
                .revoke_memory(&durable.principal_id, scope, id, at)
            {
                Ok(()) => (true, false),
                Err(SessionError::MemoryNotFound) => (false, false),
                Err(SessionError::MemoryRevoked) => (false, true),
                Err(error) => return Err(memory_store_error(error)),
            }
        } else {
            (false, false)
        };
        let mut store = self
            .store
            .lock()
            .map_err(|_| MemoryError::StoreUnavailable)?;
        let l0_removed = store
            .l0
            .iter()
            .any(|record| record.scope() == scope && record.id() == id);
        if l0_removed {
            store.revoked.insert(MemoryKey {
                scope: scope.clone(),
                id: id.clone(),
            });
            store.audit.push(MemoryAuditEntry::new(
                id.clone(),
                MemoryTier::L0,
                MemoryAuditAction::Revoked,
                scope.clone(),
                at,
                None,
            ));
            store
                .l0
                .retain(|record| !(record.scope() == scope && record.id() == id));
        }
        if durable_revoked || durable_already_revoked {
            store.revoked.insert(MemoryKey {
                scope: scope.clone(),
                id: id.clone(),
            });
        }
        if durable_revoked || l0_removed {
            return Ok(());
        }
        if durable_already_revoked {
            return Err(MemoryError::Revoked);
        }
        if self.durable.is_some() {
            return Err(MemoryError::NotFound);
        }
        let tier = find_tier(&store, scope, id).ok_or(MemoryError::NotFound)?;
        store.revoked.insert(MemoryKey {
            scope: scope.clone(),
            id: id.clone(),
        });
        store.audit.push(MemoryAuditEntry::new(
            id.clone(),
            tier,
            MemoryAuditAction::Revoked,
            scope.clone(),
            at,
            None,
        ));
        if tier == MemoryTier::L0 {
            store
                .l0
                .retain(|record| !(record.scope() == scope && record.id() == id));
        }
        Ok(())
    }

    pub fn audit(&self, scope: &MemoryScope) -> Vec<MemoryAuditEntry> {
        self.try_audit(scope).unwrap_or_default()
    }

    pub fn try_audit(&self, scope: &MemoryScope) -> Result<Vec<MemoryAuditEntry>, MemoryError> {
        let store = self
            .store
            .lock()
            .map_err(|_| MemoryError::StoreUnavailable)?;
        let mut audit = store
            .audit
            .iter()
            .filter(|entry| entry.scope() == scope)
            .cloned()
            .collect::<Vec<_>>();
        drop(store);
        if let Some(durable) = &self.durable {
            audit.extend(
                durable
                    .store
                    .memory_audit(&durable.principal_id, scope)
                    .map_err(memory_store_error)?,
            );
            audit.sort_by_key(|entry| entry.at());
        }
        Ok(audit)
    }

    pub fn compact_revoked(
        &self,
        scope: &MemoryScope,
        revoked_before_or_at: Timestamp,
    ) -> Result<usize, MemoryError> {
        let Some(durable) = &self.durable else {
            return Ok(0);
        };
        durable
            .store
            .compact_revoked_memory(&durable.principal_id, scope, revoked_before_or_at)
            .map_err(memory_store_error)
    }
}

fn memory_store_error(error: SessionError) -> MemoryError {
    match error {
        SessionError::MemoryNotFound => MemoryError::NotFound,
        SessionError::MemoryAlreadyExists => MemoryError::AlreadyExists,
        SessionError::MemoryAlreadyPromoted => MemoryError::AlreadyPromoted,
        SessionError::MemoryCapacityExceeded | SessionError::L1EvidenceCapacityExceeded => {
            MemoryError::CapacityExceeded
        }
        SessionError::MemoryRevoked => MemoryError::Revoked,
        SessionError::ScopeViolation => MemoryError::ScopeViolation,
        SessionError::InvalidMemory | SessionError::InvalidMemoryLimit => {
            MemoryError::InvalidRecord
        }
        _ => MemoryError::StoreUnavailable,
    }
}

fn prune_l0(records: &mut VecDeque<MemoryRecord>, now: Timestamp) {
    records.retain(|record| !record.is_expired(now));
}

fn is_revoked(revoked: &HashSet<MemoryKey>, record: &MemoryRecord) -> bool {
    revoked.contains(&MemoryKey {
        scope: record.scope().clone(),
        id: record.id().clone(),
    })
}

fn find_tier(store: &MemoryStore, scope: &MemoryScope, id: &MemoryId) -> Option<MemoryTier> {
    if store
        .l0
        .iter()
        .any(|record| record.scope() == scope && record.id() == id)
    {
        return Some(MemoryTier::L0);
    }
    if store
        .l1
        .iter()
        .any(|record| record.scope() == scope && record.id() == id)
    {
        return Some(MemoryTier::L1);
    }
    if store
        .l2
        .iter()
        .any(|record| record.scope() == scope && record.id() == id)
    {
        return Some(MemoryTier::L2);
    }
    None
}

fn record_added(
    store: &mut MemoryStore,
    record: &MemoryRecord,
    action: MemoryAuditAction,
    at: Timestamp,
    approval_id: Option<String>,
) {
    store.audit.push(MemoryAuditEntry::new(
        record.id().clone(),
        record.tier(),
        action,
        record.scope().clone(),
        at,
        approval_id,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::{PrincipalId, Session, SessionId, TenantId, WorkspaceId};

    fn scope(provider: &str) -> MemoryScope {
        MemoryScope::new(
            TenantId::new("tenant-a").unwrap(),
            WorkspaceId::new("workspace-a").unwrap(),
            SessionId::new("session-a").unwrap(),
            provider,
        )
        .unwrap()
    }

    #[test]
    fn l0_is_expiring_and_bounded() {
        let engine = MemoryEngine::new(2);
        let scope = scope("provider-a");

        engine
            .remember_l0(
                scope.clone(),
                "trace-1",
                "first",
                ContextClassification::Internal,
                Timestamp::from_unix_seconds(1),
                Some(Timestamp::from_unix_seconds(10)),
                "execution:1",
            )
            .unwrap();
        engine
            .remember_l0(
                scope.clone(),
                "trace-2",
                "second",
                ContextClassification::Internal,
                Timestamp::from_unix_seconds(2),
                Some(Timestamp::from_unix_seconds(4)),
                "execution:2",
            )
            .unwrap();
        engine
            .remember_l0(
                scope.clone(),
                "trace-3",
                "third",
                ContextClassification::Internal,
                Timestamp::from_unix_seconds(3),
                Some(Timestamp::from_unix_seconds(5)),
                "execution:3",
            )
            .unwrap();

        let records = engine.recall(&scope, MemoryTier::L0, Timestamp::from_unix_seconds(3));
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].id().as_str(), "trace-2");
        assert_eq!(records[1].id().as_str(), "trace-3");

        assert!(
            engine
                .recall(&scope, MemoryTier::L0, Timestamp::from_unix_seconds(10))
                .is_empty()
        );
    }

    #[test]
    fn secret_content_is_rejected_and_l1_keeps_provenance() {
        let engine = MemoryEngine::new(2);
        let scope = scope("provider-a");

        assert_eq!(
            engine.remember_l0(
                scope.clone(),
                "secret-1",
                "token",
                ContextClassification::Secret,
                Timestamp::from_unix_seconds(1),
                None,
                "execution:1",
            ),
            Err(MemoryError::SecretContent)
        );

        let record = engine
            .distill_l1(
                scope,
                "failure-1",
                MemoryKind::Failure,
                "redacted failure summary",
                ContextClassification::Internal,
                Timestamp::from_unix_seconds(2),
                "execution:2/tool:1",
            )
            .unwrap();
        assert_eq!(record.summary(), "redacted failure summary");
        assert_eq!(record.provenance(), "execution:2/tool:1");
        assert_eq!(record.tier(), MemoryTier::L1);
    }

    #[test]
    fn recall_isolated_by_workspace_session_and_provider() {
        let engine = MemoryEngine::new(2);
        let original = scope("provider-a");
        let other_provider = scope("provider-b");
        let other_workspace = MemoryScope::new(
            TenantId::new("tenant-a").unwrap(),
            WorkspaceId::new("workspace-b").unwrap(),
            SessionId::new("session-a").unwrap(),
            "provider-a",
        )
        .unwrap();

        engine
            .distill_l1(
                original.clone(),
                "decision-1",
                MemoryKind::Decision,
                "use the verified plan",
                ContextClassification::Internal,
                Timestamp::from_unix_seconds(1),
                "execution:1",
            )
            .unwrap();

        assert_eq!(
            engine
                .recall(&original, MemoryTier::L1, Timestamp::from_unix_seconds(2))
                .len(),
            1
        );
        assert!(
            engine
                .recall(
                    &other_provider,
                    MemoryTier::L1,
                    Timestamp::from_unix_seconds(2)
                )
                .is_empty()
        );
        assert!(
            engine
                .recall(
                    &other_workspace,
                    MemoryTier::L1,
                    Timestamp::from_unix_seconds(2)
                )
                .is_empty()
        );
    }

    #[test]
    fn l2_requires_explicit_approval() {
        let engine = MemoryEngine::new(2);
        let scope = scope("provider-a");
        let candidate = engine
            .distill_l1(
                scope.clone(),
                "lesson-1",
                MemoryKind::Lesson,
                "prefer the bounded retry",
                ContextClassification::Internal,
                Timestamp::from_unix_seconds(1),
                "evaluation:1",
            )
            .unwrap();

        assert_eq!(
            engine.promote_l2(
                &scope,
                candidate.id(),
                None,
                Timestamp::from_unix_seconds(2)
            ),
            Err(MemoryError::ApprovalRequired)
        );

        let approval = MemoryApproval::new("approval-1", "operator-1").unwrap();
        let promoted = engine
            .promote_l2(
                &scope,
                candidate.id(),
                Some(approval),
                Timestamp::from_unix_seconds(2),
            )
            .unwrap();
        assert_eq!(promoted.tier(), MemoryTier::L2);
        assert_eq!(
            engine.recall(&scope, MemoryTier::L2, Timestamp::from_unix_seconds(3)),
            vec![promoted]
        );
    }

    #[test]
    fn forget_adds_a_revocation_audit_entry() {
        let engine = MemoryEngine::new(2);
        let scope = scope("provider-a");
        let record = engine
            .distill_l1(
                scope.clone(),
                "decision-1",
                MemoryKind::Decision,
                "do not reuse stale evidence",
                ContextClassification::Internal,
                Timestamp::from_unix_seconds(1),
                "execution:1",
            )
            .unwrap();

        engine
            .forget(&scope, record.id(), Timestamp::from_unix_seconds(2))
            .unwrap();
        assert!(
            engine
                .recall(&scope, MemoryTier::L1, Timestamp::from_unix_seconds(3))
                .is_empty()
        );
        assert!(
            engine
                .audit(&scope)
                .iter()
                .any(|entry| entry.action() == MemoryAuditAction::Revoked)
        );
    }

    #[test]
    fn durable_l1_and_l2_survive_reopen_with_revocation_and_audit() {
        let root = crate::test_support::new_temp_dir("pandora-durable-memory-test").unwrap();
        let path = root.join("sessions.sqlite3");
        let principal = PrincipalId::new("principal-a").unwrap();
        let scope = scope("provider-a");
        let session = Session::new(
            scope.session_id().clone(),
            principal.clone(),
            scope.tenant_id().clone(),
            scope.workspace_id().clone(),
            Timestamp::from_unix_seconds(1),
        );
        crate::sessions::SessionStore::open(&path)
            .unwrap()
            .create(&session)
            .unwrap();

        {
            let engine = MemoryEngine::open(&path, 2, principal.clone()).unwrap();
            let lesson = engine
                .distill_l1(
                    scope.clone(),
                    "lesson-1",
                    MemoryKind::Lesson,
                    "prefer verified evidence",
                    ContextClassification::Internal,
                    Timestamp::from_unix_seconds(2),
                    "evaluation:1",
                )
                .unwrap();
            engine
                .promote_l2(
                    &scope,
                    lesson.id(),
                    Some(MemoryApproval::new("approval-1", "operator-1").unwrap()),
                    Timestamp::from_unix_seconds(3),
                )
                .unwrap();
        }

        let engine = MemoryEngine::open(&path, 2, principal).unwrap();
        assert_eq!(
            engine
                .try_recall(&scope, MemoryTier::L1, Timestamp::from_unix_seconds(4))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            engine
                .try_recall(&scope, MemoryTier::L2, Timestamp::from_unix_seconds(4))
                .unwrap()
                .len(),
            1
        );

        engine
            .forget(
                &scope,
                &MemoryId::new("lesson-1").unwrap(),
                Timestamp::from_unix_seconds(5),
            )
            .unwrap();
        assert!(
            engine
                .try_recall(&scope, MemoryTier::L1, Timestamp::from_unix_seconds(6))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            engine
                .try_audit(&scope)
                .unwrap()
                .iter()
                .filter(|entry| entry.action() == MemoryAuditAction::Revoked)
                .count(),
            2
        );
        assert_eq!(
            engine
                .compact_revoked(&scope, Timestamp::from_unix_seconds(5))
                .unwrap(),
            2
        );
        assert_eq!(engine.try_audit(&scope).unwrap().len(), 4);
    }

    #[test]
    fn durable_engine_never_reloads_l0_trace_data() {
        let root = crate::test_support::new_temp_dir("pandora-ephemeral-memory-test").unwrap();
        let path = root.join("sessions.sqlite3");
        let principal = PrincipalId::new("principal-a").unwrap();
        let scope = scope("provider-a");
        let session = Session::new(
            scope.session_id().clone(),
            principal.clone(),
            scope.tenant_id().clone(),
            scope.workspace_id().clone(),
            Timestamp::from_unix_seconds(1),
        );
        crate::sessions::SessionStore::open(&path)
            .unwrap()
            .create(&session)
            .unwrap();

        {
            let engine = MemoryEngine::open(&path, 2, principal.clone()).unwrap();
            engine
                .remember_l0(
                    scope.clone(),
                    "trace-1",
                    "ephemeral trace",
                    ContextClassification::Internal,
                    Timestamp::from_unix_seconds(2),
                    Some(Timestamp::from_unix_seconds(10)),
                    "execution:1",
                )
                .unwrap();
        }

        let reopened = MemoryEngine::open(path, 2, principal).unwrap();
        assert!(
            reopened
                .try_recall(&scope, MemoryTier::L0, Timestamp::from_unix_seconds(3))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn one_forget_revokes_matching_l0_and_durable_memory() {
        let root = crate::test_support::new_temp_dir("pandora-memory-collision-test").unwrap();
        let path = root.join("sessions.sqlite3");
        let principal = PrincipalId::new("principal-a").unwrap();
        let scope = scope("provider-a");
        let session = Session::new(
            scope.session_id().clone(),
            principal.clone(),
            scope.tenant_id().clone(),
            scope.workspace_id().clone(),
            Timestamp::from_unix_seconds(1),
        );
        crate::sessions::SessionStore::open(&path)
            .unwrap()
            .create(&session)
            .unwrap();

        let engine = MemoryEngine::open(&path, 2, principal.clone()).unwrap();
        engine
            .distill_l1(
                scope.clone(),
                "shared-id",
                MemoryKind::Lesson,
                "retain verified evidence",
                ContextClassification::Internal,
                Timestamp::from_unix_seconds(2),
                "evaluation:1",
            )
            .unwrap();
        engine
            .remember_l0(
                scope.clone(),
                "shared-id",
                "ephemeral trace",
                ContextClassification::Internal,
                Timestamp::from_unix_seconds(3),
                Some(Timestamp::from_unix_seconds(10)),
                "execution:1",
            )
            .unwrap();

        engine
            .forget(
                &scope,
                &MemoryId::new("shared-id").unwrap(),
                Timestamp::from_unix_seconds(4),
            )
            .unwrap();
        assert!(
            engine
                .try_recall(&scope, MemoryTier::L0, Timestamp::from_unix_seconds(5))
                .unwrap()
                .is_empty()
        );
        assert!(
            engine
                .try_recall(&scope, MemoryTier::L1, Timestamp::from_unix_seconds(5))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            engine
                .compact_revoked(&scope, Timestamp::from_unix_seconds(4))
                .unwrap(),
            1
        );
        drop(engine);

        let reopened = MemoryEngine::open(path, 2, principal).unwrap();
        assert!(
            reopened
                .try_recall(&scope, MemoryTier::L1, Timestamp::from_unix_seconds(5))
                .unwrap()
                .is_empty()
        );
    }
}
