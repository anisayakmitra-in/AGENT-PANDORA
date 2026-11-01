#![forbid(unsafe_code)]

use crate::PrpValidationReport;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The action to perform when finalizing an isolated worktree task.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinalizeAction {
    /// Squash-merge the isolated branch changes into the base branch.
    SquashMerge,
    /// Prepare and generate a pull request branch with verification receipts.
    PullRequest,
    /// Discard the worktree and its changes without merging.
    Discard,
}

impl FinalizeAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SquashMerge => "squash_merge",
            Self::PullRequest => "pull_request",
            Self::Discard => "discard",
        }
    }
}

/// The lifecycle status of an ephemeral worktree task.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeStatus {
    Provisioned,
    Running,
    GatesPassed,
    GatesFailed,
    Merged,
    PrCreated,
    Discarded,
}

impl WorktreeStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Provisioned => "provisioned",
            Self::Running => "running",
            Self::GatesPassed => "gates_passed",
            Self::GatesFailed => "gates_failed",
            Self::Merged => "merged",
            Self::PrCreated => "pr_created",
            Self::Discarded => "discarded",
        }
    }
}

/// Metadata and state for an isolated worktree execution task.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorktreeTaskContext {
    pub task_id: String,
    pub branch_name: String,
    pub base_branch: String,
    pub base_commit: String,
    pub worktree_path: PathBuf,
    pub created_at: u64,
    pub status: WorktreeStatus,
}

impl WorktreeTaskContext {
    pub fn new(
        task_id: impl Into<String>,
        branch_name: impl Into<String>,
        base_branch: impl Into<String>,
        base_commit: impl Into<String>,
        worktree_path: impl Into<PathBuf>,
        created_at: u64,
    ) -> Self {
        Self {
            task_id: task_id.into(),
            branch_name: branch_name.into(),
            base_branch: base_branch.into(),
            base_commit: base_commit.into(),
            worktree_path: worktree_path.into(),
            created_at,
            status: WorktreeStatus::Provisioned,
        }
    }
}

/// The result of attempting to squash-merge or finalize an isolated worktree task.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorktreeMergeResult {
    pub task_id: String,
    pub success: bool,
    pub squash_commit: Option<String>,
    pub base_branch: String,
    pub branch_name: String,
    pub gate_report: Option<PrpValidationReport>,
    pub worktree_cleaned: bool,
    pub message: String,
}

/// The result of preparing and creating a PR branch with verification receipts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PrBranchResult {
    pub task_id: String,
    pub branch_name: String,
    pub base_branch: String,
    pub head_commit: String,
    pub pr_title: String,
    pub pr_body: String,
    pub gate_report: Option<PrpValidationReport>,
    pub worktree_cleaned: bool,
    pub gh_pr_url: Option<String>,
}

/// Information describing an active or discovered worktree.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorktreeInfo {
    pub task_id: String,
    pub path: PathBuf,
    pub branch: String,
    pub head: String,
    pub is_clean: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_finalize_action_serde() {
        let action = FinalizeAction::SquashMerge;
        let json = serde_json::to_string(&action).unwrap();
        assert_eq!(json, "\"squash_merge\"");
        let deserialized: FinalizeAction = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, action);
    }

    #[test]
    fn test_worktree_status_serde() {
        let status = WorktreeStatus::GatesPassed;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"gates_passed\"");
        let deserialized: WorktreeStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, status);
    }

    #[test]
    fn test_worktree_task_context_roundtrip() {
        let ctx = WorktreeTaskContext::new(
            "task-42",
            "goal/task-42",
            "main",
            "abcdef1234567890abcdef1234567890abcdef12",
            PathBuf::from("/tmp/worktree-42"),
            1730419200,
        );
        let json = serde_json::to_string(&ctx).unwrap();
        let deserialized: WorktreeTaskContext = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, ctx);
    }

    #[test]
    fn test_merge_result_roundtrip() {
        let res = WorktreeMergeResult {
            task_id: "task-100".to_string(),
            success: true,
            squash_commit: Some("c0ffee".to_string()),
            base_branch: "main".to_string(),
            branch_name: "goal/task-100".to_string(),
            gate_report: None,
            worktree_cleaned: true,
            message: "Squash merged successfully".to_string(),
        };
        let json = serde_json::to_string(&res).unwrap();
        let deserialized: WorktreeMergeResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, res);
    }
}
