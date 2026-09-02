# Pandora interface design brief

Status: implementation-ready brief for the Open Design workflow

## Product intent

Design a calm, evidence-first desktop command surface for a local governed runtime. The interface should make the next safe action obvious without implying that inspection, planning, or routing grants execution authority. Every state must be understandable when the runtime is offline, waiting for approval, running, interrupted, failed, or complete.

The primary user is a technical operator who needs to submit work, monitor background execution, inspect the exact governance trail, manage modular capabilities, and recover from interrupted operations. The desktop, CLI, and TUI should use the same nouns, statuses, and authority boundaries.

## Information architecture

Use a persistent left rail with three groups:

- **Operate:** Command, Background Runs, Council, Memory, Workflows
- **Inspect:** Harness Lab, Runtime Inventory, Tools, Connections, Audit, Evolution
- **Configure:** Settings

The main area uses a compact top bar with a breadcrumb, runtime connection status, quick-open search, and context actions. The Command view may expose a right-side or bottom Witness Dock for Flow, Evidence, Work, and Browser inspection. The dock placement and size are user preferences and must survive relaunch.

## Core screens

### Command Center

The landing view. Lead with one task composer, a concise explanation of the governed path, selected Harness/provider/model chips, context attachments, and a clear primary action. Show the latest result below the composer with status, receipts, events, and a safe retry action. Never present a retry as reusing a permit.

### Background Runs

Split the page into a run list and a selected-run detail panel. Surface queued, running, interrupted, completed, failed, and cancelled counts first. Include Fleet health, heartbeat/lease state, bounded controls, and an explicit recovery explanation for interrupted work.

### Council

Use three equal chambers for Parliament, Shadow Council, and ReferenceMonitor. Each chamber shows runtime-backed evidence, scope, status, and identifiers. A boundary callout must make clear that deliberation and routing cannot issue permits. Link to the redacted audit trace.

### Memory

Present scoped records, provenance, audit, tombstones, compaction, and synthesis schedules as separate tabs. Make scope, retention, promotion state, and revocation visible before record content. Destructive actions require a preview and explicit confirmation.

### Workflows

Show saved recipes as compact cards with task, profile, Harness, last run, and run/remove actions. The empty state should guide the operator to create a recipe from Command without adding another authority path.

### Harness Lab

Use a browse-and-inspect layout for Genes, Extensions, Skills, Packages, Authority, and Receipts. Package authoring must show the exact manifest shape and unverified posture before copy/export. Make package state, version, publisher, digest, and admission result scannable.

### Runtime Inventory and Tools

Use searchable lists with category filters and a stable detail panel. Show inputs, outputs, invariants, evidence, source modules, and documentation. Selecting a component or tool is inspection only; keep the boundary text adjacent to any action link.

### Connections

Group provider, MCP, and registry configuration into tabs. Display local-only storage, secret references, health, and last verification. Avoid rendering secret values. Make connection failures actionable without implying that a provider can bypass governance.

### Audit and Evolution

Audit uses an append-only event timeline with filters for session, execution, outcome, and evidence class. Evolution uses proposal cards, evidence details, canary stages, approval state, activation history, and rollback controls with strong separation between proposal and activation.

### Settings

Keep General, Appearance, Workspace, Intelligence, and Authority & evidence in a searchable directory. Appearance controls include theme, accent, preset, companion position/scale/motion, reduced motion, increased contrast, forced colors, and reduced transparency behavior.

## Visual direction

- Dark-first charcoal canvas with restrained grid texture and warm paper text.
- One high-salience signal color for focus, primary actions, and active rail state; support cyan and violet accents.
- Use the existing Foundry, Verdant, and light tokens as named presets rather than one-off colors.
- Serif display face for product headings; compact sans-serif for controls; monospace for IDs, hashes, versions, and evidence values.
- Prefer rectangular panels with small radii, thin borders, dense but breathable spacing, and short labels.
- Reserve green for verified/healthy, amber for waiting/approval, red for failure, blue for informational evidence, and gold for governance or release state.

## Interaction rules

- Every primary action has a disabled/offline state and a runtime-backed explanation.
- Preserve keyboard order: rail, top bar, main action, detail tabs, then secondary actions.
- Use live regions for connection, run, approval, and failure transitions.
- Keep destructive or authority-adjacent actions behind preview, confirmation, and explicit scope text.
- Maintain visible focus at all scales. Test 100%, 150%, and 200% equivalents, increased contrast, forced colors, reduced motion, and reduced transparency.
- Empty states explain what evidence is missing and provide one safe next action.

## Cross-surface language

Use the same labels in desktop, CLI, and TUI: Command, Background Runs, Council, Memory, Workflows, Harness Lab, Runtime Inventory, Tools, Connections, Audit, Evolution, approval required, runtime offline, recorded, interrupted, and rollback. Do not expose internal implementation names or imply authority transfer through UI navigation.

## Acceptance criteria

1. An operator can submit a governed task, identify its selected route, and locate its evidence without leaving Command.
2. An interrupted worker run can be inspected and safely resumed or cancelled with visible lease and receipt state.
3. Council, package, tool, and evolution surfaces clearly separate inspection from authority.
4. The same status vocabulary and safe-action guidance appears in the CLI/TUI.
5. All advertised native platforms pass the existing accessibility test profile and retain clean-machine evidence.
6. The design is implementable with the current React/Tauri view model without adding a second execution path.
