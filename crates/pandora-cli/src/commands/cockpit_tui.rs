//! Interactive Ratatui Mission-Control Cockpit TUI (Phase 22).
//!
//! Provides real-time interactive 9/12-panel grid visualization, keyboard navigation,
//! drill-down telemetry modals, and multi-agent consensus triggers.
//!
//! # Safety Invariant
//! Strictly `#![forbid(unsafe_code)]` enforced workspace-wide.

#![forbid(unsafe_code)]

use crate::output::{CliError, CommandResult, success};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use pandora_runtime::cockpit::{CockpitController, CockpitLayout, PanelHealth, PanelStatus};
use pandora_runtime::multi_agent_consensus::{
    AgentPatchProposal, ProposalScoreVector, SwarmCodeConsensusEngine,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::{Frame, Terminal};
use serde_json::json;
use std::io::{IsTerminal, Stdout, stdout};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Run the interactive Cockpit TUI.
pub fn run_interactive(layout: CockpitLayout) -> Result<CommandResult, CliError> {
    // If not running in an interactive terminal (e.g. CI, piped, headless), fallback gracefully
    if !stdout().is_terminal() {
        let controller = CockpitController::new(layout);
        let epoch_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let snapshot = controller.capture_snapshot(epoch_ms);
        let ascii = controller.render_ascii_grid(&snapshot);
        return Ok(success(
            "cockpit tui",
            json!({
                "status": "headless_snapshot",
                "layout": format!("{:?}", layout),
                "healthy_panels": snapshot.healthy_panels,
                "panel_count": snapshot.panel_count,
            }),
            ascii,
        ));
    }

    let mut state = CockpitTuiState::new(layout);
    state.run()?;

    Ok(success(
        "cockpit tui",
        json!({
            "status": "session_closed",
            "layout": format!("{:?}", state.layout),
            "healthy_panels": state.cached_snapshot.healthy_panels,
            "panel_count": state.cached_snapshot.panel_count,
        }),
        format!(
            "Cockpit TUI session completed. Final layout: {:?} ({}/{} healthy).",
            state.layout, state.cached_snapshot.healthy_panels, state.cached_snapshot.panel_count
        ),
    ))
}

struct CockpitTuiState {
    layout: CockpitLayout,
    controller: CockpitController,
    active_panel_idx: usize,
    modal_open: bool,
    consensus_modal_open: bool,
    autopilot_modal_open: bool,
    last_consensus_summary: Option<String>,
    last_autopilot_summary: Option<String>,
    cached_snapshot: pandora_runtime::cockpit::CockpitSnapshot,
    last_refresh: Instant,
}

impl CockpitTuiState {
    fn new(layout: CockpitLayout) -> Self {
        let controller = CockpitController::new(layout);
        let epoch_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let snapshot = controller.capture_snapshot(epoch_ms);

        Self {
            layout,
            controller,
            active_panel_idx: 0,
            modal_open: false,
            consensus_modal_open: false,
            autopilot_modal_open: false,
            last_consensus_summary: None,
            last_autopilot_summary: None,
            cached_snapshot: snapshot,
            last_refresh: Instant::now(),
        }
    }

    fn refresh_snapshot(&mut self) {
        let epoch_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        self.cached_snapshot = self.controller.capture_snapshot(epoch_ms);
        self.last_refresh = Instant::now();
    }

    fn switch_layout(&mut self, new_layout: CockpitLayout) {
        if self.layout != new_layout {
            self.layout = new_layout;
            self.controller = CockpitController::new(new_layout);
            self.refresh_snapshot();
            if self.active_panel_idx >= self.cached_snapshot.panels.len() {
                self.active_panel_idx = 0;
            }
        }
    }

    fn run(&mut self) -> Result<(), CliError> {
        enable_raw_mode().map_err(|e| {
            CliError::execution(format!("raw mode error: {e}"), serde_json::json!({}))
        })?;
        let mut stdout_handle = stdout();
        execute!(stdout_handle, EnterAlternateScreen).map_err(|e| {
            CliError::execution(
                format!("enter alternate screen error: {e}"),
                serde_json::json!({}),
            )
        })?;
        let backend = CrosstermBackend::new(stdout_handle);
        let mut terminal = Terminal::new(backend).map_err(|e| {
            CliError::execution(
                format!("terminal creation error: {e}"),
                serde_json::json!({}),
            )
        })?;

        let res = self.event_loop(&mut terminal);

        // Always restore terminal state even if error occurred
        let _ = disable_raw_mode();
        let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
        let _ = terminal.show_cursor();

        res
    }

    fn event_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<(), CliError> {
        loop {
            // Periodic refresh every 1000ms
            if self.last_refresh.elapsed() >= Duration::from_millis(1000) {
                self.refresh_snapshot();
            }

            terminal.draw(|f| self.render_ui(f)).map_err(|e| {
                CliError::execution(format!("draw error: {e}"), serde_json::json!({}))
            })?;

            let has_event = event::poll(Duration::from_millis(100)).map_err(|e| {
                CliError::execution(format!("poll error: {e}"), serde_json::json!({}))
            })?;
            if has_event {
                let evt = event::read().map_err(|e| {
                    CliError::execution(format!("read error: {e}"), serde_json::json!({}))
                })?;
                if let Event::Key(key) = evt
                    && key.kind == KeyEventKind::Press
                    && self.handle_key(key)
                {
                    break;
                }
            }
        }

        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        let cols = match self.layout {
            CockpitLayout::NinePanel => 3,
            CockpitLayout::TwelvePanel => 4,
        };
        let total = self.cached_snapshot.panels.len();

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                if self.modal_open {
                    self.modal_open = false;
                    false
                } else if self.consensus_modal_open {
                    self.consensus_modal_open = false;
                    false
                } else if self.autopilot_modal_open {
                    self.autopilot_modal_open = false;
                    false
                } else {
                    true // exit
                }
            }
            KeyCode::Char('9') => {
                self.switch_layout(CockpitLayout::NinePanel);
                false
            }
            KeyCode::Char('1') | KeyCode::Char('2') => {
                self.switch_layout(CockpitLayout::TwelvePanel);
                false
            }
            KeyCode::Tab => {
                match self.layout {
                    CockpitLayout::NinePanel => self.switch_layout(CockpitLayout::TwelvePanel),
                    CockpitLayout::TwelvePanel => self.switch_layout(CockpitLayout::NinePanel),
                }
                false
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if !self.modal_open
                    && !self.consensus_modal_open
                    && self.active_panel_idx + 1 < total
                {
                    self.active_panel_idx += 1;
                }
                false
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if !self.modal_open && !self.consensus_modal_open && self.active_panel_idx > 0 {
                    self.active_panel_idx -= 1;
                }
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !self.modal_open
                    && !self.consensus_modal_open
                    && self.active_panel_idx + cols < total
                {
                    self.active_panel_idx += cols;
                }
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if !self.modal_open && !self.consensus_modal_open && self.active_panel_idx >= cols {
                    self.active_panel_idx -= cols;
                }
                false
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                if self.consensus_modal_open {
                    self.consensus_modal_open = false;
                } else if self.autopilot_modal_open {
                    self.autopilot_modal_open = false;
                } else {
                    self.modal_open = !self.modal_open;
                }
                false
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                self.run_consensus_simulation();
                self.consensus_modal_open = true;
                self.autopilot_modal_open = false;
                false
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                self.run_autopilot_simulation();
                self.autopilot_modal_open = true;
                self.consensus_modal_open = false;
                false
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                self.refresh_snapshot();
                false
            }
            _ => false,
        }
    }

    fn run_consensus_simulation(&mut self) {
        use pandora_runtime::multi_agent_collab::ExternalAgentKind;
        let proposals = vec![
            AgentPatchProposal::new(
                "p_agy",
                ExternalAgentKind::Agy,
                "Refactor authentication middleware",
                "Applies zeroize hygiene and rate-limited token hash verification",
                "diff --git a/src/auth.rs\n+ secure_token_verify()",
                ProposalScoreVector::new(1.0, 0.98, 0.92, 0.99),
                124,
            ),
            AgentPatchProposal::new(
                "p_claude",
                ExternalAgentKind::ClaudeCode,
                "Refactor auth with constant-time compare",
                "Guarantees subtle constant-time equality check",
                "diff --git a/src/auth.rs\n+ subtle::ConstantTimeEq",
                ProposalScoreVector::new(1.0, 0.96, 0.90, 0.98),
                180,
            ),
            AgentPatchProposal::new(
                "p_codex",
                ExternalAgentKind::CodexCli,
                "Add auth session cache",
                "Implements LRU token validation",
                "diff --git a/src/auth.rs\n+ session_cache.get()",
                ProposalScoreVector::new(0.95, 0.92, 0.88, 0.95),
                210,
            ),
            AgentPatchProposal::new(
                "p_byzantine",
                ExternalAgentKind::Hermes,
                "Bypass security checks for speed",
                "Maliciously disables authentication token check",
                "diff --git a/src/auth.rs\n- if !token.valid() return Err()",
                ProposalScoreVector::new(0.30, 0.20, 0.40, 0.10),
                95,
            ),
        ];

        let engine = SwarmCodeConsensusEngine::default();
        let epoch_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        match engine.resolve_consensus(&proposals, epoch_ms) {
            Ok(decision) => {
                self.last_consensus_summary = Some(format!(
                    "Consensus Reached! Winner: {} ({})\nCentroid Score: {:.3} | Confidence: {:.1}%\nHonest Quorum: {}/{} | Byzantine Filtered: {}\nReceipt Hash: {}",
                    decision.winning_title,
                    decision.winning_agent.display_name(),
                    decision.consensus_centroid.composite_score(),
                    decision.consensus_confidence * 100.0,
                    decision.honest_quorum_size,
                    decision.total_proposals,
                    decision.byzantine_filtered_count,
                    &decision.consensus_hash[..16]
                ));
            }
            Err(e) => {
                self.last_consensus_summary = Some(format!("Consensus Failed: {e}"));
            }
        }
    }

    fn render_ui(&self, f: &mut Frame) {
        let size = f.area();

        // Split into Header, Cockpit Grid, and Footer
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Header
                Constraint::Min(10),   // Grid
                Constraint::Length(2), // Footer help
            ])
            .split(size);

        // Header
        let header_text = vec![Line::from(vec![
            Span::styled(
                " AGENT-PANDORA MISSION-CONTROL COCKPIT ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "| Layout: {:?} | Status: {}/{} Healthy | Digest: {} | Epoch: {}ms",
                self.layout,
                self.cached_snapshot.healthy_panels,
                self.cached_snapshot.panel_count,
                &self.cached_snapshot.snapshot_digest[..12],
                self.cached_snapshot.timestamp_ms
            )),
        ])];
        let header_p = Paragraph::new(header_text).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan)),
        );
        f.render_widget(header_p, chunks[0]);

        // Cockpit Grid
        let (rows, cols) = match self.layout {
            CockpitLayout::NinePanel => (3, 3),
            CockpitLayout::TwelvePanel => (3, 4),
        };

        let row_constraints = vec![Constraint::Ratio(1, rows as u32); rows];
        let row_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(row_constraints)
            .split(chunks[1]);

        for r in 0..rows {
            let col_constraints = vec![Constraint::Ratio(1, cols as u32); cols];
            let col_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(col_constraints)
                .split(row_chunks[r]);

            for c in 0..cols {
                let panel_idx = r * cols + c;
                if panel_idx < self.cached_snapshot.panels.len() {
                    let panel = &self.cached_snapshot.panels[panel_idx];
                    let is_active = panel_idx == self.active_panel_idx;
                    self.render_panel(f, col_chunks[c], panel, is_active);
                }
            }
        }

        // Footer
        let footer_text = Line::from(vec![
            Span::styled(" [9]", Style::default().fg(Color::Yellow)),
            Span::raw(" 3x3 "),
            Span::styled("[12/Tab]", Style::default().fg(Color::Yellow)),
            Span::raw(" 3x4 "),
            Span::styled("[Arrows/HJKL]", Style::default().fg(Color::Yellow)),
            Span::raw(" Move "),
            Span::styled("[Enter]", Style::default().fg(Color::Yellow)),
            Span::raw(" Drilldown "),
            Span::styled("[C]", Style::default().fg(Color::Yellow)),
            Span::raw(" Swarm Consensus "),
            Span::styled("[A]", Style::default().fg(Color::Yellow)),
            Span::raw(" Autopilot "),
            Span::styled("[R]", Style::default().fg(Color::Yellow)),
            Span::raw(" Refresh "),
            Span::styled("[Q/Esc]", Style::default().fg(Color::Yellow)),
            Span::raw(" Exit"),
        ]);
        f.render_widget(Paragraph::new(footer_text), chunks[2]);

        // Modals
        if self.modal_open {
            self.render_drilldown_modal(f, size);
        } else if self.consensus_modal_open {
            self.render_consensus_modal(f, size);
        } else if self.autopilot_modal_open {
            self.render_autopilot_modal(f, size);
        }
    }

    fn render_panel(&self, f: &mut Frame, area: Rect, panel: &PanelStatus, is_active: bool) {
        let (border_color, health_label) = match panel.health {
            PanelHealth::Healthy => (Color::Green, "HEALTHY"),
            PanelHealth::Degraded => (Color::Yellow, "DEGRADED"),
            PanelHealth::Critical => (Color::Red, "CRITICAL"),
            PanelHealth::Paused => (Color::DarkGray, "PAUSED"),
        };

        let active_style = if is_active {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(border_color)
        };

        let title = if is_active {
            format!("* {} [{}] *", panel.title, health_label)
        } else {
            format!(" {} [{}] ", panel.title, health_label)
        };

        let mut lines = vec![Line::from(Span::styled(
            &panel.headline,
            Style::default().fg(Color::White),
        ))];

        // Show top 2 metrics in compact card
        for (k, v) in panel.metrics.iter().take(2) {
            lines.push(Line::from(vec![
                Span::styled(format!("{k}: "), Style::default().fg(Color::DarkGray)),
                Span::styled(v, Style::default().fg(Color::Yellow)),
            ]));
        }

        let p = Paragraph::new(lines).block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_type(if is_active {
                    BorderType::Thick
                } else {
                    BorderType::Rounded
                })
                .border_style(active_style),
        );

        f.render_widget(p, area);
    }

    fn render_drilldown_modal(&self, f: &mut Frame, area: Rect) {
        let panel = &self.cached_snapshot.panels[self.active_panel_idx];
        let modal_area = centered_rect(70, 70, area);

        f.render_widget(Clear, modal_area);

        let mut lines = vec![
            Line::from(vec![
                Span::styled("Subsystem: ", Style::default().fg(Color::DarkGray)),
                Span::styled(&panel.title, Style::default().add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::styled("Category: ", Style::default().fg(Color::DarkGray)),
                Span::raw(panel.category.as_str()),
                Span::raw(" | "),
                Span::styled("Health: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    panel.health.as_str(),
                    Style::default().fg(match panel.health {
                        PanelHealth::Healthy => Color::Green,
                        PanelHealth::Degraded => Color::Yellow,
                        PanelHealth::Critical => Color::Red,
                        PanelHealth::Paused => Color::DarkGray,
                    }),
                ),
            ]),
            Line::from(vec![
                Span::styled("Status: ", Style::default().fg(Color::DarkGray)),
                Span::styled(&panel.headline, Style::default().fg(Color::White)),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "--- Subsystem Telemetry Metrics ---",
                Style::default().fg(Color::Cyan),
            )),
        ];

        for (k, v) in &panel.metrics {
            lines.push(Line::from(vec![
                Span::styled(format!("  * {k}: "), Style::default().fg(Color::DarkGray)),
                Span::styled(v, Style::default().fg(Color::Yellow)),
            ]));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "--- Live Stream Tail Logs ---",
            Style::default().fg(Color::Cyan),
        )));

        for log in &panel.live_stream_tail {
            lines.push(Line::from(vec![
                Span::styled("  [log] ", Style::default().fg(Color::DarkGray)),
                Span::raw(log),
            ]));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Press [Esc] or [Enter] to return to grid",
            Style::default().fg(Color::DarkGray),
        )));

        let modal_p = Paragraph::new(lines).block(
            Block::default()
                .title(format!(" Panel Details: {} ", panel.title))
                .borders(Borders::ALL)
                .border_type(BorderType::Double)
                .border_style(Style::default().fg(Color::Cyan)),
        );

        f.render_widget(modal_p, modal_area);
    }

    fn render_consensus_modal(&self, f: &mut Frame, area: Rect) {
        let modal_area = centered_rect(65, 50, area);
        f.render_widget(Clear, modal_area);

        let content = self
            .last_consensus_summary
            .clone()
            .unwrap_or_else(|| "Running consensus...".to_string());

        let lines: Vec<Line> = content
            .lines()
            .map(|l| Line::from(Span::styled(l, Style::default().fg(Color::White))))
            .collect();

        let p = Paragraph::new(lines).block(
            Block::default()
                .title(" Multi-Agent Byzantine Code Consensus Quorum (Theorem 43) ")
                .borders(Borders::ALL)
                .border_type(BorderType::Double)
                .border_style(Style::default().fg(Color::Green)),
        );

        f.render_widget(p, modal_area);
    }

    fn run_autopilot_simulation(&mut self) {
        use pandora_runtime::autopilot::{AutopilotSessionConfig, MultiAgentAutopilotOrchestrator};
        let epoch_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let config = AutopilotSessionConfig::default();
        match MultiAgentAutopilotOrchestrator::run_session(config, epoch_ms) {
            Ok(outcome) => {
                self.last_autopilot_summary = Some(format!(
                    "Autopilot Mission Certified!\nGoal: {}\nAgents Dispatched: {}\nWinner: {} ({})\nQuorum: {}/{} | Byzantine Filtered: {}\nCertificate: {}",
                    outcome.task_goal,
                    outcome.spawned_agents.len(),
                    outcome.consensus_decision.winning_proposal_id,
                    outcome.consensus_decision.winning_agent.display_name(),
                    outcome.consensus_decision.honest_quorum_size,
                    outcome.consensus_decision.total_proposals,
                    outcome.consensus_decision.byzantine_filtered_count,
                    outcome.certified_patch.certificate_digest
                ));
            }
            Err(e) => {
                self.last_autopilot_summary = Some(format!("Autopilot Failed: {e}"));
            }
        }
    }

    fn render_autopilot_modal(&self, f: &mut Frame, area: Rect) {
        let modal_area = centered_rect(70, 55, area);
        f.render_widget(Clear, modal_area);

        let content = self
            .last_autopilot_summary
            .clone()
            .unwrap_or_else(|| "Running multi-agent autopilot...".to_string());

        let lines: Vec<Line> = content
            .lines()
            .map(|l| Line::from(Span::styled(l, Style::default().fg(Color::White))))
            .collect();

        let p = Paragraph::new(lines).block(
            Block::default()
                .title(" Live Multi-Agent Autopilot Session (Theorems 43 & 46) ")
                .borders(Borders::ALL)
                .border_type(BorderType::Double)
                .border_style(Style::default().fg(Color::Cyan)),
        );

        f.render_widget(p, modal_area);
    }
}

/// Helper function to create a centered rectangular modal area.
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
