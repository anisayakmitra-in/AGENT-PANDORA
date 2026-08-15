use crate::output::{CliError, CommandResult};
use pandora_runtime::config::{ConfigError, ConfigOverrides, RuntimeConfig};
use pandora_runtime::sessions::{SessionError, SessionStore};
use pandora_types::{
    OrchestrationRole, PrincipalId, Session, SessionId, TenantId, Timestamp, WorkspaceId,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

mod approval;
mod chat;
mod completions;
mod doctor;
mod harness;
mod migration;
mod provider;
mod run;
mod session;
mod setup;
mod skill;
mod tool;
mod uninstall;
mod update;

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
    let json_requested = raw_args.iter().any(|argument| argument == "--json");
    let args = raw_args
        .into_iter()
        .filter(|argument| argument != "--json")
        .collect::<Vec<_>>();
    let command = args
        .first()
        .ok_or_else(|| CliError::usage(usage()))?
        .as_str();
    match command {
        "approval" => approval::execute(&args[1..]),
        "chat" if json_requested => Err(CliError::usage("chat does not support --json")),
        "chat" => chat::execute(&args[1..]),
        "completions" => completions::execute(&args[1..]),
        "harness" => harness::execute(&args[1..]),
        "migrate" => migration::execute(&args[1..]),
        "setup" => setup::execute(&args[1..]),
        "run" => run::execute(&args[1..]),
        "session" => session::execute(&args[1..]),
        "skill" => skill::execute(&args[1..]),
        "provider" => provider::execute(&args[1..]),
        "strategies" => strategies(&args[1..]),
        "tool" => tool::execute(&args[1..]),
        "uninstall" => uninstall::execute(&args[1..]),
        "update" => update::execute(&args[1..]),
        "orchestration" => orchestration(&args[1..]),
        "doctor" => doctor::execute(&args[1..]),
        "--help" | "help" => {
            if args.len() != 1 {
                return Err(CliError::usage("help does not accept additional arguments"));
            }
            Ok(crate::output::success(
                "help",
                json!({"usage": usage()}),
                usage(),
            ))
        }
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
            if is_flag(name) {
                if values.insert(name.to_owned(), String::new()).is_some() {
                    return Err(CliError::usage(format!("option '--{name}' was repeated")));
                }
                index += 1;
                continue;
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

fn is_flag(name: &str) -> bool {
    matches!(
        name,
        "agent" | "allow" | "deny" | "dry-run" | "interactive" | "plan" | "rollback" | "yes"
    )
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

pub(crate) fn config_path(parsed: &ParsedArgs) -> PathBuf {
    parsed
        .value("config")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("PANDORA_CONFIG").map(PathBuf::from))
        .unwrap_or_else(pandora_runtime::config::default_config_path)
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
    if let Some(provider) = parsed.value("provider") {
        overrides = overrides.with_provider_name(provider);
    }
    if let Some(url) = parsed.value("provider-url") {
        overrides = overrides.with_provider_url(url);
    }
    if let Some(model) = parsed.value("model") {
        overrides = overrides.with_provider_model(model);
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
    r#"usage: pandora <help|setup|run|chat|harness|session|skill|approval|provider|tool|orchestration|strategies|completions|migrate|update|uninstall|doctor> [options]

commands:
  help (or --help)
  setup [--interactive] [--provider-url <url>] [--model <model>]
  run [--provider <name>] [--agent] [--max-turns <n>] [--max-tools <n>] [--harness <id>] [--gene <id>] [--plan] [--model <model>] [--approval <id>] <task>
  chat [--provider <name>] [--session <id>] [--max-turns <n>] [--max-tools <n>]
  harness list|inspect|run
  session list|resume|inspect <id>
  skill list|inspect|enable|disable|suspend|remove|restore <id>
  tool list|inspect <id>
  approval list|inspect|resolve
  provider list|set|use|test
  orchestration roles
  strategies list
  completions <powershell|bash|zsh|fish>
  migrate config
  update [--artifact <path> --sha256 <digest> | --rollback]
  uninstall [--dry-run|--yes]
  doctor"#
}

fn orchestration(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("orchestration requires 'roles'"))?;
    if subcommand != "roles" || args.len() != 1 {
        return Err(CliError::usage("orchestration supports only 'roles'"));
    }
    let roles = OrchestrationRole::standard()
        .into_iter()
        .map(|role| role.as_str().to_owned())
        .collect::<Vec<_>>();
    Ok(crate::output::success(
        "orchestration roles",
        json!({"roles": roles}),
        "planner, maker, critic, verifier",
    ))
}

fn strategies(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("strategies requires 'list'"))?;
    if subcommand != "list" || args.len() != 1 {
        return Err(CliError::usage("strategies supports only 'list'"));
    }
    Ok(crate::output::success(
        "strategies list",
        json!({
            "default": "react",
            "available": [
                {"id": "react", "profile": "production"},
                {"id": "reflexion", "profile": "production"},
                {"id": "lats", "profile": "research"}
            ]
        }),
        "react, reflexion, lats (research)",
    ))
}
