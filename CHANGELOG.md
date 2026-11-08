# Changelog

## v3.0.7

### Phases 18-20 Mathematical Runtime Frontier (Theorems 35-40), 12-Engine Unified Assurance Pipeline & Live Gateway Daemon Guard
- **Phase 18 Mean-Field Game Equilibrium & Congestion Control (`crates/pandora-runtime/src/mean_field_game.rs`, `docs/math/phase18_mean_field_game.md`)**:
  - Discrete McKean-Vlasov / Nash-MFG fixed-point existence and Picard contraction under Lipschitz swarm interaction costs (Theorem 35) and Price-of-Anarchy ($PoA \le 4/3$) smoothness bounding for decentralized multi-agent tool/channel contention (Theorem 36) via `pandora runtime mfg`.
- **Phase 19 PAC-Bayesian Generalization & Prompt/Policy Risk Certificates (`crates/pandora-runtime/src/pac_bayes_monitor.rs`, `docs/math/phase19_pac_bayes_monitor.md`)**:
  - McAllester / Maurer empirical-Bernstein PAC-Bayes KL-divergence generalization bound $\mathrm{kl}(\hat{L}_n(Q) \| L(Q)) \le \frac{D_{\mathrm{KL}}(Q\|P) + \ln(2\sqrt{n}/\delta)}{n}$ (Theorem 37) and Catoni optimal Gibbs posterior temperature $\beta^*$ selection with finite-sample policy promotion gating (Theorem 38) via `pandora runtime pac-bayes`.
- **Phase 20 Tropical / Max-Plus Algebra DAG Critical-Path & Latency Calculus (`crates/pandora-runtime/src/tropical_dag.rs`, `docs/math/phase20_tropical_dag.md`)**:
  - Max-Plus semiring $(\mathbb{R}\cup\{-\infty\}, \oplus, \otimes)$ Kleene star $A^*$ convergence, critical-path makespan computation, and positive-cycle detection (Theorem 39) alongside Karp's maximum cycle mean $\lambda(G) = \max_i \min_{k<n} \frac{(A^n)_{ii} - (A^k)_{ii}}{n-k}$ for periodic pipeline throughput and SLA deadlock prevention (Theorem 40) via `pandora runtime tropical`.
- **12-Engine Unified Mathematical Assurance Receipt (`crates/pandora-runtime/src/assurance_receipt.rs`)**:
  - Expanded `UnifiedAssuranceReceipt` certifying all 12 mathematical runtime engines across Phases 9–20 (Theorems 17–40) with SHA-256 hash-chaining and append-only audit ledger commitment (`pandora runtime assurance`).
- **Live Telemetry Gateway & Autonomous Self-Healing Daemon Guard (`crates/pandora-runtime/src/gateway.rs`, `crates/pandora-cli/src/commands/runtime.rs`)**:
  - `GatewayController` actively ingesting provider latencies, token budgets, error rates, and DAG topologies; autonomous circuit-breaking and provider rerouting on theorem invariant violations; background daemon loop via `pandora runtime daemon-guard`; and cryptographic verification via `pandora audit verify`.
- **Workspace Release & Safety Invariants**:
  - Workspace manifests, C/C++ ABI headers, Go SDK, TypeScript client launcher, Python gateway bridge, and installers synchronized to `3.0.7` under strict safe Rust `#![forbid(unsafe_code)]`.

## v3.0.6

### Phases 15-17 Mathematical Runtime & Governance Frontier (Theorems 29-34) & Unified Assurance Pipeline
- **Phase 15 Conformal Prediction & Distribution-Free Uncertainty Calibration (`crates/pandora-runtime/src/conformal_calibrator.rs`, `docs/math/phase15_conformal_calibration.md`)**:
  - Split Conformal Prediction order-statistic rank calibration ($k = \lceil (n+1)(1-\alpha) \rceil$) with finite-sample marginal coverage guarantee $1 - \alpha \le \Pr(Y_{n+1} \in \widehat{C}_\alpha(X_{n+1})) \le 1 - \alpha + 1/(n+1)$ (Theorem 29) and Gibbs-Candès Adaptive Conformal Inference (ACI) online miscoverage tracking $\alpha_{t+1} = \alpha_t + \gamma(\alpha - \mathrm{err}_t)$ with deterministic $O(1/(T\gamma))$ long-run coverage convergence under arbitrary distribution shift (Theorem 30) via `pandora runtime conformal`.
- **Phase 16 Spectral Graph Partitioning & Algebraic Connectivity (`crates/pandora-runtime/src/spectral_topology.rs`, `docs/math/phase16_spectral_topology.md`)**:
  - Unnormalized ($L = D - A$) and symmetric normalized ($\mathcal{L}_{\mathrm{sym}} = I - D^{-1/2} A D^{-1/2}$) Graph Laplacian spectral analysis computing the Fiedler value $\lambda_2(L)$ via deflated shifted power iteration (Theorem 31) and certifying multi-agent communication bottlenecks via Cheeger's Inequality $\nu_2/2 \le h(G) \le \phi_{\mathrm{sweep}} \le \sqrt{2\nu_2}$ (Theorem 32) via `pandora runtime topology`.
- **Phase 17 Exp3 / Hedge Adversarial Multi-Armed Bandit Router (`crates/pandora-runtime/src/adversarial_bandit.rs`, `docs/math/phase17_adversarial_bandit.md`)**:
  - Log-domain stabilized Exp3 non-stochastic bandit router with unbiased importance-weighted reward estimation $\mathbb{E}[\hat{r}_{i,t} \mid \mathcal{F}_{t-1}] = r_{i,t}$ (Theorem 33) and minimax expected regret upper bound $\mathbb{E}[R_T] \le 2\sqrt{(e-1)TK\ln K}$ (Theorem 34) via `pandora runtime bandit`.
- **Unified Mathematical Assurance Receipt Pipeline (`crates/pandora-runtime/src/assurance_receipt.rs`)**:
  - End-to-end self-certifying cryptographic + statistical assurance receipt (`UnifiedAssuranceReceipt`) across all 9 advanced mathematical runtime engines (Phases 9–17, Theorems 17–34) committed via SHA-256 (`pandora runtime assurance`).
- **Security & CI Hardening**:
  - Updated `rustls` to `0.23.46` in `fuzz/Cargo.lock`, configured `.github/codeql/codeql-config.yml`, and eliminated APFS `fsync` timing variance in large-corpus diff-cache tests.

## v3.0.5

### Phase 6-14 CLI Subcommand Wiring & Next-Generation Mathematical Roadmap (Phases 9-14)
- **Phase 6-8 CLI Subcommand Wiring (`crates/pandora-cli/src/commands/models.rs`, `audit.rs`, `runtime.rs`, `hardware.rs`)**:
  - `pandora models route`: UCB1 and Gaussian-Gamma Thompson Sampling router simulation with sublinear $O(\sqrt{KT\ln T})$ regret telemetry.
  - `pandora audit`: RFC 6962 domain-separated ($0\text{x}00$ leaf, $0\text{x}01$ internal node) $O(\log_2 N)$ Merkle inclusion proofs and causal DAG lineage verification (`record`, `proof`, `verify`, `tree`, `simulate`).
  - `pandora runtime speculative benchmark` & `kv-cache`: Leviathan et al. exact target-distribution acceptance telemetry and Radix Trie KV cache prefix compression ratios.
- **Phase 9 Distributed Differential Privacy Budget Tracker (`crates/pandora-runtime/src/privacy_budget.rs`, `crates/pandora-cli/src/commands/privacy.rs`, `docs/math/phase9_privacy_budget_tracker.md`)**:
  - Rényi Differential Privacy (RDP) sequential composition (Theorem 17) and optimal $(\alpha, \varepsilon_{\text{RDP}}) \to (\varepsilon, \delta)$-DP tail bound conversion (Theorem 18) with `pandora privacy status|reserve|commit|release|record|simulate`.
- **Phase 10 Zero-Knowledge Succinct State Attestation (`crates/pandora-runtime/src/zk_attestation.rs`, `crates/pandora-cli/src/commands/zk.rs`, `docs/math/phase10_zk_attestation.md`)**:
  - Univariate polynomial commitments over the Goldilocks prime field $\mathbb{F}_p$ ($p = 2^{64} - 2^{32} + 1$) with Fiat-Shamir non-interactive challenge generation, $O(1)$ succinct verification, and Schwartz-Zippel soundness bound $\le k/p$ (Theorems 19 & 20) via `pandora zk prove|verify|benchmark`.
- **Phase 11 Dynamic Adaptive Work-Stealing Task Scheduler (`crates/pandora-runtime/src/work_stealing.rs`, `docs/math/phase11_work_stealing_scheduler.md`)**:
  - Blumofe-Leiserson double-ended work-stealing deques with proven makespan bound $\mathbb{E}[T_P] \le T_1/P + O(T_\infty)$ (Theorem 21) and bounded busy-leaves space $S_P \le P \cdot S_1$ (Theorem 22) via `pandora runtime scheduler` and `pandora hardware scheduler`.
- **Phase 12 Byzantine-Resilient Multi-Agent Consensus Aggregator (`crates/pandora-runtime/src/byzantine_aggregator.rs`, `docs/math/phase12_byzantine_consensus.md`)**:
  - Coordinate-wise $\beta$-Trimmed Mean, Coordinate Median, and $(n, f)$-Multi-Krum geometric consensus with proven breakdown points and bounded cone deviation (Theorems 23 & 24) via `pandora runtime byzantine`.
- **Phase 13 Entropic Optimal Transport Semantic Drift Detector (`crates/pandora-runtime/src/wasserstein_drift.rs`, `docs/math/phase13_wasserstein_drift.md`)**:
  - Log-domain stabilized Sinkhorn-Knopp dual iterations with Birkhoff-Hopf projective contraction $\kappa < 1$ and debiased Sinkhorn divergence $S_\varepsilon(\mu, \nu) \ge 0$ (Theorems 25 & 26) via `pandora runtime drift`.
- **Phase 14 Martingale Anytime-Valid Canary Gate (`crates/pandora-runtime/src/martingale_gate.rs`, `docs/math/phase14_martingale_canary_gate.md`)**:
  - Non-negative test supermartingale $e$-processes ($E_t = \prod_{i=1}^t (1 + \lambda_i X_i)$) with Ville's maximal inequality $\Pr_{H_0}(\exists t : E_t \ge 1/\alpha) \le \alpha$ and Howard et al. sub-Gaussian mixture confidence sequences (Theorems 27 & 28) via `pandora runtime canary-gate`.
- **Version 3.0.5 Release & Publication**:
  - Synchronized all workspace manifests, C/C++ headers, Go SDK, TypeScript launcher, Python gateway bridge, and installers to `3.0.5`.

## v3.0.4

### Phase 2-5 CLI and Daemon Endpoint Wiring (R1)
- **RFC 6455 Streaming WebSocket Daemon Endpoint (`crates/pandora-service/src/websocket.rs`, `crates/pandora-service/src/lib.rs`, `crates/pandora-cli/src/commands/service.rs`)**:
  - GET `/v1/ws` streaming upgrade endpoint operational alongside loopback JSON-RPC `/v1/rpc`.
  - Cryptographic handshake implementation computing SHA-1 base64 Sec-WebSocket-Accept token with magic GUID RFC 6455 framing.
  - Readiness metadata export with `ws_endpoint: ws://{address}/v1/ws` in `.pandora/service.ready.json`.
- **Epidemic Gossip Mesh & Bloom Filter Routing (`crates/pandora-cli/src/commands/peer.rs`, `crates/pandora-runtime/src/swarm_mesh.rs`)**:
  - CLI subcommands `peer gossip broadcast`, `peer gossip receive`, and `peer gossip filter`.
  - Configurable fanout parameter $K$, vector clocks, and 8192-bit Bloom filter message deduplication routing with zero false negatives.
- **BFT Quorum Consensus & Dynamic DAG Scheduling (`crates/pandora-cli/src/commands/fleet.rs`, `crates/pandora-runtime/src/swarm_mesh.rs`)**:
  - CLI subcommands `fleet quorum check`, `fleet quorum propose`, and `fleet quorum vote`.
  - Classical Byzantine fault tolerance verification establishing quorum size $Q \ge 2f + 1$ under $N \ge 3f + 1$ node boundaries.
  - Topological task DAG scheduling simulation with dependency resolution and stage scheduling.
- **Lyapunov-Damped Force-Directed Memory Graph & TUI (`crates/pandora-cli/src/commands/graph.rs`, `crates/pandora-cli/src/commands/tui.rs`, `crates/pandora-runtime/src/visual_graph.rs`)**:
  - CLI subcommand `graph visual` rendering ascii topological layouts of memory and semantic graph structures.
  - Interactive TUI Memory tab live topological graph rendering via `AsciiGridRenderer`.
  - Lyapunov stability guarantees preventing numerical oscillations during force relaxation.
- **Roofline Batch Vector Compute & Hardware Telemetry (`crates/pandora-cli/src/commands/hardware.rs`, `crates/pandora-runtime/src/hardware_batch_dispatch.rs`)**:
  - CLI subcommands `hardware roofline` and `hardware benchmark`.
  - Arithmetic intensity modeling against machine peak throughput $\pi$ and memory bandwidth $\beta$.
  - Empirical microbenchmarking across vector dimensions (128, 256, 512, 1536) for scalar, AVX2, and NEON architectures.

### Next-Generation Mathematical Roadmap (Phases 6-8) (R3)
- **Phase 6 Contextual Multi-Armed Bandit Model Router (`crates/pandora-runtime/src/bandit_router.rs`, `docs/math/phase6_bandit_model_router.md`)**:
  - Upper Confidence Bound (UCB1) and Linear Contextual Thompson Sampling router balancing exploration and exploitation across LLM provider arms.
  - Rigorous mathematical proofs establishing sublinear cumulative regret bound $\mathbb{E}[R(T)] \le 2\sqrt{8KT\ln T} + O(K) = O(\sqrt{KT\ln T})$.
- **Phase 7 Merkle-DAG Cryptographic Audit Log & Verifiable State Proofs (`crates/pandora-runtime/src/merkle_dag.rs`, `docs/math/phase7_merkle_dag_audit.md`)**:
  - Authenticated Merkle tree with SHA-256 node hashing and causal DAG tracking of agent execution mutations.
  - Exact logarithmic inclusion proof generation and verification verifying proof size $|\pi(i)| = \lceil \log_2 N \rceil = O(\log N)$.
  - Collision resistance theorem bounding preimage collision probability by $\le \frac{q^2}{2^{256}} \le 2^{-128}$.
- **Phase 8 Speculative Decoding & Prefix-Tree KV Cache Compression (`crates/pandora-runtime/src/speculative_decoding.rs`, `docs/math/phase8_speculative_kv_cache.md`)**:
  - Speculative rejection sampling engine verifying draft token proposals against target distribution while preserving exact target token distribution.
  - Theoretical speedup formulation $S = \frac{1 + \alpha\gamma}{1 + \gamma \cdot \frac{T_{\text{draft}}}{T_{\text{target}}}}$.
  - Radix Trie KV cache prefix tree compressing shared conversational prefix contexts with formal asymptotic compression ratio $C_R \to \frac{1}{1-\eta}$.
- **Phased Roadmap Formal Mathematics Extension (`docs/math/phased_roadmap_mathematics.md`)**:
  - Comprehensive formal integration spanning all Phases 1 through 8.

### Version 3.0.4 Release & Publication (R2)
- Manifests and package versions synchronized to `3.0.4` across workspace root `Cargo.toml`, 7 workspace crates, `Cargo.lock`, `npm/pandora-cli`, C/C++ headers (`pandora.h`, `pandora.hpp`), Go SDK (`types.go`), gateway bridge (`pandora_bridge.py`), LSP engine, installers, and test fixtures.
- Strict `#![forbid(unsafe_code)]` compliance maintained across all Rust crates with zero clippy warnings.
- Clean execution across all platform CI suites and release identity verification scripts.

## v3.0.3

### Hardware Acceleration Parity (ARM64 / Apple Silicon NEON) (R1)
- **128-Bit ARM64 NEON Vector Dot Product Assembly (`native/asm/pandora_simd_aarch64.s`)**:
  - Native 128-bit NEON vector dot product kernel using `fmla.4s` unrolled 4x across independent accumulator vectors (`v0`, `v1`, `v2`, `v3`).
  - Hides execution latency ($L_{\text{FMLA}} \in [3,4]$ cycles) and eliminates Read-After-Write (RAW) pipeline bubbles.
  - Zero callee-saved register modifications and zero stack frame allocation overhead under AAPCS64 conventions.
  - 64-byte burst loads via paired quadwords (`ldp`), matching standard 64-byte L1D cache line width.
  - Pairwise horizontal reduction tree (`faddp.4s`) minimizing floating-point roundoff error down to $O(\log_2 N \cdot \epsilon_{\text{mach}})$.
  - Dual ELF and Mach-O symbol exports (`pandora_simd_dot_product_128_neon` and `_pandora_simd_dot_product_128_neon`) supporting Linux AArch64 and Apple Silicon macOS.
- **Dynamic C ABI & C++20 Dispatch (`include/pandora.h`, `include/pandora.hpp`, `native/c/pandora_c_api.c`)**:
  - `pandora_cpu_has_neon()` CPUID feature detection and runtime dispatch in `pandora_simd_dot_product_128`.
  - C++20 RAII client wrappers (`pandora::Client::SimdDotProductNeon`, `pandora::Client::HasNeon`).
  - Safe host fallback execution on x86_64 host toolchains.

### ContextVfs Int8 Vector Quantization & Incremental Diff Caching (R2)
- **Scalar Int8 Dense Vector Quantization (`crates/pandora-runtime/src/semantic_memory.rs`)**:
  - Uniform symmetric scalar quantization (`QuantizedVector`: `pub scale: f32`, `pub data: [i8; 128]`) converting 128 continuous `f32` coordinates into signed 8-bit integers in $[-127, 127]$.
  - Reduces vector coordinate storage from 512 bytes to 128 bytes (75.00% exact reduction).
  - Reduces net serialized vector footprint from 512 bytes to 132 bytes (74.22% net reduction, saving 380 bytes per document).
  - Quantization noise variance bounded by $\sigma_\epsilon^2 \le \frac{s^2}{12} \le 5.16 \times 10^{-6}$, delivering typical SQNR $\approx 40.91\text{ dB}$.
  - Cauchy-Schwarz asymmetric dot product error bound $|E| \le 0.0445$.
  - Guaranteed $>99\%$ ranking preservation: Spearman rank correlation $\rho > 0.99$ and $\ge 99.9\%$ pairwise concordance for margin $\delta \ge 0.003$.
- **Accelerated SIMD Int8 Dot Products (`crates/pandora-runtime/src/arch_simd.rs`)**:
  - `simd_dot_product_f32_i8` unrolled across 8 independent `f32` accumulators with scale multiplication and $[0.0, 1.0]$ clamping.
  - `simd_dot_product_i8` unrolled across 8 independent 32-bit integer accumulators preventing overflow.
- **Binary Index Version 2 & Incremental Diff Caching (`crates/pandora-runtime/src/context_vfs.rs`)**:
  - Binary index serialization format Version 2 (`PVFS` v2, `VFS_INDEX_VERSION = 2`, `VFS_QUANTIZED_VECTOR_SIZE = 132`).
  - Seamless backward compatibility with legacy Version 1 (512-byte float) indices.
  - Canonical 32-byte SHA-256 buffer fingerprinting (`compute_doc_fingerprint`) across URI, title, content, tier, and intent.
  - In-memory differential caching (`diff_cache: RwLock<HashMap<String, CachedBufferEntry>>`) with atomic hit/miss telemetry, bypassing tokenization and feature hashing on warm cache hits.
  - Eviction of deleted documents and automatic cache synchronization.
- **CLI Storage Reporting (`crates/pandora-cli/src/commands/memory.rs`)**:
  - Updated `pandora memory vfs index` and `pandora memory vfs stats` with `vector_bytes_saved`, `cache_hits`, and `cache_misses`.

### External Gateway Security Hardening (R3)
- **Cryptographic HMAC-SHA256 Signature Verification (`scripts/gateway/pandora_bridge.py`)**:
  - Webhook endpoint (`POST /webhook`) verification over header `X-Pandora-Signature-256`.
  - Normalization supporting both raw 64-character lowercase/uppercase hex strings and `sha256=` prefixed strings.
  - Constant-time verification using Python's `hmac.compare_digest` to eliminate timing side-channel attacks.
  - Strict RFC 7235 / 9110 status separation: HTTP 401 Unauthorized for missing signature/token; HTTP 403 Forbidden for invalid or tampered signatures.
  - Dual-authentication support requiring both Bearer token and HMAC signature when both are configured.
  - CLI flag `--webhook-secret` reading from environment variable `PANDORA_WEBHOOK_SECRET`.
- **Gateway Security Test Suite (`scripts/test_gateway_service.py`)**:
  - Comprehensive test suite expanded to 17 unit and integration tests covering valid HMAC, prefixed signatures, missing headers, signature forgery, body tampering, and full dual-auth permutation matrices.

### Multi-Dimension Benchmarking & #![forbid(unsafe_code)] Compliance (R4)
- **Multi-Dimension Statistical Microbenchmarks (`crates/pandora-runtime/src/arch_simd.rs`)**:
  - Comprehensive microbenchmarking engine evaluating dimensions 128, 256, 512, and 1536 over $N \ge 30$ independent trials.
  - Calculates sample mean, unbiased sample variance ($s^2$), standard deviation, and Student's t-distribution 95% confidence intervals with exact tables ($\nu \le 30$) and Cornish-Fisher expansions ($\nu > 30$).
  - Evaluates speedup ratios with first-order bivariate Taylor series (Delta method) error propagation confidence intervals.
  - Demonstrates statistically significant speedups up to $4.81\times$ across dimensions ($p < 10^{-12}$).
- **Workspace-Wide Safety Invariants**:
  - 100% compliance with `#![forbid(unsafe_code)]` maintained across all 6 core production crates (`pandora-types`, `pandora-runtime`, `pandora-harnesses`, `pandora-provider`, `pandora-service`, `pandora-cli`).

### Mathematical Justifications & Engineering Specifications (R5)
- **Formal Documentation (`docs/MATHEMATICAL_JUSTIFICATIONS.md`)**:
  - Comprehensive 4-pillar engineering specification detailing ARM64 NEON microarchitecture latency hiding, Int8 quantization error bounds and ranking preservation proofs, HMAC-SHA256 cryptographic properties and collision resistance bounds, and microbenchmark statistical models.
- **Architectural Updates (`docs/ARCHITECTURE.md`)**:
  - Updated high-level architectural blueprints, repository layout, and cross-references detailing v3.0.3 SIMD parity, ContextVfs Int8 quantization, and gateway security.

### Author Identity Invariant (R6)
- **Git Commit & Tag Author Enforcement**:
  - All repository commits, tags, and documentation artifacts strictly authored under invariant identity: `ANISAYAK MITRA <266799942+anisayakmitra-in@users.noreply.github.com>`.

## v3.0.2

### Persistent Semantic Vector Index to Disk (`ContextVfs`)
- **Binary Index Serialization & Deserialization (`.pandora/indices/vfs.idx`)**:
  - High-performance binary VFS serialization and cold-start deserialization (`PVFS` v1 format) storing Okapi BM25 indices and 128-dimensional dense embeddings.
  - Instantaneous cold-start retrieval with zero network dependencies and cryptographic SHA-256 integrity validation.
  - Dedicated CLI commands for index management: `pandora memory vfs index` and `pandora memory vfs stats`.

### Live External Messaging Gateway Service (`scripts/gateway/pandora_bridge.py`)
- **Bidirectional Event Streaming Service**:
  - Multi-threaded local HTTP webhook receiver (`POST /webhook`) with optional Bearer token authentication.
  - High-throughput TCP socket server streaming NDJSON event frames bidirectionally.
  - Live subprocess execution runner streaming line-by-line tool executions and responses.
  - Dedicated service mode (`--service`) and automated integration test suite (`scripts/test_gateway_service.py`).

### Live LSP Diagnostics Watcher (`pandora lsp`)
- **Real-Time Workspace Diagnostics Daemon**:
  - Background daemon thread watching workspace file modifications with debounced compilation feedback.
  - Real-time visual pill status badges (`[Err]`, `[Warn]`, `[Clean]`) in the interactive TUI header bar and status line.
  - Interactive `/diagnostics` palette modal with severity filtering and rapid error navigation.

### SIMD Acceleration & Benchmark Suite
- **AVX2 Dot Product Acceleration**:
  - 256-bit AVX2 assembly dot product vector acceleration with runtime CPUID dynamic dispatch and scalar fallback for non-AVX2 CPUs.
  - Benchmark suite measuring latency, throughput, and speedup ratios comparing AVX2 and scalar Rust implementations.

## v3.0.1

### External Messaging Gateway Bridge (`scripts/gateway/`)
- **Telegram & Discord Autonomous Gateway**:
  - Long-polling Telegram bot supporting `/status`, `/doctor`, `/pending`, `/approve <id>`, `/reject <id>`, `/run <task>`, and `/help`.
  - Discord webhook event dispatcher sending formatted embeds for approvals, task completions, and health warnings.
  - Autonomous cron execution monitor polling active scheduled tasks and dispatching updates to messaging channels.
  - Human-in-the-loop remote approval resolution executing authorized tasks securely through Pandora's ReferenceMonitor.
  - Windows (`start_bridge.bat`, `start_bridge.ps1`) launchers with `--smoke` verification.

### Enhanced TUI Ergonomics (`pandora tui`)
- **Advanced Mouse & Keyboard Controls**:
  - Horizontal scroll wheel (`ScrollLeft` / `ScrollRight`) view cycling across Console, Kanban, Memory, and Governance tabs.
  - Skills Hub Modal mouse integration: mouse wheel scrolling, direct click-to-toggle on skill rows, click outside to close.
  - Clickable top bar shortcut buttons: `[F1 Help]`, `[F2 Model]`, `[/keys]`, `[/skills]`, and `[F6 S1/S2]`.
  - Direct authorization key shortcuts: pressing `y`/`Y` or `n`/`N` on empty prompt immediately resolves pending approvals.
  - Standard readline editing: `Ctrl+Backspace` for previous word deletion, `Ctrl+Delete` for next word deletion, and `Alt+[` / `Alt+]` for bracket tab navigation.
  - Added full test coverage in `test_advanced_mouse_and_keyboard_controls` (40/40 TUI tests passing).

## v3.0.0

### Pandora v3.0.0-SUNLESS: Autonomous Evolutionary Harness Runtime

### Declarative Structured Reasoning (DSRs) — DSPy for Rust (`pandora dsrs`)
- **Type-safe Reasoning Signatures (`pandora-types::dsrs`)**:
  - `Signature`, `SignatureField`, `SignatureFieldKind`, `DsrDemonstration`, and `DsrsError`.
  - Dynamic prompt formatting with strict type assertions, few-shot demonstration binding, and input value substitution.
  - Robust output parsing supporting both multi-field delimiters and single-field fallback.
  - Automatic `into_chain_of_thought()` converting any signature into an explicit reasoning module with prepended `rationale`.
  - Built-in canonical signatures: `CodeReviewer`, `ReActModule`, `SqlSynthesizer`.
  - CLI subcommands: `pandora dsrs show`, `pandora dsrs inspect <name>`, and `pandora dsrs cot <name>`.
  - TUI integration: `/dsrs` Spotlight command and palette action.

### Genetic-Pareto (GEPA) Optimizer (`pandora gepa`)
- **Multi-Objective Pareto Evolution (`pandora-runtime::gepa`)**:
  - Multi-objective Pareto frontier tracking 3 orthogonal axes: `pass_rate` (maximize), `token_cost` (minimize), and `latency_ms` (minimize).
  - Strict Pareto dominance check ($A \succ B$), non-dominated candidate preservation, and dominated ancestor pruning.
  - `GepaReflectiveMutator`: extracts concrete avoidance invariants from failure traces $\mathcal{T}_{\text{fail}}$ to guide targeted prompt mutations.
  - CLI subcommands: `pandora gepa frontier` and `pandora gepa optimize`.
  - TUI integration: `/gepa` Spotlight command and palette action.

### The 6-Level Optimization Ladder ($L_0 \to L_5$) (`pandora evolution ladder`)
- **Taxonomy of Harness Self-Improvement** (synthesized from `Awesome-Harness-Self-Improvement`):
  - Formalized 6 rungs of self-improvement:
    - **L0: Instruction Prompts** (APE, OPRO, Promptbreeder, GEPA)
    - **L1: Context & Memory** (ACE, ReasoningBank, layered memory, context compression)
    - **L2: Workflow Graph** (ADAS, AFlow, GPTSwarm, dynamic branching)
    - **L3: Harness Code** (Darwin Gödel Machines, SICA, Gödel Agent)
    - **L4: Meta-Optimizer** (STOP, Meta-Harness, MCE, Meta^n recursive depth)
    - **L5: Harness + Weights** (Joint harness scaffolding and foundation model weight co-optimization; SIA, SEAL)
  - Triad of Self-Improvement Failure Modes & Built-in Safeguards:
    - **Reward Hacking**: Tamper-proof signatures and isolated external evaluator.
    - **Capability Collapse**: Multi-domain holdout regression suites and monotonic Pareto frontier checks.
    - **Harness Drift**: Strict pre-flight invariance contract assertions and cross-validation holdouts.
  - CLI subcommand: `pandora evolution ladder`.
  - TUI integration: `/ladder`.

### Darwin Whole-Agent Genomes & Task-Domain Routing Cards (`pandora evolution genome`, `routing-card`)
- **Whole-Agent Natural Selection** (synthesized from `Darwin`):
  - Whole-Agent Genome representation: $\mathcal{G} = [\text{system\_prompt}, \text{tool\_bindings}, \text{model\_target}, \text{hyperparameters}]$.
  - 4-Point Safety Spine (enforced as code):
    1. *Sandboxed Self-Modification*: Variant modification and execution occurs exclusively within isolated sandboxes.
    2. *Immutable External Grader*: Fitness evaluator is isolated from the mutator's reach and cannot be gamed.
    3. *Monotonic Rollback*: Any variant regressing on holdout fitness is killed and automatically restored from parent snapshot.
    4. *Hard Compute Cap*: Strict generation, token, and turn budgets with mandatory human veto points.
  - Task-Domain Routing Cards: Synthesizes empirical Pareto champions across task classes (`coding`, `reasoning`, `synthesis`, `data_analysis`).
  - CLI subcommands: `pandora evolution genome [--step]` and `pandora evolution routing-card`.
  - TUI integration: `/genome` and `/routing-card`.

### Meta^n Recursive Emergent Depth & Fixed-Point $\Omega$ (`pandora evolution depth`)
- **Emergent Depth Recurrence** (synthesized from `meta-n` arXiv:2608.24735):
  - Constructive $N$-th order meta-layers generated from lower-layer execution traces.
  - Constructive code injection: `pre_process` normalization hooks and `code_library` helper functions.
  - Fixed-Point Termination ($\Omega$): Detects when successive meta-reflection reaches convergence attractor $M(\mathcal{P}^*) = \mathcal{P}^*$.
  - CLI subcommand: `pandora evolution depth`.
  - TUI integration: `/depth`.

### Product Requirement Prompts (PRP) Pipeline (`pandora prp`)
- **Progressive Validation Gates & Contracts** (synthesized from `hack`):
  - 4-Level Task Hierarchy: $\text{Phase} \to \text{Milestone} \to \text{Task} \to \text{Subtask}$.
  - Context-dense prompt contracts: Pre-flight invariants, acceptance criteria, and snapshot rollback plans.
  - 4 Progressive Validation Gates:
    - **Gate 1: Syntax** — Compiler diagnostics, AST validation, and strict linter clean checks.
    - **Gate 2: Unit** — Fast isolated unit test suites in memory or sandbox.
    - **Gate 3: Integration** — Multi-agent end-to-end task execution and contract verification.
    - **Gate 4: Receipt Check** — Cryptographic evidence receipts and manual review gates.
  - Fast-fail progressive evaluation stops immediately on the first defect.
  - New top-level CLI command: `pandora prp inspect`, `pandora prp scaffold`, and `pandora prp validate`.
  - TUI integration: `/prp`.

### Universal Evolutionary Lineage DAG & Interactive Visualizer (`pandora evolution lineage`)
- **Lineage DAG & Standalone Visualizer** (synthesized from `darwinian_evolver`):
  - Complete DAG tracking candidate organisms, parent-child mutation edges, score deltas, and failure cases addressed.
  - ASCII DAG tree output in terminal.
  - Standalone dependency-free interactive HTML/SVG lineage visualizer export with node inspector, score deltas, and generation timeline.
  - CLI subcommand: `pandora evolution lineage [--export-html <path>]`.
  - TUI integration: `/lineage`.

### Harness² 8-Component Tree & Recursive Harnessing (`pandora harness`)
- **Component Decomposition & Recursive Engine** (synthesized from `harness2`):
  - Canonical $c_1 \dots c_8$ decomposition: System Prompt, Guardrails, Memory, Skills, Tool Guidance, Scripts, Subagents, Plugins.
  - Recursive Propose–Probe–Compose engine supporting Parallel (contrastive Pareto exploration across $K$ budget) and Sequential recursive refinement.
  - Persistent Improvement Harness Library tracking pass rates, token metrics, latency, and synthesized reflection summaries.
  - CLI subcommands: `pandora harness tree show|diff`, `pandora harness recursive run`, and `pandora harness library list|inspect`.
  - TUI integration: `/harness`.

### Extreme Keyboard & Full Mouse Navigation in TUI
- **Extreme Keyboard Navigation**: Full binding support across `F1`–`F12`, `Alt+F`/`Alt+B`, `Ctrl+A`/`Ctrl+E`, `Alt+Backspace`, `Ctrl+U`, `Ctrl+K`, `Home`/`End`, etc.
- **Full Mouse Navigation**: Interactive mouse click support across all tabs (Console, Kanban, Memory, Governance), model badge, effort selector, brand title, and custom input cursor positioning.

### Major Version Bump & Shell Completions Parity
- Bumped workspace version from `2.0.0` to `3.0.0`.
- 100% completion script parity across PowerShell, Bash, Zsh (all 36 branch entries), and Fish for all new commands and subcommands (`dsrs`, `gepa`, `prp`, `evolution ladder|genome|routing-card|lineage|depth`).
- Strict `#![forbid(unsafe_code)]` and zero warnings on `cargo clippy --workspace --all-targets --locked -- -D warnings`.

## v2.0.0

- **Custom model provider support & management**: `pandora provider add <name> <base_url> [model]`,
  `pandora provider remove <name>`, `--no-api-key`, `--api-key-stdin`, `--set-default`,
  and in-TUI `/provider add <name> <base_url> [model]` for seamless connection to
  local inference servers (Ollama, vLLM, LM Studio) and custom OpenAI-compatible or Anthropic endpoints.
- **OAuth 2.0 PKCE loopback authentication**: local HTTP callback server on
  `127.0.0.1:1455/auth/callback` with random CSRF state, PKCE code verifier and challenge
  (S256), automatic browser authorization launcher, and secure token exchange.
- **Multi-account credential management**: persistent multi-account credential storage
  and active account switching via `pandora provider accounts`, `/login`, and `/account`.
- **Spotlight model & provider picker**: interactive modal (`Ctrl+M` / `/model`) in the
  workbench TUI with fuzzy search, instant switching of active provider and model, and
  status bar synchronization.
- **TrueColor terminal mural canvas**: retro-futuristic backdrop rendering (`Alt+B` /
  `/backdrop`) with terminal size adaptivity and rich color palettes.
- **Interactive workbench TUI by default**: bare `pandora` invocation launches the
  interactive terminal workbench immediately; non-interactive commands execute directly.
- **Brand identity & OpenGraph social preview**: official 1280×640 px preview image
  (`docs/assets/social-preview.png` and `.jpg`) and hardened Mermaid sequence diagrams.

## v2.0.0-beta.10

- **OS-level process confinement**: Windows Job Object boundary (no breakaway,
  kill-on-close, active-process and job-memory limits) and Landlock/Seatbelt
  platform detection in `pandora-sandbox`; native process execution fails closed
  without confinement evidence unless `--trusted-process` is explicit.
- **Filesystem TOCTOU hardening**: handle-relative containment verification
  (`/proc/self/fd` on Linux, `fcntl(F_GETPATH)` on macOS, canonical handle checks
  on Windows) against concurrent path and symlink substitution.
- **Active policy hooks (`pandora policy hooks list|validate`, `--hooks <path>`)**:
  declarative lifecycle-hook vetoes loaded into the execution controller; vetoed
  operations fail closed with `policy_denied` before effect authorization.
- **Recursive self-improvement substrate**: LATS Monte Carlo tree search, System 1
  to System 2 escalation gate, Deep Reflexion diagnostics, Voyager curriculum
  (`pandora curriculum`, `--strategy lats`), tripartite cognitive governance,
  Pareto and Bayesian verifiers, and signed memory capsule export.
- **Reasoning effort control and prompt cache optimization**, plus expanded
  architecture, verification, evolution, and swarm Harness domains.
- **Module decomposition** of the execution controller, agent loop, and provider
  client with public APIs unchanged.
- **Security**: `tokio` 1.54.1 (RUSTSEC-2026-0386), `rustls-webpki` 0.103.16 and
  `webpki-roots` 1.0.10 (RUSTSEC-2026-0381, RUSTSEC-2026-0382).
- Root `install.sh` and `install.ps1` entry points; job store busy-retry fix.

## v2.0.0-beta.9

- **Durable Multi-Agent Kanban Board (`pandora kanban`)**: SQLite-backed coordination
  state machine managing task lifecycles (`to_do`, `in_progress`, `review`, `done`, `blocked`),
  worker heartbeats, and automated zombie task reclamation (`pandora kanban task create|update|claim|heartbeat|reclaim|board`).
- **Bot Mode & Inter-Agent Society (`pandora peer`)**: Multi-agent social collaboration
  featuring built-in personas (`@architect`, `@coder`, `@reviewer`, `@curator`, `@auditor`),
  custom persona registration, and threaded peer direct messaging (`pandora peer list|register|inspect|send|inbox|read|reply|thread|delete`).
- **Session Auto-Pruning & SQLite Vacuum Optimizer (`pandora session`)**: Lifecycle storage
  maintenance engine with age-based session/event/turn pruning and conditional or forced
  SQLite VACUUM disk space reclamation (`pandora session prune`, `pandora session vacuum`).
- **Agent-to-Agent (A2A) JSON-RPC 2.0 Protocol v1.0 (`pandora peer a2a`)**: Standards-compliant
  cross-machine JSON-RPC 2.0 protocol for agent discovery, capability handshakes (Agent Cards),
  and cryptographic SHA-256 verified remote task delegation (`pandora peer a2a card|rpc|delegate|status`).
- **Browser PKCE OAuth 2.1 Connectors Platform (`pandora connector`)**: Secure RFC 7636
  Proof Key for Code Exchange (PKCE) OAuth 2.1 platform for external SaaS integrations
  (GitHub, Linear, Notion, Asana, Custom) with loopback code exchange and encrypted vault token storage (`pandora connector list|authorize|exchange|inspect|revoke`).

## v2.0.0-beta.8

- **5-Tier Layered Crystallized Memory Architecture (L0-L4)**: Crystallizes memory
  across task execution stages: L0 (Constitutional Meta Rules), L1 (Insight Index
  partitioned across `repo`, `procedure`, `coding`, and `user` intents), L2 (Durable
  Global Facts), L3 (Task Skills / Standard Operating Procedure DAGs), and L4
  (Session Archives with execution trajectories, diff statistics, and receipt
  digests). Backed by SQLite Migration 10 introducing `memory_sops` and
  `memory_archives` tables with compound intent/tier indexes and multi-tenant
  isolation (`pandora memory sop list|inspect`, `pandora memory archive list|inspect`).
- **Hierarchical Virtual Context Filesystem (`pandora://`)**: Added an in-memory
  VFS engine for uniform navigation and inspection across constitutional rules,
  insight indexes, durable facts, crystallized SOPs, session archives, and codebase
  feature maps with strict path-traversal confinement (`pandora memory vfs ls`,
  `pandora memory vfs cat`).
- **System 1 Fast-Path Decision & Reflex Engine (`pandora-decision`)**: Single-pass
  logit engine executing fast-path reflexive decisions (`ToolSafetyChoice`,
  `IntentTierChoice`, `ConvergenceChoice`) with calibrated softmax, temperature tuning,
  and anti-thrash loop detection tracking consecutive decision cycles to prevent
  infinite oscillatory failure states.
- **GEPA Multi-Objective Pareto Skill Evolution & Bayesian Canary Gating**:
  Integrated Pareto frontier tracking (`ParetoFrontier`) across non-dominated trade-offs
  in PassRate, TokenCost, and StepLatency. Multi-sample canary validation is governed
  by `BayesianCanaryGate`, computing exact Beta(1+k, 1+(n-k)) numerical integrals and
  large-sample normal approximations to enforce 95% lower credible bounds before
  promoting evolved candidates.
- **Signed Evolutionary Memory Capsules**: Cryptographic SHA-256 digest export
  and import protocol (`MemoryCapsule`) for packaging and safely sharing validated
  L3 SOPs across workspaces with author provenance verification and DAG acyclicity
  validation (`pandora memory capsule export|import`).
- **Active-Learning Uncertainty Harvester & Golden Dataset Synthesis**: Automated
  harvesting of low-margin uncertain decisions from the System 1 decision engine
  and exporting them into validated golden evaluation test suites ready for
  evaluation loops (`pandora decision harvest`).
- **Darwinian Island-Model Multi-Population Genetic Evolver**: Manages isolated
  subpopulations ("islands") targeting distinct optimization objectives (`QualityFirst`,
  `CostMinimal`, `LatencyOptimized`) with tournament selection, crossover recombination,
  mutation operators, and periodic elite migration (`pandora evolution island run`).
- **Dynamic Emergent Swarm Topology Synthesis**: Autonomous intent-driven synthesis
  transforming architectural task intents into validated diamond execution topologies
  (Planner $\to$ Parallel Specialist Roles $\to$ Synthesizer $\to$ Canary Gate) with
  duplicate and reserved role validation (`pandora orchestration synthesize`).
- **Autonomous Reflexion & Memory Self-Crystallization**: Differential trajectory
  analysis inspecting completed L4 session archives, abstracting concrete workspace
  literals (absolute paths, 40-character commit hashes, session UUIDs, SHA-256 digests)
  into parameterized `SopStep` templates, verifying candidates through the Bayesian
  canary gate, and automatically promoting them to active L3 storage in SQLite
  (`pandora memory crystallize`, `pandora evolution loop`).
- **Deterministic Time-Travel Replay & Checkpoint Forking**: Differential turn
  snapshots with SHA-256 step digests, an execution rollback engine to restore
  workspace and session state to prior step indices, and an isolated Git worktree
  forking engine to evaluate competing models or speculative branches (`pandora
  session rewind`, `pandora session fork`, `pandora session diff`).
- **Decentralized P2P Capsule Mesh Synchronization**: Peer discovery and Ed25519
  identity handshake supporting gossip broadcast and synchronization of signed
  memory capsules with 3-point validation (SHA-256 digest, signature verification,
  DAG acyclicity check) before admitting into local SQLite (`pandora memory mesh
  peers`, `pandora memory mesh broadcast`, `pandora memory mesh sync`).
- **Production Release Ecosystem & Multi-Platform Manifests (Phase 6)**: Automated
  generator for Homebrew bottle formula, Scoop bucket JSON, and winget YAML manifests
  (`scripts/generate_release_manifests.py`) verified in CI against release checksums
  and keyless cosign signatures.
- **Provider Setup Presets & Model Validation**: Streamlined CLI interactive setup
  (`pandora setup`) with presets for Ollama, DeepSeek, OpenRouter, Groq, Anthropic,
  and OpenAI, validating model tags and credentials before persistence.
- **CLI-Only Focus & Desktop Product Retirement**: Retired legacy Tauri desktop
  product and desktop release paths on 2026-10-05, locking release scope to `cli-only`
  and focusing all runtime performance on the high-throughput CLI boundary.

- Added an honest stable rollback closure record. RC and stable tags now validate
  the four native accessibility manifests against their exact commit. Stable
  publication waits for every published CLI and desktop lifecycle job, then
  records the compatible stable predecessor and workflow run. The first stable
  in a line is explicitly `pending_first_patch`; a patch without a compatible
  stable predecessor fails closed instead of claiming synthetic rollback proof.

- Hardened release-candidate and stable publication. Both channels now require
  explicit approval plus Windows and Apple signing/notarization credentials;
  the publish job accepts only `v*` tags through a protected human-reviewed
  environment. Release evidence records the vendor-signing requirement, and
  clean published-package jobs independently verify Windows Authenticode,
  Apple code signatures, notarization tickets, and Gatekeeper before retaining
  signature and lifecycle evidence.

- Added retained desktop-readiness evidence on all four CI platforms. The
  rendered suite now covers keyboard-only visible focus, 100%, 150%, and 200%
  scale equivalents, forced colors, increased contrast, reduced motion, and
  reduced transparency, with screenshots retained for 90 days. Fresh-runner
  package and synthetic upgrade drills emit privacy-safe JSON evidence for
  install, start, exact desktop/CLI sidecar identity, update, rollback,
  uninstall, and cleanup. A strict native-evidence validator and manual workflow
  require exact-commit NVDA, VoiceOver, and Orca records for every advertised
  platform; missing, substituted, traversing, tampered, or incomplete evidence
  fails closed.

- Added signed remote distribution for Gene, Domain Harness, Meta Harness, Skill,
  and Provider packages. Registry discovery and exact-version or pinned-commit
  GitHub downloads now verify the Official publisher root, Ed25519 signature,
  runtime compatibility, kind, and artifact digest into an inert durable cache.
  Offline verification and append-only evidence expose the exact publisher/key,
  source revision, manifest digest, artifact digest, dependencies, and admission
  binding across CLI, TUI, and desktop. Admission is a separate dry-run/confirmed
  boundary and leaves Skills disabled and Providers inactive; enablement,
  Provider selection, approval, and effect permits remain separate. Identity
  conflicts, signature substitution, downgrades, traversal, missing dependencies,
  untrusted publishers, and revoked keys fail closed, while revocation suspends
  matching managed Skills and quarantines matching Provider profiles.

- Added durable governed rollout stages for canary, limited, expanded, and
  complete. Every stage carries explicit cost, duration, failure, quality,
  latency, and stability limits; records a durable scorecard; and requires a
  separate non-expired human approval bound to the exact proposal, commit,
  artifact, channel, evidence, and scorecard. CLI, TUI, service, and desktop
  surfaces expose pause, resume, reject, bounded retry, promotion, and rollback
  evidence. Protected release environments may create one approved tag while
  preserving the existing artifact activation and tag-driven publication paths.

- Added one durable aggregate execution budget across dependent orchestration
  roles and repositories. SQLite now atomically reserves role capacity at
  dispatch and settles receipt-linked token, tool, elapsed-time, and cost usage
  with completion. Interrupted effects keep their reservation until explicit
  evidence reconciliation; duplicate reconciliation cannot double-consume, and
  retries require remaining headroom. Unknown provider cost stays visibly
  unknown while enforcement conservatively charges the full reservation. CLI,
  TUI, Fleet dashboard, and desktop Background Runs share the privacy-safe
  ceiling/reserved/consumed/remaining view.

- Added retained worker-stress campaigns with selectable ten-minute, two-hour,
  checkpointed eight-hour, and checkpointed twenty-four-hour profiles across
  all four desktop platforms. Segment evidence now combines cancellation races,
  crash and stale-lease recovery, exact-once receipts, queue and process
  metrics, clean shutdown, and partial multi-repository failures into one
  fail-closed campaign summary. Current job stores also reopen through a
  read-only schema fast path so concurrent producers do not contend on an
  already-complete migration. Fleet persistence now uses WAL with a bounded
  busy wait, and a drain that wins a heartbeat race is classified as a normal
  stop request while cleanup still must release every lease.

- Backported the upstream RUSTSEC-2024-0429 `glib::VariantStrIter` fix onto the
  exact crates.io `glib 0.18.5` source required by Tauri's Linux GTK3 stack.
  Cargo now resolves the reviewed local source, and repository validation
  binds its provenance, fixed source digest, and lockfile override until the
  upstream GTK4 migration makes `glib 0.20` reachable.

- Added one privacy-safe Fleet operations snapshot across CLI, TUI, and
  desktop, with queue depth, lease age, stale supervisor, failure, and budget
  ceiling views that exclude prompts, outputs, credentials, and hidden
  reasoning. Added retained four-platform worker-soak automation, a staged SDK
  package/evaluation/canary pipeline, and a distinct release-candidate update
  channel on the existing signed release evidence path.

- Added a self-contained Tauri desktop app for Windows, macOS, and Linux.
  Release builds stage the same-commit `pandora` CLI as a native sidecar;
  release launchers fail closed when that sidecar is missing or unsafe.
- Added native macOS 26 Liquid Glass using AppKit's supported Clear material,
  with semantic vibrancy fallback on older macOS. Linux and Windows keep their
  opaque application surfaces.
- Moved desktop CI to macOS 26 and verified Tauri bundles on macOS, Ubuntu, and
  Windows. The release matrix now covers macOS Intel and Apple Silicon
  separately.

- Relicensed Pandora-owned material under the Apache License 2.0. Third-party
  dependency and example-package licenses remain governed by their respective
  owners and package manifests.
- Added bounded Auto Route hints for admitted Domain Harnesses. Shadow Council
  selects the most specific active route, rejects equal matches, and preserves
  explicit operator selection. Route metadata grants no runtime capability.
- Added a deterministic local Domain Harness starter kit, CLI scaffolder, and
  TUI package discovery. Duplicate owned Gene IDs and undeclared package
  capability fields now fail closed before admission.
- Added a deterministic, composition-only Meta Harness starter across the CLI,
  TUI, and desktop inspector. Exact built-in Domain dependencies now resolve
  correctly, while duplicate, self-cyclic, missing, disabled, wrong-kind, and
  over-limit compositions fail closed without effect authority.
- Added a validated declarative Gene example pack with static guidance,
  bounded-read, and approval-bound write-proposal contracts. CLI, TUI, and
  desktop inspection now expose exact capability declarations, artifact
  provenance, owning Domains, disabled-by-default lifecycle, and rollback
  evidence. Contracted effects remain on the Parliament, approval,
  ReferenceMonitor, permit, executor, and receipt path.
- Joined durable evaluation schedules with registered evidence-backed and
  task-backed suites. Proposal-bound schedules are one-shot canaries that
  retain exact report digests and case counts, derive the versioned
  zero-failure result, and stop before the separate activation gate.
- Added scoped memory-retention controls across the runtime, CLI, TUI guidance,
  and desktop. Operators preview an exact revocation-time boundary before typed
  confirmation removes already-revoked logical records; tombstones and audit
  evidence remain, and the UI explicitly distinguishes compaction from secure
  erasure of database pages, WAL files, backups, and storage snapshots.
- Bound research evolution proposals to the canonical IDs of the exact memory
  records included in their evidence. The bounded list is covered by the
  evidence digest, survives restart, remains backward compatible with older
  proposals, and is visible in CLI, service, and desktop lineage views.
- Added versioned cross-project memory consolidation policy across the core
  contract, CLI, TUI guidance, and desktop disclosure. Transfers require exact
  workspace IDs, stay within one tenant and provider, reject sensitive or L2
  sources, and resolve target collisions only as fail-closed `reject` or
  non-mutating `keep-target`; overwrites and tombstoned-ID reuse remain denied.
- Added verified, signed replacement of optional built-in Domain and Meta
  Harnesses with exact rollback to the compiled default. The constitutional
  Source Harness remains immutable.
- Upgraded package signatures to payload version 2 so package kind, exact
  dependencies, Meta composition, and Domain routes are bound by the signature.
  Packages signed with the beta version 1 payload must be signed and admitted
  again.
- Reworked the roadmap from a live implementation audit and added issue forms
  for bounded feature proposals and first-time contribution work.


  service identities with viewer, operator, and administrator roles,
  Ed25519-bound device credentials with freshness and replay checks, encrypted
  provider secret vaults, authenticated encrypted backup/restore with SQLite
  integrity validation, and local redacted crash and operational records.
- Added explicit stable and beta update-channel validation. Stable release tags
  now fail closed without an explicit release approval plus Windows signing
  and Apple signing/notarization credentials; existing checksum, Cosign, SBOM,
  provenance, and cross-platform lifecycle gates remain required.

- Added governed admitted-artifact evolution activation. The durable artifact
  catalog resolves bounded replacement chains, rejects cycles and out-of-order
  rollback, and exposes active bindings read-only through the local service and
  desktop. New CLI stage, canary, activate, and rollback commands preserve
  Parliament, evaluation, package-admission, and no-authority-expansion gates.

- Wired active admitted replacements into custom Wasm Gene runtime assembly.
  Each run keeps the base Gene identity, snapshots the exact resolved content
  hash into its execution profile and permit, and exposes that evidence without
  changing runtime authority. Approval IDs now include the durable session
  identity so governed runs in separate CLI processes cannot collide.

- Added runtime-owned approval list, inspection, resolution, and exact-digest
  run resumption to the authenticated local service. Pandora Desktop now shows
  the real approval subject and digest, supports deny or allow-once decisions,
  and resumes the paused run only after the approved grant is consumed.

- Fixed a process-output polling race that could report a completed child
  reader as an I/O failure after its bounded output had already been received.
  This stabilizes governed process execution across runner timing differences.

- Fixed desktop run-state transitions so active runs prevent duplicate
  submission while inspecting an existing session leaves the composer usable.
  Added rendered desktop regression tests and made them release gates.

- Hardened local Skill admission with exclusive staging and atomic destination
  commit. Existing Skills are never overwritten by a collision, concurrent
  installs fail closed, and newly admitted Skills remain disabled.

- Added the fixed `tests.run` Coding Gene, `workspace.test` tool, and `/test`
  command. It runs only `cargo test --locked` through the existing approval,
  permit, timeout, cancellation, and receipt path.

- Added the fixed `format.check` Coding Gene, `workspace.format` tool, and
  `/format` command. It runs only `cargo fmt --all -- --check` through the same
  governed process path.

- Added the fixed `lint.check` Coding Gene, `workspace.lint` tool, and `/lint`
  command. It runs only the locked workspace Clippy check through the same
  governed process path.

- Added the fixed `build.check` Coding Gene, `workspace.build` tool, and
  `/build` command. It runs only `cargo build --locked` through the governed
  process path.

- Added the read-only `workspace.status` Coding Gene, tool, and `/status`
  command. It runs only `git status --short` inside the configured workspace
  and returns the bounded output through the existing process receipt path.

- Added the read-only `workspace.diff` Coding Gene, tool, and `/diff` command.
  It runs only `git diff --no-ext-diff --unified=3` inside the configured
  workspace and returns the bounded diff through the existing process receipt
  path.

- Added the read-only `workspace.log` Coding Gene, tool, and `/log` command.
  It runs only `git --no-pager log --oneline --decorate -n 20` inside the
  configured workspace and returns bounded recent history through the existing
  process receipt path.

- Added the read-only `workspace.refs` Coding Gene, tool, and `/refs` command.
  It enumerates the fifty most recent local branches, remotes, and tags with a
  fixed Git ref query through the existing process receipt path.

## v2.0.0-beta.7

- Added `SelfHealingEngine` and connected recovery candidates to
  `CodingFeedbackLoop`. Recovery selection remains policy-bounded and emits
  the existing adaptation receipt; it does not execute actions or expand
  authority.

- Added a bounded runtime composition ledger. Execution profiles now bind
  deterministic component provenance and identity digests for the runtime,
  selected executor, and containment evidence without creating a second
  authority or activation path.

- Added a bounded, stale-safe memory synthesis contract. Synthesized L1
  candidates retain explicit origin and evidence IDs, exclude sensitive inputs,
  and never bypass approval for L2 promotion or effect authority.

- Added an explicit graph snapshot store. `pandora graph` remains stateless by
  default and can persist a validated, scope-isolated snapshot with `--store`.

- Added a bounded `pandora evolution evaluate` holdout runner. It records
  trajectory, outcome, policy, and regression evidence through the existing
  durable EvolutionEngine without executing tools, calling providers, or
  granting authority.
- Added `pandora evolution submit` for bounded proposal intake through the
  existing durable EvolutionEngine. Submission remains evidence-only and
  cannot approve, activate, or execute a candidate.

## v2.0.0-beta.6

- Adopted the MIT license for Pandora-owned source, packages, fixtures, and
  documentation. Third-party dependency licenses remain governed by their
  respective packages.
- Added a native Gemini `generateContent` provider adapter with bounded text
  and tool normalization, continuation mapping, and `x-goog-api-key`
  credential isolation. The existing provider permit and receipt boundary is
  unchanged.
- Hardened verification and local MCP process cleanup so descendants are
  terminated and reaped with the owning operation.
- Added durable SQLite evolution records with restart recovery and read-only
  `pandora evolution list` and `pandora evolution inspect` commands. Stored
  records remain evidence and workflow state; they cannot authorize effects.
- Documented the Security Domain as a governed, evidence-first assessment
  workflow with explicit boundaries and no unsupported scanner or remediation
  claims.

## v2.0.0-beta.5

- Added the read-only `data-domain` Harness with bounded inventory, schema,
  quality, lineage, analysis, and guidance Genes. It records local evidence
  without database connections, query execution, data mutation, or statistical
  correctness claims.
- Added explicit `security.deep-scan` and `security.diff-scan` Genes to the
  read-only Security Domain. They mirror staged security workflow boundaries
  for broader evidence collection and changed-code review while remaining
  non-scanning, non-remediating, and governed by workspace read permits.
- Added `security.assess` as the single bounded read-only entry point for a
  fixed-marker evidence pass across the Security Domain lifecycle. It does not
  execute scanners, assign findings, or certify compliance.
- Extended the read-only `security-domain` Harness with explicit discovery,
  attack-path, fix-planning, fix-verification, vulnerability-writeup, and
  finding-tracking workflow Genes modeled on the same staged lifecycle.
  These Genes collect local evidence only; they do not execute scanners,
  assign findings, modify code, or mutate a finding database.

## v2.0.0-beta.4

- Expanded the read-only `security-domain` Harness with bounded scan,
  threat-model, triage, validation, and hardening evidence Genes. These
  workflow names mirror the security assessment lifecycle without claiming
  scanner execution, verdict assignment, or remediation.
- Added the read-only `debugging-domain` Harness with bounded failure, test,
  regression, diagnostic, inventory, and guidance Genes. It reports fixed
  local evidence without running tests, changing files, or assigning root
  causes.

## v2.0.0-beta.3

- Added the read-only `security-domain` Harness with bounded audit,
  dependency-marker, policy-marker, and static-guidance Genes. Its evidence
  workflows use only the existing workspace-scoped filesystem read path and
  do not claim complete vulnerability scanning or automated remediation.
- Added a typed TypeScript client for invoking the existing JSON CLI contract
  without shell interpolation or a second runtime authority.
- Added governed `pandora mcp catalog` and `pandora mcp call` commands for
  one-shot local stdio discovery and invocation. Both require explicit
  operator consent, persist their bounded runtime-event batches atomically,
  and keep the existing Rust runtime as the authority.
- Added a typed TypeScript release-target boundary for the Node/Bun launcher;
  it shares platform and SemVer selection with the verified native download
  path without moving runtime execution out of Rust.
- CLI executions now persist a redacted rollout summary in the existing
  session store, and `pandora rollout inspect` exposes its verified scope
  digests without replaying or authorizing effects.
- Added read-only `pandora evaluation inspect`, exposing durable evaluation
  receipts and aggregate result counts by session or execution without replay
  or authority changes.
- Agent context assembly can now use a bounded, atomic `context-cache.json`
  in the configured data directory. Cache records remain scope- and
  provenance-bound; corrupt, stale, oversized, or sensitive records are
  ignored and the cache cannot authorize execution.
- Added a bounded `GraphIntelligenceEngine` with deterministic Code, Knowledge,
  Review, and Architecture evidence projections. Graph snapshots are scoped,
  provenance-digested, and descriptive only; the engine has no direct effect
  authority.
- Added a capped deterministic golden-set evaluator. It compares only redacted
  outcomes, returns per-case evaluation evidence and a stable report digest, and
  never includes expected output in the report.
- Added the durable local Fleet control plane with deterministic capability
  dispatch, worker identities, bounded leases and budgets, quarantine, revoke,
  kill, and expiry transitions. Fleet leases do not issue effect permits or
  replace the ReferenceMonitor.
- L1 and approved L2 memory now persist in the scoped session database while
  L0 traces remain an expiring RAM-only ring buffer. Existing canonical L1
  execution evidence migrates into the durable schema.
- Durable memory recall is bounded, scope-checked, revocation-aware, and
  fail-closed on corrupt records. Revocation tombstones survive record
  compaction and retain append-only audit evidence.
- Added the built-in `design-domain` Harness with bounded workspace inventory,
  token-marker evidence, source inspection, source comparison, accessibility
  evidence, and static guidance Genes. Every effect is workspace-scoped and
  read-only.
- `coordination-meta` can now compose Coding, Research, Design, Operations, and
  Security Domains.
  Design Genes use the existing ToolEngine, slash-command catalog, Parliament,
  Reference Monitor, executors, receipts, and runtime events.
- Added the built-in `research-domain` Harness with bounded evidence inventory,
  search, source reading, source comparison, citation-marker inventory, and
  static guidance Genes. Every effect is workspace-scoped and read-only.
- `coordination-meta` can now compose the Coding and Research Domains. Research
  Genes are available to declarative Domain profiles, the ToolEngine, canonical
  slash commands, and the existing governed execution path.
- Added scoped `pandora memory` commands for durable recall, audit, revocation,
  and approval-bound L1-to-L2 promotion. L0 remains process-local.

## v2.0.0-beta.2

This beta advances the CLI release line with durable context assembly caching,
bounded graph intelligence, deterministic golden-set evaluation, and the local
Fleet control plane. It exposes these capabilities through stable JSON commands
while preserving Pandora's existing governance and effect-authority boundary.

### Shipped

- Context assembly can use a bounded, atomic, scope- and provenance-bound cache.
- Code, Knowledge, Review, and Architecture graph projections are available
  through the `pandora graph` CLI command.
- Golden-set evaluation is available through `pandora evaluation golden`, with
  bounded inputs, redacted outcome comparison, and stable report digests.
- Local Fleet identities, leases, capability dispatch, budgets, quarantine,
  revocation, and kill controls are available through `pandora fleet`.
- Fleet leases and graph/evaluation projections remain descriptive or
  scheduling evidence; none can issue effect permits or replace the
  ReferenceMonitor.
- Added `pandora package validate` for non-persisting manifest, hash, and
  import-free WASM Gene validation before local admission.

## v2.0.0-beta.1

This beta consolidates Pandora's governed CLI, local service, extensibility,
isolation, research evolution, and release boundaries without changing the
one-shot effect-authority model.

### Shipped

- Every effect request now carries an immutable `ExecutionProfile` that binds
  runtime, platform, policy, workspace, containment, executor, provider,
  Harness, Gene, Skill, and tool-catalog evidence without storing credentials
  or unrestricted paths.
- Executor containment is reported as bounded evidence. Partial and unavailable
  boundaries remain explicit and cannot grant authority.
- Declarative lifecycle hooks can veto work before Parliament evaluation. They
  cannot mutate requests, resolve approvals, execute code, or issue permits.
- An authenticated loopback RPC service exposes the existing scoped runtime
  facade through a private local bearer token; it does not create a second
  execution path.
- Local MCP stdio supports the modern `2026-07-28` protocol and an isolated
  legacy `2025-11-25` compatibility path. Spawn and tool calls use the same
  Parliament, ReferenceMonitor, one-shot permit, receipt, and event boundary as
  built-in tools.
- MCP profiles are metadata-only until explicitly started. Catalog revisions,
  schemas, canonical arguments, and exact remote tool names are digest-bound so
  stale or mismatched calls fail before RPC.
- Isolated subagents use scoped identities, durable lifecycle records, bounded
  cooperative controls, managed Git worktrees, exact provider and Harness
  bindings, and explicit cleanup permits. Unknown outcomes are never replayed
  automatically.
- Managed worktree creation and cleanup are effectful operations with canonical
  destinations, no-replace behavior, dirty-worktree preservation, and separate
  one-shot permits.
- Coding runs can use a governed feedback loop that separates verified success,
  retryable failure, policy failure, reflection, and approved adaptation without
  persisting raw model output.
- Canonical context provenance and redacted rollout evidence bind assembled
  context, permits, receipts, policy digests, and replay order without retaining
  hidden reasoning or credentials.
- A research-only population strategy adds bounded candidate populations,
  novelty-aware deterministic selection, train and holdout failure evidence,
  mutation prechecks, atomic generation receipts, and bounded lineage queries.
  It cannot activate candidates or bypass evaluation and governance.
- Exact registry releases can be downloaded, verified, admitted, and locked by
  canonical identity, strict SemVer, artifact hash, trust evidence, dependency
  graph, and runtime compatibility. Admission does not grant execution authority.
- The coding Domain Harness ships bounded analysis, debt, review, measurement,
  guidance, read, search, patch, and verification Genes with namespaced slash
  commands and operation-specific effect requests.
- Execution evaluations are persisted atomically with session events, while
  trajectory, outcome, policy, regression, adversarial, and human evaluation
  remain separate evidence classes.
- Release-critical JSON envelope `0.1` is regression-tested for version, setup,
  doctor, update, rollback, uninstall, and bounded errors.
- Local and published release tests cover clean setup, diagnostics, install,
  upgrade, downgrade, checksum-verified update, rollback, uninstall, workspace
  preservation, and cross-platform reliability baselines.

- Jobs now persist the worker that claims them. Only that worker can finish a
  claimed job. `pandora job mark-interrupted <job-id> --reason "..." --yes`
  records an operator-reviewed, terminal unknown-outcome state without
  replaying or requeueing work.
- `pandora job submit|work|list|inspect|cancel|mark-interrupted` provides a scoped, durable local
  queue. The worker reuses the existing `run` path, can process one job or a
  bounded sequential FIFO batch, persists versioned results, stops at approval
  or failure, and never replays claimed work automatically.
- Direct runs now reject unclassified natural-language tasks instead of routing
  them to an unregistered default Harness; use `run --agent` for those tasks.
- Built-in Harness construction and lookup now use one shared catalog across the
  runtime and CLI.
- Source and Meta Harnesses now fail closed as non-runnable execution targets;
  only Domain Harnesses can enter Gene planning.
- Source Harness manifests now bind both their constitutional service and its
  exact implementation version; discovery exposes both values for inspection.
- Harnesses, Genes, and Source service bindings now require exact SemVer;
  Harness manifests reject duplicate owned Gene declarations.
- Harness discovery now reports each Harness execution mode explicitly:
  Source Harnesses augment the system, Meta Harnesses compose Domains, and
  Domain Harnesses are runnable only when they own executable Genes.
- `AdaptiveEngine::select_with_efficiency` can opt into bounded cost, latency, or
  verified-completion ranking while retaining the existing score-based default.
- Efficiency ranking can also minimize measured token usage per task class.
- CLI runs persist bounded token, latency, completion, and explicitly known
  cost evidence; `pandora efficiency rank` exposes read-only rankings.
- Provider profiles can declare input and output token rates so direct runs
  record auditable cost evidence; fallback runs remain cost-unknown.
- Agent and planning runs can opt into evidence-based provider selection with
  `--optimize cost|latency|tokens|certainty`; missing evidence preserves the
  active provider without changing configuration or policy.
- Invalid agent tool arguments now return bounded, actionable feedback so a
  provider can repair the call without exposing raw argument values.
- Agent tool outputs and restored tool messages are framed as untrusted data
  before a provider sees them; unbound tool history is rejected.
- The terminal TUI can approve or deny a pending effect and resume the exact
  approved task through the existing approval and execution path.
- The TUI bounds its in-memory transcript and task history for long-running
  terminal sessions.
- `pandora harness inspect` resolves canonical IDs from the built-in catalog;
  `coding` remains a compatibility alias for `coding-domain`.
- `pandora harness run` resolves executable Domain Harnesses from the catalog
  and reports metadata-only Source Harnesses as non-runnable.
- Added a built-in `coordination-meta` Meta Harness with an explicit Domain
  membership list and handoff ceiling.
- Meta-bound orchestration plans are rejected before registration when they
  reference an undeclared Domain Harness or exceed the Meta handoff ceiling.
- Custom Meta package profiles now carry validated Domain membership and
  handoff limits, are admitted without runtime authority, and expose their
  composition through Harness inspection.
- Custom Meta package admission now rejects missing, non-Domain, or ambiguous
  Domain members before recording the composition profile.
- Package removal now refuses to delete a Domain Harness that an admitted Meta
  Harness composition still names.
- Custom Domain and Meta profiles cannot shadow a built-in Harness identity.
- `pandora harness inspect` now resolves admitted Meta profiles by exact version
  and reports their non-runnable composition boundary.
- Declarative Domain Harness profiles can now be admitted when they declare at
  least one required available Gene dependency; admission remains
  non-authoritative and grants no runtime authority.
- Admitted Domain profiles can now be selected by exact version when their
  dependencies resolve to built-in Gene implementations; package artifacts
  remain non-executable.
- `pandora harness inspect` can now resolve an admitted Domain profile by exact
  version without enabling it or granting runtime authority.
- Local package admission now persists verified package metadata and artifact
  evidence in `packages.sqlite3`; `pandora package list` and `inspect` reload
  only records that pass the same validation boundary.
- Admitted packages can now be removed by exact ID and version with a dry-run or
  explicit confirmation; required dependents block removal transactionally.
- Package manifests and dependencies now require strict SemVer, while preserving
  prerelease and build metadata as part of exact package identity.
- Package admission now enforces the declared non-wildcard `pandora` SemVer
  compatibility requirement against the running runtime, including standard
  prerelease matching rules.
- Package-store reload now revalidates every deserialized manifest field, so
  malformed versions, dependency records, hashes, text fields, or control data
  in trust fields fail closed instead of becoming local package state.
- Local package admission now verifies `verified` Ed25519 evidence over the
  exact package identity and artifact hash; `official` claims remain rejected
  until a publisher trust root is configured.
- Skill lifecycle state now exposes only the states implemented by the local
  engine; stale `verified` and `installed` state values fail closed on reload.
- ReferenceMonitor now recomputes Parliament decisions from its bound policy,
  so a caller cannot widen effect authority with a fabricated decision.
- Filesystem executors are now bound to one canonical workspace before they can
  read, search, or write; the unbound compatibility constructor fails closed.
- Provider permits now bind the selected protocol, endpoint, and credential
  reference as well as the canonical model request payload, including messages,
  tools, model, token budget, timeout, and trace identifiers.
- Agent, planning, and provider-test model requests now pass through the runtime
  ProviderExecutor and require a scoped, one-shot `provider.invoke` permit.
- Agent runs now assemble the constitutional prompt and enabled Skill guidance
  through ContextEngine, returning a bounded context receipt in JSON. Locally
  admitted Skill guidance is sensitive, non-cacheable, and never authoritative.
- ContextEngine now reuses exact constitutional and active-plan assemblies
  within the current process. Cache identity includes the tenant, workspace,
  session, provider, model, policy, token budget, fragment contents, metadata,
  and expiry; sensitive, dynamic, and oversized context bypasses storage.
- Session events now retain their explicit recording time. `pandora session
  inspect` derives a bounded trace and reliability summary from those canonical
  records without exposing event payloads or inventing missing measurements.
- CLI runs now add bounded, redacted L1 execution evidence to the private
  session ledger. Session inspection exposes only the record count; automatic
  context reuse is limited to up to eight canonical records from the exact
  same session and provider.
- Recalled L1 evidence is non-cacheable descriptive context. AgentLoop rejects
  a mismatched evidence scope before contacting a provider.

## v2.0.0-alpha.6

This prerelease connects explicitly enabled Skills to agent context without
changing the governed execution model.

### Shipped

- Enabled Skills contribute bounded reference guidance to the agent system
  instruction; disabled and suspended Skills are excluded.
- Skill guidance is size-limited and fails closed rather than being silently
  truncated.
- Skill text is never treated as permission, policy, approval, or script
  execution authority, and is not persisted in the session transcript.

## v2.0.0-alpha.5

This prerelease adds local Skill admission without changing the governed
execution model.

### Shipped

- `pandora skill install <local-skill-directory>` validates and stages one
  local `SKILL.md` package under the configured Skills root.
- Installed Skills preserve their source, reject duplicate IDs and symlinks,
  and start disabled until explicitly enabled.
- Shell completions and CLI documentation include the install command.

## v2.0.0-alpha.4

This prerelease adds interactive terminal clients without changing the
governed execution model.

### Shipped

- `pandora chat` provides a line-oriented interactive agent session.
- `pandora tui` provides a full-screen terminal client with task history,
  session status, transcript clearing, and clean terminal restoration.
- Both clients reuse the existing AgentLoop, session scope, approval records,
  and governed effect path.

## v2.0.0-alpha.3

This prerelease extends the CLI foundation without changing the governed
execution model.

### Shipped

- The built-in `core-source` Source Harness appears in discovery and binds the
  `pandora-runtime` constitutional service without adding a runnable Gene.
- `pandora session inspect` reports scoped session metadata and bounded event
  counts without exposing stored event payloads.
- Shell completion keeps session subcommands under `pandora session` across
  Bash, Fish, PowerShell, and Zsh.

## v2.0.0-alpha.2

This prerelease updates the CLI foundation without changing the governed
execution model.

### Shipped

- Named provider profiles with one-run provider selection and isolated
  credential environment-variable names.
- Bounded agent transcripts with session resume and one-shot continuation of
  approved pending actions.
- Atomic private configuration writes that preserve the previous valid file on
  replacement failure.
- A `pandora-agent` Node/Bun launcher tarball attached to the GitHub release;
  it safely replaces stale cached binaries on Windows and forwards to verified
  native artifacts.
- GitHub Actions workflows refreshed to current action runtimes for the
  Windows, macOS, and Linux release checks.

This release remains CLI-only and is a prerelease. Native artifacts are valid
only when the tagged release workflow and its checksums, signature, SBOM, and
provenance assets are present.

## v2.0.0-alpha.1

This is the first plain-SemVer Pandora Agent CLI prerelease.

### Shipped

- Parliament, Shadow Council, Harness, and Gene contracts with one governed execution path.
- Reference-monitor permits, one-shot approvals, effect receipts, and workspace-scoped executors.
- Coding Domain Harness operations for reading, searching, reviewing, patching, and verification.
- Bounded agent mode with strict tool schemas, turn and tool budgets, provider continuation, and session events.
- OpenAI-compatible provider configuration, connectivity testing, structured-output validation, and repair handling.
- SQLite-backed sessions, configuration migration, shell completions, diagnostics, verified updates, rollback, and uninstall.
- Native CLI release workflow for Windows x64, Linux x64, and macOS Intel and Apple Silicon.
- Checksum verification, optional Cosign verification, SPDX SBOM generation, and GitHub build provenance.

### Scope

- This prerelease supports the CLI only.
- Read-only work can run without approval. Filesystem writes and verification processes require an exact approval before execution.
- Skills are local, disabled by default, and cannot execute scripts outside the governed ToolEngine path.
- Desktop, mobile, remote execution, and marketplace clients are outside this release.

This release is a prerelease. Its native artifacts are valid only when the tagged
release workflow and its platform checks have completed successfully.
