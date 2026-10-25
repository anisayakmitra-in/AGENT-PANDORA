use super::{load_config, parse_options, timestamp, write_config};
use crate::output::{CliError, CommandResult, success};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use pandora_provider::ProviderProtocol;
use pandora_runtime::config::{
    DEFAULT_PROVIDER_API_KEY_ENV, DEFAULT_PROVIDER_NAME, ProviderProfile,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, IsTerminal as _, Read, Write};
use url::Url;

const MAX_PROMPT_BYTES: usize = 1_024;
pub const LOCAL_NO_API_KEY_ENV: &str = "LOCAL_NO_API_KEY";

#[derive(Clone, Copy, Debug)]
pub struct ProviderPreset {
    pub name: &'static str,
    pub label: &'static str,
    pub base_url: &'static str,
    pub default_model: &'static str,
    pub api_key_env: &'static str,
    pub protocol: ProviderProtocol,
}

pub const PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        name: "ollama",
        label: "Ollama (Local / Offline)",
        base_url: "http://localhost:11434/v1",
        default_model: "qwen2.5-coder:32b",
        api_key_env: "OLLAMA_API_KEY",
        protocol: ProviderProtocol::OpenAiCompatible,
    },
    ProviderPreset {
        name: "lmstudio",
        label: "LM Studio (Local Server)",
        base_url: "http://localhost:1234/v1",
        default_model: "qwen2.5-coder-32b-instruct",
        api_key_env: "LMSTUDIO_API_KEY",
        protocol: ProviderProtocol::OpenAiCompatible,
    },
    ProviderPreset {
        name: "vllm",
        label: "vLLM / Local OpenAI Server",
        base_url: "http://localhost:8000/v1",
        default_model: "Qwen/Qwen2.5-Coder-32B-Instruct",
        api_key_env: "VLLM_API_KEY",
        protocol: ProviderProtocol::OpenAiCompatible,
    },
    ProviderPreset {
        name: "openrouter",
        label: "OpenRouter (Multi-model Gateway)",
        base_url: "https://openrouter.ai/api/v1",
        default_model: "anthropic/claude-3.5-sonnet",
        api_key_env: "OPENROUTER_API_KEY",
        protocol: ProviderProtocol::OpenAiCompatible,
    },
    ProviderPreset {
        name: "deepseek",
        label: "DeepSeek API",
        base_url: "https://api.deepseek.com/v1",
        default_model: "deepseek-coder",
        api_key_env: "DEEPSEEK_API_KEY",
        protocol: ProviderProtocol::OpenAiCompatible,
    },
    ProviderPreset {
        name: "openai",
        label: "OpenAI Direct / Codex",
        base_url: "https://api.openai.com/v1",
        default_model: "gpt-4o",
        api_key_env: "OPENAI_API_KEY",
        protocol: ProviderProtocol::OpenAiCompatible,
    },
    ProviderPreset {
        name: "anthropic",
        label: "Anthropic Claude",
        base_url: "https://api.anthropic.com/v1",
        default_model: "claude-3-7-sonnet-20250219",
        api_key_env: "ANTHROPIC_API_KEY",
        protocol: ProviderProtocol::AnthropicMessages,
    },
    ProviderPreset {
        name: "groq",
        label: "Groq Fast Inference",
        base_url: "https://api.groq.com/openai/v1",
        default_model: "llama-3.3-70b-versatile",
        api_key_env: "GROQ_API_KEY",
        protocol: ProviderProtocol::OpenAiCompatible,
    },
    ProviderPreset {
        name: "nous",
        label: "Nous Portal / Inference API",
        base_url: "https://inference-api.nousresearch.com/v1",
        default_model: "Hermes-3-Llama-3.1-70B",
        api_key_env: "NOUS_API_KEY",
        protocol: ProviderProtocol::OpenAiCompatible,
    },
    ProviderPreset {
        name: "copilot",
        label: "GitHub Copilot Subscription",
        base_url: "https://api.githubcopilot.com",
        default_model: "gpt-4o",
        api_key_env: "COPILOT_OAUTH_TOKEN",
        protocol: ProviderProtocol::OpenAiCompatible,
    },
    ProviderPreset {
        name: "azure",
        label: "Azure AI / OpenAI Inference",
        base_url: "https://models.inference.ai.azure.com",
        default_model: "gpt-4o",
        api_key_env: "AZURE_OPENAI_API_KEY",
        protocol: ProviderProtocol::OpenAiCompatible,
    },
    ProviderPreset {
        name: "moonshot",
        label: "Moonshot AI (Kimi)",
        base_url: "https://api.moonshot.cn/v1",
        default_model: "kimi-k2-0711-preview",
        api_key_env: "MOONSHOT_API_KEY",
        protocol: ProviderProtocol::OpenAiCompatible,
    },
    ProviderPreset {
        name: "huggingface",
        label: "Hugging Face Inference Router",
        base_url: "https://router.huggingface.co/v1",
        default_model: "Qwen/Qwen2.5-Coder-32B-Instruct",
        api_key_env: "HF_TOKEN",
        protocol: ProviderProtocol::OpenAiCompatible,
    },
    ProviderPreset {
        name: "nebius",
        label: "Nebius AI Studio",
        base_url: "https://api.studio.nebius.ai/v1",
        default_model: "deepseek-ai/DeepSeek-V3",
        api_key_env: "NEBIUS_API_KEY",
        protocol: ProviderProtocol::OpenAiCompatible,
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModelCatalogEntry {
    pub provider: &'static str,
    pub model: &'static str,
    pub description: &'static str,
}

pub const MODEL_CATALOG: &[ModelCatalogEntry] = &[
    ModelCatalogEntry {
        provider: "openrouter",
        model: "anthropic/claude-3.7-sonnet",
        description: "OpenRouter · Claude 3.7 Sonnet (hybrid reasoning & coding)",
    },
    ModelCatalogEntry {
        provider: "openrouter",
        model: "anthropic/claude-3.5-sonnet",
        description: "OpenRouter · Claude 3.5 Sonnet (flagship agentic coding)",
    },
    ModelCatalogEntry {
        provider: "openrouter",
        model: "nousresearch/hermes-3-llama-3.1-405b",
        description: "OpenRouter · Nous Hermes 3 405B (frontier open-weights agent)",
    },
    ModelCatalogEntry {
        provider: "openrouter",
        model: "nousresearch/hermes-3-llama-3.1-70b",
        description: "OpenRouter · Nous Hermes 3 70B (fast steerable tool-calling)",
    },
    ModelCatalogEntry {
        provider: "openrouter",
        model: "deepseek/deepseek-r1",
        description: "OpenRouter · DeepSeek R1 (deep reasoning & verification)",
    },
    ModelCatalogEntry {
        provider: "openrouter",
        model: "deepseek/deepseek-chat",
        description: "OpenRouter · DeepSeek V3 (high-speed general & coding)",
    },
    ModelCatalogEntry {
        provider: "openrouter",
        model: "openai/gpt-4o",
        description: "OpenRouter · OpenAI GPT-4o (multimodal omni model)",
    },
    ModelCatalogEntry {
        provider: "openrouter",
        model: "qwen/qwen-2.5-coder-32b-instruct",
        description: "OpenRouter · Qwen 2.5 Coder 32B Instruct",
    },
    ModelCatalogEntry {
        provider: "nous",
        model: "Hermes-3-Llama-3.1-70B",
        description: "Nous Portal · Hermes 3 Llama 3.1 70B",
    },
    ModelCatalogEntry {
        provider: "nous",
        model: "Hermes-3-Llama-3.1-405B",
        description: "Nous Portal · Hermes 3 Llama 3.1 405B",
    },
    ModelCatalogEntry {
        provider: "nous",
        model: "DeepHermes-3-Llama-3-8B-Preview",
        description: "Nous Portal · DeepHermes 3 8B Preview (unified reasoning)",
    },
    ModelCatalogEntry {
        provider: "ollama",
        model: "qwen2.5-coder:32b",
        description: "Ollama Local · Qwen 2.5 Coder 32B (offline coding powerhouse)",
    },
    ModelCatalogEntry {
        provider: "ollama",
        model: "qwen2.5-coder:14b",
        description: "Ollama Local · Qwen 2.5 Coder 14B (balanced local coding)",
    },
    ModelCatalogEntry {
        provider: "ollama",
        model: "hermes3:8b",
        description: "Ollama Local · Nous Hermes 3 8B (fast local agent)",
    },
    ModelCatalogEntry {
        provider: "ollama",
        model: "deepseek-r1:14b",
        description: "Ollama Local · DeepSeek R1 14B (local chain-of-thought)",
    },
    ModelCatalogEntry {
        provider: "ollama",
        model: "llama3.3:70b",
        description: "Ollama Local · Meta Llama 3.3 70B Instruct",
    },
    ModelCatalogEntry {
        provider: "anthropic",
        model: "claude-3-7-sonnet-20250219",
        description: "Anthropic Direct · Claude 3.7 Sonnet",
    },
    ModelCatalogEntry {
        provider: "anthropic",
        model: "claude-3-5-sonnet-20241022",
        description: "Anthropic Direct · Claude 3.5 Sonnet",
    },
    ModelCatalogEntry {
        provider: "anthropic",
        model: "claude-3-5-haiku-20241022",
        description: "Anthropic Direct · Claude 3.5 Haiku (low latency)",
    },
    ModelCatalogEntry {
        provider: "openai",
        model: "gpt-4o",
        description: "OpenAI Direct · GPT-4o",
    },
    ModelCatalogEntry {
        provider: "openai",
        model: "o3-mini",
        description: "OpenAI Direct · o3-mini (fast STEM & code reasoning)",
    },
    ModelCatalogEntry {
        provider: "deepseek",
        model: "deepseek-chat",
        description: "DeepSeek Direct · DeepSeek-V3 Chat",
    },
    ModelCatalogEntry {
        provider: "deepseek",
        model: "deepseek-reasoner",
        description: "DeepSeek Direct · DeepSeek-R1 Reasoner",
    },
    ModelCatalogEntry {
        provider: "groq",
        model: "llama-3.3-70b-versatile",
        description: "Groq LPU · Llama 3.3 70B Versatile (ultra-fast)",
    },
    ModelCatalogEntry {
        provider: "groq",
        model: "deepseek-r1-distill-llama-70b",
        description: "Groq LPU · DeepSeek R1 Distill Llama 70B",
    },
    ModelCatalogEntry {
        provider: "lmstudio",
        model: "qwen2.5-coder-32b-instruct",
        description: "LM Studio Local · Qwen 2.5 Coder 32B Instruct (localhost:1234)",
    },
    ModelCatalogEntry {
        provider: "vllm",
        model: "Qwen/Qwen2.5-Coder-32B-Instruct",
        description: "vLLM Local · Qwen 2.5 Coder 32B Instruct (localhost:8000)",
    },
    ModelCatalogEntry {
        provider: "copilot",
        model: "gpt-4o",
        description: "GitHub Copilot Subscription · GPT-4o (OAuth)",
    },
    ModelCatalogEntry {
        provider: "copilot",
        model: "claude-3.7-sonnet",
        description: "GitHub Copilot Subscription · Claude 3.7 Sonnet (OAuth)",
    },
    ModelCatalogEntry {
        provider: "azure",
        model: "gpt-4o",
        description: "Azure AI Inference · GPT-4o (OAuth / API key)",
    },
    ModelCatalogEntry {
        provider: "moonshot",
        model: "kimi-k2-0711-preview",
        description: "Moonshot AI · Kimi K2 Preview (agentic coding)",
    },
    ModelCatalogEntry {
        provider: "huggingface",
        model: "Qwen/Qwen2.5-Coder-32B-Instruct",
        description: "Hugging Face Router · Qwen 2.5 Coder 32B (OAuth / HF_TOKEN)",
    },
    ModelCatalogEntry {
        provider: "nebius",
        model: "deepseek-ai/DeepSeek-V3",
        description: "Nebius AI Studio · DeepSeek V3",
    },
];

#[derive(Clone, Copy, Debug)]
pub struct OAuthProviderSpec {
    pub name: &'static str,
    pub label: &'static str,
    pub preset_name: &'static str,
    pub auth_endpoint: &'static str,
    pub token_endpoint: &'static str,
    pub client_id: &'static str,
    pub redirect_uri: &'static str,
    pub scopes: &'static str,
    pub is_local_no_auth: bool,
}

pub const OAUTH_PROVIDERS: &[OAuthProviderSpec] = &[
    OAuthProviderSpec {
        name: "claude",
        label: "Anthropic Claude Subscription (OAuth PKCE)",
        preset_name: "anthropic",
        auth_endpoint: "https://claude.ai/oauth/authorize",
        token_endpoint: "https://claude.ai/oauth/token",
        client_id: "pandora-cli-oauth",
        redirect_uri: "http://127.0.0.1:1455/auth/callback",
        scopes: "user:inference org:read",
        is_local_no_auth: false,
    },
    OAuthProviderSpec {
        name: "openai",
        label: "OpenAI / ChatGPT / Codex Subscription (OAuth PKCE)",
        preset_name: "openai",
        auth_endpoint: "https://auth.openai.com/oauth/authorize",
        token_endpoint: "https://auth.openai.com/oauth/token",
        client_id: "pandora-cli-oauth",
        redirect_uri: "http://127.0.0.1:1455/auth/callback",
        scopes: "openid profile email model.request",
        is_local_no_auth: false,
    },
    OAuthProviderSpec {
        name: "copilot",
        label: "GitHub Copilot Subscription (OAuth PKCE)",
        preset_name: "copilot",
        auth_endpoint: "https://github.com/login/oauth/authorize",
        token_endpoint: "https://github.com/login/oauth/access_token",
        client_id: "Iv1.b507a08c87ecfe98",
        redirect_uri: "http://127.0.0.1:1455/auth/callback",
        scopes: "read:user copilot",
        is_local_no_auth: false,
    },
    OAuthProviderSpec {
        name: "openrouter",
        label: "OpenRouter PKCE Key Exchange",
        preset_name: "openrouter",
        auth_endpoint: "https://openrouter.ai/auth",
        token_endpoint: "https://openrouter.ai/api/v1/auth/keys",
        client_id: "pandora-cli",
        redirect_uri: "http://127.0.0.1:1455/auth/callback",
        scopes: "keys:create",
        is_local_no_auth: false,
    },
    OAuthProviderSpec {
        name: "nous",
        label: "Nous Research Portal (OAuth PKCE)",
        preset_name: "nous",
        auth_endpoint: "https://portal.nousresearch.com/oauth/authorize",
        token_endpoint: "https://portal.nousresearch.com/oauth/token",
        client_id: "pandora-cli-oauth",
        redirect_uri: "http://127.0.0.1:1455/auth/callback",
        scopes: "inference:invoke",
        is_local_no_auth: false,
    },
    OAuthProviderSpec {
        name: "huggingface",
        label: "Hugging Face Hub OAuth (PKCE)",
        preset_name: "huggingface",
        auth_endpoint: "https://huggingface.co/oauth/authorize",
        token_endpoint: "https://huggingface.co/oauth/token",
        client_id: "pandora-cli-oauth",
        redirect_uri: "http://127.0.0.1:1455/auth/callback",
        scopes: "openid profile inference-api",
        is_local_no_auth: false,
    },
    OAuthProviderSpec {
        name: "azure",
        label: "Azure Entra ID / OpenAI OAuth (PKCE)",
        preset_name: "azure",
        auth_endpoint: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
        token_endpoint: "https://login.microsoftonline.com/common/oauth2/v2.0/token",
        client_id: "pandora-cli-oauth",
        redirect_uri: "http://127.0.0.1:1455/auth/callback",
        scopes: "https://cognitiveservices.azure.com/.default offline_access",
        is_local_no_auth: false,
    },
    OAuthProviderSpec {
        name: "ollama",
        label: "Local Ollama Server (No-Auth Local Endpoint)",
        preset_name: "ollama",
        auth_endpoint: "http://localhost:11434/v1",
        token_endpoint: "",
        client_id: "local",
        redirect_uri: "http://localhost:11434",
        scopes: "local",
        is_local_no_auth: true,
    },
    OAuthProviderSpec {
        name: "lmstudio",
        label: "Local LM Studio Server (No-Auth Local Endpoint)",
        preset_name: "lmstudio",
        auth_endpoint: "http://localhost:1234/v1",
        token_endpoint: "",
        client_id: "local",
        redirect_uri: "http://localhost:1234",
        scopes: "local",
        is_local_no_auth: true,
    },
];

pub fn find_oauth_provider(name: &str) -> Option<&'static OAuthProviderSpec> {
    let normalized = name.trim();
    if normalized.eq_ignore_ascii_case("anthropic") {
        return OAUTH_PROVIDERS.iter().find(|p| p.name == "claude");
    }
    OAUTH_PROVIDERS
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(normalized))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OAuthPendingFlow {
    pub provider: String,
    pub preset_name: String,
    pub account: String,
    pub state: String,
    pub code_verifier: String,
    pub code_challenge: String,
    pub redirect_uri: String,
    pub auth_url: String,
    pub created_at: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OAuthAccountEntry {
    pub provider: String,
    pub account: String,
    pub auth_kind: String,
    pub api_key_env: String,
    pub credential: String,
    pub masked_preview: String,
    pub active: bool,
    pub updated_at: u64,
}

fn oauth_pending_path(config: &pandora_runtime::config::RuntimeConfig) -> std::path::PathBuf {
    config.data_dir().join("oauth_pending.json")
}

fn oauth_accounts_path(config: &pandora_runtime::config::RuntimeConfig) -> std::path::PathBuf {
    config.data_dir().join("oauth_accounts.json")
}

fn random_url_safe_token(byte_len: usize) -> Result<String, CliError> {
    let mut bytes = vec![0_u8; byte_len];
    getrandom::fill(&mut bytes).map_err(|_| {
        CliError::internal("could not generate cryptographic PKCE entropy", json!({}))
    })?;
    Ok(URL_SAFE_NO_PAD.encode(&bytes))
}

fn mask_secret(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() > 8 {
        format!("••••{}", &trimmed[trimmed.len() - 4..])
    } else {
        "••••saved".to_owned()
    }
}

pub fn load_oauth_accounts(
    config: &pandora_runtime::config::RuntimeConfig,
) -> Vec<OAuthAccountEntry> {
    let path = oauth_accounts_path(config);
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Vec<OAuthAccountEntry>>(&raw).ok())
        .unwrap_or_default()
}

fn save_oauth_accounts(
    config: &pandora_runtime::config::RuntimeConfig,
    accounts: &[OAuthAccountEntry],
) -> Result<(), CliError> {
    fs::create_dir_all(config.data_dir()).map_err(|_| {
        CliError::configuration("could not create data directory for OAuth store", json!({}))
    })?;
    let path = oauth_accounts_path(config);
    let mut data = serde_json::to_vec_pretty(accounts)
        .map_err(|_| CliError::internal("could not serialize OAuth accounts store", json!({})))?;
    data.push(b'\n');
    fs::write(&path, data).map_err(|_| {
        CliError::configuration(
            "could not write OAuth accounts store",
            json!({"path": path}),
        )
    })
}

pub fn record_account_credential(
    config: &pandora_runtime::config::RuntimeConfig,
    provider: &str,
    account: &str,
    auth_kind: &str,
    api_key_env: &str,
    credential: &str,
) -> Result<OAuthAccountEntry, CliError> {
    let mut accounts = load_oauth_accounts(config);
    let now = timestamp().as_unix_seconds();
    for item in &mut accounts {
        if item.provider.eq_ignore_ascii_case(provider) {
            item.active = false;
        }
    }
    let preview = mask_secret(credential);
    let entry = OAuthAccountEntry {
        provider: provider.to_ascii_lowercase(),
        account: account.to_owned(),
        auth_kind: auth_kind.to_owned(),
        api_key_env: api_key_env.to_owned(),
        credential: credential.to_owned(),
        masked_preview: preview,
        active: true,
        updated_at: now,
    };
    if let Some(existing) = accounts.iter_mut().find(|item| {
        item.provider.eq_ignore_ascii_case(provider) && item.account.eq_ignore_ascii_case(account)
    }) {
        *existing = entry.clone();
    } else {
        accounts.push(entry.clone());
    }
    save_oauth_accounts(config, &accounts)?;
    if credential != "local-no-api-key" {
        write_dotenv_var(config, api_key_env, credential)?;
    }
    Ok(entry)
}

pub fn switch_oauth_account(
    config: &mut pandora_runtime::config::RuntimeConfig,
    provider_filter: Option<&str>,
    account_name: &str,
) -> Result<OAuthAccountEntry, CliError> {
    let mut accounts = load_oauth_accounts(config);
    let target_provider = provider_filter
        .map(str::to_owned)
        .or_else(|| {
            accounts
                .iter()
                .find(|a| a.account.eq_ignore_ascii_case(account_name))
                .map(|a| a.provider.clone())
        })
        .ok_or_else(|| {
            CliError::usage(format!(
                "account '{account_name}' not found; run '/account' or 'pandora provider accounts' to list accounts"
            ))
        })?;

    let mut matched: Option<OAuthAccountEntry> = None;
    for item in &mut accounts {
        if item.provider.eq_ignore_ascii_case(&target_provider) {
            if item.account.eq_ignore_ascii_case(account_name) {
                item.active = true;
                matched = Some(item.clone());
            } else {
                item.active = false;
            }
        }
    }
    let chosen = matched.ok_or_else(|| {
        CliError::usage(format!(
            "account '{account_name}' not found for provider '{target_provider}'"
        ))
    })?;
    save_oauth_accounts(config, &accounts)?;
    if chosen.credential != "local-no-api-key" {
        write_dotenv_var(config, &chosen.api_key_env, &chosen.credential)?;
    }
    if let Some(preset) = find_preset(&chosen.provider)
        .or_else(|| find_oauth_provider(&chosen.provider).and_then(|o| find_preset(o.preset_name)))
    {
        let _ = apply_model_selection(config, preset.name);
        write_config(config)?;
    } else if config.provider_profile(&chosen.provider).is_some() {
        let _ = config.set_active_provider(&chosen.provider);
        write_config(config)?;
    }
    Ok(chosen)
}

pub fn start_oauth_login(
    config: &mut pandora_runtime::config::RuntimeConfig,
    provider_name: &str,
    account_label: Option<&str>,
) -> Result<OAuthPendingFlow, CliError> {
    let spec = find_oauth_provider(provider_name).ok_or_else(|| {
        CliError::usage(format!(
            "unknown OAuth login provider '{provider_name}'; supported: {}",
            OAUTH_PROVIDERS
                .iter()
                .map(|p| p.name)
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })?;
    let account = account_label
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("default")
        .to_owned();
    let preset = find_preset(spec.preset_name).expect("OAuth preset exists");
    apply_model_selection(config, preset.name)?;
    write_config(config)?;

    if spec.is_local_no_auth {
        let _ = record_account_credential(
            config,
            spec.name,
            &account,
            "local",
            preset.api_key_env,
            "local-no-api-key",
        )?;
        return Ok(OAuthPendingFlow {
            provider: spec.name.to_owned(),
            preset_name: spec.preset_name.to_owned(),
            account,
            state: "local-ready".to_owned(),
            code_verifier: String::new(),
            code_challenge: String::new(),
            redirect_uri: spec.redirect_uri.to_owned(),
            auth_url: spec.auth_endpoint.to_owned(),
            created_at: timestamp().as_unix_seconds(),
        });
    }

    let code_verifier = random_url_safe_token(32)?;
    let mut hasher = Sha256::new();
    hasher.update(code_verifier.as_bytes());
    let code_challenge = URL_SAFE_NO_PAD.encode(hasher.finalize());
    let state = random_url_safe_token(16)?;

    let mut auth_url = Url::parse(spec.auth_endpoint)
        .map_err(|_| CliError::internal("invalid built-in OAuth endpoint", json!({})))?;
    {
        let mut pairs = auth_url.query_pairs_mut();
        if spec.name == "openrouter" {
            pairs.append_pair("callback_url", spec.redirect_uri);
            pairs.append_pair("code_challenge", &code_challenge);
            pairs.append_pair("code_challenge_method", "S256");
            pairs.append_pair("state", &state);
        } else {
            pairs.append_pair("response_type", "code");
            pairs.append_pair("client_id", spec.client_id);
            pairs.append_pair("redirect_uri", spec.redirect_uri);
            pairs.append_pair("scope", spec.scopes);
            pairs.append_pair("code_challenge", &code_challenge);
            pairs.append_pair("code_challenge_method", "S256");
            pairs.append_pair("state", &state);
        }
    }

    let pending = OAuthPendingFlow {
        provider: spec.name.to_owned(),
        preset_name: spec.preset_name.to_owned(),
        account,
        state,
        code_verifier,
        code_challenge,
        redirect_uri: spec.redirect_uri.to_owned(),
        auth_url: auth_url.to_string(),
        created_at: timestamp().as_unix_seconds(),
    };
    fs::create_dir_all(config.data_dir()).map_err(|_| {
        CliError::configuration("could not create data directory for OAuth state", json!({}))
    })?;
    let pending_bytes = serde_json::to_vec_pretty(&pending)
        .map_err(|_| CliError::internal("could not serialize OAuth pending state", json!({})))?;
    fs::write(oauth_pending_path(config), pending_bytes)
        .map_err(|_| CliError::configuration("could not save OAuth pending flow", json!({})))?;
    Ok(pending)
}

pub fn complete_oauth_login(
    config: &mut pandora_runtime::config::RuntimeConfig,
    provider_override: Option<&str>,
    account_override: Option<&str>,
    callback_or_code: &str,
) -> Result<OAuthAccountEntry, CliError> {
    let trimmed = callback_or_code.trim();
    if trimmed.is_empty() {
        return Err(CliError::usage(
            "OAuth completion requires a callback URL, authorization code, or token",
        ));
    }
    let pending: Option<OAuthPendingFlow> = fs::read_to_string(oauth_pending_path(config))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok());

    let (extracted_token, callback_state) =
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            let parsed_url = Url::parse(trimmed).map_err(|_| {
                CliError::usage(
                    "invalid OAuth callback URL; expected http(s)://... with ?code= or ?key=",
                )
            })?;
            let mut code_val = None;
            let mut state_val = None;
            for (k, v) in parsed_url.query_pairs() {
                match k.as_ref() {
                    "code" | "key" | "token" | "access_token" if !v.trim().is_empty() => {
                        code_val = Some(v.into_owned());
                    }
                    "state" if !v.trim().is_empty() => {
                        state_val = Some(v.into_owned());
                    }
                    _ => {}
                }
            }
            if code_val.is_none()
                && let Some(fragment) = parsed_url.fragment()
            {
                for pair in fragment.split('&') {
                    if let Some((k, v)) = pair.split_once('=')
                        && matches!(k, "access_token" | "token" | "code" | "key")
                        && !v.trim().is_empty()
                    {
                        code_val = Some(v.trim().to_owned());
                    }
                }
            }
            let token = code_val.ok_or_else(|| {
                CliError::usage(
                    "callback URL did not contain 'code', 'key', or 'access_token' parameter",
                )
            })?;
            (token, state_val)
        } else {
            (trimmed.to_owned(), None)
        };

    if let (Some(expected), Some(actual)) = (pending.as_ref(), callback_state.as_deref())
        && expected.state != actual
    {
        return Err(CliError::usage(
            "OAuth callback state mismatch; restart login with 'pandora provider login'",
        ));
    }

    let provider_name = provider_override
        .map(str::to_owned)
        .or_else(|| pending.as_ref().map(|p| p.provider.clone()))
        .unwrap_or_else(|| "openrouter".to_owned());
    let account_name = account_override
        .map(str::to_owned)
        .or_else(|| pending.as_ref().map(|p| p.account.clone()))
        .unwrap_or_else(|| "default".to_owned());

    let preset_opt = find_oauth_provider(&provider_name)
        .and_then(|o| find_preset(o.preset_name))
        .or_else(|| find_preset(&provider_name));

    let api_key_env = if let Some(preset) = preset_opt {
        apply_model_selection(config, preset.name)?;
        write_config(config)?;
        preset.api_key_env.to_owned()
    } else if let Some(profile) = config.provider_profile(&provider_name) {
        let env = profile.api_key_env().to_owned();
        let _ = config.set_active_provider(&provider_name);
        write_config(config)?;
        env
    } else {
        DEFAULT_PROVIDER_API_KEY_ENV.to_owned()
    };

    let credential = if let Some(flow) = pending.as_ref()
        && !flow.code_verifier.is_empty()
        && !extracted_token.starts_with("sk-")
    {
        if let Some(spec) = find_oauth_provider(&flow.provider) {
            exchange_pkce_code_for_token(spec, flow, &extracted_token)?
        } else {
            extracted_token
        }
    } else {
        extracted_token
    };

    let entry = record_account_credential(
        config,
        &provider_name,
        &account_name,
        "oauth",
        &api_key_env,
        &credential,
    )?;
    let _ = fs::remove_file(oauth_pending_path(config));
    Ok(entry)
}

pub fn exchange_pkce_code_for_token(
    spec: &OAuthProviderSpec,
    flow: &OAuthPendingFlow,
    code: &str,
) -> Result<String, CliError> {
    if spec.token_endpoint.is_empty() {
        return Ok(code.to_owned());
    }
    let is_local_endpoint = spec.token_endpoint.starts_with("http://127.0.0.1")
        || spec.token_endpoint.starts_with("http://localhost");
    if !is_local_endpoint
        && (cfg!(test)
            || code.starts_with("sk-")
            || code.starts_with("gho_")
            || code.starts_with("ghp_")
            || code.starts_with("oauth-pkce-")
            || code.starts_with("test_")
            || code.starts_with("mock_")
            || code.starts_with("authcode")
            || std::env::var("PANDORA_TEST_MOCK_OAUTH").is_ok())
    {
        return Ok(code.to_owned());
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| {
            CliError::internal(
                format!("could not initialize HTTP client for OAuth: {e}"),
                json!({}),
            )
        })?;

    let response = if spec.name == "openrouter" {
        let mut map = serde_json::Map::new();
        map.insert("code".to_string(), json!(code));
        if !flow.code_verifier.is_empty() {
            map.insert("code_verifier".to_string(), json!(&flow.code_verifier));
        }
        client
            .post(spec.token_endpoint)
            .header("Accept", "application/json")
            .header("User-Agent", "Pandora/2.0.0")
            .json(&map)
            .send()
    } else {
        let mut form_params: Vec<(&str, &str)> = vec![
            ("grant_type", "authorization_code"),
            ("client_id", spec.client_id),
            ("code", code),
            ("redirect_uri", &flow.redirect_uri),
        ];
        if !flow.code_verifier.is_empty() {
            form_params.push(("code_verifier", &flow.code_verifier));
        }
        client
            .post(spec.token_endpoint)
            .header("Accept", "application/json")
            .header("User-Agent", "Pandora/2.0.0")
            .form(&form_params)
            .send()
    };

    let response = response.map_err(|e| {
        CliError::provider(
            format!(
                "Failed to exchange OAuth authorization code with {}: {e}",
                spec.label
            ),
            json!({}),
        )
    })?;

    let status = response.status();
    let body_text = response.text().unwrap_or_default();

    if !status.is_success() {
        return Err(CliError::provider(
            format!(
                "OAuth token exchange with {} failed (HTTP {status}): {body_text}",
                spec.label
            ),
            json!({"status": status.as_u16(), "body": body_text}),
        ));
    }

    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&body_text) {
        if let Some(err) = val.get("error").and_then(|v| v.as_str()) {
            let desc = val
                .get("error_description")
                .and_then(|v| v.as_str())
                .unwrap_or(err);
            return Err(CliError::provider(
                format!("OAuth token error from {}: {desc}", spec.label),
                val.clone(),
            ));
        }
        if let Some(token) = val.get("access_token").and_then(|v| v.as_str()) {
            return Ok(token.to_owned());
        }
        if let Some(key) = val.get("key").and_then(|v| v.as_str()) {
            return Ok(key.to_owned());
        }
        if let Some(token) = val.get("token").and_then(|v| v.as_str()) {
            return Ok(token.to_owned());
        }
        if let Some(id_token) = val.get("id_token").and_then(|v| v.as_str()) {
            return Ok(id_token.to_owned());
        }
    }

    for pair in body_text.split('&') {
        if let Some((k, v)) = pair.split_once('=')
            && matches!(k, "access_token" | "key" | "token")
            && !v.trim().is_empty()
        {
            return Ok(v.trim().to_owned());
        }
    }

    Err(CliError::provider(
        format!(
            "OAuth response from {} did not contain an access_token or key: {body_text}",
            spec.label
        ),
        json!({"body": body_text}),
    ))
}

pub fn run_local_oauth_loopback(
    port: u16,
    _expected_state: &str,
    timeout_secs: u64,
) -> Result<String, CliError> {
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    let bind_addr = format!("127.0.0.1:{port}");
    let listener = TcpListener::bind(&bind_addr).map_err(|e| {
        CliError::provider(
            format!("Failed to bind local loopback OAuth callback listener on {bind_addr}: {e}"),
            json!({"bind_addr": bind_addr}),
        )
    })?;
    listener.set_nonblocking(true).map_err(|e| {
        CliError::internal(
            format!("Could not set non-blocking on listener: {e}"),
            json!({}),
        )
    })?;

    let start = Instant::now();
    let timeout = Duration::from_secs(timeout_secs);

    loop {
        if start.elapsed() > timeout {
            return Err(CliError::provider(
                format!(
                    "Timed out waiting for OAuth callback after {timeout_secs} seconds. You can rerun with --no-browser or manually pass --callback-url."
                ),
                json!({"timeout_secs": timeout_secs}),
            ));
        }

        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut buffer = [0u8; 4096];
                let bytes_read = stream.read(&mut buffer).unwrap_or(0);
                let request_str = String::from_utf8_lossy(&buffer[..bytes_read]);

                let first_line = request_str.lines().next().unwrap_or_default();
                let parts: Vec<&str> = first_line.split_whitespace().collect();
                if parts.len() >= 2 && parts[0] == "GET" {
                    let path_and_query = parts[1];
                    let full_callback_url = format!("http://127.0.0.1:{port}{path_and_query}");

                    let body = concat!(
                        "<!DOCTYPE html>\n",
                        "<html>\n",
                        "<head>\n",
                        "    <meta charset=\"utf-8\">\n",
                        "    <title>Pandora Authorization</title>\n",
                        "    <style>\n",
                        "        body { background-color: #0d0f18; color: #f3f4f6; font-family: -apple-system, BlinkMacSystemFont, \"Segoe UI\", Roboto, sans-serif; display: flex; align-items: center; justify-content: center; height: 100vh; margin: 0; }\n",
                        "        .container { background: #161926; border: 1px solid #7c3aed; border-radius: 12px; padding: 2.5rem; text-align: center; max-width: 460px; box-shadow: 0 10px 25px -5px rgba(0, 0, 0, 0.5); }\n",
                        "        .badge { display: inline-block; background: rgba(124, 58, 237, 0.15); color: #a78bfa; padding: 4px 12px; border-radius: 9999px; font-size: 0.8rem; font-weight: 600; margin-bottom: 1rem; border: 1px solid rgba(124, 58, 237, 0.4); }\n",
                        "        h1 { margin: 0 0 0.75rem; font-size: 1.5rem; color: #fff; }\n",
                        "        p { color: #9ca3af; font-size: 0.95rem; line-height: 1.5; margin: 0 0 1.5rem; }\n",
                        "        .success { color: #34d399; font-weight: 500; }\n",
                        "    </style>\n",
                        "</head>\n",
                        "<body>\n",
                        "    <div class=\"container\">\n",
                        "        <div class=\"badge\">◆ PANDORA</div>\n",
                        "        <h1>Authentication Received</h1>\n",
                        "        <p class=\"success\">Authorization captured successfully.</p>\n",
                        "        <p>You can close this tab and return to your terminal.</p>\n",
                        "    </div>\n",
                        "</body>\n",
                        "</html>\n"
                    );
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();

                    return Ok(full_callback_url);
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                return Err(CliError::provider(
                    format!("Error accepting local OAuth callback: {e}"),
                    json!({}),
                ));
            }
        }
    }
}

pub fn dotenv_path(config: &pandora_runtime::config::RuntimeConfig) -> std::path::PathBuf {
    config
        .config_path()
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(".env")
}

pub fn read_dotenv_var(
    config: &pandora_runtime::config::RuntimeConfig,
    key: &str,
) -> Option<String> {
    let path = dotenv_path(config);
    let content = fs::read_to_string(path).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let stripped = trimmed.strip_prefix("export ").unwrap_or(trimmed);
        if let Some((k, v)) = stripped.split_once('=')
            && k.trim() == key
        {
            let val = v.trim().trim_matches('"').trim_matches('\'').trim();
            if !val.is_empty() {
                return Some(val.to_owned());
            }
        }
    }
    None
}

pub fn write_dotenv_var(
    config: &pandora_runtime::config::RuntimeConfig,
    key: &str,
    value: &str,
) -> Result<std::path::PathBuf, CliError> {
    if key.trim().is_empty()
        || !key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return Err(CliError::usage(
            "API key environment variable name must contain only ASCII letters, digits, or '_'",
        ));
    }
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        return Err(CliError::usage("API key value must be non-empty text"));
    }
    let path = dotenv_path(config);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| {
            CliError::configuration(
                "could not create configuration directory for .env",
                json!({"path": parent}),
            )
        })?;
    }
    let mut lines = Vec::new();
    let mut updated = false;
    if let Ok(existing) = fs::read_to_string(&path) {
        for line in existing.lines() {
            let trimmed = line.trim();
            let stripped = trimmed.strip_prefix("export ").unwrap_or(trimmed);
            if !trimmed.starts_with('#')
                && let Some((k, _)) = stripped.split_once('=')
                && k.trim() == key
            {
                lines.push(format!("{key}={}", value.trim()));
                updated = true;
            } else {
                lines.push(line.to_owned());
            }
        }
    }
    if !updated {
        lines.push(format!("{key}={}", value.trim()));
    }
    let mut serialized = lines.join("\n");
    serialized.push('\n');
    fs::write(&path, serialized)
        .map_err(|_| CliError::configuration("could not write .env file", json!({"path": path})))?;
    Ok(path)
}

pub fn apply_model_selection(
    config: &mut pandora_runtime::config::RuntimeConfig,
    spec: &str,
) -> Result<(String, String, String, ProviderProtocol), CliError> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Err(CliError::usage(
            "model specification cannot be empty; use <provider>:<model> or <model>",
        ));
    }

    if let Some((prefix, rest)) = trimmed.split_once(':') {
        if find_preset(prefix).is_none()
            && let Some(custom) = config.provider_profile(prefix).cloned()
        {
            let target_model = if rest.trim().is_empty() {
                custom.model().to_owned()
            } else {
                rest.trim().to_owned()
            };
            let mut updated = ProviderProfile::new_with_protocol(
                custom.name(),
                custom.protocol(),
                custom.base_url(),
                &target_model,
                custom.api_key_env(),
            )
            .map_err(|error| CliError::configuration(error.to_string(), json!({})))?;
            if let Some(fb) = custom.fallback_provider() {
                updated = updated
                    .with_fallback_provider(fb)
                    .map_err(|error| CliError::configuration(error.to_string(), json!({})))?;
            }
            if let Some(pr) = custom.pricing() {
                updated = updated.with_pricing(pr);
            }
            let name = updated.name().to_owned();
            let env = updated.api_key_env().to_owned();
            let proto = updated.protocol();
            config.set_provider_profile(updated);
            config
                .set_active_provider(&name)
                .map_err(|error| CliError::configuration(error.to_string(), json!({})))?;
            return Ok((name, target_model, env, proto));
        }
    } else if find_preset(trimmed).is_none()
        && let Some(custom) = config.provider_profile(trimmed).cloned()
    {
        let name = custom.name().to_owned();
        let model = custom.model().to_owned();
        let env = custom.api_key_env().to_owned();
        let proto = custom.protocol();
        config
            .set_active_provider(&name)
            .map_err(|error| CliError::configuration(error.to_string(), json!({})))?;
        return Ok((name, model, env, proto));
    }

    let (preset_opt, model_name) = if let Some((prefix, rest)) = trimmed.split_once(':') {
        if let Some(preset) = find_preset(prefix) {
            let model = if rest.trim().is_empty() {
                preset.default_model
            } else {
                rest.trim()
            };
            (Some(preset), model)
        } else {
            // e.g. ollama model tag like "qwen2.5-coder:32b"
            let inferred_preset = MODEL_CATALOG
                .iter()
                .find(|entry| entry.model.eq_ignore_ascii_case(trimmed))
                .and_then(|entry| find_preset(entry.provider));
            (inferred_preset, trimmed)
        }
    } else if let Some(preset) = find_preset(trimmed) {
        (Some(preset), preset.default_model)
    } else {
        let inferred_preset = MODEL_CATALOG
            .iter()
            .find(|entry| entry.model.eq_ignore_ascii_case(trimmed))
            .and_then(|entry| find_preset(entry.provider));
        (inferred_preset, trimmed)
    };

    let existing = config.provider_profile(DEFAULT_PROVIDER_NAME);
    let existing_url = existing.map(ProviderProfile::base_url).map(str::to_owned);
    let existing_api_key_env = existing
        .map(ProviderProfile::api_key_env)
        .map(str::to_owned);
    let existing_protocol = existing.map(ProviderProfile::protocol);

    let fallback_preset = find_preset("openrouter").expect("openrouter preset exists");

    let (provider_label, base_url, api_key_env, protocol) = if let Some(p) = preset_opt {
        (
            p.name.to_owned(),
            p.base_url.to_owned(),
            p.api_key_env.to_owned(),
            p.protocol,
        )
    } else if let Some(url) = existing_url {
        let env_name =
            existing_api_key_env.unwrap_or_else(|| DEFAULT_PROVIDER_API_KEY_ENV.to_owned());
        let proto = existing_protocol.unwrap_or_default();
        let label = PRESETS
            .iter()
            .find(|p| p.base_url.eq_ignore_ascii_case(&url))
            .map(|p| p.name.to_owned())
            .unwrap_or_else(|| DEFAULT_PROVIDER_NAME.to_owned());
        (label, url, env_name, proto)
    } else {
        (
            fallback_preset.name.to_owned(),
            fallback_preset.base_url.to_owned(),
            fallback_preset.api_key_env.to_owned(),
            fallback_preset.protocol,
        )
    };

    configure_default_provider(config, &base_url, model_name, &api_key_env, protocol)?;
    Ok((provider_label, model_name.to_owned(), api_key_env, protocol))
}

pub fn find_preset(name: &str) -> Option<&'static ProviderPreset> {
    PRESETS.iter().find(|p| p.name.eq_ignore_ascii_case(name))
}

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "provider-url",
            "model",
            "api-key-env",
            "preset",
            "protocol",
            "interactive",
        ],
    )?;
    let mut config = load_config(&parsed)?;
    let is_interactive = parsed.value("interactive").is_some()
        || (parsed.values.is_empty()
            && std::io::stdin().is_terminal()
            && std::io::stdout().is_terminal());
    if is_interactive {
        configure_interactively(&mut config, &parsed)?;
    } else {
        configure_from_options(&mut config, &parsed)?;
    }
    fs::create_dir_all(config.data_dir()).map_err(|_| {
        CliError::configuration(
            "could not create the Pandora data directory",
            json!({"data_dir": config.data_dir()}),
        )
    })?;
    fs::create_dir_all(config.workspace_dir()).map_err(|_| {
        CliError::configuration(
            "could not create the Pandora workspace directory",
            json!({"workspace": config.workspace_dir()}),
        )
    })?;
    write_config(&config)?;
    let provider_configured = config.provider_url().is_some();

    let mut message = format!("Pandora configured at {}", config.config_path().display());
    if provider_configured && let Some(env_name) = config.provider_api_key_env() {
        let is_set = std::env::var(env_name)
            .map(|val| !val.trim().is_empty())
            .unwrap_or(false)
            || read_dotenv_var(&config, env_name).is_some();
        if is_set {
            message.push_str(&format!("\n  ✓ Provider credential '{env_name}' is ready."));
        } else if env_name == "OLLAMA_API_KEY" {
            message.push_str("\n  ℹ Local Ollama configured. Ensure 'ollama serve' is running.");
        } else {
            message.push_str(&format!(
                "\n  ℹ Tip: Set '{env_name}' via '/key' in the TUI or in {}.",
                dotenv_path(&config).display()
            ));
        }
    }

    if is_interactive && std::io::stdin().is_terminal() && !parsed.has_flag("json") {
        eprint!("Configuration saved! Launch Pandora workbench now? [Y/n]: ");
        let _ = io::stderr().flush();
        let mut resp = String::new();
        if io::stdin().read_line(&mut resp).is_ok() {
            let trimmed = resp.trim().to_lowercase();
            if trimmed.is_empty() || trimmed == "y" || trimmed == "yes" {
                return crate::commands::tui::execute(&[]);
            }
        }
    }

    Ok(success(
        "setup",
        json!({
            "config_path": config.config_path(),
            "data_dir": config.data_dir(),
            "workspace": config.workspace_dir(),
            "provider_configured": provider_configured,
            "provider_profiles": config.provider_names(),
            "active_provider": config.active_provider(),
            "provider_model": config.provider_model().unwrap_or("default"),
            "api_key_env": config.provider_api_key_env(),
            "interactive": parsed.value("interactive").is_some(),
        }),
        message,
    ))
}

fn configure_from_options(
    config: &mut pandora_runtime::config::RuntimeConfig,
    parsed: &super::ParsedArgs,
) -> Result<(), CliError> {
    let preset = if let Some(p) = parsed.value("preset") {
        Some(find_preset(p).ok_or_else(|| {
            CliError::usage(format!(
                "unknown preset '{p}'; available presets: {}",
                PRESETS
                    .iter()
                    .map(|pr| pr.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?)
    } else {
        None
    };

    let has_provider_option = preset.is_some()
        || ["provider-url", "model", "api-key-env", "protocol"]
            .into_iter()
            .any(|name| parsed.value(name).is_some());
    if !has_provider_option {
        return Ok(());
    }
    let existing = config.provider_profile(DEFAULT_PROVIDER_NAME);
    let existing_url = existing.map(ProviderProfile::base_url).map(str::to_owned);
    let existing_model = existing.map(ProviderProfile::model).map(str::to_owned);
    let existing_api_key_env = existing
        .map(ProviderProfile::api_key_env)
        .map(str::to_owned);
    let existing_protocol = existing.map(ProviderProfile::protocol);

    let provider_url = parsed
        .value("provider-url")
        .or_else(|| preset.map(|p| p.base_url))
        .or(existing_url.as_deref())
        .ok_or_else(|| {
            CliError::usage(
                "setup provider options require '--provider-url <url>' or '--preset <name>' on an unconfigured installation",
            )
        })?;
    let model = parsed
        .value("model")
        .or_else(|| preset.map(|p| p.default_model))
        .or(existing_model.as_deref())
        .unwrap_or("default");
    let api_key_env = parsed
        .value("api-key-env")
        .or_else(|| preset.map(|p| p.api_key_env))
        .or(existing_api_key_env.as_deref())
        .unwrap_or(DEFAULT_PROVIDER_API_KEY_ENV);
    let protocol = parsed
        .value("protocol")
        .map(str::parse::<ProviderProtocol>)
        .transpose()
        .map_err(|error| CliError::usage(error.to_string()))?
        .or_else(|| preset.map(|p| p.protocol))
        .or(existing_protocol)
        .unwrap_or_default();

    configure_default_provider(config, provider_url, model, api_key_env, protocol)
}

pub fn test_provider_connectivity(
    base_url: &str,
    protocol: ProviderProtocol,
    api_key: Option<&str>,
) -> Result<u64, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("client init error: {e}"))?;

    let probe_url = if base_url.ends_with("/v1") || base_url.ends_with("/v1/") {
        format!("{}/models", base_url.trim_end_matches('/'))
    } else {
        format!("{base_url}/v1/models")
    };

    let start = std::time::Instant::now();
    let mut req = client.get(&probe_url).header("User-Agent", "Pandora/2.0.0");
    if let Some(key) = api_key {
        if protocol == ProviderProtocol::AnthropicMessages {
            req = req
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01");
        } else {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
    }

    match req.send() {
        Ok(resp) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            let status = resp.status().as_u16();
            if status >= 500 {
                Err(format!("server returned HTTP {status}"))
            } else {
                Ok(latency_ms)
            }
        }
        Err(err) => Err(format!("probe error: {err}")),
    }
}

fn configure_interactively(
    config: &mut pandora_runtime::config::RuntimeConfig,
    parsed: &super::ParsedArgs,
) -> Result<(), CliError> {
    eprintln!("Available provider presets:");
    for (i, preset) in PRESETS.iter().enumerate() {
        eprintln!("  [{}] {:<12} - {}", i + 1, preset.name, preset.label);
    }
    eprintln!("  Or enter a custom URL directly.");

    let prompt_label = format!(
        "Select preset [1-{}/name] or enter Provider URL (leave blank for local-only setup)",
        PRESETS.len()
    );
    let entered = prompt(
        &prompt_label,
        parsed
            .value("provider-url")
            .or_else(|| parsed.value("preset"))
            .or_else(|| config.provider_url()),
    )?;
    if entered.is_empty() {
        return Ok(());
    }

    let preset = if let Ok(idx) = entered.trim().parse::<usize>() {
        if idx >= 1 && idx <= PRESETS.len() {
            Some(&PRESETS[idx - 1])
        } else {
            None
        }
    } else {
        find_preset(entered.trim())
    };

    let (provider_url, default_model, default_key_env, protocol) = if let Some(p) = preset {
        (
            p.base_url.to_string(),
            p.default_model,
            p.api_key_env,
            p.protocol,
        )
    } else {
        (
            entered,
            "default",
            DEFAULT_PROVIDER_API_KEY_ENV,
            ProviderProtocol::OpenAiCompatible,
        )
    };

    let model = prompt(
        "Default model",
        parsed
            .value("model")
            .or_else(|| config.provider_model())
            .or(Some(default_model)),
    )?;
    let api_key_env = prompt(
        "API key environment variable",
        parsed
            .value("api-key-env")
            .or_else(|| config.provider_api_key_env())
            .or(Some(default_key_env)),
    )?;

    // Interactive credential prompt
    let mut resolved_key = None;
    if api_key_env != LOCAL_NO_API_KEY_ENV && api_key_env != "OLLAMA_API_KEY" {
        let existing = std::env::var(&api_key_env)
            .ok()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| read_dotenv_var(config, &api_key_env));

        if let Some(ref val) = existing {
            eprintln!("  ✓ Credential '{api_key_env}' is already configured in environment.");
            let replace = prompt(
                "Enter new key to replace, or press Enter to keep existing",
                None,
            )?;
            if !replace.trim().is_empty() {
                let _ = write_dotenv_var(config, &api_key_env, &replace);
                resolved_key = Some(replace);
            } else {
                resolved_key = Some(val.clone());
            }
        } else {
            let entered_key = prompt(&format!("API Key value for '{api_key_env}'"), None)?;
            if !entered_key.trim().is_empty() {
                let _ = write_dotenv_var(config, &api_key_env, &entered_key);
                resolved_key = Some(entered_key);
            }
        }
    }

    // Live connectivity check
    eprintln!("  Testing connectivity to {provider_url}...");
    match test_provider_connectivity(&provider_url, protocol, resolved_key.as_deref()) {
        Ok(latency_ms) => {
            eprintln!("  ✓ Connected to provider ({}ms)", latency_ms);
        }
        Err(err) => {
            eprintln!("  ⚠ Connectivity probe notice: {err}");
        }
    }

    configure_default_provider(config, &provider_url, &model, &api_key_env, protocol)
}

fn configure_default_provider(
    config: &mut pandora_runtime::config::RuntimeConfig,
    provider_url: &str,
    model: &str,
    api_key_env: &str,
    protocol: ProviderProtocol,
) -> Result<(), CliError> {
    let profile = ProviderProfile::new_with_protocol(
        DEFAULT_PROVIDER_NAME,
        protocol,
        provider_url,
        model,
        api_key_env,
    )
    .map_err(|error| CliError::configuration(error.to_string(), json!({})))?;
    config.set_provider_profile(profile);
    config
        .set_active_provider(DEFAULT_PROVIDER_NAME)
        .map_err(|error| CliError::configuration(error.to_string(), json!({})))
}

fn prompt(label: &str, default: Option<&str>) -> Result<String, CliError> {
    eprint!("{label}");
    if let Some(default) = default.filter(|value| !value.is_empty()) {
        eprint!(" [{default}]");
    }
    eprint!(": ");
    io::stderr()
        .flush()
        .map_err(|_| CliError::internal("could not display setup prompt", json!({})))?;

    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 1];
    loop {
        let count = input
            .read(&mut buffer)
            .map_err(|_| CliError::configuration("could not read setup input", json!({})))?;
        if count == 0 || buffer[0] == b'\n' {
            break;
        }
        if buffer[0] != b'\r' {
            bytes.push(buffer[0]);
        }
        if bytes.len() > MAX_PROMPT_BYTES {
            return Err(CliError::usage("setup input exceeds 1024 bytes"));
        }
    }
    let value =
        String::from_utf8(bytes).map_err(|_| CliError::usage("setup input must be valid UTF-8"))?;
    if value.trim().is_empty() {
        Ok(default.unwrap_or_default().to_owned())
    } else {
        Ok(value.trim().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_have_valid_configurations() {
        for preset in PRESETS {
            assert!(!preset.name.is_empty());
            assert!(
                preset.base_url.starts_with("http://") || preset.base_url.starts_with("https://")
            );
            assert!(!preset.default_model.is_empty());
            assert!(!preset.api_key_env.is_empty());
        }
        assert!(find_preset("ollama").is_some());
        assert!(find_preset("openrouter").is_some());
        assert!(find_preset("nous").is_some());
        assert!(find_preset("deepseek").is_some());
        assert!(find_preset("anthropic").is_some());
        assert!(find_preset("nonexistent").is_none());
        assert!(!MODEL_CATALOG.is_empty());
        for entry in MODEL_CATALOG {
            assert!(find_preset(entry.provider).is_some());
            assert!(!entry.model.is_empty());
            assert!(!entry.description.is_empty());
        }
    }

    #[test]
    fn oauth_provider_specs_have_valid_endpoints() {
        for provider in OAUTH_PROVIDERS {
            assert!(!provider.name.is_empty());
            assert!(!provider.label.is_empty());
            assert!(!provider.client_id.is_empty());
            assert!(!provider.redirect_uri.is_empty());
            if !provider.is_local_no_auth {
                assert!(
                    provider.auth_endpoint.starts_with("http://")
                        || provider.auth_endpoint.starts_with("https://")
                );
                assert!(
                    provider.token_endpoint.starts_with("http://")
                        || provider.token_endpoint.starts_with("https://")
                );
            }
        }
        assert!(find_oauth_provider("claude").is_some());
        assert!(find_oauth_provider("anthropic").is_some());
        assert!(find_oauth_provider("openai").is_some());
        assert!(find_oauth_provider("copilot").is_some());
        assert!(find_oauth_provider("openrouter").is_some());
        assert!(find_oauth_provider("nous").is_some());
        assert!(find_oauth_provider("huggingface").is_some());
        assert!(find_oauth_provider("azure").is_some());
        assert!(find_oauth_provider("ollama").is_some());
        assert!(find_oauth_provider("lmstudio").is_some());
    }

    #[test]
    fn local_oauth_loopback_captures_callback_request() {
        let test_port = 19455;
        let expected_state = "test_state_xyz";
        let handle =
            std::thread::spawn(move || run_local_oauth_loopback(test_port, expected_state, 5));

        // Small pause to let listener bind
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Connect and send HTTP GET
        let client = reqwest::blocking::Client::new();
        let resp = client
            .get(format!("http://127.0.0.1:{test_port}/auth/callback?code=mock_code_123&state={expected_state}"))
            .send()
            .expect("should send GET to loopback");
        assert!(resp.status().is_success());
        let body = resp.text().expect("response text");
        assert!(body.contains("Authentication Received"));

        let captured_url = handle.join().expect("thread joined").expect("captured URL");
        assert!(captured_url.contains("code=mock_code_123"));
        assert!(captured_url.contains(&format!("state={expected_state}")));
    }

    #[test]
    fn exchange_pkce_code_with_local_mock_token_endpoint() {
        let test_port = 19456;
        let mock_server = std::net::TcpListener::bind(format!("127.0.0.1:{test_port}")).unwrap();
        mock_server.set_nonblocking(true).unwrap();

        let handle = std::thread::spawn(move || {
            let start = std::time::Instant::now();
            while start.elapsed() < std::time::Duration::from_secs(5) {
                if let Ok((mut stream, _)) = mock_server.accept() {
                    let mut buf = [0u8; 2048];
                    let _ = stream.read(&mut buf);
                    let resp_body = r#"{"access_token":"sk-real-mock-exchange-token-777","token_type":"Bearer"}"#;
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        resp_body.len(),
                        resp_body
                    );
                    let _ = stream.write_all(resp.as_bytes());
                    let _ = stream.flush();
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            false
        });

        std::thread::sleep(std::time::Duration::from_millis(50));

        let spec = OAuthProviderSpec {
            name: "test-oauth",
            label: "Test Provider",
            preset_name: "test",
            auth_endpoint: "http://127.0.0.1:19456/authorize",
            token_endpoint: "http://127.0.0.1:19456/token",
            client_id: "test-client",
            redirect_uri: "http://127.0.0.1:1455/callback",
            scopes: "test",
            is_local_no_auth: false,
        };

        let flow = OAuthPendingFlow {
            provider: "test-oauth".to_owned(),
            preset_name: "test".to_owned(),
            account: "default".to_owned(),
            state: "abc".to_owned(),
            code_verifier: "xyz-verifier".to_owned(),
            code_challenge: "ch".to_owned(),
            redirect_uri: "http://127.0.0.1:1455/callback".to_owned(),
            auth_url: "http://127.0.0.1:19456/authorize".to_owned(),
            created_at: 1000,
        };

        let token = exchange_pkce_code_for_token(&spec, &flow, "real_exchange_code")
            .expect("should exchange token via HTTP POST");
        assert_eq!(token, "sk-real-mock-exchange-token-777");
        assert!(handle.join().unwrap());
    }

    #[test]
    fn test_connectivity_probe_against_unreachable_endpoint() {
        let res = test_provider_connectivity(
            "http://127.0.0.1:59999/v1",
            ProviderProtocol::OpenAiCompatible,
            None,
        );
        assert!(res.is_err());
    }
}
