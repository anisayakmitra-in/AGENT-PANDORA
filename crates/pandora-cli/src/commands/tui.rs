use super::{ParsedArgs, parse_options};
use crate::output::{CliError, CommandResult, success};
use crossterm::cursor::MoveTo;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::queue;
use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use serde_json::{Value, json};
use std::cmp::min;
use std::io::{self, IsTerminal, Stdout, Write};
use std::time::Duration;

const MAX_DISPLAY_CHARS: usize = 64 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(250);

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
        return Err(CliError::usage("tui does not accept a positional task"));
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(CliError::configuration(
            "tui requires an interactive terminal",
            json!({}),
        ));
    }

    let mut terminal = TerminalSession::enter()?;
    let mut app = App::new(parsed);
    let result = app.run(&mut terminal.stdout);
    terminal.restore();
    result?;

    Ok(success(
        "tui",
        json!({
            "session_id": app.session_id,
            "status": "closed",
            "turns": app.turns,
        }),
        format!("TUI closed after {} turn(s)", app.turns),
    ))
}

struct TerminalSession {
    stdout: Stdout,
    active: bool,
}

impl TerminalSession {
    fn enter() -> Result<Self, CliError> {
        terminal::enable_raw_mode().map_err(terminal_error)?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen) {
            let _ = terminal::disable_raw_mode();
            return Err(terminal_error(error));
        }
        Ok(Self {
            stdout,
            active: true,
        })
    }

    fn restore(&mut self) {
        if !self.active {
            return;
        }
        let _ = execute!(self.stdout, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
        self.active = false;
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        self.restore();
    }
}

struct App {
    args: ParsedArgs,
    input: Vec<char>,
    cursor: usize,
    history: Vec<String>,
    history_index: Option<usize>,
    messages: Vec<String>,
    session_id: Option<String>,
    pending: Option<PendingTask>,
    turns: u32,
}

struct PendingTask {
    task: String,
    approval_id: String,
}

impl App {
    fn new(args: ParsedArgs) -> Self {
        Self {
            session_id: args.value("session").map(str::to_owned),
            args,
            input: Vec::new(),
            cursor: 0,
            history: Vec::new(),
            history_index: None,
            messages: vec![
                "Pandora TUI".to_owned(),
                "Enter a task. Press Ctrl-C or Esc to close; /help lists commands.".to_owned(),
            ],
            turns: 0,
            pending: None,
        }
    }

    fn run(&mut self, stdout: &mut Stdout) -> Result<(), CliError> {
        loop {
            self.draw(stdout)?;
            if !event::poll(POLL_INTERVAL).map_err(terminal_error)? {
                continue;
            }
            let event = event::read().map_err(terminal_error)?;
            if let Event::Key(key) = event
                && self.handle_key(key)
            {
                break;
            }
        }
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return true;
        }
        match key.code {
            KeyCode::Esc => true,
            KeyCode::Enter => self.submit(),
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.input.remove(self.cursor);
                }
                false
            }
            KeyCode::Delete => {
                if self.cursor < self.input.len() {
                    self.input.remove(self.cursor);
                }
                false
            }
            KeyCode::Left => {
                self.cursor = self.cursor.saturating_sub(1);
                false
            }
            KeyCode::Right => {
                self.cursor = min(self.cursor.saturating_add(1), self.input.len());
                false
            }
            KeyCode::Home => {
                self.cursor = 0;
                false
            }
            KeyCode::End => {
                self.cursor = self.input.len();
                false
            }
            KeyCode::Up => {
                self.previous_history();
                false
            }
            KeyCode::Down => {
                self.next_history();
                false
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.input.insert(self.cursor, character);
                self.cursor += 1;
                false
            }
            _ => false,
        }
    }

    fn submit(&mut self) -> bool {
        let line = self.input.iter().collect::<String>();
        self.input.clear();
        self.cursor = 0;
        self.history_index = None;
        let task = line.trim().to_owned();
        if task.is_empty() {
            return false;
        }
        match task.as_str() {
            "/exit" | "/quit" => {
                self.messages.push("Closing...".to_owned());
                return true;
            }
            "/help" => self.messages.extend([
                "/help       show TUI commands".to_owned(),
                "/session    show the active session ID".to_owned(),
                "/clear      clear the transcript".to_owned(),
                "/approve    approve and resume the pending task".to_owned(),
                "/deny       deny the pending task".to_owned(),
                "/exit       close the TUI".to_owned(),
            ]),
            "/session" => self.messages.push(match self.session_id.as_deref() {
                Some(session_id) => format!("session: {session_id}"),
                None => "session: not started".to_owned(),
            }),
            "/clear" => self.messages.clear(),
            "/approve" => self.resolve_pending(true),
            "/deny" => self.resolve_pending(false),
            value if value.starts_with("/approve ") || value.starts_with("/deny ") => {
                self.messages
                    .push("usage> /approve and /deny do not accept arguments".to_owned());
            }
            _ => self.run_task(task),
        }
        if !line.trim().is_empty()
            && !matches!(
                line.trim(),
                "/exit" | "/quit" | "/help" | "/session" | "/clear" | "/approve" | "/deny"
            )
            && !line.trim().starts_with("/approve ")
            && !line.trim().starts_with("/deny ")
            && self.history.last().map(String::as_str) != Some(line.trim())
        {
            self.history.push(line.trim().to_owned());
        }
        false
    }

    fn run_task(&mut self, task: String) {
        self.run_task_with_approval(task, None);
    }

    fn run_task_with_approval(&mut self, task: String, approval_id: Option<&str>) {
        if approval_id.is_none() && self.pending.is_some() {
            self.messages
                .push("approval> resolve the pending approval first".to_owned());
            return;
        }
        self.messages.push(format!("you> {task}"));
        let message_index = self.messages.len();
        self.messages.push("pandora> working...".to_owned());
        let args = self.run_args(&task, approval_id);
        match super::run::execute(&args) {
            Ok(result) => {
                update_session(&mut self.session_id, result.data.get("session_id"));
                let output = result
                    .data
                    .get("output")
                    .and_then(Value::as_str)
                    .unwrap_or(&result.human);
                self.messages[message_index] = format!("pandora> {}", clean_text(output));
            }
            Err(error) => {
                update_session(&mut self.session_id, error.details.get("session_id"));
                self.messages[message_index] = format!("error> {}", clean_text(&error.message));
                if let Some(approval_id) = error.details.get("approval_id").and_then(Value::as_str)
                {
                    self.messages[message_index].push_str(&format!(" (approval: {approval_id})"));
                    self.pending = Some(PendingTask {
                        task,
                        approval_id: approval_id.to_owned(),
                    });
                }
            }
        }
        self.turns = self.turns.saturating_add(1);
    }

    fn resolve_pending(&mut self, allow: bool) {
        let Some(pending) = self.pending.take() else {
            self.messages
                .push("approval> no pending approval".to_owned());
            return;
        };
        let approval_args = self.approval_args(&pending.approval_id, allow);
        match super::approval::execute(&approval_args) {
            Ok(result) => {
                self.messages
                    .push(format!("approval> {}", clean_text(&result.human)));
                if allow {
                    self.run_task_with_approval(pending.task, Some(&pending.approval_id));
                }
            }
            Err(error) => {
                self.messages
                    .push(format!("error> {}", clean_text(&error.message)));
                self.pending = Some(pending);
            }
        }
    }

    fn approval_args(&self, approval_id: &str, allow: bool) -> Vec<String> {
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
            if let Some(value) = self.args.value(name) {
                args.push(format!("--{name}"));
                args.push(value.to_owned());
            }
        }
        args
    }

    fn run_args(&self, task: &str, approval_id: Option<&str>) -> Vec<String> {
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
            if let Some(value) = self.args.value(name) {
                args.push(format!("--{name}"));
                args.push(value.to_owned());
            }
        }
        if let Some(session_id) = self.session_id.as_deref() {
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

    fn previous_history(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let index = self
            .history_index
            .map_or(self.history.len() - 1, |index| index.saturating_sub(1));
        self.history_index = Some(index);
        self.input = self.history[index].chars().collect();
        self.cursor = self.input.len();
    }

    fn next_history(&mut self) {
        let Some(index) = self.history_index else {
            return;
        };
        if index + 1 >= self.history.len() {
            self.history_index = None;
            self.input.clear();
            self.cursor = 0;
            return;
        }
        let next = index + 1;
        self.history_index = Some(next);
        self.input = self.history[next].chars().collect();
        self.cursor = self.input.len();
    }

    fn draw(&self, stdout: &mut Stdout) -> Result<(), CliError> {
        let (width, height) = terminal::size().map_err(terminal_error)?;
        let width = usize::from(width).max(1);
        let height = usize::from(height).max(4);
        let body_height = height.saturating_sub(4);
        let mut lines = Vec::new();
        for message in &self.messages {
            wrap_message(message, width.saturating_sub(2), &mut lines);
        }
        let first = lines.len().saturating_sub(body_height);

        queue!(
            stdout,
            Clear(ClearType::All),
            MoveTo(0, 0),
            SetForegroundColor(Color::Cyan),
            Print("PANDORA TUI"),
            ResetColor,
            MoveTo(0, 1),
            SetForegroundColor(Color::DarkGrey),
            Print("Governed execution · Ctrl-C/Esc exits · /help for commands"),
            ResetColor,
        )
        .map_err(terminal_error)?;
        for (offset, line) in lines.iter().skip(first).take(body_height).enumerate() {
            queue!(
                stdout,
                MoveTo(1, (2 + offset) as u16),
                Print(truncate(line, width - 2))
            )
            .map_err(terminal_error)?;
        }

        let session = self.session_id.as_deref().unwrap_or("not started");
        let status = truncate(
            &format!("session: {session} · turns: {}", self.turns),
            width,
        );
        queue!(
            stdout,
            MoveTo(0, height.saturating_sub(2) as u16),
            SetForegroundColor(Color::DarkGrey),
            Print(status),
            ResetColor,
        )
        .map_err(terminal_error)?;

        let (input, cursor) = visible_input(&self.input, self.cursor, width);
        queue!(
            stdout,
            MoveTo(0, height.saturating_sub(1) as u16),
            SetForegroundColor(Color::Green),
            Print("> "),
            ResetColor,
            Print(input),
            MoveTo(cursor, height.saturating_sub(1) as u16),
        )
        .map_err(terminal_error)?;
        stdout.flush().map_err(terminal_error)
    }
}

fn update_session(session_id: &mut Option<String>, value: Option<&Value>) {
    if let Some(value) = value.and_then(Value::as_str) {
        *session_id = Some(value.to_owned());
    }
}

fn visible_input(input: &[char], cursor: usize, width: usize) -> (String, u16) {
    let available = width.saturating_sub(2).max(1);
    let start = cursor.saturating_sub(available);
    let end = min(input.len(), start + available);
    let visible = input[start..end].iter().collect::<String>();
    let cursor = min(2 + cursor.saturating_sub(start), width.saturating_sub(1));
    (visible, cursor as u16)
}

fn wrap_message(message: &str, width: usize, output: &mut Vec<String>) {
    let width = width.max(1);
    let mut line = String::new();
    for character in clean_text(message).chars() {
        if character == '\n' || line.chars().count() >= width {
            output.push(line);
            line = String::new();
            if character == '\n' {
                continue;
            }
        }
        line.push(character);
    }
    if !line.is_empty() || output.is_empty() {
        output.push(line);
    }
}

fn truncate(value: &str, width: usize) -> String {
    value.chars().take(width).collect()
}

fn clean_text(value: &str) -> String {
    value
        .chars()
        .filter(|character| *character == '\n' || *character == '\t' || !character.is_control())
        .take(MAX_DISPLAY_CHARS)
        .collect()
}

fn terminal_error(error: impl std::fmt::Display) -> CliError {
    CliError::internal(format!("terminal error: {error}"), json!({}))
}

#[cfg(test)]
mod tests {
    use super::{App, visible_input, wrap_message};
    use crate::commands::ParsedArgs;
    use std::collections::BTreeMap;

    fn app() -> App {
        App::new(ParsedArgs {
            values: BTreeMap::new(),
            positionals: Vec::new(),
        })
    }

    #[test]
    fn visible_input_keeps_cursor_inside_the_window() {
        let input = "abcdefgh".chars().collect::<Vec<_>>();
        let (visible, cursor) = visible_input(&input, 8, 6);
        assert_eq!(visible, "efgh");
        assert_eq!(cursor, 5);
    }

    #[test]
    fn wrapped_messages_preserve_newlines() {
        let mut lines = Vec::new();
        wrap_message("one\ntwo", 20, &mut lines);
        assert_eq!(lines, ["one", "two"]);
    }

    #[test]
    fn help_lists_approval_commands() {
        let mut app = app();
        app.input = "/help".chars().collect();
        app.submit();
        assert!(
            app.messages
                .iter()
                .any(|message| message == "/approve    approve and resume the pending task")
        );
        assert!(
            app.messages
                .iter()
                .any(|message| message == "/deny       deny the pending task")
        );
    }

    #[test]
    fn approval_commands_report_when_no_task_is_pending() {
        let mut app = app();
        app.input = "/approve".chars().collect();
        app.submit();
        assert_eq!(
            app.messages.last().map(String::as_str),
            Some("approval> no pending approval")
        );
    }

    #[test]
    fn approval_id_is_forwarded_when_resuming_a_task() {
        let app = app();
        assert_eq!(
            app.run_args("patch:README.md:updated", Some("approval-1")),
            [
                "--agent",
                "--approval",
                "approval-1",
                "patch:README.md:updated"
            ]
        );
    }
}
