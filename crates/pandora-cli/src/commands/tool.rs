use super::parse_options;
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::ToolEngine;
use serde_json::json;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("tool requires 'list'"))?;
    match subcommand.as_str() {
        "list" => list(&args[1..]),
        unknown => Err(CliError::usage(format!("unknown tool command '{unknown}'"))),
    }
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "tool list does not accept positional arguments",
        ));
    }
    let tools = ToolEngine::with_builtins()
        .list()
        .into_iter()
        .map(|tool| {
            json!({
                "id": tool.id(),
                "version": tool.version(),
                "name": tool.name(),
                "capability": tool.capability().as_str(),
                "operation": tool.operation().as_str(),
                "input_schema": tool.input_schema(),
            })
        })
        .collect::<Vec<_>>();
    Ok(success(
        "tool list",
        json!({"tools": tools}),
        format!("{} built-in tools available", tools.len()),
    ))
}
