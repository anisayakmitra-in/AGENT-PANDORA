//! CLI wiring tests for `cockpit web` and `cockpit autopilot` (R1/R2).

#![forbid(unsafe_code)]

use serde_json::Value;
use std::process::Command;

fn run(args: &[&str]) -> (bool, String, String) {
    let dir = std::env::temp_dir().join(format!("pandora-w-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let out = Command::new(env!("CARGO_BIN_EXE_pandora"))
        .args(args)
        .env("PANDORA_CONFIG", dir.join("c.json"))
        .env("PANDORA_DATA_DIR", dir.join("d"))
        .env("PANDORA_WORKSPACE", &dir)
        .output()
        .expect("run");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn autopilot_cli_json() {
    let (ok, out, err) = run(&["cockpit", "autopilot", "--goal", "harden parser", "--json"]);
    assert!(ok, "{err}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["status"], "certified");
    assert_eq!(v["spawned_agents"].as_array().unwrap().len(), 9);
    assert!(
        v["patch_certificate"]
            .as_str()
            .unwrap()
            .starts_with("PANDORA_PATCH_CERT_V1:")
    );
}

#[test]
fn autopilot_slash_and_agent_subset() {
    let (ok, out, err) = run(&[
        "/autopilot",
        "fix",
        "bug",
        "--agents",
        "agy,claude,codex",
        "--json",
    ]);
    assert!(ok, "{err}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["spawned_agents"].as_array().unwrap().len(), 3);
}

#[test]
fn autopilot_requires_goal_and_valid_agents() {
    let (ok, _, _) = run(&["cockpit", "autopilot"]);
    assert!(!ok);
    let (ok, _, _) = run(&["cockpit", "autopilot", "--goal", "x", "--agents", "nobody"]);
    assert!(!ok);
}

#[test]
fn web_cli_reports_url_without_blocking() {
    let (ok, out, err) = run(&[
        "cockpit",
        "web",
        "--port",
        "0",
        "--bind",
        "127.0.0.1",
        "--dry-run",
        "--json",
    ]);
    assert!(ok, "{err}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["status"], "ok");
    assert_eq!(v["layout"], "twelve_panel");
    assert_eq!(v["sse_endpoint"], "/api/events");
}

#[test]
fn web_cli_rejects_bad_port() {
    let (ok, _, _) = run(&["cockpit", "web", "--port", "99999"]);
    assert!(!ok);
}
