//! End-to-End Integration Tests for Autonomous Daemon Guard & Cockpit Web Telemetry / WebSocket Streaming.
//!
//! Validates:
//! 1. Option 1: Autonomous Self-Healing Daemon Guard & Causal Merkle DAG Audit.
//!    - Multi-engine telemetry evaluation ticks (`pandora runtime daemon-guard`).
//!    - Self-healing actions, circuit breaker tracking, and phased assurance receipts (12 engines).
//!    - Causal Merkle DAG persistence (`merkle_audit.json`), inclusion verification (`pandora audit verify --dag-node`), and tree inspection (`pandora audit tree`).
//! 2. Option 2: Cockpit Web Real-Time Multi-Agent Consensus Streaming.
//!    - CLI non-blocking initialization (`pandora cockpit web --dry-run --json`).
//!    - HTTP REST snapshot (`/api/snapshot`) and PWA webmanifest endpoints.
//!    - RFC 6455 WebSocket handshake (`/api/ws`), initial telemetry streaming, ping/pong frames, layout reconfiguration, and Byzantine swarm consensus resolution (Theorem 43 / Theorem 47).
//! 3. Integrated telemetry & causal audit synchronization loop.
//!
//! # Safety Invariant
//! Strictly `#![forbid(unsafe_code)]` enforced workspace-wide.

#![forbid(unsafe_code)]

use pandora_runtime::cockpit::CockpitLayout;
use pandora_runtime::cockpit_web::{CockpitWebConfig, CockpitWebServer};
use pandora_runtime::gateway::{GatewayConfig, GatewayController, SelfHealingAction};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(1000);

struct Fixture {
    root: PathBuf,
    config: PathBuf,
    data: PathBuf,
    workspace: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock available")
            .as_nanos();
        let sequence = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let temp_root = std::env::temp_dir()
            .canonicalize()
            .expect("temporary directory canonical path");
        let root = temp_root.join(format!(
            "pandora-daemon-cockpit-e2e-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        let data = root.join("data");
        fs::create_dir_all(&workspace).expect("workspace created");
        fs::create_dir_all(&data).expect("data dir created");
        Self {
            config: root.join("config.json"),
            data,
            workspace,
            root,
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_pandora"));
        cmd.args(args)
            .env("PANDORA_CONFIG", &self.config)
            .env("PANDORA_DATA_DIR", &self.data)
            .env("PANDORA_WORKSPACE", &self.workspace);
        cmd
    }

    fn run_success(&self, args: &[&str]) -> Value {
        let output = self.command(args).output().expect("command executes");
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        assert!(
            output.status.success(),
            "command '{:?}' failed with stderr: {}\nstdout: {}",
            args,
            stderr,
            stdout
        );
        serde_json::from_str(&stdout).expect("output must be valid JSON envelope")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Helper harness for spawning an in-process CockpitWebServer on an ephemeral OS port.
struct RunningWebServer {
    port: u16,
    flag: Arc<std::sync::atomic::AtomicBool>,
    server: Arc<CockpitWebServer>,
    handle: Option<thread::JoinHandle<()>>,
}

impl RunningWebServer {
    fn start(layout: CockpitLayout) -> Self {
        let config = CockpitWebConfig {
            bind_addr: "127.0.0.1".into(),
            port: 0,
            initial_layout: layout,
            sse_tick_interval_ms: 100,
        };
        let server =
            Arc::new(CockpitWebServer::with_config(config, None).expect("bind web server"));
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

    fn request(&self, method: &str, path: &str, body: &str) -> (u16, String) {
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).expect("connect to web server");
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
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
        let body_part = raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
        (status, body_part)
    }
}

impl Drop for RunningWebServer {
    fn drop(&mut self) {
        self.flag.store(true, Ordering::SeqCst);
        self.server.shutdown();
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
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
    s.read_exact(&mut hdr).expect("read frame header");
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
fn test_daemon_guard_self_healing_circuit_breaker_and_causal_dag() {
    let fixture = Fixture::new();
    let store_path = fixture.data.join("merkle_audit.json");
    let store_str = store_path.display().to_string();

    // 1. Run 3 iterations of daemon-guard with 20ms interval
    let guard_res = fixture.run_success(&[
        "runtime",
        "daemon-guard",
        "--iterations",
        "3",
        "--interval-ms",
        "20",
        "--store",
        &store_str,
        "--json",
    ]);

    assert_eq!(guard_res["command"], "runtime daemon-guard");
    assert_eq!(guard_res["sequence_number"], 2);
    assert_eq!(guard_res["phases_certified"], 12);
    assert!(guard_res["all_invariants_verified"].as_bool().unwrap());
    assert!(
        guard_res["receipt_sha256"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );

    let tick_reports = guard_res["tick_reports"]
        .as_array()
        .expect("tick_reports array");
    assert_eq!(tick_reports.len(), 3);
    for (i, tick) in tick_reports.iter().enumerate() {
        assert_eq!(tick["sequence_number"], i);
        assert!(
            tick["receipt"]["receipt_sha256"]
                .as_str()
                .unwrap()
                .starts_with("sha256:")
        );
    }

    // 2. Read the resulting merkle_audit.json to extract DAG node IDs
    let store_bytes = fs::read(&store_path).expect("read merkle_audit.json");
    let audit_file: Value = serde_json::from_slice(&store_bytes).expect("parse store json");
    let dag_entries = audit_file["dag_entries"]
        .as_array()
        .expect("dag_entries present");
    assert!(dag_entries.len() >= 3);

    let last_node_id = guard_res["audit_node_id"]
        .as_str()
        .expect("audit_node_id string");

    // 3. Cryptographically verify DAG node causal lineage via `pandora audit verify`
    let verify_res = fixture.run_success(&[
        "audit",
        "verify",
        "--dag-node",
        last_node_id,
        "--store",
        &store_str,
        "--json",
    ]);

    assert_eq!(verify_res["command"], "audit verify");
    assert_eq!(verify_res["verified"], true);
    assert_eq!(verify_res["tamper_status"], "UNMODIFIED");
    assert_eq!(verify_res["node_id"], last_node_id);

    // 4. Inspect Merkle-DAG topological tree via `pandora audit tree`
    let tree_res = fixture.run_success(&["audit", "tree", "--store", store_str.as_str(), "--json"]);

    assert_eq!(tree_res["command"], "audit tree");
    assert_eq!(tree_res["is_acyclic"], true);
    assert!(tree_res["dag_node_count"].as_u64().unwrap() >= 3);
}

#[test]
fn test_cockpit_web_live_snapshot_and_websocket_consensus_stream() {
    let fixture = Fixture::new();

    // 1. Verify CLI output structure without blocking
    let cli_res = fixture.run_success(&[
        "cockpit",
        "web",
        "--port",
        "0",
        "--bind",
        "127.0.0.1",
        "--dry-run",
        "--json",
    ]);

    assert_eq!(cli_res["status"], "ok");
    assert_eq!(cli_res["layout"], "twelve_panel");
    assert_eq!(cli_res["panel_count"], 12);
    assert_eq!(cli_res["sse_endpoint"], "/api/events");
    assert_eq!(cli_res["snapshot_endpoint"], "/api/snapshot");

    // 2. Launch live in-process micro-server
    let web_server = RunningWebServer::start(CockpitLayout::TwelvePanel);

    // 3. Test HTTP /api/snapshot endpoint
    let (code, snap_body) = web_server.request("GET", "/api/snapshot", "");
    assert_eq!(code, 200);
    let snap: Value = serde_json::from_str(&snap_body).expect("valid snapshot json");
    assert_eq!(snap["layout"], "twelve_panel");
    assert_eq!(snap["panel_count"], 12);
    let panels = snap["panels"].as_array().unwrap();
    assert_eq!(panels.len(), 12);
    let daemon_panel = panels
        .iter()
        .find(|p| p["title"] == "[01:Daemon-Guard]")
        .expect("daemon guard panel present");
    assert_eq!(daemon_panel["health"], "healthy");
    assert_eq!(daemon_panel["metrics"]["circuit_breaker"], "CLOSED");
    assert!(
        snap["snapshot_digest"]
            .as_str()
            .unwrap()
            .starts_with("PANDORA_COCKPIT_V1:")
    );

    // 4. Test PWA Web Manifest
    let (m_code, m_body) = web_server.request("GET", "/manifest.webmanifest", "");
    assert_eq!(m_code, 200);
    assert!(m_body.contains("AGENT-PANDORA Mobile Cockpit"));

    // 5. Connect via RFC 6455 WebSocket
    let mut ws_stream =
        TcpStream::connect(("127.0.0.1", web_server.port)).expect("connect to websocket");
    ws_stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();

    let key = "dGhlIHNhbXBsZSBub25jZQ==";
    let req = format!(
        "GET /api/ws HTTP/1.1\r\n\
         Host: 127.0.0.1:{}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: {key}\r\n\
         Sec-WebSocket-Version: 13\r\n\r\n",
        web_server.port
    );
    ws_stream.write_all(req.as_bytes()).unwrap();

    // Verify 101 Handshake
    let mut reader = BufReader::new(ws_stream.try_clone().unwrap());
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
        "Sec-WebSocket-Accept must match RFC 6455 hash"
    );

    // 6. Read initial telemetry frame emitted by server
    let (op, payload) = read_server_test_frame(&mut ws_stream);
    assert_eq!(op, 0x1);
    let init_frame: Value = serde_json::from_slice(&payload).expect("initial json");
    assert_eq!(init_frame["event"], "telemetry");

    // 7. Ping / Pong interaction over WebSocket
    let ping_msg = json!({"action": "ping"}).to_string();
    let ping_frame = encode_client_test_frame(0x1, ping_msg.as_bytes(), [1, 3, 3, 7]);
    ws_stream.write_all(&ping_frame).unwrap();

    let (op, payload) = read_server_test_frame(&mut ws_stream);
    assert_eq!(op, 0x1);
    let pong_val: Value = serde_json::from_slice(&payload).expect("pong json");
    assert_eq!(pong_val["event"], "pong");

    // 8. Reconfigure Layout over WebSocket (TwelvePanel -> NinePanel)
    let layout_msg = json!({"action": "layout", "layout": "nine_panel"}).to_string();
    let layout_frame = encode_client_test_frame(0x1, layout_msg.as_bytes(), [2, 4, 6, 8]);
    ws_stream.write_all(&layout_frame).unwrap();

    let (op, payload) = read_server_test_frame(&mut ws_stream);
    assert_eq!(op, 0x1);
    let first_resp: Value = serde_json::from_slice(&payload).expect("layout resp json");
    if first_resp["event"] == "telemetry" {
        let (op2, payload2) = read_server_test_frame(&mut ws_stream);
        assert_eq!(op2, 0x1);
        let second_resp: Value = serde_json::from_slice(&payload2).expect("layout resp 2");
        assert_eq!(second_resp["event"], "layout_updated");
        assert_eq!(second_resp["layout"], "nine_panel");
    } else {
        assert_eq!(first_resp["event"], "layout_updated");
        assert_eq!(first_resp["layout"], "nine_panel");
    }

    // 9. Byzantine Swarm Consensus Resolution (Theorem 43 / Theorem 47) over WebSocket
    let consensus_req =
        json!({"action": "consensus", "threshold": 0.50, "divergence": 0.45}).to_string();
    let consensus_frame = encode_client_test_frame(0x1, consensus_req.as_bytes(), [9, 8, 7, 6]);
    ws_stream.write_all(&consensus_frame).unwrap();

    let (op, payload) = read_server_test_frame(&mut ws_stream);
    assert_eq!(op, 0x1);
    let decision_frame: Value = serde_json::from_slice(&payload).expect("consensus decision json");
    assert_eq!(decision_frame["event"], "consensus_decision");
    assert_eq!(decision_frame["status"], "consensus_achieved");
    assert_eq!(decision_frame["winning_proposal_id"], "p_agy");
    assert_eq!(decision_frame["winning_agent"], "agy");
    assert!(decision_frame["honest_quorum_size"].as_u64().unwrap() >= 2);
    assert_eq!(decision_frame["consensus_hash"].as_str().unwrap().len(), 64);
}

#[test]
fn test_integrated_daemon_guard_and_cockpit_telemetry_flow() {
    let fixture = Fixture::new();
    let store_path = fixture.data.join("merkle_audit.json");
    let store_str = store_path.display().to_string();

    // 1. Run single tick of daemon guard
    let guard_res = fixture.run_success(&[
        "runtime",
        "daemon-guard",
        "--once",
        "--store",
        &store_str,
        "--json",
    ]);

    assert_eq!(guard_res["sequence_number"], 0);
    assert_eq!(guard_res["phases_certified"], 12);
    assert!(store_path.exists());

    // 2. Launch web server and verify 12-panel layout displays active circuit breakers
    let web_server = RunningWebServer::start(CockpitLayout::TwelvePanel);
    let (code, body) = web_server.request("GET", "/api/snapshot", "");
    assert_eq!(code, 200);

    let snap: Value = serde_json::from_str(&body).expect("snapshot json");
    assert_eq!(snap["panel_count"], 12);
    let panels = snap["panels"].as_array().expect("panels array");
    let daemon_guard_panel = panels
        .iter()
        .find(|p| p["title"] == "[01:Daemon-Guard]")
        .expect("daemon guard panel present");
    assert_eq!(daemon_guard_panel["health"], "healthy");
    assert_eq!(daemon_guard_panel["metrics"]["circuit_breaker"], "CLOSED");

    // 3. Verify audit record is traceable via audit verify
    let node_id = guard_res["audit_node_id"]
        .as_str()
        .expect("audit_node_id string");
    let verify_res = fixture.run_success(&[
        "audit",
        "verify",
        "--dag-node",
        node_id,
        "--store",
        &store_str,
        "--json",
    ]);

    assert_eq!(verify_res["verified"], true);
    assert_eq!(verify_res["tamper_status"], "UNMODIFIED");
}

#[test]
fn test_daemon_guard_degraded_provider_self_healing_circuit_breaker_reroute() {
    let fixture = Fixture::new();
    let store_path = fixture.data.join("merkle_audit.json");

    let mut fallback_map = BTreeMap::new();
    fallback_map.insert("anthropic".to_string(), "openai".to_string());

    let gateway = GatewayController::new(GatewayConfig {
        tick_interval_ms: 10,
        circuit_breaker_threshold: 2,
        cooldown_secs: 60,
        audit_store_path: store_path.clone(),
        fallback_provider_map: fallback_map,
    });

    let mut degraded_telem = gateway.collect_live_telemetry();
    degraded_telem
        .provider_latencies
        .insert("anthropic".to_string(), 1850.0);

    // Tick 1: Failure count = 1 < threshold 2 -> Breaker remains closed
    let report1 = gateway
        .evaluate_telemetry_tick(degraded_telem.clone())
        .expect("tick 1 evaluation");
    assert_eq!(report1.sequence_number, 0);
    assert!(report1.self_healing_actions.is_empty());

    // Tick 2: Failure count = 2 >= threshold 2 -> Breaker trips, provider reroutes to openai
    let report2 = gateway
        .evaluate_telemetry_tick(degraded_telem)
        .expect("tick 2 evaluation");
    assert_eq!(report2.sequence_number, 1);
    assert!(!report2.self_healing_actions.is_empty());

    let has_trip = report2
        .self_healing_actions
        .iter()
        .any(|a| matches!(a, SelfHealingAction::TripCircuitBreaker { target, .. } if target == "anthropic"));
    assert!(has_trip, "Anthropic circuit breaker should trip");

    let has_reroute = report2.self_healing_actions.iter().any(|a| {
        matches!(
            a,
            SelfHealingAction::RerouteProvider {
                from_provider,
                to_provider,
                ..
            } if from_provider == "anthropic" && to_provider == "openai"
        )
    });
    assert!(
        has_reroute,
        "Provider should reroute from anthropic to openai"
    );

    // Verify cryptographic DAG node commit
    assert!(store_path.exists());
    let store_bytes = fs::read(&store_path).expect("read store");
    let audit_file: Value = serde_json::from_slice(&store_bytes).expect("parse store");
    let dag_entries = audit_file["dag_entries"]
        .as_array()
        .expect("dag entries array");
    assert_eq!(dag_entries.len(), 2);
    assert_eq!(dag_entries[1]["node"]["id"], report2.audit_node_id);
}

#[test]
fn test_cockpit_web_sse_streaming_and_mobile_pairing_flow() {
    let web_server = RunningWebServer::start(CockpitLayout::TwelvePanel);

    // 1. SSE /api/events test
    let mut sse_stream =
        TcpStream::connect(("127.0.0.1", web_server.port)).expect("connect to sse");
    sse_stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();

    let req = format!(
        "GET /api/events HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n",
        web_server.port
    );
    sse_stream.write_all(req.as_bytes()).unwrap();

    let mut reader = BufReader::new(sse_stream);
    let mut status_line = String::new();
    reader.read_line(&mut status_line).unwrap();
    assert!(status_line.contains("200 OK"));

    let mut found_telemetry_event = false;
    for _ in 0..10 {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line.contains("event: telemetry") {
            found_telemetry_event = true;
            break;
        }
    }
    assert!(
        found_telemetry_event,
        "SSE stream must emit initial telemetry event"
    );

    // 2. Mobile Pairing QR Code SVG test (`/api/pair/qr`)
    let (qr_code, qr_body) = web_server.request("GET", "/api/pair/qr", "");
    assert_eq!(qr_code, 200);
    let qr_json: Value = serde_json::from_str(&qr_body).expect("qr json");
    assert_eq!(qr_json["status"], "ready");
    assert!(!qr_json["token"].as_str().unwrap().is_empty());
    assert!(!qr_json["challenge"].as_str().unwrap().is_empty());
    assert!(!qr_json["workstation_pubkey"].as_str().unwrap().is_empty());
    let svg = qr_json["qr_svg"].as_str().expect("qr svg string");
    assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
    assert!(
        qr_json["pairing_uri"]
            .as_str()
            .unwrap()
            .starts_with("pandora://pair?")
    );

    // 3. Parity check: /api/status is alias to /api/snapshot
    let (s_code, s_body) = web_server.request("GET", "/api/status", "");
    assert_eq!(s_code, 200);
    let status_json: Value = serde_json::from_str(&s_body).expect("status json");
    assert_eq!(status_json["layout"], "twelve_panel");
    assert_eq!(status_json["panel_count"], 12);
}
