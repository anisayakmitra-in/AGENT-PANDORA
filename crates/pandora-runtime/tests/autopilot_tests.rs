//! Integration tests for the multi-agent autopilot orchestrator (R1).

#![forbid(unsafe_code)]

use pandora_runtime::autopilot::{
    AutopilotCoordinator, AutopilotError, AutopilotGoal, AutopilotSessionConfig,
    MultiAgentAutopilotOrchestrator,
};
use pandora_runtime::multi_agent_collab::ExternalAgentKind;

#[test]
fn full_session_certifies_patch_and_filters_byzantine() {
    let out =
        MultiAgentAutopilotOrchestrator::run_session(AutopilotSessionConfig::default(), 1_000)
            .expect("session");
    assert!(out.success);
    assert_eq!(out.spawned_agents.len(), 9);
    assert_eq!(out.partitioned_roles.len(), 9);
    assert_eq!(out.pexer_ipc_commands.len(), 9);
    assert!(out.consensus_decision.byzantine_filtered_count >= 1);
    assert!(
        out.certified_patch
            .certificate_digest
            .starts_with("PANDORA_PATCH_CERT_V1:")
    );
    assert_eq!(out.updated_reputations.len(), 9);
}

#[test]
fn reputation_reflects_outcomes() {
    let out =
        MultiAgentAutopilotOrchestrator::run_session(AutopilotSessionConfig::default(), 1).unwrap();
    let hermes = out.updated_reputations["hermes"];
    let agy = out.updated_reputations["agy"];
    assert!(agy > hermes);
}

#[test]
fn empty_goal_and_agents_rejected() {
    let cfg = AutopilotSessionConfig {
        task_goal: "  ".into(),
        ..Default::default()
    };
    assert!(MultiAgentAutopilotOrchestrator::run_session(cfg, 1).is_err());
    let cfg = AutopilotSessionConfig {
        candidate_agents: vec![],
        ..Default::default()
    };
    assert!(MultiAgentAutopilotOrchestrator::run_session(cfg, 1).is_err());
}

#[test]
fn coordinator_validates_and_runs() {
    let c = AutopilotCoordinator::default();
    let mut g = AutopilotGoal::new("t1", "   ");
    assert_eq!(
        c.execute_session(&g).unwrap_err(),
        AutopilotError::EmptyGoal
    );
    g.prompt = "harden parser".into();
    g.participating_agents.clear();
    assert_eq!(
        c.execute_session(&g).unwrap_err(),
        AutopilotError::NoParticipatingAgents
    );
    g.participating_agents = vec![
        ExternalAgentKind::Agy,
        ExternalAgentKind::ClaudeCode,
        ExternalAgentKind::CodexCli,
    ];
    let r = c.execute_session(&g).unwrap();
    assert_eq!(r.task_id, "t1");
    assert!(r.patch_certificate.starts_with("PANDORA_PATCH_CERT_V1:"));
}

#[test]
fn certificate_is_deterministic_per_inputs() {
    let a =
        MultiAgentAutopilotOrchestrator::run_session(AutopilotSessionConfig::default(), 5).unwrap();
    let b =
        MultiAgentAutopilotOrchestrator::run_session(AutopilotSessionConfig::default(), 5).unwrap();
    assert_eq!(
        a.certified_patch.certificate_digest,
        b.certified_patch.certificate_digest
    );
}
