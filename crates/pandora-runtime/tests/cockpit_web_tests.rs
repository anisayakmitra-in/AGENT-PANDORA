//! Integration tests for the embedded cockpit web micro-server (R2).

#![forbid(unsafe_code)]

use pandora_runtime::cockpit::CockpitLayout;
use pandora_runtime::cockpit_web::{CockpitWebConfig, CockpitWebServer, format_sse_event};
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
        let server = CockpitWebServer::with_config(config, None).expect("bind");
        let port = server.port();
        let flag = server.shutdown_handle();
        let handle = thread::spawn(move || {
            let _ = server.run();
        });
        Self {
            port,
            flag,
            handle: Some(handle),
        }
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
