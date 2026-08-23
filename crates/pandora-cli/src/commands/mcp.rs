use super::{load_config, parse_options, write_config};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::{McpProtocolMode, McpStdioConfig};
use serde_json::{Value, json};

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("mcp requires 'list', 'inspect', 'set', or 'remove'"))?;
    match subcommand.as_str() {
        "list" => list(&args[1..]),
        "inspect" => inspect(&args[1..]),
        "set" => set(&args[1..]),
        "remove" => remove(&args[1..]),
        unknown => Err(CliError::usage(format!("unknown mcp command '{unknown}'"))),
    }
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "mcp list does not accept positional arguments",
        ));
    }
    let config = load_config(&parsed)?;
    let servers = config
        .mcp_server_ids()
        .into_iter()
        .filter_map(|server_id| config.mcp_server(&server_id).map(server_value))
        .collect::<Vec<_>>();
    Ok(success(
        "mcp list",
        json!({"servers": servers}),
        format!("{} MCP server(s) configured", servers.len()),
    ))
}

fn inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "mcp inspect requires exactly one server ID",
        ));
    }
    let config = load_config(&parsed)?;
    let server_id = &parsed.positionals[0];
    let server = config.mcp_server(server_id).ok_or_else(|| {
        CliError::configuration("MCP server is not configured", json!({"server": server_id}))
    })?;
    Ok(success(
        "mcp inspect",
        json!({"server": server_value(server)}),
        format!("Inspected MCP server {server_id}"),
    ))
}

fn set(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "program",
            "arguments-json",
            "mode",
        ],
    )?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage("mcp set requires exactly one server ID"));
    }
    let program = parsed
        .value("program")
        .ok_or_else(|| CliError::usage("mcp set requires '--program <absolute-path>'"))?;
    let arguments = parsed
        .value("arguments-json")
        .map(parse_arguments)
        .transpose()?
        .unwrap_or_default();
    let mode = parse_mode(parsed.value("mode").unwrap_or("auto"))?;
    let server =
        McpStdioConfig::new(&parsed.positionals[0], program, arguments, mode).map_err(|error| {
            CliError::configuration(
                "MCP server configuration is invalid",
                json!({"reason": error.code()}),
            )
        })?;
    let mut config = load_config(&parsed)?;
    config.set_mcp_server(server);
    write_config(&config)?;
    let server = config
        .mcp_server(&parsed.positionals[0])
        .expect("configured MCP server should be available");
    Ok(success(
        "mcp set",
        json!({"server": server_value(server)}),
        format!("MCP server {} configured", server.server_id()),
    ))
}

fn remove(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "yes"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage("mcp remove requires exactly one server ID"));
    }
    if parsed.value("yes").is_none() {
        return Err(CliError::usage("mcp remove requires '--yes'"));
    }
    let server_id = &parsed.positionals[0];
    let mut config = load_config(&parsed)?;
    if !config.remove_mcp_server(server_id) {
        return Err(CliError::configuration(
            "MCP server is not configured",
            json!({"server": server_id}),
        ));
    }
    write_config(&config)?;
    Ok(success(
        "mcp remove",
        json!({"server": {"id": server_id, "state": "removed"}}),
        format!("MCP server {server_id} removed"),
    ))
}

fn parse_arguments(value: &str) -> Result<Vec<String>, CliError> {
    serde_json::from_str(value)
        .map_err(|_| CliError::usage("--arguments-json must be a JSON array of strings"))
}

fn parse_mode(value: &str) -> Result<McpProtocolMode, CliError> {
    match value {
        "auto" => Ok(McpProtocolMode::Auto),
        "modern-only" => Ok(McpProtocolMode::ModernOnly),
        "legacy-only" => Ok(McpProtocolMode::LegacyOnly),
        _ => Err(CliError::usage(
            "--mode must be 'auto', 'modern-only', or 'legacy-only'",
        )),
    }
}

fn server_value(server: &McpStdioConfig) -> Value {
    json!({
        "id": server.server_id(),
        "program": server.program(),
        "argument_count": server.arguments().len(),
        "mode": server.mode().as_str(),
    })
}
