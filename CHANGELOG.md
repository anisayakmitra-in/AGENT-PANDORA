# Changelog

## Unreleased

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
- `coordination-meta` can now compose Coding, Research, and Design Domains.
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
