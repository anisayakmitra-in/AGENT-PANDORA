#![forbid(unsafe_code)]

use pandora_runtime::{BatchVectorDispatcher, Precision, RooflineModel};
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(100);

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
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let temp_root = std::env::temp_dir()
            .canonicalize()
            .expect("canonical temp dir");
        let root = temp_root.join(format!(
            "pandora-challenger-m1-1-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).expect("workspace should be created");
        Self {
            config: root.join("config.json"),
            data: root.join("data"),
            workspace,
            root,
        }
    }

    fn setup(&self) {
        let output = Command::new(env!("CARGO_BIN_EXE_pandora"))
            .args(["setup", "--config"])
            .arg(&self.config)
            .args(["--data-dir"])
            .arg(&self.data)
            .env("PANDORA_CONFIG", &self.config)
            .env("PANDORA_DATA_DIR", &self.data)
            .env("PANDORA_WORKSPACE", &self.workspace)
            .output()
            .expect("setup command should start");
        assert!(
            output.status.success(),
            "setup failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(self.config.exists(), "config file should exist");
    }

    fn run(&self, args: &[&str]) -> JsonResponse {
        let mut full_args = args.to_vec();
        if !full_args.contains(&"--json") {
            full_args.push("--json");
        }
        let output = Command::new(env!("CARGO_BIN_EXE_pandora"))
            .args(&full_args)
            .env("PANDORA_CONFIG", &self.config)
            .env("PANDORA_DATA_DIR", &self.data)
            .env("PANDORA_WORKSPACE", &self.workspace)
            .output()
            .expect("pandora command should execute");
        JsonResponse::from_output(output)
    }

    fn spawn_service(&self) -> std::process::Child {
        Command::new(env!("CARGO_BIN_EXE_pandora"))
            .args(["service", "start", "--port", "0"])
            .env("PANDORA_CONFIG", &self.config)
            .env("PANDORA_DATA_DIR", &self.data)
            .env("PANDORA_WORKSPACE", &self.workspace)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("pandora service start should spawn")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct JsonResponse {
    status: ExitStatus,
    raw_stdout: String,
    raw_stderr: String,
    value: Value,
}

impl JsonResponse {
    fn from_output(output: Output) -> Self {
        let raw_stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let raw_stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let value = serde_json::from_str(&raw_stdout).unwrap_or(Value::Null);
        Self {
            status: output.status,
            raw_stdout,
            raw_stderr,
            value,
        }
    }

    fn success(&self, expected_command: &str) -> &Value {
        assert!(
            self.status.success(),
            "command failed with status {:?}, stderr: {}",
            self.status,
            self.raw_stderr
        );
        assert_eq!(
            self.value["command"].as_str().unwrap_or(""),
            expected_command,
            "unexpected command in JSON: {}",
            self.raw_stdout
        );
        &self.value
    }
}

fn send_raw_http(addr: &str, request_str: &str) -> (u16, String, Vec<(String, String)>) {
    let mut stream = TcpStream::connect(addr).expect("connect to service TCP socket");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set read timeout");
    stream
        .write_all(request_str.as_bytes())
        .expect("write request to stream");
    stream.flush().expect("flush stream");

    let mut response_bytes = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                response_bytes.extend_from_slice(&buf[..n]);
                if response_bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                break;
            }
            Err(e) => panic!("socket read failed: {e}"),
        }
    }
    let response_str = String::from_utf8_lossy(&response_bytes).to_string();
    let status_line = response_str.lines().next().unwrap_or("");
    let parts: Vec<&str> = status_line.split_whitespace().collect();
    let code: u16 = if parts.len() >= 2 {
        parts[1].parse().unwrap_or(0)
    } else {
        0
    };

    let mut headers = Vec::new();
    for line in response_str.lines().skip(1) {
        if line.is_empty() || line == "\r" {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    (code, response_str, headers)
}

// ============================================================================
// TEST 1: Hardware Roofline Analytical Oracle & Stress Validation
// ============================================================================
#[test]
fn test_adversarial_hardware_roofline_intensity_oracle() {
    let fixture = Fixture::new();
    fixture.setup();

    // Theoretical model: I(B, N) = (2 / b) * (B * N) / (B + N)
    // where b = 4 for F32, b = 1 for Int8.
    let test_cases: Vec<(usize, usize, Precision, &str)> = vec![
        // Standard batch and corpus
        (64, 1024, Precision::F32, "f32"),
        (64, 1024, Precision::Int8, "int8"),
        (32, 4096, Precision::F32, "f32"),
        (32, 4096, Precision::Int8, "int8"),
        // Extreme minimal (B = 1, N = 1)
        (1, 1, Precision::F32, "f32"),
        (1, 1, Precision::Int8, "int8"),
        // Single query with huge corpus (B = 1, N = 1_000_000)
        (1, 1_000_000, Precision::F32, "f32"),
        (1, 1_000_000, Precision::Int8, "int8"),
        // Huge batch with standard corpus (B = 1_000_000, N = 1024)
        (1_000_000, 1024, Precision::F32, "f32"),
        (1_000_000, 1024, Precision::Int8, "int8"),
        // Extreme two-sided (B = 1_000_000, N = 1_000_000)
        (1_000_000, 1_000_000, Precision::F32, "f32"),
        (1_000_000, 1_000_000, Precision::Int8, "int8"),
    ];

    for (b, n, prec, prec_arg) in test_cases {
        let b_str = b.to_string();
        let n_str = n.to_string();
        let resp = fixture.run(&[
            "hardware",
            "roofline",
            "--batch",
            &b_str,
            "--corpus",
            &n_str,
            "--precision",
            prec_arg,
        ]);
        let val = resp.success("hardware roofline");

        let b_val = val["batch_size"].as_u64().unwrap() as usize;
        let n_val = val["corpus_size"].as_u64().unwrap() as usize;
        assert_eq!(b_val, b);
        assert_eq!(n_val, n);

        let observed_intensity = val["operational_intensity"].as_f64().unwrap();
        let direct_runtime_intensity = RooflineModel::calculate_intensity(b, n, prec);

        // Theoretical calculation:
        let bytes_per_elem = prec.bytes_per_element() as f64;
        let bf = b as f64;
        let nf = n as f64;
        let theoretical_intensity = (2.0 / bytes_per_elem) * (bf * nf) / (bf + nf);

        assert_eq!(
            val["bytes_per_element"].as_u64().unwrap() as usize,
            prec.bytes_per_element()
        );
        assert!(
            (observed_intensity - theoretical_intensity).abs() < 1e-5,
            "Roofline I(B,N) discrepancy for B={b}, N={n}, prec={prec:?}: observed={observed_intensity}, theoretical={theoretical_intensity}"
        );
        assert!(
            (observed_intensity - direct_runtime_intensity).abs() < 1e-9,
            "Discrepancy between CLI JSON intensity and runtime calculate_intensity"
        );
    }

    // Verify exact 4x scaling between Int8 and F32:
    let resp_f32 = fixture.run(&[
        "hardware",
        "roofline",
        "--batch",
        "128",
        "--corpus",
        "2048",
        "--precision",
        "f32",
    ]);
    let val_f32 = resp_f32.success("hardware roofline");
    let resp_int8 = fixture.run(&[
        "hardware",
        "roofline",
        "--batch",
        "128",
        "--corpus",
        "2048",
        "--precision",
        "int8",
    ]);
    let val_int8 = resp_int8.success("hardware roofline");

    let int8_intensity = val_int8["operational_intensity"].as_f64().unwrap();
    let f32_intensity = val_f32["operational_intensity"].as_f64().unwrap();
    let scaling = int8_intensity / f32_intensity;
    assert!(
        (scaling - 4.0).abs() < 1e-6,
        "Int8/F32 intensity ratio must be exactly 4.0, got {scaling}"
    );

    // Amdahl's Law Speedup verification: S(f, p) = 1 / ((1 - f) + f/p)
    let amdahl_cases = vec![
        (0.95, 8, 1.0 / (0.05 + 0.95 / 8.0)),
        (0.50, 4, 1.0 / (0.50 + 0.50 / 4.0)),
        (0.00, 8, 1.0),
        (1.00, 16, 16.0),
    ];
    for (f, p, expected_s) in amdahl_cases {
        let f_str = f.to_string();
        let p_str = p.to_string();
        let resp = fixture.run(&[
            "hardware",
            "roofline",
            "--parallel-fraction",
            &f_str,
            "--simd-lanes",
            &p_str,
        ]);
        let val = resp.success("hardware roofline");
        let speedup = val["amdahl_speedup"].as_f64().unwrap();
        assert!(
            (speedup - expected_s).abs() < 1e-5,
            "Amdahl speedup mismatch for f={f}, p={p}: expected {expected_s}, got {speedup}"
        );
    }

    // Zero parameters verification:
    let resp_zero_b = fixture.run(&["hardware", "roofline", "--batch", "0", "--corpus", "1024"]);
    let val_zero_b = resp_zero_b.success("hardware roofline");
    assert_eq!(val_zero_b["operational_intensity"].as_f64().unwrap(), 0.0);

    let resp_zero_n = fixture.run(&["hardware", "roofline", "--batch", "64", "--corpus", "0"]);
    let val_zero_n = resp_zero_n.success("hardware roofline");
    assert_eq!(val_zero_n["operational_intensity"].as_f64().unwrap(), 0.0);

    // Negative parameters rejection:
    let resp_neg_b = fixture.run(&["hardware", "roofline", "--batch=-5"]);
    assert!(!resp_neg_b.status.success(), "negative batch must fail");
    assert_eq!(resp_neg_b.value["code"], "usage_error");
    assert!(
        resp_neg_b.value["message"].as_str().unwrap().contains("--batch must be an integer"),
        "error message must mention batch integer requirement: {}",
        resp_neg_b.raw_stdout
    );

    let resp_neg_c = fixture.run(&["hardware", "roofline", "--corpus=-10"]);
    assert!(!resp_neg_c.status.success(), "negative corpus must fail");
    assert_eq!(resp_neg_c.value["code"], "usage_error");
    assert!(
        resp_neg_c.value["message"].as_str().unwrap().contains("--corpus must be an integer"),
        "error message must mention corpus integer requirement: {}",
        resp_neg_c.raw_stdout
    );
}

// ============================================================================
// TEST 2: Hardware Benchmark Stress Testing
// ============================================================================
#[test]
fn test_adversarial_hardware_benchmark_stress() {
    let fixture = Fixture::new();
    fixture.setup();

    // 1. Standard benchmark execution
    let resp = fixture.run(&[
        "hardware",
        "benchmark",
        "--dim",
        "64",
        "--batch",
        "16",
        "--corpus",
        "32",
        "--iterations",
        "5",
        "--top-k",
        "3",
    ]);
    let val = resp.success("hardware benchmark");
    assert_eq!(val["dim"], 64);
    assert_eq!(val["batch_size"], 16);
    assert_eq!(val["corpus_size"], 32);
    assert_eq!(val["matrix_rows"], 16);
    assert_eq!(val["matrix_cols"], 32);
    assert_eq!(val["int8_rows"], 16);
    assert!(val["f32_duration_us"].as_u64().unwrap() > 0);
    assert!(val["int8_duration_us"].as_u64().unwrap() > 0);
    assert!(val["speedup"].as_f64().unwrap() > 0.0);

    let top_k = val["sample_top_k"].as_array().unwrap();
    assert_eq!(top_k.len(), 3);
    // Verify scores are monotonically descending
    let score0 = top_k[0][1].as_f64().unwrap();
    let score1 = top_k[1][1].as_f64().unwrap();
    let score2 = top_k[2][1].as_f64().unwrap();
    assert!(
        score0 >= score1 && score1 >= score2,
        "top-k scores must be sorted descending: [{score0}, {score1}, {score2}]"
    );

    // 2. Boundary minimal dimension and batch: dim=1, batch=1, corpus=1
    let resp_min = fixture.run(&[
        "hardware",
        "benchmark",
        "--dim",
        "1",
        "--batch",
        "1",
        "--corpus",
        "1",
        "--iterations",
        "1",
        "--top-k",
        "1",
    ]);
    let val_min = resp_min.success("hardware benchmark");
    assert_eq!(val_min["matrix_rows"], 1);
    assert_eq!(val_min["matrix_cols"], 1);
    assert_eq!(val_min["sample_top_k"].as_array().unwrap().len(), 1);

    // 3. Top-K exceeding corpus size (clamp gracefully without panic)
    let resp_clamp = fixture.run(&[
        "hardware",
        "benchmark",
        "--dim",
        "16",
        "--batch",
        "4",
        "--corpus",
        "5",
        "--top-k",
        "50",
    ]);
    let val_clamp = resp_clamp.success("hardware benchmark");
    assert_eq!(val_clamp["corpus_size"], 5);
    // Top-k must clamp to available corpus elements
    assert_eq!(val_clamp["sample_top_k"].as_array().unwrap().len(), 5);

    // 4. Zero batch size (no divide-by-zero or out-of-bounds panic)
    let resp_zero_b = fixture.run(&["hardware", "benchmark", "--batch", "0", "--corpus", "16"]);
    let val_zero_b = resp_zero_b.success("hardware benchmark");
    assert_eq!(val_zero_b["matrix_rows"], 0);
    assert_eq!(val_zero_b["sample_top_k"].as_array().unwrap().len(), 0);

    // 5. Zero corpus size (no panic)
    let resp_zero_c = fixture.run(&["hardware", "benchmark", "--batch", "8", "--corpus", "0"]);
    let val_zero_c = resp_zero_c.success("hardware benchmark");
    assert_eq!(val_zero_c["matrix_cols"], 0);
    assert_eq!(val_zero_c["sample_top_k"].as_array().unwrap().len(), 0);

    // 6. Direct BatchVectorDispatcher numerical sanity check
    let a = vec![1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let b = vec![2.0f32, 0.5, 1.0, 2.0, 1.0, 0.5, 2.0, 1.0];
    // dot = 1*2 + 2*0.5 + 3*1 + 4*2 + 5*1 + 6*0.5 + 7*2 + 8*1 = 2 + 1 + 3 + 8 + 5 + 3 + 14 + 8 = 44
    let dot = BatchVectorDispatcher::dot_product_f32(&a, &b);
    assert_eq!(dot, 44.0);

    let ai = vec![10i8, 20, 30, 40, 50, 60, 70, 80];
    let bi = vec![1i8, 2, 1, 2, 1, 2, 1, 2];
    // dot = 10*1 + 20*2 + 30*1 + 40*2 + 50*1 + 60*2 + 70*1 + 80*2 = 10 + 40 + 30 + 80 + 50 + 120 + 70 + 160 = 560
    let doti = BatchVectorDispatcher::dot_product_int8(&ai, &bi);
    assert_eq!(doti, 560);
}

// ============================================================================
// TEST 3: Daemon /v1/ws RFC 6455 Streaming Handshake Verification & Stress
// ============================================================================
#[test]
fn test_adversarial_ws_handshake_streaming_daemon() {
    let fixture = Fixture::new();
    fixture.setup();

    let mut child = fixture.spawn_service();
    let stdout = child.stdout.take().expect("service stdout available");
    let mut reader = BufReader::new(stdout);
    let mut readiness_line = String::new();
    reader
        .read_line(&mut readiness_line)
        .expect("read readiness JSON line from daemon");

    let readiness: Value = serde_json::from_str(&readiness_line).expect("valid readiness json");
    let ws_endpoint = readiness["ws_endpoint"]
        .as_str()
        .expect("ws_endpoint present");
    let http_endpoint = readiness["endpoint"].as_str().expect("endpoint present");

    assert!(
        ws_endpoint.starts_with("ws://127.0.0.1:"),
        "ws_endpoint must be loopback ws://, got {ws_endpoint}"
    );
    assert!(
        ws_endpoint.ends_with("/v1/ws"),
        "ws_endpoint must end with /v1/ws, got {ws_endpoint}"
    );
    assert!(
        http_endpoint.ends_with("/v1/rpc"),
        "endpoint must end with /v1/rpc, got {http_endpoint}"
    );

    let token_path = readiness["token_path"]
        .as_str()
        .expect("token_path present");
    let token = fs::read_to_string(token_path).expect("read service token");
    let device_id = readiness["device_id"].as_str().expect("device_id present");
    let addr = ws_endpoint
        .strip_prefix("ws://")
        .unwrap()
        .strip_suffix("/v1/ws")
        .unwrap();

    let rfc_key = "dGhlIHNhbXBsZSBub25jZQ==";
    let expected_accept = "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=";

    // ------------------------------------------------------------------------
    // Scenario A: Valid Upgrade Handshake via Authorization Bearer Header
    // ------------------------------------------------------------------------
    let req_a = format!(
        "GET /v1/ws HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: {rfc_key}\r\n\
         Sec-WebSocket-Version: 13\r\n\
         Authorization: Bearer {token}\r\n\
         x-pandora-device-id: {device_id}\r\n\
         \r\n"
    );
    let (code_a, _raw_a, headers_a) = send_raw_http(addr, &req_a);
    assert_eq!(
        code_a, 101,
        "Valid handshake must yield 101 Switching Protocols"
    );
    let accept_header = headers_a
        .iter()
        .find(|(k, _)| k == "sec-websocket-accept")
        .map(|(_, v)| v.as_str());
    assert_eq!(
        accept_header,
        Some(expected_accept),
        "Sec-WebSocket-Accept must match RFC 6455 test vector"
    );
    let upgrade_header = headers_a
        .iter()
        .find(|(k, _)| k == "upgrade")
        .map(|(_, v)| v.as_str());
    assert_eq!(upgrade_header, Some("websocket"));

    // ------------------------------------------------------------------------
    // Scenario B: Valid Upgrade with Subprotocol Negotiation (pandora-v1)
    // ------------------------------------------------------------------------
    let req_b = format!(
        "GET /v1/ws HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: {rfc_key}\r\n\
         Sec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Protocol: pandora-v1\r\n\
         Authorization: Bearer {token}\r\n\
         x-pandora-device-id: {device_id}\r\n\
         \r\n"
    );
    let (code_b, _raw_b, headers_b) = send_raw_http(addr, &req_b);
    assert_eq!(code_b, 101);
    let proto_header = headers_b
        .iter()
        .find(|(k, _)| k == "sec-websocket-protocol")
        .map(|(_, v)| v.as_str());
    assert_eq!(proto_header, Some("pandora-v1"));

    // ------------------------------------------------------------------------
    // Scenario C: Valid Upgrade via Query String ?token=...&device_id=...
    // ------------------------------------------------------------------------
    let req_c = format!(
        "GET /v1/ws?token={token}&device_id={device_id} HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: {rfc_key}\r\n\
         Sec-WebSocket-Version: 13\r\n\
         \r\n"
    );
    let (code_c, _raw_c, _) = send_raw_http(addr, &req_c);
    assert_eq!(code_c, 101, "Query string token must authenticate");

    // ------------------------------------------------------------------------
    // Scenario D: Valid Upgrade via Query String ?bearer=...
    // ------------------------------------------------------------------------
    let req_d = format!(
        "GET /v1/ws?bearer={token}&device_id={device_id} HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: {rfc_key}\r\n\
         Sec-WebSocket-Version: 13\r\n\
         \r\n"
    );
    let (code_d, _raw_d, _) = send_raw_http(addr, &req_d);
    assert_eq!(code_d, 101, "Query string bearer must authenticate");

    // ------------------------------------------------------------------------
    // Scenario E: Valid Upgrade via Query String ?session_token=...
    // ------------------------------------------------------------------------
    let req_e = format!(
        "GET /v1/ws?session_token={token}&device_id={device_id} HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: {rfc_key}\r\n\
         Sec-WebSocket-Version: 13\r\n\
         \r\n"
    );
    let (code_e, _raw_e, _) = send_raw_http(addr, &req_e);
    assert_eq!(code_e, 101, "Query string session_token must authenticate");

    // ------------------------------------------------------------------------
    // Scenario F: Valid Upgrade via Sec-WebSocket-Protocol bearer token
    // ------------------------------------------------------------------------
    let req_f = format!(
        "GET /v1/ws?device_id={device_id} HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: {rfc_key}\r\n\
         Sec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Protocol: bearer, {token}, pandora-v1\r\n\
         \r\n"
    );
    let (code_f, _raw_f, _) = send_raw_http(addr, &req_f);
    assert_eq!(code_f, 101, "Sec-WebSocket-Protocol bearer must authenticate");

    // ------------------------------------------------------------------------
    // Adversarial G: Missing Upgrade Header -> 400 Bad Request
    // ------------------------------------------------------------------------
    let req_g = format!(
        "GET /v1/ws HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: {rfc_key}\r\n\
         Authorization: Bearer {token}\r\n\
         x-pandora-device-id: {device_id}\r\n\
         \r\n"
    );
    let (code_g, _, _) = send_raw_http(addr, &req_g);
    assert_eq!(code_g, 400, "Missing Upgrade header must return 400");

    // ------------------------------------------------------------------------
    // Adversarial H: Wrong Upgrade Header (not websocket) -> 400 Bad Request
    // ------------------------------------------------------------------------
    let req_h = format!(
        "GET /v1/ws HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Upgrade: h2c\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: {rfc_key}\r\n\
         Authorization: Bearer {token}\r\n\
         x-pandora-device-id: {device_id}\r\n\
         \r\n"
    );
    let (code_h, _, _) = send_raw_http(addr, &req_h);
    assert_eq!(code_h, 400, "Non-websocket Upgrade must return 400");

    // ------------------------------------------------------------------------
    // Adversarial I: Missing Connection Header -> 400 Bad Request
    // ------------------------------------------------------------------------
    let req_i = format!(
        "GET /v1/ws HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Upgrade: websocket\r\n\
         Sec-WebSocket-Key: {rfc_key}\r\n\
         Authorization: Bearer {token}\r\n\
         x-pandora-device-id: {device_id}\r\n\
         \r\n"
    );
    let (code_i, _, _) = send_raw_http(addr, &req_i);
    assert_eq!(code_i, 400, "Missing Connection header must return 400");

    // ------------------------------------------------------------------------
    // Adversarial J: Non-Upgrade Connection Header -> 400 Bad Request
    // ------------------------------------------------------------------------
    let req_j = format!(
        "GET /v1/ws HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Upgrade: websocket\r\n\
         Connection: keep-alive\r\n\
         Sec-WebSocket-Key: {rfc_key}\r\n\
         Authorization: Bearer {token}\r\n\
         x-pandora-device-id: {device_id}\r\n\
         \r\n"
    );
    let (code_j, _, _) = send_raw_http(addr, &req_j);
    assert_eq!(code_j, 400, "Non-Upgrade Connection must return 400");

    // ------------------------------------------------------------------------
    // Adversarial K: Missing Sec-WebSocket-Key -> 400 Bad Request
    // ------------------------------------------------------------------------
    let req_k = format!(
        "GET /v1/ws HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Authorization: Bearer {token}\r\n\
         x-pandora-device-id: {device_id}\r\n\
         \r\n"
    );
    let (code_k, _, _) = send_raw_http(addr, &req_k);
    assert_eq!(code_k, 400, "Missing Sec-WebSocket-Key must return 400");

    // ------------------------------------------------------------------------
    // Adversarial L: Empty Sec-WebSocket-Key -> 400 Bad Request
    // ------------------------------------------------------------------------
    let req_l = format!(
        "GET /v1/ws HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: \r\n\
         Authorization: Bearer {token}\r\n\
         x-pandora-device-id: {device_id}\r\n\
         \r\n"
    );
    let (code_l, _, _) = send_raw_http(addr, &req_l);
    assert_eq!(code_l, 400, "Empty Sec-WebSocket-Key must return 400");

    // ------------------------------------------------------------------------
    // Adversarial M: Whitespace-only Sec-WebSocket-Key -> 400 Bad Request
    // ------------------------------------------------------------------------
    let req_m = format!(
        "GET /v1/ws HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key:    \r\n\
         Authorization: Bearer {token}\r\n\
         x-pandora-device-id: {device_id}\r\n\
         \r\n"
    );
    let (code_m, _, _) = send_raw_http(addr, &req_m);
    assert_eq!(
        code_m, 400,
        "Whitespace-only Sec-WebSocket-Key must return 400"
    );

    // ------------------------------------------------------------------------
    // Adversarial N: Unauthenticated (no token) -> 401 Unauthorized
    // ------------------------------------------------------------------------
    let req_n = format!(
        "GET /v1/ws HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: {rfc_key}\r\n\
         \r\n"
    );
    let (code_n, _, _) = send_raw_http(addr, &req_n);
    assert_eq!(code_n, 401, "Unauthenticated request must return 401");

    // ------------------------------------------------------------------------
    // Adversarial O: Invalid Bearer Token -> 401 Unauthorized
    // ------------------------------------------------------------------------
    let req_o = format!(
        "GET /v1/ws HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: {rfc_key}\r\n\
         Authorization: Bearer invalid-forged-token-xyz\r\n\
         x-pandora-device-id: {device_id}\r\n\
         \r\n"
    );
    let (code_o, _, _) = send_raw_http(addr, &req_o);
    assert_eq!(code_o, 401, "Invalid Bearer token must return 401");

    // ------------------------------------------------------------------------
    // Adversarial P: Invalid Query Token -> 401 Unauthorized
    // ------------------------------------------------------------------------
    let req_p = format!(
        "GET /v1/ws?token=invalid-query-token&device_id={device_id} HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: {rfc_key}\r\n\
         \r\n"
    );
    let (code_p, _, _) = send_raw_http(addr, &req_p);
    assert_eq!(code_p, 401, "Invalid query token must return 401");

    // ------------------------------------------------------------------------
    // Adversarial Q: Invalid Subprotocol Token -> 401 Unauthorized
    // ------------------------------------------------------------------------
    let req_q = format!(
        "GET /v1/ws?device_id={device_id} HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: {rfc_key}\r\n\
         Sec-WebSocket-Protocol: bearer, forged-token\r\n\
         \r\n"
    );
    let (code_q, _, _) = send_raw_http(addr, &req_q);
    assert_eq!(code_q, 401, "Invalid subprotocol token must return 401");

    // ------------------------------------------------------------------------
    // Adversarial R: Non-GET HTTP Method (POST) -> 405 Method Not Allowed
    // ------------------------------------------------------------------------
    let req_r = format!(
        "POST /v1/ws HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: {rfc_key}\r\n\
         Authorization: Bearer {token}\r\n\
         x-pandora-device-id: {device_id}\r\n\
         \r\n"
    );
    let (code_r, _, _) = send_raw_http(addr, &req_r);
    assert_eq!(code_r, 405, "POST /v1/ws must return 405 Method Not Allowed");

    // Clean teardown: kill child daemon
    let _ = child.kill();
    let _ = child.wait();
}
