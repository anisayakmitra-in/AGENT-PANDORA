use std::process::Command;

#[test]
fn cli_reports_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_pandora"))
        .arg("--version")
        .output()
        .expect("pandora binary should start");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        concat!("pandora ", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn cli_reports_version_in_the_json_envelope_when_requested() {
    let output = Command::new(env!("CARGO_BIN_EXE_pandora"))
        .args(["--version", "--json"])
        .output()
        .expect("pandora binary should start");

    assert!(output.status.success());
    let response: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("version output should be JSON");
    assert_eq!(response["command"], "version");
    assert_eq!(response["version"], "0.1");
    assert_eq!(response["pandora_version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn cli_reports_version_when_json_precedes_the_version_flag() {
    let output = Command::new(env!("CARGO_BIN_EXE_pandora"))
        .args(["--json", "--version"])
        .output()
        .expect("pandora binary should start");

    assert!(output.status.success());
    let response: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("version output should be JSON");
    assert_eq!(response["command"], "version");
    assert_eq!(response["pandora_version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn cli_help_is_successful_and_lists_the_primary_commands() {
    let output = Command::new(env!("CARGO_BIN_EXE_pandora"))
        .arg("--help")
        .output()
        .expect("pandora binary should start");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("usage: pandora"));
    assert!(stdout.contains("run"));
    assert!(stdout.contains("session list|resume|inspect <id>"));
    assert!(stdout.contains("chat [--provider <name>]"));
    assert!(stdout.contains("tui [--provider <name>]"));
    assert!(stdout.contains("doctor"));
    assert!(output.stderr.is_empty());
}

#[test]
fn cli_without_arguments_keeps_noninteractive_automation_explicit() {
    let output = Command::new(env!("CARGO_BIN_EXE_pandora"))
        .output()
        .expect("pandora binary should start");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("usage: pandora"));
}
