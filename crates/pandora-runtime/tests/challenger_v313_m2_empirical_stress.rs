//! Empirical stress and adversarial challenge harness for Milestone 2 (M2: Requirement R2).
//!
//! Validates:
//! 1. Tool safety evaluation under corrupt JSON inputs, deep nesting, and hostile payloads.
//! 2. Non-finite values (NaN / Inf margin, threshold, entropy) in escalation gates.
//! 3. Precise boundary behavior around normalized Shannon entropy H_n = 0.666 and margin = threshold.
//! 4. High-concurrency permit generation with strict thread safety and nonce uniqueness.
//! 5. System 2 MCTS tree search safety escalation under uncertain reflex evaluations.

use std::collections::HashSet;
use std::sync::Arc;
use std::thread;

use pandora_runtime::decision_engine::{
    SinglePassLogitEngine, TropicalDecisionDriver, normalized_entropy, should_escalate,
    should_escalate_with_entropy,
};
use pandora_runtime::lats_engine::LatsEngine;
use pandora_runtime::parliament::{Parliament, ToolSafetyGateVerdict};
use pandora_runtime::reference_monitor::{AuthorizationError, ReferenceMonitor};
use pandora_runtime::strategies::StrategyBudget;
use pandora_types::{
    Capability, EffectTarget, ExecutionId, ExecutionProfile, ExecutionProfileBinding,
    ExecutionProfileBindingKind, GeneId, Operation, OperationRequest, ParliamentDecision,
    PolicyContext, PrincipalId, ResourceScope, SessionId, Timestamp, hash_artifact,
};

fn make_test_profile() -> ExecutionProfile {
    ExecutionProfile::new(
        "2.0.0-alpha.6",
        "windows",
        "x86_64",
        1,
        "workspace-1",
        hash_artifact(b"containment"),
        vec![
            ExecutionProfileBinding::new(
                ExecutionProfileBindingKind::Executor,
                "system",
                Some("2.0.0-alpha.6"),
                hash_artifact(b"system"),
            )
            .unwrap(),
        ],
    )
    .unwrap()
}

fn make_test_request(capability: Capability, operation: Operation) -> OperationRequest {
    OperationRequest::new(
        ExecutionId::new("exec-stress-1").unwrap(),
        SessionId::new("sess-stress-1").unwrap(),
        PrincipalId::new("user-challenger").unwrap(),
        make_test_profile(),
        GeneId::new("gene-stress-1").unwrap(),
        None,
        capability,
        operation,
        EffectTarget::path("workspace/stress_test.dat"),
        ResourceScope::none(),
    )
    .expect("valid test operation request")
}

// =========================================================================
// 1. Corrupt JSON and Hostile Input Fuzzing
// =========================================================================

#[test]
fn test_corrupt_and_hostile_json_inputs() {
    let parliament = Parliament::new(1);
    let request = make_test_request(Capability::FilesystemRead, Operation::Read);
    let policy = PolicyContext::new(1, [Capability::FilesystemRead], []);

    // 1a. Various JSON primitives that are not objects
    let primitive_args = vec![
        serde_json::Value::Null,
        serde_json::Value::Bool(false),
        serde_json::Value::Bool(true),
        serde_json::Value::Number(serde_json::Number::from(-999999)),
        serde_json::Value::Number(serde_json::Number::from_f64(1.23456789e20).unwrap()),
        serde_json::Value::String("malformed unclosed { [ \"".into()),
        serde_json::json!([]),
        serde_json::json!({}),
    ];

    for args in primitive_args {
        let res = parliament.evaluate_tool_safety("test_tool", &args);
        assert!(
            res.is_ok(),
            "evaluate_tool_safety must handle primitive JSON without panicking: {args:?}"
        );

        let reflex_res =
            parliament.evaluate_tool_safety_reflex(&request, &policy, "test_tool", &args);
        assert!(
            reflex_res.is_ok(),
            "evaluate_tool_safety_reflex must handle primitive JSON without panicking: {args:?}"
        );
    }

    // 1b. Deeply nested JSON structures (e.g. 50 levels deep)
    let mut deeply_nested = serde_json::json!("leaf");
    for _ in 0..50 {
        deeply_nested = serde_json::json!({ "nested": deeply_nested });
    }
    let nested_res = parliament.evaluate_tool_safety("read_file", &deeply_nested);
    assert!(
        nested_res.is_ok(),
        "Deeply nested JSON must not cause stack overflow or panic"
    );

    // 1c. Very large string payload (100 KB with null bytes and control chars)
    let large_string = "A\0\t\r\n".repeat(20_000);
    let large_args = serde_json::json!({ "content": large_string });
    let large_res = parliament.evaluate_tool_safety("read_file", &large_args);
    assert!(large_res.is_ok(), "Large string arguments must not fail");

    // 1d. Hostile destructive commands with variations and obfuscations
    let destructive_payloads = vec![
        serde_json::json!({ "cmd": "rm -rf /" }),
        serde_json::json!({ "cmd": "RM -RF /" }), // uppercase
        serde_json::json!({ "cmd": "drop table users;" }),
        serde_json::json!({ "cmd": "DROP DATABASE prod;" }),
        serde_json::json!({ "cmd": "cat script.sh | sh" }),
        serde_json::json!({ "cmd": "cat script.sh | BASH" }),
        serde_json::json!({ "cmd": "kill -9 1" }),
        serde_json::json!({ "cmd": "FORMAT C:" }),
        serde_json::json!({ "cmd": "git reset --hard HEAD~1" }),
        serde_json::json!({ "cmd": "git clean -fdx" }),
        serde_json::json!({ "cmd": "rmdir /s /q c:\\test" }),
        serde_json::json!({ "cmd": "del /f /s /q *.*" }),
    ];

    for payload in destructive_payloads {
        let verdict = parliament
            .evaluate_tool_safety("shell", &payload)
            .expect("safety eval");
        assert!(
            matches!(verdict, ToolSafetyGateVerdict::Reject { .. }),
            "Destructive command must be classified as Reject: {payload:?}"
        );
    }

    // 1e. Empty tool name and special characters in tool name
    let weird_names = [
        "",
        " ",
        "   ",
        "\0",
        "tool/with/slashes",
        "tool\nwith\nnewlines",
    ];
    for name in weird_names {
        let res = parliament.evaluate_tool_safety(name, &serde_json::json!({}));
        assert!(res.is_ok(), "Special tool name must not panic: {name:?}");
    }
}

// =========================================================================
// 2. Non-Finite Values (NaN / Inf Margin and Entropy)
// =========================================================================

#[test]
fn test_non_finite_values_escalate() {
    // 2a. should_escalate under NaN / Inf values
    // Any non-finite value in margin, uncertainty_threshold, or normalized_entropy must trigger escalation
    assert!(should_escalate(f64::NAN, 0.15, 0.5));
    assert!(should_escalate(0.5, f64::NAN, 0.5));
    assert!(should_escalate(0.5, 0.15, f64::NAN));
    assert!(should_escalate(f64::INFINITY, 0.15, 0.5));
    assert!(should_escalate(0.5, f64::INFINITY, 0.5));
    assert!(should_escalate(0.5, 0.15, f64::INFINITY));
    assert!(should_escalate(f64::NEG_INFINITY, 0.15, 0.5));
    assert!(should_escalate(0.5, f64::NEG_INFINITY, 0.5));
    assert!(should_escalate(0.5, 0.15, f64::NEG_INFINITY));
    assert!(should_escalate(f64::NAN, f64::NAN, f64::NAN));
    assert!(should_escalate(f64::INFINITY, f64::INFINITY, f64::INFINITY));

    // should_escalate_with_entropy alias must match
    assert!(should_escalate_with_entropy(f64::NAN, 0.15, 0.5));
    assert!(should_escalate_with_entropy(0.5, 0.15, f64::NAN));

    // 2b. normalized_entropy with non-finite values in probability distribution
    // Must return 1.0 (maximum uncertainty), triggering escalation
    let non_finite_probs = [
        vec![f64::NAN, 0.5],
        vec![0.5, f64::NAN],
        vec![f64::INFINITY, 0.2],
        vec![0.3, f64::NEG_INFINITY],
        vec![f64::NAN, f64::NAN, f64::NAN],
    ];

    for probs in &non_finite_probs {
        let h = TropicalDecisionDriver::normalized_entropy(probs);
        assert_eq!(
            h, 1.0,
            "Non-finite probability must yield normalized entropy 1.0"
        );
        assert!(normalized_entropy(probs) >= 1.0);
        // And passing to should_escalate must escalate
        assert!(should_escalate(0.5, 0.15, h));
    }

    // 2c. Invalid temperature in SinglePassLogitEngine
    assert!(SinglePassLogitEngine::with_temperature(f64::NAN).is_err());
    assert!(SinglePassLogitEngine::with_temperature(f64::INFINITY).is_err());
    assert!(SinglePassLogitEngine::with_temperature(f64::NEG_INFINITY).is_err());
    assert!(SinglePassLogitEngine::with_temperature(0.0).is_err());
    assert!(SinglePassLogitEngine::with_temperature(-1.0).is_err());

    // 2d. Parliament rejects non-finite uncertainty threshold
    let p = Parliament::new(1).with_uncertainty_threshold(f64::NAN);
    // Should retain default finite threshold
    assert!(p.uncertainty_threshold().is_finite());
}

// =========================================================================
// 3. Boundary Entropy (H_n = 0.666) and Boundary Margin
// =========================================================================

#[test]
fn test_boundary_entropy_and_margin() {
    let threshold = 0.15;
    let safe_margin = 0.50; // safe_margin >= threshold

    // Contract: margin < uncertainty_threshold || normalized_entropy > 0.666
    // Exactly at H_n = 0.666:
    // 0.666 > 0.666 is FALSE. When margin >= threshold, should NOT escalate.
    assert!(
        !should_escalate(safe_margin, threshold, 0.666),
        "At exactly H_n = 0.666 with safe margin, should_escalate must be false"
    );

    // Strictly above H_n = 0.666 by epsilon:
    // 0.6660000000000001 > 0.666 is TRUE -> MUST escalate
    assert!(
        should_escalate(safe_margin, threshold, 0.6660000000000001),
        "At H_n = 0.666 + 1e-16 with safe margin, should_escalate must be true"
    );
    assert!(
        should_escalate(safe_margin, threshold, 0.6661),
        "At H_n = 0.6661 with safe margin, should_escalate must be true"
    );

    // Strictly below H_n = 0.666 by epsilon:
    // 0.6659999999999999 > 0.666 is FALSE -> should NOT escalate
    assert!(
        !should_escalate(safe_margin, threshold, 0.6659999999999999),
        "At H_n = 0.666 - 1e-16 with safe margin, should_escalate must be false"
    );
    assert!(
        !should_escalate(safe_margin, threshold, 0.6659),
        "At H_n = 0.6659 with safe margin, should_escalate must be false"
    );

    // Margin boundary:
    let safe_entropy = 0.40; // safe_entropy <= 0.666
    // Exactly at margin = threshold:
    // 0.15 < 0.15 is FALSE -> should NOT escalate
    assert!(
        !should_escalate(0.15, 0.15, safe_entropy),
        "At margin == threshold with safe entropy, should_escalate must be false"
    );

    // Strictly below threshold by epsilon:
    // 0.1499999999999999 < 0.15 is TRUE -> MUST escalate
    assert!(
        should_escalate(0.1499999999999999, 0.15, safe_entropy),
        "At margin < threshold, should_escalate must be true"
    );

    // Strictly above threshold by epsilon:
    // 0.1500000000000001 < 0.15 is FALSE -> should NOT escalate
    assert!(
        !should_escalate(0.1500000000000001, 0.15, safe_entropy),
        "At margin > threshold with safe entropy, should_escalate must be false"
    );

    // Combined edge cases:
    // 1. Both at boundaries: margin = 0.15, H_n = 0.666 -> neither strictly satisfies condition -> false
    assert!(!should_escalate(0.15, 0.15, 0.666));

    // 2. Margin failing, H_n safe: margin = 0.149, H_n = 0.666 -> true
    assert!(should_escalate(0.149, 0.15, 0.666));

    // 3. Margin safe, H_n failing: margin = 0.15, H_n = 0.6661 -> true
    assert!(should_escalate(0.15, 0.15, 0.6661));

    // 4. Both failing: margin = 0.10, H_n = 0.90 -> true
    assert!(should_escalate(0.10, 0.15, 0.90));
}

// =========================================================================
// 4. High-Concurrency Permit Generation
// =========================================================================

#[test]
fn test_concurrent_permit_generation() {
    let monitor = Arc::new(ReferenceMonitor::new(1, 3600));
    let num_threads = 16;
    let permits_per_thread = 50;
    let total_permits = num_threads * permits_per_thread;

    let mut handles = Vec::with_capacity(num_threads);

    for thread_idx in 0..num_threads {
        let mon = Arc::clone(&monitor);
        handles.push(thread::spawn(move || {
            let mut issued = Vec::with_capacity(permits_per_thread);
            for i in 0..permits_per_thread {
                let req = OperationRequest::new(
                    ExecutionId::new(format!("exec-{thread_idx}-{i}")).unwrap(),
                    SessionId::new("sess-concurrent").unwrap(),
                    PrincipalId::new("user-concurrent").unwrap(),
                    make_test_profile(),
                    GeneId::new("gene-concurrent").unwrap(),
                    None,
                    Capability::FilesystemRead,
                    Operation::Read,
                    EffectTarget::path(format!("workspace/file_{thread_idx}_{i}.txt")),
                    ResourceScope::none(),
                )
                .expect("valid request");

                let permit = mon
                    .authorize_tool_safety(
                        req.clone(),
                        "read_file",
                        &serde_json::json!({ "path": format!("workspace/file_{thread_idx}_{i}.txt") }),
                        Timestamp::from_unix_seconds(1000 + i as u64),
                    )
                    .expect("authorization must succeed for safe read");
                issued.push((permit, req));
            }
            issued
        }));
    }

    let mut all_permits = Vec::with_capacity(total_permits);
    for h in handles {
        let thread_permits = h.join().expect("thread join must succeed");
        all_permits.extend(thread_permits);
    }

    assert_eq!(all_permits.len(), total_permits);

    // Verify all nonces are strictly unique and sequential (1..=total_permits)
    let nonces: HashSet<u64> = all_permits.iter().map(|(p, _)| p.nonce()).collect();
    assert_eq!(
        nonces.len(),
        total_permits,
        "All issued permit nonces must be strictly unique"
    );

    for expected_nonce in 1..=(total_permits as u64) {
        assert!(
            nonces.contains(&expected_nonce),
            "Nonce sequence must contain nonce {expected_nonce}"
        );
    }

    // Verify all permit IDs are unique
    let permit_ids: HashSet<String> = all_permits
        .iter()
        .map(|(p, _)| p.permit_id().as_str().to_string())
        .collect();
    assert_eq!(
        permit_ids.len(),
        total_permits,
        "All permit IDs must be unique"
    );

    // Verify all permits are registered in PermitStore and can be consumed
    let store = monitor.store();
    for (permit, req) in &all_permits {
        let consume_res = store.consume(permit.clone(), req, Timestamp::from_unix_seconds(2000));
        assert!(
            consume_res.is_ok(),
            "Registered permit must be consumable from store: {consume_res:?}"
        );
    }
}

// =========================================================================
// 5. System 2 MCTS Tree Search Resolves Safety Under Uncertain Reflex
// =========================================================================

#[test]
fn test_system2_mcts_resolves_safety_under_uncertain_reflex() {
    let parliament = Parliament::new(1);
    let policy = PolicyContext::new(
        1,
        [Capability::FilesystemRead, Capability::ProviderInvoke],
        [Operation::Write, Operation::Execute],
    );

    // 5a. Ambiguous tool invocation triggering escalation
    // Commands containing "force", "bypass", "override", etc. trigger ambiguous logits [2.0, 2.1, 1.9]
    let uncertain_args = serde_json::json!({
        "cmd": "system-sync --force --override-cache"
    });

    let gate_verdict = parliament
        .evaluate_tool_safety("sync_tool", &uncertain_args)
        .expect("gate evaluation");

    assert!(
        gate_verdict.is_escalated(),
        "Ambiguous tool invocation must produce EscalateToSystem2 gate verdict"
    );
    if let ToolSafetyGateVerdict::EscalateToSystem2 {
        margin,
        normalized_entropy,
        threshold,
        ..
    } = gate_verdict
    {
        assert!(
            margin < threshold || normalized_entropy > 0.666,
            "Escalation invariant: margin {margin} < threshold {threshold} OR entropy {normalized_entropy} > 0.666"
        );
    } else {
        panic!("Expected EscalateToSystem2");
    }

    // 5b. Evaluate reflex with escalation to System 2 MCTS
    let read_request = make_test_request(Capability::FilesystemRead, Operation::Read);
    let reflex_verdict = parliament
        .evaluate_tool_safety_reflex(&read_request, &policy, "sync_tool", &uncertain_args)
        .expect("reflex evaluation with MCTS escalation");

    assert!(
        reflex_verdict.is_escalated(),
        "Verdit must indicate escalated: true"
    );
    assert!(
        reflex_verdict.mcts_action.is_some(),
        "MCTS action must be resolved when escalated"
    );
    let mcts_act = reflex_verdict.mcts_action.unwrap();
    assert!(
        mcts_act <= 2,
        "MCTS action must be one of [0: allow, 1: require_confirm, 2: reject]"
    );

    // Check decision correspondence to MCTS action
    match mcts_act {
        0 => {
            // Action 0 is Allow
            assert!(
                matches!(reflex_verdict.decision, ParliamentDecision::Allow { .. }),
                "MCTS action 0 corresponds to Allow"
            );
        }
        1 => {
            // Action 1 is RequireConfirm -> RequireApproval
            assert!(
                matches!(
                    reflex_verdict.decision,
                    ParliamentDecision::RequireApproval { .. }
                ),
                "MCTS action 1 corresponds to RequireApproval"
            );
        }
        _ => {
            // Action 2 is Reject -> Deny
            assert!(
                matches!(reflex_verdict.decision, ParliamentDecision::Deny { .. }),
                "MCTS action 2 corresponds to Deny"
            );
        }
    }

    // 5c. End-to-end ReferenceMonitor with MCTS escalation
    let monitor = ReferenceMonitor::new_with_policy(policy.clone(), 3600);

    // If MCTS allowed, permit is issued. If MCTS rejected/required approval, proper error is returned.
    let auth_res = monitor.authorize_tool_safety_with_verdict(
        read_request.clone(),
        "sync_tool",
        &uncertain_args,
        Timestamp::from_unix_seconds(5000),
    );

    match reflex_verdict.decision {
        ParliamentDecision::Allow { .. } => {
            assert!(auth_res.is_ok(), "Allowed MCTS decision must issue permit");
            let (permit, verdict) = auth_res.unwrap();
            assert!(verdict.is_escalated());
            assert_eq!(permit.policy_version(), 1);
        }
        ParliamentDecision::RequireApproval { .. } => {
            assert!(
                matches!(auth_res, Err(AuthorizationError::ApprovalRequired { .. })),
                "RequireApproval MCTS decision must return ApprovalRequired"
            );
        }
        ParliamentDecision::Deny { .. } => {
            assert!(
                matches!(auth_res, Err(AuthorizationError::Denied { .. })),
                "Deny MCTS decision must return Denied"
            );
        }
    }

    // 5d. Custom configured LatsEngine in Parliament
    let custom_budget = StrategyBudget::new(2, 4, 500, 4, 5, 500).unwrap();
    let custom_lats = Arc::new(LatsEngine::new(custom_budget, SinglePassLogitEngine::new()));
    let parliament_with_lats = Parliament::new(1)
        .with_lats_engine(custom_lats)
        .with_uncertainty_threshold(0.25);

    assert_eq!(parliament_with_lats.uncertainty_threshold(), 0.25);
    let custom_reflex = parliament_with_lats
        .evaluate_tool_safety_reflex(&read_request, &policy, "sync_tool", &uncertain_args)
        .expect("custom lats reflex eval");
    assert!(custom_reflex.is_escalated());
    assert!(custom_reflex.mcts_action.is_some());

    // 5e. Defense-in-depth: capability not allowed in policy denies BEFORE reflex/escalation
    let unauthorized_req = make_test_request(Capability::NetworkConnect, Operation::Connect);
    let deny_verdict = parliament
        .evaluate_tool_safety_reflex(&unauthorized_req, &policy, "sync_tool", &uncertain_args)
        .expect("reflex evaluation");
    assert!(
        !deny_verdict.is_escalated(),
        "Unauthorized capability must fail fast without escalating to MCTS"
    );
    assert!(matches!(
        deny_verdict.decision,
        ParliamentDecision::Deny { .. }
    ));
}
