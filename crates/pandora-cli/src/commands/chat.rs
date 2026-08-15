use super::run;
use super::{ParsedArgs, parse_options};
use crate::output::{CliError, CommandResult, success};
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};

const MAX_DISPLAY_CHARS: usize = 64 * 1024;

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
            task => {
                let run_args = run_args(&parsed, session_id.as_deref(), task);
                match run::execute(&run_args) {
                    Ok(result) => {
                        update_session(&mut session_id, result.data.get("session_id"));
                        print_result(&result);
                    }
                    Err(error) => {
                        update_session(&mut session_id, error.details.get("session_id"));
                        print_error(&error);
                    }
                }
                turns = turns.saturating_add(1);
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

fn run_args(parsed: &ParsedArgs, session_id: Option<&str>, task: &str) -> Vec<String> {
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
    println!("/exit       close the chat");
    println!("/quit       close the chat");
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
