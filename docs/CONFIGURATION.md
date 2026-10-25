# Pandora Configuration Reference

Pandora's configuration is managed through a JSON configuration file (by default `.pandora/config.json` or path specified by `PANDORA_CONFIG`).

---

## 1. Configuration File Structure

Example `.pandora/config.json`:

```json
{
  "version": "2.0.0",
  "active_provider": "openrouter",
  "active_model": "anthropic/claude-3.7-sonnet",
  "workspace_dir": "/path/to/repo",
  "data_dir": "/path/to/.pandora/data",
  "governance": {
    "default_mode": "supervised",
    "require_operator_allow": true
  },
  "providers": {
    "openrouter": {
      "base_url": "https://openrouter.ai/api/v1",
      "protocol": "openai_compatible",
      "api_key_env": "OPENROUTER_API_KEY",
      "default_model": "anthropic/claude-3.7-sonnet",
      "timeout_seconds": 120
    },
    "local-ollama": {
      "base_url": "http://127.0.0.1:11434/v1",
      "protocol": "openai_compatible",
      "api_key_env": "OLLAMA_API_KEY",
      "default_model": "llama3.3:70b",
      "timeout_seconds": 300
    }
  },
  "mcp_servers": {
    "filesystem": {
      "transport": "stdio",
      "program": "mcp-server-filesystem",
      "arguments": ["/path/to/repo"]
    },
    "corp-analytics": {
      "transport": "http",
      "url": "https://analytics.internal.corp/mcp",
      "auth_type": "bearer",
      "headers": {
        "X-Corporate-Tenant": "finance-prod"
      }
    }
  }
}
```

---

## 2. Multi-Provider Management

### Registering and Switching Providers
```bash
# Add a custom provider profile
pandora provider add custom-deepseek \
  --base-url https://api.deepseek.com/v1 \
  --protocol openai_compatible \
  --api-key-env DEEPSEEK_API_KEY \
  --model deepseek-reasoner

# Switch active provider
pandora provider use custom-deepseek

# List configured providers
pandora provider list

# Test provider connectivity and latency
pandora provider test custom-deepseek
```

---

## 3. Corporate Proxy Configuration

Pandora features native corporate proxy authentication and tunneling for enterprise networks.

### Environment Variable Proxy Resolution
Pandora automatically discovers proxies in the following precedence order:
1. `HTTPS_PROXY` or `https_proxy`
2. `ALL_PROXY` or `all_proxy`
3. `HTTP_PROXY` or `http_proxy`

### Authenticated Proxy Tunneling
For proxies requiring authentication, provide credentials directly in the URL:
```bash
export HTTPS_PROXY="http://corp_domain%5Cuser:SecretPassword123@proxy.internal.corp:8080"
export NO_PROXY="localhost,127.0.0.1,::1,.corp.internal"
```
Pandora extracts username and password, configures HTTP Basic authentication on the proxy tunnel, and routes provider calls, MCP HTTP endpoints, and release updates transparently through the proxy.

### Windows System Proxy Support
On Windows, Pandora automatically inspects `HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings` when environment variables are not set.

---

## 4. Encrypted Secret Vault (`SecretVault`)

Sensitive credentials (API keys, MCP bearer tokens, OAuth refresh tokens) are encrypted at rest using **Argon2id** password hashing and **XChaCha20Poly1305** authenticated encryption.

```bash
# Set master encryption passphrase
export PANDORA_MASTER_KEY="your-high-entropy-master-passphrase-32-chars"

# Store a secret
pandora secret set OPENROUTER_API_KEY "sk-or-v1-abcdef1234567890"

# View vault status
pandora secret status

# Remove a secret
pandora secret remove OPENROUTER_API_KEY
```
All secrets are zeroized in memory upon drop (`zeroize` crate) and never logged to stdout or serialized in session journals.
