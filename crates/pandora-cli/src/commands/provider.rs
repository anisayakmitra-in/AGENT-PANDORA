use super::{
    load_config, load_config_storage, parse_options, session_scope, timestamp, write_config,
};
use crate::output::{CliError, CommandResult, success};
use pandora_provider::{
    ChatMessage, FailoverProvider, HttpProvider, ModelRequest, Provider, ProviderManifest,
    ProviderProtocol,
};
use pandora_runtime::config::{
    DEFAULT_PROVIDER_API_KEY_ENV, DEFAULT_PROVIDER_NAME, ProviderPricing, ProviderProfile,
    RuntimeConfig,
};
use pandora_runtime::executors::WorkspaceRoot;
use pandora_runtime::{ExecutionController, SecretVault};
use pandora_types::{Capability, PolicyContext, Session, SessionId};
use serde_json::json;

pub(crate) fn configured_provider_for(
    config: &RuntimeConfig,
    model: &str,
    operation: &str,
    provider_override: Option<&str>,
) -> Result<Box<dyn Provider>, CliError> {
    configured_provider_for_with_failover(config, model, operation, provider_override, true)
}

/// Resolves exactly the named provider without a fallback route.
///
/// Research generation uses this path because its bounded evidence is scoped
/// and persisted against that provider identity; silently sending it to a
/// fallback would break that provenance boundary.
pub(crate) fn configured_research_provider_for(
    config: &RuntimeConfig,
    model: &str,
    operation: &str,
    provider_override: Option<&str>,
) -> Result<Box<dyn Provider>, CliError> {
    configured_provider_for_with_failover(config, model, operation, provider_override, false)
}

fn configured_provider_for_with_failover(
    config: &RuntimeConfig,
    model: &str,
    operation: &str,
    provider_override: Option<&str>,
    allow_fallback: bool,
) -> Result<Box<dyn Provider>, CliError> {
    let provider_name = provider_override
        .or(config.active_provider())
        .unwrap_or(DEFAULT_PROVIDER_NAME);
    let selected_profile = config.provider_profile(provider_name);
    if provider_override.is_some() && selected_profile.is_none() {
        return Err(CliError::configuration(
            "provider is not configured",
            json!({"provider": provider_name}),
        ));
    }
    let base_url = selected_profile
        .map(ProviderProfile::base_url)
        .or_else(|| config.provider_url())
        .ok_or_else(|| {
            CliError::configuration(
                format!(
                    "{operation} requires a configured provider; run 'pandora provider set' first"
                ),
                json!({"config_path": config.config_path()}),
            )
        })?;
    let api_key_env = selected_profile
        .map(ProviderProfile::api_key_env)
        .or_else(|| config.provider_api_key_env())
        .unwrap_or(DEFAULT_PROVIDER_API_KEY_ENV);
    let protocol = selected_profile
        .map(ProviderProfile::protocol)
        .unwrap_or_default();
    let manifest = ProviderManifest::new_with_protocol(
        provider_name,
        provider_name,
        protocol,
        base_url,
        model,
        api_key_env,
    )
    .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    let primary_secret = configured_credential(config, api_key_env)?
        .ok_or_else(|| CliError::provider("provider credential unavailable", json!({})))?;
    let primary = HttpProvider::new(manifest, primary_secret)
        .map_err(|error| CliError::provider(error.to_string(), json!({})))?;

    if !allow_fallback {
        return Ok(Box::new(primary));
    }

    let Some(fallback_name) = config
        .provider_profile(provider_name)
        .and_then(ProviderProfile::fallback_provider)
    else {
        return Ok(Box::new(primary));
    };
    let fallback_profile = config
        .provider_profile(fallback_name)
        .ok_or_else(|| CliError::configuration("provider is not configured", json!({})))?;
    let fallback_manifest = ProviderManifest::new_with_protocol(
        fallback_profile.name(),
        fallback_profile.name(),
        fallback_profile.protocol(),
        fallback_profile.base_url(),
        fallback_profile.model(),
        fallback_profile.api_key_env(),
    )
    .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    let fallback_secret = configured_credential(config, fallback_profile.api_key_env())?
        .ok_or_else(|| CliError::provider("provider credential unavailable", json!({})))?;
    let fallback = HttpProvider::new(fallback_manifest, fallback_secret)
        .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    Ok(Box::new(FailoverProvider::new(
        Box::new(primary),
        Box::new(fallback),
    )))
}

pub(crate) fn provider_credential_available(
    config: &RuntimeConfig,
    name: &str,
) -> Result<bool, CliError> {
    Ok(configured_credential(config, name)?.is_some())
}

fn resolve_candidate_env(config: &RuntimeConfig, key: &str) -> Option<String> {
    if let Ok(value) = std::env::var(key) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_owned());
        }
    }
    if let Some(value) = super::setup::read_dotenv_var(config, key) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_owned());
        }
    }
    None
}

const STANDARD_VENDOR_KEY_ENVS: &[&str] = &[
    "GROQ_API_KEY",
    "OPENROUTER_API_KEY",
    "ANTHROPIC_API_KEY",
    "OPENAI_API_KEY",
    "DEEPSEEK_API_KEY",
];

pub(crate) fn configured_credential(
    config: &RuntimeConfig,
    name: &str,
) -> Result<Option<String>, CliError> {
    if let Some(value) = resolve_candidate_env(config, name) {
        return Ok(Some(value));
    }

    if name == DEFAULT_PROVIDER_API_KEY_ENV || name == "PANDORA_API_KEY" {
        if let Some(active_env) = config
            .active_provider()
            .and_then(|p| config.provider_profile(p))
            .map(ProviderProfile::api_key_env)
            && active_env != name
            && let Some(value) = resolve_candidate_env(config, active_env)
        {
            return Ok(Some(value));
        }
        for &vendor_key in STANDARD_VENDOR_KEY_ENVS {
            if let Some(value) = resolve_candidate_env(config, vendor_key) {
                return Ok(Some(value));
            }
        }
    } else if STANDARD_VENDOR_KEY_ENVS.contains(&name) {
        if let Some(value) = resolve_candidate_env(config, DEFAULT_PROVIDER_API_KEY_ENV) {
            return Ok(Some(value));
        }
        if let Some(value) = resolve_candidate_env(config, "PANDORA_API_KEY") {
            return Ok(Some(value));
        }
    }

    if matches!(
        name,
        "OLLAMA_API_KEY" | "LMSTUDIO_API_KEY" | "VLLM_API_KEY" | super::setup::LOCAL_NO_API_KEY_ENV
    ) {
        return Ok(Some("local-no-api-key".to_owned()));
    }
    let Some(passphrase) = std::env::var_os("PANDORA_MASTER_KEY") else {
        return Ok(None);
    };
    let passphrase = passphrase.into_string().map_err(|_| {
        CliError::configuration(
            "PANDORA_MASTER_KEY must contain valid text",
            json!({"environment": "PANDORA_MASTER_KEY"}),
        )
    })?;
    let (_, tenant, workspace) = session_scope();
    let vault = SecretVault::open(config.data_dir(), tenant, workspace, passphrase)
        .map_err(|error| CliError::configuration(error.to_string(), json!({})))?;

    if let Some(secret) = vault
        .get(name)
        .map_err(|error| CliError::configuration(error.to_string(), json!({})))?
    {
        return Ok(Some(secret.into_string()));
    }

    if name == DEFAULT_PROVIDER_API_KEY_ENV || name == "PANDORA_API_KEY" {
        for &vendor_key in STANDARD_VENDOR_KEY_ENVS {
            if let Some(secret) = vault
                .get(vendor_key)
                .map_err(|error| CliError::configuration(error.to_string(), json!({})))?
            {
                return Ok(Some(secret.into_string()));
            }
        }
    } else if STANDARD_VENDOR_KEY_ENVS.contains(&name) {
        if let Some(secret) = vault
            .get(DEFAULT_PROVIDER_API_KEY_ENV)
            .map_err(|error| CliError::configuration(error.to_string(), json!({})))?
        {
            return Ok(Some(secret.into_string()));
        }
        if let Some(secret) = vault
            .get("PANDORA_API_KEY")
            .map_err(|error| CliError::configuration(error.to_string(), json!({})))?
        {
            return Ok(Some(secret.into_string()));
        }
    }

    Ok(None)
}

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage(
            "provider requires 'list', 'set', 'add', 'remove', 'use', 'login', 'accounts', 'models', or 'test'",
        )
    })?;
    match subcommand.as_str() {
        "list" => list(&args[1..]),
        "set" | "add" => set(&args[1..]),
        "remove" | "rm" => remove_provider(&args[1..]),
        "use" => use_provider(&args[1..]),
        "login" | "oauth" => login_provider(&args[1..]),
        "accounts" | "account" => accounts_command(&args[1..]),
        "models" | "inspect" | "scan" => super::models::execute(&args[1..]),
        "test" | "auth-test" => test(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown provider command '{unknown}'"
        ))),
    }
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    let config = load_config(&parsed)?;
    let providers = config
        .provider_names()
        .into_iter()
        .map(|name| {
            let profile = config
                .provider_profile(&name)
                .expect("provider names must resolve to profiles");
            let manifest = ProviderManifest::new_with_protocol(
                profile.name(),
                profile.name(),
                profile.protocol(),
                profile.base_url(),
                profile.model(),
                profile.api_key_env(),
            )
            .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
            let mut value = serde_json::to_value(manifest).map_err(|_| {
                CliError::internal("could not serialize provider metadata", json!({}))
            })?;
            if let Some(object) = value.as_object_mut() {
                object.insert(
                    "active".to_owned(),
                    json!(config.active_provider() == Some(profile.name())),
                );
                object.insert(
                    "fallback_provider".to_owned(),
                    json!(profile.fallback_provider()),
                );
                object.insert(
                    "pricing".to_owned(),
                    json!(profile.pricing().map(|pricing| {
                        json!({
                            "input_micros_per_million_tokens": pricing
                                .input_micros_per_million_tokens(),
                            "output_micros_per_million_tokens": pricing
                                .output_micros_per_million_tokens(),
                        })
                    })),
                );
            }
            Ok(value)
        })
        .collect::<Result<Vec<_>, CliError>>()?;
    Ok(success(
        "provider list",
        json!({"providers": providers}),
        if providers.is_empty() {
            "No providers configured".to_owned()
        } else {
            format!("{} provider configured", providers.len())
        },
    ))
}

fn set(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "name",
            "protocol",
            "provider-url",
            "base-url",
            "model",
            "api-key-env",
            "api-key",
            "api-key-stdin",
            "no-api-key",
            "set-default",
            "fallback-provider",
            "input-micros-per-million-tokens",
            "output-micros-per-million-tokens",
        ],
    )?;
    if parsed.positionals.len() > 1 {
        return Err(CliError::usage(
            "provider set/add accepts at most one positional provider profile name",
        ));
    }
    let resolved_url = parsed
        .value("provider-url")
        .or_else(|| parsed.value("base-url"))
        .ok_or_else(|| {
            CliError::usage("provider set requires '--provider-url <url>' (or '--base-url <url>')")
        })?;
    let mut config = load_config(&parsed)?;
    let name = parsed
        .value("name")
        .or_else(|| parsed.positionals.first().map(String::as_str))
        .unwrap_or(DEFAULT_PROVIDER_NAME);
    let model = parsed
        .value("model")
        .or_else(|| config.provider_profile(name).map(ProviderProfile::model))
        .unwrap_or("default");

    let default_custom_env = if name != DEFAULT_PROVIDER_NAME
        && (parsed.value("api-key").is_some() || parsed.has_flag("api-key-stdin"))
    {
        let sanitized = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect::<String>();
        format!("{sanitized}_API_KEY")
    } else {
        DEFAULT_PROVIDER_API_KEY_ENV.to_owned()
    };

    let api_key_env: String = if parsed.has_flag("no-api-key") {
        super::setup::LOCAL_NO_API_KEY_ENV.to_owned()
    } else if let Some(explicit_env) = parsed.value("api-key-env") {
        explicit_env.to_owned()
    } else if let Some(existing_env) = config
        .provider_profile(name)
        .map(ProviderProfile::api_key_env)
    {
        existing_env.to_owned()
    } else {
        default_custom_env
    };

    if parsed.has_flag("api-key-stdin") {
        use std::io::Read as _;
        let mut stdin_buf = String::new();
        std::io::stdin()
            .read_to_string(&mut stdin_buf)
            .map_err(|_| CliError::configuration("could not read API key from stdin", json!({})))?;
        let key_trimmed = stdin_buf.trim();
        if key_trimmed.is_empty() {
            return Err(CliError::usage("--api-key-stdin received an empty API key"));
        }
        let _ = super::setup::record_account_credential(
            &config,
            name,
            "default",
            "api_key",
            &api_key_env,
            key_trimmed,
        )?;
    } else if let Some(inline_key) = parsed.value("api-key") {
        let _ = super::setup::record_account_credential(
            &config,
            name,
            "default",
            "api_key",
            &api_key_env,
            inline_key.trim(),
        )?;
    }

    let protocol = parsed
        .value("protocol")
        .map(str::parse::<ProviderProtocol>)
        .transpose()
        .map_err(|error| CliError::usage(error.to_string()))?
        .or_else(|| config.provider_profile(name).map(ProviderProfile::protocol))
        .unwrap_or_default();
    let fallback_provider = parsed.value("fallback-provider").or_else(|| {
        config
            .provider_profile(name)
            .and_then(ProviderProfile::fallback_provider)
    });
    let input_pricing = parsed
        .value("input-micros-per-million-tokens")
        .map(|value| parse_pricing(value, "input-micros-per-million-tokens"))
        .transpose()?;
    let output_pricing = parsed
        .value("output-micros-per-million-tokens")
        .map(|value| parse_pricing(value, "output-micros-per-million-tokens"))
        .transpose()?;
    let pricing = match (input_pricing, output_pricing) {
        (None, None) => config
            .provider_profile(name)
            .and_then(ProviderProfile::pricing),
        (Some(input), Some(output)) => Some(ProviderPricing::new(input, output)),
        _ => {
            return Err(CliError::usage(
                "provider pricing requires both input and output token rates",
            ));
        }
    };
    let mut profile =
        ProviderProfile::new_with_protocol(name, protocol, resolved_url, model, &api_key_env)
            .map_err(|error| CliError::configuration(error.to_string(), json!({})))?;
    if let Some(fallback_provider) = fallback_provider {
        profile = profile
            .with_fallback_provider(fallback_provider)
            .map_err(|error| CliError::configuration(error.to_string(), json!({})))?;
    }
    if let Some(pricing) = pricing {
        profile = profile.with_pricing(pricing);
    }
    config.set_provider_profile(profile);
    config
        .set_active_provider(name)
        .map_err(|error| CliError::configuration(error.to_string(), json!({})))?;
    write_config(&config)?;
    Ok(success(
        "provider set",
        json!({
            "provider": name,
            "protocol": protocol,
            "base_url": config.provider_url(),
            "model": config.provider_model().unwrap_or("default"),
            "api_key_env": config.provider_api_key_env(),
            "fallback_provider": config
                .provider_profile(name)
                .and_then(ProviderProfile::fallback_provider),
            "pricing": config
                .provider_profile(name)
                .and_then(ProviderProfile::pricing)
                .map(|pricing| json!({
                    "input_micros_per_million_tokens": pricing
                        .input_micros_per_million_tokens(),
                    "output_micros_per_million_tokens": pricing
                        .output_micros_per_million_tokens(),
                })),
            "active": config.active_provider() == Some(name),
        }),
        format!("Provider {name} configured"),
    ))
}

fn remove_provider(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "name"])?;
    let name = parsed
        .value("name")
        .or_else(|| parsed.positionals.first().map(String::as_str))
        .ok_or_else(|| CliError::usage("provider remove requires '<name>'"))?;
    let mut config = load_config(&parsed)?;
    if !config.quarantine_provider_profile(name) {
        return Err(CliError::configuration(
            "provider is not configured",
            json!({"provider": name}),
        ));
    }
    write_config(&config)?;
    Ok(success(
        "provider remove",
        json!({
            "removed": name,
            "active_provider": config.active_provider(),
            "providers": config.provider_names(),
        }),
        format!("Provider {name} removed"),
    ))
}

fn login_provider(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "provider",
            "account",
            "no-browser",
            "print-auth-url",
            "callback-url",
            "auth-code",
            "token",
        ],
    )?;
    let mut config = load_config_storage(&parsed)?;
    let provider_arg = parsed
        .value("provider")
        .or_else(|| parsed.positionals.first().map(String::as_str));

    if let Some(callback_or_code) = parsed
        .value("callback-url")
        .or_else(|| parsed.value("auth-code"))
        .or_else(|| parsed.value("token"))
    {
        let entry = super::setup::complete_oauth_login(
            &mut config,
            provider_arg,
            parsed.value("account"),
            callback_or_code,
        )?;
        return Ok(success(
            "provider login",
            json!({
                "status": "authenticated",
                "provider": entry.provider,
                "account": entry.account,
                "auth_kind": entry.auth_kind,
                "api_key_env": entry.api_key_env,
                "credential_preview": entry.masked_preview,
                "active": entry.active,
            }),
            format!(
                "OAuth login completed for provider '{}' (account '{}', {})",
                entry.provider, entry.account, entry.masked_preview
            ),
        ));
    }

    let Some(provider_name) = provider_arg else {
        let flows = super::setup::OAUTH_PROVIDERS
            .iter()
            .map(|p| {
                json!({
                    "provider": p.name,
                    "label": p.label,
                    "preset": p.preset_name,
                    "local_no_auth": p.is_local_no_auth,
                })
            })
            .collect::<Vec<_>>();
        let lines = [
            "Available Pandora OAuth & Provider Login Flows:",
            "  pandora provider login --provider claude       # Anthropic Claude subscription OAuth",
            "  pandora provider login --provider openai       # OpenAI / ChatGPT / Codex OAuth",
            "  pandora provider login --provider copilot      # GitHub Copilot subscription OAuth",
            "  pandora provider login --provider openrouter   # OpenRouter PKCE key exchange",
            "  pandora provider login --provider nous         # Nous Research Portal OAuth",
            "  pandora provider login --provider huggingface  # Hugging Face Hub OAuth",
            "  pandora provider login --provider azure        # Azure OpenAI Entra OAuth",
            "  pandora provider login --provider ollama       # Local Ollama server (no-auth)",
            "  pandora provider login --provider lmstudio     # Local LM Studio server (no-auth)",
            "Headless / Remote flags: --no-browser, --print-auth-url, --callback-url <url>, --auth-code <code>",
        ];
        return Ok(success(
            "provider login",
            json!({"available_flows": flows}),
            lines.join("\n"),
        ));
    };

    let flow =
        super::setup::start_oauth_login(&mut config, provider_name, parsed.value("account"))?;
    let headless = parsed.has_flag("no-browser")
        || parsed.has_flag("print-auth-url")
        || std::env::var("PANDORA_HEADLESS").is_ok()
        || std::env::var("CI").is_ok();
    if flow.state == "local-ready" {
        return Ok(success(
            "provider login",
            json!({
                "provider": flow.provider,
                "preset": flow.preset_name,
                "account": flow.account,
                "auth_url": flow.auth_url,
                "redirect_uri": flow.redirect_uri,
                "state": flow.state,
                "headless": headless,
            }),
            format!(
                "Local provider '{}' configured at {} (account '{}', no API key required)",
                flow.provider, flow.auth_url, flow.account
            ),
        ));
    }

    if headless {
        let message = format!(
            "OAuth PKCE flow started for '{}' (account '{}'):\n  Authorize URL: {}\n  Complete with: pandora provider login --provider {} --callback-url \"<redirect-url>\" (or --auth-code <code>)",
            flow.provider, flow.account, flow.auth_url, flow.provider
        );
        return Ok(success(
            "provider login",
            json!({
                "provider": flow.provider,
                "preset": flow.preset_name,
                "account": flow.account,
                "auth_url": flow.auth_url,
                "redirect_uri": flow.redirect_uri,
                "code_challenge_method": if flow.code_challenge.is_empty() { "none" } else { "S256" },
                "state": flow.state,
                "headless": true,
            }),
            message,
        ));
    }

    eprintln!("Opening authorization URL in default browser...");
    eprintln!("  {}", flow.auth_url);
    eprintln!("Waiting for OAuth loopback callback on http://127.0.0.1:1455/auth/callback...");

    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/c", "start", "", &flow.auth_url])
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg(&flow.auth_url)
            .spawn();
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open")
            .arg(&flow.auth_url)
            .spawn();
    }

    let callback_url = super::setup::run_local_oauth_loopback(1455, &flow.state, 90)?;
    let entry = super::setup::complete_oauth_login(
        &mut config,
        Some(&flow.provider),
        Some(&flow.account),
        &callback_url,
    )?;

    Ok(success(
        "provider login",
        json!({
            "status": "authenticated",
            "provider": entry.provider,
            "account": entry.account,
            "auth_kind": entry.auth_kind,
            "api_key_env": entry.api_key_env,
            "credential_preview": entry.masked_preview,
            "active": entry.active,
        }),
        format!(
            "OAuth login completed for provider '{}' (account '{}', {})",
            entry.provider, entry.account, entry.masked_preview
        ),
    ))
}

fn accounts_command(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "provider", "account"],
    )?;
    let mut config = load_config_storage(&parsed)?;
    if parsed.positionals.first().map(String::as_str) == Some("use")
        || parsed.value("account").is_some()
    {
        let account_name = parsed
            .value("account")
            .or_else(|| parsed.positionals.get(1).map(String::as_str))
            .ok_or_else(|| {
                CliError::usage("provider account use requires '<account>' or '--account <name>'")
            })?;
        let chosen = super::setup::switch_oauth_account(
            &mut config,
            parsed.value("provider"),
            account_name,
        )?;
        return Ok(success(
            "provider accounts",
            json!({
                "switched": true,
                "provider": chosen.provider,
                "account": chosen.account,
                "auth_kind": chosen.auth_kind,
                "api_key_env": chosen.api_key_env,
                "credential_preview": chosen.masked_preview,
            }),
            format!(
                "Active account for '{}' switched to '{}' ({})",
                chosen.provider, chosen.account, chosen.masked_preview
            ),
        ));
    }

    let filter = parsed.value("provider");
    let accounts = super::setup::load_oauth_accounts(&config)
        .into_iter()
        .filter(|a| filter.is_none_or(|f| a.provider.eq_ignore_ascii_case(f)))
        .map(|a| {
            json!({
                "provider": a.provider,
                "account": a.account,
                "auth_kind": a.auth_kind,
                "api_key_env": a.api_key_env,
                "credential_preview": a.masked_preview,
                "active": a.active,
                "updated_at": a.updated_at,
            })
        })
        .collect::<Vec<_>>();
    Ok(success(
        "provider accounts",
        json!({"accounts": accounts, "count": accounts.len()}),
        format!("{} configured provider account(s)", accounts.len()),
    ))
}

fn parse_pricing(value: &str, name: &str) -> Result<u64, CliError> {
    value.parse::<u64>().map_err(|_| {
        CliError::usage(format!(
            "--{name} must be a non-negative integer in micro-units per million tokens"
        ))
    })
}

fn use_provider(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "provider use requires exactly one provider name",
        ));
    }
    let mut config = load_config(&parsed)?;
    let name = parsed.positionals[0].as_str();
    config
        .set_active_provider(name)
        .map_err(|error| CliError::configuration(error.to_string(), json!({})))?;
    write_config(&config)?;
    Ok(success(
        "provider use",
        json!({
            "provider": name,
            "model": config.provider_model().unwrap_or("default"),
            "api_key_env": config.provider_api_key_env(),
        }),
        format!("Provider {name} is active"),
    ))
}

fn test(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "provider", "model"],
    )?;
    let config = load_config(&parsed)?;
    super::require_config_file(&config)?;
    let model = parsed
        .value("model")
        .or(config.provider_model())
        .unwrap_or("default");
    let provider_name = parsed.value("provider");
    let provider = configured_provider_for(&config, model, "provider test", provider_name)?;
    let manifest = provider.manifest().clone();
    let request = ModelRequest::new(
        manifest.id().clone(),
        manifest.default_model().clone(),
        vec![
            ChatMessage::user("Reply with exactly: ready")
                .map_err(|error| CliError::provider(error.to_string(), json!({})))?,
        ],
    )
    .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    let workspace = WorkspaceRoot::new(config.workspace_dir())
        .map_err(|_| CliError::configuration("workspace path is invalid", json!({})))?;
    let controller = ExecutionController::with_policy(
        workspace,
        PolicyContext::new(1, [Capability::ProviderInvoke], []),
    );
    let (principal, tenant, workspace_id) = super::session_scope();
    let session = Session::new(
        SessionId::new("provider-test").expect("provider test session ID is valid"),
        principal,
        tenant,
        workspace_id,
        timestamp(),
    );
    let invocation = controller
        .invoke_provider(provider.as_ref(), request, &session, timestamp())
        .map_err(|_| CliError::provider("provider effect was not authorized", json!({})))?;
    let response = invocation
        .result()
        .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    let metrics = invocation.metrics();
    Ok(success(
        "provider test",
        json!({
            "provider": manifest.id(),
            "model": manifest.default_model(),
            "status": "ready",
            "output": response.text(),
            "usage": {
                "prompt_tokens": response.usage().prompt_tokens(),
                "completion_tokens": response.usage().completion_tokens(),
                "cached_prompt_tokens": response.usage().cached_prompt_tokens(),
                "cache_write_prompt_tokens": response.usage().cache_write_prompt_tokens(),
                "total_tokens": response.usage().total_tokens(),
            },
            "metrics": {
                "elapsed_ms": metrics.elapsed_ms(),
                "input_tokens": metrics.input_tokens(),
                "output_tokens": metrics.output_tokens(),
                "cached_input_tokens": metrics.cached_input_tokens(),
                "cache_write_input_tokens": metrics.cache_write_input_tokens(),
                "succeeded": metrics.succeeded(),
            },
        }),
        format!("Provider {} is ready", manifest.id()),
    ))
}
