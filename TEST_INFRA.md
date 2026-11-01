# AGENT-PANDORA v3.0.1 Test Infrastructure & E2E Test Architecture

## 1. Test Architecture & Design Philosophy

The AGENT-PANDORA v3.0.1 End-to-End (E2E) testing infrastructure provides a robust, requirement-driven, opaque-box verification framework. It validates that the autonomous agent runtime, developer tooling, steering mechanics, semantic memory, visual verification, and git worktree isolation operate reliably under strict safety constraints.

### Core Architectural Invariants
1. **Opaque-Box Boundary**: Tests interact strictly through public CLI entry points, published JSON-RPC protocol interfaces, and public library APIs. Internal private details are never coupled directly into test assertions.
2. **Strict `#![forbid(unsafe_code)]`**: Every Rust test module enforces `#![forbid(unsafe_code)]` at line 1, honoring workspace integrity.
3. **Zero Network / Offline Guarantee**: No external network calls, remote API downloads, or browser binary dependencies are permitted. All services (LSP, CDP, SQLite) use synthetic self-contained fixtures.
4. **Isolated Sandboxed Execution**: Every test instantiates a dedicated `E2eFixture` directory with isolated temporary workspace, data, and configuration paths. RAII teardown cleans all temporary directories upon completion.
5. **Deterministic Expected Outputs**: Every test derives expected outputs from specifications defined in `ORIGINAL_REQUEST.md` and `PROJECT.md`.

---

## 2. Four-Tier Test Case Design Methodology

| Tier | Tier Focus | Test Count | Description |
|---|---|---|---|
| **Tier 1** | Feature Coverage | 31 tests | Isolated, happy-path functional verification covering all capabilities (>=5 tests per feature). |
| **Tier 2** | Boundary & Corner Cases | 30 tests | Adversarial inputs, coordinate bounds, empty queries, missing binaries, disconnects, and process crash recovery (>=5 tests per feature). |
| **Tier 3** | Cross-Feature Combinations | 8 tests | Pairwise subsystem integration (e.g., LSP diagnostics inside isolated worktrees, memory search during paused TUI state). |
| **Tier 4** | Real-World Application Scenarios | 5 workloads | Multi-step end-to-end engineering workflows (autonomous refactoring, visual regression, session recovery, and PR generation). |
| **Total** | | **74 tests** | Comprehensive 100% test coverage across all features. |

---

## 3. Feature Inventory to Test Mapping

| Feature # | Capability | Requirement | Tier 1 Tests | Tier 2 Tests | Tier 3 & 4 Tests |
|---|---|---|---|---|---|
| **1-6** | LSP Engine (`pandora lsp`) | R1 | `test_tier1_r1_lsp_status_and_server_detection`<br>`test_tier1_r1_lsp_servers_list_supported`<br>`test_tier1_r1_lsp_definition_query`<br>`test_tier1_r1_lsp_references_query`<br>`test_tier1_r1_lsp_type_definition_query`<br>`test_tier1_r1_lsp_pretest_diagnostics` | `test_tier2_r1_missing_server_binary_graceful_error`<br>`test_tier2_r1_out_of_bounds_definition_coords`<br>`test_tier2_r1_nonexistent_file_diagnostics`<br>`test_tier2_r1_malformed_json_rpc_header`<br>`test_tier2_r1_server_process_crash_or_disconnect` | `test_tier3_lsp_precheck_inside_ephemeral_worktree`<br>`test_tier3_steer_guidance_injected_from_lsp_diagnostics`<br>`test_tier4_workload1_autonomous_goal_refactoring_with_lsp_and_squash`<br>`test_tier4_workload4_multilingual_lsp_diagnostics_resolution` |
| **7-11** | Steerable TUI Loop | R2 | `test_tier1_r2_tui_agent_run_control_checkpoint`<br>`test_tier1_r2_tui_steer_inflight_inspection`<br>`test_tier1_r2_tui_steer_guidance_injection`<br>`test_tier1_r2_tui_steer_skip_step`<br>`test_tier1_r2_tui_steer_cancellation_rollback` | `test_tier2_r2_rapid_repeated_pause_requests`<br>`test_tier2_r2_guidance_injection_empty_or_whitespace`<br>`test_tier2_r2_rollback_at_turn_zero`<br>`test_tier2_r2_skip_step_when_no_active_tool`<br>`test_tier2_r2_uncommitted_dirty_files_handling` | `test_tier3_semantic_memory_lookup_during_tui_paused_checkpoint`<br>`test_tier3_rollback_restoring_worktree_after_failed_visual_gate`<br>`test_tier4_workload5_full_autonomous_pipeline_with_steering` |
| **12-15** | Semantic Memory & Hybrid RAG | R3 | `test_tier1_r3_semantic_bm25_inverted_index`<br>`test_tier1_r3_semantic_dense_feature_hashing`<br>`test_tier1_r3_semantic_hybrid_rrf_scoring`<br>`test_tier1_r3_context_vfs_search_mounted_nodes`<br>`test_tier1_r3_session_store_error_pattern_lessons` | `test_tier2_r3_empty_query_string`<br>`test_tier2_r3_stopwords_only_query`<br>`test_tier2_r3_zero_token_and_extreme_token_docs`<br>`test_tier2_r3_dense_hasher_unicode_and_emojis`<br>`test_tier2_r3_search_limit_boundaries` | `test_tier3_semantic_memory_lookup_during_tui_paused_checkpoint`<br>`test_tier3_visual_verification_results_indexed_into_semantic_memory`<br>`test_tier4_workload3_session_crash_and_semantic_recovery` |
| **16-19** | Mock CDP & Visual Verification | R4 | `test_tier1_r4_mock_cdp_page_navigate_frame_id`<br>`test_tier1_r4_mock_cdp_dom_query_selector`<br>`test_tier1_r4_mock_cdp_capture_screenshot_base64`<br>`test_tier1_r4_synthetic_png_canvas_rfc1951`<br>`test_tier1_r4_visual_verification_gene_role` | `test_tier2_r4_navigate_malformed_url`<br>`test_tier2_r4_query_selector_zero_matches`<br>`test_tier2_r4_extreme_viewport_dimensions`<br>`test_tier2_r4_corrupted_truncated_dom`<br>`test_tier2_r4_png_chunk_crc32_checksum_mismatch` | `test_tier3_mock_cdp_visual_verification_in_worktree`<br>`test_tier3_visual_verification_results_indexed_into_semantic_memory`<br>`test_tier3_rollback_restoring_worktree_after_failed_visual_gate`<br>`test_tier4_workload2_web_ui_visual_regression_flow` |
| **20-23** | Ephemeral Git Worktrees | R5 | `test_tier1_r5_worktree_provision_under_managed_root`<br>`test_tier1_r5_worktree_environment_sanitization`<br>`test_tier1_r5_worktree_prp_gates_evaluation`<br>`test_tier1_r5_worktree_squash_merge_on_gate_success`<br>`test_tier1_r5_branch_and_pr_cli_management` | `test_tier2_r5_worktree_collision_existing_directory`<br>`test_tier2_r5_worktree_destination_outside_managed_root`<br>`test_tier2_r5_worktree_teardown_dirty_uncommitted`<br>`test_tier2_r5_prp_gate_failure_aborts_squash`<br>`test_tier2_r5_invalid_commit_sha_rejection` | `test_tier3_lsp_precheck_inside_ephemeral_worktree`<br>`test_tier3_mock_cdp_visual_verification_in_worktree`<br>`test_tier3_worktree_pr_creation_with_semantic_lesson_and_receipts`<br>`test_tier4_workload1_autonomous_goal_refactoring_with_lsp_and_squash` |
| **24-28** | Architecture & Invariants | R6 | `test_tier1_r6_forbid_unsafe_code_across_crates`<br>`test_tier1_r6_gnu_toolchain_and_gcc_compatibility`<br>`test_tier1_r6_python_embed_validation_scripts`<br>`test_tier1_r6_typescript_client_build_and_launcher`<br>`test_tier1_r6_in_memory_blaze_adapter_contract` | `test_tier2_r6_release_identity_version_mismatch_detected`<br>`test_tier2_r6_unfinished_documentation_markers_rejected`<br>`test_tier2_r6_unpinned_github_action_sha_rejected`<br>`test_tier2_r6_private_key_marker_rejected`<br>`test_tier2_r6_zero_network_isolation_boundary` | `test_tier3_blaze_incremental_compilation_with_lsp_diagnostics`<br>`test_tier3_worktree_pr_creation_with_semantic_lesson_and_receipts`<br>`test_tier4_workload4_multilingual_lsp_diagnostics_resolution` |

---

## 4. Test Files and Layout

```
crates/pandora-cli/tests/
├── fixtures/
│   └── e2e_support.rs           # Common test fixture, git workspace init, PNG validator, UNC normalizer
├── e2e_tier1_features.rs        # 31 Tier 1 isolated feature tests (R1 - R6)
├── e2e_tier2_boundaries.rs      # 30 Tier 2 boundary and corner case tests (R1 - R6)
├── e2e_tier3_combinations.rs    # 8 Tier 3 cross-feature combination tests (Pairwise)
└── e2e_tier4_scenarios.rs       # 5 Tier 4 real-world engineering workloads

tests/e2e/
├── README.md                    # Test suite overview and run commands
├── test_support.rs              # Standalone fixture and test helper functions
├── tier1_features.rs            # Tier 1 module declarations
├── tier2_boundaries.rs          # Tier 2 module declarations
├── tier3_combinations.rs        # Tier 3 module declarations
└── tier4_scenarios.rs           # Tier 4 module declarations
```

---

## 5. Execution Commands

### Prerequisites
All tests run with the portable GNU toolchain and embedded Python:
```powershell
$env:PATH = "C:\Users\infin\.gemini\antigravity\scratch\sandbox-workspace\python-embed;C:\Users\infin\.gemini\antigravity\scratch\sandbox-workspace\w64devkit\bin;" + $env:PATH
$env:RUSTUP_TOOLCHAIN = "stable-x86_64-pc-windows-gnu"
Set-Location "C:\Users\infin\.gemini\antigravity\scratch\sandbox-workspace\AGENT-PANDORA"
```

### Running Test Tiers Individually
```powershell
# Tier 1: Feature Coverage (31 tests)
cargo test -p pandora-cli --test e2e_tier1_features

# Tier 2: Boundary & Corner Cases (30 tests)
cargo test -p pandora-cli --test e2e_tier2_boundaries

# Tier 3: Cross-Feature Combinations (8 tests)
cargo test -p pandora-cli --test e2e_tier3_combinations

# Tier 4: Real-World Scenarios (5 workloads)
cargo test -p pandora-cli --test e2e_tier4_scenarios
```

### Running the Complete E2E Suite
```powershell
cargo test -p pandora-cli --test e2e_tier1_features --test e2e_tier2_boundaries --test e2e_tier3_combinations --test e2e_tier4_scenarios
```

### Repository Quality Gate Invariants
```powershell
python scripts/validate_docs.py
python scripts/validate_repo.py
python scripts/release_identity.py v3.0.1
```
All commands execute cleanly with exit code 0.
