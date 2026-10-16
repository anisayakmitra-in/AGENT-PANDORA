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
];

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
            .unwrap_or(false);
        if is_set {
            message.push_str(&format!("\n  ✓ Provider credential '{env_name}' is set."));
        } else if env_name == "OLLAMA_API_KEY" {
            message.push_str("\n  ℹ Local Ollama configured. Ensure 'ollama serve' is running.");
        } else {
            message.push_str(&format!(
                "\n  ℹ Tip: Set '{env_name}' in your environment before running tasks."
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
        "Select preset [1-6/name] or enter Provider URL (leave blank for local-only setup)",
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
        assert!(find_preset("deepseek").is_some());
        assert!(find_preset("anthropic").is_some());
        assert!(find_preset("nonexistent").is_none());
    }
}
