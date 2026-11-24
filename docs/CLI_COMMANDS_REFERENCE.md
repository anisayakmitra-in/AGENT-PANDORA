# Pandora CLI Commands Reference

Complete command and flag reference for the `pandora` binary.

---

## Root Commands Index

| Command | Summary | Key Subcommands |
|---|---|---|
| `pandora` | Interactive TUI dashboard | — |
| `pandora setup` | Onboarding setup wizard & provider initialization | `--provider`, `--model`, `--api-key` |
| `pandora chat` | Interactive REPL chat session | `--model`, `--mode`, `--provider` |
| `pandora run` | Execute task (agent, plan, or evaluation) | `--agent`, `--max-turns`, `--mode`, `--strategy`, `--lats-rollouts`, `--lats-depth`, `--puct-c`, `--escalation-threshold`, `--json` |
| `pandora prompt` | Declarative reusable prompt presets | `save`, `list`, `inspect`, `apply`, `delete` |
| `pandora mcp` | Model Context Protocol server management | `list`, `inspect`, `add`, `add-http`, `login`, `logout`, `remove`, `catalog`, `call` |
| `pandora provider` | Model provider configuration & testing | `list`, `set`, `add`, `remove`, `use`, `login`, `accounts`, `test` |
| `pandora session` | Durable session inspection & resumption | `list`, `resume`, `inspect`, `events` |
| `pandora secret` | Encrypted SecretVault credentials | `set`, `list`, `status`, `remove` |
| `pandora cron` | Autonomous background task scheduling | `create`, `list`, `inspect`, `disable`, `remove`, `run` |
| `pandora memory` | Multi-layered memory operations | `recall`, `audit`, `forget`, `compact`, `promote`, `synthesize`, `schedule`, `vfs` |
| `pandora model` / `pandora moira` | Tropical `.pnn` neural decision models & attestation | `inspect`, `doctor`, `status`, `sync`, `calibrate`, `verify-attestation`, `list` |
| `pandora skill` | Procedural skill management | `list`, `inspect`, `install`, `enable`, `disable`, `remove` |
| `pandora package` | WebAssembly Gene package management | `scaffold`, `admit`, `validate`, `sign`, `install`, `lock` |
| `pandora evaluation` | Autonomous evaluation scorecards | `golden`, `inspect`, `scorecard`, `suite`, `regression`, `schedule` |
| `pandora evolution` | Self-improving model & strategy proposals | `generate`, `list`, `inspect`, `submit`, `evaluate`, `approve`, `rollout` |
| `pandora fleet` | Multi-node cluster orchestration | `dashboard`, `list`, `register`, `dispatch`, `supervisor`, `distill` |
| `pandora cockpit` | Real-time multi-agent observability & operations | `status`, `tui`, `web`, `distill` |
| `pandora doctor` | System diagnostic health check | `--fix` |
| `pandora policy` | Inspect active reference monitor policy | `--export` |
| `pandora runtime` | Certified mathematical runtime engines & verification | `conformal`, `topology`, `bandit`, `drift`, `canary-gate`, `tropical`, `mfg`, `pac-bayes`, `assurance` |
| `pandora swarm` | Distributed P2P cross-node swarm mesh & Byzantine consensus | `connect`, `status`, `propose`, `vote`, `sync-gradients` |
| `pandora update` | In-place self-update with signature verification | `--channel`, `--version` |
| `pandora completions` | Generate shell autocompletion scripts | `powershell`, `bash`, `zsh`, `fish` |

---

## Global Options
- `--config <path>`: Override path to `config.json` (defaults to `$PANDORA_CONFIG` or `.pandora/config.json`).
- `--data-dir <path>`: Override state database directory (defaults to `$PANDORA_DATA_DIR` or `.pandora/data`).
- `--workspace <path>`: Target workspace directory (defaults to current working directory).
- `--json`: Format command output as machine-readable JSON.
- `--help`: Display synopsis, subcommands, and flags.
- `--version`: Display binary version.

---

## `pandora model` / `pandora moira`

Manage and audit `.pnn` tropical neural network weights (`model_decision.pnn`, `model_tpan.pnn`) and their hash-chained calibration attestation ledgers (`<pnn>.attestation.jsonl`).

```text
pandora <model|moira> <inspect|doctor|status|sync|calibrate|verify-attestation|list> [options]
```

### Subcommands

- `inspect [--pnn <path>] [--workspace <dir>] [--config <path>] [--data-dir <path>]`
  Inspect active `.pnn` tropical neural weights, layer shapes, SHA-256 digests, and active drivers (`MoiraDecisionDriver` vs `HeuristicDecisionDriver`).
- `doctor | status [--pnn <path>] [--workspace <dir>] [--config <path>] [--data-dir <path>]`
  Verify integrity and runtime readiness of `.pandora/model_tpan.pnn` and `.pandora/model_decision.pnn` (or an explicit `--pnn <path>`).
- `sync [--from <repo-or-url>] [--dest <path>] [--revision <rev>] [--file <name>] [--dry-run] [--workspace <dir>] [--config <path>] [--data-dir <path>]`
  Download (or copy from local path) and SHA-256 verify `model_decision.pnn` (defaults to `anisayakmitra-in/FATE-O-SISTERS`, revision `main`, file `model_decision.pnn`; aliases: `pull`, `download`).
- `calibrate [--pairs <path>] [--pnn <path>] [--workspace <dir>] [--config <path>] [--data-dir <path>] [--dry-run]`
  Calibrate `model_decision.pnn` from `DpoPair` records (JSON array or JSONL, defaulting to `<workspace>/.pandora/dpo_pairs.jsonl` when present), gated by `DecisionBenchmarkSuite::default_real_task_traces()`, deterministic non-regression, and `BayesianCanaryGate::default()`. When promoted and not `--dry-run`, atomically writes `<pnn>` and appends a `CalibrationAttestationReceipt` to `<pnn>.attestation.jsonl`.
- `verify-attestation [--pnn <path>] [--workspace <dir>] [--config <path>] [--data-dir <path>]`
  Cryptographically audit the hash-chained `.pnn` calibration attestation ledger (`<pnn>.attestation.jsonl`) and verify that the live `.pnn` weights file exists and matches the latest attested `candidate_digest`.
- `list`
  Delegates to `pandora models` listing.

### `verify-attestation` Behavior & JSON Output

1. Resolves `--pnn <path>` (default: `<workspace>/.pandora/model_decision.pnn`, where `<workspace>` resolves from `--workspace`, `$PANDORA_WORKSPACE`, `config.workspace_dir()`, or `.`) and ledger `<pnn>.attestation.jsonl`.
2. Fails closed if `<pnn>.attestation.jsonl` is missing, empty, contains malformed JSON, or fails `CalibrationAttestationReceipt::verify_chain` (contiguous `sequence`, finite/bit-exact `lower_bound_95_bits`, SHA-256 `commitment`, and `GENESIS_PREV_HASH` / `prev_receipt_hash` -> `chain_hash` links).
3. Fails closed with `valid: false` and `pnn_digest_matches: false` if the live `.pnn` weights file is absent, fails to load, or has `net.sha256_digest != latest.candidate_digest`.

Success JSON payload (`--json`):

```json
{
  "version": "4.0.0",
  "command": "model verify-attestation",
  "valid": true,
  "chain_length": 1,
  "latest_sequence": 0,
  "latest_commitment": "<sha256>",
  "latest_chain_hash": "<sha256>",
  "pnn_digest_matches": true,
  "path": ".pandora/model_decision.pnn",
  "receipt_path": ".pandora/model_decision.pnn.attestation.jsonl"
}
```

---

## `pandora run` LATS Strategy & Telemetry

When `--strategy lats` is passed to `pandora run`, `AgentLoop` pairs the System 1 `TropicalDecisionDriver` prior with System 2 Monte Carlo Tree Search (`LatsEngine`) for tool-safety escalation gating.

### LATS Flags

- `--strategy <react|reflexion|lats>`: Select execution strategy (default `react`).
- `--lats-rollouts <1-100>`: Number of LATS rollouts (requires `--strategy lats`, default `10`).
- `--lats-depth <1-16>`: Maximum search tree depth (requires `--strategy lats`, default `4`).
- `--puct-c <float>`: Positive PUCT exploration constant (requires `--strategy lats`, default `1.4`).
- `--escalation-threshold <float>`: Entropy/margin escalation threshold in `0.0..=1.0` (requires `--strategy lats`, defaults to `tropical_driver.uncertainty_threshold()`).
- `--auto-calibrate <k>`: Enables Phase 9 autonomous in-loop `.pnn` hot-reload every `k` emitted `DpoPair` overrides (requires `--strategy lats`).
- `--dpo-output <path>`: File path to append emitted `DpoPair` records (JSONL) during execution.

### `lats_metrics` JSON Telemetry

With `--json` and `--strategy lats`, `pandora run` emits a `lats_metrics` object containing:

- `rollouts`, `max_depth_reached`, `nodes_created`, `best_action`, `best_value`, `escalated`
- `turn_rollouts`, `turn_escalations`, `dpo_pairs_emitted`, `memo_hits`
- `pnn_hot_reloads`, `active_pnn_digest`, `lats_memo_hits`
- `turns[]`: per-turn entries with `turn`, `tool_name`, `escalated`, `rollouts`, `max_depth_reached`, `nodes_created`, `best_action`, `best_value`, `reflex_action`, `dpo_pair_emitted`, and `memo_hit`.

---

## `pandora memory vfs`

Inspect and search the Virtual Context Filesystem (`ContextVfs`):

- `pandora memory vfs ls [<uri>] [--workspace <id>]`
- `pandora memory vfs cat <uri> [--workspace <id>]`
- `pandora memory vfs search <query> [--limit <n>] [--tier <tier>] [--intent <intent>] [--reindex] [--no-cache] [--tropical]`
- `pandora memory vfs tropical-search <query> [--limit <n>] [--tier <tier>] [--intent <intent>] [--reindex] [--no-cache]`
  Runs `search_tropical_with_options` (equivalent to `vfs search --tropical`) and returns `query`, `results[]` (`uri`, `title`, `snippet`, `sparse_score`, `dense_score`, `hybrid_score`, `tier`, `intent`), and `count`.
- `pandora memory vfs index [--rebuild] [--output <path>]`
- `pandora memory vfs stats`
- `pandora memory vfs hnsw-build` / `pandora memory vfs hnsw-search <query>`

---

## Runtime LATS Calibration & Memoization APIs (`pandora-runtime`)

These two capabilities live in `crates/pandora-runtime` and govern in-loop LATS execution:

- **`LatsLoopConfig::with_auto_calibrate(batch_k: usize) -> Self`** (`crates/pandora-runtime/src/agent_loop.rs`, re-exported from `pandora_runtime`):
  Enables Phase 9 autonomous in-loop `.pnn` hot-reload every `batch_k` emitted `DpoPair` overrides (`auto_calibrate_threshold = (batch_k > 0).then_some(batch_k)`; defaults to `None`). Whenever System 2 MCTS overrides the System 1 reflex prior (`mcts_act != ref_act`), a `DpoPair` is recorded; every `batch_k` pairs, `try_hot_reload_pnn` calibrates a candidate network via `TropicalDecisionDriver::calibrate_network_from_pairs(..., 0.05)` and evaluates it against `DecisionBenchmarkSuite::default_real_task_traces()`, `BayesianCanaryGate::default()`, and deterministic non-regression. On promotion, `active_tropical_driver` is updated in memory, `AgentRunSummary::pnn_hot_reloads()` increments, and the active `LatsPrefixMemoCache` is cleared.
  - **CLI exposure status**: `pandora run --strategy lats` currently sets `auto_calibrate_threshold: None` (no standalone CLI flag yet; configured via `LatsLoopConfig::with_auto_calibrate` in Rust).
- **`LatsPrefixMemoCache`** (`crates/pandora-runtime/src/lats_engine.rs`, re-exported from `pandora_runtime`):
  Bounded FIFO prefix memoization cache (`Default` capacity `N_max = 256`; `LatsPrefixMemoCache::new(capacity)` enforces `capacity.max(1)`) keyed by `LatsMemoKey::from_features(tool_name, &DecisionFeatures, pnn_digest)`, which uniformly quantizes the 7D `DecisionFeatures` vector into 4-bit integers `{0..=15}` ($L_\infty$ error $\le 1/30$; non-finite values clamp to `0.0`). Provides `new`, `get`, `insert`, `clear` (clears entries/order while preserving cumulative `hits`/`misses`), `len`, `is_empty`, `hits`, and `misses`.
  - **Runtime & CLI exposure**: Enabled by default via `LatsLoopConfig::with_memo(true)` (`enable_memo: true` in `pandora run --strategy lats`). Cache hits bypass `search_with_tropical_prompt` (`rollouts: 0`, `memo_hit: true`), cache entries are cleared upon `.pnn` hot-reload, and cumulative hits are surfaced via `AgentRunSummary::lats_memo_hits()` in Rust and `lats_metrics.memo_hits` / `turns[].memo_hit` in `pandora run --strategy lats --json`.

---

## `pandora fleet distill` & `pandora cockpit distill`

Aggregate cross-session System 2 MCTS `DpoPair` execution traces into cryptographically signed `MemoryCapsule` packages and `.pnn` checkpoints:

```text
pandora fleet distill [--sessions-dir <path>] [--dpo-input <path>] [--capsule-out <path>] [--pnn-out <path>] [--key <path>] [--dry-run]
pandora cockpit distill [--sessions-dir <path>] [--dpo-input <path>] [--capsule-out <path>] [--pnn-out <path>] [--key <path>] [--dry-run]
```

- `--sessions-dir <path>`: Source directory containing session trace logs.
- `--dpo-input <path>`: Direct JSONL file containing emitted `DpoPair` samples.
- `--capsule-out <path>`: Output destination for signed `MemoryCapsule` package.
- `--pnn-out <path>`: Output destination for calibrated `.pnn` model checkpoint.
- `--key <path>`: Path to Ed25519 signing key hex.
- `--dry-run`: Evaluate distillation and canary gating without persisting artifacts to disk.

---

## `pandora runtime`

Execute mathematical assurance engines and certified runtime subcomponents:

```text
pandora runtime <conformal|topology|bandit|drift|canary-gate|tropical|scheduler|...> [options]
```

### Subcommands

- `conformal [--samples <n>] [--alpha <f64>] [--gamma <f64>] [--steps <T>] [--json]`
  Split Conformal Prediction & Adaptive Conformal Inference (ACI) uncertainty calibration (Theorems 29 & 30).
- `topology | spectral [--nodes <n>] [--bridge-weight <f64>] [--threshold <f64>] [--json]`
  Spectral Graph Partitioning, algebraic connectivity Fiedler value $\lambda_2(L)$, and Cheeger sweep-cut conductance $h(G)$ (Theorems 31 & 32).
- `bandit | exp3 [--arms <k>] [--horizon <T>] [--gamma <f64>] [--json]`
  Exp3 adversarial multi-armed bandit routing with importance-weighted reward estimation and minimax sublinear regret bounds (Theorems 33 & 34).
- `mfg | mean-field [--resources <k>] [--base-costs <c1,c2...>] [--congestion-slopes <b1,b2...>] [--temperature <tau>] [--damping <alpha>] [--iterations <n>] [--tolerance <eps>] [--json]`
  Mean-Field Game (MFG) resource congestion control, Banach fixed-point Nash equilibrium convergence, and Price of Anarchy $\mathrm{PoA} \le 4/3$ verification (Theorems 35 & 36).
- `pac-bayes | pacbayes [--samples <m>] [--empirical-risk <r>] [--kl <d>] [--target-risk <eps>] [--delta <d>] [--hypotheses <n>] [--losses <l1,l2...>] [--json]`
  PAC-Bayes policy generalization guarantee, McAllester / Maurer empirical risk inversion, Catoni Gibbs optimal temperature $\beta^\star$, and non-vacuous candidate promotion gate (Theorems 37 & 38).
- `tropical | max-plus [--nodes <n>] [--topology <pipeline|cycle|custom>] [--sla-threshold <ms>] [--matrix <json>] [--json]`
  Tropical max-plus $((\mathbb{R} \cup \{-\infty\}, \oplus, \otimes))$ DAG algebra, Kleene star makespan closure $A^*$, and Karp cycle mean $\lambda(G)$ deadlock guard (Theorems 39 & 40).
- `assurance | assurance-receipt [--run-id <id>] [--json]`
  Generate and verify a 12-Engine Unified Mathematical Assurance Receipt covering all certified theorems (Theorems 17–40) with SHA-256 state chaining and Ed25519 signature verification.
- `daemon-guard | guard [--interval-ms <n>] [--iterations <n>] [--store <path>] [--once] [--circuit-threshold <n>] [--cooldown-secs <n>] [--json]`
  Autonomous self-healing daemon guard executing continuous multi-engine telemetry verification ticks, commit to Merkle audit store, and circuit breaker evaluation.
- `federated | federated-policy [--agents <n>] [--byzantine <f>] [--steps <k>] [--learning-rate <lr>] [--momentum <m>] [--max-norm <norm>] [--seed <seed>] [--json]`
  Byzantine-fault-tolerant federated reinforcement learning policy aggregation with coordinate-wise trimmed mean and Beta-reputation quarantine monitor (Theorems 45 & 46).

---

## `pandora swarm`

Distributed cross-node peer-to-peer swarm mesh, Byzantine patch consensus, and federated policy optimization (Theorem 47):

```text
pandora swarm <connect|status|propose|vote|sync-gradients> [options]
```

### Subcommands

- `connect --peer <addr> [--node-id <id>] [--json]`
  Connect to a cluster peer node over authenticated Ed25519 channel and update mesh topology.
- `status [--json]`
  Inspect active P2P mesh peers, cluster epoch, BFT quorum parameters ($N \ge 3f + 1$, $Q = 2f + 1$), active proposals, decisions, and 6D federated policy parameters $\boldsymbol{\theta}$.
- `propose --goal <text> [--diff <content|path>] [--title <text>] [--author <agent>] [--proposal-id <id>] [--json]`
  Broadcast candidate code patch proposal across the cluster for multi-agent evaluation.
- `vote --proposal <id> [--approve|--reject] [--voter <id>] [--pass <f64>] [--confidence <f64>] [--compactness <f64>] [--invariant <f64>] [--json]`
  Cast an Ed25519-signed Byzantine consensus vote on a candidate patch with equivocation detection and centroid scoring; commits `PANDORA_PATCH_CERT_V1:...` upon supermajority quorum.
- `sync-gradients [--epoch <n>] [--loss <f64>] [--samples <n>] [--json]`
  Disseminate local empirical gradients and aggregate cross-cluster updates using coordinate-wise trimmed mean to update global policy parameters $\boldsymbol{\theta}$.




