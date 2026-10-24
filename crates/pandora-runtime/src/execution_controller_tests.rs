use super::*;
use crate::{ApprovalRequest, HookPoint, HookSelector, LifecycleHook, LifecycleHooks, WasmGene};
use pandora_harnesses::HarnessCatalog;
use pandora_provider::{
    ChatMessage, FailoverProvider, ModelResponse, ProviderManifest, TokenUsage,
};
use pandora_types::{
    Capability, GeneId, HarnessId, Operation, PackageCompatibility, PackageDependency, PackageKind,
    PackageManifest, PolicyContext, PrincipalId, Session, SessionId, TaskIntent, TenantId,
    Timestamp, TrustEvidence, WorkspaceId, hash_artifact,
};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[test]
fn read_only_coding_task_completes_with_receipt_and_events() {
    let fixture = Fixture::new();
    let controller = ExecutionController::new(fixture.root.clone());

    let summary = controller
        .run_at(
            TaskIntent::new("read:README.md").unwrap(),
            fixture.session(),
            Timestamp::from_unix_seconds(10),
        )
        .unwrap();

    assert_eq!(summary.output().unwrap(), b"fixture\n");
    assert_eq!(summary.status(), &RunStatus::Completed);
    assert_eq!(summary.receipts().len(), 1);
    assert_eq!(summary.events().len(), 4);
    assert_eq!(summary.selected_harness().as_str(), "coding-domain");
    assert_eq!(summary.selected_gene().as_str(), "workspace.read");
}

#[test]
fn an_admitted_domain_profile_runs_through_the_existing_controller() {
    let fixture = Fixture::new();
    let artifact = b"domain profile";
    let package = PackageManifest::new(
        "example/domain",
        "1.0.0",
        PackageKind::DomainHarness,
        "publisher",
        hash_artifact(artifact),
        vec![PackageDependency::new("workspace.read", "0.1.0", false).unwrap()],
        PackageCompatibility::new("pandora>=2.0.0").unwrap(),
        "MIT",
        TrustEvidence::unsigned(),
    )
    .unwrap();
    let harnesses = HarnessCatalog::builtins()
        .with_declarative_domain(&package)
        .unwrap();
    let controller = ExecutionController::with_policy_and_harnesses(
        fixture.root.clone(),
        PolicyContext::read_only_workspace(),
        harnesses,
    );
    let intent = TaskIntent::new("read:README.md")
        .unwrap()
        .with_harness(HarnessId::new("example/domain").unwrap())
        .with_gene(GeneId::new("workspace.read").unwrap());

    let summary = controller
        .run_at(intent, fixture.session(), Timestamp::from_unix_seconds(10))
        .unwrap();

    assert_eq!(summary.status(), &RunStatus::Completed);
    assert_eq!(summary.output().unwrap(), b"fixture\n");
    assert_eq!(summary.selected_harness().as_str(), "example/domain");
    assert_eq!(summary.selected_gene().as_str(), "workspace.read");
}

#[test]
fn package_wasm_gene_requires_one_exact_approval_before_execution() {
    let fixture = Fixture::new();
    let artifact = wat::parse_str(
        r#"(module
                (memory (export "memory") 1)
                (func (export "pandora_alloc") (param i32) (result i32) i32.const 0)
                (func (export "pandora_run") (param i32 i32) (result i64)
                    local.get 0
                    i64.extend_i32_u
                    i64.const 32
                    i64.shl
                    local.get 1
                    i64.extend_i32_u
                    i64.or))"#,
    )
    .unwrap();
    let gene_package = PackageManifest::new(
        "example/echo",
        "1.0.0",
        PackageKind::Gene,
        "publisher",
        hash_artifact(&artifact),
        Vec::new(),
        PackageCompatibility::new("pandora>=2.0.0").unwrap(),
        "MIT",
        TrustEvidence::unsigned(),
    )
    .unwrap();
    let domain_package = PackageManifest::new(
        "example/wasm-domain",
        "1.0.0",
        PackageKind::DomainHarness,
        "publisher",
        hash_artifact(b"domain profile"),
        vec![PackageDependency::new("example/echo", "1.0.0", false).unwrap()],
        PackageCompatibility::new("pandora>=2.0.0").unwrap(),
        "MIT",
        TrustEvidence::unsigned(),
    )
    .unwrap();
    let gene = WasmGene::from_package(&gene_package).unwrap();
    let harnesses = HarnessCatalog::builtins()
        .with_declarative_domain_genes(&domain_package, vec![Box::new(gene)])
        .unwrap();
    let mut wasm = WasmExecutor::new();
    wasm.register(&gene_package, &artifact).unwrap();
    let policy = PolicyContext::new(1, [Capability::WasmExecute], [Operation::Execute]);
    let controller =
        ExecutionController::with_policy_and_harnesses(fixture.root.clone(), policy, harnesses)
            .with_wasm_executor(wasm);
    let intent = TaskIntent::new(r#"{"value":42}"#)
        .unwrap()
        .with_harness(HarnessId::new("example/wasm-domain").unwrap())
        .with_gene(GeneId::new("example/echo").unwrap());
    let session = fixture.session();
    let now = Timestamp::from_unix_seconds(10);

    let pending = controller
        .run_at(intent.clone(), session.clone(), now)
        .unwrap();
    assert!(matches!(
        pending.status(),
        RunStatus::ApprovalRequired { .. }
    ));
    assert!(pending.receipts().is_empty());
    let request_digest = pending
        .events()
        .iter()
        .find_map(|event| match event.payload() {
            EventPayload::Effect { request_digest, .. } => Some(request_digest.clone()),
            _ => None,
        })
        .unwrap();
    let approval_path = fixture.path.join("approvals.sqlite3");
    let approvals = ApprovalStore::open(&approval_path).unwrap();
    approvals
        .create(
            ApprovalRequest::new(
                "approval-wasm",
                session.id().clone(),
                pending.execution_id().clone(),
                session.principal_id().clone(),
                GeneId::new("example/echo").unwrap(),
                request_digest,
                "execute example/echo@1.0.0",
                1,
                Timestamp::from_unix_seconds(100),
            )
            .unwrap(),
        )
        .unwrap();
    approvals
        .resolve(
            "approval-wasm",
            session.principal_id(),
            &PrincipalId::new("approver-1").unwrap(),
            true,
            now,
        )
        .unwrap();

    let completed = controller
        .run_with_approval(intent, session, &approvals, "approval-wasm", now)
        .unwrap();

    assert_eq!(completed.status(), &RunStatus::Completed);
    assert_eq!(completed.output(), Some(br#"{"value":42}"#.as_slice()));
    assert_eq!(completed.receipts().len(), 1);
}

#[test]
fn search_returns_matching_workspace_files() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.path.join("src")).unwrap();
    std::fs::write(fixture.path.join("src/lib.rs"), b"needle\n").unwrap();
    let controller = ExecutionController::new(fixture.root.clone());

    let summary = controller
        .run_at(
            TaskIntent::new("search:needle").unwrap(),
            fixture.session(),
            Timestamp::from_unix_seconds(10),
        )
        .unwrap();

    assert_eq!(summary.status(), &RunStatus::Completed);
    assert_eq!(summary.output().unwrap(), b"src/lib.rs");
    assert_eq!(summary.selected_gene().as_str(), "workspace.search");
}

#[test]
fn research_source_comparison_is_read_only_and_receipted() {
    let fixture = Fixture::new();
    std::fs::write(fixture.path.join("first.txt"), b"first evidence\n").unwrap();
    std::fs::write(fixture.path.join("second.txt"), b"second evidence\n").unwrap();
    let controller = ExecutionController::new(fixture.root.clone());
    let intent = TaskIntent::new("source-compare:first.txt|second.txt")
        .unwrap()
        .with_harness(HarnessId::new("research-domain").unwrap())
        .with_gene(GeneId::new("source.compare").unwrap());

    let summary = controller
        .run_at(intent, fixture.session(), Timestamp::from_unix_seconds(10))
        .unwrap();

    assert_eq!(summary.status(), &RunStatus::Completed);
    assert_eq!(summary.selected_harness().as_str(), "research-domain");
    assert_eq!(summary.selected_gene().as_str(), "source.compare");
    assert_eq!(summary.receipts().len(), 2);
    let output = std::str::from_utf8(summary.output().unwrap()).unwrap();
    assert!(output.contains("first.txt:\nfirst evidence"));
    assert!(output.contains("second.txt:\nsecond evidence"));
}

#[test]
fn research_citation_inventory_uses_fixed_read_only_markers() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.path.join("sources.md"),
        b"https://example.test\ndoi:10.1000/example\n",
    )
    .unwrap();
    let controller = ExecutionController::new(fixture.root.clone());
    let intent = TaskIntent::new("citation-inventory")
        .unwrap()
        .with_harness(HarnessId::new("research-domain").unwrap())
        .with_gene(GeneId::new("citation.inventory").unwrap());

    let summary = controller
        .run_at(intent, fixture.session(), Timestamp::from_unix_seconds(10))
        .unwrap();

    assert_eq!(summary.status(), &RunStatus::Completed);
    assert_eq!(summary.receipts().len(), 3);
    let output = std::str::from_utf8(summary.output().unwrap()).unwrap();
    assert!(output.contains("https://:\nsources.md"));
    assert!(output.contains("doi::\nsources.md"));
}

#[test]
fn design_token_evidence_uses_fixed_read_only_markers() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.path.join("theme.css"),
        b":root { --color-brand: #123456; }\n.button { color: var(--color-brand); }\n",
    )
    .unwrap();
    let controller = ExecutionController::new(fixture.root.clone());
    let intent = TaskIntent::new("design-tokens")
        .unwrap()
        .with_harness(HarnessId::new("design-domain").unwrap())
        .with_gene(GeneId::new("design.tokens").unwrap());

    let summary = controller
        .run_at(intent, fixture.session(), Timestamp::from_unix_seconds(10))
        .unwrap();

    assert_eq!(summary.status(), &RunStatus::Completed);
    assert_eq!(summary.selected_harness().as_str(), "design-domain");
    assert_eq!(summary.receipts().len(), 4);
    let output = std::str::from_utf8(summary.output().unwrap()).unwrap();
    assert!(output.contains(":root:\ntheme.css"));
    assert!(output.contains("var(:\ntheme.css"));
    assert!(output.contains("--color:\ntheme.css"));
}

#[test]
fn patch_is_stopped_at_the_approval_boundary() {
    let fixture = Fixture::new();
    let policy = PolicyContext::new(
        1,
        [Capability::FilesystemRead, Capability::FilesystemWrite],
        [Operation::Write],
    );
    let controller = ExecutionController::with_policy(fixture.root.clone(), policy);

    let summary = controller
        .run_at(
            TaskIntent::new("patch:README.md:changed").unwrap(),
            fixture.session(),
            Timestamp::from_unix_seconds(10),
        )
        .unwrap();

    assert_eq!(
        summary.status(),
        &RunStatus::ApprovalRequired {
            reason: "operation requires explicit approval".to_owned()
        }
    );
    assert!(
        summary
            .events()
            .iter()
            .any(|event| event.event_type() == EventType::ApprovalRequired)
    );
}

#[test]
fn lifecycle_hooks_run_in_order_and_stop_before_authorization() {
    let fixture = Fixture::new();
    let hooks = LifecycleHooks::new()
        .with_hook(LifecycleHook::deny(
            "process-only",
            HookPoint::BeforeAuthorization,
            HookSelector::Capability(Capability::ProcessExecute),
            "processes_disabled",
        ))
        .with_hook(LifecycleHook::deny(
            "maintenance",
            HookPoint::BeforeAuthorization,
            HookSelector::Any,
            "maintenance_window",
        ))
        .with_hook(LifecycleHook::deny(
            "later-rule",
            HookPoint::BeforeAuthorization,
            HookSelector::Any,
            "later_rule_should_not_win",
        ));
    let controller = ExecutionController::with_hooks(fixture.root.clone(), hooks);

    let summary = controller
        .run_at(
            TaskIntent::new("read:README.md").unwrap(),
            fixture.session(),
            Timestamp::from_unix_seconds(10),
        )
        .unwrap();

    assert_eq!(
        summary.status(),
        &RunStatus::Denied {
            reason: "lifecycle hook maintenance denied request: maintenance_window".to_owned()
        }
    );
    assert!(summary.receipts().is_empty());
    assert!(
        summary
            .events()
            .iter()
            .any(|event| event.event_type() == EventType::PolicyDenied)
    );
    assert!(
        summary
            .events()
            .iter()
            .all(|event| event.event_type() != EventType::EffectCompleted)
    );
}

#[test]
fn lifecycle_hook_denies_provider_egress_before_the_provider_is_called() {
    let fixture = Fixture::new();
    let provider = PanickingProvider::new();
    let hooks = LifecycleHooks::new().with_hook(LifecycleHook::deny(
        "offline-mode",
        HookPoint::BeforeAuthorization,
        HookSelector::Capability(Capability::ProviderInvoke),
        "provider_egress_disabled",
    ));
    let controller = ExecutionController::with_hooks(fixture.root.clone(), hooks);
    let request = ModelRequest::new(
        provider.manifest().id().clone(),
        provider.manifest().default_model().clone(),
        vec![ChatMessage::user("hello").unwrap()],
    )
    .unwrap();

    let result = controller.invoke_provider(
        &provider,
        request,
        &fixture.session(),
        Timestamp::from_unix_seconds(10),
    );

    assert!(matches!(
        result,
        Err(RuntimeError::Denied(reason))
            if reason
                == "lifecycle hook offline-mode denied request: provider_egress_disabled"
    ));
}

#[test]
fn provider_profile_bindings_cover_endpoint_and_builtin_gene() {
    let first = ProviderManifest::new(
        "provider-a",
        "Provider A",
        "https://first.example.test/v1",
        "model-a",
        "PANDORA_PROVIDER_A_KEY",
    )
    .unwrap();
    let second = ProviderManifest::new(
        "provider-a",
        "Provider A",
        "https://second.example.test/v1",
        "model-a",
        "PANDORA_PROVIDER_A_KEY",
    )
    .unwrap();

    let first_bindings = provider_profile_bindings(&first, "model-a").unwrap();
    let second_bindings = provider_profile_bindings(&second, "model-a").unwrap();
    let first_provider = first_bindings
        .iter()
        .find(|binding| binding.kind() == ExecutionProfileBindingKind::Provider)
        .unwrap();
    let second_provider = second_bindings
        .iter()
        .find(|binding| binding.kind() == ExecutionProfileBindingKind::Provider)
        .unwrap();

    assert_ne!(first_provider.digest(), second_provider.digest());
    assert!(first_bindings.iter().any(|binding| {
        binding.kind() == ExecutionProfileBindingKind::Gene && binding.id() == "provider.invoke"
    }));
    let serialized = serde_json::to_string(&first_bindings).unwrap();
    assert!(!serialized.contains("first.example.test"));
    assert!(!serialized.contains("PANDORA_PROVIDER_A_KEY"));
}

#[test]
fn mcp_invocation_profile_bindings_cover_process_and_configuration() {
    let first_supervisor = McpCatalogSupervisor::new();
    let first_reservation = first_supervisor.reserve("local", "config-a").unwrap();
    let first_tool = crate::mcp_catalog::McpCatalogTool::new(
        "mcp.local.echo",
        "echo",
        &serde_json::json!({"type": "object"}),
    )
    .unwrap();
    let first_revision = first_reservation
        .activate(McpWireEra::Modern, 41, vec![first_tool])
        .unwrap();
    let second_supervisor = McpCatalogSupervisor::new();
    let second_reservation = second_supervisor.reserve("local", "config-b").unwrap();
    let second_tool = crate::mcp_catalog::McpCatalogTool::new(
        "mcp.local.echo",
        "echo",
        &serde_json::json!({"type": "object"}),
    )
    .unwrap();
    let second_revision = second_reservation
        .activate(McpWireEra::Modern, 42, vec![second_tool])
        .unwrap();

    let first_bindings = mcp_invocation_profile_bindings(
        &first_revision,
        "mcp.local.echo",
        first_revision.tool("mcp.local.echo").unwrap(),
    )
    .unwrap();
    let second_bindings = mcp_invocation_profile_bindings(
        &second_revision,
        "mcp.local.echo",
        second_revision.tool("mcp.local.echo").unwrap(),
    )
    .unwrap();
    let first_configuration = first_bindings
        .iter()
        .find(|binding| binding.kind() == ExecutionProfileBindingKind::Configuration)
        .unwrap();
    let second_configuration = second_bindings
        .iter()
        .find(|binding| binding.kind() == ExecutionProfileBindingKind::Configuration)
        .unwrap();

    assert_ne!(first_configuration.digest(), second_configuration.digest());
    assert!(first_bindings.iter().any(|binding| {
        binding.kind() == ExecutionProfileBindingKind::ToolCatalog && binding.id() == "local"
    }));
}

#[test]
fn worktree_profile_bindings_cover_the_managed_root_without_exposing_it() {
    let first = worktree_profile_bindings(
        "coordination.worktree.create",
        "git_worktree_create",
        r#"{"operation":"git_worktree_create"}"#,
        r"C:\work\pandora\managed-a",
    )
    .unwrap();
    let second = worktree_profile_bindings(
        "coordination.worktree.create",
        "git_worktree_create",
        r#"{"operation":"git_worktree_create"}"#,
        r"C:\work\pandora\managed-b",
    )
    .unwrap();
    let first_root = first
        .iter()
        .find(|binding| binding.id() == "managed_worktree_root")
        .unwrap();
    let second_root = second
        .iter()
        .find(|binding| binding.id() == "managed_worktree_root")
        .unwrap();

    assert_ne!(first_root.digest(), second_root.digest());
    let serialized = serde_json::to_string(&first).unwrap();
    assert!(!serialized.contains(r"C:\work\pandora\managed-a"));
}

#[test]
fn provider_fallback_requires_a_fresh_permit_and_receipt() {
    let fixture = Fixture::new();
    let primary_requests = Arc::new(Mutex::new(Vec::new()));
    let fallback_requests = Arc::new(Mutex::new(Vec::new()));
    let primary = StubProvider::new(
        "provider-a",
        "model-a",
        Err(ProviderError::Transport),
        Arc::clone(&primary_requests),
    );
    let fallback = StubProvider::new(
        "provider-b",
        "model-b",
        Ok(ModelResponse::new(
            "fallback-ready",
            Vec::new(),
            TokenUsage::new(2, 1),
        )),
        Arc::clone(&fallback_requests),
    );
    let provider = FailoverProvider::new(Box::new(primary), Box::new(fallback));
    let request = ModelRequest::new(
        provider.manifest().id().clone(),
        provider.manifest().default_model().clone(),
        vec![ChatMessage::user("hello").unwrap()],
    )
    .unwrap();
    let expected_messages = request.messages().to_vec();
    let controller = ExecutionController::new(fixture.root.clone());

    let result = controller
        .invoke_provider(
            &provider,
            request,
            &fixture.session(),
            Timestamp::from_unix_seconds(10),
        )
        .unwrap();

    assert_eq!(result.result().unwrap().text(), "fallback-ready");
    assert_eq!(primary_requests.lock().unwrap().len(), 1);
    let fallback_requests = fallback_requests.lock().unwrap();
    assert_eq!(fallback_requests.len(), 1);
    assert_eq!(fallback_requests[0].provider_id().as_str(), "provider-b");
    assert_eq!(fallback_requests[0].model_id().as_str(), "model-b");
    assert_eq!(fallback_requests[0].messages(), expected_messages);
    assert_eq!(result.receipts().len(), 2);
    assert!(matches!(
        result.receipts()[0].outcome(),
        pandora_types::EffectOutcome::Failed { code } if code == "transport"
    ));
    assert!(matches!(
        result.receipts()[1].outcome(),
        pandora_types::EffectOutcome::Succeeded
    ));
    assert_ne!(
        result.receipts()[0].permit_id(),
        result.receipts()[1].permit_id()
    );
    assert_ne!(
        result.receipts()[0].request_digest(),
        result.receipts()[1].request_digest()
    );
    assert_eq!(result.receipt(), &result.receipts()[1]);
}

#[test]
fn non_retryable_provider_error_does_not_invoke_fallback() {
    let fixture = Fixture::new();
    let primary_requests = Arc::new(Mutex::new(Vec::new()));
    let fallback_requests = Arc::new(Mutex::new(Vec::new()));
    let primary = StubProvider::new(
        "provider-a",
        "model-a",
        Err(ProviderError::InvalidRequest("invalid request".to_owned())),
        Arc::clone(&primary_requests),
    );
    let fallback = StubProvider::new(
        "provider-b",
        "model-b",
        Ok(ModelResponse::new(
            "must-not-run",
            Vec::new(),
            TokenUsage::default(),
        )),
        Arc::clone(&fallback_requests),
    );
    let provider = FailoverProvider::new(Box::new(primary), Box::new(fallback));
    let request = ModelRequest::new(
        provider.manifest().id().clone(),
        provider.manifest().default_model().clone(),
        vec![ChatMessage::user("hello").unwrap()],
    )
    .unwrap();
    let controller = ExecutionController::new(fixture.root.clone());

    let result = controller
        .invoke_provider(
            &provider,
            request,
            &fixture.session(),
            Timestamp::from_unix_seconds(10),
        )
        .unwrap();

    assert!(matches!(
        result.result(),
        Err(ProviderError::InvalidRequest(message)) if message == "invalid request"
    ));
    assert_eq!(primary_requests.lock().unwrap().len(), 1);
    assert!(fallback_requests.lock().unwrap().is_empty());
    assert_eq!(result.receipts().len(), 1);
}

#[test]
fn path_escape_is_rejected_before_effect_authorization() {
    let fixture = Fixture::new();
    let controller = ExecutionController::new(fixture.root.clone());

    assert!(matches!(
        controller.run_at(
            TaskIntent::new("read:../outside.txt").unwrap(),
            fixture.session(),
            Timestamp::from_unix_seconds(10),
        ),
        Err(RuntimeError::Planning(_))
    ));
}

#[test]
fn unknown_gene_is_not_silently_replaced() {
    let fixture = Fixture::new();
    let controller = ExecutionController::new(fixture.root.clone());
    let intent = TaskIntent::new("read:README.md")
        .unwrap()
        .with_harness(HarnessId::new("coding-domain").unwrap())
        .with_gene(GeneId::new("unknown.gene").unwrap());

    assert_eq!(
        controller.run_at(intent, fixture.session(), Timestamp::from_unix_seconds(10)),
        Err(RuntimeError::UnknownGene)
    );
}

#[test]
fn unsupported_harness_is_rejected_before_gene_planning() {
    let fixture = Fixture::new();
    let controller = ExecutionController::new(fixture.root.clone());
    let harness_id = HarnessId::new("unavailable-domain").unwrap();
    let intent = TaskIntent::new("read:README.md")
        .unwrap()
        .with_harness(harness_id.clone());

    assert_eq!(
        controller.run_at(intent, fixture.session(), Timestamp::from_unix_seconds(10)),
        Err(RuntimeError::UnsupportedHarness(harness_id))
    );
}

#[test]
fn source_and_meta_harnesses_are_not_execution_targets() {
    let fixture = Fixture::new();
    let controller = ExecutionController::new(fixture.root.clone());

    for harness_id in [
        "core-source",
        "coordination-meta",
        "evolution-meta",
        "swarm-meta",
    ] {
        let harness_id = HarnessId::new(harness_id).unwrap();
        let intent = TaskIntent::new("read:README.md")
            .unwrap()
            .with_harness(harness_id.clone());
        let kind = if harness_id.as_str() == "core-source" {
            HarnessKind::Source
        } else {
            HarnessKind::Meta
        };

        assert_eq!(
            controller.run_at(intent, fixture.session(), Timestamp::from_unix_seconds(10),),
            Err(RuntimeError::NonExecutableHarness {
                id: harness_id,
                kind,
            })
        );
    }
}

#[test]
fn security_audit_searches_fixed_markers_through_governed_reads() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.path.join("src")).unwrap();
    std::fs::write(
        fixture.path.join("src/security.rs"),
        b"fn review() { let _ = unsafe { 1 }; }\n",
    )
    .unwrap();
    let controller = ExecutionController::new(fixture.root.clone());
    let intent = TaskIntent::new("security-audit")
        .unwrap()
        .with_harness(HarnessId::new("security-domain").unwrap())
        .with_gene(GeneId::new("security.audit").unwrap());

    let summary = controller
        .run_at(intent, fixture.session(), Timestamp::from_unix_seconds(10))
        .unwrap();

    assert_eq!(summary.status(), &RunStatus::Completed);
    assert_eq!(summary.receipts().len(), 6);
    let output = String::from_utf8(summary.output().unwrap().to_vec()).unwrap();
    assert!(output.contains("unsafe:\nsrc/security.rs"));
    assert!(!output.contains("Command::new:\nsrc/security.rs"));
}

#[test]
fn security_threat_model_searches_fixed_evidence_through_governed_reads() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.path.join("docs")).unwrap();
    std::fs::write(
        fixture.path.join("docs/security.md"),
        b"The threat model documents the sandbox trust boundary.\n",
    )
    .unwrap();
    let controller = ExecutionController::new(fixture.root.clone());
    let intent = TaskIntent::new("security-threat-model")
        .unwrap()
        .with_harness(HarnessId::new("security-domain").unwrap())
        .with_gene(GeneId::new("security.threat-model").unwrap());

    let summary = controller
        .run_at(intent, fixture.session(), Timestamp::from_unix_seconds(10))
        .unwrap();

    assert_eq!(summary.status(), &RunStatus::Completed);
    assert_eq!(summary.receipts().len(), 8);
    let output = String::from_utf8(summary.output().unwrap().to_vec()).unwrap();
    assert!(output.contains("trust boundary:\ndocs/security.md"));
    assert!(output.contains("sandbox:\ndocs/security.md"));
}

#[test]
fn security_guide_is_static_and_requests_no_effect() {
    let fixture = Fixture::new();
    let controller = ExecutionController::new(fixture.root.clone());
    let intent = TaskIntent::new("security-guide")
        .unwrap()
        .with_harness(HarnessId::new("security-domain").unwrap())
        .with_gene(GeneId::new("security.guide").unwrap());

    let summary = controller
        .run_at(intent, fixture.session(), Timestamp::from_unix_seconds(10))
        .unwrap();

    assert_eq!(summary.status(), &RunStatus::Completed);
    assert!(summary.receipts().is_empty());
    assert!(String::from_utf8_lossy(summary.output().unwrap()).contains("Security Audit"));
}

#[test]
fn architecture_guide_is_static_and_requests_no_effect() {
    let fixture = Fixture::new();
    let controller = ExecutionController::new(fixture.root.clone());
    let intent = TaskIntent::new("architecture-guide")
        .unwrap()
        .with_harness(HarnessId::new("architecture-domain").unwrap())
        .with_gene(GeneId::new("architecture.guide").unwrap());

    let summary = controller
        .run_at(intent, fixture.session(), Timestamp::from_unix_seconds(10))
        .unwrap();

    assert_eq!(summary.status(), &RunStatus::Completed);
    assert!(summary.receipts().is_empty());
    assert!(String::from_utf8_lossy(summary.output().unwrap()).contains("Architecture inventory"));
}

#[test]
fn verification_guide_is_static_and_requests_no_effect() {
    let fixture = Fixture::new();
    let controller = ExecutionController::new(fixture.root.clone());
    let intent = TaskIntent::new("verification-guide")
        .unwrap()
        .with_harness(HarnessId::new("verification-domain").unwrap())
        .with_gene(GeneId::new("verification.guide").unwrap());

    let summary = controller
        .run_at(intent, fixture.session(), Timestamp::from_unix_seconds(10))
        .unwrap();

    assert_eq!(summary.status(), &RunStatus::Completed);
    assert!(summary.receipts().is_empty());
    assert!(
        String::from_utf8_lossy(summary.output().unwrap()).contains("Verification contract audits")
    );
}

#[test]
fn debugging_failure_evidence_searches_fixed_markers_through_governed_reads() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.path.join("src")).unwrap();
    std::fs::write(
        fixture.path.join("src/failure.rs"),
        b"fn run() { panic!(\"failure\"); }\n",
    )
    .unwrap();
    let controller = ExecutionController::new(fixture.root.clone());
    let intent = TaskIntent::new("debugging-failures")
        .unwrap()
        .with_harness(HarnessId::new("debugging-domain").unwrap())
        .with_gene(GeneId::new("debugging.failures").unwrap());

    let summary = controller
        .run_at(intent, fixture.session(), Timestamp::from_unix_seconds(10))
        .unwrap();

    assert_eq!(summary.status(), &RunStatus::Completed);
    assert_eq!(summary.receipts().len(), 8);
    let output = String::from_utf8(summary.output().unwrap().to_vec()).unwrap();
    assert!(output.contains("panic!:\nsrc/failure.rs"));
}

#[test]
fn daedalus_audit_returns_a_bounded_workspace_inventory() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.path.join("src")).unwrap();
    std::fs::write(fixture.path.join("src/lib.rs"), b"pub fn example() {}\n").unwrap();
    let controller = ExecutionController::new(fixture.root.clone());
    let intent = TaskIntent::new("audit")
        .unwrap()
        .with_gene(GeneId::new("daedalus.audit").unwrap());

    let summary = controller
        .run_at(intent, fixture.session(), Timestamp::from_unix_seconds(10))
        .unwrap();

    assert_eq!(summary.status(), &RunStatus::Completed);
    assert_eq!(summary.receipts().len(), 1);
    assert_eq!(summary.output().unwrap(), b"README.md\nsrc/lib.rs");
}

#[test]
fn ariadne_debt_aggregates_only_evidence_backed_matches() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.path.join("src")).unwrap();
    std::fs::write(
        fixture.path.join("src/lib.rs"),
        b"// TODO: replace fixture\n",
    )
    .unwrap();
    std::fs::write(fixture.path.join("src/clean.rs"), b"pub fn clean() {}\n").unwrap();
    let controller = ExecutionController::new(fixture.root.clone());
    let intent = TaskIntent::new("debt")
        .unwrap()
        .with_gene(GeneId::new("ariadne.debt").unwrap());

    let summary = controller
        .run_at(intent, fixture.session(), Timestamp::from_unix_seconds(10))
        .unwrap();

    assert_eq!(summary.status(), &RunStatus::Completed);
    assert_eq!(summary.receipts().len(), 4);
    let output = String::from_utf8(summary.output().unwrap().to_vec()).unwrap();
    assert!(output.contains("TODO:\nsrc/lib.rs"));
    assert!(!output.contains("src/clean.rs"));
}

#[test]
fn athena_guide_completes_without_requesting_an_effect() {
    let fixture = Fixture::new();
    let controller = ExecutionController::new(fixture.root.clone());
    let intent = TaskIntent::new("guide")
        .unwrap()
        .with_gene(GeneId::new("athena.guide").unwrap());

    let summary = controller
        .run_at(intent, fixture.session(), Timestamp::from_unix_seconds(10))
        .unwrap();

    assert_eq!(summary.status(), &RunStatus::Completed);
    assert!(summary.receipts().is_empty());
    assert!(String::from_utf8_lossy(summary.output().unwrap()).contains("Daedalus"));
}

struct Fixture {
    path: PathBuf,
    root: WorkspaceRoot,
}

struct PanickingProvider {
    manifest: ProviderManifest,
}

struct StubProvider {
    manifest: ProviderManifest,
    result: Mutex<Option<Result<ModelResponse, ProviderError>>>,
    requests: Arc<Mutex<Vec<ModelRequest>>>,
}

impl StubProvider {
    fn new(
        id: &str,
        model: &str,
        result: Result<ModelResponse, ProviderError>,
        requests: Arc<Mutex<Vec<ModelRequest>>>,
    ) -> Self {
        Self {
            manifest: ProviderManifest::new(
                id,
                id,
                format!("https://{id}.example.test/v1"),
                model,
                format!("PANDORA_{}_KEY", id.replace('-', "_").to_uppercase()),
            )
            .unwrap(),
            result: Mutex::new(Some(result)),
            requests,
        }
    }
}

impl Provider for StubProvider {
    fn manifest(&self) -> &ProviderManifest {
        &self.manifest
    }

    fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        self.requests.lock().unwrap().push(request);
        self.result
            .lock()
            .unwrap()
            .take()
            .expect("stub provider called more than once")
    }
}

impl PanickingProvider {
    fn new() -> Self {
        Self {
            manifest: ProviderManifest::new(
                "test-provider",
                "Test provider",
                "http://127.0.0.1:1/v1",
                "test-model",
                "PANDORA_TEST_PROVIDER_KEY",
            )
            .unwrap(),
        }
    }
}

impl Provider for PanickingProvider {
    fn manifest(&self) -> &ProviderManifest {
        &self.manifest
    }

    fn complete(&self, _request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        panic!("provider must not be called after hook denial")
    }
}

impl Fixture {
    fn new() -> Self {
        let path = crate::test_support::new_temp_dir("pandora-controller-test").unwrap();
        std::fs::write(path.join("README.md"), b"fixture\n").unwrap();
        let root = WorkspaceRoot::new(&path).unwrap();
        Self { path, root }
    }

    fn session(&self) -> Session {
        Session::new(
            SessionId::new("session-1").unwrap(),
            PrincipalId::new("principal-1").unwrap(),
            TenantId::new("tenant-1").unwrap(),
            WorkspaceId::new("workspace-1").unwrap(),
            Timestamp::from_unix_seconds(1),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
