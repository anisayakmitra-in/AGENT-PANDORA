use pandora_runtime::sessions::SessionStore;
use pandora_runtime::{
    ApprovalStore, ExecutionController, RuntimeService, RuntimeServiceScope,
    executors::WorkspaceRoot,
};
use pandora_types::{
    Capability, GeneId, Operation, PolicyContext, PrincipalId, ServiceEventPageRequest,
    ServiceRequest, ServiceResponse, ServiceRunRequest, ServiceRunResumeRequest, Session,
    SessionId, TenantId, Timestamp, WorkspaceId,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE_DIRECTORY: AtomicU64 = AtomicU64::new(1);

#[test]
fn service_session_list_never_crosses_its_configured_scope() {
    let fixture = Fixture::new();
    let store = SessionStore::open(fixture.root.join("sessions.sqlite3")).unwrap();
    store
        .create(&session("session-visible", "principal-a"))
        .unwrap();
    store
        .create(&session("session-hidden", "principal-b"))
        .unwrap();
    let service = RuntimeService::new(
        ExecutionController::new(WorkspaceRoot::new(&fixture.root).unwrap()),
        store,
        ApprovalStore::open(fixture.root.join("sessions.sqlite3")).unwrap(),
        scope("principal-a"),
    );

    let response = service
        .handle(
            &ServiceRequest::session_list(32).unwrap(),
            Timestamp::from_unix_seconds(10),
        )
        .unwrap();

    let ServiceResponse::SessionList { sessions, .. } = response else {
        panic!("session list must return session summaries");
    };
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id().as_str(), "session-visible");
}

#[test]
fn service_run_persists_events_for_its_returned_session() {
    let fixture = Fixture::new();
    let store = SessionStore::open(fixture.root.join("sessions.sqlite3")).unwrap();
    let service = RuntimeService::new(
        ExecutionController::new(WorkspaceRoot::new(&fixture.root).unwrap()),
        store,
        ApprovalStore::open(fixture.root.join("sessions.sqlite3")).unwrap(),
        scope("principal-a"),
    );
    let request = ServiceRequest::run(
        ServiceRunRequest::new("guide", None, Some(GeneId::new("athena.guide").unwrap())).unwrap(),
    );

    let response = service
        .handle(&request, Timestamp::from_unix_seconds(10))
        .unwrap();
    let ServiceResponse::Run { run, .. } = response else {
        panic!("run must return a terminal result");
    };
    assert_eq!(run.status(), "completed");

    let events = service
        .handle(
            &ServiceRequest::session_events(
                ServiceEventPageRequest::new(run.session_id().as_str(), None, 32).unwrap(),
            ),
            Timestamp::from_unix_seconds(10),
        )
        .unwrap();
    let ServiceResponse::SessionEvents { events, .. } = events else {
        panic!("session events must return an event page");
    };
    assert!(!events.events().is_empty());
}

#[test]
fn service_rejects_deserialized_invalid_requests_before_session_lookup() {
    let fixture = Fixture::new();
    let service = RuntimeService::new(
        ExecutionController::new(WorkspaceRoot::new(&fixture.root).unwrap()),
        SessionStore::open(fixture.root.join("sessions.sqlite3")).unwrap(),
        ApprovalStore::open(fixture.root.join("sessions.sqlite3")).unwrap(),
        scope("principal-a"),
    );
    let request: ServiceRequest = serde_json::from_value(serde_json::json!({
        "kind": "session_events",
        "protocol_version": 1,
        "request": {
            "session_id": "missing-session",
            "after_sequence": null,
            "limit": 0
        }
    }))
    .unwrap();

    let error = service
        .handle(&request, Timestamp::from_unix_seconds(10))
        .unwrap_err();

    assert_eq!(error.code(), "invalid_service_request");
}

#[test]
fn service_creates_resolves_and_consumes_an_exact_run_approval() {
    let fixture = Fixture::new();
    let database = fixture.root.join("sessions.sqlite3");
    let policy = PolicyContext::new(
        1,
        [Capability::FilesystemRead, Capability::FilesystemWrite],
        [Operation::Write],
    );
    let service = RuntimeService::new(
        ExecutionController::with_policy(WorkspaceRoot::new(&fixture.root).unwrap(), policy),
        SessionStore::open(&database).unwrap(),
        ApprovalStore::open(&database).unwrap(),
        scope("principal-a"),
    );
    let run_request = ServiceRunRequest::new("patch:README.md:approved\n", None, None).unwrap();

    let first = service
        .handle(
            &ServiceRequest::run(run_request.clone()),
            Timestamp::from_unix_seconds(10),
        )
        .unwrap();
    let ServiceResponse::Run { run, .. } = first else {
        panic!("run must return a governed result");
    };
    assert_eq!(run.status(), "approval_required");
    let approval = run.approval().expect("approval metadata must be returned");
    assert_eq!(approval.status(), "pending");
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("README.md")).unwrap(),
        "fixture\n"
    );

    let resolved = service
        .handle(
            &ServiceRequest::approval_resolve(approval.approval_id(), true).unwrap(),
            Timestamp::from_unix_seconds(11),
        )
        .unwrap();
    let ServiceResponse::ApprovalResolve { approval, .. } = resolved else {
        panic!("approval resolution must return its record");
    };
    assert_eq!(approval.status(), "approved");

    let resumed = service
        .handle(
            &ServiceRequest::run_resume(
                ServiceRunResumeRequest::new(approval.approval_id(), run_request).unwrap(),
            ),
            Timestamp::from_unix_seconds(12),
        )
        .unwrap();
    let ServiceResponse::Run { run, .. } = resumed else {
        panic!("resumed run must return a governed result");
    };
    assert_eq!(run.status(), "completed");
    assert_eq!(run.session_id(), approval.session_id());
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("README.md")).unwrap(),
        "approved\n"
    );

    let inspected = service
        .handle(
            &ServiceRequest::approval_inspect(approval.approval_id()).unwrap(),
            Timestamp::from_unix_seconds(13),
        )
        .unwrap();
    let ServiceResponse::ApprovalInspect { approval, .. } = inspected else {
        panic!("approval inspection must return its record");
    };
    assert_eq!(approval.status(), "consumed");
}

fn scope(principal: &str) -> RuntimeServiceScope {
    RuntimeServiceScope::new(
        PrincipalId::new(principal).unwrap(),
        TenantId::new("tenant-a").unwrap(),
        WorkspaceId::new("workspace-a").unwrap(),
    )
}

fn session(id: &str, principal: &str) -> Session {
    Session::new(
        SessionId::new(id).unwrap(),
        PrincipalId::new(principal).unwrap(),
        TenantId::new("tenant-a").unwrap(),
        WorkspaceId::new("workspace-a").unwrap(),
        Timestamp::from_unix_seconds(1),
    )
}

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = unique_directory("pandora-service-facade-test");
        std::fs::write(root.join("README.md"), "fixture\n").unwrap();
        Self { root }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn unique_directory(prefix: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = NEXT_FIXTURE_DIRECTORY.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "{prefix}-{}-{timestamp}-{sequence}",
        std::process::id()
    ));
    std::fs::create_dir(&path).unwrap();
    path
}
