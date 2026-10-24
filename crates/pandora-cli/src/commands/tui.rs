use super::{ParsedArgs, parse_options, slash};
use crate::output::{CliError, CommandResult, success};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Color as TuiColor, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, Gauge, List, ListItem, Paragraph, Scrollbar,
    ScrollbarOrientation, ScrollbarState, Sparkline, Tabs,
};
use ratatui::{Frame, Terminal};
use serde_json::{Value, json};
use std::cmp::min;
use std::io::{self, IsTerminal, Stdout};
use std::time::Duration;

const MAX_DISPLAY_CHARS: usize = 64 * 1024;
const MAX_TUI_MESSAGES: usize = 512;
const MAX_TUI_HISTORY: usize = 100;
const MAX_SPARKLINE_POINTS: usize = 32;
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const WIDE_LAYOUT_MIN_WIDTH: u16 = 88;
const WIDE_LAYOUT_MIN_HEIGHT: u16 = 16;

type TuiTerminal = Terminal<CrosstermBackend<Stdout>>;

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
            "theme",
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

    let theme = TuiTheme::parse(parsed.value("theme"))?;
    let mut terminal = TerminalSession::enter()?;
    let mut app = App::new(parsed, theme);
    let result = app.run(&mut terminal.terminal);
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
    terminal: TuiTerminal,
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
        let terminal = match Terminal::new(CrosstermBackend::new(stdout)) {
            Ok(terminal) => terminal,
            Err(error) => {
                let _ = terminal::disable_raw_mode();
                let _ = execute!(io::stdout(), LeaveAlternateScreen);
                return Err(terminal_error(error));
            }
        };
        Ok(Self {
            terminal,
            active: true,
        })
    }

    fn restore(&mut self) {
        if !self.active {
            return;
        }
        let _ = self.terminal.show_cursor();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
        self.active = false;
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        self.restore();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TuiTab {
    Console,
    Kanban,
    Memory,
    Governance,
}

impl TuiTab {
    const ALL: [Self; 4] = [Self::Console, Self::Kanban, Self::Memory, Self::Governance];

    fn index(self) -> usize {
        match self {
            Self::Console => 0,
            Self::Kanban => 1,
            Self::Memory => 2,
            Self::Governance => 3,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Console => "console",
            Self::Kanban => "kanban",
            Self::Memory => "memory",
            Self::Governance => "governance",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Console => " 1:Console ",
            Self::Kanban => " 2:Kanban & Fleet ",
            Self::Memory => " 3:Memory & Graph ",
            Self::Governance => " 4:Governance & Trust ",
        }
    }

    fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    fn previous(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "1" | "console" | "chat" | "agent" => Some(Self::Console),
            "2" | "kanban" | "fleet" | "board" => Some(Self::Kanban),
            "3" | "memory" | "graph" | "context" => Some(Self::Memory),
            "4" | "governance" | "trust" | "packages" => Some(Self::Governance),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PaletteEntry {
    command: &'static str,
    category: &'static str,
    description: &'static str,
    immediate: bool,
}

const PALETTE_ENTRIES: &[PaletteEntry] = &[
    PaletteEntry {
        command: "/help",
        category: "Core",
        description: "Show all TUI commands and workflow shortcuts",
        immediate: true,
    },
    PaletteEntry {
        command: "/setup",
        category: "Setup",
        description: "Show available model provider presets (ollama, openrouter, deepseek, openai, anthropic, groq)",
        immediate: true,
    },
    PaletteEntry {
        command: "/setup ollama",
        category: "Setup",
        description: "Configure local Ollama provider preset (http://localhost:11434/v1)",
        immediate: true,
    },
    PaletteEntry {
        command: "/setup openrouter",
        category: "Setup",
        description: "Configure OpenRouter gateway provider preset",
        immediate: true,
    },
    PaletteEntry {
        command: "/setup anthropic",
        category: "Setup",
        description: "Configure Anthropic Claude provider preset",
        immediate: true,
    },
    PaletteEntry {
        command: "/setup openai",
        category: "Setup",
        description: "Configure OpenAI Direct provider preset",
        immediate: true,
    },
    PaletteEntry {
        command: "/fleet-health",
        category: "Fleet",
        description: "Inspect local privacy-safe fleet & budget operations snapshot",
        immediate: true,
    },
    PaletteEntry {
        command: "/kanban",
        category: "Kanban",
        description: "Load and display local Kanban board tasks by status column",
        immediate: true,
    },
    PaletteEntry {
        command: "/packages",
        category: "Packages",
        description: "List locally admitted package identities and states",
        immediate: true,
    },
    PaletteEntry {
        command: "/distribution",
        category: "Packages",
        description: "List verified downloads and their trust/admission state",
        immediate: true,
    },
    PaletteEntry {
        command: "/approve",
        category: "Governance",
        description: "Approve and resume the currently pending governed task",
        immediate: true,
    },
    PaletteEntry {
        command: "/deny",
        category: "Governance",
        description: "Deny and abort the currently pending governed task",
        immediate: true,
    },
    PaletteEntry {
        command: "/status",
        category: "Coding Gene",
        description: "Run core Coding Gene: workspace git status inspection",
        immediate: true,
    },
    PaletteEntry {
        command: "/diff",
        category: "Coding Gene",
        description: "Run core Coding Gene: inspect current workspace git diff",
        immediate: true,
    },
    PaletteEntry {
        command: "/test",
        category: "Coding Gene",
        description: "Run core Coding Gene: execute workspace test verification",
        immediate: true,
    },
    PaletteEntry {
        command: "/lint",
        category: "Coding Gene",
        description: "Run core Coding Gene: run static linter analysis",
        immediate: true,
    },
    PaletteEntry {
        command: "/build",
        category: "Coding Gene",
        description: "Run core Coding Gene: run workspace build check",
        immediate: true,
    },
    PaletteEntry {
        command: "/audit",
        category: "Workflow Gene",
        description: "Run Daedalus workspace architecture & inventory audit",
        immediate: true,
    },
    PaletteEntry {
        command: "/debt",
        category: "Workflow Gene",
        description: "Run Ariadne technical debt marker inspection",
        immediate: true,
    },
    PaletteEntry {
        command: "/measure",
        category: "Workflow Gene",
        description: "Run Hephaestus verification measurement workflow",
        immediate: true,
    },
    PaletteEntry {
        command: "/guide",
        category: "Workflow Gene",
        description: "Run Athena architectural guidance workflow",
        immediate: true,
    },
    PaletteEntry {
        command: "/coding",
        category: "Harness",
        description: "Inspect the Coding Domain Harness capabilities",
        immediate: true,
    },
    PaletteEntry {
        command: "/tab console",
        category: "View",
        description: "Switch workspace view to Tab 1: Agent Console",
        immediate: true,
    },
    PaletteEntry {
        command: "/tab kanban",
        category: "View",
        description: "Switch workspace view to Tab 2: Kanban & Fleet",
        immediate: true,
    },
    PaletteEntry {
        command: "/tab memory",
        category: "View",
        description: "Switch workspace view to Tab 3: Memory & Graph",
        immediate: true,
    },
    PaletteEntry {
        command: "/tab governance",
        category: "View",
        description: "Switch workspace view to Tab 4: Governance & Trust",
        immediate: true,
    },
    PaletteEntry {
        command: "/sidebar",
        category: "View",
        description: "Toggle the right-hand Telemetry & Inspector sidebar",
        immediate: true,
    },
    PaletteEntry {
        command: "/theme auto",
        category: "Theme",
        description: "Switch presentation palette to Auto (Cyan/Green)",
        immediate: true,
    },
    PaletteEntry {
        command: "/theme dark",
        category: "Theme",
        description: "Switch presentation palette to High-Contrast Dark",
        immediate: true,
    },
    PaletteEntry {
        command: "/theme light",
        category: "Theme",
        description: "Switch presentation palette to Daylight Blue/Green",
        immediate: true,
    },
    PaletteEntry {
        command: "/theme mono",
        category: "Theme",
        description: "Switch presentation palette to Monochrome terminal style",
        immediate: true,
    },
    PaletteEntry {
        command: "/rollout ",
        category: "Evolution",
        description: "Inspect staged rollout budgets, scorecards, and approvals by ID",
        immediate: false,
    },
    PaletteEntry {
        command: "/memory-retention",
        category: "Memory",
        description: "Show the scoped revoked-record compaction workflow",
        immediate: true,
    },
    PaletteEntry {
        command: "/memory-transfer",
        category: "Memory",
        description: "Show the cross-project memory consolidation policy",
        immediate: true,
    },
    PaletteEntry {
        command: "/storage-lifecycle",
        category: "Governance",
        description: "Show provider storage lifecycle evidence commands",
        immediate: true,
    },
    PaletteEntry {
        command: "/trust-transparency",
        category: "Governance",
        description: "Show chained trust-root and package admission transparency commands",
        immediate: true,
    },
    PaletteEntry {
        command: "/domain-starter",
        category: "SDK",
        description: "Show the safe Domain Harness scaffold workflow",
        immediate: true,
    },
    PaletteEntry {
        command: "/meta-starter",
        category: "SDK",
        description: "Show the safe Meta Harness scaffold workflow",
        immediate: true,
    },
    PaletteEntry {
        command: "/gene-pack",
        category: "SDK",
        description: "Show the declarative Gene pack evaluation workflow",
        immediate: true,
    },
    PaletteEntry {
        command: "/canary-loop",
        category: "Evolution",
        description: "Show the governed scheduled-canary evaluation workflow",
        immediate: true,
    },
    PaletteEntry {
        command: "/session",
        category: "Core",
        description: "Display the active governed session ID",
        immediate: true,
    },
    PaletteEntry {
        command: "/activity",
        category: "Core",
        description: "Display the typed public TUI activity state",
        immediate: true,
    },
    PaletteEntry {
        command: "/clear",
        category: "Core",
        description: "Clear the console transcript buffer",
        immediate: true,
    },
    PaletteEntry {
        command: "/exit",
        category: "Core",
        description: "Close the Pandora TUI session",
        immediate: true,
    },
];

#[derive(Clone, Debug, Default)]
struct KanbanSnapshot {
    board_id: String,
    todo: Vec<String>,
    in_progress: Vec<String>,
    blocked: Vec<String>,
    done: Vec<String>,
}

#[derive(Clone, Debug, Default)]
struct FleetSnapshot {
    status: String,
    ready_nodes: u64,
    running_supervisors: u64,
    stale_supervisors: u64,
    jobs_queued: u64,
    jobs_running: u64,
    orchestrations_queued: u64,
    orchestrations_running: u64,
    active_leases: u64,
    run_count: u64,
    tokens_consumed: u64,
    tokens_ceiling: u64,
    cost_display: String,
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
    theme: TuiTheme,
    activity: TuiActivity,
    active_tab: TuiTab,
    sidebar_visible: bool,
    scroll_offset: usize,
    palette_open: bool,
    palette_query: Vec<char>,
    palette_index: usize,
    sparkline_history: Vec<u64>,
    kanban_snapshot: KanbanSnapshot,
    fleet_snapshot: FleetSnapshot,
    packages_cache: Vec<String>,
    distribution_cache: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TuiActivity {
    Idle,
    Working,
    Waiting,
    Success,
    Failure,
}

impl TuiActivity {
    fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Working => "working",
            Self::Waiting => "waiting",
            Self::Success => "success",
            Self::Failure => "failure",
        }
    }

    fn badge_label(self) -> &'static str {
        match self {
            Self::Idle => "● IDLE",
            Self::Working => "◉ WORKING",
            Self::Waiting => "⚠ WAITING APPROVAL",
            Self::Success => "✔ SUCCESS",
            Self::Failure => "✖ FAILURE",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TuiTheme {
    Auto,
    Dark,
    Light,
    Mono,
}

impl TuiTheme {
    fn parse(value: Option<&str>) -> Result<Self, CliError> {
        match value.unwrap_or("auto") {
            "auto" => Ok(Self::Auto),
            "dark" => Ok(Self::Dark),
            "light" => Ok(Self::Light),
            "mono" => Ok(Self::Mono),
            value => Err(CliError::usage(format!(
                "invalid tui theme '{value}'; expected auto, dark, light, or mono"
            ))),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Dark => "dark",
            Self::Light => "light",
            Self::Mono => "mono",
        }
    }

    fn title(self) -> Style {
        let style = match self {
            Self::Auto => Style::default().fg(TuiColor::Cyan),
            Self::Dark => Style::default().fg(TuiColor::LightCyan),
            Self::Light => Style::default().fg(TuiColor::Blue),
            Self::Mono => Style::default(),
        };
        style.add_modifier(Modifier::BOLD)
    }

    fn muted(self) -> Style {
        match self {
            Self::Auto | Self::Light => Style::default().fg(TuiColor::DarkGray),
            Self::Dark => Style::default().fg(TuiColor::Gray),
            Self::Mono => Style::default().add_modifier(Modifier::DIM),
        }
    }

    fn prompt(self) -> Style {
        match self {
            Self::Auto => Style::default().fg(TuiColor::Green),
            Self::Dark => Style::default().fg(TuiColor::LightGreen),
            Self::Light => Style::default().fg(TuiColor::Green),
            Self::Mono => Style::default().add_modifier(Modifier::BOLD),
        }
    }

    fn accent(self) -> Style {
        match self {
            Self::Auto => Style::default()
                .fg(TuiColor::Magenta)
                .add_modifier(Modifier::BOLD),
            Self::Dark => Style::default()
                .fg(TuiColor::LightMagenta)
                .add_modifier(Modifier::BOLD),
            Self::Light => Style::default()
                .fg(TuiColor::Blue)
                .add_modifier(Modifier::BOLD),
            Self::Mono => Style::default().add_modifier(Modifier::UNDERLINED | Modifier::BOLD),
        }
    }

    fn border(self) -> Style {
        match self {
            Self::Auto => Style::default().fg(TuiColor::Cyan),
            Self::Dark => Style::default().fg(TuiColor::DarkGray),
            Self::Light => Style::default().fg(TuiColor::Blue),
            Self::Mono => Style::default(),
        }
    }

    fn warning(self) -> Style {
        match self {
            Self::Auto | Self::Dark => Style::default()
                .fg(TuiColor::Yellow)
                .add_modifier(Modifier::BOLD),
            Self::Light => Style::default()
                .fg(TuiColor::LightRed)
                .add_modifier(Modifier::BOLD),
            Self::Mono => Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD),
        }
    }

    fn error(self) -> Style {
        match self {
            Self::Auto | Self::Light => Style::default()
                .fg(TuiColor::Red)
                .add_modifier(Modifier::BOLD),
            Self::Dark => Style::default()
                .fg(TuiColor::LightRed)
                .add_modifier(Modifier::BOLD),
            Self::Mono => Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        }
    }

    fn activity_style(self, activity: TuiActivity) -> Style {
        match activity {
            TuiActivity::Idle => self.muted(),
            TuiActivity::Working => self.title(),
            TuiActivity::Waiting => self.warning(),
            TuiActivity::Success => self.prompt(),
            TuiActivity::Failure => self.error(),
        }
    }
}

struct PendingTask {
    invocation: PendingInvocation,
    approval_id: String,
}

enum PendingInvocation {
    Agent(String),
    Slash(String),
}

impl PendingInvocation {
    fn summary(&self) -> &str {
        match self {
            Self::Agent(task) | Self::Slash(task) => task.as_str(),
        }
    }
}

impl App {
    fn new(args: ParsedArgs, theme: TuiTheme) -> Self {
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
            theme,
            activity: TuiActivity::Idle,
            active_tab: TuiTab::Console,
            sidebar_visible: true,
            scroll_offset: 0,
            palette_open: false,
            palette_query: Vec::new(),
            palette_index: 0,
            sparkline_history: vec![1, 2, 1, 3, 2, 4, 2, 1],
            kanban_snapshot: KanbanSnapshot {
                board_id: "default".to_owned(),
                todo: Vec::new(),
                in_progress: Vec::new(),
                blocked: Vec::new(),
                done: Vec::new(),
            },
            fleet_snapshot: FleetSnapshot {
                status: "idle".to_owned(),
                ready_nodes: 0,
                running_supervisors: 0,
                stale_supervisors: 0,
                jobs_queued: 0,
                jobs_running: 0,
                orchestrations_queued: 0,
                orchestrations_running: 0,
                active_leases: 0,
                run_count: 0,
                tokens_consumed: 0,
                tokens_ceiling: 0,
                cost_display: "0".to_owned(),
            },
            packages_cache: Vec::new(),
            distribution_cache: Vec::new(),
        }
    }

    fn run(&mut self, terminal: &mut TuiTerminal) -> Result<(), CliError> {
        loop {
            terminal
                .draw(|frame| self.render(frame))
                .map_err(terminal_error)?;
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

        if self.palette_open {
            return self.handle_palette_key(key);
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('p') => {
                    self.open_palette();
                    return false;
                }
                KeyCode::Char('b') => {
                    self.sidebar_visible = !self.sidebar_visible;
                    return false;
                }
                KeyCode::Char('y') => {
                    self.resolve_pending(true);
                    return false;
                }
                KeyCode::Char('n') => {
                    self.resolve_pending(false);
                    return false;
                }
                KeyCode::Char('l') => {
                    self.messages.clear();
                    self.scroll_offset = 0;
                    return false;
                }
                KeyCode::Char('a') => {
                    self.cursor = 0;
                    return false;
                }
                KeyCode::Char('e') => {
                    self.cursor = self.input.len();
                    return false;
                }
                KeyCode::Char('u') => {
                    if self.cursor > 0 {
                        self.input.drain(..self.cursor);
                        self.cursor = 0;
                    }
                    return false;
                }
                KeyCode::Char('k') => {
                    if self.cursor < self.input.len() {
                        self.input.truncate(self.cursor);
                    }
                    return false;
                }
                KeyCode::Char('w') => {
                    self.delete_previous_word();
                    return false;
                }
                KeyCode::Up => {
                    self.scroll_offset = self.scroll_offset.saturating_add(1);
                    return false;
                }
                KeyCode::Down => {
                    self.scroll_offset = self.scroll_offset.saturating_sub(1);
                    return false;
                }
                _ => {}
            }
        }

        if key.modifiers.contains(KeyModifiers::ALT) {
            match key.code {
                KeyCode::Char('1') => {
                    self.active_tab = TuiTab::Console;
                    return false;
                }
                KeyCode::Char('2') => {
                    self.active_tab = TuiTab::Kanban;
                    self.refresh_kanban_silent();
                    self.refresh_fleet_silent();
                    return false;
                }
                KeyCode::Char('3') => {
                    self.active_tab = TuiTab::Memory;
                    return false;
                }
                KeyCode::Char('4') => {
                    self.active_tab = TuiTab::Governance;
                    return false;
                }
                _ => {}
            }
        }

        match key.code {
            KeyCode::Esc => true,
            KeyCode::F(1) => {
                self.open_palette();
                false
            }
            KeyCode::Tab => {
                self.active_tab = self.active_tab.next();
                if self.active_tab == TuiTab::Kanban {
                    self.refresh_kanban_silent();
                    self.refresh_fleet_silent();
                }
                false
            }
            KeyCode::BackTab => {
                self.active_tab = self.active_tab.previous();
                if self.active_tab == TuiTab::Kanban {
                    self.refresh_kanban_silent();
                    self.refresh_fleet_silent();
                }
                false
            }
            KeyCode::PageUp => {
                self.scroll_offset = self.scroll_offset.saturating_add(8);
                false
            }
            KeyCode::PageDown => {
                self.scroll_offset = self.scroll_offset.saturating_sub(8);
                false
            }
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

    fn open_palette(&mut self) {
        self.palette_open = true;
        self.palette_query.clear();
        self.palette_index = 0;
    }

    fn filtered_palette_entries(&self) -> Vec<&'static PaletteEntry> {
        let query = self
            .palette_query
            .iter()
            .collect::<String>()
            .trim()
            .to_ascii_lowercase();
        if query.is_empty() {
            return PALETTE_ENTRIES.iter().collect();
        }
        PALETTE_ENTRIES
            .iter()
            .filter(|entry| {
                let cmd = entry.command.to_ascii_lowercase();
                let desc = entry.description.to_ascii_lowercase();
                let cat = entry.category.to_ascii_lowercase();
                cmd.contains(&query)
                    || desc.contains(&query)
                    || cat.contains(&query)
                    || fuzzy_subsequence(&query, &cmd)
            })
            .collect()
    }

    fn handle_palette_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => {
                self.palette_open = false;
                false
            }
            KeyCode::F(1) => {
                self.palette_open = false;
                false
            }
            KeyCode::Up => {
                self.palette_index = self.palette_index.saturating_sub(1);
                false
            }
            KeyCode::Down => {
                let len = self.filtered_palette_entries().len();
                if len > 0 {
                    self.palette_index = min(self.palette_index.saturating_add(1), len - 1);
                }
                false
            }
            KeyCode::Backspace => {
                self.palette_query.pop();
                self.palette_index = 0;
                false
            }
            KeyCode::Enter => {
                let entries = self.filtered_palette_entries();
                if let Some(selected) = entries.get(self.palette_index).copied() {
                    self.palette_open = false;
                    self.input = selected.command.chars().collect();
                    self.cursor = self.input.len();
                    if selected.immediate {
                        return self.submit();
                    }
                } else {
                    self.palette_open = false;
                }
                false
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.palette_open = false;
                false
            }
            KeyCode::Char(ch)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.palette_query.push(ch);
                self.palette_index = 0;
                false
            }
            _ => false,
        }
    }

    fn delete_previous_word(&mut self) {
        while self.cursor > 0 && self.input[self.cursor - 1].is_whitespace() {
            self.cursor -= 1;
            self.input.remove(self.cursor);
        }
        while self.cursor > 0 && !self.input[self.cursor - 1].is_whitespace() {
            self.cursor -= 1;
            self.input.remove(self.cursor);
        }
    }

    fn record_sparkline(&mut self, weight: u64) {
        self.sparkline_history.push(weight.max(1));
        if self.sparkline_history.len() > MAX_SPARKLINE_POINTS {
            let excess = self.sparkline_history.len() - MAX_SPARKLINE_POINTS;
            self.sparkline_history.drain(..excess);
        }
    }

    fn submit(&mut self) -> bool {
        let line = self.input.iter().collect::<String>();
        self.input.clear();
        self.cursor = 0;
        self.history_index = None;
        self.scroll_offset = 0;
        let task = line.trim().to_owned();
        if task.is_empty() {
            return false;
        }
        match task.as_str() {
            "/exit" | "/quit" => {
                self.push_message("Closing...");
                return true;
            }
            "/help" => {
                for message in [
                    "/help       show TUI commands",
                    "/setup      list or configure model provider presets (/setup <preset>)",
                    "/session    show the active session ID",
                    "/clear      clear the transcript",
                    "/theme      show or select auto, dark, light, or mono",
                    "/activity   show the typed public TUI activity state",
                    "/tab        switch workspace view (console, kanban, memory, governance)",
                    "/sidebar    toggle the telemetry & context sidebar",
                    "/palette    open the interactive fuzzy command palette (Ctrl-P / F1)",
                    "/kanban     inspect local Kanban board tasks across status columns",
                    "/packages   list locally admitted package identities",
                    "/distribution list verified downloads and their trust/admission state",
                    "/domain-starter show the safe Domain Harness scaffold workflow",
                    "/meta-starter show the safe Meta Harness scaffold workflow",
                    "/gene-pack  show the declarative Gene pack evaluation workflow",
                    "/canary-loop show the governed scheduled-canary workflow",
                    "/rollout ID inspect staged rollout budgets, scorecards, approvals, and evidence",
                    "/memory-retention show the scoped revoked-record compaction workflow",
                    "/memory-transfer show the cross-project consolidation policy",
                    "/storage-lifecycle show provider lifecycle evidence commands",
                    "/trust-transparency show trust and admission evidence commands",
                    "/fleet-health show the local privacy-safe operations snapshot",
                    "/approve    approve and resume the pending task",
                    "/deny       deny the pending task",
                    "/coding     inspect the Coding Domain Harness",
                    "/read, /search, /patch, /verify, /test, /format, /lint, /build, /status, /diff, /log, /refs, /review run core Coding Genes",
                    "/audit, /argus-review, /debt, /measure, /guide run workflow Genes",
                    "/exit       close the TUI",
                ] {
                    self.push_message(message);
                }
            }
            "/session" => {
                self.push_message(match self.session_id.as_deref() {
                    Some(session_id) => format!("session: {session_id}"),
                    None => "session: not started".to_owned(),
                });
            }
            "/clear" => {
                self.messages.clear();
                self.scroll_offset = 0;
            }
            "/theme" => {
                self.push_message(format!("theme> {}", self.theme.name()));
            }
            "/activity" => {
                self.push_message(format!(
                    "activity> {} (typed public state only)",
                    self.activity.name()
                ));
            }
            "/tab" => {
                self.push_message(format!("tab> {}", self.active_tab.name()));
            }
            value if value.starts_with("/tab ") => {
                let target = value.trim_start_matches("/tab ").trim();
                match TuiTab::parse(target) {
                    Some(tab) => {
                        self.active_tab = tab;
                        if tab == TuiTab::Kanban {
                            self.refresh_kanban_silent();
                            self.refresh_fleet_silent();
                        }
                        self.push_message(format!("tab> {} (view switched)", tab.name()));
                    }
                    None => {
                        self.push_message("usage> /tab <console|kanban|memory|governance>");
                    }
                }
            }
            "/sidebar" => {
                self.sidebar_visible = !self.sidebar_visible;
                self.push_message(format!(
                    "sidebar> {}",
                    if self.sidebar_visible {
                        "visible"
                    } else {
                        "hidden"
                    }
                ));
            }
            "/palette" => {
                self.open_palette();
            }
            "/setup" => {
                self.push_message("setup> Available provider presets (run /setup <preset>):");
                for preset in super::setup::PRESETS {
                    self.push_message(format!(
                        "  /setup {:<11} - {} [model: {}, env: {}]",
                        preset.name, preset.label, preset.default_model, preset.api_key_env
                    ));
                }
            }
            value if value.starts_with("/setup ") => {
                let target = value.trim_start_matches("/setup ").trim();
                self.run_setup_preset(target);
            }
            "/kanban" => self.show_kanban(),
            "/packages" => self.show_packages(),
            "/distribution" => self.show_distribution(),
            "/domain-starter" => {
                self.push_message(
                    "starter> pandora package scaffold domain-harness --output <new-directory>",
                );
                self.push_message(
                    "starter> generation is local-only; validate and dry-run enable before --yes",
                );
            }
            "/meta-starter" => {
                self.push_message(
                    "starter> pandora package scaffold meta-harness --output <new-directory>",
                );
                self.push_message(
                    "starter> composition-only; exact Domains must be enabled before --yes",
                );
            }
            "/gene-pack" => {
                self.push_message(
                    "gene-pack> validate and admit sdk/gene-pack examples; every Gene starts disabled",
                );
                self.push_message(
                    "gene-pack> declared effects still require Parliament, approval, and a one-shot permit",
                );
            }
            "/canary-loop" => {
                self.push_message(
                    "canary> register a reviewed suite, stage the approved proposal, then create an evaluation schedule with --proposal",
                );
                self.push_message(
                    "canary> the one-shot run records report evidence and stops before explicit evolution activate",
                );
            }
            "/rollout" => {
                self.push_message("usage> /rollout <proposal-id>");
            }
            value if value.starts_with("/rollout ") => {
                self.show_evolution_rollout(value.trim_start_matches("/rollout ").trim())
            }
            "/memory-retention" => {
                self.push_message(
                    "memory> preview scoped revoked records with memory compact --before <unix-seconds>",
                );
                self.push_message(
                    "memory> --yes removes only logical records; tombstones and audit remain, and database pages, WAL files, backups, and snapshots need separate lifecycle controls",
                );
            }
            "/memory-transfer" => {
                self.push_message(
                    "memory> cross-project consolidate requires exact source/target workspace IDs, the same tenant/provider, and --conflict reject|keep-target",
                );
                self.push_message(
                    "memory> dry-run first; --yes copies only non-sensitive L1, never overwrites, and never reuses tombstoned IDs",
                );
            }
            "/storage-lifecycle" => {
                self.push_message(
                    "storage> validate with backup lifecycle preview --input <manifest>; record digest-bound operator evidence only with record --input <manifest> --yes",
                );
                self.push_message(
                    "storage> provider actions happen outside the runtime; list and inspect are append-only evidence views, not secure-erasure guarantees",
                );
            }
            "/trust-transparency" => {
                self.push_message(
                    "trust> package transparency list shows chained trust-root changes and package admission allow/deny decisions; filter by --event-kind or --outcome",
                );
                self.push_message(
                    "trust> package transparency inspect --sequence <id> verifies one append-only event; evidence is read-only and grants no runtime authority",
                );
            }
            "/fleet-health" => self.show_fleet_health(),
            value if value.starts_with("/theme ") => {
                match TuiTheme::parse(Some(value.trim_start_matches("/theme ").trim())) {
                    Ok(theme) => {
                        self.theme = theme;
                        self.push_message(format!("theme> {} (presentation only)", theme.name()));
                    }
                    Err(error) => {
                        self.push_message(format!("usage> {}", error.message));
                    }
                }
            }
            "/approve" => self.resolve_pending(true),
            "/deny" => self.resolve_pending(false),
            value if value.starts_with("/approve ") || value.starts_with("/deny ") => {
                self.push_message("usage> /approve and /deny do not accept arguments");
            }
            value if value.starts_with('/') => self.run_slash(value.to_owned(), None),
            _ => self.run_task(task),
        }
        if !line.trim().is_empty()
            && !matches!(
                line.trim(),
                "/exit"
                    | "/quit"
                    | "/help"
                    | "/session"
                    | "/clear"
                    | "/approve"
                    | "/deny"
                    | "/palette"
            )
            && !line.trim().starts_with("/approve ")
            && !line.trim().starts_with("/deny ")
            && self.history.last().map(String::as_str) != Some(line.trim())
        {
            self.remember_history(line.trim());
        }
        false
    }

    fn run_setup_preset(&mut self, target: &str) {
        let mut args = Vec::new();
        for option in ["config", "data-dir", "workspace"] {
            if let Some(value) = self.args.value(option) {
                args.push(format!("--{option}"));
                args.push(value.to_owned());
            }
        }
        let tokens = target.split_whitespace().collect::<Vec<_>>();
        if tokens.is_empty() {
            self.push_message(
                "usage> /setup <ollama|openrouter|deepseek|openai|anthropic|groq> [--model <id>]",
            );
            return;
        }
        if !tokens[0].starts_with("--") {
            args.push("--preset".to_owned());
            args.push(tokens[0].to_owned());
            for token in &tokens[1..] {
                args.push((*token).to_owned());
            }
        } else {
            for token in &tokens {
                args.push((*token).to_owned());
            }
        }
        match super::setup::execute(&args) {
            Ok(result) => {
                self.activity = TuiActivity::Success;
                for line in result.human.lines() {
                    self.push_message(format!("setup> {}", line.trim()));
                }
            }
            Err(error) => {
                self.activity = TuiActivity::Failure;
                self.push_message(format!("error> {}", error.message));
            }
        }
    }

    fn show_kanban(&mut self) {
        self.activity = TuiActivity::Working;
        let mut args = vec!["board".to_owned()];
        for option in ["config", "data-dir", "workspace"] {
            if let Some(value) = self.args.value(option) {
                args.push(format!("--{option}"));
                args.push(value.to_owned());
            }
        }
        match super::kanban::execute(&args) {
            Ok(result) => {
                self.activity = TuiActivity::Success;
                self.apply_kanban_data(&result.data);
                let total = self.kanban_snapshot.todo.len()
                    + self.kanban_snapshot.in_progress.len()
                    + self.kanban_snapshot.blocked.len()
                    + self.kanban_snapshot.done.len();
                self.push_message(format!(
                    "kanban> board '{}' · {} task(s) (todo: {}, in_progress: {}, blocked: {}, done: {})",
                    self.kanban_snapshot.board_id,
                    total,
                    self.kanban_snapshot.todo.len(),
                    self.kanban_snapshot.in_progress.len(),
                    self.kanban_snapshot.blocked.len(),
                    self.kanban_snapshot.done.len(),
                ));
                self.record_sparkline(3);
            }
            Err(error) => {
                self.activity = TuiActivity::Failure;
                self.push_message(format!("error> {}", clean_text(&error.message)));
            }
        }
    }

    fn refresh_kanban_silent(&mut self) {
        let mut args = vec!["board".to_owned()];
        for option in ["config", "data-dir", "workspace"] {
            if let Some(value) = self.args.value(option) {
                args.push(format!("--{option}"));
                args.push(value.to_owned());
            }
        }
        if let Ok(result) = super::kanban::execute(&args) {
            self.apply_kanban_data(&result.data);
        }
    }

    fn apply_kanban_data(&mut self, data: &Value) {
        let board_id = data
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("default")
            .to_owned();
        let mut todo = Vec::new();
        let mut in_progress = Vec::new();
        let mut blocked = Vec::new();
        let mut done = Vec::new();

        if let Some(tasks) = data.get("tasks").and_then(Value::as_array) {
            for task in tasks.iter().take(64) {
                let id = task.get("id").and_then(Value::as_str).unwrap_or("task");
                let title = task
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("untitled");
                let priority = task.get("priority").and_then(Value::as_u64).unwrap_or(3);
                let status = task
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("todo")
                    .to_ascii_lowercase();
                let worker = task
                    .get("assigned_worker")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let label = if worker.is_empty() {
                    format!("[P{priority}] {id}: {title}")
                } else {
                    format!("[P{priority}] {id}: {title} (@{worker})")
                };
                match status.as_str() {
                    "in_progress" | "inprogress" | "running" => in_progress.push(label),
                    "blocked" | "review" | "failed" => blocked.push(label),
                    "done" | "completed" => done.push(label),
                    _ => todo.push(label),
                }
            }
        }

        self.kanban_snapshot = KanbanSnapshot {
            board_id,
            todo,
            in_progress,
            blocked,
            done,
        };
    }

    fn show_packages(&mut self) {
        self.activity = TuiActivity::Working;
        let mut args = vec!["list".to_owned()];
        for option in ["config", "data-dir", "workspace"] {
            if let Some(value) = self.args.value(option) {
                args.push(format!("--{option}"));
                args.push(value.to_owned());
            }
        }
        match super::package::execute(&args) {
            Ok(result) => {
                self.activity = TuiActivity::Success;
                let packages = result
                    .data
                    .get("packages")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                self.packages_cache.clear();
                if packages.is_empty() {
                    self.push_message("packages> none admitted locally");
                    return;
                }
                let total = packages.len();
                for package in packages.iter().take(50) {
                    let id = package
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    let version = package
                        .get("version")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    let kind = package
                        .get("kind")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    let state = package
                        .get("activation")
                        .and_then(|activation| activation.get("state"))
                        .and_then(Value::as_str)
                        .unwrap_or("disabled");
                    let formatted =
                        format!("package> {id}@{version} · {kind} · {state} · authority none");
                    self.packages_cache
                        .push(format!("{id}@{version} ({kind}) [{state}]"));
                    self.push_message(formatted);
                }
                if total > 50 {
                    self.push_message(format!(
                        "packages> showing 50 of {total}; use the JSON CLI for the full list"
                    ));
                }
            }
            Err(error) => {
                self.activity = TuiActivity::Failure;
                self.push_message(format!("error> {}", clean_text(&error.message)));
            }
        }
    }

    fn show_distribution(&mut self) {
        self.activity = TuiActivity::Working;
        let mut args = vec!["cache".to_owned(), "list".to_owned()];
        for option in ["config", "data-dir", "workspace"] {
            if let Some(value) = self.args.value(option) {
                args.push(format!("--{option}"));
                args.push(value.to_owned());
            }
        }
        match super::package::execute(&args) {
            Ok(result) => {
                self.activity = TuiActivity::Success;
                let packages = result
                    .data
                    .get("packages")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                self.distribution_cache.clear();
                if packages.is_empty() {
                    self.push_message("distribution> no verified downloads cached");
                    return;
                }
                let total = packages.len();
                for package in packages.iter().take(50) {
                    let id = package
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    let version = package
                        .get("version")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    let kind = package
                        .get("kind")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    let state = package
                        .get("state")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    let verification = package
                        .get("trust")
                        .and_then(|trust| trust.get("verification"))
                        .and_then(Value::as_str)
                        .unwrap_or("failed_closed");
                    let publisher = package
                        .get("publisher")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    let key_id = package
                        .get("publisher_key_id")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    let digest = package
                        .get("artifact_digest")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    self.distribution_cache.push(format!(
                        "{id}@{version} · {verification} · {publisher}/{key_id}"
                    ));
                    self.push_message(format!(
                        "distribution> {id}@{version} · {kind} · {state} · {verification} · publisher {publisher}/{key_id} · {digest} · authority none"
                    ));
                }
                if total > 50 {
                    self.push_message(format!(
                        "distribution> showing 50 of {total}; use package cache list --json for the full evidence"
                    ));
                }
                self.push_message(
                    "distribution> admission, enablement, and effect authority always require separate commands",
                );
            }
            Err(error) => {
                self.activity = TuiActivity::Failure;
                self.push_message(format!("error> {}", clean_text(&error.message)));
            }
        }
    }

    fn show_evolution_rollout(&mut self, proposal_id: &str) {
        if proposal_id.is_empty() || proposal_id.chars().any(char::is_whitespace) {
            self.push_message("usage> /rollout <proposal-id>");
            return;
        }
        self.activity = TuiActivity::Working;
        let mut args = vec![
            "inspect".to_owned(),
            "--id".to_owned(),
            proposal_id.to_owned(),
        ];
        for option in ["config", "data-dir"] {
            if let Some(value) = self.args.value(option) {
                args.push(format!("--{option}"));
                args.push(value.to_owned());
            }
        }
        match super::evolution::execute(&args) {
            Ok(result) => {
                self.activity = TuiActivity::Success;
                let Some(rollout) = result.data.get("rollout").filter(|value| !value.is_null())
                else {
                    self.push_message(format!(
                        "rollout> {proposal_id}: not configured; legacy canary gate remains"
                    ));
                    return;
                };
                let stage = rollout
                    .get("current_stage")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let status = rollout
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let channel = rollout
                    .pointer("/binding/channel")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let transitions = rollout
                    .get("transitions")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                let scorecards = rollout
                    .get("scorecards")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                self.push_message(format!(
                    "rollout> {proposal_id}: {channel} · {stage} · {status} · {scorecards} scorecard(s) · {transitions} evidence event(s)"
                ));
                if let Some(limits) =
                    rollout
                        .get("limits")
                        .and_then(Value::as_array)
                        .and_then(|limits| {
                            limits.iter().find(|limits| {
                                limits.get("stage").and_then(Value::as_str) == Some(stage)
                            })
                        })
                {
                    self.push_message(format!(
                        "rollout> limits: cost≤{}μ duration≤{}s failures≤{} quality≥{} p95≤{}ms stability≥{}",
                        limits.get("max_cost_micros").and_then(Value::as_u64).unwrap_or(0),
                        limits.get("max_duration_seconds").and_then(Value::as_u64).unwrap_or(0),
                        limits.get("max_failure_count").and_then(Value::as_u64).unwrap_or(0),
                        limits.get("min_quality_score").and_then(Value::as_u64).unwrap_or(0),
                        limits.get("max_p95_latency_millis").and_then(Value::as_u64).unwrap_or(0),
                        limits.get("min_stability_score").and_then(Value::as_u64).unwrap_or(0),
                    ));
                }
                let approval = rollout
                    .get("pending_approval")
                    .filter(|value| !value.is_null())
                    .and_then(|approval| approval.get("approver"))
                    .and_then(Value::as_str)
                    .unwrap_or("none");
                self.push_message(format!("rollout> pending human approval: {approval}"));
            }
            Err(error) => {
                self.activity = TuiActivity::Failure;
                self.push_message(format!("rollout> {}", error.message));
            }
        }
    }

    fn show_fleet_health(&mut self) {
        self.activity = TuiActivity::Working;
        let mut args = vec!["dashboard".to_owned()];
        for option in ["config", "data-dir"] {
            if let Some(value) = self.args.value(option) {
                args.push(format!("--{option}"));
                args.push(value.to_owned());
            }
        }
        match super::fleet::execute(&args) {
            Ok(result) => {
                self.activity = TuiActivity::Success;
                self.apply_fleet_data(&result.data);
                let health = &result.data["health"];
                let jobs = &result.data["queue"]["jobs"];
                let orchestrations = &result.data["queue"]["orchestrations"];
                let leases = &result.data["fleet"]["leases"];
                let budgets = &result.data["aggregate_budgets"];
                let ceiling = &budgets["ceiling"];
                let reserved = &budgets["reserved"];
                let consumed = &budgets["consumed"];
                let remaining = &budgets["remaining"];
                self.push_message(format!(
                    "fleet> {} · {} ready nodes · {} running supervisors · {} stale supervisors",
                    health["status"].as_str().unwrap_or("unknown"),
                    health["ready_nodes"].as_u64().unwrap_or_default(),
                    health["running_supervisors"].as_u64().unwrap_or_default(),
                    health["stale_supervisors"].as_u64().unwrap_or_default(),
                ));
                self.push_message(format!(
                    "queue> jobs {} queued / {} running · orchestrations {} queued / {} running · {} active leases",
                    jobs["queued"].as_u64().unwrap_or_default(),
                    jobs["running"].as_u64().unwrap_or_default(),
                    orchestrations["queued"].as_u64().unwrap_or_default(),
                    orchestrations["running"].as_u64().unwrap_or_default(),
                    leases["by_state"]["active"].as_u64().unwrap_or_default(),
                ));
                let cost = consumed["cost_micros"]
                    .as_u64()
                    .map_or_else(|| "unknown".to_owned(), |value| value.to_string());
                self.push_message(format!(
                    "budget> {} governed run(s) · tokens {} consumed / {} reserved / {} remaining / {} ceiling · tools {} / {} / {} / {} · actual cost {cost} micros ({} unknown receipt(s)); prompts, outputs, credentials, and hidden reasoning are excluded",
                    budgets["run_count"].as_u64().unwrap_or_default(),
                    consumed["tokens"].as_u64().unwrap_or_default(),
                    reserved["tokens"].as_u64().unwrap_or_default(),
                    remaining["tokens"].as_u64().unwrap_or_default(),
                    ceiling["tokens"].as_u64().unwrap_or_default(),
                    consumed["tools"].as_u64().unwrap_or_default(),
                    reserved["tools"].as_u64().unwrap_or_default(),
                    remaining["tools"].as_u64().unwrap_or_default(),
                    ceiling["tools"].as_u64().unwrap_or_default(),
                    consumed["unknown_cost_receipts"].as_u64().unwrap_or_default(),
                ));
                self.record_sparkline(4);
            }
            Err(error) => {
                self.activity = TuiActivity::Failure;
                self.push_message(format!("error> {}", clean_text(&error.message)));
            }
        }
    }

    fn refresh_fleet_silent(&mut self) {
        let mut args = vec!["dashboard".to_owned()];
        for option in ["config", "data-dir"] {
            if let Some(value) = self.args.value(option) {
                args.push(format!("--{option}"));
                args.push(value.to_owned());
            }
        }
        if let Ok(result) = super::fleet::execute(&args) {
            self.apply_fleet_data(&result.data);
        }
    }

    fn apply_fleet_data(&mut self, data: &Value) {
        let health = &data["health"];
        let jobs = &data["queue"]["jobs"];
        let orchestrations = &data["queue"]["orchestrations"];
        let leases = &data["fleet"]["leases"];
        let budgets = &data["aggregate_budgets"];
        let ceiling = &budgets["ceiling"];
        let consumed = &budgets["consumed"];
        let cost_display = consumed["cost_micros"]
            .as_u64()
            .map_or_else(|| "unknown".to_owned(), |value| value.to_string());

        self.fleet_snapshot = FleetSnapshot {
            status: health["status"].as_str().unwrap_or("idle").to_owned(),
            ready_nodes: health["ready_nodes"].as_u64().unwrap_or_default(),
            running_supervisors: health["running_supervisors"].as_u64().unwrap_or_default(),
            stale_supervisors: health["stale_supervisors"].as_u64().unwrap_or_default(),
            jobs_queued: jobs["queued"].as_u64().unwrap_or_default(),
            jobs_running: jobs["running"].as_u64().unwrap_or_default(),
            orchestrations_queued: orchestrations["queued"].as_u64().unwrap_or_default(),
            orchestrations_running: orchestrations["running"].as_u64().unwrap_or_default(),
            active_leases: leases["by_state"]["active"].as_u64().unwrap_or_default(),
            run_count: budgets["run_count"].as_u64().unwrap_or_default(),
            tokens_consumed: consumed["tokens"].as_u64().unwrap_or_default(),
            tokens_ceiling: ceiling["tokens"].as_u64().unwrap_or_default(),
            cost_display,
        };
    }

    fn run_task(&mut self, task: String) {
        self.run_task_with_approval(task, None);
    }

    fn run_task_with_approval(&mut self, task: String, approval_id: Option<&str>) {
        if approval_id.is_none() && self.pending.is_some() {
            self.push_message("approval> resolve the pending approval first");
            return;
        }
        self.push_message(format!("you> {task}"));
        let message_index = self.push_message("pandora> working...");
        self.activity = TuiActivity::Working;
        let args = self.run_args(&task, approval_id);
        match super::run::execute(&args) {
            Ok(result) => {
                self.activity = TuiActivity::Success;
                update_session(&mut self.session_id, result.data.get("session_id"));
                let output = result
                    .data
                    .get("output")
                    .and_then(Value::as_str)
                    .unwrap_or(&result.human);
                self.messages[message_index] = format!("pandora> {}", clean_text(output));
                let weight = (output.len() as u64 / 64).clamp(2, 12);
                self.record_sparkline(weight);
            }
            Err(error) => {
                self.activity = TuiActivity::Failure;
                update_session(&mut self.session_id, error.details.get("session_id"));
                self.messages[message_index] = format!("error> {}", clean_text(&error.message));
                if let Some(approval_id) = error.details.get("approval_id").and_then(Value::as_str)
                {
                    self.activity = TuiActivity::Waiting;
                    self.messages[message_index].push_str(&format!(" (approval: {approval_id})"));
                    self.pending = Some(PendingTask {
                        invocation: PendingInvocation::Agent(task),
                        approval_id: approval_id.to_owned(),
                    });
                }
                self.record_sparkline(5);
            }
        }
        self.turns = self.turns.saturating_add(1);
    }

    fn run_slash(&mut self, line: String, approval_id: Option<&str>) {
        if approval_id.is_none() && self.pending.is_some() {
            self.push_message("approval> resolve the pending approval first");
            return;
        }
        self.push_message(format!("you> {line}"));
        let message_index = self.push_message("pandora> working...");
        self.activity = TuiActivity::Working;
        match slash::execute_interactive(&line, &self.args, self.session_id.as_deref(), approval_id)
        {
            Ok(result) => {
                self.activity = TuiActivity::Success;
                update_session(&mut self.session_id, result.data.get("session_id"));
                let output = result
                    .data
                    .get("output")
                    .and_then(Value::as_str)
                    .unwrap_or(&result.human);
                self.messages[message_index] = format!("pandora> {}", clean_text(output));
                let weight = (output.len() as u64 / 64).clamp(2, 10);
                self.record_sparkline(weight);
            }
            Err(error) => {
                self.activity = TuiActivity::Failure;
                update_session(&mut self.session_id, error.details.get("session_id"));
                self.messages[message_index] = format!("error> {}", clean_text(&error.message));
                if let Some(approval_id) = error.details.get("approval_id").and_then(Value::as_str)
                {
                    self.activity = TuiActivity::Waiting;
                    self.messages[message_index].push_str(&format!(" (approval: {approval_id})"));
                    self.pending = Some(PendingTask {
                        invocation: PendingInvocation::Slash(line),
                        approval_id: approval_id.to_owned(),
                    });
                }
                self.record_sparkline(4);
            }
        }
        self.turns = self.turns.saturating_add(1);
    }

    fn resolve_pending(&mut self, allow: bool) {
        let Some(pending) = self.pending.take() else {
            self.push_message("approval> no pending approval");
            return;
        };
        let PendingTask {
            invocation,
            approval_id,
        } = pending;
        let approval_args = self.approval_args(&approval_id, allow);
        match super::approval::execute(&approval_args) {
            Ok(result) => {
                self.push_message(format!("approval> {}", result.human));
                if allow {
                    match invocation {
                        PendingInvocation::Agent(task) => {
                            self.run_task_with_approval(task, Some(&approval_id));
                        }
                        PendingInvocation::Slash(line) => {
                            self.run_slash(line, Some(&approval_id));
                        }
                    }
                } else {
                    self.activity = TuiActivity::Failure;
                }
            }
            Err(error) => {
                self.activity = TuiActivity::Waiting;
                self.push_message(format!("error> {}", error.message));
                self.pending = Some(PendingTask {
                    invocation,
                    approval_id,
                });
            }
        }
    }

    fn push_message(&mut self, message: impl Into<String>) -> usize {
        self.messages.push(clean_text(&message.into()));
        let excess = self.messages.len().saturating_sub(MAX_TUI_MESSAGES);
        if excess > 0 {
            self.messages.drain(..excess);
        }
        self.messages.len().saturating_sub(1)
    }

    fn remember_history(&mut self, line: &str) {
        if self.history.len() >= MAX_TUI_HISTORY {
            self.history.remove(0);
        }
        self.history.push(line.to_owned());
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

    fn render(&self, frame: &mut Frame) {
        let area = frame.area();
        let wide_mode =
            area.width >= WIDE_LAYOUT_MIN_WIDTH && area.height >= WIDE_LAYOUT_MIN_HEIGHT;

        if !wide_mode {
            self.render_compact(frame, area);
            if self.palette_open {
                self.render_palette_modal(frame, area);
            }
            return;
        }

        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(8),
                Constraint::Length(1),
                Constraint::Length(3),
            ])
            .split(area);

        self.render_top_bar(frame, sections[0]);

        match self.active_tab {
            TuiTab::Console => self.render_console_workspace(frame, sections[1]),
            TuiTab::Kanban => self.render_kanban_workspace(frame, sections[1]),
            TuiTab::Memory => self.render_memory_workspace(frame, sections[1]),
            TuiTab::Governance => self.render_governance_workspace(frame, sections[1]),
        }

        self.render_status_bar(frame, sections[2]);
        self.render_input_box(frame, sections[3]);

        if self.palette_open {
            self.render_palette_modal(frame, area);
        }
    }

    fn render_compact(&self, frame: &mut Frame, area: Rect) {
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Min(3),
                Constraint::Length(1),
                Constraint::Length(3),
            ])
            .split(area);

        let header = Paragraph::new(vec![
            Line::from(Span::styled("PANDORA TUI", self.theme.title())),
            Line::from(Span::styled(
                "Governed execution · Ctrl-C/Esc exits · /help for commands",
                self.theme.muted(),
            )),
        ]);
        frame.render_widget(header, sections[0]);

        self.render_transcript_pane(frame, sections[1]);
        self.render_status_bar(frame, sections[2]);
        self.render_input_box(frame, sections[3]);
    }

    fn render_top_bar(&self, frame: &mut Frame, area: Rect) {
        let horizontal = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(28), Constraint::Min(40)])
            .split(area);

        let brand = Paragraph::new(vec![
            Line::from(vec![
                Span::styled(" ◆ PANDORA TUI ", self.theme.title()),
                Span::styled(
                    format!("v{}", env!("CARGO_PKG_VERSION")),
                    self.theme.muted(),
                ),
            ]),
            Line::from(Span::styled(" Governed Agent Console", self.theme.muted())),
        ])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(self.theme.border()),
        );
        frame.render_widget(brand, horizontal[0]);

        let tab_titles = TuiTab::ALL
            .iter()
            .map(|tab| Line::from(tab.title()))
            .collect::<Vec<_>>();
        let tabs = Tabs::new(tab_titles)
            .select(self.active_tab.index())
            .highlight_style(self.theme.accent())
            .divider("│")
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(self.theme.border())
                    .title(" Workspace Views (Tab / Alt+1..4 · Ctrl+P Palette · Ctrl+B Sidebar) "),
            );
        frame.render_widget(tabs, horizontal[1]);
    }

    fn render_console_workspace(&self, frame: &mut Frame, area: Rect) {
        if !self.sidebar_visible {
            self.render_transcript_pane(frame, area);
            return;
        }
        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
            .split(area);

        self.render_transcript_pane(frame, panes[0]);
        self.render_telemetry_sidebar(frame, panes[1]);
    }

    fn render_transcript_pane(&self, frame: &mut Frame, area: Rect) {
        let inner_width = usize::from(area.width.saturating_sub(2));
        let mut lines = Vec::new();
        for message in &self.messages {
            wrap_message(message, inner_width, &mut lines);
        }
        let body_height = usize::from(area.height.saturating_sub(2));
        let max_scroll = lines.len().saturating_sub(body_height);
        let effective_scroll = min(self.scroll_offset, max_scroll);
        let first = max_scroll.saturating_sub(effective_scroll);

        let visible_lines = lines
            .iter()
            .skip(first)
            .take(body_height)
            .map(|line| style_transcript_line(line, inner_width, self.theme))
            .collect::<Vec<_>>();

        let title = if effective_scroll > 0 {
            format!("Transcript (scrolled +{effective_scroll} · PageDown to follow)")
        } else {
            "Transcript".to_owned()
        };

        let transcript = Paragraph::new(visible_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(self.theme.border())
                .title(title),
        );
        frame.render_widget(transcript, area);

        if lines.len() > body_height {
            let mut scrollbar_state = ScrollbarState::new(lines.len()).position(first);
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("▲"))
                .end_symbol(Some("▼"));
            frame.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
        }
    }

    fn render_telemetry_sidebar(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5),
                Constraint::Length(4),
                Constraint::Min(6),
            ])
            .split(area);

        let sparkline = Sparkline::default()
            .block(
                Block::default()
                    .title(" Activity Cadence ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(self.theme.border()),
            )
            .data(&self.sparkline_history)
            .style(self.theme.prompt());
        frame.render_widget(sparkline, chunks[0]);

        let max_turns = self
            .args
            .value("max-turns")
            .and_then(|val| val.parse::<u32>().ok())
            .unwrap_or(32)
            .max(1);
        let ratio_pct = min(100, ((self.turns.saturating_mul(100)) / max_turns) as u16);
        let gauge = Gauge::default()
            .block(
                Block::default()
                    .title(" Turn Budget ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(self.theme.border()),
            )
            .gauge_style(self.theme.title())
            .percent(ratio_pct)
            .label(format!(
                "{} / {} turns ({ratio_pct}%)",
                self.turns, max_turns
            ));
        frame.render_widget(gauge, chunks[1]);

        let session = self.session_id.as_deref().unwrap_or("not started");
        let provider = self.args.value("provider").unwrap_or("default");
        let model = self.args.value("model").unwrap_or("configured-default");
        let mut inspector_lines = vec![
            Line::from(vec![
                Span::styled(" Status:   ", self.theme.muted()),
                Span::styled(
                    self.activity.badge_label(),
                    self.theme.activity_style(self.activity),
                ),
            ]),
            Line::from(vec![
                Span::styled(" Session:  ", self.theme.muted()),
                Span::raw(truncate(
                    session,
                    usize::from(chunks[2].width.saturating_sub(13)),
                )),
            ]),
            Line::from(vec![
                Span::styled(" Provider: ", self.theme.muted()),
                Span::raw(truncate(
                    provider,
                    usize::from(chunks[2].width.saturating_sub(13)),
                )),
            ]),
            Line::from(vec![
                Span::styled(" Model:    ", self.theme.muted()),
                Span::raw(truncate(
                    model,
                    usize::from(chunks[2].width.saturating_sub(13)),
                )),
            ]),
            Line::from(vec![
                Span::styled(" Theme:    ", self.theme.muted()),
                Span::raw(self.theme.name()),
            ]),
        ];

        if let Some(pending) = &self.pending {
            inspector_lines.push(Line::from(""));
            inspector_lines.push(Line::from(Span::styled(
                " ⚠ PENDING APPROVAL ",
                self.theme.warning(),
            )));
            inspector_lines.push(Line::from(Span::styled(
                format!(" ID: {}", pending.approval_id),
                self.theme.warning(),
            )));
            inspector_lines.push(Line::from(Span::styled(
                format!(" Task: {}", pending.invocation.summary()),
                self.theme.muted(),
            )));
            inspector_lines.push(Line::from(Span::styled(
                " [Ctrl+Y] Approve · [Ctrl+N] Deny",
                self.theme.prompt(),
            )));
        } else {
            inspector_lines.push(Line::from(""));
            inspector_lines.push(Line::from(Span::styled(
                " Quick Shortcuts:",
                self.theme.title(),
            )));
            inspector_lines.push(Line::from(Span::styled(
                " • Ctrl+P / F1  Command Palette",
                self.theme.muted(),
            )));
            inspector_lines.push(Line::from(Span::styled(
                " • Tab / Alt+1-4 Switch Views",
                self.theme.muted(),
            )));
            inspector_lines.push(Line::from(Span::styled(
                " • PgUp / PgDn   Scroll History",
                self.theme.muted(),
            )));
        }

        let inspector = Paragraph::new(inspector_lines).block(
            Block::default()
                .title(" Runtime Telemetry ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(self.theme.border()),
        );
        frame.render_widget(inspector, chunks[2]);
    }

    fn render_kanban_workspace(&self, frame: &mut Frame, area: Rect) {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
            .split(area);

        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(25),
                Constraint::Percentage(25),
                Constraint::Percentage(25),
                Constraint::Percentage(25),
            ])
            .split(rows[0]);

        let board_cols = [
            (
                " TODO / BACKLOG ",
                &self.kanban_snapshot.todo,
                self.theme.muted(),
            ),
            (
                " IN PROGRESS ",
                &self.kanban_snapshot.in_progress,
                self.theme.title(),
            ),
            (
                " BLOCKED / REVIEW ",
                &self.kanban_snapshot.blocked,
                self.theme.warning(),
            ),
            (
                " COMPLETED ",
                &self.kanban_snapshot.done,
                self.theme.prompt(),
            ),
        ];

        for (idx, (title, items, style)) in board_cols.iter().enumerate() {
            let list_items = if items.is_empty() {
                vec![ListItem::new(" (empty) ").style(self.theme.muted())]
            } else {
                items
                    .iter()
                    .map(|item| {
                        ListItem::new(truncate(
                            item,
                            usize::from(columns[idx].width.saturating_sub(3)),
                        ))
                        .style(*style)
                    })
                    .collect::<Vec<_>>()
            };
            let list = List::new(list_items).block(
                Block::default()
                    .title(format!("{title}({}) ", items.len()))
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(self.theme.border()),
            );
            frame.render_widget(list, columns[idx]);
        }

        let f = &self.fleet_snapshot;
        let fleet_lines = vec![
            Line::from(vec![
                Span::styled(" Fleet Status: ", self.theme.title()),
                Span::raw(format!(
                    "{} · {} ready nodes · {} running supervisors · {} stale supervisors",
                    f.status, f.ready_nodes, f.running_supervisors, f.stale_supervisors
                )),
            ]),
            Line::from(vec![
                Span::styled(" Queue Depth:  ", self.theme.title()),
                Span::raw(format!(
                    "jobs {} queued / {} running · orchestrations {} queued / {} running · {} active leases",
                    f.jobs_queued,
                    f.jobs_running,
                    f.orchestrations_queued,
                    f.orchestrations_running,
                    f.active_leases
                )),
            ]),
            Line::from(vec![
                Span::styled(" Budgets:      ", self.theme.title()),
                Span::raw(format!(
                    "{} governed run(s) · tokens {} / {} ceiling · actual cost {} micros",
                    f.run_count, f.tokens_consumed, f.tokens_ceiling, f.cost_display
                )),
            ]),
            Line::from(Span::styled(
                " Tip: Run /kanban or /fleet-health in the prompt to refresh live store snapshots.",
                self.theme.muted(),
            )),
        ];
        let fleet_panel = Paragraph::new(fleet_lines).block(
            Block::default()
                .title(format!(
                    " Fleet Operations & Kanban Board '{}' ",
                    self.kanban_snapshot.board_id
                ))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(self.theme.border()),
        );
        frame.render_widget(fleet_panel, rows[1]);
    }

    fn render_memory_workspace(&self, frame: &mut Frame, area: Rect) {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(area);

        let tiers = vec![
            Line::from(Span::styled(
                " ◆ L0 Working Buffer (Ephemeral Turn Window)",
                self.theme.title(),
            )),
            Line::from(Span::styled(
                "   Active prompt assembly, bounded token budget & zero-copy cache hit validation.",
                self.theme.muted(),
            )),
            Line::from(""),
            Line::from(Span::styled(
                " ◆ L1 Session Scratchpad (Scoped Session Store)",
                self.theme.prompt(),
            )),
            Line::from(Span::styled(
                "   SQLite WAL-backed session journal; eligible for non-sensitive cross-project transfer.",
                self.theme.muted(),
            )),
            Line::from(""),
            Line::from(Span::styled(
                " ◆ L2 Episodic Memory (Temporal Trajectory Log)",
                self.theme.accent(),
            )),
            Line::from(Span::styled(
                "   Append-only execution receipts, reflexion episodes, and tool evidence digests.",
                self.theme.muted(),
            )),
            Line::from(""),
            Line::from(Span::styled(
                " ◆ L3 Semantic Memory (Consolidated Knowledge Graph)",
                self.theme.title(),
            )),
            Line::from(Span::styled(
                "   Distilled workspace invariants, architectural relations, and feature map nodes.",
                self.theme.muted(),
            )),
            Line::from(""),
            Line::from(Span::styled(
                " ◆ L4 Procedural Memory (Verified SOP DAGs & Skills)",
                self.theme.prompt(),
            )),
            Line::from(Span::styled(
                "   Executable workflow genes, Voyager curriculum skills, and governed playbooks.",
                self.theme.muted(),
            )),
        ];

        let left_panel = Paragraph::new(tiers).block(
            Block::default()
                .title(" 5-Tier Layered Memory Architecture ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(self.theme.border()),
        );
        frame.render_widget(left_panel, cols[0]);

        let right_lines = vec![
            Line::from(Span::styled(
                " Memory Lifecycle & Graph Controls",
                self.theme.title(),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled(" • /memory-retention ", self.theme.prompt()),
                Span::styled("Scoped revoked-record compaction", self.theme.muted()),
            ]),
            Line::from(vec![
                Span::styled(" • /memory-transfer  ", self.theme.prompt()),
                Span::styled("Cross-project L1 consolidation", self.theme.muted()),
            ]),
            Line::from(vec![
                Span::styled(" • /storage-lifecycle", self.theme.prompt()),
                Span::styled("Digest-bound lifecycle evidence", self.theme.muted()),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                " Coding & Architecture Workflow Genes",
                self.theme.title(),
            )),
            Line::from(vec![
                Span::styled(" • /audit        ", self.theme.accent()),
                Span::styled("Daedalus bounded workspace inventory", self.theme.muted()),
            ]),
            Line::from(vec![
                Span::styled(" • /argus-review ", self.theme.accent()),
                Span::styled("Argus multi-perspective code review", self.theme.muted()),
            ]),
            Line::from(vec![
                Span::styled(" • /debt         ", self.theme.accent()),
                Span::styled("Ariadne technical debt marker scan", self.theme.muted()),
            ]),
            Line::from(vec![
                Span::styled(" • /measure      ", self.theme.accent()),
                Span::styled("Hephaestus verification measurement", self.theme.muted()),
            ]),
            Line::from(vec![
                Span::styled(" • /guide        ", self.theme.accent()),
                Span::styled("Athena static architectural guide", self.theme.muted()),
            ]),
        ];

        let right_panel = Paragraph::new(right_lines).block(
            Block::default()
                .title(" Knowledge Graph & Workflow Genes ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(self.theme.border()),
        );
        frame.render_widget(right_panel, cols[1]);
    }

    fn render_governance_workspace(&self, frame: &mut Frame, area: Rect) {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);

        let mut gov_lines = vec![
            Line::from(Span::styled(
                " Tripartite Constitutional Governance",
                self.theme.title(),
            )),
            Line::from(Span::styled(
                " • Legislature: Declarative policy rules & capability bounds",
                self.theme.muted(),
            )),
            Line::from(Span::styled(
                " • Executive:   Sandboxed runtime controller & one-shot permits",
                self.theme.muted(),
            )),
            Line::from(Span::styled(
                " • Judiciary:   Shadow Council & Reference Monitor audit trail",
                self.theme.muted(),
            )),
            Line::from(""),
            Line::from(Span::styled(
                " Admitted Local Packages (/packages):",
                self.theme.prompt(),
            )),
        ];

        if self.packages_cache.is_empty() {
            gov_lines.push(Line::from(Span::styled(
                "   (none loaded — run /packages to query store)",
                self.theme.muted(),
            )));
        } else {
            for pkg in self.packages_cache.iter().take(6) {
                gov_lines.push(Line::from(format!("   • {pkg}")));
            }
        }

        let left_panel = Paragraph::new(gov_lines).block(
            Block::default()
                .title(" Constitutional Governance & Packages ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(self.theme.border()),
        );
        frame.render_widget(left_panel, cols[0]);

        let mut trust_lines = vec![Line::from(Span::styled(
            " Verified Distribution Cache (/distribution):",
            self.theme.title(),
        ))];
        if self.distribution_cache.is_empty() {
            trust_lines.push(Line::from(Span::styled(
                "   (no cached downloads loaded — run /distribution)",
                self.theme.muted(),
            )));
        } else {
            for item in self.distribution_cache.iter().take(6) {
                trust_lines.push(Line::from(format!("   • {item}")));
            }
        }
        trust_lines.push(Line::from(""));
        trust_lines.push(Line::from(Span::styled(
            " Human-in-the-Loop Approval Gate:",
            self.theme.warning(),
        )));
        if let Some(pending) = &self.pending {
            trust_lines.push(Line::from(Span::styled(
                format!("   Pending Approval ID: {}", pending.approval_id),
                self.theme.warning(),
            )));
            trust_lines.push(Line::from(Span::styled(
                format!("   Invocation: {}", pending.invocation.summary()),
                self.theme.muted(),
            )));
            trust_lines.push(Line::from(Span::styled(
                "   Press Ctrl+Y (/approve) or Ctrl+N (/deny)",
                self.theme.prompt(),
            )));
        } else {
            trust_lines.push(Line::from(Span::styled(
                "   No pending approval requests.",
                self.theme.muted(),
            )));
        }

        let right_panel = Paragraph::new(trust_lines).block(
            Block::default()
                .title(" Supply-Chain Trust & Approvals ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(self.theme.border()),
        );
        frame.render_widget(right_panel, cols[1]);
    }

    fn render_status_bar(&self, frame: &mut Frame, area: Rect) {
        let session = self.session_id.as_deref().unwrap_or("not started");
        let status = format!(
            "session: {session} · turns: {} · state: {} · view: {} · theme: {} · presentation only",
            self.turns,
            self.activity.name(),
            self.active_tab.name(),
            self.theme.name()
        );
        frame.render_widget(
            Paragraph::new(truncate(&status, usize::from(area.width))).style(self.theme.muted()),
            area,
        );
    }

    fn render_input_box(&self, frame: &mut Frame, area: Rect) {
        let (input, cursor) = visible_input(&self.input, self.cursor, usize::from(area.width));
        let input_line = Line::from(vec![
            Span::styled("> ", self.theme.prompt()),
            Span::raw(input),
        ]);
        frame.render_widget(
            Paragraph::new(input_line).block(Block::default().borders(Borders::TOP)),
            area,
        );
        if !self.palette_open {
            frame.set_cursor_position(Position::new(
                area.x.saturating_add(cursor),
                area.y.saturating_add(1),
            ));
        }
    }

    fn render_palette_modal(&self, frame: &mut Frame, area: Rect) {
        let modal_area = centered_rect(72, 65, area);
        frame.render_widget(Clear, modal_area);

        let outer = Block::default()
            .title(" ◆ Command Palette (Type to filter · Enter to run · Esc to close) ")
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(self.theme.accent());
        let inner = outer.inner(modal_area);
        frame.render_widget(outer, modal_area);

        if inner.height < 3 || inner.width < 10 {
            return;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(1)])
            .split(inner);

        let query_str = self.palette_query.iter().collect::<String>();
        let search_line = Line::from(vec![
            Span::styled(" Search: ", self.theme.title()),
            Span::raw(&query_str),
        ]);
        frame.render_widget(
            Paragraph::new(search_line).block(Block::default().borders(Borders::BOTTOM)),
            chunks[0],
        );

        let entries = self.filtered_palette_entries();
        let max_items = usize::from(chunks[1].height.max(1));
        let start = self
            .palette_index
            .saturating_sub(max_items.saturating_sub(1));
        let items = entries
            .iter()
            .enumerate()
            .skip(start)
            .take(max_items)
            .map(|(idx, entry)| {
                let selected = idx == self.palette_index;
                let prefix = if selected { " ▸ " } else { "   " };
                let line_text = format!(
                    "{prefix}{:<20} [{}] {}",
                    entry.command, entry.category, entry.description
                );
                let style = if selected {
                    self.theme.title()
                } else {
                    Style::default()
                };
                ListItem::new(truncate(&line_text, usize::from(chunks[1].width))).style(style)
            })
            .collect::<Vec<_>>();

        frame.render_widget(List::new(items), chunks[1]);
    }
}

fn style_transcript_line(line: &str, width: usize, theme: TuiTheme) -> Line<'static> {
    let text = truncate(line, width);
    if let Some(rest) = text.strip_prefix("you> ") {
        Line::from(vec![
            Span::styled("you> ", theme.prompt()),
            Span::raw(rest.to_owned()),
        ])
    } else if let Some(rest) = text.strip_prefix("pandora> ") {
        Line::from(vec![
            Span::styled("pandora> ", theme.title()),
            Span::raw(rest.to_owned()),
        ])
    } else if let Some(rest) = text.strip_prefix("error> ") {
        Line::from(vec![
            Span::styled("error> ", theme.error()),
            Span::raw(rest.to_owned()),
        ])
    } else if let Some(rest) = text.strip_prefix("approval> ") {
        Line::from(vec![
            Span::styled("approval> ", theme.warning()),
            Span::raw(rest.to_owned()),
        ])
    } else if text.starts_with('+') && !text.starts_with("+++") {
        Line::from(Span::styled(text, theme.prompt()))
    } else if text.starts_with('-') && !text.starts_with("---") {
        Line::from(Span::styled(text, theme.error()))
    } else if text.starts_with("@@") {
        Line::from(Span::styled(text, theme.title()))
    } else {
        Line::from(text)
    }
}

fn fuzzy_subsequence(needle: &str, haystack: &str) -> bool {
    let mut hay_chars = haystack.chars();
    for n in needle.chars() {
        if !hay_chars.any(|h| h == n) {
            return false;
        }
    }
    true
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
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
    use super::{
        App, MAX_TUI_HISTORY, MAX_TUI_MESSAGES, TuiActivity, TuiTab, TuiTheme, visible_input,
        wrap_message,
    };
    use crate::commands::ParsedArgs;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};
    use std::collections::BTreeMap;

    fn app() -> App {
        App::new(
            ParsedArgs {
                values: BTreeMap::new(),
                positionals: Vec::new(),
            },
            TuiTheme::Auto,
        )
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
    fn help_lists_every_coding_workflow_alias() {
        let mut app = app();
        app.input = "/help".chars().collect();
        app.submit();
        assert!(app.messages.iter().any(|message| {
            message == "/audit, /argus-review, /debt, /measure, /guide run workflow Genes"
        }));
    }

    #[test]
    fn help_exposes_package_and_domain_starter_discovery() {
        let mut app = app();
        app.input = "/help".chars().collect();
        app.submit();

        assert!(
            app.messages
                .iter()
                .any(|message| message == "/packages   list locally admitted package identities")
        );
        assert!(app.messages.iter().any(|message| {
            message == "/distribution list verified downloads and their trust/admission state"
        }));
        assert!(app.messages.iter().any(|message| {
            message == "/domain-starter show the safe Domain Harness scaffold workflow"
        }));
        assert!(app.messages.iter().any(|message| {
            message == "/meta-starter show the safe Meta Harness scaffold workflow"
        }));
        assert!(app.messages.iter().any(|message| {
            message == "/gene-pack  show the declarative Gene pack evaluation workflow"
        }));
        assert!(app.messages.iter().any(|message| {
            message == "/canary-loop show the governed scheduled-canary workflow"
        }));
        assert!(app.messages.iter().any(|message| {
            message
                == "/rollout ID inspect staged rollout budgets, scorecards, approvals, and evidence"
        }));
        assert!(app.messages.iter().any(|message| {
            message == "/memory-retention show the scoped revoked-record compaction workflow"
        }));
        assert!(app.messages.iter().any(|message| {
            message == "/memory-transfer show the cross-project consolidation policy"
        }));
        assert!(app.messages.iter().any(|message| {
            message == "/storage-lifecycle show provider lifecycle evidence commands"
        }));
        assert!(app.messages.iter().any(|message| {
            message == "/trust-transparency show trust and admission evidence commands"
        }));
        assert!(app.messages.iter().any(|message| {
            message == "/fleet-health show the local privacy-safe operations snapshot"
        }));
    }

    #[test]
    fn distribution_command_exposes_the_inert_verified_cache() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("test clock must follow the Unix epoch")
            .as_nanos();
        let data_dir = std::env::temp_dir().join(format!(
            "pandora-tui-distribution-{}-{nonce}",
            std::process::id()
        ));
        let mut values = BTreeMap::new();
        values.insert(
            "data-dir".to_owned(),
            data_dir.to_string_lossy().into_owned(),
        );
        let mut app = App::new(
            ParsedArgs {
                values,
                positionals: Vec::new(),
            },
            TuiTheme::Auto,
        );
        app.input = "/distribution".chars().collect();
        app.submit();

        assert!(
            app.messages
                .iter()
                .any(|message| message == "distribution> no verified downloads cached")
        );
        assert_eq!(app.activity, TuiActivity::Success);
        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn rollout_command_requires_an_exact_proposal_id() {
        let mut app = app();
        app.input = "/rollout".chars().collect();
        app.submit();

        assert!(
            app.messages
                .iter()
                .any(|message| message == "usage> /rollout <proposal-id>")
        );
        assert_eq!(app.activity, TuiActivity::Idle);
    }

    #[test]
    fn domain_starter_command_is_guidance_only() {
        let mut app = app();
        app.input = "/domain-starter".chars().collect();
        app.submit();

        assert!(app.messages.iter().any(|message| {
            message == "starter> pandora package scaffold domain-harness --output <new-directory>"
        }));
        assert_eq!(app.activity, TuiActivity::Idle);
        assert_eq!(app.run_args("inspect", None), ["--agent", "inspect"]);
    }

    #[test]
    fn meta_starter_command_is_guidance_only() {
        let mut app = app();
        app.input = "/meta-starter".chars().collect();
        app.submit();

        assert!(app.messages.iter().any(|message| {
            message == "starter> pandora package scaffold meta-harness --output <new-directory>"
        }));
        assert!(app.messages.iter().any(|message| {
            message == "starter> composition-only; exact Domains must be enabled before --yes"
        }));
        assert_eq!(app.activity, TuiActivity::Idle);
        assert_eq!(app.run_args("inspect", None), ["--agent", "inspect"]);
    }

    #[test]
    fn gene_pack_command_is_guidance_only() {
        let mut app = app();
        app.input = "/gene-pack".chars().collect();
        app.submit();

        assert!(app.messages.iter().any(|message| {
            message
                == "gene-pack> validate and admit sdk/gene-pack examples; every Gene starts disabled"
        }));
        assert!(app.messages.iter().any(|message| {
            message
                == "gene-pack> declared effects still require Parliament, approval, and a one-shot permit"
        }));
        assert_eq!(app.activity, TuiActivity::Idle);
    }

    #[test]
    fn canary_loop_command_is_guidance_only() {
        let mut app = app();
        app.input = "/canary-loop".chars().collect();
        app.submit();

        assert!(app.messages.iter().any(|message| {
            message
                == "canary> register a reviewed suite, stage the approved proposal, then create an evaluation schedule with --proposal"
        }));
        assert!(app.messages.iter().any(|message| {
            message
                == "canary> the one-shot run records report evidence and stops before explicit evolution activate"
        }));
        assert_eq!(app.activity, TuiActivity::Idle);
    }

    #[test]
    fn memory_retention_command_is_guidance_only() {
        let mut app = app();
        app.input = "/memory-retention".chars().collect();
        app.submit();

        assert!(app.messages.iter().any(|message| {
            message
                == "memory> preview scoped revoked records with memory compact --before <unix-seconds>"
        }));
        assert!(app.messages.iter().any(|message| {
            message.contains("tombstones and audit remain")
                && message.contains("backups, and snapshots")
        }));
        assert_eq!(app.activity, TuiActivity::Idle);
    }

    #[test]
    fn memory_transfer_command_is_guidance_only() {
        let mut app = app();
        app.input = "/memory-transfer".chars().collect();
        app.submit();

        assert!(app.messages.iter().any(|message| {
            message.contains("exact source/target workspace IDs")
                && message.contains("--conflict reject|keep-target")
        }));
        assert!(app.messages.iter().any(|message| {
            message.contains("never overwrites") && message.contains("tombstoned IDs")
        }));
        assert_eq!(app.activity, TuiActivity::Idle);
    }

    #[test]
    fn storage_lifecycle_command_is_guidance_only() {
        let mut app = app();
        app.input = "/storage-lifecycle".chars().collect();
        app.submit();

        assert!(app.messages.iter().any(|message| {
            message.contains("backup lifecycle preview")
                && message.contains("record --input <manifest> --yes")
        }));
        assert!(app.messages.iter().any(|message| {
            message.contains("outside the runtime")
                && message.contains("not secure-erasure guarantees")
        }));
        assert_eq!(app.activity, TuiActivity::Idle);
    }

    #[test]
    fn trust_transparency_command_is_guidance_only() {
        let mut app = app();
        app.input = "/trust-transparency".chars().collect();
        app.submit();

        assert!(app.messages.iter().any(|message| {
            message.contains("package transparency list")
                && message.contains("admission allow/deny decisions")
        }));
        assert!(app.messages.iter().any(|message| {
            message.contains("transparency inspect --sequence <id>")
                && message.contains("grants no runtime authority")
        }));
        assert_eq!(app.activity, TuiActivity::Idle);
    }

    #[test]
    fn fleet_health_command_renders_only_the_bounded_operations_summary() {
        let data_dir =
            std::env::temp_dir().join(format!("pandora-tui-fleet-health-{}", std::process::id()));
        let mut values = BTreeMap::new();
        values.insert(
            "data-dir".to_owned(),
            data_dir.to_string_lossy().into_owned(),
        );
        let mut app = App::new(
            ParsedArgs {
                values,
                positionals: Vec::new(),
            },
            TuiTheme::Auto,
        );
        app.input = "/fleet-health".chars().collect();
        app.submit();

        assert!(
            app.messages
                .iter()
                .any(|message| message.starts_with("fleet> idle"))
        );
        assert!(app.messages.iter().any(|message| {
            message.contains("0 governed run(s)")
                && message.contains("actual cost 0 micros")
                && message
                    .contains("prompts, outputs, credentials, and hidden reasoning are excluded")
        }));
        assert_eq!(app.activity, TuiActivity::Success);
        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn theme_command_changes_presentation_without_runtime_arguments() {
        let mut app = app();
        app.input = "/theme mono".chars().collect();
        app.submit();

        assert_eq!(app.theme, TuiTheme::Mono);
        assert_eq!(
            app.messages.last().map(String::as_str),
            Some("theme> mono (presentation only)")
        );
        assert_eq!(app.run_args("inspect", None), ["--agent", "inspect"]);
    }

    #[test]
    fn invalid_theme_fails_closed_to_the_current_palette() {
        let mut app = app();
        app.input = "/theme executable".chars().collect();
        app.submit();

        assert_eq!(app.theme, TuiTheme::Auto);
        assert!(
            app.messages
                .last()
                .is_some_and(|message| message.contains("expected auto, dark, light, or mono"))
        );
    }

    #[test]
    fn activity_command_exposes_only_the_typed_public_state() {
        let mut app = app();
        app.activity = TuiActivity::Waiting;
        app.input = "/activity".chars().collect();
        app.submit();

        assert_eq!(
            app.messages.last().map(String::as_str),
            Some("activity> waiting (typed public state only)")
        );
        assert_eq!(app.run_args("inspect", None), ["--agent", "inspect"]);
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

    #[test]
    fn transcript_keeps_only_the_newest_bounded_messages() {
        let mut app = app();
        for index in 0..(MAX_TUI_MESSAGES + 2) {
            app.push_message(format!("message-{index}"));
        }

        assert_eq!(app.messages.len(), MAX_TUI_MESSAGES);
        assert_eq!(app.messages.first().map(String::as_str), Some("message-2"));
        assert_eq!(app.messages.last().map(String::as_str), Some("message-513"));
    }

    #[test]
    fn history_keeps_only_the_newest_bounded_tasks() {
        let mut app = app();
        for index in 0..(MAX_TUI_HISTORY + 2) {
            app.remember_history(&format!("task-{index}"));
        }

        assert_eq!(app.history.len(), MAX_TUI_HISTORY);
        assert_eq!(app.history.first().map(String::as_str), Some("task-2"));
        assert_eq!(app.history.last().map(String::as_str), Some("task-101"));
    }

    #[test]
    fn ratatui_renderer_draws_the_initial_screen() {
        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let app = app();

        terminal
            .draw(|frame| app.render(frame))
            .expect("render initial TUI");

        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("PANDORA TUI"));
        assert!(rendered.contains("Enter a task"));
    }

    #[test]
    fn wide_dashboard_renders_tabs_and_telemetry_sidebar() {
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = app();
        app.active_tab = TuiTab::Console;

        terminal
            .draw(|frame| app.render(frame))
            .expect("render wide TUI");

        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("PANDORA TUI"));
        assert!(rendered.contains("1:Console"));
        assert!(rendered.contains("2:Kanban & Fleet"));
        assert!(rendered.contains("3:Memory & Graph"));
        assert!(rendered.contains("4:Governance & Trust"));
        assert!(rendered.contains("Activity Cadence"));
        assert!(rendered.contains("Turn Budget"));
        assert!(rendered.contains("Runtime Telemetry"));
    }

    #[test]
    fn tab_switching_renders_kanban_memory_and_governance_views() {
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = app();

        app.input = "/tab kanban".chars().collect();
        app.submit();
        assert_eq!(app.active_tab, TuiTab::Kanban);
        terminal.draw(|frame| app.render(frame)).unwrap();
        let kanban_view = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(kanban_view.contains("IN PROGRESS"));
        assert!(kanban_view.contains("Fleet Operations"));

        app.input = "/tab memory".chars().collect();
        app.submit();
        assert_eq!(app.active_tab, TuiTab::Memory);
        terminal.draw(|frame| app.render(frame)).unwrap();
        let memory_view = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(memory_view.contains("5-Tier Layered Memory Architecture"));
        assert!(memory_view.contains("L0 Working Buffer"));

        app.input = "/tab governance".chars().collect();
        app.submit();
        assert_eq!(app.active_tab, TuiTab::Governance);
        terminal.draw(|frame| app.render(frame)).unwrap();
        let gov_view = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(gov_view.contains("Tripartite Constitutional Governance"));
        assert!(gov_view.contains("Supply-Chain Trust & Approvals"));
    }

    #[test]
    fn command_palette_filters_and_executes_selected_command() {
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = app();

        // Open palette with Ctrl+P
        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(app.palette_open);

        // Type "mono" to filter to "/theme mono"
        for ch in "mono".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        let filtered = app.filtered_palette_entries();
        assert!(!filtered.is_empty());
        assert_eq!(filtered[0].command, "/theme mono");

        // Render modal overlay
        terminal.draw(|frame| app.render(frame)).unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("Command Palette"));

        // Press Enter to execute "/theme mono"
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!app.palette_open);
        assert_eq!(app.theme, TuiTheme::Mono);
    }

    #[test]
    fn readline_shortcuts_manipulate_input_buffer() {
        let mut app = app();
        app.input = "hello world".chars().collect();
        app.cursor = app.input.len();

        // Ctrl+W deletes "world"
        app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(app.input.iter().collect::<String>(), "hello ");

        // Ctrl+A moves cursor to 0
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
        assert_eq!(app.cursor, 0);

        // Ctrl+K truncates from cursor
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
        assert!(app.input.is_empty());
    }
}
