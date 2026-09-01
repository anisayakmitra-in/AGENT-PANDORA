use base64::Engine as _;
use ed25519_dalek::{Signer, SigningKey};
use pandora_runtime::{
    DeviceKeyStore, DeviceProofRequest, EfficiencyStore, FleetBudget, FleetEngine, FleetNode,
    JobStore, MemoryEngine, SessionStore, SubagentPreparation, SubagentScope, SubagentStore,
};
use pandora_types::{
    ContextClassification, DomainRoutingProfile, EffectOutcome, EffectReceipt, EventType,
    ExecutionId, GovernedOrchestrationPlan, Handoff, HarnessId, JobCommand, JobId, JobRequest,
    JobStatus, JobWorkerId, MemoryId, MemoryKind, MemoryScope, MetaComposition, OrchestrationPlan,
    OrchestrationRole, OrchestrationRoleReceipt, OrchestrationRunId, PackageCompatibility,
    PackageDependency, PackageKind, PackageManifest, PermitId, PlanId, PrincipalId, ReceiptId,
    RepositoryBinding, RepositoryId, RequestDigest, RoleAssignment, RoleId, RoleRepositoryBinding,
    Session, SessionId, SubagentBudgets, SubagentId, SubagentRequest, TenantId, Timestamp,
    TrustEvidence, TrustLevel, WorkspaceId, hash_artifact,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

struct Fixture {
    root: PathBuf,
    config: PathBuf,
    data: PathBuf,
    workspace: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be available")
            .as_nanos();
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let temp_root = std::env::temp_dir()
            .canonicalize()
            .expect("temporary directory should have a canonical path");
        let root = temp_root.join(format!(
            "pandora-cli-smoke-{}-{suffix}-{sequence}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).expect("workspace should be created");
        fs::write(workspace.join("README.md"), "fixture\n").expect("fixture should be written");
        Self {
            config: root.join("config.json"),
            data: root.join("data"),
            workspace,
            root,
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_pandora"));
        command
            .args(args)
            .env("PANDORA_CONFIG", &self.config)
            .env("PANDORA_DATA_DIR", &self.data)
            .env("PANDORA_WORKSPACE", &self.workspace)
            .env_remove("PANDORA_PROVIDER_URL");
        command
    }

    fn setup(&self) -> Value {
        let mut output = self.command(&[
            "setup",
            "--provider-url",
            "http://127.0.0.1:4317/v1",
            "--json",
        ]);
        let output = output.output().expect("setup should start");
        assert_success(&output);
        parse_json(&output)
    }

    fn initialize_git_workspace(&self) -> String {
        for arguments in [
            ["init"].as_slice(),
            ["config", "user.email", "fixture@example.test"].as_slice(),
            ["config", "user.name", "Pandora Fixture"].as_slice(),
            ["add", "README.md"].as_slice(),
            ["commit", "-m", "fixture"].as_slice(),
        ] {
            let output = Command::new("git")
                .args(arguments)
                .current_dir(&self.workspace)
                .output()
                .expect("git fixture command should start");
            assert!(
                output.status.success(),
                "git fixture command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let output = Command::new("git")
            .args(["rev-parse", "--verify", "HEAD"])
            .current_dir(&self.workspace)
            .output()
            .expect("git revision lookup should start");
        assert!(output.status.success());
        String::from_utf8(output.stdout)
            .expect("git revision should be UTF-8")
            .trim()
            .to_owned()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn local_subagent_scope() -> SubagentScope {
    SubagentScope::new(
        PrincipalId::new("local-user").unwrap(),
        TenantId::new("local-tenant").unwrap(),
        WorkspaceId::new("local-workspace").unwrap(),
    )
}

fn seed_subagent(fixture: &Fixture, id: &str, scope: SubagentScope, running: bool) {
    let store = SubagentStore::open(fixture.data.join("jobs.sqlite3")).unwrap();
    let id = SubagentId::new(id).unwrap();
    let request = SubagentRequest::new(
        SessionId::new(format!("parent-session-{}", id.as_str())).unwrap(),
        ExecutionId::new(format!("parent-execution-{}", id.as_str())).unwrap(),
        0,
        "a".repeat(40),
        "fixture task",
        SubagentBudgets::new(1, 1, 1, 1, 0, 8_192).unwrap(),
    )
    .unwrap();
    let prepared = store
        .prepare(SubagentPreparation::new(
            id.clone(),
            JobId::new(format!("job-{}", id.as_str())).unwrap(),
            scope.clone(),
            SessionId::new(format!("child-session-{}", id.as_str())).unwrap(),
            ExecutionId::new(format!("child-execution-{}", id.as_str())).unwrap(),
            request,
            fixture.workspace.clone(),
            fixture.data.join("subagents").join(id.as_str()),
            None,
            None,
            Timestamp::from_unix_seconds(10),
        ))
        .unwrap();
    let receipt = EffectReceipt::new(
        ReceiptId::new(format!("create-receipt-{}", id.as_str())).unwrap(),
        PermitId::new(format!("create-permit-{}", id.as_str())).unwrap(),
        RequestDigest::new(format!("create-digest-{}", id.as_str())).unwrap(),
        Timestamp::from_unix_seconds(20),
        EffectOutcome::Succeeded,
    );
    store
        .queue(
            prepared.id(),
            &scope,
            &receipt,
            Timestamp::from_unix_seconds(20),
        )
        .unwrap();
    if running {
        store
            .claim_next(
                &scope,
                &JobWorkerId::new(format!("worker-{}", id.as_str())).unwrap(),
                Timestamp::from_unix_seconds(30),
            )
            .unwrap()
            .expect("queued subagent should be claimable");
    }
}

fn loopback_provider_calls(listener: TcpListener) -> thread::JoinHandle<usize> {
    provider_calls_with_timeout(listener, Duration::from_secs(2))
}

fn expected_provider_call(listener: TcpListener) -> thread::JoinHandle<usize> {
    provider_calls_with_timeout(listener, Duration::from_secs(30))
}

fn provider_calls_with_timeout(
    listener: TcpListener,
    timeout: Duration,
) -> thread::JoinHandle<usize> {
    listener
        .set_nonblocking(true)
        .expect("provider fixture should become non-blocking");
    thread::spawn(move || {
        let deadline = Instant::now() + timeout;
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_nonblocking(false)
                        .expect("provider connection should become blocking");
                    stream
                        .set_read_timeout(Some(timeout))
                        .expect("provider connection should keep a bounded read");
                    serve_provider_response(&mut stream);
                    return 1;
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return 0;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("provider fixture should accept: {error}"),
            }
        }
    })
}

fn serve_provider_response(stream: &mut TcpStream) {
    let mut request = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 1_024];
        let bytes_read = stream
            .read(&mut chunk)
            .expect("provider request should read");
        assert_ne!(bytes_read, 0, "provider request ended before its headers");
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
        .expect("provider request should send a content length");
    while request.len() < header_end + content_length {
        let mut chunk = [0_u8; 1_024];
        let bytes_read = stream
            .read(&mut chunk)
            .expect("provider request body should read");
        assert_ne!(bytes_read, 0, "provider request body ended early");
        request.extend_from_slice(&chunk[..bytes_read]);
    }
    let response = br#"{"choices":[{"message":{"content":"fixture complete"}}],"usage":{"prompt_tokens":2,"completion_tokens":1}}"#;
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.len()
    )
    .expect("provider response headers should be written");
    stream
        .write_all(response)
        .expect("provider response should be written");
}

struct HeldToolProvider {
    request_arrived: Receiver<()>,
    release_response: SyncSender<()>,
    stop: SyncSender<()>,
    calls: Arc<AtomicUsize>,
    server: thread::JoinHandle<usize>,
}

const HELD_PROVIDER_BARRIER_TIMEOUT: Duration = Duration::from_secs(60);

impl HeldToolProvider {
    fn start(listener: TcpListener) -> Self {
        listener
            .set_nonblocking(true)
            .expect("provider fixture should become non-blocking");
        let (request_arrived_tx, request_arrived) = mpsc::sync_channel(0);
        let (release_response, release_response_rx) = mpsc::sync_channel(0);
        let (stop, stop_rx) = mpsc::sync_channel(0);
        let calls = Arc::new(AtomicUsize::new(0));
        let server_calls = Arc::clone(&calls);
        let server = thread::spawn(move || {
            let request_deadline = Instant::now() + HELD_PROVIDER_BARRIER_TIMEOUT;
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < request_deadline,
                            "subagent did not reach the held provider"
                        );
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("held provider should accept: {error}"),
                }
            };
            stream
                .set_nonblocking(false)
                .expect("held provider connection should become blocking");
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .expect("held provider should keep a bounded read");
            stream
                .set_write_timeout(Some(Duration::from_secs(10)))
                .expect("held provider should keep a bounded write");
            read_provider_request(&mut stream);
            server_calls.fetch_add(1, Ordering::SeqCst);
            request_arrived_tx
                .send(())
                .expect("provider request barrier should remain connected");
            release_response_rx
                .recv_timeout(HELD_PROVIDER_BARRIER_TIMEOUT)
                .expect("provider response release should arrive");
            write_provider_response(
                &mut stream,
                br#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"workspace.read","arguments":"{\"path\":\"README.md\"}"}}]}}],"usage":{"prompt_tokens":4,"completion_tokens":2}}"#,
            );

            let replay_deadline = Instant::now() + Duration::from_secs(30);
            loop {
                match stop_rx.try_recv() {
                    Ok(()) | Err(TryRecvError::Disconnected) => break,
                    Err(TryRecvError::Empty) => {}
                }
                match listener.accept() {
                    Ok((mut replay, _)) => {
                        replay
                            .set_nonblocking(false)
                            .expect("replay connection should become blocking");
                        replay
                            .set_read_timeout(Some(Duration::from_secs(10)))
                            .expect("replay request should keep a bounded read");
                        read_provider_request(&mut replay);
                        server_calls.fetch_add(1, Ordering::SeqCst);
                        write_provider_response(
                            &mut replay,
                            br#"{"choices":[{"message":{"content":"unexpected replay"}}],"usage":{"prompt_tokens":2,"completion_tokens":1}}"#,
                        );
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < replay_deadline,
                            "held provider fixture exceeded its bounded lifetime"
                        );
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("held provider should accept replay checks: {error}"),
                }
            }
            server_calls.load(Ordering::SeqCst)
        });
        Self {
            request_arrived,
            release_response,
            stop,
            calls,
            server,
        }
    }

    fn wait_for_request(&self) {
        self.request_arrived
            .recv_timeout(HELD_PROVIDER_BARRIER_TIMEOUT)
            .expect("subagent should reach the provider barrier");
    }

    fn release(&self) {
        self.release_response
            .send(())
            .expect("provider response barrier should remain connected");
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn finish(self) -> usize {
        self.stop
            .send(())
            .expect("provider stop barrier should remain connected");
        self.server
            .join()
            .expect("held provider fixture should finish")
    }
}

fn read_provider_request(stream: &mut TcpStream) {
    let mut request = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 1_024];
        let bytes_read = stream
            .read(&mut chunk)
            .expect("provider request should read");
        assert_ne!(bytes_read, 0, "provider request ended before its headers");
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
        .expect("provider request should send a content length");
    while request.len() < header_end + content_length {
        let mut chunk = [0_u8; 1_024];
        let bytes_read = stream
            .read(&mut chunk)
            .expect("provider request body should read");
        assert_ne!(bytes_read, 0, "provider request body ended early");
        request.extend_from_slice(&chunk[..bytes_read]);
    }
}

fn write_provider_response(stream: &mut TcpStream, response: &[u8]) {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.len()
    )
    .expect("provider response headers should be written");
    stream
        .write_all(response)
        .expect("provider response should be written");
}

fn wait_for_child(mut child: Child, timeout: Duration, label: &str) -> Output {
    let deadline = Instant::now() + timeout;
    loop {
        match child
            .try_wait()
            .expect("child status should remain inspectable")
        {
            Some(_) => {
                return child
                    .wait_with_output()
                    .expect("finished child output should be readable");
            }
            None if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            None => {
                let _ = child.kill();
                let output = child
                    .wait_with_output()
                    .expect("timed-out child output should be readable");
                panic!(
                    "{label} exceeded {timeout:?}: stdout={} stderr={}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }
}

#[test]
fn provider_fixture_waits_for_request_bytes_after_accept() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("provider fixture should bind");
    let address = listener
        .local_addr()
        .expect("provider fixture should expose its address");
    let server = provider_calls_with_timeout(listener, Duration::from_secs(2));
    let mut stream = TcpStream::connect(address).expect("provider client should connect");

    thread::sleep(Duration::from_millis(50));
    stream
        .write_all(b"POST /v1 HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}")
        .expect("provider request should be written");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .expect("provider response should be read");

    assert_eq!(server.join().expect("provider fixture should finish"), 1);
    assert!(response.starts_with(b"HTTP/1.1 200 OK"));
}

fn admit_domain_harness(fixture: &Fixture, artifact: &[u8], gene_ids: &[&str]) {
    let manifest = PackageManifest::new(
        "example/subagent-domain",
        "1.0.0",
        PackageKind::DomainHarness,
        "local-publisher",
        hash_artifact(artifact),
        gene_ids
            .iter()
            .map(|gene_id| PackageDependency::new(*gene_id, "0.1.0", false).unwrap())
            .collect(),
        PackageCompatibility::new(concat!("pandora>=", env!("CARGO_PKG_VERSION"))).unwrap(),
        "MIT",
        TrustEvidence::unsigned(),
    )
    .unwrap();
    let manifest_path = fixture.root.join("subagent-domain.json");
    let artifact_path = fixture.root.join("subagent-domain.artifact");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(&artifact_path, artifact).unwrap();
    let admitted = fixture
        .command(&[
            "package",
            "admit",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--artifact",
            artifact_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("domain Harness admission should start");
    assert_success(&admitted);
}

#[test]
fn setup_and_read_only_run_return_versioned_json() {
    let fixture = Fixture::new();
    let setup = fixture.setup();
    assert_eq!(setup["version"], "0.1");
    assert_eq!(setup["command"], "setup");
    assert_eq!(setup["provider_model"], "default");

    let output = fixture
        .command(&["run", "read:README.md", "--json"])
        .output()
        .expect("run should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["version"], "0.1");
    assert_eq!(response["command"], "run");
    assert_eq!(response["status"], "completed");
    assert!(response["elapsed_ms"].as_u64().is_some());
    assert_eq!(response["output"], "fixture\n");
    assert_eq!(response["efficiency_recorded"], true);
    assert_eq!(response["feedback_recorded"], false);
    assert_eq!(response["evaluation"]["recorded"], true);
    assert_eq!(response["evaluation"]["outcome_available"], false);
    assert_eq!(
        response["evaluation"]["receipt"]["results"][0]["kind"],
        "trajectory"
    );
    assert_eq!(
        response["evaluation"]["receipt"]["results"][0]["status"],
        "passed"
    );
    assert_eq!(
        response["evaluation"]["receipt"]["results"][1]["kind"],
        "policy"
    );
    assert_eq!(
        response["evaluation"]["receipt"]["results"][1]["status"],
        "passed"
    );
    assert!(
        !response["session_id"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );

    let output = fixture
        .command(&[
            "efficiency",
            "rank",
            "--task-class",
            "general",
            "--objective",
            "latency",
            "--json",
        ])
        .output()
        .expect("efficiency ranking should start");
    assert_success(&output);
    let ranking = parse_json(&output);
    assert_eq!(ranking["command"], "efficiency rank");
    assert!(!ranking["rankings"].as_array().unwrap().is_empty());
}

#[test]
fn direct_run_can_record_coding_feedback() {
    let fixture = Fixture::new();
    fixture.setup();
    let output = fixture
        .command(&[
            "run",
            "read:README.md",
            "--expected-output",
            "fixture",
            "--json",
        ])
        .output()
        .expect("feedback run should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["status"], "completed");
    assert_eq!(response["coding_feedback"]["decision"], "completed");
    assert_eq!(
        response["coding_feedback"]["evaluations"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        response["coding_feedback"]["evaluations"][1]["kind"],
        "outcome"
    );
    assert_eq!(
        response["coding_feedback"]["evaluations"][1]["status"],
        "passed"
    );
}

#[test]
fn direct_run_feedback_can_recommend_bounded_retry() {
    let fixture = Fixture::new();
    fixture.setup();
    let output = fixture
        .command(&[
            "run",
            "read:README.md",
            "--expected-output",
            "different",
            "--retryable",
            "--json",
        ])
        .output()
        .expect("feedback run should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["status"], "completed");
    assert_eq!(response["coding_feedback"]["decision"], "retry");
    assert_eq!(
        response["coding_feedback"]["adaptation"]["decision"]["selected"]["label"],
        "coding.safe_retry"
    );
}

#[test]
fn evolution_cli_can_submit_evaluate_and_approve() {
    let fixture = Fixture::new();
    fixture.setup();

    let proposal_path = fixture.root.join("proposal.json");
    fs::write(
        &proposal_path,
        br#"{"proposal_id":"proposal-cli","source":"gepa","base_artifact":"base-cli","candidate_artifact":"candidate-cli","evidence_digest":"evidence-cli","expected_outcome":"improve coding reliability"}"#,
    )
    .unwrap();
    let submitted = fixture
        .command(&[
            "evolution",
            "submit",
            "--input",
            proposal_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("evolution submit should start");
    assert_success_with_context(&submitted, "evolution submit");

    let holdout_path = fixture.root.join("holdout.json");
    fs::write(
        &holdout_path,
        br#"{"cases":[{"id":"case-cli","execution_id":"execution-cli","output":"candidate","expected_output":"candidate","baseline_output":"candidate"}]}"#,
    )
    .unwrap();
    let evaluated = fixture
        .command(&[
            "evolution",
            "evaluate",
            "--id",
            "proposal-cli",
            "--input",
            holdout_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("evolution evaluate should start");
    assert_success_with_context(&evaluated, "evolution evaluate");

    let approval_path = fixture.root.join("approval.json");
    fs::write(
        &approval_path,
        br#"{"proposal_id":"proposal-cli","approver":"parliament-cli","policy_version":1,"artifact_id":"candidate-cli","signer":"signer-cli","signature":"signed-candidate"}"#,
    )
    .unwrap();
    let approved = fixture
        .command(&[
            "evolution",
            "approve",
            "--input",
            approval_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("evolution approve should start");
    assert_success_with_context(&approved, "evolution approve");
    let response = parse_json(&approved);
    assert_eq!(response["state"], "approved");
    assert_eq!(response["approver"], "parliament-cli");
    assert_eq!(response["signer"], "signer-cli");
}

#[test]
fn scheduled_canary_binds_a_reviewed_suite_and_stops_before_activation() {
    let fixture = Fixture::new();
    fixture.setup();
    let proposal = fixture.root.join("scheduled-proposal.json");
    fs::write(
        &proposal,
        br#"{"proposal_id":"proposal-scheduled","source":"gepa","base_artifact":"base-scheduled","candidate_artifact":"candidate-scheduled","evidence_digest":"evidence-scheduled","expected_outcome":"improve scheduled verification"}"#,
    )
    .unwrap();
    assert_success_with_context(
        &fixture
            .command(&[
                "evolution",
                "submit",
                "--input",
                proposal.to_str().unwrap(),
                "--json",
            ])
            .output()
            .unwrap(),
        "scheduled proposal submit",
    );
    let holdout = fixture.root.join("scheduled-holdout.json");
    fs::write(
        &holdout,
        br#"{"cases":[{"id":"scheduled-case","execution_id":"scheduled-execution","output":"candidate","expected_output":"candidate","baseline_output":"candidate"}]}"#,
    )
    .unwrap();
    assert_success_with_context(
        &fixture
            .command(&[
                "evolution",
                "evaluate",
                "--id",
                "proposal-scheduled",
                "--input",
                holdout.to_str().unwrap(),
                "--json",
            ])
            .output()
            .unwrap(),
        "scheduled proposal evaluation",
    );
    let approval = fixture.root.join("scheduled-approval.json");
    fs::write(
        &approval,
        br#"{"proposal_id":"proposal-scheduled","approver":"parliament-scheduled","policy_version":1,"artifact_id":"candidate-scheduled","signer":"signer-scheduled","signature":"signed-scheduled-candidate"}"#,
    )
    .unwrap();
    assert_success_with_context(
        &fixture
            .command(&[
                "evolution",
                "approve",
                "--input",
                approval.to_str().unwrap(),
                "--json",
            ])
            .output()
            .unwrap(),
        "scheduled proposal approval",
    );
    let suite = fixture.root.join("scheduled-suite.json");
    fs::write(
        &suite,
        br#"{"suite_id":"candidate-suite","cases":[{"id":"candidate-case","target":{"kind":"workflow","id":"workflow-1"},"task":"evaluate the candidate","execution_id":"candidate-execution","output":"verified","expected_output":"verified"}]}"#,
    )
    .unwrap();
    assert_success_with_context(
        &fixture
            .command(&[
                "evaluation",
                "suite",
                "register",
                "--id",
                "candidate-suite",
                "--input",
                suite.to_str().unwrap(),
                "--json",
            ])
            .output()
            .unwrap(),
        "scheduled candidate suite registration",
    );

    let blocked = fixture
        .command(&[
            "evaluation",
            "schedule",
            "create",
            "--id",
            "candidate-canary",
            "--name",
            "Candidate canary",
            "--suite",
            "candidate-suite",
            "--proposal",
            "proposal-scheduled",
            "--interval-seconds",
            "60",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(!blocked.status.success());
    assert_success_with_context(
        &fixture
            .command(&["evolution", "stage", "--id", "proposal-scheduled", "--json"])
            .output()
            .unwrap(),
        "scheduled proposal stage",
    );
    let created = fixture
        .command(&[
            "evaluation",
            "schedule",
            "create",
            "--id",
            "candidate-canary",
            "--name",
            "Candidate canary",
            "--suite",
            "candidate-suite",
            "--proposal",
            "proposal-scheduled",
            "--interval-seconds",
            "60",
            "--json",
        ])
        .output()
        .unwrap();
    assert_success_with_context(&created, "candidate canary schedule creation");
    let created = parse_json(&created);
    assert_eq!(created["proposal_id"], "proposal-scheduled");
    assert_eq!(created["one_shot"], true);
    assert_eq!(created["activation_performed"], false);

    let run = fixture
        .command(&[
            "evaluation",
            "schedule",
            "run",
            "--id",
            "candidate-canary",
            "--worker",
            "candidate-worker",
            "--json",
        ])
        .output()
        .unwrap();
    assert_success_with_context(&run, "candidate canary scheduled run");
    let run = parse_json(&run);
    assert_eq!(run["canary"]["state"], "canary_passed");
    assert_eq!(run["canary"]["activation_performed"], false);
    assert_eq!(run["activation_performed"], false);
    assert_eq!(run["run"]["evidence"]["total_cases"], 1);
    assert_eq!(run["run"]["evidence"]["failed_cases"], 0);
    assert!(
        run["run"]["evidence"]["report_digest"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );

    let inspected = fixture
        .command(&[
            "evolution",
            "inspect",
            "--id",
            "proposal-scheduled",
            "--json",
        ])
        .output()
        .unwrap();
    assert_success_with_context(&inspected, "scheduled proposal inspection");
    assert_eq!(parse_json(&inspected)["state"], "canary_passed");
    let history = fixture
        .command(&[
            "evaluation",
            "schedule",
            "runs",
            "--id",
            "candidate-canary",
            "--json",
        ])
        .output()
        .unwrap();
    assert_success_with_context(&history, "candidate canary run history");
    let history = parse_json(&history);
    assert_eq!(history["count"], 1);
    assert_eq!(history["runs"][0]["proposal_id"], "proposal-scheduled");
    assert_eq!(history["runs"][0]["status"], "completed");
}

#[test]
fn evolution_cli_generates_a_research_candidate_then_requires_every_governance_gate() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("research provider should bind");
    let address = listener
        .local_addr()
        .expect("research provider should expose its address");
    let candidate = b"Improve verification by recording the exact holdout digest.\n";
    let generated_content = serde_json::json!({
        "proposal_id": "research-prompt-1",
        "expected_outcome": "reduce unverified workflow regressions",
        "artifact_base64": base64::engine::general_purpose::STANDARD.encode(candidate),
    })
    .to_string();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("research provider should connect");
        let mut request = Vec::new();
        let header_end = loop {
            let mut chunk = [0_u8; 1_024];
            let bytes_read = stream
                .read(&mut chunk)
                .expect("research request should read");
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
            .expect("research request should send a content length");
        while request.len() < header_end + content_length {
            let mut chunk = [0_u8; 1_024];
            let bytes_read = stream
                .read(&mut chunk)
                .expect("research request body should read");
            request.extend_from_slice(&chunk[..bytes_read]);
        }
        let request_body =
            serde_json::from_slice::<Value>(&request[header_end..header_end + content_length])
                .expect("research request should be JSON");
        let system = request_body["messages"][0]["content"]
            .as_str()
            .expect("research request should include a system boundary");
        let user = request_body["messages"][1]["content"]
            .as_str()
            .expect("research request should include bounded evidence");
        assert!(system.contains("untrusted research proposer"));
        assert!(user.contains("base_artifact_base64"));
        assert!(user.contains("evaluation_summaries"));
        let research_input = serde_json::from_str::<Value>(user)
            .expect("research evidence should use the stable JSON contract");
        assert_eq!(
            research_input["bounded_research_evidence"]["memory_evidence_ids"][0],
            "evolution-memory-1"
        );
        assert_eq!(
            research_input["bounded_research_evidence"]["feedback_summaries"][0]["id"],
            "evolution-memory-1"
        );
        let response = serde_json::json!({
            "choices": [{"message": {"content": generated_content}}],
            "usage": {"prompt_tokens": 12, "completion_tokens": 8},
        })
        .to_string();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response.len()
        )
        .expect("research provider response headers should be written");
        stream
            .write_all(response.as_bytes())
            .expect("research provider response should be written");
    });

    let fixture = Fixture::new();
    let configured = fixture
        .command(&[
            "provider",
            "set",
            "--provider-url",
            &format!("http://{address}/v1"),
            "--model",
            "research-fixture",
            "--json",
        ])
        .output()
        .expect("provider set should start");
    assert_success(&configured);
    let session = Session::new(
        SessionId::new("research-session-1").unwrap(),
        PrincipalId::new("local-user").unwrap(),
        TenantId::new("local-tenant").unwrap(),
        WorkspaceId::new("local-workspace").unwrap(),
        Timestamp::from_unix_seconds(10),
    );
    SessionStore::open(fixture.data.join("sessions.sqlite3"))
        .unwrap()
        .create(&session)
        .unwrap();
    let memory = MemoryEngine::open(
        fixture.data.join("sessions.sqlite3"),
        64,
        PrincipalId::new("local-user").unwrap(),
    )
    .unwrap();
    memory
        .distill_l1(
            MemoryScope::new(
                TenantId::new("local-tenant").unwrap(),
                WorkspaceId::new("local-workspace").unwrap(),
                SessionId::new("research-session-1").unwrap(),
                "openai-compatible",
            )
            .unwrap(),
            "evolution-memory-1",
            MemoryKind::Lesson,
            "retain the exact verification evidence",
            ContextClassification::Internal,
            Timestamp::from_unix_seconds(10),
            "evaluation:research-fixture",
        )
        .unwrap();
    let base_path = fixture.root.join("base-prompt.txt");
    let output_path = fixture.root.join("candidate-prompt.txt");
    fs::write(&base_path, "Verify every change against a holdout.\n").unwrap();
    let generated = fixture
        .command(&[
            "evolution",
            "generate",
            "--session",
            "research-session-1",
            "--kind",
            "prompt",
            "--target-id",
            "planner.system",
            "--base",
            base_path.to_str().unwrap(),
            "--output",
            output_path.to_str().unwrap(),
            "--json",
        ])
        .env("PANDORA_PROVIDER_API_KEY", "research-fixture-key")
        .output()
        .expect("research generation should start");
    assert_success_with_context(&generated, "evolution generate");
    let generated = parse_json(&generated);
    assert_eq!(generated["state"], "proposed");
    assert_eq!(generated["kind"], "prompt");
    assert_eq!(generated["memory_evidence_ids"][0], "evolution-memory-1");
    assert_eq!(generated["runtime_authority_changed"], false);
    assert_eq!(fs::read(&output_path).unwrap(), candidate);
    let inspected = fixture
        .command(&[
            "evolution",
            "inspect",
            "--id",
            "research-prompt-1",
            "--json",
        ])
        .output()
        .expect("research inspect should start");
    assert_success_with_context(&inspected, "evolution inspect");
    let inspected = parse_json(&inspected);
    assert_eq!(
        inspected["research_candidate"]["target_id"],
        "planner.system"
    );
    assert_eq!(
        inspected["research_candidate"]["provider"],
        "openai-compatible"
    );
    assert_eq!(
        inspected["proposal"]["memory_evidence_ids"][0],
        "evolution-memory-1"
    );

    let holdout_path = fixture.root.join("research-holdout.json");
    fs::write(
        &holdout_path,
        br#"{"cases":[{"id":"research-case","execution_id":"research-execution","output":"candidate","expected_output":"candidate","baseline_output":"candidate"}]}"#,
    )
    .unwrap();
    let evaluated = fixture
        .command(&[
            "evolution",
            "evaluate",
            "--id",
            "research-prompt-1",
            "--input",
            holdout_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("research evaluation should start");
    assert_success_with_context(&evaluated, "evolution evaluate");
    let approval_path = fixture.root.join("research-approval.json");
    fs::write(
        &approval_path,
        serde_json::to_vec(&serde_json::json!({
            "proposal_id": "research-prompt-1",
            "approver": "parliament-fixture",
            "policy_version": 1,
            "artifact_id": generated["candidate_artifact"],
            "signer": "signer-fixture",
            "signature": "research-candidate-signature",
        }))
        .unwrap(),
    )
    .unwrap();
    let approved = fixture
        .command(&[
            "evolution",
            "approve",
            "--input",
            approval_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("research approval should start");
    assert_success_with_context(&approved, "evolution approve");
    let staged = fixture
        .command(&["evolution", "stage", "--id", "research-prompt-1", "--json"])
        .output()
        .expect("research stage should start");
    assert_success_with_context(&staged, "evolution stage");
    let canary_path = fixture.root.join("research-canary.json");
    fs::write(
        &canary_path,
        br#"{"proposal_id":"research-prompt-1","passed":true,"failure_count":0,"note":"research canary passed"}"#,
    )
    .unwrap();
    let canary = fixture
        .command(&[
            "evolution",
            "canary",
            "--input",
            canary_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("research canary should start");
    assert_success_with_context(&canary, "evolution canary");
    let activated = fixture
        .command(&[
            "evolution",
            "activate",
            "--id",
            "research-prompt-1",
            "--json",
        ])
        .output()
        .expect("research activation should start");
    assert_success_with_context(&activated, "evolution activate");
    let activated = parse_json(&activated);
    assert_eq!(activated["activation_scope"]["kind"], "prompt");
    assert_eq!(activated["activation_scope"]["research_only"], true);
    let rolled_back = fixture
        .command(&[
            "evolution",
            "rollback",
            "--id",
            "research-prompt-1",
            "--reason",
            "research rollback verification",
            "--json",
        ])
        .output()
        .expect("research rollback should start");
    assert_success_with_context(&rolled_back, "evolution rollback");
    server.join().expect("research provider should finish");
}

#[test]
fn evolution_cli_activates_only_admitted_artifacts_and_rolls_back() {
    let fixture = Fixture::new();
    fixture.setup();
    let base_artifact = wat::parse_str(
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
    let candidate_artifact = wat::parse_str(
        r#"(module
            (memory (export "memory") 1)
            (data (i32.const 1024) "{\"generation\":\"candidate\"}")
            (func (export "pandora_alloc") (param i32) (result i32) i32.const 0)
            (func (export "pandora_run") (param i32 i32) (result i64)
                i64.const 1024
                i64.const 32
                i64.shl
                i64.const 26
                i64.or))"#,
    )
    .unwrap();
    let base_manifest = PackageManifest::new(
        "evolution/base-gene",
        "1.0.0",
        PackageKind::Gene,
        "local-publisher",
        hash_artifact(&base_artifact),
        Vec::new(),
        PackageCompatibility::new(concat!("pandora>=", env!("CARGO_PKG_VERSION"))).unwrap(),
        "MIT",
        TrustEvidence::unsigned(),
    )
    .unwrap();
    let candidate_manifest = PackageManifest::new(
        "evolution/candidate-gene",
        "1.0.0",
        PackageKind::Gene,
        "local-publisher",
        hash_artifact(&candidate_artifact),
        Vec::new(),
        PackageCompatibility::new(concat!("pandora>=", env!("CARGO_PKG_VERSION"))).unwrap(),
        "MIT",
        TrustEvidence::unsigned(),
    )
    .unwrap();
    let domain_artifact = b"evolution domain profile\n";
    let domain_manifest = PackageManifest::new(
        "evolution/domain",
        "1.0.0",
        PackageKind::DomainHarness,
        "local-publisher",
        hash_artifact(domain_artifact),
        vec![PackageDependency::new("evolution/base-gene", "1.0.0", false).unwrap()],
        PackageCompatibility::new(concat!("pandora>=", env!("CARGO_PKG_VERSION"))).unwrap(),
        "MIT",
        TrustEvidence::unsigned(),
    )
    .unwrap();

    for (name, manifest, artifact) in [
        ("base", &base_manifest, base_artifact.as_slice()),
        (
            "candidate",
            &candidate_manifest,
            candidate_artifact.as_slice(),
        ),
        ("domain", &domain_manifest, domain_artifact.as_slice()),
    ] {
        let manifest_path = fixture.root.join(format!("{name}-evolution.json"));
        let artifact_path = fixture.root.join(format!("{name}-evolution.artifact"));
        fs::write(&manifest_path, serde_json::to_vec_pretty(manifest).unwrap()).unwrap();
        fs::write(&artifact_path, artifact).unwrap();
        let admitted = fixture
            .command(&[
                "package",
                "admit",
                "--manifest",
                manifest_path.to_str().unwrap(),
                "--artifact",
                artifact_path.to_str().unwrap(),
                "--json",
            ])
            .output()
            .expect("evolution package admission should start");
        assert_success_with_context(&admitted, "evolution package admission");
    }

    for (id, version) in [
        ("evolution/base-gene", "1.0.0"),
        ("evolution/candidate-gene", "1.0.0"),
        ("evolution/domain", "1.0.0"),
    ] {
        let enabled = fixture
            .command(&["package", "enable", id, version, "--yes", "--json"])
            .output()
            .expect("evolution package enable should start");
        assert_success_with_context(&enabled, "evolution package enable");
    }

    let proposal_path = fixture.root.join("admitted-proposal.json");
    fs::write(
        &proposal_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "proposal_id": "proposal-admitted-cli",
            "source": "gepa",
            "base_artifact": base_manifest.content_hash(),
            "candidate_artifact": candidate_manifest.content_hash(),
            "evidence_digest": "evidence-admitted-cli",
            "expected_outcome": "improve admitted gene reliability"
        }))
        .unwrap(),
    )
    .unwrap();
    let submitted = fixture
        .command(&[
            "evolution",
            "submit",
            "--input",
            proposal_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("admitted evolution submit should start");
    assert_success_with_context(&submitted, "admitted evolution submit");

    let holdout_path = fixture.root.join("admitted-holdout.json");
    fs::write(
        &holdout_path,
        br#"{"cases":[{"id":"case-admitted-cli","execution_id":"execution-admitted-cli","output":"candidate","expected_output":"candidate","baseline_output":"candidate"}]}"#,
    )
    .unwrap();
    let evaluated = fixture
        .command(&[
            "evolution",
            "evaluate",
            "--id",
            "proposal-admitted-cli",
            "--input",
            holdout_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("admitted evolution evaluation should start");
    assert_success_with_context(&evaluated, "admitted evolution evaluation");

    let approval_path = fixture.root.join("admitted-approval.json");
    fs::write(
        &approval_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "proposal_id": "proposal-admitted-cli",
            "approver": "parliament-cli",
            "policy_version": 1,
            "artifact_id": candidate_manifest.content_hash(),
            "signer": "signer-cli",
            "signature": "signed-admitted-candidate"
        }))
        .unwrap(),
    )
    .unwrap();
    let approved = fixture
        .command(&[
            "evolution",
            "approve",
            "--input",
            approval_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("admitted evolution approval should start");
    assert_success_with_context(&approved, "admitted evolution approval");

    let staged = fixture
        .command(&[
            "evolution",
            "stage",
            "--id",
            "proposal-admitted-cli",
            "--json",
        ])
        .output()
        .expect("evolution staging should start");
    assert_success_with_context(&staged, "evolution staging");
    assert_eq!(parse_json(&staged)["state"], "staged");

    let canary_path = fixture.root.join("admitted-canary.json");
    fs::write(
        &canary_path,
        br#"{"proposal_id":"proposal-admitted-cli","passed":true,"failure_count":0,"note":"shadow canary passed"}"#,
    )
    .unwrap();
    let canary = fixture
        .command(&[
            "evolution",
            "canary",
            "--input",
            canary_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("evolution canary recording should start");
    assert_success_with_context(&canary, "evolution canary recording");
    assert_eq!(parse_json(&canary)["state"], "canary_passed");

    let activated = fixture
        .command(&[
            "evolution",
            "activate",
            "--id",
            "proposal-admitted-cli",
            "--json",
        ])
        .output()
        .expect("admitted evolution activation should start");
    assert_success_with_context(&activated, "admitted evolution activation");
    let activated = parse_json(&activated);
    assert_eq!(activated["state"], "active");
    assert_eq!(activated["runtime_authority_changed"], false);
    assert_eq!(
        activated["candidate_artifact"],
        candidate_manifest.content_hash()
    );

    let run_gene = |task: &str, existing_session: Option<&str>| -> (Value, String) {
        let mut pending_args = vec![
            "run",
            "--harness",
            "evolution/domain",
            "--harness-version",
            "1.0.0",
            "--gene",
            "evolution/base-gene",
            task,
        ];
        if let Some(session_id) = existing_session {
            pending_args.extend(["--session", session_id]);
        }
        pending_args.push("--json");
        let pending = fixture
            .command(&pending_args)
            .output()
            .expect("evolved Wasm Gene should request approval");
        assert_eq!(
            pending.status.code(),
            Some(40),
            "unexpected evolved Wasm response: stdout={} stderr={}",
            String::from_utf8_lossy(&pending.stdout),
            String::from_utf8_lossy(&pending.stderr)
        );
        let pending = parse_json(&pending);
        let approval_id = pending["details"]["approval_id"].as_str().unwrap();
        let session_id = pending["details"]["session_id"].as_str().unwrap();
        let inspected = fixture
            .command(&["approval", "inspect", approval_id, "--json"])
            .output()
            .expect("evolved Wasm approval should be inspectable");
        assert_success_with_context(&inspected, "inspect evolved Wasm approval");
        let summary = parse_json(&inspected)["approval"]["request_summary"]
            .as_str()
            .unwrap()
            .to_owned();
        let resolved = fixture
            .command(&["approval", "resolve", approval_id, "--allow", "--json"])
            .output()
            .expect("evolved Wasm approval should resolve");
        assert_success_with_context(&resolved, "resolve evolved Wasm approval");
        let completed = fixture
            .command(&[
                "run",
                "--approval",
                approval_id,
                "--session",
                session_id,
                "--harness",
                "evolution/domain",
                "--harness-version",
                "1.0.0",
                "--gene",
                "evolution/base-gene",
                task,
                "--json",
            ])
            .output()
            .expect("approved evolved Wasm Gene should run");
        assert_success_with_context(&completed, "approved evolved Wasm Gene");
        (parse_json(&completed), summary)
    };

    let (candidate_run, candidate_approval) = run_gene(r#"{"generation":"input"}"#, None);
    assert_eq!(candidate_run["output"], r#"{"generation":"candidate"}"#);
    assert_eq!(
        candidate_run["artifact_resolution"]["replacement_active"],
        true
    );
    assert_eq!(
        candidate_run["artifact_resolution"]["base_artifact"],
        base_manifest.content_hash()
    );
    assert_eq!(
        candidate_run["artifact_resolution"]["resolved_artifact"],
        candidate_manifest.content_hash()
    );
    assert_eq!(
        candidate_run["artifact_resolution"]["runtime_authority_changed"],
        false
    );
    assert!(candidate_approval.contains(base_manifest.content_hash()));
    assert!(candidate_approval.contains(candidate_manifest.content_hash()));
    assert!(candidate_approval.contains("runtime authority unchanged"));

    let rolled_back = fixture
        .command(&[
            "evolution",
            "rollback",
            "--id",
            "proposal-admitted-cli",
            "--reason",
            "operator regression review",
            "--json",
        ])
        .output()
        .expect("admitted evolution rollback should start");
    assert_success_with_context(&rolled_back, "admitted evolution rollback");
    let rolled_back = parse_json(&rolled_back);
    assert_eq!(rolled_back["state"], "rolled_back");
    assert_eq!(
        rolled_back["restored_artifact"],
        base_manifest.content_hash()
    );

    let base_task = r#"{"generation":"base-input"}"#;
    let candidate_session = candidate_run["session_id"].as_str().unwrap();
    let (base_run, base_approval) = run_gene(base_task, Some(candidate_session));
    assert_eq!(base_run["output"], base_task);
    assert_eq!(base_run["artifact_resolution"]["replacement_active"], false);
    assert_eq!(
        base_run["artifact_resolution"]["resolved_artifact"],
        base_manifest.content_hash()
    );
    assert!(base_approval.contains(base_manifest.content_hash()));
    assert!(!base_approval.contains(candidate_manifest.content_hash()));

    let inspected = fixture
        .command(&[
            "evolution",
            "inspect",
            "--id",
            "proposal-admitted-cli",
            "--json",
        ])
        .output()
        .expect("rolled-back evolution inspection should start");
    assert_success_with_context(&inspected, "rolled-back evolution inspection");
    assert_eq!(parse_json(&inspected)["state"], "rolled_back");
}

#[test]
fn service_start_reports_a_loopback_endpoint_and_token_path() {
    let fixture = Fixture::new();
    fixture.setup();
    let mut child = fixture
        .command(&["service", "start", "--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("service should start");
    let stdout = child
        .stdout
        .take()
        .expect("service stdout should be available");
    let mut readiness = String::new();
    let bytes = BufReader::new(stdout)
        .read_line(&mut readiness)
        .expect("service readiness should be readable");

    assert!(bytes > 0, "service should print readiness JSON");
    let readiness =
        serde_json::from_str::<Value>(&readiness).expect("service readiness should be valid JSON");
    assert!(
        readiness["endpoint"]
            .as_str()
            .expect("readiness should include an endpoint")
            .starts_with("http://127.0.0.1:")
    );
    assert!(readiness["endpoint"].as_str().unwrap().ends_with("/v1/rpc"));
    assert!(readiness.get("token").is_none());
    assert_eq!(
        readiness["token_path"]
            .as_str()
            .expect("readiness should include a token path"),
        fixture.data.join("service-token").to_string_lossy(),
    );
    assert!(fixture.data.join("service-token").is_file());
    assert_eq!(
        readiness["device_key_path"]
            .as_str()
            .expect("readiness should include a device key path"),
        fixture.data.join("service-device.key").to_string_lossy(),
    );
    let device_key =
        DeviceKeyStore::load_or_create(fixture.data.join("service-device.key")).unwrap();
    assert_eq!(readiness["device_id"], device_key.device_id());
    let address = readiness["endpoint"]
        .as_str()
        .unwrap()
        .strip_prefix("http://")
        .unwrap()
        .strip_suffix("/v1/rpc")
        .unwrap();
    let token = fs::read_to_string(fixture.data.join("service-token"))
        .expect("service token should be readable from its declared path");
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"runtime.health"}"#;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let nonce = "a".repeat(32);
    let proof = DeviceProofRequest::new(
        &token,
        timestamp,
        &nonce,
        "POST",
        "/v1/rpc",
        body.as_bytes(),
    );
    let signature = device_key.sign(&proof).unwrap();
    let mut stream =
        TcpStream::connect(address).expect("service endpoint should accept loopback RPC");
    stream
        .write_all(
            format!(
                "POST /v1/rpc HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\nX-Pandora-Device-Id: {}\r\nX-Pandora-Timestamp: {timestamp}\r\nX-Pandora-Nonce: {nonce}\r\nX-Pandora-Signature: {signature}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                device_key.device_id(),
                body.len(),
            )
            .as_bytes(),
        )
        .expect("service RPC should be sent");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("service RPC response should be readable");
    assert!(response.starts_with("HTTP/1.1 200"));
    assert!(response.contains("\"status\":\"ready\""));

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn release_update_rejects_an_invalid_tag_before_network_access() {
    let fixture = Fixture::new();
    let output = fixture
        .command(&[
            "update",
            "--release",
            "vnot-a-version",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("update should start");

    assert_eq!(output.status.code(), Some(70));
    let response = parse_json(&output);
    assert_eq!(response["code"], "update_error");
    assert_eq!(response["details"]["reason"], "invalid_release");
}

#[test]
fn subagent_list_returns_a_versioned_scoped_empty_lifecycle() {
    let fixture = Fixture::new();
    fixture.setup();

    let output = fixture
        .command(&["subagent", "list", "--json"])
        .output()
        .expect("subagent list should start");

    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["version"], "0.1");
    assert_eq!(response["command"], "subagent list");
    assert_eq!(response["count"], 0);
    assert_eq!(response["subagents"], serde_json::json!([]));
}

#[test]
fn subagent_cli_scopes_cancellation_interruption_and_worker_limits() {
    let fixture = Fixture::new();
    fixture.setup();
    let scope = local_subagent_scope();
    seed_subagent(&fixture, "subagent-running", scope.clone(), true);
    seed_subagent(&fixture, "subagent-queued", scope.clone(), false);
    seed_subagent(
        &fixture,
        "subagent-other-scope",
        SubagentScope::new(
            PrincipalId::new("other-user").unwrap(),
            TenantId::new("other-tenant").unwrap(),
            WorkspaceId::new("other-workspace").unwrap(),
        ),
        false,
    );

    let listed = fixture
        .command(&["subagent", "list", "--json"])
        .output()
        .expect("subagent list should start");
    assert_success(&listed);
    assert_eq!(parse_json(&listed)["count"], 2);
    assert!(!parse_json(&listed).to_string().contains("other-scope"));

    let queued = fixture
        .command(&["subagent", "cancel", "subagent-queued", "--json"])
        .output()
        .expect("queued cancellation should start");
    assert_success(&queued);
    assert_eq!(parse_json(&queued)["lifecycle"]["status"], "cancelled");

    let running = fixture
        .command(&["subagent", "cancel", "subagent-running", "--json"])
        .output()
        .expect("running cancellation should start");
    assert_success(&running);
    let running = parse_json(&running);
    assert_eq!(running["lifecycle"]["status"], "running");
    assert!(running["worker"]["cancel_requested_at"].is_u64());

    for args in [
        ["subagent", "mark-interrupted", "subagent-running", "--json"].as_slice(),
        [
            "subagent",
            "mark-interrupted",
            "subagent-running",
            "--yes",
            "--reason",
            "   ",
            "--json",
        ]
        .as_slice(),
    ] {
        let output = fixture
            .command(args)
            .output()
            .expect("invalid interruption should start");
        assert_eq!(output.status.code(), Some(2));
        assert_eq!(parse_json(&output)["code"], "usage_error");
    }

    let interrupted = fixture
        .command(&[
            "subagent",
            "mark-interrupted",
            "subagent-running",
            "--yes",
            "--reason",
            "worker exited",
            "--json",
        ])
        .output()
        .expect("interruption should start");
    assert_success(&interrupted);
    let interrupted = parse_json(&interrupted);
    assert_eq!(interrupted["lifecycle"]["status"], "interrupted");
    assert_eq!(interrupted["result"]["code"], "worker_interrupted");
    assert_eq!(interrupted["result"]["outcome_known"], false);

    for count in ["0", "9"] {
        let output = fixture
            .command(&["subagent", "work", "--max-agents", count, "--json"])
            .output()
            .expect("invalid worker limit should start");
        assert_eq!(output.status.code(), Some(2));
        assert_eq!(parse_json(&output)["code"], "usage_error");
    }

    for command in ["inspect", "cancel"] {
        let output = fixture
            .command(&["subagent", command, "subagent-other-scope", "--json"])
            .output()
            .expect("cross-scope operation should start");
        assert_eq!(output.status.code(), Some(50));
        assert_eq!(parse_json(&output)["code"], "execution_failed");
    }
}

#[test]
fn subagent_work_returns_multiple_records_in_id_order() {
    let fixture = Fixture::new();
    fixture.setup();
    let scope = local_subagent_scope();
    seed_subagent(&fixture, "subagent-order-z", scope.clone(), false);
    seed_subagent(&fixture, "subagent-order-a", scope, false);

    let worked = fixture
        .command(&["subagent", "work", "--max-agents", "2", "--json"])
        .output()
        .expect("subagent work should start");
    assert_success(&worked);
    let worked = parse_json(&worked);
    assert_eq!(worked["processed_count"], 2);
    assert_eq!(worked["subagents"][0]["subagent_id"], "subagent-order-a");
    assert_eq!(worked["subagents"][1]["subagent_id"], "subagent-order-z");
    assert_eq!(worked["subagents"][0]["lifecycle"]["status"], "failed");
    let fleet = FleetEngine::open(fixture.data.join("fleet.sqlite3")).unwrap();
    let supervisors = fleet.list_supervisors().unwrap();
    assert_eq!(supervisors.len(), 1);
    assert_eq!(supervisors[0].state().as_str(), "stopped");
    assert!(supervisors[0].process_id().is_some());
    assert!(
        fleet
            .list_leases()
            .unwrap()
            .iter()
            .all(|lease| lease.state().as_str() != "active")
    );
    assert_eq!(worked["subagents"][1]["lifecycle"]["status"], "failed");
}

#[test]
fn fleet_supervisor_restart_requires_a_stale_heartbeat() {
    let fixture = Fixture::new();
    fixture.setup();
    let fleet = FleetEngine::open(fixture.data.join("fleet.sqlite3")).unwrap();
    let node = FleetNode::new(
        "node-restart".to_owned(),
        "2.0.0-beta.7",
        "local",
        vec!["subagent.work".to_owned()],
        1,
    )
    .unwrap();
    fleet.register_node(&node).unwrap();
    fleet
        .start_supervisor_for_process("node-restart", 41, 1)
        .unwrap();

    let output = fixture
        .command(&[
            "fleet",
            "supervisor",
            "restart",
            "--node",
            "node-restart",
            "--process-id",
            "42",
            "--stale-after",
            "10",
            "--now",
            "20",
            "--json",
        ])
        .output()
        .expect("fleet supervisor restart should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["supervisor"]["state"], "running");
    assert_eq!(response["supervisor"]["generation"], 2);
    assert_eq!(response["supervisor"]["process_id"], 42);
}

#[test]
fn fleet_supervisor_reap_is_exposed_as_a_bounded_cli_operation() {
    let fixture = Fixture::new();
    fixture.setup();
    let fleet = FleetEngine::open(fixture.data.join("fleet.sqlite3")).unwrap();
    let node = FleetNode::new(
        "node-reap".to_owned(),
        "2.0.0-beta.7",
        "local",
        vec!["subagent.work".to_owned()],
        1,
    )
    .unwrap();
    fleet.register_node(&node).unwrap();
    fleet.start_supervisor("node-reap", 1).unwrap();

    let output = fixture
        .command(&[
            "fleet",
            "supervisor",
            "reap",
            "--stale-after",
            "10",
            "--now",
            "20",
            "--json",
        ])
        .output()
        .expect("fleet supervisor reap should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["reaped"], 1);
    assert_eq!(response["supervisors"][0]["node_id"], "node-reap");
    assert_eq!(response["supervisors"][0]["state"], "recovering");
}

#[test]
fn fleet_dashboard_aggregates_operations_without_sensitive_payloads() {
    let fixture = Fixture::new();
    fixture.setup();
    let fleet = FleetEngine::open(fixture.data.join("fleet.sqlite3")).unwrap();
    let node = FleetNode::new(
        "node-dashboard".to_owned(),
        "2.0.0-beta.7",
        "local",
        vec!["job.work".to_owned()],
        1,
    )
    .unwrap();
    fleet.register_node(&node).unwrap();
    fleet
        .start_supervisor_for_process("node-dashboard", 41, 10)
        .unwrap();
    fleet
        .acquire_lease(
            "lease-dashboard".to_owned(),
            "node-dashboard",
            "execution-dashboard",
            FleetBudget::new(1_000, 20, 300, 50_000),
            10,
            100,
        )
        .unwrap();

    let principal = PrincipalId::new("local-user").unwrap();
    let tenant = TenantId::new("local-tenant").unwrap();
    let workspace = WorkspaceId::new("local-workspace").unwrap();
    let jobs = JobStore::open(fixture.data.join("jobs.sqlite3")).unwrap();
    let failed_id = JobId::new("job-dashboard-failed").unwrap();
    jobs.submit(
        &failed_id,
        &principal,
        &tenant,
        &workspace,
        &JobRequest::new(
            JobCommand::Run,
            vec!["private prompt must not appear".to_owned()],
        )
        .unwrap(),
        Timestamp::from_unix_seconds(1),
    )
    .unwrap();
    let worker = JobWorkerId::new("worker-dashboard").unwrap();
    jobs.claim_next(
        &principal,
        &tenant,
        &workspace,
        &worker,
        Timestamp::from_unix_seconds(2),
    )
    .unwrap()
    .unwrap();
    jobs.finish(
        &failed_id,
        &principal,
        &tenant,
        &workspace,
        &worker,
        JobStatus::Failed,
        &serde_json::json!({"output": "private provider output must not appear"}),
        Timestamp::from_unix_seconds(3),
    )
    .unwrap();
    jobs.submit(
        &JobId::new("job-dashboard-queued").unwrap(),
        &principal,
        &tenant,
        &workspace,
        &JobRequest::new(JobCommand::Run, vec!["another private prompt".to_owned()]).unwrap(),
        Timestamp::from_unix_seconds(4),
    )
    .unwrap();

    let output = fixture
        .command(&[
            "fleet",
            "dashboard",
            "--now",
            "80",
            "--stale-after",
            "30",
            "--json",
        ])
        .output()
        .expect("fleet dashboard should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["command"], "fleet dashboard");
    assert_eq!(response["health"]["status"], "attention");
    assert_eq!(response["health"]["ready_nodes"], 1);
    assert_eq!(response["health"]["stale_supervisors"], 1);
    assert_eq!(response["fleet"]["leases"]["by_state"]["active"], 1);
    assert_eq!(response["fleet"]["leases"]["active"][0]["age_seconds"], 70);
    assert_eq!(response["queue"]["jobs"]["queued"], 1);
    assert_eq!(response["queue"]["jobs"]["failure_count"], 1);
    assert_eq!(response["failures"]["records"][0]["id"], failed_id.as_str());
    assert_eq!(response["budget_ceilings"]["max_tokens"], 1_000);
    assert_eq!(response["budget_ceilings"]["max_tools"], 20);
    assert_eq!(response["budget_ceilings"]["max_cost_micros"], 50_000);
    assert_eq!(response["budget_ceilings"]["actual_spend_available"], false);
    assert_eq!(response["boundary"]["read_only"], true);
    assert_eq!(response["boundary"]["runtime_authority"], false);
    assert_eq!(response["boundary"]["prompts_included"], false);
    assert_eq!(response["boundary"]["outputs_included"], false);
    assert_eq!(response["boundary"]["credentials_included"], false);
    assert_eq!(response["boundary"]["hidden_reasoning_included"], false);
    let serialized = response.to_string();
    assert!(!serialized.contains("private prompt"));
    assert!(!serialized.contains("private provider output"));
}

#[test]
fn fleet_dashboard_rejects_unbounded_staleness_windows() {
    let fixture = Fixture::new();
    for value in ["0", "86401", "invalid"] {
        let output = fixture
            .command(&["fleet", "dashboard", "--stale-after", value, "--json"])
            .output()
            .expect("invalid fleet dashboard should start");
        assert_eq!(output.status.code(), Some(2));
        assert_eq!(parse_json(&output)["code"], "usage_error");
    }
}

#[test]
fn subagent_list_and_inspect_filter_legacy_approval_details() {
    let fixture = Fixture::new();
    fixture.setup();
    let scope = local_subagent_scope();
    seed_subagent(&fixture, "subagent-approval-result", scope, true);
    let store = SubagentStore::open(fixture.data.join("jobs.sqlite3")).unwrap();
    store
        .finish(
            &SubagentId::new("subagent-approval-result").unwrap(),
            &JobWorkerId::new("worker-subagent-approval-result").unwrap(),
            pandora_types::SubagentStatus::ApprovalRequired,
            &serde_json::json!({
                "code": "approval_required",
                "details": {"approval_id": "approval-secret", "raw_response": "provider-secret"},
            }),
            Timestamp::from_unix_seconds(40),
        )
        .unwrap();

    for args in [
        ["subagent", "list", "--json"].as_slice(),
        ["subagent", "inspect", "subagent-approval-result", "--json"].as_slice(),
    ] {
        let output = fixture
            .command(args)
            .output()
            .expect("subagent query should start");
        assert_success(&output);
        let response = parse_json(&output);
        assert!(response.to_string().contains("approval_required"));
        assert!(!response.to_string().contains("approval_id"));
        assert!(!response.to_string().contains("approval-secret"));
        assert!(!response.to_string().contains("provider-secret"));
    }
}

#[test]
fn subagent_list_and_inspect_reject_arbitrary_legacy_result_shapes() {
    let fixture = Fixture::new();
    fixture.setup();
    let cases = [
        ("string", serde_json::json!("approval-secret"), Value::Null),
        ("array", serde_json::json!(["provider-secret"]), Value::Null),
        (
            "nested",
            serde_json::json!({"reason": {"approval_id": "approval-secret"}}),
            Value::Null,
        ),
        (
            "unknown",
            serde_json::json!({"code": "completed", "raw_response": "provider-secret"}),
            Value::Null,
        ),
        (
            "valid",
            serde_json::json!({"code": "worker_interrupted", "outcome_known": false}),
            serde_json::json!({"code": "worker_interrupted", "outcome_known": false}),
        ),
    ];

    for (suffix, result, expected) in &cases {
        let id = format!("subagent-legacy-{suffix}");
        seed_subagent(&fixture, &id, local_subagent_scope(), true);
        SubagentStore::open(fixture.data.join("jobs.sqlite3"))
            .unwrap()
            .finish(
                &SubagentId::new(&id).unwrap(),
                &JobWorkerId::new(format!("worker-{id}")).unwrap(),
                pandora_types::SubagentStatus::Failed,
                result,
                Timestamp::from_unix_seconds(40),
            )
            .unwrap();
        let inspected = fixture
            .command(&["subagent", "inspect", &id, "--json"])
            .output()
            .expect("subagent inspect should start");
        assert_success(&inspected);
        assert_eq!(parse_json(&inspected)["result"], *expected, "{suffix}");
    }

    let listed = fixture
        .command(&["subagent", "list", "--json"])
        .output()
        .expect("subagent list should start");
    assert_success(&listed);
    let listed = parse_json(&listed);
    assert_eq!(listed["count"], cases.len());
    assert!(!listed.to_string().contains("approval-secret"));
    assert!(!listed.to_string().contains("provider-secret"));
}

#[test]
fn subagent_spawn_materializes_default_bindings_for_a_scoped_parent_execution() {
    let fixture = Fixture::new();
    let exact_commit = fixture.initialize_git_workspace();
    fixture.setup();

    let parent = fixture
        .command(&["run", "read:README.md", "--json"])
        .output()
        .expect("parent run should start");
    assert_success(&parent);
    let parent = parse_json(&parent);
    let session_id = parent["session_id"]
        .as_str()
        .expect("parent run should return a session ID");
    let execution_id = parent["execution_id"]
        .as_str()
        .expect("parent run should return an execution ID");

    let spawned = fixture
        .command(&[
            "subagent",
            "spawn",
            "--session",
            session_id,
            "--execution",
            execution_id,
            "--provider",
            "openai-compatible",
            "--harness",
            "coding",
            "--max-turns",
            "1",
            "--max-tools",
            "1",
            "--max-tokens",
            "100",
            "--max-duration",
            "60",
            "--max-depth",
            "1",
            "--max-result-bytes",
            "8192",
            "Read the README",
            "--json",
        ])
        .output()
        .expect("subagent spawn should start");
    assert_success_with_context(&spawned, "subagent spawn");
    let spawned = parse_json(&spawned);
    assert_eq!(spawned["command"], "subagent spawn");
    assert_eq!(spawned["lifecycle"]["status"], "queued");
    assert_eq!(spawned["request"]["exact_commit"], exact_commit);
    assert_eq!(spawned["request"]["provider_profile"], "openai-compatible");
    assert_eq!(spawned["request"]["harness"]["harness_id"], "coding-domain");
    assert!(spawned["request"]["harness"]["harness_id"].is_string());
    assert!(spawned["request"]["harness"]["version"].is_string());
    assert_eq!(spawned["worktree"]["state"], "ready");
    assert!(spawned["receipts"]["create"]["receipt_id"].is_string());
}

#[test]
fn subagent_binding_provider_model_drift_stops_before_provider_call() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("provider fixture should bind");
    let provider_url = format!(
        "http://{}/v1",
        listener
            .local_addr()
            .expect("provider fixture should expose its address")
    );
    let server = loopback_provider_calls(listener);
    let fixture = Fixture::new();
    fixture.initialize_git_workspace();
    fixture.setup();
    let parent = fixture
        .command(&["run", "read:README.md", "--json"])
        .output()
        .expect("parent run should start");
    assert_success(&parent);
    let parent = parse_json(&parent);
    let configured = fixture
        .command(&[
            "provider",
            "set",
            "--provider-url",
            &provider_url,
            "--model",
            "fixture-model",
            "--json",
        ])
        .output()
        .expect("provider setup should start");
    assert_success(&configured);
    let spawned = fixture
        .command(&[
            "subagent",
            "spawn",
            "--session",
            parent["session_id"].as_str().unwrap(),
            "--execution",
            parent["execution_id"].as_str().unwrap(),
            "--provider",
            "openai-compatible",
            "--harness",
            "coding",
            "Read the README",
            "--json",
        ])
        .output()
        .expect("subagent spawn should start");
    assert_success(&spawned);
    let configured = fixture
        .command(&[
            "provider",
            "set",
            "--provider-url",
            &provider_url,
            "--model",
            "fixture-model-drifted",
            "--json",
        ])
        .output()
        .expect("provider drift should be configured");
    assert_success(&configured);

    let worked = fixture
        .command(&["subagent", "work", "--json"])
        .env("PANDORA_PROVIDER_API_KEY", "fixture-provider-key")
        .output()
        .expect("subagent work should start");
    assert_success(&worked);
    let worked = parse_json(&worked);
    assert_eq!(worked["subagents"][0]["lifecycle"]["status"], "failed");
    assert_eq!(
        worked["subagents"][0]["result"],
        serde_json::json!({"code": "subagent_binding_changed", "status": "failed"})
    );
    assert_eq!(server.join().expect("provider fixture should finish"), 0);
}

#[test]
fn subagent_binding_harness_drift_stops_before_provider_call() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("provider fixture should bind");
    let provider_url = format!(
        "http://{}/v1",
        listener
            .local_addr()
            .expect("provider fixture should expose its address")
    );
    let server = loopback_provider_calls(listener);
    let fixture = Fixture::new();
    fixture.initialize_git_workspace();
    fixture.setup();
    admit_domain_harness(
        &fixture,
        b"subagent Harness original\n",
        &["workspace.read"],
    );
    let enabled = fixture
        .command(&[
            "package",
            "enable",
            "example/subagent-domain",
            "1.0.0",
            "--yes",
            "--json",
        ])
        .output()
        .expect("domain Harness enable should start");
    assert_success(&enabled);
    let parent = fixture
        .command(&["run", "read:README.md", "--json"])
        .output()
        .expect("parent run should start");
    assert_success(&parent);
    let parent = parse_json(&parent);
    let configured = fixture
        .command(&[
            "provider",
            "set",
            "--provider-url",
            &provider_url,
            "--model",
            "fixture-model",
            "--json",
        ])
        .output()
        .expect("provider setup should start");
    assert_success(&configured);
    let spawned = fixture
        .command(&[
            "subagent",
            "spawn",
            "--session",
            parent["session_id"].as_str().unwrap(),
            "--execution",
            parent["execution_id"].as_str().unwrap(),
            "--provider",
            "openai-compatible",
            "--harness",
            "example/subagent-domain",
            "--harness-version",
            "1.0.0",
            "Read the README",
            "--json",
        ])
        .output()
        .expect("subagent spawn should start");
    assert_success(&spawned);
    let disabled = fixture
        .command(&[
            "package",
            "disable",
            "example/subagent-domain",
            "1.0.0",
            "--yes",
            "--json",
        ])
        .output()
        .expect("domain Harness disable should start");
    assert_success(&disabled);
    let removed = fixture
        .command(&[
            "package",
            "remove",
            "example/subagent-domain",
            "1.0.0",
            "--yes",
            "--json",
        ])
        .output()
        .expect("domain Harness removal should start");
    assert_success(&removed);
    admit_domain_harness(
        &fixture,
        b"subagent Harness changed\n",
        &["workspace.read", "workspace.search"],
    );
    let enabled = fixture
        .command(&[
            "package",
            "enable",
            "example/subagent-domain",
            "1.0.0",
            "--yes",
            "--json",
        ])
        .output()
        .expect("changed domain Harness enable should start");
    assert_success(&enabled);

    let worked = fixture
        .command(&["subagent", "work", "--json"])
        .env("PANDORA_PROVIDER_API_KEY", "fixture-provider-key")
        .output()
        .expect("subagent work should start");
    assert_success(&worked);
    let worked = parse_json(&worked);
    assert_eq!(worked["subagents"][0]["lifecycle"]["status"], "failed");
    assert_eq!(
        worked["subagents"][0]["result"],
        serde_json::json!({"code": "subagent_binding_changed", "status": "failed"})
    );
    assert_eq!(server.join().expect("provider fixture should finish"), 0);
}

#[test]
fn subagent_cleanup_preserves_a_dirty_managed_worktree() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("provider fixture should bind");
    let provider_url = format!(
        "http://{}/v1",
        listener
            .local_addr()
            .expect("provider fixture should expose its address")
    );
    let server = expected_provider_call(listener);
    let fixture = Fixture::new();
    fixture.initialize_git_workspace();
    fixture.setup();
    let parent = fixture
        .command(&["run", "read:README.md", "--json"])
        .output()
        .expect("parent run should start");
    assert_success(&parent);
    let parent = parse_json(&parent);
    let configured = fixture
        .command(&[
            "provider",
            "set",
            "--provider-url",
            &provider_url,
            "--model",
            "fixture-model",
            "--json",
        ])
        .output()
        .expect("provider setup should start");
    assert_success(&configured);
    let spawned = fixture
        .command(&[
            "subagent",
            "spawn",
            "--session",
            parent["session_id"].as_str().unwrap(),
            "--execution",
            parent["execution_id"].as_str().unwrap(),
            "Read the README",
            "--json",
        ])
        .output()
        .expect("subagent spawn should start");
    assert_success(&spawned);
    let subagent_id = parse_json(&spawned)["subagent_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let worked = fixture
        .command(&["subagent", "work", "--json"])
        .env("PANDORA_PROVIDER_API_KEY", "fixture-provider-key")
        .output()
        .expect("subagent work should start");
    assert_success(&worked);
    assert_eq!(
        parse_json(&worked)["subagents"][0]["lifecycle"]["status"],
        "completed",
        "{}",
        parse_json(&worked)
    );
    assert_eq!(server.join().expect("provider fixture should finish"), 1);
    let inspected = fixture
        .command(&["subagent", "inspect", &subagent_id, "--json"])
        .output()
        .expect("subagent inspect should start");
    assert_success(&inspected);
    let worktree = PathBuf::from(parse_json(&inspected)["worktree"]["path"].as_str().unwrap());
    let dirty_file = worktree.join("dirty.txt");
    fs::write(&dirty_file, "preserve\n").expect("child worktree should become dirty");

    let cleaned = fixture
        .command(&["subagent", "cleanup", &subagent_id, "--yes", "--json"])
        .output()
        .expect("subagent cleanup should start");
    assert_eq!(cleaned.status.code(), Some(50));
    assert_eq!(parse_json(&cleaned)["code"], "execution_failed");
    assert!(worktree.is_dir());
    assert!(dirty_file.is_file());
    let inspected = fixture
        .command(&["subagent", "inspect", &subagent_id, "--json"])
        .output()
        .expect("subagent inspect should start");
    assert_success(&inspected);
    let inspected = parse_json(&inspected);
    assert_eq!(inspected["lifecycle"]["status"], "completed");
    assert_eq!(inspected["worktree"]["state"], "preserved");
    assert_eq!(
        inspected["receipts"]["remove"]["outcome"]["status"],
        "failed"
    );
}

#[test]
fn subagent_help_and_completion_surfaces_list_all_lifecycle_commands() {
    let fixture = Fixture::new();
    let help = fixture
        .command(&["help"])
        .output()
        .expect("help should start");
    assert_success(&help);
    let help = String::from_utf8(help.stdout).expect("help should be UTF-8");
    assert!(help.contains("subagent"));
    for command in [
        "spawn",
        "work",
        "list",
        "inspect",
        "cancel",
        "mark-interrupted",
        "cleanup",
    ] {
        assert!(help.contains(command), "help missing {command}");
    }

    for shell in ["powershell", "bash", "zsh", "fish"] {
        let output = fixture
            .command(&["completions", shell, "--json"])
            .output()
            .expect("completion generation should start");
        assert_success(&output);
        let script = parse_json(&output)["script"].as_str().unwrap().to_owned();
        assert!(script.contains("subagent"), "{shell} missing subagent");
        for command in [
            "spawn",
            "work",
            "list",
            "inspect",
            "cancel",
            "mark-interrupted",
            "cleanup",
        ] {
            assert!(script.contains(command), "{shell} missing {command}");
        }
    }
}

#[test]
fn subagent_work_inspect_and_cleanup_complete_an_exact_commit_lifecycle() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("provider fixture should bind");
    let address = listener
        .local_addr()
        .expect("provider fixture should expose its address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("subagent should connect");
        let mut request = Vec::new();
        let header_end = loop {
            let mut chunk = [0_u8; 1_024];
            let bytes_read = stream
                .read(&mut chunk)
                .expect("subagent request should read");
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
            .expect("subagent should send a content length");
        while request.len() < header_end + content_length {
            let mut chunk = [0_u8; 1_024];
            let bytes_read = stream.read(&mut chunk).expect("subagent body should read");
            request.extend_from_slice(&chunk[..bytes_read]);
        }
        let response = br#"{"choices":[{"message":{"content":"subagent complete"}}],"usage":{"prompt_tokens":2,"completion_tokens":1}}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response.len()
        )
        .expect("subagent response headers should be written");
        stream
            .write_all(response)
            .expect("subagent response should be written");
    });

    let fixture = Fixture::new();
    fixture.initialize_git_workspace();
    fixture.setup();
    let parent = fixture
        .command(&["run", "read:README.md", "--json"])
        .output()
        .expect("parent run should start");
    assert_success(&parent);
    let parent = parse_json(&parent);
    let provider_url = format!("http://{address}/v1");
    let configured = fixture
        .command(&[
            "provider",
            "set",
            "--provider-url",
            &provider_url,
            "--model",
            "fixture-model",
            "--json",
        ])
        .output()
        .expect("provider setup should start");
    assert_success(&configured);
    let spawned = fixture
        .command(&[
            "subagent",
            "spawn",
            "--session",
            parent["session_id"].as_str().unwrap(),
            "--execution",
            parent["execution_id"].as_str().unwrap(),
            "--max-turns",
            "1",
            "--max-tools",
            "1",
            "Read the README",
            "--json",
        ])
        .output()
        .expect("subagent spawn should start");
    assert_success(&spawned);
    let subagent_id = parse_json(&spawned)["subagent_id"]
        .as_str()
        .expect("spawn should return a subagent ID")
        .to_owned();

    let worked = fixture
        .command(&["subagent", "work", "--max-agents", "1", "--json"])
        .env("PANDORA_PROVIDER_API_KEY", "fixture-provider-key")
        .output()
        .expect("subagent work should start");
    assert_success_with_context(&worked, "subagent work");
    let worked = parse_json(&worked);
    assert_eq!(worked["command"], "subagent work");
    assert_eq!(worked["worker_count"], 1);
    assert_eq!(worked["processed_count"], 1);
    assert_eq!(
        worked["subagents"][0]["lifecycle"]["status"], "completed",
        "{worked}"
    );

    let inspected = fixture
        .command(&["subagent", "inspect", &subagent_id, "--json"])
        .output()
        .expect("subagent inspect should start");
    assert_success(&inspected);
    let inspected = parse_json(&inspected);
    assert_eq!(inspected["lifecycle"]["status"], "completed");
    assert_eq!(inspected["result"]["code"], "completed");
    assert_eq!(inspected["result"]["status"], "completed");
    assert!(inspected["result"].get("output").is_none());
    assert!(!inspected.to_string().contains("fixture-provider-key"));

    let cleaned = fixture
        .command(&["subagent", "cleanup", &subagent_id, "--yes", "--json"])
        .output()
        .expect("subagent cleanup should start");
    assert_success_with_context(&cleaned, "subagent cleanup");
    assert_eq!(parse_json(&cleaned)["worktree"]["state"], "removed");

    server.join().expect("provider fixture should finish");
}

#[test]
fn cancellation_during_provider_return_survives_worker_restart_without_replay() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("provider fixture should bind");
    let address = listener
        .local_addr()
        .expect("provider fixture should expose its address");
    let provider = HeldToolProvider::start(listener);

    let fixture = Fixture::new();
    fixture.initialize_git_workspace();
    fixture.setup();
    let parent = fixture
        .command(&["run", "read:README.md", "--json"])
        .output()
        .expect("parent run should start");
    assert_success(&parent);
    let parent = parse_json(&parent);
    let configured = fixture
        .command(&[
            "provider",
            "set",
            "--provider-url",
            &format!("http://{address}/v1"),
            "--model",
            "held-provider-model",
            "--json",
        ])
        .output()
        .expect("provider setup should start");
    assert_success(&configured);
    let spawned = fixture
        .command(&[
            "subagent",
            "spawn",
            "--session",
            parent["session_id"].as_str().unwrap(),
            "--execution",
            parent["execution_id"].as_str().unwrap(),
            "--max-turns",
            "1",
            "--max-tools",
            "1",
            "Read the README",
            "--json",
        ])
        .output()
        .expect("subagent spawn should start");
    assert_success(&spawned);
    let spawned = parse_json(&spawned);
    let subagent_id = spawned["subagent_id"]
        .as_str()
        .expect("spawn should return a subagent ID")
        .to_owned();
    let child_session_id = spawned["child"]["session_id"]
        .as_str()
        .expect("spawn should return a child session ID")
        .to_owned();

    let first_worker = fixture
        .command(&["subagent", "work", "--max-agents", "1", "--json"])
        .env("PANDORA_PROVIDER_API_KEY", "held-provider-key")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("first independent subagent worker should start");
    let first_process_id = first_worker.id();
    provider.wait_for_request();
    assert_eq!(provider.calls(), 1);

    let fleet = fixture
        .command(&["fleet", "list", "--json"])
        .output()
        .expect("fresh fleet inspection should start");
    assert_success(&fleet);
    let fleet = parse_json(&fleet);
    let running = fleet["supervisors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|supervisor| supervisor["node_id"] == "subagent-worker")
        .expect("running subagent supervisor should be inspectable");
    assert_eq!(running["state"], "running");
    assert_eq!(running["process_id"], first_process_id);
    let first_generation = running["generation"]
        .as_u64()
        .expect("supervisor generation should be numeric");
    assert!(
        fleet["leases"]
            .as_array()
            .unwrap()
            .iter()
            .any(|lease| { lease["node_id"] == "subagent-worker" && lease["state"] == "active" }),
        "the first process lease should remain inspectable while the provider is held: {fleet}"
    );

    let cancelled = fixture
        .command(&["subagent", "cancel", &subagent_id, "--json"])
        .output()
        .expect("independent cancellation should start");
    assert_success(&cancelled);
    let cancelled = parse_json(&cancelled);
    assert_eq!(cancelled["lifecycle"]["status"], "running");
    assert!(
        cancelled["worker"]["cancel_requested_at"].is_number(),
        "the cancellation request should be durable before provider release: {cancelled}"
    );

    let drained = fixture
        .command(&["fleet", "supervisor", "drain", "subagent-worker", "--json"])
        .output()
        .expect("independent supervisor drain should start");
    assert_success(&drained);
    let drained = parse_json(&drained);
    assert_eq!(drained["supervisor"]["state"], "draining");
    assert_eq!(drained["supervisor"]["generation"], first_generation);
    assert_eq!(drained["supervisor"]["process_id"], first_process_id);

    provider.release();
    let first_output = wait_for_child(
        first_worker,
        Duration::from_secs(30),
        "first independent subagent worker",
    );
    assert!(
        first_output.status.success() || first_output.status.code() == Some(50),
        "drained worker should exit in a bounded, classified state: stdout={} stderr={}",
        String::from_utf8_lossy(&first_output.stdout),
        String::from_utf8_lossy(&first_output.stderr)
    );

    let inspected = fixture
        .command(&["subagent", "inspect", &subagent_id, "--json"])
        .output()
        .expect("fresh subagent inspection should start");
    assert_success(&inspected);
    let inspected = parse_json(&inspected);
    assert_eq!(inspected["lifecycle"]["status"], "cancelled");
    assert_eq!(inspected["result"]["code"], "agent_controlled_stop");
    assert_eq!(inspected["result"]["status"], "cancelled");
    assert_eq!(inspected["result"]["reason"], "cancelled");
    assert!(inspected["lifecycle"]["finished_at"].is_number());

    let listed = fixture
        .command(&["subagent", "list", "--json"])
        .output()
        .expect("fresh subagent list should start");
    assert_success(&listed);
    let listed = parse_json(&listed);
    assert_eq!(listed["count"], 1);
    assert_eq!(listed["subagents"][0]["subagent_id"], subagent_id);
    assert_eq!(listed["subagents"][0]["lifecycle"]["status"], "cancelled");

    let child_workspace_digest = hash_artifact(subagent_id.as_bytes());
    let child_workspace_digest = child_workspace_digest
        .strip_prefix("sha256:")
        .expect("hash_artifact should return a SHA-256 digest");
    let child_workspace = WorkspaceId::new(format!("subagent-{child_workspace_digest}")).unwrap();
    let child_session = SessionStore::open(fixture.data.join("sessions.sqlite3"))
        .unwrap()
        .resume(
            &SessionId::new(child_session_id).unwrap(),
            &PrincipalId::new("local-user").unwrap(),
            &TenantId::new("local-tenant").unwrap(),
            &child_workspace,
        )
        .expect("child session should survive fresh-process inspection");
    assert!(
        child_session.evaluations().is_empty(),
        "cancellation at the post-provider checkpoint must prevent every governed tool/effect permit"
    );

    let fleet = fixture
        .command(&["fleet", "list", "--json"])
        .output()
        .expect("post-shutdown fleet inspection should start");
    assert_success(&fleet);
    let fleet = parse_json(&fleet);
    let stopped = fleet["supervisors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|supervisor| supervisor["node_id"] == "subagent-worker")
        .expect("stopped subagent supervisor should remain inspectable");
    assert_eq!(stopped["state"], "stopped");
    assert_eq!(stopped["generation"], first_generation);
    assert_eq!(stopped["process_id"], first_process_id);
    let first_updated_at = stopped["updated_at"]
        .as_u64()
        .expect("supervisor update time should be numeric");
    let first_worker_leases = fleet["leases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|lease| lease["node_id"] == "subagent-worker")
        .collect::<Vec<_>>();
    assert_eq!(first_worker_leases.len(), 1);
    assert!(
        first_worker_leases
            .iter()
            .all(|lease| lease["state"] == "released")
    );

    let reconcile_now = (first_updated_at + 31).to_string();
    let reconciled = fixture
        .command(&[
            "fleet",
            "supervisor",
            "reconcile",
            "subagent-worker",
            "--now",
            &reconcile_now,
            "--stale-after",
            "30",
            "--json",
        ])
        .output()
        .expect("fresh supervisor reconciliation should start");
    assert_success(&reconciled);
    let reconciled = parse_json(&reconciled);
    assert_eq!(reconciled["supervisor"]["state"], "stopped");
    assert_eq!(
        reconciled["supervisor"]["generation"], first_generation,
        "reconciliation must not manufacture a replay generation"
    );
    assert_eq!(provider.calls(), 1);

    let second_worker = fixture
        .command(&["subagent", "work", "--max-agents", "1", "--json"])
        .env("PANDORA_PROVIDER_API_KEY", "held-provider-key")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("second independent subagent worker should start");
    let second_process_id = second_worker.id();
    let second_output = wait_for_child(
        second_worker,
        Duration::from_secs(30),
        "second independent subagent worker",
    );
    assert_success_with_context(&second_output, "restarted subagent work");
    let second_output = parse_json(&second_output);
    assert_eq!(second_output["processed_count"], 0);
    assert_eq!(provider.calls(), 1, "restart must not replay the provider");

    let fleet = fixture
        .command(&["fleet", "list", "--json"])
        .output()
        .expect("restarted fleet inspection should start");
    assert_success(&fleet);
    let fleet = parse_json(&fleet);
    let restarted = fleet["supervisors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|supervisor| supervisor["node_id"] == "subagent-worker")
        .expect("restarted supervisor should remain inspectable");
    assert_eq!(restarted["state"], "stopped");
    assert_eq!(restarted["generation"], first_generation + 1);
    assert_eq!(restarted["process_id"], second_process_id);
    let worker_leases = fleet["leases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|lease| lease["node_id"] == "subagent-worker")
        .collect::<Vec<_>>();
    assert_eq!(worker_leases.len(), 2);
    assert!(
        worker_leases
            .iter()
            .all(|lease| lease["state"] == "released")
    );

    let inspected = fixture
        .command(&["subagent", "inspect", &subagent_id, "--json"])
        .output()
        .expect("final fresh subagent inspection should start");
    assert_success(&inspected);
    let inspected = parse_json(&inspected);
    assert_eq!(inspected["lifecycle"]["status"], "cancelled");
    assert_eq!(inspected["result"]["code"], "agent_controlled_stop");
    assert_eq!(provider.finish(), 1);
}

#[test]
fn headless_job_worker_executes_the_existing_run_command() {
    let fixture = Fixture::new();
    fixture.setup();
    let submitted = fixture
        .command(&["job", "submit", "--", "guide", "--json"])
        .output()
        .expect("job submission should start");
    assert_success(&submitted);
    let submitted = parse_json(&submitted);
    let job_id = submitted["job_id"]
        .as_str()
        .expect("submission should return a job ID");
    assert_eq!(submitted["status"], "queued");

    let worked = fixture
        .command(&["job", "work", "--json"])
        .output()
        .expect("job worker should start");
    assert_success(&worked);
    let worked = parse_json(&worked);
    assert_eq!(worked["command"], "job work");
    assert_eq!(worked["job_id"], job_id);
    assert_eq!(worked["status"], "completed");
    assert_eq!(worked["result"]["command"], "run");

    let inspected = fixture
        .command(&["job", "inspect", job_id, "--json"])
        .output()
        .expect("job inspection should start");
    assert_success(&inspected);
    let inspected = parse_json(&inspected);
    assert_eq!(inspected["status"], "completed");
    assert_eq!(inspected["result"]["command"], "run");

    let fleet = FleetEngine::open(fixture.data.join("fleet.sqlite3")).unwrap();
    let supervisor = fleet
        .list_supervisors()
        .unwrap()
        .into_iter()
        .find(|supervisor| supervisor.node_id() == "job-worker")
        .expect("headless job worker supervisor should persist");
    assert_eq!(supervisor.state().as_str(), "stopped");
    assert!(supervisor.process_id().is_some());
    assert!(
        fleet
            .list_leases()
            .unwrap()
            .iter()
            .all(|lease| lease.state().as_str() != "active")
    );
}

#[test]
fn headless_job_worker_drains_a_bounded_fifo_batch() {
    let fixture = Fixture::new();
    fixture.setup();
    let mut job_ids = Vec::new();
    for _ in 0..3 {
        let submitted = fixture
            .command(&["job", "submit", "--", "guide", "--json"])
            .output()
            .expect("job submission should start");
        assert_success(&submitted);
        job_ids.push(
            parse_json(&submitted)["job_id"]
                .as_str()
                .expect("submission should return a job ID")
                .to_owned(),
        );
    }

    let worked = fixture
        .command(&["job", "work", "--max-jobs", "2", "--json"])
        .output()
        .expect("batch worker should start");
    assert_success(&worked);
    let worked = parse_json(&worked);
    assert_eq!(worked["command"], "job work");
    assert_eq!(worked["processed_count"], 2);
    assert_eq!(worked["stop_reason"], "limit_reached");
    assert_eq!(worked["jobs"][0]["job_id"], job_ids[0]);
    assert_eq!(worked["jobs"][1]["job_id"], job_ids[1]);
    assert!(worked["jobs"][0].get("result").is_none());

    let remaining = fixture
        .command(&["job", "work", "--json"])
        .output()
        .expect("remaining job worker should start");
    assert_success(&remaining);
    assert_eq!(parse_json(&remaining)["job_id"], job_ids[2]);
}

#[test]
fn independently_launched_job_worker_watch_window_has_durable_liveness_and_shutdown() {
    let fixture = Fixture::new();
    fixture.setup();
    let submitted = fixture
        .command(&["job", "submit", "--", "guide", "--json"])
        .output()
        .expect("job submission should start");
    assert_success(&submitted);
    let job_id = parse_json(&submitted)["job_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let child = fixture
        .command(&["job", "work", "--watch", "--idle-timeout", "1", "--json"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("watched worker should start as an independent process");
    let fleet = FleetEngine::open(fixture.data.join("fleet.sqlite3")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let process_id = loop {
        if let Some(supervisor) = fleet
            .list_supervisors()
            .unwrap()
            .into_iter()
            .find(|supervisor| {
                supervisor.node_id() == "job-worker" && supervisor.state().as_str() == "running"
            })
        {
            break supervisor
                .process_id()
                .expect("watched worker should bind its PID");
        }
        assert!(
            Instant::now() < deadline,
            "watched worker did not publish a running supervisor"
        );
        thread::sleep(Duration::from_millis(25));
    };
    assert_ne!(process_id, std::process::id());

    let output = child
        .wait_with_output()
        .expect("watched worker should shut down after its idle window");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["watched"], true);
    assert_eq!(response["stop_reason"], "idle_timeout");
    assert_eq!(response["processed_count"], 1);
    assert_eq!(response["jobs"][0]["job_id"], job_id);

    let supervisor = fleet
        .list_supervisors()
        .unwrap()
        .into_iter()
        .find(|supervisor| supervisor.node_id() == "job-worker")
        .expect("worker supervisor should remain inspectable after shutdown");
    assert_eq!(supervisor.state().as_str(), "stopped");
    assert_eq!(supervisor.process_id(), Some(process_id));
}

#[test]
fn crashed_independent_job_worker_reconciles_and_restarts_without_replay() {
    let fixture = Fixture::new();
    fixture.setup();
    let mut child = fixture
        .command(&["job", "work", "--watch", "--idle-timeout", "30", "--json"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("watched worker should start as an independent process");
    let fleet = FleetEngine::open(fixture.data.join("fleet.sqlite3")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let running = loop {
        if let Some(supervisor) = fleet
            .list_supervisors()
            .unwrap()
            .into_iter()
            .find(|supervisor| {
                supervisor.node_id() == "job-worker" && supervisor.state().as_str() == "running"
            })
        {
            break supervisor;
        }
        assert!(
            Instant::now() < deadline,
            "crash fixture worker did not publish a running supervisor"
        );
        thread::sleep(Duration::from_millis(25));
    };
    let process_id = running
        .process_id()
        .expect("crash fixture worker should bind its PID");
    child
        .kill()
        .expect("worker process should be killable for the crash fixture");
    let _ = child.wait().expect("killed worker should terminate");

    let persisted = fleet
        .list_supervisors()
        .unwrap()
        .into_iter()
        .find(|supervisor| supervisor.node_id() == "job-worker")
        .expect("crashed worker record should remain durable");
    assert_eq!(persisted.state().as_str(), "running");
    assert_eq!(persisted.process_id(), Some(process_id));

    let recovery_now = persisted.updated_at() + 3_601;
    let recovering = fleet
        .reconcile_supervisor("job-worker", recovery_now, 30)
        .expect("stale crashed worker should reconcile");
    assert_eq!(recovering.state().as_str(), "recovering");
    assert_eq!(recovering.reason(), Some("heartbeat_expired"));
    assert!(
        fleet
            .list_leases()
            .unwrap()
            .iter()
            .all(|lease| lease.state().as_str() != "active")
    );

    let restarted = fleet
        .start_supervisor_for_process("job-worker", process_id.saturating_add(1), recovery_now + 1)
        .expect("reconciled worker should accept a new process binding");
    assert_eq!(restarted.state().as_str(), "running");
    assert_eq!(restarted.generation(), running.generation() + 1);
    assert_eq!(restarted.process_id(), Some(process_id.saturating_add(1)));
}

#[test]
fn long_lived_job_daemon_finishes_current_queue_and_stops_after_external_drain() {
    let fixture = Fixture::new();
    fixture.setup();
    let submitted = fixture
        .command(&["job", "submit", "--", "guide", "--json"])
        .output()
        .expect("job submission should start");
    assert_success(&submitted);
    let job_id = parse_json(&submitted)["job_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let child = fixture
        .command(&["job", "work", "--daemon", "--json"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("daemon worker should start as an independent process");
    let fleet = FleetEngine::open(fixture.data.join("fleet.sqlite3")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let process_id = loop {
        if let Some(supervisor) = fleet
            .list_supervisors()
            .unwrap()
            .into_iter()
            .find(|supervisor| {
                supervisor.node_id() == "job-worker" && supervisor.state().as_str() == "running"
            })
        {
            break supervisor
                .process_id()
                .expect("daemon worker should bind its PID");
        }
        assert!(
            Instant::now() < deadline,
            "daemon worker did not publish a running supervisor"
        );
        thread::sleep(Duration::from_millis(25));
    };

    let store = JobStore::open(fixture.data.join("jobs.sqlite3")).unwrap();
    let principal = PrincipalId::new("local-user").unwrap();
    let tenant = TenantId::new("local-tenant").unwrap();
    let workspace = WorkspaceId::new("local-workspace").unwrap();
    let job_record_id = JobId::new(job_id.clone()).unwrap();
    let claim_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let job = store
            .inspect(&job_record_id, &principal, &tenant, &workspace)
            .unwrap();
        if matches!(job.status().as_str(), "running" | "completed") {
            break;
        }
        assert!(
            Instant::now() < claim_deadline,
            "daemon worker did not durably claim the queued job"
        );
        thread::sleep(Duration::from_millis(25));
    }

    let drained = fixture
        .command(&["fleet", "supervisor", "drain", "job-worker", "--json"])
        .output()
        .expect("daemon drain request should start");
    assert_success(&drained);
    assert_eq!(parse_json(&drained)["supervisor"]["state"], "draining");

    let output = child
        .wait_with_output()
        .expect("daemon should finish after external drain");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["daemon"], true);
    assert_eq!(response["stop_reason"], "external_drain");
    assert_eq!(response["processed_count"], 1);
    assert_eq!(response["jobs"][0]["job_id"], job_id);

    let supervisor = fleet
        .list_supervisors()
        .unwrap()
        .into_iter()
        .find(|supervisor| supervisor.node_id() == "job-worker")
        .expect("daemon supervisor should remain inspectable after drain");
    assert_eq!(supervisor.state().as_str(), "stopped");
    assert_eq!(supervisor.process_id(), Some(process_id));
}

#[test]
fn long_lived_job_daemon_handles_staggered_enqueue_without_duplicate_completion() {
    let fixture = Fixture::new();
    fixture.setup();
    let child = fixture
        .command(&["job", "work", "--daemon", "--json"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("daemon worker should start as an independent process");
    let fleet = FleetEngine::open(fixture.data.join("fleet.sqlite3")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let process_id = loop {
        if let Some(supervisor) = fleet
            .list_supervisors()
            .unwrap()
            .into_iter()
            .find(|supervisor| {
                supervisor.node_id() == "job-worker" && supervisor.state().as_str() == "running"
            })
        {
            break supervisor
                .process_id()
                .expect("daemon worker should bind its PID");
        }
        assert!(
            Instant::now() < deadline,
            "staggered daemon did not publish a running supervisor"
        );
        thread::sleep(Duration::from_millis(25));
    };
    assert_ne!(process_id, std::process::id());

    let mut job_ids = Vec::new();
    for _ in 0..4 {
        for _ in 0..4 {
            let submitted = fixture
                .command(&["job", "submit", "--", "guide", "--json"])
                .output()
                .expect("job submission should start");
            assert_success(&submitted);
            job_ids.push(
                parse_json(&submitted)["job_id"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            );
        }
        thread::sleep(Duration::from_millis(150));
    }

    let store = JobStore::open(fixture.data.join("jobs.sqlite3")).unwrap();
    let principal = PrincipalId::new("local-user").unwrap();
    let tenant = TenantId::new("local-tenant").unwrap();
    let workspace = WorkspaceId::new("local-workspace").unwrap();
    // Windows process startup and SQLite handoff are substantially slower than
    // the in-process worker path; keep the soak bounded but allow the full
    // staggered batch to drain on a hosted runner.
    let completion_deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let jobs = store.list(&principal, &tenant, &workspace).unwrap();
        if jobs.len() == job_ids.len()
            && jobs.iter().all(|job| job.status().as_str() == "completed")
        {
            break;
        }
        assert!(
            Instant::now() < completion_deadline,
            "staggered daemon did not complete the full queue"
        );
        thread::sleep(Duration::from_millis(50));
    }

    let drained = fixture
        .command(&["fleet", "supervisor", "drain", "job-worker", "--json"])
        .output()
        .expect("daemon drain request should start");
    assert_success(&drained);

    let output = child
        .wait_with_output()
        .expect("staggered daemon should finish after external drain");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["daemon"], true);
    assert_eq!(response["stop_reason"], "external_drain");
    assert_eq!(response["processed_count"], job_ids.len());
    let mut processed_ids = response["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|job| job["job_id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    processed_ids.sort();
    job_ids.sort();
    assert_eq!(processed_ids, job_ids);
    let supervisor = fleet
        .list_supervisors()
        .unwrap()
        .into_iter()
        .find(|supervisor| supervisor.node_id() == "job-worker")
        .expect("staggered daemon supervisor should remain inspectable");
    assert_eq!(supervisor.state().as_str(), "stopped");
    assert_eq!(supervisor.process_id(), Some(process_id));
}

#[test]
fn interactive_setup_configures_a_provider_without_echoing_secrets() {
    let fixture = Fixture::new();
    let mut command = fixture.command(&["setup", "--interactive", "--json"]);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("interactive setup should start");
    let mut input = child
        .stdin
        .take()
        .expect("interactive setup should accept input");
    input
        .write_all(b"http://127.0.0.1:4317/v1\nfixture-model\nPANDORA_FIXTURE_KEY\n")
        .expect("interactive answers should be written");
    drop(input);

    let output = child
        .wait_with_output()
        .expect("interactive setup should finish");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["command"], "setup");
    assert_eq!(response["provider_configured"], true);
    assert_eq!(response["provider_model"], "fixture-model");
    assert_eq!(response["api_key_env"], "PANDORA_FIXTURE_KEY");

    let config = fs::read_to_string(&fixture.config).expect("interactive config should exist");
    assert!(config.contains("PANDORA_FIXTURE_KEY"));
    assert!(!config.contains("secret"));
}

#[test]
fn scripted_setup_persists_the_requested_credential_environment() {
    let fixture = Fixture::new();
    let output = fixture
        .command(&[
            "setup",
            "--provider-url",
            "http://127.0.0.1:4317/v1",
            "--model",
            "scripted-model",
            "--api-key-env",
            "PANDORA_SCRIPTED_KEY",
            "--json",
        ])
        .output()
        .expect("scripted setup should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["active_provider"], "openai-compatible");
    assert_eq!(response["provider_model"], "scripted-model");
    assert_eq!(response["api_key_env"], "PANDORA_SCRIPTED_KEY");

    let config = fs::read_to_string(&fixture.config).expect("scripted config should exist");
    assert!(config.contains("PANDORA_SCRIPTED_KEY"));
}

#[test]
fn scripted_setup_rejects_provider_options_without_a_provider_url() {
    let fixture = Fixture::new();
    let output = fixture
        .command(&["setup", "--api-key-env", "PANDORA_SCRIPTED_KEY", "--json"])
        .output()
        .expect("scripted setup should start");
    assert_eq!(output.status.code(), Some(2));
    let response = parse_json(&output);
    assert_eq!(response["code"], "usage_error");
    assert!(
        response["message"]
            .as_str()
            .unwrap_or_default()
            .contains("--provider-url")
    );
    assert!(!fixture.config.exists());
}

#[test]
fn chat_can_exit_without_provider_configuration() {
    let fixture = Fixture::new();
    let mut command = fixture.command(&["chat"]);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("chat should start");
    child
        .stdin
        .take()
        .expect("chat should accept input")
        .write_all(b"/exit\n")
        .expect("chat command should be written");

    let output = child.wait_with_output().expect("chat should finish");
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Chat closed"));
}

#[test]
fn chat_handles_local_commands_without_provider_configuration() {
    let fixture = Fixture::new();
    let mut command = fixture.command(&["chat"]);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("chat should start");
    child
        .stdin
        .take()
        .expect("chat should accept input")
        .write_all(b"/help\n/session\n/approve\n/deny\n/quit\n")
        .expect("chat commands should be written");

    let output = child.wait_with_output().expect("chat should finish");
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("show chat commands"));
    assert!(stdout.contains("session: not started"));
    assert!(stdout.contains("approval> no pending approval"));
    assert!(stdout.contains("Chat closed after 0 turn(s)"));
}

#[test]
fn chat_rejects_machine_readable_output() {
    let fixture = Fixture::new();
    let output = fixture
        .command(&["chat", "--json"])
        .output()
        .expect("chat should start");
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(parse_json(&output)["code"], "usage_error");
    assert_eq!(
        parse_json(&output)["message"],
        "chat does not support --json"
    );
}

#[test]
fn tui_requires_an_interactive_terminal() {
    let fixture = Fixture::new();
    let output = fixture
        .command(&["tui"])
        .output()
        .expect("tui should start");
    assert_eq!(output.status.code(), Some(10));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("error: tui requires an interactive terminal")
    );
}

#[test]
fn tui_rejects_machine_readable_output() {
    let fixture = Fixture::new();
    let output = fixture
        .command(&["tui", "--json"])
        .output()
        .expect("tui should start");
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(parse_json(&output)["code"], "usage_error");
    assert_eq!(
        parse_json(&output)["message"],
        "tui does not support --json"
    );
}

#[test]
fn search_run_returns_matching_workspace_files() {
    let fixture = Fixture::new();
    fixture.setup();
    fs::create_dir(fixture.workspace.join("src")).unwrap();
    fs::write(fixture.workspace.join("src/lib.rs"), "needle\n").unwrap();

    let output = fixture
        .command(&["run", "search:needle", "--json"])
        .output()
        .expect("search should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["status"], "completed");
    assert_eq!(response["output"], "src/lib.rs");
}

#[test]
fn skill_list_and_inspect_are_metadata_only() {
    let fixture = Fixture::new();
    let skill_root = fixture.data.join("skills").join("alpha");
    fs::create_dir_all(&skill_root).unwrap();
    fs::write(
        skill_root.join("SKILL.md"),
        "---\nid: alpha\nversion: 0.1.0\nname: Alpha Skill\ndescription: Reads project guidance\npublisher: pandora\nresources: workspace.read\n---\n# Alpha\n\nUse the read tool.\n",
    )
    .unwrap();

    let output = fixture
        .command(&["skill", "list", "--json"])
        .output()
        .expect("skill list should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["skills"][0]["id"], "alpha");
    assert_eq!(response["skills"][0]["state"], "disabled");

    let output = fixture
        .command(&["skill", "inspect", "alpha", "--json"])
        .output()
        .expect("skill inspect should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["skill"]["id"], "alpha");
    assert!(
        response["skill"]["body"]
            .as_str()
            .unwrap()
            .contains("read tool")
    );
}

#[test]
fn skill_install_admits_a_local_package_without_enabling_it() {
    let fixture = Fixture::new();
    let source = fixture.root.join("incoming").join("beta");
    fs::create_dir_all(source.join("scripts")).unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nid: beta\nversion: 0.1.0\nname: Beta Skill\ndescription: Reads project guidance\npublisher: pandora\nresources: workspace.read\n---\n# Beta\n",
    )
    .unwrap();
    fs::write(source.join("scripts/check.py"), "print('ok')").unwrap();

    let source = source.to_str().unwrap();
    let output = fixture
        .command(&["skill", "install", source, "--json"])
        .output()
        .expect("skill install should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["skill"]["id"], "beta");
    assert_eq!(response["skill"]["state"], "disabled");
    assert!(fixture.data.join("skills/beta/SKILL.md").is_file());
    assert!(fixture.root.join("incoming/beta/SKILL.md").is_file());
}

#[test]
fn tool_list_and_inspect_expose_governed_contracts_only() {
    let fixture = Fixture::new();

    let output = fixture
        .command(&["tool", "list", "--json"])
        .output()
        .expect("tool list should start");
    assert_success(&output);
    let response = parse_json(&output);
    let tools = response["tools"].as_array().unwrap();
    assert!(tools.iter().any(|tool| tool["id"] == "workspace.read"));

    let output = fixture
        .command(&["tool", "inspect", "workspace.read", "--json"])
        .output()
        .expect("tool inspect should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["tool"]["id"], "workspace.read");
    assert_eq!(response["tool"]["version"], "1.0.0");
    assert_eq!(response["tool"]["capability"], "filesystem.read");
    assert_eq!(response["tool"]["operation"], "read");
    assert_eq!(response["tool"]["input_schema"]["required"][0], "path");
    assert_eq!(
        response["tool"]["input_schema"]["additionalProperties"],
        false
    );

    let output = fixture
        .command(&["tool", "inspect", "missing", "--json"])
        .output()
        .expect("unknown tool inspect should start");
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(parse_json(&output)["code"], "usage_error");
}

#[test]
fn skill_state_transitions_persist_without_running_scripts() {
    let fixture = Fixture::new();
    let skill_root = fixture.data.join("skills").join("alpha");
    fs::create_dir_all(skill_root.join("scripts")).unwrap();
    fs::write(
        skill_root.join("SKILL.md"),
        "---\nid: alpha\nversion: 0.1.0\nname: Alpha Skill\ndescription: Reads project guidance\npublisher: pandora\nresources: workspace.read\n---\n# Alpha\n",
    )
    .unwrap();
    fs::write(skill_root.join("scripts").join("marker.txt"), "untouched").unwrap();

    let output = fixture
        .command(&["skill", "enable", "alpha", "--json"])
        .output()
        .expect("skill enable should start");
    assert_success(&output);
    assert_eq!(parse_json(&output)["skill"]["state"], "enabled");

    let output = fixture
        .command(&["skill", "list", "--json"])
        .output()
        .expect("skill list should start");
    assert_success(&output);
    assert_eq!(parse_json(&output)["skills"][0]["state"], "enabled");

    let output = fixture
        .command(&["skill", "suspend", "alpha", "--json"])
        .output()
        .expect("skill suspend should start");
    assert_success(&output);
    assert_eq!(parse_json(&output)["skill"]["state"], "suspended");

    let output = fixture
        .command(&["skill", "disable", "alpha", "--json"])
        .output()
        .expect("skill disable should start");
    assert_success(&output);
    assert_eq!(parse_json(&output)["skill"]["state"], "disabled");
    assert_eq!(
        fs::read_to_string(skill_root.join("scripts").join("marker.txt")).unwrap(),
        "untouched"
    );

    let output = fixture
        .command(&["skill", "remove", "alpha", "--json"])
        .output()
        .expect("unconfirmed skill remove should start");
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(parse_json(&output)["code"], "usage_error");
    assert!(skill_root.is_dir());

    let output = fixture
        .command(&["skill", "remove", "alpha", "--dry-run", "--json"])
        .output()
        .expect("skill remove dry-run should start");
    assert_success(&output);
    assert_eq!(parse_json(&output)["dry_run"], true);
    assert!(skill_root.is_dir());

    let output = fixture
        .command(&["skill", "remove", "alpha", "--yes", "--json"])
        .output()
        .expect("skill remove should start");
    assert_success(&output);
    assert_eq!(parse_json(&output)["skill"]["state"], "removed");
    assert!(!skill_root.exists());

    let output = fixture
        .command(&["skill", "restore", "alpha", "--json"])
        .output()
        .expect("skill restore should start");
    assert_success(&output);
    assert_eq!(parse_json(&output)["skill"]["state"], "disabled");
    assert!(skill_root.is_dir());
    assert_eq!(
        fs::read_to_string(skill_root.join("scripts").join("marker.txt")).unwrap(),
        "untouched"
    );
}

#[test]
fn orchestration_roles_are_discoverable_without_runtime_setup() {
    let fixture = Fixture::new();
    let output = fixture
        .command(&["orchestration", "roles", "--json"])
        .output()
        .expect("orchestration discovery should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["command"], "orchestration roles");
    assert_eq!(response["roles"][0], "planner");
    assert_eq!(response["roles"][3], "verifier");
}

#[test]
fn orchestration_cli_persists_partial_failure_across_processes() {
    let fixture = Fixture::new();
    fixture.setup();
    let run_id = "run-process-partial";
    let plan = OrchestrationPlan::new(
        PlanId::new("multi-repository-process").unwrap(),
        vec![
            RoleAssignment::new(
                RoleId::new("planner").unwrap(),
                OrchestrationRole::Planner,
                HarnessId::new("coding-domain").unwrap(),
                Vec::new(),
            )
            .unwrap(),
            RoleAssignment::new(
                RoleId::new("maker").unwrap(),
                OrchestrationRole::Maker,
                HarnessId::new("design-domain").unwrap(),
                vec![RoleId::new("planner").unwrap()],
            )
            .unwrap(),
        ],
        2,
        1,
        vec![Handoff::new(
            RoleId::new("planner").unwrap(),
            RoleId::new("maker").unwrap(),
            Some(HarnessId::new("coordination-meta").unwrap()),
        )],
    )
    .unwrap();
    let governed = GovernedOrchestrationPlan::new(
        plan,
        MetaComposition::new(
            vec![
                HarnessId::new("coding-domain").unwrap(),
                HarnessId::new("design-domain").unwrap(),
            ],
            1,
        )
        .unwrap(),
        vec![
            RepositoryBinding::new(
                RepositoryId::new("api").unwrap(),
                WorkspaceId::new("workspace-api").unwrap(),
                "commit-api",
            )
            .unwrap(),
            RepositoryBinding::new(
                RepositoryId::new("desktop").unwrap(),
                WorkspaceId::new("workspace-desktop").unwrap(),
                "commit-desktop",
            )
            .unwrap(),
        ],
        vec![
            RoleRepositoryBinding::new(
                RoleId::new("planner").unwrap(),
                RepositoryId::new("api").unwrap(),
            ),
            RoleRepositoryBinding::new(
                RoleId::new("maker").unwrap(),
                RepositoryId::new("desktop").unwrap(),
            ),
        ],
    )
    .unwrap();
    let plan_path = fixture.root.join("process-plan.json");
    fs::write(&plan_path, serde_json::to_vec(&governed).unwrap()).unwrap();
    let plan_path = plan_path.to_str().unwrap();

    let submitted = fixture
        .command(&[
            "orchestration",
            "submit",
            "--input",
            plan_path,
            "--id",
            run_id,
            "--json",
        ])
        .output()
        .expect("orchestration submit should start");
    assert_success(&submitted);

    let claimed = fixture
        .command(&["orchestration", "claim", "--worker", "worker-a", "--json"])
        .output()
        .expect("orchestration claim should start");
    assert_success(&claimed);
    assert_eq!(parse_json(&claimed)["assignments"][0]["role_id"], "planner");

    let planner_receipt = OrchestrationRoleReceipt::new(
        ReceiptId::new("receipt-planner-process").unwrap(),
        OrchestrationRunId::new(run_id).unwrap(),
        RoleId::new("planner").unwrap(),
        RepositoryId::new("api").unwrap(),
        WorkspaceId::new("workspace-api").unwrap(),
        "commit-api",
        Vec::new(),
        Some(RequestDigest::new("planner-evidence-process").unwrap()),
    )
    .unwrap();
    let receipt_path = fixture.root.join("planner-receipt.json");
    fs::write(&receipt_path, serde_json::to_vec(&planner_receipt).unwrap()).unwrap();
    let receipt_path = receipt_path.to_str().unwrap();
    let completed = fixture
        .command(&[
            "orchestration",
            "complete",
            run_id,
            "--worker",
            "worker-a",
            "--role",
            "planner",
            "--receipt",
            receipt_path,
            "--json",
        ])
        .output()
        .expect("orchestration complete should start");
    assert_success(&completed);
    assert_eq!(parse_json(&completed)["assignments"][0]["role_id"], "maker");

    let interrupted = fixture
        .command(&[
            "orchestration",
            "mark-interrupted",
            run_id,
            "--reason",
            "maker worker exited after planner completed",
            "--yes",
            "--json",
        ])
        .output()
        .expect("orchestration interruption should start");
    assert_success(&interrupted);

    let inspected = fixture
        .command(&["orchestration", "inspect", run_id, "--json"])
        .output()
        .expect("orchestration inspect should start");
    assert_success(&inspected);
    let inspected = parse_json(&inspected);
    assert_eq!(inspected["status"], "interrupted");
    assert_eq!(inspected["completed_roles"].as_array().unwrap().len(), 1);
    assert_eq!(inspected["active_roles"].as_array().unwrap().len(), 1);
    assert_eq!(inspected["role_receipts"].as_array().unwrap().len(), 1);

    let resumed = fixture
        .command(&["orchestration", "resume", run_id, "--json"])
        .output()
        .expect("orchestration resume should start");
    assert_eq!(resumed.status.code(), Some(50));
    assert!(
        parse_json(&resumed)["message"]
            .as_str()
            .unwrap()
            .contains("active roles that require receipt reconciliation")
    );
}

#[test]
fn strategy_profiles_are_discoverable_without_runtime_setup() {
    let fixture = Fixture::new();
    let output = fixture
        .command(&["strategies", "list", "--json"])
        .output()
        .expect("strategy discovery should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["command"], "strategies list");
    assert_eq!(response["default"], "react");
    assert_eq!(response["available"].as_array().unwrap().len(), 4);
    assert_eq!(response["available"][2]["id"], "lats");
    assert_eq!(response["available"][2]["profile"], "research");
    assert_eq!(response["available"][3]["id"], "population");
    assert_eq!(response["available"][3]["profile"], "research");
}

#[test]
fn patch_run_returns_approval_required_without_writing() {
    let fixture = Fixture::new();
    fixture.setup();

    let output = fixture
        .command(&["run", "patch:README.md:changed", "--json"])
        .output()
        .expect("run should start");
    assert_eq!(output.status.code(), Some(40));
    let response = parse_json(&output);
    assert_eq!(response["version"], "0.1");
    assert_eq!(response["code"], "approval_required");
    assert_eq!(response["details"]["status"], "approval_required");
    assert_eq!(response["details"]["feedback_recorded"], false);
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("README.md")).unwrap(),
        "fixture\n"
    );
}

#[test]
fn verify_run_returns_approval_required_before_process_execution() {
    let fixture = Fixture::new();
    fixture.setup();

    let output = fixture
        .command(&["run", "verify", "--json"])
        .output()
        .expect("verify run should start");
    assert_eq!(output.status.code(), Some(40));
    let response = parse_json(&output);
    assert_eq!(response["code"], "approval_required");
    assert_eq!(response["details"]["status"], "approval_required");
}

#[test]
fn session_resume_returns_persisted_events() {
    let fixture = Fixture::new();
    fixture.setup();
    let run = fixture
        .command(&["run", "read:README.md", "--json"])
        .output()
        .expect("run should start");
    assert_success(&run);
    let session_id = parse_json(&run)["session_id"]
        .as_str()
        .expect("run should return a session")
        .to_owned();

    let output = fixture
        .command(&["session", "resume", &session_id, "--json"])
        .output()
        .expect("resume should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["version"], "0.1");
    assert_eq!(response["command"], "session resume");
    assert_eq!(response["session_id"], session_id);
    assert_eq!(response["event_count"], 4);
    assert_eq!(response["agent_message_count"], 0);
    assert_eq!(response["l1_evidence_count"], 1);
    assert_eq!(response["evaluation_count"], 1);
}

#[test]
fn memory_cli_recalls_a_scoped_record_and_requires_confirmation_to_revoke() {
    let fixture = Fixture::new();
    fixture.setup();
    let run = fixture
        .command(&["run", "read:README.md", "--json"])
        .output()
        .expect("run should start");
    assert_success(&run);
    let session_id = parse_json(&run)["session_id"]
        .as_str()
        .expect("run should return a session")
        .to_owned();
    let engine = MemoryEngine::open(
        fixture.data.join("sessions.sqlite3"),
        64,
        PrincipalId::new("local-user").unwrap(),
    )
    .unwrap();
    let scope = MemoryScope::new(
        TenantId::new("local-tenant").unwrap(),
        WorkspaceId::new("local-workspace").unwrap(),
        SessionId::new(&session_id).unwrap(),
        "openai-compatible",
    )
    .unwrap();
    engine
        .distill_l1(
            scope,
            "manual-memory",
            MemoryKind::Lesson,
            "bounded lesson",
            ContextClassification::Internal,
            Timestamp::from_unix_seconds(1),
            "test",
        )
        .unwrap();

    let synthesis_preview = fixture
        .command(&[
            "memory",
            "synthesize",
            "--session",
            &session_id,
            "--provider",
            "openai-compatible",
            "--id",
            "synthesized-memory",
            "--kind",
            "lesson",
            "--summary",
            "A reviewed lesson from this session",
            "--json",
        ])
        .output()
        .expect("memory synthesis preview should start");
    assert_success(&synthesis_preview);
    let synthesis_preview = parse_json(&synthesis_preview);
    assert_eq!(synthesis_preview["command"], "memory synthesize");
    assert_eq!(synthesis_preview["dry_run"], true);
    assert_eq!(synthesis_preview["candidate"]["origin"], "synthesized");
    assert!(
        !synthesis_preview["evidence_ids"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let synthesized = fixture
        .command(&[
            "memory",
            "synthesize",
            "--session",
            &session_id,
            "--provider",
            "openai-compatible",
            "--id",
            "synthesized-memory",
            "--kind",
            "lesson",
            "--summary",
            "A reviewed lesson from this session",
            "--yes",
            "--json",
        ])
        .output()
        .expect("memory synthesis commit should start");
    assert_success(&synthesized);
    let synthesized = parse_json(&synthesized);
    assert_eq!(synthesized["dry_run"], false);
    assert_eq!(synthesized["promotion_required"], true);
    assert_eq!(synthesized["committed"]["tier"], "l1");
    assert_eq!(synthesized["committed"]["origin"], "synthesized");

    let provenance = fixture
        .command(&[
            "memory",
            "provenance",
            "--session",
            &session_id,
            "--provider",
            "openai-compatible",
            "synthesized-memory",
            "--json",
        ])
        .output()
        .expect("memory provenance should start");
    assert_success(&provenance);
    let provenance = parse_json(&provenance);
    assert_eq!(provenance["command"], "memory provenance");
    assert_eq!(provenance["root_id"], "synthesized-memory");
    assert_eq!(provenance["bounded"], true);
    assert!(provenance["nodes"].as_array().unwrap().len() >= 2);
    assert!(!provenance["edges"].as_array().unwrap().is_empty());

    let recalled = fixture
        .command(&[
            "memory",
            "recall",
            "--session",
            &session_id,
            "--provider",
            "openai-compatible",
            "--tier",
            "l1",
            "--json",
        ])
        .output()
        .expect("memory recall should start");
    assert_success(&recalled);
    let recalled = parse_json(&recalled);
    assert_eq!(recalled["command"], "memory recall");
    assert_eq!(recalled["durability"], "session-store");
    let memory_id = recalled["records"][0]["id"]
        .as_str()
        .expect("recall should return a memory ID")
        .to_owned();

    let audit = fixture
        .command(&[
            "memory",
            "audit",
            "--session",
            &session_id,
            "--provider",
            "openai-compatible",
            "--json",
        ])
        .output()
        .expect("memory audit should start");
    assert_success(&audit);
    assert!(parse_json(&audit)["count"].as_u64().unwrap() >= 1);

    let dry_run = fixture
        .command(&[
            "memory",
            "forget",
            "--session",
            &session_id,
            "--provider",
            "openai-compatible",
            &memory_id,
            "--json",
        ])
        .output()
        .expect("memory forget dry run should start");
    assert_success(&dry_run);
    assert_eq!(parse_json(&dry_run)["dry_run"], true);

    let forgotten = fixture
        .command(&[
            "memory",
            "forget",
            "--session",
            &session_id,
            "--provider",
            "openai-compatible",
            &memory_id,
            "--yes",
            "--json",
        ])
        .output()
        .expect("memory forget should start");
    assert_success(&forgotten);
    assert_eq!(parse_json(&forgotten)["revoked"], true);

    let compaction_preview = fixture
        .command(&[
            "memory",
            "compact",
            "--session",
            &session_id,
            "--provider",
            "openai-compatible",
            "--before",
            "4102444800",
            "--json",
        ])
        .output()
        .expect("memory compaction preview should start");
    assert_success(&compaction_preview);
    let compaction_preview = parse_json(&compaction_preview);
    assert_eq!(compaction_preview["command"], "memory compact");
    assert_eq!(compaction_preview["dry_run"], true);
    assert!(compaction_preview["compactable_records"].as_u64().unwrap() >= 1);
    assert_eq!(
        compaction_preview["boundary"]["secure_erasure_guaranteed"],
        false
    );

    let compacted = fixture
        .command(&[
            "memory",
            "compact",
            "--session",
            &session_id,
            "--provider",
            "openai-compatible",
            "--before",
            "4102444800",
            "--yes",
            "--json",
        ])
        .output()
        .expect("memory compaction should start");
    assert_success(&compacted);
    let compacted = parse_json(&compacted);
    assert_eq!(compacted["dry_run"], false);
    assert!(compacted["compacted_records"].as_u64().unwrap() >= 1);
    assert_eq!(compacted["boundary"]["tombstones_retained"], true);
    assert_eq!(compacted["boundary"]["audit_retained"], true);

    let missing = fixture
        .command(&[
            "memory",
            "recall",
            "--session",
            &session_id,
            "--provider",
            "openai-compatible",
            "--tier",
            "l1",
            "--id",
            &memory_id,
            "--json",
        ])
        .output()
        .expect("revoked memory recall should start");
    assert_eq!(missing.status.code(), Some(50));
}

#[test]
fn memory_cli_consolidates_l1_between_scoped_sessions() {
    let fixture = Fixture::new();
    fixture.setup();
    let source_run = fixture
        .command(&["run", "read:README.md", "--json"])
        .output()
        .expect("source run should start");
    assert_success(&source_run);
    let source_session = parse_json(&source_run)["session_id"]
        .as_str()
        .expect("source run should return a session")
        .to_owned();
    let target_run = fixture
        .command(&["run", "read:README.md", "--json"])
        .output()
        .expect("target run should start");
    assert_success(&target_run);
    let target_session = parse_json(&target_run)["session_id"]
        .as_str()
        .expect("target run should return a session")
        .to_owned();

    let engine = MemoryEngine::open(
        fixture.data.join("sessions.sqlite3"),
        64,
        PrincipalId::new("local-user").unwrap(),
    )
    .unwrap();
    let source_scope = MemoryScope::new(
        TenantId::new("local-tenant").unwrap(),
        WorkspaceId::new("local-workspace").unwrap(),
        SessionId::new(&source_session).unwrap(),
        "openai-compatible",
    )
    .unwrap();
    engine
        .distill_l1(
            source_scope,
            "cross-session-lesson",
            MemoryKind::Lesson,
            "retain the verified plan",
            ContextClassification::Internal,
            Timestamp::from_unix_seconds(2),
            "evaluation:source",
        )
        .unwrap();

    let dry_run = fixture
        .command(&[
            "memory",
            "consolidate",
            "--source-session",
            &source_session,
            "--target-session",
            &target_session,
            "--provider",
            "openai-compatible",
            "--source-id",
            "cross-session-lesson",
            "--target-id",
            "target-lesson",
            "--json",
        ])
        .output()
        .expect("memory consolidation dry run should start");
    assert_success(&dry_run);
    let dry_run = parse_json(&dry_run);
    assert_eq!(dry_run["dry_run"], true);
    assert_eq!(dry_run["candidate"]["scope"]["session_id"], target_session);
    assert_eq!(dry_run["candidate"]["origin"], "explicit");

    let committed = fixture
        .command(&[
            "memory",
            "consolidate",
            "--source-session",
            &source_session,
            "--target-session",
            &target_session,
            "--provider",
            "openai-compatible",
            "--source-id",
            "cross-session-lesson",
            "--target-id",
            "target-lesson",
            "--yes",
            "--json",
        ])
        .output()
        .expect("memory consolidation should start");
    assert_success(&committed);
    let committed = parse_json(&committed);
    assert_eq!(committed["dry_run"], false);
    assert_eq!(
        committed["consolidated"]["scope"]["session_id"],
        target_session
    );
    assert!(
        committed["consolidated"]["provenance"]
            .as_str()
            .unwrap()
            .contains("consolidated-from:")
    );

    let recalled = fixture
        .command(&[
            "memory",
            "recall",
            "--session",
            &target_session,
            "--provider",
            "openai-compatible",
            "--tier",
            "l1",
            "--id",
            "target-lesson",
            "--json",
        ])
        .output()
        .expect("consolidated memory recall should start");
    assert_success(&recalled);
    assert_eq!(parse_json(&recalled)["records"][0]["id"], "target-lesson");
}

#[test]
fn memory_cli_cross_project_transfer_requires_policy_and_resolves_conflicts() {
    let fixture = Fixture::new();
    fixture.setup();
    let source_session = Session::new(
        SessionId::new("project-source-session").unwrap(),
        PrincipalId::new("local-user").unwrap(),
        TenantId::new("local-tenant").unwrap(),
        WorkspaceId::new("project-a").unwrap(),
        Timestamp::from_unix_seconds(10),
    );
    let target_session = Session::new(
        SessionId::new("project-target-session").unwrap(),
        PrincipalId::new("local-user").unwrap(),
        TenantId::new("local-tenant").unwrap(),
        WorkspaceId::new("project-b").unwrap(),
        Timestamp::from_unix_seconds(11),
    );
    let sessions = SessionStore::open(fixture.data.join("sessions.sqlite3")).unwrap();
    sessions.create(&source_session).unwrap();
    sessions.create(&target_session).unwrap();
    let memory = MemoryEngine::open(
        fixture.data.join("sessions.sqlite3"),
        64,
        PrincipalId::new("local-user").unwrap(),
    )
    .unwrap();
    memory
        .distill_l1(
            MemoryScope::new(
                TenantId::new("local-tenant").unwrap(),
                WorkspaceId::new("project-a").unwrap(),
                SessionId::new("project-source-session").unwrap(),
                "openai-compatible",
            )
            .unwrap(),
            "project-source-memory",
            MemoryKind::Lesson,
            "retain the verified cross-project plan",
            ContextClassification::Internal,
            Timestamp::from_unix_seconds(12),
            "evaluation:project-a",
        )
        .unwrap();

    let missing_policy = fixture
        .command(&[
            "memory",
            "consolidate",
            "--source-session",
            "project-source-session",
            "--target-session",
            "project-target-session",
            "--source-workspace",
            "project-a",
            "--target-workspace",
            "project-b",
            "--provider",
            "openai-compatible",
            "--source-id",
            "project-source-memory",
            "--target-id",
            "project-target-memory",
            "--json",
        ])
        .output()
        .expect("cross-project policy rejection should start");
    assert_eq!(missing_policy.status.code(), Some(2));

    let dry_run = fixture
        .command(&[
            "memory",
            "consolidate",
            "--source-session",
            "project-source-session",
            "--target-session",
            "project-target-session",
            "--source-workspace",
            "project-a",
            "--target-workspace",
            "project-b",
            "--provider",
            "openai-compatible",
            "--source-id",
            "project-source-memory",
            "--target-id",
            "project-target-memory",
            "--conflict",
            "reject",
            "--json",
        ])
        .output()
        .expect("cross-project dry run should start");
    assert_success(&dry_run);
    let dry_run = parse_json(&dry_run);
    assert_eq!(dry_run["dry_run"], true);
    assert_eq!(dry_run["applied"], false);
    assert_eq!(dry_run["transfer_policy"]["policy_version"], 1);
    assert_eq!(dry_run["transfer_policy"]["boundary"], "cross_project");
    assert_eq!(
        dry_run["transfer_policy"]["source"]["workspace_id"],
        "project-a"
    );
    assert_eq!(
        dry_run["transfer_policy"]["target"]["workspace_id"],
        "project-b"
    );
    assert_eq!(dry_run["conflict"]["rule"], "reject");
    assert_eq!(dry_run["conflict"]["detected"], false);

    let committed = fixture
        .command(&[
            "memory",
            "consolidate",
            "--source-session",
            "project-source-session",
            "--target-session",
            "project-target-session",
            "--source-workspace",
            "project-a",
            "--target-workspace",
            "project-b",
            "--provider",
            "openai-compatible",
            "--source-id",
            "project-source-memory",
            "--target-id",
            "project-target-memory",
            "--conflict",
            "reject",
            "--yes",
            "--json",
        ])
        .output()
        .expect("cross-project transfer should start");
    assert_success(&committed);
    let committed = parse_json(&committed);
    assert_eq!(committed["applied"], true);
    assert_eq!(
        committed["consolidated"]["scope"]["workspace_id"],
        "project-b"
    );
    assert!(
        committed["consolidated"]["provenance"]
            .as_str()
            .unwrap()
            .contains("boundary=cross_project")
    );

    let kept = fixture
        .command(&[
            "memory",
            "consolidate",
            "--source-session",
            "project-source-session",
            "--target-session",
            "project-target-session",
            "--source-workspace",
            "project-a",
            "--target-workspace",
            "project-b",
            "--provider",
            "openai-compatible",
            "--source-id",
            "project-source-memory",
            "--target-id",
            "project-target-memory",
            "--conflict",
            "keep-target",
            "--yes",
            "--json",
        ])
        .output()
        .expect("keep-target transfer should start");
    assert_success(&kept);
    let kept = parse_json(&kept);
    assert_eq!(kept["applied"], false);
    assert_eq!(kept["conflict"]["detected"], true);
    assert_eq!(kept["conflict"]["resolution"], "keep_target");

    let rejected = fixture
        .command(&[
            "memory",
            "consolidate",
            "--source-session",
            "project-source-session",
            "--target-session",
            "project-target-session",
            "--source-workspace",
            "project-a",
            "--target-workspace",
            "project-b",
            "--provider",
            "openai-compatible",
            "--source-id",
            "project-source-memory",
            "--target-id",
            "project-target-memory",
            "--conflict",
            "reject",
            "--yes",
            "--json",
        ])
        .output()
        .expect("reject conflict transfer should start");
    assert_eq!(rejected.status.code(), Some(30));

    memory
        .forget(
            &MemoryScope::new(
                TenantId::new("local-tenant").unwrap(),
                WorkspaceId::new("project-b").unwrap(),
                SessionId::new("project-target-session").unwrap(),
                "openai-compatible",
            )
            .unwrap(),
            &MemoryId::new("project-target-memory").unwrap(),
            Timestamp::from_unix_seconds(20),
        )
        .unwrap();
    let tombstoned = fixture
        .command(&[
            "memory",
            "consolidate",
            "--source-session",
            "project-source-session",
            "--target-session",
            "project-target-session",
            "--source-workspace",
            "project-a",
            "--target-workspace",
            "project-b",
            "--provider",
            "openai-compatible",
            "--source-id",
            "project-source-memory",
            "--target-id",
            "project-target-memory",
            "--conflict",
            "keep-target",
            "--yes",
            "--json",
        ])
        .output()
        .expect("tombstoned identity rejection should start");
    assert_eq!(tombstoned.status.code(), Some(30));
}

#[test]
fn memory_cli_promotes_only_after_an_exact_approval() {
    let fixture = Fixture::new();
    fixture.setup();
    let run = fixture
        .command(&["run", "read:README.md", "--json"])
        .output()
        .expect("run should start");
    assert_success(&run);
    let session_id = parse_json(&run)["session_id"]
        .as_str()
        .expect("run should return a session")
        .to_owned();
    let engine = MemoryEngine::open(
        fixture.data.join("sessions.sqlite3"),
        64,
        PrincipalId::new("local-user").unwrap(),
    )
    .unwrap();
    let scope = MemoryScope::new(
        TenantId::new("local-tenant").unwrap(),
        WorkspaceId::new("local-workspace").unwrap(),
        SessionId::new(&session_id).unwrap(),
        "openai-compatible",
    )
    .unwrap();
    engine
        .distill_l1(
            scope,
            "manual-lesson",
            MemoryKind::Lesson,
            "bounded lesson",
            ContextClassification::Internal,
            Timestamp::from_unix_seconds(1),
            "test",
        )
        .unwrap();

    let pending = fixture
        .command(&[
            "memory",
            "promote",
            "--session",
            &session_id,
            "--provider",
            "openai-compatible",
            "manual-lesson",
            "--json",
        ])
        .output()
        .expect("memory promotion should create approval");
    assert_eq!(pending.status.code(), Some(40));
    let pending_json = parse_json(&pending);
    let approval_id = pending_json["details"]["approval_id"]
        .as_str()
        .expect("promotion should return approval ID")
        .to_owned();

    let resolved = fixture
        .command(&["approval", "resolve", &approval_id, "--allow", "--json"])
        .output()
        .expect("memory approval should resolve");
    assert_success(&resolved);

    let promoted = fixture
        .command(&[
            "memory",
            "promote",
            "--session",
            &session_id,
            "--provider",
            "openai-compatible",
            "manual-lesson",
            "--approval",
            &approval_id,
            "--json",
        ])
        .output()
        .expect("approved memory promotion should start");
    assert_success(&promoted);
    let promoted = parse_json(&promoted);
    assert_eq!(promoted["command"], "memory promote");
    assert_eq!(promoted["approval_consumed"], true);
    assert_eq!(promoted["promoted"]["tier"], "l2");

    let recalled = fixture
        .command(&[
            "memory",
            "recall",
            "--session",
            &session_id,
            "--provider",
            "openai-compatible",
            "--tier",
            "l2",
            "--id",
            "manual-lesson",
            "--json",
        ])
        .output()
        .expect("promoted memory recall should start");
    assert_success(&recalled);
    assert_eq!(
        parse_json(&recalled)["records"][0]["approval"]["approval_id"],
        approval_id
    );
}

#[test]
fn session_inspect_returns_metadata_without_event_payloads() {
    let fixture = Fixture::new();
    fixture.setup();
    let run = fixture
        .command(&["run", "read:README.md", "--json"])
        .output()
        .expect("run should start");
    assert_success(&run);
    let session_id = parse_json(&run)["session_id"]
        .as_str()
        .expect("run should return a session")
        .to_owned();

    let output = fixture
        .command(&["session", "inspect", &session_id, "--json"])
        .output()
        .expect("inspect should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["version"], "0.1");
    assert_eq!(response["command"], "session inspect");
    assert_eq!(response["session_id"], session_id);
    assert_eq!(response["metadata"]["principal_id"], "local-user");
    assert_eq!(response["metadata"]["tenant_id"], "local-tenant");
    assert_eq!(response["metadata"]["workspace_id"], "local-workspace");
    assert_eq!(response["event_count"], 4);
    assert_eq!(response["agent_message_count"], 0);
    assert_eq!(response["last_event_type"], "effect_completed");
    assert!(response["last_event_timestamp"].as_u64().is_some());
    assert_eq!(response["observability"]["trace_count"], 1);
    assert_eq!(response["observability"]["span_count"], 4);
    assert_eq!(response["observability"]["uninstrumented_event_count"], 0);
    assert_eq!(response["observability"]["error_count"], 0);
    assert_eq!(response["observability"]["reliability_bps"], 10_000);
    assert_eq!(response["evaluations"]["count"], 1);
    assert_eq!(
        response["evaluations"]["latest"]["results"][0]["kind"],
        "trajectory"
    );
    assert_eq!(
        response["evaluations"]["latest"]["results"][1]["kind"],
        "policy"
    );
    assert!(response.get("events").is_none());
}

#[test]
fn evaluation_inspect_returns_persisted_receipts_and_supports_execution_filter() {
    let fixture = Fixture::new();
    fixture.setup();
    let run = fixture
        .command(&["run", "read:README.md", "--json"])
        .output()
        .expect("run should start");
    assert_success(&run);
    let run = parse_json(&run);
    let session_id = run["session_id"].as_str().unwrap();
    let execution_id = run["execution_id"].as_str().unwrap();

    let output = fixture
        .command(&[
            "evaluation",
            "inspect",
            "--session",
            session_id,
            "--execution",
            execution_id,
            "--json",
        ])
        .output()
        .expect("evaluation inspect should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["command"], "evaluation inspect");
    assert_eq!(response["session_id"], session_id);
    assert_eq!(response["execution_id"], execution_id);
    assert_eq!(response["count"], 1);
    assert_eq!(response["result_counts"]["passed"], 2);
    assert_eq!(response["result_counts"]["failed"], 0);
    assert_eq!(response["receipts"].as_array().unwrap().len(), 1);
}

#[test]
fn evaluation_suite_registry_drives_a_durable_scheduled_run() {
    let fixture = Fixture::new();
    fixture.setup();
    let input = fixture.root.join("golden-suite.json");
    fs::write(
        &input,
        r#"{"suite_id":"nightly-suite","cases":[{"id":"case-a","target":{"kind":"workflow","id":"workflow-1"},"task":"run the bounded workflow case","execution_id":"exec-a","output":"done","expected_output":"done"}]}"#,
    )
    .expect("suite input should be written");

    let registered = fixture
        .command(&[
            "evaluation",
            "suite",
            "register",
            "--id",
            "nightly-suite",
            "--input",
            input.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("suite registration should start");
    assert_success(&registered);
    let registered = parse_json(&registered);
    assert_eq!(registered["command"], "evaluation suite register");
    assert_eq!(registered["id"], "nightly-suite");
    assert_eq!(registered["case_count"], 1);
    assert_eq!(registered["targeted_case_count"], 1);
    assert_eq!(registered["target_kinds"]["workflow"], 1);

    let scheduled = fixture
        .command(&[
            "evaluation",
            "schedule",
            "create",
            "--id",
            "nightly",
            "--name",
            "Nightly",
            "--suite",
            "nightly-suite",
            "--interval-seconds",
            "60",
            "--json",
        ])
        .output()
        .expect("schedule creation should start");
    assert_success(&scheduled);

    let run = fixture
        .command(&[
            "evaluation",
            "schedule",
            "run",
            "--id",
            "nightly",
            "--worker",
            "local-evaluator",
            "--json",
        ])
        .output()
        .expect("scheduled run should start");
    assert_success(&run);
    let run = parse_json(&run);
    assert_eq!(run["command"], "evaluation schedule run");
    assert_eq!(run["passed"], true);
    assert_eq!(run["run"]["status"], "completed");
    assert_eq!(run["report"]["passed"], 1);

    let second = fixture
        .command(&[
            "evaluation",
            "schedule",
            "run",
            "--id",
            "nightly",
            "--worker",
            "local-evaluator",
            "--json",
        ])
        .output()
        .expect("second scheduled run should start");
    assert_eq!(second.status.code(), Some(50));
}

#[test]
fn task_backed_suite_runs_a_governed_builtin_workflow() {
    let fixture = Fixture::new();
    fixture.setup();
    let direct = fixture
        .command(&["run", "guide", "--gene", "athena.guide", "--json"])
        .output()
        .expect("direct workflow run should start");
    assert_success_with_context(&direct, "direct workflow run");
    let direct = parse_json(&direct);
    let expected = direct["output"]
        .as_str()
        .expect("workflow should return bounded output")
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .trim()
        .to_owned();
    let input = fixture.root.join("task-suite.json");
    let definition = serde_json::json!({
        "suite_id": "task-suite",
        "cases": [{
            "id": "guide-case",
            "target": {"kind": "workflow", "id": "athena.guide"},
            "task": "guide",
            "expected_output": expected
        }]
    });
    fs::write(&input, serde_json::to_vec(&definition).unwrap())
        .expect("task suite input should be written");

    let registered = fixture
        .command(&[
            "evaluation",
            "suite",
            "register",
            "--id",
            "task-suite",
            "--input",
            input.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("task suite registration should start");
    assert_success_with_context(&registered, "task suite registration");
    let run = fixture
        .command(&["evaluation", "suite", "run", "--id", "task-suite", "--json"])
        .output()
        .expect("task suite run should start");
    assert_success_with_context(&run, "task suite run");
    let run = parse_json(&run);
    assert_eq!(run["command"], "evaluation suite run");
    assert_eq!(run["total"], 1);
    assert_eq!(run["passed"], 1);
    assert_eq!(run["cases"][0]["target"]["id"], "athena.guide");

    let scheduled = fixture
        .command(&[
            "evaluation",
            "schedule",
            "create",
            "--id",
            "task-schedule",
            "--name",
            "Task schedule",
            "--suite",
            "task-suite",
            "--interval-seconds",
            "60",
            "--json",
        ])
        .output()
        .expect("task schedule creation should start");
    assert_success_with_context(&scheduled, "task schedule creation");
    let scheduled_run = fixture
        .command(&[
            "evaluation",
            "schedule",
            "run",
            "--id",
            "task-schedule",
            "--worker",
            "task-worker",
            "--json",
        ])
        .output()
        .expect("task scheduled run should start");
    assert_success_with_context(&scheduled_run, "task scheduled run");
    let scheduled_run = parse_json(&scheduled_run);
    assert_eq!(scheduled_run["passed"], true);
    assert_eq!(scheduled_run["report"]["passed"], 1);
    assert_eq!(scheduled_run["run"]["evidence"]["total_cases"], 1);
}

#[test]
fn evaluation_regression_candidates_require_review_before_suite_registration() {
    let fixture = Fixture::new();
    fixture.setup();
    let input = fixture.root.join("failed-regression.json");
    fs::write(
        &input,
        r#"{"suite_id":"reviewed-regression","cases":[{"id":"workflow-smoke","target":{"kind":"workflow","id":"workflow-1"},"task":"run the bounded workflow case","execution_id":"exec-workflow-smoke","output":"wrong","expected_output":"done"}]}"#,
    )
    .expect("regression input should be written");

    let proposed = fixture
        .command(&[
            "evaluation",
            "regression",
            "propose",
            "--id",
            "candidate-1",
            "--input",
            input.to_str().unwrap(),
            "--case",
            "workflow-smoke",
            "--json",
        ])
        .output()
        .expect("candidate proposal should start");
    assert_success(&proposed);
    let proposed = parse_json(&proposed);
    assert_eq!(proposed["status"], "proposed");
    assert_eq!(proposed["review_required_before_suite"], true);

    let blocked = fixture
        .command(&[
            "evaluation",
            "suite",
            "register",
            "--id",
            "reviewed-regression",
            "--input",
            input.to_str().unwrap(),
            "--candidate",
            "candidate-1",
            "--json",
        ])
        .output()
        .expect("blocked registration should start");
    assert!(!blocked.status.success());

    let reviewed = fixture
        .command(&[
            "evaluation",
            "regression",
            "review",
            "--id",
            "candidate-1",
            "--decision",
            "accept",
            "--json",
        ])
        .output()
        .expect("candidate review should start");
    assert_success(&reviewed);
    assert_eq!(parse_json(&reviewed)["status"], "accepted");

    let registered = fixture
        .command(&[
            "evaluation",
            "suite",
            "register",
            "--id",
            "reviewed-regression",
            "--input",
            input.to_str().unwrap(),
            "--candidate",
            "candidate-1",
            "--json",
        ])
        .output()
        .expect("reviewed registration should start");
    assert_success(&registered);
    let registered = parse_json(&registered);
    assert_eq!(registered["review_gate"], "accepted-regression-candidate");
    assert_eq!(registered["candidate_id"], "candidate-1");
}

#[test]
fn evaluation_scorecard_aggregates_persisted_results_without_rerunning() {
    let fixture = Fixture::new();
    fixture.setup();
    let run = fixture
        .command(&["run", "read:README.md", "--json"])
        .output()
        .expect("run should start");
    assert_success(&run);
    let run = parse_json(&run);
    let session_id = run["session_id"].as_str().unwrap();

    let output = fixture
        .command(&["evaluation", "scorecard", "--session", session_id, "--json"])
        .output()
        .expect("evaluation scorecard should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["command"], "evaluation scorecard");
    assert_eq!(response["receipt_count"], 1);
    assert_eq!(response["result_count"], 2);
    assert_eq!(response["result_counts"]["passed"], 2);
    assert_eq!(response["result_counts"]["failed"], 0);
    assert_eq!(response["pass_rate_percent"], 100);
    assert_eq!(response["by_kind"]["policy"]["count"], 1);
    assert_eq!(response["by_kind"]["trajectory"]["count"], 1);
    assert!(response["digest"].as_str().unwrap().starts_with("sha256:"));

    let gated = fixture
        .command(&[
            "evaluation",
            "scorecard",
            "--session",
            session_id,
            "--fail-on-non-passed",
            "--json",
        ])
        .output()
        .expect("evaluation scorecard quality gate should start");
    assert_success(&gated);
    assert_eq!(parse_json(&gated)["pass_rate_percent"], 100);
}

#[test]
fn rollout_inspect_returns_a_durable_summary_for_an_execution() {
    let fixture = Fixture::new();
    fixture.setup();
    let run = fixture
        .command(&["run", "read:README.md", "--json"])
        .output()
        .expect("run should start");
    assert_success(&run);
    let run = parse_json(&run);
    let session_id = run["session_id"].as_str().unwrap();
    let execution_id = run["execution_id"].as_str().unwrap();

    let output = fixture
        .command(&[
            "rollout",
            "inspect",
            "--session",
            session_id,
            "--execution",
            execution_id,
            "--json",
        ])
        .output()
        .expect("rollout inspect should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["command"], "rollout inspect");
    assert_eq!(response["session_id"], session_id);
    assert_eq!(response["execution_id"], execution_id);
    assert_eq!(response["count"], 1);
    assert_eq!(response["durability"], "session-store");
    assert!(
        response["rollouts"][0]["record_count"]
            .as_u64()
            .is_some_and(|count| count > 0)
    );
    assert!(
        response["rollouts"][0]["final_digest"]
            .as_str()
            .is_some_and(|digest| digest.starts_with("sha256:"))
    );
}

#[test]
fn session_inspect_missing_session_preserves_resume_error() {
    let fixture = Fixture::new();
    fixture.setup();

    let output = fixture
        .command(&["session", "inspect", "missing-session", "--json"])
        .output()
        .expect("inspect should start");
    assert_eq!(output.status.code(), Some(60));
    let response = parse_json(&output);
    assert_eq!(response["version"], "0.1");
    assert_eq!(response["code"], "internal_error");
    assert_eq!(response["message"], "session was not found");
}

#[test]
fn mcp_profiles_round_trip_without_starting_the_server() {
    let fixture = Fixture::new();
    let program = PathBuf::from(env!("CARGO_BIN_EXE_pandora"))
        .canonicalize()
        .expect("Pandora binary path should be absolute");
    let program = program
        .to_str()
        .expect("Pandora binary path should be UTF-8");

    for (server_id, cli_mode, stored_mode) in [
        ("auto", "auto", "auto"),
        ("modern", "modern-only", "modern_only"),
        ("legacy", "legacy-only", "legacy_only"),
    ] {
        let first_argument = format!("argument-alpha-{server_id}-7f0ac4");
        let second_argument = format!("argument-omega-{server_id}-c91de2");
        let arguments = serde_json::to_string(&[&first_argument, &second_argument]).unwrap();
        let output = fixture
            .command(&[
                "mcp",
                "set",
                server_id,
                "--program",
                program,
                "--arguments-json",
                &arguments,
                "--mode",
                cli_mode,
                "--json",
            ])
            .output()
            .expect("MCP profile configuration should start");
        assert_success(&output);
        let response = parse_json(&output);
        assert_eq!(response["command"], "mcp set");
        assert_eq!(response["server"]["id"], server_id);
        assert_eq!(response["server"]["program"], program);
        assert_eq!(response["server"]["argument_count"], 2);
        assert_eq!(response["server"]["mode"], stored_mode);
        for argument in [&first_argument, &second_argument] {
            assert!(!String::from_utf8_lossy(&output.stdout).contains(argument));
            assert!(!String::from_utf8_lossy(&output.stderr).contains(argument));
        }

        let stored: Value = serde_json::from_slice(&fs::read(&fixture.config).unwrap()).unwrap();
        assert_eq!(
            stored["mcp_servers"][server_id]["arguments"],
            serde_json::json!([first_argument, second_argument])
        );
        assert_eq!(stored["mcp_servers"][server_id]["program"], program);
        assert_eq!(stored["mcp_servers"][server_id]["mode"], stored_mode);

        let output = fixture
            .command(&[
                "mcp",
                "set",
                server_id,
                "--program",
                program,
                "--arguments-json",
                &arguments,
                "--mode",
                cli_mode,
            ])
            .output()
            .expect("MCP profile human-readable configuration should start");
        assert_success(&output);
        for argument in [&first_argument, &second_argument] {
            assert!(!String::from_utf8_lossy(&output.stdout).contains(argument));
            assert!(!String::from_utf8_lossy(&output.stderr).contains(argument));
        }

        let output = fixture
            .command(&["mcp", "inspect", server_id, "--json"])
            .output()
            .expect("MCP profile inspection should start");
        assert_success(&output);
        assert_eq!(parse_json(&output)["server"]["mode"], stored_mode);
        for argument in [&first_argument, &second_argument] {
            assert!(!String::from_utf8_lossy(&output.stdout).contains(argument));
            assert!(!String::from_utf8_lossy(&output.stderr).contains(argument));
        }

        let output = fixture
            .command(&["mcp", "inspect", server_id])
            .output()
            .expect("MCP profile human-readable inspection should start");
        assert_success(&output);
        for argument in [&first_argument, &second_argument] {
            assert!(!String::from_utf8_lossy(&output.stdout).contains(argument));
            assert!(!String::from_utf8_lossy(&output.stderr).contains(argument));
        }

        let output = fixture
            .command(&["mcp", "list"])
            .output()
            .expect("MCP profile human-readable list should start");
        assert_success(&output);
        for argument in [&first_argument, &second_argument] {
            assert!(!String::from_utf8_lossy(&output.stdout).contains(argument));
            assert!(!String::from_utf8_lossy(&output.stderr).contains(argument));
        }

        let output = fixture
            .command(&["mcp", "remove", server_id, "--yes", "--json"])
            .output()
            .expect("MCP profile removal should start");
        assert_success(&output);
        assert_eq!(parse_json(&output)["server"]["state"], "removed");
    }

    let output = fixture
        .command(&["mcp", "list", "--json"])
        .output()
        .expect("empty MCP profile list should start");
    assert_success(&output);
    assert!(
        parse_json(&output)["servers"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn mcp_profile_commands_never_launch_the_configured_program() {
    let fixture = Fixture::new();
    let program = PathBuf::from(env!("CARGO_BIN_EXE_pandora"))
        .canonicalize()
        .expect("Pandora binary path should be absolute");
    let program = program
        .to_str()
        .expect("Pandora binary path should be UTF-8");
    let marker_config = fixture.root.join("spawned-config.json");
    let marker_data = fixture.root.join("spawned-data");
    let marker_workspace = fixture.root.join("spawned-workspace");
    let arguments = vec![
        "setup".to_owned(),
        "--config".to_owned(),
        marker_config.display().to_string(),
        "--data-dir".to_owned(),
        marker_data.display().to_string(),
        "--workspace".to_owned(),
        marker_workspace.display().to_string(),
        "--json".to_owned(),
    ];
    let arguments_json = serde_json::to_string(&arguments).unwrap();

    let outputs = [
        fixture
            .command(&[
                "mcp",
                "set",
                "no-spawn",
                "--program",
                program,
                "--arguments-json",
                &arguments_json,
                "--json",
            ])
            .output()
            .expect("MCP profile configuration should start"),
        fixture
            .command(&["mcp", "list", "--json"])
            .output()
            .expect("MCP profile list should start"),
        fixture
            .command(&["mcp", "inspect", "no-spawn", "--json"])
            .output()
            .expect("MCP profile inspection should start"),
        fixture
            .command(&["mcp", "remove", "no-spawn", "--yes", "--json"])
            .output()
            .expect("MCP profile removal should start"),
    ];

    for output in outputs {
        assert_success(&output);
        for argument in &arguments {
            assert!(!String::from_utf8_lossy(&output.stdout).contains(argument));
            assert!(!String::from_utf8_lossy(&output.stderr).contains(argument));
        }
        assert!(!marker_config.exists());
        assert!(!marker_data.exists());
        assert!(!marker_workspace.exists());
    }
}

#[test]
fn mcp_execution_requires_explicit_consent_before_starting_a_server() {
    let fixture = Fixture::new();
    let program = PathBuf::from(env!("CARGO_BIN_EXE_pandora"))
        .canonicalize()
        .expect("Pandora binary path should be absolute");
    let program = program
        .to_str()
        .expect("Pandora binary path should be UTF-8");
    let marker = fixture.root.join("should-not-exist.json");
    let marker_config = marker.display().to_string();
    let marker_data = fixture.root.join("nested-data").display().to_string();
    let marker_workspace = fixture.root.join("nested-workspace").display().to_string();
    let arguments = serde_json::to_string(&[
        "setup",
        "--config",
        marker_config.as_str(),
        "--data-dir",
        marker_data.as_str(),
        "--workspace",
        marker_workspace.as_str(),
        "--json",
    ])
    .unwrap();
    let configured = fixture
        .command(&[
            "mcp",
            "set",
            "consent",
            "--program",
            program,
            "--arguments-json",
            &arguments,
            "--json",
        ])
        .output()
        .expect("MCP profile configuration should start");
    assert_success(&configured);

    for command in [
        vec!["mcp", "catalog", "consent", "--json"],
        vec![
            "mcp",
            "call",
            "consent",
            "tool",
            "--arguments-json",
            "{}",
            "--idempotency-key",
            "consent-check",
            "--json",
        ],
    ] {
        let output = fixture
            .command(&command)
            .output()
            .expect("command should start");
        assert_eq!(output.status.code(), Some(2));
        let response = parse_json(&output);
        assert_eq!(response["code"], "usage_error");
        assert!(String::from_utf8_lossy(&output.stdout).contains("--allow"));
    }
    assert!(!marker.exists());
}

#[test]
fn provider_set_and_list_use_the_public_configuration_api() {
    let fixture = Fixture::new();
    let output = fixture
        .command(&[
            "provider",
            "set",
            "--provider-url",
            "http://127.0.0.1:4317/v1",
            "--model",
            "fixture-model",
            "--json",
        ])
        .output()
        .expect("provider set should start");
    assert_success(&output);

    let output = fixture
        .command(&["provider", "list", "--json"])
        .output()
        .expect("provider list should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["version"], "0.1");
    assert_eq!(response["providers"][0]["id"], "openai-compatible");
    assert_eq!(
        response["providers"][0]["base_url"],
        "http://127.0.0.1:4317/v1"
    );
    assert_eq!(response["providers"][0]["default_model"], "fixture-model");
    assert_eq!(response["providers"][0]["protocol"], "open_ai_compatible");
}

#[test]
fn provider_protocol_is_configurable_and_persisted() {
    let fixture = Fixture::new();
    let output = fixture
        .command(&[
            "provider",
            "set",
            "--name",
            "anthropic",
            "--protocol",
            "anthropic_messages",
            "--provider-url",
            "https://api.anthropic.com/v1",
            "--model",
            "claude-sonnet-4-20250514",
            "--api-key-env",
            "PANDORA_ANTHROPIC_API_KEY",
            "--json",
        ])
        .output()
        .expect("provider set should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["protocol"], "anthropic_messages");

    let output = fixture
        .command(&["provider", "list", "--json"])
        .output()
        .expect("provider list should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["providers"][0]["id"], "anthropic");
    assert_eq!(response["providers"][0]["protocol"], "anthropic_messages");
}

#[test]
fn named_provider_profiles_can_be_selected_without_exposing_credentials() {
    let fixture = Fixture::new();
    for (name, url, model, key_env) in [
        (
            "design",
            "https://design.example/v1",
            "vision-model",
            "PANDORA_DESIGN_API_KEY",
        ),
        (
            "coding",
            "https://coding.example/v1",
            "coding-model",
            "PANDORA_CODING_API_KEY",
        ),
    ] {
        let output = fixture
            .command(&[
                "provider",
                "set",
                "--name",
                name,
                "--provider-url",
                url,
                "--model",
                model,
                "--api-key-env",
                key_env,
                "--json",
            ])
            .output()
            .expect("provider profile should start");
        assert_success(&output);
    }

    let output = fixture
        .command(&[
            "provider",
            "set",
            "--name",
            "coding",
            "--provider-url",
            "https://coding.example/v1",
            "--model",
            "coding-model",
            "--api-key-env",
            "PANDORA_CODING_API_KEY",
            "--fallback-provider",
            "design",
            "--input-micros-per-million-tokens",
            "2000000",
            "--output-micros-per-million-tokens",
            "4000000",
            "--json",
        ])
        .output()
        .expect("provider fallback should be configurable");
    assert_success(&output);

    let output = fixture
        .command(&["provider", "use", "design", "--json"])
        .output()
        .expect("provider selection should start");
    assert_success(&output);

    let output = fixture
        .command(&["provider", "list", "--json"])
        .output()
        .expect("provider list should start");
    assert_success(&output);
    let response = parse_json(&output);
    let providers = response["providers"]
        .as_array()
        .expect("provider list should be an array");
    let design = providers
        .iter()
        .find(|provider| provider["id"] == "design")
        .expect("design profile should be listed");
    let coding = providers
        .iter()
        .find(|provider| provider["id"] == "coding")
        .expect("coding profile should be listed");
    assert_eq!(design["default_model"], "vision-model");
    assert_eq!(design["api_key_env"], "PANDORA_DESIGN_API_KEY");
    assert_eq!(design["active"], true);
    assert_eq!(coding["fallback_provider"], "design");
    assert_eq!(
        coding["pricing"]["input_micros_per_million_tokens"],
        2_000_000
    );
    assert_eq!(
        coding["pricing"]["output_micros_per_million_tokens"],
        4_000_000
    );
    assert!(!response.to_string().contains("test-provider-key"));
}

#[test]
fn provider_test_completes_a_configured_request_without_echoing_credentials() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("provider fixture should bind");
    let address = listener
        .local_addr()
        .expect("provider fixture should expose its address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("provider should connect");
        let mut request = Vec::new();
        loop {
            let mut chunk = [0_u8; 1_024];
            let bytes_read = stream.read(&mut chunk).expect("request should be readable");
            request.extend_from_slice(&chunk[..bytes_read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let header_end = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("request should contain headers")
            + 4;
        let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
        let content_length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .expect("provider should send a content length");
        while request.len() < header_end + content_length {
            let mut chunk = [0_u8; 1_024];
            let bytes_read = stream
                .read(&mut chunk)
                .expect("request body should be readable");
            request.extend_from_slice(&chunk[..bytes_read]);
        }
        let body = String::from_utf8_lossy(&request[header_end..header_end + content_length]);
        assert!(headers.contains("authorization: bearer test-provider-key"));
        assert!(body.contains("\"model\":\"fixture-model\""));
        let response =
            br#"{"choices":[{"message":{"content":"ready"}}],"usage":{"prompt_tokens":2,"completion_tokens":1}}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response.len()
        )
        .expect("provider response headers should be written");
        stream
            .write_all(response)
            .expect("provider response should be written");
    });

    let fixture = Fixture::new();
    let provider_url = format!("http://{address}/v1");
    let configured = fixture
        .command(&[
            "provider",
            "set",
            "--provider-url",
            &provider_url,
            "--model",
            "fixture-model",
            "--json",
        ])
        .output()
        .expect("provider set should start");
    assert_success(&configured);

    let output = fixture
        .command(&["provider", "test", "--json"])
        .env("PANDORA_PROVIDER_API_KEY", "test-provider-key")
        .output()
        .expect("provider test should start");
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("test-provider-key"));
    let response = parse_json(&output);
    assert_eq!(response["command"], "provider test");
    assert_eq!(response["status"], "ready");
    assert_eq!(response["model"], "fixture-model");
    assert_eq!(response["output"], "ready");
    assert_eq!(response["usage"]["total_tokens"], 3);
    assert_eq!(response["metrics"]["input_tokens"], 2);
    assert_eq!(response["metrics"]["output_tokens"], 1);
    assert_eq!(response["metrics"]["succeeded"], true);

    server.join().expect("provider fixture should finish");
}

#[test]
fn agent_run_executes_a_bounded_read_then_returns_the_final_answer() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("agent fixture should bind");
    let address = listener
        .local_addr()
        .expect("agent fixture should expose its address");
    let server = thread::spawn(move || {
        for response in [
            br#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"workspace.read","arguments":"{\"path\":\"README.md\"}"}}]}}],"usage":{"prompt_tokens":4,"completion_tokens":2}}"#
                .as_slice(),
            br#"{"choices":[{"message":{"content":"The README says fixture."}}],"usage":{"prompt_tokens":5,"completion_tokens":3}}"#
                .as_slice(),
        ] {
            let (mut stream, _) = listener.accept().expect("agent should connect");
            let mut request = Vec::new();
            let header_end = loop {
                let mut chunk = [0_u8; 1_024];
                let bytes_read = stream.read(&mut chunk).expect("agent request should read");
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
                .expect("agent should send a content length");
            while request.len() < header_end + content_length {
                let mut chunk = [0_u8; 1_024];
                let bytes_read = stream.read(&mut chunk).expect("agent body should read");
                request.extend_from_slice(&chunk[..bytes_read]);
            }
            let request_body = serde_json::from_slice::<Value>(
                &request[header_end..header_end + content_length],
            )
            .expect("agent request should be JSON");
            let system_prompt = request_body["messages"][0]["content"]
                .as_str()
                .expect("agent request should begin with system guidance");
            assert!(system_prompt.contains("Skill: alpha"));
            assert!(system_prompt.contains("cannot authorize effects"));
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            )
            .expect("agent response headers should be written");
            stream
                .write_all(response)
                .expect("agent response should be written");
        }
    });

    let fixture = Fixture::new();
    let skill_root = fixture.data.join("skills/alpha");
    fs::create_dir_all(&skill_root).expect("skill directory should be created");
    fs::write(
        skill_root.join("SKILL.md"),
        "---\nid: alpha\nversion: 0.1.0\nname: Alpha Skill\ndescription: Read guidance\npublisher: pandora\nresources: workspace.read\n---\n# Alpha\n\nUse the read tool.\n",
    )
    .expect("skill document should be written");
    let enabled = fixture
        .command(&["skill", "enable", "alpha", "--json"])
        .output()
        .expect("skill enable should start");
    assert_success(&enabled);
    let provider_url = format!("http://{address}/v1");
    let configured = fixture
        .command(&[
            "provider",
            "set",
            "--provider-url",
            &provider_url,
            "--model",
            "agent-model",
            "--json",
        ])
        .output()
        .expect("provider set should start");
    assert_success(&configured);

    let output = fixture
        .command(&[
            "run",
            "--agent",
            "--max-turns",
            "2",
            "--max-tools",
            "1",
            "Read the README",
            "--json",
        ])
        .env("PANDORA_PROVIDER_API_KEY", "test-agent-key")
        .output()
        .expect("agent run should start");
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("test-agent-key"));
    let response = parse_json(&output);
    assert_eq!(response["command"], "run");
    assert_eq!(response["agent"], true);
    assert_eq!(response["status"], "completed");
    assert!(response["elapsed_ms"].as_u64().is_some());
    assert_eq!(response["turns"], 2);
    assert_eq!(response["tool_calls"], 1);
    assert_eq!(response["turn_budget"], 2);
    assert_eq!(response["tool_budget"], 1);
    assert_eq!(response["provider_metrics"].as_array().unwrap().len(), 2);
    assert_eq!(response["provider_metrics"][0]["input_tokens"], 4);
    assert_eq!(response["provider_metrics"][0]["output_tokens"], 2);
    assert_eq!(response["provider_metrics"][0]["succeeded"], true);
    assert_eq!(response["provider_metrics"][1]["input_tokens"], 5);
    assert_eq!(response["provider_metrics"][1]["output_tokens"], 3);
    assert_eq!(response["provider_metrics"][1]["succeeded"], true);
    let efficiency = EfficiencyStore::open(fixture.data.join("efficiency.sqlite3"))
        .expect("agent efficiency store should open");
    let samples = efficiency
        .load_task_class("general")
        .expect("agent efficiency samples should load");
    assert_eq!(samples.len(), 2);
    let mut targets = samples
        .iter()
        .map(|sample| sample.target())
        .collect::<Vec<_>>();
    let mut expected_targets = response["provider_metrics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|metric| {
            format!(
                "{}/{}",
                metric["provider_id"].as_str().unwrap(),
                metric["model_id"].as_str().unwrap()
            )
        })
        .collect::<Vec<_>>();
    targets.sort_unstable();
    expected_targets.sort_unstable();
    assert_eq!(targets, expected_targets);
    assert_eq!(
        response["context"]["included"],
        serde_json::json!([
            "agent.constitution",
            "agent.skill-boundary",
            "agent.enabled-skills",
        ])
    );
    assert_eq!(response["context"]["dropped"], serde_json::json!([]));
    assert_eq!(response["context"]["cacheable"], false);
    assert_eq!(response["context"]["cache_disposition"], "bypass");
    assert_eq!(response["context"]["projection_version"], 2);
    assert_eq!(response["context"]["provenance_complete"], true);
    assert!(
        response["context"]["manifest_digest"]
            .as_str()
            .is_some_and(|digest| digest.starts_with("sha256:"))
    );
    assert!(response["context"]["token_cost"].as_u64().unwrap() > 0);
    assert_eq!(response["output"], "The README says fixture.");

    server.join().expect("agent fixture should finish");
}

#[test]
fn agent_session_reuses_persisted_conversation() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("agent fixture should bind");
    let address = listener
        .local_addr()
        .expect("agent fixture should expose its address");
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        for response in [
            br#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"workspace.read","arguments":"{\"path\":\"README.md\"}"}}]}}],"usage":{"prompt_tokens":4,"completion_tokens":2}}"#.as_slice(),
            br#"{"choices":[{"message":{"content":"first answer"}}]}"#.as_slice(),
            br#"{"choices":[{"message":{"content":"continued answer"}}]}"#.as_slice(),
        ] {
            let (mut stream, _) = listener.accept().expect("agent should connect");
            let mut request = Vec::new();
            let header_end = loop {
                let mut chunk = [0_u8; 1_024];
                let bytes_read = stream.read(&mut chunk).expect("agent request should read");
                request.extend_from_slice(&chunk[..bytes_read]);
                if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    break position + 4;
                }
            };
            let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
            let content_length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .expect("agent should send a content length");
            while request.len() < header_end + content_length {
                let mut chunk = [0_u8; 1_024];
                let bytes_read = stream.read(&mut chunk).expect("agent body should read");
                request.extend_from_slice(&chunk[..bytes_read]);
            }
            requests.push(
                serde_json::from_slice::<Value>(&request[header_end..header_end + content_length])
                    .expect("agent request should be JSON"),
            );
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            )
            .expect("agent response headers should be written");
            stream
                .write_all(response)
                .expect("agent response should be written");
        }
        requests
    });

    let fixture = Fixture::new();
    let provider_url = format!("http://{address}/v1");
    let configured = fixture
        .command(&[
            "provider",
            "set",
            "--provider-url",
            &provider_url,
            "--model",
            "agent-model",
            "--json",
        ])
        .output()
        .expect("provider set should start");
    assert_success(&configured);

    let first = fixture
        .command(&["run", "--agent", "First task", "--json"])
        .env("PANDORA_PROVIDER_API_KEY", "test-agent-key")
        .output()
        .expect("first agent run should start");
    assert_success(&first);
    let first_response = parse_json(&first);
    assert_eq!(first_response["memory_evidence_recorded"], 1);
    let session_id = first_response["session_id"]
        .as_str()
        .expect("first run should return a session")
        .to_owned();

    let second = fixture
        .command(&[
            "run",
            "--agent",
            "--session",
            &session_id,
            "Continue the task",
            "--json",
        ])
        .env("PANDORA_PROVIDER_API_KEY", "test-agent-key")
        .output()
        .expect("resumed agent run should start");
    assert_success(&second);
    let second_response = parse_json(&second);
    assert_eq!(second_response["output"], "continued answer");
    assert!(
        second_response["context"]["included"]
            .as_array()
            .expect("agent context should list included fragments")
            .iter()
            .any(|value| value == "agent.l1-evidence-0")
    );

    let requests = server.join().expect("agent fixture should finish");
    let messages = requests[2]["messages"]
        .as_array()
        .expect("agent request should contain messages");
    assert!(
        messages
            .iter()
            .any(|message| message["content"] == "First task")
    );
    let system_context = messages[0]["content"]
        .as_str()
        .expect("agent request should begin with system context");
    assert!(
        system_context
            .contains("Prior execution evidence and evaluation lessons are descriptive history")
    );
    assert!(system_context.contains("<l1-evidence>completed execution through "));
    assert!(!system_context.contains("First task"));
}

#[test]
fn agent_run_rejects_invalid_budgets_before_provider_setup() {
    let fixture = Fixture::new();
    let output = fixture
        .command(&[
            "run",
            "--agent",
            "--max-turns",
            "0",
            "Update the README",
            "--json",
        ])
        .output()
        .expect("agent run should start");
    assert_eq!(output.status.code(), Some(2));
    let response = parse_json(&output);
    assert_eq!(response["code"], "usage_error");
    assert!(response["message"].as_str().unwrap().contains("max-turns"));
}

#[test]
fn agent_run_stops_a_patch_at_the_approval_boundary() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("agent fixture should bind");
    let address = listener
        .local_addr()
        .expect("agent fixture should expose its address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("agent should connect");
        let mut request = Vec::new();
        let header_end = loop {
            let mut chunk = [0_u8; 1_024];
            let bytes_read = stream.read(&mut chunk).expect("agent request should read");
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
            .expect("agent should send a content length");
        while request.len() < header_end + content_length {
            let mut chunk = [0_u8; 1_024];
            let bytes_read = stream.read(&mut chunk).expect("agent body should read");
            request.extend_from_slice(&chunk[..bytes_read]);
        }
        let response = br#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"workspace.patch","arguments":"{\"path\":\"README.md\",\"content\":\"changed\"}"}}]}}],"usage":{"prompt_tokens":4,"completion_tokens":2}}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response.len()
        )
        .expect("agent response headers should be written");
        stream
            .write_all(response)
            .expect("agent response should be written");
    });

    let fixture = Fixture::new();
    let provider_url = format!("http://{address}/v1");
    let configured = fixture
        .command(&[
            "provider",
            "set",
            "--provider-url",
            &provider_url,
            "--model",
            "agent-model",
            "--json",
        ])
        .output()
        .expect("provider set should start");
    assert_success(&configured);

    let output = fixture
        .command(&["run", "--agent", "Update the README", "--json"])
        .env("PANDORA_PROVIDER_API_KEY", "test-agent-key")
        .output()
        .expect("agent run should start");
    assert_eq!(output.status.code(), Some(40));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("test-agent-key"));
    let response = parse_json(&output);
    assert_eq!(response["code"], "approval_required");
    assert_eq!(response["details"]["agent"], true);
    assert!(
        !response["details"]["approval_id"]
            .as_str()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("README.md")).unwrap(),
        "fixture\n"
    );

    server.join().expect("agent fixture should finish");
}

#[test]
fn approved_agent_run_resumes_the_pending_tool_call_once() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("agent fixture should bind");
    let address = listener
        .local_addr()
        .expect("agent fixture should expose its address");
    let server = thread::spawn(move || {
        let first_response = br#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"workspace.patch","arguments":"{\"path\":\"README.md\",\"content\":\"approved\"}"}}]}}],"usage":{"prompt_tokens":4,"completion_tokens":2}}"#;
        let second_response = br#"{"choices":[{"message":{"content":"approved"}}],"usage":{"prompt_tokens":5,"completion_tokens":2}}"#;
        for response in [first_response.as_slice(), second_response.as_slice()] {
            let (mut stream, _) = listener.accept().expect("agent should connect");
            let mut request = Vec::new();
            let header_end = loop {
                let mut chunk = [0_u8; 1_024];
                let bytes_read = stream.read(&mut chunk).expect("agent request should read");
                request.extend_from_slice(&chunk[..bytes_read]);
                if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    break position + 4;
                }
            };
            let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
            let content_length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .expect("agent should send a content length");
            while request.len() < header_end + content_length {
                let mut chunk = [0_u8; 1_024];
                let bytes_read = stream.read(&mut chunk).expect("agent body should read");
                request.extend_from_slice(&chunk[..bytes_read]);
            }
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            )
            .expect("agent response headers should be written");
            stream
                .write_all(response)
                .expect("agent response should be written");
        }
    });

    let fixture = Fixture::new();
    let provider_url = format!("http://{address}/v1");
    let configured = fixture
        .command(&[
            "provider",
            "set",
            "--provider-url",
            &provider_url,
            "--model",
            "agent-model",
            "--json",
        ])
        .output()
        .expect("provider set should start");
    assert_success_with_context(&configured, "provider set");

    let first = fixture
        .command(&["run", "--agent", "Update the README", "--json"])
        .env("PANDORA_PROVIDER_API_KEY", "test-agent-key")
        .output()
        .expect("agent run should start");
    assert_eq!(first.status.code(), Some(40));
    let first_response = parse_json(&first);
    let approval_id = first_response["details"]["approval_id"]
        .as_str()
        .expect("approval ID should be returned")
        .to_owned();
    let session_id = first_response["details"]["session_id"]
        .as_str()
        .expect("agent approval should include its session")
        .to_owned();

    let resolved = fixture
        .command(&["approval", "resolve", &approval_id, "--allow", "--json"])
        .output()
        .expect("approval resolution should start");
    assert_success_with_context(&resolved, "approval resolve");

    let resumed = fixture
        .command(&[
            "run",
            "--agent",
            "--approval",
            &approval_id,
            "--session",
            &session_id,
            "Continue after approval",
            "--json",
        ])
        .env("PANDORA_PROVIDER_API_KEY", "test-agent-key")
        .output()
        .expect("approved agent run should start");
    assert_success_with_context(&resumed, "approved agent resume");
    assert_eq!(parse_json(&resumed)["output"], "approved");
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("README.md")).unwrap(),
        "approved"
    );

    let inspected = fixture
        .command(&["approval", "inspect", &approval_id, "--json"])
        .output()
        .expect("approval inspection should start");
    assert_success(&inspected);
    assert_eq!(parse_json(&inspected)["approval"]["status"], "consumed");
    server.join().expect("agent fixture should finish");
}

#[test]
fn chat_approves_and_resumes_the_pending_agent_task() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("agent fixture should bind");
    let address = listener
        .local_addr()
        .expect("agent fixture should expose its address");
    let server = thread::spawn(move || {
        let first_response = br#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"workspace.patch","arguments":"{\"path\":\"README.md\",\"content\":\"approved\"}"}}]}}],"usage":{"prompt_tokens":4,"completion_tokens":2}}"#;
        let second_response =
            br#"{"choices":[{"message":{"content":"approved"}}],"usage":{"prompt_tokens":5,"completion_tokens":2}}"#;
        for response in [first_response.as_slice(), second_response.as_slice()] {
            let (mut stream, _) = listener.accept().expect("agent should connect");
            let mut request = Vec::new();
            let header_end = loop {
                let mut chunk = [0_u8; 1_024];
                let bytes_read = stream.read(&mut chunk).expect("agent request should read");
                request.extend_from_slice(&chunk[..bytes_read]);
                if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    break position + 4;
                }
            };
            let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
            let content_length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .expect("agent should send a content length");
            while request.len() < header_end + content_length {
                let mut chunk = [0_u8; 1_024];
                let bytes_read = stream.read(&mut chunk).expect("agent body should read");
                request.extend_from_slice(&chunk[..bytes_read]);
            }
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            )
            .expect("agent response headers should be written");
            stream
                .write_all(response)
                .expect("agent response should be written");
        }
    });

    let fixture = Fixture::new();
    let provider_url = format!("http://{address}/v1");
    let configured = fixture
        .command(&[
            "provider",
            "set",
            "--provider-url",
            &provider_url,
            "--model",
            "agent-model",
            "--json",
        ])
        .output()
        .expect("provider set should start");
    assert_success(&configured);

    let mut command = fixture.command(&["chat"]);
    command
        .env("PANDORA_PROVIDER_API_KEY", "test-agent-key")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("chat should start");
    child
        .stdin
        .take()
        .expect("chat should accept input")
        .write_all(b"Update the README\n/approve\n/exit\n")
        .expect("chat commands should be written");

    let output = child.wait_with_output().expect("chat should finish");
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("approval:"));
    assert!(stdout.contains("approved"));
    assert!(!stdout.contains("test-agent-key"));
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("README.md")).unwrap(),
        "approved"
    );

    server.join().expect("agent fixture should finish");
}

#[test]
fn run_plan_uses_structured_provider_output_before_governed_execution() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("planner fixture should bind");
    let address = listener
        .local_addr()
        .expect("planner fixture should expose its address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("planner should connect");
        let mut request = Vec::new();
        let header_end = loop {
            let mut chunk = [0_u8; 1_024];
            let bytes_read = stream
                .read(&mut chunk)
                .expect("planner request should read");
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
            .expect("planner should send a content length");
        while request.len() < header_end + content_length {
            let mut chunk = [0_u8; 1_024];
            let bytes_read = stream
                .read(&mut chunk)
                .expect("planner request body should read");
            request.extend_from_slice(&chunk[..bytes_read]);
        }
        let body = String::from_utf8_lossy(&request[header_end..header_end + content_length]);
        assert!(body.contains("\"model\":\"planner-model\""));
        assert!(body.contains("inspect the README"));
        assert!(!body.contains("\"tools\":[{"));
        let response =
            br#"{"choices":[{"message":{"content":"{\"task\":\"read:README.md\"}"}}],"usage":{"prompt_tokens":3,"completion_tokens":2}}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response.len()
        )
        .expect("planner response headers should be written");
        stream
            .write_all(response)
            .expect("planner response should be written");
    });

    let fixture = Fixture::new();
    let provider_url = format!("http://{address}/v1");
    let configured = fixture
        .command(&[
            "provider",
            "set",
            "--provider-url",
            &provider_url,
            "--model",
            "planner-model",
            "--json",
        ])
        .output()
        .expect("provider set should start");
    assert_success(&configured);

    let output = fixture
        .command(&["run", "--plan", "inspect the README", "--json"])
        .env("PANDORA_PROVIDER_API_KEY", "test-planner-key")
        .output()
        .expect("planned run should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["status"], "completed");
    assert_eq!(response["output"], "fixture\n");
    assert_eq!(response["planning"]["enabled"], true);
    assert_eq!(response["planning"]["model"], "planner-model");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("test-planner-key"));

    server.join().expect("planner fixture should finish");
}

#[test]
fn run_plan_rejects_malformed_provider_output_before_execution() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("planner fixture should bind");
    let address = listener
        .local_addr()
        .expect("planner fixture should expose its address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("planner should connect");
        let mut request = [0_u8; 4_096];
        let _ = stream
            .read(&mut request)
            .expect("planner request should read");
        let response =
            br#"{"choices":[{"message":{"content":"not-json"}}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response.len()
        )
        .expect("planner response headers should be written");
        stream
            .write_all(response)
            .expect("planner response should be written");
    });

    let fixture = Fixture::new();
    let provider_url = format!("http://{address}/v1");
    let configured = fixture
        .command(&[
            "provider",
            "set",
            "--provider-url",
            &provider_url,
            "--model",
            "planner-model",
            "--json",
        ])
        .output()
        .expect("provider set should start");
    assert_success(&configured);

    let output = fixture
        .command(&["run", "--plan", "inspect the README", "--json"])
        .env("PANDORA_PROVIDER_API_KEY", "test-planner-key")
        .output()
        .expect("planned run should start");
    assert_eq!(output.status.code(), Some(20));
    let response = parse_json(&output);
    assert_eq!(response["code"], "provider_error");
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("README.md")).unwrap(),
        "fixture\n"
    );

    server.join().expect("planner fixture should finish");
}

#[test]
fn doctor_reports_missing_configuration_with_stable_error() {
    let fixture = Fixture::new();
    let output = fixture
        .command(&["doctor", "--json"])
        .output()
        .expect("doctor should start");
    assert_eq!(output.status.code(), Some(10));
    let response = parse_json(&output);
    assert_eq!(response["version"], "0.1");
    assert_eq!(response["code"], "configuration_error");
    assert!(response["message"].as_str().is_some());
    assert!(response.get("details").is_some());
    assert_eq!(response["details"]["containment"]["version"], 1);
    assert_eq!(
        response["details"]["containment"]["executors"]
            .as_array()
            .expect("doctor should report containment even when unhealthy")
            .len(),
        7
    );
}

#[test]
fn doctor_accepts_a_valid_local_only_setup_without_a_provider() {
    let fixture = Fixture::new();
    let mut command = fixture.command(&["setup", "--interactive", "--json"]);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("interactive setup should start");
    child
        .stdin
        .take()
        .expect("interactive setup should accept input")
        .write_all(b"\n")
        .expect("local-only setup answer should be written");
    let setup = child
        .wait_with_output()
        .expect("interactive setup should finish");
    assert_success(&setup);

    let output = fixture
        .command(&["doctor", "--json"])
        .output()
        .expect("doctor should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["healthy"], true);
    assert_eq!(response["provider"]["configured"], false);
    assert_eq!(response["provider"]["connectivity"], "not_configured");
    let containment = &response["containment"];
    assert_eq!(containment["version"], 1);
    assert!(
        containment["digest"]
            .as_str()
            .is_some_and(|digest| digest.starts_with("sha256:"))
    );
    let executors = containment["executors"]
        .as_array()
        .expect("doctor should report executor containment");
    assert_eq!(
        executors
            .iter()
            .map(|executor| executor["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "filesystem",
            "git_worktree",
            "mcp_stdio",
            "network",
            "process",
            "provider",
            "wasm"
        ]
    );
    let mcp = executors
        .iter()
        .find(|executor| executor["id"] == "mcp_stdio")
        .expect("doctor should report MCP containment");
    assert_eq!(mcp["worker_class"], "child_process");
    assert!(
        mcp["boundaries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|boundary| {
                boundary["kind"] == "network"
                    && boundary["level"] == "unavailable"
                    && boundary["limitation"] == "network_not_restricted"
            })
    );
    let storage_write = response["checks"]
        .as_array()
        .expect("doctor checks should be an array")
        .iter()
        .find(|check| check["check"] == "storage_write")
        .expect("doctor should check storage writeability");
    assert_eq!(storage_write["status"], "ok");
}

#[test]
fn doctor_rejects_a_configured_provider_without_a_credential() {
    let fixture = Fixture::new();
    fixture.setup();
    let output = fixture
        .command(&["doctor", "--json"])
        .env_remove("PANDORA_PROVIDER_API_KEY")
        .output()
        .expect("doctor should start");

    assert_eq!(output.status.code(), Some(10));
    let response = parse_json(&output);
    assert_eq!(response["code"], "configuration_error");
    assert_eq!(response["details"]["healthy"], false);
    assert_eq!(response["details"]["provider"]["credential"], "missing");
    assert!(
        response["details"]["checks"]
            .as_array()
            .expect("diagnostic checks should be an array")
            .iter()
            .any(|check| check["check"] == "credential" && check["status"] == "failed")
    );
}

#[test]
fn harness_discovery_exposes_the_built_in_domains_without_runtime_internals() {
    let fixture = Fixture::new();
    fixture.setup();

    let output = fixture
        .command(&["harness", "list", "--json"])
        .output()
        .expect("harness list should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["version"], "0.1");
    let harnesses = response["harnesses"].as_array().unwrap();
    assert!(harnesses.iter().any(|harness| {
        harness["id"] == "core-source"
            && harness["kind"] == "source"
            && harness["constitutional_service"] == "pandora-runtime"
            && harness["constitutional_service_version"] == env!("CARGO_PKG_VERSION")
            && harness["execution"]["runnable"] == false
            && harness["execution"]["mode"] == "system_augmentation"
    }));
    assert!(
        harnesses
            .iter()
            .any(|harness| harness["id"] == "coding-domain")
    );
    assert!(
        harnesses
            .iter()
            .any(|harness| harness["id"] == "research-domain")
    );
    assert!(
        harnesses
            .iter()
            .any(|harness| harness["id"] == "design-domain")
    );
    assert!(
        harnesses
            .iter()
            .any(|harness| harness["id"] == "operations-domain")
    );
    assert!(
        harnesses
            .iter()
            .any(|harness| harness["id"] == "security-domain")
    );
    let meta = harnesses
        .iter()
        .find(|harness| harness["id"] == "coordination-meta")
        .expect("coordination Meta Harness should be discoverable");
    assert_eq!(meta["kind"], "meta");
    assert_eq!(meta["execution"]["runnable"], false);
    assert_eq!(meta["execution"]["mode"], "composition_only");
    assert_eq!(meta["meta_composition"]["max_handoffs"], 8);
    let allowed_domains = meta["meta_composition"]["allowed_domains"]
        .as_array()
        .unwrap();
    assert!(
        allowed_domains
            .iter()
            .any(|domain| domain == "coding-domain")
    );
    assert!(
        allowed_domains
            .iter()
            .any(|domain| domain == "research-domain")
    );
    assert!(
        allowed_domains
            .iter()
            .any(|domain| domain == "design-domain")
    );
    assert!(
        allowed_domains
            .iter()
            .any(|domain| domain == "operations-domain")
    );

    let output = fixture
        .command(&["harness", "inspect", "core-source", "--json"])
        .output()
        .expect("source harness inspect should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["harness"]["kind"], "source");
    assert_eq!(response["harness"]["execution"]["runnable"], false);
    assert_eq!(
        response["harness"]["constitutional_service"],
        "pandora-runtime"
    );
    assert_eq!(
        response["harness"]["constitutional_service_version"],
        env!("CARGO_PKG_VERSION")
    );
    assert!(response["harness"]["genes"].as_array().unwrap().is_empty());

    let output = fixture
        .command(&["harness", "inspect", "coordination-meta", "--json"])
        .output()
        .expect("Meta harness inspect should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["harness"]["kind"], "meta");
    assert_eq!(response["harness"]["execution"]["runnable"], false);
    assert_eq!(response["harness"]["meta_composition"]["max_handoffs"], 8);
    assert!(response["harness"]["genes"].as_array().unwrap().is_empty());

    let output = fixture
        .command(&["harness", "inspect", "coding", "--json"])
        .output()
        .expect("harness inspect should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["harness"]["kind"], "domain");
    assert_eq!(response["harness"]["execution"]["runnable"], true);

    let output = fixture
        .command(&["harness", "inspect", "design", "--json"])
        .output()
        .expect("design harness inspect should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["harness"]["id"], "design-domain");
    assert_eq!(response["harness"]["kind"], "domain");
    assert_eq!(response["harness"]["execution"]["runnable"], true);
    assert_eq!(response["harness"]["execution"]["mode"], "domain_execution");
    assert!(response["harness"]["genes"].as_array().unwrap().len() >= 5);

    let output = fixture
        .command(&["harness", "inspect", "operations", "--json"])
        .output()
        .expect("operations harness inspect should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["harness"]["id"], "operations-domain");
    assert_eq!(response["harness"]["kind"], "domain");
    assert_eq!(response["harness"]["execution"]["runnable"], true);

    let output = fixture
        .command(&["harness", "inspect", "coding-domain", "--json"])
        .output()
        .expect("canonical harness inspect should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["harness"]["id"], "coding-domain");
    let gene_ids = response["harness"]["genes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|gene| gene["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    for expected in [
        "daedalus.audit",
        "argus.review",
        "ariadne.debt",
        "hephaestus.measure",
        "tests.run",
        "format.check",
        "lint.check",
        "build.check",
        "workspace.status",
        "workspace.diff",
        "workspace.log",
        "workspace.refs",
        "athena.guide",
    ] {
        assert!(gene_ids.contains(&expected));
    }

    let output = fixture
        .command(&["harness", "inspect", "research", "--json"])
        .output()
        .expect("research harness inspect should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["harness"]["id"], "research-domain");
    assert_eq!(response["harness"]["kind"], "domain");
    assert_eq!(response["harness"]["execution"]["runnable"], true);
    assert!(
        response["harness"]["genes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gene| gene["id"] == "browser.fetch")
    );

    let output = fixture
        .command(&["harness", "run", "core-source", "--json"])
        .output()
        .expect("source harness run should start");
    assert_eq!(output.status.code(), Some(50));
    let response = parse_json(&output);
    assert_eq!(response["code"], "execution_failed");
    assert_eq!(response["message"], "harness 'core-source' is not runnable");
    assert_eq!(response["details"]["kind"], "source");

    let output = fixture
        .command(&[
            "run",
            "--harness",
            "core-source",
            "read:README.md",
            "--json",
        ])
        .output()
        .expect("direct source harness run should start");
    assert_eq!(output.status.code(), Some(50));
    let response = parse_json(&output);
    assert_eq!(response["code"], "execution_failed");
    assert_eq!(response["message"], "requested harness is not runnable");
    assert_eq!(response["details"]["harness_id"], "core-source");
    assert_eq!(response["details"]["kind"], "source");

    let output = fixture
        .command(&[
            "harness",
            "run",
            "coding",
            "--gene",
            "workspace.read",
            "--task",
            "read:README.md",
            "--json",
        ])
        .output()
        .expect("harness run should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["command"], "harness run");
    assert_eq!(response["status"], "completed");

    let output = fixture
        .command(&[
            "run",
            "--harness",
            "coding",
            "--gene",
            "workspace.read",
            "read:README.md",
            "--json",
        ])
        .output()
        .expect("explicit harness run should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["harness_id"], "coding-domain");
}

#[test]
fn coding_harness_reports_short_git_status_through_the_governed_runtime() {
    let fixture = Fixture::new();
    fixture.setup();
    fixture.initialize_git_workspace();

    let output = fixture
        .command(&[
            "run",
            "--harness",
            "coding",
            "--gene",
            "workspace.status",
            "status",
            "--json",
        ])
        .output()
        .expect("workspace status should start");
    assert_eq!(output.status.code(), Some(40));
    let response = parse_json(&output);
    assert_eq!(response["code"], "approval_required");
    let approval_id = response["details"]["approval_id"]
        .as_str()
        .expect("workspace status should return an approval")
        .to_owned();

    let output = fixture
        .command(&["approval", "resolve", &approval_id, "--allow", "--json"])
        .output()
        .expect("workspace status approval should resolve");
    assert_success_with_context(&output, "workspace status approval");

    let output = fixture
        .command(&[
            "run",
            "--approval",
            &approval_id,
            "--harness",
            "coding",
            "--gene",
            "workspace.status",
            "status",
            "--json",
        ])
        .output()
        .expect("approved workspace status should start");
    assert_success_with_context(&output, "approved workspace status");
    let response = parse_json(&output);
    assert_eq!(response["gene_id"], "workspace.status");
    assert_eq!(response["status"], "completed");
    assert_eq!(response["output"], "");

    let diff_fixture = Fixture::new();
    diff_fixture.setup();
    diff_fixture.initialize_git_workspace();
    fs::write(diff_fixture.workspace.join("README.md"), "changed\n")
        .expect("fixture change should be written");
    let output = diff_fixture
        .command(&[
            "run",
            "--harness",
            "coding",
            "--gene",
            "workspace.diff",
            "diff",
            "--json",
        ])
        .output()
        .expect("workspace diff should start");
    assert_eq!(output.status.code(), Some(40));
    let response = parse_json(&output);
    assert_eq!(response["code"], "approval_required");
    let approval_id = response["details"]["approval_id"]
        .as_str()
        .expect("workspace diff should return an approval")
        .to_owned();

    let output = diff_fixture
        .command(&["approval", "resolve", &approval_id, "--allow", "--json"])
        .output()
        .expect("workspace diff approval should resolve");
    assert_success_with_context(&output, "workspace diff approval");

    let output = diff_fixture
        .command(&[
            "run",
            "--approval",
            &approval_id,
            "--harness",
            "coding",
            "--gene",
            "workspace.diff",
            "diff",
            "--json",
        ])
        .output()
        .expect("approved workspace diff should start");
    assert_success_with_context(&output, "approved workspace diff");
    let response = parse_json(&output);
    assert_eq!(response["gene_id"], "workspace.diff");
    assert_eq!(response["status"], "completed");
    assert!(response["output"].as_str().unwrap().contains("README.md"));
}

#[test]
fn direct_run_rejects_unclassified_tasks_without_a_phantom_harness() {
    let fixture = Fixture::new();
    fixture.setup();

    let output = fixture
        .command(&["run", "summarize the workspace", "--json"])
        .output()
        .expect("direct run should start");
    assert_eq!(output.status.code(), Some(50));
    let response = parse_json(&output);
    assert_eq!(response["code"], "execution_failed");
    assert_eq!(response["message"], "no default harness is available");
}

#[test]
fn coding_harness_reports_recent_git_log_through_the_governed_runtime() {
    let fixture = Fixture::new();
    fixture.initialize_git_workspace();
    fixture.setup();

    let output = fixture
        .command(&[
            "run",
            "--harness",
            "coding",
            "--gene",
            "workspace.log",
            "log",
            "--json",
        ])
        .output()
        .expect("workspace log should start");
    assert_eq!(output.status.code(), Some(40));
    let response = parse_json(&output);
    assert_eq!(response["code"], "approval_required");
    let approval_id = response["details"]["approval_id"]
        .as_str()
        .expect("workspace log should return an approval")
        .to_owned();

    let output = fixture
        .command(&["approval", "resolve", &approval_id, "--allow", "--json"])
        .output()
        .expect("workspace log approval should resolve");
    assert_success_with_context(&output, "workspace log approval");

    let output = fixture
        .command(&[
            "run",
            "--approval",
            &approval_id,
            "--harness",
            "coding",
            "--gene",
            "workspace.log",
            "log",
            "--json",
        ])
        .output()
        .expect("approved workspace log should start");
    assert_success_with_context(&output, "approved workspace log");
    let response = parse_json(&output);
    assert_eq!(response["gene_id"], "workspace.log");
    assert_eq!(response["status"], "completed");
    assert!(response["output"].as_str().unwrap().contains("fixture"));
}

#[test]
fn coding_harness_reports_git_refs_through_the_governed_runtime() {
    let fixture = Fixture::new();
    fixture.initialize_git_workspace();
    for arguments in [
        ["branch", "fixture-branch"].as_slice(),
        ["tag", "fixture-tag"].as_slice(),
    ] {
        let output = Command::new("git")
            .args(arguments)
            .current_dir(&fixture.workspace)
            .output()
            .expect("git reference fixture command should start");
        assert!(output.status.success());
    }
    fixture.setup();

    let output = fixture
        .command(&[
            "run",
            "--harness",
            "coding",
            "--gene",
            "workspace.refs",
            "refs",
            "--json",
        ])
        .output()
        .expect("workspace refs should start");
    assert_eq!(output.status.code(), Some(40));
    let response = parse_json(&output);
    assert_eq!(response["code"], "approval_required");
    let approval_id = response["details"]["approval_id"]
        .as_str()
        .expect("workspace refs should return an approval")
        .to_owned();

    let output = fixture
        .command(&["approval", "resolve", &approval_id, "--allow", "--json"])
        .output()
        .expect("workspace refs approval should resolve");
    assert_success_with_context(&output, "workspace refs approval");

    let output = fixture
        .command(&[
            "run",
            "--approval",
            &approval_id,
            "--harness",
            "coding",
            "--gene",
            "workspace.refs",
            "refs",
            "--json",
        ])
        .output()
        .expect("approved workspace refs should start");
    assert_success_with_context(&output, "approved workspace refs");
    let response = parse_json(&output);
    assert_eq!(response["gene_id"], "workspace.refs");
    assert_eq!(response["status"], "completed");
    let result = response["output"].as_str().unwrap();
    assert!(result.lines().any(|line| line == "fixture-branch"));
    assert!(result.lines().any(|line| line == "fixture-tag"));
}

#[test]
fn direct_run_accepts_the_design_harness_alias() {
    let fixture = Fixture::new();
    fixture.setup();

    let output = fixture
        .command(&[
            "run",
            "--harness",
            "design",
            "--gene",
            "design.guide",
            "design-guide",
            "--json",
        ])
        .output()
        .expect("direct design run should start");
    assert_success_with_context(&output, "direct design run");
    let response = parse_json(&output);
    assert_eq!(response["harness_id"], "design-domain");
    assert_eq!(response["gene_id"], "design.guide");
}

#[test]
fn direct_run_accepts_the_operations_harness_alias() {
    let fixture = Fixture::new();
    fixture.setup();

    let output = fixture
        .command(&[
            "run",
            "--harness",
            "operations",
            "--gene",
            "operations.guide",
            "operations-guide",
            "--json",
        ])
        .output()
        .expect("direct operations run should start");
    assert_success_with_context(&output, "direct operations run");
    let response = parse_json(&output);
    assert_eq!(response["harness_id"], "operations-domain");
    assert_eq!(response["gene_id"], "operations.guide");
}

#[test]
fn security_harness_runs_read_only_audit_and_guide_genes() {
    let fixture = Fixture::new();
    fixture.setup();

    let output = fixture
        .command(&[
            "run",
            "--harness",
            "security",
            "--gene",
            "security.guide",
            "security-guide",
            "--json",
        ])
        .output()
        .expect("security guide should start");
    assert_success_with_context(&output, "security guide");
    let response = parse_json(&output);
    assert_eq!(response["harness_id"], "security-domain");
    assert_eq!(response["gene_id"], "security.guide");
    assert_eq!(response["status"], "completed");

    let output = fixture
        .command(&[
            "harness",
            "run",
            "security",
            "--gene",
            "security.audit",
            "--task",
            "security-audit",
            "--json",
        ])
        .output()
        .expect("security audit should start");
    assert_success_with_context(&output, "security audit");
    let response = parse_json(&output);
    assert_eq!(response["harness_id"], "security-domain");
    assert_eq!(response["gene_id"], "security.audit");
    assert_eq!(response["status"], "completed");
    assert!(response["output"].is_string());

    for (gene, action) in [
        ("security.assess", "security-assess"),
        ("security.deep-scan", "security-deep-scan"),
        ("security.diff-scan", "security-diff-scan"),
    ] {
        let output = fixture
            .command(&[
                "run",
                "--harness",
                "security",
                "--gene",
                gene,
                action,
                "--json",
            ])
            .output()
            .expect("security evidence workflow should start");
        assert_success_with_context(&output, "security evidence workflow");
        let response = parse_json(&output);
        assert_eq!(response["harness_id"], "security-domain");
        assert_eq!(response["gene_id"], gene);
        assert_eq!(response["status"], "completed");
    }

    let output = fixture
        .command(&[
            "run",
            "--harness",
            "security",
            "--gene",
            "security.threat-model",
            "security-threat-model",
            "--json",
        ])
        .output()
        .expect("security threat model should start");
    assert_success_with_context(&output, "security threat model");
    let response = parse_json(&output);
    assert_eq!(response["harness_id"], "security-domain");
    assert_eq!(response["gene_id"], "security.threat-model");
    assert_eq!(response["status"], "completed");

    let output = fixture
        .command(&[
            "run",
            "--harness",
            "security",
            "--gene",
            "security.discovery",
            "security-discovery",
            "--json",
        ])
        .output()
        .expect("security discovery should start");
    assert_success_with_context(&output, "security discovery");
    let response = parse_json(&output);
    assert_eq!(response["harness_id"], "security-domain");
    assert_eq!(response["gene_id"], "security.discovery");
    assert_eq!(response["status"], "completed");
}

#[test]
fn debugging_harness_runs_read_only_failure_and_guide_genes() {
    let fixture = Fixture::new();
    fixture.setup();

    let output = fixture
        .command(&[
            "run",
            "--harness",
            "debugging",
            "--gene",
            "debugging.guide",
            "debugging-guide",
            "--json",
        ])
        .output()
        .expect("debugging guide should start");
    assert_success_with_context(&output, "debugging guide");
    let response = parse_json(&output);
    assert_eq!(response["harness_id"], "debugging-domain");
    assert_eq!(response["gene_id"], "debugging.guide");
    assert_eq!(response["status"], "completed");

    let output = fixture
        .command(&[
            "harness",
            "run",
            "debugging",
            "--gene",
            "debugging.failures",
            "--task",
            "debugging-failures",
            "--json",
        ])
        .output()
        .expect("debugging failure evidence should start");
    assert_success_with_context(&output, "debugging failure evidence");
    let response = parse_json(&output);
    assert_eq!(response["harness_id"], "debugging-domain");
    assert_eq!(response["gene_id"], "debugging.failures");
    assert_eq!(response["status"], "completed");
}

#[test]
fn data_harness_runs_read_only_schema_and_guide_genes() {
    let fixture = Fixture::new();
    fixture.setup();

    let output = fixture
        .command(&[
            "run",
            "--harness",
            "data",
            "--gene",
            "data.guide",
            "data-guide",
            "--json",
        ])
        .output()
        .expect("data guide should start");
    assert_success_with_context(&output, "data guide");
    let response = parse_json(&output);
    assert_eq!(response["harness_id"], "data-domain");
    assert_eq!(response["gene_id"], "data.guide");
    assert_eq!(response["status"], "completed");

    let output = fixture
        .command(&[
            "harness",
            "run",
            "data",
            "--gene",
            "data.schema",
            "--task",
            "data-schema",
            "--json",
        ])
        .output()
        .expect("data schema evidence should start");
    assert_success_with_context(&output, "data schema evidence");
    let response = parse_json(&output);
    assert_eq!(response["harness_id"], "data-domain");
    assert_eq!(response["gene_id"], "data.schema");
    assert_eq!(response["status"], "completed");
}

#[test]
fn operations_harness_executes_bounded_workspace_reads() {
    let fixture = Fixture::new();
    fixture.setup();

    let output = fixture
        .command(&[
            "run",
            "--harness",
            "operations",
            "--gene",
            "config.inspect",
            "config-inspect:README.md",
            "--json",
        ])
        .output()
        .expect("configuration inspection should start");
    assert_success_with_context(&output, "configuration inspection");
    let response = parse_json(&output);
    assert_eq!(response["output"], "fixture\n");

    let output = fixture
        .command(&[
            "run",
            "--harness",
            "operations",
            "--gene",
            "deployment.evidence",
            "deployment-evidence",
            "--json",
        ])
        .output()
        .expect("deployment evidence should start");
    assert_success_with_context(&output, "deployment evidence");
    let response = parse_json(&output);
    let evidence = response["output"].as_str().unwrap();
    assert!(evidence.contains("FROM :"));
    assert!(evidence.contains("services::"));
    assert!(evidence.contains("apiVersion::"));
    assert!(evidence.contains("workflow_dispatch::"));
}

#[test]
fn slash_commands_cover_the_built_in_domains_and_execute_workflow_genes() {
    let fixture = Fixture::new();
    fixture.setup();

    let output = fixture
        .command(&["slash", "list", "--json"])
        .output()
        .expect("slash list should start");
    assert_success(&output);
    let response = parse_json(&output);
    let commands = response["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|command| command["command"].as_str().unwrap())
        .collect::<Vec<_>>();
    for expected in [
        "/coding",
        "/read",
        "/search",
        "/patch",
        "/verify",
        "/test",
        "/format",
        "/lint",
        "/build",
        "/status",
        "/diff",
        "/log",
        "/refs",
        "/review",
        "/audit",
        "/argus-review",
        "/debt",
        "/measure",
        "/guide",
        "/research",
        "/evidence-inventory",
        "/evidence-search",
        "/source-read",
        "/source-compare",
        "/citation-inventory",
        "/research-guide",
        "/design",
        "/design-inventory",
        "/design-tokens",
        "/design-inspect",
        "/design-compare",
        "/accessibility-evidence",
        "/design-guide",
        "/operations",
        "/operations-inventory",
        "/operations-search",
        "/config-inspect",
        "/config-compare",
        "/deployment-evidence",
        "/operations-guide",
        "/security",
        "/security-audit",
        "/security-scan",
        "/security-deep-scan",
        "/security-diff-scan",
        "/security-dependencies",
        "/security-threat-model",
        "/security-discovery",
        "/security-triage",
        "/security-attack-path",
        "/security-validation",
        "/security-fix",
        "/security-verify-fix",
        "/security-writeup",
        "/security-track",
        "/security-hardening",
        "/security-policy",
        "/security-guide",
    ] {
        assert!(commands.contains(&expected), "missing {expected}");
    }

    let output = fixture
        .command(&["slash", "resolve", "/audit", "--json"])
        .output()
        .expect("slash resolve should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["target"]["harness_id"], "coding-domain");
    assert_eq!(response["target"]["gene_id"], "daedalus.audit");

    let output = fixture
        .command(&["/guide", "--json"])
        .output()
        .expect("direct slash command should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["harness_id"], "coding-domain");
    assert_eq!(response["gene_id"], "athena.guide");
    assert!(response["output"].as_str().unwrap().contains("Daedalus"));

    let output = fixture
        .command(&["/research-guide", "--json"])
        .output()
        .expect("research guide slash command should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["harness_id"], "research-domain");
    assert_eq!(response["gene_id"], "research.guide");
    assert!(
        response["output"]
            .as_str()
            .unwrap()
            .contains("Evidence inventory")
    );

    let output = fixture
        .command(&["/design-guide", "--json"])
        .output()
        .expect("design guide slash command should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["harness_id"], "design-domain");
    assert_eq!(response["gene_id"], "design.guide");
    assert!(
        response["output"]
            .as_str()
            .unwrap()
            .contains("Design inventory")
    );

    let output = fixture
        .command(&["/operations-guide", "--json"])
        .output()
        .expect("operations guide slash command should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["harness_id"], "operations-domain");
    assert_eq!(response["gene_id"], "operations.guide");
    assert!(
        response["output"]
            .as_str()
            .unwrap()
            .contains("Operations inventory")
    );
}

#[test]
fn package_meta_admission_survives_cli_restart_without_runtime_authority() {
    let fixture = Fixture::new();
    fixture.setup();
    let artifact = b"custom meta profile\n";
    let manifest = PackageManifest::new_meta(
        "example/meta",
        "1.0.0",
        "local-publisher",
        hash_artifact(artifact),
        Vec::new(),
        PackageCompatibility::new(concat!("pandora>=", env!("CARGO_PKG_VERSION"))).unwrap(),
        "MIT",
        TrustEvidence::unsigned(),
        MetaComposition::new(vec![HarnessId::new("coding-domain").unwrap()], 4).unwrap(),
    )
    .unwrap();
    let manifest_path = fixture.root.join("meta.json");
    let artifact_path = fixture.root.join("meta.artifact");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("manifest should serialize"),
    )
    .expect("manifest should be written");
    fs::write(&artifact_path, artifact).expect("artifact should be written");

    let output = fixture
        .command(&[
            "package",
            "admit",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--artifact",
            artifact_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("package admission should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["package"]["kind"], "meta_harness");
    assert_eq!(response["package"]["state"], "admitted");
    assert_eq!(response["package"]["activation"]["state"], "disabled");
    assert_eq!(response["package"]["runtime_authority"], false);

    let output = fixture
        .command(&["package", "list", "--json"])
        .output()
        .expect("package listing should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["packages"].as_array().unwrap().len(), 1);
    assert_eq!(response["packages"][0]["id"], "example/meta");
    assert_eq!(
        response["packages"][0]["meta_composition"]["max_handoffs"],
        4
    );

    let output = fixture
        .command(&["harness", "list", "--json"])
        .output()
        .expect("harness listing should start");
    assert_success(&output);
    let response = parse_json(&output);
    let admitted = response["package_records"]
        .as_array()
        .expect("harness listing should expose local package records");
    assert_eq!(admitted.len(), 1);
    assert_eq!(admitted[0]["id"], "example/meta");
    assert_eq!(admitted[0]["kind"], "meta_harness");
    assert_eq!(admitted[0]["state"], "admitted");
    assert_eq!(admitted[0]["runtime_authority"], false);
    let profiles = response["admitted_profiles"]
        .as_array()
        .expect("harness listing should expose admitted profiles");
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0]["id"], "example/meta");
    assert_eq!(profiles[0]["kind"], "meta");
    assert_eq!(profiles[0]["package_kind"], "meta_harness");
    assert_eq!(profiles[0]["execution"]["runnable"], false);
    assert_eq!(profiles[0]["execution"]["mode"], "composition_only");
    assert_eq!(profiles[0]["runtime_authority"], false);

    let output = fixture
        .command(&["package", "inspect", "example/meta", "1.0.0", "--json"])
        .output()
        .expect("package inspection should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["package"]["state"], "admitted");

    let output = fixture
        .command(&[
            "package",
            "enable",
            "example/meta",
            "1.0.0",
            "--yes",
            "--json",
        ])
        .output()
        .expect("Meta profile activation should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["package"]["activation"]["state"], "enabled");
    assert_eq!(response["binding"]["active_version"], "1.0.0");
    assert_eq!(response["binding"]["runtime_authority"], false);

    let output = fixture
        .command(&[
            "harness",
            "inspect",
            "example/meta",
            "--harness-version",
            "1.0.0",
            "--json",
        ])
        .output()
        .expect("Meta profile inspection should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["harness"]["kind"], "meta");
    assert_eq!(response["harness"]["execution"]["runnable"], false);
    assert_eq!(response["harness"]["meta_composition"]["max_handoffs"], 4);

    let output = fixture
        .command(&[
            "harness",
            "run",
            "example/meta",
            "--harness-version",
            "1.0.0",
            "--gene",
            "workspace.read",
            "--task",
            "read:README.md",
            "--json",
        ])
        .output()
        .expect("Meta profile run should start");
    assert_eq!(output.status.code(), Some(50));
    let response = parse_json(&output);
    assert_eq!(response["code"], "execution_failed");
    assert_eq!(
        response["message"],
        "harness 'example/meta' is not runnable"
    );
    assert_eq!(response["details"]["kind"], "meta");

    let output = fixture
        .command(&["session", "list", "--json"])
        .output()
        .expect("session listing should start");
    assert_success(&output);
    assert!(
        parse_json(&output)["sessions"]
            .as_array()
            .expect("sessions should be an array")
            .is_empty(),
        "a non-runnable Harness must not create a session"
    );

    let output = fixture
        .command(&[
            "package",
            "disable",
            "example/meta",
            "1.0.0",
            "--yes",
            "--json",
        ])
        .output()
        .expect("Meta profile disable should start");
    assert_success(&output);
    assert_eq!(parse_json(&output)["binding"]["state"], "disabled");

    let output = fixture
        .command(&["package", "remove", "example/meta", "1.0.0", "--json"])
        .output()
        .expect("unconfirmed package removal should start");
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(parse_json(&output)["code"], "usage_error");

    let output = fixture
        .command(&[
            "package",
            "remove",
            "example/meta",
            "1.0.0",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("package removal dry-run should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["dry_run"], true);
    assert_eq!(response["removed"], false);

    let output = fixture
        .command(&[
            "package",
            "remove",
            "example/meta",
            "1.0.0",
            "--yes",
            "--json",
        ])
        .output()
        .expect("package removal should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["dry_run"], false);
    assert_eq!(response["removed"], true);

    let output = fixture
        .command(&["package", "inspect", "example/meta", "1.0.0", "--json"])
        .output()
        .expect("removed package inspection should start");
    assert_eq!(output.status.code(), Some(50));
    assert_eq!(parse_json(&output)["code"], "execution_failed");
}

#[test]
fn registry_profiles_are_persistent_selectable_and_removable() {
    let fixture = Fixture::new();
    for arguments in [
        [
            "registry",
            "set",
            "--name",
            "private",
            "--registry-url",
            "https://registry.example.test/",
            "--token-env",
            "PANDORA_PRIVATE_REGISTRY_TOKEN",
            "--json",
        ]
        .as_slice(),
        [
            "registry",
            "set",
            "--name",
            "public",
            "--registry-url",
            "https://public.example.test",
            "--json",
        ]
        .as_slice(),
        ["registry", "use", "private", "--json"].as_slice(),
    ] {
        let output = fixture
            .command(arguments)
            .output()
            .expect("registry command should start");
        assert_success_with_context(&output, "registry profile lifecycle");
    }

    let listed = fixture
        .command(&["registry", "list", "--json"])
        .output()
        .expect("registry list should start");
    assert_success(&listed);
    let listed = parse_json(&listed);
    let registries = listed["registries"].as_array().unwrap();
    assert_eq!(registries.len(), 2);
    assert_eq!(registries[0]["name"], "private");
    assert_eq!(registries[0]["token_env"], "PANDORA_PRIVATE_REGISTRY_TOKEN");
    assert_eq!(registries[0]["active"], true);
    assert_eq!(registries[0]["base_url"], "https://registry.example.test");
    assert_eq!(registries[1]["name"], "public");
    assert!(registries[1]["token_env"].is_null());

    let refused = fixture
        .command(&["registry", "remove", "private", "--json"])
        .output()
        .expect("registry removal preview should start");
    assert!(!refused.status.success());

    let removed = fixture
        .command(&["registry", "remove", "private", "--yes", "--json"])
        .output()
        .expect("registry removal should start");
    assert_success(&removed);
    assert_eq!(parse_json(&removed)["active_registry"], "public");
}

#[test]
fn package_install_fetches_and_admits_one_exact_registry_release() {
    let artifact = b"registry gene artifact\n";
    let content_hash = hash_artifact(artifact);
    let server_content_hash = content_hash.clone();
    let listener = TcpListener::bind("127.0.0.1:0").expect("registry fixture should bind");
    let address = listener
        .local_addr()
        .expect("registry fixture should expose its address");
    let server = thread::spawn(move || {
        for (expected_path, body, content_type) in [
            (
                "/api/v1/packages/owner%2Fpackage/versions/1.0.0-beta.1%2Bbuild.5",
                serde_json::to_vec(&serde_json::json!({
                    "id": "owner/package",
                    "name": "Package",
                    "version": "1.0.0-beta.1+build.5",
                    "kind": "gene",
                    "description": "Registry fixture",
                    "author": "owner",
                    "license": "MIT",
                    "trust": {
                        "level": "community",
                        "signature": null,
                        "public_key": null,
                        "content_hash": server_content_hash,
                        "publisher": "owner"
                    },
                    "capabilities": {"provides": ["fixture"], "requires": []},
                    "downloads": 0,
                    "success_rate": 0.0,
                    "compatibility": {
                        "runtimes": ["pandora>=2.0.0-alpha.1"],
                        "platforms": []
                    },
                    "repository": "https://example.com/owner/package",
                    "artifact_url": "https://127.0.0.1:9/must-not-be-contacted",
                    "homepage": null,
                    "tags": ["fixture"],
                    "yanked": false,
                    "deprecated": null,
                    "provenance": null,
                    "created_at": "2026-08-22T00:00:00Z",
                    "updated_at": "2026-08-22T00:00:00Z"
                }))
                .unwrap(),
                "application/json",
            ),
            (
                "/api/v1/packages/owner%2Fpackage/versions/1.0.0-beta.1%2Bbuild.5/download",
                artifact.to_vec(),
                "application/octet-stream",
            ),
        ] {
            let (mut stream, _) = listener.accept().expect("Pandora should connect");
            let mut request = Vec::new();
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let mut chunk = [0_u8; 1_024];
                let bytes_read = stream.read(&mut chunk).expect("request should be readable");
                assert_ne!(bytes_read, 0, "request ended before its headers");
                request.extend_from_slice(&chunk[..bytes_read]);
            }
            let headers = String::from_utf8_lossy(&request);
            assert!(
                headers.starts_with(&format!("GET {expected_path} HTTP/1.1\r\n")),
                "unexpected registry request: {headers}"
            );
            assert!(
                headers
                    .to_ascii_lowercase()
                    .contains("authorization: bearer registry-secret\r\n")
            );
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .expect("response headers should be written");
            stream
                .write_all(&body)
                .expect("response body should be written");
        }
    });

    let fixture = Fixture::new();
    let registry = format!("http://{address}");
    let configured = fixture
        .command(&[
            "registry",
            "set",
            "--name",
            "fixture",
            "--registry-url",
            &registry,
            "--token-env",
            "PANDORA_TEST_REGISTRY_TOKEN",
            "--json",
        ])
        .output()
        .expect("registry profile setup should start");
    assert_success_with_context(&configured, "registry set");
    let output = fixture
        .command(&[
            "package",
            "install",
            "owner/package",
            "1.0.0-beta.1+build.5",
            "--json",
        ])
        .env("PANDORA_TEST_REGISTRY_TOKEN", "registry-secret")
        .output()
        .expect("registry installation should start");
    assert_success_with_context(&output, "package install");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("registry-secret"));
    let response = parse_json(&output);
    assert_eq!(response["package"]["id"], "owner/package");
    assert_eq!(response["package"]["version"], "1.0.0-beta.1+build.5");
    assert_eq!(response["package"]["kind"], "gene");
    assert_eq!(response["package"]["state"], "installed");
    assert_eq!(response["package"]["runtime_authority"], false);

    let output = fixture
        .command(&[
            "package",
            "inspect",
            "owner/package",
            "1.0.0-beta.1+build.5",
            "--json",
        ])
        .output()
        .expect("installed package inspection should start");
    assert_success(&output);
    assert_eq!(parse_json(&output)["package"]["content_hash"], content_hash);
    server.join().expect("registry fixture should finish");
}

#[test]
fn package_trust_root_cli_supports_official_admission_and_revocation() {
    let fixture = Fixture::new();
    fixture.setup();
    let artifact = b"official gene artifact\n";
    let signing_key = SigningKey::from_bytes(&[41_u8; 32]);
    let public_key = signing_key
        .verifying_key()
        .to_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let unsigned = PackageManifest::new(
        "example/official-gene",
        "1.0.0",
        PackageKind::Gene,
        "official-publisher",
        hash_artifact(artifact),
        Vec::new(),
        PackageCompatibility::new(concat!("pandora>=", env!("CARGO_PKG_VERSION"))).unwrap(),
        "MIT",
        TrustEvidence::unsigned(),
    )
    .unwrap();
    let signature = signing_key.sign(unsigned.signing_message().as_bytes());
    let signature = signature
        .to_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let manifest = PackageManifest::new(
        "example/official-gene",
        "1.0.0",
        PackageKind::Gene,
        "official-publisher",
        hash_artifact(artifact),
        Vec::new(),
        PackageCompatibility::new(concat!("pandora>=", env!("CARGO_PKG_VERSION"))).unwrap(),
        "MIT",
        TrustEvidence::new(
            TrustLevel::Official,
            Some(signature),
            Some(public_key.clone()),
        )
        .unwrap(),
    )
    .unwrap();
    let manifest_path = fixture.root.join("official.json");
    let artifact_path = fixture.root.join("official.artifact");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("manifest should serialize"),
    )
    .expect("manifest should be written");
    fs::write(&artifact_path, artifact).expect("artifact should be written");

    let output = fixture
        .command(&[
            "package",
            "trust-root",
            "add",
            "--publisher",
            "official-publisher",
            "--key-id",
            "official-key-1",
            "--public-key",
            public_key.as_str(),
            "--json",
        ])
        .output()
        .expect("trust-root add should start");
    assert_success(&output);
    assert_eq!(parse_json(&output)["active"], true);

    let output = fixture
        .command(&[
            "package",
            "admit",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--artifact",
            artifact_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("official package admission should start");
    assert_success(&output);
    assert_eq!(parse_json(&output)["package"]["state"], "installed");

    let output = fixture
        .command(&[
            "package",
            "admit",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--artifact",
            artifact_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("duplicate package admission should start");
    assert_eq!(output.status.code(), Some(50));

    let output = fixture
        .command(&[
            "package",
            "trust-root",
            "revoke",
            "--publisher",
            "official-publisher",
            "--key-id",
            "official-key-1",
            "--yes",
            "--json",
        ])
        .output()
        .expect("trust-root revoke should start");
    assert_success(&output);
    assert_eq!(parse_json(&output)["active"], false);

    let output = fixture
        .command(&["package", "transparency", "list", "--limit", "10", "--json"])
        .output()
        .expect("package transparency list should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["count"], 4);
    assert_eq!(response["events"][0]["event_kind"], "trust_root_revoked");
    assert_eq!(response["events"][1]["outcome"], "denied");
    assert_eq!(response["events"][1]["reason_code"], "duplicate_identity");
    assert_eq!(response["events"][2]["outcome"], "allowed");
    assert_eq!(response["events"][3]["event_kind"], "trust_root_added");
    assert_eq!(response["durability"], "append-only-sqlite");
    assert_eq!(response["integrity"], "sha256-event-chain");

    let output = fixture
        .command(&[
            "package",
            "transparency",
            "list",
            "--event-kind",
            "admission_decision",
            "--outcome",
            "denied",
            "--json",
        ])
        .output()
        .expect("filtered package transparency list should start");
    assert_success(&output);
    assert_eq!(parse_json(&output)["count"], 1);

    let output = fixture
        .command(&[
            "package",
            "transparency",
            "inspect",
            "--sequence",
            "2",
            "--json",
        ])
        .output()
        .expect("package transparency inspection should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["event"]["sequence"], 2);
    assert_eq!(response["event"]["reason_code"], "admitted");
    assert_eq!(response["runtime_authority"], false);

    let output = fixture
        .command(&["package", "list", "--json"])
        .output()
        .expect("package list after revocation should start");
    assert_eq!(output.status.code(), Some(50));
}

#[test]
fn package_keygen_and_sign_keep_private_material_in_the_vault() {
    let fixture = Fixture::new();
    fixture.setup();
    let artifact = b"locally signed gene artifact
";
    let unsigned = PackageManifest::new(
        "example/local-signed-gene",
        "1.0.0",
        PackageKind::Gene,
        "local-publisher",
        hash_artifact(artifact),
        Vec::new(),
        PackageCompatibility::new(concat!("pandora>=", env!("CARGO_PKG_VERSION"))).unwrap(),
        "MIT",
        TrustEvidence::unsigned(),
    )
    .unwrap();
    let manifest_path = fixture.root.join("unsigned.json");
    let artifact_path = fixture.root.join("signed.artifact");
    let signed_path = fixture.root.join("signed.json");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&unsigned).expect("manifest should serialize"),
    )
    .expect("manifest should be written");
    fs::write(&artifact_path, artifact).expect("artifact should be written");

    let output = fixture
        .command(&[
            "package",
            "keygen",
            "--publisher",
            "local-publisher",
            "--key-id",
            "local-key-1",
            "--secret-name",
            "PANDORA_PACKAGE_SIGNING_KEY",
            "--json",
        ])
        .env("PANDORA_MASTER_KEY", "test-master-key-1234")
        .output()
        .expect("package keygen should start");
    assert_success_with_context(&output, "package signing test");
    let keygen = parse_json(&output);
    assert_eq!(keygen["private_key_exposed"], false);
    assert_eq!(keygen["stored"], true);
    let public_key = keygen["public_key"].as_str().unwrap().to_owned();
    assert!(!String::from_utf8_lossy(&output.stdout).contains(r#""private_key":"#));

    let output = fixture
        .command(&[
            "package",
            "sign",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--artifact",
            artifact_path.to_str().unwrap(),
            "--secret-name",
            "PANDORA_PACKAGE_SIGNING_KEY",
            "--output",
            signed_path.to_str().unwrap(),
            "--json",
        ])
        .env("PANDORA_MASTER_KEY", "test-master-key-1234")
        .output()
        .expect("package sign should start");
    assert_success(&output);
    let signed = parse_json(&output);
    assert_eq!(signed["private_key_exposed"], false);
    assert_eq!(signed["key_id"], "local-key-1");
    assert_eq!(signed["public_key"], public_key);
    assert_eq!(signed["signature_present"], true);
    let signed_manifest: PackageManifest = serde_json::from_slice(
        &fs::read(&signed_path).expect("signed manifest should be readable"),
    )
    .expect("signed manifest should deserialize");
    assert_eq!(signed_manifest.trust().level(), TrustLevel::Verified);
    assert_eq!(
        signed_manifest.trust().public_key(),
        Some(public_key.as_str())
    );
    assert!(signed_manifest.trust().signature().is_some());

    let output = fixture
        .command(&[
            "package",
            "admit",
            "--manifest",
            signed_path.to_str().unwrap(),
            "--artifact",
            artifact_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("signed package admission should start");
    assert_success(&output);
    assert_eq!(parse_json(&output)["package"]["state"], "installed");

    let output = fixture
        .command(&["secret", "list", "--json"])
        .env("PANDORA_MASTER_KEY", "test-master-key-1234")
        .output()
        .expect("secret listing should start");
    assert_success(&output);
    assert_eq!(parse_json(&output)["values_exposed"], false);
}

#[test]
fn package_admission_rejects_invalid_signed_trust_evidence() {
    let fixture = Fixture::new();
    let artifact = b"signed gene artifact\n";
    let manifest = PackageManifest::new(
        "example/gene",
        "1.0.0",
        PackageKind::Gene,
        "local-publisher",
        hash_artifact(artifact),
        Vec::new(),
        PackageCompatibility::new(concat!("pandora>=", env!("CARGO_PKG_VERSION"))).unwrap(),
        "MIT",
        TrustEvidence::new(
            pandora_types::TrustLevel::Verified,
            Some("signature".to_owned()),
            Some("public-key".to_owned()),
        )
        .unwrap(),
    )
    .unwrap();
    let manifest_path = fixture.root.join("gene.json");
    let artifact_path = fixture.root.join("gene.artifact");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("manifest should serialize"),
    )
    .expect("manifest should be written");
    fs::write(&artifact_path, artifact).expect("artifact should be written");

    let output = fixture
        .command(&[
            "package",
            "admit",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--artifact",
            artifact_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("package admission should start");
    assert_eq!(output.status.code(), Some(50));
    let response = parse_json(&output);
    assert_eq!(response["code"], "execution_failed");
    assert_eq!(
        response["message"],
        "package signature evidence is not valid fixed-width hex"
    );

    let output = fixture
        .command(&["package", "list", "--json"])
        .output()
        .expect("package listing should start");
    assert_success(&output);
    assert!(
        parse_json(&output)["packages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn package_lock_is_written_and_verified_against_the_local_store() {
    let fixture = Fixture::new();
    fixture.setup();
    let artifact = b"lockable gene artifact\n";
    let manifest = PackageManifest::new(
        "example/gene",
        "1.0.0-beta.1+build.5",
        PackageKind::Gene,
        "local-publisher",
        hash_artifact(artifact),
        Vec::new(),
        PackageCompatibility::new(concat!("pandora>=", env!("CARGO_PKG_VERSION"))).unwrap(),
        "MIT",
        TrustEvidence::unsigned(),
    )
    .unwrap();
    let manifest_path = fixture.root.join("gene.json");
    let artifact_path = fixture.root.join("gene.artifact");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(&artifact_path, artifact).unwrap();
    let output = fixture
        .command(&[
            "package",
            "admit",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--artifact",
            artifact_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("package admission should start");
    assert_success(&output);

    let output = fixture
        .command(&["package", "lock", "--json"])
        .output()
        .expect("package lock should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["package_count"], 1);
    assert_eq!(response["format_version"], 1);
    assert_eq!(
        response["path"],
        fixture.workspace.join("pandora.lock").display().to_string()
    );
    let lock: Value =
        serde_json::from_slice(&fs::read(fixture.workspace.join("pandora.lock")).unwrap()).unwrap();
    assert_eq!(lock["packages"][0]["id"], "example/gene");
    assert_eq!(lock["packages"][0]["version"], "1.0.0-beta.1+build.5");
    assert_eq!(lock["packages"][0]["content_hash"], hash_artifact(artifact));

    let output = fixture
        .command(&["package", "verify-lock", "--json"])
        .output()
        .expect("package lock verification should start");
    assert_success(&output);
    assert_eq!(parse_json(&output)["verified"], true);

    let output = fixture
        .command(&[
            "package",
            "remove",
            "example/gene",
            "1.0.0-beta.1+build.5",
            "--yes",
            "--json",
        ])
        .output()
        .expect("package removal should start");
    assert_success(&output);
    let output = fixture
        .command(&["package", "verify-lock", "--json"])
        .output()
        .expect("stale package lock verification should start");
    assert_eq!(output.status.code(), Some(50));
    assert_eq!(
        parse_json(&output)["message"],
        "package lock does not match the admitted package set"
    );
}

#[test]
fn package_lifecycle_updates_and_rolls_back_one_exact_version() {
    let fixture = Fixture::new();
    fixture.setup();
    for (version, artifact) in [
        ("1.0.0", b"version one".as_slice()),
        ("2.0.0", b"version two".as_slice()),
    ] {
        let manifest = PackageManifest::new(
            "example/versioned",
            version,
            PackageKind::Gene,
            "local-publisher",
            hash_artifact(artifact),
            Vec::new(),
            PackageCompatibility::new(concat!("pandora>=", env!("CARGO_PKG_VERSION"))).unwrap(),
            "MIT",
            TrustEvidence::unsigned(),
        )
        .unwrap();
        let manifest_path = fixture.root.join(format!("versioned-{version}.json"));
        let artifact_path = fixture.root.join(format!("versioned-{version}.wasm"));
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        fs::write(&artifact_path, artifact).unwrap();
        let output = fixture
            .command(&[
                "package",
                "admit",
                "--manifest",
                manifest_path.to_str().unwrap(),
                "--artifact",
                artifact_path.to_str().unwrap(),
                "--json",
            ])
            .output()
            .expect("versioned package admission should start");
        assert_success(&output);
        assert_eq!(
            parse_json(&output)["package"]["activation"]["state"],
            "disabled"
        );
    }

    let output = fixture
        .command(&[
            "package",
            "enable",
            "example/versioned",
            "1.0.0",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("activation preview should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["ready"], true);
    assert_eq!(response["changed"], false);

    for version in ["1.0.0", "2.0.0"] {
        let output = fixture
            .command(&[
                "package",
                "enable",
                "example/versioned",
                version,
                "--yes",
                "--json",
            ])
            .output()
            .expect("exact activation should start");
        assert_success(&output);
    }
    let response = parse_json(
        &fixture
            .command(&["package", "inspect", "example/versioned", "2.0.0", "--json"])
            .output()
            .unwrap(),
    );
    assert_eq!(response["package"]["activation"]["active_version"], "2.0.0");
    assert_eq!(
        response["package"]["activation"]["previous_version"],
        "1.0.0"
    );

    let output = fixture
        .command(&[
            "package",
            "rollback",
            "example/versioned",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("rollback preview should start");
    assert_success(&output);
    assert_eq!(parse_json(&output)["target_version"], "1.0.0");

    let output = fixture
        .command(&[
            "package",
            "rollback",
            "example/versioned",
            "--yes",
            "--json",
        ])
        .output()
        .expect("rollback should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["active_version"], "1.0.0");
    assert_eq!(response["binding"]["previous_version"], "2.0.0");

    let output = fixture
        .command(&[
            "package",
            "disable",
            "example/versioned",
            "1.0.0",
            "--yes",
            "--json",
        ])
        .output()
        .expect("disable should start");
    assert_success(&output);
    assert_eq!(parse_json(&output)["binding"]["state"], "disabled");
}

#[test]
fn admitted_domain_profile_runs_with_an_explicit_version() {
    let fixture = Fixture::new();
    fixture.setup();
    let artifact = b"domain profile\n";
    let manifest = PackageManifest::new(
        "example/domain",
        "1.0.0",
        PackageKind::DomainHarness,
        "local-publisher",
        hash_artifact(artifact),
        vec![PackageDependency::new("workspace.read", "0.1.0", false).unwrap()],
        PackageCompatibility::new(concat!("pandora>=", env!("CARGO_PKG_VERSION"))).unwrap(),
        "MIT",
        TrustEvidence::unsigned(),
    )
    .unwrap()
    .with_domain_routing(DomainRoutingProfile::new(vec!["read:readme.md".to_owned()]).unwrap())
    .unwrap();
    let manifest_path = fixture.root.join("domain.json");
    let artifact_path = fixture.root.join("domain.artifact");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("manifest should be written"),
    )
    .expect("manifest should be written");
    fs::write(&artifact_path, artifact).expect("artifact should be written");

    let output = fixture
        .command(&[
            "package",
            "admit",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--artifact",
            artifact_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("domain profile admission should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["package"]["state"], "admitted");
    assert_eq!(response["package"]["activation"]["state"], "disabled");
    assert_eq!(
        response["package"]["domain_routing"]["hints"][0],
        "read:readme.md"
    );

    let output = fixture
        .command(&["harness", "list", "--json"])
        .output()
        .expect("harness listing should start");
    assert_success(&output);
    let response = parse_json(&output);
    let profiles = response["admitted_profiles"]
        .as_array()
        .expect("harness listing should expose admitted profiles");
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0]["id"], "example/domain");
    assert_eq!(profiles[0]["kind"], "domain");
    assert_eq!(profiles[0]["package_kind"], "domain_harness");
    assert_eq!(profiles[0]["execution"]["runnable"], true);
    assert_eq!(profiles[0]["execution"]["mode"], "domain_execution");
    assert_eq!(profiles[0]["runtime_authority"], false);

    let output = fixture
        .command(&[
            "package",
            "enable",
            "example/domain",
            "1.0.0",
            "--yes",
            "--json",
        ])
        .output()
        .expect("Domain profile activation should start");
    assert_success(&output);
    assert_eq!(
        parse_json(&output)["package"]["activation"]["state"],
        "enabled"
    );

    let output = fixture
        .command(&[
            "harness",
            "inspect",
            "example/domain",
            "--harness-version",
            "1.0.0",
            "--json",
        ])
        .output()
        .expect("domain profile inspection should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["harness"]["id"], "example/domain");
    assert_eq!(response["harness"]["kind"], "domain");
    assert_eq!(response["harness"]["execution"]["runnable"], true);
    assert_eq!(response["harness"]["genes"][0]["id"], "workspace.read");

    let output = fixture
        .command(&[
            "run",
            "--harness",
            "example/domain",
            "--harness-version",
            "1.0.0",
            "--gene",
            "workspace.read",
            "read:README.md",
            "--json",
        ])
        .output()
        .expect("domain profile run should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["status"], "completed");
    assert_eq!(response["harness_id"], "example/domain");
    assert_eq!(response["gene_id"], "workspace.read");

    let output = fixture
        .command(&["run", "read:README.md", "--json"])
        .output()
        .expect("Auto Route should load the active custom Domain catalog");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["harness_id"], "example/domain");
    assert_eq!(response["gene_id"], "workspace.read");

    let output = fixture
        .command(&[
            "harness",
            "run",
            "example/domain",
            "--harness-version",
            "1.0.0",
            "--gene",
            "workspace.read",
            "--task",
            "read:README.md",
            "--json",
        ])
        .output()
        .expect("custom Domain Harness command should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["command"], "harness run");
    assert_eq!(response["status"], "completed");
    assert_eq!(response["harness_id"], "example/domain");

    let output = fixture
        .command(&[
            "/gene:example%2Fdomain@1.0.0:workspace.read",
            "README.md",
            "--json",
        ])
        .output()
        .expect("custom Domain Harness slash command should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["status"], "completed");
    assert_eq!(response["harness_id"], "example/domain");
    assert_eq!(response["gene_id"], "workspace.read");
}

#[test]
fn signed_optional_builtin_replacement_activates_and_restores_the_compiled_domain() {
    let fixture = Fixture::new();
    fixture.setup();
    let artifact = b"signed coding replacement\n";
    let dependencies = vec![PackageDependency::new("workspace.read", "0.1.0", false).unwrap()];
    let routing = DomainRoutingProfile::new(vec!["firmware development".to_owned()]).unwrap();
    let unsigned = PackageManifest::new(
        "coding-domain",
        "9.0.0",
        PackageKind::DomainHarness,
        "local-publisher",
        hash_artifact(artifact),
        dependencies.clone(),
        PackageCompatibility::new(concat!("pandora>=", env!("CARGO_PKG_VERSION"))).unwrap(),
        "MIT",
        TrustEvidence::unsigned(),
    )
    .unwrap()
    .with_domain_routing(routing.clone())
    .unwrap();
    let signing_key = SigningKey::from_bytes(&[23_u8; 32]);
    let signature = signing_key.sign(unsigned.signing_message().as_bytes());
    let signature = signature
        .to_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let public_key = signing_key
        .verifying_key()
        .to_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let manifest = PackageManifest::new(
        "coding-domain",
        "9.0.0",
        PackageKind::DomainHarness,
        "local-publisher",
        hash_artifact(artifact),
        dependencies,
        PackageCompatibility::new(concat!("pandora>=", env!("CARGO_PKG_VERSION"))).unwrap(),
        "MIT",
        TrustEvidence::new(TrustLevel::Verified, Some(signature), Some(public_key)).unwrap(),
    )
    .unwrap()
    .with_domain_routing(routing)
    .unwrap();
    let manifest_path = fixture.root.join("coding-replacement.json");
    let artifact_path = fixture.root.join("coding-replacement.artifact");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(&artifact_path, artifact).unwrap();

    let admitted = fixture
        .command(&[
            "package",
            "admit",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--artifact",
            artifact_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("signed built-in replacement admission should start");
    assert_success(&admitted);
    assert_eq!(parse_json(&admitted)["package"]["replaces_builtin"], true);

    let enabled = fixture
        .command(&[
            "package",
            "enable",
            "coding-domain",
            "9.0.0",
            "--yes",
            "--json",
        ])
        .output()
        .expect("signed built-in replacement activation should start");
    assert_success(&enabled);

    let replacement = fixture
        .command(&["harness", "inspect", "coding-domain", "--json"])
        .output()
        .expect("active replacement should be inspectable");
    assert_success(&replacement);
    let replacement = parse_json(&replacement);
    assert_eq!(replacement["harness"]["version"], "9.0.0");
    assert_eq!(replacement["harness"]["genes"].as_array().unwrap().len(), 1);
    assert_eq!(replacement["harness"]["genes"][0]["id"], "workspace.read");

    let disabled = fixture
        .command(&[
            "package",
            "disable",
            "coding-domain",
            "9.0.0",
            "--yes",
            "--json",
        ])
        .output()
        .expect("built-in replacement disable should start");
    assert_success(&disabled);

    let restored = fixture
        .command(&["harness", "inspect", "coding-domain", "--json"])
        .output()
        .expect("compiled domain should be restored");
    assert_success(&restored);
    let restored = parse_json(&restored);
    assert_ne!(restored["harness"]["version"], "9.0.0");
    assert!(restored["harness"]["genes"].as_array().unwrap().len() > 1);
}

#[test]
fn package_validate_reports_wasm_boundary_without_persisting() {
    let fixture = Fixture::new();
    fixture.setup();
    let wasm = wat::parse_str(
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
    let manifest = PackageManifest::new(
        "example/validate",
        "1.0.0",
        PackageKind::Gene,
        "local-publisher",
        hash_artifact(&wasm),
        Vec::new(),
        PackageCompatibility::new(concat!("pandora>=", env!("CARGO_PKG_VERSION"))).unwrap(),
        "MIT",
        TrustEvidence::unsigned(),
    )
    .unwrap();
    let manifest_path = fixture.root.join("validate.json");
    let artifact_path = fixture.root.join("validate.wasm");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(&artifact_path, &wasm).unwrap();

    let output = fixture
        .command(&[
            "package",
            "validate",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--artifact",
            artifact_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("package validation should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["command"], "package validate");
    assert_eq!(response["valid"], true);
    assert_eq!(response["execution_boundary"], "wasm");
    assert_eq!(response["persisted"], false);
    assert_eq!(response["package"]["id"], "example/validate");

    let output = fixture
        .command(&["package", "list", "--json"])
        .output()
        .expect("package listing should start");
    assert_success(&output);
    assert!(
        parse_json(&output)["packages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn tracked_domain_harness_reference_package_validates() {
    let fixture = Fixture::new();
    fixture.setup();
    let starter = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("sdk/domain-harness-starter");
    let output = fixture
        .command(&[
            "package",
            "validate",
            "--manifest",
            starter.join("pandora.package.json").to_str().unwrap(),
            "--artifact",
            starter.join("domain-harness.artifact").to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("tracked Domain Harness starter validation should start");

    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["package"]["id"], "example/domain-starter");
    assert_eq!(response["execution_boundary"], "metadata-only");
    assert_eq!(response["persisted"], false);
}

#[test]
fn tracked_meta_harness_reference_package_validates() {
    let fixture = Fixture::new();
    fixture.setup();
    let starter = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("sdk/meta-harness-starter");
    let output = fixture
        .command(&[
            "package",
            "validate",
            "--manifest",
            starter.join("pandora.package.json").to_str().unwrap(),
            "--artifact",
            starter.join("meta-harness.artifact").to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("tracked Meta Harness starter validation should start");

    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["package"]["id"], "example/meta-starter");
    assert_eq!(response["package"]["kind"], "meta_harness");
    assert_eq!(response["execution_boundary"], "metadata-only");
    assert_eq!(response["persisted"], false);
}

#[test]
fn tracked_gene_pack_validates_and_exposes_full_lifecycle_inspection() {
    let fixture = Fixture::new();
    fixture.setup();
    let pack = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("sdk/gene-pack");
    let genes = [
        ("static-guide", "static-guide.wasm", "example/static-guide"),
        ("bounded-read", "bounded-read.wasm", "example/bounded-read"),
        (
            "patch-proposal",
            "patch-proposal.wasm",
            "example/patch-proposal",
        ),
    ];

    for (directory, artifact, id) in genes {
        let root = pack.join("genes").join(directory);
        let manifest = root.join("pandora.package.json");
        let artifact = root.join(artifact);
        let validated = fixture
            .command(&[
                "package",
                "validate",
                "--manifest",
                manifest.to_str().unwrap(),
                "--artifact",
                artifact.to_str().unwrap(),
                "--json",
            ])
            .output()
            .expect("tracked Gene validation should start");
        assert_success(&validated);
        assert_eq!(parse_json(&validated)["execution_boundary"], "wasm");

        let admitted = fixture
            .command(&[
                "package",
                "admit",
                "--manifest",
                manifest.to_str().unwrap(),
                "--artifact",
                artifact.to_str().unwrap(),
                "--json",
            ])
            .output()
            .expect("tracked Gene admission should start");
        assert_success(&admitted);
        let admitted = parse_json(&admitted);
        assert_eq!(admitted["package"]["id"], id);
        assert_eq!(admitted["package"]["state"], "installed");
        assert_eq!(admitted["package"]["activation"]["state"], "disabled");
        assert_eq!(admitted["package"]["runtime_authority"], false);
    }

    let domain = pack.join("domain");
    let admitted_domain = fixture
        .command(&[
            "package",
            "admit",
            "--manifest",
            domain.join("pandora.package.json").to_str().unwrap(),
            "--artifact",
            domain.join("gene-pack-domain.artifact").to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("Gene pack Domain admission should start");
    assert_success(&admitted_domain);
    assert_eq!(
        parse_json(&admitted_domain)["package"]["activation"]["state"],
        "disabled"
    );

    let patch_manifest_path = pack.join("genes/patch-proposal/pandora.package.json");
    let patch_artifact_path = pack.join("genes/patch-proposal/patch-proposal.wasm");
    let mut second_manifest: Value =
        serde_json::from_slice(&fs::read(&patch_manifest_path).unwrap()).unwrap();
    second_manifest["version"] = serde_json::json!("2.0.0");
    let second_manifest_path = fixture.root.join("patch-proposal-v2.json");
    fs::write(
        &second_manifest_path,
        serde_json::to_vec_pretty(&second_manifest).unwrap(),
    )
    .unwrap();
    let admitted_second = fixture
        .command(&[
            "package",
            "admit",
            "--manifest",
            second_manifest_path.to_str().unwrap(),
            "--artifact",
            patch_artifact_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("second Gene version admission should start");
    assert_success(&admitted_second);

    let preview = fixture
        .command(&[
            "package",
            "enable",
            "example/patch-proposal",
            "1.0.0",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("Gene enable preview should start");
    assert_success(&preview);
    assert_eq!(parse_json(&preview)["ready"], true);
    assert_eq!(parse_json(&preview)["changed"], false);

    for version in ["1.0.0", "2.0.0"] {
        let enabled = fixture
            .command(&[
                "package",
                "enable",
                "example/patch-proposal",
                version,
                "--yes",
                "--json",
            ])
            .output()
            .expect("Gene enable should start");
        assert_success(&enabled);
        assert_eq!(parse_json(&enabled)["binding"]["active_version"], version);
    }

    let rollback_preview = fixture
        .command(&[
            "package",
            "rollback",
            "example/patch-proposal",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("Gene rollback preview should start");
    assert_success(&rollback_preview);
    assert_eq!(parse_json(&rollback_preview)["target_version"], "1.0.0");
    assert_eq!(parse_json(&rollback_preview)["changed"], false);

    let rolled_back = fixture
        .command(&[
            "package",
            "rollback",
            "example/patch-proposal",
            "--yes",
            "--json",
        ])
        .output()
        .expect("Gene rollback should start");
    assert_success(&rolled_back);
    assert_eq!(parse_json(&rolled_back)["active_version"], "1.0.0");

    for id in ["example/static-guide", "example/bounded-read"] {
        let enabled = fixture
            .command(&["package", "enable", id, "1.0.0", "--yes", "--json"])
            .output()
            .expect("Gene enable should start");
        assert_success(&enabled);
    }
    let enabled_domain = fixture
        .command(&[
            "package",
            "enable",
            "example/gene-pack-domain",
            "1.0.0",
            "--yes",
            "--json",
        ])
        .output()
        .expect("Gene pack Domain enable should start");
    assert_success(&enabled_domain);

    let inspected = fixture
        .command(&[
            "package",
            "inspect",
            "example/patch-proposal",
            "1.0.0",
            "--json",
        ])
        .output()
        .expect("Gene inspection should start");
    assert_success(&inspected);
    let inspected_value = parse_json(&inspected);
    let package = &inspected_value["package"];
    assert_eq!(package["gene_contract"]["execution"], "effect_request");
    assert_eq!(
        package["gene_contract"]["capabilities"],
        serde_json::json!(["filesystem.write"])
    );
    assert_eq!(package["gene_contract"]["approval_required"], true);
    assert_eq!(package["gene_contract"]["direct_executor_access"], false);
    assert_eq!(package["provenance"]["artifact_verified"], true);
    assert_eq!(package["provenance"]["publisher"], "pandora-community");
    assert_eq!(
        package["owning_domains"][0]["id"],
        "example/gene-pack-domain"
    );
    assert_eq!(package["owning_domains"][0]["version"], "1.0.0");
    assert_eq!(package["activation"]["generation"], 3);
    assert_eq!(package["runtime_authority"], false);

    let disable_preview = fixture
        .command(&[
            "package",
            "disable",
            "example/gene-pack-domain",
            "1.0.0",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("Gene pack Domain disable preview should start");
    assert_success(&disable_preview);
    assert_eq!(parse_json(&disable_preview)["ready"], true);
    let disabled_domain = fixture
        .command(&[
            "package",
            "disable",
            "example/gene-pack-domain",
            "1.0.0",
            "--yes",
            "--json",
        ])
        .output()
        .expect("Gene pack Domain disable should start");
    assert_success(&disabled_domain);
    let disabled_gene = fixture
        .command(&[
            "package",
            "disable",
            "example/patch-proposal",
            "1.0.0",
            "--yes",
            "--json",
        ])
        .output()
        .expect("Gene disable should start");
    assert_success(&disabled_gene);
    assert_eq!(parse_json(&disabled_gene)["binding"]["state"], "disabled");
}

#[test]
fn gene_pack_negative_fixtures_fail_before_activation() {
    let fixture = Fixture::new();
    fixture.setup();
    let pack = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("sdk/gene-pack");
    let artifact = pack.join("genes/static-guide/static-guide.wasm");

    for (name, expected) in [
        ("undeclared-capability.json", "Gene contract"),
        ("incompatible-runtime.json", "incompatible"),
        ("traversal-id.json", "invalid path component"),
    ] {
        let rejected = fixture
            .command(&[
                "package",
                "validate",
                "--manifest",
                pack.join("fixtures/negative").join(name).to_str().unwrap(),
                "--artifact",
                artifact.to_str().unwrap(),
                "--json",
            ])
            .output()
            .expect("negative Gene fixture validation should start");
        assert!(!rejected.status.success());
        assert!(
            parse_json(&rejected)["message"]
                .as_str()
                .unwrap()
                .to_ascii_lowercase()
                .contains(&expected.to_ascii_lowercase())
        );
    }

    let manifest = pack.join("genes/static-guide/pandora.package.json");
    let admitted = fixture
        .command(&[
            "package",
            "admit",
            "--manifest",
            manifest.to_str().unwrap(),
            "--artifact",
            artifact.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("Gene admission should start");
    assert_success(&admitted);
    let duplicate = fixture
        .command(&[
            "package",
            "admit",
            "--manifest",
            manifest.to_str().unwrap(),
            "--artifact",
            artifact.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("duplicate Gene admission should start");
    assert!(!duplicate.status.success());
    assert!(
        parse_json(&duplicate)["message"]
            .as_str()
            .unwrap()
            .contains("already installed")
    );
    let inspected = fixture
        .command(&[
            "package",
            "inspect",
            "example/static-guide",
            "1.0.0",
            "--json",
        ])
        .output()
        .expect("Gene inspection should start");
    assert_success(&inspected);
    assert_eq!(
        parse_json(&inspected)["package"]["activation"]["state"],
        "disabled"
    );
}

#[test]
fn meta_harness_scaffold_supports_exact_enable_inspect_disable_and_rollback() {
    let fixture = Fixture::new();
    fixture.setup();
    for version in ["1.0.0", "2.0.0"] {
        let directory = fixture.root.join(format!("meta-starter-{version}"));
        let output = fixture
            .command(&[
                "package",
                "scaffold",
                "meta-harness",
                "--output",
                directory.to_str().unwrap(),
                "--id",
                "example/starter-meta",
                "--version",
                version,
                "--publisher",
                "starter-test",
                "--domains",
                "coding-domain@0.1.0,research-domain@0.1.0",
                "--max-handoffs",
                "4",
                "--json",
            ])
            .env("PANDORA_GITHUB_TOKEN", "must-not-be-read")
            .output()
            .expect("Meta Harness scaffold should start");
        assert_success(&output);
        let scaffolded = parse_json(&output);
        assert_eq!(scaffolded["scaffold"]["kind"], "meta_harness");
        assert_eq!(scaffolded["scaffold"]["package"]["version"], version);
        assert_eq!(scaffolded["network_requested"], false);
        assert_eq!(scaffolded["credential_accessed"], false);
        assert_eq!(scaffolded["persisted_package"], false);
        assert_eq!(scaffolded["runtime_authority"], false);
        assert!(!String::from_utf8_lossy(&output.stdout).contains("must-not-be-read"));

        let manifest = directory.join("pandora.package.json");
        let artifact = directory.join("meta-harness.artifact");
        assert!(manifest.is_file());
        assert!(artifact.is_file());
        assert!(directory.join("README.md").is_file());
        assert!(directory.join("ARCHITECTURE.md").is_file());

        let admitted = fixture
            .command(&[
                "package",
                "admit",
                "--manifest",
                manifest.to_str().unwrap(),
                "--artifact",
                artifact.to_str().unwrap(),
                "--json",
            ])
            .output()
            .expect("Meta Harness admission should start");
        assert_success(&admitted);
        let admitted = parse_json(&admitted);
        assert_eq!(admitted["package"]["state"], "admitted");
        assert_eq!(admitted["package"]["activation"]["state"], "disabled");
        assert_eq!(admitted["package"]["runtime_authority"], false);
    }

    let preview = fixture
        .command(&[
            "package",
            "enable",
            "example/starter-meta",
            "1.0.0",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("Meta Harness enable preview should start");
    assert_success(&preview);
    let preview = parse_json(&preview);
    assert_eq!(preview["ready"], true);
    assert_eq!(preview["changed"], false);
    let dependencies = preview["dependencies"].as_array().unwrap();
    assert_eq!(dependencies.len(), 2);
    assert!(
        dependencies.iter().all(|dependency| {
            dependency["source"] == "built_in" && dependency["enabled"] == true
        })
    );

    for version in ["1.0.0", "2.0.0"] {
        let enabled = fixture
            .command(&[
                "package",
                "enable",
                "example/starter-meta",
                version,
                "--yes",
                "--json",
            ])
            .output()
            .expect("Meta Harness enable should start");
        assert_success(&enabled);
        assert_eq!(parse_json(&enabled)["binding"]["active_version"], version);
    }

    let inspected = fixture
        .command(&[
            "package",
            "inspect",
            "example/starter-meta",
            "2.0.0",
            "--json",
        ])
        .output()
        .expect("Meta Harness inspection should start");
    assert_success(&inspected);
    let inspected = parse_json(&inspected);
    assert_eq!(inspected["package"]["meta_composition"]["max_handoffs"], 4);
    assert_eq!(
        inspected["package"]["meta_composition"]["allowed_domains"],
        serde_json::json!(["coding-domain", "research-domain"])
    );
    assert_eq!(inspected["package"]["trust"]["level"], "unverified");
    assert_eq!(
        inspected["package"]["activation"]["active_version"],
        "2.0.0"
    );
    assert_eq!(
        inspected["package"]["activation"]["previous_version"],
        "1.0.0"
    );
    assert_eq!(inspected["package"]["activation"]["generation"], 2);
    assert_eq!(inspected["package"]["runtime_authority"], false);

    let rolled_back = fixture
        .command(&[
            "package",
            "rollback",
            "example/starter-meta",
            "--yes",
            "--json",
        ])
        .output()
        .expect("Meta Harness rollback should start");
    assert_success(&rolled_back);
    assert_eq!(parse_json(&rolled_back)["active_version"], "1.0.0");

    let disabled = fixture
        .command(&[
            "package",
            "disable",
            "example/starter-meta",
            "1.0.0",
            "--yes",
            "--json",
        ])
        .output()
        .expect("Meta Harness disable should start");
    assert_success(&disabled);
    assert_eq!(parse_json(&disabled)["binding"]["state"], "disabled");
}

#[test]
fn meta_harness_starter_rejects_duplicate_self_cyclic_unknown_and_over_limit_composition() {
    let fixture = Fixture::new();
    fixture.setup();
    for (name, domains, max_handoffs, expected) in [
        (
            "duplicate",
            "coding-domain@0.1.0,coding-domain@0.1.0",
            "4",
            "unique",
        ),
        (
            "self-cycle",
            "example/self-cycle@1.0.0",
            "4",
            "cannot include itself",
        ),
        (
            "over-limit",
            "coding-domain@0.1.0",
            "65",
            "between 1 and 64",
        ),
    ] {
        let directory = fixture.root.join(name);
        let output = fixture
            .command(&[
                "package",
                "scaffold",
                "meta-harness",
                "--output",
                directory.to_str().unwrap(),
                "--id",
                if name == "self-cycle" {
                    "example/self-cycle"
                } else {
                    "example/invalid-meta"
                },
                "--domains",
                domains,
                "--max-handoffs",
                max_handoffs,
                "--json",
            ])
            .output()
            .expect("invalid Meta Harness scaffold should start");
        assert!(!output.status.success());
        assert!(
            parse_json(&output)["message"]
                .as_str()
                .unwrap()
                .contains(expected)
        );
        assert!(!directory.exists());
    }

    let unknown = fixture.root.join("unknown");
    let scaffolded = fixture
        .command(&[
            "package",
            "scaffold",
            "meta-harness",
            "--output",
            unknown.to_str().unwrap(),
            "--id",
            "example/unknown-meta",
            "--domains",
            "example/missing-domain@1.0.0",
            "--json",
        ])
        .output()
        .expect("unknown Meta Harness scaffold should start");
    assert_success(&scaffolded);
    let refused = fixture
        .command(&[
            "package",
            "admit",
            "--manifest",
            unknown.join("pandora.package.json").to_str().unwrap(),
            "--artifact",
            unknown.join("meta-harness.artifact").to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("unknown Domain admission should start");
    assert!(!refused.status.success());
    assert!(
        parse_json(&refused)["message"]
            .as_str()
            .unwrap()
            .contains("required package dependency")
    );
    let listed = fixture
        .command(&["package", "list", "--json"])
        .output()
        .expect("package list should start");
    assert_success(&listed);
    assert!(
        parse_json(&listed)["packages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn domain_harness_scaffold_supports_the_full_local_lifecycle() {
    let fixture = Fixture::new();
    fixture.setup();
    let mut generated = Vec::new();
    for version in ["1.0.0", "2.0.0"] {
        let directory = fixture.root.join(format!("starter-{version}"));
        let output = fixture
            .command(&[
                "package",
                "scaffold",
                "domain-harness",
                "--output",
                directory.to_str().unwrap(),
                "--id",
                "example/starter-domain",
                "--version",
                version,
                "--publisher",
                "starter-test",
                "--gene",
                "workspace.read@0.1.0",
                "--route-hint",
                "starter domain",
                "--json",
            ])
            .env("PANDORA_GITHUB_TOKEN", "must-not-be-read")
            .output()
            .expect("Domain Harness scaffold should start");
        assert_success(&output);
        let response = parse_json(&output);
        assert_eq!(response["scaffold"]["format_version"], 1);
        assert_eq!(response["scaffold"]["package"]["version"], version);
        assert_eq!(response["network_requested"], false);
        assert_eq!(response["credential_accessed"], false);
        assert_eq!(response["persisted_package"], false);
        assert_eq!(response["runtime_authority"], false);
        assert!(!String::from_utf8_lossy(&output.stdout).contains("must-not-be-read"));

        let manifest = directory.join("pandora.package.json");
        let artifact = directory.join("domain-harness.artifact");
        assert!(manifest.is_file());
        assert!(artifact.is_file());
        assert!(directory.join("README.md").is_file());
        assert!(directory.join("ARCHITECTURE.md").is_file());

        let validated = fixture
            .command(&[
                "package",
                "validate",
                "--manifest",
                manifest.to_str().unwrap(),
                "--artifact",
                artifact.to_str().unwrap(),
                "--json",
            ])
            .output()
            .expect("starter validation should start");
        assert_success(&validated);
        let validated = parse_json(&validated);
        assert_eq!(validated["valid"], true);
        assert_eq!(validated["execution_boundary"], "metadata-only");
        assert_eq!(validated["persisted"], false);

        let admitted = fixture
            .command(&[
                "package",
                "admit",
                "--manifest",
                manifest.to_str().unwrap(),
                "--artifact",
                artifact.to_str().unwrap(),
                "--json",
            ])
            .output()
            .expect("starter admission should start");
        assert_success(&admitted);
        let admitted = parse_json(&admitted);
        assert_eq!(admitted["package"]["state"], "admitted");
        assert_eq!(admitted["package"]["activation"]["state"], "disabled");
        assert_eq!(admitted["package"]["runtime_authority"], false);
        assert_eq!(
            admitted["package"]["activation"]["runtime_authority"],
            false
        );
        generated.push((manifest, artifact));
    }

    let preview = fixture
        .command(&[
            "package",
            "enable",
            "example/starter-domain",
            "1.0.0",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("starter enable preview should start");
    assert_success(&preview);
    let preview = parse_json(&preview);
    assert_eq!(preview["ready"], true);
    assert_eq!(preview["changed"], false);
    assert_eq!(preview["dependencies"][0]["source"], "built_in");

    for version in ["1.0.0", "2.0.0"] {
        let enabled = fixture
            .command(&[
                "package",
                "enable",
                "example/starter-domain",
                version,
                "--yes",
                "--json",
            ])
            .output()
            .expect("starter enable should start");
        assert_success(&enabled);
        let enabled = parse_json(&enabled);
        assert_eq!(enabled["binding"]["active_version"], version);
        assert_eq!(enabled["binding"]["runtime_authority"], false);
    }

    let inspected = fixture
        .command(&[
            "package",
            "inspect",
            "example/starter-domain",
            "2.0.0",
            "--json",
        ])
        .output()
        .expect("starter inspection should start");
    assert_success(&inspected);
    let inspected = parse_json(&inspected);
    assert_eq!(
        inspected["package"]["activation"]["active_version"],
        "2.0.0"
    );
    assert_eq!(
        inspected["package"]["activation"]["previous_version"],
        "1.0.0"
    );

    let rollback_preview = fixture
        .command(&[
            "package",
            "rollback",
            "example/starter-domain",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("starter rollback preview should start");
    assert_success(&rollback_preview);
    assert_eq!(parse_json(&rollback_preview)["target_version"], "1.0.0");

    let rolled_back = fixture
        .command(&[
            "package",
            "rollback",
            "example/starter-domain",
            "--yes",
            "--json",
        ])
        .output()
        .expect("starter rollback should start");
    assert_success(&rolled_back);
    assert_eq!(parse_json(&rolled_back)["active_version"], "1.0.0");

    let disabled = fixture
        .command(&[
            "package",
            "disable",
            "example/starter-domain",
            "1.0.0",
            "--yes",
            "--json",
        ])
        .output()
        .expect("starter disable should start");
    assert_success(&disabled);
    assert_eq!(parse_json(&disabled)["binding"]["state"], "disabled");
    assert_eq!(generated.len(), 2);
}

#[test]
fn domain_harness_starter_validation_fails_closed() {
    let fixture = Fixture::new();
    fixture.setup();
    let directory = fixture.root.join("invalid-starter");
    let scaffolded = fixture
        .command(&[
            "package",
            "scaffold",
            "domain-harness",
            "--output",
            directory.to_str().unwrap(),
            "--id",
            "example/invalid-starter",
            "--gene",
            "workspace.read@0.1.0",
            "--route-hint",
            "starter route",
            "--json",
        ])
        .output()
        .expect("invalid starter base should be scaffolded");
    assert_success(&scaffolded);
    let manifest_path = directory.join("pandora.package.json");
    let artifact_path = directory.join("domain-harness.artifact");
    let original: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();

    let mut unsupported_capability = original.clone();
    unsupported_capability["capabilities"] = serde_json::json!(["workspace.write"]);
    let unsupported_path = directory.join("unsupported-capability.json");
    fs::write(
        &unsupported_path,
        serde_json::to_vec_pretty(&unsupported_capability).unwrap(),
    )
    .unwrap();
    let refused = fixture
        .command(&[
            "package",
            "validate",
            "--manifest",
            unsupported_path.to_str().unwrap(),
            "--artifact",
            artifact_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("unsupported capability validation should start");
    assert!(!refused.status.success());
    assert!(
        parse_json(&refused)["message"]
            .as_str()
            .unwrap()
            .contains("unknown field")
    );

    let mut duplicate_route = original;
    duplicate_route["domain_routing"]["hints"] =
        serde_json::json!(["starter route", "starter route"]);
    let duplicate_route_path = directory.join("duplicate-route.json");
    fs::write(
        &duplicate_route_path,
        serde_json::to_vec_pretty(&duplicate_route).unwrap(),
    )
    .unwrap();
    let refused = fixture
        .command(&[
            "package",
            "validate",
            "--manifest",
            duplicate_route_path.to_str().unwrap(),
            "--artifact",
            artifact_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("duplicate route validation should start");
    assert!(!refused.status.success());
    assert!(
        parse_json(&refused)["message"]
            .as_str()
            .unwrap()
            .contains("routing")
    );

    let missing_directory = fixture.root.join("missing-dependency");
    let scaffolded = fixture
        .command(&[
            "package",
            "scaffold",
            "domain-harness",
            "--output",
            missing_directory.to_str().unwrap(),
            "--id",
            "example/missing-dependency",
            "--gene",
            "example/unavailable-gene@1.0.0",
            "--json",
        ])
        .output()
        .expect("missing dependency starter should be scaffolded");
    assert_success(&scaffolded);
    let refused = fixture
        .command(&[
            "package",
            "admit",
            "--manifest",
            missing_directory
                .join("pandora.package.json")
                .to_str()
                .unwrap(),
            "--artifact",
            missing_directory
                .join("domain-harness.artifact")
                .to_str()
                .unwrap(),
            "--json",
        ])
        .output()
        .expect("missing dependency admission should start");
    assert!(!refused.status.success());
    let message = parse_json(&refused)["message"].as_str().unwrap().to_owned();
    assert!(message.contains("required package dependency"));
    assert!(message.contains("is not installed"));

    let existing = fixture
        .command(&[
            "package",
            "scaffold",
            "domain-harness",
            "--output",
            directory.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("existing scaffold path check should start");
    assert!(!existing.status.success());
    let unchanged: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    assert_eq!(unchanged["id"], "example/invalid-starter");
}

#[test]
fn admitted_wasm_gene_is_versioned_approved_and_receipted() {
    let fixture = Fixture::new();
    fixture.setup();
    let wasm = wat::parse_str(
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
    let gene = PackageManifest::new(
        "example/echo",
        "1.0.0",
        PackageKind::Gene,
        "local-publisher",
        hash_artifact(&wasm),
        Vec::new(),
        PackageCompatibility::new(concat!("pandora>=", env!("CARGO_PKG_VERSION"))).unwrap(),
        "MIT",
        TrustEvidence::unsigned(),
    )
    .unwrap();
    let domain_artifact = b"wasm domain\n";
    let domain = PackageManifest::new(
        "example/wasm-domain",
        "1.0.0",
        PackageKind::DomainHarness,
        "local-publisher",
        hash_artifact(domain_artifact),
        vec![PackageDependency::new("example/echo", "1.0.0", false).unwrap()],
        PackageCompatibility::new(concat!("pandora>=", env!("CARGO_PKG_VERSION"))).unwrap(),
        "MIT",
        TrustEvidence::unsigned(),
    )
    .unwrap();
    for (name, manifest, artifact) in [
        ("echo", &gene, wasm.as_slice()),
        ("wasm-domain", &domain, domain_artifact.as_slice()),
    ] {
        let manifest_path = fixture.root.join(format!("{name}.json"));
        let artifact_path = fixture.root.join(format!("{name}.artifact"));
        fs::write(&manifest_path, serde_json::to_vec_pretty(manifest).unwrap()).unwrap();
        fs::write(&artifact_path, artifact).unwrap();
        let output = fixture
            .command(&[
                "package",
                "admit",
                "--manifest",
                manifest_path.to_str().unwrap(),
                "--artifact",
                artifact_path.to_str().unwrap(),
                "--json",
            ])
            .output()
            .expect("package admission should start");
        assert_success(&output);
    }

    for id in ["example/echo", "example/wasm-domain"] {
        let output = fixture
            .command(&["package", "enable", id, "1.0.0", "--yes", "--json"])
            .output()
            .expect("Wasm package enable should start");
        assert_success_with_context(&output, "enable Wasm package");
    }

    let output = fixture
        .command(&["slash", "list", "--json"])
        .output()
        .expect("slash listing should start");
    assert_success_with_context(&output, "slash list for Wasm Gene");
    assert!(
        parse_json(&output)["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| {
                command["command"] == "/gene:example%2Fwasm-domain@1.0.0:example%2Fecho"
            })
    );

    let task = r#"{"value":42}"#;
    let output = fixture
        .command(&[
            "/gene:example%2Fwasm-domain@1.0.0:example%2Fecho",
            task,
            "--json",
        ])
        .output()
        .expect("Wasm Gene run should request approval");
    assert_eq!(output.status.code(), Some(40));
    let response = parse_json(&output);
    let approval_id = response["details"]["approval_id"].as_str().unwrap();
    let session_id = response["details"]["session_id"].as_str().unwrap();

    let output = fixture
        .command(&["approval", "inspect", approval_id, "--json"])
        .output()
        .expect("Wasm Gene approval should be inspectable");
    assert_success(&output);
    let response = parse_json(&output);
    let request_summary = response["approval"]["request_summary"].as_str().unwrap();
    assert!(request_summary.contains("example/wasm-domain@1.0.0"));
    assert!(request_summary.contains("example/echo@1.0.0"));
    assert!(request_summary.contains("local-publisher"));
    assert!(request_summary.contains(gene.content_hash()));
    assert!(!response.to_string().contains(task));

    let output = fixture
        .command(&["approval", "resolve", approval_id, "--allow", "--json"])
        .output()
        .expect("Wasm Gene approval should resolve");
    assert_success(&output);
    let output = fixture
        .command(&[
            "/gene:example%2Fwasm-domain@1.0.0:example%2Fecho",
            task,
            "--session",
            session_id,
            "--approval",
            approval_id,
            "--json",
        ])
        .output()
        .expect("approved Wasm Gene should run");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["status"], "completed");
    assert_eq!(response["output"], task);
    assert_eq!(response["gene_id"], "example/echo");
}

#[test]
fn approval_can_be_inspected_and_resolved_without_exposing_patch_content() {
    let fixture = Fixture::new();
    fixture.setup();
    let output = fixture
        .command(&["run", "patch:README.md:sk-live-secret", "--json"])
        .output()
        .expect("run should start");
    assert_eq!(output.status.code(), Some(40));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("sk-live-secret"));
    let response = parse_json(&output);
    let approval_id = response["details"]["approval_id"]
        .as_str()
        .expect("approval ID should be returned")
        .to_owned();

    let output = fixture
        .command(&["approval", "inspect", &approval_id, "--json"])
        .output()
        .expect("approval inspect should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["approval"]["status"], "pending");
    assert_eq!(response["approval"]["gene_id"], "patch.apply");
    assert!(!response.to_string().contains("sk-live-secret"));

    let output = fixture
        .command(&["approval", "resolve", &approval_id, "--allow", "--json"])
        .output()
        .expect("approval resolve should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["approval"]["status"], "approved");
    assert_eq!(response["approval"]["approver_id"], "local-user");

    let output = fixture
        .command(&["approval", "list", "--json"])
        .output()
        .expect("approval list should start");
    assert_success(&output);
    assert_eq!(
        parse_json(&output)["approvals"].as_array().unwrap().len(),
        1
    );
}

#[test]
fn approved_patch_resumes_once_through_the_governed_executor() {
    let fixture = Fixture::new();
    fixture.setup();
    let output = fixture
        .command(&["run", "patch:README.md:changed", "--json"])
        .output()
        .expect("run should start");
    assert_eq!(output.status.code(), Some(40));
    let approval_id = parse_json(&output)["details"]["approval_id"]
        .as_str()
        .expect("approval ID should be returned")
        .to_owned();

    let output = fixture
        .command(&["approval", "resolve", &approval_id, "--allow", "--json"])
        .output()
        .expect("approval resolution should start");
    assert_success(&output);

    let output = fixture
        .command(&[
            "run",
            "--approval",
            &approval_id,
            "patch:README.md:changed",
            "--json",
        ])
        .output()
        .expect("approved run should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["status"], "completed");
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("README.md")).unwrap(),
        "changed"
    );

    let output = fixture
        .command(&["approval", "inspect", &approval_id, "--json"])
        .output()
        .expect("approval inspection should start");
    assert_success(&output);
    assert_eq!(parse_json(&output)["approval"]["status"], "consumed");

    let output = fixture
        .command(&[
            "run",
            "--approval",
            &approval_id,
            "patch:README.md:changed-again",
            "--json",
        ])
        .output()
        .expect("replayed approval run should start");
    assert_eq!(output.status.code(), Some(40));
    assert_eq!(parse_json(&output)["code"], "approval_required");
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("README.md")).unwrap(),
        "changed"
    );
}

#[test]
fn completions_generate_a_bash_script() {
    let fixture = Fixture::new();
    let output = fixture
        .command(&["completions", "bash", "--json"])
        .output()
        .expect("completion generation should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["command"], "completions bash");
    assert!(response["script"].as_str().unwrap().contains("pandora"));
    assert!(response["script"].as_str().unwrap().contains("skill"));
}

#[test]
fn completions_include_session_inspect_for_each_shell() {
    let fixture = Fixture::new();
    let expectations = [
        (
            "powershell",
            "if ($elements.Count -gt 1 -and $elements[1] -eq 'session')",
            "'list','resume','inspect'",
        ),
        (
            "bash",
            "if [[ \"$previous\" == \"session\" ]]",
            "compgen -W 'list resume inspect'",
        ),
        (
            "zsh",
            "if [[ ${words[2]} == session ]]",
            "'2:session command:(list resume inspect)'",
        ),
        (
            "fish",
            "__fish_seen_subcommand_from session",
            "list resume inspect",
        ),
    ];
    for (shell, parent_condition, subcommands) in expectations {
        let output = fixture
            .command(&["completions", shell, "--json"])
            .output()
            .expect("completion generation should start");
        assert_success(&output);
        let response = parse_json(&output);
        let script = response["script"].as_str().unwrap();
        assert!(!script.contains("session inspect"));
        assert!(script.contains("chat"));
        assert!(script.contains("tui"));
        assert!(script.contains("package"));
        assert!(script.contains(parent_condition));
        assert!(script.contains(subcommands));
        if shell == "zsh" {
            assert!(!script.contains(
                "_arguments \\\n+    '1:command:(setup run harness session skill approval provider tool orchestration strategies completions migrate update uninstall doctor)' \\\n+    '2:session command:(list resume inspect)'"
            ));
        }
    }
}

#[test]
fn migration_converts_legacy_config_and_keeps_a_backup() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.root.join("data")).expect("legacy data should exist");
    fs::write(
        &fixture.config,
        r#"{"provider":{"url":"http://127.0.0.1:4317/v1"},"provider_model":"legacy-model","data_path":"data","workspace_path":"workspace"}"#,
    )
    .expect("legacy config should be written");

    let output = fixture
        .command(&[
            "migrate",
            "config",
            "--config",
            fixture.config.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("migration should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["command"], "migrate config");
    assert!(fixture.config.with_extension("json.bak").is_file());
    let current: Value = serde_json::from_slice(&fs::read(&fixture.config).unwrap()).unwrap();
    assert_eq!(current["format_version"], 1);
    assert_eq!(current["provider_url"], "http://127.0.0.1:4317/v1");
    assert_eq!(current["provider_model"], "legacy-model");
}

#[test]
fn migration_preserves_invalid_config() {
    let fixture = Fixture::new();
    let original = b"{not-json";
    fs::write(&fixture.config, original).expect("invalid config should be written");

    let output = fixture
        .command(&[
            "migrate",
            "config",
            "--config",
            fixture.config.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("migration should start");
    assert_eq!(output.status.code(), Some(10));
    assert_eq!(fs::read(&fixture.config).unwrap(), original);
    assert!(!fixture.config.with_extension("json.bak").exists());
}

#[test]
fn update_rejects_a_checksum_mismatch() {
    let fixture = Fixture::new();
    let artifact = fixture.root.join("pandora.bin");
    fs::write(&artifact, b"verified artifact").expect("artifact should be written");
    let output = fixture
        .command(&[
            "update",
            "--artifact",
            artifact.to_str().unwrap(),
            "--sha256",
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("update should start");
    assert_eq!(output.status.code(), Some(70));
    let response = parse_json(&output);
    assert_eq!(response["code"], "update_error");
    assert_eq!(response["details"]["reason"], "checksum_mismatch");
}

#[test]
fn update_can_rollback_the_previous_verified_artifact() {
    let fixture = Fixture::new();
    let first = fixture.root.join("first.bin");
    let second = fixture.root.join("second.bin");
    fs::write(&first, b"first").expect("first artifact should be written");
    fs::write(&second, b"second").expect("second artifact should be written");
    for (artifact, content) in [
        (&first, b"first".as_slice()),
        (&second, b"second".as_slice()),
    ] {
        let output = fixture
            .command(&[
                "update",
                "--artifact",
                artifact.to_str().unwrap(),
                "--sha256",
                hash_artifact(content).as_str(),
                "--json",
            ])
            .output()
            .expect("update should start");
        assert_success(&output);
    }
    let output = fixture
        .command(&["update", "--rollback", "--json"])
        .output()
        .expect("rollback should start");
    assert_success(&output);
    assert_eq!(
        fs::read(fixture.data.join("updates/current/pandora")).unwrap(),
        b"first"
    );
}

#[test]
fn uninstall_dry_run_preserves_user_data() {
    let fixture = Fixture::new();
    fixture.setup();
    let output = fixture
        .command(&["uninstall", "--dry-run", "--json"])
        .output()
        .expect("uninstall should start");
    assert_success(&output);
    let response = parse_json(&output);
    assert_eq!(response["dry_run"], true);
    assert!(fixture.config.is_file());
    assert!(fixture.data.is_dir());
    assert!(fixture.workspace.is_dir());
}

#[test]
fn uninstall_handles_a_config_file_stored_under_the_data_root() {
    let fixture = Fixture::new();
    let config = fixture.data.join("config.json");
    let config = config.to_str().unwrap();
    let output = fixture
        .command(&[
            "setup",
            "--config",
            config,
            "--provider-url",
            "http://127.0.0.1:4317/v1",
            "--json",
        ])
        .output()
        .expect("setup should start");
    assert_success(&output);

    let output = fixture
        .command(&["uninstall", "--config", config, "--yes", "--json"])
        .output()
        .expect("uninstall should start");
    assert_success(&output);
    assert!(!fixture.data.exists());
    assert!(fixture.workspace.is_dir());
}

#[test]
fn uninstall_refuses_a_data_root_that_contains_the_workspace() {
    let fixture = Fixture::new();
    let data_dir = fixture.root.to_str().unwrap();
    let workspace = fixture.workspace.to_str().unwrap();
    let output = fixture
        .command(&[
            "setup",
            "--data-dir",
            data_dir,
            "--workspace",
            workspace,
            "--provider-url",
            "http://127.0.0.1:4317/v1",
            "--json",
        ])
        .output()
        .expect("setup should start");
    assert_success(&output);

    let output = fixture
        .command(&[
            "uninstall",
            "--data-dir",
            data_dir,
            "--workspace",
            workspace,
            "--yes",
            "--json",
        ])
        .output()
        .expect("uninstall should start");
    assert_eq!(output.status.code(), Some(10));
    let response = parse_json(&output);
    assert_eq!(response["code"], "configuration_error");
    assert!(
        response["message"]
            .as_str()
            .unwrap_or_default()
            .contains("contains the workspace")
    );
    assert!(fixture.workspace.is_dir());
}

#[test]
fn doctor_reports_connectivity_state_without_exposing_credentials() {
    let fixture = Fixture::new();
    fixture.setup();
    let output = fixture
        .command(&["doctor", "--json"])
        .env("PANDORA_PROVIDER_API_KEY", "sk-live-secret")
        .output()
        .expect("doctor should start");
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("sk-live-secret"));
    let response = parse_json(&output);
    assert_eq!(response["healthy"], true);
    assert_eq!(response["provider"]["credential"], "available");
    assert_eq!(response["provider"]["connectivity"], "not_checked");
    assert_eq!(response["policy"]["mode"], "governed");
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_success_with_context(output: &Output, command: &str) {
    assert!(
        output.status.success(),
        "{command} failed: {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn parse_json(output: &Output) -> Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).unwrap_or_else(|error| {
        panic!(
            "expected JSON output, got {error}: {stdout}; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}
#[derive(Clone, Copy, Debug)]
struct Phase7AcceptanceProfile {
    producer_count: usize,
    warmup_job_count: usize,
    recovery_job_count: usize,
    rounds: usize,
    warmup_spread: Duration,
    recovery_spread: Duration,
    completion_timeout: Duration,
    mode: &'static str,
}

impl Phase7AcceptanceProfile {
    fn from_environment() -> Self {
        let soak = std::env::var("PANDORA_PHASE7_SOAK")
            .is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "yes"));
        if !soak {
            return Self {
                producer_count: 3,
                warmup_job_count: 6,
                recovery_job_count: 12,
                rounds: 1,
                warmup_spread: Duration::from_millis(300),
                recovery_spread: Duration::from_millis(900),
                completion_timeout: Duration::from_secs(180),
                mode: "ci",
            };
        }

        let producer_count = phase7_environment_number("PANDORA_PHASE7_SOAK_PRODUCERS", 4, 2, 8);
        let total_jobs =
            phase7_environment_number("PANDORA_PHASE7_SOAK_JOBS", 512, producer_count * 4, 4_096);
        let soak_seconds =
            phase7_environment_number("PANDORA_PHASE7_SOAK_SECONDS", 600, 60, 86_400);
        let rounds = phase7_environment_number("PANDORA_PHASE7_SOAK_ROUNDS", 1, 1, 16);
        let warmup_job_count = producer_count * 2;
        Self {
            producer_count,
            warmup_job_count,
            recovery_job_count: total_jobs - warmup_job_count,
            rounds,
            warmup_spread: Duration::from_secs(2),
            recovery_spread: Duration::from_secs(soak_seconds as u64),
            completion_timeout: Duration::from_secs(180),
            mode: "soak",
        }
    }

    fn recovery_jobs(self) -> usize {
        self.recovery_job_count * self.rounds
    }

    fn total_jobs(self) -> usize {
        self.warmup_job_count + self.recovery_jobs()
    }
}

fn phase7_environment_number(name: &str, default: usize, minimum: usize, maximum: usize) -> usize {
    let value = std::env::var(name).map_or(default, |value| {
        value
            .parse::<usize>()
            .unwrap_or_else(|_| panic!("{name} must be an integer"))
    });
    assert!(
        (minimum..=maximum).contains(&value),
        "{name} must be between {minimum} and {maximum}"
    );
    value
}

#[derive(Clone, Debug, Default)]
struct Phase7ProcessMetrics {
    samples: usize,
    first_rss_bytes: Option<u64>,
    last_rss_bytes: Option<u64>,
    peak_rss_bytes: u64,
    max_cpu_percent: f64,
    last_cpu_total_seconds: Option<f64>,
    last_cpu_sampled_at: Option<Instant>,
}

#[derive(Clone, Debug, Default)]
struct Phase7OperationalMetrics {
    state_samples: usize,
    state_sample_errors: usize,
    resource_sample_errors: usize,
    max_queue_depth: usize,
    max_running_jobs: usize,
    max_active_leases: usize,
    max_active_lease_age_seconds: u64,
    max_stale_supervisors: usize,
    final_queue_depth: usize,
    final_running_jobs: usize,
    final_active_leases: usize,
    final_non_stopped_supervisors: usize,
    processes: BTreeMap<u32, Phase7ProcessMetrics>,
}

impl Phase7OperationalMetrics {
    fn resource_samples(&self) -> usize {
        self.processes.values().map(|process| process.samples).sum()
    }

    fn peak_rss_bytes(&self) -> u64 {
        self.processes
            .values()
            .map(|process| process.peak_rss_bytes)
            .max()
            .unwrap_or(0)
    }

    fn max_cpu_percent(&self) -> f64 {
        self.processes
            .values()
            .map(|process| process.max_cpu_percent)
            .fold(0.0, f64::max)
    }

    fn max_memory_growth_bytes(&self) -> i64 {
        self.processes
            .values()
            .filter_map(|process| {
                Some(
                    i64::try_from(process.last_rss_bytes?).unwrap_or(i64::MAX)
                        - i64::try_from(process.first_rss_bytes?).unwrap_or(i64::MAX),
                )
            })
            .max()
            .unwrap_or(0)
    }

    fn as_json(&self) -> Value {
        let processes = self
            .processes
            .iter()
            .map(|(process_id, process)| {
                let memory_growth_bytes = match (process.first_rss_bytes, process.last_rss_bytes) {
                    (Some(first), Some(last)) => (i128::from(last) - i128::from(first))
                        .clamp(i128::from(i64::MIN), i128::from(i64::MAX))
                        as i64,
                    _ => 0,
                };
                serde_json::json!({
                    "process_id": process_id,
                    "samples": process.samples,
                    "first_rss_bytes": process.first_rss_bytes,
                    "last_rss_bytes": process.last_rss_bytes,
                    "peak_rss_bytes": process.peak_rss_bytes,
                    "memory_growth_bytes": memory_growth_bytes,
                    "max_cpu_percent": process.max_cpu_percent,
                })
            })
            .collect::<Vec<_>>();
        serde_json::json!({
            "state_samples": self.state_samples,
            "state_sample_errors": self.state_sample_errors,
            "resource_samples": self.resource_samples(),
            "resource_sample_errors": self.resource_sample_errors,
            "max_queue_depth": self.max_queue_depth,
            "max_running_jobs": self.max_running_jobs,
            "max_active_leases": self.max_active_leases,
            "max_active_lease_age_seconds": self.max_active_lease_age_seconds,
            "max_stale_supervisors": self.max_stale_supervisors,
            "final_queue_depth": self.final_queue_depth,
            "final_running_jobs": self.final_running_jobs,
            "final_active_leases": self.final_active_leases,
            "final_non_stopped_supervisors": self.final_non_stopped_supervisors,
            "peak_rss_bytes": self.peak_rss_bytes(),
            "max_memory_growth_bytes": self.max_memory_growth_bytes(),
            "max_cpu_percent": self.max_cpu_percent(),
            "processes": processes,
        })
    }
}

struct Phase7SoakMonitor {
    data: PathBuf,
    stop: Arc<AtomicBool>,
    worker_process_id: Arc<AtomicU32>,
    metrics: Arc<Mutex<Phase7OperationalMetrics>>,
    monitor: Option<thread::JoinHandle<()>>,
}

impl Phase7SoakMonitor {
    fn start(data: PathBuf) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_process_id = Arc::new(AtomicU32::new(0));
        let metrics = Arc::new(Mutex::new(Phase7OperationalMetrics::default()));
        let monitor_data = data.clone();
        let monitor_stop = Arc::clone(&stop);
        let monitor_process_id = Arc::clone(&worker_process_id);
        let monitor_metrics = Arc::clone(&metrics);
        let monitor = thread::spawn(move || {
            let jobs = match JobStore::open(monitor_data.join("jobs.sqlite3")) {
                Ok(jobs) => jobs,
                Err(_) => {
                    monitor_metrics.lock().unwrap().state_sample_errors += 1;
                    return;
                }
            };
            let fleet = match FleetEngine::open(monitor_data.join("fleet.sqlite3")) {
                Ok(fleet) => fleet,
                Err(_) => {
                    monitor_metrics.lock().unwrap().state_sample_errors += 1;
                    return;
                }
            };
            let mut sample_index = 0_usize;
            while !monitor_stop.load(Ordering::Acquire) {
                phase7_collect_operational_sample(
                    &jobs,
                    &fleet,
                    monitor_process_id.load(Ordering::Acquire),
                    phase7_current_seconds(),
                    sample_index.is_multiple_of(6),
                    &monitor_metrics,
                );
                sample_index = sample_index.saturating_add(1);
                for _ in 0..50 {
                    if monitor_stop.load(Ordering::Acquire) {
                        return;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
            }
        });
        Self {
            data,
            stop,
            worker_process_id,
            metrics,
            monitor: Some(monitor),
        }
    }

    fn set_worker_process_id(&self, process_id: u32) {
        self.worker_process_id.store(process_id, Ordering::Release);
        self.sample_now_at(phase7_current_seconds(), true);
    }

    fn clear_worker_process_id(&self) {
        self.worker_process_id.store(0, Ordering::Release);
        self.sample_now_at(phase7_current_seconds(), false);
    }

    fn sample_now_at(&self, now: u64, collect_resource: bool) {
        let jobs = JobStore::open(self.data.join("jobs.sqlite3"))
            .expect("Phase 7 soak monitor should open the job store");
        let fleet = FleetEngine::open(self.data.join("fleet.sqlite3"))
            .expect("Phase 7 soak monitor should open Fleet");
        phase7_collect_operational_sample(
            &jobs,
            &fleet,
            self.worker_process_id.load(Ordering::Acquire),
            now,
            collect_resource,
            &self.metrics,
        );
    }

    fn finish(mut self) -> Phase7OperationalMetrics {
        self.stop_and_join();
        self.metrics.lock().unwrap().clone()
    }

    fn stop_and_join(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(monitor) = self.monitor.take() {
            monitor
                .join()
                .expect("Phase 7 soak monitor should stop cleanly");
        }
    }
}

impl Drop for Phase7SoakMonitor {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

fn phase7_current_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after the Unix epoch")
        .as_secs()
}

fn phase7_collect_operational_sample(
    jobs: &JobStore,
    fleet: &FleetEngine,
    worker_process_id: u32,
    now: u64,
    collect_resource: bool,
    metrics: &Arc<Mutex<Phase7OperationalMetrics>>,
) {
    let principal = PrincipalId::new("local-user").unwrap();
    let tenant = TenantId::new("local-tenant").unwrap();
    let workspace = WorkspaceId::new("local-workspace").unwrap();
    let state = jobs
        .list(&principal, &tenant, &workspace)
        .map_err(|error| error.to_string())
        .and_then(|jobs| {
            let leases = fleet.list_leases().map_err(|error| error.to_string())?;
            let supervisors = fleet
                .list_supervisors()
                .map_err(|error| error.to_string())?;
            let queue_depth = jobs
                .iter()
                .filter(|job| job.status() == JobStatus::Queued)
                .count();
            let running_jobs = jobs
                .iter()
                .filter(|job| job.status() == JobStatus::Running)
                .count();
            let active_leases = leases
                .iter()
                .filter(|lease| lease.state().as_str() == "active")
                .collect::<Vec<_>>();
            let max_active_lease_age_seconds = active_leases
                .iter()
                .map(|lease| now.saturating_sub(lease.issued_at()))
                .max()
                .unwrap_or(0);
            let stale_supervisors = supervisors
                .iter()
                .filter(|supervisor| {
                    supervisor.state().as_str() == "running"
                        && now.saturating_sub(supervisor.updated_at()) > 30
                })
                .count();
            let non_stopped_supervisors = supervisors
                .iter()
                .filter(|supervisor| supervisor.state().as_str() != "stopped")
                .count();
            Ok((
                queue_depth,
                running_jobs,
                active_leases.len(),
                max_active_lease_age_seconds,
                stale_supervisors,
                non_stopped_supervisors,
            ))
        });

    let process_sample = if collect_resource && worker_process_id != 0 {
        phase7_process_resource_sample(worker_process_id)
    } else {
        None
    };
    let mut metrics = metrics.lock().unwrap();
    match state {
        Ok((
            queue_depth,
            running_jobs,
            active_leases,
            max_active_lease_age_seconds,
            stale_supervisors,
            non_stopped_supervisors,
        )) => {
            metrics.state_samples = metrics.state_samples.saturating_add(1);
            metrics.max_queue_depth = metrics.max_queue_depth.max(queue_depth);
            metrics.max_running_jobs = metrics.max_running_jobs.max(running_jobs);
            metrics.max_active_leases = metrics.max_active_leases.max(active_leases);
            metrics.max_active_lease_age_seconds = metrics
                .max_active_lease_age_seconds
                .max(max_active_lease_age_seconds);
            metrics.max_stale_supervisors = metrics.max_stale_supervisors.max(stale_supervisors);
            metrics.final_queue_depth = queue_depth;
            metrics.final_running_jobs = running_jobs;
            metrics.final_active_leases = active_leases;
            metrics.final_non_stopped_supervisors = non_stopped_supervisors;
        }
        Err(_) => {
            metrics.state_sample_errors = metrics.state_sample_errors.saturating_add(1);
        }
    }
    if collect_resource && worker_process_id != 0 {
        if let Some(sample) = process_sample {
            let process = metrics.processes.entry(worker_process_id).or_default();
            process.samples = process.samples.saturating_add(1);
            process.first_rss_bytes.get_or_insert(sample.rss_bytes);
            process.last_rss_bytes = Some(sample.rss_bytes);
            process.peak_rss_bytes = process.peak_rss_bytes.max(sample.rss_bytes);
            if sample.cpu_is_cumulative {
                let sampled_at = Instant::now();
                if let (Some(previous_cpu), Some(previous_at)) =
                    (process.last_cpu_total_seconds, process.last_cpu_sampled_at)
                {
                    let elapsed = sampled_at.duration_since(previous_at).as_secs_f64();
                    if elapsed > 0.0 {
                        process.max_cpu_percent = process
                            .max_cpu_percent
                            .max((sample.cpu_value - previous_cpu).max(0.0) / elapsed * 100.0);
                    }
                }
                process.last_cpu_total_seconds = Some(sample.cpu_value);
                process.last_cpu_sampled_at = Some(sampled_at);
            } else {
                process.max_cpu_percent = process.max_cpu_percent.max(sample.cpu_value);
            }
        } else {
            metrics.resource_sample_errors = metrics.resource_sample_errors.saturating_add(1);
        }
    }
}

struct Phase7ProcessResourceSample {
    rss_bytes: u64,
    cpu_value: f64,
    cpu_is_cumulative: bool,
}

#[cfg(unix)]
fn phase7_process_resource_sample(process_id: u32) -> Option<Phase7ProcessResourceSample> {
    let output = Command::new("ps")
        .args(["-p", &process_id.to_string(), "-o", "rss=", "-o", "%cpu="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let output = String::from_utf8(output.stdout).ok()?;
    let mut fields = output.split_whitespace();
    let rss_kib = fields.next()?.parse::<u64>().ok()?;
    let cpu_percent = fields.next()?.parse::<f64>().ok()?;
    Some(Phase7ProcessResourceSample {
        rss_bytes: rss_kib.saturating_mul(1_024),
        cpu_value: cpu_percent,
        cpu_is_cumulative: false,
    })
}

#[cfg(windows)]
fn phase7_process_resource_sample(process_id: u32) -> Option<Phase7ProcessResourceSample> {
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$p = Get-Process -Id ([int]$env:PANDORA_SOAK_SAMPLE_PID) -ErrorAction Stop; [pscustomobject]@{rss=[uint64]$p.WorkingSet64;cpu=[double]$p.CPU} | ConvertTo-Json -Compress",
        ])
        .env("PANDORA_SOAK_SAMPLE_PID", process_id.to_string())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let output = serde_json::from_slice::<Value>(&output.stdout).ok()?;
    let rss_bytes = output["rss"].as_u64()?;
    let cpu_total_seconds = output["cpu"].as_f64()?;
    Some(Phase7ProcessResourceSample {
        rss_bytes,
        cpu_value: cpu_total_seconds,
        cpu_is_cumulative: true,
    })
}

fn submit_phase7_jobs(
    fixture: &Fixture,
    producer_count: usize,
    job_count: usize,
    spread: Duration,
    soak_monitor: Option<&Phase7SoakMonitor>,
) -> Vec<String> {
    assert!(producer_count >= 2);
    assert!(job_count >= producer_count);
    let barrier = Arc::new(Barrier::new(producer_count));
    thread::scope(|scope| {
        let mut producers = Vec::with_capacity(producer_count);
        for producer_index in 0..producer_count {
            let barrier = Arc::clone(&barrier);
            producers.push(scope.spawn(move || {
                let producer_jobs = job_count / producer_count
                    + usize::from(producer_index < job_count % producer_count);
                let interval = if producer_jobs > 1 {
                    spread / u32::try_from(producer_jobs - 1).unwrap()
                } else {
                    Duration::ZERO
                };
                let mut job_ids = Vec::with_capacity(producer_jobs);
                barrier.wait();
                for ordinal in 0..producer_jobs {
                    if ordinal > 0 {
                        thread::sleep(interval);
                    }
                    let submitted = fixture
                        .command(&["job", "submit", "--", "read:README.md", "--json"])
                        .output()
                        .expect("independent Phase 7 producer should start");
                    assert_success_with_context(&submitted, "Phase 7 producer submission");
                    if (ordinal == 0 || ordinal.is_multiple_of(32))
                        && let Some(monitor) = soak_monitor
                    {
                        monitor.sample_now_at(phase7_current_seconds(), false);
                    }
                    job_ids.push(
                        parse_json(&submitted)["job_id"]
                            .as_str()
                            .expect("producer submission should return a job ID")
                            .to_owned(),
                    );
                }
                job_ids
            }));
        }
        producers
            .into_iter()
            .flat_map(|producer| producer.join().expect("Phase 7 producer should finish"))
            .collect()
    })
}

fn wait_for_phase7_jobs(fixture: &Fixture, expected_job_ids: &BTreeSet<String>, timeout: Duration) {
    let store = JobStore::open(fixture.data.join("jobs.sqlite3")).unwrap();
    let principal = PrincipalId::new("local-user").unwrap();
    let tenant = TenantId::new("local-tenant").unwrap();
    let workspace = WorkspaceId::new("local-workspace").unwrap();
    let deadline = Instant::now() + timeout;
    loop {
        let jobs = store.list(&principal, &tenant, &workspace).unwrap();
        let matching = jobs
            .iter()
            .filter(|job| expected_job_ids.contains(job.id().as_str()))
            .collect::<Vec<_>>();
        if matching.len() == expected_job_ids.len()
            && matching
                .iter()
                .all(|job| job.status().as_str() == "completed")
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Phase 7 queue did not reach durable completion: expected={} observed={} completed={}",
            expected_job_ids.len(),
            matching.len(),
            matching
                .iter()
                .filter(|job| job.status().as_str() == "completed")
                .count()
        );
        thread::sleep(Duration::from_millis(50));
    }
}

fn wait_for_phase7_supervisor(
    fleet: &FleetEngine,
    process_id: u32,
    generation: u64,
    timeout: Duration,
) -> pandora_runtime::FleetSupervisor {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(supervisor) = fleet
            .list_supervisors()
            .unwrap()
            .into_iter()
            .find(|supervisor| {
                supervisor.node_id() == "job-worker"
                    && supervisor.state().as_str() == "running"
                    && supervisor.process_id() == Some(process_id)
                    && supervisor.generation() == generation
            })
        {
            return supervisor;
        }
        assert!(
            Instant::now() < deadline,
            "Phase 7 worker PID {process_id} generation {generation} did not publish liveness"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

#[derive(Debug, Eq, PartialEq)]
struct Phase7TerminalEvidence {
    job_results: BTreeMap<String, String>,
    session_ids: BTreeSet<String>,
    execution_ids: BTreeSet<String>,
    receipt_ids: BTreeSet<String>,
    worker_ids: BTreeSet<String>,
}

fn phase7_terminal_evidence(
    fixture: &Fixture,
    expected_job_ids: &BTreeSet<String>,
    expected_worker_count: usize,
) -> Phase7TerminalEvidence {
    let principal = PrincipalId::new("local-user").unwrap();
    let tenant = TenantId::new("local-tenant").unwrap();
    let workspace = WorkspaceId::new("local-workspace").unwrap();
    let jobs = JobStore::open(fixture.data.join("jobs.sqlite3"))
        .unwrap()
        .list(&principal, &tenant, &workspace)
        .unwrap();
    assert_eq!(jobs.len(), expected_job_ids.len());
    let sessions = SessionStore::open(fixture.data.join("sessions.sqlite3")).unwrap();
    let mut evidence = Phase7TerminalEvidence {
        job_results: BTreeMap::new(),
        session_ids: BTreeSet::new(),
        execution_ids: BTreeSet::new(),
        receipt_ids: BTreeSet::new(),
        worker_ids: BTreeSet::new(),
    };
    for job in jobs {
        assert!(expected_job_ids.contains(job.id().as_str()));
        assert_eq!(job.status().as_str(), "completed");
        assert!(job.finished_at().is_some());
        evidence.worker_ids.insert(
            job.worker_id()
                .expect("completed job should retain its worker")
                .as_str()
                .to_owned(),
        );
        let result = job
            .result()
            .expect("completed job should retain its result");
        assert_eq!(result["command"], "run");
        assert_eq!(result["status"], "completed");
        let session_id = result["session_id"].as_str().unwrap().to_owned();
        let execution_id = result["execution_id"].as_str().unwrap().to_owned();
        assert!(evidence.session_ids.insert(session_id.clone()));
        assert!(
            evidence
                .execution_ids
                .insert(format!("{session_id}:{execution_id}"))
        );
        let snapshot = sessions
            .resume(
                &SessionId::new(session_id).unwrap(),
                &principal,
                &tenant,
                &workspace,
            )
            .expect("completed job session should be durable");
        let effect_receipts = snapshot
            .events()
            .iter()
            .filter(|event| event.event_type() == EventType::EffectCompleted)
            .map(|event| {
                event
                    .context()
                    .receipt_id()
                    .expect("completed effect event should link its receipt")
                    .as_str()
                    .to_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(effect_receipts.len(), 1);
        assert!(evidence.receipt_ids.insert(effect_receipts[0].clone()));
        assert_eq!(snapshot.evaluations().len(), 1);
        assert_eq!(snapshot.rollouts().len(), 1);
        evidence.job_results.insert(
            job.id().as_str().to_owned(),
            serde_json::to_string(result).unwrap(),
        );
    }
    assert_eq!(evidence.job_results.len(), expected_job_ids.len());
    assert_eq!(evidence.session_ids.len(), expected_job_ids.len());
    assert_eq!(evidence.execution_ids.len(), expected_job_ids.len());
    assert_eq!(evidence.receipt_ids.len(), expected_job_ids.len());
    assert_eq!(evidence.worker_ids.len(), expected_worker_count);
    evidence
}

fn assert_phase7_fresh_process_terminal_evidence(
    fixture: &Fixture,
    evidence: &Phase7TerminalEvidence,
) {
    for job_id in evidence.job_results.keys() {
        let inspected = fixture
            .command(&["job", "inspect", job_id, "--json"])
            .output()
            .expect("fresh Phase 7 job inspection CLI should start");
        assert_success_with_context(&inspected, "fresh Phase 7 job inspection");
        let inspected = parse_json(&inspected);
        assert_eq!(inspected["status"], "completed");
        assert_eq!(
            serde_json::to_string(&inspected["result"]).unwrap(),
            evidence.job_results[job_id]
        );
        let session_id = inspected["result"]["session_id"].as_str().unwrap();
        let session = fixture
            .command(&["session", "inspect", session_id, "--json"])
            .output()
            .expect("fresh Phase 7 session inspection CLI should start");
        assert_success_with_context(&session, "fresh Phase 7 session inspection");
        let session = parse_json(&session);
        assert_eq!(session["last_event_type"], "effect_completed");
        assert_eq!(session["evaluations"]["count"], 1);
    }
}

fn assert_phase7_fresh_supervisor_snapshot(
    fixture: &Fixture,
    expected_state: &str,
    expected_process_id: u32,
    expected_generation: u64,
) {
    let output = fixture
        .command(&["fleet", "supervisor", "list", "--json"])
        .output()
        .expect("fresh Phase 7 supervisor inspection CLI should start");
    assert_success_with_context(&output, "fresh Phase 7 supervisor inspection");
    let inspection = parse_json(&output);
    let supervisors = inspection["supervisors"]
        .as_array()
        .expect("supervisor inspection should return an array");
    let supervisor = supervisors
        .iter()
        .find(|supervisor| supervisor["node_id"] == "job-worker")
        .expect("job worker supervisor should be inspectable");
    assert_eq!(supervisor["state"], expected_state);
    assert_eq!(supervisor["process_id"], expected_process_id);
    assert_eq!(supervisor["generation"], expected_generation);
}

fn assert_phase7_fresh_process_leases_released(fixture: &Fixture) {
    let output = fixture
        .command(&["fleet", "list", "--json"])
        .output()
        .expect("fresh Phase 7 Fleet inspection CLI should start");
    assert_success_with_context(&output, "fresh Phase 7 Fleet inspection");
    let inspection = parse_json(&output);
    let leases = inspection["leases"]
        .as_array()
        .expect("Fleet inspection should return leases");
    assert!(
        leases.iter().all(|lease| lease["state"] != "active"),
        "every process lease must be released: {leases:?}"
    );
}

#[test]
fn phase7_worker_operations_recover_without_replaying_durable_effects() {
    let profile = Phase7AcceptanceProfile::from_environment();
    let soak_started = Instant::now();
    let soak_evidence_path =
        std::env::var_os("PANDORA_PHASE7_SOAK_EVIDENCE_PATH").map(PathBuf::from);
    assert!(
        soak_evidence_path.is_none() || profile.mode == "soak",
        "Phase 7 retained evidence requires the soak profile"
    );
    eprintln!(
        "Phase 7 worker-operations mode={} producers={} rounds={} jobs={} recovery_spread={:?}",
        profile.mode,
        profile.producer_count,
        profile.rounds,
        profile.total_jobs(),
        profile.recovery_spread
    );

    let fixture = Fixture::new();
    let exact_commit = fixture.initialize_git_workspace();
    fixture.setup();
    let fleet = FleetEngine::open(fixture.data.join("fleet.sqlite3")).unwrap();
    let soak_monitor = soak_evidence_path
        .as_ref()
        .map(|_| Phase7SoakMonitor::start(fixture.data.clone()));

    let mut first_worker = fixture
        .command(&["job", "work", "--daemon", "--json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("first Phase 7 daemon should start independently");
    let first_process_id = first_worker.id();
    let first_running =
        wait_for_phase7_supervisor(&fleet, first_process_id, 1, Duration::from_secs(15));
    if let Some(monitor) = &soak_monitor {
        monitor.set_worker_process_id(first_process_id);
    }

    let warmup_job_ids = submit_phase7_jobs(
        &fixture,
        profile.producer_count,
        profile.warmup_job_count,
        profile.warmup_spread,
        soak_monitor.as_ref(),
    );
    let warmup_job_ids = warmup_job_ids.into_iter().collect::<BTreeSet<_>>();
    assert_eq!(warmup_job_ids.len(), profile.warmup_job_count);
    wait_for_phase7_jobs(&fixture, &warmup_job_ids, profile.completion_timeout);

    first_worker
        .kill()
        .expect("first Phase 7 daemon should accept a forced stop");
    let killed = first_worker
        .wait_with_output()
        .expect("forced Phase 7 daemon should terminate");
    assert!(!killed.status.success());

    let stale = fleet
        .list_supervisors()
        .unwrap()
        .into_iter()
        .find(|supervisor| supervisor.node_id() == "job-worker")
        .expect("forced-stop supervisor should remain durable");
    assert_eq!(stale.state().as_str(), "running");
    assert_eq!(stale.generation(), first_running.generation());
    assert_eq!(stale.process_id(), Some(first_process_id));
    assert_phase7_fresh_supervisor_snapshot(
        &fixture,
        "running",
        first_process_id,
        first_running.generation(),
    );
    let active_leases = fleet
        .list_leases()
        .unwrap()
        .into_iter()
        .filter(|lease| lease.node_id() == "job-worker" && lease.state().as_str() == "active")
        .collect::<Vec<_>>();
    assert_eq!(active_leases.len(), 1);
    let recovery_now = stale
        .updated_at()
        .saturating_add(31)
        .max(active_leases[0].expires_at().saturating_add(1));
    if let Some(monitor) = &soak_monitor {
        monitor.sample_now_at(recovery_now, false);
    }
    let recovery_now = recovery_now.to_string();
    let reconciled = fixture
        .command(&[
            "fleet",
            "supervisor",
            "reconcile",
            "job-worker",
            "--now",
            &recovery_now,
            "--stale-after",
            "30",
            "--json",
        ])
        .output()
        .expect("fresh reconciliation CLI should start");
    assert_success_with_context(&reconciled, "Phase 7 stale-supervisor reconciliation");
    let reconciled = parse_json(&reconciled);
    assert_eq!(reconciled["supervisor"]["state"], "recovering");
    assert_eq!(
        reconciled["supervisor"]["generation"],
        first_running.generation()
    );
    assert_eq!(reconciled["supervisor"]["process_id"], first_process_id);
    assert!(
        fleet
            .list_leases()
            .unwrap()
            .iter()
            .all(|lease| lease.state().as_str() != "active")
    );

    let second_worker = fixture
        .command(&["job", "work", "--daemon", "--json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("recovery Phase 7 daemon should start independently");
    let second_process_id = second_worker.id();
    assert_ne!(second_process_id, first_process_id);
    let second_running = wait_for_phase7_supervisor(
        &fleet,
        second_process_id,
        first_running.generation() + 1,
        Duration::from_secs(15),
    );
    if let Some(monitor) = &soak_monitor {
        monitor.set_worker_process_id(second_process_id);
    }
    assert_eq!(second_running.process_id(), Some(second_process_id));
    assert_phase7_fresh_supervisor_snapshot(
        &fixture,
        "running",
        second_process_id,
        second_running.generation(),
    );

    let mut recovery_job_ids = BTreeSet::new();
    for round in 0..profile.rounds {
        eprintln!(
            "Phase 7 recovery submission round {}/{}",
            round + 1,
            profile.rounds
        );
        let round_job_ids = submit_phase7_jobs(
            &fixture,
            profile.producer_count,
            profile.recovery_job_count,
            profile.recovery_spread,
            soak_monitor.as_ref(),
        )
        .into_iter()
        .collect::<BTreeSet<_>>();
        assert_eq!(round_job_ids.len(), profile.recovery_job_count);
        recovery_job_ids.extend(round_job_ids);
    }
    assert_eq!(recovery_job_ids.len(), profile.recovery_jobs());
    let mut all_job_ids = warmup_job_ids.clone();
    all_job_ids.extend(recovery_job_ids.iter().cloned());
    assert_eq!(all_job_ids.len(), profile.total_jobs());
    wait_for_phase7_jobs(&fixture, &all_job_ids, profile.completion_timeout);

    let drained = fixture
        .command(&["fleet", "supervisor", "drain", "job-worker", "--json"])
        .output()
        .expect("fresh drain CLI should start");
    assert_success_with_context(&drained, "Phase 7 recovery worker drain");
    assert_eq!(parse_json(&drained)["supervisor"]["state"], "draining");
    let second_output = wait_for_child(
        second_worker,
        profile.completion_timeout,
        "recovery Phase 7 daemon",
    );
    assert_success_with_context(&second_output, "Phase 7 recovery daemon");
    let second_output = parse_json(&second_output);
    assert_eq!(second_output["stop_reason"], "external_drain");
    assert_eq!(second_output["processed_count"], profile.recovery_jobs());
    let processed_after_recovery = second_output["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|job| job["job_id"].as_str().unwrap().to_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(processed_after_recovery, recovery_job_ids);
    if let Some(monitor) = &soak_monitor {
        monitor.clear_worker_process_id();
    }

    let before_idle_restart = phase7_terminal_evidence(&fixture, &all_job_ids, 2);
    assert_phase7_fresh_process_terminal_evidence(&fixture, &before_idle_restart);
    let idle_restart = fixture
        .command(&["job", "work", "--max-jobs", "64", "--json"])
        .output()
        .expect("post-recovery idle worker should start as a fresh CLI process");
    assert_success_with_context(&idle_restart, "post-recovery idle worker");
    let idle_restart = parse_json(&idle_restart);
    assert_eq!(idle_restart["processed_count"], 0);
    assert_eq!(idle_restart["stop_reason"], "queue_empty");
    let after_idle_restart = phase7_terminal_evidence(&fixture, &all_job_ids, 2);
    assert_eq!(after_idle_restart, before_idle_restart);
    assert_phase7_fresh_process_terminal_evidence(&fixture, &after_idle_restart);
    assert_phase7_fresh_process_leases_released(&fixture);

    let run_id = "phase7-multi-repository-recovery";
    let plan = OrchestrationPlan::new(
        PlanId::new("phase7-worker-operations-plan").unwrap(),
        vec![
            RoleAssignment::new(
                RoleId::new("planner").unwrap(),
                OrchestrationRole::Planner,
                HarnessId::new("coding-domain").unwrap(),
                Vec::new(),
            )
            .unwrap(),
            RoleAssignment::new(
                RoleId::new("maker").unwrap(),
                OrchestrationRole::Maker,
                HarnessId::new("design-domain").unwrap(),
                vec![RoleId::new("planner").unwrap()],
            )
            .unwrap(),
        ],
        2,
        1,
        vec![Handoff::new(
            RoleId::new("planner").unwrap(),
            RoleId::new("maker").unwrap(),
            Some(HarnessId::new("coordination-meta").unwrap()),
        )],
    )
    .unwrap();
    let governed = GovernedOrchestrationPlan::new(
        plan,
        MetaComposition::new(
            vec![
                HarnessId::new("coding-domain").unwrap(),
                HarnessId::new("design-domain").unwrap(),
            ],
            1,
        )
        .unwrap(),
        vec![
            RepositoryBinding::new(
                RepositoryId::new("api").unwrap(),
                WorkspaceId::new("local-workspace").unwrap(),
                exact_commit.clone(),
            )
            .unwrap(),
            RepositoryBinding::new(
                RepositoryId::new("desktop").unwrap(),
                WorkspaceId::new("workspace-desktop").unwrap(),
                "desktop-commit",
            )
            .unwrap(),
        ],
        vec![
            RoleRepositoryBinding::new(
                RoleId::new("planner").unwrap(),
                RepositoryId::new("api").unwrap(),
            ),
            RoleRepositoryBinding::new(
                RoleId::new("maker").unwrap(),
                RepositoryId::new("desktop").unwrap(),
            ),
        ],
    )
    .unwrap();
    let plan_path = fixture.root.join("phase7-plan.json");
    fs::write(&plan_path, serde_json::to_vec(&governed).unwrap()).unwrap();
    let submitted = fixture
        .command(&[
            "orchestration",
            "submit",
            "--input",
            plan_path.to_str().unwrap(),
            "--id",
            run_id,
            "--json",
        ])
        .output()
        .expect("fresh orchestration submit CLI should start");
    assert_success_with_context(&submitted, "Phase 7 orchestration submit");

    let claimed = fixture
        .command(&[
            "orchestration",
            "claim",
            "--worker",
            "role-worker-a",
            "--json",
        ])
        .output()
        .expect("first orchestration worker process should claim");
    assert_success_with_context(&claimed, "Phase 7 orchestration claim");
    assert_eq!(parse_json(&claimed)["assignments"][0]["role_id"], "planner");

    let governed_effect_receipt = ReceiptId::new(
        before_idle_restart
            .receipt_ids
            .iter()
            .next()
            .expect("governed queue work should persist an effect receipt")
            .clone(),
    )
    .unwrap();
    let planner_receipt = OrchestrationRoleReceipt::new(
        ReceiptId::new("phase7-planner-receipt").unwrap(),
        OrchestrationRunId::new(run_id).unwrap(),
        RoleId::new("planner").unwrap(),
        RepositoryId::new("api").unwrap(),
        WorkspaceId::new("local-workspace").unwrap(),
        exact_commit.clone(),
        vec![governed_effect_receipt.clone()],
        None,
    )
    .unwrap();
    let planner_receipt_path = fixture.root.join("phase7-planner-receipt.json");
    fs::write(
        &planner_receipt_path,
        serde_json::to_vec(&planner_receipt).unwrap(),
    )
    .unwrap();
    let completed_planner = fixture
        .command(&[
            "orchestration",
            "complete",
            run_id,
            "--worker",
            "role-worker-a",
            "--role",
            "planner",
            "--receipt",
            planner_receipt_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("restarted planner CLI should record its governed receipt");
    assert_success_with_context(&completed_planner, "Phase 7 planner completion");
    let completed_planner = parse_json(&completed_planner);
    assert_eq!(completed_planner["run"]["status"], "running");
    assert_eq!(completed_planner["assignments"][0]["role_id"], "maker");

    let duplicate_planner = fixture
        .command(&[
            "orchestration",
            "complete",
            run_id,
            "--worker",
            "role-worker-a",
            "--role",
            "planner",
            "--receipt",
            planner_receipt_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("duplicate planner completion CLI should start");
    assert_eq!(duplicate_planner.status.code(), Some(50));
    assert!(
        parse_json(&duplicate_planner)["message"]
            .as_str()
            .unwrap()
            .contains("duplicated")
    );

    let maker_receipt = OrchestrationRoleReceipt::new(
        ReceiptId::new("phase7-maker-receipt").unwrap(),
        OrchestrationRunId::new(run_id).unwrap(),
        RoleId::new("maker").unwrap(),
        RepositoryId::new("desktop").unwrap(),
        WorkspaceId::new("workspace-desktop").unwrap(),
        "desktop-commit",
        Vec::new(),
        Some(RequestDigest::new("phase7-maker-evidence").unwrap()),
    )
    .unwrap();
    let maker_receipt_path = fixture.root.join("phase7-maker-receipt.json");
    fs::write(
        &maker_receipt_path,
        serde_json::to_vec(&maker_receipt).unwrap(),
    )
    .unwrap();
    let wrong_worker = fixture
        .command(&[
            "orchestration",
            "complete",
            run_id,
            "--worker",
            "role-worker-b",
            "--role",
            "maker",
            "--receipt",
            maker_receipt_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("independently restarted wrong worker CLI should start");
    assert_eq!(wrong_worker.status.code(), Some(50));
    assert!(
        parse_json(&wrong_worker)["message"]
            .as_str()
            .unwrap()
            .contains("owned by another worker")
    );

    let wrong_repository_receipt = OrchestrationRoleReceipt::new(
        ReceiptId::new("phase7-maker-wrong-repository").unwrap(),
        OrchestrationRunId::new(run_id).unwrap(),
        RoleId::new("maker").unwrap(),
        RepositoryId::new("api").unwrap(),
        WorkspaceId::new("local-workspace").unwrap(),
        exact_commit,
        Vec::new(),
        Some(RequestDigest::new("phase7-maker-wrong-repository-evidence").unwrap()),
    )
    .unwrap();
    let wrong_repository_path = fixture.root.join("phase7-maker-wrong-repository.json");
    fs::write(
        &wrong_repository_path,
        serde_json::to_vec(&wrong_repository_receipt).unwrap(),
    )
    .unwrap();
    let partial_failure = fixture
        .command(&[
            "orchestration",
            "complete",
            run_id,
            "--worker",
            "role-worker-a",
            "--role",
            "maker",
            "--receipt",
            wrong_repository_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("independently restarted maker failure CLI should start");
    assert_eq!(partial_failure.status.code(), Some(2));
    assert_eq!(parse_json(&partial_failure)["code"], "usage_error");

    let interrupted = fixture
        .command(&[
            "orchestration",
            "mark-interrupted",
            run_id,
            "--reason",
            "desktop role failed after the API receipt became durable",
            "--yes",
            "--json",
        ])
        .output()
        .expect("fresh interruption CLI should start");
    assert_success_with_context(&interrupted, "Phase 7 orchestration interruption");
    assert_eq!(parse_json(&interrupted)["status"], "interrupted");

    let inspected = fixture
        .command(&["orchestration", "inspect", run_id, "--json"])
        .output()
        .expect("fresh partial-failure inspection CLI should start");
    assert_success_with_context(&inspected, "Phase 7 orchestration inspection");
    let inspected = parse_json(&inspected);
    assert_eq!(inspected["status"], "interrupted");
    assert_eq!(inspected["completed_roles"][0], "planner");
    assert_eq!(inspected["active_roles"][0], "maker");
    assert_eq!(inspected["role_receipts"].as_array().unwrap().len(), 1);
    assert_eq!(
        inspected["role_receipts"][0]["governed_effect_receipts"][0],
        governed_effect_receipt.as_str()
    );

    let replacement_claim = fixture
        .command(&[
            "orchestration",
            "claim",
            "--worker",
            "role-worker-b",
            "--json",
        ])
        .output()
        .expect("replacement orchestration worker CLI should start");
    assert_success_with_context(&replacement_claim, "replacement orchestration claim");
    let replacement_claim = parse_json(&replacement_claim);
    assert!(replacement_claim["run"].is_null());
    assert_eq!(replacement_claim["status"], "idle");

    let blocked_resume = fixture
        .command(&["orchestration", "resume", run_id, "--json"])
        .output()
        .expect("fresh blocked-resume CLI should start");
    assert_eq!(blocked_resume.status.code(), Some(50));
    assert!(
        parse_json(&blocked_resume)["message"]
            .as_str()
            .unwrap()
            .contains("receipt reconciliation")
    );

    let blocked_resume_after_worker_restart = fixture
        .command(&["orchestration", "resume", run_id, "--json"])
        .output()
        .expect("post-restart blocked-resume CLI should start");
    assert_eq!(blocked_resume_after_worker_restart.status.code(), Some(50));
    assert!(
        parse_json(&blocked_resume_after_worker_restart)["message"]
            .as_str()
            .unwrap()
            .contains("receipt reconciliation")
    );

    let final_inspection = fixture
        .command(&["orchestration", "inspect", run_id, "--json"])
        .output()
        .expect("final fresh orchestration inspection CLI should start");
    assert_success_with_context(&final_inspection, "final orchestration inspection");
    let final_inspection = parse_json(&final_inspection);
    assert_eq!(final_inspection["status"], "interrupted");
    assert_eq!(
        final_inspection["role_receipts"],
        inspected["role_receipts"]
    );
    assert_eq!(
        phase7_terminal_evidence(&fixture, &all_job_ids, 2),
        before_idle_restart
    );

    if let (Some(path), Some(monitor)) = (soak_evidence_path, soak_monitor) {
        let metrics = monitor.finish();
        let memory_growth_within_limit = metrics.max_memory_growth_bytes() <= 256 * 1_024 * 1_024;
        let resource_samples_present =
            metrics.resource_samples() >= 2 && metrics.processes.len() == 2;
        let gates = serde_json::json!({
            "all_jobs_completed": before_idle_restart.job_results.len() == profile.total_jobs(),
            "exactly_once": before_idle_restart.job_results.len() == profile.total_jobs()
                && before_idle_restart.session_ids.len() == profile.total_jobs()
                && before_idle_restart.execution_ids.len() == profile.total_jobs()
                && before_idle_restart.receipt_ids.len() == profile.total_jobs(),
            "no_active_leases": metrics.final_active_leases == 0,
            "no_running_supervisors": metrics.final_non_stopped_supervisors == 0,
            "resource_samples_present": resource_samples_present,
            "stale_supervisor_observed": metrics.max_stale_supervisors >= 1,
            "state_sampling_reliable": metrics.state_samples > 0 && metrics.state_sample_errors == 0,
            "memory_growth_within_limit": memory_growth_within_limit,
            "clean_restart_and_shutdown": second_running.generation() == 2,
            "partial_multi_repository_failure_preserved": final_inspection["status"] == "interrupted",
        });
        let passed = gates
            .as_object()
            .expect("Phase 7 evidence gates should be an object")
            .values()
            .all(|value| value.as_bool() == Some(true));
        let evidence = serde_json::json!({
            "schema_version": 1,
            "status": if passed { "passed" } else { "failed" },
            "mode": profile.mode,
            "elapsed_seconds": soak_started.elapsed().as_secs_f64(),
            "configuration": {
                "producers": profile.producer_count,
                "rounds": profile.rounds,
                "warmup_jobs": profile.warmup_job_count,
                "recovery_jobs_per_round": profile.recovery_job_count,
                "recovery_spread_seconds": profile.recovery_spread.as_secs(),
            },
            "outcomes": {
                "total_jobs": profile.total_jobs(),
                "completed_jobs": before_idle_restart.job_results.len(),
                "unique_sessions": before_idle_restart.session_ids.len(),
                "unique_executions": before_idle_restart.execution_ids.len(),
                "unique_effect_receipts": before_idle_restart.receipt_ids.len(),
                "worker_processes": before_idle_restart.worker_ids.len(),
                "supervisor_generations": second_running.generation(),
            },
            "metrics": metrics.as_json(),
            "gates": gates,
        });
        assert!(passed, "Phase 7 retained evidence gates failed: {evidence}");
        let mut serialized = serde_json::to_vec_pretty(&evidence).unwrap();
        serialized.push(b'\n');
        fs::write(&path, serialized).expect("Phase 7 retained evidence should be written");
        eprintln!("Phase 7 retained evidence written to {}", path.display());
    }

    eprintln!(
        "Phase 7 worker-operations evidence passed: jobs={} workers=2 generations={} receipts={} partial_run={run_id}",
        profile.total_jobs(),
        second_running.generation(),
        before_idle_restart.receipt_ids.len(),
    );
}
