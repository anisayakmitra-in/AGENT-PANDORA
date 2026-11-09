//! Integration tests for the embedded cockpit web micro-server (R2).

#![forbid(unsafe_code)]

use pandora_runtime::cockpit_web::{CockpitWebConfig, CockpitWebServer, format_sse_event};
use pandora_runtime::cockpit::CockpitLayout;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::Ordering;
use std::sync::Arc;
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
        Self { port, flag, handle: Some(handle) }
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
    let (code, body) = srv.request("POST", "/api/action/layout", &json!({"layout": "nine_panel"}).to_string());
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
    let (c1, _) = srv.request("POST", "/api/action/layout", &json!({"layout": "bogus"}).to_string());
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
    s.write_all(b"GET /api/events HTTP/1.1\r\nHost: x\r\nAccept: text/event-stream\r\n\r\n").unwrap();
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
