//! Tests for Mission-Control Cockpit (9/12 panels) and Multi-Agent Collaboration Orchestrator.
//!
//! Grounded in Theorems 41 and 42.
//!
//! # Safety Invariant
//! Strictly `#![forbid(unsafe_code)]` enforced.

#![forbid(unsafe_code)]

use pandora_runtime::cockpit::{CockpitController, CockpitLayout};
use pandora_runtime::multi_agent_collab::{
    ExternalAgentKind, MultiAgentCollabManager, SlashDispatchCommand,
};

#[test]
fn test_cockpit_layout_panels() {
    let c9 = CockpitController::new(CockpitLayout::NinePanel);
    let snap9 = c9.capture_snapshot(1_700_000_000_000);
    assert_eq!(snap9.panel_count, 9);
    assert_eq!(snap9.panels.len(), 9);
    assert!(snap9.verify_digest());
    assert_eq!(snap9.healthy_panels, 9);

    let c12 = CockpitController::new(CockpitLayout::TwelvePanel);
    let snap12 = c12.capture_snapshot(1_700_000_000_000);
    assert_eq!(snap12.panel_count, 12);
    assert_eq!(snap12.panels.len(), 12);
    assert!(snap12.verify_digest());
    assert_eq!(snap12.healthy_panels, 12);
}

#[test]
fn test_cockpit_ascii_grid_rendering() {
    let c12 = CockpitController::new(CockpitLayout::TwelvePanel);
    let snap12 = c12.capture_snapshot(1_700_000_000_000);
    let rendered = c12.render_ascii_grid(&snap12);

    assert!(rendered.contains("AGENT-PANDORA MISSION-CONTROL COCKPIT"));
    assert!(rendered.contains("12-PANEL (3x4)"));
    assert!(rendered.contains("[01:Daemon-Guard]"));
    assert!(rendered.contains("[02:Assurance-12]"));
    assert!(rendered.contains("[03:Genes-Evolution]"));
    assert!(rendered.contains("[04:Harness-Bench]"));
    assert!(rendered.contains("[05:Config-Settings]"));
    assert!(rendered.contains("[06:Swarm-MFG-BFT]"));
    assert!(rendered.contains("[07:Tropical-DAG]"));
    assert!(rendered.contains("[08:Bandit-Router]"));
    assert!(rendered.contains("[09:Drift-Conformal]"));
    assert!(rendered.contains("[10:Privacy-ZK]"));
    assert!(rendered.contains("[11:Merkle-Audit]"));
    assert!(rendered.contains("[12:Command-Deck]"));
}

#[test]
fn test_cockpit_pexer_ipc_generation() {
    let c12 = CockpitController::new(CockpitLayout::TwelvePanel);
    let cmds = c12.generate_pexer_commands("PANDORA-COCKPIT", Some("pexer.exe"));

    assert!(!cmds.is_empty());
    assert_eq!(cmds[0], "pexer.exe workspace new PANDORA-COCKPIT");
    assert!(cmds[1].contains("pane rename 0"));
    assert!(
        cmds.iter()
            .any(|c| c.contains("split right") || c.contains("split down"))
    );
    assert_eq!(*cmds.last().unwrap(), "pexer.exe focus 0");
}

#[test]
fn test_multi_agent_slash_command_parsing() {
    // Single agent commands
    let cmd_agy =
        MultiAgentCollabManager::parse_slash_command("/agy refactor authentication module")
            .expect("parse agy");
    assert_eq!(
        cmd_agy,
        SlashDispatchCommand::SingleAgent {
            agent: ExternalAgentKind::Agy,
            prompt: "refactor authentication module".to_string(),
        }
    );

    let cmd_claude = MultiAgentCollabManager::parse_slash_command("/claude run unit tests")
        .expect("parse claude");
    assert_eq!(
        cmd_claude,
        SlashDispatchCommand::SingleAgent {
            agent: ExternalAgentKind::ClaudeCode,
            prompt: "run unit tests".to_string(),
        }
    );

    let cmd_codex = MultiAgentCollabManager::parse_slash_command("/codex implement BFT consensus")
        .expect("parse codex");
    assert_eq!(
        cmd_codex,
        SlashDispatchCommand::SingleAgent {
            agent: ExternalAgentKind::CodexCli,
            prompt: "implement BFT consensus".to_string(),
        }
    );

    let cmd_gemini =
        MultiAgentCollabManager::parse_slash_command("/gemini check security advisories")
            .expect("parse gemini");
    assert_eq!(
        cmd_gemini,
        SlashDispatchCommand::SingleAgent {
            agent: ExternalAgentKind::GeminiCli,
            prompt: "check security advisories".to_string(),
        }
    );

    // Multi-agent collaboration command
    let cmd_collab = MultiAgentCollabManager::parse_slash_command(
        "/collab agy claude codex gemini -- audit cryptographic hash chains",
    )
    .expect("parse collab");

    match cmd_collab {
        SlashDispatchCommand::CollaborativeSwarm { agents, prompt } => {
            assert_eq!(agents.len(), 4);
            assert_eq!(agents[0], ExternalAgentKind::Agy);
            assert_eq!(agents[1], ExternalAgentKind::ClaudeCode);
            assert_eq!(agents[2], ExternalAgentKind::CodexCli);
            assert_eq!(agents[3], ExternalAgentKind::GeminiCli);
            assert_eq!(prompt, "audit cryptographic hash chains");
        }
        _ => panic!("expected CollaborativeSwarm"),
    }
}

#[test]
fn test_multi_agent_dispatch_and_permission_detection() {
    let mut manager = MultiAgentCollabManager::new();
    let res = manager
        .dispatch("/collab agy claude -- verify zero unsafe code")
        .expect("dispatch collab");

    assert_eq!(res.spawned_agents.len(), 2);
    assert_eq!(res.pexer_ipc_commands.len(), 2);
    assert_eq!(manager.active_sessions().len(), 2);

    // Test permission prompt heuristics
    let p1 = MultiAgentCollabManager::detect_permission_prompt("allow command? [y/N]");
    assert!(p1.is_some());
    assert_eq!(p1.unwrap().0, "allow command?");

    let p2 =
        MultiAgentCollabManager::detect_permission_prompt("Cline needs permission to edit file");
    assert!(p2.is_some());
    assert_eq!(p2.unwrap().0, "Cline needs permission");

    let p3 =
        MultiAgentCollabManager::detect_permission_prompt("Waiting for permission from host...");
    assert!(p3.is_some());

    let p_none = MultiAgentCollabManager::detect_permission_prompt("Normal stdout compilation log");
    assert!(p_none.is_none());
}
