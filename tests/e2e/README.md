# AGENT-PANDORA v3.0.2 E2E Test Suite

## Overview

The AGENT-PANDORA v3.0.2 End-to-End (E2E) test suite implements comprehensive, requirement-driven, opaque-box testing across the six core capability pillars:
- **R1: Language Server Protocol (LSP) Engine (`pandora lsp`)**
- **R2: Steerable & Interruptible Execution Loop in the TUI**
- **R3: Semantic Memory & Hybrid RAG (`pandora://` ContextVfs)**
- **R4: Headless Browser & Visual Verification Engine (CDP / Playwright Wasm Gene)**
- **R5: Ephemeral Git Worktree Isolation (`pandora branch / pr`)**
- **R6: Architecture & Invariant Constraints**

## Test Hierarchy & Methodology

The test suite strictly follows the 4-tier test case design methodology:

| Tier | Focus | Test Count | Description |
|---|---|---|---|
| **Tier 1** | Feature Coverage | 31 tests | Happy-path isolated feature verification across R1–R6 (>=5 tests per feature). |
| **Tier 2** | Boundary & Corner Cases | 30 tests | Edge conditions, malformed inputs, timeouts, process crashes, and limits across R1–R6 (>=5 tests per feature). |
| **Tier 3** | Cross-Feature Combinations | 8 tests | Pairwise interaction testing combining subsystems (e.g. LSP inside worktrees, memory lookup while paused). |
| **Tier 4** | Real-World Application Scenarios | 5 workloads | Full end-to-end multi-step engineering workloads simulating autonomous refactoring, visual regression, crash recovery, and PR export. |
| **Total** | | **74 tests** | Complete coverage across all 28 feature inventory items. |

## Execution Commands

All test suites are automated through Cargo integration test targets under `crates/pandora-cli/tests/`:

```powershell
# Setup toolchain environment
$env:PATH = "C:\Users\infin\.gemini\antigravity\scratch\sandbox-workspace\python-embed;C:\Users\infin\.gemini\antigravity\scratch\sandbox-workspace\w64devkit\bin;" + $env:PATH
$env:RUSTUP_TOOLCHAIN = "stable-x86_64-pc-windows-gnu"

# Run all Tier 1 Feature Coverage tests
cargo test -p pandora-cli --test e2e_tier1_features

# Run all Tier 2 Boundary & Corner Case tests
cargo test -p pandora-cli --test e2e_tier2_boundaries

# Run all Tier 3 Cross-Feature Combination tests
cargo test -p pandora-cli --test e2e_tier3_combinations

# Run all Tier 4 Real-World Scenario workloads
cargo test -p pandora-cli --test e2e_tier4_scenarios

# Run the complete E2E test suite across all 4 tiers
cargo test -p pandora-cli --test e2e_tier1_features --test e2e_tier2_boundaries --test e2e_tier3_combinations --test e2e_tier4_scenarios
```

## Architectural Invariants
- **`#![forbid(unsafe_code)]`**: Strictly maintained across all test files.
- **Zero Network**: Tests run self-contained and offline, using local synthetic servers and SQLite stores.
- **Deterministic Cleanup**: Every test fixture operates in an isolated temporary sandbox with automatic RAII teardown.
