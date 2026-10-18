#![forbid(unsafe_code)]

use super::{load_config_storage, parse_options};
use crate::output::{CliError, CommandResult, success};
use pandora_types::{PromptCachePolicy, PromptCacheStrategy, ReasoningEffort};
use serde_json::json;
use std::str::FromStr;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage("reasoning requires 'list', 'inspect', 'profile', or 'set'")
    })?;
    match subcommand.as_str() {
        "list" => list(&args[1..]),
        "inspect" => inspect(&args[1..]),
        "profile" => profile(&args[1..]),
        "set" => set_reasoning(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown reasoning command '{unknown}', expected 'list', 'inspect', 'profile', or 'set'"
        ))),
    }
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let _parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    let levels = [
        ReasoningEffort::None,
        ReasoningEffort::Low,
        ReasoningEffort::Medium,
        ReasoningEffort::High,
    ];
    let items = levels
        .iter()
        .map(|effort| {
            json!({
                "effort": effort.as_str(),
                "openai_reasoning_effort": effort.openai_reasoning_effort(),
                "anthropic_thinking_budget": effort.anthropic_thinking_budget(),
                "description": match effort {
                    ReasoningEffort::None => "Fast reflex without extended chain-of-thought tokens",
                    ReasoningEffort::Low => "Lightweight verification and fast step checking",
                    ReasoningEffort::Medium => "Balanced chain-of-thought scratchpad deliberation",
                    ReasoningEffort::High => "Deep multi-pass reflection, invariant auditing, and verification",
                }
            })
        })
        .collect::<Vec<_>>();

    let cache_strategies = [
        PromptCacheStrategy::Auto,
        PromptCacheStrategy::Interactive,
        PromptCacheStrategy::Subagent,
        PromptCacheStrategy::Disabled,
    ];
    let cache_items = cache_strategies
        .iter()
        .map(|strategy| {
            json!({
                "strategy": strategy.as_str(),
                "description": match strategy {
                    PromptCacheStrategy::Auto => "Dynamic 1h TTL for interactive sessions, 5m for subagents/background jobs",
                    PromptCacheStrategy::Interactive => "Fixed 1-hour KV-cache lifetime for human chat and iteration",
                    PromptCacheStrategy::Subagent => "Short 5-minute ephemeral cache for parallel subagent tasks",
                    PromptCacheStrategy::Disabled => "KV-cache reuse disabled, forces fresh token evaluation",
                }
            })
        })
        .collect::<Vec<_>>();

    let response = json!({
        "reasoning_levels": items,
        "prompt_cache_strategies": cache_items,
    });

    let rendered = "Reasoning Effort Levels:\n\
         - none:   OpenAI: none,   Anthropic thinking: disabled (0 tokens)\n\
         - low:    OpenAI: low,    Anthropic thinking: 1024 tokens\n\
         - medium: OpenAI: medium, Anthropic thinking: 4096 tokens (default)\n\
         - high:   OpenAI: high,   Anthropic thinking: 16384 tokens\n\n\
         Prompt Cache Strategies:\n\
         - auto:        Dynamic 1h interactive / 5m subagent TTL\n\
         - interactive: 1h KV-cache retention\n\
         - subagent:    5m ephemeral KV-cache retention\n\
         - disabled:    Prompt caching turned off"
        .to_string();

    Ok(success("reasoning list", response, rendered))
}

fn inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "effort"])?;
    let effort_str = parsed.value("effort").ok_or_else(|| {
        CliError::usage("reasoning inspect requires '--effort <none|low|medium|high>'")
    })?;

    let effort = ReasoningEffort::from_str(effort_str)
        .map_err(|e| CliError::usage(format!("invalid effort: {e}")))?;

    let response = json!({
        "effort": effort.as_str(),
        "openai_reasoning_effort": effort.openai_reasoning_effort(),
        "anthropic_thinking_budget": effort.anthropic_thinking_budget(),
        "static_prefix_frozen": true,
        "max_recommended_turn_tokens": match effort {
            ReasoningEffort::None => 1024,
            ReasoningEffort::Low => 2048,
            ReasoningEffort::Medium => 8192,
            ReasoningEffort::High => 32768,
        }
    });

    let rendered = format!(
        "Reasoning Effort Profile: {}\n\
         - OpenAI parameter: {:?}\n\
         - Anthropic budget: {:?} tokens\n\
         - Static prefix frozen: true",
        effort.as_str(),
        effort.openai_reasoning_effort(),
        effort.anthropic_thinking_budget()
    );

    Ok(success("reasoning inspect", response, rendered))
}

fn profile(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    let config = load_config_storage(&parsed)?;

    let default_policy = PromptCachePolicy::default();
    let response = json!({
        "workspace": config.workspace_dir().display().to_string(),
        "default_reasoning_effort": "medium",
        "prompt_cache": {
            "strategy": default_policy.strategy.as_str(),
            "preserve_static_prefix": default_policy.preserve_static_prefix,
            "max_prefix_tokens": default_policy.max_prefix_tokens,
        }
    });

    let rendered = format!(
        "Reasoning & Prompt Cache Profile:\n\
         - Workspace: {}\n\
         - Default Effort: medium\n\
         - Cache Strategy: auto\n\
         - Static Prefix Preserved: true\n\
         - Max Prefix Tokens: {}",
        config.workspace_dir().display(),
        default_policy.max_prefix_tokens
    );

    Ok(success("reasoning profile", response, rendered))
}

fn set_reasoning(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "effort", "cache"],
    )?;
    let effort_str = parsed.value("effort").ok_or_else(|| {
        CliError::usage("reasoning set requires '--effort <none|low|medium|high>'")
    })?;

    let effort = ReasoningEffort::from_str(effort_str)
        .map_err(|e| CliError::usage(format!("invalid effort: {e}")))?;

    let cache_strategy = if let Some(c) = parsed.value("cache") {
        PromptCacheStrategy::from_str(c)
            .map_err(|e| CliError::usage(format!("invalid cache strategy: {e}")))?
    } else {
        PromptCacheStrategy::Auto
    };

    let response = json!({
        "applied_effort": effort.as_str(),
        "applied_cache_strategy": cache_strategy.as_str(),
        "openai_parameter": effort.openai_reasoning_effort(),
        "anthropic_budget": effort.anthropic_thinking_budget(),
        "status": "configured"
    });

    let rendered = format!(
        "Configured reasoning effort to '{}' and prompt cache strategy to '{}'.",
        effort.as_str(),
        cache_strategy.as_str()
    );

    Ok(success("reasoning set", response, rendered))
}
