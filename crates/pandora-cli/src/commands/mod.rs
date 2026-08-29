use crate::output::{CliError, CommandResult};
use pandora_runtime::config::{ConfigError, ConfigOverrides, RuntimeConfig};
use pandora_runtime::sessions::{SessionError, SessionStore};
use pandora_runtime::{PopulationStrategy, PopulationStrategyError, StrategyProfile};
use pandora_types::{
    LineageLimits, MutationLimits, PopulationId, PopulationPolicy, PrincipalId, Session, SessionId,
    TenantId, Timestamp, Usage, WorkspaceId,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

mod approval;
mod auth;
mod backup;
mod chat;
mod completions;
mod doctor;
mod efficiency;
mod evaluation;
mod evolution;
mod feedback;
mod fleet;
mod graph;
mod harness;
mod job;
mod mcp;
mod memory;
mod migration;
mod orchestration;
mod package;
mod provider;
mod registry;
mod rollout;
mod run;
mod secret;
mod service;
mod session;
mod setup;
mod skill;
mod slash;
mod subagent;
#[allow(dead_code)]
pub(crate) mod subagent_run;
mod tool;
mod tui;
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
    if starts_interactive_tui(&args, json_requested, interactive_terminal()) {
        return open_interactive_default();
    }
    let command = args
        .first()
        .ok_or_else(|| CliError::usage(usage()))?
        .as_str();
    if command.starts_with('/') {
        return slash::execute_direct(command, &args[1..]);
    }
    match command {
        "approval" => approval::execute(&args[1..]),
        "auth" => auth::execute(&args[1..]),
        "backup" => backup::execute(&args[1..]),
        "chat" if json_requested => Err(CliError::usage("chat does not support --json")),
        "chat" => chat::execute(&args[1..]),
        "completions" => completions::execute(&args[1..]),
        "harness" => harness::execute(&args[1..]),
        "job" => job::execute(&args[1..]),
        "migrate" => migration::execute(&args[1..]),
        "mcp" => mcp::execute(&args[1..]),
        "memory" => memory::execute(&args[1..]),
        "package" => package::execute(&args[1..]),
        "setup" => setup::execute(&args[1..]),
        "run" => run::execute(&args[1..]),
        "service" => service::execute(&args[1..]),
        "secret" => secret::execute(&args[1..]),
        "session" => session::execute(&args[1..]),
        "slash" => slash::execute(&args[1..]),
        "skill" => skill::execute(&args[1..]),
        "subagent" => subagent::execute(&args[1..]),
        "provider" => provider::execute(&args[1..]),
        "registry" => registry::execute(&args[1..]),
        "strategies" => strategies(&args[1..]),
        "tool" => tool::execute(&args[1..]),
        "tui" if json_requested => Err(CliError::usage("tui does not support --json")),
        "tui" => tui::execute(&args[1..]),
        "uninstall" => uninstall::execute(&args[1..]),
        "update" => update::execute(&args[1..]),
        "orchestration" => orchestration::execute(&args[1..]),
        "doctor" => doctor::execute(&args[1..]),
        "evaluation" => evaluation::execute(&args[1..]),
        "evolution" => evolution::execute(&args[1..]),
        "feedback" => feedback::execute(&args[1..]),
        "rollout" => rollout::execute(&args[1..]),
        "efficiency" => efficiency::execute(&args[1..]),
        "fleet" => fleet::execute(&args[1..]),
        "graph" => graph::execute(&args[1..]),
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

fn starts_interactive_tui(
    args: &[String],
    json_requested: bool,
    interactive_terminal: bool,
) -> bool {
    args.is_empty() && !json_requested && interactive_terminal
}

fn interactive_terminal() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

fn open_interactive_default() -> Result<CommandResult, CliError> {
    let config = RuntimeConfig::load(ConfigOverrides::default()).map_err(config_error)?;
    if needs_interactive_setup(config.config_path().is_file()) {
        setup::execute(&["--interactive".to_owned()])?;
    }
    tui::execute(&[])
}

fn needs_interactive_setup(config_exists: bool) -> bool {
    !config_exists
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
        "agent"
            | "allow"
            | "deny"
            | "dry-run"
            | "fail-on-failure"
            | "fail-on-non-passed"
            | "interactive"
            | "plan"
            | "rollback"
            | "yes"
            | "retryable"
            | "value-stdin"
            | "watch"
            | "daemon"
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
    r#"usage: pandora <help|setup|run|service|auth|secret|backup|chat|tui|harness|slash|session|job|subagent|skill|package|registry|memory|approval|provider|mcp|tool|orchestration|strategies|evaluation|evolution|feedback|rollout|efficiency|fleet|graph|completions|migrate|update|uninstall|doctor> [options]

commands:
  help (or --help)
  setup [--interactive] [--provider-url <url>] [--model <model>] [--api-key-env <name>]
  run [--provider <name>] [--session <id>] [--agent] [--max-turns <n>] [--max-tools <n>] [--harness <id>] [--harness-version <version>] [--gene <id>] [--plan] [--model <model>] [--task-class <name>] [--approval <id>] [--optimize <cost|latency|tokens|certainty>] <task>
  service start [--port <port>]
  auth enroll --principal <id> --tenant <id> --workspace-id <id> --role <viewer|operator|administrator> [--device-key-file <path>] [--token-file <path>] | list | revoke <identity-id> --yes
  secret set <ENV_NAME> --value-stdin | list | status <ENV_NAME> | remove <ENV_NAME> --yes
  backup create --output <path> [--passphrase-env <name>] | inspect --input <path> [--passphrase-env <name>] | restore --input <path> [--passphrase-env <name>] --yes
  chat [--provider <name>] [--session <id>] [--max-turns <n>] [--max-tools <n>]
  tui [--provider <name>] [--session <id>] [--max-turns <n>] [--max-tools <n>]
  harness list|inspect|run [--harness-version <version>]
  slash list|resolve <command>
  session list|resume|inspect <id>
  job submit|work|list|inspect|cancel|mark-interrupted (work accepts --max-jobs <1-64>, bounded --watch --idle-timeout <1-3600>, or --daemon)
  subagent spawn --session <id> --execution <id> [--commit <sha>] [--provider <name>] [--harness <id> --harness-version <version>] [--max-turns <n>] [--max-tools <n>] [--max-tokens <n>] [--max-duration <seconds>] [--max-depth <n>] [--max-result-bytes <n>] <task>
  subagent list|inspect|cancel|mark-interrupted|cleanup <id> | work [--max-agents <1-8>]
  skill list|inspect|install|enable|disable|suspend|remove|restore <id-or-path>
  package admit --manifest <path> --artifact <path> | validate --manifest <path> --artifact <path> | install <id> [version] [--registry <url>|--registry-profile <name>] [--token-env <name>] | install-github --repository <url> --commit <sha> --manifest <repo-path> --artifact <repo-path> [--token-env <name>] | list | inspect <id> <version> | enable|disable <id> <version> [--dry-run|--yes] | rollback <id> [--dry-run|--yes] | lock [--output <path>] | verify-lock [--lock <path>] | remove <id> <version> [--dry-run|--yes]
  registry list | set --name <name> --registry-url <url> [--token-env <name>] | use <name> | remove <name> --yes
  memory recall --session <id> --provider <name> --tier <l1|l2> [--id <memory-id>] [--limit <1-256>] | audit --session <id> --provider <name> | forget --session <id> --provider <name> <memory-id> [--yes] | promote --session <id> --provider <name> <memory-id> [--approval <id>] | synthesize --session <id> --provider <name> --id <memory-id> --summary <text> [--kind <kind>] [--classification <public|internal>] [--yes] | provenance --session <id> --provider <name> <memory-id>
  tool list|inspect <id>
  approval list|inspect|resolve
  provider list|set|use|test
  mcp list|inspect|set|remove|catalog <server> --allow|call <server> <tool> --arguments-json <object> --idempotency-key <key> --allow
  orchestration roles|submit|claim|complete|list|inspect|cancel|mark-interrupted|resume
  strategies list | population list --state <path> | population inspect --state <path> --id <id>
  evaluation golden --input <path> [--fail-on-failure]
  evaluation inspect --session <id> [--execution <id>]
  evaluation scorecard --session <id> [--fail-on-non-passed]
  evolution generate --session <id> [--provider <name>] [--model <id>] --kind prompt|skill|workflow|wasm_gene --target-id <id> --base <path> --output <path> | list [--limit <1-256>] | inspect --id <proposal-id> | submit --input <path> | evaluate --id <proposal-id> --input <path> [--fail-on-failure] | approve --input <path> | stage --id <proposal-id> | canary --input <path> | activate --id <proposal-id> | rollback --id <proposal-id> --reason <text>
  feedback coding --session <id> --execution <id> --request-digest <digest> --expected-output <text> --output <text> [--terminal-failure <text>] [--retryable]
  evolution inspect --id <proposal-id>
  rollout inspect --session <id> [--execution <id>]
  efficiency rank [--task-class <name>] [--objective <cost|latency|tokens|certainty>]
  fleet list|register|dispatch|lease|renew|release|expire|supervisor [list|start|drain|stop|recover|heartbeat|reconcile]|quarantine|revoke|kill
  graph code|knowledge|review|architecture --input <path> [--store <path>] [--tenant <id>] [--workspace <id>]
  completions <powershell|bash|zsh|fish>
  migrate config
  update [--release <tag> [--channel <stable|beta>] | --artifact <path> --sha256 <digest> | --rollback]
  uninstall [--dry-run|--yes]
  doctor"#
}

fn strategies(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("strategies requires 'list'"))?;
    match subcommand.as_str() {
        "list" if args.len() == 1 => Ok(crate::output::success(
            "strategies list",
            json!({
                "default": "react",
                "available": [
                    {"id": "react", "profile": "production"},
                    {"id": "reflexion", "profile": "production"},
                    {"id": "lats", "profile": "research"},
                    {"id": "population", "profile": "research"}
                ]
            }),
            "react, reflexion, lats (research), population (research)",
        )),
        "population" => population_strategy(&args[1..]),
        _ => Err(CliError::usage(
            "strategies supports 'list' or 'population list|inspect'",
        )),
    }
}

fn population_strategy(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("strategies population requires 'list' or 'inspect'"))?;
    let parsed = parse_options(&args[1..], &["state", "id"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "strategies population does not accept positional arguments",
        ));
    }
    let state = parsed
        .value("state")
        .ok_or_else(|| CliError::usage("strategies population requires '--state <path>'"))?;
    let strategy = PopulationStrategy::open(
        state,
        StrategyProfile::Research,
        population_inspection_policy(),
    )
    .map_err(population_strategy_error)?;
    match subcommand.as_str() {
        "list" => {
            if parsed.value("id").is_some() {
                return Err(CliError::usage(
                    "strategies population list does not accept '--id'",
                ));
            }
            let populations = strategy
                .population_ids()
                .map_err(population_strategy_error)?;
            let count = populations.len();
            Ok(crate::output::success(
                "strategies population list",
                json!({"populations": populations, "count": count}),
                format!("Listed {count} research population(s)"),
            ))
        }
        "inspect" => {
            let id = parsed
                .value("id")
                .ok_or_else(|| {
                    CliError::usage("strategies population inspect requires '--id <id>'")
                })
                .and_then(|value| {
                    PopulationId::new(value.to_owned())
                        .map_err(|_| CliError::usage("population ID is invalid"))
                })?;
            let population = strategy
                .population(&id)
                .map_err(population_strategy_error)?;
            Ok(crate::output::success(
                "strategies population inspect",
                json!({
                    "population_id": population.scope().population_id(),
                    "tenant_id": population.scope().tenant_id(),
                    "workspace_id": population.scope().workspace_id(),
                    "session_id": population.scope().session_id(),
                    "generation": population.generation(),
                    "candidate_count": population.candidates().len(),
                    "research_only": true,
                }),
                format!("Inspected research population {}", id.as_str()),
            ))
        }
        _ => Err(CliError::usage(
            "strategies population supports only 'list' or 'inspect'",
        )),
    }
}

fn population_inspection_policy() -> PopulationPolicy {
    PopulationPolicy::new(
        256,
        16,
        256,
        256,
        MutationLimits::new(256, 4 * 1024 * 1024, 256).expect("fixed inspection policy is valid"),
        LineageLimits::new(64, 256, 4 * 1024 * 1024).expect("fixed inspection policy is valid"),
        100,
        1,
        Usage::new(u64::MAX, u32::MAX, u64::MAX, u64::MAX),
    )
    .expect("fixed inspection policy is valid")
}

fn population_strategy_error(error: PopulationStrategyError) -> CliError {
    CliError::execution(
        error.to_string(),
        json!({"reason": "population_state_unavailable"}),
    )
}

#[cfg(test)]
mod tests {
    use super::{execute, needs_interactive_setup, starts_interactive_tui};

    #[test]
    fn empty_argv_opens_tui_only_for_interactive_human_sessions() {
        assert!(starts_interactive_tui(&[], false, true));
        assert!(!starts_interactive_tui(&[], true, true));
        assert!(!starts_interactive_tui(&[], false, false));
        assert!(!starts_interactive_tui(&["run".to_owned()], false, true));
    }

    #[test]
    fn missing_configuration_requires_onboarding_before_the_tui() {
        assert!(needs_interactive_setup(false));
        assert!(!needs_interactive_setup(true));
    }

    #[test]
    fn job_command_is_dispatched() {
        let error = match execute(vec!["job".to_owned()]) {
            Ok(_) => panic!("job without a subcommand should fail"),
            Err(error) => error,
        };

        assert_eq!(error.message, "job requires a subcommand");
    }

    #[test]
    fn help_lists_the_headless_job_surface() {
        let result = execute(vec!["help".to_owned()]).unwrap();
        let usage = result.data["usage"].as_str().unwrap();

        assert!(usage.contains("job submit|work|list|inspect|cancel|mark-interrupted"));
        assert!(usage.contains("work accepts --max-jobs <1-64>"));
        assert!(usage.contains("evaluation golden --input <path> [--fail-on-failure]"));
        assert!(usage.contains("strategies list | population list --state <path>"));
        assert!(usage.contains("evaluation inspect --session <id> [--execution <id>]"));
        assert!(usage.contains("evolution generate --session <id>"));
        assert!(usage.contains("list [--limit <1-256>]"));
        assert!(usage.contains("evolution inspect --id <proposal-id>"));
        assert!(usage.contains("graph code|knowledge|review|architecture --input <path>"));
    }
}
