use super::{ParsedArgs, load_config, parse_options};
use super::{approval, run, slash};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::prompt_engine::{PresetKind, PromptEngine, PromptPreset};
use pandora_types::GovernanceMode;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{self, BufRead, IsTerminal as _, Write};
use std::path::Path;

const MAX_DISPLAY_CHARS: usize = 64 * 1024;

struct PendingTask {
    invocation: PendingInvocation,
    approval_id: String,
}

enum PendingInvocation {
    Agent(String),
    Slash(String),
}

struct InteractiveSessionState {
    active_model: Option<String>,
    active_provider: Option<String>,
    active_mode: Option<GovernanceMode>,
    active_persona: Option<String>,
}

impl InteractiveSessionState {
    fn new(parsed: &ParsedArgs) -> Self {
        let active_model = parsed.value("model").map(str::to_owned);
        let active_provider = parsed.value("provider").map(str::to_owned);
        let active_mode = parsed
            .value("mode")
            .or_else(|| parsed.value("permission"))
            .and_then(GovernanceMode::parse);
        Self {
            active_model,
            active_provider,
            active_mode,
            active_persona: None,
        }
    }
}

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "provider",
            "session",
            "model",
            "max-turns",
            "max-tools",
            "mode",
            "permission",
            "tool-profile",
            "tools",
        ],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage("chat does not accept a positional task"));
    }

    let mut state = InteractiveSessionState::new(&parsed);
    let mut session_id = parsed.value("session").map(str::to_owned);
    let mut turns = 0u32;
    let mut pending = None;
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut stdout = io::stdout();
    if io::stdout().is_terminal() {
        let config = super::load_config(&parsed).ok();
        let provider = state
            .active_provider
            .as_deref()
            .or_else(|| config.as_ref().and_then(|c| c.active_provider()));
        let model = state
            .active_model
            .as_deref()
            .or_else(|| config.as_ref().and_then(|c| c.provider_model()));
        let default_workspace =
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let workspace = config
            .as_ref()
            .map(|c| c.workspace_dir())
            .unwrap_or(&default_workspace);
        let banner = super::welcome::render_welcome_banner(
            provider,
            model,
            workspace,
            session_id.as_deref(),
            true,
        );
        print!("{banner}");
        let _ = stdout.flush();
    }
    println!("Pandora chat. Type /help for commands or /exit to close.");
    if io::stdout().is_terminal() {
        println!(
            "\x1b[38;2;245;158;11m✦ Tip:\x1b[0m Slash commands (/model, /mode, /key, /preset, /tools, /mcp, /update, /uninstall) give instant hot-swapping and zero-turn latency.\n"
        );
    }

    loop {
        print!("pandora> ");
        stdout
            .flush()
            .map_err(|_| CliError::internal("could not flush chat prompt", json!({})))?;
        let mut line = String::new();
        let read = input
            .read_line(&mut line)
            .map_err(|_| CliError::internal("could not read chat input", json!({})))?;
        if read == 0 {
            break;
        }
        let input = line.trim();
        if input.is_empty() {
            continue;
        }
        match input {
            "/exit" | "/quit" => break,
            "/help" => print_help(),
            "/session" => print_session(session_id.as_deref()),
            "/model" => {
                let model = state
                    .active_model
                    .as_deref()
                    .unwrap_or("default (from config)");
                let provider = state.active_provider.as_deref().unwrap_or("default");
                println!("model> active model: {model} (provider: {provider})");
            }
            value if value.starts_with("/model ") => {
                let spec = value.trim_start_matches("/model ").trim();
                if spec.is_empty() {
                    println!("usage: /model <model> or /model <provider:model>");
                } else if let Some((p, m)) = spec.split_once(':') {
                    state.active_provider = Some(p.trim().to_owned());
                    state.active_model = Some(m.trim().to_owned());
                    println!(
                        "model> switched to provider '{}' and model '{}' (in-session hot-swap, history preserved)",
                        p.trim(),
                        m.trim()
                    );
                } else {
                    state.active_model = Some(spec.to_owned());
                    println!(
                        "model> switched to model '{spec}' (in-session hot-swap, history preserved)"
                    );
                }
            }
            "/mode" | "/permission" => {
                let mode = state.active_mode.unwrap_or(GovernanceMode::Supervised);
                println!(
                    "governance> active mode: {} (level: {})",
                    mode.as_str(),
                    mode.level()
                );
            }
            value if value.starts_with("/mode ") || value.starts_with("/permission ") => {
                let arg = if value.starts_with("/mode ") {
                    value.trim_start_matches("/mode ").trim()
                } else {
                    value.trim_start_matches("/permission ").trim()
                };
                match GovernanceMode::parse(arg) {
                    Some(m) => {
                        state.active_mode = Some(m);
                        println!(
                            "governance> switched to '{}' mode (level {})",
                            m.as_str(),
                            m.level()
                        );
                    }
                    None => {
                        println!(
                            "usage: /mode <autonomous|supervised|read-only> or /permission <0|1|2>"
                        );
                    }
                }
            }
            "/preset" => {
                handle_preset_list(&parsed);
            }
            value if value.starts_with("/preset save ") => {
                let rest = value.trim_start_matches("/preset save ").trim();
                handle_preset_save(&parsed, rest);
            }
            value if value.starts_with("/preset ") => {
                let rest = value.trim_start_matches("/preset ").trim();
                handle_preset_apply(
                    &parsed,
                    &mut state,
                    &mut session_id,
                    &mut pending,
                    rest,
                    &mut turns,
                );
            }
            "/approve" => resolve_pending(
                &parsed,
                &state,
                &mut session_id,
                &mut pending,
                true,
                &mut turns,
            ),
            "/deny" => resolve_pending(
                &parsed,
                &state,
                &mut session_id,
                &mut pending,
                false,
                &mut turns,
            ),
            value if value.starts_with("/approve ") || value.starts_with("/deny ") => {
                println!("usage: /approve and /deny do not accept arguments");
            }
            "/update" => handle_chat_update(&parsed, None),
            value if value.starts_with("/update ") => {
                let spec = value.trim_start_matches("/update ").trim();
                handle_chat_update(&parsed, Some(spec));
            }
            "/uninstall" => {
                println!(
                    "uninstall> Warning: This will remove Pandora configuration and data directories."
                );
                println!("uninstall> Workspace files will be preserved.");
                println!("uninstall> To confirm uninstallation, type: /uninstall --yes");
            }
            value if value.starts_with("/uninstall ") => {
                let arg = value.trim_start_matches("/uninstall ").trim();
                if arg == "--yes" || arg == "confirm" || arg == "-y" {
                    match super::uninstall::execute(&["--yes".to_owned()]) {
                        Ok(res) => {
                            println!("uninstall> {}", res.human);
                            break;
                        }
                        Err(err) => {
                            println!("uninstall> Error: {}", err.message);
                        }
                    }
                } else {
                    println!(
                        "uninstall> Unknown option '{arg}'. Type /uninstall --yes to confirm."
                    );
                }
            }
            command if command.starts_with('/') => run_slash(
                &parsed,
                &state,
                &mut session_id,
                &mut pending,
                command,
                None,
                &mut turns,
            ),
            task => {
                run_task(
                    &parsed,
                    &state,
                    &mut session_id,
                    &mut pending,
                    task,
                    None,
                    &mut turns,
                );
            }
        }
    }

    Ok(success(
        "chat",
        json!({
            "session_id": session_id,
            "status": "closed",
            "turns": turns,
        }),
        format!("Chat closed after {turns} turn(s)"),
    ))
}

fn handle_preset_list(parsed: &ParsedArgs) {
    let Ok(config) = load_config(parsed) else {
        println!("preset> could not load workspace configuration");
        return;
    };
    let user_config_dir = config
        .config_path()
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let engine = PromptEngine::open(config.workspace_dir(), user_config_dir);
    match engine.list() {
        Ok(presets) => {
            println!("presets available ({}):", presets.len());
            for (p, tier) in presets {
                let desc = p
                    .description
                    .as_deref()
                    .unwrap_or_else(|| &p.content[..p.content.len().min(40)]);
                println!(
                    "  • {} [{}, {}]: {}",
                    p.name,
                    p.kind.as_str(),
                    tier.as_str(),
                    desc
                );
            }
            println!(
                "Use '/preset <name> [task]' to apply or '/preset save <name> <content...>' to save."
            );
        }
        Err(err) => {
            println!("preset> error listing presets: {err}");
        }
    }
}

fn handle_preset_save(parsed: &ParsedArgs, rest: &str) {
    let (name, content) = match rest.split_once(char::is_whitespace) {
        Some((n, c)) => (n.trim(), c.trim()),
        None => {
            println!("usage: /preset save <name> <content...>");
            return;
        }
    };
    if name.is_empty() || content.is_empty() {
        println!("usage: /preset save <name> <content...>");
        return;
    }
    let Ok(config) = load_config(parsed) else {
        println!("preset> could not load workspace configuration");
        return;
    };
    let user_config_dir = config
        .config_path()
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let engine = PromptEngine::open(config.workspace_dir(), user_config_dir);
    let preset = PromptPreset {
        name: name.to_owned(),
        description: None,
        kind: PresetKind::Template,
        content: content.to_owned(),
        parameters: PromptEngine::extract_parameters(content),
        model: None,
        mode: None,
        created_at: 0,
        updated_at: 0,
    };
    match engine.save(preset, true) {
        Ok(path) => {
            println!("preset> saved '{}' to workspace ({})", name, path.display());
        }
        Err(err) => {
            println!("preset> error saving preset: {err}");
        }
    }
}

fn handle_preset_apply(
    parsed: &ParsedArgs,
    state: &mut InteractiveSessionState,
    session_id: &mut Option<String>,
    pending: &mut Option<PendingTask>,
    rest: &str,
    turns: &mut u32,
) {
    let (name, task_arg) = match rest.split_once(char::is_whitespace) {
        Some((n, t)) => (n.trim(), t.trim()),
        None => (rest.trim(), ""),
    };
    let Ok(config) = load_config(parsed) else {
        println!("preset> could not load workspace configuration");
        return;
    };
    let user_config_dir = config
        .config_path()
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let engine = PromptEngine::open(config.workspace_dir(), user_config_dir);
    match engine.get(name) {
        Ok((preset, _tier)) => {
            if preset.kind == PresetKind::Persona {
                state.active_persona = Some(preset.name.clone());
                if let Some(ref m) = preset.mode
                    && let Some(gov) = GovernanceMode::parse(m)
                {
                    state.active_mode = Some(gov);
                }
                if let Some(ref mdl) = preset.model {
                    state.active_model = Some(mdl.clone());
                }
                println!(
                    "preset> active persona set to '{}' (guidance active across turns)",
                    preset.name
                );
            } else {
                let mut params = BTreeMap::new();
                params.insert(
                    "workspace".to_owned(),
                    config.workspace_dir().display().to_string(),
                );
                params.insert(
                    "lang".to_owned(),
                    PromptEngine::detect_workspace_lang(config.workspace_dir()),
                );

                // Parse task_arg: support inline parameter overrides (target=foo or --target foo)
                let mut final_task_words = Vec::new();
                let words: Vec<&str> = task_arg.split_whitespace().collect();
                let mut i = 0;
                while i < words.len() {
                    let word = words[i];
                    if let Some((k, v)) = word.split_once('=') {
                        if !k.is_empty() && !v.is_empty() {
                            params.insert(k.trim().to_owned(), v.trim().to_owned());
                            i += 1;
                            continue;
                        }
                    } else if let Some(key) = word.strip_prefix("--")
                        && i + 1 < words.len()
                    {
                        i += 1;
                        params.insert(key.trim().to_owned(), words[i].trim().to_owned());
                        i += 1;
                        continue;
                    }
                    final_task_words.push(word);
                    i += 1;
                }

                // If target is missing, check if first positional word is a target path/module
                if !params.contains_key("target")
                    && (preset.parameters.iter().any(|p| p == "target")
                        || preset.content.contains("{{target}}"))
                {
                    if final_task_words.len() >= 2
                        && (final_task_words[0].contains('/')
                            || final_task_words[0].contains('\\')
                            || final_task_words[0].contains('.')
                            || config.workspace_dir().join(final_task_words[0]).exists())
                    {
                        params.insert("target".to_owned(), final_task_words.remove(0).to_owned());
                    } else {
                        let ws_name = config
                            .workspace_dir()
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("workspace");
                        params.insert("target".to_owned(), ws_name.to_owned());
                    }
                }

                let clean_task = if final_task_words.is_empty() {
                    if task_arg.is_empty() {
                        "inspect and improve implementation".to_owned()
                    } else {
                        task_arg.to_owned()
                    }
                } else {
                    final_task_words.join(" ")
                };
                params.insert("task".to_owned(), clean_task);

                match PromptEngine::interpolate(&preset.content, &params) {
                    Ok(interpolated) => {
                        println!("preset> applied '{}': {}", preset.name, interpolated);
                        run_task(
                            parsed,
                            state,
                            session_id,
                            pending,
                            &interpolated,
                            None,
                            turns,
                        );
                    }
                    Err(err) => {
                        println!("preset> interpolation error: {err}");
                    }
                }
            }
        }
        Err(err) => {
            println!("preset> error: {err}");
        }
    }
}

fn run_task(
    parsed: &ParsedArgs,
    state: &InteractiveSessionState,
    session_id: &mut Option<String>,
    pending: &mut Option<PendingTask>,
    task: &str,
    approval_id: Option<&str>,
    turns: &mut u32,
) {
    if approval_id.is_none() && pending.is_some() {
        println!("approval> resolve the pending approval first");
        return;
    }
    let run_args = run_args(parsed, state, session_id.as_deref(), task, approval_id);
    match run::execute(&run_args) {
        Ok(result) => {
            update_session(session_id, result.data.get("session_id"));
            print_result(&result);
        }
        Err(error) => {
            update_session(session_id, error.details.get("session_id"));
            let approval_id = error
                .details
                .get("approval_id")
                .and_then(Value::as_str)
                .map(str::to_owned);
            print_error(&error);
            if let Some(approval_id) = approval_id {
                println!(
                    "approval> inspect with 'pandora approval inspect {approval_id}', then use /approve or /deny"
                );
                *pending = Some(PendingTask {
                    invocation: PendingInvocation::Agent(task.to_owned()),
                    approval_id,
                });
            }
        }
    }
    *turns = turns.saturating_add(1);
}

fn run_slash(
    parsed: &ParsedArgs,
    state: &InteractiveSessionState,
    session_id: &mut Option<String>,
    pending: &mut Option<PendingTask>,
    line: &str,
    approval_id: Option<&str>,
    turns: &mut u32,
) {
    if approval_id.is_none() && pending.is_some() {
        println!("approval> resolve the pending approval first");
        return;
    }
    let mode = state.active_mode.map(|m| m.as_str());
    match slash::execute_interactive(
        line,
        parsed,
        session_id.as_deref(),
        approval_id,
        mode,
        state.active_provider.as_deref(),
        state.active_model.as_deref(),
    ) {
        Ok(result) => {
            update_session(session_id, result.data.get("session_id"));
            print_result(&result);
        }
        Err(error) => {
            update_session(session_id, error.details.get("session_id"));
            let approval_id = error
                .details
                .get("approval_id")
                .and_then(Value::as_str)
                .map(str::to_owned);
            print_error(&error);
            if let Some(approval_id) = approval_id {
                println!(
                    "approval> inspect with 'pandora approval inspect {approval_id}', then use /approve or /deny"
                );
                *pending = Some(PendingTask {
                    invocation: PendingInvocation::Slash(line.to_owned()),
                    approval_id,
                });
            }
        }
    }
    *turns = turns.saturating_add(1);
}

fn resolve_pending(
    parsed: &ParsedArgs,
    state: &InteractiveSessionState,
    session_id: &mut Option<String>,
    pending: &mut Option<PendingTask>,
    allow: bool,
    turns: &mut u32,
) {
    let Some(pending_task) = pending.take() else {
        println!("approval> no pending approval");
        return;
    };
    let PendingTask {
        invocation,
        approval_id,
    } = pending_task;
    match approval::execute(&approval_args(parsed, &approval_id, allow)) {
        Ok(result) => {
            print_result(&result);
            if allow {
                match invocation {
                    PendingInvocation::Agent(task) => run_task(
                        parsed,
                        state,
                        session_id,
                        pending,
                        &task,
                        Some(&approval_id),
                        turns,
                    ),
                    PendingInvocation::Slash(line) => run_slash(
                        parsed,
                        state,
                        session_id,
                        pending,
                        &line,
                        Some(&approval_id),
                        turns,
                    ),
                }
            }
        }
        Err(error) => {
            print_error(&error);
            *pending = Some(PendingTask {
                invocation,
                approval_id,
            });
        }
    }
}

fn approval_args(parsed: &ParsedArgs, approval_id: &str, allow: bool) -> Vec<String> {
    let mut args = vec![
        "resolve".to_owned(),
        approval_id.to_owned(),
        if allow {
            "--allow".to_owned()
        } else {
            "--deny".to_owned()
        },
    ];
    for name in ["config", "data-dir", "workspace"] {
        if let Some(value) = parsed.value(name) {
            args.push(format!("--{name}"));
            args.push(value.to_owned());
        }
    }
    args
}

fn run_args(
    parsed: &ParsedArgs,
    state: &InteractiveSessionState,
    session_id: Option<&str>,
    task: &str,
    approval_id: Option<&str>,
) -> Vec<String> {
    let mut args = vec!["--agent".to_owned()];
    for name in [
        "config",
        "data-dir",
        "workspace",
        "max-turns",
        "max-tools",
        "tool-profile",
        "tools",
    ] {
        if let Some(value) = parsed.value(name) {
            args.push(format!("--{name}"));
            args.push(value.to_owned());
        }
    }
    if let Some(provider) = state
        .active_provider
        .as_deref()
        .or_else(|| parsed.value("provider"))
    {
        args.push("--provider".to_owned());
        args.push(provider.to_owned());
    }
    if let Some(model) = state
        .active_model
        .as_deref()
        .or_else(|| parsed.value("model"))
    {
        args.push("--model".to_owned());
        args.push(model.to_owned());
    }
    let mode = state.active_mode.unwrap_or(GovernanceMode::Supervised);
    args.push("--mode".to_owned());
    args.push(mode.as_str().to_owned());

    if let Some(session_id) = session_id {
        args.push("--session".to_owned());
        args.push(session_id.to_owned());
    }
    if let Some(approval_id) = approval_id {
        args.push("--approval".to_owned());
        args.push(approval_id.to_owned());
    }
    args.push(task.to_owned());
    args
}

fn update_session(session_id: &mut Option<String>, value: Option<&Value>) {
    if let Some(value) = value.and_then(Value::as_str) {
        *session_id = Some(value.to_owned());
    }
}

fn handle_chat_update(parsed: &super::ParsedArgs, target: Option<&str>) {
    println!(
        "update> Checking for updates (current version: v{})...",
        env!("CARGO_PKG_VERSION")
    );
    let mut update_args = Vec::new();
    if let Some(config) = parsed.value("config") {
        update_args.extend(["--config".to_owned(), config.to_owned()]);
    }
    if let Some(data_dir) = parsed.value("data-dir") {
        update_args.extend(["--data-dir".to_owned(), data_dir.to_owned()]);
    }
    if let Some(target_spec) = target {
        match target_spec {
            "--check" | "check" => update_args.push("--check".to_owned()),
            "--dry-run" | "dry-run" => update_args.push("--dry-run".to_owned()),
            "rollback" => update_args.push("--rollback".to_owned()),
            version => update_args.extend(["--release".to_owned(), version.to_owned()]),
        }
    }
    match super::update::execute(&update_args) {
        Ok(res) => {
            println!("update> {}", res.human);
        }
        Err(err) => {
            println!("update> Error: {}", err.message);
        }
    }
}

fn print_help() {
    println!("/help       show chat commands");
    println!("/session    show the active session ID");
    println!("/model      show or switch active model (/model <provider:model>)");
    println!(
        "/mode       show or switch governance mode (/mode <autonomous|supervised|read-only>)"
    );
    println!("/permission show or switch permission level (/permission <0|1|2>)");
    println!("/preset     list, apply, or save prompt presets (/preset <name> [task])");
    println!("/approve    approve and resume the pending task");
    println!("/deny       deny the pending task");
    println!(
        "/update     check for updates or upgrade to a version (/update [ver|check|rollback])"
    );
    println!("/uninstall  uninstall Pandora configuration and data (/uninstall --yes)");
    println!("/exit       close the chat");
    println!("/quit       close the chat");
    println!("/coding     inspect the Coding Domain Harness");
    println!(
        "/read, /search, /patch, /verify, /test, /format, /lint, /build, /status, /diff, /log, /refs, /review run its core Genes"
    );
    println!("/audit, /argus-review, /debt, /measure, /guide run its workflow Genes");
    println!("Any other line is sent as a bounded agent task.");
}

fn print_session(session_id: Option<&str>) {
    match session_id {
        Some(session_id) => println!("session: {session_id}"),
        None => println!("session: not started"),
    }
}

fn print_result(result: &CommandResult) {
    if let Some(output) = result.data.get("output").and_then(Value::as_str) {
        println!("{}", clean_text(output));
    } else {
        println!("{}", clean_text(&result.human));
    }
}

fn print_error(error: &CliError) {
    if let Some(approval_id) = error.details.get("approval_id").and_then(Value::as_str) {
        println!(
            "{} (approval: {})",
            clean_text(&error.message),
            clean_text(approval_id)
        );
    } else {
        println!("error: {}", clean_text(&error.message));
    }
}

fn clean_text(value: &str) -> String {
    value
        .chars()
        .filter(|character| *character == '\n' || *character == '\t' || !character.is_control())
        .take(MAX_DISPLAY_CHARS)
        .collect()
}
