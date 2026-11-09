# AGENT-PANDORA v3.0.10 Test Infrastructure & E2E Test Architecture

## 1. Test Architecture & Design Philosophy

The AGENT-PANDORA v3.0.10 End-to-End (E2E) testing infrastructure provides a robust, requirement-driven, opaque-box verification framework for the v3.0.10 release synchronization. It rigorously tests the three core capability pillars:
- **R1: Live Interactive Multi-Agent Autopilot Session** (Child CLI dispatch across 9 agents, Pexer IPC directives, SwarmCodeConsensusEngine Theorem 43 Byzantine Consensus, dynamic reputation updates, and cryptographic patch certification).
- **R2: Web / Mobile Remote Telemetry Dashboard Micro-Server** (Embedded zero-dependency safe Rust HTTP/1.1 & Server-Sent Events micro-server, responsive single-page dashboard calibrated for Xiaomi 15 mobile viewport and desktop displays, 9/12 panel SSE streaming, and interactive remote control actions).
- **R3: Phase 23 Federated Policy Engine & Dynamic Reputation** (Federated policy gradient optimization under Theorem 45 coordinate-wise trimmed mean, Beta-conjugate reputation tracking under Theorem 46, Thompson sampling, and CLI subcommands).

### Core Architectural Invariants
1. **Opaque-Box Boundary**: Tests interact strictly through public CLI entry points (`pandora cockpit reputation`, `pandora cockpit consensus`, `/autopilot`, etc.), published standard network protocol interfaces (HTTP/1.1 and EventSource RFC 8895 / WHATWG), and public library abstractions.
2. **Strict `#![forbid(unsafe_code)]`**: Every Rust test module enforces `#![forbid(unsafe_code)]` at line 1, honoring repository integrity.
3. **Zero Network / Air-Gapped Offline Guarantee**: No external network calls, remote CDNs, or unpinned external services are used. All network interactions run on local loopback sockets with synthetic self-contained fixtures.
4. **Isolated Sandboxed Execution**: Every test instantiates a dedicated `E2eFixture` directory with isolated temporary workspace, data, and configuration paths. Automatic RAII cleanup guarantees deterministic teardown.
5. **Deterministic Expected Outputs**: Every test derives expected outputs from specifications defined in `ORIGINAL_REQUEST.md` and `PROJECT.md`.

---

## 2. Four-Tier Test Case Design Methodology

The test suite strictly follows the 4-tier test case design methodology:

| Tier | Focus | Test Count | Description |
|---|---|---|---|
| **Tier 1** | Feature Coverage | **16 tests** | Happy-path isolated feature verification across R1, R2, and R3 (>=5 tests per feature). |
| **Tier 2** | Boundary & Corner Cases | **15 tests** | Edge conditions, zero norms, NaN clipping, oversized payloads (>64 KB), malformed diffs, rapid reconnects, sub-quorum handling, and quarantine decays (>=5 tests per feature). |
| **Tier 3** | Cross-Feature Combinations | **5 tests** | Pairwise subsystem interaction testing combining Web SSE triggers, Swarm Consensus, Autopilot task execution, and Federated Reputation tracking. |
| **Tier 4** | Real-World Application Scenarios | **4 workloads** | Full end-to-end multi-step engineering workloads simulating autonomous refactoring, Byzantine swarm quarantine & rehabilitation, mobile companion control loops, and multi-step policy convergence. |
| **Total** | | **40 tests** | Complete coverage across all v3.0.10 capabilities. |

---

## 3. Feature Inventory to Test Mapping

| Feature # | Capability | Requirement | Tier 1 Tests | Tier 2 Tests | Tier 3 & 4 Tests |
|---|---|---|---|---|---|
| **R1.1–R1.6** | Multi-Agent Autopilot Session | R1 | `test_tier1_r1_01_autopilot_goal_decomposition_and_roles`<br>`test_tier1_r1_02_autopilot_child_cli_dispatch_and_piping`<br>`test_tier1_r1_03_autopilot_swarm_consensus_resolution`<br>`test_tier1_r1_04_autopilot_reputation_update_on_consensus`<br>`test_tier1_r1_05_autopilot_patch_certification_receipt`<br>`test_tier1_r1_06_autopilot_cli_and_slash_dispatch` | `test_tier2_r1_01_autopilot_empty_or_whitespace_goal`<br>`test_tier2_r1_02_autopilot_subquorum_single_agent`<br>`test_tier2_r1_03_autopilot_all_byzantine_rejection`<br>`test_tier2_r1_04_autopilot_malformed_diff_injection`<br>`test_tier2_r1_05_autopilot_quarantined_agent_exclusion` | `test_tier3_01_web_action_triggers_consensus_and_updates_reputation`<br>`test_tier3_03_autopilot_certification_reflects_in_web_snapshot`<br>`test_tier3_04_permission_approval_unblocks_autopilot_flow`<br>`test_tier4_01_full_lifecycle_autonomous_mission`<br>`test_tier4_02_byzantine_swarm_quarantine_and_recovery` |
| **R2.1–R2.5** | Web / Mobile Telemetry Dashboard Micro-Server | R2 | `test_tier1_r2_01_web_microserver_startup_and_status`<br>`test_tier1_r2_02_responsive_xiaomi15_mobile_html`<br>`test_tier1_r2_03_realtime_sse_telemetry_stream`<br>`test_tier1_r2_04_interactive_layout_switch_action`<br>`test_tier1_r2_05_interactive_consensus_and_approve_actions` | `test_tier2_r2_01_web_rapid_reconnect_and_stream_close`<br>`test_tier2_r2_02_web_invalid_json_payload`<br>`test_tier2_r2_03_web_oversized_payload_protection`<br>`test_tier2_r2_04_web_unknown_endpoint_404`<br>`test_tier2_r2_05_web_port_conflict_handling` | `test_tier3_01_web_action_triggers_consensus_and_updates_reputation`<br>`test_tier3_02_layout_switch_during_live_sse_stream`<br>`test_tier3_03_autopilot_certification_reflects_in_web_snapshot`<br>`test_tier4_01_full_lifecycle_autonomous_mission`<br>`test_tier4_03_mobile_companion_interactive_control_loop` |
| **R3.1–R3.5** | Phase 23 Federated Policy Engine | R3 | `test_tier1_r3_01_policy_gradient_vector_primitives`<br>`test_tier1_r3_02_agent_reputation_record_beta_conjugate`<br>`test_tier1_r3_03_federated_trimmed_mean_aggregation`<br>`test_tier1_r3_04_federated_policy_step_and_receipt`<br>`test_tier1_r3_05_cli_reputation_status_all_9_agents` | `test_tier2_r3_01_gradient_zero_and_infinite_norms`<br>`test_tier2_r3_02_empty_gradient_set_error`<br>`test_tier2_r3_03_extreme_learning_rate_and_momentum`<br>`test_tier2_r3_04_quarantine_threshold_decay`<br>`test_tier2_r3_05_thompson_sampling_extremes` | `test_tier3_01_web_action_triggers_consensus_and_updates_reputation`<br>`test_tier3_05_federated_gradient_update_and_reputation_quarantine_sync`<br>`test_tier4_02_byzantine_swarm_quarantine_and_recovery`<br>`test_tier4_04_multi_step_federated_policy_convergence` |

---

## 4. Test Files and Layout

```
crates/pandora-cli/tests/
├── fixtures/
│   └── e2e_support.rs           # Common sandbox fixture, process runner, RAII teardown
└── e2e_v3010_tests.rs           # Complete 40-test 4-tier E2E suite for AGENT-PANDORA v3.0.10

tests/
└── e2e_v3010_tests.rs           # Repository root entrypoint linking to crates/pandora-cli/tests/
```

---

## 5. Execution Commands

### Environment Setup (Windows / GNU Toolchain)
```powershell
$env:PATH = "C:\Users\infin\.gemini\antigravity\scratch\sandbox-workspace\python-embed;C:\Users\infin\.gemini\antigravity\scratch\sandbox-workspace\w64devkit\bin;" + $env:PATH
$env:RUSTUP_TOOLCHAIN = "stable-x86_64-pc-windows-gnu"
Set-Location "C:\Users\infin\.gemini\antigravity\scratch\sandbox-workspace\AGENT-PANDORA"
```

### Running the Complete v3.0.10 E2E Test Suite
```powershell
cargo test --test e2e_v3010_tests
```

### Filtering Specific Tiers or Features
```powershell
# Run Tier 1 Feature Coverage
cargo test --test e2e_v3010_tests -- test_tier1

# Run Tier 2 Boundary Cases
cargo test --test e2e_v3010_tests -- test_tier2

# Run Tier 3 Combinations
cargo test --test e2e_v3010_tests -- test_tier3

# Run Tier 4 Real-World Scenarios
cargo test --test e2e_v3010_tests -- test_tier4
```
