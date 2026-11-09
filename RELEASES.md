# Release policy

Pandora ships as a native Rust CLI and interactive TUI.

Pandora uses plain SemVer tags and release titles. Stable and prerelease
versions use the same tag format; prereleases use neutral suffixes such as
`v2.0.0-alpha.1`, `v2.0.0-beta.1`, and `v2.0.0-rc.1`.

Existing legacy preview tags remain immutable.

The older `v2.0.0-anubis.1`, `v2.0.0-anubis.2`, and `v2.0.0-anubis.3` tags are
archived previews from an earlier naming scheme. They remain available for
history and reproducibility, but they are not the active release line or
recommended install targets.

The canonical published prerelease tags for this line are `v2.0.0-alpha.1`
through `v2.0.0-alpha.6`, followed by `v2.0.0-beta.1` through `v2.0.0-beta.10`,
and the stable releases `v2.0.0`, `v3.0.0`, `v3.0.1`, `v3.0.2`, `v3.0.3`, `v3.0.4`, `v3.0.5`, `v3.0.6`, `v3.0.7`, `v3.0.8`, `v3.0.9`, and `v3.0.10`. The active release identity
is `v3.0.10`.

## Source-bound release scope

Every new tag is evaluated against the exact tracked `release-scope.json` at its
source commit. This is a release contract, not a mutable workflow input:

`cli-only` is the only scope. It publishes the native CLI, launcher, installers,
checksums, signature, SBOM, provenance, notes, and CLI lifecycle evidence.
The release gate asserts
`cli-only` explicitly rather than assuming it, and the policy rejects any
other value.

Release-candidate and stable use the same `cli-only` boundary as alpha and beta.
No release channel requires a platform signature: binaries are not
Authenticode- or codesign-signed and are not notarized.

## v3.0.10

This release implements Phase 23 of the mathematical runtime and governance roadmap: the Federated Policy Engine with reputation-weighted coordinate-wise trimmed mean gradient aggregation (Theorem 45) and dynamic Beta-conjugate Thompson sampling reputation with automated quarantine (Theorem 46) documented in `docs/math/phase23_federated_policy_and_reputation.md`, the Live Interactive Multi-Agent Autopilot Session orchestrator (`pandora cockpit autopilot`, `/autopilot`, and TUI `[A]`), and the Web / Mobile Remote Telemetry Dashboard micro-server (`pandora cockpit web --port 8080 --bind 0.0.0.0`) delivering real-time SSE telemetry streams and mobile/desktop responsive companion control under strict `#![forbid(unsafe_code)]`.

## v3.0.9

This release implements Phase 22 of the mathematical runtime and governance roadmap: the Interactive Ratatui Mission-Control Cockpit TUI (dynamically switchable 9-panel and 12-panel matrix layouts, keyboard navigation, drill-down telemetry modal, and headless snapshot fallbacks), the Swarm Byzantine Code Consensus Engine (coordinate-wise trimmed mean centroid and Krum outlier filtering over 4D metric quality space under Theorem 43), and the Pexer Multiplexer IPC Automation Engine (automated grid tiling, dry-run validation, and cross-platform script synthesis under Theorem 44) documented in `docs/math/phase22_cockpit_tui_and_consensus.md`.

## v3.0.8

This release implements Phase 21 of the mathematical runtime and governance roadmap: the 9/12-Panel Mission-Control Cockpit monitoring all runtime settings, harnesses, genes, and mathematical engines across Phases 1–20, and the Parallel Multi-Agent Swarm Collaboration Orchestrator supporting external coding agents (Antigravity, Claude Code, Codex, Gemini, Cursor, Devin, Cline, Hermes, Qwen) multiplexed via `pexer.exe` CLI/IPC or headless fallbacks, backed by Theorem 41 (Multi-Agent Byzantine Consensus Quorum) and Theorem 42 (Bounded Orchestration Latency and Permission Convergence) in `docs/math/cockpit_multi_agent_collaboration.md`.

## v3.0.7

This release implements Phases 18 through 20 of the mathematical runtime and governance roadmap (Discrete McKean-Vlasov Mean-Field Game equilibrium and Price-of-Anarchy bounding, McAllester/Maurer Empirical-Bernstein PAC-Bayesian generalization risk certificates and Catoni Gibbs posterior policy promotion gating, and Max-Plus semiring tropical DAG Kleene star makespan and Karp maximum cycle mean SLA throughput calculus) along with the expanded 12-Engine Unified Assurance Receipt pipeline wired directly into the live `GatewayController`, autonomous self-healing daemon guard loop (`pandora runtime daemon-guard`), and append-only hash-chained audit ledger checkpoints verified by `pandora audit verify` backed by 40 formal theorems across 20 phases in `docs/math/`.

## v3.0.6

This release implements Phases 15 through 17 of the mathematical runtime and governance roadmap (Split and Adaptive Conformal Prediction uncertainty calibration, Spectral Graph Laplacian Fiedler algebraic connectivity and Cheeger conductance certification, and Log-Domain Exp3 adversarial multi-armed bandit routing) along with the Unified Mathematical Assurance Receipt pipeline (`pandora runtime conformal|topology|bandit|assurance`) backed by 34 formal theorems across 17 phases in `docs/math/`.

## v3.0.5

This release completes CLI subcommand wiring across Phases 6 through 14 (`pandora models route`, `pandora audit`, `pandora privacy`, `pandora zk`, `pandora runtime speculative|kv-cache|scheduler|byzantine|drift|canary-gate`, and `pandora hardware scheduler`) and implements the next-generation mathematical runtime engines for Phases 9 through 14 (Rényi Differential Privacy accountant, Goldilocks-field Zero-Knowledge succinct state attestation, Blumofe-Leiserson adaptive work-stealing scheduler, Byzantine-resilient Multi-Krum & trimmed-mean consensus aggregator, log-domain Sinkhorn-Knopp entropic optimal transport semantic drift detector, and Ville martingale anytime-valid canary gate) backed by 28 formal theorems in `docs/math/`.

## v3.0.4

This release introduces direct CLI command and daemon endpoint wiring across Phases 2 through 5 (WebSocket RFC 6455 GET `/v1/ws` streaming daemon endpoint, epidemic gossip rounds and Bloom filter routing under `pandora peer gossip`, BFT quorum checks and DAG scheduling under `pandora fleet quorum`, Lyapunov-damped force-directed memory graph inspection under `pandora graph visual` and TUI Memory tab, and Roofline batch similarity and hardware benchmark telemetry under `pandora hardware`), alongside next-generation mathematical roadmap implementations (Phase 6 Contextual Thompson Sampling and UCB1 Multi-Armed Bandit model router with proven $O(\sqrt{K T \ln T})$ regret bounds, Phase 7 Merkle-DAG cryptographic audit log and verifiable state proofs with proven $O(\log N)$ inclusion proofs, and Phase 8 speculative token decoding and Radix Trie KV cache prefix compression).

## v3.0.3

This release introduces pure ARM64 NEON vector assembly dot-product acceleration (`fmla.4s` 4x unrolled) matching AVX2 throughput on Apple Silicon and Linux AArch64, scalar Int8 dense vector quantization (`QuantizedVector`) shrinking `ContextVfs` index storage by 75% while maintaining >99% cosine similarity ranking precision, SHA-256 incremental buffer diff caching, RFC 2104 compliant HMAC-SHA256 gateway security verification, and complete formal mathematical justifications.

## v3.0.2

This release introduces persistent disk serialization for the semantic vector index (`.pandora/indices/vfs.idx`), a live external messaging gateway service with HTTP webhook and bidirectional TCP socket event streaming (`pandora_bridge.py`), a background LSP diagnostics daemon streaming real-time compilation indicators into the TUI header, and AVX2 SIMD dot product acceleration with scalar fallback.

## v3.0.1

This patch release adds external messaging gateway integration (Telegram long-polling
daemon and Discord webhook dispatching with autonomous cron monitoring) and comprehensive
mouse and keyboard ergonomics to the Pandora interactive TUI workbench (horizontal scroll
tab switching, skills hub modal click-to-toggle, quick-action top bar buttons, direct approval
shortcuts, and readline word editing).

## v3.0.0

This is the major stable release of Pandora v3.0.0-SUNLESS. It updates the coupled Rust,
npm, installer, documentation, and SDK runtime surfaces to `3.0.0` while
retaining the source-bound `cli-only` release scope. It bundles Declarative Structured
Reasoning (DSRs), the Genetic-Pareto (GEPA) Optimizer, the 6-Level Optimization Ladder
($L_0 \to L_5$), Darwin Whole-Agent Genomes with task routing cards, Meta^n Recursive Emergent
Depth with fixed-point $\Omega$ convergence detection, Product Requirement Prompts (PRP)
progressive pipelines with 4-gate verification, and Universal Evolutionary Lineage tracing.

## v2.0.0

This is the first stable release of Pandora v2. It updates the coupled Rust,
npm, installer, documentation, and SDK runtime surfaces to `2.0.0` while
retaining the source-bound `cli-only` release scope. It bundles custom model
provider management, OAuth 2.0 PKCE loopback authentication with multi-account
switching, an interactive spotlight model/provider picker, a TrueColor terminal
mural backdrop, interactive workbench TUI startup, OS-level process confinement,
filesystem TOCTOU hardening, active policy lifecycle hooks, recursive self-improvement
governance, and dependency security updates.

## v2.0.0-beta.10

This is the next coherent prerelease identity. It updates the coupled Rust,
npm, installer, documentation, and SDK runtime surfaces to `2.0.0-beta.10` while
retaining the source-bound `cli-only` release scope. It bundles OS-level process
confinement, filesystem TOCTOU hardening, active policy hook enforcement, the
recursive self-improvement search and governance substrate, and dependency
security updates.

## v2.0.0-beta.9

This is the next coherent prerelease identity. It updates the coupled Rust,
npm, installer, documentation, and SDK runtime surfaces to `2.0.0-beta.9` while
retaining the source-bound `cli-only` release scope. It bundles durable Kanban
coordination, inter-agent society bot mode, session auto-prune and SQLite vacuum
compaction, agent-to-agent JSON-RPC 2.0 protocol v1.0, and browser PKCE OAuth 2.1
SaaS connectors.

## v2.0.0-beta.8

This is the next coherent prerelease identity. It updates the coupled Rust,
npm, desktop, installer, documentation, and SDK runtime surfaces to
`2.0.0-beta.8` while retaining the source-bound `cli-only` release scope. The
identity is prepared for review and is not a public release until its exact tag
workflow completes. The same CLI-only workflow is used for beta, RC, and stable.

## v2.0.0-beta.7

This beta groups bounded self-healing feedback, composition provenance,
verified memory synthesis, persisted graph snapshots, holdout evaluation, and
evidence-only evolution proposal intake. It is a CLI-only prerelease, not a
stable distribution claim.
The release workflow publishes release-evidence.json, linking checksums,
signatures, SBOM, provenance subjects, and platform artifacts.

## v2.0.0-beta.6

This beta adopts MIT licensing for Pandora-owned material, adds the native
Gemini provider adapter, hardens process-tree cleanup for verification and
MCP operations, adds durable evolution records and read-only evolution
inspection, and documents the governed Security Domain workflow. It is a
CLI-only prerelease; no desktop, remote-execution, or marketplace support is
claimed.

## v2.0.0-beta.5

This beta adds the read-only Data Domain Harness and completes the bounded
Security Domain workflow surface with a standard assessment entry point,
deep-scan evidence, and changed-code evidence. It remains a CLI-only
prerelease; these workflows do not execute scanners, modify code, or claim
security certification.

## v2.0.0-beta.4

This beta consolidates the read-only Security and Debugging Domain Harnesses.
It is a CLI-only prerelease with bounded evidence workflows, cross-platform
release verification, and no desktop or remote-execution support claim.

## v2.0.0-beta.3

This beta packages the governed Security Domain Harness, typed TypeScript CLI
client, and the current local MCP and runtime authority improvements. It is a
CLI-only prerelease; the built-in security workflows provide bounded evidence
and do not claim complete vulnerability scanning or automated remediation.

## v2.0.0-beta.2

This beta advances the CLI release line with durable context assembly caching,
bounded graph intelligence, deterministic golden-set evaluation, and the local
Fleet control plane. It also exposes those capabilities through stable JSON
commands for graph projections, evaluation, and Fleet operations. Runtime
authority remains unchanged: these projections and leases cannot issue effect
permits or bypass the ReferenceMonitor.

## v2.0.0-beta.1

This is the current CLI-only beta. It consolidates the governed local runtime,
authenticated loopback service, durable jobs, isolated subagents, local MCP,
signed package admission, bounded research evolution, release lifecycle tests,
and stable JSON automation contract recorded in [CHANGELOG.md](CHANGELOG.md).
Every effect remains bound to Parliament policy, a scoped execution profile,
the ReferenceMonitor, and a one-shot permit.

## v2.0.0-alpha.6

This was the final alpha prerelease. Enabled Skills contribute bounded
reference guidance to agent context. Disabled and suspended Skills are omitted,
oversized guidance fails closed, and Skill text cannot authorize effects,
change policy, satisfy approvals, or execute scripts.

## v2.0.0-alpha.1

This was the first CLI-only prerelease. Its shipped scope is recorded in
[CHANGELOG.md](CHANGELOG.md).

Historical preview tags are retained as immutable compatibility references and
are not the active release line.

It provides the CLI-first foundation:

- Parliament, Shadow Council, Domain Harness, Gene, and governed execution contracts.
- One-shot effect permits, receipts, policy decisions, and workspace-scoped executors.
- A coding Domain Harness with bounded read, search, patch, verify, and review operations.
- Provider manifests, bounded model requests, OpenAI-compatible HTTP transport, tool-call validation, and structured-output repair.
- Windows, macOS, and Linux CI, repository validation, dependency auditing, and release-build checks.

The preview is not a stable release and does not claim desktop support. Native
release artifacts are valid only when the tagged release workflow has
completed and its checksums, signature, SBOM, and provenance assets are
present.

## v2.0.0-alpha.2

This was a CLI-only prerelease. It carried the provider, session,
configuration, launcher, and CI improvements listed in
[CHANGELOG.md](CHANGELOG.md). The public command name remains `pandora`, and
the npm/Bun package name is `pandora-agent`.

The release remains a prerelease and does not claim desktop, mobile, remote,
or marketplace support. Native artifacts are valid only when the tagged
release workflow has completed and its checksums, signature, SBOM, and
provenance assets are present.

## v2.0.0-alpha.3

This CLI-only prerelease added the built-in `core-source` Source Harness,
session inspection, and completion support for session subcommands.

## v2.0.0-alpha.5

This was a previous CLI-only prerelease. It added local Skill package admission
through `pandora skill install`; admitted Skills remain disabled until an
operator enables them. The existing terminal clients and governed execution
path are unchanged.

## v2.0.0-alpha.4

This was a previous CLI-only prerelease. It added the line-oriented `pandora
chat` client and full-screen `pandora tui` client. Both reuse the existing
AgentLoop, session, approval, and governed effect path.

## Version rules

- Patch releases (`v2.0.x`) fix regressions without changing the public contract.
- Minor releases (`v2.x.0`) add compatible capabilities within the current release line.
- Stable major releases change the public contract and increment the major version.
- Prereleases use `-alpha.N`, `-beta.N`, or `-rc.N` suffixes.
- Release tags are immutable. Each release must publish notes, checksums, a signature, an SBOM, a release evidence index, supported-platform results, and rollback instructions.
- A release is not considered stable until local and GitHub checks pass on Windows, macOS, and Linux.
- Release-candidate and stable publication must pass the protected
  `release-publication` environment. Only `v*` tags may deploy through it, and a
  configured human reviewer must approve the publication job.
- A prerelease must not be described as a release candidate or as stable. Alpha
  and beta tags are prereleases; only `v*` tags enter the protected publication
  environment.
- No release channel requires a platform signature. Windows Authenticode, Apple
  Developer ID signing, and notarization were removed from the release process
  on 2026-10-05: Pandora is a terminal CLI distributed through channels that do
  not need an OS signature. Release binaries are therefore **not** OS-signed.
  Integrity is established instead by `checksums.txt`, its keyless cosign
  signature, and GitHub build attestations, all of which are verified in the
  release workflow and by the installers.

## Stable release approval

Alpha and beta tags are marked as prereleases and never enter the protected
publication environment. A release-candidate tag enters it only when
`PANDORA_RELEASE_CANDIDATE_APPROVED` is exactly `1`; a plain SemVer stable tag
likewise requires `PANDORA_STABLE_RELEASE_APPROVED` to be exactly `1`.

No release credential is required to publish. There is no certificate, no
signing identity, and no notarization account. The `release-publication`
environment is an approval gate and nothing else, so there is no secret that
could be leaked, logged, or committed by a signing job.

The published-package smoke job re-downloads checksum-bound artifacts,
authenticates the manifest and the binary with cosign, verifies the GitHub
build attestation, and exercises the CLI install, update, rollback, backup,
restore, and uninstall lifecycle. Current evidence therefore covers native
lifecycle and manifest integrity. It does not cover platform-signature or
notarization evidence, because no release artifact claims one.

## Stable rollback closure

The stable release index never claims rollback closure before published
lifecycle jobs run. After every stable publication, the release workflow waits
for all CLI install/update/backup/restore/rollback/uninstall jobs and
then emits `stable-rollback-evidence.json` bound to the tag, commit, predecessor,
and workflow run.

The first stable release in a compatible line has no older stable artifact. Its
evidence therefore records `pending_first_patch`, not success. For the 2.0 line,
the stable-to-stable gate can close only when `v2.0.1` legitimately rolls back
to the published `v2.0.0` artifacts. A patch tag without a compatible stable
predecessor fails closed.

All channels continue to require the signed checksum manifest, SPDX SBOM,
GitHub provenance, release notes, cross-platform tests, and published
install/update/rollback smoke tests described in
[production readiness](docs/PRODUCTION.md).
