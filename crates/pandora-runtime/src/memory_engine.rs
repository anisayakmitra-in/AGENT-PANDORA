use pandora_types::{
    ContextClassification, MemoryApproval, MemoryAuditAction, MemoryAuditEntry,
    MemoryContractError, MemoryId, MemoryKind, MemoryRecord, MemoryScope, MemoryTier, Timestamp,
};
use std::collections::{HashSet, VecDeque};
use std::sync::Mutex;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MemoryError {
    NotFound,
    ApprovalRequired,
    AlreadyPromoted,
    InvalidCapacity,
    SecretContent,
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
        }
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
        let Ok(mut store) = self.store.lock() else {
            return Vec::new();
        };
        if tier == MemoryTier::L0 {
            prune_l0(&mut store.l0, now);
        }
        let records = match tier {
            MemoryTier::L0 => store.l0.iter().collect::<Vec<_>>(),
            MemoryTier::L1 => store.l1.iter().collect::<Vec<_>>(),
            MemoryTier::L2 => store.l2.iter().collect::<Vec<_>>(),
        };
        records
            .into_iter()
            .filter(|record| {
                record.scope() == scope
                    && !is_revoked(&store.revoked, record)
                    && !record.is_expired(now)
            })
            .cloned()
            .collect()
    }

    pub fn forget(
        &self,
        scope: &MemoryScope,
        id: &MemoryId,
        at: Timestamp,
    ) -> Result<(), MemoryError> {
        let mut store = self
            .store
            .lock()
            .map_err(|_| MemoryError::StoreUnavailable)?;
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
        let Ok(store) = self.store.lock() else {
            return Vec::new();
        };
        store
            .audit
            .iter()
            .filter(|entry| entry.scope() == scope)
            .cloned()
            .collect()
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
    use pandora_types::{SessionId, TenantId, WorkspaceId};

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
}
