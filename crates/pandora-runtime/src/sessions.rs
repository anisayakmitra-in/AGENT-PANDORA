use pandora_types::{
    PrincipalId, RuntimeEvent, Session, SessionId, TenantId, Timestamp, WorkspaceId,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::fmt;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

const CURRENT_SCHEMA_VERSION: i64 = 1;
const MAX_EVENT_BYTES: usize = 1_048_576;

#[derive(Debug)]
pub enum SessionError {
    Database(rusqlite::Error),
    Io(std::io::Error),
    Serialization(serde_json::Error),
    CorruptRecord,
    EventTooLarge,
    LockPoisoned,
    ScopeViolation,
    SessionAlreadyExists,
    SessionNotFound,
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
            Self::LockPoisoned => formatter.write_str("session database lock is unavailable"),
            Self::ScopeViolation => formatter.write_str("session is outside the requested scope"),
            Self::SessionAlreadyExists => formatter.write_str("session already exists"),
            Self::SessionNotFound => formatter.write_str("session was not found"),
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
}

impl SessionSnapshot {
    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn events(&self) -> &[RuntimeEvent] {
        &self.events
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
            "SELECT event_json FROM session_events
             WHERE session_id = ?1 ORDER BY sequence ASC",
        )?;
        let rows =
            statement.query_map(params![session_id.as_str()], |row| row.get::<_, String>(0))?;
        let mut events = Vec::new();
        for row in rows {
            let serialized = row?;
            events.push(serde_json::from_str(&serialized).map_err(SessionError::Serialization)?);
        }
        Ok(SessionSnapshot { session, events })
    }

    pub fn append_event(
        &self,
        session_id: &SessionId,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
        event: &RuntimeEvent,
    ) -> Result<(), SessionError> {
        let serialized = serde_json::to_vec(event)?;
        if serialized.len() > MAX_EVENT_BYTES {
            return Err(SessionError::EventTooLarge);
        }
        let connection = self.lock()?;
        let session =
            load_session(&connection, session_id)?.ok_or(SessionError::SessionNotFound)?;
        ensure_scope(&session, principal_id, tenant_id, workspace_id)?;
        connection.execute(
            "INSERT INTO session_events (session_id, event_json) VALUES (?1, ?2)",
            params![
                session_id.as_str(),
                String::from_utf8(serialized).map_err(|_| SessionError::CorruptRecord)?
            ],
        )?;
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
    let version = transaction.query_row(
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
