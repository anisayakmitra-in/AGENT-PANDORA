use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

pub const FLEET_SCHEMA_VERSION: u32 = 3;
pub const MAX_FLEET_NODES: usize = 256;
pub const MAX_FLEET_LEASES: usize = 4_096;
pub const MAX_FLEET_CAPABILITIES: usize = 64;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FleetNodeState {
    Ready,
    Quarantined,
    Revoked,
    Killed,
}

impl FleetNodeState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Quarantined => "quarantined",
            Self::Revoked => "revoked",
            Self::Killed => "killed",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FleetLeaseState {
    Active,
    Released,
    Expired,
    Revoked,
    Killed,
}

impl FleetLeaseState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Released => "released",
            Self::Expired => "expired",
            Self::Revoked => "revoked",
            Self::Killed => "killed",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FleetSupervisorState {
    Stopped,
    Running,
    Draining,
    Recovering,
}

impl FleetSupervisorState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Running => "running",
            Self::Draining => "draining",
            Self::Recovering => "recovering",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FleetSupervisor {
    node_id: String,
    state: FleetSupervisorState,
    generation: u64,
    process_id: Option<u32>,
    reason: Option<String>,
    updated_at: u64,
}

impl FleetSupervisor {
    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    pub const fn state(&self) -> FleetSupervisorState {
        self.state
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub const fn process_id(&self) -> Option<u32> {
        self.process_id
    }

    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    pub const fn updated_at(&self) -> u64 {
        self.updated_at
    }
}

pub struct FleetQuiescenceGuard {
    connection: Arc<Mutex<Connection>>,
    owner: String,
}

impl FleetQuiescenceGuard {
    pub fn owner(&self) -> &str {
        &self.owner
    }
}

impl Drop for FleetQuiescenceGuard {
    fn drop(&mut self) {
        if let Ok(connection) = self.connection.lock() {
            let _ = connection.execute(
                "DELETE FROM fleet_quiescence WHERE id = 1 AND owner = ?1",
                params![self.owner],
            );
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FleetBudget {
    max_tokens: u64,
    max_tools: u64,
    max_duration_seconds: u64,
    max_cost_micros: u64,
}

impl FleetBudget {
    pub const fn new(
        max_tokens: u64,
        max_tools: u64,
        max_duration_seconds: u64,
        max_cost_micros: u64,
    ) -> Self {
        Self {
            max_tokens,
            max_tools,
            max_duration_seconds,
            max_cost_micros,
        }
    }

    pub const fn max_tokens(&self) -> u64 {
        self.max_tokens
    }

    pub const fn max_tools(&self) -> u64 {
        self.max_tools
    }

    pub const fn max_duration_seconds(&self) -> u64 {
        self.max_duration_seconds
    }

    pub const fn max_cost_micros(&self) -> u64 {
        self.max_cost_micros
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FleetNode {
    id: String,
    implementation_version: String,
    worker_class: String,
    capabilities: Vec<String>,
    state: FleetNodeState,
    registered_at: u64,
}

impl FleetNode {
    pub fn new(
        id: impl Into<String>,
        implementation_version: impl Into<String>,
        worker_class: impl Into<String>,
        capabilities: impl IntoIterator<Item = String>,
        registered_at: u64,
    ) -> Result<Self, FleetError> {
        let mut capabilities = capabilities.into_iter().collect::<Vec<_>>();
        if capabilities.len() > MAX_FLEET_CAPABILITIES {
            return Err(FleetError::CapabilityLimitExceeded);
        }
        for capability in &mut capabilities {
            let value = std::mem::take(capability);
            *capability = validate_text("capability", value, 128)?;
        }
        capabilities.sort();
        if capabilities.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(FleetError::DuplicateCapability);
        }
        Ok(Self {
            id: validate_text("node ID", id.into(), 256)?,
            implementation_version: validate_text(
                "implementation version",
                implementation_version.into(),
                128,
            )?,
            worker_class: validate_text("worker class", worker_class.into(), 128)?,
            capabilities,
            state: FleetNodeState::Ready,
            registered_at,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn implementation_version(&self) -> &str {
        &self.implementation_version
    }

    pub fn worker_class(&self) -> &str {
        &self.worker_class
    }

    pub fn capabilities(&self) -> &[String] {
        &self.capabilities
    }

    pub const fn state(&self) -> FleetNodeState {
        self.state
    }

    pub const fn registered_at(&self) -> u64 {
        self.registered_at
    }

    pub fn supports(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|value| value == capability)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FleetLease {
    id: String,
    node_id: String,
    execution_id: String,
    budget: FleetBudget,
    issued_at: u64,
    expires_at: u64,
    state: FleetLeaseState,
}

impl FleetLease {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    pub fn execution_id(&self) -> &str {
        &self.execution_id
    }

    pub fn budget(&self) -> &FleetBudget {
        &self.budget
    }

    pub const fn issued_at(&self) -> u64 {
        self.issued_at
    }

    pub const fn expires_at(&self) -> u64 {
        self.expires_at
    }

    pub const fn state(&self) -> FleetLeaseState {
        self.state
    }
}

#[derive(Debug)]
pub enum FleetError {
    Database(rusqlite::Error),
    Serialization(serde_json::Error),
    InvalidField(&'static str),
    CapabilityLimitExceeded,
    DuplicateCapability,
    NodeAlreadyRegistered,
    NodeNotFound,
    NodeUnavailable(FleetNodeState),
    LeaseAlreadyExists,
    LeaseNotFound,
    LeaseNotActive(FleetLeaseState),
    LeaseExecutionMismatch,
    SupervisorNotFound,
    SupervisorAlreadyRunning,
    SupervisorNotStale,
    SupervisorNotAcceptingWork(FleetSupervisorState),
    SupervisorProcessMismatch,
    ActiveLeasesPresent,
    InvalidSupervisorTransition {
        state: FleetSupervisorState,
        action: &'static str,
    },
    InvalidSupervisorStaleness,
    QuiescenceHeld,
    QuiescenceActive,
    InvalidLeaseDuration,
    FleetNodeLimitExceeded,
    FleetLeaseLimitExceeded,
    CorruptRecord,
    LockPoisoned,
}

impl fmt::Display for FleetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(_) => formatter.write_str("fleet database operation failed"),
            Self::Serialization(_) => formatter.write_str("fleet record is invalid"),
            Self::InvalidField(field) => write!(formatter, "{field} is invalid"),
            Self::CapabilityLimitExceeded => {
                formatter.write_str("fleet node capability limit was exceeded")
            }
            Self::DuplicateCapability => formatter.write_str("fleet node capability is duplicated"),
            Self::NodeAlreadyRegistered => formatter.write_str("fleet node is already registered"),
            Self::NodeNotFound => formatter.write_str("fleet node was not found"),
            Self::NodeUnavailable(state) => {
                write!(formatter, "fleet node is {}", state.as_str())
            }
            Self::LeaseAlreadyExists => formatter.write_str("fleet lease already exists"),
            Self::LeaseNotFound => formatter.write_str("fleet lease was not found"),
            Self::LeaseNotActive(state) => {
                write!(formatter, "fleet lease is already {}", state.as_str())
            }
            Self::LeaseExecutionMismatch => {
                formatter.write_str("fleet lease execution identity does not match")
            }
            Self::SupervisorNotFound => formatter.write_str("fleet supervisor was not found"),
            Self::SupervisorAlreadyRunning => {
                formatter.write_str("fleet supervisor is already running")
            }
            Self::SupervisorNotStale => {
                formatter.write_str("fleet supervisor heartbeat is not stale enough to restart")
            }
            Self::SupervisorNotAcceptingWork(state) => {
                write!(formatter, "fleet supervisor is {}", state.as_str())
            }
            Self::SupervisorProcessMismatch => {
                formatter.write_str("fleet supervisor belongs to another process")
            }
            Self::ActiveLeasesPresent => {
                formatter.write_str("fleet supervisor still has active leases")
            }
            Self::InvalidSupervisorTransition { state, action } => {
                write!(
                    formatter,
                    "cannot {action} a {} fleet supervisor",
                    state.as_str()
                )
            }
            Self::InvalidSupervisorStaleness => {
                formatter.write_str("supervisor staleness window is invalid")
            }
            Self::QuiescenceHeld => formatter.write_str("fleet quiescence is already held"),
            Self::QuiescenceActive => formatter.write_str("fleet quiescence blocks new work"),
            Self::InvalidLeaseDuration => formatter.write_str("fleet lease duration is invalid"),
            Self::FleetNodeLimitExceeded => formatter.write_str("fleet node limit was exceeded"),
            Self::FleetLeaseLimitExceeded => formatter.write_str("fleet lease limit was exceeded"),
            Self::CorruptRecord => formatter.write_str("fleet database contains an invalid record"),
            Self::LockPoisoned => formatter.write_str("fleet database lock is unavailable"),
        }
    }
}

impl std::error::Error for FleetError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            Self::Serialization(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for FleetError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

impl From<serde_json::Error> for FleetError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}

pub struct FleetEngine {
    connection: Arc<Mutex<Connection>>,
}

impl FleetEngine {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, FleetError> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent).map_err(|_| {
                FleetError::Database(rusqlite::Error::InvalidPath(parent.to_path_buf()))
            })?;
        }
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        let journal_mode =
            connection.query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            connection.execute_batch("PRAGMA journal_mode = WAL;")?;
        }
        connection.pragma_update(None, "user_version", FLEET_SCHEMA_VERSION)?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS fleet_nodes (
                 id TEXT PRIMARY KEY,
                 implementation_version TEXT NOT NULL,
                 worker_class TEXT NOT NULL,
                 capabilities_json TEXT NOT NULL,
                 state TEXT NOT NULL,
                 registered_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS fleet_leases (
                 id TEXT PRIMARY KEY,
                 node_id TEXT NOT NULL,
                 execution_id TEXT NOT NULL,
                 budget_json TEXT NOT NULL,
                 issued_at INTEGER NOT NULL,
                 expires_at INTEGER NOT NULL,
                 state TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS fleet_leases_node_idx
                 ON fleet_leases(node_id, state);
             CREATE TABLE IF NOT EXISTS fleet_supervisors (
                 node_id TEXT PRIMARY KEY,
                 state TEXT NOT NULL CHECK (state IN ('stopped', 'running', 'draining', 'recovering')),
                 generation INTEGER NOT NULL CHECK (generation > 0),
                 process_id INTEGER,
                 reason TEXT,
                 updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS fleet_quiescence (
                 id INTEGER PRIMARY KEY CHECK (id = 1),
                 owner TEXT NOT NULL,
                 acquired_at INTEGER NOT NULL,
                 expires_at INTEGER NOT NULL
             );",
        )?;
        let has_process_id = connection
            .query_row(
                "SELECT 1 FROM pragma_table_info('fleet_supervisors') WHERE name = 'process_id'",
                [],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !has_process_id {
            connection.execute(
                "ALTER TABLE fleet_supervisors ADD COLUMN process_id INTEGER",
                [],
            )?;
        }
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    pub fn register_node(&self, node: &FleetNode) -> Result<FleetNode, FleetError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let count = transaction.query_row("SELECT COUNT(*) FROM fleet_nodes", [], |row| {
            row.get::<_, i64>(0)
        })?;
        if usize::try_from(count).map_err(|_| FleetError::CorruptRecord)? >= MAX_FLEET_NODES {
            return Err(FleetError::FleetNodeLimitExceeded);
        }
        let result = transaction.execute(
            "INSERT INTO fleet_nodes
             (id, implementation_version, worker_class, capabilities_json, state, registered_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                node.id,
                node.implementation_version,
                node.worker_class,
                serde_json::to_string(&node.capabilities)?,
                node.state.as_str(),
                to_i64(node.registered_at)?,
            ],
        );
        match result {
            Ok(_) => {
                transaction.commit()?;
                Ok(node.clone())
            }
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(FleetError::NodeAlreadyRegistered)
            }
            Err(error) => Err(FleetError::Database(error)),
        }
    }

    pub fn list_nodes(&self) -> Result<Vec<FleetNode>, FleetError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id, implementation_version, worker_class, capabilities_json,
                    state, registered_at
             FROM fleet_nodes ORDER BY id ASC",
        )?;
        let rows = statement.query_map([], decode_node)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(FleetError::Database)
    }

    pub fn dispatch_node(&self, capability: &str) -> Result<FleetNode, FleetError> {
        let supervisors = self.list_supervisors()?;
        let node = self.list_nodes()?.into_iter().find(|node| {
            let supervisor_allows_work = supervisors
                .iter()
                .find(|supervisor| supervisor.node_id() == node.id())
                .is_none_or(|supervisor| supervisor.state() == FleetSupervisorState::Running);
            node.state == FleetNodeState::Ready
                && node.supports(capability)
                && supervisor_allows_work
        });
        node.ok_or(FleetError::NodeNotFound)
    }

    pub fn list_supervisors(&self) -> Result<Vec<FleetSupervisor>, FleetError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT node_id, state, generation, process_id, reason, updated_at
             FROM fleet_supervisors ORDER BY node_id ASC",
        )?;
        let rows = statement.query_map([], decode_supervisor)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(FleetError::Database)
    }

    pub fn acquire_quiescence(
        &self,
        owner: impl Into<String>,
        now: u64,
        duration_seconds: u64,
    ) -> Result<FleetQuiescenceGuard, FleetError> {
        if duration_seconds == 0 {
            return Err(FleetError::InvalidLeaseDuration);
        }
        let owner = validate_text("quiescence owner", owner.into(), 256)?;
        let expires_at = now
            .checked_add(duration_seconds)
            .ok_or(FleetError::InvalidLeaseDuration)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "UPDATE fleet_leases SET state = 'expired'
             WHERE state = 'active' AND expires_at <= ?1",
            params![to_i64(now)?],
        )?;
        transaction.execute(
            "DELETE FROM fleet_quiescence WHERE expires_at <= ?1",
            params![to_i64(now)?],
        )?;
        let held = transaction
            .query_row(
                "SELECT 1 FROM fleet_quiescence WHERE id = 1 AND expires_at > ?1",
                params![to_i64(now)?],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if held.is_some() {
            return Err(FleetError::QuiescenceHeld);
        }
        let active = transaction.query_row(
            "SELECT COUNT(*) FROM fleet_leases WHERE state = 'active'",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        if active > 0 {
            return Err(FleetError::ActiveLeasesPresent);
        }
        transaction.execute(
            "INSERT INTO fleet_quiescence (id, owner, acquired_at, expires_at)
             VALUES (1, ?1, ?2, ?3)",
            params![owner, to_i64(now)?, to_i64(expires_at)?],
        )?;
        transaction.commit()?;
        Ok(FleetQuiescenceGuard {
            connection: Arc::clone(&self.connection),
            owner,
        })
    }

    pub fn heartbeat_supervisor(
        &self,
        node_id: &str,
        now: u64,
    ) -> Result<FleetSupervisor, FleetError> {
        self.heartbeat_supervisor_with_process(node_id, None, now)
    }

    pub fn heartbeat_supervisor_for_process(
        &self,
        node_id: &str,
        process_id: u32,
        now: u64,
    ) -> Result<FleetSupervisor, FleetError> {
        self.heartbeat_supervisor_with_process(node_id, Some(process_id), now)
    }

    fn heartbeat_supervisor_with_process(
        &self,
        node_id: &str,
        process_id: Option<u32>,
        now: u64,
    ) -> Result<FleetSupervisor, FleetError> {
        let node_id = validate_text("node ID", node_id.to_owned(), 256)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current =
            load_supervisor(&transaction, &node_id)?.ok_or(FleetError::SupervisorNotFound)?;
        if current.state != FleetSupervisorState::Running {
            return Err(FleetError::InvalidSupervisorTransition {
                state: current.state,
                action: "heartbeat",
            });
        }
        if let Some(process_id) = process_id
            && current.process_id != Some(process_id)
        {
            return Err(FleetError::SupervisorProcessMismatch);
        }
        let supervisor = FleetSupervisor {
            reason: Some("worker_heartbeat".to_owned()),
            updated_at: now,
            ..current
        };
        save_supervisor(&transaction, &supervisor)?;
        transaction.commit()?;
        Ok(supervisor)
    }

    pub fn reconcile_supervisor(
        &self,
        node_id: &str,
        now: u64,
        stale_after: u64,
    ) -> Result<FleetSupervisor, FleetError> {
        if stale_after == 0 {
            return Err(FleetError::InvalidSupervisorStaleness);
        }
        let node_id = validate_text("node ID", node_id.to_owned(), 256)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current =
            load_supervisor(&transaction, &node_id)?.ok_or(FleetError::SupervisorNotFound)?;
        if current.state != FleetSupervisorState::Running
            || now.saturating_sub(current.updated_at) <= stale_after
        {
            transaction.commit()?;
            return Ok(current);
        }
        transaction.execute(
            "UPDATE fleet_leases SET state = ?1
             WHERE node_id = ?2 AND state = ?3 AND expires_at <= ?4",
            params![
                FleetLeaseState::Expired.as_str(),
                node_id,
                FleetLeaseState::Active.as_str(),
                to_i64(now)?,
            ],
        )?;
        let supervisor = FleetSupervisor {
            state: FleetSupervisorState::Recovering,
            reason: Some("heartbeat_expired".to_owned()),
            updated_at: now,
            ..current
        };
        save_supervisor(&transaction, &supervisor)?;
        transaction.commit()?;
        Ok(supervisor)
    }

    pub fn reap_stale_supervisors(
        &self,
        now: u64,
        stale_after: u64,
    ) -> Result<Vec<FleetSupervisor>, FleetError> {
        if stale_after == 0 {
            return Err(FleetError::InvalidSupervisorStaleness);
        }
        self.list_supervisors()?
            .into_iter()
            .filter(|supervisor| {
                supervisor.state == FleetSupervisorState::Running
                    && now.saturating_sub(supervisor.updated_at) > stale_after
            })
            .map(|supervisor| self.reconcile_supervisor(&supervisor.node_id, now, stale_after))
            .collect()
    }

    pub fn restart_supervisor_for_process(
        &self,
        node_id: &str,
        process_id: u32,
        now: u64,
        stale_after: u64,
    ) -> Result<FleetSupervisor, FleetError> {
        if stale_after == 0 {
            return Err(FleetError::InvalidSupervisorStaleness);
        }
        let node_id = validate_text("node ID", node_id.to_owned(), 256)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let node_state = transaction
            .query_row(
                "SELECT state FROM fleet_nodes WHERE id = ?1",
                params![node_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or(FleetError::NodeNotFound)?;
        if decode_node_state(&node_state)? != FleetNodeState::Ready {
            return Err(FleetError::NodeUnavailable(decode_node_state(&node_state)?));
        }
        let current =
            load_supervisor(&transaction, &node_id)?.ok_or(FleetError::SupervisorNotFound)?;
        let current = match current.state {
            FleetSupervisorState::Running => {
                if now.saturating_sub(current.updated_at) <= stale_after {
                    return Err(FleetError::SupervisorNotStale);
                }
                transaction.execute(
                    "UPDATE fleet_leases SET state = ?1
                     WHERE node_id = ?2 AND state = ?3 AND expires_at <= ?4",
                    params![
                        FleetLeaseState::Expired.as_str(),
                        node_id,
                        FleetLeaseState::Active.as_str(),
                        to_i64(now)?,
                    ],
                )?;
                FleetSupervisor {
                    state: FleetSupervisorState::Recovering,
                    reason: Some("heartbeat_expired".to_owned()),
                    updated_at: now,
                    ..current
                }
            }
            FleetSupervisorState::Stopped | FleetSupervisorState::Recovering => current,
            state => {
                return Err(FleetError::InvalidSupervisorTransition {
                    state,
                    action: "restart",
                });
            }
        };
        save_supervisor(&transaction, &current)?;
        if active_lease_count(&transaction, &node_id)? > 0 {
            return Err(FleetError::ActiveLeasesPresent);
        }
        let supervisor = FleetSupervisor {
            node_id,
            state: FleetSupervisorState::Running,
            generation: current
                .generation
                .checked_add(1)
                .ok_or(FleetError::CorruptRecord)?,
            process_id: Some(process_id),
            reason: Some("operator_restart".to_owned()),
            updated_at: now,
        };
        save_supervisor(&transaction, &supervisor)?;
        transaction.commit()?;
        Ok(supervisor)
    }

    pub fn start_supervisor(&self, node_id: &str, now: u64) -> Result<FleetSupervisor, FleetError> {
        self.start_supervisor_with_process(node_id, None, now)
    }

    pub fn start_supervisor_for_process(
        &self,
        node_id: &str,
        process_id: u32,
        now: u64,
    ) -> Result<FleetSupervisor, FleetError> {
        self.start_supervisor_with_process(node_id, Some(process_id), now)
    }

    fn start_supervisor_with_process(
        &self,
        node_id: &str,
        process_id: Option<u32>,
        now: u64,
    ) -> Result<FleetSupervisor, FleetError> {
        let node_id = validate_text("node ID", node_id.to_owned(), 256)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let node_state = transaction
            .query_row(
                "SELECT state FROM fleet_nodes WHERE id = ?1",
                params![node_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or(FleetError::NodeNotFound)?;
        let node_state = decode_node_state(&node_state)?;
        if node_state != FleetNodeState::Ready {
            return Err(FleetError::NodeUnavailable(node_state));
        }
        let current = load_supervisor(&transaction, &node_id)?;
        let supervisor = match current {
            None => FleetSupervisor {
                node_id,
                state: FleetSupervisorState::Running,
                generation: 1,
                process_id,
                reason: None,
                updated_at: now,
            },
            Some(current) => match current.state {
                FleetSupervisorState::Running => {
                    return Err(FleetError::SupervisorAlreadyRunning);
                }
                FleetSupervisorState::Draining => {
                    return Err(FleetError::InvalidSupervisorTransition {
                        state: current.state,
                        action: "start",
                    });
                }
                FleetSupervisorState::Stopped | FleetSupervisorState::Recovering => {
                    if active_lease_count(&transaction, &current.node_id)? > 0 {
                        return Err(FleetError::ActiveLeasesPresent);
                    }
                    FleetSupervisor {
                        node_id: current.node_id,
                        state: FleetSupervisorState::Running,
                        generation: current
                            .generation
                            .checked_add(1)
                            .ok_or(FleetError::CorruptRecord)?,
                        process_id,
                        reason: None,
                        updated_at: now,
                    }
                }
            },
        };
        save_supervisor(&transaction, &supervisor)?;
        transaction.commit()?;
        Ok(supervisor)
    }

    pub fn drain_supervisor(&self, node_id: &str, now: u64) -> Result<FleetSupervisor, FleetError> {
        let node_id = validate_text("node ID", node_id.to_owned(), 256)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current =
            load_supervisor(&transaction, &node_id)?.ok_or(FleetError::SupervisorNotFound)?;
        if current.state != FleetSupervisorState::Running {
            return Err(FleetError::InvalidSupervisorTransition {
                state: current.state,
                action: "drain",
            });
        }
        let supervisor = FleetSupervisor {
            state: FleetSupervisorState::Draining,
            reason: Some("operator_draining".to_owned()),
            updated_at: now,
            ..current
        };
        save_supervisor(&transaction, &supervisor)?;
        transaction.commit()?;
        Ok(supervisor)
    }

    pub fn stop_supervisor(&self, node_id: &str, now: u64) -> Result<FleetSupervisor, FleetError> {
        let node_id = validate_text("node ID", node_id.to_owned(), 256)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current =
            load_supervisor(&transaction, &node_id)?.ok_or(FleetError::SupervisorNotFound)?;
        if !matches!(
            current.state,
            FleetSupervisorState::Draining | FleetSupervisorState::Recovering
        ) {
            return Err(FleetError::InvalidSupervisorTransition {
                state: current.state,
                action: "stop",
            });
        }
        if active_lease_count(&transaction, &node_id)? > 0 {
            return Err(FleetError::ActiveLeasesPresent);
        }
        let supervisor = FleetSupervisor {
            state: FleetSupervisorState::Stopped,
            reason: Some("operator_stopped".to_owned()),
            updated_at: now,
            ..current
        };
        save_supervisor(&transaction, &supervisor)?;
        transaction.commit()?;
        Ok(supervisor)
    }

    pub fn shutdown_supervisor_for_process(
        &self,
        node_id: &str,
        process_id: u32,
        lease_id: &str,
        now: u64,
    ) -> Result<FleetSupervisor, FleetError> {
        let node_id = validate_text("node ID", node_id.to_owned(), 256)?;
        let lease_id = validate_text("lease ID", lease_id.to_owned(), 256)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current =
            load_supervisor(&transaction, &node_id)?.ok_or(FleetError::SupervisorNotFound)?;
        if current.process_id != Some(process_id) {
            return Err(FleetError::SupervisorProcessMismatch);
        }
        let lease = transaction
            .query_row(
                "SELECT id, node_id, execution_id, budget_json, issued_at,
                        expires_at, state
                 FROM fleet_leases WHERE id = ?1",
                params![lease_id],
                decode_lease,
            )
            .optional()?
            .ok_or(FleetError::LeaseNotFound)?;
        if lease.node_id != node_id {
            return Err(FleetError::LeaseExecutionMismatch);
        }
        if lease.state == FleetLeaseState::Active {
            transaction.execute(
                "UPDATE fleet_leases SET state = ?1 WHERE id = ?2 AND state = ?3",
                params![
                    FleetLeaseState::Released.as_str(),
                    lease_id,
                    FleetLeaseState::Active.as_str(),
                ],
            )?;
        }
        if active_lease_count(&transaction, &node_id)? > 0 {
            return Err(FleetError::ActiveLeasesPresent);
        }
        if current.state == FleetSupervisorState::Stopped {
            transaction.commit()?;
            return Ok(current);
        }
        let supervisor = FleetSupervisor {
            state: FleetSupervisorState::Stopped,
            reason: Some("process_shutdown".to_owned()),
            updated_at: now,
            ..current
        };
        save_supervisor(&transaction, &supervisor)?;
        transaction.commit()?;
        Ok(supervisor)
    }

    pub fn recover_supervisor(
        &self,
        node_id: &str,
        now: u64,
    ) -> Result<FleetSupervisor, FleetError> {
        let node_id = validate_text("node ID", node_id.to_owned(), 256)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current =
            load_supervisor(&transaction, &node_id)?.ok_or(FleetError::SupervisorNotFound)?;
        transaction.execute(
            "UPDATE fleet_leases SET state = ?1
             WHERE node_id = ?2 AND state = ?3 AND expires_at <= ?4",
            params![
                FleetLeaseState::Expired.as_str(),
                node_id,
                FleetLeaseState::Active.as_str(),
                to_i64(now)?,
            ],
        )?;
        let supervisor = FleetSupervisor {
            state: FleetSupervisorState::Recovering,
            reason: Some("operator_recovery".to_owned()),
            updated_at: now,
            ..current
        };
        save_supervisor(&transaction, &supervisor)?;
        transaction.commit()?;
        Ok(supervisor)
    }

    pub fn acquire_lease(
        &self,
        lease_id: impl Into<String>,
        node_id: impl Into<String>,
        execution_id: impl Into<String>,
        budget: FleetBudget,
        now: u64,
        duration_seconds: u64,
    ) -> Result<FleetLease, FleetError> {
        if duration_seconds == 0 {
            return Err(FleetError::InvalidLeaseDuration);
        }
        let lease_id = validate_text("lease ID", lease_id.into(), 256)?;
        let node_id = validate_text("node ID", node_id.into(), 256)?;
        let execution_id = validate_text("execution ID", execution_id.into(), 256)?;
        let expires_at = now
            .checked_add(duration_seconds)
            .ok_or(FleetError::InvalidLeaseDuration)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_work_permitted(&transaction, now)?;
        let count = transaction.query_row("SELECT COUNT(*) FROM fleet_leases", [], |row| {
            row.get::<_, i64>(0)
        })?;
        if usize::try_from(count).map_err(|_| FleetError::CorruptRecord)? >= MAX_FLEET_LEASES {
            return Err(FleetError::FleetLeaseLimitExceeded);
        }
        let node = transaction
            .query_row(
                "SELECT state FROM fleet_nodes WHERE id = ?1",
                params![node_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(state) = node else {
            return Err(FleetError::NodeNotFound);
        };
        let state = decode_node_state(&state)?;
        if state != FleetNodeState::Ready {
            return Err(FleetError::NodeUnavailable(state));
        }
        if let Some(supervisor_state) = transaction
            .query_row(
                "SELECT state FROM fleet_supervisors WHERE node_id = ?1",
                params![node_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|state| decode_supervisor_state(&state))
            .transpose()?
            && supervisor_state != FleetSupervisorState::Running
        {
            return Err(FleetError::SupervisorNotAcceptingWork(supervisor_state));
        }
        let result = transaction.execute(
            "INSERT INTO fleet_leases
             (id, node_id, execution_id, budget_json, issued_at, expires_at, state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active')",
            params![
                lease_id,
                node_id,
                execution_id,
                serde_json::to_string(&budget)?,
                to_i64(now)?,
                to_i64(expires_at)?,
            ],
        );
        match result {
            Ok(_) => {
                transaction.commit()?;
                Ok(FleetLease {
                    id: lease_id,
                    node_id,
                    execution_id,
                    budget,
                    issued_at: now,
                    expires_at,
                    state: FleetLeaseState::Active,
                })
            }
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(FleetError::LeaseAlreadyExists)
            }
            Err(error) => Err(FleetError::Database(error)),
        }
    }

    pub fn renew_lease(
        &self,
        lease_id: &str,
        execution_id: &str,
        now: u64,
        duration_seconds: u64,
    ) -> Result<FleetLease, FleetError> {
        if duration_seconds == 0 {
            return Err(FleetError::InvalidLeaseDuration);
        }
        let execution_id = validate_text("execution ID", execution_id.to_owned(), 256)?;
        let expires_at = now
            .checked_add(duration_seconds)
            .ok_or(FleetError::InvalidLeaseDuration)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_work_permitted(&transaction, now)?;
        let lease = transaction
            .query_row(
                "SELECT state, execution_id, expires_at
                 FROM fleet_leases WHERE id = ?1",
                params![lease_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        decode_u64(row.get(2)?)?,
                    ))
                },
            )
            .optional()?;
        let Some((state, stored_execution_id, current_expires_at)) = lease else {
            return Err(FleetError::LeaseNotFound);
        };
        let state = decode_lease_state(&state)?;
        if state != FleetLeaseState::Active {
            return Err(FleetError::LeaseNotActive(state));
        }
        if stored_execution_id != execution_id {
            return Err(FleetError::LeaseExecutionMismatch);
        }
        if current_expires_at <= now {
            transaction.execute(
                "UPDATE fleet_leases SET state = 'expired'
                 WHERE id = ?1 AND state = 'active'",
                params![lease_id],
            )?;
            transaction.commit()?;
            return Err(FleetError::LeaseNotActive(FleetLeaseState::Expired));
        }
        transaction.execute(
            "UPDATE fleet_leases SET expires_at = ?1
             WHERE id = ?2 AND state = 'active' AND execution_id = ?3",
            params![to_i64(expires_at)?, lease_id, execution_id],
        )?;
        let renewed = transaction.query_row(
            "SELECT id, node_id, execution_id, budget_json, issued_at,
                    expires_at, state
             FROM fleet_leases WHERE id = ?1",
            params![lease_id],
            decode_lease,
        )?;
        transaction.commit()?;
        Ok(renewed)
    }

    pub fn list_leases(&self) -> Result<Vec<FleetLease>, FleetError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id, node_id, execution_id, budget_json, issued_at,
                    expires_at, state
             FROM fleet_leases ORDER BY id ASC",
        )?;
        let rows = statement.query_map([], decode_lease)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(FleetError::Database)
    }

    pub fn release_lease(&self, lease_id: &str) -> Result<FleetLease, FleetError> {
        self.transition_lease(lease_id, FleetLeaseState::Released)
    }

    pub fn expire_leases(&self, now: u64) -> Result<usize, FleetError> {
        let connection = self.lock()?;
        let changed = connection.execute(
            "UPDATE fleet_leases SET state = 'expired'
             WHERE state = 'active' AND expires_at <= ?1",
            params![to_i64(now)?],
        )?;
        Ok(changed)
    }

    pub fn quarantine_node(&self, node_id: &str) -> Result<(), FleetError> {
        self.transition_node(
            node_id,
            FleetNodeState::Quarantined,
            FleetLeaseState::Revoked,
        )
    }

    pub fn revoke_node(&self, node_id: &str) -> Result<(), FleetError> {
        self.transition_node(node_id, FleetNodeState::Revoked, FleetLeaseState::Revoked)
    }

    pub fn kill_node(&self, node_id: &str) -> Result<(), FleetError> {
        self.transition_node(node_id, FleetNodeState::Killed, FleetLeaseState::Killed)
    }

    fn transition_node(
        &self,
        node_id: &str,
        state: FleetNodeState,
        lease_state: FleetLeaseState,
    ) -> Result<(), FleetError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE fleet_nodes SET state = ?1 WHERE id = ?2",
            params![state.as_str(), node_id],
        )?;
        if changed == 0 {
            return Err(FleetError::NodeNotFound);
        }
        transaction.execute(
            "UPDATE fleet_leases SET state = ?1
             WHERE node_id = ?2 AND state = 'active'",
            params![lease_state.as_str(), node_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    fn transition_lease(
        &self,
        lease_id: &str,
        state: FleetLeaseState,
    ) -> Result<FleetLease, FleetError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE fleet_leases SET state = ?1
             WHERE id = ?2 AND state = 'active'",
            params![state.as_str(), lease_id],
        )?;
        if changed == 0 {
            let state = transaction
                .query_row(
                    "SELECT state FROM fleet_leases WHERE id = ?1",
                    params![lease_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            if let Some(state) = state {
                return Err(FleetError::LeaseNotActive(decode_lease_state(&state)?));
            }
            return Err(FleetError::LeaseNotFound);
        }
        let lease = transaction.query_row(
            "SELECT id, node_id, execution_id, budget_json, issued_at,
                    expires_at, state
             FROM fleet_leases WHERE id = ?1",
            params![lease_id],
            decode_lease,
        )?;
        transaction.commit()?;
        Ok(lease)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, FleetError> {
        self.connection.lock().map_err(|_| FleetError::LockPoisoned)
    }
}

fn load_supervisor(
    connection: &Connection,
    node_id: &str,
) -> Result<Option<FleetSupervisor>, FleetError> {
    connection
        .query_row(
            "SELECT node_id, state, generation, process_id, reason, updated_at
             FROM fleet_supervisors WHERE node_id = ?1",
            params![node_id],
            decode_supervisor,
        )
        .optional()
        .map_err(FleetError::Database)
}

fn save_supervisor(
    connection: &Connection,
    supervisor: &FleetSupervisor,
) -> Result<(), FleetError> {
    connection.execute(
        "INSERT INTO fleet_supervisors (node_id, state, generation, process_id, reason, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(node_id) DO UPDATE SET state = excluded.state,
             generation = excluded.generation, process_id = excluded.process_id,
             reason = excluded.reason, updated_at = excluded.updated_at",
        params![
            supervisor.node_id,
            supervisor.state.as_str(),
            to_i64(supervisor.generation)?,
            supervisor
                .process_id
                .map(u64::from)
                .map(to_i64)
                .transpose()?,
            supervisor.reason,
            to_i64(supervisor.updated_at)?,
        ],
    )?;
    Ok(())
}

fn ensure_work_permitted(connection: &Connection, now: u64) -> Result<(), FleetError> {
    connection.execute(
        "DELETE FROM fleet_quiescence WHERE expires_at <= ?1",
        params![to_i64(now)?],
    )?;
    let held = connection
        .query_row(
            "SELECT 1 FROM fleet_quiescence WHERE id = 1 AND expires_at > ?1",
            params![to_i64(now)?],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    if held.is_some() {
        Err(FleetError::QuiescenceActive)
    } else {
        Ok(())
    }
}

fn active_lease_count(connection: &Connection, node_id: &str) -> Result<u64, FleetError> {
    let count = connection.query_row(
        "SELECT COUNT(*) FROM fleet_leases WHERE node_id = ?1 AND state = ?2",
        params![node_id, FleetLeaseState::Active.as_str()],
        |row| row.get::<_, i64>(0),
    )?;
    u64::try_from(count).map_err(|_| FleetError::CorruptRecord)
}

fn decode_supervisor(row: &rusqlite::Row<'_>) -> rusqlite::Result<FleetSupervisor> {
    Ok(FleetSupervisor {
        node_id: row.get(0)?,
        state: decode_supervisor_state(&row.get::<_, String>(1)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        generation: decode_u64(row.get(2)?)?,
        process_id: row.get::<_, Option<i64>>(3)?.map(decode_u32).transpose()?,
        reason: row.get(4)?,
        updated_at: decode_u64(row.get(5)?)?,
    })
}

fn decode_supervisor_state(value: &str) -> Result<FleetSupervisorState, FleetError> {
    match value {
        "stopped" => Ok(FleetSupervisorState::Stopped),
        "running" => Ok(FleetSupervisorState::Running),
        "draining" => Ok(FleetSupervisorState::Draining),
        "recovering" => Ok(FleetSupervisorState::Recovering),
        _ => Err(FleetError::CorruptRecord),
    }
}

fn decode_node(row: &rusqlite::Row<'_>) -> rusqlite::Result<FleetNode> {
    let capabilities = serde_json::from_str::<Vec<String>>(&row.get::<_, String>(3)?)
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    Ok(FleetNode {
        id: row.get(0)?,
        implementation_version: row.get(1)?,
        worker_class: row.get(2)?,
        capabilities,
        state: decode_node_state(&row.get::<_, String>(4)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        registered_at: decode_u64(row.get(5)?)?,
    })
}

fn decode_lease(row: &rusqlite::Row<'_>) -> rusqlite::Result<FleetLease> {
    Ok(FleetLease {
        id: row.get(0)?,
        node_id: row.get(1)?,
        execution_id: row.get(2)?,
        budget: serde_json::from_str(&row.get::<_, String>(3)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        issued_at: decode_u64(row.get(4)?)?,
        expires_at: decode_u64(row.get(5)?)?,
        state: decode_lease_state(&row.get::<_, String>(6)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
    })
}

fn decode_node_state(value: &str) -> Result<FleetNodeState, FleetError> {
    match value {
        "ready" => Ok(FleetNodeState::Ready),
        "quarantined" => Ok(FleetNodeState::Quarantined),
        "revoked" => Ok(FleetNodeState::Revoked),
        "killed" => Ok(FleetNodeState::Killed),
        _ => Err(FleetError::CorruptRecord),
    }
}

fn decode_lease_state(value: &str) -> Result<FleetLeaseState, FleetError> {
    match value {
        "active" => Ok(FleetLeaseState::Active),
        "released" => Ok(FleetLeaseState::Released),
        "expired" => Ok(FleetLeaseState::Expired),
        "revoked" => Ok(FleetLeaseState::Revoked),
        "killed" => Ok(FleetLeaseState::Killed),
        _ => Err(FleetError::CorruptRecord),
    }
}

fn validate_text(
    field: &'static str,
    value: String,
    max_bytes: usize,
) -> Result<String, FleetError> {
    let value = value.trim().to_owned();
    if value.is_empty() || value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(FleetError::InvalidField(field));
    }
    Ok(value)
}

fn to_i64(value: u64) -> Result<i64, FleetError> {
    i64::try_from(value).map_err(|_| FleetError::CorruptRecord)
}

fn decode_u64(value: i64) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|_| rusqlite::Error::InvalidQuery)
}

fn decode_u32(value: i64) -> rusqlite::Result<u32> {
    u32::try_from(value).map_err(|_| rusqlite::Error::InvalidQuery)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, capabilities: &[&str]) -> FleetNode {
        FleetNode::new(
            id,
            "2.0.0-beta.1",
            "local",
            capabilities.iter().map(|value| (*value).to_owned()),
            10,
        )
        .unwrap()
    }

    fn engine(name: &str) -> FleetEngine {
        FleetEngine::open(
            crate::test_support::new_temp_dir(name)
                .unwrap()
                .join("fleet.sqlite3"),
        )
        .unwrap()
    }

    #[test]
    fn database_uses_wal_and_a_bounded_busy_wait() {
        let fleet = engine("pandora-fleet-concurrency");
        let connection = fleet.connection.lock().unwrap();
        let journal_mode = connection
            .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
            .unwrap();
        let busy_timeout = connection
            .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, u64>(0))
            .unwrap();

        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
        assert_eq!(busy_timeout, 5_000);
    }

    #[test]
    fn registration_and_dispatch_are_durable_and_deterministic() {
        let root = crate::test_support::new_temp_dir("pandora-fleet-durable").unwrap();
        let path = root.join("fleet.sqlite3");
        let first = FleetEngine::open(&path).unwrap();
        first.register_node(&node("node-b", &["coding"])).unwrap();
        first
            .register_node(&node("node-a", &["coding", "review"]))
            .unwrap();
        assert_eq!(first.dispatch_node("coding").unwrap().id(), "node-a");
        drop(first);

        let second = FleetEngine::open(&path).unwrap();
        assert_eq!(second.list_nodes().unwrap().len(), 2);
        assert!(second.dispatch_node("review").unwrap().supports("review"));
    }

    #[test]
    fn leases_are_bounded_and_expire_atomically() {
        let fleet = engine("pandora-fleet-lease");
        fleet.register_node(&node("node-a", &["coding"])).unwrap();
        let lease = fleet
            .acquire_lease(
                "lease-a",
                "node-a",
                "execution-a",
                FleetBudget::new(100, 4, 30, 10_000),
                10,
                20,
            )
            .unwrap();
        assert_eq!(lease.state(), FleetLeaseState::Active);
        assert_eq!(fleet.expire_leases(29).unwrap(), 0);
        assert_eq!(fleet.expire_leases(30).unwrap(), 1);
        assert_eq!(
            fleet.list_leases().unwrap()[0].state(),
            FleetLeaseState::Expired
        );
    }

    #[test]
    fn active_lease_renews_only_for_its_execution_and_cannot_be_resurrected() {
        let fleet = engine("pandora-fleet-renew");
        fleet.register_node(&node("node-a", &["coding"])).unwrap();
        let lease = fleet
            .acquire_lease(
                "lease-a",
                "node-a",
                "execution-a",
                FleetBudget::new(100, 4, 30, 10_000),
                10,
                20,
            )
            .unwrap();
        assert_eq!(lease.expires_at(), 30);
        assert!(matches!(
            fleet.renew_lease("lease-a", "execution-b", 20, 60),
            Err(FleetError::LeaseExecutionMismatch)
        ));
        let renewed = fleet.renew_lease("lease-a", "execution-a", 20, 60).unwrap();
        assert_eq!(renewed.expires_at(), 80);
        assert_eq!(fleet.expire_leases(80).unwrap(), 1);
        assert!(matches!(
            fleet.renew_lease("lease-a", "execution-a", 80, 60),
            Err(FleetError::LeaseNotActive(FleetLeaseState::Expired))
        ));
        assert_eq!(
            fleet.list_leases().unwrap()[0].state(),
            FleetLeaseState::Expired
        );
    }

    #[test]
    fn quarantine_revoke_and_kill_stop_new_leases_and_revoke_active_work() {
        let fleet = engine("pandora-fleet-controls");
        fleet.register_node(&node("node-a", &["coding"])).unwrap();
        fleet
            .acquire_lease(
                "lease-a",
                "node-a",
                "execution-a",
                FleetBudget::new(1, 1, 1, 1),
                1,
                10,
            )
            .unwrap();
        fleet.quarantine_node("node-a").unwrap();
        assert!(matches!(
            fleet.acquire_lease(
                "lease-b",
                "node-a",
                "execution-b",
                FleetBudget::new(1, 1, 1, 1),
                1,
                10,
            ),
            Err(FleetError::NodeUnavailable(FleetNodeState::Quarantined))
        ));
        assert_eq!(
            fleet.list_leases().unwrap()[0].state(),
            FleetLeaseState::Revoked
        );
        fleet.revoke_node("node-a").unwrap();
        fleet.kill_node("node-a").unwrap();
        assert_eq!(
            fleet.list_nodes().unwrap()[0].state(),
            FleetNodeState::Killed
        );
    }

    #[test]
    fn supervisor_lifecycle_gates_new_leases_and_requires_drain_before_stop() {
        let fleet = engine("pandora-fleet-supervisor-lifecycle");
        fleet.register_node(&node("node-a", &["coding"])).unwrap();
        let started = fleet.start_supervisor("node-a", 1).unwrap();
        assert_eq!(started.state(), FleetSupervisorState::Running);
        assert_eq!(started.generation(), 1);
        fleet
            .acquire_lease(
                "lease-a",
                "node-a",
                "execution-a",
                FleetBudget::new(1, 1, 10, 1),
                1,
                10,
            )
            .unwrap();
        let draining = fleet.drain_supervisor("node-a", 2).unwrap();
        assert!(matches!(
            fleet.dispatch_node("coding"),
            Err(FleetError::NodeNotFound)
        ));
        assert_eq!(draining.state(), FleetSupervisorState::Draining);
        assert!(matches!(
            fleet.acquire_lease(
                "lease-b",
                "node-a",
                "execution-b",
                FleetBudget::new(1, 1, 10, 1),
                2,
                10,
            ),
            Err(FleetError::SupervisorNotAcceptingWork(
                FleetSupervisorState::Draining
            ))
        ));
        assert!(matches!(
            fleet.stop_supervisor("node-a", 2),
            Err(FleetError::ActiveLeasesPresent)
        ));
        fleet.release_lease("lease-a").unwrap();
        let stopped = fleet.stop_supervisor("node-a", 3).unwrap();
        assert_eq!(stopped.state(), FleetSupervisorState::Stopped);
        assert!(matches!(
            fleet.acquire_lease(
                "lease-c",
                "node-a",
                "execution-c",
                FleetBudget::new(1, 1, 10, 1),
                3,
                10,
            ),
            Err(FleetError::SupervisorNotAcceptingWork(
                FleetSupervisorState::Stopped
            ))
        ));
        let restarted = fleet.start_supervisor("node-a", 4).unwrap();
        assert_eq!(restarted.state(), FleetSupervisorState::Running);
        assert_eq!(restarted.generation(), 2);
    }

    #[test]
    fn process_shutdown_releases_its_lease_and_stops_atomically_and_idempotently() {
        let fleet = engine("pandora-fleet-atomic-process-shutdown");
        fleet.register_node(&node("node-a", &["coding"])).unwrap();
        fleet.start_supervisor_for_process("node-a", 42, 1).unwrap();
        for (lease_id, execution_id) in [("lease-a", "execution-a"), ("lease-b", "execution-b")] {
            fleet
                .acquire_lease(
                    lease_id,
                    "node-a",
                    execution_id,
                    FleetBudget::new(1, 1, 10, 1),
                    1,
                    10,
                )
                .unwrap();
        }

        assert!(matches!(
            fleet.shutdown_supervisor_for_process("node-a", 42, "lease-a", 2),
            Err(FleetError::ActiveLeasesPresent)
        ));
        assert!(
            fleet
                .list_leases()
                .unwrap()
                .iter()
                .all(|lease| lease.state() == FleetLeaseState::Active)
        );
        assert!(matches!(
            fleet.shutdown_supervisor_for_process("node-a", 7, "lease-a", 2),
            Err(FleetError::SupervisorProcessMismatch)
        ));

        fleet.release_lease("lease-b").unwrap();
        let stopped = fleet
            .shutdown_supervisor_for_process("node-a", 42, "lease-a", 3)
            .unwrap();
        assert_eq!(stopped.state(), FleetSupervisorState::Stopped);
        assert_eq!(
            fleet
                .list_leases()
                .unwrap()
                .into_iter()
                .find(|lease| lease.id() == "lease-a")
                .unwrap()
                .state(),
            FleetLeaseState::Released
        );
        let replayed = fleet
            .shutdown_supervisor_for_process("node-a", 42, "lease-a", 4)
            .unwrap();
        assert_eq!(replayed.state(), FleetSupervisorState::Stopped);
    }

    #[test]
    fn supervisor_recovery_expires_stale_leases_before_restart() {
        let fleet = engine("pandora-fleet-supervisor-recovery");
        fleet.register_node(&node("node-a", &["coding"])).unwrap();
        fleet.start_supervisor("node-a", 1).unwrap();
        fleet
            .acquire_lease(
                "lease-a",
                "node-a",
                "execution-a",
                FleetBudget::new(1, 1, 10, 1),
                1,
                10,
            )
            .unwrap();
        fleet.drain_supervisor("node-a", 2).unwrap();
        assert_eq!(
            fleet.recover_supervisor("node-a", 5).unwrap().state(),
            FleetSupervisorState::Recovering
        );
        assert!(matches!(
            fleet.start_supervisor("node-a", 5),
            Err(FleetError::ActiveLeasesPresent)
        ));
        fleet.recover_supervisor("node-a", 11).unwrap();
        assert_eq!(
            fleet.list_leases().unwrap()[0].state(),
            FleetLeaseState::Expired
        );
        assert_eq!(
            fleet.start_supervisor("node-a", 12).unwrap().state(),
            FleetSupervisorState::Running
        );
    }

    #[test]
    fn quiescence_guard_blocks_cross_process_work_and_releases_on_drop() {
        let root = crate::test_support::new_temp_dir("pandora-fleet-quiescence").unwrap();
        let path = root.join("fleet.sqlite3");
        let first = FleetEngine::open(&path).unwrap();
        first.register_node(&node("node-a", &["coding"])).unwrap();
        let second = FleetEngine::open(&path).unwrap();
        let guard = first.acquire_quiescence("evolution-a", 10, 30).unwrap();
        assert_eq!(guard.owner(), "evolution-a");
        assert!(matches!(
            second.acquire_quiescence("evolution-b", 11, 30),
            Err(FleetError::QuiescenceHeld)
        ));
        assert!(matches!(
            second.acquire_lease(
                "lease-a",
                "node-a",
                "execution-a",
                FleetBudget::new(1, 1, 10, 1),
                11,
                10,
            ),
            Err(FleetError::QuiescenceActive)
        ));
        drop(guard);
        second
            .acquire_lease(
                "lease-a",
                "node-a",
                "execution-a",
                FleetBudget::new(1, 1, 10, 1),
                11,
                10,
            )
            .unwrap();
        assert!(matches!(
            second.acquire_quiescence("evolution-c", 12, 30),
            Err(FleetError::ActiveLeasesPresent)
        ));
        second.expire_leases(21).unwrap();
        let recovered = second.acquire_quiescence("evolution-c", 21, 30).unwrap();
        assert_eq!(recovered.owner(), "evolution-c");
    }

    #[test]
    fn supervisor_heartbeat_is_durable_and_reconcile_is_bounded() {
        let root = crate::test_support::new_temp_dir("pandora-fleet-heartbeat").unwrap();
        let path = root.join("fleet.sqlite3");
        let fleet = FleetEngine::open(&path).unwrap();
        fleet.register_node(&node("node-a", &["coding"])).unwrap();
        fleet
            .start_supervisor_for_process("node-a", 41, 10)
            .unwrap();
        assert_eq!(fleet.list_supervisors().unwrap()[0].process_id(), Some(41));
        assert!(matches!(
            fleet.heartbeat_supervisor_for_process("node-a", 42, 20),
            Err(FleetError::SupervisorProcessMismatch)
        ));
        fleet
            .acquire_lease(
                "lease-a",
                "node-a",
                "execution-a",
                FleetBudget::new(1, 1, 10, 1),
                10,
                40,
            )
            .unwrap();
        let heartbeat = fleet
            .heartbeat_supervisor_for_process("node-a", 41, 20)
            .unwrap();
        assert_eq!(heartbeat.updated_at(), 20);
        drop(fleet);

        let reopened = FleetEngine::open(&path).unwrap();
        assert_eq!(reopened.list_supervisors().unwrap()[0].updated_at(), 20);
        assert_eq!(
            reopened
                .reconcile_supervisor("node-a", 30, 10)
                .unwrap()
                .state(),
            FleetSupervisorState::Running
        );
        let recovering = reopened.reconcile_supervisor("node-a", 31, 10).unwrap();
        assert_eq!(recovering.state(), FleetSupervisorState::Recovering);
        assert_eq!(recovering.reason(), Some("heartbeat_expired"));
        assert_eq!(
            reopened.list_leases().unwrap()[0].state(),
            FleetLeaseState::Active
        );
        assert!(matches!(
            reopened.start_supervisor("node-a", 31),
            Err(FleetError::ActiveLeasesPresent)
        ));
        reopened.recover_supervisor("node-a", 51).unwrap();
        assert_eq!(
            reopened.list_leases().unwrap()[0].state(),
            FleetLeaseState::Expired
        );
        assert_eq!(
            reopened
                .start_supervisor("node-a", 52)
                .unwrap()
                .generation(),
            2
        );
        assert!(reopened.heartbeat_supervisor("node-a", 53).is_ok());
    }

    #[test]
    fn restart_handoff_requires_staleness_and_replaces_the_process_binding() {
        let fleet = engine("pandora-fleet-restart");
        fleet.register_node(&node("node-a", &["coding"])).unwrap();
        fleet.start_supervisor_for_process("node-a", 41, 1).unwrap();
        fleet
            .acquire_lease(
                "lease-a",
                "node-a",
                "execution-a",
                FleetBudget::new(1, 1, 10, 1),
                1,
                10,
            )
            .unwrap();

        let restarted = fleet
            .restart_supervisor_for_process("node-a", 42, 20, 10)
            .unwrap();
        assert_eq!(restarted.state(), FleetSupervisorState::Running);
        assert_eq!(restarted.generation(), 2);
        assert_eq!(restarted.process_id(), Some(42));
        assert_eq!(restarted.reason(), Some("operator_restart"));
        assert_eq!(
            fleet.list_leases().unwrap()[0].state(),
            FleetLeaseState::Expired
        );
        assert!(matches!(
            fleet.restart_supervisor_for_process("node-a", 43, 21, 10),
            Err(FleetError::SupervisorNotStale)
        ));
    }

    #[test]
    fn reap_stale_supervisors_recovers_only_expired_heartbeats() {
        let fleet = engine("pandora-fleet-reaper");
        fleet.register_node(&node("node-a", &["coding"])).unwrap();
        fleet.register_node(&node("node-b", &["coding"])).unwrap();
        fleet.start_supervisor("node-a", 1).unwrap();
        fleet.start_supervisor("node-b", 1).unwrap();
        fleet.heartbeat_supervisor("node-b", 15).unwrap();

        let reaped = fleet.reap_stale_supervisors(20, 10).unwrap();
        assert_eq!(reaped.len(), 1);
        assert_eq!(reaped[0].node_id(), "node-a");
        assert_eq!(reaped[0].state(), FleetSupervisorState::Recovering);
        assert_eq!(
            fleet
                .list_supervisors()
                .unwrap()
                .into_iter()
                .find(|supervisor| supervisor.node_id() == "node-b")
                .unwrap()
                .state(),
            FleetSupervisorState::Running
        );
        assert!(fleet.reap_stale_supervisors(20, 10).unwrap().is_empty());
    }

    #[test]
    fn malformed_registration_and_duplicate_lease_fail_closed() {
        assert!(matches!(
            FleetNode::new("", "2.0.0", "local", Vec::<String>::new(), 1),
            Err(FleetError::InvalidField("node ID"))
        ));
        let fleet = engine("pandora-fleet-duplicate");
        fleet.register_node(&node("node-a", &["coding"])).unwrap();
        fleet
            .acquire_lease(
                "lease-a",
                "node-a",
                "execution-a",
                FleetBudget::new(1, 1, 1, 1),
                1,
                10,
            )
            .unwrap();
        assert!(matches!(
            fleet.acquire_lease(
                "lease-a",
                "node-a",
                "execution-b",
                FleetBudget::new(1, 1, 1, 1),
                1,
                10,
            ),
            Err(FleetError::LeaseAlreadyExists)
        ));
        assert!(matches!(
            fleet.release_lease("lease-a"),
            Ok(FleetLease {
                state: FleetLeaseState::Released,
                ..
            })
        ));
        assert!(matches!(
            fleet.release_lease("lease-a"),
            Err(FleetError::LeaseNotActive(FleetLeaseState::Released))
        ));
    }
}
