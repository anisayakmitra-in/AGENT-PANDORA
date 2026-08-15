use super::parse_options;
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::ToolEngine;
use serde_json::json;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("tool requires 'list' or 'inspect'"))?;
    match subcommand.as_str() {
        "list" => list(&args[1..]),
        "inspect" => inspect(&args[1..]),
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
        .map(tool_value)
        .collect::<Vec<_>>();
    Ok(success(
        "tool list",
        json!({"tools": tools}),
        format!("{} built-in tools available", tools.len()),
    ))
}

fn inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage("tool inspect requires exactly one tool ID"));
    }
    let tool_id = &parsed.positionals[0];
    let tool = ToolEngine::with_builtins()
        .list()
        .into_iter()
        .find(|tool| tool.id().as_str() == tool_id)
        .ok_or_else(|| CliError::usage(format!("unknown tool '{tool_id}'")))?;
    Ok(success(
        "tool inspect",
        json!({"tool": tool_value(tool)}),
        format!("Inspected tool {tool_id}"),
    ))
}

fn tool_value(tool: pandora_runtime::tool_engine::ToolDefinition) -> serde_json::Value {
    json!({
        "id": tool.id(),
        "version": tool.version(),
        "name": tool.name(),
        "capability": tool.capability().as_str(),
        "operation": tool.operation().as_str(),
        "input_schema": tool.input_schema(),
    })
}
