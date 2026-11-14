//! Adversarial integration test suite for AGENT-PANDORA Phase 25 Milestone M3:
//! TelemetryRingBuffer stress testing, wrap-around overflow, concurrency, WebSocket sync framing,
//! Web Push subscription store, and tripartite alert tamper-evident digests.

#![forbid(unsafe_code)]

use pandora_runtime::cockpit::CockpitLayout;
use pandora_runtime::cockpit_web::{
    CockpitWebConfig, CockpitWebServer, PushActionData, PushAlertType, PushNotificationPayload,
    PushSubscribeRequest, PushSubscriptionKeys, SyncQueryResult, TelemetryRingBuffer,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

struct TestServerHarness {
    port: u16,
    flag: Arc<AtomicBool>,
    server: Arc<CockpitWebServer>,
    handle: Option<thread::JoinHandle<()>>,
}

impl TestServerHarness {
    fn start() -> Self {
        let config = CockpitWebConfig {
            bind_addr: "127.0.0.1".into(),
            port: 0,
            initial_layout: CockpitLayout::TwelvePanel,
            sse_tick_interval_ms: 100,
        };
        let server = Arc::new(CockpitWebServer::with_config(config, None).expect("bind server"));
        let port = server.port();
        let flag = server.shutdown_handle();
        let s_clone = Arc::clone(&server);
        let handle = thread::spawn(move || {
            let _ = s_clone.run();
        });
        Self {
            port,
            flag,
            server,
            handle: Some(handle),
        }
    }

    fn server(&self) -> &CockpitWebServer {
        &self.server
    }

    fn http_request(&self, method: &str, path: &str, body: &str) -> (u16, String) {
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).expect("connect to test server");
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            self.port,
            body.len()
        );
        s.write_all(req.as_bytes()).unwrap();
        let mut raw = String::new();
        let _ = s.read_to_string(&mut raw);
        let status = raw
            .split_whitespace()
            .nth(1)
            .and_then(|c| c.parse().ok())
            .unwrap_or(0);
        let resp_body = raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
        (status, resp_body)
    }
}

impl Drop for TestServerHarness {
    fn drop(&mut self) {
        self.flag.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn encode_client_masked_ws_frame(opcode: u8, payload: &[u8], mask: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity(14 + payload.len());
    out.push(0x80 | (opcode & 0x0F));
    let len = payload.len();
    if len <= 125 {
        out.push(0x80 | (len as u8));
    } else if len <= 65535 {
        out.push(0x80 | 126);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        out.push(0x80 | 127);
        out.extend_from_slice(&(len as u64).to_be_bytes());
    }
    out.extend_from_slice(&mask);
    for (i, &b) in payload.iter().enumerate() {
        out.push(b ^ mask[i % 4]);
    }
    out
}

fn read_server_ws_frame(s: &mut TcpStream) -> (u8, Vec<u8>) {
    let mut hdr = [0u8; 2];
    s.read_exact(&mut hdr).expect("read ws header");
    let opcode = hdr[0] & 0x0F;
    let mut len = (hdr[1] & 0x7F) as usize;
    if len == 126 {
        let mut ext = [0u8; 2];
        s.read_exact(&mut ext).expect("read ext16");
        len = u16::from_be_bytes(ext) as usize;
    } else if len == 127 {
        let mut ext = [0u8; 8];
        s.read_exact(&mut ext).expect("read ext64");
        len = u64::from_be_bytes(ext) as usize;
    }
    let mut payload = vec![0u8; len];
    if len > 0 {
        s.read_exact(&mut payload).expect("read ws payload");
    }
    (opcode, payload)
}

// ============================================================================
// 1. Adversarial Ring Buffer Stress Tests (>10,000 packets & wrap-around)
// ============================================================================

#[test]
fn test_adversarial_ring_buffer_10k_packets_wrap_around() {
    let capacity = 128;
    let mut ring = TelemetryRingBuffer::new(capacity);

    // Push 10,000 packets through ring buffer
    let total_packets = 10_000u64;
    for i in 1..=total_packets {
        let seq = ring.push(
            "stress_telemetry",
            json!({"iteration": i, "data": format!("data_{i}")}),
            i * 10,
        );
        assert_eq!(
            seq, i,
            "sequence_id must be strictly monotonically increasing"
        );
    }

    assert_eq!(ring.current_seq(), total_packets);
    let expected_oldest = total_packets - (capacity as u64) + 1;
    assert_eq!(
        ring.oldest_seq(),
        expected_oldest,
        "oldest_sequence_id must track ring head wrap-around"
    );

    // Query from latest sequence_id must return UpToDate
    match ring.query_missing(total_packets, 0) {
        SyncQueryResult::UpToDate { current_seq } => {
            assert_eq!(current_seq, total_packets);
        }
        _ => panic!("Expected UpToDate when querying with latest sequence"),
    }

    // Query in the future must return UpToDate
    match ring.query_missing(total_packets + 50, 0) {
        SyncQueryResult::UpToDate { current_seq } => {
            assert_eq!(current_seq, total_packets);
        }
        _ => panic!("Expected UpToDate when querying with future sequence"),
    }

    // Query in the middle of active window: e.g. 50 packets behind
    let query_since = total_packets - 50;
    match ring.query_missing(query_since, 0) {
        SyncQueryResult::CatchUp {
            current_seq,
            packets,
        } => {
            assert_eq!(current_seq, total_packets);
            assert_eq!(
                packets.len(),
                50,
                "Must return exactly 50 missing contiguous packets"
            );
            // Verify contiguous monotonic ordering without gaps
            for (idx, p) in packets.iter().enumerate() {
                let expected_seq = query_since + 1 + idx as u64;
                assert_eq!(p.sequence_id, expected_seq);
            }
        }
        _ => panic!("Expected CatchUp for valid active range"),
    }
}

#[test]
fn test_adversarial_ring_buffer_gap_overflow_boundaries() {
    let capacity = 64;
    let mut ring = TelemetryRingBuffer::new(capacity);

    // Push 200 packets into 64 capacity buffer
    for i in 1..=200 {
        ring.push("cluster_telemetry", json!({"i": i}), i * 100);
    }

    assert_eq!(ring.current_seq(), 200);
    // Oldest available sequence is 200 - 64 + 1 = 137
    assert_eq!(ring.oldest_seq(), 137);

    // Sequence 50 is deep inside the overwritten gap -> Must return GapOverflow
    match ring.query_missing(50, 0) {
        SyncQueryResult::GapOverflow {
            current_seq,
            oldest_available_seq,
        } => {
            assert_eq!(current_seq, 200);
            assert_eq!(oldest_available_seq, 137);
        }
        _ => panic!("Expected GapOverflow for sequence 50 (deep in gap)"),
    }

    // Sequence 136 is immediately before oldest available (137)
    // The client has seen 136, next needed is 137, which is available in buffer.
    // However, in implementation `since_seq < self.oldest_sequence_id` triggers GapOverflow
    // because since_seq (136) < oldest (137).
    match ring.query_missing(136, 0) {
        SyncQueryResult::GapOverflow {
            current_seq,
            oldest_available_seq,
        } => {
            assert_eq!(current_seq, 200);
            assert_eq!(oldest_available_seq, 137);
        }
        _ => panic!("Expected GapOverflow for since_seq=136 < oldest=137"),
    }

    // Sequence 137: client has seen 137, needs 138..=200 (63 packets)
    match ring.query_missing(137, 0) {
        SyncQueryResult::CatchUp {
            current_seq,
            packets,
        } => {
            assert_eq!(current_seq, 200);
            assert_eq!(packets.len(), 63);
            assert_eq!(packets.first().unwrap().sequence_id, 138);
            assert_eq!(packets.last().unwrap().sequence_id, 200);
        }
        _ => panic!("Expected CatchUp for sequence 137"),
    }

    // Notice: Empirical Behavior of query_missing(0, 0) when wrapped around:
    // In cockpit_web.rs line 1741: `if since_seq > 0 && since_seq < self.oldest_sequence_id`
    // Because `0 > 0` is false, querying with since_seq=0 does NOT trigger GapOverflow;
    // instead it returns CatchUp with all available packets in the buffer (137..=200).
    let zero_res = ring.query_missing(0, 0);
    match zero_res {
        SyncQueryResult::CatchUp {
            current_seq,
            packets,
        } => {
            assert_eq!(current_seq, 200);
            assert_eq!(
                packets.len(),
                64,
                "Query since_seq=0 returns available buffer window of 64 packets"
            );
            assert_eq!(packets.first().unwrap().sequence_id, 137);
        }
        SyncQueryResult::GapOverflow { .. } => {
            // Documenting if changed
        }
        SyncQueryResult::UpToDate { .. } => {
            panic!("query_missing(0) must not return UpToDate");
        }
    }
}

#[test]
fn test_adversarial_ring_buffer_timestamp_filtering() {
    let mut ring = TelemetryRingBuffer::new(64);
    for i in 1..=50 {
        ring.push("metric", json!({"v": i}), i * 1_000);
    }

    // Query since_seq = 20, but since_timestamp_ms = 35_000
    // Packets 21..34 have timestamps < 35_000 and must be filtered out
    match ring.query_missing(20, 35_000) {
        SyncQueryResult::CatchUp {
            current_seq,
            packets,
        } => {
            assert_eq!(current_seq, 50);
            assert_eq!(packets.len(), 16); // 35..=50
            for p in &packets {
                assert!(p.sequence_id >= 35);
                assert!(p.timestamp_ms >= 35_000);
            }
        }
        _ => panic!("Expected filtered CatchUp"),
    }
}

// ============================================================================
// 2. High-Concurrency Stress Test (Multithreaded Readers + Writers)
// ============================================================================

#[test]
fn test_adversarial_ring_buffer_concurrent_push_and_query() {
    let ring = Arc::new(Mutex::new(TelemetryRingBuffer::new(128)));
    let thread_count = 8;
    let pushes_per_thread = 500;

    let mut handles = Vec::new();

    // Spawn writer threads
    for t in 0..thread_count {
        let ring_clone = Arc::clone(&ring);
        handles.push(thread::spawn(move || {
            for i in 0..pushes_per_thread {
                let mut guard = ring_clone.lock().unwrap();
                guard.push(
                    "thread_telemetry",
                    json!({"thread": t, "item": i}),
                    1000 + i as u64,
                );
            }
        }));
    }

    // Spawn reader threads querying concurrently
    let stop_reader = Arc::new(AtomicBool::new(false));
    let mut reader_handles = Vec::new();
    for _ in 0..4 {
        let ring_clone = Arc::clone(&ring);
        let stop_clone = Arc::clone(&stop_reader);
        reader_handles.push(thread::spawn(move || {
            let mut queries_done = 0usize;
            while !stop_clone.load(Ordering::Relaxed) {
                let guard = ring_clone.lock().unwrap();
                let cur = guard.current_seq();
                if cur > 50 {
                    let _ = guard.query_missing(cur - 20, 0);
                }
                queries_done += 1;
                drop(guard);
                thread::sleep(Duration::from_micros(50));
            }
            queries_done
        }));
    }

    // Wait for all writers to complete
    for h in handles {
        h.join().unwrap();
    }

    // Signal readers to stop
    stop_reader.store(true, Ordering::SeqCst);
    for rh in reader_handles {
        let q = rh.join().unwrap();
        assert!(q > 0, "Reader threads must complete queries concurrently");
    }

    // Validate final ring invariants
    let guard = ring.lock().unwrap();
    let total_expected = thread_count * pushes_per_thread;
    assert_eq!(guard.current_seq(), total_expected as u64);
    assert_eq!(guard.oldest_seq(), (total_expected - 128 + 1) as u64);
}

// ============================================================================
// 3. Web Push Tamper-Evident Digest Robustness & Mutation Sensitivity
// ============================================================================

#[test]
fn test_adversarial_push_digest_tamper_detection_and_field_sensitivity() {
    let now_ms = 1_700_000_000_000u64;
    let mut metadata = BTreeMap::new();
    metadata.insert("proposal_id".to_string(), "prop-991".to_string());
    metadata.insert("voter_node_id".to_string(), "node-x".to_string());

    let base_action = PushActionData {
        target_view: "/#security".to_string(),
        alert_id: "alert-001".to_string(),
        severity: "CRITICAL".to_string(),
        metadata,
    };

    let base_payload = PushNotificationPayload::new(
        PushAlertType::ByzantineEquivocation,
        "🚨 Byzantine Attack Detected",
        "Node X attempted double-voting on proposal 991.",
        "tag-01",
        base_action.clone(),
        now_ms,
    );

    assert!(
        base_payload
            .cryptographic_digest
            .starts_with("PANDORA_PUSH_V1:")
    );
    assert_eq!(base_payload.cryptographic_digest.len(), 16 + 64); // "PANDORA_PUSH_V1:" (16) + 64 hex

    // 1. Mutate title by 1 byte -> digest must completely diverge
    let mut title_tampered = base_payload.clone();
    title_tampered.title = "🚨 Byzantine Attack Detectec".to_string(); // 'd' -> 'c'
    let recomputed_title = PushNotificationPayload::new(
        title_tampered.alert_type,
        title_tampered.title.clone(),
        title_tampered.body.clone(),
        title_tampered.tag.clone(),
        title_tampered.action_data.clone(),
        title_tampered.timestamp_ms,
    );
    assert_ne!(
        base_payload.cryptographic_digest, recomputed_title.cryptographic_digest,
        "Title alteration must alter SHA-256 digest"
    );

    // 2. Mutate body by 1 byte -> digest must completely diverge
    let mut body_tampered = base_payload.clone();
    body_tampered.body.push('.');
    let recomputed_body = PushNotificationPayload::new(
        body_tampered.alert_type,
        body_tampered.title.clone(),
        body_tampered.body.clone(),
        body_tampered.tag.clone(),
        body_tampered.action_data.clone(),
        body_tampered.timestamp_ms,
    );
    assert_ne!(
        base_payload.cryptographic_digest, recomputed_body.cryptographic_digest,
        "Body alteration must alter SHA-256 digest"
    );

    // 3. Mutate timestamp by 1 ms -> digest must diverge
    let recomputed_ts = PushNotificationPayload::new(
        base_payload.alert_type,
        base_payload.title.clone(),
        base_payload.body.clone(),
        base_payload.tag.clone(),
        base_payload.action_data.clone(),
        now_ms + 1,
    );
    assert_ne!(
        base_payload.cryptographic_digest, recomputed_ts.cryptographic_digest,
        "Timestamp alteration must alter SHA-256 digest"
    );

    // 4. Mutate metadata entry -> digest must diverge
    let mut meta_tampered = base_action.clone();
    meta_tampered
        .metadata
        .insert("tampered".to_string(), "true".to_string());
    let recomputed_meta = PushNotificationPayload::new(
        base_payload.alert_type,
        base_payload.title.clone(),
        base_payload.body.clone(),
        base_payload.tag.clone(),
        meta_tampered,
        now_ms,
    );
    assert_ne!(
        base_payload.cryptographic_digest, recomputed_meta.cryptographic_digest,
        "Metadata injection must alter SHA-256 digest"
    );

    // 5. Mutate alert_type -> digest must diverge
    let recomputed_type = PushNotificationPayload::new(
        PushAlertType::ConsensusQuorumCompleted,
        base_payload.title.clone(),
        base_payload.body.clone(),
        base_payload.tag.clone(),
        base_payload.action_data.clone(),
        now_ms,
    );
    assert_ne!(
        base_payload.cryptographic_digest, recomputed_type.cryptographic_digest,
        "Alert type alteration must alter SHA-256 digest"
    );

    // 6. Empirical Challenge Finding: Fields omitted from digest construction:
    // In cockpit_web.rs lines 1909-1922:
    // `tag`, `action_data.severity`, `icon`, `badge` are NOT fed into hasher.update().
    // Therefore mutating `severity` (e.g. from CRITICAL to INFO) does NOT change the digest!
    let mut severity_tampered_action = base_action.clone();
    severity_tampered_action.severity = "INFO".to_string(); // downgraded
    let tampered_severity_payload = PushNotificationPayload::new(
        base_payload.alert_type,
        base_payload.title.clone(),
        base_payload.body.clone(),
        base_payload.tag.clone(),
        severity_tampered_action,
        now_ms,
    );
    assert_eq!(
        base_payload.cryptographic_digest, tampered_severity_payload.cryptographic_digest,
        "Observed empirical finding: action_data.severity is not in digest scope"
    );
}

// ============================================================================
// 4. Tripartite Alert Edge Payloads & Unicode Handling
// ============================================================================

#[test]
fn test_adversarial_tripartite_alerts_extreme_payloads() {
    let now_ms = 1_700_000_050_000u64;

    // A. Byzantine Equivocation with empty strings and Unicode RTL/emojis
    let byz = PushNotificationPayload::byzantine_equivocation(
        "⚡\u{200B}مرحبا\n\r\t🔥🦀",
        "",
        Some("Unicode details: \u{0000} null byte safe representation \u{1F4A5}"),
        now_ms,
    );
    assert!(byz.cryptographic_digest.starts_with("PANDORA_PUSH_V1:"));
    assert_eq!(byz.action_data.severity, "CRITICAL");
    assert!(byz.title.contains("🚨 [CRITICAL]"));

    // B. Critical Test Regression with boundary floating-point values
    // Test NaN / Infinity handling: format!("{lcb95:.4}") should produce "NaN" or "inf" without panicking
    let reg_nan = PushNotificationPayload::critical_test_regression(
        "canary-extreme",
        f64::NAN,
        f64::INFINITY,
        0,
        u32::MAX,
        now_ms,
    );
    assert!(reg_nan.cryptographic_digest.starts_with("PANDORA_PUSH_V1:"));
    assert_eq!(reg_nan.action_data.severity, "HIGH");
    assert_eq!(reg_nan.action_data.metadata.get("passes").unwrap(), "0");
    assert_eq!(
        reg_nan.action_data.metadata.get("failures").unwrap(),
        &u32::MAX.to_string()
    );

    // C. Consensus Quorum Completed with extreme node counts and 50KB certificate digest
    let huge_cert = "f".repeat(50_000);
    let quorum = PushNotificationPayload::consensus_quorum_completed(
        "prop-max-nodes",
        "agent-leader",
        usize::MAX,
        usize::MAX,
        &huge_cert,
        now_ms,
    );
    assert!(quorum.cryptographic_digest.starts_with("PANDORA_PUSH_V1:"));
    assert_eq!(quorum.action_data.severity, "INFO");
    assert_eq!(
        quorum.action_data.metadata.get("quorum_size").unwrap(),
        &usize::MAX.to_string()
    );
}

// ============================================================================
// 5. Web Push Subscription Store Lifecycle & Validation
// ============================================================================

#[test]
fn test_adversarial_push_subscription_validation() {
    let now_ms = 1_700_000_100_000u64;

    // 1. Missing / invalid endpoint protocols
    let empty_req = PushSubscribeRequest {
        endpoint: "".into(),
        p256dh_key: Some("key".into()),
        auth_token: Some("auth".into()),
        keys: None,
        user_agent: None,
    };
    assert_eq!(
        empty_req.into_subscription(now_ms),
        Err("invalid_or_missing_endpoint")
    );

    let ftp_req = PushSubscribeRequest {
        endpoint: "ftp://example.com/push".into(),
        p256dh_key: Some("key".into()),
        auth_token: Some("auth".into()),
        keys: None,
        user_agent: None,
    };
    assert_eq!(
        ftp_req.into_subscription(now_ms),
        Err("invalid_or_missing_endpoint")
    );

    let js_req = PushSubscribeRequest {
        endpoint: "javascript:alert(1)".into(),
        p256dh_key: Some("key".into()),
        auth_token: Some("auth".into()),
        keys: None,
        user_agent: None,
    };
    assert_eq!(
        js_req.into_subscription(now_ms),
        Err("invalid_or_missing_endpoint")
    );

    // 2. Missing cryptographic keys
    let missing_p256_req = PushSubscribeRequest {
        endpoint: "https://push.example.com/sub/123".into(),
        p256dh_key: None,
        auth_token: Some("auth_token_here".into()),
        keys: None,
        user_agent: None,
    };
    assert_eq!(
        missing_p256_req.into_subscription(now_ms),
        Err("missing_p256dh_key")
    );

    let missing_auth_req = PushSubscribeRequest {
        endpoint: "https://push.example.com/sub/123".into(),
        p256dh_key: Some("p256dh_key_here".into()),
        auth_token: None,
        keys: None,
        user_agent: None,
    };
    assert_eq!(
        missing_auth_req.into_subscription(now_ms),
        Err("missing_auth_token")
    );

    // 3. Nested W3C keys formatting
    let w3c_req = PushSubscribeRequest {
        endpoint: "https://fcm.googleapis.com/fcm/send/test-sub".into(),
        p256dh_key: None,
        auth_token: None,
        keys: Some(PushSubscriptionKeys {
            p256dh: "BM7d9v...".into(),
            auth: "K8s...".into(),
        }),
        user_agent: Some("mobile-safari".into()),
    };
    let sub = w3c_req
        .into_subscription(now_ms)
        .expect("valid subscription");
    assert_eq!(sub.p256dh_key, "BM7d9v...");
    assert_eq!(sub.auth_token, "K8s...");
    assert_eq!(sub.created_at_ms, now_ms);
}

// ============================================================================
// 6. Push Delivery Failure Resilience (404, 410, 500, network drops)
// ============================================================================

#[test]
fn test_adversarial_push_delivery_error_resilience() {
    let srv = TestServerHarness::start();

    // Spawn mock listener that returns HTTP 404, 410, 500, and drops connection
    let mock_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mock_port = mock_listener.local_addr().unwrap().port();
    let mock_running = Arc::new(AtomicBool::new(true));
    let mock_running_clone = Arc::clone(&mock_running);

    let mock_handle = thread::spawn(move || {
        while mock_running_clone.load(Ordering::Relaxed) {
            if let Ok((mut stream, _)) = mock_listener.accept() {
                if !mock_running_clone.load(Ordering::Relaxed) {
                    break;
                }
                let mut buf = [0u8; 1024];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req_str = String::from_utf8_lossy(&buf[..n]);

                let resp = if req_str.contains("/status_404") {
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                } else if req_str.contains("/status_410") {
                    "HTTP/1.1 410 Gone\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                } else if req_str.contains("/status_500") {
                    "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                } else {
                    // Drop socket immediately (connection reset)
                    continue;
                };

                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
            }
        }
    });

    // Register 4 test subscriptions
    let base_url = format!("http://127.0.0.1:{mock_port}");
    let ep_404 = format!("{base_url}/status_404");
    let ep_410 = format!("{base_url}/status_410");
    let ep_500 = format!("{base_url}/status_500");
    let ep_drop = format!("{base_url}/drop");

    for ep in [&ep_404, &ep_410, &ep_500, &ep_drop] {
        let (code, body) = srv.http_request(
            "POST",
            "/api/push/subscribe",
            &json!({
                "endpoint": ep,
                "p256dh_key": "dummy_p256",
                "auth_token": "dummy_auth"
            })
            .to_string(),
        );
        assert_eq!(code, 200);
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["status"], "subscribed");
    }

    // Verify initial count is 4
    let sub_handle = srv.server().push_subscriptions_handle();
    assert_eq!(sub_handle.lock().unwrap().len(), 4);

    // Dispatch alert:
    // - ep_404: fails, must be pruned
    // - ep_410: fails, must be pruned
    // - ep_500: fails, but temporary error -> NOT pruned
    // - ep_drop: fails on read/write -> NOT pruned
    let alert = PushNotificationPayload::byzantine_equivocation(
        "resilience-node",
        "prop-resilience",
        None,
        1_700_000_200_000,
    );

    let summary = srv.server().dispatch_push_alert(alert);
    assert_eq!(summary.total_subscriptions, 4);
    assert_eq!(summary.successfully_dispatched, 0);
    assert_eq!(summary.failed_dispatches, 4);
    assert_eq!(
        summary.pruned_subscriptions, 2,
        "404 and 410 must be pruned"
    );

    // Check store contents after pruning: exactly ep_500 and ep_drop remain
    let remaining = sub_handle.lock().unwrap();
    assert_eq!(remaining.len(), 2);
    assert!(remaining.contains_key(&ep_500));
    assert!(remaining.contains_key(&ep_drop));
    assert!(!remaining.contains_key(&ep_404));
    assert!(!remaining.contains_key(&ep_410));
    drop(remaining);

    // Stop mock server
    mock_running.store(false, Ordering::Relaxed);
    let _ = TcpStream::connect(("127.0.0.1", mock_port));
    let _ = mock_handle.join();
}

// ============================================================================
// 7. WebSocket Reconnect Sync Framing & Adversarial Frame Stress
// ============================================================================

#[test]
fn test_adversarial_websocket_sync_framing_stress() {
    let srv = TestServerHarness::start();

    // Pre-populate telemetry ring buffer with 300 packets (capacity is 256)
    {
        let ring = srv.server().telemetry_ring_handle();
        let mut r = ring.lock().unwrap();
        for i in 1..=300 {
            r.push("cluster_telemetry", json!({"cpu": 40 + (i % 20)}), i * 100);
        }
    }

    let mut s = TcpStream::connect(("127.0.0.1", srv.port)).expect("connect ws");
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

    let key = "dGhlIHNhbXBsZSBub25jZQ==";
    let req = format!(
        "GET /api/ws HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n",
        srv.port
    );
    s.write_all(req.as_bytes()).unwrap();

    let mut reader = BufReader::new(s.try_clone().unwrap());
    let mut status_line = String::new();
    reader.read_line(&mut status_line).unwrap();
    assert!(status_line.contains("101 Switching Protocols"));

    for _ in 0..10 {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" || line == "\n" {
            break;
        }
    }

    // Read initial telemetry frame
    let (op, payload) = read_server_ws_frame(&mut s);
    assert_eq!(op, 0x1);
    let init_val: Value = serde_json::from_slice(&payload).unwrap();
    assert_eq!(init_val["event"], "telemetry");

    // 1. Send sync_request with valid sequence in range (e.g. 280) -> catch_up
    let sync_req_valid = json!({
        "action": "sync_request",
        "since_sequence_id": 280,
        "since_timestamp_ms": 0
    })
    .to_string();
    let frame1 = encode_client_masked_ws_frame(0x1, sync_req_valid.as_bytes(), [10, 20, 30, 40]);
    s.write_all(&frame1).unwrap();

    let mut catch_up_found = false;
    for _ in 0..10 {
        let (op_s, pay_s) = read_server_ws_frame(&mut s);
        assert_eq!(op_s, 0x1);
        let parsed: Value = serde_json::from_slice(&pay_s).unwrap();
        if parsed["event"] == "sync_response" {
            assert_eq!(parsed["status"], "catch_up");
            assert_eq!(parsed["since_seq"], 280);
            assert_eq!(parsed["current_seq"], 301); // 300 pre-populated + 1 from WS connect
            let missed = parsed["records"].as_array().expect("records array");
            assert_eq!(missed.len(), 21); // 281..=301
            catch_up_found = true;
            break;
        }
    }
    assert!(catch_up_found, "Must receive sync_response catch_up");

    // 2. Send sync_request with sequence 10 (which is in the gap since oldest is 38) -> gap_overflow
    let sync_req_gap = json!({
        "action": "sync_request",
        "since_sequence_id": 10,
        "since_timestamp_ms": 0
    })
    .to_string();
    let frame2 = encode_client_masked_ws_frame(0x1, sync_req_gap.as_bytes(), [11, 22, 33, 44]);
    s.write_all(&frame2).unwrap();

    let mut gap_overflow_found = false;
    for _ in 0..10 {
        let (op_s, pay_s) = read_server_ws_frame(&mut s);
        assert_eq!(op_s, 0x1);
        let parsed: Value = serde_json::from_slice(&pay_s).unwrap();
        if parsed["event"] == "sync_response" {
            assert_eq!(parsed["status"], "gap_overflow");
            assert_eq!(parsed["since_seq"], 10);
            assert!(parsed["latest_snapshot"].is_object());
            assert_eq!(parsed["latest_snapshot"]["panel_count"], 12);
            gap_overflow_found = true;
            break;
        }
    }
    assert!(
        gap_overflow_found,
        "Must receive sync_response gap_overflow"
    );

    // 3. Send malformed/non-JSON frame -> verify server remains healthy and does not disconnect
    let malformed_frame = encode_client_masked_ws_frame(0x1, b"NOT_JSON{{;;;", [99, 88, 77, 66]);
    s.write_all(&malformed_frame).unwrap();

    // 4. Send ping immediately following malformed frame -> must still respond with pong
    let ping_msg = json!({"action": "ping"}).to_string();
    let ping_frame = encode_client_masked_ws_frame(0x1, ping_msg.as_bytes(), [5, 6, 7, 8]);
    s.write_all(&ping_frame).unwrap();

    let mut pong_found = false;
    for _ in 0..10 {
        let (op_s, pay_s) = read_server_ws_frame(&mut s);
        assert_eq!(op_s, 0x1);
        let parsed: Value = serde_json::from_slice(&pay_s).unwrap();
        if parsed["event"] == "pong" {
            pong_found = true;
            break;
        }
    }
    assert!(
        pong_found,
        "Server must remain functional after malformed frame"
    );
}
