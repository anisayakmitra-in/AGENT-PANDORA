use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use subtle::ConstantTimeEq;

pub const FLEET_SCHEMA_VERSION: u32 = 5;
pub const MAX_FLEET_NODES: usize = 256;
pub const MAX_FLEET_LEASES: usize = 4_096;
pub const MAX_FLEET_FENCES: usize = 4_096;
pub const MAX_FLEET_CAPABILITIES: usize = 64;
pub const MAX_FENCE_OPERATION_ROWS: usize = 4_096;
pub const MAX_FENCE_OPERATION_DURATION_SECONDS: u64 = 86_400;
pub const MAX_FENCE_OPERATION_RECOVERY_LIMIT: usize = 256;

/// The operation columns, in the order `decode_operation_row` expects.
const OPERATION_COLUMNS: &str = "operation_hash, fence_key, fence_owner_id, \
     fence_generation, fence_token_hash, fence_issued_at, acquired_at, \
     last_renewed_at, expires_at";

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
pub enum FleetFenceState {
    Active,
    Released,
    Expired,
}

impl FleetFenceState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Released => "released",
            Self::Expired => "expired",
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

#[derive(Clone)]
pub struct FleetLeaseFence {
    key: String,
    owner_id: String,
    generation: u64,
    token: String,
    issued_at: u64,
    expires_at: u64,
    state: FleetFenceState,
}

impl FleetLeaseFence {
    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn owner_id(&self) -> &str {
        &self.owner_id
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub const fn issued_at(&self) -> u64 {
        self.issued_at
    }

    pub const fn expires_at(&self) -> u64 {
        self.expires_at
    }

    pub const fn state(&self) -> FleetFenceState {
        self.state
    }
}

impl fmt::Debug for FleetLeaseFence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FleetLeaseFence")
            .field("key", &self.key)
            .field("owner_id", &self.owner_id)
            .field("generation", &self.generation)
            .field("issued_at", &self.issued_at)
            .field("expires_at", &self.expires_at)
            .field("state", &self.state)
            .field("token", &"[REDACTED]")
            .finish()
    }
}

pub struct FenceOperationGuard {
    operation_hash: [u8; 32],
    fence_key: String,
    fence_owner_id: String,
    fence_generation: u64,
    fence_token_hash: String,
    fence_issued_at: u64,
    acquired_at: u64,
    last_renewed_at: u64,
    expires_at: u64,
}

impl FenceOperationGuard {
    pub fn fence_key(&self) -> &str {
        &self.fence_key
    }

    pub fn acquired_at(&self) -> u64 {
        self.acquired_at
    }

    pub fn last_renewed_at(&self) -> u64 {
        self.last_renewed_at
    }

    pub fn expires_at(&self) -> u64 {
        self.expires_at
    }
}

impl fmt::Debug for FenceOperationGuard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FenceOperationGuard")
            .field("fence_key", &self.fence_key)
            .field("fence_owner_id", &self.fence_owner_id)
            .field("fence_generation", &self.fence_generation)
            .field("acquired_at", &self.acquired_at)
            .field("last_renewed_at", &self.last_renewed_at)
            .field("expires_at", &self.expires_at)
            .field("operation_hash", &"[REDACTED]")
            .field("fence_token_hash", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FenceOperationRecovery {
    removed_hashes: Vec<[u8; 32]>,
    has_more: bool,
}

impl FenceOperationRecovery {
    pub fn removed(&self) -> usize {
        self.removed_hashes.len()
    }

    pub fn has_more(&self) -> bool {
        self.has_more
    }

    pub fn removed_hashes(&self) -> &[[u8; 32]] {
        &self.removed_hashes
    }
}

/// One stored fence row, decoded.
struct FenceRow {
    owner_id: String,
    generation: u64,
    token_hash: String,
    issued_at: u64,
    expires_at: u64,
    state: String,
}

struct FenceOperationRow {
    operation_hash: [u8; 32],
    fence_key: String,
    fence_owner_id: String,
    fence_generation: u64,
    fence_token_hash: String,
    fence_issued_at: u64,
    acquired_at: u64,
    last_renewed_at: u64,
    expires_at: u64,
}

#[derive(Clone)]
enum TrustedClock {
    System,
    #[cfg(test)]
    Fixed(u64),
}

impl TrustedClock {
    fn now(&self) -> Result<u64, FleetError> {
        match self {
            Self::System => std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| FleetError::ClockUnavailable)
                .map(|duration| duration.as_secs()),
            #[cfg(test)]
            Self::Fixed(now) => Ok(*now),
        }
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
    InvalidFenceDuration,
    FenceAlreadyActive,
    FenceNotFound,
    FenceExpired,
    FenceMismatch,
    FenceLimitExceeded,
    InvalidFenceOperationDuration,
    FenceOperationActive,
    FenceOperationAlreadyActive,
    FenceOperationRecoveryRequired,
    FenceOperationWouldOutliveFence,
    FenceOperationLost,
    FenceOperationNonIncreasing,
    FenceOperationClockRegressed,
    FenceOperationLimitExceeded,
    ActiveFenceOperationsPresent,
    ClockUnavailable,
    UnsupportedSchemaVersion,
    Random,
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
            Self::InvalidFenceDuration => formatter.write_str("fleet fence duration is invalid"),
            Self::FenceAlreadyActive => formatter.write_str("fleet fence is already active"),
            Self::FenceNotFound => formatter.write_str("fleet fence was not found"),
            Self::FenceExpired => formatter.write_str("fleet fence has expired"),
            Self::FenceMismatch => formatter.write_str("fleet fence ownership does not match"),
            Self::FenceLimitExceeded => formatter.write_str("fleet fence limit was exceeded"),
            Self::InvalidFenceOperationDuration => {
                formatter.write_str("fence operation duration is invalid")
            }
            Self::FenceOperationActive => formatter.write_str("fence operation is active"),
            Self::FenceOperationAlreadyActive => {
                formatter.write_str("fence operation is already active")
            }
            Self::FenceOperationRecoveryRequired => {
                formatter.write_str("expired fence operation recovery is required")
            }
            Self::FenceOperationWouldOutliveFence => {
                formatter.write_str("fence operation would outlive its fence")
            }
            Self::FenceOperationLost => formatter.write_str("fence operation was lost"),
            Self::FenceOperationNonIncreasing => {
                formatter.write_str("fence operation renewal must extend its expiry")
            }
            Self::FenceOperationClockRegressed => {
                formatter.write_str("fence operation clock moved backwards")
            }
            Self::FenceOperationLimitExceeded => {
                formatter.write_str("fence operation row limit was exceeded")
            }
            Self::ActiveFenceOperationsPresent => {
                formatter.write_str("active fence operations block quiescence")
            }
            Self::ClockUnavailable => formatter.write_str("trusted clock is unavailable"),
            Self::UnsupportedSchemaVersion => {
                formatter.write_str("fleet schema version is unsupported")
            }
            Self::Random => formatter.write_str("fleet fence token generation failed"),
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
    clock: Arc<Mutex<TrustedClock>>,
}

impl FleetEngine {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, FleetError> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent).map_err(|_| {
                FleetError::Database(rusqlite::Error::InvalidPath(parent.to_path_buf()))
            })?;
        }
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        let initial_version = read_schema_version(&connection)?;
        if initial_version > FLEET_SCHEMA_VERSION {
            return Err(FleetError::UnsupportedSchemaVersion);
        }
        let journal_mode =
            connection.query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            connection.execute_batch("PRAGMA journal_mode = WAL;")?;
        }
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version = read_schema_version(&transaction)?;
        if version > FLEET_SCHEMA_VERSION {
            return Err(FleetError::UnsupportedSchemaVersion);
        }
        match version {
            0 => {
                ensure_fresh_database(&transaction)?;
                create_base_schema(&transaction)?;
                create_operation_schema(&transaction)?;
                set_schema_version(&transaction, FLEET_SCHEMA_VERSION)?;
            }
            4 => {
                validate_schema(&transaction, false)?;
                create_operation_schema(&transaction)?;
                set_schema_version(&transaction, FLEET_SCHEMA_VERSION)?;
            }
            5 => validate_schema(&transaction, true)?,
            _ => return Err(FleetError::UnsupportedSchemaVersion),
        }
        transaction.commit()?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            clock: Arc::new(Mutex::new(TrustedClock::System)),
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
        // Active fence operations are work in their own right, and the lease
        // count above cannot see them: a released lease would otherwise hide an
        // operation that is still running. Every row is strictly validated
        // first, so a malformed row fails this closed rather than being skipped
        // by the `expires_at > ?` predicate and read as "no work present".
        let now_trusted = self.trusted_now()?;
        let operation_rows = read_operation_rows(
            &transaction,
            &format!(
                "SELECT {OPERATION_COLUMNS} FROM fleet_fence_operations
                 ORDER BY acquired_at, operation_hash"
            ),
            [],
        )?;
        // An expired row is not active work, but it is still present and still
        // has to be reaped explicitly, so it is deliberately not counted here.
        if operation_rows
            .iter()
            .any(|row| row.expires_at > now_trusted)
        {
            return Err(FleetError::ActiveFenceOperationsPresent);
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

    pub fn acquire_fence(
        &self,
        key: impl Into<String>,
        owner_id: impl Into<String>,
        now: u64,
        duration_seconds: u64,
    ) -> Result<FleetLeaseFence, FleetError> {
        if duration_seconds == 0 {
            return Err(FleetError::InvalidFenceDuration);
        }
        let key = validate_text("fence key", key.into(), 256)?;
        let owner_id = validate_text("fence owner", owner_id.into(), 256)?;
        let expires_at = now
            .checked_add(duration_seconds)
            .ok_or(FleetError::InvalidFenceDuration)?;
        let token = generate_fence_token()?;
        let token_hash = fence_token_hash(&token);
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_work_permitted(&transaction, now)?;
        // Replacement is a fence mutation like any other, so a live operation
        // blocks it. Sampling the trusted clock here rather than reusing `now`
        // is deliberate: the caller's idea of the time is untrusted input to a
        // liveness decision, and an operation holder is exactly the party that
        // would want to understate it.
        self.block_fence_mutation(&transaction, &key, self.trusted_now()?)?;
        let existing = transaction
            .query_row(
                "SELECT generation, state, expires_at
                 FROM fleet_fences WHERE fence_key = ?1",
                params![key],
                |row| {
                    Ok((
                        decode_u64(row.get(0)?)?,
                        row.get::<_, String>(1)?,
                        decode_u64(row.get(2)?)?,
                    ))
                },
            )
            .optional()?;
        let generation = match existing {
            None => {
                let count: i64 =
                    transaction
                        .query_row("SELECT COUNT(*) FROM fleet_fences", [], |row| row.get(0))?;
                if usize::try_from(count).map_err(|_| FleetError::CorruptRecord)?
                    >= MAX_FLEET_FENCES
                {
                    return Err(FleetError::FenceLimitExceeded);
                }
                1
            }
            Some((generation, state, current_expires_at)) => {
                let state = decode_fence_state(&state)?;
                if state == FleetFenceState::Active && current_expires_at > now {
                    return Err(FleetError::FenceAlreadyActive);
                }
                generation.checked_add(1).ok_or(FleetError::CorruptRecord)?
            }
        };
        transaction.execute(
            "INSERT INTO fleet_fences
                (fence_key, owner_id, generation, token_hash, issued_at, expires_at, state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active')
             ON CONFLICT(fence_key) DO UPDATE SET
                owner_id = excluded.owner_id,
                generation = excluded.generation,
                token_hash = excluded.token_hash,
                issued_at = excluded.issued_at,
                expires_at = excluded.expires_at,
                state = excluded.state",
            params![
                key,
                owner_id,
                to_i64(generation)?,
                token_hash,
                to_i64(now)?,
                to_i64(expires_at)?,
            ],
        )?;
        transaction.commit()?;
        Ok(FleetLeaseFence {
            key,
            owner_id,
            generation,
            token,
            issued_at: now,
            expires_at,
            state: FleetFenceState::Active,
        })
    }

    /// Invalidate a logical fence after the owning supervisor has been fenced.
    ///
    /// The next acquisition of the same key receives a new generation. The
    /// method intentionally does not accept a claim token; callers use it only
    /// after the fleet supervisor lifecycle has established takeover authority.
    pub fn invalidate_fence(&self, key: &str, owner_id: &str) -> Result<bool, FleetError> {
        let key = validate_text("fence key", key.to_owned(), 256)?;
        let owner_id = validate_text("fence owner", owner_id.to_owned(), 256)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        self.block_fence_mutation(&transaction, &key, self.trusted_now()?)?;
        let record = transaction
            .query_row(
                "SELECT owner_id, state FROM fleet_fences WHERE fence_key = ?1",
                params![key],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let Some((current_owner, state)) = record else {
            transaction.commit()?;
            return Ok(false);
        };
        if current_owner != owner_id {
            return Err(FleetError::FenceMismatch);
        }
        if decode_fence_state(&state)? == FleetFenceState::Active {
            transaction.execute(
                "UPDATE fleet_fences SET state = 'released'
                 WHERE fence_key = ?1 AND owner_id = ?2 AND state = 'active'",
                params![key, owner_id],
            )?;
        }
        transaction.commit()?;
        Ok(true)
    }

    pub fn renew_fence(
        &self,
        fence: &FleetLeaseFence,
        now: u64,
        duration_seconds: u64,
    ) -> Result<FleetLeaseFence, FleetError> {
        if duration_seconds == 0 {
            return Err(FleetError::InvalidFenceDuration);
        }
        let expires_at = now
            .checked_add(duration_seconds)
            .ok_or(FleetError::InvalidFenceDuration)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        self.block_fence_mutation(&transaction, &fence.key, self.trusted_now()?)?;
        validate_fence(&transaction, fence, now)?;
        transaction.execute(
            "UPDATE fleet_fences SET expires_at = ?1
             WHERE fence_key = ?2 AND generation = ?3 AND state = 'active'",
            params![to_i64(expires_at)?, fence.key, to_i64(fence.generation)?],
        )?;
        transaction.commit()?;
        Ok(FleetLeaseFence {
            key: fence.key.clone(),
            owner_id: fence.owner_id.clone(),
            generation: fence.generation,
            token: fence.token.clone(),
            issued_at: fence.issued_at,
            expires_at,
            state: FleetFenceState::Active,
        })
    }

    pub fn assert_fence(&self, fence: &FleetLeaseFence, now: u64) -> Result<(), FleetError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_fence(&transaction, fence, now)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn release_fence(&self, fence: &FleetLeaseFence, now: u64) -> Result<(), FleetError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        self.block_fence_mutation(&transaction, &fence.key, self.trusted_now()?)?;
        validate_fence(&transaction, fence, now)?;
        transaction.execute(
            "UPDATE fleet_fences SET state = 'released'
             WHERE fence_key = ?1 AND generation = ?2 AND state = 'active'",
            params![fence.key, to_i64(fence.generation)?],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Expire every fence whose lifetime has elapsed.
    ///
    /// This is a batch, and a batch is where a per-fence blocker is easiest to
    /// get wrong: a single `UPDATE ... WHERE state = 'active' AND expires_at <= ?`
    /// cannot consult a per-fence condition, so a live operation would simply be
    /// overwritten along with everyone else. The candidates are therefore
    /// collected first, every one of them is checked, and only then is anything
    /// written. If any single fence is blocked the whole batch fails and nothing
    /// is expired, which is the only fail-closed reading: expiring the others and
    /// reporting an error would leave the caller unable to tell what happened.
    pub fn expire_fences(&self, now: u64) -> Result<usize, FleetError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let blocker_now = self.trusted_now()?;
        let mut candidates = {
            let mut statement = transaction.prepare(
                "SELECT fence_key FROM fleet_fences
                 WHERE state = 'active' AND expires_at <= ?1
                 ORDER BY fence_key",
            )?;
            let rows = statement.query_map(params![to_i64(now)?], |row| row.get::<_, String>(0))?;
            let mut keys = Vec::new();
            for row in rows {
                keys.push(row?);
            }
            keys
        };
        for key in &candidates {
            self.block_fence_mutation(&transaction, key, blocker_now)?;
        }
        let mut expired = 0usize;
        for key in candidates.drain(..) {
            expired += transaction.execute(
                "UPDATE fleet_fences SET state = 'expired'
                 WHERE fence_key = ?1 AND state = 'active' AND expires_at <= ?2",
                params![key, to_i64(now)?],
            )?;
        }
        transaction.commit()?;
        Ok(expired)
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

    fn trusted_now(&self) -> Result<u64, FleetError> {
        self.clock
            .lock()
            .map_err(|_| FleetError::LockPoisoned)?
            .now()
    }

    /// The active fence operation bound to `fence_key`, if any row exists.
    ///
    /// Every row for the key is read and strictly decoded, then classified with
    /// the trusted sample. A predicate such as `expires_at > ?` is deliberately
    /// not used for the read, because a malformed row that a predicate skipped
    /// would then look exactly like an absent operation, and the whole point of
    /// the registry is that a blocker never guesses.
    fn operation_for_fence(
        &self,
        transaction: &rusqlite::Transaction<'_>,
        fence_key: &str,
        now: u64,
    ) -> Result<Option<FenceOperationRow>, FleetError> {
        let rows = read_operation_rows(
            transaction,
            &format!(
                "SELECT {OPERATION_COLUMNS} FROM fleet_fence_operations
                 WHERE fence_key = ?1 ORDER BY acquired_at, operation_hash"
            ),
            params![fence_key],
        )?;
        let mut found: Option<FenceOperationRow> = None;
        for row in rows {
            if found.is_some() {
                // More than one row for a fence is a corrupt registry, not a
                // choice. The schema allows it; the invariant does not.
                return Err(FleetError::CorruptRecord);
            }
            found = Some(row);
        }
        let Some(row) = found else {
            return Ok(None);
        };
        if row.expires_at <= now {
            // A valid but expired row stays visible until the explicit reaper
            // removes it, so a stale row cannot masquerade as a live blocker
            // and cannot masquerade as absent either.
            return Err(FleetError::FenceOperationRecoveryRequired);
        }
        Ok(Some(row))
    }

    /// The single blocker every fence mutation consults.
    ///
    /// Returns `FenceOperationActive` while an operation is live and
    /// `FenceOperationRecoveryRequired` while an expired row is still present.
    /// Callers must already hold an immediate transaction.
    fn block_fence_mutation(
        &self,
        transaction: &rusqlite::Transaction<'_>,
        fence_key: &str,
        now: u64,
    ) -> Result<(), FleetError> {
        match self.operation_for_fence(transaction, fence_key, now)? {
            Some(_) => Err(FleetError::FenceOperationActive),
            None => Ok(()),
        }
    }

    /// Load the fence row a guard is bound to, without mutating it.
    fn fence_row_for_operation(
        transaction: &rusqlite::Transaction<'_>,
        fence_key: &str,
    ) -> Result<Option<FenceRow>, FleetError> {
        let record = transaction
            .query_row(
                "SELECT owner_id, generation, token_hash, issued_at, expires_at, state
                 FROM fleet_fences WHERE fence_key = ?1",
                params![fence_key],
                |row| {
                    Ok(FenceRow {
                        owner_id: row.get(0)?,
                        generation: decode_u64(row.get(1)?)?,
                        token_hash: row.get(2)?,
                        issued_at: decode_u64(row.get(3)?)?,
                        expires_at: decode_u64(row.get(4)?)?,
                        state: row.get(5)?,
                    })
                },
            )
            .optional()?;
        Ok(record)
    }

    /// Reject a duration that is zero, oversized, or overflows.
    fn validate_operation_duration(duration_seconds: u64) -> Result<(), FleetError> {
        if duration_seconds == 0 || duration_seconds > MAX_FENCE_OPERATION_DURATION_SECONDS {
            return Err(FleetError::InvalidFenceOperationDuration);
        }
        Ok(())
    }

    /// Acquire a durable operation over `fence`, blocking later fence mutations.
    ///
    /// The expiry is capped at the fence's own expiry, so an operation can never
    /// outlive the authority it was granted under. A valid expired row must be
    /// reaped first: acquiring beside one would let two rows describe the same
    /// fence with different lifetimes.
    pub fn acquire_fence_operation(
        &self,
        fence: &FleetLeaseFence,
        duration_seconds: u64,
    ) -> Result<FenceOperationGuard, FleetError> {
        Self::validate_operation_duration(duration_seconds)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Sample trusted time only after the write lock is held, so two
        // concurrent acquisitions cannot both read a stale clock.
        let now = self.trusted_now()?;
        // Admission first, judged on trusted time. An operation is work, so
        // quiescence has to be able to see one starting.
        ensure_work_permitted(&transaction, now)?;

        let record = Self::fence_row_for_operation(&transaction, &fence.key)?
            .ok_or(FleetError::FenceNotFound)?;
        let FenceRow {
            owner_id,
            generation,
            token_hash,
            issued_at,
            expires_at: fence_expires_at,
            state,
        } = record;
        // Every field of the binding is compared, not just the ones a forged
        // handle cannot cheaply match. `issued_at` is included because the
        // operation row binds it immutably: a handle that keeps the right token
        // but a different issue time is still not the fence the row was granted
        // against.
        if decode_fence_state(&state)? != FleetFenceState::Active
            || fence_expires_at <= now
            || issued_at > now
            || issued_at != fence.issued_at
            || owner_id != fence.owner_id
            || generation != fence.generation
            || !fence_token_matches(&token_hash, &fence_token_hash(&fence.token))
        {
            return Err(FleetError::FenceMismatch);
        }
        if self
            .operation_for_fence(&transaction, &fence.key, now)?
            .is_some()
        {
            return Err(FleetError::FenceOperationAlreadyActive);
        }
        // An expired row is reported by operation_for_fence as
        // FenceOperationRecoveryRequired, so reaching here means none exists.

        let total: i64 =
            transaction.query_row("SELECT COUNT(*) FROM fleet_fence_operations", [], |row| {
                row.get(0)
            })?;
        if usize::try_from(total).unwrap_or(usize::MAX) >= MAX_FENCE_OPERATION_ROWS {
            return Err(FleetError::FenceOperationLimitExceeded);
        }

        let requested = now
            .checked_add(duration_seconds)
            .ok_or(FleetError::InvalidFenceOperationDuration)?;
        let expires_at = requested.min(fence_expires_at);
        if expires_at <= now {
            return Err(FleetError::FenceOperationWouldOutliveFence);
        }

        // A fresh random nonce, stored only as a hash. The raw value lives in
        // the guard and never reaches SQLite, JSON, logs, or Debug.
        let nonce = random_nonce()?;
        let operation_hash = sha256_bytes(&nonce);
        let token_hash = fence_token_hash(&fence.token);
        transaction.execute(
            "INSERT INTO fleet_fence_operations
                (operation_hash, fence_key, fence_owner_id, fence_generation,
                 fence_token_hash, fence_issued_at, acquired_at, last_renewed_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8)",
            params![
                operation_hash.to_vec(),
                fence.key,
                fence.owner_id,
                to_i64(generation)?,
                token_hash,
                to_i64(issued_at)?,
                to_i64(now)?,
                to_i64(expires_at)?,
            ],
        )?;
        transaction.commit()?;
        Ok(FenceOperationGuard {
            operation_hash,
            fence_key: fence.key.clone(),
            fence_owner_id: fence.owner_id.clone(),
            fence_generation: generation,
            fence_token_hash: token_hash,
            fence_issued_at: issued_at,
            acquired_at: now,
            last_renewed_at: now,
            expires_at,
        })
    }

    /// Validate a guard's own row and its bound fence, without mutating either.
    pub fn assert_fence_operation(&self, guard: &FenceOperationGuard) -> Result<(), FleetError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = self.trusted_now()?;
        let row = self.matching_operation(&transaction, guard, now)?;
        let Some(row) = row else {
            return Err(FleetError::FenceOperationLost);
        };
        if row.expires_at <= now {
            return Err(FleetError::FenceOperationLost);
        }
        let FenceRow {
            owner_id,
            generation,
            token_hash,
            issued_at,
            state,
            ..
        } = Self::fence_row_for_operation(&transaction, &guard.fence_key)?
            .ok_or(FleetError::FenceOperationLost)?;
        if decode_fence_state(&state)? != FleetFenceState::Active
            || owner_id != guard.fence_owner_id
            || generation != guard.fence_generation
            || issued_at != guard.fence_issued_at
            || token_hash != guard.fence_token_hash
        {
            return Err(FleetError::CorruptRecord);
        }
        transaction.commit()?;
        Ok(())
    }

    /// Extend an operation, never shortening it and never outliving its fence.
    pub fn renew_fence_operation(
        &self,
        guard: &mut FenceOperationGuard,
        duration_seconds: u64,
    ) -> Result<(), FleetError> {
        Self::validate_operation_duration(duration_seconds)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = self.trusted_now()?;
        if now < guard.last_renewed_at {
            return Err(FleetError::FenceOperationClockRegressed);
        }
        let Some(_row) = self.matching_operation(&transaction, guard, now)? else {
            return Err(FleetError::FenceOperationLost);
        };
        let fence_expires_at = Self::fence_row_for_operation(&transaction, &guard.fence_key)?
            .ok_or(FleetError::FenceOperationLost)?
            .expires_at;
        let requested = now
            .checked_add(duration_seconds)
            .ok_or(FleetError::InvalidFenceOperationDuration)?;
        if requested > fence_expires_at {
            return Err(FleetError::InvalidFenceOperationDuration);
        }
        if requested <= guard.expires_at {
            return Err(FleetError::FenceOperationNonIncreasing);
        }
        transaction.execute(
            "UPDATE fleet_fence_operations
             SET last_renewed_at = ?1, expires_at = ?2
             WHERE operation_hash = ?3",
            params![
                to_i64(now)?,
                to_i64(requested)?,
                guard.operation_hash.to_vec(),
            ],
        )?;
        transaction.commit()?;
        guard.last_renewed_at = now;
        guard.expires_at = requested;
        Ok(())
    }

    /// Consume a guard and delete exactly its own row.
    ///
    /// Returns `FenceOperationLost` when the row is already gone, so a stale
    /// guard cannot silently appear to have released anything.
    pub fn release_fence_operation(&self, guard: FenceOperationGuard) -> Result<(), FleetError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = self.trusted_now()?;
        if self
            .matching_operation(&transaction, &guard, now)?
            .is_none()
        {
            return Err(FleetError::FenceOperationLost);
        }
        let removed = transaction.execute(
            "DELETE FROM fleet_fence_operations WHERE operation_hash = ?1",
            params![guard.operation_hash.to_vec()],
        )?;
        if removed != 1 {
            return Err(FleetError::CorruptRecord);
        }
        transaction.commit()?;
        Ok(())
    }

    /// The only automatic removal path: delete expired operation rows.
    ///
    /// Bounded by `limit`, and reports `has_more` via a `limit + 1` probe so a
    /// caller can page without `OFFSET`. Only operation rows are touched: never a
    /// fence, and never a successor operation.
    pub fn recover_expired_fence_operations(
        &self,
        limit: usize,
    ) -> Result<FenceOperationRecovery, FleetError> {
        if limit == 0 || limit > MAX_FENCE_OPERATION_RECOVERY_LIMIT {
            return Err(FleetError::InvalidField("recovery limit"));
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = self.trusted_now()?;

        // Validate every row before deleting anything, so a malformed row cannot
        // be removed as a side effect of a sweep.
        read_operation_rows(
            &transaction,
            &format!(
                "SELECT {OPERATION_COLUMNS} FROM fleet_fence_operations
                 ORDER BY expires_at, operation_hash"
            ),
            [],
        )?;

        let mut candidates = transaction.prepare(
            "SELECT operation_hash FROM fleet_fence_operations
             WHERE expires_at <= ?1 ORDER BY expires_at, operation_hash LIMIT ?2",
        )?;
        let probe = candidates
            .query_map(
                params![to_i64(now)?, i64::try_from(limit + 1).unwrap_or(i64::MAX)],
                |row| row.get::<_, Vec<u8>>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(candidates);
        let has_more = probe.len() > limit;
        let mut removed_hashes = Vec::with_capacity(probe.len().min(limit));
        for raw in probe.into_iter().take(limit) {
            if raw.len() != 32 {
                return Err(FleetError::CorruptRecord);
            }
            let mut hash = [0u8; 32];
            hash.copy_from_slice(&raw);
            let removed = transaction.execute(
                "DELETE FROM fleet_fence_operations
                 WHERE operation_hash = ?1 AND expires_at <= ?2",
                params![hash.to_vec(), to_i64(now)?],
            )?;
            if removed != 1 {
                return Err(FleetError::CorruptRecord);
            }
            removed_hashes.push(hash);
        }
        transaction.commit()?;
        Ok(FenceOperationRecovery {
            removed_hashes,
            has_more,
        })
    }

    /// The guard's own row, strictly decoded, or `None` when it is absent.
    ///
    /// A row that exists but no longer matches the guard's full fence binding is
    /// corruption rather than absence: the guard's identity was reused for
    /// different content.
    fn matching_operation(
        &self,
        transaction: &rusqlite::Transaction<'_>,
        guard: &FenceOperationGuard,
        now: u64,
    ) -> Result<Option<FenceOperationRow>, FleetError> {
        let row = transaction
            .query_row(
                &format!(
                    "SELECT {OPERATION_COLUMNS} FROM fleet_fence_operations
                     WHERE operation_hash = ?1"
                ),
                params![guard.operation_hash.to_vec()],
                decode_operation_row,
            )
            .optional()?;
        let Some(row) = row else {
            return Ok(None);
        };
        if row.operation_hash != guard.operation_hash {
            // Unreachable through the query predicate, so treat it as corruption
            // rather than a mismatch: the identity is the row's primary key and
            // a value that disagrees with it means the store is not trustworthy.
            return Err(FleetError::CorruptRecord);
        }
        if row.fence_key != guard.fence_key
            || row.fence_owner_id != guard.fence_owner_id
            || row.fence_generation != guard.fence_generation
            || row.fence_token_hash != guard.fence_token_hash
            || row.fence_issued_at != guard.fence_issued_at
            || row.acquired_at != guard.acquired_at
        {
            return Err(FleetError::CorruptRecord);
        }
        if now < row.acquired_at || now < row.last_renewed_at {
            return Err(FleetError::FenceOperationClockRegressed);
        }
        Ok(Some(row))
    }

    #[cfg(test)]
    fn set_trusted_now_for_test(&self, now: u64) {
        *self.clock.lock().expect("trusted clock lock") = TrustedClock::Fixed(now);
    }

    #[cfg(test)]
    fn schema_version_for_test(&self) -> u32 {
        self.lock()
            .unwrap()
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap()
            .try_into()
            .unwrap()
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, FleetError> {
        self.connection.lock().map_err(|_| FleetError::LockPoisoned)
    }
}

fn read_schema_version(connection: &Connection) -> Result<u32, FleetError> {
    let version = connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?;
    u32::try_from(version).map_err(|_| FleetError::CorruptRecord)
}

fn ensure_fresh_database(connection: &Connection) -> Result<(), FleetError> {
    let object_count = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type IN ('table', 'index', 'trigger', 'view')
           AND name NOT LIKE 'sqlite_%'",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    if object_count != 0 {
        return Err(FleetError::CorruptRecord);
    }
    Ok(())
}

fn create_base_schema(connection: &Connection) -> Result<(), FleetError> {
    connection.execute_batch(
        "CREATE TABLE fleet_nodes (
             id TEXT PRIMARY KEY,
             implementation_version TEXT NOT NULL,
             worker_class TEXT NOT NULL,
             capabilities_json TEXT NOT NULL,
             state TEXT NOT NULL,
             registered_at INTEGER NOT NULL
         );
         CREATE TABLE fleet_leases (
             id TEXT PRIMARY KEY,
             node_id TEXT NOT NULL,
             execution_id TEXT NOT NULL,
             budget_json TEXT NOT NULL,
             issued_at INTEGER NOT NULL,
             expires_at INTEGER NOT NULL,
             state TEXT NOT NULL
         );
         CREATE INDEX fleet_leases_node_idx ON fleet_leases(node_id, state);
         CREATE TABLE fleet_fences (
             fence_key TEXT PRIMARY KEY,
             owner_id TEXT NOT NULL,
             generation INTEGER NOT NULL CHECK (generation > 0),
             token_hash TEXT NOT NULL,
             issued_at INTEGER NOT NULL,
             expires_at INTEGER NOT NULL,
             state TEXT NOT NULL CHECK (state IN ('active', 'released', 'expired'))
         );
         CREATE INDEX fleet_fences_state_idx ON fleet_fences(state, expires_at);
         CREATE TABLE fleet_supervisors (
             node_id TEXT PRIMARY KEY,
             state TEXT NOT NULL CHECK (state IN ('stopped', 'running', 'draining', 'recovering')),
             generation INTEGER NOT NULL CHECK (generation > 0),
             process_id INTEGER,
             reason TEXT,
             updated_at INTEGER NOT NULL
         );
         CREATE TABLE fleet_quiescence (
             id INTEGER PRIMARY KEY CHECK (id = 1),
             owner TEXT NOT NULL,
             acquired_at INTEGER NOT NULL,
             expires_at INTEGER NOT NULL
         );",
    )?;
    Ok(())
}

fn create_operation_schema(connection: &Connection) -> Result<(), FleetError> {
    connection.execute_batch(
        "CREATE TABLE fleet_fence_operations (
             operation_hash BLOB NOT NULL PRIMARY KEY
                 CHECK (typeof(operation_hash) = 'blob' AND length(operation_hash) = 32),
             fence_key TEXT NOT NULL
                 CHECK (typeof(fence_key) = 'text' AND length(fence_key) BETWEEN 1 AND 256),
             fence_owner_id TEXT NOT NULL
                 CHECK (typeof(fence_owner_id) = 'text' AND length(fence_owner_id) BETWEEN 1 AND 256),
             fence_generation INTEGER NOT NULL
                 CHECK (typeof(fence_generation) = 'integer' AND fence_generation > 0),
             fence_token_hash TEXT NOT NULL
                 CHECK (typeof(fence_token_hash) = 'text' AND length(fence_token_hash) = 64),
             fence_issued_at INTEGER NOT NULL
                 CHECK (typeof(fence_issued_at) = 'integer' AND fence_issued_at >= 0),
             acquired_at INTEGER NOT NULL
                 CHECK (typeof(acquired_at) = 'integer' AND acquired_at >= 0),
             last_renewed_at INTEGER NOT NULL
                 CHECK (typeof(last_renewed_at) = 'integer' AND last_renewed_at >= acquired_at),
             expires_at INTEGER NOT NULL
                 CHECK (typeof(expires_at) = 'integer' AND expires_at > acquired_at
                    AND last_renewed_at < expires_at)
         ) STRICT;
         CREATE INDEX fleet_fence_operations_expiry_idx
             ON fleet_fence_operations(fence_key, expires_at);
         CREATE INDEX fleet_fence_operations_total_idx
             ON fleet_fence_operations(expires_at, operation_hash);",
    )?;
    Ok(())
}

fn set_schema_version(connection: &Connection, version: u32) -> Result<(), FleetError> {
    connection.pragma_update(None, "user_version", version)?;
    Ok(())
}

fn validate_schema(connection: &Connection, operations: bool) -> Result<(), FleetError> {
    let expected_tables = [
        "fleet_nodes",
        "fleet_leases",
        "fleet_fences",
        "fleet_supervisors",
        "fleet_quiescence",
    ];
    let expected_indexes = ["fleet_leases_node_idx", "fleet_fences_state_idx"];
    let mut expected_table_names = expected_tables
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();
    let mut expected_index_names = expected_indexes
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();
    let mut actual_tables = Vec::new();
    let mut statement = connection.prepare(
        "SELECT name FROM sqlite_master
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    for row in rows {
        actual_tables.push(row?);
    }
    if operations {
        expected_table_names.push("fleet_fence_operations".to_owned());
        expected_index_names.extend([
            "fleet_fence_operations_expiry_idx".to_owned(),
            "fleet_fence_operations_total_idx".to_owned(),
        ]);
        let sql = connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
                params!["fleet_fence_operations"],
                |row| row.get::<_, Option<String>>(0),
            )?
            .ok_or(FleetError::CorruptRecord)?;
        if !sql.to_ascii_uppercase().contains("STRICT") {
            return Err(FleetError::CorruptRecord);
        }
    }
    actual_tables.sort();
    expected_table_names.sort();
    if actual_tables != expected_table_names {
        return Err(FleetError::CorruptRecord);
    }

    let mut actual_indexes = Vec::new();
    let mut statement = connection.prepare(
        "SELECT name FROM sqlite_master
         WHERE type = 'index' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    for row in rows {
        actual_indexes.push(row?);
    }
    expected_index_names.sort();
    actual_indexes.sort();
    if actual_indexes != expected_index_names {
        return Err(FleetError::CorruptRecord);
    }

    validate_table_columns(
        connection,
        "fleet_nodes",
        &[
            ("id", "TEXT", true),
            ("implementation_version", "TEXT", false),
            ("worker_class", "TEXT", false),
            ("capabilities_json", "TEXT", false),
            ("state", "TEXT", false),
            ("registered_at", "INTEGER", false),
        ],
    )?;
    validate_table_columns(
        connection,
        "fleet_leases",
        &[
            ("id", "TEXT", true),
            ("node_id", "TEXT", false),
            ("execution_id", "TEXT", false),
            ("budget_json", "TEXT", false),
            ("issued_at", "INTEGER", false),
            ("expires_at", "INTEGER", false),
            ("state", "TEXT", false),
        ],
    )?;
    validate_table_columns(
        connection,
        "fleet_fences",
        &[
            ("fence_key", "TEXT", true),
            ("owner_id", "TEXT", false),
            ("generation", "INTEGER", false),
            ("token_hash", "TEXT", false),
            ("issued_at", "INTEGER", false),
            ("expires_at", "INTEGER", false),
            ("state", "TEXT", false),
        ],
    )?;
    validate_table_columns(
        connection,
        "fleet_supervisors",
        &[
            ("node_id", "TEXT", true),
            ("state", "TEXT", false),
            ("generation", "INTEGER", false),
            ("process_id", "INTEGER", false),
            ("reason", "TEXT", false),
            ("updated_at", "INTEGER", false),
        ],
    )?;
    validate_table_columns(
        connection,
        "fleet_quiescence",
        &[
            ("id", "INTEGER", true),
            ("owner", "TEXT", false),
            ("acquired_at", "INTEGER", false),
            ("expires_at", "INTEGER", false),
        ],
    )?;
    if operations {
        validate_table_columns(
            connection,
            "fleet_fence_operations",
            &[
                ("operation_hash", "BLOB", true),
                ("fence_key", "TEXT", false),
                ("fence_owner_id", "TEXT", false),
                ("fence_generation", "INTEGER", false),
                ("fence_token_hash", "TEXT", false),
                ("fence_issued_at", "INTEGER", false),
                ("acquired_at", "INTEGER", false),
                ("last_renewed_at", "INTEGER", false),
                ("expires_at", "INTEGER", false),
            ],
        )?;
    }
    Ok(())
}

fn validate_table_columns(
    connection: &Connection,
    table: &str,
    expected: &[(&str, &str, bool)],
) -> Result<(), FleetError> {
    let mut statement =
        connection.prepare("SELECT name, type, pk FROM pragma_table_info(?1) ORDER BY cid")?;
    let rows = statement.query_map(params![table], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    let mut actual = Vec::new();
    for row in rows {
        actual.push(row?);
    }
    if actual.len() != expected.len() {
        return Err(FleetError::CorruptRecord);
    }
    for (index, (name, ty, primary_key)) in expected.iter().enumerate() {
        let (actual_name, actual_type, actual_primary_key) = &actual[index];
        if actual_name != name
            || !actual_type.eq_ignore_ascii_case(ty)
            || (*actual_primary_key == 1) != *primary_key
        {
            return Err(FleetError::CorruptRecord);
        }
    }
    Ok(())
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

fn generate_fence_token() -> Result<String, FleetError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| FleetError::Random)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn fence_token_hash(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

fn fence_token_matches(expected: &str, actual: &str) -> bool {
    expected.as_bytes().ct_eq(actual.as_bytes()).into()
}

/// Check a fence's binding without changing it.
///
/// This is pure on purpose. It used to mark an elapsed fence `expired` as a side
/// effect of being observed, which meant a read path could take a write lock and
/// change the answer a later caller got. Elapsed is now a statement about the
/// supplied `now`, and the transition to `expired` belongs to `expire_fences`.
fn validate_fence(
    connection: &Connection,
    fence: &FleetLeaseFence,
    now: u64,
) -> Result<(), FleetError> {
    let record = connection
        .query_row(
            "SELECT owner_id, generation, token_hash, issued_at, expires_at, state
             FROM fleet_fences WHERE fence_key = ?1",
            params![fence.key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    decode_u64(row.get(1)?)?,
                    row.get::<_, String>(2)?,
                    decode_u64(row.get(3)?)?,
                    decode_u64(row.get(4)?)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .optional()?;
    let Some((owner_id, generation, token_hash, issued_at, expires_at, state)) = record else {
        return Err(FleetError::FenceNotFound);
    };
    let state = decode_fence_state(&state)?;
    if state != FleetFenceState::Active {
        return Err(FleetError::FenceMismatch);
    }
    if expires_at <= now {
        return Err(FleetError::FenceExpired);
    }
    if issued_at > now
        || owner_id != fence.owner_id
        || generation != fence.generation
        || !fence_token_matches(&token_hash, &fence_token_hash(&fence.token))
    {
        return Err(FleetError::FenceMismatch);
    }
    Ok(())
}

/// A fresh 32-byte random nonce for an operation identity.
fn random_nonce() -> Result<[u8; 32], FleetError> {
    let mut nonce = [0u8; 32];
    getrandom::fill(&mut nonce).map_err(|_| FleetError::Random)?;
    Ok(nonce)
}

fn sha256_bytes(bytes: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(&Sha256::digest(bytes));
    out
}

/// Strictly decode one operation row.
///
/// The table is `STRICT` and every column carries a `CHECK`, so these same
/// invariants are already enforced on write. Repeating them here is deliberate
/// redundancy, not an oversight: a `CHECK` guards writes that go through the
/// table, and this guards reads against a store that is no longer enforcing
/// them, such as a corrupted page or a rewritten schema. A row that fails any
/// check below is corruption, never absent and never expired, so a read can
/// never mistake an untrustworthy row for "no operation".
fn decode_operation_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FenceOperationRow> {
    let corrupt = || {
        rusqlite::Error::InvalidColumnType(
            0,
            "a valid fence operation row".to_owned(),
            rusqlite::types::Type::Blob,
        )
    };
    let raw = row.get::<_, Vec<u8>>(0)?;
    if raw.len() != 32 {
        return Err(corrupt());
    }
    let mut operation_hash = [0u8; 32];
    operation_hash.copy_from_slice(&raw);
    let fence_key = row.get::<_, String>(1)?;
    let fence_owner_id = row.get::<_, String>(2)?;
    let fence_generation = decode_u64(row.get(3)?)?;
    let fence_token_hash = row.get::<_, String>(4)?;
    let fence_issued_at = decode_u64(row.get(5)?)?;
    let acquired_at = decode_u64(row.get(6)?)?;
    let last_renewed_at = decode_u64(row.get(7)?)?;
    let expires_at = decode_u64(row.get(8)?)?;
    if fence_key.is_empty()
        || fence_key.len() > 256
        || fence_owner_id.is_empty()
        || fence_owner_id.len() > 256
        || fence_token_hash.len() != 64
        || fence_generation == 0
        || last_renewed_at < acquired_at
        || expires_at <= acquired_at
        || last_renewed_at >= expires_at
    {
        return Err(corrupt());
    }
    Ok(FenceOperationRow {
        operation_hash,
        fence_key,
        fence_owner_id,
        fence_generation,
        fence_token_hash,
        fence_issued_at,
        acquired_at,
        last_renewed_at,
        expires_at,
    })
}

/// Read and strictly decode operation rows.
///
/// This exists so the malformed-row mapping is stated once. A decode failure
/// becomes `CorruptRecord`, which is what a caller must act on; without it the
/// same condition arrives as a generic `Database` error that reads like an
/// ordinary failure and invites a retry. Errors from preparing or stepping the
/// statement stay `Database`, because those are genuinely about the connection.
fn read_operation_rows<P: rusqlite::Params>(
    transaction: &rusqlite::Transaction<'_>,
    sql: &str,
    parameter: P,
) -> Result<Vec<FenceOperationRow>, FleetError> {
    let mut statement = transaction.prepare(sql)?;
    let mut rows = statement.query(parameter)?;
    let mut decoded = Vec::new();
    while let Some(row) = rows.next()? {
        decoded.push(decode_operation_row(row).map_err(|_| FleetError::CorruptRecord)?);
    }
    Ok(decoded)
}

fn decode_fence_state(value: &str) -> Result<FleetFenceState, FleetError> {
    match value {
        "active" => Ok(FleetFenceState::Active),
        "released" => Ok(FleetFenceState::Released),
        "expired" => Ok(FleetFenceState::Expired),
        _ => Err(FleetError::CorruptRecord),
    }
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
    fn invalidated_fence_is_reclaimed_with_the_next_generation() {
        let fleet = engine("pandora-fleet-fence-invalidation");
        let first = fleet.acquire_fence("job-1", "worker-a", 10, 20).unwrap();
        assert!(fleet.invalidate_fence("job-1", "worker-a").unwrap());
        let replacement = fleet.acquire_fence("job-1", "worker-a", 11, 20).unwrap();
        assert_eq!(replacement.generation(), 2);
        assert!(matches!(
            fleet.assert_fence(&first, 12),
            Err(FleetError::FenceMismatch)
        ));
    }

    #[test]
    fn fenced_lease_renewal_and_reclaim_are_generation_bound() {
        let fleet = engine("pandora-fleet-fence-generation");
        let first = fleet.acquire_fence("job-1", "worker-a", 10, 20).unwrap();
        assert_eq!(first.generation(), 1);
        let renewed = fleet.renew_fence(&first, 15, 20).unwrap();
        assert_eq!(renewed.expires_at(), 35);
        fleet.assert_fence(&renewed, 34).unwrap();

        assert!(matches!(
            fleet.assert_fence(&renewed, 35),
            Err(FleetError::FenceExpired)
        ));
        let replacement = fleet.acquire_fence("job-1", "worker-b", 35, 20).unwrap();
        assert_eq!(replacement.generation(), 2);
        assert!(matches!(
            fleet.renew_fence(&renewed, 36, 20),
            Err(FleetError::FenceMismatch)
        ));
        fleet.assert_fence(&replacement, 36).unwrap();
    }

    #[test]
    fn fenced_lease_survives_restart_and_rejects_a_stale_owner() {
        let root = crate::test_support::new_temp_dir("pandora-fleet-fence-restart").unwrap();
        let path = root.join("fleet.sqlite3");
        let first = FleetEngine::open(&path).unwrap();
        let fence = first.acquire_fence("job-1", "worker-a", 10, 20).unwrap();
        drop(first);

        let second = FleetEngine::open(&path).unwrap();
        second.assert_fence(&fence, 11).unwrap();
        assert!(matches!(second.release_fence(&fence, 12), Ok(())));
        assert!(matches!(
            second.assert_fence(&fence, 12),
            Err(FleetError::FenceMismatch)
        ));
    }

    #[test]
    fn only_one_concurrent_fence_acquisition_wins() {
        let root = crate::test_support::new_temp_dir("pandora-fleet-fence-race").unwrap();
        let path = root.join("fleet.sqlite3");
        let fleet = Arc::new(FleetEngine::open(&path).unwrap());
        let mut handles = Vec::new();
        for owner in ["worker-a", "worker-b"] {
            let fleet = Arc::clone(&fleet);
            handles.push(std::thread::spawn(move || {
                fleet.acquire_fence("job-1", owner, 10, 20)
            }));
        }
        let outcomes: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, Err(FleetError::FenceAlreadyActive)))
                .count(),
            1
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

    #[test]
    fn fence_operation_registry_is_durable_and_schema_v5() {
        let root = crate::test_support::new_temp_dir("pandora-fence-operation-durable").unwrap();
        let path = root.join("fleet.sqlite3");
        let first = FleetEngine::open(&path).unwrap();
        first.set_trusted_now_for_test(10);
        let fence = first.acquire_fence("job-1", "worker-a", 10, 100).unwrap();
        let guard = first.acquire_fence_operation(&fence, 30).unwrap();
        drop(first);

        let second = FleetEngine::open(&path).unwrap();
        second.set_trusted_now_for_test(11);
        second.assert_fence_operation(&guard).unwrap();
        assert_eq!(second.schema_version_for_test(), 5);
    }

    #[test]
    fn active_operation_blocks_fence_mutations_until_explicit_recovery() {
        let fleet = engine("pandora-fence-operation-blocker");
        fleet.set_trusted_now_for_test(10);
        let fence = fleet.acquire_fence("job-1", "worker-a", 10, 20).unwrap();
        fleet.set_trusted_now_for_test(11);
        let guard = fleet.acquire_fence_operation(&fence, 5).unwrap();

        assert!(matches!(
            fleet.invalidate_fence("job-1", "worker-a"),
            Err(FleetError::FenceOperationActive)
        ));
        assert!(matches!(
            fleet.release_fence(&fence, 11),
            Err(FleetError::FenceOperationActive)
        ));
        assert!(matches!(
            fleet.acquire_fence("job-1", "worker-b", 11, 20),
            Err(FleetError::FenceOperationActive)
        ));
        assert!(matches!(
            fleet.expire_fences(30),
            Err(FleetError::FenceOperationActive)
        ));

        fleet.set_trusted_now_for_test(16);
        let recovery = fleet.recover_expired_fence_operations(10).unwrap();
        assert_eq!(recovery.removed(), 1);
        assert!(!recovery.has_more());
        assert!(matches!(
            fleet.release_fence_operation(guard),
            Err(FleetError::FenceOperationLost)
        ));
        assert!(fleet.invalidate_fence("job-1", "worker-a").unwrap());
    }

    #[test]
    fn operation_renewal_uses_trusted_time_and_caps_at_fence_expiry() {
        let fleet = engine("pandora-fence-operation-renewal");
        fleet.set_trusted_now_for_test(10);
        let fence = fleet.acquire_fence("job-1", "worker-a", 10, 20).unwrap();
        fleet.set_trusted_now_for_test(11);
        let mut guard = fleet.acquire_fence_operation(&fence, 5).unwrap();
        assert_eq!(guard.expires_at(), 16);

        fleet.set_trusted_now_for_test(12);
        fleet.renew_fence_operation(&mut guard, 10).unwrap();
        assert_eq!(guard.expires_at(), 22);
        assert!(matches!(
            fleet.renew_fence_operation(&mut guard, 0),
            Err(FleetError::InvalidFenceOperationDuration)
        ));
        assert!(matches!(
            fleet.renew_fence_operation(&mut guard, 100),
            Err(FleetError::InvalidFenceOperationDuration)
        ));
        assert!(matches!(
            fleet.renew_fence_operation(&mut guard, 1),
            Err(FleetError::FenceOperationNonIncreasing)
        ));
    }

    /// Count operation rows directly, for tests that need to assert something
    /// was not deleted or not duplicated.
    fn operation_row_count(fleet: &FleetEngine) -> usize {
        fleet
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM fleet_fence_operations", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap()
            .try_into()
            .unwrap()
    }

    /// Write a row the table's own `CHECK` constraints would reject.
    ///
    /// The constraints are switched off for the insert so that a malformed row
    /// can exist at all. This is fault injection, not a normal write: it models a
    /// store that is not enforcing its constraints, which is the only situation
    /// the strict decoder exists to catch.
    fn inject_malformed_operation(fleet: &FleetEngine) {
        let connection = fleet.lock().unwrap();
        connection
            .execute_batch("PRAGMA ignore_check_constraints = ON")
            .unwrap();
        // generation = 0 and expires_at <= acquired_at: both are invariants the
        // table rejects on write.
        let inserted = connection.execute(
            "INSERT INTO fleet_fence_operations
                (operation_hash, fence_key, fence_owner_id, fence_generation,
                 fence_token_hash, fence_issued_at, acquired_at, last_renewed_at,
                 expires_at)
             VALUES (?1, 'job-1', 'worker-a', 0, ?2, 10, 10, 10, 10)",
            params![vec![9u8; 32], "0".repeat(64)],
        );
        connection
            .execute_batch("PRAGMA ignore_check_constraints = OFF")
            .unwrap();
        assert_eq!(
            inserted.unwrap(),
            1,
            "the fault-injected row must be present"
        );
    }

    #[test]
    fn the_blocker_reads_the_trusted_clock_and_ignores_a_lying_caller() {
        let fleet = engine("pandora-fence-operation-clock");
        fleet.set_trusted_now_for_test(10);
        let fence = fleet.acquire_fence("job-1", "worker-a", 10, 100).unwrap();
        fleet.set_trusted_now_for_test(11);
        fleet.acquire_fence_operation(&fence, 5).unwrap();

        // The operation is live until trusted time 16. A caller claiming `now`
        // far in the past is the one thing an operation holder controls, so the
        // blocker must not consult it.
        assert!(matches!(
            fleet.release_fence(&fence, 0),
            Err(FleetError::FenceOperationActive)
        ));
        assert!(matches!(
            fleet.acquire_fence("job-1", "worker-b", 0, 100),
            Err(FleetError::FenceOperationActive)
        ));
        assert!(matches!(
            fleet.expire_fences(200),
            Err(FleetError::FenceOperationActive)
        ));

        // Past expiry the blocker changes its verdict: the row is still present,
        // so it is not "no operation" but "recovery owed". It is never silently
        // treated as absent.
        fleet.set_trusted_now_for_test(16);
        assert!(matches!(
            fleet.release_fence(&fence, 16),
            Err(FleetError::FenceOperationRecoveryRequired)
        ));
        assert!(matches!(
            fleet.acquire_fence_operation(&fence, 5),
            Err(FleetError::FenceOperationRecoveryRequired)
        ));
    }

    #[test]
    fn asserting_a_fence_or_operation_never_changes_stored_state() {
        let fleet = engine("pandora-fence-operation-pure");
        fleet.set_trusted_now_for_test(10);
        let fence = fleet.acquire_fence("job-1", "worker-a", 10, 10).unwrap();
        fleet.set_trusted_now_for_test(11);
        let guard = fleet.acquire_fence_operation(&fence, 5).unwrap();
        fleet.assert_fence_operation(&guard).unwrap();
        fleet.assert_fence(&fence, 11).unwrap();

        // Observing an elapsed fence used to mark it expired as a side effect.
        fleet.set_trusted_now_for_test(40);
        assert!(matches!(
            fleet.assert_fence(&fence, 40),
            Err(FleetError::FenceExpired)
        ));
        assert!(matches!(
            fleet.assert_fence_operation(&guard),
            Err(FleetError::FenceOperationLost)
        ));

        // Purity is checked against a bare connection rather than through
        // `assert_fence`. Every production caller rolls its transaction back
        // after a failed validation, so the old side-effecting `UPDATE` was
        // never observable through them and asserting on those paths proves
        // nothing. On a bare connection the write would auto-commit, so this
        // genuinely fails if the read starts mutating again.
        {
            let connection = fleet.lock().unwrap();
            assert!(matches!(
                validate_fence(&connection, &fence, 40),
                Err(FleetError::FenceExpired)
            ));
            let state: String = connection
                .query_row(
                    "SELECT state FROM fleet_fences WHERE fence_key = 'job-1'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                state, "active",
                "validate_fence must not write: a read that transitions state is \
                 what made a second caller's answer depend on who looked first"
            );
        }
        assert_eq!(operation_row_count(&fleet), 1);
    }

    #[test]
    fn a_released_operation_allows_a_new_one_and_never_coexists() {
        let fleet = engine("pandora-fence-operation-reincarnation");
        fleet.set_trusted_now_for_test(10);
        let fence = fleet.acquire_fence("job-1", "worker-a", 10, 100).unwrap();
        fleet.set_trusted_now_for_test(11);
        let first = fleet.acquire_fence_operation(&fence, 5).unwrap();

        // A second operation over the same live fence is refused, so a fence can
        // never carry two holders at once.
        assert!(matches!(
            fleet.acquire_fence_operation(&fence, 5),
            Err(FleetError::FenceOperationAlreadyActive)
        ));
        assert!(matches!(
            fleet.acquire_fence_operation(&fence, 0),
            Err(FleetError::InvalidFenceOperationDuration)
        ));

        fleet.release_fence_operation(first).unwrap();
        assert_eq!(operation_row_count(&fleet), 0);

        // The guard is move-only and not `Clone`, so a released guard cannot be
        // reused in safe code at all. Reincarnation is therefore a clean row
        // swap rather than two holders coexisting on one fence.
        fleet.set_trusted_now_for_test(12);
        let second = fleet.acquire_fence_operation(&fence, 5).unwrap();
        assert_eq!(operation_row_count(&fleet), 1);
        fleet.assert_fence_operation(&second).unwrap();
        fleet.release_fence_operation(second).unwrap();
        assert_eq!(operation_row_count(&fleet), 0);
    }

    #[test]
    fn an_operation_is_bound_to_the_whole_fence_not_just_its_key() {
        let fleet = engine("pandora-fence-operation-binding");
        fleet.set_trusted_now_for_test(10);
        let fence = fleet.acquire_fence("job-1", "worker-a", 10, 100).unwrap();
        fleet.set_trusted_now_for_test(11);
        let guard = fleet.acquire_fence_operation(&fence, 5).unwrap();

        // A fence handle is checked against the whole stored binding, not just
        // the key, so a handle that merely names the same key is refused.
        let mut forged = fence.clone();
        forged.generation = fence.generation() + 1;
        assert!(matches!(
            fleet.acquire_fence_operation(&forged, 5),
            Err(FleetError::FenceMismatch)
        ));
        let mut wrong_owner = fence.clone();
        wrong_owner.owner_id = "worker-b".to_owned();
        assert!(matches!(
            fleet.acquire_fence_operation(&wrong_owner, 5),
            Err(FleetError::FenceMismatch)
        ));
        // The bound fence handle is validated too, not just the fence row.
        let mut wrong_issued_at = fence.clone();
        wrong_issued_at.issued_at = fence.issued_at() + 1;
        assert!(matches!(
            fleet.acquire_fence_operation(&wrong_issued_at, 5),
            Err(FleetError::FenceMismatch)
        ));

        // Rewriting the stored fence binding under a live guard is corruption,
        // not a silent rebind.
        {
            let connection = fleet.lock().unwrap();
            connection
                .execute(
                    "UPDATE fleet_fences SET owner_id = 'worker-b' WHERE fence_key = 'job-1'",
                    [],
                )
                .unwrap();
        }
        assert!(matches!(
            fleet.assert_fence_operation(&guard),
            Err(FleetError::CorruptRecord)
        ));
    }

    #[test]
    fn a_live_operation_blocks_a_separate_connection_to_the_same_database() {
        let root = crate::test_support::new_temp_dir("pandora-fence-operation-xconn").unwrap();
        let path = root.join("fleet.sqlite3");
        let first = FleetEngine::open(&path).unwrap();
        first.set_trusted_now_for_test(10);
        let fence = first.acquire_fence("job-1", "worker-a", 10, 100).unwrap();
        first.set_trusted_now_for_test(11);
        first.acquire_fence_operation(&fence, 30).unwrap();

        // A second engine on the same file is a different connection and a
        // different process boundary, so the block cannot be an in-memory flag.
        let second = FleetEngine::open(&path).unwrap();
        second.set_trusted_now_for_test(12);
        assert!(matches!(
            second.invalidate_fence("job-1", "worker-a"),
            Err(FleetError::FenceOperationActive)
        ));
        assert!(matches!(
            second.acquire_fence_operation(&fence, 5),
            Err(FleetError::FenceOperationAlreadyActive)
        ));
        assert!(matches!(
            second.release_fence(&fence, 12),
            Err(FleetError::FenceOperationActive)
        ));
    }

    #[test]
    fn bulk_expiration_is_all_or_nothing_when_one_fence_is_blocked() {
        let fleet = engine("pandora-fence-operation-batch");
        fleet.set_trusted_now_for_test(10);
        // Two fences, both elapsed, one of them under a live operation.
        let blocked = fleet.acquire_fence("job-late", "worker-a", 10, 20).unwrap();
        fleet
            .acquire_fence("job-early", "worker-b", 10, 20)
            .unwrap();
        fleet.set_trusted_now_for_test(11);
        // The operation is capped at the fence's own expiry, so it stays live
        // while the fence is still active and the fence expires afterwards.
        fleet.acquire_fence_operation(&blocked, 1000).unwrap();
        fleet.set_trusted_now_for_test(100);

        // The operation is expired but still present, so the blocker reports
        // recovery owed rather than a live holder. The distinction matters: one
        // is resolved by waiting, the other by the holder releasing.
        assert!(matches!(
            fleet.expire_fences(100),
            Err(FleetError::FenceOperationRecoveryRequired)
        ));
        // Fail-closed means the unblocked fence was not expired either, even
        // though it was perfectly eligible. A partial batch would leave the
        // caller unable to tell what happened.
        let connection = fleet.lock().unwrap();
        let active: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM fleet_fences WHERE state = 'active'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(active, 2, "a blocked batch must expire nothing at all");
    }

    #[test]
    fn an_operation_may_never_outlive_the_fence_it_was_granted_under() {
        let fleet = engine("pandora-fence-operation-bound");
        fleet.set_trusted_now_for_test(10);
        let fence = fleet.acquire_fence("job-1", "worker-a", 10, 100).unwrap();
        fleet.set_trusted_now_for_test(11);

        // A longer request is capped at the fence, never granted beyond it.
        let guard = fleet.acquire_fence_operation(&fence, 10_000).unwrap();
        assert_eq!(guard.expires_at(), fence.expires_at());
        assert!(guard.expires_at() <= fence.expires_at);
        // A renewal that would cross the fence boundary is refused outright.
        fleet.set_trusted_now_for_test(12);
        let mut guard = guard;
        assert!(matches!(
            fleet.renew_fence_operation(&mut guard, 10_000),
            Err(FleetError::InvalidFenceOperationDuration)
        ));
        // Oversized and zero requests are refused before any write.
        assert!(matches!(
            fleet.acquire_fence_operation(&fence, u64::MAX),
            Err(FleetError::InvalidFenceOperationDuration)
        ));
    }

    #[test]
    fn recovery_is_bounded_and_never_touches_fences_or_live_operations() {
        let fleet = engine("pandora-fence-operation-recovery");
        fleet.set_trusted_now_for_test(10);
        assert!(matches!(
            fleet.recover_expired_fence_operations(0),
            Err(FleetError::InvalidField("recovery limit"))
        ));
        assert!(matches!(
            fleet.recover_expired_fence_operations(MAX_FENCE_OPERATION_RECOVERY_LIMIT + 1),
            Err(FleetError::InvalidField("recovery limit"))
        ));

        // Three operations: one still live, two expired.
        let live_fence = fleet
            .acquire_fence("job-live", "worker-a", 10, 500)
            .unwrap();
        let dead_a = fleet.acquire_fence("job-a", "worker-a", 10, 500).unwrap();
        let dead_b = fleet.acquire_fence("job-b", "worker-a", 10, 500).unwrap();
        fleet.set_trusted_now_for_test(11);
        let live = fleet.acquire_fence_operation(&live_fence, 400).unwrap();
        let gone_a = fleet.acquire_fence_operation(&dead_a, 5).unwrap();
        let gone_b = fleet.acquire_fence_operation(&dead_b, 5).unwrap();
        fleet.set_trusted_now_for_test(50);

        // `limit + 1` probing reports more work without doing unbounded work.
        let first = fleet.recover_expired_fence_operations(1).unwrap();
        assert_eq!(first.removed(), 1);
        assert!(
            first.has_more(),
            "a second expired row must still be reported"
        );
        let second = fleet.recover_expired_fence_operations(1).unwrap();
        assert_eq!(second.removed(), 1);
        assert!(!second.has_more());
        let third = fleet.recover_expired_fence_operations(1).unwrap();
        assert_eq!(third.removed(), 0);
        assert!(!third.has_more());

        // The live operation survived the sweep.
        fleet.assert_fence_operation(&live).unwrap();
        assert_eq!(operation_row_count(&fleet), 1);
        {
            let connection = fleet.lock().unwrap();
            let fences: i64 = connection
                .query_row("SELECT COUNT(*) FROM fleet_fences", [], |row| row.get(0))
                .unwrap();
            assert_eq!(fences, 3, "recovery must never delete a fence");
        }
        assert!(matches!(
            fleet.release_fence_operation(gone_a),
            Err(FleetError::FenceOperationLost)
        ));
        assert!(matches!(
            fleet.release_fence_operation(gone_b),
            Err(FleetError::FenceOperationLost)
        ));
    }

    #[test]
    fn a_malformed_operation_row_is_corruption_and_is_never_swept_away() {
        let fleet = engine("pandora-fence-operation-malformed");
        fleet.set_trusted_now_for_test(10);
        inject_malformed_operation(&fleet);

        // Every read that could classify the row must refuse it. Treating it as
        // absent would let a fence mutation proceed past a registry that cannot
        // be read; treating it as expired would let a sweep delete it.
        assert!(matches!(
            fleet.acquire_fence("job-1", "worker-a", 10, 100),
            Err(FleetError::CorruptRecord)
        ));
        assert!(matches!(
            fleet.recover_expired_fence_operations(10),
            Err(FleetError::CorruptRecord)
        ));
        assert!(matches!(
            fleet.acquire_quiescence("evolution", 10, 100),
            Err(FleetError::CorruptRecord)
        ));
        assert_eq!(
            operation_row_count(&fleet),
            1,
            "a malformed row must not be deleted as a side effect of a sweep"
        );
    }

    #[test]
    fn a_v4_database_migrates_to_v5_without_losing_the_fleet() {
        let root = crate::test_support::new_temp_dir("pandora-fleet-migrate-v4").unwrap();
        let path = root.join("fleet.sqlite3");
        {
            let seed = FleetEngine::open(&path).unwrap();
            seed.set_trusted_now_for_test(10);
            seed.acquire_fence("job-1", "worker-a", 10, 100).unwrap();
        }
        // Build a genuine v4 database: a working one with only the v5 addition
        // removed. 4 is what the current release ships, so this is the upgrade
        // every existing install performs on first open after this lands.
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch("DROP TABLE fleet_fence_operations;")
                .unwrap();
            connection.pragma_update(None, "user_version", 4).unwrap();
        }

        let migrated = FleetEngine::open(&path).unwrap();
        assert_eq!(migrated.schema_version_for_test(), 5);
        migrated.set_trusted_now_for_test(11);

        // The pre-existing fence survived rather than being reset, so migration
        // did not quietly revoke live authority.
        let connection = migrated.lock().unwrap();
        let owners: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM fleet_fences WHERE fence_key = 'job-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(owners, 1);
        drop(connection);

        // And the table the migration exists to add is actually usable.
        let fence = migrated
            .acquire_fence("job-2", "worker-b", 11, 100)
            .unwrap();
        migrated.acquire_fence_operation(&fence, 5).unwrap();
        assert_eq!(operation_row_count(&migrated), 1);
    }

    #[test]
    fn a_future_schema_version_is_refused_and_left_exactly_as_it_was() {
        let root = crate::test_support::new_temp_dir("pandora-fleet-future-version").unwrap();
        let path = root.join("fleet.sqlite3");
        {
            FleetEngine::open(&path).unwrap();
        }
        {
            let connection = Connection::open(&path).unwrap();
            connection.pragma_update(None, "user_version", 6).unwrap();
        }

        // A database written by a newer build must not be opened, because this
        // build cannot know what invariants that version added.
        assert!(matches!(
            FleetEngine::open(&path),
            Err(FleetError::UnsupportedSchemaVersion)
        ));

        // Refusal must be inert: no downgrade, no partial migration, no repair
        // of a schema this build does not understand.
        let connection = Connection::open(&path).unwrap();
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 6, "a refused open must not rewrite the version");
    }

    #[test]
    fn a_migration_that_cannot_finish_leaves_no_half_applied_schema() {
        let root = crate::test_support::new_temp_dir("pandora-fleet-migrate-atomic").unwrap();
        let path = root.join("fleet.sqlite3");
        {
            FleetEngine::open(&path).unwrap();
        }
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch("DROP TABLE fleet_fence_operations;")
                .unwrap();
            connection.pragma_update(None, "user_version", 4).unwrap();
        }
        // Occupy the name the migration needs with something it cannot adopt.
        // `CREATE TABLE` is not `IF NOT EXISTS`, so the upgrade fails partway,
        // which is exactly the case that must leave nothing behind.
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch("CREATE TABLE fleet_fence_operations (decoy TEXT);")
                .unwrap();
        }

        assert!(FleetEngine::open(&path).is_err());

        // The version did not advance and the decoy was not rewritten. A
        // migration that stamped the new version in a *separate committed
        // transaction* before finishing would leave a database claiming v5 with a
        // v4 registry, and the next open would take the v5 branch and validate a
        // table that is not the real one.
        //
        // Scope of this guard, stated honestly: reordering `set_schema_version`
        // before `create_operation_schema` *within* the same transaction is not
        // caught here, and cannot be, because SQLite rolls the whole thing back
        // either way. I verified that by mutation. So this test pins the
        // observable end state of a failed upgrade; it does not by itself prove
        // the work is inside the transaction that `open` commits at the end.
        let connection = Connection::open(&path).unwrap();
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            version, 4,
            "a failed migration must not stamp the new version"
        );
        let columns: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('fleet_fence_operations')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(columns, 1, "the conflicting table must be left untouched");
    }

    #[test]
    fn quiescence_sees_live_operations_but_not_expired_ones() {
        let fleet = engine("pandora-fence-operation-quiescence");
        fleet.set_trusted_now_for_test(10);
        let fence = fleet.acquire_fence("job-1", "worker-a", 10, 500).unwrap();
        // Both fences must exist before quiescence is taken, because quiescence
        // also blocks new fence acquisition.
        let other = fleet.acquire_fence("job-2", "worker-b", 10, 500).unwrap();
        fleet.set_trusted_now_for_test(11);
        fleet.acquire_fence_operation(&fence, 5).unwrap();

        // An operation is work even with no lease outstanding, so an evolution
        // or catalog quiescence guard cannot slip past it.
        assert!(matches!(
            fleet.acquire_quiescence("evolution", 12, 100),
            Err(FleetError::ActiveFenceOperationsPresent)
        ));

        // Once quiescence is held, a new operation cannot start.
        fleet.set_trusted_now_for_test(100);
        let quiescence = fleet.acquire_quiescence("evolution", 100, 100).unwrap();
        assert!(matches!(
            fleet.acquire_fence_operation(&other, 5),
            Err(FleetError::QuiescenceActive)
        ));
        drop(quiescence);

        // Expired-but-unreaped rows are not active work, so quiescence may be
        // taken, and the row is still there afterwards to be reaped explicitly.
        assert!(
            fleet.acquire_quiescence("evolution", 200, 100).is_ok(),
            "an expired-but-unreaped operation is not active work, so quiescence \
             may be taken"
        );
        assert_eq!(operation_row_count(&fleet), 1);
    }
}
