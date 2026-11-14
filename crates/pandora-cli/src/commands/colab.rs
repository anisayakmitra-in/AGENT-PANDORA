//! Phase 26: Google Colab Remote GPU Bridge CLI.
//!
//! Subcommands:
//! - `connect`: Establishes and persists an authenticated handshake with a Google Colab GPU instance.
//! - `status`: Inspects live connection health, GPU device, VRAM, and round-trip latency.
//! - `disconnect`: Clears the local Colab session cache.
//! - `ping`: Measures network latency to the connected Colab bridge.

use super::{parse_options, path_option};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::colab_bridge::{ColabClient, ColabError};
use serde_json::json;
use std::path::PathBuf;

const DEFAULT_SESSION_REL_PATH: &str = ".pandora/colab_session.json";

fn map_colab_error(err: ColabError) -> CliError {
    match err {
        ColabError::AuthenticationFailed(msg) => {
            CliError::execution(msg, json!({"code": "colab_auth_failed"}))
        }
        ColabError::ConnectionFailed(msg) => {
            CliError::execution(msg, json!({"code": "colab_connection_failed"}))
        }
        other => CliError::usage(other.to_string()),
    }
}

fn resolve_session_path(custom: Option<PathBuf>) -> PathBuf {
    custom.unwrap_or_else(|| PathBuf::from(DEFAULT_SESSION_REL_PATH))
}

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage("colab requires 'connect', 'status', 'disconnect', or 'ping'")
    })?;

    match subcommand.as_str() {
        "connect" => execute_connect(&args[1..]),
        "status" => execute_status(&args[1..]),
        "disconnect" => execute_disconnect(&args[1..]),
        "ping" => execute_ping(&args[1..]),
        "--help" | "help" => Ok(success("colab", json!({"usage": usage()}), usage())),
        unknown => Err(CliError::usage(format!(
            "unknown colab subcommand '{unknown}'. Available: 'connect', 'status', 'disconnect', 'ping'"
        ))),
    }
}

fn execute_connect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "url",
            "endpoint",
            "token",
            "auth",
            "session-path",
            "config",
            "data-dir",
            "workspace",
            "json",
        ],
    )?;

    let url = parsed
        .value("url")
        .or_else(|| parsed.value("endpoint"))
        .ok_or_else(|| CliError::usage("colab connect requires --url <tunnel_url>"))?;

    let token = parsed
        .value("token")
        .or_else(|| parsed.value("auth"))
        .ok_or_else(|| CliError::usage("colab connect requires --token <auth_token>"))?;

    let session_path = resolve_session_path(path_option(&parsed, "session-path"));

    let client = ColabClient::new(url, token);
    let session = client.verify_handshake().map_err(map_colab_error)?;

    ColabClient::save_session(&session, &session_path).map_err(map_colab_error)?;

    let summary = format!(
        "Successfully connected to Google Colab GPU bridge!\n  URL: {}\n  GPU: {}\n  VRAM: {} MB\n  CUDA: {}\n  Session: {}",
        session.url,
        session.gpu_device,
        session.vram_total_mb,
        session.cuda_version,
        session_path.display()
    );

    Ok(success(
        "colab_connect",
        json!({
            "status": "connected",
            "session": session,
            "session_file": session_path.display().to_string(),
        }),
        summary,
    ))
}

fn execute_status(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["session-path", "config", "data-dir", "workspace", "json"],
    )?;

    let session_path = resolve_session_path(path_option(&parsed, "session-path"));
    let session = ColabClient::load_session(&session_path)
        .map_err(map_colab_error)?
        .ok_or_else(|| {
            CliError::usage(format!(
                "No active Colab session found at {}. Connect first via: pandora colab connect --url <URL> --token <TOKEN>",
                session_path.display()
            ))
        })?;

    let client = ColabClient::new(&session.url, &session.token);
    let status = client.check_status().map_err(map_colab_error)?;

    let summary = format!(
        "Google Colab GPU Status:\n  Online: {}\n  URL: {}\n  GPU: {}\n  VRAM: {} / {} MB Free\n  Latency: {} ms\n  Active Models: {}",
        if status.online { "YES" } else { "NO" },
        status.url,
        status.gpu_device,
        status.vram_free_mb,
        status.vram_total_mb,
        status.latency_ms,
        if status.active_models.is_empty() {
            "None (Standby)".to_string()
        } else {
            status.active_models.join(", ")
        }
    );

    Ok(success(
        "colab_status",
        json!({
            "online": status.online,
            "gpu_device": status.gpu_device,
            "vram_free_mb": status.vram_free_mb,
            "vram_total_mb": status.vram_total_mb,
            "latency_ms": status.latency_ms,
            "active_models": status.active_models,
            "url": status.url,
        }),
        summary,
    ))
}

fn execute_disconnect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["session-path", "config", "data-dir", "workspace", "json"],
    )?;

    let session_path = resolve_session_path(path_option(&parsed, "session-path"));
    ColabClient::clear_session(&session_path).map_err(map_colab_error)?;

    let summary = format!(
        "Disconnected from Google Colab. Session cache at {} removed.",
        session_path.display()
    );
    Ok(success(
        "colab_disconnect",
        json!({
            "status": "disconnected",
            "cleared_file": session_path.display().to_string(),
        }),
        summary,
    ))
}

fn execute_ping(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["session-path", "config", "data-dir", "workspace", "json"],
    )?;

    let session_path = resolve_session_path(path_option(&parsed, "session-path"));
    let session = ColabClient::load_session(&session_path)
        .map_err(map_colab_error)?
        .ok_or_else(|| {
            CliError::usage(format!(
                "No active Colab session found at {}. Connect first via: pandora colab connect --url <URL> --token <TOKEN>",
                session_path.display()
            ))
        })?;

    let client = ColabClient::new(&session.url, &session.token);
    let status = client.check_status().map_err(map_colab_error)?;

    let summary = format!(
        "Pong! Colab at {} responded in {} ms (GPU: {})",
        session.url, status.latency_ms, status.gpu_device
    );
    Ok(success(
        "colab_ping",
        json!({
            "latency_ms": status.latency_ms,
            "gpu_device": status.gpu_device,
            "online": status.online,
        }),
        summary,
    ))
}

pub fn usage() -> &'static str {
    "pandora colab <connect|status|disconnect|ping> [options]

Subcommands:
  connect      Establish authenticated handshake with a Google Colab GPU instance
                 --url <tunnel_url>      Public tunnel URL (e.g. https://xxx.trycloudflare.com)
                 --token <auth_token>    Secret token generated by the Colab bridge
                 [--session-path <path>] Custom path to persist session JSON
  status       Inspect live telemetry, free VRAM, and GPU health
  disconnect   Clear stored session credentials
  ping         Measure round-trip network latency to the connected GPU instance"
}
