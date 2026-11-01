#![forbid(unsafe_code)]

use super::parse_options;
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::WorktreeIsolationManager;
use serde_json::json;
use std::path::PathBuf;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage("branch requires 'list', 'create', 'checkout', or 'cleanup'")
    })?;

    match subcommand.as_str() {
        "list" => list(&args[1..]),
        "create" => create(&args[1..]),
        "checkout" => checkout(&args[1..]),
        "cleanup" => cleanup(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown branch subcommand '{unknown}'. Available subcommands: list, create, checkout, cleanup"
        ))),
    }
}

fn resolve_workspace(parsed: &super::ParsedArgs) -> PathBuf {
    parsed
        .value("workspace")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["workspace"])?;
    let workspace = resolve_workspace(&parsed);
    let manager = WorktreeIsolationManager::new(&workspace).map_err(|err| {
        CliError::execution(format!("failed to open git repository: {err}"), json!({}))
    })?;

    let worktrees = manager.list_worktrees().map_err(|err| {
        CliError::execution(format!("failed to list worktrees: {err}"), json!({}))
    })?;

    let mut human = format!("Active isolated worktrees ({} found):\n", worktrees.len());
    for wt in &worktrees {
        human.push_str(&format!(
            "  - [{}] branch: {} · path: {} · clean: {}\n",
            wt.task_id,
            wt.branch,
            wt.path.display(),
            wt.is_clean
        ));
    }
    if worktrees.is_empty() {
        human.push_str("  (no active ephemeral worktrees)\n");
    }

    Ok(success(
        "branch list",
        json!({
            "workspace": workspace,
            "worktrees": worktrees,
        }),
        human,
    ))
}

fn create(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["workspace", "base"])?;
    if parsed.positionals.is_empty() {
        return Err(CliError::usage("branch create requires <task-id>"));
    }
    let task_id = &parsed.positionals[0];
    let base_commit = parsed.value("base");
    let workspace = resolve_workspace(&parsed);

    let manager = WorktreeIsolationManager::new(&workspace).map_err(|err| {
        CliError::execution(format!("failed to open git repository: {err}"), json!({}))
    })?;

    let ctx = manager
        .provision_worktree(task_id, base_commit)
        .map_err(|err| {
            CliError::execution(format!("failed to provision worktree: {err}"), json!({}))
        })?;

    let human = format!(
        "Provisioned isolated worktree for task '{}':\n  branch: {}\n  path:   {}\n  base:   {} ({})",
        ctx.task_id,
        ctx.branch_name,
        ctx.worktree_path.display(),
        ctx.base_branch,
        ctx.base_commit
    );

    Ok(success(
        "branch create",
        json!({
            "task_id": ctx.task_id,
            "branch_name": ctx.branch_name,
            "base_branch": ctx.base_branch,
            "base_commit": ctx.base_commit,
            "worktree_path": ctx.worktree_path,
            "created_at": ctx.created_at,
            "status": ctx.status.as_str(),
        }),
        human,
    ))
}

fn checkout(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["workspace"])?;
    if parsed.positionals.is_empty() {
        return Err(CliError::usage("branch checkout requires <task-id>"));
    }
    let task_id = &parsed.positionals[0];
    let workspace = resolve_workspace(&parsed);

    let manager = WorktreeIsolationManager::new(&workspace).map_err(|err| {
        CliError::execution(format!("failed to open git repository: {err}"), json!({}))
    })?;

    let worktrees = manager.list_worktrees().map_err(|err| {
        CliError::execution(format!("failed to list worktrees: {err}"), json!({}))
    })?;

    let target = worktrees
        .into_iter()
        .find(|wt| wt.task_id == *task_id || wt.branch == format!("goal/{task_id}"))
        .ok_or_else(|| {
            CliError::execution(
                format!("isolated worktree for task '{task_id}' not found"),
                json!({ "task_id": task_id }),
            )
        })?;

    let human = format!(
        "Worktree path for task '{}': {}\nBranch: {} (HEAD: {})",
        target.task_id,
        target.path.display(),
        target.branch,
        target.head
    );

    Ok(success(
        "branch checkout",
        json!({
            "task_id": target.task_id,
            "path": target.path,
            "branch": target.branch,
            "head": target.head,
            "is_clean": target.is_clean,
        }),
        human,
    ))
}

fn cleanup(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["workspace", "task", "all", "force"])?;
    let workspace = resolve_workspace(&parsed);
    let force = parsed.has_flag("force");

    let manager = WorktreeIsolationManager::new(&workspace).map_err(|err| {
        CliError::execution(format!("failed to open git repository: {err}"), json!({}))
    })?;

    if parsed.has_flag("all") || parsed.positionals.first().map(|s| s.as_str()) == Some("all") {
        let count = manager.cleanup_all(force).map_err(|err| {
            CliError::execution(format!("failed to cleanup all: {err}"), json!({}))
        })?;
        let human = format!("Cleaned up {count} ephemeral worktree(s)");
        return Ok(success(
            "branch cleanup",
            json!({ "cleaned_count": count }),
            human,
        ));
    }

    let task_id = parsed
        .value("task")
        .or_else(|| parsed.positionals.first().map(|s| s.as_str()))
        .ok_or_else(|| CliError::usage("branch cleanup requires --task <task-id> or --all"))?;

    let cleaned = manager.cleanup_worktree(task_id, force).map_err(|err| {
        CliError::execution(format!("failed to cleanup worktree: {err}"), json!({}))
    })?;

    let human = if cleaned {
        format!("Successfully cleaned up isolated worktree for task '{task_id}'")
    } else {
        format!("Worktree for task '{task_id}' was not found or already cleaned")
    };

    Ok(success(
        "branch cleanup",
        json!({
            "task_id": task_id,
            "cleaned": cleaned,
        }),
        human,
    ))
}
