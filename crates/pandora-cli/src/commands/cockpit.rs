use super::parse_options;
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::cockpit::{CockpitController, CockpitLayout};
use pandora_runtime::multi_agent_collab::MultiAgentCollabManager;
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

const COCKPIT_OPTIONS: &[&str] = &[
    "panels",
    "pexer-bin",
    "workspace",
    "dry-run",
    "config",
    "data-dir",
];

/// Execute the mission-control cockpit commands.
pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().map(String::as_str);
    match subcommand {
        Some("status") => status(&args[1..]),
        Some("launch") | Some("up") => launch(&args[1..]),
        Some("dispatch") => dispatch(&args[1..]),
        Some("panes") => panes(&args[1..]),
        Some(unknown) if !unknown.starts_with('-') => Err(CliError::usage(format!(
            "unknown cockpit command '{unknown}'; expected 'status', 'launch', 'dispatch', or 'panes'"
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

pub(crate) fn launch(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, COCKPIT_OPTIONS)?;
    let layout_str = parsed.value("panels").unwrap_or("12");
    let layout = CockpitLayout::parse(layout_str).ok_or_else(|| {
        CliError::usage(format!(
            "invalid --panels value '{layout_str}'; expected '9' or '12'"
        ))
    })?;

    let workspace_name = parsed.value("workspace").unwrap_or("PANDORA-COCKPIT");
    let pexer_bin = parsed.value("pexer-bin").unwrap_or("pexer.exe");
    let dry_run = parsed.has_flag("dry-run");

    let controller = CockpitController::new(layout);
    let commands = controller.generate_pexer_commands(workspace_name, Some(pexer_bin));

    let mut human = String::new();
    if dry_run {
        human.push_str(&format!(
            "Dry-run mode: generated {} Pexer commands:\n",
            commands.len()
        ));
        for (idx, cmd) in commands.iter().enumerate() {
            human.push_str(&format!("  [{idx}] {cmd}\n"));
        }
    } else {
        human.push_str(&format!(
            "Launching {} in Pexer workspace '{}'...\n",
            match layout {
                CockpitLayout::NinePanel => "9-panel cockpit",
                CockpitLayout::TwelvePanel => "12-panel cockpit",
            },
            workspace_name
        ));
        human.push_str("Commands generated (run via Pexer CLI/IPC):\n");
        for (idx, cmd) in commands.iter().enumerate() {
            human.push_str(&format!("  [{idx}] {cmd}\n"));
        }
    }

    Ok(success(
        "cockpit launch",
        json!({
            "status": "launched",
            "workspace": workspace_name,
            "panel_count": layout.panel_count(),
            "commands": commands,
            "dry_run": dry_run,
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

    let active = manager.active_sessions().len();
    Ok(success(
        "cockpit panes",
        json!({
            "status": "ok",
            "active_panes_count": active,
        }),
        human,
    ))
}
