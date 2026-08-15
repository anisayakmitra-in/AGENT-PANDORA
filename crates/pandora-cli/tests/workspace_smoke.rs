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
