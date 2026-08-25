use super::{
    create_session, load_config, parse_options, require_config_file, session_scope, session_store,
    write_config,
};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::executors::WorkspaceRoot;
use pandora_runtime::{
    ExecutionController, McpError, McpProtocolMode, McpStdioConfig, McpWireEra, ToolEngine,
};
use pandora_types::{Capability, EffectOutcome, EffectReceipt, PolicyContext, Session, SessionId};
use serde_json::{Value, json};

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage("mcp requires 'list', 'inspect', 'set', 'remove', 'catalog', or 'call'")
    })?;
    match subcommand.as_str() {
        "list" => list(&args[1..]),
        "inspect" => inspect(&args[1..]),
        "set" => set(&args[1..]),
        "remove" => remove(&args[1..]),
        "catalog" => catalog(&args[1..]),
        "call" => call(&args[1..]),
        unknown => Err(CliError::usage(format!("unknown mcp command '{unknown}'"))),
    }
}

fn catalog(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "session", "allow"],
    )?;
    require_allow(&parsed)?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "mcp catalog requires exactly one server ID",
        ));
    }
    let (config, store, session) = runtime_session(&parsed)?;
    let server_id = &parsed.positionals[0];
    let server = config.mcp_server(server_id).ok_or_else(|| {
        CliError::configuration("MCP server is not configured", json!({"server": server_id}))
    })?;
    let workspace = WorkspaceRoot::new(config.workspace_dir()).map_err(|_| {
        CliError::configuration(
            "workspace path is invalid",
            json!({"workspace": config.workspace_dir()}),
        )
    })?;
    let policy = mcp_policy();
    let controller = ExecutionController::with_policy(workspace, policy);
    let start = controller
        .start_mcp(
            &ToolEngine::with_builtins(),
            server.clone(),
            &session,
            super::timestamp(),
        )
        .map_err(mcp_failure)?;
    let events = start.events().to_vec();
    let receipts = start.receipts().to_vec();
    let selected_era = start.selected_era();
    let downgraded = start.downgraded();
    let server = start.into_server();
    let revision = server.catalog_revision();
    let data = catalog_data(
        &session,
        selected_era,
        downgraded,
        revision,
        &events,
        &receipts,
    )?;
    store_events(&store, &session, &events)?;
    drop(server);
    Ok(success(
        "mcp catalog",
        data,
        format!("Loaded MCP catalog for {server_id}"),
    ))
}

fn call(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "session",
            "allow",
            "arguments-json",
            "idempotency-key",
        ],
    )?;
    require_allow(&parsed)?;
    if parsed.positionals.len() != 2 {
        return Err(CliError::usage(
            "mcp call requires a server ID and local tool ID",
        ));
    }
    let arguments = parsed
        .value("arguments-json")
        .ok_or_else(|| CliError::usage("mcp call requires '--arguments-json <object>'"))
        .and_then(parse_object)?;
    let idempotency_key = parsed
        .value("idempotency-key")
        .ok_or_else(|| CliError::usage("mcp call requires '--idempotency-key <value>'"))?;
    if idempotency_key.is_empty() || idempotency_key.len() > 256 {
        return Err(CliError::usage(
            "--idempotency-key must contain 1 to 256 characters",
        ));
    }
    let (config, store, session) = runtime_session(&parsed)?;
    let server_id = &parsed.positionals[0];
    let local_tool = &parsed.positionals[1];
    let server = config.mcp_server(server_id).ok_or_else(|| {
        CliError::configuration("MCP server is not configured", json!({"server": server_id}))
    })?;
    let workspace = WorkspaceRoot::new(config.workspace_dir()).map_err(|_| {
        CliError::configuration(
            "workspace path is invalid",
            json!({"workspace": config.workspace_dir()}),
        )
    })?;
    let controller = ExecutionController::with_policy(workspace, mcp_policy());
    let tool_engine = ToolEngine::with_builtins();
    let start = controller
        .start_mcp(&tool_engine, server.clone(), &session, super::timestamp())
        .map_err(mcp_failure)?;
    let selected_era = start.selected_era();
    let downgraded = start.downgraded();
    let start_receipts = start.receipts().to_vec();
    let mut events = start.events().to_vec();
    let mut server = start.into_server();
    let invocation = controller.invoke_mcp(
        &tool_engine,
        &mut server,
        local_tool,
        arguments,
        idempotency_key,
        &session,
        super::timestamp(),
    );
    let result = match invocation {
        Ok(invocation) => {
            events.extend_from_slice(invocation.events());
            let mut receipts = start_receipts;
            receipts.extend_from_slice(invocation.receipts());
            let data = json!({
                "session_id": session.id(),
                "server_id": server_id,
                "tool_id": local_tool,
                "protocol_era": selected_era.as_str(),
                "downgraded": downgraded,
                "result": invocation.result().value(),
                "is_error": invocation.result().is_error(),
                "receipts": receipts_json(&receipts),
                "event_types": events
                    .iter()
                    .map(|event| event.event_type())
                    .collect::<Vec<_>>(),
                "durability": "session-store",
            });
            Ok(success(
                "mcp call",
                data,
                format!("Called MCP tool {local_tool}"),
            ))
        }
        Err(failure) => {
            events.extend_from_slice(failure.events());
            Err(mcp_failure(failure))
        }
    };
    drop(server);
    store_events(&store, &session, &events)?;
    result
}

fn require_allow(parsed: &super::ParsedArgs) -> Result<(), CliError> {
    if parsed.value("allow").is_none() {
        return Err(CliError::usage(
            "MCP execution requires '--allow' for explicit local operator consent",
        ));
    }
    Ok(())
}

fn runtime_session(
    parsed: &super::ParsedArgs,
) -> Result<
    (
        pandora_runtime::config::RuntimeConfig,
        pandora_runtime::SessionStore,
        Session,
    ),
    CliError,
> {
    let config = load_config(parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;
    let session = match parsed.value("session") {
        Some(session_id) => {
            let session_id = SessionId::new(session_id.to_owned())
                .map_err(|_| CliError::usage("session ID is invalid"))?;
            let (principal, tenant, workspace) = session_scope();
            store
                .resume(&session_id, &principal, &tenant, &workspace)
                .map_err(|error| CliError::internal(error.to_string(), json!({})))?
                .session()
                .clone()
        }
        None => {
            let workspace = session_scope().2;
            create_session(&store, &workspace)?
        }
    };
    Ok((config, store, session))
}

fn store_events(
    store: &pandora_runtime::SessionStore,
    session: &Session,
    events: &[pandora_types::RuntimeEvent],
) -> Result<(), CliError> {
    store
        .append_events_at(
            session.id(),
            session.principal_id(),
            session.tenant_id(),
            session.workspace_id(),
            events,
            super::timestamp(),
        )
        .map_err(|error| CliError::internal(error.to_string(), json!({})))
}

fn catalog_data(
    session: &Session,
    selected_era: McpWireEra,
    downgraded: bool,
    revision: &pandora_runtime::McpCatalogRevision,
    events: &[pandora_types::RuntimeEvent],
    receipts: &[pandora_types::EffectReceipt],
) -> Result<Value, CliError> {
    Ok(json!({
        "session_id": session.id(),
        "server_id": revision.server_id(),
        "protocol_era": selected_era.as_str(),
        "downgraded": downgraded,
        "revision": {
            "generation": revision.generation(),
            "process_id": revision.process_id(),
            "config_digest": revision.config_digest(),
            "catalog_digest": revision.catalog_digest(),
            "tools": revision
                .tools()
                .iter()
                .map(|tool| {
                    json!({
                        "local_id": tool.local_id(),
                        "remote_name": tool.remote_name(),
                        "schema_digest": tool.schema_digest(),
                    })
                })
                .collect::<Vec<_>>(),
        },
        "receipts": receipts_json(receipts),
        "event_types": events
            .iter()
            .map(|event| event.event_type())
            .collect::<Vec<_>>(),
        "durability": "session-store",
    }))
}

fn receipts_json(receipts: &[EffectReceipt]) -> Value {
    json!(
        receipts
            .iter()
            .map(|receipt| {
                let outcome = match receipt.outcome() {
                    EffectOutcome::Succeeded => json!({"status": "succeeded"}),
                    EffectOutcome::Failed { code } => json!({"status": "failed", "code": code}),
                    EffectOutcome::Denied { reason } => {
                        json!({"status": "denied", "reason": reason})
                    }
                };
                json!({
                    "receipt_id": receipt.receipt_id().as_str(),
                    "permit_id": receipt.permit_id().as_str(),
                    "request_digest": receipt.request_digest().as_str(),
                    "completed_at": receipt.completed_at().as_unix_seconds(),
                    "outcome": outcome,
                })
            })
            .collect::<Vec<_>>()
    )
}

fn mcp_policy() -> PolicyContext {
    PolicyContext::new(1, [Capability::ProcessExecute, Capability::McpInvoke], [])
}

fn mcp_failure(failure: pandora_runtime::McpFailure) -> CliError {
    let error = failure.error();
    let details = json!({
        "reason": error.code(),
        "receipts": receipts_json(failure.receipts()),
        "event_types": failure.event_types(),
    });
    match error {
        McpError::PermissionDenied | McpError::PolicyDenied => {
            CliError::policy("MCP operation was denied", details)
        }
        McpError::ApprovalRequired => CliError::approval("MCP approval is required", details),
        _ => CliError::execution("MCP operation failed", details),
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

fn parse_object(value: &str) -> Result<Value, CliError> {
    let value: Value = serde_json::from_str(value)
        .map_err(|_| CliError::usage("--arguments-json must be a JSON object"))?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(CliError::usage("--arguments-json must be a JSON object"))
    }
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
