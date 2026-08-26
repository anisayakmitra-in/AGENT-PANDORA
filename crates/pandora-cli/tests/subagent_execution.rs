#![allow(dead_code)]

#[path = "../src/commands/mod.rs"]
mod commands;
#[path = "../src/output.rs"]
mod output;

use commands::subagent_run::{TrustedSubagentRun, execute_trusted_subagent};
use pandora_harnesses::{HarnessCatalog, canonical_harness_binding_digest};
use pandora_provider::ChatMessage;
use pandora_runtime::config::{ConfigOverrides, RuntimeConfig};
use pandora_runtime::sessions::SessionStore;
use pandora_runtime::{
    AgentCheckpoint, AgentCheckpointKind, AgentControlStop, AgentRunControl, ApprovalRequest,
    ApprovalStore, ClaimedSubagent, PackageStore, SubagentPreparation, SubagentScope,
    SubagentStore,
};
use pandora_types::{
    EffectOutcome, EffectReceipt, ExecutionId, GeneId, HarnessId, JobId, JobWorkerId,
    PackageCompatibility, PackageDependency, PackageKind, PackageManifest, PermitId, PrincipalId,
    ReceiptId, RequestDigest, Session, SessionId, SubagentBudgets, SubagentHarnessBinding,
    SubagentId, SubagentRequest, TenantId, Timestamp, TrustEvidence, WorkspaceId, hash_artifact,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

const PROVIDER_NAME: &str = "child-provider";
const PROVIDER_MODEL: &str = "child-model";
const PROVIDER_KEY_ENV: &str = "PATH";
const HARNESS_ID: &str = "coding-domain";
const HARNESS_VERSION: &str = "0.1.0";
const CUSTOM_HARNESS_ID: &str = "example/domain";
const CUSTOM_HARNESS_VERSION: &str = "1.0.0";

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

#[test]
fn child_reads_from_persisted_worktree_not_parent_cli_workspace() {
    let fixture = CliSubagentFixture::claimed();
    fs::write(fixture.parent_workspace().join("identity.txt"), "parent").unwrap();
    fs::write(fixture.child_worktree().join("identity.txt"), "child").unwrap();
    fixture.provider_returns_read_then_final("identity.txt");

    let result = execute_trusted_subagent(fixture.trusted_run()).unwrap();

    assert_eq!(result["status"], "completed");
    assert!(fixture.provider_request_text().contains("child"));
    assert!(!fixture.provider_request_text().contains("parent"));
}

#[test]
fn provider_or_harness_binding_change_fails_before_execution() {
    let fixture = CliSubagentFixture::claimed_with_changed_provider_binding();

    let error = execute_trusted_subagent(fixture.trusted_run()).unwrap_err();

    assert_eq!(error.code, "subagent_binding_changed");
    assert_eq!(fixture.provider_calls(), 0);
}

#[test]
fn absent_provider_binding_fails_before_execution() {
    let fixture = CliSubagentFixture::claimed_without_provider_binding();
    fixture.observe_provider_if_called();

    let error = execute_trusted_subagent(fixture.trusted_run()).unwrap_err();

    assert_eq!(error.code, "subagent_binding_changed");
    assert_eq!(fixture.provider_calls(), 0);
}

#[test]
fn changed_worktree_head_fails_before_execution() {
    let fixture = CliSubagentFixture::claimed();
    fixture.advance_child_head();
    fixture.observe_provider_if_called();

    let error = execute_trusted_subagent(fixture.trusted_run()).unwrap_err();

    assert_eq!(error.code, "subagent_worktree_changed");
    assert_eq!(fixture.provider_calls(), 0);
}

#[test]
fn harness_manifest_digest_drift_fails_before_execution() {
    let fixture = CliSubagentFixture::claimed_with_changed_harness_binding();
    fixture.observe_provider_if_called();

    let error = execute_trusted_subagent(fixture.trusted_run()).unwrap_err();

    assert_eq!(error.code, "subagent_binding_changed");
    assert_eq!(fixture.provider_calls(), 0);
}

#[test]
fn child_session_uses_predetermined_identity_and_isolated_workspace() {
    let fixture = CliSubagentFixture::claimed();
    fixture.provider_returns_final();

    execute_trusted_subagent(fixture.trusted_run()).unwrap();
    let snapshot = fixture.child_snapshot();

    assert_eq!(snapshot.session().id(), fixture.record.child_session_id());
    assert_eq!(
        snapshot.session().principal_id(),
        fixture.record.scope().principal_id()
    );
    assert_eq!(
        snapshot.session().tenant_id(),
        fixture.record.scope().tenant_id()
    );
    assert_ne!(
        snapshot.session().workspace_id(),
        fixture.record.scope().workspace_id()
    );
}

#[test]
fn first_provider_request_excludes_parent_transcript_and_approval() {
    let fixture = CliSubagentFixture::claimed();
    fixture.seed_parent_state();
    fixture.provider_returns_final();

    execute_trusted_subagent(fixture.trusted_run()).unwrap();

    let request = fixture.first_provider_request_text();
    assert!(!request.contains("parent-transcript-secret"));
    assert!(!request.contains("parent-approval-secret"));
}

#[test]
fn trusted_custom_harness_is_used_for_child_tool_execution() {
    let fixture = CliSubagentFixture::claimed_with_custom_harness();
    fixture.provider_returns_read_then_final("identity.txt");
    fs::write(fixture.child_worktree().join("identity.txt"), "child").unwrap();

    execute_trusted_subagent(fixture.trusted_run()).unwrap();

    let snapshot = fixture.child_snapshot();
    assert_eq!(snapshot.l1_evidence_count(), 1);
    assert!(snapshot.events().iter().any(|event| {
        serde_json::to_value(event).unwrap()["context"]["harness_id"] == CUSTOM_HARNESS_ID
    }));
}

#[test]
fn controlled_stop_after_effect_preserves_child_audit_state() {
    let fixture = CliSubagentFixture::claimed();
    fs::write(fixture.child_worktree().join("identity.txt"), "child").unwrap();
    fixture.provider_returns_tool_call("identity.txt");
    let control = StopAfterEffectControl;

    let error = execute_trusted_subagent(TrustedSubagentRun {
        config: &fixture.config,
        record: &fixture.record,
        store: &fixture.sessions,
        approval_store: &fixture.approvals,
        control: &control,
    })
    .unwrap_err();
    let snapshot = fixture.child_snapshot();

    assert_eq!(error.code, "agent_controlled_stop");
    assert_eq!(error.details["reason"], "cancelled");
    assert_eq!(error.details["runs"], 1);
    assert_eq!(error.details["provider_calls"], 1);
    assert_eq!(
        error.details["provider_receipts"].as_array().unwrap().len(),
        1
    );
    assert!(error.details["context"]["included"].is_array());
    assert_eq!(error.details["efficiency_recorded"], true);
    assert_eq!(error.details["memory_evidence_recorded"], 1);
    assert_eq!(snapshot.l1_evidence_count(), 1);
    assert_eq!(snapshot.evaluations().len(), 1);
    assert!(!snapshot.agent_messages().is_empty());
    assert!(
        fixture
            .approvals
            .list(fixture.record.scope().principal_id())
            .unwrap()
            .is_empty()
    );
}

struct CliSubagentFixture {
    root: PathBuf,
    parent_workspace: PathBuf,
    child_worktree: PathBuf,
    config: RuntimeConfig,
    record: ClaimedSubagent,
    sessions: SessionStore,
    approvals: ApprovalStore,
    control: NoopControl,
    provider_address: SocketAddr,
    provider_calls: Arc<AtomicUsize>,
    provider_stop: Arc<AtomicBool>,
    listener: Mutex<Option<TcpListener>>,
    server: Mutex<Option<JoinHandle<Vec<Value>>>>,
    provider_requests: Mutex<Option<Vec<Value>>>,
}

struct FixtureOptions {
    bind_provider: bool,
    changed_provider_binding: bool,
    changed_harness_binding: bool,
    custom_harness: bool,
}

impl FixtureOptions {
    fn default() -> Self {
        Self {
            bind_provider: true,
            changed_provider_binding: false,
            changed_harness_binding: false,
            custom_harness: false,
        }
    }
}

impl CliSubagentFixture {
    fn claimed() -> Self {
        Self::new(FixtureOptions::default())
    }

    fn claimed_with_changed_provider_binding() -> Self {
        let fixture = Self::new(FixtureOptions {
            changed_provider_binding: true,
            ..FixtureOptions::default()
        });
        fixture.observe_provider_if_called();
        fixture
    }

    fn claimed_without_provider_binding() -> Self {
        Self::new(FixtureOptions {
            bind_provider: false,
            ..FixtureOptions::default()
        })
    }

    fn claimed_with_changed_harness_binding() -> Self {
        Self::new(FixtureOptions {
            changed_harness_binding: true,
            ..FixtureOptions::default()
        })
    }

    fn claimed_with_custom_harness() -> Self {
        Self::new(FixtureOptions {
            custom_harness: true,
            ..FixtureOptions::default()
        })
    }

    fn new(options: FixtureOptions) -> Self {
        let root = unique_temp_dir("pandora-cli-subagent-execution");
        let parent_workspace = root.join("repository");
        let child_worktree = root.join("worktrees").join("child");
        fs::create_dir_all(&parent_workspace).unwrap();
        fs::create_dir_all(child_worktree.parent().unwrap()).unwrap();
        git(&parent_workspace, &["init"]);
        git(
            &parent_workspace,
            &["config", "user.email", "pandora@example.invalid"],
        );
        git(&parent_workspace, &["config", "user.name", "Pandora Test"]);
        fs::write(parent_workspace.join("tracked.txt"), "tracked\n").unwrap();
        git(&parent_workspace, &["add", "tracked.txt"]);
        git(&parent_workspace, &["commit", "-m", "initial"]);
        let exact_commit = git_output(&parent_workspace, &["rev-parse", "HEAD"]);
        git_worktree(&parent_workspace, &child_worktree, &exact_commit);

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let provider_address = listener.local_addr().unwrap();
        let provider_url = format!("http://{provider_address}/v1");
        let config_path = root.join("config.json");
        fs::write(
            &config_path,
            serde_json::to_vec(&json!({
                "providers": {
                    PROVIDER_NAME: {
                        "base_url": provider_url,
                        "model": PROVIDER_MODEL,
                        "api_key_env": PROVIDER_KEY_ENV,
                    }
                },
                "active_provider": PROVIDER_NAME,
            }))
            .unwrap(),
        )
        .unwrap();
        let data_dir = root.join("data");
        let config = RuntimeConfig::from_sources(
            &ConfigOverrides::default(),
            &BTreeMap::new(),
            &config_path,
            data_dir.clone(),
            parent_workspace.clone(),
        )
        .unwrap();
        let sessions = SessionStore::open(data_dir.join("sessions.sqlite3")).unwrap();
        let approvals = ApprovalStore::open(data_dir.join("sessions.sqlite3")).unwrap();

        let (harness_id, harness_version, harness_digest) = if options.custom_harness {
            let artifact = b"custom domain profile";
            let package = PackageManifest::new(
                CUSTOM_HARNESS_ID,
                CUSTOM_HARNESS_VERSION,
                PackageKind::DomainHarness,
                "example",
                hash_artifact(artifact),
                vec![PackageDependency::new("workspace.read", "0.1.0", false).unwrap()],
                PackageCompatibility::new(concat!("pandora>=", env!("CARGO_PKG_VERSION"))).unwrap(),
                "MIT",
                TrustEvidence::unsigned(),
            )
            .unwrap();
            PackageStore::open(data_dir.join("packages.sqlite3"))
                .unwrap()
                .admit(&package, &package, artifact)
                .unwrap();
            let catalog = HarnessCatalog::builtins()
                .with_declarative_domain(&package)
                .unwrap();
            let manifest = catalog
                .find(&HarnessId::new(CUSTOM_HARNESS_ID).unwrap())
                .unwrap()
                .manifest();
            (
                CUSTOM_HARNESS_ID,
                CUSTOM_HARNESS_VERSION,
                canonical_harness_binding_digest(manifest),
            )
        } else {
            let catalog = HarnessCatalog::builtins();
            let manifest = catalog
                .find(&HarnessId::new(HARNESS_ID).unwrap())
                .unwrap()
                .manifest();
            (
                HARNESS_ID,
                HARNESS_VERSION,
                canonical_harness_binding_digest(manifest),
            )
        };

        let scope = SubagentScope::new(
            PrincipalId::new("principal-subagent-cli-1").unwrap(),
            TenantId::new("tenant-subagent-cli-1").unwrap(),
            WorkspaceId::new("parent-workspace-subagent-cli-1").unwrap(),
        );
        let id = SubagentId::new("subagent-cli-1").unwrap();
        let request = SubagentRequest::new(
            SessionId::new("parent-session-subagent-cli-1").unwrap(),
            ExecutionId::new("parent-execution-subagent-cli-1").unwrap(),
            1,
            exact_commit,
            "Read identity.txt",
            SubagentBudgets::new(2, 1, 1_000, 60, 1, 8_192).unwrap(),
        )
        .unwrap();
        let request = if options.bind_provider {
            request.with_provider_profile(PROVIDER_NAME).unwrap()
        } else {
            request
        };
        let request = request.with_harness(
            SubagentHarnessBinding::new(HarnessId::new(harness_id).unwrap(), harness_version)
                .unwrap(),
        );
        let persisted_provider_url = if options.changed_provider_binding {
            "http://127.0.0.1:9/v1"
        } else {
            &provider_url
        };
        let persisted_harness_digest = if options.changed_harness_binding {
            format!("{harness_digest}-stale")
        } else {
            harness_digest
        };
        let store = SubagentStore::open(root.join("jobs.sqlite3")).unwrap();
        let prepared = store
            .prepare(SubagentPreparation::new(
                id,
                JobId::new("job-subagent-cli-1").unwrap(),
                scope.clone(),
                SessionId::new("child-session-subagent-cli-1").unwrap(),
                ExecutionId::new("child-execution-subagent-cli-1").unwrap(),
                request,
                parent_workspace.clone(),
                child_worktree.clone(),
                options
                    .bind_provider
                    .then(|| provider_binding_digest(persisted_provider_url)),
                Some(persisted_harness_digest),
                Timestamp::from_unix_seconds(10),
            ))
            .unwrap();
        store
            .queue(
                prepared.id(),
                &scope,
                &EffectReceipt::new(
                    ReceiptId::new("receipt-subagent-cli-1").unwrap(),
                    PermitId::new("permit-subagent-cli-1").unwrap(),
                    RequestDigest::new("request-subagent-cli-1").unwrap(),
                    Timestamp::from_unix_seconds(20),
                    EffectOutcome::Succeeded,
                ),
                Timestamp::from_unix_seconds(20),
            )
            .unwrap();
        let record = store
            .claim_next(
                &scope,
                &JobWorkerId::new("worker-subagent-cli-1").unwrap(),
                Timestamp::from_unix_seconds(30),
            )
            .unwrap()
            .unwrap();

        Self {
            root,
            parent_workspace,
            child_worktree,
            config,
            record,
            sessions,
            approvals,
            control: NoopControl,
            provider_address,
            provider_calls: Arc::new(AtomicUsize::new(0)),
            provider_stop: Arc::new(AtomicBool::new(false)),
            listener: Mutex::new(Some(listener)),
            server: Mutex::new(None),
            provider_requests: Mutex::new(None),
        }
    }

    fn parent_workspace(&self) -> &Path {
        &self.parent_workspace
    }

    fn child_worktree(&self) -> &Path {
        &self.child_worktree
    }

    fn advance_child_head(&self) {
        fs::write(self.child_worktree.join("changed.txt"), "changed\n").unwrap();
        git(&self.child_worktree, &["add", "changed.txt"]);
        git(&self.child_worktree, &["commit", "-m", "changed"]);
    }

    fn seed_parent_state(&self) {
        let parent = Session::new(
            self.record.request().parent_session_id().clone(),
            self.record.scope().principal_id().clone(),
            self.record.scope().tenant_id().clone(),
            self.record.scope().workspace_id().clone(),
            Timestamp::from_unix_seconds(5),
        );
        self.sessions.create(&parent).unwrap();
        self.sessions
            .save_agent_transcript(
                parent.id(),
                parent.principal_id(),
                parent.tenant_id(),
                parent.workspace_id(),
                &[ChatMessage::assistant("parent-transcript-secret").unwrap()],
            )
            .unwrap();
        self.approvals
            .create(
                ApprovalRequest::new(
                    "parent-approval-subagent-cli-1",
                    parent.id().clone(),
                    self.record.request().parent_execution_id().clone(),
                    parent.principal_id().clone(),
                    GeneId::new("workspace.patch").unwrap(),
                    RequestDigest::new("parent-approval-digest-subagent-cli-1").unwrap(),
                    "parent-approval-secret",
                    1,
                    Timestamp::from_unix_seconds(600),
                )
                .unwrap(),
            )
            .unwrap();
    }

    fn child_workspace_id(&self) -> WorkspaceId {
        let digest = hash_artifact(self.record.id().as_str().as_bytes());
        let digest = digest.strip_prefix("sha256:").unwrap();
        WorkspaceId::new(format!("subagent-{digest}")).unwrap()
    }

    fn child_snapshot(&self) -> pandora_runtime::sessions::SessionSnapshot {
        self.sessions
            .resume(
                self.record.child_session_id(),
                self.record.scope().principal_id(),
                self.record.scope().tenant_id(),
                &self.child_workspace_id(),
            )
            .unwrap()
    }

    fn trusted_run(&self) -> TrustedSubagentRun<'_> {
        TrustedSubagentRun {
            config: &self.config,
            record: &self.record,
            store: &self.sessions,
            approval_store: &self.approvals,
            control: &self.control,
        }
    }

    fn provider_returns_read_then_final(&self, path: &str) {
        let listener = self.listener.lock().unwrap().take().unwrap();
        let provider_calls = Arc::clone(&self.provider_calls);
        let arguments = serde_json::to_string(&json!({"path": path})).unwrap();
        let responses = [
            serde_json::to_vec(&json!({
                "choices": [{
                    "message": {
                        "content": null,
                        "tool_calls": [{
                            "id": "call-identity",
                            "type": "function",
                            "function": {
                                "name": "workspace.read",
                                "arguments": arguments,
                            }
                        }]
                    }
                }],
                "usage": {"prompt_tokens": 4, "completion_tokens": 2}
            }))
            .unwrap(),
            serde_json::to_vec(&json!({
                "choices": [{"message": {"content": "done"}}],
                "usage": {"prompt_tokens": 5, "completion_tokens": 3}
            }))
            .unwrap(),
        ];
        let server = thread::spawn(move || {
            responses
                .iter()
                .map(|response| {
                    let (mut stream, _) = listener.accept().unwrap();
                    provider_calls.fetch_add(1, Ordering::Relaxed);
                    let request = read_http_json(&mut stream);
                    write_http_json(&mut stream, response);
                    request
                })
                .collect()
        });
        *self.server.lock().unwrap() = Some(server);
    }

    fn provider_returns_final(&self) {
        self.provider_returns_responses(vec![
            serde_json::to_vec(&json!({
                "choices": [{"message": {"content": "done"}}],
                "usage": {"prompt_tokens": 2, "completion_tokens": 1}
            }))
            .unwrap(),
        ]);
    }

    fn provider_returns_tool_call(&self, path: &str) {
        let arguments = serde_json::to_string(&json!({"path": path})).unwrap();
        self.provider_returns_responses(vec![
            serde_json::to_vec(&json!({
                "choices": [{
                    "message": {
                        "content": null,
                        "tool_calls": [{
                            "id": "call-controlled-stop",
                            "type": "function",
                            "function": {
                                "name": "workspace.read",
                                "arguments": arguments,
                            }
                        }]
                    }
                }],
                "usage": {"prompt_tokens": 2, "completion_tokens": 1}
            }))
            .unwrap(),
        ]);
    }

    fn provider_returns_responses(&self, responses: Vec<Vec<u8>>) {
        let listener = self.listener.lock().unwrap().take().unwrap();
        let provider_calls = Arc::clone(&self.provider_calls);
        let server = thread::spawn(move || {
            responses
                .iter()
                .map(|response| {
                    let (mut stream, _) = listener.accept().unwrap();
                    provider_calls.fetch_add(1, Ordering::Relaxed);
                    let request = read_http_json(&mut stream);
                    write_http_json(&mut stream, response);
                    request
                })
                .collect()
        });
        *self.server.lock().unwrap() = Some(server);
    }

    fn observe_provider_if_called(&self) {
        let listener = self.listener.lock().unwrap().take().unwrap();
        let provider_calls = Arc::clone(&self.provider_calls);
        let provider_stop = Arc::clone(&self.provider_stop);
        let response = serde_json::to_vec(&json!({
            "choices": [{"message": {"content": "unexpected provider call"}}]
        }))
        .unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            if provider_stop.load(Ordering::Acquire) {
                return Vec::new();
            }
            provider_calls.fetch_add(1, Ordering::Relaxed);
            let request = read_http_json(&mut stream);
            write_http_json(&mut stream, &response);
            vec![request]
        });
        *self.server.lock().unwrap() = Some(server);
    }

    fn provider_request_text(&self) -> String {
        let mut requests = self.provider_requests.lock().unwrap();
        if requests.is_none() {
            let server = self.server.lock().unwrap().take().unwrap();
            *requests = Some(server.join().unwrap());
        }
        serde_json::to_string(requests.as_ref().unwrap()).unwrap()
    }

    fn first_provider_request_text(&self) -> String {
        let mut requests = self.provider_requests.lock().unwrap();
        if requests.is_none() {
            let server = self.server.lock().unwrap().take().unwrap();
            *requests = Some(server.join().unwrap());
        }
        serde_json::to_string(&requests.as_ref().unwrap()[0]).unwrap()
    }

    fn provider_calls(&self) -> usize {
        self.provider_stop.store(true, Ordering::Release);
        let mut server = self.server.lock().unwrap();
        if let Some(handle) = server.take() {
            if !handle.is_finished() {
                TcpStream::connect(self.provider_address).unwrap();
            }
            *self.provider_requests.lock().unwrap() = Some(handle.join().unwrap());
        }
        self.provider_calls.load(Ordering::Relaxed)
    }
}

impl Drop for CliSubagentFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct NoopControl;

impl AgentRunControl for NoopControl {
    fn checkpoint(&self, _checkpoint: AgentCheckpoint<'_>) -> Result<(), AgentControlStop> {
        Ok(())
    }
}

struct StopAfterEffectControl;

impl AgentRunControl for StopAfterEffectControl {
    fn checkpoint(&self, checkpoint: AgentCheckpoint<'_>) -> Result<(), AgentControlStop> {
        if checkpoint.kind() == AgentCheckpointKind::AfterEffect {
            return Err(AgentControlStop::Cancelled);
        }
        Ok(())
    }
}

fn provider_binding_digest(base_url: &str) -> String {
    let canonical = format!(
        "provider-binding-v1\0{PROVIDER_NAME}\0{base_url}\0{PROVIDER_MODEL}\0{PROVIDER_KEY_ENV}\0"
    );
    format!("provider-{}", hash_artifact(canonical.as_bytes()))
}

fn read_http_json(stream: &mut TcpStream) -> Value {
    let mut request = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 1_024];
        let bytes_read = stream.read(&mut chunk).unwrap();
        request.extend_from_slice(&chunk[..bytes_read]);
        if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
    let content_length = headers
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap();
    while request.len() < header_end + content_length {
        let mut chunk = [0_u8; 1_024];
        let bytes_read = stream.read(&mut chunk).unwrap();
        request.extend_from_slice(&chunk[..bytes_read]);
    }
    serde_json::from_slice(&request[header_end..header_end + content_length]).unwrap()
}

fn write_http_json(stream: &mut TcpStream, response: &[u8]) {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.len()
    )
    .unwrap();
    stream.write_all(response).unwrap();
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "{prefix}-{}-{timestamp}-{sequence}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    root
}

fn git(repository: &Path, arguments: &[&str]) {
    let status = Command::new("git")
        .args(arguments)
        .current_dir(repository)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git command failed: {arguments:?}");
}

fn git_worktree(repository: &Path, destination: &Path, commit: &str) {
    let status = Command::new("git")
        .args(["worktree", "add", "--detach"])
        .arg(destination)
        .arg(commit)
        .current_dir(repository)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
}

fn git_output(repository: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repository)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
