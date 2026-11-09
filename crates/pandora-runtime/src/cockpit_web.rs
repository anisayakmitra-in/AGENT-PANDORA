//! Web / Mobile Remote Telemetry Dashboard Micro-Server (Phase 23 / R2).
//!
//! Provides a zero-dependency, safe Rust HTTP/1.1 + Server-Sent Events (SSE) companion
//! micro-server for real-time 9/12-panel cockpit telemetry, mobile viewport adaptation
//! (calibrated for Xiaomi 15 ~393×851 px and desktop displays), and interactive remote actions
//! (layout switching, Theorem 43 Byzantine swarm consensus, and agent permission approvals).
//!
//! # Safety Invariant
//! Strictly `#![forbid(unsafe_code)]` enforced workspace-wide.

#![forbid(unsafe_code)]

use crate::approvals::ApprovalStore;
use crate::cockpit::{CockpitController, CockpitLayout, CockpitSnapshot};
use crate::multi_agent_collab::{ExternalAgentKind, MultiAgentCollabManager};
use crate::multi_agent_consensus::{
    AgentPatchProposal, ProposalScoreVector, SwarmCodeConsensusEngine,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Error as IoError, ErrorKind, Read, Result as IoResult, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Maximum allowed HTTP POST body size (64 KB) to prevent memory exhaustion.
pub const MAX_HTTP_BODY_BYTES: usize = 64 * 1024;

/// Default SSE broadcast tick interval in milliseconds (1 Hz).
pub const DEFAULT_SSE_TICK_MS: u64 = 1000;

/// Embedded zero-dependency HTML5/CSS3/JS single-page dashboard calibrated for
/// Xiaomi 15 mobile (~393×851 px, 20:9 aspect ratio) and desktop viewports.
pub const DASHBOARD_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0, maximum-scale=1.0, user-scalable=no, viewport-fit=cover">
    <title>AGENT-PANDORA Cockpit Web v3.0.10</title>
    <style>
        :root {
            --bg: #0a0d14;
            --surface: #121824ee;
            --border: #1e293b;
            --cyan: #00f0ff;
            --purple: #8b5cf6;
            --emerald: #10b981;
            --amber: #f59e0b;
            --ruby: #ef4444;
            --text: #f8fafc;
            --muted: #94a3b8;
        }
        * { box-sizing: border-box; }
        body {
            margin: 0;
            padding: env(safe-area-inset-top, 12px) 12px env(safe-area-inset-bottom, 80px) 12px;
            background: var(--bg);
            color: var(--text);
            font-family: ui-monospace, "SF Mono", Monaco, "Cascadia Code", "Roboto Mono", Consolas, monospace;
            font-size: 13px;
        }
        header {
            display: flex;
            flex-wrap: wrap;
            justify-content: space-between;
            align-items: center;
            padding: 12px 16px;
            background: var(--surface);
            border: 1px solid var(--cyan);
            border-radius: 8px;
            margin-bottom: 12px;
        }
        .brand {
            font-weight: 700;
            color: var(--cyan);
            letter-spacing: 0.05em;
        }
        .badge {
            display: inline-block;
            padding: 4px 8px;
            border-radius: 4px;
            font-size: 11px;
            font-weight: 700;
            background: rgba(16, 185, 129, 0.15);
            color: var(--emerald);
            border: 1px solid var(--emerald);
        }
        .grid-9 {
            display: grid;
            grid-template-columns: repeat(3, 1fr);
            gap: 10px;
        }
        .grid-12 {
            display: grid;
            grid-template-columns: repeat(4, 1fr);
            gap: 10px;
        }
        .panel-card {
            background: var(--surface);
            border: 1px solid var(--border);
            border-radius: 8px;
            padding: 12px;
            transition: border-color 0.2s ease;
        }
        .panel-card:hover { border-color: var(--cyan); }
        .panel-title {
            display: flex;
            justify-content: space-between;
            font-weight: 700;
            color: var(--cyan);
            margin-bottom: 6px;
        }
        .panel-headline { color: var(--text); margin-bottom: 8px; font-size: 12px; }
        .metric-row {
            display: flex;
            justify-content: space-between;
            color: var(--muted);
            font-size: 11px;
            padding: 2px 0;
        }
        .dock {
            position: fixed;
            bottom: 0;
            left: 0;
            right: 0;
            background: var(--surface);
            border-top: 1px solid var(--border);
            padding: 10px 12px;
            display: flex;
            gap: 8px;
            justify-content: center;
            backdrop-filter: blur(10px);
        }
        .btn {
            background: #1e293b;
            color: var(--cyan);
            border: 1px solid var(--cyan);
            border-radius: 6px;
            min-height: 48px;
            padding: 12px 16px;
            font-family: inherit;
            font-size: 12px;
            font-weight: 700;
            cursor: pointer;
        }
        .btn:active { background: var(--cyan); color: var(--bg); }
        @media (max-width: 768px) {
            .grid-9, .grid-12 { grid-template-columns: 1fr; }
            .btn { min-height: 48px; padding: 12px; flex: 1; }
        }
    </style>
</head>
<body>
    <header>
        <div class="brand" id="status">PANDORA_COCKPIT_WEB_V3010</div>
        <div>
            <span class="badge" id="health-pill">12/12 HEALTHY</span>
            <span id="digest-pill" style="margin-left:8px;color:var(--muted);font-size:11px;">SSE LIVE</span>
        </div>
    </header>
    <main id="matrix" class="grid-12"></main>
    <nav class="dock">
        <button class="btn" onclick="toggleLayout()">Toggle 9P / 12P</button>
        <button class="btn" onclick="runConsensus()">Swarm Consensus</button>
        <button class="btn" onclick="approvePending()">Approve Agent</button>
    </nav>
    <script>
        let currentLayout = "twelve_panel";
        function renderSnapshot(snap) {
            if (!snap) return;
            currentLayout = snap.layout || currentLayout;
            const matrix = document.getElementById("matrix");
            matrix.className = currentLayout === "nine_panel" ? "grid-9" : "grid-12";
            document.getElementById("health-pill").textContent =
                (snap.healthy_panels || snap.panel_count || 12) + "/" + (snap.panel_count || 12) + " HEALTHY";
            if (Array.isArray(snap.panels)) {
                matrix.innerHTML = snap.panels.map(p => `
                    <div class="panel-card">
                        <div class="panel-title"><span>${p.title}</span><span>${(p.health || "healthy").toUpperCase()}</span></div>
                        <div class="panel-headline">${p.headline || ""}</div>
                    </div>
                `).join("");
            }
        }
        async function toggleLayout() {
            const next = currentLayout === "nine_panel" ? "twelve_panel" : "nine_panel";
            const res = await fetch("/api/action/layout", {
                method: "POST",
                headers: {"Content-Type": "application/json"},
                body: JSON.stringify({layout: next})
            });
            if (res.ok) { const d = await res.json(); currentLayout = d.layout; }
        }
        async function runConsensus() {
            await fetch("/api/action/consensus", {
                method: "POST",
                headers: {"Content-Type": "application/json"},
                body: JSON.stringify({threshold: 0.50, divergence: 0.45})
            });
        }
        async function approvePending() {
            await fetch("/api/action/approve", {
                method: "POST",
                headers: {"Content-Type": "application/json"},
                body: JSON.stringify({approval_id: "appr-live", allow: true, approver_id: "mobile-operator"})
            });
        }
        fetch("/api/snapshot").then(r => r.json()).then(renderSnapshot).catch(() => {});
        if (typeof EventSource !== "undefined") {
            const es = new EventSource("/api/events");
            es.addEventListener("telemetry", e => {
                try { renderSnapshot(JSON.parse(e.data)); } catch (_) {}
            });
        }
    </script>
</body>
</html>"#;

/// Configuration for the embedded Cockpit Web Micro-Server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CockpitWebConfig {
    pub bind_addr: String,
    pub port: u16,
    pub initial_layout: CockpitLayout,
    pub sse_tick_interval_ms: u64,
}

impl Default for CockpitWebConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0".to_string(),
            port: 8080,
            initial_layout: CockpitLayout::TwelvePanel,
            sse_tick_interval_ms: DEFAULT_SSE_TICK_MS,
        }
    }
}

/// Request payload for `POST /api/action/layout`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutActionRequest {
    pub layout: String,
}

/// Request payload for `POST /api/action/consensus`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsensusActionRequest {
    pub threshold: Option<f64>,
    pub divergence: Option<f64>,
}

/// Request payload for `POST /api/action/approve`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApproveActionRequest {
    pub approval_id: String,
    pub allow: bool,
    pub approver_id: Option<String>,
}

/// Format a Server-Sent Events (SSE) frame following RFC 8895.
pub fn format_sse_event(event_type: &str, id: u64, payload: &Value) -> String {
    let data = serde_json::to_string(payload).unwrap_or_else(|_| "{}".to_string());
    format!("event: {event_type}\nid: {id}\ndata: {data}\n\n")
}

/// Embedded zero-dependency HTTP/1.1 and SSE micro-server for the Mission-Control Cockpit.
pub struct CockpitWebServer {
    config: CockpitWebConfig,
    local_addr: SocketAddr,
    listener: TcpListener,
    controller: Arc<Mutex<CockpitController>>,
    approval_store: Option<Arc<ApprovalStore>>,
    approvals_map: Arc<Mutex<BTreeMap<String, String>>>,
    sse_clients: Arc<Mutex<Vec<TcpStream>>>,
    shutdown_flag: Arc<AtomicBool>,
}

impl CockpitWebServer {
    /// Bind and initialize a new `CockpitWebServer` instance.
    pub fn new(
        port: u16,
        bind_addr: &str,
        controller: Arc<Mutex<CockpitController>>,
        approval_store: Option<Arc<ApprovalStore>>,
    ) -> IoResult<Self> {
        let addr_str = format!("{bind_addr}:{port}");
        let listener = TcpListener::bind(&addr_str)?;
        listener.set_nonblocking(true)?;
        let local_addr = listener.local_addr()?;

        let initial_layout = controller
            .lock()
            .map(|c| c.capture_snapshot(0).layout)
            .unwrap_or(CockpitLayout::TwelvePanel);

        let config = CockpitWebConfig {
            bind_addr: bind_addr.to_string(),
            port: local_addr.port(),
            initial_layout,
            sse_tick_interval_ms: DEFAULT_SSE_TICK_MS,
        };

        Ok(Self {
            config,
            local_addr,
            listener,
            controller,
            approval_store,
            approvals_map: Arc::new(Mutex::new(BTreeMap::new())),
            sse_clients: Arc::new(Mutex::new(Vec::new())),
            shutdown_flag: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Create a `CockpitWebServer` from a `CockpitWebConfig`.
    pub fn with_config(
        config: CockpitWebConfig,
        approval_store: Option<Arc<ApprovalStore>>,
    ) -> IoResult<Self> {
        let controller = Arc::new(Mutex::new(CockpitController::new(config.initial_layout)));
        let mut server = Self::new(config.port, &config.bind_addr, controller, approval_store)?;
        server.config.sse_tick_interval_ms = config.sse_tick_interval_ms.max(50);
        Ok(server)
    }

    /// Return the bound socket address (useful when bound to port `0`).
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Return the bound TCP port.
    pub fn port(&self) -> u16 {
        self.local_addr.port()
    }

    /// Return the configured bind address string.
    pub fn bind_addr(&self) -> &str {
        &self.config.bind_addr
    }

    /// Return a cloneable handle to the shutdown flag.
    pub fn shutdown_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.shutdown_flag)
    }

    /// Signal the server loop to stop cleanly.
    pub fn shutdown(&self) {
        self.shutdown_flag.store(true, Ordering::SeqCst);
    }

    /// Capture a current snapshot JSON value with status and digest metadata.
    pub fn current_snapshot_json(controller: &Arc<Mutex<CockpitController>>) -> Value {
        let now_ms = current_epoch_ms();
        let snap: CockpitSnapshot = match controller.lock() {
            Ok(guard) => guard.capture_snapshot(now_ms),
            Err(poisoned) => poisoned.into_inner().capture_snapshot(now_ms),
        };
        let mut val = serde_json::to_value(&snap).unwrap_or_else(|_| json!({}));
        if let Some(obj) = val.as_object_mut() {
            obj.insert("status".to_string(), json!("ok"));
            let digest = obj
                .get("snapshot_digest")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if !digest.starts_with("PANDORA_COCKPIT_V1:") {
                obj.insert(
                    "snapshot_digest".to_string(),
                    json!(format!("PANDORA_COCKPIT_V1:{digest}")),
                );
            }
        }
        val
    }

    /// Broadcast an SSE frame to all active client streams, pruning disconnected sockets.
    fn broadcast_sse(sse_clients: &Arc<Mutex<Vec<TcpStream>>>, event_type: &str, payload: &Value) {
        let now_ms = current_epoch_ms();
        let frame = format_sse_event(event_type, now_ms, payload);
        if let Ok(mut clients) = sse_clients.lock() {
            clients.retain_mut(|stream| {
                stream.write_all(frame.as_bytes()).is_ok()
                    && stream.write_all(b": keepalive\n\n").is_ok()
                    && stream.flush().is_ok()
            });
        }
    }

    /// Run the blocking accept loop until `shutdown()` is invoked.
    pub fn run(&self) -> IoResult<()> {
        let mut last_tick = std::time::Instant::now();
        let tick_duration = Duration::from_millis(self.config.sse_tick_interval_ms.max(50));

        while !self.shutdown_flag.load(Ordering::Relaxed) {
            match self.listener.accept() {
                Ok((stream, _peer)) => {
                    let controller = Arc::clone(&self.controller);
                    let approval_store = self.approval_store.clone();
                    let approvals_map = Arc::clone(&self.approvals_map);
                    let sse_clients = Arc::clone(&self.sse_clients);
                    thread::spawn(move || {
                        let _ = handle_connection(
                            stream,
                            controller,
                            approval_store,
                            approvals_map,
                            sse_clients,
                        );
                    });
                }
                Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(e) => return Err(e),
            }

            if last_tick.elapsed() >= tick_duration {
                let snap_val = Self::current_snapshot_json(&self.controller);
                Self::broadcast_sse(&self.sse_clients, "telemetry", &snap_val);
                last_tick = std::time::Instant::now();
            }
        }

        Ok(())
    }
}

fn current_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn handle_connection(
    mut stream: TcpStream,
    controller: Arc<Mutex<CockpitController>>,
    _approval_store: Option<Arc<ApprovalStore>>,
    approvals_map: Arc<Mutex<BTreeMap<String, String>>>,
    sse_clients: Arc<Mutex<Vec<TcpStream>>>,
) -> IoResult<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_millis(1500)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() || request_line.trim().is_empty() {
        let _ = write_json_response(
            &mut stream,
            400,
            "Bad Request",
            &json!({"error": "bad_request"}),
        );
        return Ok(());
    }

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        let _ = write_json_response(
            &mut stream,
            400,
            "Bad Request",
            &json!({"error": "malformed_request_line"}),
        );
        return Ok(());
    }

    let method = parts[0].to_ascii_uppercase();
    let raw_path = parts[1];
    let path = raw_path.split('?').next().unwrap_or(raw_path);

    // Read headers
    let mut content_length = 0usize;
    loop {
        let mut header_line = String::new();
        let bytes_read = reader.read_line(&mut header_line)?;
        if bytes_read == 0 || header_line == "\r\n" || header_line == "\n" {
            break;
        }
        let lower = header_line.to_ascii_lowercase();
        if let Some(val_str) = lower.strip_prefix("content-length:") {
            content_length = val_str.trim().parse::<usize>().unwrap_or(0);
        }
    }

    if content_length > MAX_HTTP_BODY_BYTES {
        let _ = write_json_response(
            &mut stream,
            413,
            "Payload Too Large",
            &json!({"error": "payload_too_large", "limit_bytes": MAX_HTTP_BODY_BYTES}),
        );
        // Drain (bounded) so closing the socket does not reset the client mid-upload.
        let mut remaining = content_length.min(4 * 1024 * 1024);
        let mut sink = [0u8; 8192];
        while remaining > 0 {
            match reader.read(&mut sink[..remaining.min(8192)]) {
                Ok(0) | Err(_) => break,
                Ok(n) => remaining -= n,
            }
        }
        let _ = stream.shutdown(std::net::Shutdown::Write);
        return Ok(());
    }

    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }

    match (method.as_str(), path) {
        ("OPTIONS", _) => {
            let resp = "HTTP/1.1 204 No Content\r\nServer: pandora-cockpit-web/3.0.10\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            stream.write_all(resp.as_bytes())?;
            stream.flush()?;
        }
        ("GET", "/") | ("GET", "/index.html") => {
            let html_bytes = DASHBOARD_HTML.as_bytes();
            let headers = format!(
                "HTTP/1.1 200 OK\r\nServer: pandora-cockpit-web/3.0.10\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
                html_bytes.len()
            );
            stream.write_all(headers.as_bytes())?;
            stream.write_all(html_bytes)?;
            stream.flush()?;
        }
        ("GET", "/api/snapshot") | ("GET", "/api/status") => {
            let snap_val = CockpitWebServer::current_snapshot_json(&controller);
            write_json_response(&mut stream, 200, "OK", &snap_val)?;
        }
        ("GET", "/api/events") | ("GET", "/events") => {
            let headers = "HTTP/1.1 200 OK\r\nServer: pandora-cockpit-web/3.0.10\r\nContent-Type: text/event-stream; charset=utf-8\r\nCache-Control: no-cache, no-transform\r\nConnection: keep-alive\r\nAccess-Control-Allow-Origin: *\r\nX-Accel-Buffering: no\r\n\r\n";
            stream.write_all(headers.as_bytes())?;
            let snap_val = CockpitWebServer::current_snapshot_json(&controller);
            let now_ms = current_epoch_ms();
            let first_event = format_sse_event("telemetry", now_ms, &snap_val);
            stream.write_all(first_event.as_bytes())?;
            stream.write_all(b": keepalive\n\n")?;
            stream.flush()?;

            if let Ok(cloned) = stream.try_clone()
                && let Ok(mut clients) = sse_clients.lock()
            {
                clients.push(cloned);
            }
        }
        ("POST", "/api/action/layout") => match serde_json::from_slice::<Value>(&body) {
            Ok(json_body) => {
                if let Some(target_str) = json_body.get("layout").and_then(Value::as_str) {
                    if let Some(new_layout) = CockpitLayout::parse(target_str) {
                        if let Ok(mut guard) = controller.lock() {
                            *guard = CockpitController::new(new_layout);
                        }
                        let snap_val = CockpitWebServer::current_snapshot_json(&controller);
                        CockpitWebServer::broadcast_sse(&sse_clients, "telemetry", &snap_val);
                        let layout_name = match new_layout {
                            CockpitLayout::NinePanel => "nine_panel",
                            CockpitLayout::TwelvePanel => "twelve_panel",
                        };
                        let resp_val = json!({
                            "status": "ok",
                            "layout": layout_name,
                            "panel_count": new_layout.panel_count(),
                            "timestamp_ms": current_epoch_ms(),
                        });
                        write_json_response(&mut stream, 200, "OK", &resp_val)?;
                    } else {
                        write_json_response(
                            &mut stream,
                            400,
                            "Bad Request",
                            &json!({"error": "invalid_layout", "expected": ["nine_panel", "twelve_panel", "9", "12"]}),
                        )?;
                    }
                } else {
                    write_json_response(
                        &mut stream,
                        400,
                        "Bad Request",
                        &json!({"error": "missing_layout_field"}),
                    )?;
                }
            }
            Err(_) => {
                write_json_response(
                    &mut stream,
                    400,
                    "Bad Request",
                    &json!({"error": "invalid_json"}),
                )?;
            }
        },
        ("POST", "/api/action/consensus") => {
            let json_body: Value = if body.is_empty() {
                json!({})
            } else {
                match serde_json::from_slice(&body) {
                    Ok(v) => v,
                    Err(_) => {
                        write_json_response(
                            &mut stream,
                            400,
                            "Bad Request",
                            &json!({"error": "invalid_json"}),
                        )?;
                        return Ok(());
                    }
                }
            };

            let threshold = json_body
                .get("threshold")
                .and_then(Value::as_f64)
                .unwrap_or(0.50);
            let divergence = json_body
                .get("divergence")
                .and_then(Value::as_f64)
                .unwrap_or(0.45);

            let engine = SwarmCodeConsensusEngine::new(threshold, divergence);
            let proposals = default_consensus_proposals();
            let now_ms = current_epoch_ms();

            match engine.resolve_consensus(&proposals, now_ms) {
                Ok(decision) => {
                    let resp_val = json!({
                        "status": "consensus_achieved",
                        "winning_proposal_id": decision.winning_proposal_id,
                        "winning_agent": decision.winning_agent.identifier(),
                        "winning_title": decision.winning_title,
                        "consensus_confidence": decision.consensus_confidence,
                        "total_proposals": decision.total_proposals,
                        "honest_quorum_size": decision.honest_quorum_size,
                        "byzantine_filtered_count": decision.byzantine_filtered_count,
                        "proposal_verdicts": decision.proposal_verdicts,
                        "consensus_hash": decision.consensus_hash,
                        "timestamp_ms": decision.timestamp_ms,
                    });
                    CockpitWebServer::broadcast_sse(&sse_clients, "consensus", &resp_val);
                    write_json_response(&mut stream, 200, "OK", &resp_val)?;
                }
                Err(e) => {
                    write_json_response(
                        &mut stream,
                        422,
                        "Unprocessable Entity",
                        &json!({"error": "consensus_failed", "message": e}),
                    )?;
                }
            }
        }
        ("POST", "/api/action/approve") => match serde_json::from_slice::<Value>(&body) {
            Ok(json_body) => {
                let Some(approval_id) = json_body.get("approval_id").and_then(Value::as_str) else {
                    write_json_response(
                        &mut stream,
                        400,
                        "Bad Request",
                        &json!({"error": "missing_approval_id"}),
                    )?;
                    return Ok(());
                };
                if approval_id.trim().is_empty() {
                    write_json_response(
                        &mut stream,
                        400,
                        "Bad Request",
                        &json!({"error": "empty_approval_id"}),
                    )?;
                    return Ok(());
                }
                let allow = json_body
                    .get("allow")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let approver_id = json_body
                    .get("approver_id")
                    .and_then(Value::as_str)
                    .unwrap_or("local-user");

                let verdict = if allow { "approved" } else { "denied" };
                if let Ok(mut map) = approvals_map.lock() {
                    map.insert(approval_id.to_string(), verdict.to_string());
                }

                let now_ms = current_epoch_ms();
                let resp_val = json!({
                    "status": "resolved",
                    "approval_id": approval_id,
                    "verdict": verdict,
                    "approver_id": approver_id,
                    "timestamp_ms": now_ms,
                });
                CockpitWebServer::broadcast_sse(&sse_clients, "permission", &resp_val);
                write_json_response(&mut stream, 200, "OK", &resp_val)?;
            }
            Err(_) => {
                write_json_response(
                    &mut stream,
                    400,
                    "Bad Request",
                    &json!({"error": "invalid_json"}),
                )?;
            }
        },
        ("POST", "/api/action/dispatch") => match serde_json::from_slice::<Value>(&body) {
            Ok(json_body) => {
                let command = json_body
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let mut manager = MultiAgentCollabManager::new();
                match manager.dispatch(command) {
                    Ok(res) => {
                        let resp_val = json!({
                            "status": "dispatched",
                            "command_line": res.command_line,
                            "spawned_agents": res.spawned_agents,
                            "pexer_commands": res.pexer_ipc_commands,
                            "explanation": res.explanation,
                            "timestamp_ms": current_epoch_ms(),
                        });
                        write_json_response(&mut stream, 200, "OK", &resp_val)?;
                    }
                    Err(e) => {
                        write_json_response(
                            &mut stream,
                            400,
                            "Bad Request",
                            &json!({"error": "dispatch_failed", "message": e}),
                        )?;
                    }
                }
            }
            Err(_) => {
                write_json_response(
                    &mut stream,
                    400,
                    "Bad Request",
                    &json!({"error": "invalid_json"}),
                )?;
            }
        },
        _ => {
            write_json_response(
                &mut stream,
                404,
                "Not Found",
                &json!({"error": "not_found", "path": path}),
            )?;
        }
    }

    let _ = stream.shutdown(std::net::Shutdown::Write);
    Ok(())
}

fn write_json_response(
    stream: &mut TcpStream,
    status_code: u16,
    reason: &str,
    payload: &Value,
) -> IoResult<()> {
    let body_str = serde_json::to_string(payload)
        .map_err(|e| IoError::new(ErrorKind::InvalidData, e.to_string()))?;
    let resp = format!(
        "HTTP/1.1 {status_code} {reason}\r\nServer: pandora-cockpit-web/3.0.10\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{}",
        body_str.len(),
        body_str
    );
    stream.write_all(resp.as_bytes())?;
    stream.flush()
}

fn default_consensus_proposals() -> Vec<AgentPatchProposal> {
    vec![
        AgentPatchProposal::new(
            "p_agy",
            ExternalAgentKind::Agy,
            "Refactor auth token hygiene & zeroize",
            "Applies constant-time comparison and memory zeroization",
            "diff --git a/src/auth.rs\n+ subtle::ConstantTimeEq\n+ zeroize::Zeroize",
            ProposalScoreVector::new(1.0, 0.98, 0.95, 1.0),
            120,
        ),
        AgentPatchProposal::new(
            "p_claude",
            ExternalAgentKind::ClaudeCode,
            "Refactor auth with zeroize and constant-time check",
            "Guarantees side-channel resistance and zeroization on drop",
            "diff --git a/src/auth.rs\n+ subtle::ConstantTimeEq",
            ProposalScoreVector::new(0.95, 0.96, 0.90, 0.95),
            140,
        ),
        AgentPatchProposal::new(
            "p_codex",
            ExternalAgentKind::CodexCli,
            "Add auth token verification cache",
            "Implements bounded cache for token checks",
            "diff --git a/src/auth.rs\n+ token_cache.insert()",
            ProposalScoreVector::new(0.92, 0.90, 0.88, 0.90),
            130,
        ),
        AgentPatchProposal::new(
            "p_byzantine",
            ExternalAgentKind::Hermes,
            "Disable token checks for performance",
            "Bypasses invariant assertion and removes verification logic",
            "diff --git a/src/auth.rs\n- verify_token(token)?",
            ProposalScoreVector::new(0.10, 0.20, 0.15, 0.05),
            200,
        ),
    ]
}
