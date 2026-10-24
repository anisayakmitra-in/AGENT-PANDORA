use super::{load_config, parse_options, require_config_file};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::{KanbanEngine, KanbanError};
use pandora_types::{KanbanStatus, KanbanTask};
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("kanban requires 'task' or 'board'"))?;

    match subcommand.as_str() {
        "task" => task(&args[1..]),
        "board" => board(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown kanban command '{unknown}'"
        ))),
    }
}

fn kanban_error(error: KanbanError) -> CliError {
    match error {
        KanbanError::TaskNotFound(msg) => CliError::usage(msg),
        KanbanError::InvalidDependency(msg) => CliError::policy(msg, json!({})),
        KanbanError::Validation(msg) => CliError::usage(msg),
        KanbanError::Database(msg) => CliError::internal(msg, json!({})),
    }
}

fn get_engine(parsed: &super::ParsedArgs) -> Result<KanbanEngine, CliError> {
    let config = load_config(parsed)?;
    require_config_file(&config)?;
    let db_path = config.data_dir().join("kanban.sqlite3");
    KanbanEngine::new(&db_path).map_err(kanban_error)
}

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn task(args: &[String]) -> Result<CommandResult, CliError> {
    let action = args
        .first()
        .ok_or_else(|| CliError::usage("kanban task requires 'create', 'list', 'update', 'heartbeat', 'reclaim', 'inspect', or 'delete'"))?;

    match action.as_str() {
        "create" => task_create(&args[1..]),
        "list" => task_list(&args[1..]),
        "update" => task_update(&args[1..]),
        "heartbeat" => task_heartbeat(&args[1..]),
        "reclaim" => task_reclaim(&args[1..]),
        "inspect" => task_inspect(&args[1..]),
        "delete" => task_delete(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown kanban task action '{unknown}'"
        ))),
    }
}

fn task_create(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "id",
            "title",
            "desc",
            "priority",
            "assignee",
            "deps",
            "board",
        ],
    )?;

    let engine = get_engine(&parsed)?;
    let title = parsed
        .value("title")
        .ok_or_else(|| CliError::usage("kanban task create requires '--title <text>'"))?;
    let desc = parsed.value("desc").unwrap_or("");
    let board = parsed.value("board").unwrap_or("default");
    let priority: u8 = if let Some(p) = parsed.value("priority") {
        p.parse()
            .map_err(|_| CliError::usage("kanban task create --priority must be 1-5"))?
    } else {
        3
    };

    let id = if let Some(explicit_id) = parsed.value("id") {
        explicit_id.to_string()
    } else {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        format!("task-{nanos}")
    };

    let assignee = parsed.value("assignee").map(String::from);
    let dependencies = parsed
        .value("deps")
        .map(|d| {
            d.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();

    let task = KanbanTask::new(
        id,
        board,
        title,
        desc,
        priority,
        assignee,
        dependencies,
        now_epoch_secs(),
    );

    let created = engine.create_task(task).map_err(kanban_error)?;

    Ok(success(
        "kanban task create",
        serde_json::to_value(&created).unwrap_or_else(|_| json!({})),
        format!(
            "Created task '{}' on board '{}' [{}]",
            created.id,
            created.board_id,
            created.status.as_str()
        ),
    ))
}

fn task_list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "board", "status"],
    )?;
    let engine = get_engine(&parsed)?;
    let board = parsed.value("board").unwrap_or("default");
    let status_filter = if let Some(s) = parsed.value("status") {
        Some(KanbanStatus::parse(s).ok_or_else(|| {
            CliError::usage(format!(
                "unknown status '{s}'; supported: backlog, todo, in_progress, blocked, review, done"
            ))
        })?)
    } else {
        None
    };

    let tasks = engine
        .list_tasks(board, status_filter)
        .map_err(kanban_error)?;

    let mut out = format!("Board '{}' Tasks ({}):\n", board, tasks.len());
    for t in &tasks {
        let assignee_str = t.assignee.as_deref().unwrap_or("-");
        out.push_str(&format!(
            "  [{:<11}] (P{}) {:<16} {:<24} (assignee: {})\n",
            t.status.as_str(),
            t.priority,
            t.id,
            t.title,
            assignee_str,
        ));
    }

    Ok(success(
        "kanban task list",
        json!({
            "board_id": board,
            "tasks": tasks,
            "count": tasks.len(),
        }),
        out,
    ))
}

fn task_update(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "status", "worker"],
    )?;
    let task_id = parsed
        .positionals
        .first()
        .ok_or_else(|| CliError::usage("kanban task update requires <task-id>"))?;

    let status_str = parsed
        .value("status")
        .ok_or_else(|| CliError::usage("kanban task update requires '--status <status>'"))?;
    let new_status = KanbanStatus::parse(status_str)
        .ok_or_else(|| CliError::usage(format!("unknown status '{status_str}'")))?;

    let worker = parsed.value("worker");
    let engine = get_engine(&parsed)?;
    let updated = engine
        .update_status(task_id, new_status, worker)
        .map_err(kanban_error)?;

    Ok(success(
        "kanban task update",
        serde_json::to_value(&updated).unwrap_or_else(|_| json!({})),
        format!(
            "Updated task '{}' to status '{}'",
            updated.id,
            updated.status.as_str()
        ),
    ))
}

fn task_heartbeat(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "worker"])?;
    let task_id = parsed
        .positionals
        .first()
        .ok_or_else(|| CliError::usage("kanban task heartbeat requires <task-id>"))?;

    let worker = parsed
        .value("worker")
        .ok_or_else(|| CliError::usage("kanban task heartbeat requires '--worker <name>'"))?;

    let engine = get_engine(&parsed)?;
    let updated = engine.heartbeat(task_id, worker).map_err(kanban_error)?;

    Ok(success(
        "kanban task heartbeat",
        serde_json::to_value(&updated).unwrap_or_else(|_| json!({})),
        format!(
            "Heartbeat received for task '{}' from worker '{}'",
            updated.id, worker
        ),
    ))
}

fn task_reclaim(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "board", "timeout"],
    )?;
    let board = parsed.value("board").unwrap_or("default");
    let timeout: u64 = if let Some(t) = parsed.value("timeout") {
        t.parse()
            .map_err(|_| CliError::usage("kanban task reclaim --timeout must be seconds"))?
    } else {
        300
    };

    let engine = get_engine(&parsed)?;
    let report = engine
        .reclaim_zombies(board, timeout)
        .map_err(kanban_error)?;

    let out = format!(
        "Reclaimed {} zombie tasks back to Todo; marked {} tasks Blocked (retries exhausted)",
        report.reclaimed_task_ids.len(),
        report.blocked_exhausted_ids.len()
    );

    Ok(success(
        "kanban task reclaim",
        serde_json::to_value(&report).unwrap_or_else(|_| json!({})),
        out,
    ))
}

fn task_inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    let task_id = parsed
        .positionals
        .first()
        .ok_or_else(|| CliError::usage("kanban task inspect requires <task-id>"))?;

    let engine = get_engine(&parsed)?;
    let task = engine
        .get_task(task_id)
        .map_err(kanban_error)?
        .ok_or_else(|| CliError::usage(format!("kanban task '{task_id}' not found")))?;

    let text = format!(
        "Task: {}\n  Board: {}\n  Title: {}\n  Status: {}\n  Priority: {}\n  Assignee: {}\n  Dependencies: {:?}\n  Retries: {}/{}\n  Last Heartbeat: {:?}\n  Description: {}",
        task.id,
        task.board_id,
        task.title,
        task.status.as_str(),
        task.priority,
        task.assignee.as_deref().unwrap_or("-"),
        task.dependencies,
        task.retry_count,
        task.max_retries,
        task.last_heartbeat_epoch_secs,
        task.description
    );

    Ok(success(
        "kanban task inspect",
        serde_json::to_value(&task).unwrap_or_else(|_| json!({})),
        text,
    ))
}

fn task_delete(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "yes"])?;
    let task_id = parsed
        .positionals
        .first()
        .ok_or_else(|| CliError::usage("kanban task delete requires <task-id>"))?;

    if !parsed.has_flag("yes") {
        return Err(CliError::usage(
            "kanban task delete requires '--yes' to confirm deletion",
        ));
    }

    let engine = get_engine(&parsed)?;
    let deleted = engine.delete_task(task_id).map_err(kanban_error)?;

    Ok(success(
        "kanban task delete",
        json!({ "id": task_id, "deleted": deleted }),
        format!("Deleted task '{task_id}'"),
    ))
}

fn board(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "board"])?;
    let board_id = parsed.value("board").unwrap_or("default");
    let engine = get_engine(&parsed)?;
    let board = engine.get_board(board_id).map_err(kanban_error)?;

    Ok(success(
        "kanban board",
        serde_json::to_value(&board).unwrap_or_else(|_| json!({})),
        format!(
            "Board '{}' contains {} active tasks",
            board.id,
            board.tasks.len()
        ),
    ))
}
