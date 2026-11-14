//! Phase 26: Google Colab Remote GPU Bridge Client.
//!
//! Provides authenticated communication between AGENT-PANDORA and Google Colab
//! GPU instances, supporting remote model evaluation, live GPU status monitoring,
//! and session lifecycle persistence under `#![forbid(unsafe_code)]`.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Errors encountered while communicating with a Google Colab GPU bridge.
#[derive(Debug)]
pub enum ColabError {
    /// Failed to reach the remote Colab endpoint.
    ConnectionFailed(String),
    /// Invalid authentication token or handshake rejected.
    AuthenticationFailed(String),
    /// The response payload could not be decoded.
    InvalidResponse(String),
    /// Filesystem error loading or saving session state.
    Io(String),
    /// JSON serialization or deserialization failure.
    Serialization(String),
}

impl fmt::Display for ColabError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConnectionFailed(msg) => write!(f, "Colab connection failed: {msg}"),
            Self::AuthenticationFailed(msg) => write!(f, "Colab authentication failed: {msg}"),
            Self::InvalidResponse(msg) => write!(f, "Colab invalid response: {msg}"),
            Self::Io(msg) => write!(f, "Colab session I/O error: {msg}"),
            Self::Serialization(msg) => write!(f, "Colab serialization error: {msg}"),
        }
    }
}

impl std::error::Error for ColabError {}

/// Represents a persistent Google Colab authenticated session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ColabSession {
    /// Public tunnel or gateway URL (e.g. `https://xxxx.trycloudflare.com`).
    pub url: String,
    /// Secret authentication token exchanged during handshake.
    pub token: String,
    /// Unix timestamp (seconds) when the session was established.
    pub connected_at: u64,
    /// GPU device reported by PyTorch (e.g. "NVIDIA Tesla T4" or "A100-SXM4-40GB").
    pub gpu_device: String,
    /// Total available GPU memory in megabytes.
    pub vram_total_mb: u64,
    /// CUDA / Driver version on Colab runtime.
    pub cuda_version: String,
    /// Compatible Pandora bridge protocol version.
    pub bridge_version: String,
}

/// Real-time health, latency, and GPU telemetry from the Colab worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColabStatus {
    /// Whether the endpoint responded successfully.
    pub online: bool,
    /// Endpoint URL.
    pub url: String,
    /// GPU hardware name.
    pub gpu_device: String,
    /// Currently free GPU VRAM in megabytes.
    pub vram_free_mb: u64,
    /// Total GPU VRAM in megabytes.
    pub vram_total_mb: u64,
    /// Round-trip ping latency in milliseconds.
    pub latency_ms: u64,
    /// Active neural network architectures hosted on Colab worker.
    pub active_models: Vec<String>,
}

/// Client for interacting with the Google Colab GPU bridge.
#[derive(Debug, Clone)]
pub struct ColabClient {
    base_url: String,
    token: String,
}

impl ColabClient {
    /// Creates a new ColabClient with sanitized base URL.
    pub fn new(url: &str, token: &str) -> Self {
        let clean_url = url.trim().trim_end_matches('/').to_string();
        Self {
            base_url: clean_url,
            token: token.trim().to_string(),
        }
    }

    /// Base URL getter.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Auth token getter.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Verifies connectivity and exchanges credentials with the Colab bridge.
    pub fn verify_handshake(&self) -> Result<ColabSession, ColabError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(12))
            .build()
            .map_err(|e| ColabError::ConnectionFailed(e.to_string()))?;

        let endpoint = format!("{}/api/colab/handshake", self.base_url);
        let resp = client
            .post(&endpoint)
            .header("X-Pandora-Colab-Token", &self.token)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "client_version": env!("CARGO_PKG_VERSION"),
                "timestamp": current_timestamp(),
            }))
            .send()
            .map_err(|e| ColabError::ConnectionFailed(format!("HTTP request failed: {e}")))?;

        let status_code = resp.status();
        if status_code == reqwest::StatusCode::UNAUTHORIZED
            || status_code == reqwest::StatusCode::FORBIDDEN
        {
            return Err(ColabError::AuthenticationFailed(
                "Invalid authentication token or Colab bridge rejected authorization".to_string(),
            ));
        }

        if !status_code.is_success() {
            return Err(ColabError::ConnectionFailed(format!(
                "Colab bridge returned HTTP {status_code}"
            )));
        }

        let body: serde_json::Value = resp.json().map_err(|e| {
            ColabError::InvalidResponse(format!("Failed to parse handshake JSON: {e}"))
        })?;

        let gpu_device = body["gpu_device"]
            .as_str()
            .unwrap_or("CPU Fallback")
            .to_string();
        let vram_total_mb = body["vram_total_mb"].as_u64().unwrap_or(0);
        let cuda_version = body["cuda_version"].as_str().unwrap_or("N/A").to_string();
        let bridge_version = body["bridge_version"]
            .as_str()
            .unwrap_or("v3.1.0")
            .to_string();

        Ok(ColabSession {
            url: self.base_url.clone(),
            token: self.token.clone(),
            connected_at: current_timestamp(),
            gpu_device,
            vram_total_mb,
            cuda_version,
            bridge_version,
        })
    }

    /// Polls live telemetry, VRAM consumption, and latency from Colab.
    pub fn check_status(&self) -> Result<ColabStatus, ColabError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .build()
            .map_err(|e| ColabError::ConnectionFailed(e.to_string()))?;

        let endpoint = format!("{}/api/colab/status", self.base_url);
        let start = Instant::now();

        let resp = client
            .get(&endpoint)
            .header("X-Pandora-Colab-Token", &self.token)
            .send()
            .map_err(|e| ColabError::ConnectionFailed(format!("Status check failed: {e}")))?;

        let latency_ms = start.elapsed().as_millis() as u64;

        if !resp.status().is_success() {
            return Err(ColabError::ConnectionFailed(format!(
                "HTTP status probe failed with code {}",
                resp.status()
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .map_err(|e| ColabError::InvalidResponse(format!("Invalid status JSON: {e}")))?;

        let gpu_device = body["gpu_device"]
            .as_str()
            .unwrap_or("Unknown GPU")
            .to_string();
        let vram_total_mb = body["vram_total_mb"].as_u64().unwrap_or(0);
        let vram_free_mb = body["vram_free_mb"].as_u64().unwrap_or(0);
        let active_models = body["active_models"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default();

        Ok(ColabStatus {
            online: true,
            url: self.base_url.clone(),
            gpu_device,
            vram_free_mb,
            vram_total_mb,
            latency_ms,
            active_models,
        })
    }

    /// Saves session metadata atomically to disk.
    pub fn save_session(session: &ColabSession, path: &Path) -> Result<(), ColabError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| ColabError::Io(e.to_string()))?;
        }
        let serialized = serde_json::to_string_pretty(session)
            .map_err(|e| ColabError::Serialization(e.to_string()))?;
        fs::write(path, serialized).map_err(|e| ColabError::Io(e.to_string()))
    }

    /// Reads persistent session metadata from disk if available.
    pub fn load_session(path: &Path) -> Result<Option<ColabSession>, ColabError> {
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(path).map_err(|e| ColabError::Io(e.to_string()))?;
        let session: ColabSession =
            serde_json::from_str(&content).map_err(|e| ColabError::Serialization(e.to_string()))?;
        Ok(Some(session))
    }

    /// Removes an existing session file.
    pub fn clear_session(path: &Path) -> Result<(), ColabError> {
        if path.exists() {
            fs::remove_file(path).map_err(|e| ColabError::Io(e.to_string()))?;
        }
        Ok(())
    }
}

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
