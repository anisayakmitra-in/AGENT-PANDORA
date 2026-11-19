#![forbid(unsafe_code)]

//! End-to-End CLI Tests for AGENT-PANDORA v3.1.3 (Milestones 3 & 4).
//!
//! Verifies:
//! - R3: `pandora run --strategy lats` with `--lats-rollouts`, `--lats-depth`, `--puct-c`,
//!   and `--escalation-threshold` in both JSON envelope and human-readable modes, plus
//!   validation of out-of-range parameters.
//! - R4: `pandora memory vfs tropical-search` and `pandora memory vfs search --tropical`.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

struct TestTempDir(PathBuf);

impl TestTempDir {
    fn new(prefix: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("pandora_v313_{prefix}_{pid}_{nanos}"));
        fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestTempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct MockOpenAiServer {
    addr: String,
    stop: Arc<AtomicBool>,
}

impl MockOpenAiServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let port = listener.local_addr().expect("local addr").port();
        listener.set_nonblocking(true).expect("set nonblocking");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);

        thread::spawn(move || {
            let body = serde_json::json!({
                "id": "chatcmpl-v313-mock",
                "object": "chat.completion",
                "created": 1700000000u64,
                "model": "mock-v1",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "Completed LATS strategy verification."
                    },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 12,
                    "completion_tokens": 8,
                    "total_tokens": 20
                }
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );

            while !stop_flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let mut req_buf = Vec::new();
                        let mut chunk = [0u8; 4096];
                        let mut content_len: usize = 0;
                        let mut header_end: Option<usize> = None;
                        while let Ok(n) = stream.read(&mut chunk) {
                            if n == 0 {
                                break;
                            }
                            req_buf.extend_from_slice(&chunk[..n]);
                            if header_end.is_none()
                                && let Some(pos) = req_buf.windows(4).position(|w| w == b"\r\n\r\n")
                            {
                                let end = pos + 4;
                                header_end = Some(end);
                                let headers = String::from_utf8_lossy(&req_buf[..end]);
                                for line in headers.lines() {
                                    if let Some((k, v)) = line.split_once(':')
                                        && k.trim().eq_ignore_ascii_case("content-length")
                                    {
                                        content_len = v.trim().parse().unwrap_or(0);
                                    }
                                }
                            }
                            if let Some(end) = header_end
                                && req_buf.len() >= end + content_len
                            {
                                break;
                            }
                        }
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.flush();
                        let _ = stream.shutdown(std::net::Shutdown::Write);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            addr: format!("http://127.0.0.1:{port}"),
            stop,
        }
    }
}

impl Drop for MockOpenAiServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn write_test_config(dir: &Path, provider_url: Option<&str>) -> PathBuf {
    let config_path = dir.join("pandora.json");
    let data_dir = dir.join("data");
    fs::create_dir_all(&data_dir).expect("create data dir");
    let mut config_json = serde_json::json!({
        "format_version": 1,
        "data_dir": data_dir.to_string_lossy(),
        "workspace_dir": dir.to_string_lossy(),
        "provider_activation_disabled": true
    });
    if let Some(url) = provider_url {
        config_json["provider_url"] = serde_json::json!(url);
        config_json["provider_model"] = serde_json::json!("mock-v1");
    }
    fs::write(
        &config_path,
        serde_json::to_string_pretty(&config_json).expect("serialize config"),
    )
    .expect("write config");
    config_path
}

#[test]
fn cli_run_strategy_lats_reports_metrics_in_json_and_human_output() {
    let server = MockOpenAiServer::start();
    let temp = TestTempDir::new("lats_run");
    let config_path = write_test_config(temp.path(), Some(&server.addr));

    // 1. JSON envelope mode with custom LATS parameters
    let output = Command::new(env!("CARGO_BIN_EXE_pandora"))
        .env("PANDORA_PROVIDER_API_KEY", "test-key")
        .args([
            "run",
            "--config",
            config_path.to_str().unwrap(),
            "--strategy",
            "lats",
            "--lats-rollouts",
            "12",
            "--lats-depth",
            "4",
            "--puct-c",
            "1.45",
            "--escalation-threshold",
            "0.20",
            "--json",
            "Analyze tropical semiring MCTS convergence",
        ])
        .output()
        .expect("execute pandora run --strategy lats");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let body: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON envelope");
    assert_eq!(body["command"], "run");
    assert_eq!(body["strategy"], "lats");
    let metrics = &body["lats_metrics"];
    assert!(
        metrics.is_object(),
        "expected lats_metrics object in envelope: {body}"
    );
    assert_eq!(metrics["rollouts"], 12);
    assert!(metrics["max_depth_reached"].as_u64().unwrap_or(0) >= 1);
    assert!(metrics["nodes_created"].as_u64().unwrap_or(0) >= 1);

    // 2. Human-readable mode
    let human_out = Command::new(env!("CARGO_BIN_EXE_pandora"))
        .env("PANDORA_PROVIDER_API_KEY", "test-key")
        .args([
            "run",
            "--config",
            config_path.to_str().unwrap(),
            "--strategy",
            "lats",
            "--lats-rollouts",
            "8",
            "--lats-depth",
            "3",
            "Verify reflex safety gate",
        ])
        .output()
        .expect("execute pandora run human");
    assert!(
        human_out.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&human_out.stdout),
        String::from_utf8_lossy(&human_out.stderr)
    );
    let stdout_str = String::from_utf8_lossy(&human_out.stdout);
    assert!(
        stdout_str.contains("strategy: lats"),
        "stdout: {stdout_str}"
    );
    assert!(stdout_str.contains("rollouts: 8"), "stdout: {stdout_str}");
}

#[test]
fn cli_run_strategy_lats_rejects_invalid_parameter_bounds() {
    let temp = TestTempDir::new("lats_bounds");
    let config_path = write_test_config(temp.path(), None);

    // Out-of-range rollouts (> 100)
    let bad_rollouts = Command::new(env!("CARGO_BIN_EXE_pandora"))
        .args([
            "run",
            "--config",
            config_path.to_str().unwrap(),
            "--strategy",
            "lats",
            "--lats-rollouts",
            "101",
            "task",
        ])
        .output()
        .expect("execute bad rollouts");
    assert!(!bad_rollouts.status.success());

    // Out-of-range depth (> 16)
    let bad_depth = Command::new(env!("CARGO_BIN_EXE_pandora"))
        .args([
            "run",
            "--config",
            config_path.to_str().unwrap(),
            "--strategy",
            "lats",
            "--lats-depth",
            "17",
            "task",
        ])
        .output()
        .expect("execute bad depth");
    assert!(!bad_depth.status.success());

    // Non-positive puct-c
    let bad_puct = Command::new(env!("CARGO_BIN_EXE_pandora"))
        .args([
            "run",
            "--config",
            config_path.to_str().unwrap(),
            "--strategy",
            "lats",
            "--puct-c",
            "0.0",
            "task",
        ])
        .output()
        .expect("execute bad puct");
    assert!(!bad_puct.status.success());
}

#[test]
fn cli_memory_vfs_tropical_search_returns_ranked_virtual_resources() {
    let temp = TestTempDir::new("vfs_tropical");
    let config_path = write_test_config(temp.path(), None);

    let output = Command::new(env!("CARGO_BIN_EXE_pandora"))
        .args([
            "memory",
            "vfs",
            "tropical-search",
            "--config",
            config_path.to_str().unwrap(),
            "--limit",
            "5",
            "--json",
            "Constitutional Rules",
        ])
        .output()
        .expect("execute memory vfs tropical-search");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let body: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON envelope");
    assert_eq!(body["command"], "memory vfs tropical-search");
    assert!(body["count"].as_u64().unwrap_or(0) >= 1, "body: {body}");
}

#[test]
fn cli_model_calibrate_bare_relative_pnn_writes_in_current_dir() {
    let temp = TestTempDir::new("moira_bare_pnn");

    let output = Command::new(env!("CARGO_BIN_EXE_pandora"))
        .current_dir(temp.path())
        .args([
            "model",
            "calibrate",
            "--pnn",
            "custom.pnn",
            "--workspace",
            temp.path().to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("execute pandora model calibrate --pnn custom.pnn");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let body: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON envelope");
    assert_eq!(body["command"], "model calibrate");
    assert_eq!(body["canary_passed"], true);
    assert!(temp.path().join("custom.pnn").is_file());
}
