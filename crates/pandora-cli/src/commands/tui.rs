use super::{ParsedArgs, parse_options, slash};
use crate::output::{CliError, CommandResult, success};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseEvent, MouseEventKind,
};
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
use std::path::Path;
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
            "mode",
            "permission",
            "tool-profile",
            "tools",
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
        if let Err(error) = execute!(stdout, EnterAlternateScreen, EnableMouseCapture) {
            let _ = terminal::disable_raw_mode();
            return Err(terminal_error(error));
        }
        let terminal = match Terminal::new(CrosstermBackend::new(stdout)) {
            Ok(terminal) => terminal,
            Err(error) => {
                let _ = terminal::disable_raw_mode();
                let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
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
        let _ = execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        );
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
        command: "/model",
        category: "Model",
        description: "Open interactive Pandora model & provider selector modal (Ctrl+M)",
        immediate: true,
    },
    PaletteEntry {
        command: "/key",
        category: "Setup",
        description: "Configure provider API key in local .env (masked input modal)",
        immediate: true,
    },
    PaletteEntry {
        command: "/config",
        category: "Setup",
        description: "Inspect or update active runtime configuration (/config set model.default <m>)",
        immediate: true,
    },
    PaletteEntry {
        command: "/provider",
        category: "Setup",
        description: "List or add custom OpenAI/Anthropic provider profiles (/provider add <name> <url> [model])",
        immediate: true,
    },
    PaletteEntry {
        command: "/login",
        category: "Setup",
        description: "Authenticate via OAuth 2.0 PKCE or local endpoint (/login <claude|openai|copilot|openrouter|nous|ollama>)",
        immediate: true,
    },
    PaletteEntry {
        command: "/account",
        category: "Setup",
        description: "List or switch multi-account OAuth & API credentials (/account <label>)",
        immediate: true,
    },
    PaletteEntry {
        command: "/backdrop",
        category: "View",
        description: "Cycle or set TrueColor terminal mural backdrop (subtle, vivid, off) [Alt+B]",
        immediate: true,
    },
    PaletteEntry {
        command: "/tools",
        category: "Harness",
        description: "Inspect active Domain Harnesses, Coding Genes, Workflow Genes, and Skills",
        immediate: true,
    },
    PaletteEntry {
        command: "/effort",
        category: "Model",
        description: "Show or change reasoning effort level (low, medium, high, max) [Alt+Left/Right]",
        immediate: true,
    },
    PaletteEntry {
        command: "/resume",
        category: "Session",
        description: "Open interactive session picker or resume a session ID (Alt+R)",
        immediate: true,
    },
    PaletteEntry {
        command: "/fork",
        category: "Session",
        description: "Fork the active session into a fresh session branch",
        immediate: true,
    },
    PaletteEntry {
        command: "/rewind",
        category: "Session",
        description: "Rewind the transcript by one turn",
        immediate: true,
    },
    PaletteEntry {
        command: "/compact",
        category: "Session",
        description: "Compact older transcript turns into a concise handoff summary",
        immediate: true,
    },
    PaletteEntry {
        command: "/todos",
        category: "Kanban",
        description: "Toggle inline session Todo & Kanban card (Alt+X)",
        immediate: true,
    },
    PaletteEntry {
        command: "/usage",
        category: "Telemetry",
        description: "Show provider token usage, cost, turn latency, and cache state",
        immediate: true,
    },
    PaletteEntry {
        command: "/info",
        category: "Telemetry",
        description: "Toggle detailed runtime & session info card (Alt+I)",
        immediate: true,
    },
    PaletteEntry {
        command: "/hotkeys",
        category: "Core",
        description: "List all keyboard shortcuts and ergonomic bindings (/keys)",
        immediate: true,
    },
    PaletteEntry {
        command: "/skills",
        category: "Harness",
        description: "Inspect loaded procedural skills and Domain Harnesses",
        immediate: true,
    },
    PaletteEntry {
        command: "/hill-climb",
        category: "Harness",
        description: "Score a goal's hill-climbability (0..100) for continuous verifiable iteration",
        immediate: true,
    },
    PaletteEntry {
        command: "/confidence",
        category: "Harness",
        description: "Inspect stepped confidence gate & spike-check verification status",
        immediate: true,
    },
    PaletteEntry {
        command: "/autopoke",
        category: "Harness",
        description: "Inspect unfinished Todo continuation gate and retry budget (max 3 attempts)",
        immediate: true,
    },
    PaletteEntry {
        command: "/selfdev",
        category: "Harness",
        description: "Show governed Pandora Self-Development & atomic reload workflow",
        immediate: true,
    },
    PaletteEntry {
        command: "/stash",
        category: "Core",
        description: "Park or restore current composer prompt draft (Ctrl+S)",
        immediate: true,
    },
    PaletteEntry {
        command: "/focus",
        category: "View",
        description: "Toggle distraction-free Focus Mode",
        immediate: true,
    },
    PaletteEntry {
        command: "/dock",
        category: "View",
        description: "Toggle bottom Live Work Dock for active jobs & fleet subagents (Ctrl+T)",
        immediate: true,
    },
    PaletteEntry {
        command: "/setup",
        category: "Setup",
        description: "Show available model provider presets (ollama, openrouter, nous, deepseek, openai, anthropic, groq)",
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
        command: "/setup nous",
        category: "Setup",
        description: "Configure Nous Portal / Inference API preset (Hermes-3-Llama-3.1-70B)",
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
        command: "/setup deepseek",
        category: "Setup",
        description: "Configure DeepSeek API provider preset",
        immediate: true,
    },
    PaletteEntry {
        command: "/setup groq",
        category: "Setup",
        description: "Configure Groq Fast Inference LPU preset",
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
        command: "/mode autonomous",
        category: "Governance",
        description: "Switch to Autonomous execution mode (zero prompt barriers)",
        immediate: true,
    },
    PaletteEntry {
        command: "/mode supervised",
        category: "Governance",
        description: "Switch to Supervised execution mode (requires approval on mutating operations)",
        immediate: true,
    },
    PaletteEntry {
        command: "/mode read-only",
        category: "Governance",
        description: "Switch to Read-Only mode (strictly read operations only)",
        immediate: true,
    },
    PaletteEntry {
        command: "/preset",
        category: "Prompt",
        description: "List available reusable prompt presets and roles",
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReasoningEffort {
    Low,
    Medium,
    High,
    Max,
}

impl ReasoningEffort {
    const ALL: [Self; 4] = [Self::Low, Self::Medium, Self::High, Self::Max];

    fn name(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Max => "max",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Low => Self::Medium,
            Self::Medium => Self::High,
            Self::High => Self::Max,
            Self::Max => Self::Low,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Low => Self::Max,
            Self::Medium => Self::Low,
            Self::High => Self::Medium,
            Self::Max => Self::High,
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "low" | "minimal" | "fast" => Some(Self::Low),
            "medium" | "med" | "default" | "balanced" => Some(Self::Medium),
            "high" | "deep" => Some(Self::High),
            "max" | "xhigh" | "ultra" => Some(Self::Max),
            _ => None,
        }
    }
}

struct App {
    args: ParsedArgs,
    input: Vec<char>,
    cursor: usize,
    history: Vec<String>,
    history_index: Option<usize>,
    draft_stash: Vec<String>,
    autocomplete_index: usize,
    messages: Vec<String>,
    session_id: Option<String>,
    pending: Option<PendingTask>,
    turns: u32,
    last_turn_ms: Option<u128>,
    effort: ReasoningEffort,
    centered_layout: bool,
    scroll_bookmark: Option<usize>,
    todo_card_open: bool,
    info_card_open: bool,
    backdrop_mode: super::tui_backdrop::BackdropMode,
    theme: TuiTheme,
    activity: TuiActivity,
    active_tab: TuiTab,
    sidebar_visible: bool,
    focus_mode: bool,
    work_dock_open: bool,
    scroll_offset: usize,
    palette_open: bool,
    palette_query: Vec<char>,
    palette_index: usize,
    model_picker_open: bool,
    model_picker_query: Vec<char>,
    model_picker_index: usize,
    key_modal_open: bool,
    key_modal_env: String,
    key_modal_input: Vec<char>,
    active_provider_label: String,
    active_model_label: String,
    active_base_url: String,
    active_api_key_env: String,
    credential_ready: bool,
    git_branch: String,
    sparkline_history: Vec<u64>,
    kanban_snapshot: KanbanSnapshot,
    fleet_snapshot: FleetSnapshot,
    packages_cache: Vec<String>,
    distribution_cache: Vec<String>,
    governance_mode: pandora_types::GovernanceMode,
    mode_explicit: bool,
    active_persona: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ComposerMode {
    Chat,
    SlashCommand,
    DirectGene,
}

impl ComposerMode {
    fn from_input(input: &[char]) -> Self {
        let mut iter = input.iter().copied().skip_while(|ch| ch.is_whitespace());
        match iter.next() {
            Some('/') => Self::SlashCommand,
            Some('!') => Self::DirectGene,
            _ => Self::Chat,
        }
    }

    fn badge(self) -> &'static str {
        match self {
            Self::Chat => " [Chat · / commands · ! direct gene · @ file] ",
            Self::SlashCommand => " [Slash Command Mode · ↑/↓ select · Tab complete · Enter run] ",
            Self::DirectGene => " [! Direct Gene Mode · zero-token governed local execution] ",
        }
    }
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
            // 75% Purple brand hierarchy for primary typography and titles
            Self::Auto | Self::Dark => Style::default().fg(TuiColor::Rgb(168, 85, 247)),
            Self::Light => Style::default().fg(TuiColor::Rgb(124, 58, 237)),
            Self::Mono => Style::default(),
        };
        style.add_modifier(Modifier::BOLD)
    }

    fn muted(self) -> Style {
        match self {
            Self::Auto | Self::Dark => Style::default().fg(TuiColor::Rgb(148, 163, 184)),
            Self::Light => Style::default().fg(TuiColor::Rgb(100, 116, 139)),
            Self::Mono => Style::default().add_modifier(Modifier::DIM),
        }
    }

    fn prompt(self) -> Style {
        match self {
            Self::Auto | Self::Dark => Style::default().fg(TuiColor::Rgb(52, 211, 153)),
            Self::Light => Style::default().fg(TuiColor::Rgb(16, 185, 129)),
            Self::Mono => Style::default().add_modifier(Modifier::BOLD),
        }
    }

    fn accent(self) -> Style {
        match self {
            // 16% Pink / Rose accent for focal selections and active badges
            Self::Auto | Self::Dark => Style::default()
                .fg(TuiColor::Rgb(244, 63, 94))
                .add_modifier(Modifier::BOLD),
            Self::Light => Style::default()
                .fg(TuiColor::Rgb(225, 29, 72))
                .add_modifier(Modifier::BOLD),
            Self::Mono => Style::default().add_modifier(Modifier::UNDERLINED | Modifier::BOLD),
        }
    }

    fn border(self) -> Style {
        match self {
            // 75% Purple brand frame: deep violet borders for clean professional structure
            Self::Auto | Self::Dark => Style::default().fg(TuiColor::Rgb(124, 58, 237)),
            Self::Light => Style::default().fg(TuiColor::Rgb(147, 51, 234)),
            Self::Mono => Style::default(),
        }
    }

    fn warning(self) -> Style {
        match self {
            // 9% Gold accent for warnings, model tags, and security key alerts
            Self::Auto | Self::Dark => Style::default()
                .fg(TuiColor::Rgb(245, 158, 11))
                .add_modifier(Modifier::BOLD),
            Self::Light => Style::default()
                .fg(TuiColor::Rgb(217, 119, 6))
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
        let mode_explicit = args.value("mode").is_some() || args.value("permission").is_some();
        let governance_mode = args
            .value("mode")
            .or_else(|| args.value("permission"))
            .and_then(pandora_types::GovernanceMode::parse)
            .unwrap_or(pandora_types::GovernanceMode::Supervised);
        let mut app = Self {
            session_id: args.value("session").map(str::to_owned),
            args,
            input: Vec::new(),
            cursor: 0,
            history: Vec::new(),
            history_index: None,
            draft_stash: Vec::new(),
            autocomplete_index: 0,
            messages: vec![
                "Pandora TUI".to_owned(),
                "Enter a task. Press Ctrl-C or Esc to close; /help lists commands.".to_owned(),
            ],
            turns: 0,
            last_turn_ms: None,
            effort: ReasoningEffort::Medium,
            centered_layout: false,
            scroll_bookmark: None,
            todo_card_open: false,
            info_card_open: false,
            backdrop_mode: super::tui_backdrop::BackdropMode::Subtle,
            pending: None,
            theme,
            activity: TuiActivity::Idle,
            active_tab: TuiTab::Console,
            sidebar_visible: true,
            focus_mode: false,
            work_dock_open: false,
            scroll_offset: 0,
            palette_open: false,
            palette_query: Vec::new(),
            palette_index: 0,
            model_picker_open: false,
            model_picker_query: Vec::new(),
            model_picker_index: 0,
            key_modal_open: false,
            key_modal_env: "OPENROUTER_API_KEY".to_owned(),
            key_modal_input: Vec::new(),
            active_provider_label: "unconfigured".to_owned(),
            active_model_label: "unconfigured".to_owned(),
            active_base_url: "none".to_owned(),
            active_api_key_env: "OPENROUTER_API_KEY".to_owned(),
            credential_ready: false,
            git_branch: "main".to_owned(),
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
            governance_mode,
            mode_explicit,
            active_persona: None,
        };
        app.refresh_provider_telemetry();
        app
    }

    fn refresh_provider_telemetry(&mut self) {
        if let Ok(config) = super::load_config(&self.args) {
            let base_url = config.provider_url().unwrap_or("none").to_owned();
            let preset_label = super::setup::PRESETS
                .iter()
                .find(|p| p.base_url.eq_ignore_ascii_case(&base_url))
                .map(|p| p.name);
            let active_profile_name = config
                .active_provider()
                .filter(|name| *name != pandora_runtime::config::DEFAULT_PROVIDER_NAME);
            let provider_name = self
                .args
                .value("provider")
                .or(active_profile_name)
                .or(preset_label)
                .or_else(|| config.active_provider())
                .unwrap_or("unconfigured")
                .to_owned();
            let model_name = self
                .args
                .value("model")
                .or_else(|| config.provider_model())
                .unwrap_or("unconfigured")
                .to_owned();
            let api_key_env = config
                .provider_api_key_env()
                .unwrap_or("OPENROUTER_API_KEY")
                .to_owned();
            let ready = if base_url == "none" {
                false
            } else {
                super::provider::provider_credential_available(&config, &api_key_env)
                    .unwrap_or(false)
            };
            self.active_provider_label = provider_name;
            self.active_model_label = model_name;
            self.active_base_url = base_url;
            self.active_api_key_env = api_key_env;
            self.credential_ready = ready;
            self.git_branch = detect_git_branch(config.workspace_dir());
            let _ = super::tui_backdrop::ensure_backdrop_asset(config.data_dir());
        }
    }

    fn toggle_stash(&mut self) {
        let current = self.input.iter().collect::<String>();
        if !current.trim().is_empty() {
            self.draft_stash.push(current);
            self.input.clear();
            self.cursor = 0;
            self.autocomplete_index = 0;
            self.push_message(format!(
                "stash> parked draft #{} (press Ctrl+S on empty prompt or /stash to restore)",
                self.draft_stash.len()
            ));
        } else if let Some(restored) = self.draft_stash.pop() {
            self.input = restored.chars().collect();
            self.cursor = self.input.len();
            self.autocomplete_index = 0;
            self.push_message(format!(
                "stash> restored draft ({} remaining in stash)",
                self.draft_stash.len()
            ));
        } else {
            self.push_message("stash> no parked drafts (type a prompt and press Ctrl+S to stash)");
        }
    }

    fn cycle_model(&mut self, forward: bool) {
        let catalog = super::setup::MODEL_CATALOG;
        if catalog.is_empty() {
            return;
        }
        let current_idx = catalog
            .iter()
            .position(|entry| {
                entry
                    .provider
                    .eq_ignore_ascii_case(&self.active_provider_label)
                    && entry.model.eq_ignore_ascii_case(&self.active_model_label)
            })
            .unwrap_or(0);
        let next_idx = if forward {
            (current_idx + 1) % catalog.len()
        } else {
            (current_idx + catalog.len() - 1) % catalog.len()
        };
        let target = &catalog[next_idx];
        let spec = format!("{}:{}", target.provider, target.model);
        self.switch_model(&spec);
    }

    fn toggle_scroll_bookmark(&mut self) {
        if let Some(saved) = self.scroll_bookmark.take() {
            let current = self.scroll_offset;
            self.scroll_offset = saved;
            self.push_message(format!(
                "bookmark> jumped to scroll offset +{saved} (was +{current})"
            ));
        } else {
            self.scroll_bookmark = Some(self.scroll_offset);
            self.push_message(format!(
                "bookmark> saved scroll bookmark at +{} (press Ctrl+G again to jump back)",
                self.scroll_offset
            ));
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
            match event {
                Event::Key(key) => {
                    if self.handle_key(key) {
                        break;
                    }
                }
                Event::Mouse(mouse) => {
                    self.handle_mouse(mouse);
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.scroll_offset = self.scroll_offset.saturating_add(3);
            }
            MouseEventKind::ScrollDown => {
                self.scroll_offset = self.scroll_offset.saturating_sub(3);
            }
            _ => {}
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.kind != KeyEventKind::Press {
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return true;
        }

        if self.palette_open {
            return self.handle_palette_key(key);
        }
        if self.model_picker_open {
            return self.handle_model_picker_key(key);
        }
        if self.key_modal_open {
            return self.handle_key_modal_key(key);
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('p') => {
                    self.open_palette();
                    return false;
                }
                KeyCode::Char('m') => {
                    self.open_model_picker();
                    return false;
                }
                KeyCode::Char('s') => {
                    self.toggle_stash();
                    return false;
                }
                KeyCode::Char('g') => {
                    self.toggle_scroll_bookmark();
                    return false;
                }
                KeyCode::Tab => {
                    let forward = !key.modifiers.contains(KeyModifiers::SHIFT);
                    self.cycle_model(forward);
                    return false;
                }
                KeyCode::BackTab => {
                    self.cycle_model(false);
                    return false;
                }
                KeyCode::Char('t') => {
                    self.work_dock_open = !self.work_dock_open;
                    if self.work_dock_open {
                        self.refresh_kanban_silent();
                        self.refresh_fleet_silent();
                    }
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

        if key.modifiers.contains(KeyModifiers::SHIFT) {
            match key.code {
                KeyCode::Up => {
                    self.scroll_offset = self.scroll_offset.saturating_add(2);
                    return false;
                }
                KeyCode::Down => {
                    self.scroll_offset = self.scroll_offset.saturating_sub(2);
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
                KeyCode::Right => {
                    self.effort = self.effort.next();
                    self.push_message(format!(
                        "effort> {} (Alt+Left/Right or /effort <low|medium|high|max>)",
                        self.effort.name()
                    ));
                    return false;
                }
                KeyCode::Left => {
                    self.effort = self.effort.previous();
                    self.push_message(format!(
                        "effort> {} (Alt+Left/Right or /effort <low|medium|high|max>)",
                        self.effort.name()
                    ));
                    return false;
                }
                KeyCode::Char('m') => {
                    self.sidebar_visible = !self.sidebar_visible;
                    return false;
                }
                KeyCode::Char('c') => {
                    self.centered_layout = !self.centered_layout;
                    self.push_message(format!("layout> centered = {}", self.centered_layout));
                    return false;
                }
                KeyCode::Char('u') => {
                    self.scroll_offset = self.scroll_offset.saturating_add(8);
                    return false;
                }
                KeyCode::Char('d') => {
                    self.scroll_offset = self.scroll_offset.saturating_sub(8);
                    return false;
                }
                KeyCode::Char('h') => {
                    self.active_tab = self.active_tab.previous();
                    if self.active_tab == TuiTab::Kanban {
                        self.refresh_kanban_silent();
                        self.refresh_fleet_silent();
                    }
                    return false;
                }
                KeyCode::Char('l') => {
                    self.active_tab = self.active_tab.next();
                    if self.active_tab == TuiTab::Kanban {
                        self.refresh_kanban_silent();
                        self.refresh_fleet_silent();
                    }
                    return false;
                }
                KeyCode::Char('x') => {
                    self.todo_card_open = !self.todo_card_open;
                    if self.todo_card_open {
                        self.refresh_kanban_silent();
                    }
                    return false;
                }
                KeyCode::Char('i') => {
                    self.info_card_open = !self.info_card_open;
                    if self.info_card_open {
                        self.refresh_fleet_silent();
                    }
                    return false;
                }
                KeyCode::Char('b') => {
                    self.backdrop_mode = self.backdrop_mode.next();
                    self.push_message(format!(
                        "backdrop> {} (use Alt+B or /backdrop <subtle|vivid|off>)",
                        self.backdrop_mode.name()
                    ));
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
                if !self.input.is_empty() && self.apply_autocomplete() {
                    return false;
                }
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
                    self.autocomplete_index = 0;
                }
                false
            }
            KeyCode::Delete => {
                if self.cursor < self.input.len() {
                    self.input.remove(self.cursor);
                    self.autocomplete_index = 0;
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
                let suggestions = self.active_autocomplete_suggestions();
                if !suggestions.is_empty() {
                    self.autocomplete_index = self.autocomplete_index.saturating_sub(1);
                } else {
                    self.previous_history();
                }
                false
            }
            KeyCode::Down => {
                let suggestions = self.active_autocomplete_suggestions();
                if !suggestions.is_empty() {
                    self.autocomplete_index = min(
                        self.autocomplete_index.saturating_add(1),
                        suggestions.len() - 1,
                    );
                } else {
                    self.next_history();
                }
                false
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.input.insert(self.cursor, character);
                self.cursor += 1;
                self.autocomplete_index = 0;
                false
            }
            _ => false,
        }
    }

    fn open_palette(&mut self) {
        self.palette_open = true;
        self.model_picker_open = false;
        self.key_modal_open = false;
        self.palette_query.clear();
        self.palette_index = 0;
    }

    fn open_model_picker(&mut self) {
        self.refresh_provider_telemetry();
        self.model_picker_open = true;
        self.palette_open = false;
        self.key_modal_open = false;
        self.model_picker_query.clear();
        self.model_picker_index = 0;
    }

    fn open_key_modal(&mut self, env_override: Option<&str>) {
        self.refresh_provider_telemetry();
        self.key_modal_open = true;
        self.palette_open = false;
        self.model_picker_open = false;
        self.key_modal_env = env_override
            .filter(|val| !val.trim().is_empty())
            .map(|val| val.trim().to_owned())
            .unwrap_or_else(|| self.active_api_key_env.clone());
        self.key_modal_input.clear();
    }

    fn filtered_model_catalog(&self) -> Vec<&'static super::setup::ModelCatalogEntry> {
        let query = self
            .model_picker_query
            .iter()
            .collect::<String>()
            .trim()
            .to_ascii_lowercase();
        if query.is_empty() {
            return super::setup::MODEL_CATALOG.iter().collect();
        }
        super::setup::MODEL_CATALOG
            .iter()
            .filter(|entry| {
                let prov = entry.provider.to_ascii_lowercase();
                let model = entry.model.to_ascii_lowercase();
                let desc = entry.description.to_ascii_lowercase();
                let spec = format!("{prov}:{model}");
                spec.contains(&query)
                    || model.contains(&query)
                    || prov.contains(&query)
                    || desc.contains(&query)
                    || fuzzy_subsequence(&query, &spec)
            })
            .collect()
    }

    fn handle_model_picker_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => {
                self.model_picker_open = false;
                false
            }
            KeyCode::Char('m') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.model_picker_open = false;
                false
            }
            KeyCode::Up => {
                self.model_picker_index = self.model_picker_index.saturating_sub(1);
                false
            }
            KeyCode::Down => {
                let len = self.filtered_model_catalog().len();
                if len > 0 {
                    self.model_picker_index =
                        min(self.model_picker_index.saturating_add(1), len - 1);
                }
                false
            }
            KeyCode::Backspace => {
                self.model_picker_query.pop();
                self.model_picker_index = 0;
                false
            }
            KeyCode::Enter => {
                let entries = self.filtered_model_catalog();
                if let Some(selected) = entries.get(self.model_picker_index).copied() {
                    let spec = format!("{}:{}", selected.provider, selected.model);
                    self.model_picker_open = false;
                    self.switch_model(&spec);
                } else {
                    let custom = self
                        .model_picker_query
                        .iter()
                        .collect::<String>()
                        .trim()
                        .to_owned();
                    self.model_picker_open = false;
                    if !custom.is_empty() {
                        self.switch_model(&custom);
                    }
                }
                false
            }
            KeyCode::Char(ch)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.model_picker_query.push(ch);
                self.model_picker_index = 0;
                false
            }
            _ => false,
        }
    }

    fn handle_key_modal_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => {
                self.key_modal_open = false;
                self.key_modal_input.clear();
                false
            }
            KeyCode::Backspace => {
                self.key_modal_input.pop();
                false
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.key_modal_input.clear();
                false
            }
            KeyCode::Enter => {
                let env_name = self.key_modal_env.clone();
                let secret = self
                    .key_modal_input
                    .iter()
                    .collect::<String>()
                    .trim()
                    .to_owned();
                self.key_modal_open = false;
                self.key_modal_input.clear();
                if secret.is_empty() {
                    self.push_message("key> cancelled (empty key input)");
                } else {
                    self.save_api_key(&env_name, &secret);
                }
                false
            }
            KeyCode::Char(ch)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.key_modal_input.push(ch);
                false
            }
            _ => false,
        }
    }

    fn active_autocomplete_suggestions(&self) -> Vec<(String, String)> {
        if self.input.is_empty()
            || self.palette_open
            || self.model_picker_open
            || self.key_modal_open
        {
            return Vec::new();
        }
        let before_cursor = self.input[..min(self.cursor, self.input.len())]
            .iter()
            .collect::<String>();
        if (before_cursor.starts_with('/') || before_cursor.starts_with('!'))
            && !before_cursor.contains(char::is_whitespace)
        {
            let is_bang = before_cursor.starts_with('!');
            let query_slash = if is_bang {
                format!(
                    "/{}",
                    before_cursor.trim_start_matches('!').to_ascii_lowercase()
                )
            } else {
                before_cursor.to_ascii_lowercase()
            };
            let mut results = Vec::new();
            for entry in PALETTE_ENTRIES {
                if is_bang && !entry.category.contains("Gene") {
                    continue;
                }
                let cmd_token = entry
                    .command
                    .split_whitespace()
                    .next()
                    .unwrap_or(entry.command);
                if cmd_token.to_ascii_lowercase().starts_with(&query_slash) {
                    let display_token = if is_bang {
                        format!("!{}", cmd_token.trim_start_matches('/'))
                    } else {
                        cmd_token.to_owned()
                    };
                    if !results.iter().any(|(c, _)| c == &display_token) {
                        results.push((display_token, entry.description.to_owned()));
                    }
                }
            }
            if results.len() == 1 && results[0].0 == before_cursor {
                return Vec::new();
            }
            return results;
        }

        // Check for `@` file context autocomplete on the token immediately preceding cursor
        let token_start = before_cursor
            .rfind(char::is_whitespace)
            .map_or(0, |idx| idx + 1);
        let current_token = &before_cursor[token_start..];
        if let Some(file_prefix) = current_token.strip_prefix('@') {
            let prefix_lower = file_prefix.to_ascii_lowercase();
            let workspace_root = super::load_config(&self.args)
                .map(|cfg| cfg.workspace_dir().to_path_buf())
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            let mut matches = Vec::new();
            if let Ok(read_dir) = std::fs::read_dir(&workspace_root) {
                for entry in read_dir.flatten().take(64) {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name.starts_with('.') {
                        continue;
                    }
                    let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                    let display_name = if is_dir { format!("{name}/") } else { name };
                    if display_name.to_ascii_lowercase().starts_with(&prefix_lower) {
                        let kind_desc = if is_dir {
                            "workspace directory"
                        } else {
                            "workspace file"
                        };
                        matches.push((format!("@{display_name}"), kind_desc.to_owned()));
                        if matches.len() >= 6 {
                            break;
                        }
                    }
                }
            }
            return matches;
        }
        Vec::new()
    }

    fn apply_autocomplete(&mut self) -> bool {
        let suggestions = self.active_autocomplete_suggestions();
        let selected_idx = min(self.autocomplete_index, suggestions.len().saturating_sub(1));
        let Some((completion, _)) = suggestions.get(selected_idx) else {
            return false;
        };
        let before_cursor = self.input[..min(self.cursor, self.input.len())]
            .iter()
            .collect::<String>();
        let after_cursor = self.input[min(self.cursor, self.input.len())..]
            .iter()
            .collect::<String>();
        let token_start = before_cursor
            .rfind(char::is_whitespace)
            .map_or(0, |idx| idx + 1);
        let prefix = &before_cursor[..token_start];
        let suffix_space = if after_cursor.starts_with(' ') || completion.ends_with('/') {
            ""
        } else {
            " "
        };
        let updated_before = format!("{prefix}{completion}{suffix_space}");
        let full = format!("{updated_before}{after_cursor}");
        self.cursor = updated_before.chars().count();
        self.input = full.chars().collect();
        self.autocomplete_index = 0;
        true
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
        self.autocomplete_index = 0;
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
            "/" | "/commands" | "/help" | "/?" => {
                for message in [
                    "/help       show TUI commands",
                    "/model      open Pandora model & provider picker modal (Ctrl-M) or /model <provider:model>",
                    "/effort     show or change reasoning effort (low, medium, high, max) [Alt+Left/Right]",
                    "/key        open masked API key setup modal or /key [<ENV_VAR>] <api-key> (saved to .env)",
                    "/config     inspect or update active configuration (/config set model.default <model>)",
                    "/provider   list or add custom provider profiles (/provider add <name> <url> [model] [--no-api-key])",
                    "/login      OAuth 2.0 PKCE & local login flows (/login <provider> | /login callback <url-or-code>)",
                    "/account    list or switch saved OAuth / API accounts (/account [<provider>] <account>)",
                    "/backdrop   cycle or set terminal mural backdrop (subtle, vivid, off) [Alt+B]",
                    "/setup      list or configure model provider presets (/setup <preset>)",
                    "/tools      inspect active Domain Harnesses, Coding Genes, and Workflow Genes",
                    "/skills     inspect loaded procedural skills and Domain Harnesses",
                    "/hill-climb score a goal (0..100) on quantifiability & verification (/hill-climb <goal>)",
                    "/confidence inspect stepped confidence gate & spike-check verification status",
                    "/autopoke   inspect unfinished Todo continuation gate & retry budget",
                    "/selfdev    show governed Pandora self-development & atomic reload workflow",
                    "/resume     inspect or resume a session (/resume <session-id>)",
                    "/fork       fork the active session into a fresh session branch",
                    "/rewind     rewind the transcript by one turn",
                    "/compact    compact older transcript turns into a concise handoff summary",
                    "/todos      toggle inline session Todo & Kanban card (Alt+X)",
                    "/usage      show provider token usage, cost, turn latency, and cache state",
                    "/info       toggle detailed runtime & session info card (Alt+I)",
                    "/hotkeys    list all keyboard shortcuts and ergonomic bindings (/keys)",
                    "/stash      park or restore current composer prompt draft (Ctrl-S)",
                    "/focus      toggle distraction-free Focus Mode",
                    "/dock       toggle bottom Live Work Dock for active jobs & fleet subagents (Ctrl-T)",
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
                    "!<gene>     direct local Gene shortcut (e.g. !status, !diff, !test, !audit)",
                    "/exit       close the TUI",
                ] {
                    self.push_message(message);
                }
            }
            "/effort" => {
                let levels = ReasoningEffort::ALL
                    .iter()
                    .map(|lvl| {
                        if *lvl == self.effort {
                            format!("[*{}*]", lvl.name())
                        } else {
                            lvl.name().to_owned()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" · ");
                self.push_message(format!(
                    "effort> {} ({levels}) — use /effort <low|medium|high|max> or Alt+Left/Right",
                    self.effort.name()
                ));
            }
            value if value.starts_with("/effort ") => {
                let target = value.trim_start_matches("/effort ").trim();
                match ReasoningEffort::parse(target) {
                    Some(effort) => {
                        self.effort = effort;
                        self.push_message(format!(
                            "effort> switched reasoning effort to '{}'",
                            effort.name()
                        ));
                    }
                    None => {
                        self.push_message("usage> /effort <low|medium|high|max>");
                    }
                }
            }
            "/resume" => {
                let active = self.session_id.as_deref().unwrap_or("none");
                self.push_message(format!(
                    "session> active: {active} · use /resume <session-id> to attach or /fork to branch"
                ));
            }
            value if value.starts_with("/resume ") => {
                let target = value.trim_start_matches("/resume ").trim();
                if target.is_empty() || target.chars().any(char::is_whitespace) {
                    self.push_message("usage> /resume <session-id>");
                } else {
                    self.session_id = Some(target.to_owned());
                    self.push_message(format!("session> resumed session '{target}'"));
                }
            }
            "/fork" => {
                let base = self.session_id.as_deref().unwrap_or("session");
                let forked = format!("{base}-fork-{}", self.turns.saturating_add(1));
                self.session_id = Some(forked.clone());
                self.push_message(format!(
                    "session> forked active context into new session branch '{forked}'"
                ));
            }
            "/rewind" => {
                if let Some(pos) = self.messages.iter().rposition(|m| m.starts_with("you> ")) {
                    let removed = self.messages.len() - pos;
                    self.messages.truncate(pos);
                    self.turns = self.turns.saturating_sub(1);
                    self.push_message(format!(
                        "session> rewound last turn ({removed} transcript item(s) removed)"
                    ));
                } else {
                    self.push_message("session> nothing to rewind");
                }
            }
            "/compact" => {
                let count = self.messages.len();
                if count <= 6 {
                    self.push_message(format!(
                        "session> transcript is already compact ({count} messages)"
                    ));
                } else {
                    let tail = self.messages.split_off(count.saturating_sub(4));
                    let compacted_count = count - tail.len();
                    self.messages = vec![
                        "Pandora TUI".to_owned(),
                        format!(
                            "session> compacted {compacted_count} earlier messages into session summary (turns: {}, model: {}:{})",
                            self.turns, self.active_provider_label, self.active_model_label
                        ),
                    ];
                    self.messages.extend(tail);
                    self.scroll_offset = 0;
                }
            }
            "/todos" | "/todo" => {
                self.todo_card_open = !self.todo_card_open;
                self.refresh_kanban_silent();
                let total = self.kanban_snapshot.todo.len()
                    + self.kanban_snapshot.in_progress.len()
                    + self.kanban_snapshot.blocked.len()
                    + self.kanban_snapshot.done.len();
                self.push_message(format!(
                    "todos> card {} · {} item(s) on board '{}' ({} in_progress, {} todo, {} blocked, {} done) [Alt+X]",
                    if self.todo_card_open {
                        "pinned"
                    } else {
                        "hidden"
                    },
                    total,
                    self.kanban_snapshot.board_id,
                    self.kanban_snapshot.in_progress.len(),
                    self.kanban_snapshot.todo.len(),
                    self.kanban_snapshot.blocked.len(),
                    self.kanban_snapshot.done.len()
                ));
            }
            "/usage" => {
                self.refresh_fleet_silent();
                let run_count = self.fleet_snapshot.run_count;
                let tokens_consumed = self.fleet_snapshot.tokens_consumed;
                let tokens_ceiling = self.fleet_snapshot.tokens_ceiling;
                let cost_display = self.fleet_snapshot.cost_display.clone();
                let cache_state = if self.turns > 0 && self.session_id.is_some() {
                    "warm"
                } else {
                    "cold"
                };
                let latency = self
                    .last_turn_ms
                    .map_or_else(|| "n/a".to_owned(), |ms| format!("{ms}ms"));
                self.push_message(format!(
                    "usage> model {}:{} (effort: {}) · session turns: {} · last turn: {latency} · prompt cache: {cache_state}",
                    self.active_provider_label,
                    self.active_model_label,
                    self.effort.name(),
                    self.turns
                ));
                self.push_message(format!(
                    "usage> governed runs: {run_count} · tokens: {tokens_consumed} consumed / {tokens_ceiling} ceiling · actual cost: {cost_display} micros",
                ));
            }
            "/info" => {
                self.info_card_open = !self.info_card_open;
                self.refresh_provider_telemetry();
                self.refresh_fleet_silent();
                self.push_message(format!(
                    "info> session: {} · branch: {} · provider: {}:{} · effort: {} · card: {} [Alt+I]",
                    self.session_id.as_deref().unwrap_or("not started"),
                    self.git_branch,
                    self.active_provider_label,
                    self.active_model_label,
                    self.effort.name(),
                    if self.info_card_open {
                        "visible"
                    } else {
                        "hidden"
                    }
                ));
            }
            "/hotkeys" | "/keys" => {
                for line in [
                    "hotkeys> Navigation & Views:",
                    "  Tab / Shift+Tab / Alt+H,L  Cycle workspace views (Console, Kanban, Memory, Governance)",
                    "  Alt+1..4                   Jump directly to workspace view 1..4",
                    "  Ctrl+B / Alt+M             Toggle Runtime Telemetry sidebar",
                    "  Ctrl+T / /dock             Toggle Pandora Live Work Dock",
                    "  Alt+C                      Toggle centered layout mode",
                    "  Alt+X / /todos             Toggle pinned Todo & Kanban card",
                    "  Alt+I / /info              Toggle pinned Session & Usage Info card",
                    "hotkeys> Model, Effort & Prompt Composer:",
                    "  Ctrl+M / /model            Open interactive Model & Provider Selector",
                    "  Ctrl+Tab / Ctrl+Shift+Tab  Cycle next / previous model in catalog",
                    "  Alt+Right / Alt+Left       Increase / decrease reasoning effort (low..max)",
                    "  Ctrl+P / F1                Open fuzzy Command Palette",
                    "  Ctrl+S / /stash            Park or restore current composer prompt draft",
                    "  / · ! · @ + Up/Down + Tab  Navigate & complete slash commands, !genes, or @files",
                    "hotkeys> Transcript & Governance:",
                    "  PageUp/PageDown · Alt+U/D  Scroll transcript by page",
                    "  Ctrl+Up / Ctrl+Down        Scroll transcript line by line",
                    "  Ctrl+G                     Save or jump to transcript scroll bookmark",
                    "  Ctrl+Y / Ctrl+N            Approve (/approve) or Deny (/deny) pending task",
                ] {
                    self.push_message(line);
                }
            }
            "/skills" => {
                self.show_tools_summary();
            }
            "/hill-climb" | "/goal-score" => {
                self.evaluate_hill_climb("");
            }
            value if value.starts_with("/hill-climb ") || value.starts_with("/goal-score ") => {
                let goal = value
                    .trim_start_matches("/hill-climb ")
                    .trim_start_matches("/goal-score ")
                    .trim();
                self.evaluate_hill_climb(goal);
            }
            "/confidence" => {
                self.refresh_kanban_silent();
                let done_count = self.kanban_snapshot.done.len();
                let in_progress_count = self.kanban_snapshot.in_progress.len();
                let todo_count = self.kanban_snapshot.todo.len();
                self.push_message(format!(
                    "confidence> Stepped Confidence & Spike Check Gate: active ({done_count} done [100/100], {in_progress_count} in_progress [55→85/100], {todo_count} todo [35/100])"
                ));
                self.push_message(
                    "confidence> Policy: confidence is recorded at assignment (0..100) and completion (0..100); jumps >35 pts without intermediate verification (!test / !verify) trigger a Spike Check.",
                );
            }
            "/autopoke" => {
                self.refresh_kanban_silent();
                let unfinished =
                    self.kanban_snapshot.in_progress.len() + self.kanban_snapshot.todo.len();
                let blocked = self.kanban_snapshot.blocked.len();
                self.push_message(format!(
                    "autopoke> Unfinished Todo Continuation Gate: {unfinished} open item(s), {blocked} blocked · retry cap: 3 attempts for unchanged todo lists"
                ));
                self.push_message(
                    "autopoke> Policy: when a turn stops with incomplete todos, Pandora nudges continuation with an explicit unfinished-items summary and halts after 3 unchanged retries.",
                );
            }
            "/selfdev" => {
                self.push_message(
                    "selfdev> Pandora Governed Self-Development Workflow (RSI + Atomic Reload):",
                );
                self.push_message(
                    "  1. Inspect & scope: !status · !diff · /hill-climb <objective>",
                );
                self.push_message(
                    "  2. Verify changes:  !test · !lint · !build (or /canary-loop for evolutionary proposals)",
                );
                self.push_message(
                    "  3. Atomic reload:   cargo build --release -p pandora-cli --locked && replace installed binary while preserving session (/resume)",
                );
            }
            "/model" | "/models" => {
                self.open_model_picker();
            }
            value if value.starts_with("/model ") => {
                let spec = value.trim_start_matches("/model ").trim();
                self.switch_model(spec);
            }
            "/mode" | "/permission" => {
                self.push_message(format!(
                    "mode> Current governance mode: {} (options: autonomous, supervised, read-only)",
                    self.governance_mode.as_str()
                ));
            }
            value if value.starts_with("/mode ") || value.starts_with("/permission ") => {
                let target = value
                    .trim_start_matches("/permission ")
                    .trim_start_matches("/mode ")
                    .trim();
                match pandora_types::GovernanceMode::parse(target) {
                    Some(mode) => {
                        self.governance_mode = mode;
                        self.mode_explicit = true;
                        self.push_message(format!(
                            "mode> Switched governance mode to '{}' (takes effect on subsequent turns)",
                            mode.as_str()
                        ));
                    }
                    None => {
                        self.push_message("usage> /mode <autonomous|supervised|read-only>");
                    }
                }
            }
            "/preset" | "/presets" => {
                self.handle_preset_list();
            }
            value if value.starts_with("/preset ") || value.starts_with("/presets ") => {
                let rest = value
                    .trim_start_matches("/presets ")
                    .trim_start_matches("/preset ")
                    .trim();
                let parts: Vec<&str> = rest.split_whitespace().collect();
                if parts.is_empty() || parts[0] == "list" {
                    self.handle_preset_list();
                } else if parts[0] == "save" {
                    let save_rest = rest.trim_start_matches("save").trim();
                    self.handle_preset_save(save_rest);
                } else if parts[0] == "apply" || parts[0] == "use" {
                    let apply_rest = rest.trim_start_matches(parts[0]).trim();
                    self.handle_preset_apply(apply_rest);
                } else {
                    self.handle_preset_apply(rest);
                }
            }
            "/key" => {
                self.open_key_modal(None);
            }
            value if value.starts_with("/key ") => {
                let rest = value.trim_start_matches("/key ").trim();
                self.run_key_command(rest);
            }
            "/config" => {
                self.show_or_update_config("");
            }
            value if value.starts_with("/config ") => {
                let rest = value.trim_start_matches("/config ").trim();
                self.show_or_update_config(rest);
            }
            "/provider" | "/providers" => {
                self.handle_provider_command("");
            }
            value if value.starts_with("/provider ") || value.starts_with("/providers ") => {
                let rest = value
                    .trim_start_matches("/providers ")
                    .trim_start_matches("/provider ")
                    .trim();
                self.handle_provider_command(rest);
            }
            "/login" | "/oauth" => {
                self.handle_login_command("");
            }
            value if value.starts_with("/login ") || value.starts_with("/oauth ") => {
                let rest = value
                    .trim_start_matches("/login ")
                    .trim_start_matches("/oauth ")
                    .trim();
                self.handle_login_command(rest);
            }
            "/account" | "/accounts" => {
                self.handle_account_command("");
            }
            value if value.starts_with("/account ") || value.starts_with("/accounts ") => {
                let rest = value
                    .trim_start_matches("/accounts ")
                    .trim_start_matches("/account ")
                    .trim();
                self.handle_account_command(rest);
            }
            "/backdrop" => {
                self.backdrop_mode = self.backdrop_mode.next();
                self.push_message(format!(
                    "backdrop> switched terminal mural backdrop to '{}' (options: subtle, vivid, off) [Alt+B]",
                    self.backdrop_mode.name()
                ));
            }
            value if value.starts_with("/backdrop ") => {
                let target = value.trim_start_matches("/backdrop ").trim();
                match super::tui_backdrop::BackdropMode::parse(target) {
                    Some(mode) => {
                        self.backdrop_mode = mode;
                        self.push_message(format!(
                            "backdrop> set terminal mural backdrop to '{}'",
                            mode.name()
                        ));
                    }
                    None => {
                        self.push_message("usage> /backdrop <subtle|vivid|off>");
                    }
                }
            }
            "/tools" => {
                self.show_tools_summary();
            }
            "/stash" => {
                self.toggle_stash();
            }
            "/focus" => {
                self.focus_mode = !self.focus_mode;
                self.sidebar_visible = !self.focus_mode;
                self.push_message(format!(
                    "focus> {} (distraction-free mode)",
                    if self.focus_mode {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ));
            }
            "/dock" => {
                self.work_dock_open = !self.work_dock_open;
                if self.work_dock_open {
                    self.refresh_kanban_silent();
                    self.refresh_fleet_silent();
                }
                self.push_message(format!(
                    "dock> {}",
                    if self.work_dock_open {
                        "visible"
                    } else {
                        "hidden"
                    }
                ));
            }
            "/session" => {
                self.push_message(match self.session_id.as_deref() {
                    Some(session_id) => format!("session: {session_id}"),
                    None => "session: not started".to_owned(),
                });
            }
            "/new" | "/reset" | "/session new" => {
                self.session_id = None;
                self.pending = None;
                self.turns = 0;
                self.messages.clear();
                self.scroll_offset = 0;
                self.push_message(
                    "session> Started new clean session. Ready for tasks.".to_owned(),
                );
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
                self.push_message(
                    "setup> Available provider presets (run /setup <preset> or /model):",
                );
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
            value
                if self.pending.is_some()
                    && matches!(
                        value.trim().to_ascii_lowercase().as_str(),
                        "y" | "yes" | "approve" | "allow"
                    ) =>
            {
                self.resolve_pending(true);
            }
            value
                if self.pending.is_some()
                    && matches!(
                        value.trim().to_ascii_lowercase().as_str(),
                        "n" | "no" | "deny" | "reject"
                    ) =>
            {
                self.resolve_pending(false);
            }
            value if value.starts_with('/') => self.run_slash(value.to_owned(), None),
            value if value.starts_with('!') => {
                let gene = value.trim_start_matches('!').trim();
                if gene.is_empty() {
                    self.push_message(
                        "usage> !<gene> (e.g. !status, !diff, !test, !lint, !build, !audit, !debt)",
                    );
                } else {
                    self.run_slash(format!("/{gene}"), None);
                }
            }
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
                    | "/model"
                    | "/models"
                    | "/mode"
                    | "/permission"
                    | "/preset"
                    | "/presets"
                    | "/key"
                    | "/stash"
            )
            && !line.trim().starts_with("/approve ")
            && !line.trim().starts_with("/deny ")
            && !line.trim().starts_with("/key ")
            && self.history.last().map(String::as_str) != Some(line.trim())
        {
            self.remember_history(line.trim());
        }
        false
    }

    fn evaluate_hill_climb(&mut self, goal: &str) {
        let trimmed = goal.trim();
        if trimmed.is_empty() {
            self.push_message(
                "hill-climb> Hill-Climbable Goal Gate (threshold: 90/100 — quantifiable, iterable, deterministic verification):",
            );
            self.push_message(
                "  Usage: /hill-climb <task or objective> — scores 0..100 and reframes vague goals into measurable targets with a tight verification loop.",
            );
            return;
        }
        let lower = trimmed.to_ascii_lowercase();
        let has_metric = [
            "ms",
            "ns",
            "%",
            "mb",
            "kb",
            "gb",
            "fps",
            "tps",
            " throughput",
            "latency",
            "score",
            "benchmark",
            "zero",
            "0 ",
            "100%",
            "pass",
            "coverage",
        ]
        .iter()
        .any(|kw| lower.contains(kw))
            || trimmed.chars().any(|c| c.is_ascii_digit());
        let has_verifier = [
            "test", "bench", "cargo", "pytest", "verify", "lint", "clippy", "measure", "assert",
            "validate",
        ]
        .iter()
        .any(|kw| lower.contains(kw));
        let has_target = trimmed.split_whitespace().count() >= 5;

        let mut score: u32 = 45;
        if has_metric {
            score += 25;
        }
        if has_verifier {
            score += 20;
        }
        if has_target {
            score += 5;
        }
        score = score.min(98);

        if score >= 90 {
            self.push_message(format!(
                "hill-climb> score {score}/100 (PASS >= 90) · quantifiable metric + verification loop detected for: \"{trimmed}\""
            ));
            self.push_message(
                "hill-climb> Loop ready: baseline measurement -> hypothesis -> patch -> !test / !measure -> keep if improved.",
            );
        } else {
            self.push_message(format!(
                "hill-climb> score {score}/100 (BELOW 90 threshold) · goal needs sharper metric or deterministic verifier: \"{trimmed}\""
            ));
            self.push_message(format!(
                "hill-climb> Reframed target (95/100): \"{trimmed} — verified by deterministic test/benchmark with 0 regressions via !test and measurable delta\""
            ));
        }
    }

    fn handle_provider_command(&mut self, rest: &str) {
        let trimmed = rest.trim();
        let mut config = match super::load_config(&self.args) {
            Ok(cfg) => cfg,
            Err(error) => {
                self.activity = TuiActivity::Failure;
                self.push_message(format!("error> {}", error.message));
                return;
            }
        };

        if trimmed.is_empty() || trimmed == "list" {
            let names = config.provider_names();
            self.push_message(format!(
                "provider> {} configured profile(s) (active: {}):",
                names.len(),
                config.active_provider().unwrap_or("none")
            ));
            for name in &names {
                if let Some(p) = config.provider_profile(name) {
                    let marker = if config.active_provider() == Some(name.as_str()) {
                        "*"
                    } else {
                        " "
                    };
                    self.push_message(format!(
                        "  [{marker}] {:<16} · {} · model: {} · key_env: {} ({})",
                        p.name(),
                        p.base_url(),
                        p.model(),
                        p.api_key_env(),
                        p.protocol()
                    ));
                }
            }
            self.push_message(
                "provider> Custom endpoint: /provider add <name> <base-url> [model] [api_key_env|--no-api-key]",
            );
            self.push_message(
                "provider> Switch/Remove:   /provider use <name> · /provider remove <name>",
            );
            return;
        }

        if let Some(sub) = trimmed.strip_prefix("add ") {
            let parts = sub.split_whitespace().collect::<Vec<_>>();
            if parts.len() < 2 {
                self.push_message(
                    "usage> /provider add <name> <base-url> [model] [api_key_env|--no-api-key|--anthropic]",
                );
                return;
            }
            let name = parts[0];
            let base_url = parts[1];
            let mut model = "default";
            let mut no_api_key = false;
            let mut explicit_env: Option<&str> = None;
            let mut protocol = pandora_provider::ProviderProtocol::OpenAiCompatible;
            for token in &parts[2..] {
                if *token == "--no-api-key" {
                    no_api_key = true;
                } else if *token == "--anthropic" || *token == "anthropic_messages" {
                    protocol = pandora_provider::ProviderProtocol::AnthropicMessages;
                } else if *token == "openai_compatible" {
                    protocol = pandora_provider::ProviderProtocol::OpenAiCompatible;
                } else if model == "default" {
                    model = token;
                } else if explicit_env.is_none() {
                    explicit_env = Some(token);
                }
            }
            let default_env = format!(
                "{}_API_KEY",
                name.chars()
                    .map(|c| if c.is_ascii_alphanumeric() {
                        c.to_ascii_uppercase()
                    } else {
                        '_'
                    })
                    .collect::<String>()
            );
            let api_key_env = if no_api_key {
                super::setup::LOCAL_NO_API_KEY_ENV
            } else {
                explicit_env.unwrap_or(&default_env)
            };

            match pandora_runtime::config::ProviderProfile::new_with_protocol(
                name,
                protocol,
                base_url,
                model,
                api_key_env,
            ) {
                Ok(profile) => {
                    config.set_provider_profile(profile);
                    let _ = config.set_active_provider(name);
                    if let Err(error) = super::write_config(&config) {
                        self.activity = TuiActivity::Failure;
                        self.push_message(format!("error> {}", error.message));
                        return;
                    }
                    self.args.values.remove("provider");
                    self.args.values.remove("model");
                    self.refresh_provider_telemetry();
                    self.activity = TuiActivity::Success;
                    self.push_message(format!(
                        "provider> added & activated custom provider '{name}' ({base_url} · model: {model} · env: {api_key_env})"
                    ));
                }
                Err(error) => {
                    self.activity = TuiActivity::Failure;
                    self.push_message(format!("error> {error}"));
                }
            }
            return;
        }

        if let Some(target) = trimmed.strip_prefix("use ") {
            let name = target.trim();
            if let Err(error) = config.set_active_provider(name) {
                self.activity = TuiActivity::Failure;
                self.push_message(format!("error> {error}"));
                return;
            }
            if let Err(error) = super::write_config(&config) {
                self.activity = TuiActivity::Failure;
                self.push_message(format!("error> {}", error.message));
                return;
            }
            self.args.values.remove("provider");
            self.args.values.remove("model");
            self.refresh_provider_telemetry();
            self.activity = TuiActivity::Success;
            self.push_message(format!(
                "provider> active provider switched to '{name}' ({}:{})",
                self.active_provider_label, self.active_model_label
            ));
            return;
        }

        if let Some(target) = trimmed
            .strip_prefix("remove ")
            .or_else(|| trimmed.strip_prefix("rm "))
        {
            let name = target.trim();
            if !config.quarantine_provider_profile(name) {
                self.push_message(format!("error> provider '{name}' is not configured"));
                return;
            }
            let _ = super::write_config(&config);
            self.refresh_provider_telemetry();
            self.push_message(format!("provider> removed provider profile '{name}'"));
            return;
        }

        self.push_message(
            "usage> /provider [list | add <name> <base-url> [model] [--no-api-key] | use <name> | remove <name>]",
        );
    }

    fn handle_login_command(&mut self, rest: &str) {
        let trimmed = rest.trim();
        let mut config = match super::load_config(&self.args) {
            Ok(cfg) => cfg,
            Err(error) => {
                self.activity = TuiActivity::Failure;
                self.push_message(format!("error> {}", error.message));
                return;
            }
        };

        if trimmed.is_empty() || trimmed == "list" {
            self.push_message(
                "oauth> Built-in OAuth 2.0 PKCE & Local Login Flows (run /login <provider>):",
            );
            for spec in super::setup::OAUTH_PROVIDERS {
                self.push_message(format!("  /login {:<12} - {}", spec.name, spec.label));
            }
            self.push_message(
                "oauth> Complete callback: /login callback <redirect-url-or-code> [--account <label>]",
            );
            return;
        }

        if let Some(cb_rest) = trimmed.strip_prefix("callback ") {
            let parts = cb_rest.split_whitespace().collect::<Vec<_>>();
            if parts.is_empty() {
                self.push_message("usage> /login callback <redirect-url-or-code> [account-label]");
                return;
            }
            let callback_or_code = parts[0];
            let account_opt = parts.get(1).copied();
            match super::setup::complete_oauth_login(
                &mut config,
                None,
                account_opt,
                callback_or_code,
            ) {
                Ok(entry) => {
                    self.args.values.remove("provider");
                    self.args.values.remove("model");
                    self.refresh_provider_telemetry();
                    self.activity = TuiActivity::Success;
                    self.push_message(format!(
                        "oauth> authenticated '{}' (account '{}', env '{}': {})",
                        entry.provider, entry.account, entry.api_key_env, entry.masked_preview
                    ));
                }
                Err(error) => {
                    self.activity = TuiActivity::Failure;
                    self.push_message(format!("error> {}", error.message));
                }
            }
            return;
        }

        let parts = trimmed.split_whitespace().collect::<Vec<_>>();
        let provider_name = parts[0].trim_start_matches("--provider=");
        let mut account_label: Option<&str> = None;
        let mut idx = 1;
        while idx < parts.len() {
            if parts[idx] == "--account" && idx + 1 < parts.len() {
                account_label = Some(parts[idx + 1]);
                idx += 2;
            } else if !parts[idx].starts_with("--") && account_label.is_none() {
                account_label = Some(parts[idx]);
                idx += 1;
            } else {
                idx += 1;
            }
        }

        match super::setup::start_oauth_login(&mut config, provider_name, account_label) {
            Ok(flow) => {
                self.args.values.remove("provider");
                self.args.values.remove("model");
                self.refresh_provider_telemetry();
                self.activity = TuiActivity::Success;
                if flow.state == "local-ready" {
                    self.push_message(format!(
                        "oauth> local provider '{}' ready at {} (account '{}', no API key required)",
                        flow.provider, flow.auth_url, flow.account
                    ));
                } else {
                    self.push_message(format!(
                        "oauth> PKCE S256 flow started for '{}' (account '{}', state '{}'):",
                        flow.provider, flow.account, flow.state
                    ));
                    self.push_message(format!("  Authorize URL: {}", flow.auth_url));
                    self.push_message(
                        "  Finish login:  /login callback <redirect-url-or-auth-code>",
                    );
                }
            }
            Err(error) => {
                self.activity = TuiActivity::Failure;
                self.push_message(format!("error> {}", error.message));
            }
        }
    }

    fn handle_account_command(&mut self, rest: &str) {
        let trimmed = rest.trim();
        let mut config = match super::load_config(&self.args) {
            Ok(cfg) => cfg,
            Err(error) => {
                self.activity = TuiActivity::Failure;
                self.push_message(format!("error> {}", error.message));
                return;
            }
        };

        if trimmed.is_empty() || trimmed == "list" {
            let accounts = super::setup::load_oauth_accounts(&config);
            if accounts.is_empty() {
                self.push_message(
                    "account> no saved accounts yet — run /login <provider> or /key to add credentials",
                );
            } else {
                self.push_message(format!(
                    "account> {} saved provider account(s) (switch via /account [<provider>] <account>):",
                    accounts.len()
                ));
                for acc in &accounts {
                    let marker = if acc.active { "*" } else { " " };
                    self.push_message(format!(
                        "  [{marker}] {:<12} · account: {:<12} · kind: {:<7} · {}: {}",
                        acc.provider,
                        acc.account,
                        acc.auth_kind,
                        acc.api_key_env,
                        acc.masked_preview
                    ));
                }
            }
            return;
        }

        let cleaned = trimmed.strip_prefix("use ").unwrap_or(trimmed).trim();
        let parts = cleaned.split_whitespace().collect::<Vec<_>>();
        let (provider_filter, account_name) = if parts.len() >= 2 {
            (Some(parts[0]), parts[1])
        } else {
            (None, parts[0])
        };
        match super::setup::switch_oauth_account(&mut config, provider_filter, account_name) {
            Ok(chosen) => {
                self.args.values.remove("provider");
                self.args.values.remove("model");
                self.refresh_provider_telemetry();
                self.activity = TuiActivity::Success;
                self.push_message(format!(
                    "account> switched active '{}' account to '{}' ({}: {})",
                    chosen.provider, chosen.account, chosen.api_key_env, chosen.masked_preview
                ));
            }
            Err(error) => {
                self.activity = TuiActivity::Failure;
                self.push_message(format!("error> {}", error.message));
            }
        }
    }

    fn switch_model(&mut self, spec: &str) {
        let trimmed = spec.trim();
        if trimmed.is_empty() {
            self.open_model_picker();
            return;
        }
        if trimmed == "list" {
            self.push_message(
                "model> Curated Pandora Model Catalog (use /model <provider:model> or Ctrl+M):",
            );
            for entry in super::setup::MODEL_CATALOG {
                self.push_message(format!(
                    "  {:<10}:{:<36} - {}",
                    entry.provider, entry.model, entry.description
                ));
            }
            return;
        }
        let mut config = match super::load_config(&self.args) {
            Ok(cfg) => cfg,
            Err(error) => {
                self.activity = TuiActivity::Failure;
                self.push_message(format!("error> {}", error.message));
                return;
            }
        };
        match super::setup::apply_model_selection(&mut config, trimmed) {
            Ok((provider_label, model_name, api_key_env, protocol)) => {
                if let Err(error) = super::write_config(&config) {
                    self.activity = TuiActivity::Failure;
                    self.push_message(format!("error> {}", error.message));
                    return;
                }
                self.args.values.remove("model");
                self.args.values.remove("provider");
                self.refresh_provider_telemetry();
                self.activity = TuiActivity::Success;
                let key_status = if self.credential_ready {
                    format!("✓ credential '{api_key_env}' ready")
                } else {
                    format!("⚠ needs '{api_key_env}' — run /key to configure")
                };
                self.push_message(format!(
                    "model> switched to {provider_label}:{model_name} (protocol: {protocol}) · {key_status}"
                ));
                self.record_sparkline(2);
            }
            Err(error) => {
                self.activity = TuiActivity::Failure;
                self.push_message(format!("error> {}", error.message));
            }
        }
    }

    fn handle_preset_list(&mut self) {
        let Ok(config) = super::load_config(&self.args) else {
            self.push_message("preset> could not load workspace configuration");
            return;
        };
        let user_config_dir = config
            .config_path()
            .parent()
            .unwrap_or_else(|| Path::new("."));
        let engine = pandora_runtime::PromptEngine::open(config.workspace_dir(), user_config_dir);
        match engine.list() {
            Ok(presets) => {
                self.push_message(format!("presets available ({}):", presets.len()));
                for (p, tier) in presets {
                    let desc = p
                        .description
                        .as_deref()
                        .unwrap_or_else(|| &p.content[..p.content.len().min(40)]);
                    self.push_message(format!(
                        "  • {} [{}, {}]: {}",
                        p.name,
                        p.kind.as_str(),
                        tier.as_str(),
                        desc
                    ));
                }
                self.push_message("Use '/preset <name> [task]' to apply or '/preset save <name> <content...>' to save.");
            }
            Err(err) => {
                self.push_message(format!("preset> error listing presets: {err}"));
            }
        }
    }

    fn handle_preset_save(&mut self, rest: &str) {
        let (name, content) = match rest.split_once(char::is_whitespace) {
            Some((n, c)) => (n.trim(), c.trim()),
            None => {
                self.push_message("usage: /preset save <name> <content...>");
                return;
            }
        };
        if name.is_empty() || content.is_empty() {
            self.push_message("usage: /preset save <name> <content...>");
            return;
        }
        let Ok(config) = super::load_config(&self.args) else {
            self.push_message("preset> could not load workspace configuration");
            return;
        };
        let user_config_dir = config
            .config_path()
            .parent()
            .unwrap_or_else(|| Path::new("."));
        let engine = pandora_runtime::PromptEngine::open(config.workspace_dir(), user_config_dir);
        let preset = pandora_runtime::PromptPreset {
            name: name.to_owned(),
            description: None,
            kind: pandora_runtime::PresetKind::Template,
            content: content.to_owned(),
            parameters: pandora_runtime::PromptEngine::extract_parameters(content),
            model: None,
            mode: None,
            created_at: 0,
            updated_at: 0,
        };
        match engine.save(preset, true) {
            Ok(path) => {
                self.push_message(format!(
                    "preset> saved '{}' to workspace ({})",
                    name,
                    path.display()
                ));
            }
            Err(err) => {
                self.push_message(format!("preset> error saving preset: {err}"));
            }
        }
    }

    fn handle_preset_apply(&mut self, rest: &str) {
        let (name, task_arg) = match rest.split_once(char::is_whitespace) {
            Some((n, t)) => (n.trim(), t.trim()),
            None => (rest.trim(), ""),
        };
        let Ok(config) = super::load_config(&self.args) else {
            self.push_message("preset> could not load workspace configuration");
            return;
        };
        let user_config_dir = config
            .config_path()
            .parent()
            .unwrap_or_else(|| Path::new("."));
        let engine = pandora_runtime::PromptEngine::open(config.workspace_dir(), user_config_dir);
        match engine.get(name) {
            Ok((preset, _tier)) => {
                if preset.kind == pandora_runtime::PresetKind::Persona {
                    self.active_persona = Some(preset.name.clone());
                    if let Some(ref m) = preset.mode
                        && let Some(gov) = pandora_types::GovernanceMode::parse(m)
                    {
                        self.governance_mode = gov;
                        self.mode_explicit = true;
                    }
                    if let Some(ref mdl) = preset.model {
                        self.switch_model(mdl);
                    }
                    self.push_message(format!(
                        "preset> active persona set to '{}' (guidance active across turns)",
                        preset.name
                    ));
                } else {
                    let mut params = std::collections::BTreeMap::new();
                    params.insert(
                        "workspace".to_owned(),
                        config.workspace_dir().display().to_string(),
                    );
                    params.insert(
                        "lang".to_owned(),
                        pandora_runtime::PromptEngine::detect_workspace_lang(
                            config.workspace_dir(),
                        ),
                    );

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
                            params
                                .insert("target".to_owned(), final_task_words.remove(0).to_owned());
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

                    match pandora_runtime::PromptEngine::interpolate(&preset.content, &params) {
                        Ok(interpolated) => {
                            self.push_message(format!(
                                "preset> applied '{}': {}",
                                preset.name, interpolated
                            ));
                            self.run_task(interpolated);
                        }
                        Err(err) => {
                            self.push_message(format!("preset> interpolation error: {err}"));
                        }
                    }
                }
            }
            Err(err) => {
                self.push_message(format!("preset> error: {err}"));
            }
        }
    }

    fn run_key_command(&mut self, rest: &str) {
        let tokens = rest.split_whitespace().collect::<Vec<_>>();
        if tokens.is_empty() {
            self.open_key_modal(None);
            return;
        }
        if tokens.len() == 1 {
            // Check if token is an ENV_VAR name without a secret value -> open modal for that ENV_VAR
            let candidate = tokens[0];
            if candidate.ends_with("_KEY") || candidate.ends_with("_TOKEN") {
                self.open_key_modal(Some(candidate));
                return;
            }
            let env_name = self.active_api_key_env.clone();
            self.save_api_key(&env_name, candidate);
        } else {
            let env_name = tokens[0];
            let secret = tokens[1..].join(" ");
            self.save_api_key(env_name, &secret);
        }
    }

    fn save_api_key(&mut self, env_name: &str, secret: &str) {
        let config = match super::load_config(&self.args) {
            Ok(cfg) => cfg,
            Err(error) => {
                self.activity = TuiActivity::Failure;
                self.push_message(format!("error> {}", error.message));
                return;
            }
        };
        match super::setup::write_dotenv_var(&config, env_name, secret) {
            Ok(path) => {
                self.refresh_provider_telemetry();
                self.activity = TuiActivity::Success;
                let tail: String = secret
                    .chars()
                    .rev()
                    .take(4)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                let masked = format!("••••{tail}");
                self.push_message(format!(
                    "key> saved '{env_name}' ({masked}) to {}",
                    path.display()
                ));
                self.record_sparkline(2);
            }
            Err(error) => {
                self.activity = TuiActivity::Failure;
                self.push_message(format!("error> {}", error.message));
            }
        }
    }

    fn show_or_update_config(&mut self, rest: &str) {
        let tokens = rest.split_whitespace().collect::<Vec<_>>();
        if tokens.is_empty() || tokens[0].eq_ignore_ascii_case("show") {
            self.refresh_provider_telemetry();
            if let Ok(config) = super::load_config(&self.args) {
                let dotenv = super::setup::dotenv_path(&config);
                self.push_message(format!(
                    "config> file: {} · dotenv: {}",
                    config.config_path().display(),
                    dotenv.display()
                ));
                self.push_message(format!(
                    "config> model.provider: {} · model.default: {} · base_url: {}",
                    self.active_provider_label, self.active_model_label, self.active_base_url
                ));
                self.push_message(format!(
                    "config> api_key_env: {} ({}) · theme: {} · focus: {}",
                    self.active_api_key_env,
                    if self.credential_ready {
                        "ready"
                    } else {
                        "missing"
                    },
                    self.theme.name(),
                    self.focus_mode
                ));
                self.push_message(
                    "config> Tip: use '/config set model.provider <preset>' or '/config set model.default <provider:model>'",
                );
            }
            return;
        }
        if tokens[0].eq_ignore_ascii_case("set") {
            if tokens.len() < 3 {
                self.push_message(
                    "usage> /config set <model.provider|model.default|theme|focus|dock> <value>",
                );
                return;
            }
            let key = tokens[1].to_ascii_lowercase();
            let value = tokens[2..].join(" ");
            match key.as_str() {
                "model.provider" | "provider" => {
                    self.run_setup_preset(&value);
                }
                "model.default" | "model" => {
                    self.switch_model(&value);
                }
                "theme" => match TuiTheme::parse(Some(value.trim())) {
                    Ok(theme) => {
                        self.theme = theme;
                        self.push_message(format!("config> theme = {}", theme.name()));
                    }
                    Err(error) => {
                        self.push_message(format!("error> {}", error.message));
                    }
                },
                "focus" => {
                    self.focus_mode = matches!(
                        value.trim().to_ascii_lowercase().as_str(),
                        "true" | "1" | "on" | "yes"
                    );
                    self.sidebar_visible = !self.focus_mode;
                    self.push_message(format!("config> focus = {}", self.focus_mode));
                }
                "dock" => {
                    self.work_dock_open = matches!(
                        value.trim().to_ascii_lowercase().as_str(),
                        "true" | "1" | "on" | "yes"
                    );
                    self.push_message(format!("config> dock = {}", self.work_dock_open));
                }
                _ => {
                    self.push_message(
                        "usage> /config set <model.provider|model.default|theme|focus|dock> <value>",
                    );
                }
            }
            return;
        }
        self.push_message("usage> /config [show | set <key> <value>]");
    }

    fn show_tools_summary(&mut self) {
        self.push_message(
            "tools> Active Domain Harnesses: Coding, Research, Operations, Security, Data",
        );
        self.push_message(
            "tools> Core Coding Genes (13): /read /search /patch /verify /test /format /lint /build /status /diff /log /refs /review",
        );
        self.push_message(
            "tools> Workflow Genes (5): /audit (Daedalus) · /argus-review (Argus) · /debt (Ariadne) · /measure (Hephaestus) · /guide (Athena)",
        );
        self.push_message(
            "tools> Governance: Reference Monitor + Parliament + One-Shot Capability Permits",
        );
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
                "usage> /setup <ollama|openrouter|nous|deepseek|openai|anthropic|groq> [--model <id>]",
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
                self.args.values.remove("model");
                self.args.values.remove("provider");
                self.refresh_provider_telemetry();
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
        if approval_id.is_none()
            && let Some(pending) = self.pending.take()
        {
            let _ = super::approval::execute(&self.approval_args(&pending.approval_id, false));
            self.push_message(format!(
                "approval> pending approval {} discarded (superseded by new instruction)",
                pending.approval_id
            ));
        }
        self.push_message(format!("you> {task}"));
        let message_index = self.push_message("pandora> working...");
        self.activity = TuiActivity::Working;
        let started = std::time::Instant::now();
        let effective_task = if let Some(ref persona) = self.active_persona {
            format!("[Role / Persona: {persona}]\n{task}")
        } else {
            task.clone()
        };
        let args = self.run_args(&effective_task, approval_id);
        match super::run::execute(&args) {
            Ok(result) => {
                self.last_turn_ms = Some(started.elapsed().as_millis());
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
                self.last_turn_ms = Some(started.elapsed().as_millis());
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
                    self.push_message("approval> Action required: Type /approve (or press Ctrl+Y / type 'y') to allow, or /deny (Ctrl+N / 'n') to reject");
                } else if error.message.contains("has a pending approval")
                    || error.message.contains("already resolved or consumed")
                {
                    self.push_message("hint> Type /new to start a fresh clean session");
                }
                self.record_sparkline(5);
            }
        }
        self.turns = self.turns.saturating_add(1);
    }

    fn run_slash(&mut self, line: String, approval_id: Option<&str>) {
        if approval_id.is_none()
            && let Some(pending) = self.pending.take()
        {
            let _ = super::approval::execute(&self.approval_args(&pending.approval_id, false));
            self.push_message(format!(
                "approval> pending approval {} discarded (superseded by new instruction)",
                pending.approval_id
            ));
        }
        self.push_message(format!("you> {line}"));
        let message_index = self.push_message("pandora> working...");
        self.activity = TuiActivity::Working;
        let started = std::time::Instant::now();
        let mode = if self.mode_explicit {
            Some(self.governance_mode.as_str())
        } else {
            self.args
                .value("mode")
                .or_else(|| self.args.value("permission"))
        };
        let provider = if self.active_provider_label != "unconfigured"
            && !self.active_provider_label.is_empty()
        {
            Some(self.active_provider_label.as_str())
        } else {
            None
        };
        let model = if self.active_model_label != "default" && !self.active_model_label.is_empty() {
            Some(self.active_model_label.as_str())
        } else {
            None
        };
        match slash::execute_interactive(
            &line,
            &self.args,
            self.session_id.as_deref(),
            approval_id,
            mode,
            provider,
            model,
        ) {
            Ok(result) => {
                self.last_turn_ms = Some(started.elapsed().as_millis());
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
                self.last_turn_ms = Some(started.elapsed().as_millis());
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
        if self.args.has_flag("trusted-process") {
            args.push("--trusted-process".to_owned());
        }
        for name in [
            "config",
            "data-dir",
            "workspace",
            "provider",
            "model",
            "max-turns",
            "max-tools",
            "tool-profile",
            "tools",
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
        if self.mode_explicit {
            args.push("--mode".to_owned());
            args.push(self.governance_mode.as_str().to_owned());
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
            } else if self.model_picker_open {
                self.render_model_picker_modal(frame, area);
            } else if self.key_modal_open {
                self.render_key_modal(frame, area);
            }
            if self.theme != TuiTheme::Mono {
                super::tui_backdrop::apply_backdrop(
                    frame.buffer_mut(),
                    area,
                    self.backdrop_mode,
                    self.theme == TuiTheme::Light,
                );
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
        self.render_autocomplete_popup(frame, sections[1], sections[3]);

        if self.palette_open {
            self.render_palette_modal(frame, area);
        } else if self.model_picker_open {
            self.render_model_picker_modal(frame, area);
        } else if self.key_modal_open {
            self.render_key_modal(frame, area);
        }

        if self.theme != TuiTheme::Mono {
            super::tui_backdrop::apply_backdrop(
                frame.buffer_mut(),
                area,
                self.backdrop_mode,
                self.theme == TuiTheme::Light,
            );
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
            .constraints([Constraint::Length(34), Constraint::Min(40)])
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
                    .title(
                        " Views (Tab/Alt+1..4 · Ctrl+M Model · Ctrl+P Palette · Ctrl+T Dock · Ctrl+B Sidebar) ",
                    ),
            );
        frame.render_widget(tabs, horizontal[1]);
    }

    fn render_console_workspace(&self, frame: &mut Frame, area: Rect) {
        let workspace_area = if self.centered_layout && area.width >= 100 {
            let pad = (area.width.saturating_sub(96)) / 2;
            Rect::new(
                area.x.saturating_add(pad),
                area.y,
                area.width.saturating_sub(pad.saturating_mul(2)),
                area.height,
            )
        } else {
            area
        };

        let main_area = if self.work_dock_open && workspace_area.height >= 12 {
            let split = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(6), Constraint::Length(5)])
                .split(workspace_area);
            self.render_work_dock(frame, split[1]);
            split[0]
        } else {
            workspace_area
        };

        let transcript_and_cards = |this: &Self, f: &mut Frame, target: Rect| {
            let show_cards = (this.todo_card_open || this.info_card_open) && target.height >= 10;
            if show_cards {
                let split = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(4), Constraint::Min(5)])
                    .split(target);
                this.render_pinned_cards(f, split[0]);
                this.render_transcript_pane(f, split[1]);
            } else {
                this.render_transcript_pane(f, target);
            }
        };

        if !self.sidebar_visible {
            transcript_and_cards(self, frame, main_area);
            return;
        }
        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
            .split(main_area);

        transcript_and_cards(self, frame, panes[0]);
        self.render_telemetry_sidebar(frame, panes[1]);
    }

    fn render_pinned_cards(&self, frame: &mut Frame, area: Rect) {
        let f = &self.fleet_snapshot;
        let lane_dots: String = self
            .kanban_snapshot
            .in_progress
            .iter()
            .map(|_| "◉")
            .chain(self.kanban_snapshot.todo.iter().map(|_| "○"))
            .chain(self.kanban_snapshot.blocked.iter().map(|_| "⚠"))
            .chain(self.kanban_snapshot.done.iter().map(|_| "✔"))
            .take(16)
            .collect::<Vec<_>>()
            .join(" ");
        let dots_display = if lane_dots.is_empty() {
            "none".to_owned()
        } else {
            lane_dots
        };
        let cache_state = if self.turns > 0 && self.session_id.is_some() {
            "warm"
        } else {
            "cold"
        };
        let lines = vec![
            Line::from(vec![
                Span::styled(" ◆ Todo Lanes: ", self.theme.title()),
                Span::raw(format!(
                    "{dots_display} · board '{}' ({} active, {} todo, {} blocked, {} done)",
                    self.kanban_snapshot.board_id,
                    self.kanban_snapshot.in_progress.len(),
                    self.kanban_snapshot.todo.len(),
                    self.kanban_snapshot.blocked.len(),
                    self.kanban_snapshot.done.len()
                )),
            ]),
            Line::from(vec![
                Span::styled(" ◆ Session & Usage: ", self.theme.prompt()),
                Span::raw(format!(
                    "{}:{} (effort: {}) · branch: {} · cache: {cache_state} · tokens: {}/{} · cost: {}μ",
                    self.active_provider_label,
                    self.active_model_label,
                    self.effort.name(),
                    self.git_branch,
                    f.tokens_consumed,
                    f.tokens_ceiling,
                    f.cost_display
                )),
            ]),
        ];
        let card = Paragraph::new(lines).block(
            Block::default()
                .title(" Pinned Session Cards (Alt+X Todos · Alt+I Info) ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(self.theme.accent()),
        );
        frame.render_widget(card, area);
    }

    fn render_work_dock(&self, frame: &mut Frame, area: Rect) {
        let f = &self.fleet_snapshot;
        let cred_badge = if self.credential_ready {
            "✓ key ready"
        } else {
            "⚠ key missing (/key)"
        };
        let cache_state = if self.turns > 0 && self.session_id.is_some() {
            "cache: warm"
        } else {
            "cache: cold"
        };
        let dock_lines = vec![
            Line::from(vec![
                Span::styled(" ◆ Model & Key:  ", self.theme.title()),
                Span::raw(format!(
                    "{}:{} ({}) · {} · {} · branch {}",
                    self.active_provider_label,
                    self.active_model_label,
                    self.active_api_key_env,
                    cred_badge,
                    cache_state,
                    self.git_branch
                )),
            ]),
            Line::from(vec![
                Span::styled(" ◆ Fleet & Jobs: ", self.theme.prompt()),
                Span::raw(format!(
                    "status {} · jobs {} running / {} queued · orchestrations {} running · {} supervisors",
                    f.status,
                    f.jobs_running,
                    f.jobs_queued,
                    f.orchestrations_running,
                    f.running_supervisors
                )),
            ]),
            Line::from(vec![
                Span::styled(" ◆ Active Board: ", self.theme.accent()),
                Span::raw(format!(
                    "'{}' — {} in_progress · {} todo · {} blocked · {} done",
                    self.kanban_snapshot.board_id,
                    self.kanban_snapshot.in_progress.len(),
                    self.kanban_snapshot.todo.len(),
                    self.kanban_snapshot.blocked.len(),
                    self.kanban_snapshot.done.len()
                )),
            ]),
        ];
        let dock = Paragraph::new(dock_lines).block(
            Block::default()
                .title(" Pandora Live Work Dock (Ctrl+T or /dock to toggle) ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(self.theme.border()),
        );
        frame.render_widget(dock, area);
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
            format!(
                " Transcript (scrolled +{effective_scroll} · PageDown / Scroll down to follow) "
            )
        } else {
            " Transcript (Scroll wheel / PageUp / Shift+Up to view history) ".to_owned()
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
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(8),
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
        let provider = &self.active_provider_label;
        let model = &self.active_model_label;
        let key_badge = if self.credential_ready {
            format!("✓ {}", self.active_api_key_env)
        } else {
            format!("⚠ {} (/key)", self.active_api_key_env)
        };
        let key_style = if self.credential_ready {
            self.theme.prompt()
        } else {
            self.theme.warning()
        };
        let cache_label = if self.turns > 0 && self.session_id.is_some() {
            "warm"
        } else {
            "cold"
        };
        let latency_label = match self.last_turn_ms {
            Some(ms) => format!("{ms}ms · cache: {cache_label}"),
            None => format!("n/a · cache: {cache_label}"),
        };
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
                    &format!("{session} ({})", self.git_branch),
                    usize::from(chunks[2].width.saturating_sub(13)),
                )),
            ]),
            Line::from(vec![
                Span::styled(" Provider: ", self.theme.muted()),
                Span::styled(
                    truncate(
                        &format!("{provider} (effort: {})", self.effort.name()),
                        usize::from(chunks[2].width.saturating_sub(13)),
                    ),
                    self.theme.title(),
                ),
            ]),
            Line::from(vec![
                Span::styled(" Model:    ", self.theme.muted()),
                Span::raw(truncate(
                    model,
                    usize::from(chunks[2].width.saturating_sub(13)),
                )),
            ]),
            Line::from(vec![
                Span::styled(" API Key:  ", self.theme.muted()),
                Span::styled(
                    truncate(&key_badge, usize::from(chunks[2].width.saturating_sub(13))),
                    key_style,
                ),
            ]),
            Line::from(vec![
                Span::styled(" Latency:  ", self.theme.muted()),
                Span::raw(truncate(
                    &latency_label,
                    usize::from(chunks[2].width.saturating_sub(13)),
                )),
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
            if chunks[2].height >= 16 {
                inspector_lines.push(Line::from(""));
            }
            inspector_lines.push(Line::from(Span::styled(
                " Quick Shortcuts:",
                self.theme.title(),
            )));
            inspector_lines.push(Line::from(Span::styled(
                " • Ctrl+M /model  Model Picker",
                self.theme.muted(),
            )));
            inspector_lines.push(Line::from(Span::styled(
                " • /key · /config API Key & Setup",
                self.theme.muted(),
            )));
            inspector_lines.push(Line::from(Span::styled(
                " • Ctrl+P / F1    Command Palette",
                self.theme.muted(),
            )));
            inspector_lines.push(Line::from(Span::styled(
                " • Ctrl+T /dock   Live Work Dock",
                self.theme.muted(),
            )));
            inspector_lines.push(Line::from(Span::styled(
                " • Ctrl+S /stash  Prompt Stash",
                self.theme.muted(),
            )));
            inspector_lines.push(Line::from(Span::styled(
                " • / · ! · @ +Tab Autocomplete",
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
                format!(" • Governance Mode: {}", self.governance_mode.as_str()),
                self.theme.prompt(),
            )),
            Line::from(Span::styled(
                match self.governance_mode {
                    pandora_types::GovernanceMode::Autonomous => {
                        "   [Autonomous: 0 human approval barriers, direct tool execution]"
                    }
                    pandora_types::GovernanceMode::Supervised => {
                        "   [Supervised: write/execute/connect require human approval]"
                    }
                    pandora_types::GovernanceMode::ReadOnly => {
                        "   [Read-Only: mutating tools denied, inspection only]"
                    }
                },
                self.theme.warning(),
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
        let focus_tag = if self.focus_mode { " · ◉ FOCUS" } else { "" };
        let stash_tag = if self.draft_stash.is_empty() {
            String::new()
        } else {
            format!(" · 📌 {} stashed", self.draft_stash.len())
        };
        let approval_tag = if self.pending.is_some() {
            " · ⚠️ APPROVAL PENDING (Ctrl+Y / /approve)"
        } else {
            ""
        };
        let status = format!(
            "session: {session} · model: {}:{} · mode: {} · turns: {} · state: {} · view: {} · theme: {}{focus_tag}{stash_tag}{approval_tag}",
            self.active_provider_label,
            self.active_model_label,
            self.governance_mode.as_str(),
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
        let mode = ComposerMode::from_input(&self.input);
        let (prompt_glyph, prompt_style) = match mode {
            ComposerMode::Chat => ("> ", self.theme.prompt()),
            ComposerMode::SlashCommand => ("/ ", self.theme.accent()),
            ComposerMode::DirectGene => ("! ", self.theme.warning()),
        };
        let (input, cursor) = visible_input(&self.input, self.cursor, usize::from(area.width));
        let input_line = Line::from(vec![
            Span::styled(prompt_glyph, prompt_style),
            Span::raw(input),
        ]);
        let mut block = Block::default().borders(Borders::TOP);
        if self.pending.is_some() {
            block = block.title(Span::styled(
                " [⚠️ PENDING APPROVAL: /approve (Ctrl+Y) to allow · /deny (Ctrl+N) to reject] ",
                self.theme.warning(),
            ));
        } else if area.width >= WIDE_LAYOUT_MIN_WIDTH {
            block = block.title(Span::styled(mode.badge(), self.theme.muted()));
        }
        frame.render_widget(Paragraph::new(input_line).block(block), area);
        if !self.palette_open && !self.model_picker_open && !self.key_modal_open {
            frame.set_cursor_position(Position::new(
                area.x.saturating_add(cursor),
                area.y.saturating_add(1),
            ));
        }
    }

    fn render_autocomplete_popup(&self, frame: &mut Frame, workspace_area: Rect, input_area: Rect) {
        let suggestions = self.active_autocomplete_suggestions();
        if suggestions.is_empty() || workspace_area.height < 5 || input_area.width < 24 {
            return;
        }
        let total = suggestions.len();
        let max_visible = usize::from(workspace_area.height.saturating_sub(4)).clamp(3, 10);
        let visible_count = min(max_visible, total);
        let selected_idx = min(self.autocomplete_index, total.saturating_sub(1));

        let window_start = if selected_idx < visible_count {
            0
        } else {
            selected_idx.saturating_add(1).saturating_sub(visible_count)
        };
        let window_end = min(window_start + visible_count, total);

        let popup_height = (visible_count as u16).saturating_add(2);
        let popup_width = input_area.width.clamp(36, 84);
        let popup_y = input_area.y.saturating_sub(popup_height.saturating_add(1));
        let popup_area = Rect::new(input_area.x, popup_y, popup_width, popup_height);
        frame.render_widget(Clear, popup_area);

        let items = suggestions[window_start..window_end]
            .iter()
            .enumerate()
            .map(|(local_idx, (token, desc))| {
                let actual_idx = window_start + local_idx;
                let selected = actual_idx == selected_idx;
                let prefix = if selected { " ▸ " } else { "   " };
                let text = format!("{prefix}{:<22} {}", token, desc);
                let style = if selected {
                    self.theme.title()
                } else {
                    self.theme.muted()
                };
                ListItem::new(truncate(&text, usize::from(popup_width.saturating_sub(2))))
                    .style(style)
            })
            .collect::<Vec<_>>();

        let title = if total > visible_count {
            format!(
                " Commands ({}/{} · ↑/↓ navigate · Tab complete) ",
                selected_idx + 1,
                total
            )
        } else {
            format!(" Commands ({total} · ↑/↓ navigate · Tab complete) ")
        };
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(self.theme.accent());
        frame.render_widget(List::new(items).block(block), popup_area);
    }

    fn render_model_picker_modal(&self, frame: &mut Frame, area: Rect) {
        let modal_area = centered_rect(80, 72, area);
        frame.render_widget(Clear, modal_area);

        let outer = Block::default()
            .title(
                " ◆ Pandora Model & Provider Selector (Type to filter or provider:model · Enter to select · Esc to close) ",
            )
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(self.theme.accent());
        let inner = outer.inner(modal_area);
        frame.render_widget(outer, modal_area);

        if inner.height < 4 || inner.width < 12 {
            return;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(2),
                Constraint::Min(1),
            ])
            .split(inner);

        let key_status = if self.credential_ready {
            format!("✓ {} ready", self.active_api_key_env)
        } else {
            format!("⚠ {} missing (run /key)", self.active_api_key_env)
        };
        let active_banner = Line::from(vec![
            Span::styled(" Active: ", self.theme.muted()),
            Span::styled(
                format!("{}:{}", self.active_provider_label, self.active_model_label),
                self.theme.title(),
            ),
            Span::styled(format!(" · {key_status}"), self.theme.prompt()),
        ]);
        frame.render_widget(Paragraph::new(active_banner), chunks[0]);

        let query_str = self.model_picker_query.iter().collect::<String>();
        let search_line = Line::from(vec![
            Span::styled(" Filter / Custom (provider:model): ", self.theme.title()),
            Span::raw(&query_str),
        ]);
        frame.render_widget(
            Paragraph::new(search_line).block(Block::default().borders(Borders::BOTTOM)),
            chunks[1],
        );

        let config_opt = super::load_config(&self.args).ok();
        let entries = self.filtered_model_catalog();
        if entries.is_empty() {
            let custom_msg =
                format!(" ▸ Press Enter to configure custom model spec: '{query_str}'");
            frame.render_widget(
                Paragraph::new(truncate(&custom_msg, usize::from(chunks[2].width)))
                    .style(self.theme.title()),
                chunks[2],
            );
            return;
        }

        let max_items = usize::from(chunks[2].height.max(1));
        let start = self
            .model_picker_index
            .saturating_sub(max_items.saturating_sub(1));
        let items = entries
            .iter()
            .enumerate()
            .skip(start)
            .take(max_items)
            .map(|(idx, entry)| {
                let selected = idx == self.model_picker_index;
                let prefix = if selected { " ▸ " } else { "   " };
                let badge = if entry.provider == "ollama" {
                    "[✓ LOCAL]".to_owned()
                } else if let Some(preset) = super::setup::find_preset(entry.provider) {
                    let has_key = config_opt
                        .as_ref()
                        .and_then(|cfg| {
                            super::provider::provider_credential_available(cfg, preset.api_key_env)
                                .ok()
                        })
                        .unwrap_or(false);
                    if has_key {
                        "[✓ KEY READY]".to_owned()
                    } else {
                        format!("[needs {}]", preset.api_key_env)
                    }
                } else {
                    String::new()
                };
                let spec = format!("{}:{}", entry.provider, entry.model);
                let line_text = format!("{prefix}{:<44} {:<24} {}", spec, badge, entry.description);
                let style = if selected {
                    self.theme.title()
                } else {
                    Style::default()
                };
                ListItem::new(truncate(&line_text, usize::from(chunks[2].width))).style(style)
            })
            .collect::<Vec<_>>();

        frame.render_widget(List::new(items), chunks[2]);
    }

    fn render_key_modal(&self, frame: &mut Frame, area: Rect) {
        let modal_area = centered_rect(68, 42, area);
        frame.render_widget(Clear, modal_area);

        let outer = Block::default()
            .title(" ◆ Pandora Provider Credential Setup (.env Secret Store) ")
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(self.theme.accent());
        let inner = outer.inner(modal_area);
        frame.render_widget(outer, modal_area);

        if inner.height < 4 || inner.width < 12 {
            return;
        }

        let dotenv_display = super::load_config(&self.args)
            .map(|cfg| super::setup::dotenv_path(&cfg).display().to_string())
            .unwrap_or_else(|_| ".env".to_owned());
        let masked: String = std::iter::repeat_n('•', self.key_modal_input.len()).collect();
        let lines = vec![
            Line::from(vec![
                Span::styled(" Active Provider: ", self.theme.muted()),
                Span::styled(&self.active_provider_label, self.theme.title()),
                Span::styled(" · Model: ", self.theme.muted()),
                Span::raw(&self.active_model_label),
            ]),
            Line::from(vec![
                Span::styled(" Target Env Var:  ", self.theme.muted()),
                Span::styled(&self.key_modal_env, self.theme.prompt()),
            ]),
            Line::from(vec![
                Span::styled(" Secret File:     ", self.theme.muted()),
                Span::raw(truncate(
                    &dotenv_display,
                    usize::from(inner.width.saturating_sub(19)),
                )),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled(" API Key (masked): ", self.theme.title()),
                Span::styled(
                    format!("{masked} ({} chars)", self.key_modal_input.len()),
                    self.theme.prompt(),
                ),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                " Press Enter to save to .env · Ctrl+U to clear · Esc to cancel",
                self.theme.muted(),
            )),
        ];
        frame.render_widget(Paragraph::new(lines), inner);
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
    } else if let Some(rest) = text.strip_prefix("model> ") {
        Line::from(vec![
            Span::styled("model> ", theme.prompt()),
            Span::raw(rest.to_owned()),
        ])
    } else if let Some(rest) = text.strip_prefix("key> ") {
        Line::from(vec![
            Span::styled("key> ", theme.prompt()),
            Span::raw(rest.to_owned()),
        ])
    } else if let Some(rest) = text.strip_prefix("config> ") {
        Line::from(vec![
            Span::styled("config> ", theme.title()),
            Span::raw(rest.to_owned()),
        ])
    } else if let Some(rest) = text.strip_prefix("setup> ") {
        Line::from(vec![
            Span::styled("setup> ", theme.prompt()),
            Span::raw(rest.to_owned()),
        ])
    } else if let Some(rest) = text.strip_prefix("tools> ") {
        Line::from(vec![
            Span::styled("tools> ", theme.accent()),
            Span::raw(rest.to_owned()),
        ])
    } else if let Some(rest) = text.strip_prefix("stash> ") {
        Line::from(vec![
            Span::styled("stash> ", theme.accent()),
            Span::raw(rest.to_owned()),
        ])
    } else if let Some(rest) = text.strip_prefix("focus> ") {
        Line::from(vec![
            Span::styled("focus> ", theme.title()),
            Span::raw(rest.to_owned()),
        ])
    } else if let Some(rest) = text.strip_prefix("dock> ") {
        Line::from(vec![
            Span::styled("dock> ", theme.title()),
            Span::raw(rest.to_owned()),
        ])
    } else if let Some(rest) = text.strip_prefix("effort> ") {
        Line::from(vec![
            Span::styled("effort> ", theme.accent()),
            Span::raw(rest.to_owned()),
        ])
    } else if let Some(rest) = text.strip_prefix("session> ") {
        Line::from(vec![
            Span::styled("session> ", theme.prompt()),
            Span::raw(rest.to_owned()),
        ])
    } else if let Some(rest) = text.strip_prefix("todos> ") {
        Line::from(vec![
            Span::styled("todos> ", theme.title()),
            Span::raw(rest.to_owned()),
        ])
    } else if let Some(rest) = text.strip_prefix("usage> ") {
        Line::from(vec![
            Span::styled("usage> ", theme.prompt()),
            Span::raw(rest.to_owned()),
        ])
    } else if let Some(rest) = text.strip_prefix("info> ") {
        Line::from(vec![
            Span::styled("info> ", theme.title()),
            Span::raw(rest.to_owned()),
        ])
    } else if let Some(rest) = text.strip_prefix("hotkeys> ") {
        Line::from(vec![
            Span::styled("hotkeys> ", theme.accent()),
            Span::raw(rest.to_owned()),
        ])
    } else if let Some(rest) = text.strip_prefix("bookmark> ") {
        Line::from(vec![
            Span::styled("bookmark> ", theme.prompt()),
            Span::raw(rest.to_owned()),
        ])
    } else if let Some(rest) = text.strip_prefix("hill-climb> ") {
        Line::from(vec![
            Span::styled("hill-climb> ", theme.prompt()),
            Span::raw(rest.to_owned()),
        ])
    } else if let Some(rest) = text.strip_prefix("confidence> ") {
        Line::from(vec![
            Span::styled("confidence> ", theme.title()),
            Span::raw(rest.to_owned()),
        ])
    } else if let Some(rest) = text.strip_prefix("autopoke> ") {
        Line::from(vec![
            Span::styled("autopoke> ", theme.accent()),
            Span::raw(rest.to_owned()),
        ])
    } else if let Some(rest) = text.strip_prefix("selfdev> ") {
        Line::from(vec![
            Span::styled("selfdev> ", theme.title()),
            Span::raw(rest.to_owned()),
        ])
    } else if let Some(rest) = text.strip_prefix("provider> ") {
        Line::from(vec![
            Span::styled("provider> ", theme.prompt()),
            Span::raw(rest.to_owned()),
        ])
    } else if let Some(rest) = text.strip_prefix("oauth> ") {
        Line::from(vec![
            Span::styled("oauth> ", theme.title()),
            Span::raw(rest.to_owned()),
        ])
    } else if let Some(rest) = text.strip_prefix("account> ") {
        Line::from(vec![
            Span::styled("account> ", theme.accent()),
            Span::raw(rest.to_owned()),
        ])
    } else if let Some(rest) = text.strip_prefix("backdrop> ") {
        Line::from(vec![
            Span::styled("backdrop> ", theme.title()),
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

fn detect_git_branch(workspace_dir: &std::path::Path) -> String {
    let mut current = Some(workspace_dir);
    while let Some(dir) = current {
        let head_path = dir.join(".git").join("HEAD");
        if let Ok(content) = std::fs::read_to_string(&head_path) {
            let trimmed = content.trim();
            if let Some(branch) = trimmed.strip_prefix("ref: refs/heads/") {
                return branch.trim().to_owned();
            }
            if trimmed.len() >= 7 {
                return trimmed[..7].to_owned();
            }
        }
        current = dir.parent();
    }
    "main".to_owned()
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
    fn tui_mode_hotswap_updates_governance_mode_and_run_args() {
        let mut app = app();
        app.input = "/mode read-only".chars().collect();
        app.submit();

        assert_eq!(app.governance_mode, pandora_types::GovernanceMode::ReadOnly);
        assert!(app.mode_explicit);
        assert_eq!(
            app.run_args("inspect", None),
            ["--agent", "--mode", "read-only", "inspect"]
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
        assert!(rendered.contains(env!("CARGO_PKG_VERSION")));
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

    #[test]
    fn key_release_events_are_ignored_to_prevent_double_input() {
        let mut app = app();
        app.handle_key(KeyEvent::new_with_kind(
            KeyCode::Char('/'),
            KeyModifiers::NONE,
            super::KeyEventKind::Press,
        ));
        app.handle_key(KeyEvent::new_with_kind(
            KeyCode::Char('/'),
            KeyModifiers::NONE,
            super::KeyEventKind::Release,
        ));
        assert_eq!(app.input.iter().collect::<String>(), "/");
    }

    #[test]
    fn model_picker_and_dotenv_key_workflow() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("test clock must follow the Unix epoch")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!(
            "pandora-tui-model-picker-{}-{nonce}",
            std::process::id()
        ));
        let config_path = temp_dir.join("config.json");
        let data_dir = temp_dir.join("data");
        let workspace_dir = temp_dir.join("workspace");
        let mut values = BTreeMap::new();
        values.insert(
            "config".to_owned(),
            config_path.to_string_lossy().into_owned(),
        );
        values.insert(
            "data-dir".to_owned(),
            data_dir.to_string_lossy().into_owned(),
        );
        values.insert(
            "workspace".to_owned(),
            workspace_dir.to_string_lossy().into_owned(),
        );
        let mut app = App::new(
            ParsedArgs {
                values,
                positionals: Vec::new(),
            },
            TuiTheme::Auto,
        );

        // Open Pandora Model Picker with Ctrl+M
        app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::CONTROL));
        assert!(app.model_picker_open);

        // Filter to "nous:Hermes-3-Llama-3.1-70B"
        for ch in "nous:Hermes-3-Llama-3.1-70B".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!app.model_picker_open);
        assert_eq!(app.active_provider_label, "nous");
        assert_eq!(app.active_model_label, "Hermes-3-Llama-3.1-70B");
        assert_eq!(app.active_api_key_env, "NOUS_API_KEY");

        // Save API key via /key command into .env and verify it is ready & redacted from history
        app.input = "/key sk-nous-test-secret-9876".chars().collect();
        app.submit();
        assert!(app.credential_ready);
        assert!(
            app.messages
                .iter()
                .any(|m| m.contains("key> saved 'NOUS_API_KEY' (••••9876)"))
        );
        assert!(
            !app.history
                .iter()
                .any(|h| h.contains("sk-nous-test-secret"))
        );

        // Verify /config set model.default switches to ollama local model
        app.input = "/config set model.default ollama:qwen2.5-coder:32b"
            .chars()
            .collect();
        app.submit();
        assert_eq!(app.active_provider_label, "ollama");
        assert_eq!(app.active_model_label, "qwen2.5-coder:32b");
        assert!(app.credential_ready);

        // Verify slash command autocomplete via Tab
        app.input = "/foc".chars().collect();
        app.cursor = app.input.len();
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.input.iter().collect::<String>(), "/focus ");

        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn tui_branding_uses_pandora_and_supports_stash_and_autocomplete_navigation() {
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = app();

        app.input = "/help".chars().collect();
        app.submit();
        for msg in &app.messages {
            assert!(
                !msg.contains("Hermes"),
                "unexpected external branding in help message: {msg}"
            );
        }

        app.work_dock_open = true;
        terminal.draw(|frame| app.render(frame)).unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("Quick Shortcuts:"));
        assert!(rendered.contains("Pandora Live Work Dock"));
        assert!(!rendered.contains("Hermes"));

        // Prompt stash round-trip via Ctrl+S
        app.input = "investigate memory consolidation".chars().collect();
        app.cursor = app.input.len();
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(app.input.is_empty());
        assert_eq!(app.draft_stash.len(), 1);

        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert_eq!(
            app.input.iter().collect::<String>(),
            "investigate memory consolidation"
        );
        assert!(app.draft_stash.is_empty());

        // Arrow-key navigation inside floating autocomplete popover + Tab completion
        app.input = "/m".chars().collect();
        app.cursor = app.input.len();
        let suggestions = app.active_autocomplete_suggestions();
        assert!(suggestions.len() >= 2);
        let second_cmd = suggestions[1].0.clone();
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.autocomplete_index, 1);
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(
            app.input.iter().collect::<String>(),
            format!("{second_cmd} ")
        );

        // Direct Gene mode (`!`) detection and autocomplete
        app.input = "!st".chars().collect();
        app.cursor = app.input.len();
        assert_eq!(
            super::ComposerMode::from_input(&app.input),
            super::ComposerMode::DirectGene
        );
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.input.iter().collect::<String>(), "!status ");

        // Reasoning effort cycling (Alt+Right / /effort)
        assert_eq!(app.effort, super::ReasoningEffort::Medium);
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::ALT));
        assert_eq!(app.effort, super::ReasoningEffort::High);
        app.input = "/effort max".chars().collect();
        app.submit();
        assert_eq!(app.effort, super::ReasoningEffort::Max);

        // Session fork, rewind, and pinned todo/info cards (Alt+X / Alt+I)
        app.input = "/fork".chars().collect();
        app.submit();
        assert!(app.session_id.as_deref().unwrap_or("").contains("fork"));

        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT));
        assert!(app.todo_card_open);
        terminal.draw(|frame| app.render(frame)).unwrap();
        let card_rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(card_rendered.contains("Pinned Session Cards"));

        // Hill-climb goal evaluation, confidence gate, autopoke, and selfdev commands
        app.input = "/hill-climb optimize parser latency below 5ms with cargo test"
            .chars()
            .collect();
        app.submit();
        assert!(
            app.messages
                .iter()
                .any(|m| m.contains("hill-climb> score 95/100 (PASS >= 90)"))
        );

        app.input = "/confidence".chars().collect();
        app.submit();
        assert!(
            app.messages
                .iter()
                .any(|m| m.contains("Stepped Confidence & Spike Check Gate"))
        );

        app.input = "/autopoke".chars().collect();
        app.submit();
        assert!(
            app.messages
                .iter()
                .any(|m| m.contains("Unfinished Todo Continuation Gate"))
        );

        app.input = "/selfdev".chars().collect();
        app.submit();
        assert!(
            app.messages
                .iter()
                .any(|m| m.contains("Pandora Governed Self-Development Workflow"))
        );

        // Backdrop mode cycling via Alt+B and /backdrop
        assert_eq!(
            app.backdrop_mode,
            super::super::tui_backdrop::BackdropMode::Subtle
        );
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT));
        assert_eq!(
            app.backdrop_mode,
            super::super::tui_backdrop::BackdropMode::Vivid
        );
        app.input = "/backdrop off".chars().collect();
        app.submit();
        assert_eq!(
            app.backdrop_mode,
            super::super::tui_backdrop::BackdropMode::Off
        );
    }

    #[test]
    fn custom_provider_and_oauth_pkce_multi_account_workflow() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("test clock must follow the Unix epoch")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!(
            "pandora-tui-oauth-custom-{}-{nonce}",
            std::process::id()
        ));
        let config_path = temp_dir.join("config.json");
        let data_dir = temp_dir.join("data");
        let workspace_dir = temp_dir.join("workspace");
        let mut values = BTreeMap::new();
        values.insert(
            "config".to_owned(),
            config_path.to_string_lossy().into_owned(),
        );
        values.insert(
            "data-dir".to_owned(),
            data_dir.to_string_lossy().into_owned(),
        );
        values.insert(
            "workspace".to_owned(),
            workspace_dir.to_string_lossy().into_owned(),
        );
        let mut app = App::new(
            ParsedArgs {
                values,
                positionals: Vec::new(),
            },
            TuiTheme::Auto,
        );

        // 1. Add a custom OpenAI-compatible provider with --no-api-key
        app.input =
            "/provider add my-local-api http://localhost:8080/v1 custom-coder-7b --no-api-key"
                .chars()
                .collect();
        app.submit();
        assert_eq!(app.active_provider_label, "my-local-api");
        assert_eq!(app.active_model_label, "custom-coder-7b");
        assert!(app.credential_ready);

        // 2. Start OAuth PKCE S256 login flow for Claude (account 'work')
        app.input = "/login claude --account work".chars().collect();
        app.submit();
        assert!(
            app.messages
                .iter()
                .any(|m| m.contains("oauth> PKCE S256 flow started for 'claude' (account 'work'"))
        );

        // Read state from pending OAuth file to verify CSRF state validation on callback
        let pending_raw =
            std::fs::read_to_string(data_dir.join("oauth_pending.json")).expect("pending oauth");
        let pending_json: serde_json::Value =
            serde_json::from_str(&pending_raw).expect("valid json");
        let state = pending_json["state"].as_str().expect("state string");

        // 3. Complete OAuth callback
        app.input = format!(
            "/login callback http://127.0.0.1:1455/auth/callback?code=authcode9988&state={state}"
        )
        .chars()
        .collect();
        app.submit();
        assert_eq!(app.active_provider_label, "anthropic");
        assert!(app.credential_ready);
        assert!(
            app.messages
                .iter()
                .any(|m| m.contains("oauth> authenticated 'claude' (account 'work'"))
        );

        // 4. Add second account ('personal') and switch between accounts via /account
        app.input = "/login claude --account personal".chars().collect();
        app.submit();
        app.input = "/login callback sk-ant-personal-5544".chars().collect();
        app.submit();
        assert!(
            app.messages
                .iter()
                .any(|m| m.contains("account 'personal'"))
        );

        app.input = "/account claude work".chars().collect();
        app.submit();
        assert!(
            app.messages
                .iter()
                .any(|m| m.contains("account> switched active 'claude' account to 'work'"))
        );

        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
