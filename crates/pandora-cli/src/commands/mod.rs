use crate::output::{CliError, CommandResult};
use pandora_runtime::config::{ConfigError, ConfigOverrides, RuntimeConfig};
use pandora_runtime::sessions::{SessionError, SessionStore};
use pandora_types::{PrincipalId, Session, SessionId, TenantId, Timestamp, WorkspaceId};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

mod doctor;
mod provider;
mod run;
mod session;
mod setup;

pub(crate) const LOCAL_PRINCIPAL: &str = "local-user";
pub(crate) const LOCAL_TENANT: &str = "local-tenant";
pub(crate) const LOCAL_WORKSPACE: &str = "local-workspace";

pub(crate) struct ParsedArgs {
    pub values: BTreeMap<String, String>,
    pub positionals: Vec<String>,
}

impl ParsedArgs {
    pub fn value(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }
}

pub fn execute(raw_args: Vec<String>) -> Result<CommandResult, CliError> {
    let args = raw_args
        .into_iter()
        .filter(|argument| argument != "--json")
        .collect::<Vec<_>>();
    let command = args
        .first()
        .ok_or_else(|| CliError::usage(usage()))?
        .as_str();
    match command {
        "setup" => setup::execute(&args[1..]),
        "run" => run::execute(&args[1..]),
        "session" => session::execute(&args[1..]),
        "provider" => provider::execute(&args[1..]),
        "doctor" => doctor::execute(&args[1..]),
        "--help" | "help" => Err(CliError::usage(usage())),
        unknown => Err(CliError::usage(format!(
            "unknown command '{unknown}'.\n\n{}",
            usage()
        ))),
    }
}

pub(crate) fn parse_options(args: &[String], allowed: &[&str]) -> Result<ParsedArgs, CliError> {
    let mut values = BTreeMap::new();
    let mut positionals = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        if let Some(option) = argument.strip_prefix("--") {
            let (name, inline_value) = option
                .split_once('=')
                .map_or((option, None), |(name, value)| (name, Some(value)));
            if !allowed.contains(&name) {
                return Err(CliError::usage(format!("unknown option '--{name}'")));
            }
            let value = if let Some(value) = inline_value {
                value.to_owned()
            } else {
                index += 1;
                args.get(index)
                    .filter(|value| !value.starts_with('-'))
                    .cloned()
                    .ok_or_else(|| CliError::usage(format!("option '--{name}' needs a value")))?
            };
            if values.insert(name.to_owned(), value).is_some() {
                return Err(CliError::usage(format!("option '--{name}' was repeated")));
            }
        } else {
            positionals.push(argument.clone());
        }
        index += 1;
    }
    Ok(ParsedArgs {
        values,
        positionals,
    })
}

pub(crate) fn load_config(parsed: &ParsedArgs) -> Result<RuntimeConfig, CliError> {
    RuntimeConfig::load(overrides(parsed)).map_err(config_error)
}

pub(crate) fn write_config(config: &RuntimeConfig) -> Result<(), CliError> {
    config.write().map_err(config_error)
}

pub(crate) fn require_config_file(config: &RuntimeConfig) -> Result<(), CliError> {
    if config.config_path().is_file() {
        Ok(())
    } else {
        Err(CliError::configuration(
            "Pandora is not configured; run 'pandora setup' first",
            json!({"config_path": config.config_path()}),
        ))
    }
}

pub(crate) fn create_session(
    store: &SessionStore,
    workspace: &WorkspaceId,
) -> Result<Session, CliError> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let session_id = SessionId::new(format!("session-{}-{}", std::process::id(), nonce))
        .map_err(|_| CliError::internal("could not allocate a session ID", json!({})))?;
    let session = Session::new(
        session_id,
        PrincipalId::new(LOCAL_PRINCIPAL).expect("built-in principal ID is valid"),
        TenantId::new(LOCAL_TENANT).expect("built-in tenant ID is valid"),
        workspace.clone(),
        timestamp(),
    );
    store.create(&session).map_err(session_error)?;
    Ok(session)
}

pub(crate) fn session_scope() -> (PrincipalId, TenantId, WorkspaceId) {
    (
        PrincipalId::new(LOCAL_PRINCIPAL).expect("built-in principal ID is valid"),
        TenantId::new(LOCAL_TENANT).expect("built-in tenant ID is valid"),
        WorkspaceId::new(LOCAL_WORKSPACE).expect("built-in workspace ID is valid"),
    )
}

pub(crate) fn session_store(config: &RuntimeConfig) -> Result<SessionStore, CliError> {
    SessionStore::open(config.data_dir().join("sessions.sqlite3")).map_err(session_error)
}

pub(crate) fn timestamp() -> Timestamp {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    Timestamp::from_unix_seconds(seconds)
}

pub(crate) fn path_option(parsed: &ParsedArgs, name: &str) -> Option<PathBuf> {
    parsed.value(name).map(PathBuf::from)
}

fn overrides(parsed: &ParsedArgs) -> ConfigOverrides {
    let mut overrides = ConfigOverrides::default();
    if let Some(path) = path_option(parsed, "config") {
        overrides = overrides.with_config_path(path);
    }
    if let Some(path) = path_option(parsed, "data-dir") {
        overrides = overrides.with_data_dir(path);
    }
    if let Some(path) = path_option(parsed, "workspace") {
        overrides = overrides.with_workspace_dir(path);
    }
    if let Some(url) = parsed.value("provider-url") {
        overrides = overrides.with_provider_url(url);
    }
    overrides
}

fn config_error(error: ConfigError) -> CliError {
    CliError::configuration(error.to_string(), json!({}))
}

fn session_error(error: SessionError) -> CliError {
    let message = error.to_string();
    CliError::internal(message, json!({}))
}

fn usage() -> &'static str {
    "usage: pandora <setup|run|session|provider|doctor> [options]\n\n\
commands:\n  setup\n  run <task>\n  session list|resume <id>\n  provider list|set\n  doctor"
}
