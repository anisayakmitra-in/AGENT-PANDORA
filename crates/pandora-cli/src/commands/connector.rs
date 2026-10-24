use super::{load_config_storage, parse_options, require_config_file, session_store};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::ConnectorEngine;
use pandora_types::ConnectorProvider;
use serde_json::json;

fn connector_error(e: impl std::fmt::Display) -> CliError {
    CliError::execution(e.to_string(), json!({}))
}

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage(
            "connector requires 'list', 'authorize', 'exchange', 'inspect', or 'revoke'",
        )
    })?;

    match subcommand.as_str() {
        "list" => list(&args[1..]),
        "authorize" => authorize(&args[1..]),
        "exchange" => exchange(&args[1..]),
        "inspect" => inspect(&args[1..]),
        "revoke" => revoke(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown connector command '{unknown}'"
        ))),
    }
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    let config = load_config_storage(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;

    let connectors = ConnectorEngine::list_connectors(&store).map_err(connector_error)?;

    let mut lines = Vec::new();
    lines.push(format!(
        "{:<20} {:<10} {:<24} {:<20}",
        "CONNECTOR ID", "PROVIDER", "TOKEN", "SCOPES"
    ));
    lines.push("-".repeat(78));
    for c in &connectors {
        let scopes_str = if c.scopes.is_empty() {
            "default".to_string()
        } else {
            c.scopes.join(",")
        };
        lines.push(format!(
            "{:<20} {:<10} {:<24} {:<20}",
            c.connector_id,
            c.provider.as_str(),
            c.token_masked,
            scopes_str,
        ));
    }

    Ok(success(
        "connector list",
        json!({
            "count": connectors.len(),
            "connectors": connectors,
        }),
        lines.join("\n"),
    ))
}

fn authorize(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "provider",
            "client-id",
            "redirect-uri",
            "scopes",
        ],
    )?;
    let config = load_config_storage(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;

    let provider_raw = parsed.value("provider").ok_or_else(|| {
        CliError::usage(
            "connector authorize requires --provider <github|linear|notion|asana|custom>",
        )
    })?;

    let provider: ConnectorProvider = provider_raw
        .parse()
        .map_err(|_| CliError::usage(format!("unsupported connector provider '{provider_raw}'")))?;

    let client_id = parsed.value("client-id").unwrap_or("pandora-oauth-client");
    let redirect_uri = parsed
        .value("redirect-uri")
        .unwrap_or("http://localhost:8080/callback");
    let scopes: Vec<String> = parsed
        .value("scopes")
        .map(|s| {
            s.split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let session = ConnectorEngine::create_authorization_session(
        &store,
        provider,
        client_id,
        redirect_uri,
        &scopes,
    )
    .map_err(connector_error)?;

    let mut lines = Vec::new();
    lines.push("OAuth 2.1 PKCE Authorization Initialized:".to_string());
    lines.push(format!("  Provider:         {}", session.provider.as_str()));
    lines.push(format!("  Session ID:       {}", session.session_id));
    lines.push(format!("  State:            {}", session.pkce.state));
    lines.push(format!(
        "  Code Challenge:   {}",
        session.pkce.code_challenge
    ));
    lines.push(format!(
        "  Challenge Method: {}",
        session.pkce.code_challenge_method
    ));
    lines.push(format!("  Redirect URI:     {}", session.redirect_uri));
    lines.push(String::new());
    lines.push("Open the following authorization URL in your browser:".to_string());
    lines.push(format!("  {}", session.authorization_url));
    lines.push(String::new());
    lines.push("After authorizing, run the exchange command:".to_string());
    lines.push(format!(
        "  pandora connector exchange --session {} --state {} --code <code>",
        session.session_id, session.pkce.state
    ));

    Ok(success(
        "connector authorize",
        json!({
            "session_id": session.session_id,
            "provider": session.provider.as_str(),
            "authorization_url": session.authorization_url,
            "state": session.pkce.state,
            "code_challenge": session.pkce.code_challenge,
            "redirect_uri": session.redirect_uri,
            "scopes": session.scopes,
            "expires_at": session.expires_at,
        }),
        lines.join("\n"),
    ))
}

fn exchange(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "session",
            "state",
            "code",
        ],
    )?;
    let config = load_config_storage(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;

    let session_id = parsed
        .value("session")
        .ok_or_else(|| CliError::usage("connector exchange requires --session <session_id>"))?;
    let state = parsed
        .value("state")
        .ok_or_else(|| CliError::usage("connector exchange requires --state <state>"))?;
    let code = parsed
        .value("code")
        .ok_or_else(|| CliError::usage("connector exchange requires --code <code>"))?;

    let record = ConnectorEngine::complete_authorization(&store, session_id, state, code)
        .map_err(connector_error)?;

    let mut lines = Vec::new();
    lines.push("OAuth 2.1 PKCE Exchange Successful!".to_string());
    lines.push(format!("  Connector ID: {}", record.connector_id));
    lines.push(format!("  Provider:     {}", record.provider.as_str()));
    lines.push(format!("  Token Masked: {}", record.token_masked));
    lines.push(format!("  Token Type:   {}", record.token_type));
    lines.push(format!("  Vault Key ID: {}", record.secret_key_id));
    if let Some(exp) = record.expires_at {
        lines.push(format!("  Expires At:   {exp}"));
    }

    Ok(success(
        "connector exchange",
        json!({
            "status": "authorized",
            "connector": record,
        }),
        lines.join("\n"),
    ))
}

fn inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "connector"])?;
    let config = load_config_storage(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;

    let connector_id = parsed
        .value("connector")
        .or_else(|| parsed.positionals.first().map(String::as_str))
        .ok_or_else(|| {
            CliError::usage("connector inspect requires <connector_id> or --connector <id>")
        })?;

    let record = ConnectorEngine::get_connector(&store, connector_id)
        .map_err(connector_error)?
        .ok_or_else(|| connector_error(format!("connector '{connector_id}' not found")))?;

    let mut lines = Vec::new();
    lines.push(format!("Connector: {}", record.connector_id));
    lines.push(format!("  Provider:     {}", record.provider.as_str()));
    lines.push(format!("  Token Masked: {}", record.token_masked));
    lines.push(format!("  Token Type:   {}", record.token_type));
    lines.push(format!("  Scopes:       {}", record.scopes.join(", ")));
    lines.push(format!("  Secret Key:   {}", record.secret_key_id));
    lines.push(format!("  Created At:   {}", record.created_at));
    if let Some(exp) = record.expires_at {
        lines.push(format!("  Expires At:   {exp}"));
    }

    Ok(success(
        "connector inspect",
        json!({
            "connector": record,
        }),
        lines.join("\n"),
    ))
}

fn revoke(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "connector"])?;
    let config = load_config_storage(&parsed)?;
    require_config_file(&config)?;
    let store = session_store(&config)?;

    let connector_id = parsed
        .value("connector")
        .or_else(|| parsed.positionals.first().map(String::as_str))
        .ok_or_else(|| {
            CliError::usage("connector revoke requires <connector_id> or --connector <id>")
        })?;

    let deleted =
        ConnectorEngine::revoke_connector(&store, connector_id).map_err(connector_error)?;

    if !deleted {
        return Err(connector_error(format!(
            "connector '{connector_id}' was not found or already revoked"
        )));
    }

    Ok(success(
        "connector revoke",
        json!({
            "status": "revoked",
            "connector_id": connector_id,
        }),
        format!("Revoked connector '{connector_id}' successfully."),
    ))
}
