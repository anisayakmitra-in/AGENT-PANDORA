use pandora_types::{EfficiencySample, ExecutionId, Timestamp};
use rusqlite::{Connection, TransactionBehavior, params};
use std::fmt;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

#[derive(Debug)]
pub enum EfficiencyStoreError {
    Database(rusqlite::Error),
    Io(std::io::Error),
    Contract(pandora_types::EfficiencyContractError),
    CorruptRecord,
    NumericOverflow,
    LockPoisoned,
}

impl fmt::Display for EfficiencyStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(_) => formatter.write_str("efficiency database operation failed"),
            Self::Io(_) => formatter.write_str("efficiency database directory operation failed"),
            Self::Contract(error) => error.fmt(formatter),
            Self::CorruptRecord => {
                formatter.write_str("efficiency database contains an invalid record")
            }
            Self::NumericOverflow => {
                formatter.write_str("efficiency metric exceeds storage limits")
            }
            Self::LockPoisoned => formatter.write_str("efficiency database lock is unavailable"),
        }
    }
}

impl std::error::Error for EfficiencyStoreError {}

impl From<rusqlite::Error> for EfficiencyStoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

impl From<std::io::Error> for EfficiencyStoreError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<pandora_types::EfficiencyContractError> for EfficiencyStoreError {
    fn from(error: pandora_types::EfficiencyContractError) -> Self {
        Self::Contract(error)
    }
}

pub struct EfficiencyStore {
    connection: Mutex<Connection>,
}

impl EfficiencyStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, EfficiencyStoreError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        set_private_permissions(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS efficiency_samples (
                 execution_id TEXT NOT NULL,
                 task_class TEXT NOT NULL,
                 target TEXT NOT NULL,
                 input_tokens INTEGER NOT NULL,
                 output_tokens INTEGER NOT NULL,
                 cost_micros INTEGER NOT NULL,
                 cost_known INTEGER NOT NULL CHECK (cost_known IN (0, 1)),
                 latency_ms INTEGER NOT NULL,
                 completed INTEGER NOT NULL CHECK (completed IN (0, 1)),
                 recorded_at INTEGER NOT NULL,
                 PRIMARY KEY (execution_id, task_class, target)
             );
             CREATE INDEX IF NOT EXISTS efficiency_task_idx
                 ON efficiency_samples(task_class, target, recorded_at);",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn record(
        &self,
        sample: &EfficiencySample,
        max_samples_per_target: usize,
    ) -> Result<(), EfficiencyStoreError> {
        let max_samples = i64::try_from(max_samples_per_target)
            .map_err(|_| EfficiencyStoreError::NumericOverflow)?;
        if max_samples == 0 {
            return Err(EfficiencyStoreError::NumericOverflow);
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO efficiency_samples (
                 execution_id, task_class, target, input_tokens, output_tokens,
                 cost_micros, cost_known, latency_ms, completed, recorded_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT (execution_id, task_class, target) DO UPDATE SET
                 input_tokens = excluded.input_tokens,
                 output_tokens = excluded.output_tokens,
                 cost_micros = excluded.cost_micros,
                 cost_known = excluded.cost_known,
                 latency_ms = excluded.latency_ms,
                 completed = excluded.completed,
                 recorded_at = excluded.recorded_at",
            params![
                sample.execution_id().as_str(),
                sample.task_class(),
                sample.target(),
                to_i64(sample.input_tokens())?,
                to_i64(sample.output_tokens())?,
                to_i64(sample.cost_micros())?,
                i64::from(sample.cost_known()),
                to_i64(sample.latency_ms())?,
                i64::from(sample.completed()),
                to_i64(sample.recorded_at().as_unix_seconds())?,
            ],
        )?;
        transaction.execute(
            "DELETE FROM efficiency_samples
             WHERE task_class = ?1 AND target = ?2
               AND rowid NOT IN (
                   SELECT rowid FROM efficiency_samples
                   WHERE task_class = ?1 AND target = ?2
                   ORDER BY recorded_at DESC, rowid DESC LIMIT ?3
               )",
            params![sample.task_class(), sample.target(), max_samples],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn load_task_class(
        &self,
        task_class: &str,
    ) -> Result<Vec<EfficiencySample>, EfficiencyStoreError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT execution_id, task_class, target, input_tokens, output_tokens,
                    cost_micros, cost_known, latency_ms, completed, recorded_at
             FROM efficiency_samples
             WHERE task_class = ?1
             ORDER BY recorded_at ASC, execution_id ASC",
        )?;
        let mut rows = statement.query(params![task_class])?;
        let mut samples = Vec::new();
        while let Some(row) = rows.next()? {
            samples.push(decode_sample(row)?);
        }
        Ok(samples)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, EfficiencyStoreError> {
        self.connection
            .lock()
            .map_err(|_| EfficiencyStoreError::LockPoisoned)
    }
}

fn decode_sample(row: &rusqlite::Row<'_>) -> Result<EfficiencySample, EfficiencyStoreError> {
    let execution_id = ExecutionId::new(row.get::<_, String>(0)?)
        .map_err(|_| EfficiencyStoreError::CorruptRecord)?;
    let task_class = row.get::<_, String>(1)?;
    let target = row.get::<_, String>(2)?;
    let input_tokens = to_u64(row.get::<_, i64>(3)?)?;
    let output_tokens = to_u64(row.get::<_, i64>(4)?)?;
    let cost_micros = to_u64(row.get::<_, i64>(5)?)?;
    let cost_known = row.get::<_, i64>(6)?;
    let latency_ms = to_u64(row.get::<_, i64>(7)?)?;
    let completed = row.get::<_, i64>(8)?;
    let recorded_at = Timestamp::from_unix_seconds(to_u64(row.get::<_, i64>(9)?)?);
    match (cost_known, completed) {
        (0, 0 | 1) => EfficiencySample::new_without_cost(
            execution_id,
            task_class,
            target,
            input_tokens,
            output_tokens,
            latency_ms,
            completed == 1,
            recorded_at,
        )
        .map_err(Into::into),
        (1, 0 | 1) => EfficiencySample::new(
            execution_id,
            task_class,
            target,
            input_tokens,
            output_tokens,
            cost_micros,
            latency_ms,
            completed == 1,
            recorded_at,
        )
        .map_err(Into::into),
        _ => Err(EfficiencyStoreError::CorruptRecord),
    }
}

fn to_i64(value: u64) -> Result<i64, EfficiencyStoreError> {
    i64::try_from(value).map_err(|_| EfficiencyStoreError::NumericOverflow)
}

fn to_u64(value: i64) -> Result<u64, EfficiencyStoreError> {
    u64::try_from(value).map_err(|_| EfficiencyStoreError::CorruptRecord)
}

fn set_private_permissions(path: &Path) -> Result<(), EfficiencyStoreError> {
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
    use pandora_types::EfficiencySample;

    #[test]
    fn records_and_loads_bounded_samples() {
        let root = crate::test_support::new_temp_dir("pandora-efficiency-store").unwrap();
        let store = EfficiencyStore::open(root.join("efficiency.sqlite3")).unwrap();
        let sample = EfficiencySample::new(
            ExecutionId::new("execution-1").unwrap(),
            "coding",
            "provider-a",
            10,
            5,
            25,
            100,
            true,
            Timestamp::from_unix_seconds(1),
        )
        .unwrap();
        store.record(&sample, 2).unwrap();

        let loaded = store.load_task_class("coding").unwrap();
        assert_eq!(loaded, vec![sample]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn trims_oldest_target_evidence() {
        let root = crate::test_support::new_temp_dir("pandora-efficiency-trim").unwrap();
        let store = EfficiencyStore::open(root.join("efficiency.sqlite3")).unwrap();
        for (id, at) in [("one", 1), ("two", 2), ("three", 3)] {
            let sample = EfficiencySample::new(
                ExecutionId::new(id).unwrap(),
                "coding",
                "provider-a",
                10,
                5,
                25,
                100,
                true,
                Timestamp::from_unix_seconds(at),
            )
            .unwrap();
            store.record(&sample, 2).unwrap();
        }

        let loaded = store.load_task_class("coding").unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].execution_id().as_str(), "two");
        assert_eq!(loaded[1].execution_id().as_str(), "three");
        let _ = std::fs::remove_dir_all(root);
    }
}
