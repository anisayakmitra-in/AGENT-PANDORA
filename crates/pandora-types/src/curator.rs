use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CuratorPassResult {
    pub started_at: u64,
    pub completed_at: u64,
    pub sessions_evaluated: usize,
    pub sops_crystallized: usize,
    pub sops_promoted: usize,
    pub stale_supervisors_reaped: usize,
    pub wal_checkpointed: bool,
    pub database_size_before: u64,
    pub database_size_after: u64,
    pub reclaimed_bytes: u64,
    pub duration_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CuratorStatus {
    pub total_sessions: usize,
    pub crystallized_sops: usize,
    pub total_supervisors: usize,
    pub database_size_bytes: u64,
    pub wal_size_bytes: u64,
    pub last_curation_timestamp: Option<u64>,
}
