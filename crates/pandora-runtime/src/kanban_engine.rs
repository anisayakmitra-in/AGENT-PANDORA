use pandora_types::{KanbanBoard, KanbanReclaimReport, KanbanStatus, KanbanTask};
use rusqlite::{Connection, params};
use std::fmt;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub enum KanbanError {
    Database(String),
    TaskNotFound(String),
    InvalidDependency(String),
    Validation(String),
}

impl fmt::Display for KanbanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(err) => write!(formatter, "database error: {err}"),
            Self::TaskNotFound(id) => write!(formatter, "kanban task '{id}' not found"),
            Self::InvalidDependency(err) => write!(formatter, "dependency violation: {err}"),
            Self::Validation(err) => write!(formatter, "validation error: {err}"),
        }
    }
}

impl std::error::Error for KanbanError {}

pub struct KanbanEngine {
    db_path: PathBuf,
}

impl KanbanEngine {
    pub fn new(db_path: impl Into<PathBuf>) -> Result<Self, KanbanError> {
        let db_path = db_path.into();
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| KanbanError::Database(format!("failed to create dir: {e}")))?;
        }
        let conn = Connection::open(&db_path)
            .map_err(|e| KanbanError::Database(format!("failed to open sqlite: {e}")))?;

        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS kanban_tasks (
                 id TEXT PRIMARY KEY,
                 board_id TEXT NOT NULL,
                 title TEXT NOT NULL,
                 description TEXT NOT NULL,
                 status TEXT NOT NULL,
                 priority INTEGER NOT NULL,
                 assignee TEXT,
                 dependencies_json TEXT NOT NULL,
                 retry_count INTEGER NOT NULL,
                 max_retries INTEGER NOT NULL,
                 last_heartbeat_epoch_secs INTEGER,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 labels_json TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_kanban_board ON kanban_tasks(board_id, status);",
        )
        .map_err(|e| KanbanError::Database(format!("failed to init schema: {e}")))?;

        Ok(Self { db_path })
    }

    fn open(&self) -> Result<Connection, KanbanError> {
        Connection::open(&self.db_path)
            .map_err(|e| KanbanError::Database(format!("failed to open db: {e}")))
    }

    fn now_epoch_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    pub fn create_task(&self, task: KanbanTask) -> Result<KanbanTask, KanbanError> {
        if task.id.trim().is_empty() {
            return Err(KanbanError::Validation(
                "task id cannot be empty".to_string(),
            ));
        }
        if task.title.trim().is_empty() {
            return Err(KanbanError::Validation(
                "task title cannot be empty".to_string(),
            ));
        }

        let conn = self.open()?;
        let deps_json =
            serde_json::to_string(&task.dependencies).unwrap_or_else(|_| "[]".to_string());
        let labels_json = serde_json::to_string(&task.labels).unwrap_or_else(|_| "[]".to_string());

        conn.execute(
            "INSERT INTO kanban_tasks (
                id, board_id, title, description, status, priority, assignee,
                dependencies_json, retry_count, max_retries, last_heartbeat_epoch_secs,
                created_at, updated_at, labels_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                task.id,
                task.board_id,
                task.title,
                task.description,
                task.status.as_str(),
                task.priority,
                task.assignee,
                deps_json,
                task.retry_count,
                task.max_retries,
                task.last_heartbeat_epoch_secs,
                task.created_at,
                task.updated_at,
                labels_json,
            ],
        )
        .map_err(|e| KanbanError::Database(format!("failed to insert task: {e}")))?;

        Ok(task)
    }

    pub fn get_task(&self, task_id: &str) -> Result<Option<KanbanTask>, KanbanError> {
        let conn = self.open()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, board_id, title, description, status, priority, assignee,
                        dependencies_json, retry_count, max_retries, last_heartbeat_epoch_secs,
                        created_at, updated_at, labels_json
                 FROM kanban_tasks WHERE id = ?1",
            )
            .map_err(|e| KanbanError::Database(format!("prepare failed: {e}")))?;

        let mut rows = stmt
            .query(params![task_id])
            .map_err(|e| KanbanError::Database(format!("query failed: {e}")))?;

        if let Some(row) = rows
            .next()
            .map_err(|e| KanbanError::Database(e.to_string()))?
        {
            let status_str: String = row.get(4).unwrap_or_default();
            let status = KanbanStatus::parse(&status_str).unwrap_or(KanbanStatus::Todo);
            let deps_json: String = row.get(7).unwrap_or_else(|_| "[]".to_string());
            let dependencies: Vec<String> = serde_json::from_str(&deps_json).unwrap_or_default();
            let labels_json: String = row.get(13).unwrap_or_else(|_| "[]".to_string());
            let labels: Vec<String> = serde_json::from_str(&labels_json).unwrap_or_default();

            Ok(Some(KanbanTask {
                id: row
                    .get(0)
                    .map_err(|e| KanbanError::Database(e.to_string()))?,
                board_id: row
                    .get(1)
                    .map_err(|e| KanbanError::Database(e.to_string()))?,
                title: row
                    .get(2)
                    .map_err(|e| KanbanError::Database(e.to_string()))?,
                description: row
                    .get(3)
                    .map_err(|e| KanbanError::Database(e.to_string()))?,
                status,
                priority: row
                    .get(5)
                    .map_err(|e| KanbanError::Database(e.to_string()))?,
                assignee: row
                    .get(6)
                    .map_err(|e| KanbanError::Database(e.to_string()))?,
                dependencies,
                retry_count: row
                    .get(8)
                    .map_err(|e| KanbanError::Database(e.to_string()))?,
                max_retries: row
                    .get(9)
                    .map_err(|e| KanbanError::Database(e.to_string()))?,
                last_heartbeat_epoch_secs: row
                    .get(10)
                    .map_err(|e| KanbanError::Database(e.to_string()))?,
                created_at: row
                    .get(11)
                    .map_err(|e| KanbanError::Database(e.to_string()))?,
                updated_at: row
                    .get(12)
                    .map_err(|e| KanbanError::Database(e.to_string()))?,
                labels,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn list_tasks(
        &self,
        board_id: &str,
        status_filter: Option<KanbanStatus>,
    ) -> Result<Vec<KanbanTask>, KanbanError> {
        let conn = self.open()?;
        let query = if status_filter.is_some() {
            "SELECT id, board_id, title, description, status, priority, assignee,
                    dependencies_json, retry_count, max_retries, last_heartbeat_epoch_secs,
                    created_at, updated_at, labels_json
             FROM kanban_tasks WHERE board_id = ?1 AND status = ?2
             ORDER BY priority ASC, created_at ASC"
        } else {
            "SELECT id, board_id, title, description, status, priority, assignee,
                    dependencies_json, retry_count, max_retries, last_heartbeat_epoch_secs,
                    created_at, updated_at, labels_json
             FROM kanban_tasks WHERE board_id = ?1
             ORDER BY priority ASC, created_at ASC"
        };

        let mut stmt = conn
            .prepare(query)
            .map_err(|e| KanbanError::Database(format!("prepare failed: {e}")))?;

        let mut tasks = Vec::new();
        let rows_iter = if let Some(status) = status_filter {
            stmt.query_map(params![board_id, status.as_str()], Self::map_row)
        } else {
            stmt.query_map(params![board_id], Self::map_row)
        }
        .map_err(|e| KanbanError::Database(e.to_string()))?;

        for task_res in rows_iter {
            let task = task_res.map_err(|e| KanbanError::Database(e.to_string()))?;
            tasks.push(task);
        }

        Ok(tasks)
    }

    fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<KanbanTask> {
        let status_str: String = row.get(4)?;
        let status = KanbanStatus::parse(&status_str).unwrap_or(KanbanStatus::Todo);
        let deps_json: String = row.get(7).unwrap_or_else(|_| "[]".to_string());
        let dependencies: Vec<String> = serde_json::from_str(&deps_json).unwrap_or_default();
        let labels_json: String = row.get(13).unwrap_or_else(|_| "[]".to_string());
        let labels: Vec<String> = serde_json::from_str(&labels_json).unwrap_or_default();

        Ok(KanbanTask {
            id: row.get(0)?,
            board_id: row.get(1)?,
            title: row.get(2)?,
            description: row.get(3)?,
            status,
            priority: row.get(5)?,
            assignee: row.get(6)?,
            dependencies,
            retry_count: row.get(8)?,
            max_retries: row.get(9)?,
            last_heartbeat_epoch_secs: row.get(10)?,
            created_at: row.get(11)?,
            updated_at: row.get(12)?,
            labels,
        })
    }

    pub fn update_status(
        &self,
        task_id: &str,
        new_status: KanbanStatus,
        worker: Option<&str>,
    ) -> Result<KanbanTask, KanbanError> {
        let mut task = self
            .get_task(task_id)?
            .ok_or_else(|| KanbanError::TaskNotFound(task_id.to_string()))?;

        // Dependency check when starting task
        if new_status == KanbanStatus::InProgress && !task.dependencies.is_empty() {
            for dep in &task.dependencies {
                let dep_task = self.get_task(dep)?;
                match dep_task {
                    Some(d) if d.status == KanbanStatus::Done => {}
                    Some(_) => {
                        return Err(KanbanError::InvalidDependency(format!(
                            "dependency task '{dep}' is not completed yet"
                        )));
                    }
                    None => {
                        return Err(KanbanError::InvalidDependency(format!(
                            "dependency task '{dep}' does not exist"
                        )));
                    }
                }
            }
        }

        let now = Self::now_epoch_secs();
        task.status = new_status;
        task.updated_at = now;
        if let Some(w) = worker {
            task.assignee = Some(w.to_string());
        }
        if new_status == KanbanStatus::InProgress {
            task.last_heartbeat_epoch_secs = Some(now);
        }

        let conn = self.open()?;
        conn.execute(
            "UPDATE kanban_tasks SET status = ?1, assignee = ?2, last_heartbeat_epoch_secs = ?3, updated_at = ?4 WHERE id = ?5",
            params![
                task.status.as_str(),
                task.assignee,
                task.last_heartbeat_epoch_secs,
                task.updated_at,
                task.id,
            ],
        )
        .map_err(|e| KanbanError::Database(format!("update failed: {e}")))?;

        Ok(task)
    }

    pub fn heartbeat(&self, task_id: &str, worker: &str) -> Result<KanbanTask, KanbanError> {
        let mut task = self
            .get_task(task_id)?
            .ok_or_else(|| KanbanError::TaskNotFound(task_id.to_string()))?;

        if task.status != KanbanStatus::InProgress {
            return Err(KanbanError::Validation(format!(
                "cannot heartbeat task '{task_id}' with status '{:?}' (must be in_progress)",
                task.status
            )));
        }

        let now = Self::now_epoch_secs();
        task.last_heartbeat_epoch_secs = Some(now);
        task.updated_at = now;
        task.assignee = Some(worker.to_string());

        let conn = self.open()?;
        conn.execute(
            "UPDATE kanban_tasks SET last_heartbeat_epoch_secs = ?1, updated_at = ?2, assignee = ?3 WHERE id = ?4",
            params![task.last_heartbeat_epoch_secs, task.updated_at, task.assignee, task.id],
        )
        .map_err(|e| KanbanError::Database(format!("heartbeat update failed: {e}")))?;

        Ok(task)
    }

    pub fn reclaim_zombies(
        &self,
        board_id: &str,
        timeout_secs: u64,
    ) -> Result<KanbanReclaimReport, KanbanError> {
        let tasks = self.list_tasks(board_id, Some(KanbanStatus::InProgress))?;
        let now = Self::now_epoch_secs();

        let mut reclaimed_task_ids = Vec::new();
        let mut blocked_exhausted_ids = Vec::new();

        let conn = self.open()?;

        for mut task in tasks {
            if task.is_zombie(now, timeout_secs) {
                task.retry_count += 1;
                task.updated_at = now;
                task.last_heartbeat_epoch_secs = None;

                if task.retry_count >= task.max_retries {
                    task.status = KanbanStatus::Blocked;
                    if !task.labels.contains(&"zombie_exhausted".to_string()) {
                        task.labels.push("zombie_exhausted".to_string());
                    }
                    blocked_exhausted_ids.push(task.id.clone());
                } else {
                    task.status = KanbanStatus::Todo;
                    reclaimed_task_ids.push(task.id.clone());
                }

                let labels_json =
                    serde_json::to_string(&task.labels).unwrap_or_else(|_| "[]".to_string());

                conn.execute(
                    "UPDATE kanban_tasks
                     SET status = ?1, retry_count = ?2, last_heartbeat_epoch_secs = NULL, updated_at = ?3, labels_json = ?4
                     WHERE id = ?5",
                    params![
                        task.status.as_str(),
                        task.retry_count,
                        task.updated_at,
                        labels_json,
                        task.id
                    ],
                )
                .map_err(|e| KanbanError::Database(format!("reclaim update failed: {e}")))?;
            }
        }

        Ok(KanbanReclaimReport {
            reclaimed_task_ids,
            blocked_exhausted_ids,
        })
    }

    pub fn delete_task(&self, task_id: &str) -> Result<bool, KanbanError> {
        let conn = self.open()?;
        let count = conn
            .execute("DELETE FROM kanban_tasks WHERE id = ?1", params![task_id])
            .map_err(|e| KanbanError::Database(format!("delete failed: {e}")))?;
        Ok(count > 0)
    }

    pub fn get_board(&self, board_id: &str) -> Result<KanbanBoard, KanbanError> {
        let tasks = self.list_tasks(board_id, None)?;
        Ok(KanbanBoard {
            id: board_id.to_string(),
            name: format!("Board {board_id}"),
            description: "Durable multi-agent task execution board".to_string(),
            tasks,
            created_at: Self::now_epoch_secs(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kanban_lifecycle_and_zombie_reclaim() {
        let temp = crate::test_support::new_temp_dir("pandora-kanban-test").unwrap();
        let db_path = temp.join("kanban.sqlite3");
        let engine = KanbanEngine::new(&db_path).unwrap();

        // 1. Create root task and dependent task
        let root = KanbanTask::new(
            "t-0",
            "b-main",
            "Plan migration",
            "High-level design",
            1,
            None,
            vec![],
            100,
        );
        let dep = KanbanTask::new(
            "t-1",
            "b-main",
            "Execute phase 1",
            "Run steps",
            2,
            None,
            vec!["t-0".to_string()],
            100,
        );

        engine.create_task(root).unwrap();
        engine.create_task(dep).unwrap();

        // 2. Starting t-1 fails because t-0 is not done
        let err = engine.update_status("t-1", KanbanStatus::InProgress, Some("worker-1"));
        assert!(matches!(err, Err(KanbanError::InvalidDependency(_))));

        // 3. Complete t-0
        engine
            .update_status("t-0", KanbanStatus::InProgress, Some("planner"))
            .unwrap();
        engine
            .update_status("t-0", KanbanStatus::Done, Some("planner"))
            .unwrap();

        // 4. Now t-1 can be started
        let in_prog = engine
            .update_status("t-1", KanbanStatus::InProgress, Some("worker-1"))
            .unwrap();
        assert_eq!(in_prog.status, KanbanStatus::InProgress);

        // 5. Heartbeat
        let hb = engine.heartbeat("t-1", "worker-1").unwrap();
        assert!(hb.last_heartbeat_epoch_secs.is_some());

        // 6. Test zombie reclaim: with timeout 0, it reclaims immediately
        let report = engine.reclaim_zombies("b-main", 0).unwrap();
        assert_eq!(report.reclaimed_task_ids, vec!["t-1"]);
        let reclaimed = engine.get_task("t-1").unwrap().unwrap();
        assert_eq!(reclaimed.status, KanbanStatus::Todo);
        assert_eq!(reclaimed.retry_count, 1);
    }
}
