//! Phase 26: Google Colab Remote GPU Bridge CLI Integration Tests.

#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::thread;

fn cargo_bin_pandora() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_pandora"));
    cmd.current_dir(env!("CARGO_MANIFEST_DIR"));
    cmd
}

#[test]
fn test_cli_colab_help_and_unknown_subcommand() {
    let output = cargo_bin_pandora()
        .args(["colab", "help"])
        .output()
        .expect("runs pandora colab help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("pandora colab <connect|status|disconnect|ping>"));

    let err_out = cargo_bin_pandora()
        .args(["colab", "invalid_sub"])
        .output()
        .expect("runs invalid subcommand");

    assert!(!err_out.status.success());
    let stderr = String::from_utf8_lossy(&err_out.stderr);
    assert!(stderr.contains("unknown colab subcommand 'invalid_sub'"));
}

#[test]
fn test_cli_colab_status_without_session() {
    let unique = format!(
        "nonexistent_colab_session_{}.json",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let session_path = std::env::temp_dir().join(unique);

    let output = cargo_bin_pandora()
        .args([
            "colab",
            "status",
            "--session-path",
            session_path.to_str().unwrap(),
        ])
        .output()
        .expect("runs pandora colab status");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("No active Colab session found"));
}

#[test]
fn test_cli_colab_full_lifecycle_with_mock_server() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("binds ephemeral port");
    let addr = listener.local_addr().expect("gets address");
    let token = "test-secret-token-colab-42";

    // Background mock HTTP server for handshake and status
    let server_handle = thread::spawn(move || {
        for _ in 0..3 {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buffer = [0u8; 2048];
                let bytes_read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..bytes_read]);

                if request.contains("POST /api/colab/handshake") {
                    let body = r#"{"status":"connected","gpu_device":"NVIDIA Tesla T4","vram_total_mb":15109,"cuda_version":"12.2","bridge_version":"v3.1.0"}"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes());
                } else if request.contains("GET /api/colab/status") {
                    let body = r#"{"online":true,"gpu_device":"NVIDIA Tesla T4","vram_free_mb":14200,"vram_total_mb":15109,"active_models":["tropical_polyhedral"]}"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes());
                }
            }
        }
    });

    let unique = format!(
        "lifecycle_session_{}.json",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let session_path = std::env::temp_dir().join(unique);
    let mock_url = format!("http://{}", addr);

    // 1. Connect
    let connect_out = cargo_bin_pandora()
        .args([
            "colab",
            "connect",
            "--url",
            &mock_url,
            "--token",
            token,
            "--session-path",
            session_path.to_str().unwrap(),
        ])
        .output()
        .expect("executes colab connect");

    assert!(
        connect_out.status.success(),
        "connect failed: {:?}",
        connect_out
    );
    let connect_stdout = String::from_utf8_lossy(&connect_out.stdout);
    assert!(connect_stdout.contains("Successfully connected to Google Colab GPU bridge!"));
    assert!(connect_stdout.contains("NVIDIA Tesla T4"));
    assert!(session_path.exists());

    // 2. Status
    let status_out = cargo_bin_pandora()
        .args([
            "colab",
            "status",
            "--session-path",
            session_path.to_str().unwrap(),
        ])
        .output()
        .expect("executes colab status");

    assert!(
        status_out.status.success(),
        "status failed: {:?}",
        status_out
    );
    let status_stdout = String::from_utf8_lossy(&status_out.stdout);
    assert!(status_stdout.contains("Google Colab GPU Status:"));
    assert!(status_stdout.contains("Online: YES"));
    assert!(status_stdout.contains("14200 / 15109 MB Free"));

    // 3. Ping
    let ping_out = cargo_bin_pandora()
        .args([
            "colab",
            "ping",
            "--session-path",
            session_path.to_str().unwrap(),
        ])
        .output()
        .expect("executes colab ping");

    assert!(ping_out.status.success(), "ping failed: {:?}", ping_out);
    let ping_stdout = String::from_utf8_lossy(&ping_out.stdout);
    assert!(ping_stdout.contains("Pong!"));

    // 4. Disconnect
    let disc_out = cargo_bin_pandora()
        .args([
            "colab",
            "disconnect",
            "--session-path",
            session_path.to_str().unwrap(),
        ])
        .output()
        .expect("executes colab disconnect");

    assert!(disc_out.status.success());
    assert!(!session_path.exists());

    let _ = server_handle.join();
}
