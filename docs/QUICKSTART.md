# Pandora Quickstart Guide

Get up and running with **Pandora**, the high-assurance agentic coding runtime, in under 2 minutes.

---

## 1. Installation

### Quick Install (Pre-compiled Binary)

#### Linux / macOS
```bash
curl -fsSL https://raw.githubusercontent.com/anisayakmitra-in/AGENT-PANDORA/main/install.sh | bash
```

#### Windows (PowerShell)
```powershell
irm https://raw.githubusercontent.com/anisayakmitra-in/AGENT-PANDORA/main/install.ps1 | iex
```

### From Source (Rust Cargo)
```bash
git clone https://github.com/anisayakmitra-in/AGENT-PANDORA.git
cd AGENT-PANDORA
cargo install --path crates/pandora-cli --locked
```

Verify your installation:
```bash
pandora --version
# Output: pandora 2.0.0
```

---

## 2. Onboarding Setup Wizard

Initialize your workspace and configure your first LLM provider using the setup wizard:

```bash
pandora setup
```

The interactive wizard guides you through:
1. **Model Provider Selection**: OpenRouter, Anthropic, OpenAI, Nous Portal, Groq, Azure, or local Ollama / LM Studio.
2. **Authentication**: Enter your API key or use OAuth PKCE browser sign-in (`pandora provider login`).
3. **Workspace Initialization**: Sets up `.pandora/` state, reference monitors, and encryption vaults.

### Non-Interactive Setup (CI / Headless)
```bash
pandora setup \
  --provider openrouter \
  --model anthropic/claude-3.7-sonnet \
  --api-key env:OPENROUTER_API_KEY \
  --json
```

---

## 3. First Conversation (Interactive TUI)

Launch the interactive Terminal User Interface:
```bash
pandora
```

Or start a direct chat session:
```bash
pandora chat
```

### Essential Interactive Shortcuts & Slash Commands
- **`/model <provider:model>`** — Hot-swap the active reasoning model mid-session.
- **`/mode <autonomous|supervised|read-only>`** — Adjust governance and capability boundaries live.
- **`/preset <name>`** — Load a reusable prompt preset with parameterized interpolation.
- **`/status`** — View active token usage, session turn count, and judicial permits.
- **`/help`** — Show all available slash commands.
- **`Ctrl+C`** / **`/exit`** — Gracefully suspend the session with automatic state checkpointing.

---

## 4. Run a Headless Agentic Task

Execute an autonomous agent run on a specific repository task:

```bash
pandora run "Refactor error handling in src/lib.rs to use thiserror" \
  --agent \
  --mode supervised \
  --max-turns 15
```

Pandora will inspect the repository, propose bounded changes, seek judicial approval for sensitive mutations, and verify the build before completion.

---

## 5. Resuming an Existing Session

Sessions are durably checkpointed in SQLite with sliding-window compaction ($W = 10\dots15$):

```bash
# List previous sessions
pandora session list

# Resume a session seamlessly
pandora session resume <session-id>
```

---

## Next Steps
- [CLI Usage Guide](CLI_USAGE.md)
- [Configuration Reference](CONFIGURATION.md)
- [Tools & Toolsets](TOOLS.md)
- [MCP Integration & Enterprise Sign-In](MCP_INTEGRATION.md)
- [Cron Scheduling](CRON_SCHEDULING.md)
- [Environment Variables](ENVIRONMENT_VARIABLES.md)
