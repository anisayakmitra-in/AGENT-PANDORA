//! Mission-Control Cockpit CLI Command Handler (Phases 21-22).
//!
//! Grounded in Theorems 41-44 for multi-agent swarm state consistency,
//! bounded orchestration latency, Byzantine code consensus, and non-blocking Pexer IPC.
//!
//! # Safety Invariant
//! Strictly `#![forbid(unsafe_code)]` enforced workspace-wide.

#![forbid(unsafe_code)]

use super::parse_options;
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::autopilot::{AutopilotSessionConfig, MultiAgentAutopilotOrchestrator};
use pandora_runtime::cockpit::{CockpitController, CockpitLayout};
use pandora_runtime::cockpit_web::{CockpitWebConfig, CockpitWebServer, DEFAULT_SSE_TICK_MS};
use pandora_runtime::federated_policy::FederatedPolicyEngine;
use pandora_runtime::multi_agent_collab::{ExternalAgentKind, MultiAgentCollabManager};
use pandora_runtime::multi_agent_consensus::{
    AgentPatchProposal, ProposalScoreVector, SwarmCodeConsensusEngine,
};
use pandora_runtime::pexer_ipc::PexerSessionLauncher;
use serde_json::json;
use std::collections::BTreeMap;
use std::io::IsTerminal;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const COCKPIT_OPTIONS: &[&str] = &[
    "panels",
    "pexer-bin",
    "workspace",
    "dry-run",
    "config",
    "data-dir",
    "agents",
    "prompt",
    "goal",
    "pane-id",
    "keys",
    "threshold",
    "divergence",
    "agent",
    "record-honest",
    "record-byzantine",
    "quality",
    "reset",
    "port",
    "bind",
    "foreground",
    "duration-ms",
    "json",
];

/// Execute the mission-control cockpit commands.
pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().map(String::as_str);
    match subcommand {
        Some("status") => status(&args[1..]),
        Some("tui") | Some("interactive") => tui(&args[1..]),
        Some("launch") | Some("up") => launch(&args[1..]),
        Some("consensus") | Some("vote") => consensus(&args[1..]),
        Some("dispatch") => dispatch(&args[1..]),
        Some("panes") => panes(&args[1..]),
        Some("send") => send(&args[1..]),
        Some("reputation") => reputation(&args[1..]),
        Some("web") | Some("serve") => web(&args[1..]),
        Some("autopilot") => autopilot(&args[1..]),
        Some(unknown) if !unknown.starts_with('-') => Err(CliError::usage(format!(
            "unknown cockpit command '{unknown}'; expected 'status', 'tui', 'launch', 'consensus', 'dispatch', 'panes', 'send', 'reputation', 'web', or 'autopilot'"
        ))),
        _ => status(args),
    }
}

pub(crate) fn status(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, COCKPIT_OPTIONS)?;
    let layout_str = parsed.value("panels").unwrap_or("12");
    let layout = CockpitLayout::parse(layout_str).ok_or_else(|| {
        CliError::usage(format!(
            "invalid --panels value '{layout_str}'; expected '9' or '12'"
        ))
    })?;

    let controller = CockpitController::new(layout);
    let epoch_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let snapshot = controller.capture_snapshot(epoch_ms);
    let ascii_grid = controller.render_ascii_grid(&snapshot);

    let mut data = serde_json::to_value(&snapshot)
        .map_err(|e| CliError::usage(format!("serialization failed: {e}")))?;
    if let Some(obj) = data.as_object_mut() {
        obj.insert("status".to_string(), json!("ok"));
    }

    Ok(success("cockpit status", data, ascii_grid))
}

pub(crate) fn tui(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, COCKPIT_OPTIONS)?;
    let layout_str = parsed.value("panels").unwrap_or("12");
    let layout = CockpitLayout::parse(layout_str).unwrap_or(CockpitLayout::TwelvePanel);

    super::cockpit_tui::run_interactive(layout)
}

pub(crate) fn launch(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, COCKPIT_OPTIONS)?;
    let layout_str = parsed.value("panels").unwrap_or("12");
    let layout = CockpitLayout::parse(layout_str).ok_or_else(|| {
        CliError::usage(format!(
            "invalid --panels value '{layout_str}'; expected '9' or '12'"
        ))
    })?;

    let workspace_name = parsed.value("workspace").unwrap_or("PANDORA-COCKPIT");
    let pexer_bin = parsed.value("pexer-bin");
    let dry_run = parsed.has_flag("dry-run");

    let launcher = PexerSessionLauncher::new(pexer_bin);
    let outcome = launcher.launch(workspace_name, layout, dry_run);

    let mut human = format!(
        "Pexer Workspace Orchestration: {}\n  Workspace: {}\n  Layout: {:?} ({} panes)\n  Executable: {} (found: {})\n\nCommands generated:\n",
        if outcome.dry_run {
            "Dry-run verified"
        } else if outcome.success {
            "Launched successfully"
        } else {
            "Failed"
        },
        outcome.workspace_name,
        layout,
        outcome.total_panes,
        outcome.executable_resolved,
        outcome.executable_exists,
    );

    for (idx, cmd) in outcome.commands_executed.iter().enumerate() {
        human.push_str(&format!("  [{idx}] {cmd}\n"));
    }

    Ok(success(
        "cockpit launch",
        json!({
            "status": if outcome.success { "launched" } else { "failed" },
            "dry_run": outcome.dry_run,
            "workspace": outcome.workspace_name,
            "panel_count": outcome.total_panes,
            "executable": outcome.executable_resolved,
            "executable_exists": outcome.executable_exists,
            "commands": outcome.commands_executed,
            "message": outcome.message,
        }),
        human,
    ))
}

pub(crate) fn consensus(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, COCKPIT_OPTIONS)?;
    let threshold: f64 = parsed
        .value("threshold")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.50);
    let divergence: f64 = parsed
        .value("divergence")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.45);

    // Default heterogeneous proposal set for swarm consensus demonstration
    let proposals = vec![
        AgentPatchProposal::new(
            "p_agy",
            ExternalAgentKind::Agy,
            "Refactor auth token hygiene & zeroize",
            "Applies constant-time comparison and memory zeroization",
            "diff --git a/src/auth.rs\n+ subtle::ConstantTimeEq\n+ zeroize::Zeroize",
            ProposalScoreVector::new(1.0, 0.98, 0.94, 0.99),
            142,
        ),
        AgentPatchProposal::new(
            "p_claude",
            ExternalAgentKind::ClaudeCode,
            "Refactor auth with zeroize and constant-time check",
            "Guarantees side-channel resistance and zeroization on drop",
            "diff --git a/src/auth.rs\n+ subtle::ConstantTimeEq",
            ProposalScoreVector::new(1.0, 0.97, 0.91, 0.99),
            185,
        ),
        AgentPatchProposal::new(
            "p_codex",
            ExternalAgentKind::CodexCli,
            "Add auth token verification cache",
            "Implements bounded cache for token checks",
            "diff --git a/src/auth.rs\n+ token_cache.insert()",
            ProposalScoreVector::new(0.96, 0.94, 0.89, 0.96),
            220,
        ),
        AgentPatchProposal::new(
            "p_byzantine_hallucinated",
            ExternalAgentKind::Hermes,
            "Disable token checks for performance",
            "Bypasses invariant assertion and removes verification logic",
            "diff --git a/src/auth.rs\n- verify_token(token)?",
            ProposalScoreVector::new(0.20, 0.15, 0.40, 0.05),
            88,
        ),
    ];

    let engine = SwarmCodeConsensusEngine::new(threshold, divergence);
    let epoch_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let decision = engine
        .resolve_consensus(&proposals, epoch_ms)
        .map_err(|e| {
            CliError::execution(
                format!("consensus resolution error: {e}"),
                serde_json::json!({}),
            )
        })?;

    let mut human = format!(
        "Multi-Agent Swarm Byzantine Consensus (Theorem 43):\n  Winning Proposal: {} [{}]\n  Winning Agent: {}\n  Centroid Quality Score: {:.3}\n  Consensus Confidence: {:.1}%\n  Quorum: {}/{} honest proposals accepted ({} Byzantine filtered)\n  Receipt Hash: {}\n\nProposal Verdicts:\n",
        decision.winning_proposal_id,
        decision.winning_title,
        decision.winning_agent.display_name(),
        decision.consensus_centroid.composite_score(),
        decision.consensus_confidence * 100.0,
        decision.honest_quorum_size,
        decision.total_proposals,
        decision.byzantine_filtered_count,
        decision.consensus_hash,
    );

    for (id, verdict) in &decision.proposal_verdicts {
        human.push_str(&format!("  * {id}: {verdict:?}\n"));
    }

    Ok(success(
        "cockpit consensus",
        json!({
            "status": "consensus_achieved",
            "winning_proposal_id": decision.winning_proposal_id,
            "winning_agent": decision.winning_agent.identifier(),
            "winning_title": decision.winning_title,
            "consensus_confidence": decision.consensus_confidence,
            "total_proposals": decision.total_proposals,
            "honest_quorum_size": decision.honest_quorum_size,
            "byzantine_filtered_count": decision.byzantine_filtered_count,
            "proposal_verdicts": decision.proposal_verdicts,
            "consensus_hash": decision.consensus_hash,
            "timestamp_ms": decision.timestamp_ms,
        }),
        human,
    ))
}

pub(crate) fn dispatch(args: &[String]) -> Result<CommandResult, CliError> {
    if args.is_empty() {
        return Err(CliError::usage(
            "usage: pandora cockpit dispatch <slash_command...> (e.g. /collab agy claude -- test)",
        ));
    }

    let line = args.join(" ");
    let mut manager = MultiAgentCollabManager::new();
    let res = manager
        .dispatch(&line)
        .map_err(|e| CliError::usage(format!("dispatch failed: {e}")))?;

    let mut human = format!("Multi-Agent Slash Dispatch: {}\n", res.explanation);
    for cmd in &res.pexer_ipc_commands {
        human.push_str(&format!("  -> IPC: {cmd}\n"));
    }

    Ok(success(
        "cockpit dispatch",
        json!({
            "status": "dispatched",
            "command_line": res.command_line,
            "spawned_agents": res.spawned_agents,
            "pexer_commands": res.pexer_ipc_commands,
            "explanation": res.explanation,
        }),
        human,
    ))
}

pub(crate) fn panes(args: &[String]) -> Result<CommandResult, CliError> {
    let _parsed = parse_options(args, COCKPIT_OPTIONS)?;
    let manager = MultiAgentCollabManager::new();

    let mut human = format!(
        "Active Multi-Agent Multiplexer Panes ({} active):\n",
        manager.active_sessions().len()
    );
    if manager.active_sessions().is_empty() {
        human.push_str(
            "  (no active external agent panes; use '/collab' or '/agy <prompt>' to dispatch)\n",
        );
    } else {
        for s in manager.active_sessions().values() {
            human.push_str(&format!(
                "  [{}] pane_id={} title='{}'\n",
                s.session_id, s.pane_id, s.title
            ));
        }
    }

    Ok(success(
        "cockpit panes",
        json!({
            "status": "ok",
            "active_pane_count": manager.active_sessions().len(),
            "panes": manager.active_sessions(),
        }),
        human,
    ))
}

pub(crate) fn send(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, COCKPIT_OPTIONS)?;
    let pane_id = parsed.value("pane-id").unwrap_or("0");
    let keys = parsed.value("keys").unwrap_or("");
    let pexer_bin = parsed.value("pexer-bin").unwrap_or("pexer.exe");

    let escaped = keys.replace('"', "\\\"");
    let cmd = format!("{pexer_bin} send-keys {pane_id} \"{escaped}\"");

    Ok(success(
        "cockpit send",
        json!({
            "status": "keys_dispatched",
            "pane_id": pane_id,
            "keys": keys,
            "ipc_command": cmd,
        }),
        format!("Dispatched keystrokes to pane {pane_id} via Pexer IPC:\n  {cmd}"),
    ))
}

pub(crate) fn reputation(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, COCKPIT_OPTIONS)?;
    let mut engine = FederatedPolicyEngine::default();

    if let Some(target_str) = parsed.value("record-honest") {
        let agent = ExternalAgentKind::parse(target_str).ok_or_else(|| {
            CliError::usage(format!("unknown agent '{target_str}' in --record-honest"))
        })?;
        let quality: f64 = parsed
            .value("quality")
            .map(|s| {
                s.parse()
                    .map_err(|_| CliError::usage("--quality must be a float in [0.1, 1.0]"))
            })
            .transpose()?
            .unwrap_or(0.95);
        engine.update_agent_reputation(agent, true, quality);
    }

    if let Some(target_str) = parsed.value("record-byzantine") {
        let agent = ExternalAgentKind::parse(target_str).ok_or_else(|| {
            CliError::usage(format!(
                "unknown agent '{target_str}' in --record-byzantine"
            ))
        })?;
        engine.update_agent_reputation(agent, false, 0.0);
    }

    let filter_agent = parsed
        .value("agent")
        .map(|s| {
            let agent_str = if s.is_empty() {
                parsed
                    .positionals
                    .first()
                    .map(|p| p.as_str())
                    .ok_or_else(|| CliError::usage("--agent requires an agent name"))?
            } else {
                s
            };
            ExternalAgentKind::parse(agent_str).ok_or_else(|| {
                CliError::usage(format!("unknown agent '{agent_str}' in --agent filter"))
            })
        })
        .transpose()?;

    let agents_to_show: Vec<ExternalAgentKind> = if let Some(a) = filter_agent {
        vec![a]
    } else {
        ExternalAgentKind::all().to_vec()
    };

    let mut table_rows = Vec::new();
    let mut json_records = BTreeMap::new();

    for &agent in &agents_to_show {
        if let Some(rec) = engine.get_reputation(agent) {
            let status = if rec.is_quarantined {
                "QUARANTINED [FLAGGED]"
            } else {
                "ACTIVE"
            };
            table_rows.push(format!(
                "  {:<12} | {:>5.2} | {:>5.2} | {:>5} | {:>7.3} | {:>6.3} | {:<20}",
                agent.identifier(),
                rec.alpha,
                rec.beta,
                rec.total_evaluations,
                rec.expected_reliability(),
                rec.consensus_weight,
                status
            ));
            json_records.insert(
                agent.identifier().to_string(),
                json!({
                    "agent": agent.identifier(),
                    "display_name": agent.display_name(),
                    "alpha": rec.alpha,
                    "beta": rec.beta,
                    "total_evaluations": rec.total_evaluations,
                    "expected_reliability": rec.expected_reliability(),
                    "variance": rec.variance(),
                    "consensus_weight": rec.consensus_weight,
                    "is_quarantined": rec.is_quarantined,
                }),
            );
        }
    }

    let human = format!(
        "Multi-Agent Dynamic Reputation & Quarantine State (Theorem 46):\n\
         --------------------------------------------------------------------------------------\n\
           {:<12} | {:>5} | {:>5} | {:>5} | {:>7} | {:>6} | {:<20}\n\
         --------------------------------------------------------------------------------------\n\
         {}\n\
         --------------------------------------------------------------------------------------\n\
         Total Agents Tracked: {} | Quarantine Threshold: 0.25 | Min Evals: 3",
        "AGENT",
        "ALPHA",
        "BETA",
        "EVALS",
        "RELIAB",
        "WEIGHT",
        "STATUS",
        table_rows.join("\n"),
        agents_to_show.len()
    );

    Ok(success(
        "cockpit reputation",
        json!({
            "status": "ok",
            "agents": json_records,
            "total_agents": agents_to_show.len(),
            "quarantine_threshold": 0.25,
            "min_evals_for_quarantine": 3,
        }),
        human,
    ))
}

pub(crate) fn web(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, COCKPIT_OPTIONS)?;
    let port: u16 = parsed
        .value("port")
        .map(|s| {
            s.parse::<u16>()
                .map_err(|_| CliError::usage(format!("invalid --port value '{s}'")))
        })
        .transpose()?
        .unwrap_or(8080);

    let bind_addr = parsed.value("bind").unwrap_or("0.0.0.0").to_string();
    let layout_str = parsed.value("panels").unwrap_or("12");
    let layout = CockpitLayout::parse(layout_str).ok_or_else(|| {
        CliError::usage(format!(
            "invalid --panels value '{layout_str}'; expected '9' or '12'"
        ))
    })?;

    let duration_ms: Option<u64> = parsed
        .value("duration-ms")
        .map(|s| {
            s.parse::<u64>()
                .map_err(|_| CliError::usage(format!("invalid --duration-ms value '{s}'")))
        })
        .transpose()?;

    let config = CockpitWebConfig {
        bind_addr: bind_addr.clone(),
        port,
        initial_layout: layout,
        sse_tick_interval_ms: DEFAULT_SSE_TICK_MS,
    };

    let server = CockpitWebServer::with_config(config, None).map_err(|e| {
        CliError::execution(
            format!("failed to bind cockpit web server on {bind_addr}:{port}: {e}"),
            json!({"bind": bind_addr, "port": port}),
        )
    })?;

    let bound_port = server.port();
    let url = format!("http://{bind_addr}:{bound_port}/");
    let layout_name = match layout {
        CockpitLayout::NinePanel => "nine_panel",
        CockpitLayout::TwelvePanel => "twelve_panel",
    };

    let dry_run = parsed.has_flag("dry-run");
    let foreground = parsed.has_flag("foreground");

    if let Some(ms) = duration_ms {
        let shutdown_handle = server.shutdown_handle();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(ms));
            shutdown_handle.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        server.run().map_err(|e| {
            CliError::execution(
                format!("cockpit web server error: {e}"),
                json!({"url": url}),
            )
        })?;
    } else if !dry_run && (foreground || std::io::stdout().is_terminal()) {
        server.run().map_err(|e| {
            CliError::execution(
                format!("cockpit web server error: {e}"),
                json!({"url": url}),
            )
        })?;
    } else {
        server.shutdown();
    }

    let human = format!(
        "AGENT-PANDORA Remote Telemetry Dashboard Micro-Server (R2):\n  URL: {url}\n  Bind Address: {bind_addr}:{bound_port}\n  Layout: {layout_name} ({} panels)\n  SSE Stream: {url}api/events\n  Snapshot API: {url}api/snapshot\n  Viewport Calibration: Xiaomi 15 (~393x851) & Desktop Responsive",
        layout.panel_count()
    );

    Ok(success(
        "cockpit web",
        json!({
            "status": "ok",
            "url": url,
            "bind": bind_addr,
            "port": bound_port,
            "layout": layout_name,
            "panel_count": layout.panel_count(),
            "sse_endpoint": "/api/events",
            "snapshot_endpoint": "/api/snapshot",
        }),
        human,
    ))
}

pub(crate) fn autopilot(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, COCKPIT_OPTIONS)?;

    let goal_str = if let Some(g) = parsed.value("goal").or_else(|| parsed.value("prompt")) {
        g.trim().to_string()
    } else if !parsed.positionals.is_empty() {
        parsed.positionals.join(" ").trim().to_string()
    } else if parsed.has_flag("dry-run") {
        "Enforce zero-unsafe invariants and optimize subsystem telemetry".to_string()
    } else {
        return Err(CliError::usage(
            "autopilot requires a non-empty task goal (pass --goal <text> or positional prompt)",
        ));
    };

    if goal_str.is_empty() {
        return Err(CliError::usage("autopilot goal cannot be empty"));
    }

    let candidate_agents = if let Some(agents_str) = parsed.value("agents") {
        let mut list = Vec::new();
        for token in agents_str
            .split([',', ' '])
            .filter(|s| !s.trim().is_empty())
        {
            let kind = ExternalAgentKind::parse(token)
                .ok_or_else(|| CliError::usage(format!("unknown agent '{token}' in --agents")))?;
            if !list.contains(&kind) {
                list.push(kind);
            }
        }
        if list.is_empty() {
            return Err(CliError::usage("--agents list cannot be empty"));
        }
        list
    } else {
        ExternalAgentKind::all().to_vec()
    };

    let threshold: f64 = parsed
        .value("threshold")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.50);
    let divergence: f64 = parsed
        .value("divergence")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.45);
    let dry_run = parsed.has_flag("dry-run");
    let pexer_bin = parsed.value("pexer-bin").map(ToOwned::to_owned);

    let config = AutopilotSessionConfig {
        task_goal: goal_str,
        candidate_agents,
        pexer_bin,
        dry_run,
        threshold,
        divergence,
    };

    let epoch_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let outcome = MultiAgentAutopilotOrchestrator::run_session(config, epoch_ms).map_err(|e| {
        CliError::execution(
            format!("autopilot session failed: {e}"),
            serde_json::json!({}),
        )
    })?;

    let mut human = format!(
        "Live Interactive Multi-Agent Autopilot Session (Theorems 43 & 46):\n  Goal: {}\n  Participating Agents: {} ({:?})\n  Winning Proposal: {} [{}]\n  Winning Agent: {}\n  Quorum: {}/{} honest proposals ({} Byzantine filtered)\n  Consensus Hash: {}\n  Patch Certificate: {}\n\nSubagent Role Partitioning:\n",
        outcome.task_goal,
        outcome.spawned_agents.len(),
        outcome
            .spawned_agents
            .iter()
            .map(ExternalAgentKind::identifier)
            .collect::<Vec<_>>(),
        outcome.consensus_decision.winning_proposal_id,
        outcome.consensus_decision.winning_title,
        outcome.consensus_decision.winning_agent.display_name(),
        outcome.consensus_decision.honest_quorum_size,
        outcome.consensus_decision.total_proposals,
        outcome.consensus_decision.byzantine_filtered_count,
        outcome.consensus_decision.consensus_hash,
        outcome.certified_patch.certificate_digest,
    );

    for (agent_id, role_desc) in &outcome.partitioned_roles {
        human.push_str(&format!("  * [{agent_id}]: {role_desc}\n"));
    }

    Ok(success(
        "cockpit autopilot",
        json!({
            "status": "certified",
            "success": outcome.success,
            "task_goal": outcome.task_goal,
            "spawned_agents": outcome.spawned_agents,
            "partitioned_roles": outcome.partitioned_roles,
            "pexer_ipc_commands": outcome.pexer_ipc_commands,
            "winning_proposal_id": outcome.consensus_decision.winning_proposal_id,
            "winning_agent": outcome.consensus_decision.winning_agent.identifier(),
            "winning_title": outcome.consensus_decision.winning_title,
            "consensus_confidence": outcome.consensus_decision.consensus_confidence,
            "total_proposals": outcome.consensus_decision.total_proposals,
            "honest_quorum_size": outcome.consensus_decision.honest_quorum_size,
            "byzantine_filtered_count": outcome.consensus_decision.byzantine_filtered_count,
            "consensus_hash": outcome.consensus_decision.consensus_hash,
            "patch_certificate": outcome.certified_patch.certificate_digest,
            "certified_patch": outcome.certified_patch,
            "updated_reputations": outcome.updated_reputations,
            "message": outcome.message,
        }),
        human,
    ))
}
