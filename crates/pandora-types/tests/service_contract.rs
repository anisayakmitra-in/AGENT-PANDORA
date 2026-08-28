use pandora_types::{
    ExecutionId, GeneId, HarnessId, LOCAL_SERVICE_PROTOCOL_VERSION, ServiceEventPageRequest,
    ServiceRequest, ServiceRunRequest, ServiceRunResult, ServiceRunResumeRequest, SessionId,
};

#[test]
fn service_requests_reject_blank_tasks_and_out_of_range_event_pages() {
    assert!(ServiceRunRequest::new(" ", None, None).is_err());
    assert!(ServiceEventPageRequest::new("session-1", None, 257).is_err());
}

#[test]
fn service_request_serialization_keeps_protocol_version_one() {
    let request = ServiceRequest::health();

    assert_eq!(request.protocol_version(), LOCAL_SERVICE_PROTOCOL_VERSION);
    assert_eq!(
        serde_json::to_value(request).unwrap()["protocol_version"],
        LOCAL_SERVICE_PROTOCOL_VERSION
    );
}

#[test]
fn service_run_result_identifies_its_persisted_session() {
    let result = ServiceRunResult::new(
        SessionId::new("session-1").unwrap(),
        ExecutionId::new("execution-1").unwrap(),
        Some(HarnessId::new("coding").unwrap()),
        Some(GeneId::new("athena.guide").unwrap()),
        "completed",
        "guidance",
        0,
        1,
    );

    assert_eq!(result.session_id().as_str(), "session-1");
}

#[test]
fn deserialized_service_request_validates_nested_page_limits() {
    let request: ServiceRequest = serde_json::from_value(serde_json::json!({
        "kind": "session_events",
        "protocol_version": LOCAL_SERVICE_PROTOCOL_VERSION,
        "request": {
            "session_id": "session-1",
            "after_sequence": null,
            "limit": 0
        }
    }))
    .unwrap();

    assert!(request.validate().is_err());
}

#[test]
fn approval_and_resume_requests_reject_blank_approval_identifiers() {
    let run = ServiceRunRequest::new("guide", None, None).unwrap();

    assert!(ServiceRequest::approval_inspect(" ").is_err());
    assert!(ServiceRequest::approval_resolve("", true).is_err());
    assert!(ServiceRequest::approval_list(0).is_err());
    assert!(ServiceRunResumeRequest::new(" ", run).is_err());
}
