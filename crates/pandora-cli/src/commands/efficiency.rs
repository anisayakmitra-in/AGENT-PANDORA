use super::{load_config, parse_options};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::{DEFAULT_MAX_SAMPLES_PER_TARGET, EfficiencyEngine, EfficiencyStore};
use pandora_types::{EfficiencyObjective, EfficiencySummary};
use serde_json::json;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("efficiency requires 'rank'"))?;
    if subcommand != "rank" {
        return Err(CliError::usage(format!(
            "unknown efficiency command '{subcommand}'"
        )));
    }
    rank(&args[1..])
}

fn rank(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "task-class", "objective"],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "efficiency rank does not accept positional arguments",
        ));
    }
    let task_class = parsed.value("task-class").unwrap_or("general");
    validate_label(task_class, "task class")?;
    let objective = parse_objective(parsed.value("objective").unwrap_or("certainty"))?;
    let config = load_config(&parsed)?;
    let store = EfficiencyStore::open(config.data_dir().join("efficiency.sqlite3"))
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let samples = store
        .load_task_class(task_class)
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let engine = EfficiencyEngine::from_samples(DEFAULT_MAX_SAMPLES_PER_TARGET, samples)
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let rankings = engine
        .rank(task_class, objective)
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let human = human_summary(task_class, objective, &rankings);
    Ok(success(
        "efficiency rank",
        json!({
            "task_class": task_class,
            "objective": objective.as_str(),
            "rankings": rankings,
        }),
        human,
    ))
}

pub(crate) fn parse_objective(value: &str) -> Result<EfficiencyObjective, CliError> {
    match value {
        "cost" | "lowest_cost" => Ok(EfficiencyObjective::LowestCost),
        "latency" | "lowest_latency" => Ok(EfficiencyObjective::LowestLatency),
        "tokens" | "lowest_token_usage" => Ok(EfficiencyObjective::LowestTokenUsage),
        "certainty" | "highest_certainty" => Ok(EfficiencyObjective::HighestCertainty),
        _ => Err(CliError::usage(
            "objective must be cost, latency, tokens, or certainty",
        )),
    }
}

fn validate_label(value: &str, name: &str) -> Result<(), CliError> {
    if value.trim().is_empty() {
        return Err(CliError::usage(format!("{name} cannot be empty")));
    }
    if value.len() > 128 || value.chars().any(char::is_control) {
        return Err(CliError::usage(format!("{name} is invalid or too long")));
    }
    Ok(())
}

fn human_summary(
    task_class: &str,
    objective: EfficiencyObjective,
    rankings: &[EfficiencySummary],
) -> String {
    if rankings.is_empty() {
        return format!("No efficiency evidence for {task_class}");
    }
    let entries = rankings
        .iter()
        .map(|summary| {
            let cost = summary.average_known_cost_micros().map_or_else(
                || "cost unknown".to_owned(),
                |value| format!("{value} micros"),
            );
            format!(
                "{} ({}, {} tokens, {} ms, {} bps completion)",
                summary.target(),
                cost,
                summary.average_tokens(),
                summary.average_latency_ms(),
                summary.completion_bps()
            )
        })
        .collect::<Vec<_>>();
    format!(
        "Efficiency ranking for {task_class} by {}: {}",
        objective.as_str(),
        entries.join("; ")
    )
}
