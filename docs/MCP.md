# Local MCP stdio

Pandora stores local MCP server profiles in its normal configuration file. A
profile contains an exact absolute executable path, an argument vector, and a
protocol mode. Configuration does not start the program.

```text
pandora mcp set local --program /absolute/path/to/server --arguments-json '["--flag","value"]' --mode auto
pandora mcp list
pandora mcp inspect local
pandora mcp catalog local --allow
pandora mcp call local tool-name --arguments-json '{"path":"README.md"}' --idempotency-key request-1 --allow
pandora mcp remove local --yes
```

`--mode` accepts `auto`, `modern-only`, or `legacy-only`. Arguments must be a
JSON array of strings. Removing a profile requires `--yes`. `catalog` imports a
server's current tool catalog for one bounded connection, and `call` invokes
one imported local tool. Both require `--allow` as explicit local operator
consent and create a session event trail. `call` requires a JSON object and an
idempotency key; arguments are never echoed in CLI output. Configuration-only
commands do not execute the configured server. Do not put credentials in MCP
arguments.

When invoked by Pandora's runtime, the program starts directly without a shell,
inherits no environment, and uses bounded protocol frames, stderr, and request
time. `catalog` and `call` terminate the child process group when the command
completes.
Their bounded runtime-event batches are appended atomically to the selected
session store; event contexts retain receipt IDs, while receipt details are
returned in the command result.

The protocol mode is explicit:

- `Auto` is recommended for compatibility. It probes modern MCP on a
  disposable child and starts a fresh legacy child only when the probe returns
  structured, unambiguous legacy-version evidence.
- `ModernOnly` is recommended for hardened deployments. It pins the child to
  MCP `2026-07-28`, beginning with `server/discover`; `tools/list` and
  `tools/call` carry the required protocol, client, and capability metadata.
- `LegacyOnly` pins a separate adapter to MCP `2025-11-25`. `initialize` is
  request 1, followed by `notifications/initialized`, `tools/list`, and
  `tools/call` without modern request metadata.

One child uses one wire era. `Auto` does not downgrade on timeouts, process
exit, EOF, malformed or oversized data, wrong response IDs, generic errors, or
`MethodNotFound` alone. The disposable modern child is killed and reaped before
an explicitly identified legacy server is restarted.

Imported tools use Pandora's existing `ToolEngine` and its simple object-schema
subset. Unsupported JSON Schema constructs reject the entire import. Server
spawn and every tool call still require Parliament policy, a ReferenceMonitor
one-shot permit, an executor recheck of the exact target and payload, a receipt,
and canonical runtime events. Protocol selection never grants authority.

Each connected child owns one catalog revision. The revision records the child
process ID, protocol era, exact launch-configuration digest, catalog digest,
and each imported tool's local ID, remote name, and schema digest. Pandora
rejects a second active child with the same server ID. Dropping or terminating
the owner removes only that revision and its imported tools; reconnecting gets
a new generation.

Tool-call permits bind the active generation, catalog and schema digests,
process ID, exact remote tool name, and canonical arguments. A permit created
for an earlier child or catalog fails before Pandora writes an RPC request.

This governance boundary authorizes and audits server spawn and tool calls; it
does not OS-sandbox a malicious local MCP executable. Operators must treat the
configured program as trusted code. Process-group termination limits orphaned
work but does not provide host containment. This preview does not support HTTP/SSE,
OAuth, package or marketplace discovery, hooks supplied by MCP servers,
subagents, progress/tasks, sessions, or server-originated requests and
notifications. `--allow` is command-level consent, not a persisted human
approval record; deployments requiring persisted approvals must use the
existing approval workflow before invoking the runtime.
