use super::{
    create_session, load_config, parse_options, require_config_file, session_scope, session_store,
    write_config,
};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::config::{McpAuthType, McpHttpConfig, McpOAuthConfig, RuntimeConfig};
use pandora_runtime::executors::WorkspaceRoot;
use pandora_runtime::{
    ExecutionController, McpError, McpProtocolMode, McpStdioConfig, McpWireEra, StoredMcpAuth,
    ToolEngine,
};
use pandora_types::{Capability, EffectOutcome, EffectReceipt, PolicyContext, Session, SessionId};
use serde_json::{Value, json};

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage("mcp requires 'list', 'inspect', 'set', 'add', 'add-http', 'remove', 'catalog', or 'call'")
    })?;
    match subcommand.as_str() {
        "list" => list(&args[1..]),
        "inspect" => inspect(&args[1..]),
        "set" => set(&args[1..]),
        "add" => add(&args[1..]),
        "add-http" => add_http(&args[1..]),
        "login" => login(&args[1..]),
        "logout" => logout(&args[1..]),
        "remove" | "rm" => remove(&args[1..]),
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
    let target = config.mcp_target(server_id).ok_or_else(|| {
        CliError::configuration("MCP server is not configured", json!({"server": server_id}))
    })?;
    let token = resolve_mcp_auth_token(&config, server_id)?;
    let workspace = WorkspaceRoot::new(config.workspace_dir()).map_err(|_| {
        CliError::configuration(
            "workspace path is invalid",
            json!({"workspace": config.workspace_dir()}),
        )
    })?;
    let policy = mcp_policy();
    let controller = ExecutionController::with_policy(workspace, policy);
    let start = controller
        .start_mcp_target(
            &ToolEngine::with_builtins(),
            target,
            token,
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
    let target = config.mcp_target(server_id).ok_or_else(|| {
        CliError::configuration("MCP server is not configured", json!({"server": server_id}))
    })?;
    let token = resolve_mcp_auth_token(&config, server_id)?;
    let workspace = WorkspaceRoot::new(config.workspace_dir()).map_err(|_| {
        CliError::configuration(
            "workspace path is invalid",
            json!({"workspace": config.workspace_dir()}),
        )
    })?;
    let controller = ExecutionController::with_policy(workspace, mcp_policy());
    let tool_engine = ToolEngine::with_builtins();
    let start = controller
        .start_mcp_target(&tool_engine, target, token, &session, super::timestamp())
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
    PolicyContext::new(
        1,
        [
            Capability::ProcessExecute,
            Capability::NetworkConnect,
            Capability::McpInvoke,
        ],
        [],
    )
}

fn resolve_mcp_auth_token(
    config: &RuntimeConfig,
    server_id: &str,
) -> Result<Option<String>, CliError> {
    if let Ok(tok) = std::env::var("PANDORA_MCP_TOKEN") {
        let trimmed = tok.trim();
        if !trimmed.is_empty() {
            return Ok(Some(trimmed.to_owned()));
        }
    }
    let server_var = format!(
        "PANDORA_MCP_{}_TOKEN",
        server_id.to_ascii_uppercase().replace('-', "_")
    );
    if let Ok(tok) = std::env::var(&server_var) {
        let trimmed = tok.trim();
        if !trimmed.is_empty() {
            return Ok(Some(trimmed.to_owned()));
        }
    }
    if let Some(http_cfg) = config.mcp_http_config(server_id)
        && let Some(secret_ref) = http_cfg.secret_reference()
        && let Ok(val) = std::env::var(secret_ref)
    {
        let trimmed = val.trim();
        if !trimmed.is_empty() {
            return Ok(Some(trimmed.to_owned()));
        }
    }
    if let Ok(vault) = super::secret::open_vault(config)
        && let Some(auth) = vault.get_mcp_auth(server_id)
    {
        return Ok(Some(auth.access_token.clone()));
    }
    Ok(None)
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
    let mut servers = Vec::new();
    for server_id in config.mcp_server_ids() {
        if let Some(server) = config.mcp_server(&server_id) {
            servers.push(server_value(server));
        } else if let Some(url) = config.mcp_http_server(&server_id) {
            servers.push(json!({
                "id": server_id,
                "transport": "http",
                "url": url,
            }));
        }
    }
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
    if let Some(server) = config.mcp_server(server_id) {
        Ok(success(
            "mcp inspect",
            json!({"server": server_value(server)}),
            format!("Inspected MCP server {server_id}"),
        ))
    } else if let Some(http_cfg) = config.mcp_http_config(server_id) {
        Ok(success(
            "mcp inspect",
            json!({"server": {
                "id": server_id,
                "transport": "http",
                "url": &http_cfg.url,
                "mode": http_cfg.mode.as_str(),
                "auth_type": http_cfg.auth.as_str(),
                "headers": &http_cfg.headers,
                "oauth": &http_cfg.oauth,
                "secret_reference": &http_cfg.secret_reference,
            }}),
            format!("Inspected MCP server {server_id}"),
        ))
    } else {
        Err(CliError::configuration(
            "MCP server is not configured",
            json!({"server": server_id}),
        ))
    }
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

fn add(args: &[String]) -> Result<CommandResult, CliError> {
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
    if parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "mcp add requires '<name> <command> [args...]' or '--program <path>'",
        ));
    }
    let server_id = &parsed.positionals[0];
    let (program, arguments) = if let Some(prog) = parsed.value("program") {
        let args = parsed
            .value("arguments-json")
            .map(parse_arguments)
            .transpose()?
            .unwrap_or_default();
        (prog.to_owned(), args)
    } else if parsed.positionals.len() >= 2 {
        let prog = parsed.positionals[1].clone();
        let args = parsed.positionals[2..].to_vec();
        (prog, args)
    } else {
        return Err(CliError::usage(
            "mcp add requires '<name> <command> [args...]' or '--program <path>'",
        ));
    };
    let mode = parse_mode(parsed.value("mode").unwrap_or("auto"))?;
    let server = McpStdioConfig::new(server_id, program, arguments, mode).map_err(|error| {
        CliError::configuration(
            "MCP server configuration is invalid",
            json!({"reason": error.code()}),
        )
    })?;
    let mut config = load_config(&parsed)?;
    config.set_mcp_server(server);
    write_config(&config)?;
    let server = config
        .mcp_server(server_id)
        .expect("configured MCP server should be available");
    Ok(success(
        "mcp add",
        json!({"server": server_value(server)}),
        format!("MCP server {} configured", server.server_id()),
    ))
}

fn add_http(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "name",
            "url",
            "auth",
            "token",
            "token-env",
            "header",
            "client-id",
            "auth-url",
            "token-url",
            "scopes",
            "secret",
        ],
    )?;
    let server_id = if let Some(n) = parsed.value("name") {
        n.to_owned()
    } else if !parsed.positionals.is_empty() {
        parsed.positionals[0].clone()
    } else {
        return Err(CliError::usage("mcp add-http requires '<name> <url>'"));
    };
    let server_id = server_id.as_str();
    let url = if let Some(u) = parsed.value("url") {
        u.to_owned()
    } else if parsed.value("name").is_some() && !parsed.positionals.is_empty() {
        parsed.positionals[0].clone()
    } else if parsed.positionals.len() >= 2 {
        parsed.positionals[1].clone()
    } else {
        return Err(CliError::usage("mcp add-http requires '<name> <url>'"));
    };
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(CliError::configuration(
            "MCP HTTP server URL must start with http:// or https://",
            json!({"url": url}),
        ));
    }
    let auth_type = if let Some(a) = parsed.value("auth") {
        McpAuthType::parse(a).unwrap_or(McpAuthType::Bearer)
    } else if parsed.value("token").is_some() || parsed.value("token-env").is_some() {
        McpAuthType::Bearer
    } else {
        McpAuthType::None
    };

    let mut headers = std::collections::BTreeMap::new();
    if let Some(h) = parsed.value("header")
        && let Some((k, v)) = h.split_once(':')
    {
        headers.insert(k.trim().to_owned(), v.trim().to_owned());
    }

    let oauth = if let (Some(auth_ep), Some(tok_ep), Some(cid)) = (
        parsed.value("auth-url"),
        parsed.value("token-url"),
        parsed.value("client-id"),
    ) {
        Some(McpOAuthConfig {
            auth_endpoint: auth_ep.to_owned(),
            token_endpoint: tok_ep.to_owned(),
            client_id: cid.to_owned(),
            redirect_uri: None,
            scopes: parsed
                .value("scopes")
                .map(|s| s.split(',').map(str::trim).map(str::to_owned).collect())
                .unwrap_or_default(),
        })
    } else {
        None
    };

    let mut http_config =
        McpHttpConfig::new(server_id, &url, McpProtocolMode::Auto, auth_type.clone());
    http_config.headers = headers;
    http_config.oauth = oauth;
    http_config.secret_reference = parsed.value("secret").map(str::to_owned);

    let mut config = load_config(&parsed)?;
    config.set_mcp_http_config(http_config);
    write_config(&config)?;

    let mut token_stored = false;
    let token_val = parsed.value("token").map(str::to_owned).or_else(|| {
        parsed
            .value("token-env")
            .and_then(|e| std::env::var(e).ok())
    });
    if let Some(token) = token_val
        && let Ok(mut vault) = super::secret::open_vault(&config)
    {
        let auth = StoredMcpAuth {
            server_id: server_id.to_owned(),
            auth_type: auth_type.as_str().to_owned(),
            access_token: token,
            refresh_token: None,
            header_name: None,
            expires_at: None,
            token_endpoint: parsed.value("token-url").map(str::to_owned),
            client_id: parsed.value("client-id").map(str::to_owned),
            scopes: Vec::new(),
            created_at: super::timestamp().as_unix_seconds(),
            updated_at: super::timestamp().as_unix_seconds(),
        };
        let _ = vault.put_mcp_auth(auth);
        token_stored = true;
    }

    Ok(success(
        "mcp add-http",
        json!({
            "server": {
                "id": server_id,
                "transport": "http",
                "url": url,
                "auth": auth_type.as_str(),
                "token_stored": token_stored,
            }
        }),
        format!(
            "MCP HTTP server {server_id} configured (auth: {})",
            auth_type.as_str()
        ),
    ))
}

fn login(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "token",
            "token-url",
            "client-id",
        ],
    )?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage("mcp login requires '<server-id>'"));
    }
    let server_id = &parsed.positionals[0];
    let config = load_config(&parsed)?;

    if let Some(token) = parsed.value("token") {
        let mut vault = super::secret::open_vault(&config)?;
        let auth = StoredMcpAuth {
            server_id: server_id.to_owned(),
            auth_type: "bearer".to_owned(),
            access_token: token.to_owned(),
            refresh_token: None,
            header_name: None,
            expires_at: None,
            token_endpoint: parsed.value("token-url").map(str::to_owned),
            client_id: parsed.value("client-id").map(str::to_owned),
            scopes: Vec::new(),
            created_at: super::timestamp().as_unix_seconds(),
            updated_at: super::timestamp().as_unix_seconds(),
        };
        vault
            .put_mcp_auth(auth)
            .map_err(|e| CliError::internal(e.to_string(), json!({})))?;
        return Ok(success(
            "mcp login",
            json!({
                "server": server_id,
                "auth_type": "bearer",
                "status": "authenticated",
            }),
            format!("MCP server '{server_id}' authenticated via token in SecretVault"),
        ));
    }

    let http_cfg = config.mcp_http_config(server_id).ok_or_else(|| {
        CliError::configuration(
            "MCP server is not configured or not an HTTP server",
            json!({"server": server_id}),
        )
    })?;

    let oauth = http_cfg.oauth().ok_or_else(|| {
        CliError::usage(format!(
            "MCP server '{server_id}' has no OAuth config. Use 'pandora mcp login {server_id} --token <token>' or configure OAuth endpoints."
        ))
    })?;

    use base64::Engine;
    use sha2::Digest;
    let mut verifier_bytes = [0u8; 32];
    getrandom::fill(&mut verifier_bytes).map_err(|_| {
        CliError::internal("could not generate cryptographic PKCE entropy", json!({}))
    })?;
    let code_verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(verifier_bytes);
    let challenge_digest = sha2::Sha256::digest(code_verifier.as_bytes());
    let code_challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(challenge_digest);

    let mut state_bytes = [0u8; 16];
    getrandom::fill(&mut state_bytes).map_err(|_| {
        CliError::internal("could not generate cryptographic PKCE entropy", json!({}))
    })?;
    let state = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(state_bytes);

    let auth_url = format!(
        "{}?response_type=code&client_id={}&redirect_uri=http%3A%2F%2F127.0.0.1%3A1455%2Fcallback&code_challenge={}&code_challenge_method=S256&state={}",
        oauth.auth_endpoint, oauth.client_id, code_challenge, state
    );

    Ok(success(
        "mcp login",
        json!({
            "server": server_id,
            "auth_type": "oauth_pkce",
            "authorize_url": auth_url,
            "state": state,
        }),
        format!("Initiate OAuth PKCE login for '{server_id}' at: {auth_url}"),
    ))
}

fn logout(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage("mcp logout requires '<server-id>'"));
    }
    let server_id = &parsed.positionals[0];
    let config = load_config(&parsed)?;
    let mut vault = super::secret::open_vault(&config)?;
    let removed = vault
        .remove_mcp_auth(server_id)
        .map_err(|e| CliError::internal(e.to_string(), json!({})))?;
    Ok(success(
        "mcp logout",
        json!({
            "server": server_id,
            "removed": removed,
        }),
        format!("MCP server '{server_id}' credential removed from SecretVault"),
    ))
}

fn remove(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "yes"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage("mcp remove requires exactly one server ID"));
    }
    let server_id = &parsed.positionals[0];
    if parsed.value("yes").is_none() {
        use std::io::IsTerminal;
        if std::io::stdin().is_terminal() {
            eprint!("Remove MCP server '{server_id}'? [y/N]: ");
            let mut input = String::new();
            if std::io::stdin().read_line(&mut input).is_err()
                || !matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
            {
                return Err(CliError::execution(
                    "MCP server removal aborted by user",
                    json!({"server": server_id}),
                ));
            }
        } else {
            return Err(CliError::usage("mcp remove requires '--yes'"));
        }
    }
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
