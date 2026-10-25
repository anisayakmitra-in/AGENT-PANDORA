#![forbid(unsafe_code)]

use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(500);

struct Fixture {
    _root: PathBuf,
    config: PathBuf,
    data: PathBuf,
    workspace: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock available")
            .as_nanos();
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let temp_root = std::env::temp_dir()
            .canonicalize()
            .expect("temp dir canonical path");
        let root = temp_root.join(format!(
            "pandora-mcp-auth-test-{}-{suffix}-{sequence}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).expect("workspace dir created");
        Self {
            config: root.join("config.json"),
            data: root.join("data"),
            workspace,
            _root: root,
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_pandora"));
        cmd.args(args)
            .env("PANDORA_CONFIG", &self.config)
            .env("PANDORA_DATA_DIR", &self.data)
            .env("PANDORA_WORKSPACE", &self.workspace)
            .env(
                "PANDORA_MASTER_KEY",
                "pandora-master-key-fixture-32-chars-long",
            )
            .env_remove("PANDORA_PROVIDER_URL");
        cmd
    }

    fn setup(&self) {
        let output = self
            .command(&[
                "setup",
                "--provider-url",
                "http://127.0.0.1:4317/v1",
                "--json",
            ])
            .output()
            .expect("setup command should succeed");
        assert!(output.status.success(), "setup must succeed");
    }
}

#[test]
fn mcp_add_http_config_and_inspect() {
    let fixture = Fixture::new();
    fixture.setup();

    let output = fixture
        .command(&[
            "mcp",
            "add-http",
            "corp-finance",
            "https://finance.corp.internal/mcp",
            "--auth",
            "bearer",
            "--header",
            "X-Tenant-ID:finance-prod",
            "--json",
        ])
        .output()
        .expect("add-http should execute");
    assert!(
        output.status.success(),
        "add-http must succeed: {}",
        String::from_utf8_lossy(&output.stdout)
    );

    let inspect_out = fixture
        .command(&["mcp", "inspect", "corp-finance", "--json"])
        .output()
        .expect("inspect should execute");
    assert!(
        inspect_out.status.success(),
        "inspect must succeed: {}",
        String::from_utf8_lossy(&inspect_out.stdout)
    );

    let v: Value = serde_json::from_slice(&inspect_out.stdout).expect("valid json output");
    assert_eq!(v["server"]["transport"], "http");
    assert_eq!(v["server"]["url"], "https://finance.corp.internal/mcp");
    assert_eq!(v["server"]["auth_type"], "bearer");
    assert_eq!(v["server"]["headers"]["X-Tenant-ID"], "finance-prod");
}

#[test]
fn mcp_login_and_logout_vault() {
    let fixture = Fixture::new();
    fixture.setup();

    // First add an HTTP server configuration
    let add_out = fixture
        .command(&[
            "mcp",
            "add-http",
            "corp-analytics",
            "https://analytics.corp.internal/sse",
            "--auth",
            "bearer",
            "--json",
        ])
        .output()
        .expect("add-http should execute");
    assert!(
        add_out.status.success(),
        "add-http failed: {}",
        String::from_utf8_lossy(&add_out.stdout)
    );

    // Login with manual token
    let login_out = fixture
        .command(&[
            "mcp",
            "login",
            "corp-analytics",
            "--token",
            "corp-secret-token-abcdef12345",
            "--json",
        ])
        .output()
        .expect("login should execute");
    assert!(
        login_out.status.success(),
        "login must succeed: {}",
        String::from_utf8_lossy(&login_out.stdout)
    );
    let login_val: Value = serde_json::from_slice(&login_out.stdout).expect("valid json output");
    assert_eq!(login_val["server"], "corp-analytics");
    assert_eq!(login_val["status"], "authenticated");

    // Logout
    let logout_out = fixture
        .command(&["mcp", "logout", "corp-analytics", "--json"])
        .output()
        .expect("logout should execute");
    assert!(
        logout_out.status.success(),
        "logout must succeed: {}",
        String::from_utf8_lossy(&logout_out.stdout)
    );
    let logout_val: Value = serde_json::from_slice(&logout_out.stdout).expect("valid json output");
    assert_eq!(logout_val["server"], "corp-analytics");
    assert_eq!(logout_val["removed"], true);
}

#[test]
fn mcp_http_catalog_and_call_with_bearer_and_headers() {
    let fixture = Fixture::new();
    fixture.setup();

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local ephemeral port");
    let local_addr = listener.local_addr().expect("local address");
    listener.set_nonblocking(true).expect("set non-blocking");

    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();

    let server_handle = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let mut handled = 0;
        while std::time::Instant::now() < deadline && running_clone.load(Ordering::Relaxed) {
            if let Ok((mut stream, _)) = listener.accept() {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
                let mut buf = [0u8; 8192];
                let mut total_read = 0;
                while total_read < buf.len() {
                    match stream.read(&mut buf[total_read..]) {
                        Ok(0) => break,
                        Ok(n) => {
                            total_read += n;
                            if let Some(idx) =
                                buf[..total_read].windows(4).position(|w| w == b"\r\n\r\n")
                            {
                                let header_part = String::from_utf8_lossy(&buf[..idx]);
                                let body_len = header_part
                                    .lines()
                                    .find(|l| l.to_lowercase().starts_with("content-length:"))
                                    .and_then(|l| l.split(':').nth(1))
                                    .and_then(|s| s.trim().parse::<usize>().ok())
                                    .unwrap_or(0);
                                if total_read >= idx + 4 + body_len {
                                    break;
                                }
                            }
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
                if total_read == 0 {
                    continue;
                }
                let raw_text = String::from_utf8_lossy(&buf[..total_read]);
                let req_lower = raw_text.to_lowercase();

                // Verify enterprise authorization and custom header injection (case-insensitive check)
                assert!(
                    req_lower.contains("authorization: bearer test-secret-token-777"),
                    "Missing expected Authorization header in request: {raw_text}"
                );
                assert!(
                    req_lower.contains("x-custom-dept: engineering"),
                    "Missing expected X-Custom-Dept header in request: {raw_text}"
                );

                let body_str = if let Some(idx) = raw_text.find("\r\n\r\n") {
                    &raw_text[idx + 4..]
                } else if let Some(idx) = raw_text.find("\n\n") {
                    &raw_text[idx + 2..]
                } else {
                    ""
                };

                let req_json: Value = serde_json::from_str(body_str).unwrap_or_default();
                let req_id = req_json.get("id").cloned().unwrap_or(serde_json::json!(1));
                let method = req_json.get("method").and_then(Value::as_str).unwrap_or("");

                let resp_body = if method == "server/discover" {
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "error": { "code": -32601, "message": "Method not found" }
                    })
                    .to_string()
                } else if method == "tools/list" {
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "result": {
                            "resultType": "complete",
                            "tools": [
                                {
                                    "name": "enterprise_query",
                                    "description": "Query enterprise database",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": {
                                            "table": { "type": "string" }
                                        }
                                    }
                                }
                            ]
                        }
                    })
                    .to_string()
                } else if method == "tools/call" {
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "result": {
                            "resultType": "complete",
                            "content": [
                                {
                                    "type": "text",
                                    "text": "enterprise_query: retrieved 42 records"
                                }
                            ]
                        }
                    })
                    .to_string()
                } else {
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "error": { "code": -32601, "message": "Method not found" }
                    })
                    .to_string()
                };

                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    resp_body.len(),
                    resp_body
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Both);
                handled += 1;
            }
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        handled
    });

    let server_url = format!("http://{}", local_addr);

    // Add HTTP server configuration with custom header and bearer auth
    let add_out = fixture
        .command(&[
            "mcp",
            "add-http",
            "enterprise-node",
            &server_url,
            "--auth",
            "bearer",
            "--token",
            "test-secret-token-777",
            "--header",
            "X-Custom-Dept:engineering",
            "--json",
        ])
        .output()
        .expect("add-http should execute");
    assert!(
        add_out.status.success(),
        "add-http failed: {}",
        String::from_utf8_lossy(&add_out.stdout)
    );

    // Catalog tools over HTTP
    let cat_out = fixture
        .command(&["mcp", "catalog", "enterprise-node", "--allow", "--json"])
        .output()
        .expect("catalog should execute");
    assert!(
        cat_out.status.success(),
        "catalog failed: {}",
        String::from_utf8_lossy(&cat_out.stdout)
    );
    let cat_val: Value =
        serde_json::from_slice(&cat_out.stdout).expect("valid json catalog output");
    assert_eq!(cat_val["server_id"], "enterprise-node");
    let tools = cat_val["revision"]["tools"]
        .as_array()
        .expect("tools array");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["remote_name"], "enterprise_query");
    let local_id = tools[0]["local_id"].as_str().expect("tool local_id");

    // Call tool over HTTP
    let call_out = fixture
        .command(&[
            "mcp",
            "call",
            "enterprise-node",
            local_id,
            "--arguments-json",
            r#"{"table":"users"}"#,
            "--idempotency-key",
            "idem-key-777",
            "--allow",
            "--json",
        ])
        .output()
        .expect("call should execute");
    assert!(
        call_out.status.success(),
        "call failed: {}",
        String::from_utf8_lossy(&call_out.stdout)
    );
    let call_val: Value = serde_json::from_slice(&call_out.stdout).expect("valid json call output");
    assert_eq!(call_val["server_id"], "enterprise-node");
    assert_eq!(call_val["tool_id"], local_id);
    let content = call_val["result"]["content"]
        .as_array()
        .expect("content array");
    assert!(
        content[0]["text"]
            .as_str()
            .unwrap()
            .contains("retrieved 42 records")
    );

    running.store(false, Ordering::Relaxed);
    let handled_count = server_handle.join().expect("server thread finished");
    assert!(
        handled_count >= 3,
        "Server handled discover, catalog, and call requests (handled: {handled_count})"
    );
}
