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
use crate::distributed_swarm::{SwarmPeer, decode_hex_32, decode_hex_64, encode_hex};
use crate::mesh_sync::PeerIdentity;
use crate::multi_agent_collab::{ExternalAgentKind, MultiAgentCollabManager};
use crate::multi_agent_consensus::{
    AgentPatchProposal, ProposalScoreVector, SwarmCodeConsensusEngine,
};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
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
pub const DASHBOARD_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0, maximum-scale=1.0, user-scalable=no, viewport-fit=cover">
    <meta name="apple-mobile-web-app-capable" content="yes">
    <meta name="apple-mobile-web-app-status-bar-style" content="black-translucent">
    <meta name="theme-color" content="#0a0d14">
    <link rel="manifest" href="/manifest.webmanifest">
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
        .sparkline-box {
            margin-bottom: 12px;
            padding: 8px 12px;
            background: var(--surface);
            border: 1px solid var(--border);
            border-radius: 8px;
        }
        .sparkline-label {
            font-size: 11px;
            color: var(--muted);
            margin-bottom: 4px;
            display: flex;
            justify-content: space-between;
        }
        canvas#sparkline {
            width: 100%;
            height: 38px;
            display: block;
            border-radius: 4px;
            background: #0d121d;
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
            <span id="digest-pill" style="margin-left:8px;color:var(--muted);font-size:11px;">CONNECTING...</span>
        </div>
    </header>
    <div class="sparkline-box">
        <div class="sparkline-label">
            <span>LIVE TELEMETRY SPARKLINE</span>
            <span id="sparkline-rate">100% NOMINAL</span>
        </div>
        <canvas id="sparkline" width="360" height="38"></canvas>
    </div>
    <main id="matrix" class="grid-12"></main>
    <nav class="dock">
        <button class="btn" onclick="toggleLayout()">Toggle 9P / 12P</button>
        <button class="btn" onclick="runConsensus()">Swarm Consensus</button>
        <button class="btn" onclick="approvePending()">Approve Agent</button>
    </nav>
    <script>
        let currentLayout = "twelve_panel";
        const sparkData = [1.0, 1.0, 1.0, 1.0, 1.0];

        function triggerHaptic(type) {
            if (navigator.vibrate) {
                if (type === "consensus") navigator.vibrate([100, 50, 100, 50, 200]);
                else navigator.vibrate(25);
            }
            try {
                const ctx = new (window.AudioContext || window.webkitAudioContext)();
                const osc = ctx.createOscillator();
                const gain = ctx.createGain();
                osc.connect(gain);
                gain.connect(ctx.destination);
                osc.frequency.value = type === "consensus" ? 880 : 440;
                gain.gain.setValueAtTime(0.04, ctx.currentTime);
                gain.gain.exponentialRampToValueAtTime(0.001, ctx.currentTime + 0.15);
                osc.start();
                osc.stop(ctx.currentTime + 0.15);
            } catch (_) {}
        }

        function updateSparkline(ratio) {
            sparkData.push(ratio);
            if (sparkData.length > 30) sparkData.shift();
            const canvas = document.getElementById("sparkline");
            if (!canvas) return;
            const ctx = canvas.getContext("2d");
            const w = canvas.width;
            const h = canvas.height;
            ctx.clearRect(0, 0, w, h);
            ctx.fillStyle = "rgba(0, 240, 255, 0.15)";
            ctx.beginPath();
            ctx.moveTo(0, h);
            const step = w / Math.max(sparkData.length - 1, 1);
            for (let i = 0; i < sparkData.length; i++) {
                const y = h - (sparkData[i] * (h - 8)) - 4;
                ctx.lineTo(i * step, y);
            }
            ctx.lineTo((sparkData.length - 1) * step, h);
            ctx.closePath();
            ctx.fill();
            ctx.strokeStyle = "#00f0ff";
            ctx.lineWidth = 2;
            ctx.beginPath();
            for (let i = 0; i < sparkData.length; i++) {
                const y = h - (sparkData[i] * (h - 8)) - 4;
                if (i === 0) ctx.moveTo(0, y);
                else ctx.lineTo(i * step, y);
            }
            ctx.stroke();
            const pct = Math.round(ratio * 100);
            document.getElementById("sparkline-rate").textContent = pct + "% NOMINAL";
        }

        function renderSnapshot(snap) {
            if (!snap) return;
            currentLayout = snap.layout || currentLayout;
            const matrix = document.getElementById("matrix");
            matrix.className = currentLayout === "nine_panel" ? "grid-9" : "grid-12";
            const healthy = snap.healthy_panels || snap.panel_count || 12;
            const total = snap.panel_count || 12;
            document.getElementById("health-pill").textContent = healthy + "/" + total + " HEALTHY";
            updateSparkline(healthy / Math.max(total, 1));
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
            triggerHaptic("tap");
            const next = currentLayout === "nine_panel" ? "twelve_panel" : "nine_panel";
            if (ws && ws.readyState === WebSocket.OPEN) {
                ws.send(JSON.stringify({action: "layout", layout: next}));
            } else {
                const res = await fetch("/api/action/layout", {
                    method: "POST",
                    headers: {"Content-Type": "application/json"},
                    body: JSON.stringify({layout: next})
                });
                if (res.ok) { const d = await res.json(); currentLayout = d.layout; }
            }
        }

        async function runConsensus() {
            triggerHaptic("consensus");
            if (ws && ws.readyState === WebSocket.OPEN) {
                ws.send(JSON.stringify({action: "consensus", threshold: 0.50, divergence: 0.45}));
            } else {
                await fetch("/api/action/consensus", {
                    method: "POST",
                    headers: {"Content-Type": "application/json"},
                    body: JSON.stringify({threshold: 0.50, divergence: 0.45})
                });
            }
        }

        async function approvePending() {
            triggerHaptic("tap");
            await fetch("/api/action/approve", {
                method: "POST",
                headers: {"Content-Type": "application/json"},
                body: JSON.stringify({approval_id: "appr-live", allow: true, approver_id: "mobile-operator"})
            });
        }

        // Swipe gestures for Xiaomi 15 (20:9 ratio viewport)
        let touchStartX = 0;
        let touchStartY = 0;
        document.addEventListener("touchstart", e => {
            touchStartX = e.changedTouches[0].screenX;
            touchStartY = e.changedTouches[0].screenY;
        }, {passive: true});
        document.addEventListener("touchend", e => {
            const diffX = e.changedTouches[0].screenX - touchStartX;
            const diffY = e.changedTouches[0].screenY - touchStartY;
            if (Math.abs(diffX) > 60 && Math.abs(diffY) < 50) {
                toggleLayout();
            }
        }, {passive: true});

        // ============================================================================
        // Client-Side Offline Telemetry Store (IndexedDB: pandora_telemetry_db v1)
        // ============================================================================
        class TelemetryDB {
            constructor() {
                this.dbName = 'pandora_telemetry_db';
                this.version = 1;
                this.db = null;
            }
            async open() {
                if (this.db) return this.db;
                return new Promise((resolve, reject) => {
                    const req = indexedDB.open(this.dbName, this.version);
                    req.onupgradeneeded = (e) => {
                        const db = e.target.result;
                        if (!db.objectStoreNames.contains('cluster_metrics')) {
                            const store = db.createObjectStore('cluster_metrics', { keyPath: 'id' });
                            store.createIndex('timestamp', 'timestamp', { unique: false });
                            store.createIndex('node_id', 'node_id', { unique: false });
                            store.createIndex('sequence_id', 'sequence_id', { unique: true });
                        }
                        if (!db.objectStoreNames.contains('run_histories')) {
                            const store = db.createObjectStore('run_histories', { keyPath: 'run_id' });
                            store.createIndex('timestamp', 'timestamp', { unique: false });
                            store.createIndex('candidate_patch_id', 'candidate_patch_id', { unique: false });
                            store.createIndex('status', 'status', { unique: false });
                        }
                    };
                    req.onsuccess = (e) => {
                        this.db = e.target.result;
                        resolve(this.db);
                    };
                    req.onerror = (e) => reject(e.target.error);
                });
            }
            async putClusterMetric(record) {
                const db = await this.open();
                return new Promise((resolve, reject) => {
                    const tx = db.transaction(['cluster_metrics'], 'readwrite');
                    tx.objectStore('cluster_metrics').put(record);
                    tx.oncomplete = () => resolve();
                    tx.onerror = (e) => reject(e.target.error);
                });
            }
            async putClusterMetricsBatch(records) {
                if (!records || records.length === 0) return;
                const db = await this.open();
                return new Promise((resolve, reject) => {
                    const tx = db.transaction(['cluster_metrics'], 'readwrite');
                    const store = tx.objectStore('cluster_metrics');
                    for (const r of records) store.put(r);
                    tx.oncomplete = () => resolve();
                    tx.onerror = (e) => reject(e.target.error);
                });
            }
            async getLatestClusterMetric() {
                const db = await this.open();
                return new Promise((resolve, reject) => {
                    const tx = db.transaction(['cluster_metrics'], 'readonly');
                    const index = tx.objectStore('cluster_metrics').index('timestamp');
                    const req = index.openCursor(null, 'prev');
                    req.onsuccess = (e) => {
                        const cursor = e.target.result;
                        resolve(cursor ? cursor.value : null);
                    };
                    req.onerror = (e) => reject(e.target.error);
                });
            }
            async getLastSyncInfo() {
                const latest = await this.getLatestClusterMetric();
                return {
                    last_seq: latest ? (latest.sequence_id || 0) : 0,
                    last_timestamp: latest ? (latest.timestamp || 0) : 0
                };
            }
            async getRecentMetrics(limit = 30) {
                const db = await this.open();
                return new Promise((resolve, reject) => {
                    const tx = db.transaction(['cluster_metrics'], 'readonly');
                    const index = tx.objectStore('cluster_metrics').index('timestamp');
                    const req = index.openCursor(null, 'prev');
                    const results = [];
                    req.onsuccess = (e) => {
                        const cursor = e.target.result;
                        if (cursor && results.length < limit) {
                            results.push(cursor.value);
                            cursor.continue();
                        } else {
                            resolve(results.reverse());
                        }
                    };
                    req.onerror = (e) => reject(e.target.error);
                });
            }
        }

        const telemetryDb = new TelemetryDB();
        telemetryDb.open().then(async () => {
            const latest = await telemetryDb.getLatestClusterMetric();
            if (latest && !ws) {
                renderSnapshot(latest);
                document.getElementById("status").textContent = "PANDORA_COCKPIT (OFFLINE STORE)";
            }
            const recent = await telemetryDb.getRecentMetrics(30);
            if (recent && recent.length > 0) {
                sparkData.length = 0;
                for (const m of recent) {
                    sparkData.push((m.healthy_panels || 12) / Math.max(m.total_panels || 12, 1));
                }
                updateSparkline(sparkData[sparkData.length - 1]);
            }
        }).catch(() => {});

        // WebSocket with auto-reconnect, sequence synchronization, and SSE fallback
        let ws = null;
        let reconnectAttempts = 0;
        function initWebSocket() {
            const wsProto = location.protocol === "https:" ? "wss:" : "ws:";
            const wsUrl = wsProto + "//" + location.host + "/api/ws";
            try {
                ws = new WebSocket(wsUrl);
                ws.onopen = async () => {
                    reconnectAttempts = 0;
                    const pill = document.getElementById("digest-pill");
                    pill.textContent = "WS SYNCING...";
                    pill.style.color = "var(--cyan)";
                    try {
                        const syncInfo = await telemetryDb.getLastSyncInfo();
                        ws.send(JSON.stringify({
                            action: "sync_request",
                            since_sequence_id: syncInfo.last_seq,
                            since_timestamp_ms: syncInfo.last_timestamp,
                            client_id: "mobile-pwa"
                        }));
                    } catch (_) {}
                };
                ws.onmessage = (event) => {
                    try {
                        const msg = JSON.parse(event.data);
                        if (msg.event === "sync_response") {
                            const pill = document.getElementById("digest-pill");
                            if (msg.status === "up_to_date") {
                                pill.textContent = "WS LIVE (IN SYNC)";
                                pill.style.color = "var(--cyan)";
                            } else if (msg.status === "catch_up") {
                                pill.textContent = `WS LIVE (+${msg.missed_packets_count || (msg.records ? msg.records.length : 0)} SYNCED)`;
                                pill.style.color = "var(--emerald)";
                                if (Array.isArray(msg.records)) {
                                    const metrics = [];
                                    for (const r of msg.records) {
                                        if ((r.kind === "snapshot" || r.kind === "metric") && r.payload) {
                                            metrics.push({
                                                id: "cm_" + r.timestamp_ms + "_" + (r.payload.snapshot_digest || "snap"),
                                                sequence_id: r.sequence_id,
                                                timestamp: r.timestamp_ms,
                                                node_id: "local",
                                                healthy_panels: r.payload.healthy_panels || r.payload.panel_count || 12,
                                                total_panels: r.payload.panel_count || 12,
                                                layout: r.payload.layout || "twelve_panel",
                                                snapshot_digest: r.payload.snapshot_digest || ""
                                            });
                                        }
                                    }
                                    telemetryDb.putClusterMetricsBatch(metrics).catch(() => {});
                                }
                            } else if (msg.status === "gap_overflow") {
                                pill.textContent = "WS LIVE (RE-BASELINED)";
                                pill.style.color = "var(--amber)";
                                if (msg.latest_snapshot) renderSnapshot(msg.latest_snapshot);
                            }
                        } else if (msg.event === "telemetry") {
                            const snap = msg.snapshot || msg;
                            renderSnapshot(snap);
                            telemetryDb.putClusterMetric({
                                id: "cm_" + (msg.timestamp_ms || Date.now()) + "_" + (snap.snapshot_digest || "snap"),
                                sequence_id: msg.sequence_id || 0,
                                timestamp: msg.timestamp_ms || Date.now(),
                                node_id: "local",
                                healthy_panels: snap.healthy_panels || snap.panel_count || 12,
                                total_panels: snap.panel_count || 12,
                                layout: snap.layout || "twelve_panel",
                                snapshot_digest: snap.snapshot_digest || ""
                            }).catch(() => {});
                        } else if (msg.event === "consensus_decision") {
                            triggerHaptic("consensus");
                        } else if (msg.event === "push_alert") {
                            triggerHaptic(msg.alert?.action_data?.severity === "CRITICAL" ? "consensus" : "tap");
                        } else if (msg.event === "peer_paired") {
                            triggerHaptic("tap");
                        }
                    } catch (_) {}
                };
                ws.onclose = () => {
                    document.getElementById("digest-pill").textContent = "SSE FALLBACK";
                    document.getElementById("digest-pill").style.color = "var(--amber)";
                    reconnectAttempts++;
                    const delay = Math.min(10000, 500 * Math.pow(1.5, reconnectAttempts));
                    setTimeout(initWebSocket, delay);
                };
                ws.onerror = () => { if (ws) ws.close(); };
            } catch (e) {
                initSSE();
            }
        }

        function initSSE() {
            if (typeof EventSource !== "undefined") {
                const es = new EventSource("/api/events");
                es.addEventListener("telemetry", e => {
                    try { renderSnapshot(JSON.parse(e.data)); } catch (_) {}
                });
                es.addEventListener("consensus", () => triggerHaptic("consensus"));
                es.addEventListener("push_alert", () => triggerHaptic("consensus"));
            }
        }

        fetch("/api/snapshot").then(r => r.json()).then(renderSnapshot).catch(() => {});
        initWebSocket();
        initSSE();

        if ("serviceWorker" in navigator) {
            window.addEventListener("load", () => {
                navigator.serviceWorker.register("/sw.js").catch(() => {});
            });
            navigator.serviceWorker.addEventListener("message", e => {
                if (e.data && e.data.type === "NAVIGATE_VIEW") {
                    if (e.data.target_view && location.hash !== e.data.target_view) {
                        location.hash = e.data.target_view;
                    }
                }
            });
        }
    </script>
</body>
</html>"##;

/// Embedded PWA manifest JSON for standalone mobile installation.
pub const PWA_MANIFEST_JSON: &str = r##"{
  "id": "/pandora-cockpit-web",
  "name": "AGENT-PANDORA Mobile Cockpit",
  "short_name": "Pandora",
  "start_url": "/?source=pwa",
  "scope": "/",
  "display": "standalone",
  "display_override": ["window-controls-overlay", "standalone", "minimal-ui"],
  "orientation": "any",
  "background_color": "#0a0d14",
  "theme_color": "#00f0ff",
  "categories": ["utilities", "developer", "productivity"],
  "description": "Real-time Autonomous Cockpit, Byzantine Swarm Telemetry, Offline Sync & Mobile Controls",
  "icons": [
    {
      "src": "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 192 192'><rect width='192' height='192' rx='38' fill='%230a0d14'/><circle cx='96' cy='96' r='68' fill='none' stroke='%2300f0ff' stroke-width='14'/><polygon points='96,44 140,124 52,124' fill='none' stroke='%2310b981' stroke-width='10' stroke-linejoin='round'/><circle cx='96' cy='96' r='14' fill='%2300f0ff'/></svg>",
      "sizes": "192x192",
      "type": "image/svg+xml",
      "purpose": "any maskable"
    },
    {
      "src": "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 512 512'><rect width='512' height='512' rx='100' fill='%230a0d14'/><circle cx='256' cy='256' r='180' fill='none' stroke='%2300f0ff' stroke-width='36'/><polygon points='256,120 376,330 136,330' fill='none' stroke='%2310b981' stroke-width='28' stroke-linejoin='round'/><circle cx='256' cy='256' r='38' fill='%2300f0ff'/></svg>",
      "sizes": "512x512",
      "type": "image/svg+xml",
      "purpose": "any maskable"
    }
  ],
  "shortcuts": [
    {
      "name": "Toggle Layout",
      "short_name": "Layout",
      "description": "Switch between 9-panel and 12-panel cockpit view",
      "url": "/?action=layout",
      "icons": [{ "src": "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 96 96'><rect width='96' height='96' rx='20' fill='%23121824'/><rect x='20' y='20' width='24' height='24' fill='%2300f0ff'/><rect x='52' y='20' width='24' height='24' fill='%2300f0ff'/><rect x='20' y='52' width='24' height='24' fill='%2300f0ff'/><rect x='52' y='52' width='24' height='24' fill='%2300f0ff'/></svg>", "sizes": "96x96" }]
    },
    {
      "name": "Trigger Swarm Consensus",
      "short_name": "Consensus",
      "description": "Trigger Theorem 43 Byzantine consensus resolution",
      "url": "/?action=consensus",
      "icons": [{ "src": "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 96 96'><rect width='96' height='96' rx='20' fill='%23121824'/><polygon points='48,16 78,76 18,76' fill='none' stroke='%2310b981' stroke-width='8'/></svg>", "sizes": "96x96" }]
    }
  ]
}"##;

/// Embedded offline Service Worker with dual caching and Web Push notifications.
pub const SERVICE_WORKER_JS: &str = r#"const CACHE_NAME = 'pandora-cockpit-v3.0.10';
const CACHE_STATIC = CACHE_NAME;
const CACHE_TELEMETRY = 'pandora-telemetry-v3.0.10';
const STATIC_ASSETS = ['/', '/index.html', '/manifest.webmanifest'];

self.addEventListener('install', (event) => {
    event.waitUntil(
        caches.open(CACHE_STATIC).then((cache) => cache.addAll(STATIC_ASSETS))
    );
    self.skipWaiting();
});

self.addEventListener('activate', (event) => {
    event.waitUntil(
        caches.keys().then((keys) =>
            Promise.all(
                keys.filter((k) => k !== CACHE_STATIC && k !== CACHE_TELEMETRY).map((k) => caches.delete(k))
            )
        )
    );
    self.clients.claim();
});

self.addEventListener('fetch', (event) => {
    const url = new URL(event.request.url);

    // 1. Dynamic Telemetry Fallback (Network-First with Dynamic Cache Fallback)
    if (url.pathname === '/api/snapshot' || url.pathname === '/api/status') {
        event.respondWith(
            fetch(event.request)
                .then((networkResp) => {
                    if (networkResp && networkResp.status === 200) {
                        const copy = networkResp.clone();
                        caches.open(CACHE_TELEMETRY).then((cache) => cache.put(event.request, copy));
                    }
                    return networkResp;
                })
                .catch(() => {
                    return caches.match(event.request).then((cached) => {
                        if (cached) return cached;
                        const fallbackSnapshot = {
                            status: "offline_cached",
                            layout: "twelve_panel",
                            panel_count: 12,
                            healthy_panels: 12,
                            degraded_panels: 0,
                            critical_panels: 0,
                            active_external_agents: 0,
                            panels: [],
                            timestamp_ms: Date.now(),
                            snapshot_digest: "PANDORA_COCKPIT_V1:OFFLINE_FALLBACK"
                        };
                        return new Response(JSON.stringify(fallbackSnapshot), {
                            headers: { 'Content-Type': 'application/json', 'X-Pandora-Offline': 'true' }
                        });
                    });
                })
        );
        return;
    }

    // 2. Action endpoints & streaming (Network-only)
    if (url.pathname.startsWith('/api/action/') || url.pathname.startsWith('/api/pair/') || url.pathname.startsWith('/api/push/')) {
        event.respondWith(
            fetch(event.request).catch(() => new Response(JSON.stringify({ status: 'offline', error: 'network_unavailable' }), {
                headers: { 'Content-Type': 'application/json' }
            }))
        );
        return;
    }

    // 3. Static App Shell (Cache-First)
    event.respondWith(
        caches.match(event.request).then((cached) => {
            if (cached) return cached;
            return fetch(event.request).then((networkResp) => {
                if (networkResp && networkResp.status === 200) {
                    const copy = networkResp.clone();
                    caches.open(CACHE_STATIC).then((cache) => cache.put(event.request, copy));
                }
                return networkResp;
            });
        })
    );
});

// ============================================================================
// Web Push Notifications & Click Handling (Milestone M3)
// ============================================================================

self.addEventListener('push', (event) => {
    let payload = {};
    if (event.data) {
        try {
            payload = event.data.json();
        } catch (_) {
            payload = {
                title: 'AGENT-PANDORA Alert',
                body: event.data.text()
            };
        }
    } else {
        payload = {
            title: 'AGENT-PANDORA Alert',
            body: 'Runtime telemetry event received'
        };
    }

    const title = payload.title || 'AGENT-PANDORA Alert';
    const severity = payload.action_data?.severity || (payload.alert_type === 'byzantine_equivocation' ? 'CRITICAL' : 'INFO');

    // Vibration pattern calibrated for Xiaomi 15 and mobile devices
    let vibratePattern = [100, 50, 100];
    if (severity === 'CRITICAL') {
        vibratePattern = [200, 100, 200, 100, 400];
    } else if (severity === 'HIGH') {
        vibratePattern = [150, 80, 150];
    }

    const options = {
        body: payload.body || 'Alert from autonomous runtime',
        icon: payload.icon || 'data:image/svg+xml,<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><circle cx="50" cy="50" r="45" fill="%230a0d14" stroke="%2300f0ff" stroke-width="6"/><text x="50" y="62" font-size="40" text-anchor="middle" fill="%2300f0ff">P</text></svg>',
        badge: payload.badge || 'data:image/svg+xml,<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><circle cx="50" cy="50" r="45" fill="%2300f0ff"/></svg>',
        tag: payload.tag || ('pandora-alert-' + (payload.timestamp_ms || Date.now())),
        renotify: true,
        vibrate: vibratePattern,
        timestamp: payload.timestamp_ms || Date.now(),
        data: {
            url: payload.action_data?.target_view || '/',
            alert_id: payload.action_data?.alert_id || '',
            digest: payload.cryptographic_digest || '',
            alert_type: payload.alert_type || '',
            severity: severity,
            metadata: payload.action_data?.metadata || {}
        },
        actions: [
            { action: 'open_dashboard', title: 'Open View' },
            { action: 'dismiss', title: 'Dismiss' }
        ]
    };

    event.waitUntil(
        self.registration.showNotification(title, options)
    );
});

self.addEventListener('notificationclick', (event) => {
    event.notification.close();

    if (event.action === 'dismiss') {
        return;
    }

    const targetUrl = event.notification.data?.url || '/';

    event.waitUntil(
        clients.matchAll({ type: 'window', includeUncontrolled: true }).then((windowClients) => {
            for (let client of windowClients) {
                if (client.url && 'focus' in client) {
                    client.postMessage({
                        type: 'NAVIGATE_VIEW',
                        target_view: targetUrl,
                        alert_data: event.notification.data
                    });
                    if ('navigate' in client) {
                        try { client.navigate(targetUrl); } catch (_) {}
                    }
                    return client.focus();
                }
            }
            if (clients.openWindow) {
                return clients.openWindow(targetUrl);
            }
        })
    );
});"#;

const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// Pure RFC 3174 SHA-1 digest implementation strictly under `#![forbid(unsafe_code)]`.
pub fn sha1_digest(data: &[u8]) -> [u8; 20] {
    let mut h0: u32 = 0x67452301;
    let mut h1: u32 = 0xEFCDAB89;
    let mut h2: u32 = 0x98BADCFE;
    let mut h3: u32 = 0x10325476;
    let mut h4: u32 = 0xC3D2E1F0;

    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0x00);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    let mut offset = 0;
    while offset + 64 <= msg.len() {
        let chunk = &msg[offset..offset + 64];
        offset += 64;
        let mut w = [0u32; 80];
        for (i, slot) in w.iter_mut().take(16).enumerate() {
            let start = i * 4;
            *slot = u32::from_be_bytes([
                chunk[start],
                chunk[start + 1],
                chunk[start + 2],
                chunk[start + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let mut a = h0;
        let mut b = h1;
        let mut c = h2;
        let mut d = h3;
        let mut e = h4;

        for (i, &item) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };

            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(item);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }

        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut out = [0u8; 20];
    out[0..4].copy_from_slice(&h0.to_be_bytes());
    out[4..8].copy_from_slice(&h1.to_be_bytes());
    out[8..12].copy_from_slice(&h2.to_be_bytes());
    out[12..16].copy_from_slice(&h3.to_be_bytes());
    out[16..20].copy_from_slice(&h4.to_be_bytes());
    out
}

/// Compute `Sec-WebSocket-Accept` from `Sec-WebSocket-Key` per RFC 6455 Section 1.3.
pub fn compute_websocket_accept(sec_key: &str) -> String {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD as BASE64;
    let combined = format!("{}{}", sec_key.trim(), WS_GUID);
    let digest = sha1_digest(combined.as_bytes());
    BASE64.encode(digest)
}

/// Encode an unmasked server text frame per RFC 6455.
pub fn encode_ws_text_frame(payload: &[u8]) -> Vec<u8> {
    let mut header = Vec::with_capacity(10 + payload.len());
    header.push(0x81); // FIN + opcode 0x1 (text)
    let len = payload.len();
    if len <= 125 {
        header.push(len as u8);
    } else if len <= 65535 {
        header.push(126);
        header.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        header.push(127);
        header.extend_from_slice(&(len as u64).to_be_bytes());
    }
    header.extend_from_slice(payload);
    header
}

/// Read and decode a client WebSocket frame (unmasking payload per RFC 6455).
pub fn read_client_ws_frame(stream: &mut TcpStream) -> IoResult<Option<(u8, Vec<u8>)>> {
    let mut hdr = [0u8; 2];
    match stream.read_exact(&mut hdr) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {
            return Ok(None);
        }
        Err(e) => return Err(e),
    }

    let opcode = hdr[0] & 0x0F;
    let masked = (hdr[1] & 0x80) != 0;
    let mut payload_len = (hdr[1] & 0x7F) as usize;

    if payload_len == 126 {
        let mut ext = [0u8; 2];
        stream.read_exact(&mut ext)?;
        payload_len = u16::from_be_bytes(ext) as usize;
    } else if payload_len == 127 {
        let mut ext = [0u8; 8];
        stream.read_exact(&mut ext)?;
        payload_len = u64::from_be_bytes(ext) as usize;
    }

    if payload_len > 1024 * 1024 {
        return Err(IoError::new(
            ErrorKind::InvalidData,
            "WebSocket frame exceeds 1MB",
        ));
    }

    let mask_key = if masked {
        let mut key = [0u8; 4];
        stream.read_exact(&mut key)?;
        Some(key)
    } else {
        None
    };

    let mut payload = vec![0u8; payload_len];
    if payload_len > 0 {
        stream.read_exact(&mut payload)?;
    }

    if let Some(key) = mask_key {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= key[i % 4];
        }
    }

    Ok(Some((opcode, payload)))
}

// ============================================================================
// Pure Safe Rust SVG QR Matrix Generator (ISO/IEC 18004 Compliant)
// Strict #![forbid(unsafe_code)] - Zero External Dependencies
// ============================================================================

pub struct QrVersionSpec {
    pub version: usize,
    pub size: usize,
    pub data_capacity: usize,
    pub total_codewords: usize,
    pub ec_per_block: usize,
    pub g1_blocks: usize,
    pub g1_data: usize,
    pub g2_blocks: usize,
    pub g2_data: usize,
    pub alignment_coords: &'static [usize],
    pub remainder_bits: usize,
}

pub const QR_VERSIONS_M: [QrVersionSpec; 10] = [
    QrVersionSpec {
        version: 1,
        size: 21,
        data_capacity: 16,
        total_codewords: 26,
        ec_per_block: 10,
        g1_blocks: 1,
        g1_data: 16,
        g2_blocks: 0,
        g2_data: 0,
        alignment_coords: &[],
        remainder_bits: 0,
    },
    QrVersionSpec {
        version: 2,
        size: 25,
        data_capacity: 28,
        total_codewords: 44,
        ec_per_block: 16,
        g1_blocks: 1,
        g1_data: 28,
        g2_blocks: 0,
        g2_data: 0,
        alignment_coords: &[6, 18],
        remainder_bits: 7,
    },
    QrVersionSpec {
        version: 3,
        size: 29,
        data_capacity: 44,
        total_codewords: 70,
        ec_per_block: 26,
        g1_blocks: 1,
        g1_data: 44,
        g2_blocks: 0,
        g2_data: 0,
        alignment_coords: &[6, 22],
        remainder_bits: 7,
    },
    QrVersionSpec {
        version: 4,
        size: 33,
        data_capacity: 64,
        total_codewords: 100,
        ec_per_block: 18,
        g1_blocks: 2,
        g1_data: 32,
        g2_blocks: 0,
        g2_data: 0,
        alignment_coords: &[6, 26],
        remainder_bits: 7,
    },
    QrVersionSpec {
        version: 5,
        size: 37,
        data_capacity: 86,
        total_codewords: 134,
        ec_per_block: 24,
        g1_blocks: 2,
        g1_data: 43,
        g2_blocks: 0,
        g2_data: 0,
        alignment_coords: &[6, 30],
        remainder_bits: 7,
    },
    QrVersionSpec {
        version: 6,
        size: 41,
        data_capacity: 108,
        total_codewords: 172,
        ec_per_block: 16,
        g1_blocks: 4,
        g1_data: 27,
        g2_blocks: 0,
        g2_data: 0,
        alignment_coords: &[6, 34],
        remainder_bits: 7,
    },
    QrVersionSpec {
        version: 7,
        size: 45,
        data_capacity: 124,
        total_codewords: 196,
        ec_per_block: 18,
        g1_blocks: 4,
        g1_data: 31,
        g2_blocks: 0,
        g2_data: 0,
        alignment_coords: &[6, 22, 38],
        remainder_bits: 0,
    },
    QrVersionSpec {
        version: 8,
        size: 49,
        data_capacity: 154,
        total_codewords: 242,
        ec_per_block: 22,
        g1_blocks: 2,
        g1_data: 38,
        g2_blocks: 2,
        g2_data: 39,
        alignment_coords: &[6, 24, 42],
        remainder_bits: 0,
    },
    QrVersionSpec {
        version: 9,
        size: 53,
        data_capacity: 182,
        total_codewords: 292,
        ec_per_block: 22,
        g1_blocks: 3,
        g1_data: 36,
        g2_blocks: 2,
        g2_data: 37,
        alignment_coords: &[6, 26, 46],
        remainder_bits: 0,
    },
    QrVersionSpec {
        version: 10,
        size: 57,
        data_capacity: 216,
        total_codewords: 346,
        ec_per_block: 26,
        g1_blocks: 4,
        g1_data: 43,
        g2_blocks: 1,
        g2_data: 44,
        alignment_coords: &[6, 28, 50],
        remainder_bits: 0,
    },
];

pub const FORMAT_INFO_M: [u16; 8] = [
    0x46ED, 0x4398, 0x4CE7, 0x4992, 0x5465, 0x5110, 0x5E4F, 0x5B3A,
];

pub fn init_gf_tables() -> ([u8; 512], [u8; 256]) {
    let mut exp = [0u8; 512];
    let mut log = [0u8; 256];
    let mut x = 1u16;
    for i in 0..255 {
        exp[i] = x as u8;
        exp[i + 255] = x as u8;
        log[x as usize] = i as u8;
        x <<= 1;
        if x >= 256 {
            x ^= 0x11D;
        }
    }
    (exp, log)
}

#[inline]
pub fn gf_mul(a: u8, b: u8, exp: &[u8; 512], log: &[u8; 256]) -> u8 {
    if a == 0 || b == 0 {
        0
    } else {
        exp[log[a as usize] as usize + log[b as usize] as usize]
    }
}

pub fn rs_generator_poly(k: usize, exp: &[u8; 512], log: &[u8; 256]) -> Vec<u8> {
    let mut g = vec![1u8];
    for i in 0..k {
        let alpha = exp[i];
        let mut next = vec![0u8; g.len() + 1];
        for (j, &c) in g.iter().enumerate() {
            next[j] ^= gf_mul(c, alpha, exp, log);
            next[j + 1] ^= c;
        }
        g = next;
    }
    g.reverse();
    g
}

pub fn rs_compute_remainder(
    data: &[u8],
    k: usize,
    g: &[u8],
    exp: &[u8; 512],
    log: &[u8; 256],
) -> Vec<u8> {
    let mut rem = vec![0u8; k];
    for &byte in data {
        let factor = byte ^ rem[0];
        rem.copy_within(1..k, 0);
        rem[k - 1] = 0;
        for j in 0..k {
            rem[j] ^= gf_mul(g[j + 1], factor, exp, log);
        }
    }
    rem
}

pub fn generate_pairing_qr_svg(payload: &str) -> String {
    let bytes = payload.as_bytes();
    let spec = QR_VERSIONS_M
        .iter()
        .find(|v| bytes.len() <= v.data_capacity)
        .unwrap_or(&QR_VERSIONS_M[QR_VERSIONS_M.len() - 1]);

    let (exp, log) = init_gf_tables();

    // 1. Bitstream assembly
    let mut bits = Vec::with_capacity(spec.data_capacity * 8);
    // Byte mode indicator: 0100
    push_bits(&mut bits, 0b0100, 4);
    // Character count indicator
    let count_bits = if spec.version <= 9 { 8 } else { 16 };
    push_bits(&mut bits, bytes.len() as u32, count_bits);
    // Payload data
    for &b in bytes {
        push_bits(&mut bits, b as u32, 8);
    }
    // Terminator
    let max_bits = spec.data_capacity * 8;
    let term_len = 4.min(max_bits.saturating_sub(bits.len()));
    push_bits(&mut bits, 0, term_len);
    // Pad to byte boundary
    while bits.len() % 8 != 0 && bits.len() < max_bits {
        bits.push(false);
    }

    let mut data_codewords = Vec::with_capacity(spec.data_capacity);
    for chunk in bits.chunks(8) {
        let mut byte = 0u8;
        for &bit in chunk {
            byte = (byte << 1) | (if bit { 1 } else { 0 });
        }
        data_codewords.push(byte);
    }
    let pad_bytes = [0xEC, 0x11];
    let mut p_idx = 0;
    while data_codewords.len() < spec.data_capacity {
        data_codewords.push(pad_bytes[p_idx % 2]);
        p_idx += 1;
    }

    // 2. Block generation and interleaving
    let g = rs_generator_poly(spec.ec_per_block, &exp, &log);
    let total_blocks = spec.g1_blocks + spec.g2_blocks;
    let mut data_blocks = Vec::with_capacity(total_blocks);
    let mut ec_blocks = Vec::with_capacity(total_blocks);

    let mut offset = 0;
    for _ in 0..spec.g1_blocks {
        let d = &data_codewords[offset..offset + spec.g1_data];
        offset += spec.g1_data;
        let e = rs_compute_remainder(d, spec.ec_per_block, &g, &exp, &log);
        data_blocks.push(d);
        ec_blocks.push(e);
    }
    for _ in 0..spec.g2_blocks {
        let d = &data_codewords[offset..offset + spec.g2_data];
        offset += spec.g2_data;
        let e = rs_compute_remainder(d, spec.ec_per_block, &g, &exp, &log);
        data_blocks.push(d);
        ec_blocks.push(e);
    }

    let mut final_codewords = Vec::with_capacity(spec.total_codewords);
    let max_data_len = spec.g1_data.max(spec.g2_data);
    for col in 0..max_data_len {
        for block in &data_blocks {
            if col < block.len() {
                final_codewords.push(block[col]);
            }
        }
    }
    for col in 0..spec.ec_per_block {
        for block in &ec_blocks {
            final_codewords.push(block[col]);
        }
    }

    let mut interleaved_bits = Vec::with_capacity(final_codewords.len() * 8 + spec.remainder_bits);
    for &b in &final_codewords {
        push_bits(&mut interleaved_bits, b as u32, 8);
    }
    interleaved_bits.extend(std::iter::repeat_n(false, spec.remainder_bits));

    // 3. Matrix population
    let mut matrix = QrMatrix::new(spec.size);
    matrix.populate_patterns(spec);
    place_data_bits(&mut matrix, &interleaved_bits);

    let best_mask = select_optimal_mask(&matrix);
    apply_mask_and_format(&mut matrix, best_mask);

    render_svg(&matrix)
}

fn push_bits(bits: &mut Vec<bool>, val: u32, count: usize) {
    for i in (0..count).rev() {
        bits.push(((val >> i) & 1) != 0);
    }
}

#[derive(Clone)]
pub struct QrMatrix {
    pub size: usize,
    pub modules: Vec<Vec<bool>>,
    pub is_function: Vec<Vec<bool>>,
}

impl QrMatrix {
    pub fn new(size: usize) -> Self {
        Self {
            size,
            modules: vec![vec![false; size]; size],
            is_function: vec![vec![false; size]; size],
        }
    }

    pub fn set_function(&mut self, r: usize, c: usize, val: bool) {
        if r < self.size && c < self.size {
            self.modules[r][c] = val;
            self.is_function[r][c] = true;
        }
    }

    pub fn populate_patterns(&mut self, spec: &QrVersionSpec) {
        let s = self.size;
        // Finder patterns
        self.draw_finder(0, 0);
        self.draw_finder(0, s - 7);
        self.draw_finder(s - 7, 0);

        // Separators
        for c in 0..8 {
            self.set_function(7, c, false);
        }
        for r in 0..8 {
            self.set_function(r, 7, false);
        }
        for c in (s - 8)..s {
            self.set_function(7, c, false);
        }
        for r in 0..8 {
            self.set_function(r, s - 8, false);
        }
        for c in 0..8 {
            self.set_function(s - 8, c, false);
        }
        for r in (s - 8)..s {
            self.set_function(r, 7, false);
        }

        // Timing patterns
        for c in 8..(s - 8) {
            self.set_function(6, c, c % 2 == 0);
        }
        for r in 8..(s - 8) {
            self.set_function(r, 6, r % 2 == 0);
        }

        // Dark module
        self.set_function(4 * spec.version + 9, 8, true);

        // Alignment patterns
        for &r in spec.alignment_coords {
            for &c in spec.alignment_coords {
                if (r < 9 && c < 9) || (r < 9 && c >= s - 9) || (r >= s - 9 && c < 9) {
                    continue;
                }
                self.draw_alignment(r, c);
            }
        }

        // Format info reservations
        for c in 0..=8 {
            if c != 6 {
                self.is_function[8][c] = true;
            }
        }
        for r in 0..=8 {
            if r != 6 {
                self.is_function[r][8] = true;
            }
        }
        for c in (s - 8)..s {
            self.is_function[8][c] = true;
        }
        for r in (s - 7)..s {
            self.is_function[r][8] = true;
        }

        // Version info
        if spec.version >= 7 {
            self.write_version_info(spec.version);
        }
    }

    fn draw_finder(&mut self, top_r: usize, top_c: usize) {
        for dr in 0..7 {
            for dc in 0..7 {
                let dark = dr == 0
                    || dr == 6
                    || dc == 0
                    || dc == 6
                    || ((2..=4).contains(&dr) && (2..=4).contains(&dc));
                self.set_function(top_r + dr, top_c + dc, dark);
            }
        }
    }

    fn draw_alignment(&mut self, center_r: usize, center_c: usize) {
        let top_r = center_r.saturating_sub(2);
        let top_c = center_c.saturating_sub(2);
        for dr in 0..5 {
            for dc in 0..5 {
                let dark = dr == 0 || dr == 4 || dc == 0 || dc == 4 || (dr == 2 && dc == 2);
                self.set_function(top_r + dr, top_c + dc, dark);
            }
        }
    }

    fn write_version_info(&mut self, version: usize) {
        let v_code: u32 = match version {
            7 => 0x07C94,
            8 => 0x085BC,
            9 => 0x09A99,
            10 => 0x0A4D3,
            _ => return,
        };
        let s = self.size;
        for i in 0..18 {
            let bit = ((v_code >> i) & 1) != 0;
            self.set_function((s - 11) + (i % 3), i / 3, bit);
            self.set_function(i / 3, (s - 11) + (i % 3), bit);
        }
    }
}

pub fn place_data_bits(matrix: &mut QrMatrix, bits: &[bool]) {
    let s = matrix.size;
    let mut bit_idx = 0;
    let mut right_col = s - 1;
    let mut moving_up = true;

    while right_col > 0 {
        if right_col == 6 {
            right_col -= 1;
        }
        for step in 0..s {
            let r = if moving_up { s - 1 - step } else { step };
            for &c in &[right_col, right_col - 1] {
                if !matrix.is_function[r][c] {
                    let bit = if bit_idx < bits.len() {
                        bits[bit_idx]
                    } else {
                        false
                    };
                    matrix.modules[r][c] = bit;
                    bit_idx += 1;
                }
            }
        }
        moving_up = !moving_up;
        right_col = right_col.saturating_sub(2);
    }
}

pub fn mask_cond(m: usize, r: usize, c: usize) -> bool {
    match m {
        0 => (r + c).is_multiple_of(2),
        1 => r.is_multiple_of(2),
        2 => c.is_multiple_of(3),
        3 => (r + c).is_multiple_of(3),
        4 => (r / 2 + c / 3).is_multiple_of(2),
        5 => ((r * c) % 2) + ((r * c) % 3) == 0,
        6 => (((r * c) % 2) + ((r * c) % 3)).is_multiple_of(2),
        7 => (((r + c) % 2) + ((r * c) % 3)).is_multiple_of(2),
        _ => false,
    }
}

pub fn apply_mask_and_format(matrix: &mut QrMatrix, mask: usize) {
    let s = matrix.size;
    for r in 0..s {
        for c in 0..s {
            if !matrix.is_function[r][c] {
                matrix.modules[r][c] ^= mask_cond(mask, r, c);
            }
        }
    }
    let fmt_val = FORMAT_INFO_M[mask];
    let fmt_bits: Vec<bool> = (0..15).map(|i| ((fmt_val >> i) & 1) != 0).collect();

    for (i, &bit) in fmt_bits.iter().take(6).enumerate() {
        matrix.modules[8][i] = bit;
    }
    matrix.modules[8][7] = fmt_bits[6];
    matrix.modules[8][8] = fmt_bits[7];
    matrix.modules[7][8] = fmt_bits[8];
    for (i, r) in (0..=5).rev().enumerate() {
        matrix.modules[r][8] = fmt_bits[9 + i];
    }

    for (i, &bit) in fmt_bits.iter().take(7).enumerate() {
        matrix.modules[s - 1 - i][8] = bit;
    }
    for (i, &bit) in fmt_bits.iter().skip(7).enumerate() {
        matrix.modules[8][s - 8 + i] = bit;
    }
}

pub fn calculate_penalty(matrix: &QrMatrix) -> u32 {
    let s = matrix.size;
    let mut penalty = 0u32;

    for r in 0..s {
        let mut count = 0;
        let mut prev = matrix.modules[r][0];
        for c in 0..s {
            if matrix.modules[r][c] == prev {
                count += 1;
            } else {
                if count >= 5 {
                    penalty += 3 + (count - 5) as u32;
                }
                prev = matrix.modules[r][c];
                count = 1;
            }
        }
        if count >= 5 {
            penalty += 3 + (count - 5) as u32;
        }
    }

    for c in 0..s {
        let mut count = 0;
        let mut prev = matrix.modules[0][c];
        for r in 0..s {
            if matrix.modules[r][c] == prev {
                count += 1;
            } else {
                if count >= 5 {
                    penalty += 3 + (count - 5) as u32;
                }
                prev = matrix.modules[r][c];
                count = 1;
            }
        }
        if count >= 5 {
            penalty += 3 + (count - 5) as u32;
        }
    }

    for r in 0..(s - 1) {
        for c in 0..(s - 1) {
            let color = matrix.modules[r][c];
            if matrix.modules[r + 1][c] == color
                && matrix.modules[r][c + 1] == color
                && matrix.modules[r + 1][c + 1] == color
            {
                penalty += 3;
            }
        }
    }

    let pat1 = [
        false, false, false, false, true, false, true, true, true, false, true,
    ];
    let pat2 = [
        true, false, true, true, true, false, true, false, false, false, false,
    ];
    for r in 0..s {
        for c in 0..=(s.saturating_sub(11)) {
            let sub: Vec<bool> = (0..11).map(|i| matrix.modules[r][c + i]).collect();
            if sub == pat1 || sub == pat2 {
                penalty += 40;
            }
        }
    }
    for c in 0..s {
        for r in 0..=(s.saturating_sub(11)) {
            let sub: Vec<bool> = (0..11).map(|i| matrix.modules[r + i][c]).collect();
            if sub == pat1 || sub == pat2 {
                penalty += 40;
            }
        }
    }

    let total_modules = (s * s) as u32;
    let dark_count = matrix
        .modules
        .iter()
        .flat_map(|row| row.iter())
        .filter(|&&b| b)
        .count() as u32;
    let pct = (dark_count * 100) / total_modules;
    let diff = pct.abs_diff(50);
    penalty += (diff / 5) * 10;

    penalty
}

pub fn select_optimal_mask(base_matrix: &QrMatrix) -> usize {
    let mut best_mask = 0;
    let mut min_penalty = u32::MAX;
    for m in 0..8 {
        let mut candidate = base_matrix.clone();
        apply_mask_and_format(&mut candidate, m);
        let pen = calculate_penalty(&candidate);
        if pen < min_penalty {
            min_penalty = pen;
            best_mask = m;
        }
    }
    best_mask
}

pub fn render_svg(matrix: &QrMatrix) -> String {
    let quiet_zone = 4;
    let s = matrix.size;
    let total_dim = s + 2 * quiet_zone;

    let mut svg = String::with_capacity(4096);
    svg.push_str(&format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {total_dim} {total_dim}" width="{}" height="{}" shape-rendering="crispEdges">"##,
        total_dim * 8,
        total_dim * 8
    ));
    svg.push_str(r##"<rect width="100%" height="100%" fill="#ffffff"/>"##);

    for r in 0..s {
        let mut c = 0;
        while c < s {
            if matrix.modules[r][c] {
                let start_c = c;
                while c < s && matrix.modules[r][c] {
                    c += 1;
                }
                let run_len = c - start_c;
                let x = start_c + quiet_zone;
                let y = r + quiet_zone;
                svg.push_str(&format!(
                    r##"<rect x="{x}" y="{y}" width="{run_len}" height="1" fill="#000000"/>"##
                ));
            } else {
                c += 1;
            }
        }
    }
    svg.push_str("</svg>");
    svg
}

// ============================================================================
// Mobile Pairing Subsystem (Ed25519 verify_strict & Swarm Peer Admission)
// ============================================================================

/// Ephemeral pairing session record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MobilePairingSession {
    pub token: String,
    pub challenge: String,
    pub workstation_pubkey: String,
    pub created_at_sec: u64,
    pub expires_at_sec: u64,
    pub consumed: bool,
}

/// Request payload for `POST /api/pair/exchange`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairExchangeRequest {
    pub token: String,
    pub mobile_pubkey: String,
    pub signature: String,
    #[serde(default)]
    pub device_name: Option<String>,
}

/// Response payload for `POST /api/pair/exchange`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairExchangeResponse {
    pub status: String,
    pub peer_id: String,
    pub workstation_pubkey: String,
    pub session_token: String,
    pub expires_at_sec: u64,
    pub cluster_epoch: u64,
}

/// Response payload for `GET /api/pair/qr`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairQrResponse {
    pub status: String,
    pub token: String,
    pub challenge: String,
    pub workstation_pubkey: String,
    pub expires_at: u64,
    pub qr_svg: String,
    pub pairing_uri: String,
}

/// In-memory authenticated mobile peer record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticatedMobilePeer {
    pub peer_id: String,
    pub public_key: String,
    pub device_name: String,
    pub session_token: String,
    pub paired_at_sec: u64,
    pub last_seen_sec: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairingError {
    SessionNotFound,
    SessionExpired,
    AlreadyConsumed,
    InvalidHexPublicKey,
    InvalidPublicKey(String),
    InvalidHexSignature,
    SignatureVerificationFailed(String),
}

impl std::fmt::Display for PairingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SessionNotFound => write!(f, "pairing session not found"),
            Self::SessionExpired => write!(f, "pairing session expired"),
            Self::AlreadyConsumed => write!(f, "pairing session already consumed"),
            Self::InvalidHexPublicKey => {
                write!(f, "invalid hex mobile public key (expected 64 hex chars)")
            }
            Self::InvalidPublicKey(msg) => write!(f, "invalid Ed25519 public key: {msg}"),
            Self::InvalidHexSignature => {
                write!(f, "invalid hex signature (expected 128 hex chars)")
            }
            Self::SignatureVerificationFailed(msg) => {
                write!(f, "cryptographic challenge verification failed: {msg}")
            }
        }
    }
}

pub struct PairingManager {
    pub identity: PeerIdentity,
    pub active_sessions: HashMap<String, MobilePairingSession>,
    pub authenticated_peers: HashMap<String, AuthenticatedMobilePeer>,
    pub swarm_peers: HashMap<String, SwarmPeer>,
}

impl PairingManager {
    pub fn new(identity: PeerIdentity) -> Self {
        Self {
            identity,
            active_sessions: HashMap::new(),
            authenticated_peers: HashMap::new(),
            swarm_peers: HashMap::new(),
        }
    }

    pub fn prune_expired(&mut self, now_sec: u64) {
        self.active_sessions
            .retain(|_, s| s.expires_at_sec > now_sec);
    }

    pub fn create_session(&mut self, now_sec: u64, ttl_sec: u64) -> MobilePairingSession {
        let mut token_bytes = [0u8; 16];
        let mut challenge_bytes = [0u8; 16];
        getrandom::fill(&mut token_bytes).expect("random token available");
        getrandom::fill(&mut challenge_bytes).expect("random challenge available");

        let token = encode_hex(&token_bytes);
        let challenge = encode_hex(&challenge_bytes);
        let workstation_pubkey = self.identity.public_key_hex();

        let session = MobilePairingSession {
            token: token.clone(),
            challenge,
            workstation_pubkey,
            created_at_sec: now_sec,
            expires_at_sec: now_sec + ttl_sec,
            consumed: false,
        };

        self.active_sessions.insert(token, session.clone());
        session
    }

    pub fn exchange(
        &mut self,
        req: &PairExchangeRequest,
        now_sec: u64,
    ) -> Result<(PairExchangeResponse, SwarmPeer), PairingError> {
        let session = self
            .active_sessions
            .get_mut(&req.token)
            .ok_or(PairingError::SessionNotFound)?;

        if session.consumed {
            return Err(PairingError::AlreadyConsumed);
        }
        if now_sec > session.expires_at_sec {
            return Err(PairingError::SessionExpired);
        }

        // Domain-separated challenge message
        let challenge_msg = format!("pandora-pair-v1:{}:{}", session.token, session.challenge);

        let pk_bytes =
            decode_hex_32(&req.mobile_pubkey).ok_or(PairingError::InvalidHexPublicKey)?;
        let verifying_key = VerifyingKey::from_bytes(&pk_bytes)
            .map_err(|e| PairingError::InvalidPublicKey(e.to_string()))?;

        let sig_bytes = decode_hex_64(&req.signature).ok_or(PairingError::InvalidHexSignature)?;
        let signature = Signature::from_bytes(&sig_bytes);

        // Strict verification
        verifying_key
            .verify_strict(challenge_msg.as_bytes(), &signature)
            .map_err(|e| PairingError::SignatureVerificationFailed(e.to_string()))?;

        // Mark consumed to prevent replays
        session.consumed = true;

        let peer_id = format!("peer-mobile-{}", &req.mobile_pubkey[..16]);
        let mut session_token_bytes = [0u8; 32];
        getrandom::fill(&mut session_token_bytes).expect("random session token available");
        let session_token = encode_hex(&session_token_bytes);

        let mobile_peer = AuthenticatedMobilePeer {
            peer_id: peer_id.clone(),
            public_key: req.mobile_pubkey.clone(),
            device_name: req
                .device_name
                .clone()
                .unwrap_or_else(|| "mobile-device".to_string()),
            session_token: session_token.clone(),
            paired_at_sec: now_sec,
            last_seen_sec: now_sec,
        };
        self.authenticated_peers
            .insert(session_token.clone(), mobile_peer);

        let swarm_peer = SwarmPeer {
            peer_id: peer_id.clone(),
            listen_addr: "mobile-pwa".to_string(),
            public_key: req.mobile_pubkey.clone(),
            last_seen: now_sec * 1000,
            connected: true,
        };
        self.swarm_peers.insert(peer_id.clone(), swarm_peer.clone());

        let resp = PairExchangeResponse {
            status: "paired".to_string(),
            peer_id,
            workstation_pubkey: self.identity.public_key_hex(),
            session_token,
            expires_at_sec: now_sec + 86400,
            cluster_epoch: 1,
        };

        Ok((resp, swarm_peer))
    }
}

// ============================================================================
// Sequenced Telemetry & Reconnect Sync Ring Buffer (Milestone M3)
// ============================================================================

/// Individual sequenced telemetry packet stored in server ring buffer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SequencedTelemetryPacket {
    pub sequence_id: u64,
    pub timestamp_ms: u64,
    pub kind: String,
    pub payload: Value,
}

pub enum SyncQueryResult {
    UpToDate {
        current_seq: u64,
    },
    CatchUp {
        current_seq: u64,
        packets: Vec<SequencedTelemetryPacket>,
    },
    GapOverflow {
        current_seq: u64,
        oldest_available_seq: u64,
    },
}

pub struct TelemetryRingBuffer {
    capacity: usize,
    buffer: Vec<Option<SequencedTelemetryPacket>>,
    head: usize,
    next_sequence_id: u64,
    oldest_sequence_id: u64,
}

impl TelemetryRingBuffer {
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.max(64);
        Self {
            capacity: cap,
            buffer: vec![None; cap],
            head: 0,
            next_sequence_id: 1,
            oldest_sequence_id: 1,
        }
    }

    pub fn push(&mut self, kind: &str, payload: Value, timestamp_ms: u64) -> u64 {
        let seq = self.next_sequence_id;
        self.next_sequence_id += 1;

        let packet = SequencedTelemetryPacket {
            sequence_id: seq,
            timestamp_ms,
            kind: kind.to_string(),
            payload,
        };

        self.buffer[self.head] = Some(packet);
        self.head = (self.head + 1) % self.capacity;

        if seq > self.capacity as u64 {
            self.oldest_sequence_id = seq - self.capacity as u64 + 1;
        } else {
            self.oldest_sequence_id = 1;
        }

        seq
    }

    pub fn query_missing(&self, since_seq: u64, since_timestamp_ms: u64) -> SyncQueryResult {
        let current_seq = self.next_sequence_id - 1;

        if since_seq >= current_seq {
            return SyncQueryResult::UpToDate { current_seq };
        }

        if since_seq > 0 && since_seq < self.oldest_sequence_id {
            return SyncQueryResult::GapOverflow {
                current_seq,
                oldest_available_seq: self.oldest_sequence_id,
            };
        }

        let mut packets = Vec::new();
        for item in self.buffer.iter().flatten() {
            if item.sequence_id > since_seq && item.timestamp_ms >= since_timestamp_ms {
                packets.push(item.clone());
            }
        }
        packets.sort_by_key(|p| p.sequence_id);

        SyncQueryResult::CatchUp {
            current_seq,
            packets,
        }
    }

    pub fn current_seq(&self) -> u64 {
        self.next_sequence_id - 1
    }

    pub fn oldest_seq(&self) -> u64 {
        self.oldest_sequence_id
    }
}

// ============================================================================
// Web Push Subscriptions and Notification Dispatcher (Milestone M3)
// ============================================================================

/// Registered Web Push client subscription (W3C Push API / RFC 8291).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushSubscription {
    pub endpoint: String,
    pub p256dh_key: String,
    pub auth_token: String,
    pub created_at_ms: u64,
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushSubscribeRequest {
    pub endpoint: String,
    pub p256dh_key: Option<String>,
    pub auth_token: Option<String>,
    pub keys: Option<PushSubscriptionKeys>,
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushSubscriptionKeys {
    pub p256dh: String,
    pub auth: String,
}

impl PushSubscribeRequest {
    pub fn into_subscription(self, now_ms: u64) -> Result<PushSubscription, &'static str> {
        let endpoint = self.endpoint.trim().to_string();
        if endpoint.is_empty()
            || (!endpoint.starts_with("https://") && !endpoint.starts_with("http://"))
        {
            return Err("invalid_or_missing_endpoint");
        }

        let (p256dh, auth) = if let Some(keys) = self.keys {
            (keys.p256dh.trim().to_string(), keys.auth.trim().to_string())
        } else {
            (
                self.p256dh_key.unwrap_or_default().trim().to_string(),
                self.auth_token.unwrap_or_default().trim().to_string(),
            )
        };

        if p256dh.is_empty() {
            return Err("missing_p256dh_key");
        }
        if auth.is_empty() {
            return Err("missing_auth_token");
        }

        Ok(PushSubscription {
            endpoint,
            p256dh_key: p256dh,
            auth_token: auth,
            created_at_ms: now_ms,
            user_agent: self.user_agent,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PushAlertType {
    ByzantineEquivocation,
    CriticalTestRegression,
    ConsensusQuorumCompleted,
}

impl PushAlertType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ByzantineEquivocation => "byzantine_equivocation",
            Self::CriticalTestRegression => "critical_test_regression",
            Self::ConsensusQuorumCompleted => "consensus_quorum_completed",
        }
    }

    pub fn default_severity(&self) -> &'static str {
        match self {
            Self::ByzantineEquivocation => "CRITICAL",
            Self::CriticalTestRegression => "HIGH",
            Self::ConsensusQuorumCompleted => "INFO",
        }
    }

    pub fn default_target_view(&self) -> &'static str {
        match self {
            Self::ByzantineEquivocation => "/#security",
            Self::CriticalTestRegression => "/#canary",
            Self::ConsensusQuorumCompleted => "/#consensus",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushActionData {
    pub target_view: String,
    pub alert_id: String,
    pub severity: String,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushNotificationPayload {
    pub alert_type: PushAlertType,
    pub title: String,
    pub body: String,
    pub icon: String,
    pub badge: String,
    pub tag: String,
    pub timestamp_ms: u64,
    pub cryptographic_digest: String,
    pub action_data: PushActionData,
}

impl PushNotificationPayload {
    pub fn new(
        alert_type: PushAlertType,
        title: impl Into<String>,
        body: impl Into<String>,
        tag: impl Into<String>,
        action_data: PushActionData,
        now_ms: u64,
    ) -> Self {
        let title = title.into();
        let body = body.into();
        let tag = tag.into();
        let icon = match alert_type {
            PushAlertType::ByzantineEquivocation => "/icons/alert-critical.png".to_string(),
            PushAlertType::CriticalTestRegression => "/icons/alert-regression.png".to_string(),
            PushAlertType::ConsensusQuorumCompleted => "/icons/alert-quorum.png".to_string(),
        };
        let badge = "/icons/badge-pandora.png".to_string();

        let mut hasher = Sha256::new();
        hasher.update(b"PANDORA_PUSH_V1:");
        hasher.update(alert_type.as_str().as_bytes());
        hasher.update(title.as_bytes());
        hasher.update(body.as_bytes());
        hasher.update(action_data.target_view.as_bytes());
        hasher.update(action_data.alert_id.as_bytes());
        hasher.update(now_ms.to_be_bytes());
        for (k, v) in &action_data.metadata {
            hasher.update(k.as_bytes());
            hasher.update(v.as_bytes());
        }
        let digest_hex = format!("{:x}", hasher.finalize());
        let cryptographic_digest = format!("PANDORA_PUSH_V1:{digest_hex}");

        Self {
            alert_type,
            title,
            body,
            icon,
            badge,
            tag,
            timestamp_ms: now_ms,
            cryptographic_digest,
            action_data,
        }
    }

    pub fn byzantine_equivocation(
        voter: &str,
        proposal: &str,
        details: Option<&str>,
        now_ms: u64,
    ) -> Self {
        let alert_id = format!("equiv-{proposal}-{voter}-{now_ms}");
        let mut metadata = BTreeMap::new();
        metadata.insert("voter_node_id".to_string(), voter.to_string());
        metadata.insert("proposal_id".to_string(), proposal.to_string());
        metadata.insert("action".to_string(), "QUARANTINE_NODE".to_string());
        metadata.insert(
            "theorem".to_string(),
            "Theorem 47 Byzantine Equivocation Lemma 47.2".to_string(),
        );
        if let Some(d) = details {
            metadata.insert("details".to_string(), d.to_string());
        }

        let action_data = PushActionData {
            target_view: "/#security".to_string(),
            alert_id: alert_id.clone(),
            severity: "CRITICAL".to_string(),
            metadata,
        };

        Self::new(
            PushAlertType::ByzantineEquivocation,
            format!("🚨 [CRITICAL] Byzantine Equivocation: {voter}"),
            format!(
                "Node '{voter}' cast conflicting/divergent votes on proposal '{proposal}'. Node quarantined."
            ),
            format!("pandora-equivocation-{proposal}"),
            action_data,
            now_ms,
        )
    }

    pub fn critical_test_regression(
        canary_id: &str,
        lcb95: f64,
        baseline: f64,
        passes: u32,
        failures: u32,
        now_ms: u64,
    ) -> Self {
        let alert_id = format!("regr-{canary_id}-{now_ms}");
        let mut metadata = BTreeMap::new();
        metadata.insert("canary_id".to_string(), canary_id.to_string());
        metadata.insert("lcb95".to_string(), format!("{lcb95:.4}"));
        metadata.insert("baseline".to_string(), format!("{baseline:.4}"));
        metadata.insert("passes".to_string(), passes.to_string());
        metadata.insert("failures".to_string(), failures.to_string());
        metadata.insert("action".to_string(), "ABORT_PROMOTION".to_string());

        let action_data = PushActionData {
            target_view: "/#canary".to_string(),
            alert_id: alert_id.clone(),
            severity: "HIGH".to_string(),
            metadata,
        };

        Self::new(
            PushAlertType::CriticalTestRegression,
            format!("⚠️ [REGRESSION] Canary LCB95 Breach: {canary_id}"),
            format!(
                "Canary '{canary_id}' failed Bayesian gate (LCB95 {lcb95:.4} <= baseline {baseline:.4}, {passes}P/{failures}F). Mainline promotion aborted."
            ),
            format!("pandora-canary-regr-{canary_id}"),
            action_data,
            now_ms,
        )
    }

    pub fn consensus_quorum_completed(
        proposal_id: &str,
        winning_agent: &str,
        quorum_size: usize,
        total_nodes: usize,
        certificate_digest: &str,
        now_ms: u64,
    ) -> Self {
        let alert_id = format!("quorum-{proposal_id}-{now_ms}");
        let mut metadata = BTreeMap::new();
        metadata.insert("proposal_id".to_string(), proposal_id.to_string());
        metadata.insert("winning_agent".to_string(), winning_agent.to_string());
        metadata.insert("quorum_size".to_string(), quorum_size.to_string());
        metadata.insert("total_nodes".to_string(), total_nodes.to_string());
        metadata.insert(
            "certificate_digest".to_string(),
            certificate_digest.to_string(),
        );
        metadata.insert("action".to_string(), "STAGE_PROMOTION".to_string());

        let action_data = PushActionData {
            target_view: "/#consensus".to_string(),
            alert_id: alert_id.clone(),
            severity: "INFO".to_string(),
            metadata,
        };

        Self::new(
            PushAlertType::ConsensusQuorumCompleted,
            format!("✅ [QUORUM] Consensus Achieved: {proposal_id}"),
            format!(
                "Supermajority quorum reached ({quorum_size}/{total_nodes} nodes) for proposal '{proposal_id}' by {winning_agent}. Staged for promotion with cert {certificate_digest}."
            ),
            format!("pandora-quorum-{proposal_id}"),
            action_data,
            now_ms,
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushDispatchSummary {
    pub total_subscriptions: usize,
    pub successfully_dispatched: usize,
    pub failed_dispatches: usize,
    pub pruned_subscriptions: usize,
    pub cryptographic_digest: String,
    pub timestamp_ms: u64,
}

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
    ws_clients: Arc<Mutex<Vec<TcpStream>>>,
    shutdown_flag: Arc<AtomicBool>,
    pairing_manager: Arc<Mutex<PairingManager>>,
    telemetry_ring: Arc<Mutex<TelemetryRingBuffer>>,
    push_subscriptions: Arc<Mutex<BTreeMap<String, PushSubscription>>>,
    recent_alerts: Arc<Mutex<Vec<PushNotificationPayload>>>,
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

        let identity = PeerIdentity::new_random();
        let pairing_manager = Arc::new(Mutex::new(PairingManager::new(identity)));
        let telemetry_ring = Arc::new(Mutex::new(TelemetryRingBuffer::new(256)));
        let push_subscriptions = Arc::new(Mutex::new(BTreeMap::new()));
        let recent_alerts = Arc::new(Mutex::new(Vec::new()));

        Ok(Self {
            config,
            local_addr,
            listener,
            controller,
            approval_store,
            approvals_map: Arc::new(Mutex::new(BTreeMap::new())),
            sse_clients: Arc::new(Mutex::new(Vec::new())),
            ws_clients: Arc::new(Mutex::new(Vec::new())),
            shutdown_flag: Arc::new(AtomicBool::new(false)),
            pairing_manager,
            telemetry_ring,
            push_subscriptions,
            recent_alerts,
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

    /// Return a cloneable handle to the pairing manager.
    pub fn pairing_manager_handle(&self) -> Arc<Mutex<PairingManager>> {
        Arc::clone(&self.pairing_manager)
    }

    /// Return a cloneable handle to the telemetry ring buffer.
    pub fn telemetry_ring_handle(&self) -> Arc<Mutex<TelemetryRingBuffer>> {
        Arc::clone(&self.telemetry_ring)
    }

    /// Return a cloneable handle to the push subscription registry.
    pub fn push_subscriptions_handle(&self) -> Arc<Mutex<BTreeMap<String, PushSubscription>>> {
        Arc::clone(&self.push_subscriptions)
    }

    /// Return a cloneable handle to the recent alerts buffer.
    pub fn recent_alerts_handle(&self) -> Arc<Mutex<Vec<PushNotificationPayload>>> {
        Arc::clone(&self.recent_alerts)
    }

    /// Dispatch a push notification payload across all subscriptions, SSE, and WebSocket channels.
    pub fn dispatch_push_alert(&self, payload: PushNotificationPayload) -> PushDispatchSummary {
        Self::dispatch_alert_internal(
            &self.push_subscriptions,
            &self.recent_alerts,
            &self.sse_clients,
            &self.ws_clients,
            payload,
        )
    }

    /// Static helper to dispatch an alert from any worker or thread.
    pub fn dispatch_alert_internal(
        push_subscriptions: &Arc<Mutex<BTreeMap<String, PushSubscription>>>,
        recent_alerts: &Arc<Mutex<Vec<PushNotificationPayload>>>,
        sse_clients: &Arc<Mutex<Vec<TcpStream>>>,
        ws_clients: &Arc<Mutex<Vec<TcpStream>>>,
        payload: PushNotificationPayload,
    ) -> PushDispatchSummary {
        let now_ms = current_epoch_ms();
        let digest = payload.cryptographic_digest.clone();

        // 1. Record in recent alerts buffer (bounded to 100 entries)
        if let Ok(mut alerts) = recent_alerts.lock() {
            alerts.push(payload.clone());
            if alerts.len() > 100 {
                alerts.remove(0);
            }
        }

        // 2. Broadcast immediately over SSE (`event: push_alert`)
        let payload_val = serde_json::to_value(&payload).unwrap_or_default();
        Self::broadcast_sse(sse_clients, "push_alert", &payload_val);

        // 3. Broadcast immediately over WebSocket (`"event": "push_alert"`)
        Self::broadcast_ws(
            ws_clients,
            &json!({
                "event": "push_alert",
                "timestamp_ms": now_ms,
                "alert": &payload_val,
            }),
        );

        // 4. Dispatch via HTTP POST to registered Web Push endpoints
        let subscriptions: Vec<PushSubscription> = match push_subscriptions.lock() {
            Ok(guard) => guard.values().cloned().collect(),
            Err(p) => p.into_inner().values().cloned().collect(),
        };

        let total = subscriptions.len();
        let mut success = 0;
        let mut failed = 0;
        let mut pruned = Vec::new();

        if total > 0 {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(3))
                .build()
                .unwrap_or_default();

            let json_body = serde_json::to_string(&payload).unwrap_or_default();

            for sub in subscriptions {
                let res = client
                    .post(&sub.endpoint)
                    .header("Content-Type", "application/json")
                    .header("TTL", "86400")
                    .header("Urgency", "high")
                    .body(json_body.clone())
                    .send();

                match res {
                    Ok(resp) => {
                        let status = resp.status().as_u16();
                        if (200..300).contains(&status) {
                            success += 1;
                        } else if status == 404 || status == 410 {
                            pruned.push(sub.endpoint.clone());
                            failed += 1;
                        } else {
                            failed += 1;
                        }
                    }
                    Err(_) => {
                        failed += 1;
                    }
                }
            }

            if !pruned.is_empty()
                && let Ok(mut guard) = push_subscriptions.lock()
            {
                for endpoint in &pruned {
                    guard.remove(endpoint);
                }
            }
        }

        PushDispatchSummary {
            total_subscriptions: total,
            successfully_dispatched: success,
            failed_dispatches: failed,
            pruned_subscriptions: pruned.len(),
            cryptographic_digest: digest,
            timestamp_ms: now_ms,
        }
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

    /// Broadcast an RFC 6455 unmasked text frame to all active WebSocket clients.
    fn broadcast_ws(ws_clients: &Arc<Mutex<Vec<TcpStream>>>, payload: &Value) {
        let text = serde_json::to_string(payload).unwrap_or_else(|_| "{}".to_string());
        let frame = encode_ws_text_frame(text.as_bytes());
        if let Ok(mut clients) = ws_clients.lock() {
            clients.retain_mut(|stream| stream.write_all(&frame).is_ok() && stream.flush().is_ok());
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
                    let ws_clients = Arc::clone(&self.ws_clients);
                    let shutdown_flag = Arc::clone(&self.shutdown_flag);
                    let pairing_manager = Arc::clone(&self.pairing_manager);
                    let telemetry_ring = Arc::clone(&self.telemetry_ring);
                    let push_subscriptions = Arc::clone(&self.push_subscriptions);
                    let recent_alerts = Arc::clone(&self.recent_alerts);
                    let local_port = self.local_addr.port();
                    thread::spawn(move || {
                        let _ = handle_connection(
                            stream,
                            controller,
                            approval_store,
                            approvals_map,
                            sse_clients,
                            ws_clients,
                            shutdown_flag,
                            pairing_manager,
                            telemetry_ring,
                            push_subscriptions,
                            recent_alerts,
                            local_port,
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
                let now_ms = current_epoch_ms();
                let seq = if let Ok(mut ring) = self.telemetry_ring.lock() {
                    ring.push("snapshot", snap_val.clone(), now_ms)
                } else {
                    0
                };
                Self::broadcast_sse(&self.sse_clients, "telemetry", &snap_val);
                Self::broadcast_ws(
                    &self.ws_clients,
                    &json!({
                        "event": "telemetry",
                        "sequence_id": seq,
                        "timestamp_ms": now_ms,
                        "snapshot": snap_val,
                    }),
                );
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

#[allow(clippy::too_many_arguments)]
fn handle_connection(
    mut stream: TcpStream,
    controller: Arc<Mutex<CockpitController>>,
    _approval_store: Option<Arc<ApprovalStore>>,
    approvals_map: Arc<Mutex<BTreeMap<String, String>>>,
    sse_clients: Arc<Mutex<Vec<TcpStream>>>,
    ws_clients: Arc<Mutex<Vec<TcpStream>>>,
    shutdown_flag: Arc<AtomicBool>,
    pairing_manager: Arc<Mutex<PairingManager>>,
    telemetry_ring: Arc<Mutex<TelemetryRingBuffer>>,
    push_subscriptions: Arc<Mutex<BTreeMap<String, PushSubscription>>>,
    recent_alerts: Arc<Mutex<Vec<PushNotificationPayload>>>,
    local_port: u16,
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
    let mut sec_websocket_key = String::new();
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
        if lower.starts_with("sec-websocket-key:") {
            sec_websocket_key = header_line["sec-websocket-key:".len()..].trim().to_string();
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
        ("GET", "/manifest.webmanifest") => {
            let manifest_bytes = PWA_MANIFEST_JSON.as_bytes();
            let headers = format!(
                "HTTP/1.1 200 OK\r\nServer: pandora-cockpit-web/3.0.10\r\nContent-Type: application/manifest+json; charset=utf-8\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
                manifest_bytes.len()
            );
            stream.write_all(headers.as_bytes())?;
            stream.write_all(manifest_bytes)?;
            stream.flush()?;
        }
        ("GET", "/sw.js") => {
            let sw_bytes = SERVICE_WORKER_JS.as_bytes();
            let headers = format!(
                "HTTP/1.1 200 OK\r\nServer: pandora-cockpit-web/3.0.10\r\nContent-Type: application/javascript; charset=utf-8\r\nCache-Control: no-cache\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
                sw_bytes.len()
            );
            stream.write_all(headers.as_bytes())?;
            stream.write_all(sw_bytes)?;
            stream.flush()?;
        }
        ("GET", "/api/pair/qr") => {
            let now_sec = current_epoch_ms() / 1000;
            let (resp_val, status) = {
                let mut mgr = match pairing_manager.lock() {
                    Ok(guard) => guard,
                    Err(p) => p.into_inner(),
                };
                mgr.prune_expired(now_sec);
                let session = mgr.create_session(now_sec, 300);
                let uri = format!(
                    "pandora://pair?token={}&challenge={}&pubkey={}&port={}",
                    session.token, session.challenge, session.workstation_pubkey, local_port
                );
                let qr_svg = generate_pairing_qr_svg(&uri);
                let resp = PairQrResponse {
                    status: "ready".to_string(),
                    token: session.token.clone(),
                    challenge: session.challenge.clone(),
                    workstation_pubkey: session.workstation_pubkey.clone(),
                    expires_at: session.expires_at_sec,
                    qr_svg,
                    pairing_uri: uri,
                };
                (serde_json::to_value(&resp).unwrap_or_default(), 200)
            };
            write_json_response(&mut stream, status, "OK", &resp_val)?;
        }
        ("POST", "/api/pair/exchange") => {
            let now_sec = current_epoch_ms() / 1000;
            match serde_json::from_slice::<PairExchangeRequest>(&body) {
                Ok(req) => {
                    let mut mgr = match pairing_manager.lock() {
                        Ok(guard) => guard,
                        Err(p) => p.into_inner(),
                    };
                    match mgr.exchange(&req, now_sec) {
                        Ok((resp, _swarm_peer)) => {
                            let resp_val = serde_json::to_value(&resp).unwrap_or_default();
                            CockpitWebServer::broadcast_sse(
                                &sse_clients,
                                "peer_paired",
                                &json!({
                                    "event": "peer_paired",
                                    "peer_id": resp.peer_id,
                                    "mobile_pubkey": req.mobile_pubkey,
                                    "timestamp_ms": current_epoch_ms(),
                                }),
                            );
                            CockpitWebServer::broadcast_ws(
                                &ws_clients,
                                &json!({
                                    "event": "peer_paired",
                                    "peer_id": resp.peer_id,
                                    "timestamp_ms": current_epoch_ms(),
                                }),
                            );
                            write_json_response(&mut stream, 200, "OK", &resp_val)?;
                        }
                        Err(e) => {
                            let (status, reason, code) = match e {
                                PairingError::SessionNotFound => {
                                    (404, "Not Found", "session_not_found")
                                }
                                PairingError::SessionExpired => (410, "Gone", "session_expired"),
                                PairingError::AlreadyConsumed => {
                                    (409, "Conflict", "already_consumed")
                                }
                                PairingError::InvalidHexPublicKey => {
                                    (400, "Bad Request", "invalid_hex_public_key")
                                }
                                PairingError::InvalidPublicKey(_) => {
                                    (400, "Bad Request", "invalid_public_key")
                                }
                                PairingError::InvalidHexSignature => {
                                    (400, "Bad Request", "invalid_hex_signature")
                                }
                                PairingError::SignatureVerificationFailed(_) => {
                                    (401, "Unauthorized", "signature_verification_failed")
                                }
                            };
                            write_json_response(
                                &mut stream,
                                status,
                                reason,
                                &json!({"error": code, "message": e.to_string()}),
                            )?;
                        }
                    }
                }
                Err(e) => {
                    write_json_response(
                        &mut stream,
                        400,
                        "Bad Request",
                        &json!({"error": "invalid_json", "details": e.to_string()}),
                    )?;
                }
            }
        }
        ("POST", "/api/push/subscribe") => {
            match serde_json::from_slice::<PushSubscribeRequest>(&body) {
                Ok(req) => {
                    let now_ms = current_epoch_ms();
                    match req.into_subscription(now_ms) {
                        Ok(sub) => {
                            let endpoint = sub.endpoint.clone();
                            let count = if let Ok(mut map) = push_subscriptions.lock() {
                                map.insert(endpoint.clone(), sub);
                                map.len()
                            } else {
                                1
                            };
                            let resp = json!({
                                "status": "subscribed",
                                "endpoint": endpoint,
                                "subscription_count": count,
                                "timestamp_ms": now_ms,
                            });
                            write_json_response(&mut stream, 200, "OK", &resp)?;
                        }
                        Err(err_msg) => {
                            write_json_response(
                                &mut stream,
                                400,
                                "Bad Request",
                                &json!({ "error": "invalid_subscription", "message": err_msg }),
                            )?;
                        }
                    }
                }
                Err(e) => {
                    write_json_response(
                        &mut stream,
                        400,
                        "Bad Request",
                        &json!({ "error": "invalid_json", "details": e.to_string() }),
                    )?;
                }
            }
        }
        ("POST", "/api/push/unsubscribe") => {
            let json_body: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
            if let Some(endpoint) = json_body.get("endpoint").and_then(Value::as_str) {
                let removed = if let Ok(mut map) = push_subscriptions.lock() {
                    map.remove(endpoint).is_some()
                } else {
                    false
                };
                write_json_response(
                    &mut stream,
                    200,
                    "OK",
                    &json!({ "status": "unsubscribed", "removed": removed, "endpoint": endpoint }),
                )?;
            } else {
                write_json_response(
                    &mut stream,
                    400,
                    "Bad Request",
                    &json!({ "error": "missing_endpoint" }),
                )?;
            }
        }
        ("GET", "/api/push/alerts") => {
            let alerts = match recent_alerts.lock() {
                Ok(guard) => guard.clone(),
                Err(p) => p.into_inner().clone(),
            };
            write_json_response(
                &mut stream,
                200,
                "OK",
                &json!({ "status": "ok", "alerts": alerts, "count": alerts.len() }),
            )?;
        }
        ("POST", "/api/push/test") => {
            let now_ms = current_epoch_ms();
            let test_payload = PushNotificationPayload::new(
                PushAlertType::ConsensusQuorumCompleted,
                "Test Push Alert",
                "Web Push pipeline verification alert from AGENT-PANDORA runtime.",
                "pandora-test-alert",
                PushActionData {
                    target_view: "/#matrix".to_string(),
                    alert_id: format!("test-{now_ms}"),
                    severity: "INFO".to_string(),
                    metadata: BTreeMap::new(),
                },
                now_ms,
            );
            let summary = CockpitWebServer::dispatch_alert_internal(
                &push_subscriptions,
                &recent_alerts,
                &sse_clients,
                &ws_clients,
                test_payload,
            );
            let val = serde_json::to_value(&summary).unwrap_or_default();
            write_json_response(&mut stream, 200, "OK", &val)?;
        }
        ("GET", "/api/ws") => {
            if !sec_websocket_key.is_empty() {
                let accept = compute_websocket_accept(&sec_websocket_key);
                let resp = format!(
                    "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
                );
                stream.write_all(resp.as_bytes())?;
                stream.flush()?;

                if let Ok(writer_clone) = stream.try_clone()
                    && let Ok(mut clients) = ws_clients.lock()
                {
                    clients.push(writer_clone);
                }

                // Send initial sequenced telemetry frame immediately
                let snap_val = CockpitWebServer::current_snapshot_json(&controller);
                let now_ms = current_epoch_ms();
                let seq = if let Ok(mut ring) = telemetry_ring.lock() {
                    ring.push("snapshot", snap_val.clone(), now_ms)
                } else {
                    0
                };
                let init_event = json!({
                    "event": "telemetry",
                    "sequence_id": seq,
                    "timestamp_ms": now_ms,
                    "snapshot": snap_val,
                });
                let init_frame = encode_ws_text_frame(
                    serde_json::to_string(&init_event)
                        .unwrap_or_default()
                        .as_bytes(),
                );
                let _ = stream.write_all(&init_frame);
                let _ = stream.flush();

                // Enter WebSocket frame loop
                stream.set_read_timeout(Some(Duration::from_millis(50)))?;
                while !shutdown_flag.load(Ordering::Relaxed) {
                    match read_client_ws_frame(&mut stream) {
                        Ok(Some((0x8, _))) => {
                            let _ = stream.write_all(&[0x88, 0x00]);
                            let _ = stream.flush();
                            break;
                        }
                        Ok(Some((0x9, _))) => {
                            let _ = stream.write_all(&[0x8A, 0x00]);
                            let _ = stream.flush();
                        }
                        Ok(Some((0x1, payload))) => {
                            if let Ok(val) = serde_json::from_slice::<Value>(&payload) {
                                let action =
                                    val.get("action").and_then(Value::as_str).unwrap_or("");
                                match action {
                                    "sync_request" => {
                                        let since_seq = val
                                            .get("since_sequence_id")
                                            .and_then(Value::as_u64)
                                            .unwrap_or(0);
                                        let since_ts = val
                                            .get("since_timestamp_ms")
                                            .and_then(Value::as_u64)
                                            .unwrap_or(0);
                                        let query_res = if let Ok(ring) = telemetry_ring.lock() {
                                            ring.query_missing(since_seq, since_ts)
                                        } else {
                                            SyncQueryResult::UpToDate { current_seq: 0 }
                                        };
                                        let resp_val = match query_res {
                                            SyncQueryResult::UpToDate { current_seq } => json!({
                                                "event": "sync_response",
                                                "status": "up_to_date",
                                                "current_seq": current_seq,
                                                "since_seq": since_seq,
                                                "timestamp_ms": current_epoch_ms(),
                                            }),
                                            SyncQueryResult::CatchUp {
                                                current_seq,
                                                packets,
                                            } => json!({
                                                "event": "sync_response",
                                                "status": "catch_up",
                                                "current_seq": current_seq,
                                                "since_seq": since_seq,
                                                "missed_packets_count": packets.len(),
                                                "records": packets,
                                                "timestamp_ms": current_epoch_ms(),
                                            }),
                                            SyncQueryResult::GapOverflow {
                                                current_seq,
                                                oldest_available_seq,
                                            } => {
                                                let latest_snap =
                                                    CockpitWebServer::current_snapshot_json(
                                                        &controller,
                                                    );
                                                json!({
                                                    "event": "sync_response",
                                                    "status": "gap_overflow",
                                                    "current_seq": current_seq,
                                                    "oldest_available_seq": oldest_available_seq,
                                                    "since_seq": since_seq,
                                                    "latest_snapshot": latest_snap,
                                                    "timestamp_ms": current_epoch_ms(),
                                                })
                                            }
                                        };
                                        let frame = encode_ws_text_frame(
                                            serde_json::to_string(&resp_val)
                                                .unwrap_or_default()
                                                .as_bytes(),
                                        );
                                        let _ = stream.write_all(&frame);
                                        let _ = stream.flush();
                                    }
                                    "layout" => {
                                        if let Some(layout_str) =
                                            val.get("layout").and_then(Value::as_str)
                                            && let Some(new_layout) =
                                                CockpitLayout::parse(layout_str)
                                        {
                                            if let Ok(mut guard) = controller.lock() {
                                                *guard = CockpitController::new(new_layout);
                                            }
                                            let snap = CockpitWebServer::current_snapshot_json(
                                                &controller,
                                            );
                                            let now_ms = current_epoch_ms();
                                            let seq = if let Ok(mut ring) = telemetry_ring.lock() {
                                                ring.push("snapshot", snap.clone(), now_ms)
                                            } else {
                                                0
                                            };
                                            CockpitWebServer::broadcast_sse(
                                                &sse_clients,
                                                "telemetry",
                                                &snap,
                                            );
                                            CockpitWebServer::broadcast_ws(
                                                &ws_clients,
                                                &json!({
                                                    "event": "telemetry",
                                                    "sequence_id": seq,
                                                    "timestamp_ms": now_ms,
                                                    "snapshot": &snap,
                                                }),
                                            );
                                            let reply = json!({
                                                "event": "layout_updated",
                                                "layout": layout_str,
                                                "timestamp_ms": current_epoch_ms(),
                                            });
                                            let reply_frame = encode_ws_text_frame(
                                                serde_json::to_string(&reply)
                                                    .unwrap_or_default()
                                                    .as_bytes(),
                                            );
                                            let _ = stream.write_all(&reply_frame);
                                            let _ = stream.flush();
                                        }
                                    }
                                    "consensus" => {
                                        let threshold = val
                                            .get("threshold")
                                            .and_then(Value::as_f64)
                                            .unwrap_or(0.50);
                                        let divergence = val
                                            .get("divergence")
                                            .and_then(Value::as_f64)
                                            .unwrap_or(0.45);
                                        let engine =
                                            SwarmCodeConsensusEngine::new(threshold, divergence);
                                        let proposals = default_consensus_proposals();
                                        let now_ms = current_epoch_ms();
                                        if let Ok(decision) =
                                            engine.resolve_consensus(&proposals, now_ms)
                                        {
                                            let resp_val = json!({
                                                "event": "consensus_decision",
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
                                            CockpitWebServer::broadcast_sse(
                                                &sse_clients,
                                                "consensus",
                                                &resp_val,
                                            );
                                            CockpitWebServer::broadcast_ws(&ws_clients, &resp_val);
                                            let frame = encode_ws_text_frame(
                                                serde_json::to_string(&resp_val)
                                                    .unwrap_or_default()
                                                    .as_bytes(),
                                            );
                                            let _ = stream.write_all(&frame);
                                            let _ = stream.flush();

                                            // Web push alert triggers
                                            let cert_digest = format!(
                                                "PANDORA_PATCH_CERT_V1:{}",
                                                decision.consensus_hash
                                            );
                                            let alert =
                                                PushNotificationPayload::consensus_quorum_completed(
                                                    &decision.winning_proposal_id,
                                                    decision.winning_agent.identifier(),
                                                    decision.honest_quorum_size,
                                                    decision.total_proposals,
                                                    &cert_digest,
                                                    now_ms,
                                                );
                                            CockpitWebServer::dispatch_alert_internal(
                                                &push_subscriptions,
                                                &recent_alerts,
                                                &sse_clients,
                                                &ws_clients,
                                                alert,
                                            );
                                        }
                                    }
                                    "ping" => {
                                        let resp = json!({"event": "pong", "timestamp_ms": current_epoch_ms()});
                                        let frame = encode_ws_text_frame(
                                            serde_json::to_string(&resp)
                                                .unwrap_or_default()
                                                .as_bytes(),
                                        );
                                        let _ = stream.write_all(&frame);
                                        let _ = stream.flush();
                                    }
                                    _ => {}
                                }
                            }
                        }
                        Ok(Some(_)) => {}
                        Ok(None) => {
                            thread::sleep(Duration::from_millis(15));
                        }
                        Err(_) => break,
                    }
                }
                return Ok(());
            } else {
                write_json_response(
                    &mut stream,
                    400,
                    "Bad Request",
                    &json!({"error": "missing_sec_websocket_key"}),
                )?;
                return Ok(());
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
                        let now_ms = current_epoch_ms();
                        let seq = if let Ok(mut ring) = telemetry_ring.lock() {
                            ring.push("snapshot", snap_val.clone(), now_ms)
                        } else {
                            0
                        };
                        CockpitWebServer::broadcast_sse(&sse_clients, "telemetry", &snap_val);
                        CockpitWebServer::broadcast_ws(
                            &ws_clients,
                            &json!({
                                "event": "telemetry",
                                "sequence_id": seq,
                                "timestamp_ms": now_ms,
                                "snapshot": &snap_val,
                            }),
                        );
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
                    CockpitWebServer::broadcast_ws(&ws_clients, &resp_val);

                    // Web push alert triggers
                    let cert_digest = format!("PANDORA_PATCH_CERT_V1:{}", decision.consensus_hash);
                    let alert = PushNotificationPayload::consensus_quorum_completed(
                        &decision.winning_proposal_id,
                        decision.winning_agent.identifier(),
                        decision.honest_quorum_size,
                        decision.total_proposals,
                        &cert_digest,
                        now_ms,
                    );
                    CockpitWebServer::dispatch_alert_internal(
                        &push_subscriptions,
                        &recent_alerts,
                        &sse_clients,
                        &ws_clients,
                        alert,
                    );
                    if decision.byzantine_filtered_count > 0 {
                        let equiv_alert = PushNotificationPayload::byzantine_equivocation(
                            "p_byzantine",
                            &decision.winning_proposal_id,
                            Some("Conflicting scores detected in BFT consensus round"),
                            now_ms,
                        );
                        CockpitWebServer::dispatch_alert_internal(
                            &push_subscriptions,
                            &recent_alerts,
                            &sse_clients,
                            &ws_clients,
                            equiv_alert,
                        );
                    }

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
                CockpitWebServer::broadcast_ws(
                    &ws_clients,
                    &json!({
                        "event": "permission",
                        "data": &resp_val,
                    }),
                );
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
