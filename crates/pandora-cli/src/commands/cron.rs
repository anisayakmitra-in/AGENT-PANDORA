use super::{load_config, parse_options};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::cron_engine::{CronEngine, CronError};
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

fn current_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn engine_for(parsed: &super::ParsedArgs) -> Result<CronEngine, CliError> {
    let config = load_config(parsed)?;
    let storage_path = config.data_dir().join("cron").join("jobs.json");
    CronEngine::open(storage_path).map_err(map_cron_error)
}

fn map_cron_error(err: CronError) -> CliError {
    match err {
        CronError::JobNotFound(id) => {
            CliError::configuration(format!("cron job '{id}' not found"), json!({"job_id": id}))
        }
        CronError::DuplicateJobName(name) => CliError::configuration(
            format!("scheduled job name '{name}' already exists"),
            json!({"name": name}),
        ),
        CronError::InvalidSchedule(msg) => CliError::usage(format!("invalid cron schedule: {msg}")),
        CronError::Storage(msg) => {
            CliError::internal(format!("cron storage error: {msg}"), json!({}))
        }
    }
}

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage("cron requires 'list', 'add', 'remove', 'pause', 'resume', 'run', 'run-due', or 'inspect'")
    })?;

    match subcommand.as_str() {
        "list" => list(&args[1..]),
        "add" => add(&args[1..]),
        "remove" | "rm" => remove(&args[1..]),
        "pause" => pause(&args[1..]),
        "resume" => resume(&args[1..]),
        "run" => run_job(&args[1..]),
        "run-due" => run_due(&args[1..]),
        "inspect" | "status" => inspect(&args[1..]),
        unknown => Err(CliError::usage(format!("unknown cron command '{unknown}'"))),
    }
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    let engine = engine_for(&parsed)?;
    let jobs = engine.list_jobs().map_err(map_cron_error)?;
    let count = jobs.len();
    Ok(success(
        "cron list",
        json!({"jobs": jobs, "count": count}),
        format!("{count} scheduled cron job(s)"),
    ))
}

fn add(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "name",
            "id",
            "schedule",
            "command",
            "prompt",
        ],
    )?;

    let name = parsed
        .value("name")
        .or_else(|| parsed.value("id"))
        .map(ToOwned::to_owned)
        .or_else(|| parsed.positionals.first().cloned());

    let schedule = parsed
        .value("schedule")
        .map(ToOwned::to_owned)
        .or_else(|| parsed.positionals.get(1).cloned());

    let command = parsed
        .value("prompt")
        .or_else(|| parsed.value("command"))
        .map(ToOwned::to_owned)
        .or_else(|| parsed.positionals.get(2).cloned());

    let (final_name, final_sched, final_cmd) = match (name, schedule, command) {
        (Some(n), Some(s), Some(c)) => (n, s, c),
        (Some(s), Some(c), None) => {
            // Positional short form: cron add "<schedule>" "<command>"
            let generated_name = format!("job-{}", current_timestamp());
            (generated_name, s, c)
        }
        _ => {
            return Err(CliError::usage(
                "cron add requires '--name <name> --schedule <expr> --command <cmd>' or positional arguments '[name] <schedule> <command>'",
            ));
        }
    };

    let engine = engine_for(&parsed)?;
    let now = current_timestamp();
    let job = engine
        .add_job(&final_name, &final_sched, &final_cmd, None, None, now)
        .map_err(map_cron_error)?;

    Ok(success(
        "cron add",
        json!({"job": job}),
        format!("Cron job '{}' added", job.id),
    ))
}

fn remove(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "yes"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "cron remove requires exactly one job ID or name",
        ));
    }
    let id_or_name = &parsed.positionals[0];
    let engine = engine_for(&parsed)?;
    let removed = engine.remove_job(id_or_name).map_err(map_cron_error)?;
    if !removed {
        return Err(CliError::configuration(
            format!("cron job '{id_or_name}' not found"),
            json!({"job_id": id_or_name}),
        ));
    }
    Ok(success(
        "cron remove",
        json!({"removed": true, "job_id": id_or_name}),
        format!("Cron job '{id_or_name}' removed"),
    ))
}

fn pause(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "cron pause requires exactly one job ID or name",
        ));
    }
    let id_or_name = &parsed.positionals[0];
    let engine = engine_for(&parsed)?;
    let job = engine.pause_job(id_or_name).map_err(map_cron_error)?;
    Ok(success(
        "cron pause",
        json!({"job": job}),
        format!("Cron job '{}' paused", job.id),
    ))
}

fn resume(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "cron resume requires exactly one job ID or name",
        ));
    }
    let id_or_name = &parsed.positionals[0];
    let engine = engine_for(&parsed)?;
    let now = current_timestamp();
    let job = engine.resume_job(id_or_name, now).map_err(map_cron_error)?;
    Ok(success(
        "cron resume",
        json!({"job": job}),
        format!("Cron job '{}' resumed", job.id),
    ))
}

fn run_job(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "cron run requires exactly one job ID or name",
        ));
    }
    let id_or_name = &parsed.positionals[0];
    let engine = engine_for(&parsed)?;
    let now = current_timestamp();
    let (job, result) = engine.run_job(id_or_name, now).map_err(map_cron_error)?;
    Ok(success(
        "cron run",
        json!({"job": job, "result": result}),
        format!("Cron job '{}' executed", job.id),
    ))
}

fn run_due(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    let engine = engine_for(&parsed)?;
    let now = current_timestamp();
    let results = engine.run_due(now).map_err(map_cron_error)?;
    let count = results.len();
    let items: Vec<_> = results
        .into_iter()
        .map(|(job, res)| json!({"job": job, "result": res}))
        .collect();
    Ok(success(
        "cron run-due",
        json!({"executed_count": count, "runs": items}),
        format!("{count} due cron job(s) executed"),
    ))
}

fn inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "cron inspect requires exactly one job ID or name",
        ));
    }
    let id_or_name = &parsed.positionals[0];
    let engine = engine_for(&parsed)?;
    let job = engine
        .get_job(id_or_name)
        .map_err(map_cron_error)?
        .ok_or_else(|| {
            CliError::configuration(
                format!("cron job '{id_or_name}' not found"),
                json!({"job_id": id_or_name}),
            )
        })?;
    Ok(success(
        "cron inspect",
        json!({"job": job}),
        format!("Inspected cron job '{}'", job.id),
    ))
}
