# Pandora CLI Usage Guide

The Pandora Command-Line Interface (`pandora`) provides both interactive terminal controls and headless scriptable commands for agentic software engineering.

---

## 1. Interactive Interface (TUI & Chat)

### Launching
- `pandora`: Launches the full Ratatui TUI dashboard (Kanban, memory, telemetry sidebar).
- `pandora chat`: Starts an interactive REPL with live streaming and syntax highlighting.

### Inline Model & Permission Settings
Pandora allows hot-swapping reasoning models and judicial governance levels mid-session without restarting or losing conversational context:

- **`/model <provider:model>`** or **`/model <model_id>`**:
  ```text
  pandora> /model anthropic/claude-3.7-sonnet
  Switched active model to anthropic/claude-3.7-sonnet
  ```
- **`/mode <autonomous|supervised|read-only>`** or **`/permission <mode>`**:
  ```text
  pandora> /mode autonomous
  Updated governance mode to Autonomous (full tool execution without manual prompt confirmation)
  ```
  - `autonomous` (Level 2): Full execution permitted by policy without interrupting operator for non-destructive actions.
  - `supervised` (Level 1, default): Bounded mutations and network actions require explicit confirmation.
  - `read-only` (Level 0): Mutating tools (`file_write`, `patch_apply`, `git_commit`) are strictly fenced off.

---

## 2. Reusable Prompt Presets

Pandora includes a declarative prompt preset system (`PromptEngine`) supporting global and workspace-scoped templates with dynamic parameter interpolation (`{{workspace}}`, `{{task}}`, `{{lang}}`).

### CLI Management
```bash
# Save a prompt preset
pandora prompt save code-review "Perform a comprehensive security and memory audit on {{workspace}} with focus on {{task}} in {{lang}}."

# Save a global preset available across all repositories
pandora prompt save security-audit "Audit all crypto and auth flows." --global

# List available presets (workspace overrides global)
pandora prompt list

# Inspect preset template
pandora prompt inspect code-review

# Apply and render a preset with parameters
pandora prompt apply code-review \
  --param workspace="/repo/core" \
  --param task="auth verification" \
  --param lang="Rust"

# Delete a preset
pandora prompt delete code-review
```

### Interactive Usage
Inside interactive sessions (`pandora chat` or `pandora` TUI), presets can be invoked instantly:
```text
pandora> /preset code-review task="neutralize path traversal"
Loaded preset 'code-review' with parameters: task='neutralize path traversal'
```

---

## 3. Headless Task Execution (`pandora run`)

Run automated tasks from scripts or CI/CD pipelines:

```bash
# Standard agent execution
pandora run "Implement unit tests for JSON parser" \
  --agent \
  --max-turns 20 \
  --max-tools 50 \
  --mode supervised

# Read-only evaluation
pandora run "Benchmark memory usage of cache module" \
  --agent \
  --mode read-only

# Emit machine-readable JSON output
pandora run "Validate package lock integrity" \
  --agent \
  --json
```

---

## 4. Session Resumption & Compaction

Long-running agent workflows are durably checkpointed in SQLite:

```bash
# List historical sessions
pandora session list

# Inspect session details, event logs, and checkpoints
pandora session inspect <session-id>

# Resume a session seamlessly
pandora session resume <session-id>
```

### Sliding-Window Resumption Resilience
Sessions with 50+ turns automatically utilize sliding-window context compaction ($W = 10\dots15$):
- Preceding turns are compressed into an attested Historical Context Summary.
- Active recent turns are retained verbatim.
- Prevents context explosion, OOM crashes, and deserialization desync across multi-turn workflows.

---

## 5. Shell Completions

Generate fast autocompletion scripts for your shell:

```bash
# PowerShell
pandora completions powershell >> $PROFILE

# Bash
pandora completions bash >> ~/.bashrc

# Zsh
pandora completions zsh > ~/.zsh/completion/_pandora

# Fish
pandora completions fish > ~/.config/fish/completions/pandora.fish
```
