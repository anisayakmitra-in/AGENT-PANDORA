#![forbid(unsafe_code)]

use super::parse_options;
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::WorktreeIsolationManager;
use pandora_types::{WorktreeStatus, WorktreeTaskContext};
use serde_json::json;
use std::path::PathBuf;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("pr requires 'create' or 'status'"))?;

    match subcommand.as_str() {
        "create" => create(&args[1..]),
        "status" => status(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown pr subcommand '{unknown}'. Available subcommands: create, status"
        ))),
    }
}

fn resolve_workspace(parsed: &super::ParsedArgs) -> PathBuf {
    parsed
        .value("workspace")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn find_task_context(
    manager: &WorktreeIsolationManager,
    task_id: Option<&str>,
) -> Result<WorktreeTaskContext, CliError> {
    let worktrees = manager.list_worktrees().map_err(|err| {
        CliError::execution(format!("failed to list worktrees: {err}"), json!({}))
    })?;

    let wt = if let Some(id) = task_id {
        worktrees
            .into_iter()
            .find(|w| w.task_id == id || w.branch == format!("goal/{id}"))
            .ok_or_else(|| {
                CliError::execution(
                    format!("no active worktree found for task '{id}'"),
                    json!({ "task_id": id }),
                )
            })?
    } else {
        worktrees
            .into_iter()
            .find(|w| w.branch.starts_with("goal/"))
            .ok_or_else(|| {
                CliError::execution(
                    "no active goal worktree found. Specify --task <id>",
                    json!({}),
                )
            })?
    };

    Ok(WorktreeTaskContext {
        task_id: wt.task_id.clone(),
        branch_name: wt.branch,
        base_branch: "main".to_string(),
        base_commit: wt.head,
        worktree_path: wt.path,
        created_at: 0,
        status: WorktreeStatus::Running,
    })
}

fn create(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["workspace", "task", "title", "body"])?;
    let workspace = resolve_workspace(&parsed);
    let manager = WorktreeIsolationManager::new(&workspace).map_err(|err| {
        CliError::execution(format!("failed to open git repository: {err}"), json!({}))
    })?;

    let task_id = parsed
        .value("task")
        .or_else(|| parsed.positionals.first().map(|s| s.as_str()));

    let ctx = find_task_context(&manager, task_id)?;
    let title = parsed.value("title");
    let body = parsed.value("body");

    let pr_res = manager
        .generate_pr(&ctx, title, body)
        .map_err(|err| CliError::execution(format!("failed to create PR: {err}"), json!({})))?;

    let human = format!(
        "PR Prepared Successfully:\n  Branch: {}\n  Base:   {}\n  Title:  {}\n  URL:    {}",
        pr_res.branch_name,
        pr_res.base_branch,
        pr_res.pr_title,
        pr_res
            .gh_pr_url
            .as_deref()
            .unwrap_or("Local branch prepared (GitHub CLI not detected or disabled)")
    );

    Ok(success(
        "pr create",
        json!({
            "task_id": pr_res.task_id,
            "branch_name": pr_res.branch_name,
            "base_branch": pr_res.base_branch,
            "head_commit": pr_res.head_commit,
            "title": pr_res.pr_title,
            "body": pr_res.pr_body,
            "gh_pr_url": pr_res.gh_pr_url,
            "gate_report": pr_res.gate_report,
        }),
        human,
    ))
}

fn status(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["workspace", "task"])?;
    let workspace = resolve_workspace(&parsed);
    let manager = WorktreeIsolationManager::new(&workspace).map_err(|err| {
        CliError::execution(format!("failed to open git repository: {err}"), json!({}))
    })?;

    let task_id = parsed
        .value("task")
        .or_else(|| parsed.positionals.first().map(|s| s.as_str()));

    let ctx = find_task_context(&manager, task_id)?;
    let report = manager.evaluate_gates(&ctx).map_err(|err| {
        CliError::execution(format!("failed to evaluate gates: {err}"), json!({}))
    })?;

    let mut human = format!(
        "PR Status for Task '{}':\n  Branch: {}\n  Path:   {}\n  All Gates Passed: {}\n\nProgressive Validation Gates:\n",
        ctx.task_id,
        ctx.branch_name,
        ctx.worktree_path.display(),
        report.all_passed
    );

    for res in &report.gate_results {
        let badge = if res.passed { "PASS" } else { "FAIL" };
        human.push_str(&format!(
            "  [{}] {}: {} ({}ms)\n",
            badge,
            res.gate.as_str(),
            res.details,
            res.duration_ms
        ));
    }

    Ok(success(
        "pr status",
        json!({
            "task_id": ctx.task_id,
            "branch_name": ctx.branch_name,
            "all_passed": report.all_passed,
            "report": report,
        }),
        human,
    ))
}
