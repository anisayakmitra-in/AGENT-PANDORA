use serde::{Deserialize, Serialize};

/// Lifecycle state for durable multi-agent Kanban tasks.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KanbanStatus {
    Backlog,
    Todo,
    InProgress,
    Blocked,
    Review,
    Done,
}

impl KanbanStatus {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().replace('-', "_").as_str() {
            "backlog" => Some(Self::Backlog),
            "todo" | "to_do" => Some(Self::Todo),
            "in_progress" | "inprogress" => Some(Self::InProgress),
            "blocked" => Some(Self::Blocked),
            "review" => Some(Self::Review),
            "done" | "completed" => Some(Self::Done),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Backlog => "backlog",
            Self::Todo => "todo",
            Self::InProgress => "in_progress",
            Self::Blocked => "blocked",
            Self::Review => "review",
            Self::Done => "done",
        }
    }
}

/// A durable task item on a Kanban board.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KanbanTask {
    pub id: String,
    pub board_id: String,
    pub title: String,
    pub description: String,
    pub status: KanbanStatus,
    pub priority: u8,
    pub assignee: Option<String>,
    pub dependencies: Vec<String>,
    pub retry_count: usize,
    pub max_retries: usize,
    pub last_heartbeat_epoch_secs: Option<u64>,
    pub created_at: u64,
    pub updated_at: u64,
    pub labels: Vec<String>,
}

impl KanbanTask {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        board_id: impl Into<String>,
        title: impl Into<String>,
        description: impl Into<String>,
        priority: u8,
        assignee: Option<String>,
        dependencies: Vec<String>,
        now_epoch_secs: u64,
    ) -> Self {
        Self {
            id: id.into(),
            board_id: board_id.into(),
            title: title.into(),
            description: description.into(),
            status: KanbanStatus::Todo,
            priority: priority.clamp(1, 5),
            assignee,
            dependencies,
            retry_count: 0,
            max_retries: 3,
            last_heartbeat_epoch_secs: None,
            created_at: now_epoch_secs,
            updated_at: now_epoch_secs,
            labels: Vec::new(),
        }
    }

    pub fn can_start(&self, completed_tasks: &[String]) -> bool {
        self.dependencies
            .iter()
            .all(|dep| completed_tasks.contains(dep))
    }

    pub fn is_zombie(&self, now_epoch_secs: u64, timeout_secs: u64) -> bool {
        if self.status != KanbanStatus::InProgress {
            return false;
        }
        match self.last_heartbeat_epoch_secs {
            Some(last) => now_epoch_secs.saturating_sub(last) >= timeout_secs,
            None => now_epoch_secs.saturating_sub(self.updated_at) >= timeout_secs,
        }
    }
}

/// A durable Kanban board containing a collection of tasks.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KanbanBoard {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tasks: Vec<KanbanTask>,
    pub created_at: u64,
}

/// Report returned after executing a zombie reclaim pass.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KanbanReclaimReport {
    pub reclaimed_task_ids: Vec<String>,
    pub blocked_exhausted_ids: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kanban_status_parsing_and_zombie_detection() {
        assert_eq!(
            KanbanStatus::parse("in-progress"),
            Some(KanbanStatus::InProgress)
        );
        assert_eq!(KanbanStatus::parse("done"), Some(KanbanStatus::Done));
        assert_eq!(KanbanStatus::parse("invalid"), None);

        let mut task = KanbanTask::new(
            "t-1",
            "b-1",
            "Audit memory",
            "Verify sqlite triggers",
            1,
            Some("auditor".to_string()),
            vec!["t-0".to_string()],
            1000,
        );
        assert!(!task.can_start(&[]));
        assert!(task.can_start(&["t-0".to_string()]));

        task.status = KanbanStatus::InProgress;
        task.last_heartbeat_epoch_secs = Some(1050);
        assert!(!task.is_zombie(1100, 60)); // 50s elapsed < 60s
        assert!(task.is_zombie(1120, 60)); // 70s elapsed > 60s
    }
}
