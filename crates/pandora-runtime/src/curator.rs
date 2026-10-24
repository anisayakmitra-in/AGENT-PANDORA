use crate::evolution::BayesianCanaryGate;
use crate::fleet::FleetEngine;
use crate::reflexion::ReflexionCrystallizer;
use crate::sessions::SessionStore;
use pandora_types::{
    CuratorPassResult, CuratorStatus, PrincipalId, TenantId, Timestamp, WorkspaceId,
};
use rusqlite::Connection;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub enum CuratorError {
    Database(String),
    Reflexion(String),
    Fleet(String),
    Io(String),
}

impl fmt::Display for CuratorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(err) => write!(formatter, "database error: {err}"),
            Self::Reflexion(err) => write!(formatter, "reflexion error: {err}"),
            Self::Fleet(err) => write!(formatter, "fleet error: {err}"),
            Self::Io(err) => write!(formatter, "io error: {err}"),
        }
    }
}

impl std::error::Error for CuratorError {}

/// The Autonomous Curator Daemon.
///
/// Periodically inspects past sessions, auto-crystallizes candidate SOPs,
/// evaluates Bayesian canary gates, reaps stale supervisor leases, and
/// performs SQLite WAL checkpoints and index optimizations.
pub struct CuratorEngine {
    sessions_db_path: PathBuf,
    fleet_db_path: PathBuf,
    crystallizer: ReflexionCrystallizer,
}

impl CuratorEngine {
    pub fn new(sessions_db_path: impl Into<PathBuf>, fleet_db_path: impl Into<PathBuf>) -> Self {
        Self {
            sessions_db_path: sessions_db_path.into(),
            fleet_db_path: fleet_db_path.into(),
            crystallizer: ReflexionCrystallizer::new(BayesianCanaryGate::new(0.95)),
        }
    }

    pub fn with_crystallizer(mut self, crystallizer: ReflexionCrystallizer) -> Self {
        self.crystallizer = crystallizer;
        self
    }

    fn now_epoch_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    fn get_file_size(path: &Path) -> u64 {
        std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
    }

    fn wal_path_for(db_path: &Path) -> PathBuf {
        let mut path_str = db_path.as_os_str().to_os_string();
        path_str.push("-wal");
        PathBuf::from(path_str)
    }

    /// Queries the current curation and database health status.
    pub fn status(
        &self,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
    ) -> Result<CuratorStatus, CuratorError> {
        let store = SessionStore::open(&self.sessions_db_path)
            .map_err(|e| CuratorError::Database(e.to_string()))?;

        let archives = store
            .list_recent_archives(tenant_id, workspace_id, 1024)
            .unwrap_or_default();
        let sops = store
            .list_sops_by_intent(tenant_id, workspace_id, None)
            .unwrap_or_default();

        let db_size = Self::get_file_size(&self.sessions_db_path);
        let wal_path = Self::wal_path_for(&self.sessions_db_path);
        let wal_size = Self::get_file_size(&wal_path);

        let fleet_engine = FleetEngine::open(&self.fleet_db_path)
            .map_err(|e| CuratorError::Fleet(e.to_string()))?;
        let supervisors = fleet_engine.list_supervisors().unwrap_or_default();

        Ok(CuratorStatus {
            total_sessions: archives.len(),
            crystallized_sops: sops.len(),
            total_supervisors: supervisors.len(),
            database_size_bytes: db_size,
            wal_size_bytes: wal_size,
            last_curation_timestamp: archives.first().map(|a| a.created_at.as_unix_seconds()),
        })
    }

    /// Performs a single comprehensive maintenance pass across memory crystallization,
    /// stale lease reaping, and SQLite WAL compaction.
    pub fn perform_maintenance_pass(
        &self,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
        workspace_root: Option<&str>,
    ) -> Result<CuratorPassResult, CuratorError> {
        let start_time = Instant::now();
        let started_at = Self::now_epoch_secs();

        let db_size_before = Self::get_file_size(&self.sessions_db_path)
            + Self::get_file_size(&Self::wal_path_for(&self.sessions_db_path));

        // 1. Crystallize all eligible session archives
        let store = SessionStore::open(&self.sessions_db_path)
            .map_err(|e| CuratorError::Database(e.to_string()))?;

        let timestamp = Timestamp::from_unix_seconds(started_at);
        let crystallization_results = self
            .crystallizer
            .crystallize_all(
                &store,
                principal_id,
                tenant_id,
                workspace_id,
                0.50,
                workspace_root,
                timestamp,
            )
            .map_err(|e| CuratorError::Reflexion(e.to_string()))?;

        let sessions_evaluated = crystallization_results.len();
        let sops_crystallized = crystallization_results.len();
        let sops_promoted = crystallization_results
            .iter()
            .filter(|r| r.promoted)
            .count();

        // 2. Reap stale supervisors in fleet
        let fleet_engine = FleetEngine::open(&self.fleet_db_path)
            .map_err(|e| CuratorError::Fleet(e.to_string()))?;

        // Consider supervisors inactive for >60s stale
        let reaped = fleet_engine
            .reap_stale_supervisors(started_at, 60)
            .unwrap_or_default();
        let stale_supervisors_reaped = reaped.len();

        // 3. Perform SQLite WAL checkpointing and optimize
        let wal_checkpointed = if self.sessions_db_path.exists() {
            Connection::open(&self.sessions_db_path)
                .map(|conn| {
                    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA optimize;");
                    true
                })
                .unwrap_or(false)
        } else {
            false
        };

        if self.fleet_db_path.exists() {
            let _ = Connection::open(&self.fleet_db_path).map(|conn| {
                conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA optimize;")
            });
        }

        let db_size_after = Self::get_file_size(&self.sessions_db_path)
            + Self::get_file_size(&Self::wal_path_for(&self.sessions_db_path));

        let reclaimed_bytes = db_size_before.saturating_sub(db_size_after);
        let completed_at = Self::now_epoch_secs();
        let duration_ms = start_time.elapsed().as_millis() as u64;

        Ok(CuratorPassResult {
            started_at,
            completed_at,
            sessions_evaluated,
            sops_crystallized,
            sops_promoted,
            stale_supervisors_reaped,
            wal_checkpointed,
            database_size_before: db_size_before,
            database_size_after: db_size_after,
            reclaimed_bytes,
            duration_ms,
        })
    }

    /// Runs continuous curation passes in a loop with an idle sleep interval.
    pub fn run_watch(
        &self,
        principal_id: &PrincipalId,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
        workspace_root: Option<&str>,
        interval: Duration,
        max_passes: Option<usize>,
    ) -> Result<Vec<CuratorPassResult>, CuratorError> {
        let mut passes = Vec::new();
        let mut pass_count = 0;

        loop {
            let result = self.perform_maintenance_pass(
                principal_id,
                tenant_id,
                workspace_id,
                workspace_root,
            )?;
            passes.push(result);
            pass_count += 1;

            if max_passes.is_some_and(|max| pass_count >= max) {
                break;
            }

            std::thread::sleep(interval);
        }

        Ok(passes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_curator_pass_and_status() {
        let temp = crate::test_support::new_temp_dir("pandora-curator-test").unwrap();
        let sessions_path = temp.join("sessions.sqlite3");
        let fleet_path = temp.join("fleet.sqlite3");

        let curator = CuratorEngine::new(&sessions_path, &fleet_path);
        let principal = PrincipalId::new("test-principal").unwrap();
        let tenant = TenantId::new("test-tenant").unwrap();
        let workspace = WorkspaceId::new("test-workspace").unwrap();

        // 1. Check initial status
        let status = curator.status(&tenant, &workspace).unwrap();
        assert_eq!(status.total_sessions, 0);
        assert_eq!(status.crystallized_sops, 0);

        // 2. Run maintenance pass
        let pass = curator
            .perform_maintenance_pass(&principal, &tenant, &workspace, None)
            .unwrap();
        assert!(pass.completed_at >= pass.started_at);
        assert_eq!(pass.sessions_evaluated, 0);

        // 3. Run bounded watch passes
        let watch_passes = curator
            .run_watch(
                &principal,
                &tenant,
                &workspace,
                None,
                Duration::from_millis(10),
                Some(2),
            )
            .unwrap();
        assert_eq!(watch_passes.len(), 2);
    }
}
