#![forbid(unsafe_code)]

use pandora_runtime::cockpit::CockpitLayout;
use pandora_runtime::pexer_ipc::PexerSessionLauncher;

#[test]
fn test_pexer_probe_and_spec_build() {
    let launcher = PexerSessionLauncher::default();
    let spec = launcher.build_spec("test-workspace-9", CockpitLayout::NinePanel);

    assert_eq!(spec.layout, CockpitLayout::NinePanel);
    assert_eq!(spec.pane_commands.len(), 9);
    assert_eq!(spec.workspace_name, "test-workspace-9");

    let spec12 = launcher.build_spec("test-workspace-12", CockpitLayout::TwelvePanel);
    assert_eq!(spec12.layout, CockpitLayout::TwelvePanel);
    assert_eq!(spec12.pane_commands.len(), 12);
}

#[test]
fn test_pexer_ipc_commands_generation() {
    let launcher = PexerSessionLauncher::default();
    let spec = launcher.build_spec("test-grid", CockpitLayout::NinePanel);
    let commands = launcher.synthesize_ipc_commands(&spec);

    // 9 panels: commands include new-session, splits, renames, and commands
    assert!(commands.len() >= 9);

    let ps1 = launcher.generate_powershell_script(&commands);
    let bat = launcher.generate_batch_script(&commands);

    assert!(ps1.contains("Auto-generated Pexer IPC Launch Script for AGENT-PANDORA"));
    assert!(bat.contains("@echo off"));
}

#[test]
fn test_pexer_dry_run_launch() {
    let launcher = PexerSessionLauncher::default();
    let outcome = launcher.launch("dry-run-ws", CockpitLayout::NinePanel, true);

    assert!(outcome.dry_run);
    assert!(outcome.success);
    assert_eq!(outcome.total_panes, 9);
    assert!(!outcome.commands_executed.is_empty());
    assert!(!outcome.automation_script_powershell.is_empty());
    assert!(!outcome.automation_script_batch.is_empty());
}
