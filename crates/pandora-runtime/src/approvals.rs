use pandora_types::{ExecutionId, GeneId, PrincipalId, RequestDigest, SessionId, Timestamp};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::fmt;
use std::fs;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

const MAX_SUMMARY_BYTES: usize = 4_096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Denied,
    Consumed,
    Expired,
}

impl ApprovalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Denied => "denied",
            Self::Consumed => "consumed",
            Self::Expired => "expired",
        }
    }

    fn parse(value: &str) -> Result<Self, ApprovalError> {
        match value {
            "pending" => Ok(Self::Pending),
            "approved" => Ok(Self::Approved),
            "denied" => Ok(Self::Denied),
            "consumed" => Ok(Self::Consumed),
            "expired" => Ok(Self::Expired),
            _ => Err(ApprovalError::CorruptRecord),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApprovalError {
    Database,
    Io,
    CorruptRecord,
    InvalidSummary,
    LockPoisoned,
    AlreadyExists,
    NotFound,
    ScopeMismatch,
    DigestMismatch,
    Expired,
    Terminal,
}

impl fmt::Display for ApprovalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database => formatter.write_str("approval database operation failed"),
            Self::Io => formatter.write_str("approval database directory operation failed"),
            Self::CorruptRecord => formatter.write_str("approval record is corrupt"),
            Self::InvalidSummary => formatter.write_str("approval request summary is invalid"),
            Self::LockPoisoned => formatter.write_str("approval database lock is unavailable"),
            Self::AlreadyExists => formatter.write_str("approval already exists"),
            Self::NotFound => formatter.write_str("approval was not found"),
            Self::ScopeMismatch => formatter.write_str("approval scope does not match"),
            Self::DigestMismatch => formatter.write_str("approval request digest does not match"),
            Self::Expired => formatter.write_str("approval has expired"),
            Self::Terminal => formatter.write_str("approval is already resolved or consumed"),
        }
    }
}

impl std::error::Error for ApprovalError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalRequest {
    id: String,
    session_id: SessionId,
    execution_id: ExecutionId,
    principal_id: PrincipalId,
    gene_id: GeneId,
    request_digest: RequestDigest,
    request_summary: String,
    policy_version: u32,
    expires_at: Timestamp,
}

impl ApprovalRequest {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        session_id: SessionId,
        execution_id: ExecutionId,
        principal_id: PrincipalId,
        gene_id: GeneId,
        request_digest: RequestDigest,
        request_summary: impl Into<String>,
        policy_version: u32,
        expires_at: Timestamp,
    ) -> Result<Self, ApprovalError> {
        let id = id.into();
        let request_summary = redact_summary(&request_summary.into());
        if id.trim().is_empty()
            || request_summary.trim().is_empty()
            || request_summary.len() > MAX_SUMMARY_BYTES
        {
            return Err(ApprovalError::InvalidSummary);
        }
        Ok(Self {
            id,
            session_id,
            execution_id,
            principal_id,
            gene_id,
            request_digest,
            request_summary,
            policy_version,
            expires_at,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingApproval {
    id: String,
    session_id: SessionId,
    execution_id: ExecutionId,
    principal_id: PrincipalId,
    gene_id: GeneId,
    request_digest: RequestDigest,
    request_summary: String,
    policy_version: u32,
    expires_at: Timestamp,
    status: ApprovalStatus,
    approver_id: Option<PrincipalId>,
    created_at: Timestamp,
}

impl PendingApproval {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub fn execution_id(&self) -> &ExecutionId {
        &self.execution_id
    }

    pub fn principal_id(&self) -> &PrincipalId {
        &self.principal_id
    }

    pub fn gene_id(&self) -> &GeneId {
        &self.gene_id
    }

    pub fn request_digest(&self) -> &RequestDigest {
        &self.request_digest
    }

    pub fn request_summary(&self) -> &str {
        &self.request_summary
    }

    pub fn policy_version(&self) -> u32 {
        self.policy_version
    }

    pub fn expires_at(&self) -> Timestamp {
        self.expires_at
    }

    pub fn approver_id(&self) -> Option<&PrincipalId> {
        self.approver_id.as_ref()
    }

    pub fn created_at(&self) -> Timestamp {
        self.created_at
    }

    pub fn status_at(&self, now: Timestamp) -> ApprovalStatus {
        if matches!(
            self.status,
            ApprovalStatus::Pending | ApprovalStatus::Approved
        ) && now.as_unix_seconds() >= self.expires_at.as_unix_seconds()
        {
            ApprovalStatus::Expired
        } else {
            self.status.clone()
        }
    }
}

pub struct ApprovalStore {
    connection: Mutex<Connection>,
}

impl ApprovalStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ApprovalError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|_| ApprovalError::Io)?;
        }
        let connection = Connection::open(path).map_err(|_| ApprovalError::Database)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(|_| ApprovalError::Database)?;
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA foreign_keys = ON;
                 CREATE TABLE IF NOT EXISTS approvals (
                     id TEXT PRIMARY KEY NOT NULL,
                     session_id TEXT NOT NULL,
                     execution_id TEXT NOT NULL,
                     principal_id TEXT NOT NULL,
                     gene_id TEXT NOT NULL,
                     request_digest TEXT NOT NULL,
                     request_summary TEXT NOT NULL,
                     policy_version INTEGER NOT NULL CHECK (policy_version >= 0),
                     expires_at INTEGER NOT NULL CHECK (expires_at >= 0),
                     status TEXT NOT NULL,
                     approver_id TEXT,
                     resolved_at INTEGER,
                     created_at INTEGER NOT NULL CHECK (created_at >= 0)
                 );
                 CREATE INDEX IF NOT EXISTS approvals_principal_idx
                     ON approvals(principal_id, created_at, id);",
            )
            .map_err(|_| ApprovalError::Database)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn create(&self, request: ApprovalRequest) -> Result<PendingApproval, ApprovalError> {
        let created_at = Timestamp::from_unix_seconds(current_seconds());
        let connection = self.lock()?;
        let result = connection.execute(
            "INSERT INTO approvals (
                 id, session_id, execution_id, principal_id, gene_id,
                 request_digest, request_summary, policy_version, expires_at,
                 status, approver_id, resolved_at, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'pending', NULL, NULL, ?10)",
            params![
                request.id,
                request.session_id.as_str(),
                request.execution_id.as_str(),
                request.principal_id.as_str(),
                request.gene_id.as_str(),
                request.request_digest.as_str(),
                request.request_summary,
                i64::from(request.policy_version),
                to_i64(request.expires_at)?,
                to_i64(created_at)?,
            ],
        );
        match result {
            Ok(_) => {
                drop(connection);
                self.inspect_unscoped(&request.id)
            }
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(ApprovalError::AlreadyExists)
            }
            Err(_) => Err(ApprovalError::Database),
        }
    }

    pub fn inspect(
        &self,
        id: &str,
        principal_id: &PrincipalId,
    ) -> Result<PendingApproval, ApprovalError> {
        let approval = self.inspect_unscoped(id)?;
        if approval.principal_id != *principal_id {
            return Err(ApprovalError::NotFound);
        }
        Ok(approval)
    }

    pub fn list(&self, principal_id: &PrincipalId) -> Result<Vec<PendingApproval>, ApprovalError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT id, session_id, execution_id, principal_id, gene_id,
                        request_digest, request_summary, policy_version, expires_at,
                        status, approver_id, created_at
                 FROM approvals WHERE principal_id = ?1
                 ORDER BY created_at ASC, id ASC",
            )
            .map_err(|_| ApprovalError::Database)?;
        let rows = statement
            .query_map(params![principal_id.as_str()], decode_row)
            .map_err(|_| ApprovalError::Database)?;
        rows.map(|row| {
            row.map_err(|_| ApprovalError::Database)
                .and_then(decode_raw)
        })
        .collect()
    }

    pub fn resolve(
        &self,
        id: &str,
        principal_id: &PrincipalId,
        approver_id: &PrincipalId,
        allow: bool,
        now: Timestamp,
    ) -> Result<PendingApproval, ApprovalError> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| ApprovalError::Database)?;
        let current = load_approval(&transaction, id)?.ok_or(ApprovalError::NotFound)?;
        ensure_principal(&current, principal_id)?;
        if current.status_at(now) == ApprovalStatus::Expired {
            transaction
                .execute(
                    "UPDATE approvals SET status = 'expired' WHERE id = ?1 AND status IN ('pending', 'approved')",
                    params![id],
                )
                .map_err(|_| ApprovalError::Database)?;
            transaction.commit().map_err(|_| ApprovalError::Database)?;
            return Err(ApprovalError::Expired);
        }
        if current.status != ApprovalStatus::Pending {
            return Err(ApprovalError::Terminal);
        }
        let status = if allow { "approved" } else { "denied" };
        let changed = transaction
            .execute(
                "UPDATE approvals
                 SET status = ?1, approver_id = ?2, resolved_at = ?3
                 WHERE id = ?4 AND status = 'pending' AND expires_at > ?3",
                params![status, approver_id.as_str(), to_i64(now)?, id],
            )
            .map_err(|_| ApprovalError::Database)?;
        if changed != 1 {
            return Err(ApprovalError::Terminal);
        }
        transaction.commit().map_err(|_| ApprovalError::Database)?;
        drop(connection);
        self.inspect(id, principal_id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn consume(
        &self,
        id: &str,
        principal_id: &PrincipalId,
        session_id: &SessionId,
        execution_id: &ExecutionId,
        gene_id: &GeneId,
        request_digest: &RequestDigest,
        now: Timestamp,
    ) -> Result<PendingApproval, ApprovalError> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| ApprovalError::Database)?;
        let current = load_approval(&transaction, id)?.ok_or(ApprovalError::NotFound)?;
        ensure_principal(&current, principal_id)?;
        if current.status_at(now) == ApprovalStatus::Expired {
            transaction
                .execute(
                    "UPDATE approvals SET status = 'expired' WHERE id = ?1 AND status = 'approved'",
                    params![id],
                )
                .map_err(|_| ApprovalError::Database)?;
            transaction.commit().map_err(|_| ApprovalError::Database)?;
            return Err(ApprovalError::Expired);
        }
        if current.status != ApprovalStatus::Approved {
            return Err(ApprovalError::Terminal);
        }
        if current.session_id != *session_id
            || current.execution_id != *execution_id
            || current.gene_id != *gene_id
        {
            return Err(ApprovalError::ScopeMismatch);
        }
        if current.request_digest != *request_digest {
            return Err(ApprovalError::DigestMismatch);
        }
        let changed = transaction
            .execute(
                "UPDATE approvals SET status = 'consumed'
                 WHERE id = ?1 AND status = 'approved' AND expires_at > ?2",
                params![id, to_i64(now)?],
            )
            .map_err(|_| ApprovalError::Database)?;
        if changed != 1 {
            return Err(ApprovalError::Terminal);
        }
        transaction.commit().map_err(|_| ApprovalError::Database)?;
        drop(connection);
        self.inspect(id, principal_id)
    }

    fn inspect_unscoped(&self, id: &str) -> Result<PendingApproval, ApprovalError> {
        let connection = self.lock()?;
        load_approval(&connection, id)?.ok_or(ApprovalError::NotFound)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, ApprovalError> {
        self.connection
            .lock()
            .map_err(|_| ApprovalError::LockPoisoned)
    }
}

fn load_approval(
    connection: &Connection,
    id: &str,
) -> Result<Option<PendingApproval>, ApprovalError> {
    let raw = connection
        .query_row(
            "SELECT id, session_id, execution_id, principal_id, gene_id,
                    request_digest, request_summary, policy_version, expires_at,
                    status, approver_id, created_at
             FROM approvals WHERE id = ?1",
            params![id],
            decode_row,
        )
        .optional()
        .map_err(|_| ApprovalError::Database)?;
    raw.map(decode_raw).transpose()
}

type RawApproval = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    i64,
    i64,
    String,
    Option<String>,
    i64,
);

fn decode_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawApproval> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
    ))
}

fn decode_raw(raw: RawApproval) -> Result<PendingApproval, ApprovalError> {
    let (
        id,
        session_id,
        execution_id,
        principal_id,
        gene_id,
        request_digest,
        request_summary,
        policy_version,
        expires_at,
        status,
        approver_id,
        created_at,
    ) = raw;
    Ok(PendingApproval {
        id,
        session_id: SessionId::new(session_id).map_err(|_| ApprovalError::CorruptRecord)?,
        execution_id: ExecutionId::new(execution_id).map_err(|_| ApprovalError::CorruptRecord)?,
        principal_id: PrincipalId::new(principal_id).map_err(|_| ApprovalError::CorruptRecord)?,
        gene_id: GeneId::new(gene_id).map_err(|_| ApprovalError::CorruptRecord)?,
        request_digest: RequestDigest::new(request_digest)
            .map_err(|_| ApprovalError::CorruptRecord)?,
        request_summary,
        policy_version: u32::try_from(policy_version).map_err(|_| ApprovalError::CorruptRecord)?,
        expires_at: Timestamp::from_unix_seconds(
            u64::try_from(expires_at).map_err(|_| ApprovalError::CorruptRecord)?,
        ),
        status: ApprovalStatus::parse(&status)?,
        approver_id: approver_id
            .map(|value| PrincipalId::new(value).map_err(|_| ApprovalError::CorruptRecord))
            .transpose()?,
        created_at: Timestamp::from_unix_seconds(
            u64::try_from(created_at).map_err(|_| ApprovalError::CorruptRecord)?,
        ),
    })
}

fn ensure_principal(
    approval: &PendingApproval,
    principal_id: &PrincipalId,
) -> Result<(), ApprovalError> {
    if approval.principal_id == *principal_id {
        Ok(())
    } else {
        Err(ApprovalError::ScopeMismatch)
    }
}

fn redact_summary(summary: &str) -> String {
    summary
        .split_whitespace()
        .map(|token| {
            let lowercase = token.to_ascii_lowercase();
            if lowercase.contains("sk-")
                || lowercase.contains("token=")
                || lowercase.contains("api_key=")
                || lowercase.contains("apikey=")
                || lowercase == "bearer"
            {
                "[redacted]"
            } else {
                token
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn to_i64(timestamp: Timestamp) -> Result<i64, ApprovalError> {
    i64::try_from(timestamp.as_unix_seconds()).map_err(|_| ApprovalError::CorruptRecord)
}

fn current_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::{ExecutionId, GeneId, PrincipalId, RequestDigest, SessionId, Timestamp};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

    fn fixture() -> (ApprovalStore, PathBuf) {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be available")
            .as_nanos();
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "pandora-approvals-{}-{suffix}-{sequence}.sqlite3",
            std::process::id()
        ));
        let store = ApprovalStore::open(&path).expect("approval store should open");
        (store, path)
    }

    fn request(id: &str, expires_at: u64) -> ApprovalRequest {
        ApprovalRequest::new(
            id,
            SessionId::new("session-1").unwrap(),
            ExecutionId::new("execution-1").unwrap(),
            PrincipalId::new("principal-1").unwrap(),
            GeneId::new("patch.apply").unwrap(),
            RequestDigest::new("pandora-request-v1:sha256:abc").unwrap(),
            "patch README.md with sk-live-secret",
            1,
            Timestamp::from_unix_seconds(expires_at),
        )
        .unwrap()
    }

    fn principal() -> PrincipalId {
        PrincipalId::new("principal-1").unwrap()
    }

    fn approver() -> PrincipalId {
        PrincipalId::new("approver-1").unwrap()
    }

    #[test]
    fn approval_is_inspectable_without_persisting_secret_text() {
        let (store, path) = fixture();
        let approval = store.create(request("approval-1", 100)).unwrap();

        assert_eq!(approval.id(), "approval-1");
        assert_eq!(approval.session_id().as_str(), "session-1");
        assert_eq!(approval.execution_id().as_str(), "execution-1");
        assert_eq!(approval.gene_id().as_str(), "patch.apply");
        assert_eq!(
            approval.request_digest().as_str(),
            "pandora-request-v1:sha256:abc"
        );
        assert!(!approval.request_summary().contains("sk-live-secret"));
        assert!(approval.request_summary().contains("[redacted]"));
        assert_eq!(
            approval.status_at(Timestamp::from_unix_seconds(10)),
            ApprovalStatus::Pending
        );
        cleanup(path);
    }

    #[test]
    fn expired_approval_cannot_be_resolved() {
        let (store, path) = fixture();
        store.create(request("approval-1", 20)).unwrap();

        assert_eq!(
            store.resolve(
                "approval-1",
                &principal(),
                &approver(),
                true,
                Timestamp::from_unix_seconds(20)
            ),
            Err(ApprovalError::Expired)
        );
        assert_eq!(
            store
                .inspect("approval-1", &principal())
                .unwrap()
                .status_at(Timestamp::from_unix_seconds(20)),
            ApprovalStatus::Expired
        );
        cleanup(path);
    }

    #[test]
    fn consumption_requires_the_original_scope_and_digest() {
        let (store, path) = fixture();
        store.create(request("approval-1", 100)).unwrap();
        store
            .resolve(
                "approval-1",
                &principal(),
                &approver(),
                true,
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();
        let wrong_session = SessionId::new("session-2").unwrap();
        let digest = RequestDigest::new("pandora-request-v1:sha256:abc").unwrap();
        assert_eq!(
            store.consume(
                "approval-1",
                &principal(),
                &wrong_session,
                &ExecutionId::new("execution-1").unwrap(),
                &GeneId::new("patch.apply").unwrap(),
                &digest,
                Timestamp::from_unix_seconds(10),
            ),
            Err(ApprovalError::ScopeMismatch)
        );
        let wrong_digest = RequestDigest::new("pandora-request-v1:sha256:wrong").unwrap();
        assert_eq!(
            store.consume(
                "approval-1",
                &principal(),
                &SessionId::new("session-1").unwrap(),
                &ExecutionId::new("execution-1").unwrap(),
                &GeneId::new("patch.apply").unwrap(),
                &wrong_digest,
                Timestamp::from_unix_seconds(10),
            ),
            Err(ApprovalError::DigestMismatch)
        );
        cleanup(path);
    }

    #[test]
    fn approved_consumption_is_terminal_and_one_shot() {
        let (store, path) = fixture();
        store.create(request("approval-1", 100)).unwrap();
        store
            .resolve(
                "approval-1",
                &principal(),
                &approver(),
                true,
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();
        let consumed = store
            .consume(
                "approval-1",
                &principal(),
                &SessionId::new("session-1").unwrap(),
                &ExecutionId::new("execution-1").unwrap(),
                &GeneId::new("patch.apply").unwrap(),
                &RequestDigest::new("pandora-request-v1:sha256:abc").unwrap(),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();
        assert_eq!(
            consumed.status_at(Timestamp::from_unix_seconds(10)),
            ApprovalStatus::Consumed
        );
        assert_eq!(
            store.consume(
                "approval-1",
                &principal(),
                &SessionId::new("session-1").unwrap(),
                &ExecutionId::new("execution-1").unwrap(),
                &GeneId::new("patch.apply").unwrap(),
                &RequestDigest::new("pandora-request-v1:sha256:abc").unwrap(),
                Timestamp::from_unix_seconds(10),
            ),
            Err(ApprovalError::Terminal)
        );
        cleanup(path);
    }

    #[test]
    fn concurrent_resolution_allows_only_one_terminal_transition() {
        let (store, path) = fixture();
        store.create(request("approval-1", 100)).unwrap();
        let store = Arc::new(store);
        let barrier = Arc::new(Barrier::new(3));
        let mut handles = Vec::new();
        for _ in 0..2 {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                store.resolve(
                    "approval-1",
                    &principal(),
                    &approver(),
                    true,
                    Timestamp::from_unix_seconds(10),
                )
            }));
        }
        barrier.wait();
        let results = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| **result == Err(ApprovalError::Terminal))
                .count(),
            1
        );
        cleanup(path);
    }

    fn cleanup(path: PathBuf) {
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
    }
}
