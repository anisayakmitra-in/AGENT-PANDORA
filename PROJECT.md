# Project: AGENT-PANDORA (v4.1.4 Release: THANATOS - Non-Hermitian PT-Topology, Entangled MIP* & Port-Hamiltonian Consensus)

## Architecture
AGENT-PANDORA is an autonomous software engineering agent across 7 workspace crates:
- `pandora-types` (`#![forbid(unsafe_code)]`): Core schemas, protocols, errors, URIs, distillation/telemetry contracts, and service RPC types.
- `pandora-runtime` (`#![forbid(unsafe_code)]`): Execution engine (`AgentLoop`), `LatsPrefixMemoCache`, `MoiraDecisionDriver` / `CalibrationAttestationReceipt`, `SinkhornDriftDetector` (Phase 13), `MartingaleCanaryGate` (Phase 14), `ContextVfs` (`PVFS` v3), `HnswIndex` (`PVHN` v1 / tropical HNSW), `CockpitController`, `CockpitWebServer`, and `RuntimeService`.
- `pandora-harnesses` (`#![forbid(unsafe_code)]`): Verification genes, `DecisionBenchmarkSuite`, and `BayesianCanaryGate`.
- `pandora-provider` (`#![forbid(unsafe_code)]`): LLM providers and reasoning models.
- `pandora-service` (`#![forbid(unsafe_code)]`): Daemon and background service HTTP/RPC management.
- `pandora-sandbox`: Isolated job execution (helper uses Windows Job Objects; library is safe).
- `pandora-cli` (`#![forbid(unsafe_code)]`): Terminal UI (`tui.rs`, `cockpit_tui.rs`), CLI commands (`moira`, `fleet`, `cockpit`, `run`, `memory`, `code`, `swarm`, `sandbox`, `agent`), completions, and launchers.

Ecosystem bindings & docs:
- `npm/pandora-cli`: TypeScript client (`PandoraClient`) and compiled definitions in `npm/pandora-cli/lib/`.
- `docs/`: CLI reference (`docs/CLI_COMMANDS_REFERENCE.md`), TypeScript SDK docs (`docs/TYPESCRIPT.md`), math specs (`docs/math/phase13_wasserstein_drift.md`, `docs/math/phase14_martingale_canary_gate.md`).

Platform, Toolchain & Delivery Invariants:
- Rust toolchain: `1.97.1` (`rust-toolchain.toml`, `stable-x86_64-pc-windows-gnu`, MinGW GCC in `w64devkit/bin`)
- Node: `v22.23.2` (`npm.cmd ci --ignore-scripts && npm.cmd run build`)
- Python embed: `C:\Users\infin\.gemini\antigravity\scratch\sandbox-workspace\python-embed\python.exe` (for `scripts/validate_repo.py` and `scripts/validate_docs.py`)
- `#![forbid(unsafe_code)]`: Preserved across `pandora-types`, `pandora-runtime`, `pandora-harnesses`, `pandora-provider`, `pandora-service`, `pandora-cli`.
- **Versioning & Release**: Workspace version promoted to `4.1.4` (Phases 70–72, Theorems 182–190).
- **Zero Test Source Reading Rule**: Team members must NOT read test source code to understand expected behaviour before implementing.

---

## Feature Inventory
Every feature from Survey & `ORIGINAL_REQUEST.md` is assigned to a milestone:

| # | Feature | Description | Milestone | Source |
|---|---------|-------------|-----------|--------|
| F1 | `v3.1.5` CI & Release Verification | Verify GitHub Actions `CI` (`45367206163`) & `Release` (`45367215407`) runs and published `v3.1.5` binaries/checksums (report exact blocking/failure status without modifying remote `v3.1.5`) | M1 | ORIGINAL_REQUEST §R1 |
| F2 | `verify-attestation` Missing-PNN Fix | Fix `crates/pandora-cli/src/commands/moira.rs` so missing `.pnn` file does not falsely report `pnn_digest_matches: true` | M1 | ORIGINAL_REQUEST §R1 / Survey 1 |
| F3 | TypeScript SDK Telemetry Parity | Sync `npm/pandora-cli/src/client.ts` (`LatsTurnTelemetry`, `LatsMetrics`, `modelVerifyAttestation`) and rebuild `npm/pandora-cli/lib/` | M1 | ORIGINAL_REQUEST §R1 / Survey 1 |
| F4 | Reference Docs Synchronization | Sync `docs/CLI_COMMANDS_REFERENCE.md` and `docs/TYPESCRIPT.md` with `verify-attestation`, `modelVerifyAttestation`, `with_auto_calibrate`, `LatsPrefixMemoCache`, `PandoraClient`, `runWithLats`, `memoryVfsTropicalSearch` | M1 | ORIGINAL_REQUEST §R1 |
| F5 | Distillation & Telemetry Contracts | Add `DistillationTelemetrySnapshot`, `WassersteinTelemetrySummary`, `MartingaleTelemetrySummary`, and `ServiceRequest`/`ServiceResponse` RPC contracts in `pandora-types` | M2 | ORIGINAL_REQUEST §R2 |
| F6 | Cross-Session DPO Aggregation | Discover and aggregate `.pandora/dpo_pairs.jsonl` across local defaults and explicitly configured session paths with deterministic deduplication and digesting | M2 | ORIGINAL_REQUEST §R2 |
| F7 | Signed `MemoryCapsule` & `.pnn` Distillation | Calibrate `.pnn` from aggregated DPO pairs, gate via Phase 13 `SinkhornDriftDetector` + Phase 14 `MartingaleCanaryGate` + `BayesianCanaryGate`, and emit Ed25519-signed `MemoryCapsule` (`SignedCapsulePackage`) + `.pnn` | M2 | ORIGINAL_REQUEST §R2 |
| F8 | Signed `CalibrationAttestationReceipt` & Tamper Rejection | Extend `CalibrationAttestationReceipt` with Ed25519 signature, capsule digest binding, and Phase 13/14 metrics; reject invalid/tampered/unsigned/mismatched receipts | M2 | ORIGINAL_REQUEST §R2 |
| F9 | Runtime & Service Telemetry Wiring | Update `AgentLoop` (`check_control` telemetry + snapshot persistence), `CockpitController` (live telemetry replacing mock strings), `CockpitWebServer`, and `RuntimeService` / `pandora-service` | M3 | ORIGINAL_REQUEST §R2 |
| F10 | CLI, Fleet, Cockpit & TUI Integration | Add `distill` and `verify-attestation` to `pandora fleet` & `pandora cockpit`; surface `pnn_hot_reloads`, `lats_memo_hits`, `active_pnn_digest`, Phase 13/14 metrics across `fleet dashboard`, `cockpit status`, `cockpit_tui`, main `tui`, `run`, and completions | M4 | ORIGINAL_REQUEST §R2 |
| F11 | `HnswIndex` Tombstones, Incremental Updates & Compaction | Add `delete`/`mark_deleted`, `doc_id -> node_id` map, `update`/`upsert`, `compact()` with entry_point & neighbor repair, tombstone-safe traversal, and `PVHN` compatibility | M5 | ORIGINAL_REQUEST §R3 |
| F12 | `PVFS` v3 Tombstone Compaction & Incremental HNSW Sync | Add `ContextVfs` deletion/tombstone APIs, `PVFS` v3 tombstone persistence & automated threshold-triggered compaction, and incremental HNSW sync with post-compaction query equivalence | M5 | ORIGINAL_REQUEST §R3 |
| F13 | `LatsPrefixMemoCache` & Tropical HNSW Benchmarks | Add reproducible offline `--locked` pure-`std` `[[bench]]` target (`cargo bench -p pandora-runtime --locked`) and benchmark helpers reporting throughput for `LatsPrefixMemoCache` and tropical HNSW | M5 | ORIGINAL_REQUEST §R3 |
| F14 | Version & Quality Gate Compliance | Maintain `3.1.5` workspace version, `#![forbid(unsafe_code)]`, `fmt`, `clippy`, `cargo test --workspace --locked`, `validate_docs.py`, `validate_repo.py` | M6 | ORIGINAL_REQUEST §R4 |
| F15 | Opaque-Box E2E Suite & Tier 5 Hardening | Independent 4-tier E2E test suite (`TEST_READY.md`), 100% pass gate, and Tier 5 adversarial coverage hardening | E2E + M6 | Project Pattern |
| F16 | Phase 29: Swarm PBFT Consensus State Machine | 3-phase Byzantine Fault-Tolerant consensus ($N \ge 3f + 1$, Theorem 62) with view-change rotation (Theorem 63), Merkle sync (Theorem 64), Ed25519 signing, CLI `pandora swarm bft`, REST `/api/swarm/bft` | M7 | v4.0.0 Architecture |
| F17 | Phase 30: Visual Perception & Layout Verification | WCAG 2.2 luminance & contrast ratio validation (Theorem 66), perceptual hash $p\text{Hash}$ invariance (Theorem 65), layout bounding box IoU & overlap verification (Theorem 67), CLI `pandora harness visual`, REST `/api/harness/visual` | M8 | v4.0.0 Architecture |
| F18 | Phase 31: Grammar-Constrained Token-Level BNF Synthesis | Pushdown automata token logit masking (JSON, SQL, Rust, Python subsets), Theorems 68–70, CLI `pandora code synthesize`, `pandora code grammar mask/validate`, REST `/api/code/synthesize/grammar` | M9 | v4.0.1 Architecture |
| F19 | Phase 32: Decentralized Swarm CRDT & Epidemic Gossip | PN-Counter, OR-Set, Merkle DAG CRDT, Vector Clock causal precedence (Theorem 71–73), $O(\log N)$ gossip dissemination, CLI `pandora swarm crdt`, `pandora swarm gossip`, REST `/api/swarm/crdt` | M10 | v4.0.1 Architecture |
| F20 | Phase 33: Autonomous Untrusted Sandbox Isolation | Deterministic process tree containment, non-interference safety, $\le 50\text{ ms}$ kill bound, resource rooflines (Theorems 74–76), CLI `pandora sandbox status/execute/audit`, REST `/api/sandbox/telemetry` | M11 | v4.0.2 Architecture |
| F21 | Phase 34: Post-Quantum Hybrid Attestation | Poseidon algebraic sponge over Goldilocks prime field, Ed25519 + ML-DSA-65 lattice signatures (Theorems 77–79), CLI `pandora moira pq-sign/pq-verify`, REST `/api/moira/pq` | M12 | v4.0.2 Architecture |
| F22 | Phase 35: Continuous Introspective Metacognition | Shannon predictive entropy gating, monotonic backtracking convergence, PAC-Bayes generalization risk $\le 0.15$ (Theorems 80–82), CLI `pandora agent introspect/reflect`, REST `/api/agent/introspect` | M13 | v4.0.2 Architecture |
| F23 | Phase 36: Homomorphic Multi-Party Computation & Secret Sharing | Shamir (k, n)-threshold secret sharing, Byzantine reconstruction, Beaver triple homomorphic multiplication (Theorems 83–85), CLI `pandora swarm secret-share/mpc-eval`, REST `/api/swarm/mpc` | M14 | v4.0.3 Architecture |
| F24 | Phase 37: Differential Privacy & Continual Certified Unlearning | Rényi DP composition, Fisher projected Newton-step scrubbing, Lipschitz memory stability (Theorems 86–88), CLI `pandora memory scrub`, REST `/api/memory/privacy` | M15 | v4.0.3 Architecture |
| F25 | Phase 38: Distributed Speculative Tree Decoding | Rejection sampling distributional equivalence, optimal tree draft speedup >= 2.0x, amortized latency (Theorems 89–91), CLI `pandora code speculate`, REST `/api/code/speculative` | M16 | v4.0.3 Architecture |
| F26 | Phase 39: THANATOS Micro-Kernel Subsystems Topology | 5-Pillar Architecture (Cognitive, Governance, Crypto, Swarm, Memory), unified CLI `pandora cognitive/governance/crypto`, REST `/api/subsystems/*`, `/api/olympus/overview` | M17 | v4.0.4 Architecture |
| F27 | Phase 40: Distributed Multi-Tenant Mesh Orchestration & Cross-Cluster Atomic Commitments | Two-phase commit protocol, shard attestation vector digests, bounded message complexity (Theorems 92–94), CLI `pandora swarm mesh/cross-commit`, REST `/api/swarm/mesh` | M18 | v4.0.5 Architecture |
| F28 | Phase 41: Hyper-Dimensional Vector VFS & Holographic Cognitive Associative Memory | 1024-D bipolar hypervectors $\mathbb{H}^{1024}$, circular convolution binding, associative unbinding, capacity scaling (Theorems 95–97), CLI `pandora memory hyper-project/holographic-recall`, REST `/api/memory/hyper` | M18 | v4.0.5 Architecture |
| F29 | Phase 42: Formal Kernel Verification & Lean 4 / Coq Machine-Checked Proof Exports | Soundness bijection, decidability, centennial completeness for Theorems 1–100 (Theorems 98–100), CLI `pandora audit formal-verify`, REST `/api/audit/formal` | M18 | v4.0.5 Architecture |
| F30 | Phase 43: Recursive Zero-Knowledge Rollups & Algebraic STARK Proofs | Constant-size STARK proof aggregation, BabyBear prime field $\mathbb{F}_p$, 128-bit computational soundness (Theorems 101–103), CLI `pandora crypto zk-rollup/zk-verify`, REST `/api/crypto/rollup` | M19 | v4.0.6 Architecture |
| F31 | Phase 44: Autonomous Self-Healing Kernel & Dynamic Invariant Synthesis | Monotonic bisimulation equivalence, automated invariant re-derivation, zero-downtime hot-patching (Theorems 104–106), CLI `pandora governance heal/invariant-check`, REST `/api/governance/healing` | M19 | v4.0.6 Architecture |
| F32 | Phase 45: Neuromorphic Spiking Context VFS & Temporal Memory Decay | Leaky Integrate-and-Fire dynamics, STDP synaptic plasticity, exponential decay retention fidelity >= 95% (Theorems 107–109), CLI `pandora memory spike-train/temporal-decay`, REST `/api/memory/neuromorphic` | M19 | v4.0.6 Architecture |
| F33 | Phase 46: Quantum Annealing Graph Partitioning & QUBO Job Scheduling | Transverse-field Ising Hamiltonian mapping, spectral gap convergence, makespan bound (Theorems 110–112), CLI `pandora swarm anneal/qubo-schedule`, REST `/api/swarm/anneal` | M20 | v4.0.7 Architecture |
| F34 | Phase 47: Homomorphic Federated Model Distillation & Secure Aggregation | IND-CPA security, Byzantine poisoning rejection, zero mutual information leak (Theorems 113–115), CLI `pandora crypto fed-aggregate/fed-verify`, REST `/api/crypto/federated` | M20 | v4.0.7 Architecture |
| F35 | Phase 48: Causal Counterfactual Policy Inference & Pearlian Do-Calculus Engine | Backdoor intervention identifiability, monotonic counterfactual consistency, confounder invariance (Theorems 116–118), CLI `pandora cognitive do-calculus/counterfactual`, REST `/api/cognitive/causal` | M20 | v4.0.7 Architecture |
| F36 | Phase 49: Topological Quantum Error Correction & Surface Code Stabilizers | 2D surface code distance $d = 2t + 1$, 10.9% threshold, $[A_s, B_p] = 0$ non-destructive measurement, MWPM decoding (Theorems 119–121), CLI `pandora swarm qec-syndrome/qec-decode`, REST `/api/swarm/qec` | M21 | v4.0.8 Architecture |
| F37 | Phase 50: Zero-Knowledge PlonK Polynomial Commitments & KZG Schemes | $q$-SDH computational binding, Schwartz-Zippel bound $\le d/|S|$, constant-time $O(1)$ pairing verifier (Theorems 122–124), CLI `pandora crypto kzg-commit/kzg-verify`, REST `/api/crypto/kzg` | M21 | v4.0.8 Architecture |
| F38 | Phase 51: Multi-Agent Mechanism Design & VCG Truthful Equilibrium | DSIC social welfare maximization, polymatrix correlated equilibrium, axiomatic unique Shapley credit (Theorems 125–127), CLI `pandora cognitive vcg-auction/shapley-credit`, REST `/api/cognitive/mechanism` | M21 | v4.0.8 Architecture |
| F39 | Phase 52: Holographic Quantum Error Suppression & Fault-Tolerant Magic State Distillation | Bravyi-Kitaev 15-to-1 cubic suppression, transversal T-gate fidelity >= 99.99%, yield bound (Theorems 128–130), CLI `pandora swarm magic-distill/magic-state-audit`, REST `/api/swarm/magic-state` | M22 | v4.0.9 Architecture |
| F40 | Phase 53: Fully Homomorphic Encryption (FHE) with CKKS Leveled Arithmetic | Complex canonical embedding isometry, modulus switching SNR >= 80 dB, 128-bit RLWE security (Theorems 131–133), CLI `pandora crypto ckks-encrypt/ckks-rescale`, REST `/api/crypto/ckks` | M22 | v4.0.9 Architecture |
| F41 | Phase 54: Mean-Field Stochastic Differential Games & McKean-Vlasov Swarm Control | McKean-Vlasov continuum convergence O(1/sqrt(N)), HJB viscosity solution, Pareto decentralized control (Theorems 134–136), CLI `pandora cognitive mckean-vlasov/swarm-hjb`, REST `/api/cognitive/mean-field` | M22 | v4.0.9 Architecture |

---

## Milestones

| # | Name | Scope | Dependencies | Status |
|---|------|-------|-------------|--------|
| M1 | `v3.1.5` Release Verification, TS SDK & Docs (R1) | Re-verify `CI`/`Release` runs & `v3.1.5` artifacts; fix `moira.rs` `verify-attestation`; sync `npm/pandora-cli/src/client.ts` + `lib/`; sync `docs/CLI_COMMANDS_REFERENCE.md` & `docs/TYPESCRIPT.md` | None | COMPLETED |
| M2 | Cross-Session Distillation & Signed Attestation Core (R2) | `pandora-types` telemetry contracts; `pandora-runtime/src/distillation.rs`; `CalibrationAttestationReceipt` Ed25519 signing + tamper rejection; `lib.rs` exports | None | COMPLETED |
| M3 | Runtime, Service & Cockpit Controller Telemetry Wiring (R2) | `pandora-types/src/service.rs`; `AgentLoop` telemetry/persistence; `CockpitController` live Phase 13/14 + reload/memo metrics; `CockpitWebServer` rendering; `RuntimeService` & `pandora-service` handlers | M2 | COMPLETED |
| M4 | Fleet, Cockpit, Run & TUI CLI Integration (R2 + R3 CLI) | `pandora fleet distill/verify-attestation/dashboard`, `pandora cockpit distill/verify-attestation/status`, `cockpit_tui`, `tui`, `run` flags/metrics, `memory vfs` compaction/bench CLI (if exposed), `mod.rs` & `completions.rs` | M1, M2, M3, M5 | COMPLETED |
| M5 | `PVFS` v3 Compaction, Incremental HNSW & Benchmarks (R3) | `hnsw.rs` tombstones/incremental/compact; `context_vfs.rs` PVFS v3 deletion/compaction/incremental HNSW sync; `[[bench]]` pure-`std` throughput benchmarks for `LatsPrefixMemoCache` & tropical HNSW | None | COMPLETED |
| E2E | E2E Testing Track (Tiers 1-4) | Requirement-driven opaque-box E2E test suite covering F1–F14, publishing `TEST_INFRA.md` & `TEST_READY.md` | None (runs parallel) | COMPLETED |
| M6 | Final Milestone: E2E Pass & Quality Verification | 100% E2E pass on 52 tests; reproducible pure-std throughput benchmarks; full fmt/clippy/test/doc/repo validation gates; version/commit invariants preserved | M1, M2, M3, M4, M5, E2E | COMPLETED |
| M7 | Phase 29: Swarm PBFT Consensus State Machine | Byzantine Fault-Tolerant consensus ($N \ge 3f + 1$, Theorem 62–64), Ed25519 cluster quorum, CLI `swarm bft`, Cockpit `/api/swarm/bft` | M6 | COMPLETED |
| M8 | Phase 30: Visual Perception & Layout Harness | WCAG 2.2 color contrast, $p\text{Hash}$ invariance, layout bounding box IoU & overlap verification (Theorem 65–67), CLI `harness visual`, Cockpit `/api/harness/visual` | M6 | COMPLETED |
| M9 | Phase 31: Grammar-Constrained Token-Level BNF Synthesis | Pushdown automata token masking, zero syntax errors by construction (Theorems 68–70), CLI `code synthesize/grammar`, Cockpit `/api/code/synthesize/grammar` | M8 | COMPLETED |
| M10 | Phase 32: Swarm CRDT & Epidemic Gossip Synchronization | Monotonic join-semilattices (Theorem 71), $O(\log N)$ gossip dissemination (Theorem 72), causal vector clocks (Theorem 73), CLI `swarm crdt/gossip`, Cockpit `/api/swarm/crdt` | M9 | COMPLETED |
| M11 | Phase 33: Autonomous Untrusted Sandbox Isolation | Job objects/cgroups containment engine, deterministic kill bound, resource quotas (Theorems 74–76) | M10 | COMPLETED |
| M12 | Phase 34: Post-Quantum Hybrid Attestation | Goldilocks Poseidon sponge, dual classical + lattice ML-DSA-65 attestation (Theorems 77–79) | M11 | COMPLETED |
| M13 | Phase 35: Continuous Introspective Metacognition | Predictive entropy gating, monotonic backtracking convergence, PAC-Bayes bound (Theorems 80–82) | M12 | COMPLETED |
| M14 | Phase 36: Homomorphic Multi-Party Computation & Secret Sharing | Shamir threshold sharing, Beaver triple multiplication (Theorems 83–85) | M13 | COMPLETED |
| M15 | Phase 37: Differential Privacy & Continual Certified Unlearning | RDP accounting, certified unlearning distance bound (Theorems 86–88) | M14 | COMPLETED |
| M16 | Phase 38: Distributed Speculative Tree Decoding | Speculative rejection sampling, tree decoding speedup >= 2.0x (Theorems 89–91) | M15 | COMPLETED |
| M17 | Phase 39: THANATOS Subsystems Topology (v4.0.4) | 5 micro-kernel pillars, unified CLI commands, REST endpoints, 91 theorems | M16 | COMPLETED |
| M18 | Phase 40-42: Centennial Completeness (v4.0.5) | Multi-tenant mesh, hyper-dimensional holographic VFS, formal Lean 4/Coq verification (Theorems 92–100) | M17 | COMPLETED |
| M19 | Phase 43-45: Advanced Autonomy Substrate (v4.0.6) | Recursive ZK rollups, self-healing kernel, neuromorphic spiking VFS (Theorems 101–109) | M18 | COMPLETED |
| M20 | Phase 46-48: Quantum, Federated & Causal Substrate (v4.0.7) | Quantum annealing scheduling, federated homomorphic aggregation, Pearlian do-calculus (Theorems 110–118) | M19 | COMPLETED |
| M21 | Phase 49-51: QEC, PlonK KZG & Mechanism Design (v4.0.8) | Surface code QEC, KZG polynomial commitments, VCG mechanisms & Shapley credit (Theorems 119–127) | M20 | COMPLETED |
| M22 | Phase 52-54: Magic State Distillation, CKKS FHE & Mean-Field Control (v4.0.9) | Magic state distillation, CKKS approximate FHE, McKean-Vlasov mean-field control (Theorems 128–136) | M21 | COMPLETED |

---

## Interface Contracts

### M2 (`pandora-types` & `pandora-runtime::distillation`) ↔ M3 & M4
- `pandora_types::distillation`:
  - `DistillationTelemetrySnapshot`:
    - `pnn_hot_reloads: u32`
    - `lats_memo_hits: u64`
    - `active_pnn_digest: Option<String>`
    - `dpo_pairs_total: usize`
    - `last_capsule_id: Option<String>`
    - `last_capsule_digest: Option<String>`
    - `last_attestation_commitment: Option<String>`
    - `last_attestation_verified: bool`
    - `wasserstein_drift: Option<WassersteinTelemetrySummary>` (`sinkhorn_divergence`, `wasserstein_2_distance`, `birkhoff_contraction_factor`, `theoretical_entropic_bias_bound`, `drift_threshold`, `drift_detected`, `iterations`, `final_dual_residual`, `marginal_conservation_error`)
    - `martingale_canary: Option<MartingaleTelemetrySummary>` (`step_t`, `e_value`, `peak_e_value`, `anytime_p_value`, `ville_rejection_threshold`, `empirical_mean`, `confidence_radius`, `lower_confidence_bound`, `upper_confidence_bound`, `betting_fraction_lambda`, `verdict`)
    - `updated_at_unix: u64`
  - `DistillationTelemetrySnapshot::load_from_file(path)` / `save_to_file(path)` (atomic write via `atomic-write-file` in runtime helper or `serde_json`).
- `pandora_runtime::decision_engine::CalibrationAttestationReceipt`:
  - Backward-compatible `#[serde(default)]` fields: `capsule_digest: Option<String>`, `signer_public_key: Option<String>`, `signature: Option<String>`, `sinkhorn_divergence: Option<f64>`, `wasserstein_2_distance: Option<f64>`, `martingale_e_value: Option<f64>`, `martingale_verdict: Option<String>`.
  - Methods: `sign_ed25519(&mut self, signing_key: &SigningKey)`, `verify_ed25519(&self, expected_pubkey_hex: Option<&str>) -> bool`, `verify_signed_chain(receipts: &[Self], expected_pubkey_hex: Option<&str>) -> bool`.
- `pandora_runtime::distillation`:
  - `discover_dpo_sources(workspace: &Path, data_dir: Option<&Path>, explicit_paths: &[PathBuf]) -> Vec<PathBuf>`
  - `aggregate_dpo_pairs(paths: &[PathBuf]) -> Result<AggregatedDpoDataset, DistillationError>`
  - `distill_cross_session_checkpoint(config: CrossSessionDistillConfig) -> Result<CrossSessionDistillResult, DistillationError>`
  - `verify_pnn_and_capsule_attestation(config: AttestationVerifyConfig) -> Result<AttestationVerificationReport, DistillationError>`
  - `default_telemetry_path(data_dir: &Path) -> PathBuf` (`<data_dir>/cockpit_telemetry.json`), `load_telemetry_snapshot(path: &Path)`, `save_telemetry_snapshot(path: &Path, snap: &DistillationTelemetrySnapshot)`.

### M3 (`CockpitController`, `AgentLoop`, `RuntimeService`) ↔ M4 (`CLI`, `TUI`)
- `CockpitController`:
  - `with_telemetry_path(layout, telemetry_path)` / `capture_snapshot_with_telemetry(timestamp_ms, telemetry: Option<&DistillationTelemetrySnapshot>)`
  - Surfaces `pnn_hot_reloads`, `lats_memo_hits`, `active_pnn_digest`, Phase 13 Sinkhorn/Wasserstein metrics, and Phase 14 Martingale metrics in `PanelKind::GenesEvolution` & `PanelKind::DriftConformal`.
- `pandora-cli` commands:
  - `pandora fleet distill [--session-path <dir|file>]... [--pnn <path>] [--capsule-out <path>] [--key-file <path>] [--drift-threshold <f64>] [--alpha <f64>] [--dry-run]`
  - `pandora fleet verify-attestation [--pnn <path>] [--capsule <path>] [--pubkey <hex>] [--require-signature]`
  - `pandora cockpit distill ...` & `pandora cockpit verify-attestation ...` (consistent JSON/text output contract with `fleet`).
  - `pandora run`: `--auto-calibrate <k>`, `--dpo-output <path>`, JSON `lats_metrics` includes `pnn_hot_reloads`, `active_pnn_digest`, `lats_memo_hits` (plus `memo_hits`).

### M5 (`PVFS` v3 & `HnswIndex`) ↔ M4 & Benchmarks
- `HnswIndex`:
  - `mark_deleted(&mut self, doc_id: usize) -> bool`
  - `is_deleted(&self, doc_id: usize) -> bool`
  - `upsert(&mut self, doc_id: usize, vector: HnswVector) -> bool`
  - `compact(&mut self) -> HnswCompactionReport` (re-packs live nodes, remaps `doc_id -> node_id`, repairs `entry_point` and bidirectional neighbors)
  - `tombstone_count(&self) -> usize`, `live_count(&self) -> usize`
- `ContextVfs`:
  - `remove_document(&self, uri: &str) -> bool` / `tombstone_document(&self, uri: &str) -> bool`
  - `compact_index(&self) -> Result<VfsCompactionReport, VfsError>`
  - `with_auto_compact_threshold` / automatic compaction trigger when tombstones exceed configured threshold/ratio on `save_index_to_disk` / mutation
  - Incremental HNSW synchronization (`sync_hnsw_incremental` / `update_hnsw_on_mutation`) preserving post-compaction search equivalence/recall.
- `crates/pandora-runtime/benches/r3_throughput_bench.rs`:
  - Pure-`std` (`Instant`, `black_box`) `harness = false` benchmark executable via `cargo bench -p pandora-runtime --locked` measuring `LatsPrefixMemoCache` (hits, misses, eviction ops/sec) and tropical HNSW (build, incremental upsert/delete, compaction, search ops/sec).

---

## Code Layout (Write Ownership by Milestone)
- **M1**:
  - `crates/pandora-cli/src/commands/moira.rs`
  - `npm/pandora-cli/src/client.ts` and `npm/pandora-cli/lib/*`
  - `docs/CLI_COMMANDS_REFERENCE.md`, `docs/TYPESCRIPT.md`
- **M2**:
  - `crates/pandora-types/src/distillation.rs`, `crates/pandora-types/src/lib.rs`
  - `crates/pandora-runtime/src/distillation.rs`, `crates/pandora-runtime/src/decision_engine.rs`, `crates/pandora-runtime/src/lib.rs`
- **M3**:
  - `crates/pandora-types/src/service.rs`
  - `crates/pandora-runtime/src/agent_loop.rs`, `crates/pandora-runtime/src/cockpit.rs`, `crates/pandora-runtime/src/cockpit_web.rs`, `crates/pandora-runtime/src/service.rs`
  - `crates/pandora-service/src/lib.rs`
- **M4**:
  - `crates/pandora-cli/src/commands/{run.rs, fleet.rs, cockpit.rs, cockpit_tui.rs, tui.rs, slash.rs, memory.rs, mod.rs, completions.rs}`
- **M5**:
  - `crates/pandora-runtime/src/hnsw.rs`, `crates/pandora-runtime/src/context_vfs.rs`
  - `crates/pandora-runtime/Cargo.toml`, `crates/pandora-runtime/benches/r3_throughput_bench.rs`
- **E2E**:
  - `TEST_INFRA.md`, `TEST_READY.md`, `crates/pandora-cli/tests/**`, `tests/**`
