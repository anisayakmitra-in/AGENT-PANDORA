use super::{ParsedArgs, parse_options};
use super::{approval, run, slash};
use crate::output::{CliError, CommandResult, success};
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};

const MAX_DISPLAY_CHARS: usize = 64 * 1024;

struct PendingTask {
    invocation: PendingInvocation,
    approval_id: String,
}

enum PendingInvocation {
    Agent(String),
    Slash(String),
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
        ],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage("chat does not accept a positional task"));
    }

    let mut session_id = parsed.value("session").map(str::to_owned);
    let mut turns = 0u32;
    let mut pending = None;
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut stdout = io::stdout();
    println!("Pandora chat. Type /help for commands or /exit to close.");

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
            "/approve" => resolve_pending(&parsed, &mut session_id, &mut pending, true, &mut turns),
            "/deny" => resolve_pending(&parsed, &mut session_id, &mut pending, false, &mut turns),
            value if value.starts_with("/approve ") || value.starts_with("/deny ") => {
                println!("usage: /approve and /deny do not accept arguments");
            }
            command if command.starts_with('/') => run_slash(
                &parsed,
                &mut session_id,
                &mut pending,
                command,
                None,
                &mut turns,
            ),
            task => {
                run_task(
                    &parsed,
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

fn run_task(
    parsed: &ParsedArgs,
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
    let run_args = run_args(parsed, session_id.as_deref(), task, approval_id);
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
    match slash::execute_interactive(line, parsed, session_id.as_deref(), approval_id) {
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
                        session_id,
                        pending,
                        &task,
                        Some(&approval_id),
                        turns,
                    ),
                    PendingInvocation::Slash(line) => run_slash(
                        parsed,
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
    session_id: Option<&str>,
    task: &str,
    approval_id: Option<&str>,
) -> Vec<String> {
    let mut args = vec!["--agent".to_owned()];
    for name in [
        "config",
        "data-dir",
        "workspace",
        "provider",
        "model",
        "max-turns",
        "max-tools",
    ] {
        if let Some(value) = parsed.value(name) {
            args.push(format!("--{name}"));
            args.push(value.to_owned());
        }
    }
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

fn print_help() {
    println!("/help       show chat commands");
    println!("/session    show the active session ID");
    println!("/approve    approve and resume the pending task");
    println!("/deny       deny the pending task");
    println!("/exit       close the chat");
    println!("/quit       close the chat");
    println!("/coding     inspect the Coding Domain Harness");
    println!("/read, /search, /patch, /verify, /test, /review run its core Genes");
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
