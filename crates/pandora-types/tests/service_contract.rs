use pandora_types::{
    LOCAL_SERVICE_PROTOCOL_VERSION, ServiceEventPageRequest, ServiceRequest, ServiceRunRequest,
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
