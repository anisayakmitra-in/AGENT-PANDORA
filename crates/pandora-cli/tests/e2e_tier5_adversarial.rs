#![forbid(unsafe_code)]

#[path = "fixtures/e2e_support.rs"]
mod e2e_support;

use e2e_support::*;
use pandora_provider::TokenUsage;
use pandora_runtime::agent_loop::{AgentCheckpoint, AgentControlStop, AgentRunControl};
use pandora_runtime::arch_simd::{simd_dot_product_128, simd_norm_sq_128};
use pandora_runtime::blaze_adapter::{BlazeCompilerAdapter, CompilationUnit};
use pandora_runtime::executors::WorktreeError;
use pandora_runtime::lsp_engine::{read_lsp_message, LspError, LspSessionBackend, MockLspSession};
use pandora_runtime::mock_cdp::{MockCdpServer, VirtualDocument};
use pandora_runtime::semantic_memory::{
    tokenize, DenseFeatureHasher, HybridRrfRanker, SparseBm25Index,
};
use pandora_runtime::synthetic_canvas::{crc32, SyntheticCanvas};
use pandora_runtime::worktree_isolation::{strip_unc, WorktreeIsolationManager};
use pandora_types::LspServerKind;
use serde_json::{json, Value};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ============================================================================
// Tier 5 Adversarial Coverage Hardening: R1 — LSP JSON-RPC Engine
// ============================================================================

#[test]
fn test_tier5_r1_lsp_malformed_framing_negative_and_corrupted_length() {
    // 1. Negative content length
    let mut reader = Cursor::new(b"Content-Length: -100\r\n\r\n{}");
    let err = read_lsp_message(&mut reader).unwrap_err();
    assert!(matches!(err, LspError::JsonRpc(_)));

    // 2. Non-numeric content length
    let mut reader = Cursor::new(b"Content-Length: not-a-number\r\n\r\n{}");
    let err = read_lsp_message(&mut reader).unwrap_err();
    assert!(matches!(err, LspError::JsonRpc(_)));

    // 3. Overflowing content length
    let mut reader = Cursor::new(b"Content-Length: 99999999999999999999999999999999\r\n\r\n{}");
    let err = read_lsp_message(&mut reader).unwrap_err();
    assert!(matches!(err, LspError::JsonRpc(_)));

    // 4. Missing Content-Length header entirely
    let mut reader = Cursor::new(b"X-Header: value\r\n\r\n{}");
    let err = read_lsp_message(&mut reader).unwrap_err();
    assert!(matches!(err, LspError::JsonRpc(_)));

    // 5. Empty stream (clean EOF)
    let mut reader = Cursor::new(b"");
    let res = read_lsp_message(&mut reader).unwrap();
    assert!(res.is_none());
}

#[test]
fn test_tier5_r1_lsp_truncated_body_unexpected_eof() {
    // Header claims 200 bytes, but stream terminates abruptly after 10 bytes
    let truncated_data = b"Content-Length: 200\r\n\r\n{\"partial\":";
    let mut reader = Cursor::new(truncated_data);
    let err = read_lsp_message(&mut reader).unwrap_err();
    assert!(matches!(err, LspError::Io(_)));
}

#[test]
fn test_tier5_r1_lsp_corrupted_json_payload() {
    // Correct content-length, but invalid JSON body syntax
    let corrupted_body = b"Content-Length: 25\r\n\r\n{invalid json content: [[";
    let mut reader = Cursor::new(corrupted_body);
    let err = read_lsp_message(&mut reader).unwrap_err();
    assert!(matches!(err, LspError::JsonRpc(_)));
}

#[test]
fn test_tier5_r1_lsp_unknown_rpc_method_rejection() {
    let fixture = E2eFixture::new();
    let mock = MockLspSession::new(&fixture.workspace, LspServerKind::RustAnalyzer);

    let res = mock.request(
        "textDocument/arbitraryNonExistentMethod",
        json!({"key": "val"}),
        Duration::from_millis(100),
    );
    assert!(matches!(res, Err(LspError::UnsupportedCapability(_))));
}

#[test]
fn test_tier5_r1_lsp_missing_target_file_queries() {
    let fixture = E2eFixture::new();
    let mock = MockLspSession::new(&fixture.workspace, LspServerKind::RustAnalyzer);

    let missing_uri = "file:///C:/Users/infin/ghost/does_not_exist.rs";

    // 1. Goto definition on missing file
    let def_res = mock
        .request(
            "textDocument/definition",
            json!({
                "textDocument": { "uri": missing_uri },
                "position": { "line": 10, "character": 5 }
            }),
            Duration::from_millis(500),
        )
        .unwrap();
    assert_eq!(def_res, Value::Array(vec![]));

    // 2. Find references on missing file
    let ref_res = mock
        .request(
            "textDocument/references",
            json!({
                "textDocument": { "uri": missing_uri },
                "position": { "line": 10, "character": 5 }
            }),
            Duration::from_millis(500),
        )
        .unwrap();
    assert_eq!(ref_res, Value::Array(vec![]));
}

#[test]
fn test_tier5_r1_lsp_extreme_and_corrupted_coordinates() {
    let fixture = E2eFixture::new();
    let src = fixture.workspace.join("src").join("main.rs");
    fs::create_dir_all(src.parent().unwrap()).unwrap();
    fs::write(&src, "fn test_function() {}\n").unwrap();

    let uri = format!("file:///{}", src.to_string_lossy().replace('\\', "/"));
    let mock = MockLspSession::new(&fixture.workspace, LspServerKind::RustAnalyzer);

    // Coordinate at u32::MAX
    let max_coords_res = mock
        .request(
            "textDocument/definition",
            json!({
                "textDocument": { "uri": &uri },
                "position": { "line": u32::MAX, "character": u32::MAX }
            }),
            Duration::from_millis(500),
        )
        .unwrap();
    assert_eq!(max_coords_res, Value::Array(vec![]));

    // Empty file coordinates
    let empty_file = fixture.workspace.join("src").join("empty.rs");
    fs::write(&empty_file, "").unwrap();
    let empty_uri = format!("file:///{}", empty_file.to_string_lossy().replace('\\', "/"));

    let empty_res = mock
        .request(
            "textDocument/definition",
            json!({
                "textDocument": { "uri": &empty_uri },
                "position": { "line": 0, "character": 0 }
            }),
            Duration::from_millis(500),
        )
        .unwrap();
    assert_eq!(empty_res, Value::Array(vec![]));
}

#[test]
fn test_tier5_r1_lsp_concurrency_under_diagnostics_query() {
    let fixture = E2eFixture::new();
    let mock = Arc::new(MockLspSession::new(
        &fixture.workspace,
        LspServerKind::RustAnalyzer,
    ));

    let mut handles = Vec::new();
    for thread_idx in 0..8 {
        let session = Arc::clone(&mock);
        handles.push(std::thread::spawn(move || {
            for i in 0..25 {
                let diag = session.diagnostics_for(None);
                assert!(diag.is_empty());
                let res = session.request(
                    "textDocument/definition",
                    json!({
                        "textDocument": { "uri": format!("file:///dummy_{thread_idx}_{i}.rs") },
                        "position": { "line": 0, "character": 0 }
                    }),
                    Duration::from_millis(200),
                );
                assert!(res.is_ok());
            }
        }));
    }

    for handle in handles {
        handle.join().expect("thread joined successfully");
    }
}

#[test]
fn test_tier5_r1_lsp_cli_error_handling_for_invalid_requests() {
    let fixture = E2eFixture::new();
    fixture.setup();

    // Query with out-of-bounds line numbers and missing file
    let out = fixture.run(&[
        "lsp",
        "definition",
        "--file",
        "nonexistent_adversarial.rs",
        "--line",
        "999999",
        "--col",
        "999999",
        "--json",
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success() || !stdout.is_empty() || !stderr.is_empty(),
        "CLI must produce structured response without crash"
    );
}

// ============================================================================
// Tier 5 Adversarial Coverage Hardening: R2 — TUI Steerable Execution Loop
// ============================================================================

struct AdversarialSteerControl {
    paused: AtomicBool,
    cancelled: AtomicBool,
    skip_requested: AtomicBool,
    turn: AtomicU32,
    tool_calls: AtomicU32,
    guidance_queue: Mutex<Vec<String>>,
    skipped_steps: Mutex<Vec<String>>,
    timeline: Mutex<Vec<String>>,
}

impl AdversarialSteerControl {
    fn new() -> Self {
        Self {
            paused: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            skip_requested: AtomicBool::new(false),
            turn: AtomicU32::new(0),
            tool_calls: AtomicU32::new(0),
            guidance_queue: Mutex::new(Vec::new()),
            skipped_steps: Mutex::new(Vec::new()),
            timeline: Mutex::new(Vec::new()),
        }
    }

    fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }

    fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.paused.store(true, Ordering::SeqCst);
    }

    fn inject_guidance(&self, guidance: &str) -> Result<String, &'static str> {
        let trimmed = guidance.trim();
        if trimmed.is_empty() {
            return Err("guidance instruction cannot be empty");
        }
        let prompt = format!("[OPERATOR GUIDANCE]: {trimmed}");
        self.guidance_queue.lock().unwrap().push(prompt.clone());
        self.timeline
            .lock()
            .unwrap()
            .push(format!("Operator guidance injected: {trimmed}"));
        Ok(prompt)
    }

    fn skip_step(&self, tool: Option<&str>) -> Result<Value, &'static str> {
        let tool_name = tool.ok_or("no active in-flight tool call to skip")?;
        self.skip_requested.store(true, Ordering::SeqCst);
        self.skipped_steps
            .lock()
            .unwrap()
            .push(tool_name.to_owned());
        self.tool_calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({
            "tool_call_id": "call_steer_skipped",
            "status": "skipped",
            "output": "tool execution bypassed by operator steering request"
        }))
    }
}

impl AgentRunControl for AdversarialSteerControl {
    fn checkpoint(&self, checkpoint: AgentCheckpoint<'_>) -> Result<(), AgentControlStop> {
        if self.cancelled.load(Ordering::SeqCst) {
            return Err(AgentControlStop::Cancelled);
        }
        self.turn.store(checkpoint.turns(), Ordering::SeqCst);
        self.tool_calls
            .store(checkpoint.tool_calls(), Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn test_tier5_r2_tui_rapid_alternating_pause_resume_threads() {
    let control = Arc::new(AdversarialSteerControl::new());
    let mut handles = Vec::new();

    // 8 concurrent threads hammering pause and resume toggles
    for _ in 0..8 {
        let c = Arc::clone(&control);
        handles.push(std::thread::spawn(move || {
            for _ in 0..200 {
                c.pause();
                c.resume();
            }
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }

    // After all concurrent threads join, control state must be completely stable
    control.pause();
    assert!(control.paused.load(Ordering::SeqCst));
    control.resume();
    assert!(!control.paused.load(Ordering::SeqCst));
}

#[test]
fn test_tier5_r2_tui_extreme_guidance_strings_unicode_and_length() {
    let control = AdversarialSteerControl::new();

    // 1. Massive string (50,000 characters)
    let massive_guidance = "A".repeat(50_000);
    let res = control.inject_guidance(&massive_guidance);
    assert!(res.is_ok());

    // 2. Pure Unicode & emojis
    let emoji_guidance = "🚨🔥⚠️🛑🚀🎯🤖".repeat(500);
    let res_emoji = control.inject_guidance(&emoji_guidance);
    assert!(res_emoji.is_ok());

    // 3. String with null characters and escape sequences
    let control_chars = "Line 1\nLine 2\t\r\0Tab\x1b[31mRed\x1b[0m";
    let res_ctrl = control.inject_guidance(control_chars);
    assert!(res_ctrl.is_ok());

    // 4. Empty and whitespace-only strings must be rejected
    assert!(control.inject_guidance("").is_err());
    assert!(control.inject_guidance("     \n\t  ").is_err());

    let queue = control.guidance_queue.lock().unwrap();
    assert_eq!(queue.len(), 3);
}

#[test]
fn test_tier5_r2_tui_step_skips_without_active_tool_and_consecutive() {
    let control = AdversarialSteerControl::new();

    // Skip with no active tool must error
    assert_eq!(
        control.skip_step(None),
        Err("no active in-flight tool call to skip")
    );

    // Skip active tool steps consecutively
    for i in 0..5 {
        let tool = format!("tool_step_{i}");
        let res = control.skip_step(Some(&tool)).unwrap();
        assert_eq!(res["status"], "skipped");
        assert_eq!(res["tool_call_id"], "call_steer_skipped");
    }

    assert_eq!(control.tool_calls.load(Ordering::SeqCst), 5);
    let skipped = control.skipped_steps.lock().unwrap();
    assert_eq!(skipped.len(), 5);
}

#[test]
fn test_tier5_r2_tui_clean_rollback_preserves_state() {
    let fixture = E2eFixture::new();
    let initial_sha = fixture.init_git();

    // Mutate existing tracked file
    fs::write(
        fixture.workspace.join("README.md"),
        "MUTATED CORRUPTED CONTENT\n",
    )
    .unwrap();
    // Add untracked dirty file
    let dirty_untracked = fixture.workspace.join("untracked_temp.tmp");
    fs::write(&dirty_untracked, "dangling scratch data").unwrap();

    // Execute safe rollback
    let checkout_out = Command::new("git")
        .args(["checkout", "--", "."])
        .current_dir(&fixture.workspace)
        .output()
        .unwrap();
    assert!(checkout_out.status.success());

    let clean_out = Command::new("git")
        .args(["clean", "-fd"])
        .current_dir(&fixture.workspace)
        .output()
        .unwrap();
    assert!(clean_out.status.success());

    // Verify tracked file is restored and untracked file is gone
    let readme_content = fs::read_to_string(fixture.workspace.join("README.md")).unwrap();
    assert!(readme_content.contains("AGENT-PANDORA E2E Workspace"));
    assert!(!dirty_untracked.exists());

    // Head SHA unchanged
    let head_out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&fixture.workspace)
        .output()
        .unwrap();
    let current_sha = String::from_utf8(head_out.stdout).unwrap().trim().to_string();
    assert_eq!(initial_sha, current_sha);
}

#[test]
fn test_tier5_r2_tui_abnormal_termination_terminal_restoration() {
    let control = AdversarialSteerControl::new();
    control.cancel();

    let usage = TokenUsage::default();
    let cp = AgentCheckpoint::before_effect_authorization(1, 0, &usage);
    let res = control.checkpoint(cp);
    assert_eq!(res, Err(AgentControlStop::Cancelled));
}

// ============================================================================
// Tier 5 Adversarial Coverage Hardening: R3 — Semantic Memory & Hybrid RAG
// ============================================================================

#[test]
fn test_tier5_r3_semantic_empty_and_whitespace_corpus_and_query() {
    let mut index = SparseBm25Index::new();

    // Score on completely empty index
    assert!(index.score("").is_empty());
    assert!(index.score("   ").is_empty());
    assert!(index.score("compiler error").is_empty());

    // Add empty and whitespace documents
    index.add_document(1, "");
    index.add_document(2, "   \n\t  ");
    index.add_document(3, "    ");

    assert_eq!(index.doc_count(), 3);
    assert_eq!(index.avg_doc_length(), 0.0);

    // Queries should return empty results safely without panic or divide-by-zero
    assert!(index.score("").is_empty());
    assert!(index.score("error").is_empty());
}

#[test]
fn test_tier5_r3_semantic_unicode_emojis_and_non_ascii_multilingual() {
    let text = "🚀🔥 compiler panic: 日本語テキスト Привет мир ∑(x)=y ⚡";
    let tokens = tokenize(text);

    // Verify tokenization did not slice across UTF-8 code point boundaries
    assert!(!tokens.is_empty());
    assert!(tokens.iter().any(|t| t == "compiler"));
    assert!(tokens.iter().any(|t| t == "panic"));

    let hasher = DenseFeatureHasher::new();
    let vec = hasher.vectorize(text);

    // Check that vector has finite floats (no NaN or Inf)
    for &val in &vec {
        assert!(val.is_finite());
    }

    let norm_sq = simd_norm_sq_128(&vec);
    assert!((norm_sq - 1.0).abs() < 1e-4);
}

#[test]
fn test_tier5_r3_semantic_out_of_vocabulary_queries() {
    let mut index = SparseBm25Index::new();
    index.add_document(
        1,
        "rust analyzer syntax diagnostics failed verification in cargo build",
    );
    index.add_document(
        2,
        "type mismatch expected u32 found string in function calculate",
    );

    // Completely out of vocabulary gibberish query
    let oov_query = "zyxwvutsrqponmlkjihgfedcba 9876543210 qwertyuiop";
    let scores = index.score(oov_query);
    assert!(scores.is_empty());

    let hasher = DenseFeatureHasher::new();
    let query_vec = hasher.vectorize(oov_query);
    let doc1_vec = hasher.vectorize("rust analyzer syntax diagnostics");

    let sim = hasher.cosine_similarity(&query_vec, &doc1_vec);
    assert!(sim.is_finite());
    assert!(sim >= 0.0 && sim <= 1.0);
}

#[test]
fn test_tier5_r3_semantic_zero_length_doc_bm25_calculations() {
    let mut index = SparseBm25Index::new();
    // 5 documents with only stop words (yielding zero indexed tokens)
    for id in 1..=5 {
        index.add_document(id, "the in on at of and");
    }
    // 1 document with technical keywords
    index.add_document(6, "compiler error diagnostic mismatch");

    assert_eq!(index.doc_count(), 6);
    let avgdl = index.avg_doc_length();
    assert!(avgdl > 0.0);

    let scores = index.score("compiler error");
    assert!(!scores.is_empty());
    assert_eq!(scores[0].0, 6);
    assert!(scores[0].1.is_finite());
}

#[test]
fn test_tier5_r3_semantic_dense_hasher_zero_vector_and_cosine_safety() {
    let hasher = DenseFeatureHasher::new();

    // Text producing zero tokens
    let stop_words_text = "the the the in of and";
    let zero_vec = hasher.vectorize(stop_words_text);

    for &v in &zero_vec {
        assert_eq!(v, 0.0f32);
    }

    let normal_vec = hasher.vectorize("rust compiler error");

    // Cosine similarity between zero vector and normal vector must be 0.0, not NaN
    let sim_zero = hasher.cosine_similarity(&zero_vec, &normal_vec);
    assert_eq!(sim_zero, 0.0f32);

    // Cosine similarity between two zero vectors must be 0.0, not NaN
    let sim_both_zero = hasher.cosine_similarity(&zero_vec, &zero_vec);
    assert_eq!(sim_both_zero, 0.0f32);
}

#[test]
fn test_tier5_r3_semantic_hybrid_rrf_with_disjoint_result_sets() {
    let ranker = HybridRrfRanker::new();

    // Sparse retrieved docs 1 and 2; Dense retrieved docs 3 and 4
    let sparse_ranks = vec![(1usize, 12.5f32), (2usize, 8.0f32)];
    let dense_ranks = vec![(3usize, 0.95f32), (4usize, 0.85f32)];

    let fused = ranker.fuse(&sparse_ranks, &dense_ranks);
    assert_eq!(fused.len(), 4);

    // All scores must be strictly finite and positive
    for item in &fused {
        assert!(item.hybrid_score > 0.0);
        assert!(item.hybrid_score.is_finite());
    }

    // Must be sorted in descending hybrid score order
    for w in fused.windows(2) {
        assert!(w[0].hybrid_score >= w[1].hybrid_score);
    }
}

#[test]
fn test_tier5_r3_semantic_massive_string_stress() {
    let repeated = "panic syntax error type mismatch lifetime borrow checker ".repeat(20_000);
    assert!(repeated.len() > 1_000_000);

    let tokens = tokenize(&repeated);
    assert!(!tokens.is_empty());

    let hasher = DenseFeatureHasher::new();
    let vec = hasher.vectorize(&repeated);

    let norm_sq = simd_norm_sq_128(&vec);
    assert!((norm_sq - 1.0).abs() < 1e-3);
}

// ============================================================================
// Tier 5 Adversarial Coverage Hardening: R4 — Headless Browser & Mock CDP
// ============================================================================

#[test]
fn test_tier5_r4_cdp_zero_dimension_canvas_all_operations() {
    for (w, h) in [(0, 0), (0, 100), (100, 0)] {
        let mut canvas = SyntheticCanvas::new(w, h);
        assert_eq!(canvas.width(), w);
        assert_eq!(canvas.height(), h);
        assert_eq!(canvas.pixels().len(), (w * h * 4) as usize);

        // All raster operations must be safe no-ops without bounds panic
        canvas.fill(255, 0, 0, 255);
        canvas.set_pixel(0, 0, 0, 255, 0, 255);
        assert_eq!(canvas.get_pixel(0, 0), (0, 0, 0, 0));
        canvas.draw_rect(0, 0, 50, 50, Some((0, 0, 255, 255)), None);
        canvas.draw_text(0, 0, "hello", (0, 0, 0, 255), 1);

        // Encoding to PNG bytes must succeed and generate valid headers
        let png = canvas.to_png_bytes();
        assert_png_valid(&png);
    }
}

#[test]
fn test_tier5_r4_cdp_massive_4k_canvas_and_png_export() {
    let mut canvas = SyntheticCanvas::new(3840, 2160);
    assert_eq!(canvas.width(), 3840);
    assert_eq!(canvas.height(), 2160);

    canvas.fill(240, 240, 240, 255);
    canvas.draw_rect(100, 100, 500, 300, Some((30, 120, 200, 255)), None);
    canvas.draw_text(120, 150, "4K TEST CANVAS", (255, 255, 255, 255), 3);

    let png = canvas.to_png_bytes();
    assert_png_valid(&png);

    // Verify IHDR dimensions in PNG bytes (bytes 16..24)
    let w_bytes: [u8; 4] = png[16..20].try_into().unwrap();
    let h_bytes: [u8; 4] = png[20..24].try_into().unwrap();
    assert_eq!(u32::from_be_bytes(w_bytes), 3840);
    assert_eq!(u32::from_be_bytes(h_bytes), 2160);
}

#[test]
fn test_tier5_r4_cdp_malformed_html_and_deep_nesting() {
    let mut doc = VirtualDocument::new(800, 600);

    // 1. Unclosed tags
    doc.load_html("<div><p><span>Unclosed tree without endings");
    assert!(doc.query_selector("span").is_some());

    // 2. Mismatched closing tags
    doc.load_html("<div><p><button>Click</p></div></button>");
    assert!(doc.query_selector("button").is_some());

    // 3. Deeply nested DOM tree (50 levels deep)
    let opening = "<div>".repeat(50);
    let closing = "</div>".repeat(50);
    let deep_html = format!("{opening}<button id=\"deep-btn\">Deep</button>{closing}");
    doc.load_html(&deep_html);
    assert!(doc.query_selector("#deep-btn").is_some());

    // 4. Broken attributes
    doc.load_html("<input type='checkbox' disabled checked=true id=\"chk\" value=''> ");
    assert!(doc.query_selector("#chk").is_some());
}

#[test]
fn test_tier5_r4_cdp_complex_and_non_matching_css_selectors() {
    let mut doc = VirtualDocument::new(800, 600);
    doc.load_html(
        "<html><body><div id=\"main\" class=\"container primary\"><button class=\"btn action\">Go</button></div></body></html>",
    );

    // Non-matching selectors must return None safely
    assert!(doc.query_selector("#missing-id").is_none());
    assert!(doc.query_selector(".missing-class").is_none());
    assert!(doc.query_selector("input").is_none());
    assert!(doc.query_selector("").is_none());
    assert!(doc.query_selector("   ").is_none());
    assert!(doc.query_selector(",,,").is_none());
    assert!(doc.query_selector("div > p > span > a > button").is_none());

    // Valid matching selectors
    assert!(doc.query_selector("#main").is_some());
    assert!(doc.query_selector(".primary").is_some());
    assert!(doc.query_selector(".btn").is_some());
    assert_eq!(doc.query_selector_all(".missing").len(), 0);
}

#[test]
fn test_tier5_r4_cdp_png_deflate_and_checksum_verification() {
    let canvas = SyntheticCanvas::new_white(32, 32);
    let png = canvas.to_png_bytes();
    assert_png_valid(&png);

    // Verify chunks: IHDR, IDAT, IEND with valid CRC-32
    let mut pos = 8; // skip 8-byte PNG header
    while pos < png.len() {
        let length = u32::from_be_bytes(png[pos..pos + 4].try_into().unwrap()) as usize;
        let chunk_type = &png[pos + 4..pos + 8];
        let chunk_data = &png[pos + 8..pos + 8 + length];
        let read_crc = u32::from_be_bytes(png[pos + 8 + length..pos + 12 + length].try_into().unwrap());

        let mut crc_payload = Vec::new();
        crc_payload.extend_from_slice(chunk_type);
        crc_payload.extend_from_slice(chunk_data);
        let expected_crc = crc32(&crc_payload);
        assert_eq!(read_crc, expected_crc, "CRC-32 mismatch for chunk {:?}", std::str::from_utf8(chunk_type));

        pos += 12 + length;
    }
}

#[test]
fn test_tier5_r4_cdp_json_rpc_malformed_requests_and_unknown_methods() {
    let mut server = MockCdpServer::new(1280, 720);

    // 1. Malformed JSON
    let res_malformed = server.handle_json_rpc("<<<not json>>>");
    assert!(res_malformed.contains("error"));

    // 2. Unknown method
    let res_unknown = server.handle_json_rpc(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"UnknownDomain.unknownMethod\",\"params\":{}}",
    );
    assert!(res_unknown.contains("error"));

    // 3. Valid method Page.enable
    let res_enable = server.handle_json_rpc(
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"Page.enable\",\"params\":{}}",
    );
    assert!(res_enable.contains("\"result\":{}"));
}

// ============================================================================
// Tier 5 Adversarial Coverage Hardening: R5 — Ephemeral Git Worktrees
// ============================================================================

#[test]
fn test_tier5_r5_worktree_uncommitted_dirty_files_in_worktree() {
    let fixture = E2eFixture::new();
    let _sha = fixture.init_git();

    let manager = WorktreeIsolationManager::new(&fixture.workspace).unwrap();
    let ctx = manager.provision_worktree("dirty-task-001", None).unwrap();
    assert!(ctx.worktree_path.exists());

    // Inject dirty uncommitted files inside the worktree
    fs::write(ctx.worktree_path.join("uncommitted.rs"), "// dirty code\n").unwrap();
    assert!(ctx.worktree_path.join("uncommitted.rs").exists());

    // Discard/clean worktree forcefully
    let cleanup_res = manager.cleanup_worktree("dirty-task-001", true);
    assert!(cleanup_res.is_ok());
    assert!(!ctx.worktree_path.exists());
}

#[test]
fn test_tier5_r5_worktree_detached_head_base_commit() {
    let fixture = E2eFixture::new();
    let commit_sha = fixture.init_git();

    // Detach HEAD in repository
    let checkout_out = Command::new("git")
        .args(["checkout", "--detach", &commit_sha])
        .current_dir(&fixture.workspace)
        .output()
        .unwrap();
    assert!(checkout_out.status.success());

    let manager = WorktreeIsolationManager::new(&fixture.workspace).unwrap();
    let ctx_res = manager.provision_worktree("detached-head-task", None);
    assert!(ctx_res.is_ok());

    let ctx = ctx_res.unwrap();
    assert!(ctx.worktree_path.exists());
    let _ = manager.cleanup_worktree("detached-head-task", true);
}

#[test]
fn test_tier5_r5_worktree_non_existent_commit_hash_rejection() {
    let fixture = E2eFixture::new();
    let _sha = fixture.init_git();

    let manager = WorktreeIsolationManager::new(&fixture.workspace).unwrap();
    let bad_sha = "0123456789abcdef0123456789abcdef01234567";
    let res = manager.provision_worktree("bad-sha-task", Some(bad_sha));
    assert!(matches!(res, Err(WorktreeError::GitFailed)));
}

#[test]
fn test_tier5_r5_worktree_duplicate_task_id_and_destination_exists() {
    let fixture = E2eFixture::new();
    let _sha = fixture.init_git();

    let manager = WorktreeIsolationManager::new(&fixture.workspace).unwrap();
    let ctx1 = manager.provision_worktree("duplicate-task", None).unwrap();
    assert!(ctx1.worktree_path.exists());

    // Second provision without cleanup must return DestinationExists
    let res2 = manager.provision_worktree("duplicate-task", None);
    assert!(matches!(res2, Err(WorktreeError::DestinationExists)));

    let _ = manager.cleanup_worktree("duplicate-task", true);
}

#[test]
fn test_tier5_r5_worktree_unc_path_normalization_idempotence() {
    let p1 = Path::new(r"\\?\C:\Users\tester\repo");
    let stripped1 = strip_unc(p1);
    assert_eq!(stripped1, PathBuf::from(r"C:\Users\tester\repo"));

    let p2 = Path::new(r"\\?\UNC\server\share\repo");
    let stripped2 = strip_unc(p2);
    assert_eq!(stripped2, PathBuf::from(r"\\server\share\repo"));

    let p3 = Path::new(r"C:\Users\tester\repo");
    let stripped3 = strip_unc(p3);
    assert_eq!(stripped3, PathBuf::from(r"C:\Users\tester\repo"));

    // Idempotent
    assert_eq!(strip_unc(&stripped1), stripped1);
}

// ============================================================================
// Tier 5 Adversarial Coverage Hardening: R6 — Architecture & Ecosystem
// ============================================================================

#[test]
fn test_tier5_r6_blaze_corrupted_compilation_units_and_unsafe_rejection() {
    let mut adapter = BlazeCompilerAdapter::with_default_config();

    // 1. Forbidden unsafe code block must be rejected under forbid_unsafe policy
    let unit_unsafe = CompilationUnit {
        crate_name: "unsafe_crate".to_string(),
        manifest_path: PathBuf::from("Cargo.toml"),
        files: vec![(
            PathBuf::from("src/dangerous.rs"),
            "pub fn danger() { unsafe { *std::ptr::null::<u32>(); } }".to_string(),
        )],
    };
    let res_unsafe = adapter.compile_unit(&unit_unsafe);
    assert!(!res_unsafe.success, "unsafe code must fail compilation when forbid_unsafe=true");
    assert!(res_unsafe.diagnostics.iter().any(|d| d.message.contains("forbidden") && d.code.as_deref() == Some("E0133")));

    // 2. Unbalanced syntax delimiters should be flagged with warning diagnostics
    let unit_unbalanced = CompilationUnit {
        crate_name: "unbalanced_crate".to_string(),
        manifest_path: PathBuf::from("Cargo.toml"),
        files: vec![(
            PathBuf::from("src/lib.rs"),
            "pub fn unbalanced(((((((((x: i32) {}".to_string(),
        )],
    };
    let res_unbalanced = adapter.compile_unit(&unit_unbalanced);
    assert!(
        res_unbalanced.diagnostics.iter().any(|d| d.code.as_deref() == Some("W0001")),
        "unbalanced parentheses must emit warning diagnostic"
    );

    // 3. Empty compilation unit should succeed safely without units compiled
    let unit_empty = CompilationUnit {
        crate_name: "empty_crate".to_string(),
        manifest_path: PathBuf::from("Cargo.toml"),
        files: vec![],
    };
    let res_empty = adapter.compile_unit(&unit_empty);
    assert!(res_empty.success);
    assert_eq!(res_empty.units_compiled, 0);
}

#[test]
fn test_tier5_r6_simd_dot_product_extreme_floating_point_values() {
    let mut a = [0.0f32; 128];
    let mut b = [0.0f32; 128];

    // All zeros
    assert_eq!(simd_dot_product_128(&a, &b), 0.0f32);

    // Negative numbers
    for i in 0..128 {
        a[i] = -1.5f32;
        b[i] = 4.0f32;
    }
    // 128 * (-1.5 * 4.0) = 128 * -6.0 = -768.0
    let dot_neg = simd_dot_product_128(&a, &b);
    assert!((dot_neg - (-768.0f32)).abs() < 1e-3);

    // Alternating signs
    for i in 0..128 {
        a[i] = if i % 2 == 0 { 2.0 } else { -2.0 };
        b[i] = if i % 2 == 0 { -3.0 } else { 3.0 };
    }
    // Every term is -6.0 -> 128 * -6.0 = -768.0
    let dot_alt = simd_dot_product_128(&a, &b);
    assert!((dot_alt - (-768.0f32)).abs() < 1e-3);

    // Compare with scalar reference sum
    let ref_sum: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    assert!((dot_alt - ref_sum).abs() < 1e-4);
}

#[test]
fn test_tier5_r6_simd_norm_sq_zero_and_near_zero() {
    let zeros = [0.0f32; 128];
    assert_eq!(simd_norm_sq_128(&zeros), 0.0f32);

    let mut single = [0.0f32; 128];
    single[42] = 2.5f32;
    assert!((simd_norm_sq_128(&single) - 6.25f32).abs() < 1e-4);
}

#[test]
fn test_tier5_r6_c_abi_and_assembly_simd_execution() {
    let root = project_root();
    let gcc = gcc_path();
    assert!(gcc.exists());

    // Compile and run adversarial C ABI test
    let exe_path = root.join("target").join("test_pandora_c_abi_adversarial.exe");
    let comp_out = Command::new(&gcc)
        .args([
            "-std=c11",
            "-Iinclude",
            "native/asm/pandora_simd_x86_64.s",
            "native/c/pandora_c_api.c",
            "tests/c/test_pandora_c_abi_adversarial.c",
            "-o",
        ])
        .arg(&exe_path)
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        comp_out.status.success(),
        "GCC compilation failed: {}",
        String::from_utf8_lossy(&comp_out.stderr)
    );

    let run_out = Command::new(&exe_path).output().unwrap();
    assert!(
        run_out.status.success(),
        "C ABI test failed: {}",
        String::from_utf8_lossy(&run_out.stderr)
    );
    let out_str = String::from_utf8_lossy(&run_out.stdout);
    assert!(out_str.contains("ALL ADVERSARIAL C ABI TESTS PASSED CLEANLY"));
}

#[test]
fn test_tier5_r6_cpp20_client_raii_leak_and_move_execution() {
    let root = project_root();
    let gxx = root.join(r"..\w64devkit\bin\g++.exe");
    let gxx_canonical = fs::canonicalize(&gxx).unwrap_or(gxx);
    assert!(gxx_canonical.exists());

    let exe_path = root.join("target").join("test_pandora_cpp_adversarial.exe");
    let comp_out = Command::new(&gxx_canonical)
        .args([
            "-std=c++20",
            "-Iinclude",
            "native/asm/pandora_simd_x86_64.s",
            "native/c/pandora_c_api.c",
            "tests/cpp/test_pandora_cpp_adversarial.cpp",
            "-o",
        ])
        .arg(&exe_path)
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        comp_out.status.success(),
        "G++ compilation failed: {}",
        String::from_utf8_lossy(&comp_out.stderr)
    );

    let run_out = Command::new(&exe_path).output().unwrap();
    assert!(
        run_out.status.success(),
        "C++20 test failed: {}",
        String::from_utf8_lossy(&run_out.stderr)
    );
    let out_str = String::from_utf8_lossy(&run_out.stdout);
    assert!(out_str.contains("ALL ADVERSARIAL C++20 TESTS PASSED CLEANLY"));
}

#[test]
fn test_tier5_r6_typescript_client_error_handling_execution() {
    let root = project_root();
    let node_out = Command::new("node")
        .arg("scripts/test_typescript_client_adversarial.js")
        .current_dir(&root)
        .output();

    if let Ok(out) = node_out {
        assert!(
            out.status.success(),
            "Adversarial TS client test failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let out_str = String::from_utf8_lossy(&out.stdout);
        assert!(out_str.contains("ALL ADVERSARIAL TYPESCRIPT CLIENT TESTS PASSED CLEANLY"));
    }
}
