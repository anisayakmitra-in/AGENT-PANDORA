use pandora_runtime::sessions::SessionStore;
use pandora_types::{
    EvaluationKind, EvaluationReceipt, EvaluationResult, EvaluationStatus, EventContext, EventId,
    EventPayload, EventType, ExecutionId, PrincipalId, Session, SessionId, TenantId, Timestamp,
    WorkspaceId,
};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn event_pages_are_scoped_and_return_a_resumable_cursor() {
    let fixture = Fixture::new();
    let store = SessionStore::open(fixture.root.join("sessions.sqlite3")).unwrap();
    let session = session("session-1", "principal-a");
    store.create(&session).unwrap();
    let events = ["event-1", "event-2", "event-3"]
        .into_iter()
        .map(|event_id| event(&session, event_id))
        .collect::<Vec<_>>();
    store
        .append_execution(
            session.id(),
            session.principal_id(),
            session.tenant_id(),
            session.workspace_id(),
            &events,
            &evaluation(&session),
            Timestamp::from_unix_seconds(2),
        )
        .unwrap();

    let first = store
        .event_page(
            session.id(),
            session.principal_id(),
            session.tenant_id(),
            session.workspace_id(),
            None,
            2,
        )
        .unwrap();
    assert_eq!(first.events().len(), 2);
    assert_eq!(first.next_sequence(), Some(2));

    let second = store
        .event_page(
            session.id(),
            session.principal_id(),
            session.tenant_id(),
            session.workspace_id(),
            first.next_sequence(),
            2,
        )
        .unwrap();
    assert_eq!(second.events().len(), 1);
    assert_eq!(second.next_sequence(), None);
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

fn event(session: &Session, event_id: &str) -> pandora_types::RuntimeEvent {
    pandora_types::RuntimeEvent::new(
        EventId::new(event_id).unwrap(),
        EventType::SessionStarted,
        EventContext::new(session.tenant_id().clone(), session.workspace_id().clone())
            .with_session(session.id().clone())
            .with_execution(ExecutionId::new("execution-1").unwrap()),
        EventPayload::Empty,
    )
}

fn evaluation(session: &Session) -> EvaluationReceipt {
    EvaluationReceipt::new(
        session.id().clone(),
        ExecutionId::new("execution-1").unwrap(),
        Timestamp::from_unix_seconds(2),
        vec![
            EvaluationResult::new(
                EvaluationKind::Trajectory,
                EvaluationStatus::Passed,
                100,
                "fixture trajectory",
                false,
            )
            .unwrap(),
        ],
    )
    .unwrap()
}

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "pandora-session-event-page-{}-{timestamp}",
            std::process::id()
        ));
        std::fs::create_dir(&root).unwrap();
        Self { root }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
