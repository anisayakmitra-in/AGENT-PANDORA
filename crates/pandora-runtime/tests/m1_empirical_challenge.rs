#![forbid(unsafe_code)]

use pandora_harnesses::HarnessCatalog;
use pandora_runtime::executors::WorkspaceRoot;
use pandora_runtime::reference_monitor::{AuthorizationError, ReferenceMonitor};
use pandora_runtime::{ExecutionController, RunStatus};
use pandora_types::{
    Capability, EffectTarget, ExecutionId, ExecutionProfile, ExecutionProfileBinding,
    ExecutionProfileBindingKind, GeneId, GovernanceMode, Operation, OperationRequest,
    ParliamentDecision, PolicyContext, PrincipalId, ResourceScope, Session, SessionId, TaskIntent,
    Timestamp, hash_artifact,
};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU64 = AtomicU64::new(1);

fn temp_workspace() -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "pandora-m1-challenge-{}-{suffix}-{seq}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("sample.txt"), "original text\n").unwrap();
    path
}

fn profile() -> ExecutionProfile {
    ExecutionProfile::new(
        "2.0.0-alpha.6",
        std::env::consts::OS,
        std::env::consts::ARCH,
        1,
        "managed_root",
        hash_artifact(b"containment"),
        vec![
            ExecutionProfileBinding::new(
                ExecutionProfileBindingKind::Executor,
                "filesystem",
                Some("2.0.0-alpha.6"),
                hash_artifact(b"filesystem"),
            )
            .unwrap(),
        ],
    )
    .unwrap()
}

fn request_for(cap: Capability, op: Operation, session: &str) -> OperationRequest {
    OperationRequest::new(
        ExecutionId::new(format!(
            "execution-{}",
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ))
        .unwrap(),
        SessionId::new(session).unwrap(),
        PrincipalId::new("principal-1").unwrap(),
        profile(),
        GeneId::new("test.gene").unwrap(),
        None,
        cap,
        op,
        EffectTarget::path("sample.txt"),
        ResourceScope::workspace("workspace-1"),
    )
    .unwrap()
}

#[test]
fn challenge_reference_monitor_dynamic_policy_updates() {
    let mut monitor =
        ReferenceMonitor::new_with_policy(PolicyContext::for_mode(GovernanceMode::ReadOnly), 60);
    assert_eq!(monitor.governance_mode(), GovernanceMode::ReadOnly);

    let req_write = request_for(
        Capability::FilesystemWrite,
        Operation::Write,
        "session-test",
    );
    let req_read = request_for(Capability::FilesystemRead, Operation::Read, "session-test");
    let now = Timestamp::from_unix_seconds(100);

    // 1. In ReadOnly mode:
    let decision_read =
        pandora_runtime::parliament::Parliament::new(monitor.policy().policy_version())
            .decide(&req_read, monitor.policy());
    assert!(matches!(decision_read, ParliamentDecision::Allow { .. }));
    let permit = monitor.authorize(req_read.clone(), decision_read, now);
    assert!(permit.is_ok(), "ReadOnly mode must allow reading");

    let decision_write =
        pandora_runtime::parliament::Parliament::new(monitor.policy().policy_version())
            .decide(&req_write, monitor.policy());
    assert!(
        matches!(decision_write, ParliamentDecision::Deny { .. }),
        "ReadOnly mode must deny mutating operation"
    );
    let auth_res = monitor.authorize(req_write.clone(), decision_write, now);
    assert_eq!(
        auth_res,
        Err(AuthorizationError::Denied {
            reason: "capability is not enabled by policy".to_owned()
        })
    );

    // 2. Dynamic update to Autonomous mode
    monitor.update_policy(PolicyContext::for_mode(GovernanceMode::Autonomous));
    assert_eq!(monitor.governance_mode(), GovernanceMode::Autonomous);

    let decision_write_auto =
        pandora_runtime::parliament::Parliament::new(monitor.policy().policy_version())
            .decide(&req_write, monitor.policy());
    assert!(
        matches!(decision_write_auto, ParliamentDecision::Allow { .. }),
        "Autonomous mode must allow mutating operation directly"
    );
    let permit_auto = monitor.authorize(req_write.clone(), decision_write_auto, now);
    assert!(
        permit_auto.is_ok(),
        "Autonomous mode must issue permit without human approval"
    );

    // 3. Dynamic update to Supervised mode
    monitor.update_policy(PolicyContext::for_mode(GovernanceMode::Supervised));
    assert_eq!(monitor.governance_mode(), GovernanceMode::Supervised);

    let decision_write_sup =
        pandora_runtime::parliament::Parliament::new(monitor.policy().policy_version())
            .decide(&req_write, monitor.policy());
    assert!(
        matches!(
            decision_write_sup,
            ParliamentDecision::RequireApproval { .. }
        ),
        "Supervised mode must require human approval for mutating operation"
    );
    let permit_sup = monitor.authorize(req_write.clone(), decision_write_sup, now);
    assert!(
        matches!(permit_sup, Err(AuthorizationError::ApprovalRequired { .. })),
        "Supervised mode must fail authorization before approval evidence"
    );

    // 4. Dynamic update back to ReadOnly mode
    monitor.update_policy(PolicyContext::for_mode(GovernanceMode::ReadOnly));
    assert_eq!(monitor.governance_mode(), GovernanceMode::ReadOnly);

    let decision_write_ro2 =
        pandora_runtime::parliament::Parliament::new(monitor.policy().policy_version())
            .decide(&req_write, monitor.policy());
    assert!(
        matches!(decision_write_ro2, ParliamentDecision::Deny { .. }),
        "ReadOnly mode must deny write after hot-swap"
    );
}

#[test]
fn challenge_execution_controller_dynamic_policy_updates() {
    let ws_path = temp_workspace();
    let ws = WorkspaceRoot::new(&ws_path).unwrap();
    let mut controller = ExecutionController::with_policy_and_harnesses(
        ws,
        PolicyContext::for_mode(GovernanceMode::ReadOnly),
        HarnessCatalog::builtins(),
    );

    let session = Session::new(
        SessionId::new("session-ec-test").unwrap(),
        PrincipalId::new("principal-1").unwrap(),
        pandora_types::TenantId::new("tenant-1").unwrap(),
        pandora_types::WorkspaceId::new("workspace-1").unwrap(),
        Timestamp::from_unix_seconds(100),
    );
    let now = Timestamp::from_unix_seconds(100);

    // Read task in ReadOnly
    let read_intent = TaskIntent::new("read:sample.txt".to_owned()).unwrap();
    let summary_read = controller
        .run_at(read_intent, session.clone(), now)
        .unwrap();
    assert_eq!(summary_read.status(), &RunStatus::Completed);
    assert_eq!(
        String::from_utf8_lossy(summary_read.output().unwrap()).trim(),
        "original text"
    );

    // Mutating patch task in ReadOnly -> must be Denied
    let patch_intent = TaskIntent::new("patch:sample.txt:tampered_by_readonly".to_owned()).unwrap();
    let summary_patch_ro = controller
        .run_at(patch_intent.clone(), session.clone(), now)
        .unwrap();
    assert!(
        matches!(summary_patch_ro.status(), RunStatus::Denied { .. }),
        "ReadOnly mode must deny patch intent"
    );
    assert_eq!(
        fs::read_to_string(ws_path.join("sample.txt"))
            .unwrap()
            .trim(),
        "original text",
        "File must NOT be modified in ReadOnly mode"
    );

    // Dynamic hot-swap to Autonomous mode on ExecutionController
    controller.update_policy(PolicyContext::for_mode(GovernanceMode::Autonomous));
    assert_eq!(controller.policy().mode(), GovernanceMode::Autonomous);
    assert_eq!(
        controller.reference_monitor().governance_mode(),
        GovernanceMode::Autonomous
    );

    // Same mutating patch task in Autonomous -> must Complete and mutate file directly
    let summary_patch_auto = controller
        .run_at(patch_intent.clone(), session.clone(), now)
        .unwrap();
    assert_eq!(
        summary_patch_auto.status(),
        &RunStatus::Completed,
        "Autonomous mode must execute without approval"
    );
    assert_eq!(
        fs::read_to_string(ws_path.join("sample.txt"))
            .unwrap()
            .trim(),
        "tampered_by_readonly",
        "File MUST be modified in Autonomous mode"
    );

    // Dynamic hot-swap to Supervised mode on ExecutionController
    controller.update_policy(PolicyContext::for_mode(GovernanceMode::Supervised));
    assert_eq!(controller.policy().mode(), GovernanceMode::Supervised);
    assert_eq!(
        controller.reference_monitor().governance_mode(),
        GovernanceMode::Supervised
    );

    // Same mutating patch task in Supervised -> must require Approval
    let patch_intent_sup = TaskIntent::new("patch:sample.txt:supervised_edit".to_owned()).unwrap();
    let summary_patch_sup = controller
        .run_at(patch_intent_sup, session.clone(), now)
        .unwrap();
    assert!(
        matches!(
            summary_patch_sup.status(),
            RunStatus::ApprovalRequired { .. }
        ),
        "Supervised mode must halt with ApprovalRequired"
    );
    assert_eq!(
        fs::read_to_string(ws_path.join("sample.txt"))
            .unwrap()
            .trim(),
        "tampered_by_readonly",
        "File must NOT be modified before approval in Supervised mode"
    );

    let _ = fs::remove_dir_all(ws_path);
}

#[test]
fn challenge_governance_capabilities_matrix_completeness() {
    let ro = PolicyContext::for_mode(GovernanceMode::ReadOnly);
    let sup = PolicyContext::for_mode(GovernanceMode::Supervised);
    let auto = PolicyContext::for_mode(GovernanceMode::Autonomous);

    let caps = [
        Capability::FilesystemRead,
        Capability::FilesystemWrite,
        Capability::ProcessExecute,
        Capability::NetworkConnect,
        Capability::ProviderInvoke,
        Capability::WasmExecute,
        Capability::McpInvoke,
    ];

    for cap in caps {
        match cap {
            Capability::FilesystemRead | Capability::ProviderInvoke => {
                assert!(ro.allows(cap), "ReadOnly must allow {:?}", cap);
                assert!(sup.allows(cap), "Supervised must allow {:?}", cap);
                assert!(auto.allows(cap), "Autonomous must allow {:?}", cap);
            }
            _ => {
                assert!(!ro.allows(cap), "ReadOnly must block mutating {:?}", cap);
                assert!(sup.allows(cap), "Supervised must allow {:?}", cap);
                assert!(auto.allows(cap), "Autonomous must allow {:?}", cap);
            }
        }
    }

    // Approval checks
    let ops = [
        Operation::Read,
        Operation::Write,
        Operation::Execute,
        Operation::Connect,
        Operation::Install,
        Operation::Invoke,
    ];

    for op in ops {
        // Autonomous must have zero approval requirements
        assert!(
            !auto.requires_approval(op),
            "Autonomous must require 0 approvals for {:?}",
            op
        );

        if matches!(
            op,
            Operation::Write | Operation::Execute | Operation::Connect
        ) {
            assert!(
                sup.requires_approval(op),
                "Supervised must require approval for {:?}",
                op
            );
        } else {
            assert!(
                !sup.requires_approval(op),
                "Supervised should not require approval for {:?}",
                op
            );
        }
    }
}
