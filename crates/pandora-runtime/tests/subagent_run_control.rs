use pandora_harnesses::HarnessCatalog;
use pandora_provider::{
    ModelRequest, ModelResponse, Provider, ProviderError, ProviderManifest, TokenUsage, ToolCall,
};
use pandora_runtime::executors::WorkspaceRoot;
use pandora_runtime::{
    AgentCheckpoint, AgentCheckpointKind, AgentControlStop, AgentLoop, AgentLoopError,
    AgentRunControl, AgentRunRequest, AgentRunSummary, ExecutionController, RunStatus,
    RuntimeError, SubagentPreparation, SubagentRunControl, SubagentScope, SubagentStore,
};
use pandora_types::{
    Capability, EffectOutcome, EffectReceipt, ExecutionId, JobId, Operation, PackageCompatibility,
    PackageDependency, PackageKind, PackageManifest, PermitId, PolicyContext, PrincipalId,
    ReceiptId, RequestDigest, Session, SessionId, SubagentBudgets, SubagentId, SubagentRequest,
    TenantId, Timestamp, TrustEvidence, WorkspaceId, hash_artifact,
};
use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(1);

#[test]
fn trusted_harness_binding_controls_tool_execution_intent() {
    let fixture = Fixture::new();
    let provider = CountingProvider::new(vec![
        tool_response(TokenUsage::new(2, 1)),
        direct_response(TokenUsage::new(2, 1)),
    ]);
    let package = PackageManifest::new(
        "example/domain",
        "1.0.0",
        PackageKind::DomainHarness,
        "example",
        hash_artifact(b"example domain"),
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
        WorkspaceRoot::new(&fixture.path).unwrap(),
        PolicyContext::new(
            1,
            [Capability::FilesystemRead, Capability::ProviderInvoke],
            [],
        ),
        harnesses,
    );

    let summary = loop_engine()
        .run_with_request(
            &provider,
            &controller,
            fixture
                .request()
                .with_trusted_harness(pandora_types::HarnessId::new("example/domain").unwrap()),
        )
        .unwrap();

    assert_eq!(summary.runs().len(), 1);
    assert_eq!(
        summary.runs()[0].selected_harness().as_str(),
        "example/domain"
    );
    assert_eq!(
        provider.advertised_tools(),
        vec![
            vec!["workspace.read".to_owned()],
            vec!["workspace.read".to_owned()]
        ]
    );
}

#[test]
fn ordinary_run_advertises_the_complete_default_tool_catalog() {
    let fixture = Fixture::new();
    let provider = CountingProvider::new(vec![direct_response(TokenUsage::new(2, 1))]);

    loop_engine()
        .run_with_request(&provider, &fixture.controller, fixture.request())
        .unwrap();

    assert_eq!(
        provider.advertised_tools(),
        vec![vec![
            "accessibility.evidence".to_owned(),
            "argus.review".to_owned(),
            "ariadne.debt".to_owned(),
            "citation.inventory".to_owned(),
            "daedalus.audit".to_owned(),
            "design.compare".to_owned(),
            "design.inspect".to_owned(),
            "design.inventory".to_owned(),
            "design.tokens".to_owned(),
            "evidence.inventory".to_owned(),
            "evidence.search".to_owned(),
            "hephaestus.measure".to_owned(),
            "source.compare".to_owned(),
            "source.read".to_owned(),
            "workspace.build".to_owned(),
            "workspace.diff".to_owned(),
            "workspace.format".to_owned(),
            "workspace.lint".to_owned(),
            "workspace.log".to_owned(),
            "workspace.patch".to_owned(),
            "workspace.read".to_owned(),
            "workspace.search".to_owned(),
            "workspace.status".to_owned(),
            "workspace.test".to_owned(),
            "workspace.verify".to_owned(),
        ]]
    );
}

#[test]
fn unknown_trusted_harness_stops_before_provider_invocation() {
    let fixture = Fixture::new();
    let provider = CountingProvider::new(vec![direct_response(TokenUsage::new(2, 1))]);

    let error = loop_engine()
        .run_with_request(
            &provider,
            &fixture.controller,
            fixture
                .request()
                .with_trusted_harness(pandora_types::HarnessId::new("unknown-domain").unwrap()),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        AgentLoopError::Execution(RuntimeError::UnsupportedHarness(_))
    ));
    assert_eq!(provider.calls(), 0);
    assert!(provider.advertised_tools().is_empty());
}

#[test]
fn non_runnable_trusted_harness_stops_before_provider_invocation() {
    let fixture = Fixture::new();
    let provider = CountingProvider::new(vec![direct_response(TokenUsage::new(2, 1))]);

    let error = loop_engine()
        .run_with_request(
            &provider,
            &fixture.controller,
            fixture
                .request()
                .with_trusted_harness(pandora_types::HarnessId::new("core-source").unwrap()),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        AgentLoopError::Execution(RuntimeError::NonExecutableHarness { .. })
    ));
    assert_eq!(provider.calls(), 0);
    assert!(provider.advertised_tools().is_empty());
}

#[test]
fn store_backed_control_honors_persisted_cancellation() {
    let fixture = Fixture::new();
    let store_fixture = RunControlStoreFixture::new();
    let record = store_fixture.queued_subagent(subagent_budgets(100, 60));
    store_fixture
        .store()
        .request_cancel(
            record.id(),
            record.scope(),
            Timestamp::from_unix_seconds(30),
        )
        .unwrap();
    let control = SubagentRunControl::new(
        store_fixture.store(),
        record.id(),
        record.scope(),
        record.request().budgets(),
    );
    let provider = CountingProvider::new(vec![direct_response(TokenUsage::default())]);

    let error = loop_engine()
        .run_with_request(
            &provider,
            &fixture.controller,
            fixture.request().with_control(&control),
        )
        .unwrap_err();

    let (reason, _) = expect_controlled_stop(error);
    assert_eq!(reason, AgentControlStop::Cancelled);
    assert_eq!(provider.calls(), 0);
}

#[test]
fn store_backed_control_fails_closed_when_state_cannot_be_read() {
    let fixture = Fixture::new();
    let store_fixture = RunControlStoreFixture::new();
    let missing_id = SubagentId::new("missing-subagent").unwrap();
    let budgets = subagent_budgets(100, 60);
    let control = SubagentRunControl::new(
        store_fixture.store(),
        &missing_id,
        &store_fixture.scope,
        &budgets,
    );
    let provider = CountingProvider::new(vec![direct_response(TokenUsage::default())]);

    let error = loop_engine()
        .run_with_request(
            &provider,
            &fixture.controller,
            fixture.request().with_control(&control),
        )
        .unwrap_err();

    let (reason, _) = expect_controlled_stop(error);
    assert_eq!(reason, AgentControlStop::CancellationStateUnavailable);
    assert_eq!(provider.calls(), 0);
}

#[test]
fn store_backed_control_enforces_persisted_duration_budget() {
    let fixture = Fixture::new();
    let store_fixture = RunControlStoreFixture::new();
    let record = store_fixture.prepared_subagent(subagent_budgets(100, 1));
    let control = SubagentRunControl::new(
        store_fixture.store(),
        record.id(),
        record.scope(),
        record.request().budgets(),
    );
    let provider = CountingProvider::new(vec![direct_response(TokenUsage::default())]);
    std::thread::sleep(Duration::from_millis(1_100));

    let error = loop_engine()
        .run_with_request(
            &provider,
            &fixture.controller,
            fixture.request().with_control(&control),
        )
        .unwrap_err();

    let (reason, _) = expect_controlled_stop(error);
    assert_eq!(reason, AgentControlStop::DurationBudgetExceeded);
    assert_eq!(provider.calls(), 0);
}

#[test]
fn store_backed_control_stops_at_the_exact_token_ceiling() {
    let fixture = Fixture::new();
    let store_fixture = RunControlStoreFixture::new();
    let record = store_fixture.prepared_subagent(subagent_budgets(10, 60));
    let control = SubagentRunControl::new(
        store_fixture.store(),
        record.id(),
        record.scope(),
        record.request().budgets(),
    );
    let provider = CountingProvider::new(vec![direct_response(TokenUsage::new(6, 4))]);

    let error = loop_engine()
        .run_with_request(
            &provider,
            &fixture.controller,
            fixture.request().with_control(&control),
        )
        .unwrap_err();

    let (reason, summary) = expect_controlled_stop(error);
    assert_eq!(reason, AgentControlStop::TokenBudgetExceeded);
    assert_eq!(provider.calls(), 1);
    assert_eq!(summary.turns(), 1);
    assert_eq!(summary.usage().total_tokens(), 10);
    assert_eq!(summary.provider_receipts().len(), 1);
}

#[test]
fn store_backed_control_aggregates_tokens_across_provider_calls() {
    let fixture = Fixture::new();
    let store_fixture = RunControlStoreFixture::new();
    let record = store_fixture.prepared_subagent(subagent_budgets(10, 60));
    let control = SubagentRunControl::new(
        store_fixture.store(),
        record.id(),
        record.scope(),
        record.request().budgets(),
    );
    let provider = CountingProvider::new(vec![
        tool_response(TokenUsage::new(4, 2)),
        direct_response(TokenUsage::new(2, 2)),
    ]);

    let error = loop_engine()
        .run_with_request(
            &provider,
            &fixture.controller,
            fixture.request().with_control(&control),
        )
        .unwrap_err();

    let (reason, summary) = expect_controlled_stop(error);
    assert_eq!(reason, AgentControlStop::TokenBudgetExceeded);
    assert_eq!(provider.calls(), 2);
    assert_eq!(summary.turns(), 2);
    assert_eq!(summary.usage().total_tokens(), 10);
    assert_eq!(summary.provider_receipts().len(), 2);
    assert_eq!(summary.runs().len(), 1);
}

#[test]
fn cancellation_stops_before_the_next_provider_call() {
    let fixture = Fixture::new();
    let provider = CountingProvider::new(vec![direct_response(TokenUsage::default())]);
    let control = ScriptedControl::stop_at(AgentCheckpointKind::BeforeProvider);

    let error = loop_engine()
        .run_with_request(
            &provider,
            &fixture.controller,
            fixture.request().with_control(&control),
        )
        .unwrap_err();

    let (reason, summary) = expect_controlled_stop(error);
    assert_eq!(reason, AgentControlStop::Cancelled);
    assert_eq!(provider.calls(), 0);
    assert_eq!(summary.turns(), 0);
    assert_eq!(summary.usage().total_tokens(), 0);
    assert!(summary.provider_receipts().is_empty());
}

#[test]
fn token_ceiling_stops_after_recording_the_completed_call() {
    let fixture = Fixture::new();
    let provider = CountingProvider::new(vec![direct_response(TokenUsage::new(6, 5))]);
    let control = BudgetControl::new(10, Duration::from_secs(60));

    let error = loop_engine()
        .run_with_request(
            &provider,
            &fixture.controller,
            fixture.request().with_control(&control),
        )
        .unwrap_err();

    let (reason, summary) = expect_controlled_stop(error);
    assert_eq!(reason, AgentControlStop::TokenBudgetExceeded);
    assert_eq!(provider.calls(), 1);
    assert_eq!(summary.turns(), 1);
    assert_eq!(summary.usage().total_tokens(), 11);
    assert_eq!(summary.provider_receipts().len(), 1);
}

#[test]
fn cancellation_stops_before_effect_authorization() {
    let fixture = Fixture::new();
    let provider = CountingProvider::new(vec![tool_response(TokenUsage::new(2, 1))]);
    let control = ScriptedControl::stop_at(AgentCheckpointKind::BeforeEffectAuthorization);

    let error = loop_engine()
        .run_with_request(
            &provider,
            &fixture.controller,
            fixture.request().with_control(&control),
        )
        .unwrap_err();

    let (reason, summary) = expect_controlled_stop(error);
    assert_eq!(reason, AgentControlStop::Cancelled);
    assert_eq!(provider.calls(), 1);
    assert_eq!(summary.usage().total_tokens(), 3);
    assert_eq!(summary.provider_receipts().len(), 1);
    assert!(summary.runs().is_empty());
}

#[test]
fn completed_effect_is_retained_when_after_effect_checkpoint_stops() {
    let fixture = Fixture::new();
    let provider = CountingProvider::new(vec![tool_response(TokenUsage::new(2, 1))]);
    let control = ScriptedControl::stop_at(AgentCheckpointKind::AfterEffect);

    let error = loop_engine()
        .run_with_request(
            &provider,
            &fixture.controller,
            fixture.request().with_control(&control),
        )
        .unwrap_err();

    let (reason, summary) = expect_controlled_stop(error);
    assert_eq!(reason, AgentControlStop::Cancelled);
    assert_eq!(summary.usage().total_tokens(), 3);
    assert_eq!(summary.provider_receipts().len(), 1);
    assert_eq!(summary.runs().len(), 1);
    assert_eq!(summary.runs()[0].output(), Some(b"fixture\n".as_slice()));
}

#[test]
fn approval_required_is_not_replaced_by_an_after_effect_stop() {
    let fixture = Fixture::new();
    let provider = CountingProvider::new(vec![patch_response(TokenUsage::new(2, 1))]);
    let control = ScriptedControl::stop_at(AgentCheckpointKind::AfterEffect);
    let controller = ExecutionController::with_policy(
        WorkspaceRoot::new(&fixture.path).unwrap(),
        PolicyContext::new(
            1,
            [
                Capability::FilesystemRead,
                Capability::FilesystemWrite,
                Capability::ProviderInvoke,
            ],
            [Operation::Write],
        ),
    );

    let error = loop_engine()
        .run_with_request(
            &provider,
            &controller,
            fixture.request().with_control(&control),
        )
        .unwrap_err();

    let AgentLoopError::ApprovalRequired { summary, .. } = error else {
        panic!("expected approval required, got {error:?}");
    };
    assert_eq!(summary.runs().len(), 1);
    assert!(matches!(
        summary.runs()[0].status(),
        RunStatus::ApprovalRequired { .. }
    ));
    assert!(
        !control
            .observed()
            .contains(&AgentCheckpointKind::AfterEffect)
    );
    assert_eq!(
        fs::read(fixture.path.join("README.md")).unwrap(),
        b"fixture\n"
    );
}

#[test]
fn zero_reported_token_usage_does_not_exhaust_the_budget() {
    let fixture = Fixture::new();
    let provider = CountingProvider::new(vec![direct_response(TokenUsage::default())]);
    let control = BudgetControl::new(1, Duration::from_secs(60));

    let result = loop_engine()
        .run_with_request(
            &provider,
            &fixture.controller,
            fixture.request().with_control(&control),
        )
        .unwrap();

    assert_eq!(result.final_text(), "done");
    assert_eq!(result.usage().total_tokens(), 0);
    assert_eq!(provider.calls(), 1);
}

#[test]
fn duration_expiry_stops_before_work_continues() {
    let fixture = Fixture::new();
    let provider = CountingProvider::new(vec![direct_response(TokenUsage::default())]);
    let control = BudgetControl::new(10, Duration::ZERO);

    let error = loop_engine()
        .run_with_request(
            &provider,
            &fixture.controller,
            fixture.request().with_control(&control),
        )
        .unwrap_err();

    let (reason, summary) = expect_controlled_stop(error);
    assert_eq!(reason, AgentControlStop::DurationBudgetExceeded);
    assert_eq!(provider.calls(), 0);
    assert_eq!(summary.turns(), 0);
}

#[test]
fn existing_run_path_remains_uncontrolled() {
    let fixture = Fixture::new();
    let provider = CountingProvider::new(vec![direct_response(TokenUsage::new(4, 2))]);

    let result = loop_engine()
        .run(
            &provider,
            &fixture.controller,
            fixture.session(),
            "Answer directly",
            Timestamp::from_unix_seconds(10),
        )
        .unwrap();

    assert_eq!(result.final_text(), "done");
    assert_eq!(result.usage().total_tokens(), 6);
    assert_eq!(provider.calls(), 1);
}

struct ScriptedControl {
    stop_at: AgentCheckpointKind,
    observed: Mutex<Vec<AgentCheckpointKind>>,
}

impl ScriptedControl {
    fn stop_at(stop_at: AgentCheckpointKind) -> Self {
        Self {
            stop_at,
            observed: Mutex::new(Vec::new()),
        }
    }

    fn observed(&self) -> Vec<AgentCheckpointKind> {
        self.observed.lock().unwrap().clone()
    }
}

impl AgentRunControl for ScriptedControl {
    fn checkpoint(&self, checkpoint: AgentCheckpoint<'_>) -> Result<(), AgentControlStop> {
        self.observed.lock().unwrap().push(checkpoint.kind());
        if checkpoint.kind() == self.stop_at {
            return Err(AgentControlStop::Cancelled);
        }
        Ok(())
    }
}

struct BudgetControl {
    max_tokens: u32,
    max_duration: Duration,
    started_at: Instant,
}

impl BudgetControl {
    fn new(max_tokens: u32, max_duration: Duration) -> Self {
        Self {
            max_tokens,
            max_duration,
            started_at: Instant::now(),
        }
    }
}

impl AgentRunControl for BudgetControl {
    fn checkpoint(&self, checkpoint: AgentCheckpoint<'_>) -> Result<(), AgentControlStop> {
        if checkpoint.usage().total_tokens() >= self.max_tokens {
            return Err(AgentControlStop::TokenBudgetExceeded);
        }
        if self.started_at.elapsed() >= self.max_duration {
            return Err(AgentControlStop::DurationBudgetExceeded);
        }
        Ok(())
    }
}

struct CountingProvider {
    manifest: ProviderManifest,
    responses: Mutex<VecDeque<ModelResponse>>,
    advertised_tools: Mutex<Vec<Vec<String>>>,
    calls: AtomicU32,
}

impl CountingProvider {
    fn new(responses: Vec<ModelResponse>) -> Self {
        Self {
            manifest: ProviderManifest::new(
                "run-control-provider",
                "Run control provider",
                "http://127.0.0.1:1/v1",
                "model-a",
                "PANDORA_PROVIDER_KEY",
            )
            .unwrap(),
            responses: Mutex::new(responses.into()),
            advertised_tools: Mutex::new(Vec::new()),
            calls: AtomicU32::new(0),
        }
    }

    fn calls(&self) -> u32 {
        self.calls.load(Ordering::Relaxed)
    }

    fn advertised_tools(&self) -> Vec<Vec<String>> {
        self.advertised_tools.lock().unwrap().clone()
    }
}

impl Provider for CountingProvider {
    fn manifest(&self) -> &ProviderManifest {
        &self.manifest
    }

    fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        self.advertised_tools.lock().unwrap().push(
            request
                .tools()
                .iter()
                .map(|tool| tool.name().to_owned())
                .collect(),
        );
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or(ProviderError::InvalidResponse)
    }
}

struct Fixture {
    path: PathBuf,
    controller: ExecutionController,
}

impl Fixture {
    fn new() -> Self {
        let path = unique_temp_dir("pandora-subagent-run-control");
        fs::write(path.join("README.md"), b"fixture\n").unwrap();
        let controller = ExecutionController::new(WorkspaceRoot::new(&path).unwrap());
        Self { path, controller }
    }

    fn session(&self) -> Session {
        Session::new(
            SessionId::new("session-run-control-1").unwrap(),
            PrincipalId::new("principal-run-control-1").unwrap(),
            TenantId::new("tenant-run-control-1").unwrap(),
            WorkspaceId::new("workspace-run-control-1").unwrap(),
            Timestamp::from_unix_seconds(1),
        )
    }

    fn request(&self) -> AgentRunRequest<'static> {
        AgentRunRequest::new(
            self.session(),
            Vec::new(),
            "Answer directly",
            Timestamp::from_unix_seconds(10),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct RunControlStoreFixture {
    path: PathBuf,
    store: Option<SubagentStore>,
    id: SubagentId,
    scope: SubagentScope,
}

impl RunControlStoreFixture {
    fn new() -> Self {
        let path = unique_temp_dir("pandora-subagent-run-control-store");
        let store = SubagentStore::open(path.join("jobs.sqlite3")).unwrap();
        Self {
            path,
            store: Some(store),
            id: SubagentId::new("subagent-run-control-1").unwrap(),
            scope: SubagentScope::new(
                PrincipalId::new("principal-run-control-1").unwrap(),
                TenantId::new("tenant-run-control-1").unwrap(),
                WorkspaceId::new("workspace-run-control-1").unwrap(),
            ),
        }
    }

    fn store(&self) -> &SubagentStore {
        self.store.as_ref().unwrap()
    }

    fn prepared_subagent(&self, budgets: SubagentBudgets) -> pandora_runtime::SubagentRecord {
        self.store()
            .prepare(SubagentPreparation::new(
                self.id.clone(),
                JobId::new("job-run-control-1").unwrap(),
                self.scope.clone(),
                SessionId::new("child-session-run-control-1").unwrap(),
                ExecutionId::new("child-execution-run-control-1").unwrap(),
                SubagentRequest::new(
                    SessionId::new("parent-session-run-control-1").unwrap(),
                    ExecutionId::new("parent-execution-run-control-1").unwrap(),
                    1,
                    "a".repeat(40),
                    "execute the isolated task",
                    budgets,
                )
                .unwrap(),
                self.path.join("repository"),
                self.path.join("worktrees").join("subagent-run-control-1"),
                Some("provider-sha256:abc123".to_owned()),
                Some("harness-sha256:def456".to_owned()),
                Timestamp::from_unix_seconds(10),
            ))
            .unwrap()
    }

    fn queued_subagent(&self, budgets: SubagentBudgets) -> pandora_runtime::SubagentRecord {
        let prepared = self.prepared_subagent(budgets);
        self.store()
            .queue(
                prepared.id(),
                &self.scope,
                &EffectReceipt::new(
                    ReceiptId::new("receipt-run-control-1").unwrap(),
                    PermitId::new("permit-run-control-1").unwrap(),
                    RequestDigest::new("request-run-control-1").unwrap(),
                    Timestamp::from_unix_seconds(20),
                    EffectOutcome::Succeeded,
                ),
                Timestamp::from_unix_seconds(20),
            )
            .unwrap()
    }
}

impl Drop for RunControlStoreFixture {
    fn drop(&mut self) {
        self.store.take();
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn loop_engine() -> AgentLoop {
    AgentLoop::new(4, 4).unwrap()
}

fn direct_response(usage: TokenUsage) -> ModelResponse {
    ModelResponse::new("done", Vec::new(), usage)
}

fn tool_response(usage: TokenUsage) -> ModelResponse {
    ModelResponse::new(
        "",
        vec![
            ToolCall::new(
                "call-read",
                "workspace.read",
                serde_json::json!({"path": "README.md"}),
            )
            .unwrap(),
        ],
        usage,
    )
}

fn patch_response(usage: TokenUsage) -> ModelResponse {
    ModelResponse::new(
        "",
        vec![
            ToolCall::new(
                "call-patch",
                "workspace.patch",
                serde_json::json!({"path": "README.md", "content": "changed"}),
            )
            .unwrap(),
        ],
        usage,
    )
}

fn subagent_budgets(max_tokens: u32, max_duration_seconds: u64) -> SubagentBudgets {
    SubagentBudgets::new(4, 4, max_tokens, max_duration_seconds, 1, 8_192).unwrap()
}

fn expect_controlled_stop(error: AgentLoopError) -> (AgentControlStop, AgentRunSummary) {
    let AgentLoopError::ControlledStop { reason, summary } = error else {
        panic!("expected controlled stop, got {error:?}");
    };
    (reason, summary)
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "{prefix}-{}-{timestamp}-{sequence}",
        std::process::id()
    ));
    fs::create_dir(&path).unwrap();
    path
}
