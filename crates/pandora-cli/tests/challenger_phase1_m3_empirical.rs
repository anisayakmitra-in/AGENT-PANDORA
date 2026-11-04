#![forbid(unsafe_code)]

use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn pandora_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_pandora"))
}

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let unique = format!(
            "{}_{}_{}",
            prefix,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        fs::create_dir_all(&path).expect("failed to create temp dir");
        Self { path }
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn test_cli_hnsw_build_and_search_roundtrip_json() {
    let temp_guard = TempDirGuard::new("pandora_test_cli_roundtrip");
    let index_file = temp_guard.path.join("test_hnsw.idx");

    // 1. Build HNSW index with custom parameters via CLI in JSON mode
    let build_output = Command::new(pandora_bin())
        .args([
            "--json",
            "memory",
            "vfs",
            "hnsw-build",
            "--output",
            index_file.to_str().unwrap(),
            "--m",
            "8",
            "--ef-construction",
            "32",
        ])
        .output()
        .expect("failed to run pandora memory vfs hnsw-build");

    assert!(
        build_output.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&build_output.stderr)
    );

    let build_stdout = String::from_utf8(build_output.stdout).expect("valid utf8");
    let build_json: serde_json::Value =
        serde_json::from_str(&build_stdout).expect("valid build json envelope");

    assert_eq!(build_json["version"], "0.1");
    assert_eq!(build_json["command"], "memory vfs hnsw-build");
    assert_eq!(build_json["status"], "built");
    assert_eq!(build_json["m"], 8);
    assert_eq!(build_json["ef_construction"], 32);
    assert!(index_file.is_file(), "index file should exist on disk");

    // Verify PVHN binary file integrity on disk
    let file_bytes = fs::read(&index_file).expect("read index file");
    assert!(
        file_bytes.len() >= 80 + 32,
        "index file too small for header + trailer"
    );
    assert_eq!(&file_bytes[0..4], b"PVHN", "Magic bytes mismatch");
    assert_eq!(file_bytes[4], 1, "PVHN version must be 1");

    // Verify SHA-256 trailer matches body digest
    let body_len = file_bytes.len() - 32;
    let computed_digest = Sha256::digest(&file_bytes[..body_len]);
    assert_eq!(
        &file_bytes[body_len..],
        computed_digest.as_slice(),
        "SHA-256 trailer checksum verification failed"
    );

    // Verify cold-start load latency
    let cold_start_us = build_json["cold_start_load_us"].as_u64().unwrap_or(0);
    let max_cold_start = if cfg!(debug_assertions) {
        10_000
    } else {
        1_000
    };
    assert!(
        cold_start_us < max_cold_start,
        "Cold start load latency should be within budget (< {max_cold_start} µs), got {cold_start_us} µs"
    );

    // 2. Search HNSW index via CLI in JSON mode
    let search_output = Command::new(pandora_bin())
        .args([
            "--json",
            "memory",
            "vfs",
            "hnsw-search",
            "rules architectural integrity",
            "--path",
            index_file.to_str().unwrap(),
            "--limit",
            "3",
            "--ef-search",
            "16",
        ])
        .output()
        .expect("failed to run pandora memory vfs hnsw-search");

    assert!(
        search_output.status.success(),
        "search failed: {}",
        String::from_utf8_lossy(&search_output.stderr)
    );

    let search_stdout = String::from_utf8(search_output.stdout).expect("valid utf8");
    let search_json: serde_json::Value =
        serde_json::from_str(&search_stdout).expect("valid search json envelope");

    assert_eq!(search_json["version"], "0.1");
    assert_eq!(search_json["command"], "memory vfs hnsw-search");
    assert_eq!(search_json["query"], "rules architectural integrity");
    assert_eq!(search_json["limit"], 3);
    assert_eq!(search_json["ef_search"], 16);

    let search_time_us = search_json["search_time_us"].as_u64().unwrap_or(0);
    assert!(
        search_time_us < 10_000,
        "Search time should be fast, got {search_time_us} µs"
    );

    let results = search_json["results"].as_array().expect("results is array");
    assert!(!results.is_empty(), "expected at least one search result");

    let first = &results[0];
    assert!(first["uri"].is_string());
    assert!(first["title"].is_string());
    assert!(first["similarity"].is_number());
    assert!(first["distance"].is_number());
    assert!(first["dense_score"].is_number());
}

#[test]
fn test_cli_human_readable_output() {
    let temp_guard = TempDirGuard::new("pandora_test_cli_human");
    let index_file = temp_guard.path.join("human_hnsw.idx");

    let build_output = Command::new(pandora_bin())
        .args([
            "memory",
            "vfs",
            "hnsw-build",
            "--output",
            index_file.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run pandora memory vfs hnsw-build");

    assert!(build_output.status.success());
    let build_stdout = String::from_utf8(build_output.stdout).expect("valid utf8");
    assert!(
        build_stdout.contains("Built ContextVfs HNSW index at"),
        "stdout missing expected human string: {build_stdout}"
    );
    assert!(
        build_stdout.contains("SHA-256:"),
        "stdout missing SHA-256: {build_stdout}"
    );

    let search_output = Command::new(pandora_bin())
        .args([
            "memory",
            "vfs",
            "hnsw-search",
            "constitutional",
            "--path",
            index_file.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run pandora memory vfs hnsw-search");

    assert!(search_output.status.success());
    let search_stdout = String::from_utf8(search_output.stdout).expect("valid utf8");
    assert!(
        search_stdout.contains("Found") && search_stdout.contains("in ContextVfs HNSW index"),
        "stdout missing expected human search format: {search_stdout}"
    );
}

#[test]
fn test_cli_options_validation_and_errors() {
    // 1. memory vfs missing subcommand
    let out = Command::new(pandora_bin())
        .args(["memory", "vfs"])
        .output()
        .expect("exec");
    assert!(!out.status.success());
    let err_msg = String::from_utf8_lossy(&out.stderr);
    assert!(err_msg.contains("memory vfs requires 'ls', 'cat', 'search', 'index', 'stats', 'hnsw-build', or 'hnsw-search'"));

    // 2. hnsw-build rejects positional arguments
    let out = Command::new(pandora_bin())
        .args(["memory", "vfs", "hnsw-build", "positional_arg"])
        .output()
        .expect("exec");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("does not accept positional arguments"));

    // 3. hnsw-build rejects invalid m
    let out = Command::new(pandora_bin())
        .args(["memory", "vfs", "hnsw-build", "--m", "0"])
        .output()
        .expect("exec");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("m must be a positive integer"));

    let out = Command::new(pandora_bin())
        .args(["memory", "vfs", "hnsw-build", "--m", "abc"])
        .output()
        .expect("exec");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("m must be a positive integer"));

    // 4. hnsw-build rejects invalid ef-construction
    let out = Command::new(pandora_bin())
        .args(["memory", "vfs", "hnsw-build", "--ef-construction", "0"])
        .output()
        .expect("exec");
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("ef-construction must be a positive integer")
    );

    // 5. hnsw-build rejects unknown flag
    let out = Command::new(pandora_bin())
        .args(["memory", "vfs", "hnsw-build", "--invalid-flag"])
        .output()
        .expect("exec");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown option '--invalid-flag'"));

    // 6. hnsw-search requires query
    let out = Command::new(pandora_bin())
        .args(["memory", "vfs", "hnsw-search"])
        .output()
        .expect("exec");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("requires '<query>'"));

    // 7. hnsw-search rejects empty/whitespace query
    let out = Command::new(pandora_bin())
        .args(["memory", "vfs", "hnsw-search", "   "])
        .output()
        .expect("exec");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("requires '<query>'"));

    // 8. hnsw-search rejects limit 0
    let out = Command::new(pandora_bin())
        .args(["memory", "vfs", "hnsw-search", "query", "--limit", "0"])
        .output()
        .expect("exec");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("limit must be greater than zero"));

    // 9. hnsw-search rejects ef-search 0
    let out = Command::new(pandora_bin())
        .args(["memory", "vfs", "hnsw-search", "query", "--ef-search", "0"])
        .output()
        .expect("exec");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("ef-search must be greater than zero"));

    // 10. hnsw-search rejects invalid tier
    let out = Command::new(pandora_bin())
        .args(["memory", "vfs", "hnsw-search", "query", "--tier", "l5"])
        .output()
        .expect("exec");
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("memory tier must be 'l1', 'l2', 'l3', or 'l4'")
    );

    // 11. hnsw-search rejects invalid intent
    let out = Command::new(pandora_bin())
        .args([
            "memory",
            "vfs",
            "hnsw-search",
            "query",
            "--intent",
            "unknown",
        ])
        .output()
        .expect("exec");
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("memory intent must be repo, procedure, coding, or user")
    );
}

#[test]
fn test_cli_resilience_missing_and_corrupt_file() {
    let temp_guard = TempDirGuard::new("pandora_test_cli_resilience");
    let missing_file = temp_guard.path.join("does_not_exist.idx");
    let corrupt_file = temp_guard.path.join("corrupt.idx");

    // 1. Search when file does not exist -> gracefully builds index in memory and succeeds
    let search_missing = Command::new(pandora_bin())
        .args([
            "--json",
            "memory",
            "vfs",
            "hnsw-search",
            "test query",
            "--path",
            missing_file.to_str().unwrap(),
        ])
        .output()
        .expect("exec");
    assert!(search_missing.status.success());

    // 2. Corrupt file content (invalid header)
    fs::write(&corrupt_file, b"NOT_PVHN_CORRUPT_BYTES").expect("write corrupt file");

    // Search when file is corrupt -> detects corruption, recovers, does not panic
    let search_corrupt = Command::new(pandora_bin())
        .args([
            "--json",
            "memory",
            "vfs",
            "hnsw-search",
            "test query",
            "--path",
            corrupt_file.to_str().unwrap(),
        ])
        .output()
        .expect("exec");
    assert!(search_corrupt.status.success());

    // 3. Overwrite corrupt file with hnsw-build -> cleanly succeeds
    let rebuild_corrupt = Command::new(pandora_bin())
        .args([
            "--json",
            "memory",
            "vfs",
            "hnsw-build",
            "--output",
            corrupt_file.to_str().unwrap(),
            "--rebuild",
        ])
        .output()
        .expect("exec");
    assert!(rebuild_corrupt.status.success());

    let rb_stdout = String::from_utf8(rebuild_corrupt.stdout).expect("valid utf8");
    let rb_json: serde_json::Value = serde_json::from_str(&rb_stdout).expect("valid json");
    assert_eq!(rb_json["status"], "rebuilt");
}

#[test]
fn test_cli_shell_completions_cover_hnsw_subcommands() {
    for shell in ["powershell", "bash", "zsh", "fish"] {
        let out = Command::new(pandora_bin())
            .args(["completions", shell])
            .output()
            .expect("exec");
        assert!(out.status.success(), "completions {shell} failed");
        let script = String::from_utf8_lossy(&out.stdout);
        assert!(
            script.contains("hnsw-build"),
            "Completions for {shell} missing 'hnsw-build'"
        );
        assert!(
            script.contains("hnsw-search"),
            "Completions for {shell} missing 'hnsw-search'"
        );
    }
}
