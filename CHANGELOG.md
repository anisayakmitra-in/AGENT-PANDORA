# Changelog

## Unreleased

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
- Provider permits now bind the canonical model request payload, including its
  messages, tools, model, token budget, timeout, and trace identifiers.
- Agent, planning, and provider-test model requests now pass through the runtime
  ProviderExecutor and require a scoped, one-shot `provider.invoke` permit.
- Agent runs now assemble the constitutional prompt and enabled Skill guidance
  through ContextEngine, returning a bounded context receipt in JSON. Locally
  admitted Skill guidance is sensitive, non-cacheable, and never authoritative.
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
- A public `pandora-agent` npm/Bun launcher that safely replaces stale cached
  binaries on Windows and forwards to verified native artifacts.
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
