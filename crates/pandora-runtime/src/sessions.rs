use pandora_provider::ChatMessage;
use pandora_types::{
    ContextClassification, EvaluationKind, EvaluationReceipt, EvaluationResult, EvaluationStatus,
    ExecutionId, MemoryKind, MemoryRecord, MemoryScope, PrincipalId, RuntimeEvent, Session,
    SessionId, TenantId, Timestamp, WorkspaceId,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::fmt;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

const CURRENT_SCHEMA_VERSION: i64 = 5;
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
    InvalidEvaluation,
    InvalidEventPage,
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
            Self::InvalidEvaluation => formatter.write_str("execution evaluation is invalid"),
            Self::InvalidEventPage => formatter.write_str("session event page is invalid"),
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
    evaluations: Vec<EvaluationReceipt>,
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

    pub fn evaluations(&self) -> &[EvaluationReceipt] {
        &self.evaluations
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionEventPage {
    events: Vec<RuntimeEvent>,
    next_sequence: Option<u64>,
}

impl SessionEventPage {
    pub fn events(&self) -> &[RuntimeEvent] {
        &self.events
    }

    pub const fn next_sequence(&self) -> Option<u64> {
        self.next_sequence
    }
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
        let evaluations = load_evaluations(&connection, session_id)?;
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
            evaluations,
            agent_messages,
        })
    }

    pub fn event_page(
        &self,
        session_id: &SessionId,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
        after_sequence: Option<u64>,
        limit: u16,
    ) -> Result<SessionEventPage, SessionError> {
        if limit == 0 {
            return Err(SessionError::InvalidEventPage);
        }
        let after_sequence = after_sequence
            .map(|sequence| i64::try_from(sequence).map_err(|_| SessionError::CorruptRecord))
            .transpose()?
            .unwrap_or_default();
        let connection = self.lock()?;
        let session =
            load_session(&connection, session_id)?.ok_or(SessionError::SessionNotFound)?;
        ensure_scope(&session, principal_id, tenant_id, workspace_id)?;
        let mut statement = connection.prepare(
            "SELECT sequence, event_json FROM session_events
             WHERE session_id = ?1 AND sequence > ?2
             ORDER BY sequence ASC
             LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![session_id.as_str(), after_sequence, i64::from(limit) + 1],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )?;
        let mut events = Vec::with_capacity(usize::from(limit) + 1);
        for row in rows {
            let (sequence, serialized) = row?;
            events.push((
                u64::try_from(sequence).map_err(|_| SessionError::CorruptRecord)?,
                serde_json::from_str(&serialized).map_err(SessionError::Serialization)?,
            ));
        }
        let next_sequence = if events.len() > usize::from(limit) {
            events.pop();
            events.last().map(|(sequence, _)| *sequence)
        } else {
            None
        };
        Ok(SessionEventPage {
            events: events.into_iter().map(|(_, event)| event).collect(),
            next_sequence,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn append_execution(
        &self,
        session_id: &SessionId,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
        events: &[RuntimeEvent],
        evaluation: &EvaluationReceipt,
        recorded_at: Timestamp,
    ) -> Result<(), SessionError> {
        if events.is_empty()
            || evaluation.session_id() != session_id
            || events.iter().any(|event| {
                let context = event.context();
                context.session_id() != Some(session_id)
                    || context.execution_id() != Some(evaluation.execution_id())
                    || context.tenant_id() != tenant_id
                    || context.workspace_id() != workspace_id
            })
        {
            return Err(SessionError::InvalidEvaluation);
        }
        let events = events
            .iter()
            .map(|event| {
                let serialized = serde_json::to_vec(event)?;
                if serialized.len() > MAX_EVENT_BYTES {
                    return Err(SessionError::EventTooLarge);
                }
                String::from_utf8(serialized).map_err(|_| SessionError::CorruptRecord)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let recorded_at = i64::try_from(recorded_at.as_unix_seconds())
            .map_err(|_| SessionError::CorruptRecord)?;
        let evaluated_at = i64::try_from(evaluation.evaluated_at().as_unix_seconds())
            .map_err(|_| SessionError::CorruptRecord)?;
        let mut connection = self.lock()?;
        let session =
            load_session(&connection, session_id)?.ok_or(SessionError::SessionNotFound)?;
        ensure_scope(&session, principal_id, tenant_id, workspace_id)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let attempt = transaction.query_row(
            "SELECT COALESCE(MAX(attempt), -1) + 1
             FROM session_evaluations
             WHERE session_id = ?1 AND execution_id = ?2",
            params![session_id.as_str(), evaluation.execution_id().as_str()],
            |row| row.get::<_, i64>(0),
        )?;
        for event in events {
            transaction.execute(
                "INSERT INTO session_events (session_id, event_json, recorded_at)
                 VALUES (?1, ?2, ?3)",
                params![session_id.as_str(), event, recorded_at],
            )?;
        }
        for (index, result) in evaluation.results().iter().enumerate() {
            transaction.execute(
                "INSERT INTO session_evaluations
                 (session_id, execution_id, attempt, result_index, kind, status, score, reason,
                  advisory, evaluated_at)
                  VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    session_id.as_str(),
                    evaluation.execution_id().as_str(),
                    attempt,
                    i64::try_from(index).map_err(|_| SessionError::CorruptRecord)?,
                    result.kind().as_str(),
                    result.status().as_str(),
                    i64::from(result.score()),
                    result.reason(),
                    result.advisory(),
                    evaluated_at,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
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
        version = 4;
    }
    if version == 4 {
        transaction.execute_batch(
            "CREATE TABLE session_evaluations (
                 session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                 execution_id TEXT NOT NULL,
                 attempt INTEGER NOT NULL CHECK (attempt >= 0),
                 result_index INTEGER NOT NULL CHECK (result_index >= 0),
                 kind TEXT NOT NULL,
                 status TEXT NOT NULL,
                 score INTEGER NOT NULL CHECK (score BETWEEN 0 AND 100),
                 reason TEXT NOT NULL,
                 advisory INTEGER NOT NULL CHECK (advisory IN (0, 1)),
                 evaluated_at INTEGER NOT NULL CHECK (evaluated_at >= 0),
                 PRIMARY KEY (session_id, execution_id, attempt, kind),
                 UNIQUE (session_id, execution_id, attempt, result_index)
             );
             CREATE INDEX session_evaluations_session_idx
                 ON session_evaluations(
                     session_id, evaluated_at, execution_id, attempt, result_index
                 );
             INSERT INTO schema_migrations (version, applied_at) VALUES (5, strftime('%s', 'now'));",
        )?;
    }
    transaction.commit()?;
    Ok(())
}

fn load_evaluations(
    connection: &Connection,
    session_id: &SessionId,
) -> Result<Vec<EvaluationReceipt>, SessionError> {
    let mut statement = connection.prepare(
        "SELECT execution_id, attempt, kind, status, score, reason, advisory, evaluated_at
         FROM session_evaluations
         WHERE session_id = ?1
         ORDER BY evaluated_at ASC, execution_id ASC, attempt ASC, result_index ASC",
    )?;
    let rows = statement.query_map(params![session_id.as_str()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, bool>(6)?,
            row.get::<_, i64>(7)?,
        ))
    })?;
    let mut evaluations = Vec::new();
    let mut current_key: Option<(ExecutionId, i64)> = None;
    let mut current_at = None;
    let mut current_results = Vec::new();
    for row in rows {
        let (execution_id, attempt, kind, status, score, reason, advisory, evaluated_at) = row?;
        let execution_id =
            ExecutionId::new(execution_id).map_err(|_| SessionError::CorruptRecord)?;
        if attempt < 0 {
            return Err(SessionError::CorruptRecord);
        }
        let evaluated_at = u64::try_from(evaluated_at)
            .map(Timestamp::from_unix_seconds)
            .map_err(|_| SessionError::CorruptRecord)?;
        let key = (execution_id, attempt);
        if current_key.as_ref().is_some_and(|current| current != &key) {
            let (execution_id, _) = current_key.take().ok_or(SessionError::CorruptRecord)?;
            evaluations.push(
                EvaluationReceipt::new(
                    session_id.clone(),
                    execution_id,
                    current_at.take().ok_or(SessionError::CorruptRecord)?,
                    std::mem::take(&mut current_results),
                )
                .map_err(|_| SessionError::CorruptRecord)?,
            );
        }
        if current_key.is_none() {
            current_key = Some(key);
            current_at = Some(evaluated_at);
        } else if current_at != Some(evaluated_at) {
            return Err(SessionError::CorruptRecord);
        }
        let score = u8::try_from(score).map_err(|_| SessionError::CorruptRecord)?;
        current_results.push(
            EvaluationResult::new(
                parse_evaluation_kind(&kind)?,
                parse_evaluation_status(&status)?,
                score,
                reason,
                advisory,
            )
            .map_err(|_| SessionError::CorruptRecord)?,
        );
    }
    if let Some((execution_id, _)) = current_key {
        evaluations.push(
            EvaluationReceipt::new(
                session_id.clone(),
                execution_id,
                current_at.ok_or(SessionError::CorruptRecord)?,
                current_results,
            )
            .map_err(|_| SessionError::CorruptRecord)?,
        );
    }
    Ok(evaluations)
}

fn parse_evaluation_kind(value: &str) -> Result<EvaluationKind, SessionError> {
    match value {
        "trajectory" => Ok(EvaluationKind::Trajectory),
        "outcome" => Ok(EvaluationKind::Outcome),
        "policy" => Ok(EvaluationKind::Policy),
        "human" => Ok(EvaluationKind::Human),
        "regression" => Ok(EvaluationKind::Regression),
        "adversarial" => Ok(EvaluationKind::Adversarial),
        _ => Err(SessionError::CorruptRecord),
    }
}

fn parse_evaluation_status(value: &str) -> Result<EvaluationStatus, SessionError> {
    match value {
        "passed" => Ok(EvaluationStatus::Passed),
        "failed" => Ok(EvaluationStatus::Failed),
        "human_review_required" => Ok(EvaluationStatus::HumanReviewRequired),
        _ => Err(SessionError::CorruptRecord),
    }
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
    use pandora_types::{
        EvaluationKind, EvaluationReceipt, EvaluationResult, EvaluationStatus, EventContext,
        EventId, EventPayload, EventType, ExecutionId,
    };

    #[test]
    fn execution_events_and_evaluation_commit_atomically() {
        let root = crate::test_support::new_temp_dir("pandora-session-evaluation-test").unwrap();
        let store = SessionStore::open(root.join("sessions.sqlite3")).unwrap();
        let session = Session::new(
            SessionId::new("session-1").unwrap(),
            PrincipalId::new("principal-1").unwrap(),
            TenantId::new("tenant-1").unwrap(),
            WorkspaceId::new("workspace-1").unwrap(),
            Timestamp::from_unix_seconds(1),
        );
        let execution_id = ExecutionId::new("execution-1").unwrap();
        let event = RuntimeEvent::new(
            EventId::new("event-1").unwrap(),
            EventType::SessionStarted,
            EventContext::new(session.tenant_id().clone(), session.workspace_id().clone())
                .with_session(session.id().clone())
                .with_execution(execution_id.clone()),
            EventPayload::Empty,
        );
        let evaluation = EvaluationReceipt::new(
            session.id().clone(),
            execution_id,
            Timestamp::from_unix_seconds(2),
            vec![
                EvaluationResult::new(
                    EvaluationKind::Trajectory,
                    EvaluationStatus::Passed,
                    100,
                    "trajectory passed",
                    false,
                )
                .unwrap(),
                EvaluationResult::new(
                    EvaluationKind::Policy,
                    EvaluationStatus::Passed,
                    100,
                    "policy passed",
                    false,
                )
                .unwrap(),
            ],
        )
        .unwrap();
        store.create(&session).unwrap();
        store
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_evaluation_insert
                 BEFORE INSERT ON session_evaluations
                 BEGIN
                     SELECT RAISE(ABORT, 'forced evaluation failure');
                 END;",
            )
            .unwrap();

        assert!(matches!(
            store.append_execution(
                session.id(),
                session.principal_id(),
                session.tenant_id(),
                session.workspace_id(),
                std::slice::from_ref(&event),
                &evaluation,
                Timestamp::from_unix_seconds(2),
            ),
            Err(SessionError::Database(_))
        ));
        let snapshot = store
            .resume(
                session.id(),
                session.principal_id(),
                session.tenant_id(),
                session.workspace_id(),
            )
            .unwrap();
        assert!(snapshot.events().is_empty());
        assert!(snapshot.evaluations().is_empty());
        store
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_evaluation_insert;")
            .unwrap();

        store
            .append_execution(
                session.id(),
                session.principal_id(),
                session.tenant_id(),
                session.workspace_id(),
                std::slice::from_ref(&event),
                &evaluation,
                Timestamp::from_unix_seconds(2),
            )
            .unwrap();
        let second_event = RuntimeEvent::new(
            EventId::new("event-2").unwrap(),
            EventType::EffectCompleted,
            EventContext::new(session.tenant_id().clone(), session.workspace_id().clone())
                .with_session(session.id().clone())
                .with_execution(evaluation.execution_id().clone()),
            EventPayload::Empty,
        );
        store
            .append_execution(
                session.id(),
                session.principal_id(),
                session.tenant_id(),
                session.workspace_id(),
                &[second_event],
                &evaluation,
                Timestamp::from_unix_seconds(3),
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
        assert_eq!(snapshot.events().len(), 2);
        assert_eq!(snapshot.evaluations(), &[evaluation.clone(), evaluation]);
        let _ = std::fs::remove_dir_all(root);
    }

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
