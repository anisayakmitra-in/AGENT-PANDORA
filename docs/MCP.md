# Local MCP preview

Pandora's MCP client is an internal runtime preview for one explicitly
configured local stdio server. Configuration contains an exact absolute
executable path and argument vector. Pandora starts it directly without a
shell, clears its inherited environment, and bounds protocol frames, stderr,
and request time.

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

This governance boundary authorizes and audits server spawn and tool calls; it
does not OS-sandbox a malicious local MCP executable. Operators must treat the
configured program as trusted code. This preview does not support HTTP/SSE,
OAuth, package or marketplace discovery, hooks supplied by MCP servers,
subagents, progress/tasks, sessions, or server-originated requests and
notifications.
