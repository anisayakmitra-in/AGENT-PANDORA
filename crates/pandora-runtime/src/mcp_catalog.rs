use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::mcp::McpWireEra;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum McpCatalogError {
    AlreadyActive,
    AlreadyActivated,
    InvalidIdentity,
    GenerationExhausted,
    ReservationLost,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpCatalogTool {
    local_id: String,
    remote_name: String,
    schema_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpCatalogRevision {
    server_id: String,
    generation: u64,
    protocol_era: McpWireEra,
    process_id: u32,
    config_digest: String,
    catalog_digest: String,
    tools: Vec<McpCatalogTool>,
}

impl McpCatalogRevision {
    pub fn server_id(&self) -> &str {
        &self.server_id
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub const fn protocol_era(&self) -> McpWireEra {
        self.protocol_era
    }

    pub const fn process_id(&self) -> u32 {
        self.process_id
    }

    pub fn config_digest(&self) -> &str {
        &self.config_digest
    }

    pub fn catalog_digest(&self) -> &str {
        &self.catalog_digest
    }

    pub fn tools(&self) -> &[McpCatalogTool] {
        &self.tools
    }

    pub fn tool(&self, local_id: &str) -> Option<&McpCatalogTool> {
        self.tools.iter().find(|tool| tool.local_id == local_id)
    }
}

impl McpCatalogTool {
    pub fn new(
        local_id: impl Into<String>,
        remote_name: impl Into<String>,
        schema: &Value,
    ) -> Result<Self, McpCatalogError> {
        let local_id = validate_identity(local_id.into())?;
        let remote_name = validate_identity(remote_name.into())?;
        Ok(Self {
            local_id,
            remote_name,
            schema_digest: digest_json(schema),
        })
    }

    pub fn local_id(&self) -> &str {
        &self.local_id
    }

    pub fn remote_name(&self) -> &str {
        &self.remote_name
    }

    pub fn schema_digest(&self) -> &str {
        &self.schema_digest
    }
}

#[derive(Debug, Default)]
struct CatalogState {
    next_generation: u64,
    active: HashMap<String, u64>,
}

#[derive(Clone, Debug, Default)]
pub struct McpCatalogSupervisor {
    state: Arc<Mutex<CatalogState>>,
}

impl McpCatalogSupervisor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reserve(
        &self,
        server_id: impl Into<String>,
        config_digest: impl Into<String>,
    ) -> Result<McpCatalogReservation, McpCatalogError> {
        let server_id = validate_identity(server_id.into())?;
        let config_digest = validate_identity(config_digest.into())?;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.active.contains_key(&server_id) {
            return Err(McpCatalogError::AlreadyActive);
        }
        state.next_generation = state
            .next_generation
            .checked_add(1)
            .ok_or(McpCatalogError::GenerationExhausted)?;
        let generation = state.next_generation;
        state.active.insert(server_id.clone(), generation);
        Ok(McpCatalogReservation {
            inner: Arc::new(ReservationInner {
                supervisor: self.clone(),
                server_id,
                generation,
                config_digest,
                revision: Mutex::new(None),
            }),
        })
    }

    pub fn is_active(&self, server_id: &str) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .active
            .contains_key(server_id)
    }

    pub(crate) fn release(&self, server_id: &str, generation: u64) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.active.get(server_id) != Some(&generation) {
            return false;
        }
        state.active.remove(server_id);
        true
    }
}

#[derive(Debug)]
struct ReservationInner {
    supervisor: McpCatalogSupervisor,
    server_id: String,
    generation: u64,
    config_digest: String,
    revision: Mutex<Option<McpCatalogRevision>>,
}

impl Drop for ReservationInner {
    fn drop(&mut self) {
        self.supervisor.release(&self.server_id, self.generation);
    }
}

#[derive(Clone, Debug)]
pub struct McpCatalogReservation {
    inner: Arc<ReservationInner>,
}

impl McpCatalogReservation {
    pub fn server_id(&self) -> &str {
        &self.inner.server_id
    }

    pub fn generation(&self) -> u64 {
        self.inner.generation
    }

    pub fn config_digest(&self) -> &str {
        &self.inner.config_digest
    }

    pub(crate) fn activate(
        &self,
        protocol_era: McpWireEra,
        process_id: u32,
        mut tools: Vec<McpCatalogTool>,
    ) -> Result<McpCatalogRevision, McpCatalogError> {
        if process_id == 0 {
            return Err(McpCatalogError::InvalidIdentity);
        }
        let state = self
            .inner
            .supervisor
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.active.get(&self.inner.server_id) != Some(&self.inner.generation) {
            return Err(McpCatalogError::ReservationLost);
        }
        drop(state);
        tools.sort_by(|left, right| left.local_id.cmp(&right.local_id));
        let revision = McpCatalogRevision {
            server_id: self.inner.server_id.clone(),
            generation: self.inner.generation,
            protocol_era,
            process_id,
            config_digest: self.inner.config_digest.clone(),
            catalog_digest: catalog_digest(&tools),
            tools,
        };
        let mut stored = self
            .inner
            .revision
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if stored.is_some() {
            return Err(McpCatalogError::AlreadyActivated);
        }
        *stored = Some(revision.clone());
        Ok(revision)
    }
}

pub(crate) fn catalog_digest(tools: &[McpCatalogTool]) -> String {
    let mut ordered = tools.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        left.local_id
            .cmp(&right.local_id)
            .then_with(|| left.remote_name.cmp(&right.remote_name))
    });
    let mut hasher = Sha256::new();
    digest_text(&mut hasher, "pandora.mcp.catalog.v1");
    for tool in ordered {
        digest_text(&mut hasher, &tool.local_id);
        digest_text(&mut hasher, &tool.remote_name);
        digest_text(&mut hasher, &tool.schema_digest);
    }
    encode_hex(hasher.finalize().as_slice())
}

fn digest_json(value: &Value) -> String {
    let mut hasher = Sha256::new();
    hash_json(&mut hasher, value);
    encode_hex(hasher.finalize().as_slice())
}

pub(crate) fn digest_bytes(bytes: &[u8]) -> String {
    encode_hex(Sha256::digest(bytes).as_slice())
}

fn hash_json(hasher: &mut Sha256, value: &Value) {
    match value {
        Value::Null => hasher.update([0]),
        Value::Bool(value) => hasher.update([1, u8::from(*value)]),
        Value::Number(value) => {
            hasher.update([2]);
            digest_text(hasher, &value.to_string());
        }
        Value::String(value) => {
            hasher.update([3]);
            digest_text(hasher, value);
        }
        Value::Array(values) => {
            hasher.update([4]);
            hasher.update(values.len().to_be_bytes());
            for value in values {
                hash_json(hasher, value);
            }
        }
        Value::Object(values) => {
            hasher.update([5]);
            hasher.update(values.len().to_be_bytes());
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(key, _)| *key);
            for (key, value) in entries {
                digest_text(hasher, key);
                hash_json(hasher, value);
            }
        }
    }
}

fn digest_text(hasher: &mut Sha256, value: &str) {
    hasher.update(value.len().to_be_bytes());
    hasher.update(value.as_bytes());
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn validate_identity(value: String) -> Result<String, McpCatalogError> {
    if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(McpCatalogError::InvalidIdentity);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn catalog_rejects_duplicate_active_server_id() {
        let supervisor = McpCatalogSupervisor::new();
        let first = supervisor.reserve("local", "config-a").unwrap();

        assert_eq!(first.generation(), 1);
        assert_eq!(
            supervisor.reserve("local", "config-a").unwrap_err(),
            McpCatalogError::AlreadyActive
        );
    }

    #[test]
    fn catalog_releases_only_the_matching_revision() {
        let supervisor = McpCatalogSupervisor::new();
        let first = supervisor.reserve("local", "config-a").unwrap();
        let generation = first.generation();

        assert!(!supervisor.release("local", generation + 1));
        assert!(supervisor.is_active("local"));
        drop(first);
        assert!(!supervisor.is_active("local"));
    }

    #[test]
    fn reconnect_creates_a_new_revision() {
        let supervisor = McpCatalogSupervisor::new();
        let first = supervisor.reserve("local", "config-a").unwrap();
        assert_eq!(first.generation(), 1);
        drop(first);

        let second = supervisor.reserve("local", "config-a").unwrap();
        assert_eq!(second.generation(), 2);
    }

    #[test]
    fn catalog_and_schema_digests_are_deterministic() {
        let left = McpCatalogTool::new(
            "mcp.local.echo",
            "echo",
            &json!({
                "type": "object",
                "properties": {"value": {"type": "string"}},
                "required": ["value"],
                "additionalProperties": false
            }),
        )
        .unwrap();
        let right = McpCatalogTool::new(
            "mcp.local.echo",
            "echo",
            &json!({
                "required": ["value"],
                "additionalProperties": false,
                "properties": {"value": {"type": "string"}},
                "type": "object"
            }),
        )
        .unwrap();
        let changed = McpCatalogTool::new(
            "mcp.local.echo",
            "echo",
            &json!({
                "type": "object",
                "properties": {"value": {"type": "integer"}},
                "required": ["value"],
                "additionalProperties": false
            }),
        )
        .unwrap();

        assert_eq!(left.schema_digest(), right.schema_digest());
        assert_ne!(left.schema_digest(), changed.schema_digest());
        assert_eq!(catalog_digest(&[left]), catalog_digest(&[right]));
    }
}
