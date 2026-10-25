# Pandora Environment Variables Reference

Pandora respects the following environment variables across configuration, runtime execution, and enterprise networking.

---

## 1. Core Runtime Configuration

| Variable | Description | Default |
|---|---|---|
| `PANDORA_CONFIG` | Path to runtime configuration file | `.pandora/config.json` |
| `PANDORA_DATA_DIR` | Directory containing SQLite journals, event logs, and caches | `.pandora/data` |
| `PANDORA_WORKSPACE` | Root directory of the repository under governance | Current working directory |
| `PANDORA_MASTER_KEY` | Master passphrase used to derive Argon2id encryption key for `SecretVault` | — |
| `PANDORA_LOG` | Logging verbosity filter (`error`, `warn`, `info`, `debug`, `trace`) | `info` |

---

## 2. LLM Provider Credentials

| Variable | Description |
|---|---|
| `PANDORA_PROVIDER_API_KEY` | Generic fallback API key for custom OpenAI-compatible endpoints |
| `OPENROUTER_API_KEY` | Authentication key for OpenRouter gateway |
| `ANTHROPIC_API_KEY` | Direct API key for Anthropic Claude models |
| `OPENAI_API_KEY` | Direct API key for OpenAI GPT / o-series models |
| `GEMINI_API_KEY` | Direct API key for Google Gemini models |
| `GROQ_API_KEY` | Authentication key for Groq inference acceleration |
| `DEEPSEEK_API_KEY` | Authentication key for DeepSeek API |
| `AZURE_OPENAI_API_KEY` | Key for Azure OpenAI resource endpoints |
| `NOUS_API_KEY` | API key for Nous Research Portal |

---

## 3. Corporate Proxy & Network Authentication

| Variable | Description | Example |
|---|---|---|
| `HTTPS_PROXY` / `https_proxy` | Proxy server for outbound HTTPS traffic | `http://user:pass@proxy.corp:8080` |
| `HTTP_PROXY` / `http_proxy` | Proxy server for outbound HTTP traffic | `http://10.0.0.1:3128` |
| `ALL_PROXY` / `all_proxy` | Catch-all proxy server for all network protocols | `socks5://proxy.corp:1080` |
| `NO_PROXY` / `no_proxy` | Comma-separated list of hostnames/IPs to bypass the proxy | `localhost,127.0.0.1,::1,.internal.corp` |

---

## 4. MCP Server Authentication

| Variable Pattern | Description | Example |
|---|---|---|
| `PANDORA_MCP_<SERVER>_TOKEN` | Bearer token for remote MCP server (uppercased server id, dashes replaced by underscores) | `PANDORA_MCP_CORP_ANALYTICS_TOKEN` |
| `PANDORA_MCP_TIMEOUT_SECS` | Default timeout in seconds for remote MCP requests | `30` |

---

## 5. Security & Isolation

| Variable | Description | Default |
|---|---|---|
| `PANDORA_SANDBOX_STRICT` | If set to `1` or `true`, enforces strict denial of ambient filesystem lookups | `1` |
| `PANDORA_AUDIT_LOG_PATH` | Optional destination path for append-only cryptographic event audit logs | None |
