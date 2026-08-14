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
        "pandora 2.0.0-alpha.1"
    );
}
