# Pandora MCP Integration Guide

Pandora natively implements the Model Context Protocol (MCP) across both modern (2026-07-28) and legacy wire eras. It supports both local process (`stdio`) and remote network (`http` / `sse`) servers with enterprise authentication.

---

## 1. Local Process Servers (`stdio`)

Register a local command-line MCP server:

```bash
# Add an stdio MCP server
pandora mcp add filesystem -- /usr/local/bin/mcp-server-filesystem /repo/path

# Inspect server registration
pandora mcp inspect filesystem
```

---

## 2. Remote Enterprise HTTP / SSE Servers

Connect to remote enterprise endpoints with authentication headers and bearer tokens:

```bash
# Add remote HTTP server with Bearer auth and custom enterprise headers
pandora mcp add-http corp-analytics \
  https://analytics.internal.corp/mcp \
  --auth bearer \
  --token-env CORP_ANALYTICS_TOKEN \
  --header X-Corporate-Dept:Engineering \
  --json

# Or configure with direct token stored in SecretVault
pandora mcp add-http corp-data \
  https://data.internal.corp/rpc \
  --auth bearer \
  --token sk-enterprise-secret-token \
  --json
```

### Parameter Reference for `mcp add-http`
| Option | Description |
|---|---|
| `<name>` / `--name` | Unique identifier for the MCP server |
| `<url>` / `--url` | Target HTTP/HTTPS endpoint |
| `--auth <none\|bearer\|api-key>` | Authentication scheme (default: `bearer` if token provided) |
| `--token <token>` | Explicit secret token to store in encrypted `SecretVault` |
| `--token-env <VAR>` | Name of environment variable containing the authentication token |
| `--header <K:V>` | Custom HTTP headers injected into all requests (can be repeated) |
| `--auth-url <url>` | OAuth 2.0 authorization endpoint for PKCE flows |
| `--token-url <url>` | OAuth 2.0 token exchange endpoint |
| `--client-id <id>` | OAuth 2.0 client ID |
| `--scopes <scopes>` | Comma-separated OAuth scopes |

---

## 3. Enterprise MCP Sign-In & Authentication

### Token Login
Store an updated access token securely in the encrypted `SecretVault`:
```bash
pandora mcp login corp-analytics --token "new-bearer-token-12345"
```

### OAuth 2.0 with PKCE Sign-In
Initiate browser-based OAuth 2.0 Authorization Code flow with Proof Key for Code Exchange (RFC 7636):
```bash
pandora mcp login enterprise-gateway
```
Pandora starts a local loopback callback receiver on `http://127.0.0.1:1455/callback`, opens the corporate identity provider in your browser, securely exchanges the authorization code for access/refresh tokens, and persists them into `SecretVault`.

### Logout & Credential Revocation
Remove stored credentials from `SecretVault`:
```bash
pandora mcp logout corp-analytics
```

---

## 4. Querying and Invoking MCP Tools

### Cataloging Available Tools
Query the remote server and register its tool definitions:
```bash
pandora mcp catalog corp-analytics --allow
```

### Direct Tool Invocation
Invoke a specific tool directly through the CLI:
```bash
pandora mcp call corp-analytics mcp.corp-analytics.query_metrics \
  --arguments-json '{"metric": "cpu_utilization", "window": "1h"}' \
  --idempotency-key "idem-run-001" \
  --allow
```
All invocations generate cryptographic `EffectReceipt` records and are subject to the active governance policy.
