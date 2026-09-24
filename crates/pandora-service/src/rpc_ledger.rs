use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::Value;
use sha2::{Digest, Sha256};

const MAX_RESPONSE_BYTES: usize = 1_048_576;
const MAX_KEY_FIELD_BYTES: usize = 256;
const MAX_LEDGER_ROWS: i64 = 100_000;
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub enum RpcLedgerError {
    Database(rusqlite::Error),
    InvalidKey(String),
    ResponseTooLarge,
    InvalidResponse,
    LedgerFull,
    Conflict,
    NotPending,
    Poisoned,
}

impl fmt::Display for RpcLedgerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(_) => formatter.write_str("durable RPC ledger database error"),
            Self::InvalidKey(message) => {
                write!(formatter, "invalid RPC idempotency key: {message}")
            }
            Self::ResponseTooLarge => {
                formatter.write_str("durable RPC response exceeds the size limit")
            }
            Self::InvalidResponse => formatter.write_str("durable RPC response is not valid JSON"),
            Self::LedgerFull => formatter.write_str("durable RPC ledger is full"),
            Self::Conflict => formatter
                .write_str("durable RPC idempotency key conflicts with an existing request"),
            Self::NotPending => {
                formatter.write_str("durable RPC request is not pending completion")
            }
            Self::Poisoned => formatter.write_str("durable RPC ledger lock is poisoned"),
        }
    }
}

impl std::error::Error for RpcLedgerError {}

impl From<rusqlite::Error> for RpcLedgerError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RpcRequestKey {
    scope: String,
    request_id: String,
    method: String,
    request_digest: String,
}

impl RpcRequestKey {
    pub fn new(
        scope: impl Into<String>,
        request_id: impl Into<String>,
        method: impl Into<String>,
        request_digest: impl Into<String>,
    ) -> Result<Self, RpcLedgerError> {
        let key = Self {
            scope: scope.into(),
            request_id: request_id.into(),
            method: method.into(),
            request_digest: request_digest.into(),
        };
        key.validate()?;
        Ok(key)
    }

    pub fn scope(&self) -> &str {
        &self.scope
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub fn method(&self) -> &str {
        &self.method
    }

    pub fn request_digest(&self) -> &str {
        &self.request_digest
    }

    fn validate(&self) -> Result<(), RpcLedgerError> {
        for (name, value) in [
            ("scope", &self.scope),
            ("request_id", &self.request_id),
            ("method", &self.method),
        ] {
            if value.is_empty() || value.len() > MAX_KEY_FIELD_BYTES {
                return Err(RpcLedgerError::InvalidKey(format!(
                    "{name} must be between 1 and {MAX_KEY_FIELD_BYTES} bytes"
                )));
            }
            if value.chars().any(char::is_control) {
                return Err(RpcLedgerError::InvalidKey(format!(
                    "{name} contains a control character"
                )));
            }
        }
        if self.request_digest.len() != 64
            || !self
                .request_digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(RpcLedgerError::InvalidKey(
                "request_digest must be lowercase SHA-256 hex".to_owned(),
            ));
        }
        Ok(())
    }
}

pub fn digest_request(method: &str, params: &Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(method.as_bytes());
    hasher.update([0]);
    hasher.update(serde_json::to_vec(params).expect("serde_json Value serialization cannot fail"));
    format!("{:x}", hasher.finalize())
}

#[derive(Debug)]
pub enum RpcBegin {
    Execute,
    Replay { response: String },
    InProgress,
    Conflict,
}

pub struct DurableRpcLedger {
    connection: Mutex<Connection>,
    path: PathBuf,
}

impl DurableRpcLedger {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RpcLedgerError> {
        let path = path.as_ref().to_path_buf();
        if path.is_symlink() {
            return Err(RpcLedgerError::InvalidKey(
                "ledger path must not be a symlink".to_owned(),
            ));
        }
        let connection = Connection::open(&path)?;
        connection.busy_timeout(BUSY_TIMEOUT)?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             CREATE TABLE IF NOT EXISTS rpc_idempotency (
                 scope TEXT NOT NULL,
                 request_id TEXT NOT NULL,
                 method TEXT NOT NULL,
                 request_digest TEXT NOT NULL,
                 state TEXT NOT NULL CHECK (state IN ('pending', 'completed')),
                 response TEXT,
                 created_at INTEGER NOT NULL,
                 completed_at INTEGER,
                 PRIMARY KEY (scope, request_id)
             );
             CREATE INDEX IF NOT EXISTS rpc_idempotency_created_at
                 ON rpc_idempotency(created_at);",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
            path,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn begin(&self, key: &RpcRequestKey, now: u64) -> Result<RpcBegin, RpcLedgerError> {
        key.validate()?;
        let now = to_i64(now)?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| RpcLedgerError::Poisoned)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = transaction
            .query_row(
                "SELECT method, request_digest, state, response
                 FROM rpc_idempotency
                 WHERE scope = ?1 AND request_id = ?2",
                params![key.scope, key.request_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()?;
        let outcome = match existing {
            None => {
                let count: i64 =
                    transaction
                        .query_row("SELECT COUNT(*) FROM rpc_idempotency", [], |row| row.get(0))?;
                if count >= MAX_LEDGER_ROWS {
                    return Err(RpcLedgerError::LedgerFull);
                }
                transaction.execute(
                    "INSERT INTO rpc_idempotency
                        (scope, request_id, method, request_digest, state, created_at)
                     VALUES (?1, ?2, ?3, ?4, 'pending', ?5)",
                    params![
                        key.scope,
                        key.request_id,
                        key.method,
                        key.request_digest,
                        now
                    ],
                )?;
                RpcBegin::Execute
            }
            Some((method, request_digest, state, response)) => {
                if method != key.method || request_digest != key.request_digest {
                    RpcBegin::Conflict
                } else if state == "completed" {
                    RpcBegin::Replay {
                        response: response.ok_or(RpcLedgerError::NotPending)?,
                    }
                } else if state == "pending" {
                    RpcBegin::InProgress
                } else {
                    return Err(RpcLedgerError::Conflict);
                }
            }
        };
        transaction.commit()?;
        Ok(outcome)
    }

    pub fn complete(
        &self,
        key: &RpcRequestKey,
        response: &str,
        completed_at: u64,
    ) -> Result<(), RpcLedgerError> {
        key.validate()?;
        let completed_at = to_i64(completed_at)?;
        if response.len() > MAX_RESPONSE_BYTES {
            return Err(RpcLedgerError::ResponseTooLarge);
        }
        if serde_json::from_str::<Value>(response).is_err() {
            return Err(RpcLedgerError::InvalidResponse);
        }
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| RpcLedgerError::Poisoned)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = transaction
            .query_row(
                "SELECT method, request_digest, state, response
                 FROM rpc_idempotency
                 WHERE scope = ?1 AND request_id = ?2",
                params![key.scope, key.request_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((method, request_digest, state, stored_response)) = existing else {
            return Err(RpcLedgerError::NotPending);
        };
        if method != key.method || request_digest != key.request_digest {
            return Err(RpcLedgerError::Conflict);
        }
        match (state.as_str(), stored_response) {
            ("pending", _) => {
                transaction.execute(
                    "UPDATE rpc_idempotency
                     SET state = 'completed', response = ?3, completed_at = ?4
                     WHERE scope = ?1 AND request_id = ?2 AND state = 'pending'",
                    params![key.scope, key.request_id, response, completed_at],
                )?;
            }
            ("completed", Some(stored)) if stored == response => {}
            ("completed", _) => return Err(RpcLedgerError::Conflict),
            _ => return Err(RpcLedgerError::Conflict),
        }
        transaction.commit()?;
        Ok(())
    }
}

fn to_i64(value: u64) -> Result<i64, RpcLedgerError> {
    i64::try_from(value)
        .map_err(|_| RpcLedgerError::InvalidKey("timestamp is too large".to_owned()))
}
