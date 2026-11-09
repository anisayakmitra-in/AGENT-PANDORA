//! Multi-Agent Collaboration Orchestrator & Parallel Slash-Command Dispatcher.
//!
//! Enables parallel dispatch, permission-state monitoring, and consensus aggregation
//! across external coding agents (Agy, Claude, Codex, Gemini, Cursor, Devin, Cline, Hermes, Qwen).
//!
//! # Safety Invariant
//! Strictly `#![forbid(unsafe_code)]` enforced workspace-wide.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Recognized external AI coding agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalAgentKind {
    /// Google Antigravity CLI ('agy' / 'antigravity-cli')
    Agy,
    /// Anthropic Claude Code ('claude' / 'claude-code' / 'claude-cli')
    ClaudeCode,
    /// OpenAI Codex CLI ('codex' / 'codex-cli')
    CodexCli,
    /// Google Gemini CLI ('gemini' / 'gemini-cli')
    GeminiCli,
    /// Cursor Agent ('cursor-agent' / 'cursor-cli')
    CursorAgent,
    /// Cognition Devin ('devin' / 'devin-cli')
    Devin,
    /// Cline Autonomous Agent ('cline' / 'cline-cli')
    Cline,
    /// Hermes Agent ('hermes-agent' / 'hermes')
    Hermes,
    /// Alibaba Qwen Code ('qwen-code' / 'qwen')
    QwenCode,
}

impl ExternalAgentKind {
    pub fn all() -> &'static [ExternalAgentKind] {
        &[
            ExternalAgentKind::Agy,
            ExternalAgentKind::ClaudeCode,
            ExternalAgentKind::CodexCli,
            ExternalAgentKind::GeminiCli,
            ExternalAgentKind::CursorAgent,
            ExternalAgentKind::Devin,
            ExternalAgentKind::Cline,
            ExternalAgentKind::Hermes,
            ExternalAgentKind::QwenCode,
        ]
    }

    pub fn identifier(&self) -> &'static str {
        match self {
            ExternalAgentKind::Agy => "agy",
            ExternalAgentKind::ClaudeCode => "claude",
            ExternalAgentKind::CodexCli => "codex",
            ExternalAgentKind::GeminiCli => "gemini",
            ExternalAgentKind::CursorAgent => "cursor",
            ExternalAgentKind::Devin => "devin",
            ExternalAgentKind::Cline => "cline",
            ExternalAgentKind::Hermes => "hermes",
            ExternalAgentKind::QwenCode => "qwen",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            ExternalAgentKind::Agy => "Antigravity CLI (agy)",
            ExternalAgentKind::ClaudeCode => "Claude Code",
            ExternalAgentKind::CodexCli => "Codex CLI",
            ExternalAgentKind::GeminiCli => "Gemini CLI",
            ExternalAgentKind::CursorAgent => "Cursor Agent",
            ExternalAgentKind::Devin => "Devin AI",
            ExternalAgentKind::Cline => "Cline Agent",
            ExternalAgentKind::Hermes => "Hermes Agent",
            ExternalAgentKind::QwenCode => "Qwen Code",
        }
    }

    pub fn default_binary_name(&self) -> &'static str {
        match self {
            ExternalAgentKind::Agy => "agy",
            ExternalAgentKind::ClaudeCode => "claude",
            ExternalAgentKind::CodexCli => "codex",
            ExternalAgentKind::GeminiCli => "gemini",
            ExternalAgentKind::CursorAgent => "cursor",
            ExternalAgentKind::Devin => "devin",
            ExternalAgentKind::Cline => "cline",
            ExternalAgentKind::Hermes => "hermes",
            ExternalAgentKind::QwenCode => "qwen",
        }
    }

    /// Match a string token or slash command to a known agent.
    pub fn parse(s: &str) -> Option<Self> {
        let clean = s.trim().trim_start_matches('/').to_ascii_lowercase();
        match clean.as_str() {
            "agy" | "antigravity" | "antigravity-cli" => Some(ExternalAgentKind::Agy),
            "claude" | "claude-code" | "claude-cli" => Some(ExternalAgentKind::ClaudeCode),
            "codex" | "codex-cli" => Some(ExternalAgentKind::CodexCli),
            "gemini" | "gemini-cli" => Some(ExternalAgentKind::GeminiCli),
            "cursor" | "cursor-agent" | "cursor-cli" => Some(ExternalAgentKind::CursorAgent),
            "devin" | "devin-cli" => Some(ExternalAgentKind::Devin),
            "cline" | "cline-cli" => Some(ExternalAgentKind::Cline),
            "hermes" | "hermes-agent" => Some(ExternalAgentKind::Hermes),
            "qwen" | "qwen-code" => Some(ExternalAgentKind::QwenCode),
            _ => None,
        }
    }
}

/// Operational state of an active external agent terminal session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSessionState {
    Idle,
    Running {
        active_task: String,
        started_epoch_ms: u64,
    },
    WaitingPermission {
        prompt: String,
        suggested_action: Option<String>,
    },
    Completed {
        exit_code: i32,
        duration_ms: u64,
    },
    Failed {
        error: String,
    },
}

/// Tracking metadata for a spawned agent terminal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSession {
    pub session_id: String,
    pub agent: ExternalAgentKind,
    pub pane_id: u32,
    pub title: String,
    pub state: AgentSessionState,
    pub recent_output_tail: Vec<String>,
}

/// Parsed slash-command dispatch request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashDispatchCommand {
    /// Single agent task dispatch: `/agy <prompt>`, `/claude <prompt>`, etc.
    SingleAgent {
        agent: ExternalAgentKind,
        prompt: String,
    },
    /// Parallel multi-agent collaboration: `/collab agy claude codex -- <prompt>`
    CollaborativeSwarm {
        agents: Vec<ExternalAgentKind>,
        prompt: String,
    },
    /// List active multiplexer panes: `/panes`
    ListPanes,
    /// Zoom a specific pane: `/zoom <id>`
    ZoomPane { pane_id: u32 },
    /// Broadcast keystrokes to all active agents: `/broadcast <text>`
    BroadcastText { text: String },
    /// Display current reputation scores across all external coding agents: `/reputation`
    ReputationStatus,
    /// Automated multi-agent autopilot orchestration session: `/autopilot <goal>`
    AutopilotSession {
        goal: String,
        agents: Vec<ExternalAgentKind>,
    },
}

/// Result of parsing and executing a slash-command dispatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollabDispatchResult {
    pub command_line: String,
    pub spawned_agents: Vec<ExternalAgentKind>,
    pub pexer_ipc_commands: Vec<String>,
    pub explanation: String,
}

/// Central manager orchestrating multi-agent collaboration and slash-command routing.
#[derive(Debug, Default)]
pub struct MultiAgentCollabManager {
    sessions: BTreeMap<String, AgentSession>,
    next_pane_id: u32,
}

impl MultiAgentCollabManager {
    pub fn new() -> Self {
        Self {
            sessions: BTreeMap::new(),
            next_pane_id: 1,
        }
    }

    /// Parse an interactive slash-command line.
    pub fn parse_slash_command(line: &str) -> Result<SlashDispatchCommand, String> {
        let trimmed = line.trim();
        if !trimmed.starts_with('/') {
            return Err("slash command must start with '/'".to_string());
        }

        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.is_empty() {
            return Err("empty slash command".to_string());
        }

        let cmd = parts[0].to_ascii_lowercase();
        match cmd.as_str() {
            "/panes" => Ok(SlashDispatchCommand::ListPanes),
            "/zoom" => {
                let id = parts
                    .get(1)
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(0);
                Ok(SlashDispatchCommand::ZoomPane { pane_id: id })
            }
            "/broadcast" => {
                let text = parts[1..].join(" ");
                if text.is_empty() {
                    return Err("usage: /broadcast <message>".to_string());
                }
                Ok(SlashDispatchCommand::BroadcastText { text })
            }
            "/reputation" => Ok(SlashDispatchCommand::ReputationStatus),
            "/autopilot" => {
                let goal = parts[1..].join(" ");
                if goal.is_empty() {
                    return Err("usage: /autopilot <goal>".to_string());
                }
                Ok(SlashDispatchCommand::AutopilotSession {
                    goal,
                    agents: ExternalAgentKind::all().to_vec(),
                })
            }
            "/collab" => {
                // Format: /collab agy claude codex -- <prompt>
                let mut agents = Vec::new();
                let mut prompt_idx = None;

                for (idx, part) in parts.iter().enumerate().skip(1) {
                    if *part == "--" {
                        prompt_idx = Some(idx + 1);
                        break;
                    }
                    if let Some(agent) = ExternalAgentKind::parse(part) {
                        agents.push(agent);
                    } else {
                        return Err(format!("unknown agent '{part}' in /collab specification"));
                    }
                }

                if agents.is_empty() {
                    // Default to primary trio
                    agents.push(ExternalAgentKind::Agy);
                    agents.push(ExternalAgentKind::ClaudeCode);
                    agents.push(ExternalAgentKind::CodexCli);
                }

                let prompt = if let Some(idx) = prompt_idx {
                    parts[idx..].join(" ")
                } else {
                    return Err("usage: /collab <agent1> <agent2> ... -- <prompt>".to_string());
                };

                if prompt.is_empty() {
                    return Err("prompt after '--' cannot be empty".to_string());
                }

                Ok(SlashDispatchCommand::CollaborativeSwarm { agents, prompt })
            }
            agent_cmd => {
                if let Some(agent) = ExternalAgentKind::parse(agent_cmd) {
                    let prompt = parts[1..].join(" ");
                    if prompt.is_empty() {
                        return Err(format!("usage: {agent_cmd} <prompt>"));
                    }
                    Ok(SlashDispatchCommand::SingleAgent { agent, prompt })
                } else {
                    Err(format!("unrecognized slash command '{agent_cmd}'"))
                }
            }
        }
    }

    fn spawn_pane(
        &mut self,
        agent: ExternalAgentKind,
        title: String,
        prompt: &str,
        ipc: &mut Vec<String>,
        spawned: &mut Vec<ExternalAgentKind>,
    ) -> u32 {
        let pane_id = self.next_pane_id;
        self.next_pane_id += 1;
        let session_id = format!("{}_{}", agent.identifier(), pane_id);
        let escaped_prompt = prompt.replace('"', "\\\"");
        let binary = agent.default_binary_name();

        ipc.push(format!(
            "pexer.exe split right --title \"{title}\" \"{binary} \\\"{escaped_prompt}\\\"\""
        ));

        self.sessions.insert(
            session_id.clone(),
            AgentSession {
                session_id,
                agent,
                pane_id,
                title,
                state: AgentSessionState::Running {
                    active_task: prompt.to_string(),
                    started_epoch_ms: 0,
                },
                recent_output_tail: vec![format!("dispatched task: {prompt}")],
            },
        );

        spawned.push(agent);
        pane_id
    }

    /// Dispatch a slash command, producing Pexer CLI IPC commands and updating session states.
    pub fn dispatch(&mut self, line: &str) -> Result<CollabDispatchResult, String> {
        let parsed = Self::parse_slash_command(line)?;
        let mut ipc = Vec::new();
        let mut spawned = Vec::new();

        let explanation = match parsed {
            SlashDispatchCommand::SingleAgent { agent, prompt } => {
                let title = format!("[Agent:{}]", agent.display_name());
                let pane_id = self.spawn_pane(agent, title, &prompt, &mut ipc, &mut spawned);
                format!(
                    "Spawned terminal pane {pane_id} for {}",
                    agent.display_name()
                )
            }
            SlashDispatchCommand::CollaborativeSwarm { agents, prompt } => {
                for agent in &agents {
                    let title = format!("[Swarm:{}]", agent.identifier().to_ascii_uppercase());
                    self.spawn_pane(*agent, title, &prompt, &mut ipc, &mut spawned);
                }
                format!(
                    "Spawned {} collaborative agent panes for parallel swarm task",
                    agents.len()
                )
            }
            SlashDispatchCommand::ListPanes => {
                ipc.push("pexer.exe pane list".to_string());
                format!("Listing {} active agent sessions", self.sessions.len())
            }
            SlashDispatchCommand::ZoomPane { pane_id } => {
                ipc.push(format!("pexer.exe focus {pane_id}"));
                ipc.push("pexer.exe zoom".to_string());
                format!("Zoomed active pane {pane_id}")
            }
            SlashDispatchCommand::BroadcastText { text } => {
                let escaped = text.replace('"', "\\\"");
                for s in self.sessions.values() {
                    ipc.push(format!("pexer.exe send-keys {} \"{escaped}\"", s.pane_id));
                }
                format!(
                    "Broadcasted input to {} active agent terminal panes",
                    self.sessions.len()
                )
            }
            SlashDispatchCommand::ReputationStatus => {
                "Inspecting multi-agent reputation standings (Theorem 46)".to_string()
            }
            SlashDispatchCommand::AutopilotSession { ref goal, ref agents } => {
                for agent in agents {
                    let title = format!("[Autopilot:{}]", agent.identifier().to_ascii_uppercase());
                    self.spawn_pane(*agent, title, goal, &mut ipc, &mut spawned);
                }
                format!(
                    "Dispatched autopilot session across {} agents for goal: {goal}",
                    agents.len()
                )
            }
        };

        Ok(CollabDispatchResult {
            command_line: line.to_string(),
            spawned_agents: spawned,
            pexer_ipc_commands: ipc,
            explanation,
        })
    }

    /// Check if a terminal output line contains an agent permission prompt requiring user approval.
    pub fn detect_permission_prompt(line: &str) -> Option<(&'static str, Option<&'static str>)> {
        let lower = line.to_ascii_lowercase();
        const PROMPTS: &[(&[&str], &str, &str)] = &[
            (
                &["allow command?", "allow execution"],
                "allow command?",
                "press Enter or 'y' to confirm",
            ),
            (
                &["waiting for permission", "requesting permission"],
                "waiting for permission",
                "authorize tool call in terminal",
            ),
            (
                &["enter to confirm", "enter to select"],
                "enter to confirm",
                "press Enter to accept proposal",
            ),
            (
                &["cline needs permission"],
                "Cline needs permission",
                "grant permission to Cline",
            ),
            (
                &["hermes needs your"],
                "Hermes needs permission",
                "grant permission to Hermes",
            ),
            (
                &["usage of agy"],
                "Antigravity prompt",
                "enter agy command parameters",
            ),
        ];

        for &(patterns, prompt, action) in PROMPTS {
            if patterns.iter().any(|&p| lower.contains(p)) {
                return Some((prompt, Some(action)));
            }
        }
        None
    }

    pub fn active_sessions(&self) -> &BTreeMap<String, AgentSession> {
        &self.sessions
    }
}
