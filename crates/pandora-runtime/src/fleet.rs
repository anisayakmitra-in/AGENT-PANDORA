use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

pub const FLEET_SCHEMA_VERSION: u32 = 1;
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
    connection: Mutex<Connection>,
}

impl FleetEngine {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, FleetError> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent).map_err(|_| {
                FleetError::Database(rusqlite::Error::InvalidPath(parent.to_path_buf()))
            })?;
        }
        let connection = Connection::open(path)?;
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
                 ON fleet_leases(node_id, state);",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
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
        let node = self
            .list_nodes()?
            .into_iter()
            .find(|node| node.state == FleetNodeState::Ready && node.supports(capability));
        node.ok_or(FleetError::NodeNotFound)
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
