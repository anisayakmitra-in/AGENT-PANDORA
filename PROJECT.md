# Project: AGENT-PANDORA v3.0.1 Advanced Capabilities

## Architecture
AGENT-PANDORA v3.0.1 is an autonomous software engineering agent running across 7 workspace crates:
- `pandora-types`: Core schemas, protocols, errors, URIs, and validation contracts.
- `pandora-runtime`: Execution engine, tool runtime, session store, ContextVfs, git executors, and agent loop.
- `pandora-harnesses`: Verification genes (Contract, Golden, Property, Guide, Visual) and test harnesses.
- `pandora-provider`: LLM providers and reasoning models.
- `pandora-service`: Daemon and background service management.
- `pandora-sandbox`: Isolated job execution (OS handle management under `#![deny(unsafe_code)]`).
- `pandora-cli`: Terminal UI, CLI commands, completions, and launchers.

Supported ecosystem bindings:
- `npm/pandora-cli`: TypeScript client and type definitions.
- `indexable-inc/blaze`: In-memory incremental compilation engine for accelerated build/verify cycles.
- C++ FFI: Native modules compiled with `w64devkit` GCC 16.2.0 under clean safe C ABI boundaries.

Platform & Toolchain Invariants:
- Rust toolchain: `stable-x86_64-pc-windows-gnu`
- MinGW GCC: `w64devkit` (GCC 16.2.0)
- Python embed: Portable Python 3.11.9 (`sandbox-workspace/python-embed/python.exe`)
- Active version: `3.0.1`
- `#![forbid(unsafe_code)]`: Enforced across all crates (`pandora-types`, `pandora-runtime`, `pandora-harnesses`, `pandora-provider`, `pandora-service`, `pandora-cli`).

---

## Feature Inventory
| # | Feature | Description | Milestone | Source |
|---|---------|-------------|-----------|--------|
| 1 | LSP Stdio Client | Lightweight JSON-RPC 2.0 client for rust-analyzer, tsserver, pyright, gopls | M1 | ORIGINAL_REQUEST §R1 |
| 2 | LSP Goto Definition | Symbol navigation via `textDocument/definition` | M1 | ORIGINAL_REQUEST §R1 |
| 3 | LSP Find References | Reference resolution via `textDocument/references` | M1 | ORIGINAL_REQUEST §R1 |
| 4 | LSP Type Definition | Type lookup via `textDocument/typeDefinition` | M1 | ORIGINAL_REQUEST §R1 |
| 5 | LSP Pre-test Diagnostics | Real-time compiler diagnostics extraction (`textDocument/publishDiagnostics`) | M1 | ORIGINAL_REQUEST §R1 |
| 6 | LSP CLI & Tool Engine | `pandora lsp` command, completion parity, and `lsp.*` tools in `ToolEngine` | M1 | ORIGINAL_REQUEST §R1 |
| 7 | TUI Execution Interruption | Non-destructive `Esc` or `Ctrl+K` handler during active tool executions | M2 | ORIGINAL_REQUEST §R2 |
| 8 | TUI In-flight Inspection | Modal dialog viewing active step, tool input, and execution timeline | M2 | ORIGINAL_REQUEST §R2 |
| 9 | TUI Guidance Injection | Interactive operator prompt injection into active agent context | M2 | ORIGINAL_REQUEST §R2 |
| 10 | TUI Step Skipping | Operator ability to bypass an in-flight tool step with synthetic feedback | M2 | ORIGINAL_REQUEST §R2 |
| 11 | TUI Clean Rollback | Safe cancellation restoring previous stable state without corrupting transcript | M2 | ORIGINAL_REQUEST §R2 |
| 12 | Pure-Rust Okapi BM25 Index | Zero-network sparse inverted index over `pandora://` ContextVfs | M3 | ORIGINAL_REQUEST §R3 |
| 13 | Local Dense Feature Hashing | Deterministic subword n-gram embedding vectorizer ($d=128$) | M3 | ORIGINAL_REQUEST §R3 |
| 14 | Hybrid RRF Search | Reciprocal Rank Fusion combining BM25 and dense cosine similarity | M3 | ORIGINAL_REQUEST §R3 |
| 15 | Error Pattern Lessons Index | Indexing and querying resolved error patterns (`eval-lesson-*`) and past sessions | M3 | ORIGINAL_REQUEST §R3 |
| 16 | Mock CDP JSON-RPC Server | Self-contained server implementing Page, DOM, Runtime, Input, Emulation | M4 | ORIGINAL_REQUEST §R4 |
| 17 | Virtual DOM Automation | DOM tree parsing, node selection, click, fill, evaluate in headless harness | M4 | ORIGINAL_REQUEST §R4 |
| 18 | Synthetic PNG Canvas | Pure safe Rust PNG screenshot encoder (Deflate + Adler32/CRC32, no external deps) | M4 | ORIGINAL_REQUEST §R4 |
| 19 | Visual Verification Gene | Playwright Wasm Gene integrating visual assertions into `VerificationGeneRole::Visual` | M4 | ORIGINAL_REQUEST §R4 |
| 20 | Worktree Isolation Manager | Ephemeral worktree manager at `.pandora/worktrees/<task-id>` for `/goal` | M5 | ORIGINAL_REQUEST §R5 |
| 21 | PRP Validation Pipeline | Automated Syntax, Unit, Integration, ReceiptCheck gate evaluation | M5 | ORIGINAL_REQUEST §R5 |
| 22 | Auto Squash-Merge & PR | Automatic branch squash-merge or PR branch creation upon gate completion | M5 | ORIGINAL_REQUEST §R5 |
| 23 | Branch & PR CLI & TUI | `pandora branch` and `pandora pr` subcommands, slash commands, shell completions | M5 | ORIGINAL_REQUEST §R5 |
| 24 | In-Memory Blaze Compiler | `indexable-inc/blaze` build acceleration engine wired to verify/build pipeline | M6 | ORIGINAL_REQUEST §R6 |
| 25 | TypeScript Client Validation | Build and test parity for `npm/pandora-cli` | M6 | ORIGINAL_REQUEST §R6 |
| 26 | C++ Integration & FFI | Native C++ modules compiled with GCC 16.2.0 with safe memory boundaries | M6 | ORIGINAL_REQUEST §R6 |
| 27 | Invariants & Quality Gates | Strict `#![forbid(unsafe_code)]`, GNU toolchain, Python embed, scripts passing | M6 | ORIGINAL_REQUEST §R6 |
| 28 | Full E2E Test Suite Pass | 100% pass across Tiers 1-4 opaque-box tests & Tier 5 adversarial hardening | M7 | ORIGINAL_REQUEST Acceptance |
| 29 | Rust Core Runtime | Core execution engine, tool runtime, sandbox, and governance engine | M6 | ORIGINAL_REQUEST Follow-up |
| 30 | Go Client SDK & Bindings | Native Go SDK/harness bindings for JSON-RPC/IPC communication | M6 | ORIGINAL_REQUEST Follow-up |
| 31 | TypeScript SDK & Bindings | Complete typing contracts and client bindings in `npm/pandora-cli` | M6 | ORIGINAL_REQUEST Follow-up |
| 32 | C++20 RAII Client Wrapper | C++20 header-only wrapper/client (`include/pandora.hpp` / `bindings/cpp/`) | M6 | ORIGINAL_REQUEST Follow-up |
| 33 | Stable C ABI FFI Header | Universal FFI export header (`include/pandora.h` / `bindings/c/`) | M6 | ORIGINAL_REQUEST Follow-up |
| 34 | Assembly SIMD Acceleration | Low-level architecture SIMD/CPUID vector acceleration for tokenization & hashing | M6 | ORIGINAL_REQUEST Follow-up |

---

## Milestones
| # | Name | Scope | Dependencies | Status |
|---|------|-------|-------------|--------|
| M1 | Language Server Protocol Engine | `pandora-runtime/src/lsp_engine.rs`, `pandora-cli/src/commands/lsp.rs`, tool registration, completions | None | DONE (worker_m1: 150+686+147 tests passed) |
| M2 | Steerable TUI Execution Loop | `pandora-cli/src/commands/tui.rs`, `SteerableRunControl`, modal steering UI, rollback | None | DONE (worker_m2: 153 unit + 43 E2E tests passed) |
| M3 | Semantic Memory & Hybrid RAG | `pandora-runtime/src/semantic_memory.rs`, BM25 + dense hashing, ContextVfs search | None | DONE (worker_m3: 686 tests passed) |
| M4 | Headless Browser & Mock CDP Engine | `pandora-runtime/src/mock_cdp.rs`, synthetic PNG canvas, `VerificationGeneRole::Visual` | None | DONE (worker_m4: 78+5 tests passed) |
| M5 | Ephemeral Git Worktree Isolation | `pandora-runtime/src/worktree_isolation.rs`, `/goal` isolation, squash-merge, `branch`/`pr` CLI | None | DONE (worker_m5: 154 CLI + 688 runtime + 6 integration tests passed) |
| M6 | Architecture & Invariant Enforcement | `blaze` acceleration, multi-language bindings (Rust, Go, TypeScript, C++, C, Assembly SIMD), gates validation | M1, M2, M3, M4, M5 | DONE (worker_m6: C/C++ GCC 16.2.0, TS build, cargo test workspace passed) |
| M7 | E2E Verification & Adversarial Hardening | Pass 100% E2E test suite (Tiers 1-4) & adversarial test hardening (Tier 5) | M1, M2, M3, M4, M5, M6 | DONE (111/111 E2E passed, auditor CLEAN, reviewer APPROVE) |

---

## Interface Contracts

### M1 (LSP Engine) ↔ Runtime & CLI
- Structs: `LspClientConfig`, `LspServerKind` (`RustAnalyzer`, `TsServer`, `Pyright`, `Gopls`), `LspDiagnostic`, `LspLocation`, `LspDefinitionResult`.
- Methods on `LspEngine`:
  - `start_server(kind: LspServerKind, root_path: &Path) -> Result<LspSession, LspError>`
  - `goto_definition(session: &LspSession, file: &Path, line: u32, col: u32) -> Result<Vec<LspLocation>, LspError>`
  - `find_references(session: &LspSession, file: &Path, line: u32, col: u32) -> Result<Vec<LspLocation>, LspError>`
  - `type_definition(session: &LspSession, file: &Path, line: u32, col: u32) -> Result<Vec<LspLocation>, LspError>`
  - `compiler_diagnostics(session: &LspSession, file: &Path) -> Result<Vec<LspDiagnostic>, LspError>`
- CLI Command: `pandora lsp [status|servers|definition|references|type-definition|diagnostics]`.

### M2 (Steerable TUI Loop) ↔ Runtime Agent Loop
- Structs: `SteerControlState` (`Running`, `Paused(Checkpoint)`, `Steering(SteerAction)`), `SteerAction` (`Resume`, `InjectGuidance(String)`, `SkipStep`, `Rollback`).
- Handler: `SteerableRunControl` implementing `AgentRunControl` hooks (`before_effect_authorization`, `after_effect`, `before_turn`).
- Key bindings: `Esc` or `Ctrl+K` during active tool execution invokes `pause()`, opens Steer Modal in TUI, and cleanly applies `SteerAction`.

### M3 (Semantic Memory / Hybrid RAG) ↔ ContextVfs & SessionStore
- Structs: `HybridSearchResult`, `SearchResultItem`, `SparseBm25Index`, `DenseFeatureHasher`.
- Signatures:
  - `ContextVfs::search(&self, query: &str, limit: usize) -> Result<Vec<SearchResultItem>, VfsError>`
  - `SessionStore::query_error_patterns(&self, error_pattern: &str, limit: usize) -> Result<Vec<LessonRecord>, SessionError>`
  - Combined scoring: $Score(d) = RRF(Rank_{BM25}(d), Rank_{Dense}(d))$.

### M4 (Mock CDP & Visual Verification) ↔ Harnesses & Runtime
- Structs: `MockCdpServer`, `CdpPage`, `CdpElement`, `SyntheticPngCanvas`, `VisualVerificationGene`.
- Protocol: JSON-RPC 2.0 handling methods `Page.navigate`, `Page.captureScreenshot`, `DOM.getDocument`, `DOM.querySelector`, `Runtime.evaluate`, `Input.dispatchMouseEvent`.
- PNG Output: Valid 8-bit RGBA PNG image bytes formatted with `IHDR`, `IDAT` (RFC 1951 Deflate), `IEND`.
- Harness trait: `VerificationGeneRole::Visual` asserting layout, DOM state, or screenshot hash.

### M5 (Ephemeral Git Worktrees) ↔ CLI & Runtime
- Structs: `WorktreeIsolationManager`, `WorktreeTaskContext`, `WorktreeMergeResult`, `PrBranchResult`.
- Signatures:
  - `WorktreeIsolationManager::provision_worktree(task_id: &str, base_commit: Option<&str>) -> Result<WorktreeTaskContext, WorktreeError>`
  - `WorktreeIsolationManager::evaluate_gates(ctx: &WorktreeTaskContext) -> Result<PrpValidationReport, WorktreeError>`
  - `WorktreeIsolationManager::finalize_task(ctx: &WorktreeTaskContext, action: FinalizeAction) -> Result<WorktreeMergeResult, WorktreeError>`
- CLI: `pandora branch [list|create|checkout|cleanup]` and `pandora pr [create|status]`.

### M6 (Architecture & Invariants) ↔ Acceleration & Compliance
- Structs: `BlazeCompilerAdapter`, `BlazeBuildResult`, `BlazeDiagnostics`.
- Integrations:
  - `workspace.verify` / `workspace.build` supports `compiler = "blaze"` hot in-memory rebuilds.
  - TypeScript client `npm/pandora-cli` passes `npm run build` and client tests.
  - Native C++ FFI module compiled with GCC 16.2.0.
  - `#![forbid(unsafe_code)]` preserved across all crates.
  - Scripts `validate_docs.py`, `validate_repo.py`, `release_identity.py v3.0.1` pass with code 0.

---

## Code Layout
- `crates/pandora-types/src/`:
  - `lib.rs`: `#![forbid(unsafe_code)]`
  - `lsp.rs`: LSP schemas and payload types
  - `visual.rs`: Visual verification types and CDP schemas
  - `worktree.rs`: Worktree isolation schemas
- `crates/pandora-runtime/src/`:
  - `lib.rs`: `#![forbid(unsafe_code)]`
  - `lsp_engine.rs`: JSON-RPC 2.0 LSP client implementation
  - `semantic_memory.rs`: BM25, dense hashing, and hybrid search
  - `mock_cdp.rs`: Synthetic CDP server and DOM tree
  - `synthetic_canvas.rs`: Pure safe Rust PNG screenshot generator
  - `worktree_isolation.rs`: Ephemeral worktrees, squash-merge, PR branch generation
  - `blaze_adapter.rs`: `indexable-inc/blaze` build acceleration binding
- `crates/pandora-harnesses/src/`:
  - `lib.rs`: `#![forbid(unsafe_code)]`
  - `verification.rs`: `VerificationGeneRole::Visual` implementation
- `crates/pandora-cli/src/`:
  - `commands/lsp.rs`: `pandora lsp` CLI commands
  - `commands/branch.rs`: `pandora branch` CLI command
  - `commands/pr.rs`: `pandora pr` CLI command
  - `commands/tui.rs`: Steerable execution loop, pause/checkpoint modal (`Esc`/`Ctrl+K`)
  - `commands/completions.rs`: Shell completions for new subcommands
- `npm/pandora-cli/`:
  - TypeScript client, type definitions, package build
- `tests/e2e/`:
  - E2E opaque-box test suites (Tiers 1-4) and adversarial suite (Tier 5)
