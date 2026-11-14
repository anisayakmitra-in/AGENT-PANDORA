//! Integration tests for the embedded cockpit web micro-server (R2).

#![forbid(unsafe_code)]

use ed25519_dalek::{Signer, SigningKey};
use pandora_runtime::cockpit::CockpitLayout;
use pandora_runtime::cockpit_web::{
    CockpitWebConfig, CockpitWebServer, PairExchangeRequest, PairExchangeResponse, PairQrResponse,
    PushAlertType, PushNotificationPayload, PushSubscribeRequest, PushSubscriptionKeys,
    TelemetryRingBuffer, format_sse_event, generate_pairing_qr_svg,
};
use pandora_runtime::distributed_swarm::encode_hex;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

struct Running {
    port: u16,
    flag: Arc<std::sync::atomic::AtomicBool>,
    server: Arc<CockpitWebServer>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Running {
    fn start() -> Self {
        let config = CockpitWebConfig {
            bind_addr: "127.0.0.1".into(),
            port: 0,
            initial_layout: CockpitLayout::TwelvePanel,
            sse_tick_interval_ms: 100,
        };
        let server = Arc::new(CockpitWebServer::with_config(config, None).expect("bind"));
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

    fn request(&self, method: &str, path: &str, body: &str) -> (u16, String) {
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).expect("connect");
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
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
        let body = raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
        (status, body)
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.flag.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[test]
fn serves_html_dashboard() {
    let srv = Running::start();
    let (code, body) = srv.request("GET", "/", "");
    assert_eq!(code, 200);
    assert!(body.contains("viewport-fit=cover"));
    assert!(body.contains("max-width: 768px"));
    assert!(body.contains("min-height: 48px"));
}

#[test]
fn snapshot_reports_twelve_panels() {
    let srv = Running::start();
    let (code, body) = srv.request("GET", "/api/snapshot", "");
    assert_eq!(code, 200);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["panel_count"], 12);
    assert!(v["snapshot_digest"].as_str().unwrap().len() > 8);
}

#[test]
fn layout_switch_persists() {
    let srv = Running::start();
    let (code, body) = srv.request(
        "POST",
        "/api/action/layout",
        &json!({"layout": "nine_panel"}).to_string(),
    );
    assert_eq!(code, 200);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["panel_count"], 9);
    let (_, snap) = srv.request("GET", "/api/snapshot", "");
    let s: Value = serde_json::from_str(&snap).unwrap();
    assert_eq!(s["panel_count"], 9);
}

#[test]
fn invalid_layout_and_json_rejected() {
    let srv = Running::start();
    let (c1, _) = srv.request(
        "POST",
        "/api/action/layout",
        &json!({"layout": "bogus"}).to_string(),
    );
    assert_eq!(c1, 400);
    let (c2, _) = srv.request("POST", "/api/action/layout", "not json{{");
    assert_eq!(c2, 400);
}

#[test]
fn oversized_payload_rejected() {
    let srv = Running::start();
    let (code, _) = srv.request("POST", "/api/action/layout", &"A".repeat(70 * 1024));
    assert_eq!(code, 413);
}

#[test]
fn unknown_route_404() {
    let srv = Running::start();
    let (code, _) = srv.request("GET", "/nope", "");
    assert_eq!(code, 404);
}

#[test]
fn consensus_and_approve_actions() {
    let srv = Running::start();
    let (c, body) = srv.request("POST", "/api/action/consensus", "{}");
    assert_eq!(c, 200);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["status"], "consensus_achieved");
    assert_eq!(v["total_proposals"], 4);
    assert_eq!(v["byzantine_filtered_count"], 1);

    let (c, body) = srv.request(
        "POST",
        "/api/action/approve",
        &json!({"approval_id": "a1", "allow": true}).to_string(),
    );
    assert_eq!(c, 200);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["verdict"], "approved");

    let (c, _) = srv.request("POST", "/api/action/approve", "{}");
    assert_eq!(c, 400);
}

#[test]
fn sse_stream_emits_telemetry() {
    let srv = Running::start();
    let mut s = TcpStream::connect(("127.0.0.1", srv.port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s.write_all(b"GET /api/events HTTP/1.1\r\nHost: x\r\nAccept: text/event-stream\r\n\r\n")
        .unwrap();
    let mut r = BufReader::new(s);
    let mut seen_sse = false;
    let mut seen_event = false;
    for _ in 0..20 {
        let mut line = String::new();
        if r.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        if line.to_lowercase().contains("text/event-stream") {
            seen_sse = true;
        }
        if line.starts_with("event: telemetry") {
            seen_event = true;
            break;
        }
    }
    assert!(seen_sse && seen_event);
}

#[test]
fn sse_frame_format() {
    let f = format_sse_event("telemetry", 7, &json!({"a": 1}));
    assert_eq!(f, "event: telemetry\nid: 7\ndata: {\"a\":1}\n\n");
}

fn encode_client_test_frame(opcode: u8, payload: &[u8], mask: [u8; 4]) -> Vec<u8> {
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

fn read_server_test_frame(s: &mut TcpStream) -> (u8, Vec<u8>) {
    let mut hdr = [0u8; 2];
    s.read_exact(&mut hdr).expect("read header");
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
        s.read_exact(&mut payload).expect("read payload");
    }
    (opcode, payload)
}

#[test]
fn serves_pwa_manifest() {
    let srv = Running::start();
    let (code, body) = srv.request("GET", "/manifest.webmanifest", "");
    assert_eq!(code, 200);
    assert!(body.contains("standalone"));
    assert!(body.contains("AGENT-PANDORA Mobile Cockpit"));
    assert!(body.contains("theme_color"));
}

#[test]
fn serves_service_worker() {
    let srv = Running::start();
    let (code, body) = srv.request("GET", "/sw.js", "");
    assert_eq!(code, 200);
    assert!(body.contains("CACHE_NAME"));
    assert!(body.contains("addEventListener('fetch'"));
    assert!(body.contains("/manifest.webmanifest"));
}

#[test]
fn websocket_handshake_and_bidirectional_frames() {
    let srv = Running::start();
    let mut s = TcpStream::connect(("127.0.0.1", srv.port)).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

    let key = "dGhlIHNhbXBsZSBub25jZQ==";
    let req = format!(
        "GET /api/ws HTTP/1.1\r\n\
         Host: 127.0.0.1:{}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: {key}\r\n\
         Sec-WebSocket-Version: 13\r\n\r\n",
        srv.port
    );
    s.write_all(req.as_bytes()).unwrap();

    // 1. Read HTTP 101 Handshake Response
    let mut reader = BufReader::new(s.try_clone().unwrap());
    let mut status_line = String::new();
    reader.read_line(&mut status_line).unwrap();
    assert!(status_line.contains("101 Switching Protocols"));

    let mut found_accept = false;
    for _ in 0..10 {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" || line == "\n" {
            break;
        }
        if line
            .to_ascii_lowercase()
            .starts_with("sec-websocket-accept:")
        {
            assert!(line.contains("s3pPLMBiTxaQ9kYGzzhZRbK+xOo="));
            found_accept = true;
        }
    }
    assert!(
        found_accept,
        "Handshake must include RFC 6455 Sec-WebSocket-Accept"
    );

    // 2. Read initial telemetry frame sent by server
    let (op, payload) = read_server_test_frame(&mut s);
    assert_eq!(op, 0x1, "Server must emit initial text frame");
    let init_json: Value = serde_json::from_slice(&payload).expect("valid json");
    assert_eq!(init_json["event"], "telemetry");

    // 3. Client sends masked Ping frame
    let ping_msg = json!({"action": "ping"}).to_string();
    let frame = encode_client_test_frame(0x1, ping_msg.as_bytes(), [1, 2, 3, 4]);
    s.write_all(&frame).unwrap();

    // 4. Server replies with Pong event
    let (op, payload) = read_server_test_frame(&mut s);
    assert_eq!(op, 0x1);
    let pong_json: Value = serde_json::from_slice(&payload).expect("valid json");
    assert_eq!(pong_json["event"], "pong");

    // 5. Client sends layout switch frame
    let layout_msg = json!({"action": "layout", "layout": "nine_panel"}).to_string();
    let frame = encode_client_test_frame(0x1, layout_msg.as_bytes(), [5, 6, 7, 8]);
    s.write_all(&frame).unwrap();

    // 6. Server updates layout and broadcasts telemetry + replies with layout_updated event
    let (op, payload) = read_server_test_frame(&mut s);
    assert_eq!(op, 0x1);
    let first_json: Value = serde_json::from_slice(&payload).expect("valid json");
    if first_json["event"] == "telemetry" {
        let (op2, payload2) = read_server_test_frame(&mut s);
        assert_eq!(op2, 0x1);
        let second_json: Value = serde_json::from_slice(&payload2).expect("valid json");
        assert_eq!(second_json["event"], "layout_updated");
        assert_eq!(second_json["layout"], "nine_panel");
    } else {
        assert_eq!(first_json["event"], "layout_updated");
        assert_eq!(first_json["layout"], "nine_panel");
    }
}

#[test]
fn test_pure_safe_qr_generator_svg_validity() {
    let payload = "pandora-pair-v1:a1b2c3d4e5f60718:deadbeefcafebabe:http://127.0.0.1:8080";
    let svg = generate_pairing_qr_svg(payload);

    assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
    assert!(svg.contains("shape-rendering=\"crispEdges\""));
    assert!(svg.contains("<rect width=\"100%\" height=\"100%\" fill=\"#ffffff\"/>"));
    assert!(svg.contains("<rect x=\""));
    assert!(svg.ends_with("</svg>"));

    // Check version scaling (ISO/IEC 18004 quiet zone = 4 on each side => size + 8):
    // Version 1 (13 bytes <= 16: 21x21 + 8 border = 29x29)
    let short_payload = "pandora:short";
    let short_svg = generate_pairing_qr_svg(short_payload);
    assert!(short_svg.contains("viewBox=\"0 0 29 29\""));

    // Version 2 (20 bytes <= 28: 25x25 + 8 border = 33x33)
    let v2_payload = "a".repeat(20);
    let v2_svg = generate_pairing_qr_svg(&v2_payload);
    assert!(v2_svg.contains("viewBox=\"0 0 33 33\""));

    // Version 3 (35 bytes <= 44: 29x29 + 8 border = 37x37)
    let med_payload = "a".repeat(35);
    let med_svg = generate_pairing_qr_svg(&med_payload);
    assert!(med_svg.contains("viewBox=\"0 0 37 37\""));

    // Version 4 (50 bytes <= 64: 33x33 + 8 border = 41x41)
    let long_payload = "a".repeat(50);
    let long_svg = generate_pairing_qr_svg(&long_payload);
    assert!(long_svg.contains("viewBox=\"0 0 41 41\""));

    // Ensure svg rendering runs deterministic across runs
    let svg2 = generate_pairing_qr_svg(payload);
    assert_eq!(svg, svg2);
}

#[test]
fn test_pairing_qr_endpoint_and_token_generation() {
    let srv = Running::start();
    let (code, body) = srv.request("GET", "/api/pair/qr", "");
    assert_eq!(code, 200);

    let res: PairQrResponse = serde_json::from_str(&body).expect("valid PairQrResponse");
    assert_eq!(res.status, "ready");
    assert_eq!(res.token.len(), 32); // 16 bytes = 32 hex chars
    assert_eq!(res.challenge.len(), 32); // 16 bytes = 32 hex chars
    assert_eq!(res.workstation_pubkey.len(), 64); // 32 bytes = 64 hex chars
    assert!(res.pairing_uri.starts_with("pandora://pair?token="));
    assert!(
        res.pairing_uri
            .contains(&format!("&challenge={}", res.challenge))
    );
    assert!(
        res.pairing_uri
            .contains(&format!("&pubkey={}", res.workstation_pubkey))
    );
    assert!(
        res.qr_svg
            .starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\"")
    );
    assert!(res.expires_at > 0);

    // Verify session exists in server PairingManager
    let pm = srv.server().pairing_manager_handle();
    let lock = pm.lock().unwrap();
    assert!(lock.active_sessions.contains_key(&res.token));
}

#[test]
fn test_pairing_exchange_valid_signature_admitted() {
    let srv = Running::start();
    let (code, body) = srv.request("GET", "/api/pair/qr", "");
    assert_eq!(code, 200);
    let qr_res: PairQrResponse = serde_json::from_str(&body).unwrap();

    // Generate real Ed25519 mobile keypair using standard seed
    let seed = [0x42u8; 32];
    let signing_key = SigningKey::from_bytes(&seed);
    let verifying_key = signing_key.verifying_key();
    let mobile_pubkey_hex = encode_hex(&verifying_key.to_bytes());

    // Sign canonical mutual challenge payload: "pandora-pair-v1:{token}:{challenge}"
    let expected_payload = format!("pandora-pair-v1:{}:{}", qr_res.token, qr_res.challenge);
    let signature = signing_key.sign(expected_payload.as_bytes());
    let signature_hex = encode_hex(&signature.to_bytes());

    let exchange_req = PairExchangeRequest {
        token: qr_res.token.clone(),
        device_name: Some("Xiaomi 15 Pro (HyperOS 2.0)".into()),
        mobile_pubkey: mobile_pubkey_hex.clone(),
        signature: signature_hex,
    };

    let (ex_code, ex_body) = srv.request(
        "POST",
        "/api/pair/exchange",
        &serde_json::to_string(&exchange_req).unwrap(),
    );
    assert_eq!(ex_code, 200);

    let ex_res: PairExchangeResponse = serde_json::from_str(&ex_body).unwrap();
    assert_eq!(ex_res.status, "paired");
    assert!(ex_res.peer_id.starts_with("peer-mobile-"));
    assert_eq!(ex_res.workstation_pubkey.len(), 64);
    assert_eq!(ex_res.session_token.len(), 64);
    assert!(ex_res.expires_at_sec > 0);

    // Verify peer is recorded in server's admitted peers
    let pm = srv.server().pairing_manager_handle();
    let lock = pm.lock().unwrap();
    let peer = lock
        .authenticated_peers
        .get(&ex_res.session_token)
        .expect("peer present");
    assert_eq!(peer.device_name, "Xiaomi 15 Pro (HyperOS 2.0)");
    assert_eq!(peer.public_key, mobile_pubkey_hex);

    // Verify swarm peer was admitted
    let swarm_peer = lock
        .swarm_peers
        .get(&ex_res.peer_id)
        .expect("swarm peer present");
    assert_eq!(swarm_peer.listen_addr, "mobile-pwa");
    assert!(swarm_peer.connected);
}

#[test]
fn test_pairing_exchange_replay_rejected() {
    let srv = Running::start();
    let (code, body) = srv.request("GET", "/api/pair/qr", "");
    assert_eq!(code, 200);
    let qr_res: PairQrResponse = serde_json::from_str(&body).unwrap();

    let seed = [0x55u8; 32];
    let signing_key = SigningKey::from_bytes(&seed);
    let verifying_key = signing_key.verifying_key();
    let mobile_pubkey_hex = encode_hex(&verifying_key.to_bytes());

    let expected_payload = format!("pandora-pair-v1:{}:{}", qr_res.token, qr_res.challenge);
    let signature = signing_key.sign(expected_payload.as_bytes());
    let signature_hex = encode_hex(&signature.to_bytes());

    let req = PairExchangeRequest {
        token: qr_res.token.clone(),
        device_name: Some("Replay Test Device".into()),
        mobile_pubkey: mobile_pubkey_hex,
        signature: signature_hex,
    };

    let req_json = serde_json::to_string(&req).unwrap();
    let (c1, _) = srv.request("POST", "/api/pair/exchange", &req_json);
    assert_eq!(c1, 200);

    // Second attempt with identical token MUST be rejected with 409 Conflict
    let (c2, body2) = srv.request("POST", "/api/pair/exchange", &req_json);
    assert_eq!(c2, 409);
    assert!(body2.contains("already_consumed") || body2.contains("already consumed"));
}

#[test]
fn test_pairing_exchange_invalid_signature_rejected() {
    let srv = Running::start();
    let (code, body) = srv.request("GET", "/api/pair/qr", "");
    assert_eq!(code, 200);
    let qr_res: PairQrResponse = serde_json::from_str(&body).unwrap();

    let seed = [0x77u8; 32];
    let signing_key = SigningKey::from_bytes(&seed);
    let verifying_key = signing_key.verifying_key();
    let mobile_pubkey_hex = encode_hex(&verifying_key.to_bytes());

    // Sign WRONG payload
    let wrong_payload = "wrong-challenge-string";
    let signature = signing_key.sign(wrong_payload.as_bytes());
    let signature_hex = encode_hex(&signature.to_bytes());

    let req = PairExchangeRequest {
        token: qr_res.token,
        device_name: Some("Forged Signature Device".into()),
        mobile_pubkey: mobile_pubkey_hex,
        signature: signature_hex,
    };

    let (c, body) = srv.request(
        "POST",
        "/api/pair/exchange",
        &serde_json::to_string(&req).unwrap(),
    );
    assert_eq!(c, 401);
    assert!(body.contains("signature_verification_failed") || body.contains("verification failed"));
}

#[test]
fn test_websocket_reconnect_sync_framing() {
    let srv = Running::start();

    // Pre-populate telemetry ring buffer with sequenced packets
    {
        let ring = srv.server().telemetry_ring_handle();
        let mut r = ring.lock().unwrap();
        r.push("cluster_telemetry", json!({"metric": "m1"}), 1000);
        r.push("cluster_telemetry", json!({"metric": "m2"}), 2000);
        r.push("cluster_telemetry", json!({"metric": "m3"}), 3000);
        r.push("cluster_telemetry", json!({"metric": "m4"}), 4000);
        r.push("cluster_telemetry", json!({"metric": "m5"}), 5000);
    }

    let mut s = TcpStream::connect(("127.0.0.1", srv.port)).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

    let key = "dGhlIHNhbXBsZSBub25jZQ==";
    let req = format!(
        "GET /api/ws HTTP/1.1\r\n\
         Host: 127.0.0.1:{}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: {key}\r\n\
         Sec-WebSocket-Version: 13\r\n\r\n",
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
    let (op, payload) = read_server_test_frame(&mut s);
    assert_eq!(op, 0x1);
    let init_json: Value = serde_json::from_slice(&payload).expect("valid json");
    assert_eq!(init_json["event"], "telemetry");
    assert!(init_json["sequence_id"].as_u64().unwrap() >= 5);

    // Client sends sync_request for missed packets since sequence_id 2
    let sync_req = json!({
        "action": "sync_request",
        "since_sequence_id": 2,
        "since_timestamp_ms": 1500
    })
    .to_string();
    let frame = encode_client_test_frame(0x1, sync_req.as_bytes(), [11, 22, 33, 44]);
    s.write_all(&frame).unwrap();

    // Server should respond with sync_response (skip any interleaved background telemetry frames)
    let mut sync_json = None;
    for _ in 0..10 {
        let (op_sync, payload_sync) = read_server_test_frame(&mut s);
        assert_eq!(op_sync, 0x1);
        let parsed: Value = serde_json::from_slice(&payload_sync).expect("valid json");
        if parsed["event"] == "sync_response" {
            sync_json = Some(parsed);
            break;
        }
    }
    let sync_json = sync_json.expect("Must receive sync_response event");
    assert_eq!(sync_json["status"], "catch_up");
    assert_eq!(sync_json["since_seq"], 2);
    assert!(sync_json["missed_packets_count"].as_u64().unwrap() >= 3);

    // Test ring buffer unit behavior for gap overflow:
    let mut ring = TelemetryRingBuffer::new(64);
    for i in 1..=100 {
        ring.push("t", json!({"x": i}), i * 100);
    }
    // With capacity 64, packets 1..=36 have been overwritten (oldest is 37)
    // Client requesting sequence 10 experiences gap overflow:
    let res = ring.query_missing(10, 0);
    match res {
        pandora_runtime::cockpit_web::SyncQueryResult::GapOverflow {
            current_seq,
            oldest_available_seq,
        } => {
            assert_eq!(current_seq, 100);
            assert_eq!(oldest_available_seq, 37);
        }
        _ => panic!("Expected GapOverflow for sequence 10"),
    }
}

#[test]
fn test_push_subscription_lifecycle_and_alert_dispatch() {
    let srv = Running::start();

    // Spin up lightweight mock push receiver on localhost to receive and respond to real Web Push HTTP dispatches
    let mock_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mock_port = mock_listener.local_addr().unwrap().port();
    let mock_running = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let mock_running_clone = Arc::clone(&mock_running);
    let mock_handle = thread::spawn(move || {
        while mock_running_clone.load(Ordering::Relaxed) {
            if let Ok((mut stream, _)) = mock_listener.accept() {
                if !mock_running_clone.load(Ordering::Relaxed) {
                    break;
                }
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let resp = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
            }
        }
    });

    let mock_endpoint = format!("http://127.0.0.1:{mock_port}/push");

    // 1. Subscribe to push notifications
    let sub_req = PushSubscribeRequest {
        endpoint: mock_endpoint.clone(),
        p256dh_key: None,
        auth_token: None,
        keys: Some(PushSubscriptionKeys {
            p256dh: "BNcRdreALRFXTkOOUHK1EtK2wtaz5Ry4YfYCA_0QT9AcUbVlWmkaU002ErnvYEl35uKdhNmORQOTcVb-IzxFlY8".into(),
            auth: "tBHItJI5svbpez7KI4CCXg".into(),
        }),
        user_agent: Some("xiaomi-15-pro-mobile".into()),
    };

    let (c_sub, b_sub) = srv.request(
        "POST",
        "/api/push/subscribe",
        &serde_json::to_string(&sub_req).unwrap(),
    );
    assert_eq!(c_sub, 200);
    let v_sub: Value = serde_json::from_str(&b_sub).unwrap();
    assert_eq!(v_sub["status"], "subscribed");
    assert_eq!(v_sub["subscription_count"], 1);

    // 2. Dispatch Byzantine Equivocation alert
    let byz_alert = PushNotificationPayload::byzantine_equivocation(
        "agent-byz-07",
        "prop-4412",
        Some("Contradictory votes detected on branch main"),
        1_700_000_000_000,
    );
    assert!(
        byz_alert
            .cryptographic_digest
            .starts_with("PANDORA_PUSH_V1:")
    );
    assert_eq!(byz_alert.action_data.severity, "CRITICAL");
    assert_eq!(byz_alert.action_data.target_view, "/#security");
    assert_eq!(
        byz_alert
            .action_data
            .metadata
            .get("action")
            .map(|s| s.as_str()),
        Some("QUARANTINE_NODE")
    );

    let summary1 = srv.server().dispatch_push_alert(byz_alert);
    assert_eq!(summary1.successfully_dispatched, 1);
    assert_eq!(summary1.failed_dispatches, 0);

    // 3. Dispatch Critical Test Regression alert
    let reg_alert = PushNotificationPayload::critical_test_regression(
        "canary-m3-eval",
        0.84,
        0.95,
        42,
        8,
        1_700_000_000_100,
    );
    assert!(
        reg_alert
            .cryptographic_digest
            .starts_with("PANDORA_PUSH_V1:")
    );
    assert_eq!(reg_alert.alert_type, PushAlertType::CriticalTestRegression);
    assert_eq!(reg_alert.action_data.severity, "HIGH");
    assert_eq!(
        reg_alert
            .action_data
            .metadata
            .get("action")
            .map(|s| s.as_str()),
        Some("ABORT_PROMOTION")
    );

    let summary2 = srv.server().dispatch_push_alert(reg_alert);
    assert_eq!(summary2.successfully_dispatched, 1);

    // 4. Dispatch Consensus Quorum Completed alert
    let quorum_alert = PushNotificationPayload::consensus_quorum_completed(
        "prop-8819",
        "agent-orchestrator",
        4,
        4,
        "0xabcdef1234567890",
        1_700_000_000_200,
    );
    assert!(
        quorum_alert
            .cryptographic_digest
            .starts_with("PANDORA_PUSH_V1:")
    );
    assert_eq!(
        quorum_alert.alert_type,
        PushAlertType::ConsensusQuorumCompleted
    );
    assert_eq!(quorum_alert.action_data.severity, "INFO");
    assert_eq!(quorum_alert.action_data.target_view, "/#consensus");

    let summary3 = srv.server().dispatch_push_alert(quorum_alert);
    assert_eq!(summary3.successfully_dispatched, 1);

    // 5. Test GET /api/push/alerts route
    let (c_alerts, b_alerts) = srv.request("GET", "/api/push/alerts", "");
    assert_eq!(c_alerts, 200);
    let v_alerts: Value = serde_json::from_str(&b_alerts).unwrap();
    let alerts_arr = v_alerts["alerts"].as_array().expect("array of alerts");
    assert!(alerts_arr.len() >= 3);
    assert!(
        alerts_arr
            .iter()
            .any(|a| a["alert_type"] == "byzantine_equivocation")
    );
    assert!(
        alerts_arr
            .iter()
            .any(|a| a["alert_type"] == "critical_test_regression")
    );
    assert!(
        alerts_arr
            .iter()
            .any(|a| a["alert_type"] == "consensus_quorum_completed")
    );

    // 6. Test POST /api/push/test route
    let (c_test, b_test) = srv.request(
        "POST",
        "/api/push/test",
        &json!({"title": "Custom Test", "body": "Testing Web Push"}).to_string(),
    );
    assert_eq!(c_test, 200);
    let v_test: Value = serde_json::from_str(&b_test).unwrap();
    assert_eq!(v_test["successfully_dispatched"], 1);

    // 7. Unsubscribe
    let (c_unsub, b_unsub) = srv.request(
        "POST",
        "/api/push/unsubscribe",
        &json!({"endpoint": mock_endpoint}).to_string(),
    );
    assert_eq!(c_unsub, 200);
    let v_unsub: Value = serde_json::from_str(&b_unsub).unwrap();
    assert_eq!(v_unsub["status"], "unsubscribed");
    assert_eq!(v_unsub["removed"], true);

    // Dispatching after unsubscribe has 0 delivered subscribers
    let summary_after =
        srv.server()
            .dispatch_push_alert(PushNotificationPayload::byzantine_equivocation(
                "agent-post-unsub",
                "prop-0",
                None,
                1_700_000_001_000,
            ));
    assert_eq!(summary_after.successfully_dispatched, 0);

    mock_running.store(false, Ordering::Relaxed);
    let _ = std::net::TcpStream::connect(("127.0.0.1", mock_port));
    let _ = mock_handle.join();
}

#[test]
fn test_service_worker_offline_telemetry_and_manifest_enhancements() {
    let srv = Running::start();

    // Check PWA Manifest
    let (m_code, m_body) = srv.request("GET", "/manifest.webmanifest", "");
    assert_eq!(m_code, 200);
    assert!(m_body.contains("\"orientation\": \"any\""));
    assert!(m_body.contains("\"display_override\""));
    assert!(m_body.contains("\"shortcuts\""));
    assert!(m_body.contains("\"512x512\""));
    assert!(m_body.contains("\"purpose\": \"any maskable\""));

    // Check Service Worker
    let (sw_code, sw_body) = srv.request("GET", "/sw.js", "");
    assert_eq!(sw_code, 200);
    assert!(sw_body.contains("CACHE_TELEMETRY"));
    assert!(sw_body.contains("pandora-telemetry-v3.0.10"));
    assert!(sw_body.contains("addEventListener('push'"));
    assert!(sw_body.contains("addEventListener('notificationclick'"));
    assert!(sw_body.contains("[200, 100, 200, 100, 400]"));
    assert!(sw_body.contains("NAVIGATE_VIEW"));

    // Check Dashboard HTML for TelemetryDB and offline seeding
    let (h_code, h_body) = srv.request("GET", "/", "");
    assert_eq!(h_code, 200);
    assert!(h_body.contains("class TelemetryDB"));
    assert!(h_body.contains("pandora_telemetry_db"));
    assert!(h_body.contains("cluster_metrics"));
    assert!(h_body.contains("run_histories"));
    assert!(h_body.contains("sync_request"));
    assert!(h_body.contains("sync_response"));
    assert!(h_body.contains("OFFLINE STORE"));
}
