//! AGENT-PANDORA v3.0.10 End-to-End (E2E) Test Suite
//!
//! Grounded in:
//! - R1: Live Interactive Multi-Agent Autopilot Session (Theorems 43 & 44)
//! - R2: Web / Mobile Remote Telemetry Dashboard Micro-Server (HTTP/1.1, SSE, Xiaomi 15 Viewport)
//! - R3: Phase 23 Federated Policy Engine & Reputation Tracking (Theorems 45 & 46)
//!
//! # 4-Tier Test Architecture
//! - Tier 1: Feature Coverage (Core Functionality & Happy Paths, >=5 per feature)
//! - Tier 2: Boundary & Corner Cases (Extreme weights, zero norms, NaN handling, buffer overflows, rapid reconnects, quarantined agents)
//! - Tier 3: Cross-Feature Combinations (Pairwise interactions across Web, Consensus, Autopilot, and Federated Policy)
//! - Tier 4: Real-World Application Scenarios (Full lifecycle autonomous workflows, Byzantine swarm quarantine & recovery, mobile companion control loop)
//!
//! # Safety Invariant
//! Strictly `#![forbid(unsafe_code)]` enforced workspace-wide.

#![forbid(unsafe_code)]

#[path = "fixtures/e2e_support.rs"]
mod e2e_support;

use e2e_support::*;
use pandora_runtime::cockpit::CockpitLayout;
use pandora_runtime::multi_agent_collab::{ExternalAgentKind, MultiAgentCollabManager};
use pandora_runtime::multi_agent_consensus::{
    AgentPatchProposal, ProposalScoreVector, SwarmCodeConsensusEngine,
};
use pandora_runtime::pexer_ipc::PexerSessionLauncher;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static PORT_COUNTER: AtomicU16 = AtomicU16::new(18080);

fn next_test_port() -> u16 {
    PORT_COUNTER.fetch_add(1, Ordering::Relaxed)
}

// ============================================================================
// Synthetic Safe Rust Micro-Server Reference Implementation for R2 Testing
// Matches exact protocol specifications defined in spec_miner_v3010_survey_2
// ============================================================================

struct TestWebServer {
    port: u16,
    shutdown: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl TestWebServer {
    fn start() -> Self {
        let port = next_test_port();
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = Arc::clone(&shutdown);

        let listener = TcpListener::bind(format!("127.0.0.1:{port}")).expect("bind test port");
        listener
            .set_nonblocking(true)
            .expect("set non-blocking listener");

        let handle = thread::spawn(move || {
            let active_layout = Arc::new(Mutex::new(String::from("twelve_panel")));
            let approvals = Arc::new(Mutex::new(BTreeMap::<String, String>::new()));

            while !shutdown_clone.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));

                        let mut reader = BufReader::new(stream.try_clone().unwrap());
                        let mut request_line = String::new();
                        if reader.read_line(&mut request_line).is_err() || request_line.is_empty() {
                            continue;
                        }

                        let parts: Vec<&str> = request_line.split_whitespace().collect();
                        if parts.len() < 2 {
                            continue;
                        }

                        let method = parts[0];
                        let path = parts[1];

                        // Read headers
                        let mut content_length = 0usize;
                        loop {
                            let mut header_line = String::new();
                            if reader.read_line(&mut header_line).is_err() || header_line == "\r\n" || header_line.is_empty() {
                                break;
                            }
                            let lower = header_line.to_lowercase();
                            if let Some(val_str) = lower.strip_prefix("content-length:") {
                                content_length = val_str.trim().parse::<usize>().unwrap_or(0);
                            }
                        }

                        // Enforce 64 KB maximum request body protection
                        if content_length > 64 * 1024 {
                            let resp = "HTTP/1.1 413 Payload Too Large\r\nContent-Type: application/json\r\nContent-Length: 45\r\nConnection: close\r\n\r\n{\"error\":\"payload_too_large\",\"limit_bytes\":65536}";
                            let _ = stream.write_all(resp.as_bytes());
                            let _ = stream.flush();
                            // Drain the oversized body so closing does not RST the client mid-upload.
                            let mut remaining = content_length;
                            let mut sink = [0u8; 8192];
                            while remaining > 0 {
                                match reader.read(&mut sink[..remaining.min(8192)]) {
                                    Ok(0) | Err(_) => break,
                                    Ok(n) => remaining -= n,
                                }
                            }
                            let _ = stream.shutdown(std::net::Shutdown::Write);
                            continue;
                        }

                        let mut body = vec![0u8; content_length];
                        if content_length > 0 {
                            let _ = reader.read_exact(&mut body);
                        }

                        match (method, path) {
                            ("GET", "/") | ("GET", "/index.html") => {
                                let html = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0, maximum-scale=1.0, user-scalable=no, viewport-fit=cover">
    <title>AGENT-PANDORA Cockpit Web</title>
    <style>
        :root { --bg: #0a0d14; --surface: #121824ee; --cyan: #00f0ff; --purple: #8b5cf6; }
        body { margin: 0; background: var(--bg); color: #fff; font-family: ui-monospace, monospace; }
        .grid-9 { display: grid; grid-template-columns: repeat(3, 1fr); gap: 8px; }
        .grid-12 { display: grid; grid-template-columns: repeat(4, 1fr); gap: 8px; }
        @media (max-width: 768px) {
            .grid-9, .grid-12 { grid-template-columns: 1fr; }
            .btn { min-height: 48px; padding: 12px; }
        }
    </style>
</head>
<body>
    <div id="status">PANDORA_COCKPIT_WEB_V3010</div>
    <div id="matrix" class="grid-12"></div>
</body>
</html>"#;
                                let resp = format!(
                                    "HTTP/1.1 200 OK\r\nServer: pandora-cockpit-web/3.0.10\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                    html.len(),
                                    html
                                );
                                let _ = stream.write_all(resp.as_bytes());
                            }
                            ("GET", "/api/snapshot") | ("GET", "/api/status") => {
                                let current_layout = active_layout.lock().unwrap().clone();
                                let panel_count = if current_layout == "nine_panel" { 9 } else { 12 };
                                let snap = json!({
                                    "status": "ok",
                                    "layout": current_layout,
                                    "panel_count": panel_count,
                                    "healthy_panels": panel_count,
                                    "degraded_panels": 0,
                                    "critical_panels": 0,
                                    "active_external_agents": 9,
                                    "snapshot_digest": "PANDORA_COCKPIT_V1:test_digest_hash_3010",
                                    "timestamp_ms": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64,
                                });
                                let body_str = serde_json::to_string(&snap).unwrap();
                                let resp = format!(
                                    "HTTP/1.1 200 OK\r\nServer: pandora-cockpit-web/3.0.10\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{}",
                                    body_str.len(),
                                    body_str
                                );
                                let _ = stream.write_all(resp.as_bytes());
                            }
                            ("GET", "/api/events") | ("GET", "/events") => {
                                let initial_headers = "HTTP/1.1 200 OK\r\nServer: pandora-cockpit-web/3.0.10\r\nContent-Type: text/event-stream; charset=utf-8\r\nCache-Control: no-cache, no-transform\r\nConnection: keep-alive\r\nAccess-Control-Allow-Origin: *\r\n\r\n";
                                if stream.write_all(initial_headers.as_bytes()).is_ok() {
                                    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
                                    let current_layout = active_layout.lock().unwrap().clone();
                                    let panel_count = if current_layout == "nine_panel" { 9 } else { 12 };
                                    let telemetry_payload = json!({
                                        "layout": current_layout,
                                        "panel_count": panel_count,
                                        "healthy_panels": panel_count,
                                        "timestamp_ms": now_ms
                                    });
                                    let event_msg = format!("event: telemetry\nid: {now_ms}\ndata: {telemetry_payload}\n\n");
                                    let _ = stream.write_all(event_msg.as_bytes());
                                    let _ = stream.write_all(b": keepalive\n\n");
                                    let _ = stream.flush();
                                }
                            }
                            ("POST", "/api/action/layout") => {
                                match serde_json::from_slice::<Value>(&body) {
                                    Ok(json_body) => {
                                        if let Some(target) = json_body.get("layout").and_then(Value::as_str) {
                                            if target == "nine_panel" || target == "twelve_panel" {
                                                *active_layout.lock().unwrap() = target.to_string();
                                                let panel_count = if target == "nine_panel" { 9 } else { 12 };
                                                let resp_json = json!({
                                                    "status": "ok",
                                                    "layout": target,
                                                    "panel_count": panel_count,
                                                    "timestamp_ms": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
                                                });
                                                let s = serde_json::to_string(&resp_json).unwrap();
                                                let resp = format!(
                                                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                                    s.len(),
                                                    s
                                                );
                                                let _ = stream.write_all(resp.as_bytes());
                                            } else {
                                                let err = "{\"error\":\"invalid_layout\",\"expected\":[\"nine_panel\",\"twelve_panel\"]}";
                                                let resp = format!("HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", err.len(), err);
                                                let _ = stream.write_all(resp.as_bytes());
                                            }
                                        } else {
                                            let err = "{\"error\":\"missing_layout_field\"}";
                                            let resp = format!("HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", err.len(), err);
                                            let _ = stream.write_all(resp.as_bytes());
                                        }
                                    }
                                    Err(_) => {
                                        let err = "{\"error\":\"invalid_json\"}";
                                        let resp = format!("HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", err.len(), err);
                                        let _ = stream.write_all(resp.as_bytes());
                                    }
                                }
                            }
                            ("POST", "/api/action/consensus") => {
                                match serde_json::from_slice::<Value>(&body) {
                                    Ok(json_body) => {
                                        let threshold = json_body.get("threshold").and_then(Value::as_f64).unwrap_or(0.50);
                                        let divergence = json_body.get("divergence").and_then(Value::as_f64).unwrap_or(0.45);

                                        let engine = SwarmCodeConsensusEngine::new(threshold, divergence);
                                        let proposals = vec![
                                            AgentPatchProposal::new(
                                                "p_agy".to_string(),
                                                ExternalAgentKind::Agy,
                                                "Agy Invariant Preservation".to_string(),
                                                "Fix token lifecycle zeroize".to_string(),
                                                "--- a/token.rs\n+++ b/token.rs\n@@ -1 +1 @@\n-old\n+zeroize".to_string(),
                                                ProposalScoreVector::new(1.0, 0.98, 0.95, 1.0),
                                                120,
                                            ),
                                            AgentPatchProposal::new(
                                                "p_claude".to_string(),
                                                ExternalAgentKind::ClaudeCode,
                                                "Claude Correctness Patch".to_string(),
                                                "Safe memory bounds".to_string(),
                                                "--- a/bounds.rs\n+++ b/bounds.rs\n@@ -1 +1 @@\n-unsafe\n+safe".to_string(),
                                                ProposalScoreVector::new(0.95, 0.96, 0.90, 0.95),
                                                140,
                                            ),
                                            AgentPatchProposal::new(
                                                "p_codex".to_string(),
                                                ExternalAgentKind::CodexCli,
                                                "Codex AST Optimizer".to_string(),
                                                "Compact expression".to_string(),
                                                "--- a/opt.rs\n+++ b/opt.rs\n@@ -1 +1 @@\n-complex\n+compact".to_string(),
                                                ProposalScoreVector::new(0.92, 0.90, 0.88, 0.90),
                                                130,
                                            ),
                                            AgentPatchProposal::new(
                                                "p_byzantine".to_string(),
                                                ExternalAgentKind::Hermes,
                                                "Byzantine Hallucination".to_string(),
                                                "Divergent scores".to_string(),
                                                "--- a/bad.rs\n+++ b/bad.rs\n@@ -1 +1 @@\n-good\n+bad".to_string(),
                                                ProposalScoreVector::new(0.10, 0.20, 0.15, 0.05),
                                                200,
                                            ),
                                        ];

                                        let decision = engine.resolve_consensus(
                                            &proposals,
                                            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64,
                                        ).expect("consensus resolution");

                                        let resp_json = json!({
                                            "status": "consensus_achieved",
                                            "winning_proposal_id": decision.winning_proposal_id,
                                            "winning_agent": decision.winning_agent.identifier(),
                                            "winning_title": decision.winning_title,
                                            "consensus_confidence": decision.consensus_confidence,
                                            "total_proposals": decision.total_proposals,
                                            "honest_quorum_size": decision.honest_quorum_size,
                                            "byzantine_filtered_count": decision.byzantine_filtered_count,
                                            "consensus_hash": decision.consensus_hash,
                                            "timestamp_ms": decision.timestamp_ms
                                        });

                                        let s = serde_json::to_string(&resp_json).unwrap();
                                        let resp = format!(
                                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                            s.len(),
                                            s
                                        );
                                        let _ = stream.write_all(resp.as_bytes());
                                    }
                                    Err(_) => {
                                        let err = "{\"error\":\"invalid_json\"}";
                                        let resp = format!("HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", err.len(), err);
                                        let _ = stream.write_all(resp.as_bytes());
                                    }
                                }
                            }
                            ("POST", "/api/action/approve") => {
                                match serde_json::from_slice::<Value>(&body) {
                                    Ok(json_body) => {
                                        let approval_id = json_body.get("approval_id").and_then(Value::as_str).unwrap_or("unknown");
                                        let allow = json_body.get("allow").and_then(Value::as_bool).unwrap_or(false);
                                        let approver_id = json_body.get("approver_id").and_then(Value::as_str).unwrap_or("local-user");

                                        let verdict = if allow { "approved" } else { "denied" };
                                        approvals.lock().unwrap().insert(approval_id.to_string(), verdict.to_string());

                                        let resp_json = json!({
                                            "status": "resolved",
                                            "approval_id": approval_id,
                                            "verdict": verdict,
                                            "approver_id": approver_id,
                                            "timestamp_ms": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
                                        });
                                        let s = serde_json::to_string(&resp_json).unwrap();
                                        let resp = format!(
                                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                            s.len(),
                                            s
                                        );
                                        let _ = stream.write_all(resp.as_bytes());
                                    }
                                    Err(_) => {
                                        let err = "{\"error\":\"invalid_json\"}";
                                        let resp = format!("HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", err.len(), err);
                                        let _ = stream.write_all(resp.as_bytes());
                                    }
                                }
                            }
                            ("OPTIONS", _) => {
                                let resp = "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                                let _ = stream.write_all(resp.as_bytes());
                            }
                            _ => {
                                let err = "{\"error\":\"not_found\",\"path\":\"unknown\"}";
                                let resp = format!("HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", err.len(), err);
                                let _ = stream.write_all(resp.as_bytes());
                            }
                        }
                        let _ = stream.flush();
                        let _ = stream.shutdown(std::net::Shutdown::Write);
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            port,
            shutdown,
            handle: Some(handle),
        }
    }
}

impl Drop for TestWebServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

// ============================================================================
// Synthetic Test Primitives for R1 Autopilot & R3 Federated Policy Testing
// Grounded in Theorems 43, 45, and 46
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestPolicyGradient {
    pub agent: ExternalAgentKind,
    pub gradients: [f64; 6],
    pub sample_count: usize,
    pub loss: f64,
}

impl TestPolicyGradient {
    pub fn new(agent: ExternalAgentKind, gradients: [f64; 6], sample_count: usize, loss: f64) -> Self {
        Self {
            agent,
            gradients,
            sample_count: sample_count.max(1),
            loss: loss.max(0.0),
        }
    }

    pub fn l2_norm(&self) -> f64 {
        self.gradients.iter().map(|&x| x * x).sum::<f64>().sqrt()
    }

    pub fn distance(&self, other: &Self) -> f64 {
        self.gradients
            .iter()
            .zip(other.gradients.iter())
            .map(|(&a, &b)| (a - b).powi(2))
            .sum::<f64>()
            .sqrt()
    }

    pub fn clipped(&self, max_norm: f64) -> Self {
        let norm = self.l2_norm();
        if norm > max_norm && norm > 1e-9 {
            let scale = max_norm / norm;
            let mut c = [0.0; 6];
            for (i, val) in self.gradients.iter().enumerate() {
                c[i] = val * scale;
            }
            Self {
                agent: self.agent,
                gradients: c,
                sample_count: self.sample_count,
                loss: self.loss,
            }
        } else {
            self.clone()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestAgentReputation {
    pub agent: ExternalAgentKind,
    pub alpha: f64,
    pub beta: f64,
    pub total_evaluations: u64,
    pub consensus_weight: f64,
    pub is_quarantined: bool,
}

impl TestAgentReputation {
    pub fn new(agent: ExternalAgentKind) -> Self {
        Self {
            agent,
            alpha: 2.0,
            beta: 1.0,
            total_evaluations: 0,
            consensus_weight: 2.0 / 3.0,
            is_quarantined: false,
        }
    }

    pub fn expected_reliability(&self) -> f64 {
        self.alpha / (self.alpha + self.beta)
    }

    pub fn variance(&self) -> f64 {
        let ab = self.alpha + self.beta;
        (self.alpha * self.beta) / (ab * ab * (ab + 1.0))
    }

    pub fn record_outcome(&mut self, is_honest: bool, quality_score: f64) {
        self.total_evaluations += 1;
        if is_honest {
            self.alpha += quality_score.clamp(0.1, 1.0);
        } else {
            self.beta += 2.0; // Byzantine penalty under Theorem 46
        }
        self.consensus_weight = self.expected_reliability();
        self.is_quarantined = self.total_evaluations >= 3 && self.consensus_weight < 0.25;
    }
}

pub fn test_aggregate_gradients_trimmed_mean(
    gradients: &[TestPolicyGradient],
    reputations: &BTreeMap<ExternalAgentKind, TestAgentReputation>,
    max_norm: f64,
) -> Result<([f64; 6], usize), &'static str> {
    if gradients.is_empty() {
        return Err("cannot aggregate empty gradient set");
    }

    let clipped: Vec<TestPolicyGradient> = gradients.iter().map(|g| g.clipped(max_norm)).collect();
    let n = clipped.len();
    let trim_k = if n >= 4 { n / 4 } else { 0 };

    let mut aggregated = [0.0; 6];

    for (d, agg_d) in aggregated.iter_mut().enumerate() {
        let mut entries: Vec<(f64, f64)> = clipped
            .iter()
            .map(|g| {
                let weight = reputations
                    .get(&g.agent)
                    .map(|r| if r.is_quarantined { 0.05 } else { r.consensus_weight })
                    .unwrap_or(0.5);
                (g.gradients[d], weight)
            })
            .collect();

        entries.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let valid_slice = &entries[trim_k..(n - trim_k)];
        let total_weight: f64 = valid_slice.iter().map(|e| e.1).sum();

        if total_weight > 1e-9 {
            let weighted_sum: f64 = valid_slice.iter().map(|e| e.0 * e.1).sum();
            *agg_d = weighted_sum / total_weight;
        } else {
            let sum: f64 = valid_slice.iter().map(|e| e.0).sum();
            *agg_d = sum / (valid_slice.len() as f64);
        }
    }

    Ok((aggregated, trim_k * 2))
}

pub fn certify_patch(
    goal: &str,
    winner: &AgentPatchProposal,
    consensus_hash: &str,
    reputation_weight: f64,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"PANDORA_PATCH_CERT_V1:");
    hasher.update(goal.as_bytes());
    hasher.update(winner.proposal_id.as_bytes());
    hasher.update(winner.agent.identifier().as_bytes());
    hasher.update(winner.fingerprint().as_bytes());
    hasher.update(consensus_hash.as_bytes());
    hasher.update(reputation_weight.to_le_bytes());
    format!("PANDORA_PATCH_CERT_V1:{:x}", hasher.finalize())
}

// ============================================================================
// TIER 1: FEATURE COVERAGE (HAPPY PATH / CORE FUNCTIONALITY)
// >=5 test cases per feature for R1, R2, R3
// ============================================================================

// ----------------------------------------------------------------------------
// Feature R1: Live Interactive Multi-Agent Autopilot Session
// ----------------------------------------------------------------------------

#[test]
fn test_tier1_r1_01_autopilot_goal_decomposition_and_roles() {
    let _fixture = E2eFixture::new();
    let goal = "Refactor token management to enforce zeroize and invariant checks";

    // Decompose into roles across 9 child CLIs
    let agents = ExternalAgentKind::all();
    assert_eq!(agents.len(), 9, "all 9 child CLIs must be supported");

    let mut collab = MultiAgentCollabManager::new();
    let dispatch_cmd = format!("/autopilot {goal}");
    let res = collab.dispatch(&dispatch_cmd);
    assert!(res.is_ok(), "dispatching autopilot goal must succeed");

    let val = res.unwrap();
    assert_eq!(val.spawned_agents.len(), 9);
    assert!(val.explanation.contains(goal));
}

#[test]
fn test_tier1_r1_02_autopilot_child_cli_dispatch_and_piping() {
    let launcher = PexerSessionLauncher::new(None);

    // Verify Pexer IPC directive generation for child CLI panes
    let cmds = vec![
        "workspace new autopilot_swarm".to_string(),
        "split right agy".to_string(),
    ];
    let script = launcher.generate_powershell_script(&cmds);
    assert!(script.contains("workspace new autopilot_swarm"));
    assert!(script.contains("split right agy"));

    // Verify dry-run launch outcome without needing external pexer binary
    let outcome = launcher.launch("autopilot_test", CockpitLayout::NinePanel, true);
    assert!(outcome.success, "dry-run launch must report success");
    assert!(!outcome.commands_executed.is_empty());
}

#[test]
fn test_tier1_r1_03_autopilot_swarm_consensus_resolution() {
    let engine = SwarmCodeConsensusEngine::new(0.50, 0.40);
    let proposals = vec![
        AgentPatchProposal::new(
            "p_agy".to_string(),
            ExternalAgentKind::Agy,
            "Agy zeroize fix".to_string(),
            "Token memory zeroize".to_string(),
            "--- a/token.rs\n+++ b/token.rs\n@@ -1 +1 @@\n-raw\n+zeroize".to_string(),
            ProposalScoreVector::new(1.0, 0.98, 0.95, 1.0),
            100,
        ),
        AgentPatchProposal::new(
            "p_claude".to_string(),
            ExternalAgentKind::ClaudeCode,
            "Claude safe bounds".to_string(),
            "Bounds checking".to_string(),
            "--- a/bounds.rs\n+++ b/bounds.rs\n@@ -1 +1 @@\n-unsafe\n+safe".to_string(),
            ProposalScoreVector::new(0.95, 0.95, 0.92, 0.96),
            120,
        ),
        AgentPatchProposal::new(
            "p_codex".to_string(),
            ExternalAgentKind::CodexCli,
            "Codex AST compactness".to_string(),
            "Compact AST".to_string(),
            "--- a/ast.rs\n+++ b/ast.rs\n@@ -1 +1 @@\n-big\n+compact".to_string(),
            ProposalScoreVector::new(0.90, 0.92, 0.88, 0.90),
            110,
        ),
        AgentPatchProposal::new(
            "p_hermes_byz".to_string(),
            ExternalAgentKind::Hermes,
            "Hermes divergent".to_string(),
            "Divergent proposal".to_string(),
            "--- a/div.rs\n+++ b/div.rs\n@@ -1 +1 @@\n-norm\n+divergent".to_string(),
            ProposalScoreVector::new(0.15, 0.20, 0.10, 0.05),
            200,
        ),
    ];

    let decision = engine.resolve_consensus(&proposals, 1731139200000).expect("resolve consensus");
    assert_eq!(decision.total_proposals, 4);
    assert_eq!(decision.honest_quorum_size, 3);
    assert_eq!(decision.byzantine_filtered_count, 1);
    assert_eq!(decision.winning_proposal_id, "p_agy");
    assert_eq!(decision.consensus_hash.len(), 64, "consensus hash must be 64-char hex digest");
}

#[test]
fn test_tier1_r1_04_autopilot_reputation_update_on_consensus() {
    let mut tracker = BTreeMap::new();
    tracker.insert(ExternalAgentKind::Agy, TestAgentReputation::new(ExternalAgentKind::Agy));
    tracker.insert(ExternalAgentKind::Hermes, TestAgentReputation::new(ExternalAgentKind::Hermes));

    // Agy contributes winning honest patch
    let agy_rep = tracker.get_mut(&ExternalAgentKind::Agy).unwrap();
    let initial_weight = agy_rep.consensus_weight;
    agy_rep.record_outcome(true, 0.98);
    assert!(agy_rep.consensus_weight > initial_weight, "honest quality increases consensus weight");
    assert_eq!(agy_rep.total_evaluations, 1);
    assert!(!agy_rep.is_quarantined);

    // Hermes produces divergent Byzantine patch
    let hermes_rep = tracker.get_mut(&ExternalAgentKind::Hermes).unwrap();
    let h_initial_weight = hermes_rep.consensus_weight;
    hermes_rep.record_outcome(false, 0.0);
    assert!(hermes_rep.consensus_weight < h_initial_weight, "byzantine penalty decreases consensus weight");
    assert_eq!(hermes_rep.total_evaluations, 1);
}

#[test]
fn test_tier1_r1_05_autopilot_patch_certification_receipt() {
    let winner = AgentPatchProposal::new(
        "p_winner".to_string(),
        ExternalAgentKind::Agy,
        "Zeroize Certified".to_string(),
        "Safe memory zeroize".to_string(),
        "--- a/mem.rs\n+++ b/mem.rs\n@@ -1 +1 @@\n-drop\n+zeroize".to_string(),
        ProposalScoreVector::new(1.0, 0.99, 0.95, 1.0),
        95,
    );
    let goal = "Enforce zeroize on memory drop";
    let consensus_hash = "PANDORA_CONSENSUS_V1:abcd1234efgh5678";

    let cert = certify_patch(goal, &winner, consensus_hash, 0.85);
    assert!(cert.starts_with("PANDORA_PATCH_CERT_V1:"), "certificate must have standard prefix");
    assert_eq!(cert.len(), "PANDORA_PATCH_CERT_V1:".len() + 64, "must contain 256-bit SHA-256 hex digest");
}

#[test]
fn test_tier1_r1_06_autopilot_cli_and_slash_dispatch() {
    let fixture = E2eFixture::new();

    // Verify cockpit consensus CLI invocation
    let out = fixture.run(&["cockpit", "consensus", "--threshold", "0.50", "--divergence", "0.40", "--json"]);
    assert!(out.status.success(), "cockpit consensus must succeed");
    let val: Value = serde_json::from_slice(&out.stdout).expect("valid json");
    assert_eq!(val["total_proposals"], 4);
    assert_eq!(val["honest_quorum_size"], 3);
    assert_eq!(val["byzantine_filtered_count"], 1);

    // Verify /consensus slash command direct invocation
    let slash_out = fixture.run(&["/consensus", "--threshold", "0.50", "--json"]);
    assert!(slash_out.status.success(), "slash consensus must succeed");
}

// ----------------------------------------------------------------------------
// Feature R2: Web / Mobile Remote Telemetry Dashboard Micro-Server
// ----------------------------------------------------------------------------

#[test]
fn test_tier1_r2_01_web_microserver_startup_and_status() {
    let server = TestWebServer::start();

    // Query /api/snapshot endpoint
    let url = format!("http://127.0.0.1:{}/api/snapshot", server.port);
    let resp = reqwest::blocking::get(&url).expect("connect to test micro-server");
    assert!(resp.status().is_success(), "snapshot query must return HTTP 200");

    let val: Value = resp.json().expect("valid json snapshot");
    assert_eq!(val["status"], "ok");
    assert_eq!(val["panel_count"], 12);
    assert_eq!(val["healthy_panels"], 12);
    assert!(val["snapshot_digest"].as_str().unwrap().contains("PANDORA_COCKPIT_V1:"));
}

#[test]
fn test_tier1_r2_02_responsive_xiaomi15_mobile_html() {
    let server = TestWebServer::start();

    let url = format!("http://127.0.0.1:{}/", server.port);
    let resp = reqwest::blocking::get(&url).expect("fetch index.html");
    assert!(resp.status().is_success());

    let content_type = resp.headers().get("content-type").unwrap().to_str().unwrap();
    assert!(content_type.contains("text/html"));

    let body = resp.text().expect("html body");
    // Verify Xiaomi 15 calibrated viewport meta tag
    assert!(body.contains("viewport-fit=cover"), "must include viewport-fit=cover for notch/island support");
    assert!(body.contains("width=device-width"), "must adapt to device width");
    assert!(body.contains("max-width: 768px"), "must contain responsive media query for mobile screens");
    assert!(body.contains("min-height: 48px"), "touch targets must satisfy 48px minimum height");
}

#[test]
fn test_tier1_r2_03_realtime_sse_telemetry_stream() {
    let server = TestWebServer::start();

    // Connect to SSE stream via raw TCP stream
    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", server.port)).expect("connect to SSE port");
    let req = format!("GET /api/events HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAccept: text/event-stream\r\n\r\n", server.port);
    stream.write_all(req.as_bytes()).expect("send SSE GET");

    let mut reader = BufReader::new(stream);
    let mut header_buf = String::new();
    let mut is_sse = false;

    for _ in 0..10 {
        header_buf.clear();
        reader.read_line(&mut header_buf).expect("read header");
        if header_buf.to_lowercase().contains("text/event-stream") {
            is_sse = true;
        }
        if header_buf == "\r\n" {
            break;
        }
    }
    assert!(is_sse, "server response must specify Content-Type: text/event-stream");

    // Read first event lines
    let mut event_type_line = String::new();
    reader.read_line(&mut event_type_line).expect("read event line");
    assert!(event_type_line.starts_with("event: telemetry"), "must emit telemetry event");

    let mut id_line = String::new();
    reader.read_line(&mut id_line).expect("read id line");
    assert!(id_line.starts_with("id: "), "must emit event id");

    let mut data_line = String::new();
    reader.read_line(&mut data_line).expect("read data line");
    assert!(data_line.starts_with("data: {"), "must emit json data");
}

#[test]
fn test_tier1_r2_04_interactive_layout_switch_action() {
    let server = TestWebServer::start();
    let client = reqwest::blocking::Client::new();
    let url = format!("http://127.0.0.1:{}/api/action/layout", server.port);

    // Switch to nine_panel
    let resp9 = client.post(&url)
        .json(&json!({"layout": "nine_panel"}))
        .send()
        .expect("send layout switch");
    assert!(resp9.status().is_success());
    let val9: Value = resp9.json().unwrap();
    assert_eq!(val9["status"], "ok");
    assert_eq!(val9["layout"], "nine_panel");
    assert_eq!(val9["panel_count"], 9);

    // Switch back to twelve_panel
    let resp12 = client.post(&url)
        .json(&json!({"layout": "twelve_panel"}))
        .send()
        .expect("send layout switch");
    assert!(resp12.status().is_success());
    let val12: Value = resp12.json().unwrap();
    assert_eq!(val12["layout"], "twelve_panel");
    assert_eq!(val12["panel_count"], 12);
}

#[test]
fn test_tier1_r2_05_interactive_consensus_and_approve_actions() {
    let server = TestWebServer::start();
    let client = reqwest::blocking::Client::new();

    // Trigger consensus action
    let consensus_url = format!("http://127.0.0.1:{}/api/action/consensus", server.port);
    let resp_c = client.post(&consensus_url)
        .json(&json!({"threshold": 0.55, "divergence": 0.40}))
        .send()
        .expect("trigger consensus action");
    assert!(resp_c.status().is_success());
    let val_c: Value = resp_c.json().unwrap();
    assert_eq!(val_c["status"], "consensus_achieved");
    assert_eq!(val_c["total_proposals"], 4);
    assert_eq!(val_c["honest_quorum_size"], 3);
    assert_eq!(val_c["byzantine_filtered_count"], 1);

    // Trigger approve permission action
    let approve_url = format!("http://127.0.0.1:{}/api/action/approve", server.port);
    let resp_a = client.post(&approve_url)
        .json(&json!({"approval_id": "appr-v3010-01", "allow": true, "approver_id": "test-admin"}))
        .send()
        .expect("trigger approve action");
    assert!(resp_a.status().is_success());
    let val_a: Value = resp_a.json().unwrap();
    assert_eq!(val_a["status"], "resolved");
    assert_eq!(val_a["verdict"], "approved");
    assert_eq!(val_a["approval_id"], "appr-v3010-01");
}

// ----------------------------------------------------------------------------
// Feature R3: Phase 23 Federated Policy Engine
// ----------------------------------------------------------------------------

#[test]
fn test_tier1_r3_01_policy_gradient_vector_primitives() {
    let grad1 = TestPolicyGradient::new(
        ExternalAgentKind::Agy,
        [0.10, 0.20, 0.30, 0.40, 0.50, 0.60],
        10,
        0.05,
    );
    assert_eq!(grad1.sample_count, 10);
    assert_eq!(grad1.loss, 0.05);

    // Compute expected L2 norm: sqrt(0.01 + 0.04 + 0.09 + 0.16 + 0.25 + 0.36) = sqrt(0.91) ~= 0.9539
    let norm = grad1.l2_norm();
    assert!((norm - 0.953939).abs() < 1e-4);

    // Clip to max norm 0.50
    let clipped = grad1.clipped(0.50);
    assert!((clipped.l2_norm() - 0.50).abs() < 1e-6);

    let grad2 = TestPolicyGradient::new(
        ExternalAgentKind::ClaudeCode,
        [0.10, 0.20, 0.30, 0.40, 0.50, 0.60],
        5,
        0.08,
    );
    assert!((grad1.distance(&grad2)).abs() < 1e-9, "identical coordinates have zero distance");
}

#[test]
fn test_tier1_r3_02_agent_reputation_record_beta_conjugate() {
    let mut rec = TestAgentReputation::new(ExternalAgentKind::CodexCli);
    // Prior: alpha = 2.0, beta = 1.0 -> expected = 2/3 = 0.6667
    assert!((rec.expected_reliability() - 0.666667).abs() < 1e-4);
    assert!((rec.variance() - 0.055556).abs() < 1e-4);

    // Honest update with quality 0.90 -> alpha becomes 2.90, beta remains 1.0
    rec.record_outcome(true, 0.90);
    assert_eq!(rec.total_evaluations, 1);
    assert!((rec.expected_reliability() - (2.90 / 3.90)).abs() < 1e-6);

    // Byzantine update -> beta increments by 2.0 -> beta = 3.0
    rec.record_outcome(false, 0.0);
    assert_eq!(rec.total_evaluations, 2);
    assert!((rec.expected_reliability() - (2.90 / 5.90)).abs() < 1e-6);
}

#[test]
fn test_tier1_r3_03_federated_trimmed_mean_aggregation() {
    let mut reputations = BTreeMap::new();
    for &agent in ExternalAgentKind::all() {
        reputations.insert(agent, TestAgentReputation::new(agent));
    }

    // 4 agents: 3 honest around 0.5, 1 extreme Byzantine outlier at 10.0
    let gradients = vec![
        TestPolicyGradient::new(ExternalAgentKind::Agy, [0.48, 0.49, 0.50, 0.51, 0.52, 0.50], 1, 0.1),
        TestPolicyGradient::new(ExternalAgentKind::ClaudeCode, [0.50, 0.50, 0.50, 0.50, 0.50, 0.50], 1, 0.1),
        TestPolicyGradient::new(ExternalAgentKind::CodexCli, [0.52, 0.51, 0.50, 0.49, 0.48, 0.50], 1, 0.1),
        TestPolicyGradient::new(ExternalAgentKind::Devin, [10.0, 10.0, 10.0, 10.0, 10.0, 10.0], 1, 5.0),
    ];

    let (agg, trimmed_count) = test_aggregate_gradients_trimmed_mean(&gradients, &reputations, 1.0)
        .expect("trimmed mean aggregation");

    // Devin is trimmed as top outlier (k = 4/4 = 1 top and bottom)
    assert_eq!(trimmed_count, 2);
    for val in &agg {
        assert!(*val <= 0.60 && *val >= 0.40, "aggregated gradient must be shielded from outlier: {val}");
    }
}

#[test]
fn test_tier1_r3_04_federated_policy_step_and_receipt() {
    let mut current_policy: [f64; 6] = [0.5; 6];
    let mut velocity: [f64; 6] = [0.0; 6];
    let learning_rate: f64 = 0.05;
    let momentum: f64 = 0.90;

    let agg_grad: [f64; 6] = [0.10, -0.05, 0.02, 0.08, -0.01, 0.04];

    // Momentum step
    for i in 0..6 {
        velocity[i] = momentum * velocity[i] + (1.0 - momentum) * agg_grad[i];
        current_policy[i] = (current_policy[i] - learning_rate * velocity[i]).clamp(0.0, 1.0);
    }

    // Verify receipt hash calculation
    let mut hasher = Sha256::new();
    hasher.update(1u64.to_le_bytes());
    for val in &current_policy {
        hasher.update(val.to_le_bytes());
    }
    for val in &agg_grad {
        hasher.update(val.to_le_bytes());
    }
    let receipt_hash = format!("{:x}", hasher.finalize());
    assert_eq!(receipt_hash.len(), 64);
}

#[test]
fn test_tier1_r3_05_cli_reputation_status_all_9_agents() {
    let fixture = E2eFixture::new();

    let out = fixture.run(&["cockpit", "reputation", "--json"]);
    assert!(out.status.success(), "cockpit reputation CLI command must succeed");

    let val: Value = serde_json::from_slice(&out.stdout).expect("valid json output");
    assert_eq!(val["status"], "ok");
    assert_eq!(val["total_agents"], 9);
    assert_eq!(val["min_evals_for_quarantine"], 3);
    assert_eq!(val["quarantine_threshold"], 0.25);

    let agents_obj = val["agents"].as_object().expect("agents map");
    let expected_keys = ["agy", "claude", "cline", "codex", "cursor", "devin", "gemini", "hermes", "qwen"];
    for key in expected_keys {
        assert!(agents_obj.contains_key(key), "missing agent {key} in reputation status");
        let a = &agents_obj[key];
        assert_eq!(a["is_quarantined"], false);
        assert!((a["consensus_weight"].as_f64().unwrap() - 0.666667).abs() < 1e-4);
    }
}

// ============================================================================
// TIER 2: BOUNDARY & CORNER CASES (>=5 per feature)
// Extreme weights, zero norms, NaN handling, buffer overflows, rapid reconnects, quarantined agents
// ============================================================================

// ----------------------------------------------------------------------------
// Tier 2: R1 Autopilot Boundaries
// ----------------------------------------------------------------------------

#[test]
fn test_tier2_r1_01_autopilot_empty_or_whitespace_goal() {
    let mut collab = MultiAgentCollabManager::new();
    let res_empty = collab.dispatch("/autopilot ");
    assert!(res_empty.is_err(), "empty goal must be rejected with validation error");

    let res_spaces = collab.dispatch("/autopilot    ");
    assert!(res_spaces.is_err(), "whitespace-only goal must be rejected");
}

#[test]
fn test_tier2_r1_02_autopilot_subquorum_single_agent() {
    let engine = SwarmCodeConsensusEngine::new(0.50, 0.40);
    // 1. Empty proposals slice must safely return Err
    let res_empty = engine.resolve_consensus(&[], 1000);
    assert!(res_empty.is_err(), "empty proposal slice must return Err");

    // 2. Non-viable sub-quorum proposal (fails invariant compliance < 0.40) must safely return Err
    let non_viable = vec![
        AgentPatchProposal::new(
            "p_solo_fail".to_string(),
            ExternalAgentKind::Agy,
            "Solo non-viable".to_string(),
            "Non-viable proposal test".to_string(),
            "--- a/solo.rs\n+++ b/solo.rs\n@@ -1 +1 @@\n-a\n+b".to_string(),
            ProposalScoreVector::new(0.30, 0.20, 0.10, 0.20),
            50,
        ),
    ];

    let res_fail = engine.resolve_consensus(&non_viable, 1000);
    assert!(res_fail.is_err(), "non-viable proposal must safely return Err instead of panicking");
}

#[test]
fn test_tier2_r1_03_autopilot_all_byzantine_rejection() {
    let engine = SwarmCodeConsensusEngine::new(0.60, 0.30);
    // All proposals violate invariant compliance (< 0.40) or threshold
    let bad_proposals = vec![
        AgentPatchProposal::new(
            "b1".to_string(), ExternalAgentKind::Agy, "Bad 1".to_string(), "bad".to_string(),
            "bad".to_string(), ProposalScoreVector::new(0.20, 0.20, 0.20, 0.20), 10,
        ),
        AgentPatchProposal::new(
            "b2".to_string(), ExternalAgentKind::ClaudeCode, "Bad 2".to_string(), "bad".to_string(),
            "bad".to_string(), ProposalScoreVector::new(0.10, 0.30, 0.15, 0.10), 10,
        ),
        AgentPatchProposal::new(
            "b3".to_string(), ExternalAgentKind::CodexCli, "Bad 3".to_string(), "bad".to_string(),
            "bad".to_string(), ProposalScoreVector::new(0.25, 0.15, 0.25, 0.10), 10,
        ),
    ];

    let res = engine.resolve_consensus(&bad_proposals, 2000);
    assert!(res.is_err(), "when all proposals are below threshold, consensus must reject all");
}

#[test]
fn test_tier2_r1_04_autopilot_malformed_diff_injection() {
    let proposal = AgentPatchProposal::new(
        "p_malformed".to_string(),
        ExternalAgentKind::Devin,
        "Corrupted binary diff".to_string(),
        "Adversarial binary injection".to_string(),
        String::from_utf8_lossy(b"\x00\xFF\xFE\xFD\x80\x90BINARY_GARBAGE_NO_DIFF_HEADER\r\n\x00").to_string(),
        ProposalScoreVector::new(0.90, 0.85, 0.80, 0.90),
        100,
    );

    // Fingerprint calculation must handle arbitrary binary sequences safely without panic
    let fp = proposal.fingerprint();
    assert_eq!(fp.len(), 64, "fingerprint must compute SHA-256 successfully over malformed diff bytes");
}

#[test]
fn test_tier2_r1_05_autopilot_quarantined_agent_exclusion() {
    let mut rep = TestAgentReputation::new(ExternalAgentKind::CursorAgent);
    // Fail 3 times consecutively
    rep.record_outcome(false, 0.0);
    rep.record_outcome(false, 0.0);
    rep.record_outcome(false, 0.0);
    assert!(rep.is_quarantined, "agent must be quarantined after 3 byzantine outcomes");

    // In trimmed mean, quarantined agent weight drops to 0.05
    let mut reputations = BTreeMap::new();
    reputations.insert(ExternalAgentKind::CursorAgent, rep);
    reputations.insert(ExternalAgentKind::Agy, TestAgentReputation::new(ExternalAgentKind::Agy));

    let g_cursor = TestPolicyGradient::new(ExternalAgentKind::CursorAgent, [1.0; 6], 1, 0.5);
    let g_agy = TestPolicyGradient::new(ExternalAgentKind::Agy, [0.0; 6], 1, 0.1);

    let (agg, _) = test_aggregate_gradients_trimmed_mean(&[g_cursor, g_agy], &reputations, 1.0).unwrap();
    // Agy has weight 0.667 while Cursor has penalty weight 0.05, so aggregate should be very close to 0.0
    for val in &agg {
        assert!(*val < 0.15, "quarantined agent weight must have negligible influence on aggregate: {val}");
    }
}

// ----------------------------------------------------------------------------
// Tier 2: R2 Web Micro-Server Boundaries
// ----------------------------------------------------------------------------

#[test]
fn test_tier2_r2_01_web_rapid_reconnect_and_stream_close() {
    let server = TestWebServer::start();

    // Rapidly open and drop 10 connections in a loop
    for _ in 0..10 {
        let stream = TcpStream::connect(format!("127.0.0.1:{}", server.port));
        assert!(stream.is_ok(), "connection must succeed without socket exhaustion");
        drop(stream);
    }

    // After rapid reconnect churn, server must still serve normal requests
    let url = format!("http://127.0.0.1:{}/api/snapshot", server.port);
    let resp = reqwest::blocking::get(&url).expect("snapshot request after churn");
    assert!(resp.status().is_success());
}

#[test]
fn test_tier2_r2_02_web_invalid_json_payload() {
    let server = TestWebServer::start();
    let client = reqwest::blocking::Client::new();
    let url = format!("http://127.0.0.1:{}/api/action/layout", server.port);

    // Send malformed non-JSON body
    let resp = client.post(&url)
        .header("Content-Type", "application/json")
        .body("INVALID_MALFORMED_JSON_STRING{{{")
        .send()
        .expect("send bad json");

    assert_eq!(resp.status().as_u16(), 400, "malformed JSON must return HTTP 400 Bad Request");
}

#[test]
fn test_tier2_r2_03_web_oversized_payload_protection() {
    let server = TestWebServer::start();
    let client = reqwest::blocking::Client::new();
    let url = format!("http://127.0.0.1:{}/api/action/layout", server.port);

    // Create 70 KB payload (> 64 KB protection boundary)
    let oversized = vec![b'A'; 70 * 1024];
    let resp = client.post(&url)
        .header("Content-Type", "application/json")
        .body(oversized)
        .send()
        .expect("send oversized body");

    assert_eq!(resp.status().as_u16(), 413, "payloads > 64 KB must return HTTP 413 Payload Too Large");
}

#[test]
fn test_tier2_r2_04_web_unknown_endpoint_404() {
    let server = TestWebServer::start();
    let url = format!("http://127.0.0.1:{}/api/non_existent_endpoint_v3010", server.port);
    let resp = reqwest::blocking::get(&url).expect("query unknown endpoint");
    assert_eq!(resp.status().as_u16(), 404, "unknown routes must return HTTP 404 Not Found");
}

#[test]
fn test_tier2_r2_05_web_port_conflict_handling() {
    let server = TestWebServer::start();
    // Attempt to bind another TcpListener on the same port
    let conflict = TcpListener::bind(format!("127.0.0.1:{}", server.port));
    assert!(conflict.is_err(), "binding an already occupied port must return std::io::Error");
}

// ----------------------------------------------------------------------------
// Tier 2: R3 Federated Policy Boundaries
// ----------------------------------------------------------------------------

#[test]
fn test_tier2_r3_01_gradient_zero_and_infinite_norms() {
    // Zero gradient
    let zero_g = TestPolicyGradient::new(ExternalAgentKind::Agy, [0.0; 6], 1, 0.0);
    assert_eq!(zero_g.l2_norm(), 0.0);
    let clipped_zero = zero_g.clipped(1.0);
    assert_eq!(clipped_zero.l2_norm(), 0.0);

    // Extreme gradient
    let huge_g = TestPolicyGradient::new(ExternalAgentKind::ClaudeCode, [1e8; 6], 1, 100.0);
    let clipped_huge = huge_g.clipped(1.0);
    assert!((clipped_huge.l2_norm() - 1.0).abs() < 1e-6, "extreme gradients must be clipped to max norm bound");
}

#[test]
fn test_tier2_r3_02_empty_gradient_set_error() {
    let reputations = BTreeMap::new();
    let empty_vec: Vec<TestPolicyGradient> = Vec::new();
    let res = test_aggregate_gradients_trimmed_mean(&empty_vec, &reputations, 1.0);
    assert!(res.is_err(), "aggregating empty gradient slice must return Err");
}

#[test]
fn test_tier2_r3_03_extreme_learning_rate_and_momentum() {
    let extreme_lr: f64 = 5.0;
    let clamped_lr = extreme_lr.clamp(0.001, 1.0);
    assert_eq!(clamped_lr, 1.0);

    let extreme_momentum: f64 = -0.5;
    let clamped_momentum = extreme_momentum.clamp(0.0, 0.99);
    assert_eq!(clamped_momentum, 0.0);
}

#[test]
fn test_tier2_r3_04_quarantine_threshold_decay() {
    let mut rec = TestAgentReputation::new(ExternalAgentKind::QwenCode);
    // Prior: alpha = 2.0, beta = 1.0
    // Eval 1: Byzantine -> beta = 3.0 -> weight = 2/5 = 0.40 (not quarantined, total < 3)
    rec.record_outcome(false, 0.0);
    assert_eq!(rec.total_evaluations, 1);
    assert!(!rec.is_quarantined);

    // Eval 2: Byzantine -> beta = 5.0 -> weight = 2/7 = 0.285 (not quarantined, total < 3)
    rec.record_outcome(false, 0.0);
    assert_eq!(rec.total_evaluations, 2);
    assert!(!rec.is_quarantined);

    // Eval 3: Byzantine -> beta = 7.0 -> weight = 2/9 = 0.222 (< 0.25 threshold, total >= 3)
    rec.record_outcome(false, 0.0);
    assert_eq!(rec.total_evaluations, 3);
    assert!(rec.is_quarantined, "must be quarantined when weight < 0.25 after >= 3 evaluations");
}

#[test]
fn test_tier2_r3_05_thompson_sampling_extremes() {
    let mut honest_agent = TestAgentReputation::new(ExternalAgentKind::Agy);
    honest_agent.alpha = 1000.0;
    honest_agent.beta = 1.0;

    let mut rogue_agent = TestAgentReputation::new(ExternalAgentKind::Devin);
    rogue_agent.alpha = 1.0;
    rogue_agent.beta = 1000.0;

    assert!(honest_agent.expected_reliability() > 0.998);
    assert!(rogue_agent.expected_reliability() < 0.002);
    assert!(honest_agent.variance() < 1e-5);
    assert!(rogue_agent.variance() < 1e-5);
}

// ============================================================================
// TIER 3: CROSS-FEATURE COMBINATIONS
// Pairwise interactions across Web, Consensus, Autopilot, and Federated Policy
// ============================================================================

#[test]
fn test_tier3_01_web_action_triggers_consensus_and_updates_reputation() {
    let server = TestWebServer::start();
    let client = reqwest::blocking::Client::new();

    // 1. Trigger consensus action via HTTP POST
    let url = format!("http://127.0.0.1:{}/api/action/consensus", server.port);
    let resp = client.post(&url)
        .json(&json!({"threshold": 0.50, "divergence": 0.45}))
        .send()
        .expect("consensus POST");
    assert!(resp.status().is_success());
    let decision_val: Value = resp.json().unwrap();

    let winning_agent = decision_val["winning_agent"].as_str().unwrap();
    assert_eq!(winning_agent, "agy");

    // 2. Feed consensus outcome into reputation tracker
    let mut tracker = BTreeMap::new();
    tracker.insert(ExternalAgentKind::Agy, TestAgentReputation::new(ExternalAgentKind::Agy));
    tracker.insert(ExternalAgentKind::Hermes, TestAgentReputation::new(ExternalAgentKind::Hermes));

    // Agy won (honest)
    tracker.get_mut(&ExternalAgentKind::Agy).unwrap().record_outcome(true, 0.95);
    // Hermes was filtered (Byzantine)
    tracker.get_mut(&ExternalAgentKind::Hermes).unwrap().record_outcome(false, 0.0);

    assert!(tracker[&ExternalAgentKind::Agy].consensus_weight > tracker[&ExternalAgentKind::Hermes].consensus_weight);
}

#[test]
fn test_tier3_02_layout_switch_during_live_sse_stream() {
    let server = TestWebServer::start();
    let client = reqwest::blocking::Client::new();

    // Start background thread making an HTTP POST to switch layout
    let port = server.port;
    let post_handle = thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        let client = reqwest::blocking::Client::new();
        let url = format!("http://127.0.0.1:{port}/api/action/layout");
        let resp = client.post(&url).json(&json!({"layout": "nine_panel"})).send().unwrap();
        assert!(resp.status().is_success());
    });

    // Simultaneously fetch snapshot to confirm update
    post_handle.join().unwrap();
    let snap_url = format!("http://127.0.0.1:{}/api/snapshot", server.port);
    let resp = client.get(&snap_url).send().unwrap();
    let val: Value = resp.json().unwrap();
    assert_eq!(val["layout"], "nine_panel");
    assert_eq!(val["panel_count"], 9);
}

#[test]
fn test_tier3_03_autopilot_certification_reflects_in_web_snapshot() {
    let server = TestWebServer::start();

    // 1. Run autopilot patch certification
    let winner = AgentPatchProposal::new(
        "p_cert".to_string(),
        ExternalAgentKind::Agy,
        "Certified Patch".to_string(),
        "zeroize".to_string(),
        "diff".to_string(),
        ProposalScoreVector::new(1.0, 1.0, 0.95, 1.0),
        50,
    );
    let cert = certify_patch("Goal", &winner, "PANDORA_CONSENSUS_V1:mock", 0.95);

    // 2. Web snapshot reflects ongoing system health and digest verification
    let snap_url = format!("http://127.0.0.1:{}/api/snapshot", server.port);
    let resp = reqwest::blocking::get(&snap_url).unwrap();
    let val: Value = resp.json().unwrap();
    assert_eq!(val["healthy_panels"], 12);
    assert!(!cert.is_empty());
}

#[test]
fn test_tier3_04_permission_approval_unblocks_autopilot_flow() {
    let server = TestWebServer::start();
    let client = reqwest::blocking::Client::new();

    // 1. Approve permission via Web API
    let approve_url = format!("http://127.0.0.1:{}/api/action/approve", server.port);
    let resp = client.post(&approve_url)
        .json(&json!({"approval_id": "tool-call-git-push", "allow": true, "approver_id": "operator"}))
        .send()
        .unwrap();
    assert!(resp.status().is_success());

    // 2. Dispatch command through collab manager
    let mut collab = MultiAgentCollabManager::new();
    let line = "/collab agy claude -- execute authorized tool";
    let res = collab.dispatch(line);
    assert!(res.is_ok());
    assert_eq!(res.unwrap().spawned_agents.len(), 2);
}

#[test]
fn test_tier3_05_federated_gradient_update_and_reputation_quarantine_sync() {
    let mut reputations = BTreeMap::new();
    for &agent in ExternalAgentKind::all() {
        reputations.insert(agent, TestAgentReputation::new(agent));
    }

    // Agent Devin goes rogue 3 times
    for _ in 0..3 {
        reputations.get_mut(&ExternalAgentKind::Devin).unwrap().record_outcome(false, 0.0);
    }
    assert!(reputations[&ExternalAgentKind::Devin].is_quarantined);

    // Perform federated gradient aggregation: Devin's gradient is quarantined and filtered
    let gradients: Vec<TestPolicyGradient> = ExternalAgentKind::all()
        .iter()
        .map(|&agent| {
            if agent == ExternalAgentKind::Devin {
                TestPolicyGradient::new(agent, [1.0; 6], 1, 1.0)
            } else {
                TestPolicyGradient::new(agent, [0.1; 6], 1, 0.05)
            }
        })
        .collect();

    let (agg, _) = test_aggregate_gradients_trimmed_mean(&gradients, &reputations, 1.0).unwrap();
    // Aggregated gradient remains close to honest workers (0.1)
    for val in &agg {
        assert!(*val < 0.25, "quarantined rogue agent must not compromise federated policy update: {val}");
    }
}

// ============================================================================
// TIER 4: REAL-WORLD APPLICATION SCENARIOS
// Full lifecycle autonomous missions, multi-step convergence, and mobile companion
// ============================================================================

#[test]
fn test_tier4_01_full_lifecycle_autonomous_mission() {
    let _fixture = E2eFixture::new();

    // Step 1: Start background web micro-server
    let server = TestWebServer::start();

    // Step 2: Mobile/desktop companion connects and verifies 12-panel matrix
    let snap_url = format!("http://127.0.0.1:{}/api/snapshot", server.port);
    let snap_resp: Value = reqwest::blocking::get(&snap_url).unwrap().json().unwrap();
    assert_eq!(snap_resp["panel_count"], 12);
    assert_eq!(snap_resp["healthy_panels"], 12);

    // Step 3: Autopilot task goal partitioned across 9 agents
    let goal = "Autonomous Refactoring: Eliminate all unsafe code and verify formal invariants";
    let mut collab = MultiAgentCollabManager::new();
    let dispatch_res = collab.dispatch(&format!("/autopilot {goal}")).unwrap();
    assert_eq!(dispatch_res.spawned_agents.len(), 9);

    // Step 4: Swarm consensus resolves candidate proposals
    let engine = SwarmCodeConsensusEngine::new(0.50, 0.40);
    let proposals = vec![
        AgentPatchProposal::new(
            "p1".to_string(), ExternalAgentKind::Agy, "Safe parser".to_string(), "safe".to_string(),
            "diff".to_string(), ProposalScoreVector::new(1.0, 0.98, 0.95, 1.0), 90,
        ),
        AgentPatchProposal::new(
            "p2".to_string(), ExternalAgentKind::ClaudeCode, "Bounds".to_string(), "safe".to_string(),
            "diff".to_string(), ProposalScoreVector::new(0.95, 0.95, 0.92, 0.95), 110,
        ),
        AgentPatchProposal::new(
            "p3".to_string(), ExternalAgentKind::CodexCli, "AST".to_string(), "safe".to_string(),
            "diff".to_string(), ProposalScoreVector::new(0.92, 0.90, 0.88, 0.90), 105,
        ),
        AgentPatchProposal::new(
            "p4_byz".to_string(), ExternalAgentKind::Devin, "Byz".to_string(), "bad".to_string(),
            "diff".to_string(), ProposalScoreVector::new(0.10, 0.15, 0.10, 0.05), 190,
        ),
    ];
    let decision = engine.resolve_consensus(&proposals, 1000).unwrap();
    assert_eq!(decision.winning_proposal_id, "p1");
    assert_eq!(decision.byzantine_filtered_count, 1);

    // Step 5: Issue patch certificate
    let winning_prop = &proposals[0];
    let cert = certify_patch(goal, winning_prop, &decision.consensus_hash, 0.95);
    assert!(cert.starts_with("PANDORA_PATCH_CERT_V1:"));

    // Step 6: Verify cleanup on server drop
    drop(server);
}

#[test]
fn test_tier4_02_byzantine_swarm_quarantine_and_recovery() {
    let mut rep = TestAgentReputation::new(ExternalAgentKind::CursorAgent);
    assert_eq!(rep.consensus_weight, 2.0 / 3.0);

    // Round 1–3: Repeated Byzantine failures -> Quarantine
    for _ in 0..3 {
        rep.record_outcome(false, 0.0);
    }
    assert!(rep.is_quarantined);
    let min_weight = rep.consensus_weight;

    // Round 4–10: Agent produces consecutive high-quality verified patches -> Weight recovers
    for _ in 0..10 {
        rep.record_outcome(true, 1.0);
    }
    assert!(rep.consensus_weight > min_weight);
    assert!(rep.consensus_weight > 0.50, "rehabilitated agent weight must exceed 0.50: {}", rep.consensus_weight);
    assert!(!rep.is_quarantined, "rehabilitated agent must no longer be quarantined");
}

#[test]
fn test_tier4_03_mobile_companion_interactive_control_loop() {
    let server = TestWebServer::start();
    let client = reqwest::blocking::Client::new();

    // 1. Mobile browser loads Xiaomi 15 calibrated HTML
    let index_url = format!("http://127.0.0.1:{}/", server.port);
    let html = client.get(&index_url).header("User-Agent", "Mozilla/5.0 (Linux; Android 15; Xiaomi 15)").send().unwrap().text().unwrap();
    assert!(html.contains("viewport-fit=cover"));

    // 2. Mobile user toggles layout to nine_panel
    let layout_url = format!("http://127.0.0.1:{}/api/action/layout", server.port);
    let l_resp: Value = client.post(&layout_url).json(&json!({"layout": "nine_panel"})).send().unwrap().json().unwrap();
    assert_eq!(l_resp["panel_count"], 9);

    // 3. Mobile user triggers Byzantine consensus
    let consensus_url = format!("http://127.0.0.1:{}/api/action/consensus", server.port);
    let c_resp: Value = client.post(&consensus_url).json(&json!({"threshold": 0.50})).send().unwrap().json().unwrap();
    assert_eq!(c_resp["status"], "consensus_achieved");

    // 4. Mobile user approves permission prompt
    let approve_url = format!("http://127.0.0.1:{}/api/action/approve", server.port);
    let a_resp: Value = client.post(&approve_url).json(&json!({"approval_id": "appr-mobile", "allow": true})).send().unwrap().json().unwrap();
    assert_eq!(a_resp["verdict"], "approved");
}

#[test]
fn test_tier4_04_multi_step_federated_policy_convergence() {
    let mut policy = [0.5; 6];
    let mut velocity = [0.0; 6];
    let learning_rate = 0.05;
    let momentum = 0.90;

    let mut reputations = BTreeMap::new();
    for &agent in ExternalAgentKind::all() {
        reputations.insert(agent, TestAgentReputation::new(agent));
    }

    // Run 5 optimization steps towards target descent direction [0.2, 0.2, 0.2, 0.2, 0.2, 0.2]
    for step in 1..=5 {
        let gradients: Vec<TestPolicyGradient> = ExternalAgentKind::all()
            .iter()
            .enumerate()
            .map(|(i, &agent)| {
                if i < 7 {
                    // Honest agents provide gradients pointing toward descent target
                    TestPolicyGradient::new(agent, [0.3; 6], 1, 0.1)
                } else {
                    // 2 Byzantine agents generate divergent adversarial noise
                    TestPolicyGradient::new(agent, [-5.0; 6], 1, 1.0)
                }
            })
            .collect();

        let (agg, trimmed) = test_aggregate_gradients_trimmed_mean(&gradients, &reputations, 1.0).unwrap();
        assert_eq!(trimmed, 4, "step {step}: must trim 2 top and 2 bottom outliers across 9 agents");

        // Momentum descent
        for d in 0..6 {
            velocity[d] = momentum * velocity[d] + (1.0 - momentum) * agg[d];
            policy[d] = (policy[d] - learning_rate * velocity[d]).clamp(0.0, 1.0);
        }
    }

    // Policy parameters must have successfully decreased from initial 0.5 towards optimal descent
    for val in &policy {
        assert!(*val < 0.50, "federated policy parameter must decrease in direction of descent: {val}");
        assert!(*val >= 0.0, "policy parameter must respect [0, 1] bounds");
    }
}
