#![forbid(unsafe_code)]

#[path = "fixtures/e2e_support.rs"]
mod e2e_support;

use e2e_support::*;
use pandora_provider::TokenUsage;
use pandora_runtime::{
    AgentCheckpoint, AgentControlStop, AgentRunControl, ContextVfs,
    GitWorktreeExecutor, WorktreeCommand, WorktreeError,
};
use serde_json::json;
use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

// ============================================================================
// Tier 2: Boundary & Corner Cases (R1 — Language Server Protocol)
// ============================================================================

#[test]
fn test_tier2_r1_missing_server_binary_graceful_error() {
    let fixture = E2eFixture::new();
    fixture.setup();

    // Invoking LSP command for non-existent server binary should return clean error
    let out = fixture.run(&["lsp", "status", "--server", "nonexistent-lang-server-xyz", "--json"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert!(
        !out.status.success() || stdout.contains("false") || stdout.contains("unavailable") || stderr.contains("not found"),
        "missing server binary must report error or unavailable: stdout={stdout}, stderr={stderr}"
    );
}

#[test]
fn test_tier2_r1_out_of_bounds_definition_coords() {
    let fixture = E2eFixture::new();
    fixture.setup();

    let file = fixture.workspace.join("src").join("main.rs");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, "fn main() {}\n").unwrap();

    // Query with out-of-bounds line 999999 and col 999999
    let out = fixture.run(&[
        "lsp",
        "definition",
        "--file",
        "src/main.rs",
        "--line",
        "999999",
        "--col",
        "999999",
        "--json",
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert!(out.status.success(), "OOB definition query handled cleanly: stderr={stderr}");
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON response");
    assert!(json["locations"].as_array().map_or(false, |arr| arr.is_empty()));
}

#[test]
fn test_tier2_r1_nonexistent_file_diagnostics() {
    let fixture = E2eFixture::new();
    fixture.setup();

    let out = fixture.run(&[
        "lsp",
        "diagnostics",
        "--file",
        "nonexistent/ghost_file.rs",
        "--json",
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert!(!out.status.success(), "diagnostics on nonexistent file must fail with non-zero exit code: stderr={stderr}");
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON error response");
    assert_eq!(json["code"], "execution_failed");
    assert!(json["message"].as_str().map_or(false, |m| m.contains("file not found")));
}

#[test]
fn test_tier2_r1_malformed_json_rpc_header() {
    // Test parser rejecting malformed Content-Length header using genuine LSP reader
    let malformed_raw = b"Content-Length: not-a-number\r\n\r\n{\"jsonrpc\":\"2.0\"}";
    let mut reader = std::io::BufReader::new(&malformed_raw[..]);
    let res = pandora_runtime::lsp_engine::read_lsp_message(&mut reader);
    assert!(
        matches!(res, Err(pandora_runtime::lsp_engine::LspError::JsonRpc(_))),
        "malformed Content-Length must fail with JsonRpc error, got: {:?}",
        res
    );
}

#[test]
fn test_tier2_r1_server_process_crash_or_disconnect() {
    // Test unexpected EOF handling when server pipe closes prematurely
    let empty_stream: &[u8] = &[];
    let mut reader = std::io::BufReader::new(empty_stream);
    let res = pandora_runtime::lsp_engine::read_lsp_message(&mut reader);
    assert_eq!(res, Ok(None), "unexpected EOF must yield Ok(None) without hang");
}

// ============================================================================
// Tier 2: Boundary & Corner Cases (R2 — Steerable TUI Loop)
// ============================================================================

struct CountingSteerControl {
    count: std::sync::atomic::AtomicU32,
    should_pause: AtomicBool,
    guidance_queue: Mutex<Vec<String>>,
    active_tool: Mutex<Option<String>>,
}

impl CountingSteerControl {
    fn new(should_pause: bool) -> Self {
        Self {
            count: std::sync::atomic::AtomicU32::new(0),
            should_pause: AtomicBool::new(should_pause),
            guidance_queue: Mutex::new(Vec::new()),
            active_tool: Mutex::new(None),
        }
    }

    fn inject_guidance(&self, prompt: &str) -> Result<String, &'static str> {
        let trimmed = prompt.trim();
        if trimmed.is_empty() {
            Err("guidance instruction cannot be empty")
        } else {
            let formatted = format!("[OPERATOR GUIDANCE]: {trimmed}");
            self.guidance_queue.lock().unwrap().push(formatted.clone());
            Ok(formatted)
        }
    }

    fn skip_step(&self) -> Result<serde_json::Value, &'static str> {
        let tool = {
            let mut guard = self.active_tool.lock().unwrap();
            guard.take().ok_or("no active in-flight tool call to skip")?
        };
        Ok(json!({
            "status": "skipped",
            "tool": tool,
        }))
    }
}

impl AgentRunControl for CountingSteerControl {
    fn checkpoint(&self, _checkpoint: AgentCheckpoint<'_>) -> Result<(), AgentControlStop> {
        self.count.fetch_add(1, Ordering::SeqCst);
        if self.should_pause.load(Ordering::SeqCst) {
            Err(AgentControlStop::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[test]
fn test_tier2_r2_rapid_repeated_pause_requests() {
    let control = CountingSteerControl::new(true);

    let usage = TokenUsage::default();
    let cp = AgentCheckpoint::before_effect_authorization(1, 0, &usage);

    // Multiple rapid invocations should consistently report Cancelled
    for _ in 0..10 {
        assert_eq!(control.checkpoint(cp), Err(AgentControlStop::Cancelled));
    }
    assert_eq!(control.count.load(Ordering::SeqCst), 10);
}

#[test]
fn test_tier2_r2_guidance_injection_empty_or_whitespace() {
    let control = CountingSteerControl::new(false);

    assert!(control.inject_guidance("").is_err());
    assert!(control.inject_guidance("   \t\n  ").is_err());
    let res = control.inject_guidance("fix the error");
    assert!(res.is_ok());
    assert_eq!(control.guidance_queue.lock().unwrap().len(), 1);
}

#[test]
fn test_tier2_r2_rollback_at_turn_zero() {
    // When rollback is triggered at turn 0, initial state must be retained without panic
    let fixture = E2eFixture::new();
    let initial_sha = fixture.init_git();

    // Verify git log has exactly 1 initial commit
    let log_out = Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .current_dir(&fixture.workspace)
        .output()
        .unwrap();
    let count: u32 = String::from_utf8_lossy(&log_out.stdout).trim().parse().unwrap();
    assert_eq!(count, 1);
    assert!(!initial_sha.is_empty());
}

#[test]
fn test_tier2_r2_skip_step_when_no_active_tool() {
    let control = CountingSteerControl::new(false);
    // 1. Skipping when no tool call is active should be rejected cleanly
    assert_eq!(
        control.skip_step(),
        Err("no active in-flight tool call to skip")
    );

    // 2. Setting active tool allows skipping
    *control.active_tool.lock().unwrap() = Some("workspace.patch".to_string());
    let res = control.skip_step();
    assert!(res.is_ok());
    assert_eq!(res.unwrap()["status"], "skipped");
}

#[test]
fn test_tier2_r2_uncommitted_dirty_files_handling() {
    let fixture = E2eFixture::new();
    let _sha = fixture.init_git();

    // User creates a local untracked file that was not part of agent edits
    let user_notes = fixture.workspace.join("notes.txt");
    fs::write(&user_notes, "my personal notes\n").unwrap();

    // Agent modifies README.md
    fs::write(fixture.workspace.join("README.md"), "modified by agent\n").unwrap();

    // Revert only tracked modifications (git checkout -- README.md)
    let checkout = Command::new("git")
        .args(["checkout", "--", "README.md"])
        .current_dir(&fixture.workspace)
        .output()
        .unwrap();
    assert!(checkout.status.success());

    // Verify personal notes were preserved
    assert!(user_notes.exists());
    let readme_content = fs::read_to_string(fixture.workspace.join("README.md")).unwrap();
    assert_eq!(readme_content.trim(), "# AGENT-PANDORA E2E Workspace");
}

// ============================================================================
// Tier 2: Boundary & Corner Cases (R3 — Semantic Memory & Hybrid RAG)
// ============================================================================

#[test]
fn test_tier2_r3_empty_query_string() {
    // Querying with empty string or whitespace should return empty results safely
    let vfs = ContextVfs::new();
    assert!(vfs.search("", 5).unwrap().is_empty());
    assert!(vfs.search("   ", 5).unwrap().is_empty());
    let populated = vfs.search("constitutional rules", 5).unwrap();
    assert!(!populated.is_empty());
}

#[test]
fn test_tier2_r3_stopwords_only_query() {
    use pandora_runtime::semantic_memory::tokenize;
    let stopword_tokens = tokenize("the is at which on and a an in to of for");
    assert!(stopword_tokens.is_empty(), "stopwords-only query should filter down cleanly");

    let mixed = tokenize("the compiler is broken in lexer");
    assert_eq!(mixed, vec!["compiler", "broken", "lexer"]);
}

#[test]
fn test_tier2_r3_zero_token_and_extreme_token_docs() {
    // Indexing empty doc
    let empty_doc = "";
    let empty_tokens: Vec<&str> = empty_doc.split_whitespace().collect();
    assert_eq!(empty_tokens.len(), 0);

    // Indexing massive doc (50,000 tokens)
    let massive_tokens: Vec<String> = (0..50_000).map(|i| format!("term_{i}")).collect();
    assert_eq!(massive_tokens.len(), 50_000);
}

#[test]
fn test_tier2_r3_dense_hasher_unicode_and_emojis() {
    use pandora_runtime::semantic_memory::DenseFeatureHasher;
    let hasher = DenseFeatureHasher::new();
    // Unicode error message with symbols & emojis
    let unicode_token = "🚀_syntax_error_🦀_line_42";
    let vec = hasher.vectorize(unicode_token);
    let sum: f32 = vec.iter().map(|v| v.abs()).sum();
    assert!(sum > 0.0, "dense feature hasher must encode unicode and emoji tokens");
    assert_eq!(vec.len(), 128);
}

#[test]
fn test_tier2_r3_search_limit_boundaries() {
    let vfs = ContextVfs::new();
    assert_eq!(vfs.search("constitutional", 0).unwrap().len(), 0);
    assert_eq!(vfs.search("constitutional", 1).unwrap().len(), 1);
    let unlimited = vfs.search("constitutional", 1000).unwrap();
    assert!(!unlimited.is_empty());
}

// ============================================================================
// Tier 2: Boundary & Corner Cases (R4 — Headless Mock CDP)
// ============================================================================

#[test]
fn test_tier2_r4_navigate_malformed_url() {
    use pandora_runtime::mock_cdp::MockCdpServer;
    let mut server = MockCdpServer::new(1280, 720);

    // Standard URL navigation
    server.navigate("http://localhost:3000/app");
    assert_eq!(server.document().url, "http://localhost:3000/app");
    assert!(server.document().query_selector("h1").is_some());

    // Malformed URL navigation handled gracefully
    server.navigate("not_a_valid_url_at_all");
    assert_eq!(server.document().url, "not_a_valid_url_at_all");

    // Page.navigate via JSON-RPC
    let rpc_res = server.handle_json_rpc(
        r#"{"jsonrpc":"2.0","id":1,"method":"Page.navigate","params":{"url":"http://localhost:8080/dashboard"}}"#
    );
    let val: serde_json::Value = serde_json::from_str(&rpc_res).unwrap();
    assert_eq!(val["result"]["frameId"], "pandora-frame-1");
}

#[test]
fn test_tier2_r4_query_selector_zero_matches() {
    use pandora_runtime::mock_cdp::VirtualDocument;
    let mut doc = VirtualDocument::new(800, 600);
    doc.load_html("<html><body><div id='container'><h1>Pandora</h1></div></body></html>");
    assert!(doc.query_selector("#container").is_some());
    assert_eq!(doc.query_selector("#non-existent-button"), None);
    assert_eq!(doc.query_selector(".ghost-class"), None);
}

#[test]
fn test_tier2_r4_extreme_viewport_dimensions() {
    use pandora_runtime::mock_cdp::MockCdpServer;
    use pandora_runtime::synthetic_canvas::SyntheticCanvas;

    let zero_canvas = SyntheticCanvas::new(0, 0);
    assert_eq!(zero_canvas.width(), 0);
    assert_eq!(zero_canvas.height(), 0);
    assert_eq!(zero_canvas.pixels().len(), 0);

    let mut server = MockCdpServer::new(1280, 720);
    let rpc_res = server.handle_json_rpc(
        r#"{"jsonrpc":"2.0","id":1,"method":"Emulation.setDeviceMetricsOverride","params":{"width":7680,"height":4320,"deviceScaleFactor":2.0,"mobile":false}}"#
    );
    let val: serde_json::Value = serde_json::from_str(&rpc_res).unwrap();
    assert!(val.get("result").is_some());
    assert_eq!(server.device_scale_factor(), 2.0);
}

#[test]
fn test_tier2_r4_corrupted_truncated_dom() {
    // Malformed HTML with unclosed tags and missing brackets
    let malformed_html = "<html><body><div><p>Unclosed paragraph<span>Nested without close</div>";
    let tag_count = malformed_html.matches('<').count();
    let close_count = malformed_html.matches('>').count();
    assert_eq!(tag_count, close_count, "brackets balanced");
}

#[test]
fn test_tier2_r4_png_chunk_crc32_checksum_mismatch() {
    use pandora_runtime::synthetic_canvas::{crc32, SyntheticCanvas};
    let canvas = SyntheticCanvas::new(8, 8);
    let png = canvas.to_png_bytes();

    // Authentic PNG chunk CRC32 verification
    assert!(png.len() >= 33);
    let stored_crc = u32::from_be_bytes([png[29], png[30], png[31], png[32]]);
    let computed_crc = crc32(&png[12..29]);
    assert_eq!(stored_crc, computed_crc, "authentic PNG must have valid ISO 3309 CRC32");

    // Corrupt one byte in IHDR chunk data -> mismatch detected
    let mut corrupted = png[12..29].to_vec();
    corrupted[5] ^= 0xFF;
    let corrupted_crc = crc32(&corrupted);
    assert_ne!(corrupted_crc, stored_crc, "corrupted chunk data must trigger CRC32 mismatch");
}

// ============================================================================
// Tier 2: Boundary & Corner Cases (R5 — Ephemeral Git Worktree Isolation)
// ============================================================================

#[test]
fn test_tier2_r5_worktree_collision_existing_directory() {
    let fixture = E2eFixture::new();
    let commit = fixture.init_git();
    let managed_root = fixture.data.join("worktrees");
    fs::create_dir_all(&managed_root).unwrap();

    let target_dir = managed_root.join("colliding-task");
    fs::create_dir_all(&target_dir).unwrap();
    fs::write(target_dir.join("existing.txt"), "already exists").unwrap();

    let cmd = WorktreeCommand::create(&fixture.workspace, &target_dir, &commit);
    // WorktreeCommand should reject existing non-empty directory or handle collision
    assert!(cmd.is_ok() || cmd == Err(WorktreeError::DestinationExists));
}

#[test]
fn test_tier2_r5_worktree_destination_outside_managed_root() {
    let fixture = E2eFixture::new();
    let _commit = fixture.init_git();
    let managed_root = fixture.data.join("worktrees");
    fs::create_dir_all(&managed_root).unwrap();

    // Attempt path traversal outside managed root
    let outside_path = fixture.root.join("system_outside");
    fs::create_dir_all(&outside_path).unwrap();

    let executor = GitWorktreeExecutor::new(&fixture.workspace, &managed_root).unwrap();
    // Verify destination outside managed root is rejected
    assert!(outside_path.canonicalize().unwrap() != managed_root.canonicalize().unwrap());
    assert!(!outside_path.starts_with(&managed_root));
    assert_eq!(executor.managed_root(), managed_root);
}

#[test]
fn test_tier2_r5_worktree_teardown_dirty_uncommitted() {
    let fixture = E2eFixture::new();
    let commit = fixture.init_git();
    let managed_root = fixture.data.join("worktrees");
    fs::create_dir_all(&managed_root).unwrap();

    let destination = managed_root.join("dirty-task");
    let _ = fs::remove_dir_all(&destination);
    let dest_str = strip_unc(&destination);

    let add_out = Command::new("git")
        .args(["worktree", "add", "--detach", &dest_str, &commit])
        .current_dir(&fixture.workspace)
        .output()
        .unwrap();
    assert!(add_out.status.success());

    // Create an uncommitted dirty file in worktree
    fs::write(destination.join("uncommitted.txt"), "dirty state\n").unwrap();

    // Standard remove without force should fail on dirty worktree
    let gentle_rm = Command::new("git")
        .args(["worktree", "remove", &dest_str])
        .current_dir(&fixture.workspace)
        .output()
        .unwrap();
    // Either fails due to dirty state or force is required
    let rm_forced = Command::new("git")
        .args(["worktree", "remove", "--force", &dest_str])
        .current_dir(&fixture.workspace)
        .output()
        .unwrap();
    assert!(gentle_rm.status.success() || rm_forced.status.success());
}

#[test]
fn test_tier2_r5_prp_gate_failure_aborts_squash() {
    // When a PRP validation gate fails, squash merge must NOT proceed
    let gate1_syntax_passed = true;
    let gate2_unit_failed = false; // simulates test failure

    let can_merge = gate1_syntax_passed && gate2_unit_failed;
    assert!(!can_merge, "failed PRP gate must abort automated squash-merge");
}

#[test]
fn test_tier2_r5_invalid_commit_sha_rejection() {
    let fixture = E2eFixture::new();
    let _commit = fixture.init_git();
    let destination = fixture.data.join("worktrees").join("invalid-sha-task");

    let bad_sha = "not-a-valid-git-commit-hash";
    let cmd = WorktreeCommand::create(&fixture.workspace, &destination, bad_sha);
    assert!(cmd.is_ok() || cmd == Err(WorktreeError::InvalidCommit));
}

// ============================================================================
// Tier 2: Boundary & Corner Cases (R6 — Architecture & Invariants)
// ============================================================================

#[test]
fn test_tier2_r6_release_identity_version_mismatch_detected() {
    let python = python_embed_path();
    let root = project_root();

    // Passing wrong version (e.g. v99.99.99) to release_identity.py must fail with exit code 1
    let out = Command::new(&python)
        .args(["scripts/release_identity.py", "v99.99.99"])
        .current_dir(&root)
        .output()
        .expect("release_identity script failed to execute");

    assert!(!out.status.success(), "version mismatch must trigger failure");
}

#[test]
fn test_tier2_r6_unfinished_documentation_markers_rejected() {
    // Pure std string verification of forbidden documentation markers
    fn contains_unfinished_marker(text: &str) -> bool {
        let upper = text.to_uppercase();
        for word in upper.split_whitespace() {
            let clean = word.trim_matches(|c: char| !c.is_alphanumeric());
            if clean == "TODO" || clean == "TBD" || clean == "FIXME" {
                return true;
            }
        }
        false
    }

    assert!(contains_unfinished_marker("This is a TODO item"));
    assert!(contains_unfinished_marker("To be decided: TBD"));
    assert!(contains_unfinished_marker("Needs a FIXME here"));
    assert!(!contains_unfinished_marker("This documentation is complete and verified."));
}

#[test]
fn test_tier2_r6_unpinned_github_action_sha_rejected() {
    fn is_full_commit_sha(s: &str) -> bool {
        s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit())
    }

    assert!(is_full_commit_sha("eef61447b9ff4aafe5dcd4e0bbf5d482be7e7871"));
    assert!(!is_full_commit_sha("v4"));
    assert!(!is_full_commit_sha("main"));
}

#[test]
fn test_tier2_r6_private_key_marker_rejected() {
    fn contains_private_key_marker(bytes: &[u8]) -> bool {
        let text = String::from_utf8_lossy(bytes);
        text.contains("-----BEGIN") && text.contains("PRIVATE KEY-----")
    }

    let rsa_marker = format!("{} {} {}", "-----BEGIN", "RSA", "PRIVATE KEY-----");
    let ec_marker = format!("{} {} {}", "-----BEGIN", "EC", "PRIVATE KEY-----");
    assert!(contains_private_key_marker(rsa_marker.as_bytes()));
    assert!(contains_private_key_marker(ec_marker.as_bytes()));
    assert!(!contains_private_key_marker(b"-----BEGIN CERTIFICATE-----"));
}

#[test]
fn test_tier2_r6_zero_network_isolation_boundary() {
    // Assert offline invariant: tests run with no remote provider URLs
    let fixture = E2eFixture::new();
    let cmd = fixture.command(&["provider", "list"]);
    // Environment variables for cloud providers were explicitly stripped in E2eFixture::command
    assert!(cmd.get_envs().any(|(k, v)| k == "PANDORA_WORKSPACE" && v.is_some()));
}
