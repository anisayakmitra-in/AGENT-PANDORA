use pandora_provider::{
    ModelRequest, ModelResponse, Provider, ProviderError, ProviderManifest, TokenUsage, ToolCall,
};
use pandora_runtime::executors::WorkspaceRoot;
use pandora_runtime::{
    AgentCheckpoint, AgentCheckpointKind, AgentControlStop, AgentLoop, AgentLoopError,
    AgentRunControl, AgentRunRequest, AgentRunSummary, ExecutionController,
};
use pandora_types::{PrincipalId, Session, SessionId, TenantId, Timestamp, WorkspaceId};
use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(1);

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
}

impl ScriptedControl {
    fn stop_at(stop_at: AgentCheckpointKind) -> Self {
        Self { stop_at }
    }
}

impl AgentRunControl for ScriptedControl {
    fn checkpoint(&self, checkpoint: AgentCheckpoint<'_>) -> Result<(), AgentControlStop> {
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
        if checkpoint.usage().total_tokens() > self.max_tokens {
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
            calls: AtomicU32::new(0),
        }
    }

    fn calls(&self) -> u32 {
        self.calls.load(Ordering::Relaxed)
    }
}

impl Provider for CountingProvider {
    fn manifest(&self) -> &ProviderManifest {
        &self.manifest
    }

    fn complete(&self, _request: ModelRequest) -> Result<ModelResponse, ProviderError> {
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
