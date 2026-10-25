# Pandora Tools & Toolsets Reference

Pandora provides a suite of 40+ built-in, harness-provided, and MCP-integrated tools governing workspace operations, source inspection, testing, and system evolution.

---

## 1. Tool Architectural Hierarchy

Pandora enforces tripartite separation of concerns for all tool executions:
1. **ToolEngine**: Catalog of registered tool definitions, schemas, and parameter validation.
2. **ReferenceMonitor & Judicial Parliament**: Evaluates cryptographic capabilities (`ProcessExecute`, `NetworkConnect`, `McpInvoke`, `StorageRead`, `StorageWrite`) before dispatching an effect.
3. **ExecutionController**: Dispatches the permitted action inside a sandbox confinement boundary, generating attested `EffectReceipt` records.

---

## 2. Core Built-In Tools

### Workspace & File Manipulation
- `workspace_read`: Read file contents with strict byte bounds, line-number offsets, and path containment validation.
- `workspace_write`: Create or overwrite files atomically via `atomic-write-file` with automatic parent directory creation.
- `workspace_edit`: Apply precise contiguous replacement blocks.
- `workspace_list`: Enumerate directories with `.gitignore` respect and file-size metadata.
- `workspace_search`: High-speed regex and literal text searching across the workspace tree.
- `workspace_diff`: Compute unified Git diffs between current workspace state and HEAD.

### Execution & Verification
- `process_execute`: Execute bounded terminal commands inside isolated subprocesses with timeout, stdout/stderr byte limits, and environment clearing.
- `cargo_check`: Run Rust compiler verification with JSON diagnostic parsing.
- `cargo_test`: Execute targeted unit or integration test suites.
- `git_status`: Query working tree porcelain status.
- `git_commit`: Create verified Git commits with structured changelogs.

### Memory & Context Inspection
- `memory_recall`: Query episodic and semantic memory fragments across previous sessions.
- `memory_synthesize`: Synthesize procedural lessons and task heuristics.
- `context_inspect`: Inspect active token budgets and context fragment origins.

---

## 3. Custom Harnesses & WebAssembly Genes

Pandora allows extending tool capabilities through compiled WebAssembly components (`WasmGene`):
- Packaged as cryptographically signed `.wasm` binaries.
- Executed inside a zero-trust WASM sandbox with strictly bounded linear memory.
- Registered via `HarnessRegistry` and admitted via `pandora package admit`.

---

## 4. MCP Tools

Any tool exposed by configured Model Context Protocol (MCP) servers (both standard I/O and HTTP/SSE) is dynamically registered under the `mcp.<server_id>.<tool_name>` namespace.
- Schemas are validated against JSON Schema Draft-07.
- Tools require explicit operator consent (`--allow` flag or TUI approval) under supervised governance.
