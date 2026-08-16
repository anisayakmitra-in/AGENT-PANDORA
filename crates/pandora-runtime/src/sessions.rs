use pandora_provider::ChatMessage;
use pandora_types::{
    ContextClassification, MemoryKind, MemoryRecord, MemoryScope, PrincipalId, RuntimeEvent,
    Session, SessionId, TenantId, Timestamp, WorkspaceId,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::fmt;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

const CURRENT_SCHEMA_VERSION: i64 = 4;
const MAX_EVENT_BYTES: usize = 1_048_576;
const MAX_AGENT_MESSAGE_BYTES: usize = 1_048_576;
const MAX_AGENT_TRANSCRIPT_BYTES: usize = 8 * 1_048_576;
const MAX_AGENT_MESSAGES: usize = 256;
pub const MAX_L1_EVIDENCE_PER_SCOPE: usize = 64;
pub const MAX_L1_EVIDENCE_CONTEXT_RECORDS: usize = 8;

#[derive(Debug)]
pub enum SessionError {
    Database(rusqlite::Error),
    Io(std::io::Error),
    Serialization(serde_json::Error),
    CorruptRecord,
    EventTooLarge,
    AgentMessageTooLarge,
    AgentTranscriptTooLarge,
    LockPoisoned,
    ScopeViolation,
    SessionAlreadyExists,
    SessionNotFound,
    L1EvidenceAlreadyExists,
    L1EvidenceCapacityExceeded,
    InvalidL1Evidence,
    UnsupportedSchemaVersion(i64),
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(_) => formatter.write_str("session database operation failed"),
            Self::Io(_) => formatter.write_str("session database directory operation failed"),
            Self::Serialization(_) => formatter.write_str("session event is invalid"),
            Self::CorruptRecord => {
                formatter.write_str("session database contains an invalid record")
            }
            Self::EventTooLarge => formatter.write_str("session event exceeds its size limit"),
            Self::AgentMessageTooLarge => {
                formatter.write_str("agent transcript message exceeds its size limit")
            }
            Self::AgentTranscriptTooLarge => {
                formatter.write_str("agent transcript exceeds its message limit")
            }
            Self::LockPoisoned => formatter.write_str("session database lock is unavailable"),
            Self::ScopeViolation => formatter.write_str("session is outside the requested scope"),
            Self::SessionAlreadyExists => formatter.write_str("session already exists"),
            Self::SessionNotFound => formatter.write_str("session was not found"),
            Self::L1EvidenceAlreadyExists => formatter.write_str("L1 evidence already exists"),
            Self::L1EvidenceCapacityExceeded => {
                formatter.write_str("L1 evidence capacity is exhausted")
            }
            Self::InvalidL1Evidence => formatter.write_str("L1 evidence record is invalid"),
            Self::UnsupportedSchemaVersion(version) => {
                write!(
                    formatter,
                    "session database schema version {version} is unsupported"
                )
            }
        }
    }
}

impl std::error::Error for SessionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Serialization(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for SessionError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

impl From<std::io::Error> for SessionError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for SessionError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionSnapshot {
    session: Session,
    events: Vec<RuntimeEvent>,
    event_metadata: Vec<EventMetadata>,
    l1_evidence_count: usize,
    agent_messages: Vec<ChatMessage>,
}

impl SessionSnapshot {
    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn events(&self) -> &[RuntimeEvent] {
        &self.events
    }

    pub fn recorded_events(
        &self,
    ) -> impl DoubleEndedIterator<Item = RecordedEvent<'_>> + ExactSizeIterator + '_ {
        self.events
            .iter()
            .zip(&self.event_metadata)
            .map(|(event, metadata)| RecordedEvent {
                sequence: metadata.sequence,
                recorded_at: metadata.recorded_at,
                event,
            })
    }

    pub const fn l1_evidence_count(&self) -> usize {
        self.l1_evidence_count
    }

    pub fn agent_messages(&self) -> &[ChatMessage] {
        &self.agent_messages
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecordedEvent<'a> {
    sequence: u64,
    recorded_at: Option<Timestamp>,
    event: &'a RuntimeEvent,
}

impl RecordedEvent<'_> {
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    pub const fn recorded_at(&self) -> Option<Timestamp> {
        self.recorded_at
    }

    pub fn event(&self) -> &RuntimeEvent {
        self.event
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct EventMetadata {
    sequence: u64,
    recorded_at: Option<Timestamp>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct L1EvidenceContext {
    scope: MemoryScope,
    records: Vec<MemoryRecord>,
}

impl L1EvidenceContext {
    pub const fn len(&self) -> usize {
        self.records.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub(crate) fn records(&self) -> &[MemoryRecord] {
        &self.records
    }

    pub(crate) fn matches(&self, session: &Session, provider: &str) -> bool {
        self.scope.session_id() == session.id()
            && self.scope.tenant_id() == session.tenant_id()
            && self.scope.workspace_id() == session.workspace_id()
            && self.scope.provider() == provider
    }
}

pub struct SessionStore {
    connection: Mutex<Connection>,
}

impl SessionStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, SessionError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let mut connection = Connection::open(path)?;
        set_private_permissions(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;",
        )?;
        migrate(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn create(&self, session: &Session) -> Result<(), SessionError> {
        let connection = self.lock()?;
        let result = connection.execute(
            "INSERT INTO sessions (id, principal_id, tenant_id, workspace_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                session.id().as_str(),
                session.principal_id().as_str(),
                session.tenant_id().as_str(),
                session.workspace_id().as_str(),
                i64::try_from(session.created_at().as_unix_seconds())
                    .map_err(|_| SessionError::CorruptRecord)?,
            ],
        );
        match result {
            Ok(_) => Ok(()),
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(SessionError::SessionAlreadyExists)
            }
            Err(error) => Err(SessionError::Database(error)),
        }
    }

    pub fn resume(
        &self,
        session_id: &SessionId,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
    ) -> Result<SessionSnapshot, SessionError> {
        let connection = self.lock()?;
        let session =
            load_session(&connection, session_id)?.ok_or(SessionError::SessionNotFound)?;
        ensure_scope(&session, principal_id, tenant_id, workspace_id)?;
        let mut statement = connection.prepare(
            "SELECT sequence, event_json, recorded_at FROM session_events
             WHERE session_id = ?1 ORDER BY sequence ASC",
        )?;
        let rows = statement.query_map(params![session_id.as_str()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
            ))
        })?;
        let mut events = Vec::new();
        let mut event_metadata = Vec::new();
        for row in rows {
            let (sequence, serialized, recorded_at) = row?;
            let event: RuntimeEvent =
                serde_json::from_str(&serialized).map_err(SessionError::Serialization)?;
            let sequence = u64::try_from(sequence).map_err(|_| SessionError::CorruptRecord)?;
            let recorded_at = recorded_at
                .map(|value| {
                    u64::try_from(value)
                        .map(Timestamp::from_unix_seconds)
                        .map_err(|_| SessionError::CorruptRecord)
                })
                .transpose()?;
            events.push(event);
            event_metadata.push(EventMetadata {
                sequence,
                recorded_at,
            });
        }
        let l1_evidence_count = connection.query_row(
            "SELECT COUNT(*) FROM session_l1_evidence WHERE session_id = ?1",
            params![session_id.as_str()],
            |row| row.get::<_, i64>(0),
        )?;
        let l1_evidence_count =
            usize::try_from(l1_evidence_count).map_err(|_| SessionError::CorruptRecord)?;
        let mut statement = connection.prepare(
            "SELECT message_json FROM agent_messages
             WHERE session_id = ?1 ORDER BY sequence ASC",
        )?;
        let rows =
            statement.query_map(params![session_id.as_str()], |row| row.get::<_, String>(0))?;
        let mut agent_messages = Vec::new();
        for row in rows {
            let serialized = row?;
            agent_messages
                .push(serde_json::from_str(&serialized).map_err(SessionError::Serialization)?);
        }
        Ok(SessionSnapshot {
            session,
            events,
            event_metadata,
            l1_evidence_count,
            agent_messages,
        })
    }

    pub fn append_event(
        &self,
        session_id: &SessionId,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
        event: &RuntimeEvent,
    ) -> Result<(), SessionError> {
        self.append_event_recorded_at(
            session_id,
            principal_id,
            tenant_id,
            workspace_id,
            event,
            None,
        )
    }

    pub fn append_event_at(
        &self,
        session_id: &SessionId,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
        event: &RuntimeEvent,
        recorded_at: Timestamp,
    ) -> Result<(), SessionError> {
        self.append_event_recorded_at(
            session_id,
            principal_id,
            tenant_id,
            workspace_id,
            event,
            Some(recorded_at),
        )
    }

    fn append_event_recorded_at(
        &self,
        session_id: &SessionId,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
        event: &RuntimeEvent,
        recorded_at: Option<Timestamp>,
    ) -> Result<(), SessionError> {
        let serialized = serde_json::to_vec(event)?;
        if serialized.len() > MAX_EVENT_BYTES {
            return Err(SessionError::EventTooLarge);
        }
        let connection = self.lock()?;
        let session =
            load_session(&connection, session_id)?.ok_or(SessionError::SessionNotFound)?;
        ensure_scope(&session, principal_id, tenant_id, workspace_id)?;
        let recorded_at = recorded_at
            .map(|value| {
                i64::try_from(value.as_unix_seconds()).map_err(|_| SessionError::CorruptRecord)
            })
            .transpose()?;
        connection.execute(
            "INSERT INTO session_events (session_id, event_json, recorded_at) VALUES (?1, ?2, ?3)",
            params![
                session_id.as_str(),
                String::from_utf8(serialized).map_err(|_| SessionError::CorruptRecord)?,
                recorded_at,
            ],
        )?;
        Ok(())
    }

    pub fn record_l1_evidence(
        &self,
        principal_id: &PrincipalId,
        record: &MemoryRecord,
    ) -> Result<(), SessionError> {
        if record.tier() != pandora_types::MemoryTier::L1
            || record.kind() != MemoryKind::ExecutionEvidence
            || record.classification() != ContextClassification::Internal
            || !is_canonical_l1_execution_evidence(record)
        {
            return Err(SessionError::InvalidL1Evidence);
        }
        let scope = record.scope();
        let mut connection = self.lock()?;
        let session =
            load_session(&connection, scope.session_id())?.ok_or(SessionError::SessionNotFound)?;
        ensure_scope(
            &session,
            principal_id,
            scope.tenant_id(),
            scope.workspace_id(),
        )?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let count = transaction.query_row(
            "SELECT COUNT(*) FROM session_l1_evidence
             WHERE session_id = ?1 AND provider = ?2",
            params![scope.session_id().as_str(), scope.provider()],
            |row| row.get::<_, i64>(0),
        )?;
        let count = usize::try_from(count).map_err(|_| SessionError::CorruptRecord)?;
        if count >= MAX_L1_EVIDENCE_PER_SCOPE {
            return Err(SessionError::L1EvidenceCapacityExceeded);
        }
        let result = transaction.execute(
            "INSERT INTO session_l1_evidence
             (session_id, provider, memory_id, summary, created_at, provenance)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                scope.session_id().as_str(),
                scope.provider(),
                record.id().as_str(),
                record.summary(),
                i64::try_from(record.created_at().as_unix_seconds())
                    .map_err(|_| SessionError::CorruptRecord)?,
                record.provenance(),
            ],
        );
        match result {
            Ok(_) => transaction.commit().map_err(SessionError::Database),
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(SessionError::L1EvidenceAlreadyExists)
            }
            Err(error) => Err(SessionError::Database(error)),
        }
    }

    pub fn l1_evidence_context(
        &self,
        session_id: &SessionId,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
        provider: impl Into<String>,
    ) -> Result<L1EvidenceContext, SessionError> {
        let scope = MemoryScope::new(
            tenant_id.clone(),
            workspace_id.clone(),
            session_id.clone(),
            provider,
        )
        .map_err(|_| SessionError::InvalidL1Evidence)?;
        let connection = self.lock()?;
        let session =
            load_session(&connection, session_id)?.ok_or(SessionError::SessionNotFound)?;
        ensure_scope(&session, principal_id, tenant_id, workspace_id)?;
        let mut statement = connection.prepare(
            "SELECT memory_id, summary, created_at, provenance
             FROM session_l1_evidence
             WHERE session_id = ?1 AND provider = ?2
             ORDER BY created_at DESC, memory_id DESC
             LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![
                session_id.as_str(),
                scope.provider(),
                i64::try_from(MAX_L1_EVIDENCE_CONTEXT_RECORDS)
                    .map_err(|_| SessionError::CorruptRecord)?,
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )?;
        let mut records = Vec::new();
        for row in rows {
            let (id, summary, created_at, provenance) = row?;
            let created_at = u64::try_from(created_at)
                .map(Timestamp::from_unix_seconds)
                .map_err(|_| SessionError::CorruptRecord)?;
            let record = MemoryRecord::new_l1(
                id,
                MemoryKind::ExecutionEvidence,
                scope.clone(),
                summary,
                ContextClassification::Internal,
                created_at,
                provenance,
            )
            .map_err(|_| SessionError::CorruptRecord)?;
            if !is_canonical_l1_execution_evidence(&record) {
                return Err(SessionError::CorruptRecord);
            }
            records.push(record);
        }
        Ok(L1EvidenceContext { scope, records })
    }

    pub fn save_agent_transcript(
        &self,
        session_id: &SessionId,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
        messages: &[ChatMessage],
    ) -> Result<(), SessionError> {
        if messages.len() > MAX_AGENT_MESSAGES {
            return Err(SessionError::AgentTranscriptTooLarge);
        }
        let serialized = messages
            .iter()
            .map(|message| {
                let bytes = serde_json::to_vec(message)?;
                if bytes.len() > MAX_AGENT_MESSAGE_BYTES {
                    return Err(SessionError::AgentMessageTooLarge);
                }
                String::from_utf8(bytes).map_err(|_| SessionError::CorruptRecord)
            })
            .collect::<Result<Vec<_>, SessionError>>()?;
        let total_bytes = serialized.iter().map(String::len).sum::<usize>();
        if total_bytes > MAX_AGENT_TRANSCRIPT_BYTES {
            return Err(SessionError::AgentTranscriptTooLarge);
        }

        let mut connection = self.lock()?;
        let session =
            load_session(&connection, session_id)?.ok_or(SessionError::SessionNotFound)?;
        ensure_scope(&session, principal_id, tenant_id, workspace_id)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "DELETE FROM agent_messages WHERE session_id = ?1",
            params![session_id.as_str()],
        )?;
        for message in serialized {
            transaction.execute(
                "INSERT INTO agent_messages (session_id, message_json) VALUES (?1, ?2)",
                params![session_id.as_str(), message],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn list(
        &self,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<Session>, SessionError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id, principal_id, tenant_id, workspace_id, created_at
             FROM sessions
             WHERE principal_id = ?1 AND tenant_id = ?2 AND workspace_id = ?3
             ORDER BY created_at ASC, id ASC",
        )?;
        let rows = statement.query_map(
            params![
                principal_id.as_str(),
                tenant_id.as_str(),
                workspace_id.as_str()
            ],
            decode_session_row,
        )?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row.map_err(SessionError::Database)??);
        }
        Ok(sessions)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, SessionError> {
        self.connection
            .lock()
            .map_err(|_| SessionError::LockPoisoned)
    }
}

fn migrate(connection: &mut Connection) -> Result<(), SessionError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             version INTEGER PRIMARY KEY,
             applied_at INTEGER NOT NULL
         );",
    )?;
    let mut version = transaction.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    if version > CURRENT_SCHEMA_VERSION {
        return Err(SessionError::UnsupportedSchemaVersion(version));
    }
    if version == 0 {
        transaction.execute_batch(
            "CREATE TABLE sessions (
                 id TEXT PRIMARY KEY NOT NULL,
                 principal_id TEXT NOT NULL,
                 tenant_id TEXT NOT NULL,
                 workspace_id TEXT NOT NULL,
                 created_at INTEGER NOT NULL CHECK (created_at >= 0)
             );
             CREATE TABLE session_events (
                 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                 session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                 event_json TEXT NOT NULL
             );
             CREATE INDEX session_events_session_idx ON session_events(session_id, sequence);
             CREATE INDEX sessions_scope_idx ON sessions(principal_id, tenant_id, workspace_id);
             INSERT INTO schema_migrations (version, applied_at) VALUES (1, strftime('%s', 'now'));",
        )?;
        version = 1;
    }
    if version == 1 {
        transaction.execute_batch(
            "CREATE TABLE agent_messages (
                 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                 session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                 message_json TEXT NOT NULL
             );
             CREATE INDEX agent_messages_session_idx ON agent_messages(session_id, sequence);
             INSERT INTO schema_migrations (version, applied_at) VALUES (2, strftime('%s', 'now'));",
        )?;
        version = 2;
    }
    if version == 2 {
        transaction.execute_batch(
            "ALTER TABLE session_events ADD COLUMN recorded_at INTEGER CHECK (recorded_at >= 0);
             INSERT INTO schema_migrations (version, applied_at) VALUES (3, strftime('%s', 'now'));",
        )?;
        version = 3;
    }
    if version == 3 {
        transaction.execute_batch(
            "CREATE TABLE session_l1_evidence (
                 session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                 provider TEXT NOT NULL,
                 memory_id TEXT NOT NULL,
                 summary TEXT NOT NULL,
                 created_at INTEGER NOT NULL CHECK (created_at >= 0),
                 provenance TEXT NOT NULL,
                 PRIMARY KEY (session_id, provider, memory_id)
             );
             CREATE INDEX session_l1_evidence_scope_idx
                 ON session_l1_evidence(session_id, provider, created_at);
             INSERT INTO schema_migrations (version, applied_at) VALUES (4, strftime('%s', 'now'));",
        )?;
    }
    transaction.commit()?;
    Ok(())
}

fn load_session(
    connection: &Connection,
    session_id: &SessionId,
) -> Result<Option<Session>, SessionError> {
    connection
        .query_row(
            "SELECT id, principal_id, tenant_id, workspace_id, created_at
             FROM sessions WHERE id = ?1",
            params![session_id.as_str()],
            decode_session_row,
        )
        .optional()
        .map_err(SessionError::Database)?
        .transpose()
}

fn decode_session_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Result<Session, SessionError>> {
    let id = row.get::<_, String>(0)?;
    let principal_id = row.get::<_, String>(1)?;
    let tenant_id = row.get::<_, String>(2)?;
    let workspace_id = row.get::<_, String>(3)?;
    let created_at = row.get::<_, i64>(4)?;
    let session = u64::try_from(created_at)
        .ok()
        .and_then(|created_at| {
            Some(Session::new(
                SessionId::new(id).ok()?,
                PrincipalId::new(principal_id).ok()?,
                TenantId::new(tenant_id).ok()?,
                WorkspaceId::new(workspace_id).ok()?,
                Timestamp::from_unix_seconds(created_at),
            ))
        })
        .ok_or(SessionError::CorruptRecord);
    Ok(session)
}

fn ensure_scope(
    session: &Session,
    principal_id: &PrincipalId,
    tenant_id: &TenantId,
    workspace_id: &WorkspaceId,
) -> Result<(), SessionError> {
    if session.principal_id() != principal_id
        || session.tenant_id() != tenant_id
        || session.workspace_id() != workspace_id
    {
        return Err(SessionError::ScopeViolation);
    }
    Ok(())
}

fn is_canonical_l1_execution_evidence(record: &MemoryRecord) -> bool {
    let Some((status, target)) = record.summary().split_once(" execution through ") else {
        return false;
    };
    let Some((harness, gene)) = target.rsplit_once('/') else {
        return false;
    };
    matches!(
        status,
        "completed" | "denied" | "approval_required" | "failed"
    ) && !harness.is_empty()
        && !gene.is_empty()
        && record.provenance() == format!("execution:{}", record.id())
}

fn set_private_permissions(path: &Path) -> Result<(), SessionError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_provider::{ChatMessage, ToolCall};

    #[test]
    fn agent_transcript_round_trips_with_session_scope() {
        let root = crate::test_support::new_temp_dir("pandora-session-test").unwrap();
        let store = SessionStore::open(root.join("sessions.sqlite3")).unwrap();
        let session = Session::new(
            SessionId::new("session-1").unwrap(),
            PrincipalId::new("principal-1").unwrap(),
            TenantId::new("tenant-1").unwrap(),
            WorkspaceId::new("workspace-1").unwrap(),
            Timestamp::from_unix_seconds(1),
        );
        store.create(&session).unwrap();
        let call = ToolCall::new(
            "call-1",
            "workspace.read",
            serde_json::json!({"path": "README.md"}),
        )
        .unwrap();
        let transcript = vec![
            ChatMessage::user("read README").unwrap(),
            ChatMessage::assistant_tool_calls(&[call]).unwrap(),
            ChatMessage::tool_result("call-1", "fixture").unwrap(),
        ];

        store
            .save_agent_transcript(
                session.id(),
                session.principal_id(),
                session.tenant_id(),
                session.workspace_id(),
                &transcript,
            )
            .unwrap();

        let snapshot = store
            .resume(
                session.id(),
                session.principal_id(),
                session.tenant_id(),
                session.workspace_id(),
            )
            .unwrap();
        assert_eq!(snapshot.agent_messages(), transcript.as_slice());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn agent_transcript_rejects_excess_total_bytes() {
        let root = crate::test_support::new_temp_dir("pandora-session-size-test").unwrap();
        let store = SessionStore::open(root.join("sessions.sqlite3")).unwrap();
        let session = Session::new(
            SessionId::new("session-1").unwrap(),
            PrincipalId::new("principal-1").unwrap(),
            TenantId::new("tenant-1").unwrap(),
            WorkspaceId::new("workspace-1").unwrap(),
            Timestamp::from_unix_seconds(1),
        );
        store.create(&session).unwrap();
        let message = ChatMessage::user("a".repeat(1_000_000)).unwrap();
        let transcript = vec![message; 9];

        assert!(matches!(
            store.save_agent_transcript(
                session.id(),
                session.principal_id(),
                session.tenant_id(),
                session.workspace_id(),
                &transcript,
            ),
            Err(SessionError::AgentTranscriptTooLarge)
        ));
        let _ = std::fs::remove_dir_all(root);
    }
}
