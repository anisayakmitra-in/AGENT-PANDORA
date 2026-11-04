#![forbid(unsafe_code)]

use pandora_service::websocket::{
    BoundedFrameQueue, JsonRpcRequest, JsonRpcResponse, Opcode, PubSubBroker, WebSocketError,
    WebSocketFrame, compute_websocket_accept, create_session_token, decode_frame, encode_frame,
    hmac_sha256, sha1_digest, verify_session_token,
};

#[test]
fn test_rfc4231_hmac_sha256_vector() {
    // RFC 4231 Test Case 1:
    // Key: 20 bytes of 0x0b
    // Data: "Hi There"
    // HMAC-SHA256: b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7
    let key = [0x0b; 20];
    let data = b"Hi There";
    let tag = hmac_sha256(&key, data);
    assert_eq!(
        format!("{:02x?}", tag)
            .replace(", ", "")
            .replace(['[', ']'], ""),
        "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
    );
}

#[test]
fn test_sha1_rfc3174_standard_vectors() {
    // Vector 1: "abc"
    // Expected: a9993e364706816aba3e25717850c26c9cd0d89d
    let d1 = sha1_digest(b"abc");
    assert_eq!(
        format!("{:02x?}", d1)
            .replace(", ", "")
            .replace(['[', ']'], ""),
        "a9993e364706816aba3e25717850c26c9cd0d89d"
    );

    // Vector 2: "" (empty)
    // Expected: da39a3ee5e6b4b0d3255bfef95601890afd80709
    let d0 = sha1_digest(b"");
    assert_eq!(
        format!("{:02x?}", d0)
            .replace(", ", "")
            .replace(['[', ']'], ""),
        "da39a3ee5e6b4b0d3255bfef95601890afd80709"
    );

    // Vector 3: "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
    // Expected: 84983e441c3bd26ebaae4aa1f95129e5e54670f1
    let d3 = sha1_digest(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq");
    assert_eq!(
        format!("{:02x?}", d3)
            .replace(", ", "")
            .replace(['[', ']'], ""),
        "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
    );
}

#[test]
fn test_rfc6455_handshake_accept_vector() {
    // Official test vector from RFC 6455 Section 1.3:
    // Key:    "dGhlIHNhbXBsZSBub25jZQ=="
    // Accept: "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
    let key = "dGhlIHNhbXBsZSBub25jZQ==";
    let accept = compute_websocket_accept(key);
    assert_eq!(accept, "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
}

#[test]
fn test_frame_codec_text_and_binary_roundtrip() {
    // 1. Text frame unmasked
    let text_frame = WebSocketFrame::text("Hello, AGENT-PANDORA RFC 6455 Gateway!");
    let wire = encode_frame(&text_frame);
    let decoded = decode_frame(&wire).expect("decoding should succeed");
    let (frame, consumed) = decoded.expect("frame should be present");
    assert_eq!(consumed, wire.len());
    assert_eq!(frame.opcode, Opcode::Text);
    assert_eq!(
        String::from_utf8(frame.payload).unwrap(),
        "Hello, AGENT-PANDORA RFC 6455 Gateway!"
    );

    // 2. Binary frame masked (client to server)
    let binary_data = vec![0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE, 0xBA, 0xBE];
    let binary_frame =
        WebSocketFrame::binary(binary_data.clone()).with_mask([0x12, 0x34, 0x56, 0x78]);
    let wire_masked = encode_frame(&binary_frame);
    let decoded_bin = decode_frame(&wire_masked).expect("decoding should succeed");
    let (frame_bin, consumed_bin) = decoded_bin.expect("frame should be present");
    assert_eq!(consumed_bin, wire_masked.len());
    assert_eq!(frame_bin.opcode, Opcode::Binary);
    assert_eq!(frame_bin.payload, binary_data);
}

#[test]
fn test_frame_codec_medium_and_large_payloads() {
    // Medium payload (126 <= len <= 65535, 2-byte length header)
    let med_payload = vec![0x42; 1000];
    let med_frame = WebSocketFrame::binary(med_payload.clone());
    let wire_med = encode_frame(&med_frame);
    assert_eq!(wire_med[1] & 0x7F, 126);
    let (decoded_med, len_med) = decode_frame(&wire_med).unwrap().unwrap();
    assert_eq!(len_med, wire_med.len());
    assert_eq!(decoded_med.payload.len(), 1000);
    assert_eq!(decoded_med.payload, med_payload);

    // Large payload (> 65535, 8-byte length header)
    let large_payload = vec![0x99; 70_000];
    let large_frame =
        WebSocketFrame::binary(large_payload.clone()).with_mask([0xAA, 0xBB, 0xCC, 0xDD]);
    let wire_large = encode_frame(&large_frame);
    assert_eq!(wire_large[1] & 0x7F, 127);
    let (decoded_large, len_large) = decode_frame(&wire_large).unwrap().unwrap();
    assert_eq!(len_large, wire_large.len());
    assert_eq!(decoded_large.payload.len(), 70_000);
    assert_eq!(decoded_large.payload, large_payload);
}

#[test]
fn test_control_frames_ping_pong_close() {
    let ping = WebSocketFrame::ping(b"heartbeat-seq-1".to_vec());
    let wire_ping = encode_frame(&ping);
    let (d_ping, _) = decode_frame(&wire_ping).unwrap().unwrap();
    assert_eq!(d_ping.opcode, Opcode::Ping);
    assert_eq!(d_ping.payload, b"heartbeat-seq-1");

    let pong = WebSocketFrame::pong(b"heartbeat-seq-1".to_vec());
    let wire_pong = encode_frame(&pong);
    let (d_pong, _) = decode_frame(&wire_pong).unwrap().unwrap();
    assert_eq!(d_pong.opcode, Opcode::Pong);
    assert_eq!(d_pong.payload, b"heartbeat-seq-1");

    let close = WebSocketFrame::close(1000, "Normal Closure");
    let wire_close = encode_frame(&close);
    let (d_close, _) = decode_frame(&wire_close).unwrap().unwrap();
    assert_eq!(d_close.opcode, Opcode::Close);
    let code = u16::from_be_bytes([d_close.payload[0], d_close.payload[1]]);
    assert_eq!(code, 1000);
    assert_eq!(
        String::from_utf8(d_close.payload[2..].to_vec()).unwrap(),
        "Normal Closure"
    );
}

#[test]
fn test_bounded_frame_queue_backpressure() {
    let capacity = 5;
    let mut q = BoundedFrameQueue::new(capacity);
    assert_eq!(q.capacity(), capacity);
    assert!(q.is_empty());

    for i in 0..5 {
        let f = WebSocketFrame::text(format!("msg-{i}"));
        assert!(q.enqueue(f).is_ok());
    }
    assert_eq!(q.len(), 5);

    // Overflow attempt must return QueueFull(5)
    let overflow_frame = WebSocketFrame::text("overflow");
    let err = q.enqueue(overflow_frame);
    assert_eq!(err, Err(WebSocketError::QueueFull(5)));

    // FIFO draining
    for i in 0..5 {
        let frame = q.dequeue().expect("item should exist");
        assert_eq!(
            String::from_utf8(frame.payload).unwrap(),
            format!("msg-{i}")
        );
    }
    assert!(q.is_empty());
}

#[test]
fn test_session_token_authentication_and_expiry() {
    let secret = b"super-secret-pandora-signing-key-32b";
    let sub = "agent-executor-node-01";
    let now = 1_700_000_000u64;
    let valid_exp = now + 3600;

    let token = create_session_token(secret, sub, valid_exp);
    let verified_sub = verify_session_token(secret, &token, now).expect("token should be valid");
    assert_eq!(verified_sub, sub);

    // Expired token check
    let expired_time = valid_exp + 10;
    let exp_err = verify_session_token(secret, &token, expired_time);
    assert!(matches!(
        exp_err,
        Err(WebSocketError::AuthenticationFailed(_))
    ));

    // Tampered token check
    let bad_token = format!("{}X", token);
    let bad_err = verify_session_token(secret, &bad_token, now);
    assert!(matches!(
        bad_err,
        Err(WebSocketError::AuthenticationFailed(_))
    ));

    // Wrong secret check
    let wrong_secret = b"different-secret-key-00000000000000";
    let wrong_err = verify_session_token(wrong_secret, &token, now);
    assert!(matches!(
        wrong_err,
        Err(WebSocketError::AuthenticationFailed(_))
    ));
}

#[test]
fn test_pubsub_broker_subscription_multiplexing() {
    let mut broker = PubSubBroker::new();
    broker.subscribe("client-1", "logs");
    broker.subscribe("client-2", "logs");
    broker.subscribe("client-2", "swarm_events");
    broker.subscribe("client-3", "diagnostics");

    let log_subs = broker.get_subscribers("logs");
    assert_eq!(log_subs.len(), 2);
    assert!(log_subs.contains(&"client-1".to_string()));
    assert!(log_subs.contains(&"client-2".to_string()));

    let swarm_subs = broker.get_subscribers("swarm_events");
    assert_eq!(swarm_subs, vec!["client-2".to_string()]);

    broker.unsubscribe("client-1", "logs");
    let log_subs_after = broker.get_subscribers("logs");
    assert_eq!(log_subs_after, vec!["client-2".to_string()]);

    broker.remove_client("client-2");
    assert!(broker.get_subscribers("logs").is_empty());
    assert!(broker.get_subscribers("swarm_events").is_empty());
}

#[test]
fn test_jsonrpc_message_serialization() {
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(serde_json::json!(42)),
        method: "pandora/vfs/search".to_string(),
        params: Some(serde_json::json!({"query": "kernel eBPF", "limit": 5})),
    };
    let json_str = serde_json::to_string(&req).unwrap();
    let parsed: JsonRpcRequest = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed, req);

    let resp =
        JsonRpcResponse::success(Some(serde_json::json!(42)), serde_json::json!({"hits": 3}));
    let resp_str = serde_json::to_string(&resp).unwrap();
    let parsed_resp: JsonRpcResponse = serde_json::from_str(&resp_str).unwrap();
    assert_eq!(parsed_resp, resp);
}
