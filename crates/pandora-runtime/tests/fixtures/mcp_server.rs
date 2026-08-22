use std::fs::OpenOptions;
use std::io::{self, BufRead, Write};
use std::time::Duration;

fn main() {
    let mut arguments = std::env::args().skip(1);
    let mode = arguments.next().unwrap_or_else(|| "modern".to_owned());
    let log_path = arguments.next();
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    let mut era = None;
    let mut request_count = 0_u32;
    for line in stdin.lock().lines() {
        let Ok(line) = line else { return };
        request_count += 1;
        let method = method_name(&line);
        if let Some(path) = log_path.as_deref() {
            if let Ok(mut log) = OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(log, "{} {method}", std::process::id());
            }
        }
        if method == "server/discover" {
            if !has_modern_metadata(&line) {
                return;
            }
            era = Some("modern");
            match mode.as_str() {
                "hang" => {
                    std::thread::sleep(Duration::from_secs(5));
                    return;
                }
                "exit" => return,
                "malformed" => write_line(&mut stdout, "{"),
                "non-utf8" => {
                    let _ = stdout.write_all(&[0xff, b'\n']);
                    let _ = stdout.flush();
                    return;
                }
                "oversized" => write_line(&mut stdout, &"x".repeat(200)),
                "multiline" => write_line(
                    &mut stdout,
                    "{\"jsonrpc\":\"2.0\",\n\"id\":1,\"result\":{}}",
                ),
                "bad-id" => write_line(
                    &mut stdout,
                    "{\"jsonrpc\":\"2.0\",\"id\":99,\"result\":{}}",
                ),
                "unexpected-method" => write_line(
                    &mut stdout,
                    "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"sampling/createMessage\",\"params\":{}}",
                ),
                "generic-error" => write_line(
                    &mut stdout,
                    "{\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{\"code\":-32603,\"message\":\"internal\"}}",
                ),
                "method-not-found" => write_line(
                    &mut stdout,
                    "{\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{\"code\":-32601,\"message\":\"Method not found\"}}",
                ),
                "explicit-legacy" => write_line(
                    &mut stdout,
                    "{\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{\"code\":-32022,\"message\":\"Unsupported protocol version\",\"data\":{\"requested\":\"2026-07-28\",\"supported\":[\"2025-11-25\"]}}}",
                ),
                "initialize-required" => write_line(
                    &mut stdout,
                    "{\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{\"code\":-32002,\"message\":\"Initialization required\",\"data\":{\"requiredMethod\":\"initialize\",\"supported\":[\"2025-11-25\"]}}}",
                ),
                "wrong-version" => write_line(
                    &mut stdout,
                    "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"resultType\":\"complete\",\"supportedVersions\":[\"2099-01-01\"],\"capabilities\":{\"tools\":{}}}}",
                ),
                "missing-tools" => write_line(
                    &mut stdout,
                    "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"resultType\":\"complete\",\"supportedVersions\":[\"2026-07-28\"],\"capabilities\":{}}}",
                ),
                _ => write_line(
                    &mut stdout,
                    "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"resultType\":\"complete\",\"supportedVersions\":[\"2026-07-28\"],\"capabilities\":{\"tools\":{}}}}",
                ),
            }
        } else if method == "initialize" {
            if request_count != 1
                || line.contains("io.modelcontextprotocol/protocolVersion")
                || !line.contains("\"protocolVersion\":\"2025-11-25\"")
            {
                return;
            }
            era = Some("legacy");
            write_line(
                &mut stdout,
                "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{\"tools\":{}},\"serverInfo\":{\"name\":\"fixture\",\"version\":\"1\"}}}",
            );
        } else if method == "notifications/initialized" {
            if era != Some("legacy") || line.contains("\"id\"") {
                return;
            }
        } else if method == "tools/list" {
            let modern = era == Some("modern");
            if modern != has_modern_metadata(&line) {
                return;
            }
            let schema = if mode == "unsupported-schema" {
                "{\"type\":\"object\",\"oneOf\":[{\"required\":[\"value\"]}]}"
            } else {
                "{\"type\":\"object\",\"properties\":{\"value\":{\"type\":\"string\"}},\"required\":[\"value\"],\"additionalProperties\":false}"
            };
            let result_type = if modern {
                "\"resultType\":\"complete\","
            } else {
                ""
            };
            write_line(
                &mut stdout,
                &format!("{{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{{{result_type}\"tools\":[{{\"name\":\"echo\",\"description\":\"Echo text\",\"inputSchema\":{schema}}}]}}}}"),
            );
        } else if method == "tools/call" {
            let modern = era == Some("modern");
            if modern != has_modern_metadata(&line) {
                return;
            }
            if mode == "invalid-result" {
                write_line(
                    &mut stdout,
                    "{\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{\"content\":\"invalid\"}}",
                );
            } else {
                let result_type = if modern {
                    "\"resultType\":\"complete\","
                } else {
                    ""
                };
                write_line(
                    &mut stdout,
                    &format!("{{\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{{{result_type}\"content\":[{{\"type\":\"text\",\"text\":\"echoed\"}}],\"isError\":false}}}}"),
                );
            }
        } else {
            return;
        }
    }
}

fn method_name(line: &str) -> &'static str {
    for method in [
        "server/discover",
        "initialize",
        "notifications/initialized",
        "tools/list",
        "tools/call",
    ] {
        if line.contains(&format!("\"method\":\"{method}\"")) {
            return method;
        }
    }
    "unknown"
}

fn has_modern_metadata(line: &str) -> bool {
    line.contains("io.modelcontextprotocol/protocolVersion")
        && line.contains("2026-07-28")
        && line.contains("io.modelcontextprotocol/clientInfo")
        && line.contains("io.modelcontextprotocol/clientCapabilities")
}

fn write_line(stdout: &mut impl Write, line: &str) {
    let _ = writeln!(stdout, "{line}");
    let _ = stdout.flush();
}
