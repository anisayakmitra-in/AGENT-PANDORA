use super::{load_config, parse_options, write_config};
use crate::output::{CliError, CommandResult, success};
use pandora_provider::ProviderProtocol;
use pandora_runtime::config::{
    DEFAULT_PROVIDER_API_KEY_ENV, DEFAULT_PROVIDER_NAME, ProviderProfile,
};
use serde_json::json;
use std::fs;
use std::io::{self, Read, Write};

const MAX_PROMPT_BYTES: usize = 1_024;

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
        label: "OpenAI Direct",
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
];

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
    if parsed.value("interactive").is_some() {
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

fn configure_interactively(
    config: &mut pandora_runtime::config::RuntimeConfig,
    parsed: &super::ParsedArgs,
) -> Result<(), CliError> {
    eprintln!("Available provider presets:");
    for (i, preset) in PRESETS.iter().enumerate() {
        eprintln!("  [{}] {:<12} - {}", i + 1, preset.name, preset.label);
    }
    eprintln!("  Or enter a custom URL directly.");

    let entered = prompt(
        "Select preset [1-7/name] or enter Provider URL (leave blank for local-only setup)",
        parsed
            .value("provider-url")
            .or_else(|| parsed.value("preset"))
            .or_else(|| config.provider_url()),
    )?;
    if entered.is_empty() {
        return Ok(());
    }

    let preset = match entered.trim().to_lowercase().as_str() {
        "1" | "ollama" => find_preset("ollama"),
        "2" | "openrouter" => find_preset("openrouter"),
        "3" | "deepseek" => find_preset("deepseek"),
        "4" | "openai" => find_preset("openai"),
        "5" | "anthropic" => find_preset("anthropic"),
        "6" | "groq" => find_preset("groq"),
        "7" | "nous" => find_preset("nous"),
        other => find_preset(other),
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
}
