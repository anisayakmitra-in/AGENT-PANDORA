//! `pandora prompt` subcommand implementation.

use super::{load_config, parse_options};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::prompt_engine::{
    PresetKind, PromptEngine, PromptError, PromptPreset, StorageTier,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

fn current_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage("prompt requires 'preset', 'save', 'list', 'inspect', 'apply', or 'delete'")
    })?;

    match subcommand.as_str() {
        "list" | "ls" => list(&args[1..]),
        "save" => save(&args[1..]),
        "inspect" | "show" | "preset" => inspect(&args[1..]),
        "apply" => apply(&args[1..]),
        "delete" | "rm" => delete(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown prompt subcommand '{unknown}'"
        ))),
    }
}

fn engine_for(parsed: &super::ParsedArgs) -> Result<PromptEngine, CliError> {
    let config = load_config(parsed)?;
    let user_config_dir = config
        .config_path()
        .parent()
        .unwrap_or_else(|| Path::new("."));
    Ok(PromptEngine::open(config.workspace_dir(), user_config_dir))
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "global"])?;
    let engine = engine_for(&parsed)?;
    let presets = engine.list().map_err(map_prompt_error)?;
    let global_only = parsed.has_flag("global");
    let items = presets
        .iter()
        .filter(|(_, tier)| !global_only || *tier == StorageTier::User)
        .map(|(p, tier)| {
            json!({
                "name": p.name,
                "kind": p.kind.as_str(),
                "tier": tier.as_str(),
                "description": p.description,
                "parameters": p.parameters,
                "model": p.model,
                "mode": p.mode,
            })
        })
        .collect::<Vec<_>>();

    let count = items.len();
    Ok(success(
        "prompt list",
        json!({ "presets": items, "count": count }),
        format!("{count} prompt preset(s) available"),
    ))
}

fn save(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "kind",
            "description",
            "content",
            "model",
            "mode",
            "global",
            "name",
        ],
    )?;
    let name = parsed
        .value("name")
        .or_else(|| parsed.positionals.first().map(String::as_str))
        .ok_or_else(|| CliError::usage("prompt save requires a preset name"))?;

    let content = parsed
        .value("content")
        .or_else(|| {
            if parsed.value("name").is_some() {
                parsed.positionals.first().map(String::as_str)
            } else {
                parsed.positionals.get(1).map(String::as_str)
            }
        })
        .ok_or_else(|| CliError::usage("prompt save requires --content <text>"))?;

    let kind = match parsed.value("kind").unwrap_or("template") {
        "persona" => PresetKind::Persona,
        "template" => PresetKind::Template,
        other => {
            return Err(CliError::usage(format!(
                "unknown kind '{other}'; expected 'persona' or 'template'"
            )));
        }
    };

    let global = parsed.has_flag("global");
    let now = current_timestamp();
    let preset = PromptPreset {
        name: name.to_owned(),
        description: parsed.value("description").map(str::to_owned),
        kind,
        content: content.to_owned(),
        parameters: PromptEngine::extract_parameters(content),
        model: parsed.value("model").map(str::to_owned),
        mode: parsed.value("mode").map(str::to_owned),
        created_at: now,
        updated_at: now,
    };

    let engine = engine_for(&parsed)?;
    let saved_path = engine.save(preset, !global).map_err(map_prompt_error)?;

    Ok(success(
        "prompt save",
        json!({
            "name": name,
            "tier": if global { "user" } else { "workspace" },
            "path": saved_path.display().to_string(),
        }),
        format!("prompt preset '{name}' saved successfully"),
    ))
}

fn inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "name"])?;
    let name = parsed
        .value("name")
        .or_else(|| parsed.positionals.first().map(String::as_str))
        .ok_or_else(|| CliError::usage("prompt inspect requires a preset name"))?;

    let engine = engine_for(&parsed)?;
    let (preset, tier) = engine.get(name).map_err(map_prompt_error)?;

    Ok(success(
        "prompt inspect",
        json!({
            "name": preset.name,
            "kind": preset.kind.as_str(),
            "tier": tier.as_str(),
            "description": preset.description,
            "content": preset.content,
            "parameters": preset.parameters,
            "model": preset.model,
            "mode": preset.mode,
        }),
        format!(
            "Preset '{}' ({}): {}",
            preset.name,
            tier.as_str(),
            preset.content
        ),
    ))
}

fn apply(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "lang",
            "param",
            "dry-run",
            "model",
            "mode",
            "session",
            "agent",
            "name",
        ],
    )?;
    let name = parsed
        .value("name")
        .or_else(|| parsed.positionals.first().map(String::as_str))
        .ok_or_else(|| CliError::usage("prompt apply requires a preset name"))?;

    let task = if parsed.positionals.len() > 1 {
        parsed.positionals[1..].join(" ")
    } else {
        String::new()
    };

    let engine = engine_for(&parsed)?;
    let (preset, _) = engine.get(name).map_err(map_prompt_error)?;

    let mut params = BTreeMap::new();
    let config = load_config(&parsed)?;
    params.insert(
        "workspace".to_owned(),
        config.workspace_dir().display().to_string(),
    );
    params.insert("task".to_owned(), task);

    let detected_lang = parsed
        .value("lang")
        .map(str::to_owned)
        .unwrap_or_else(|| PromptEngine::detect_workspace_lang(config.workspace_dir()));
    params.insert("lang".to_owned(), detected_lang);

    // Extract any custom params from repeated --param k=v or comma-separated pairs
    for param_raw in parsed.values_for("param") {
        let parts: Vec<&str> = param_raw.split(',').collect();
        if parts.len() > 1 && parts.iter().all(|part| part.contains('=')) {
            for part in parts {
                if let Some((k, v)) = part.split_once('=') {
                    let k = k.trim();
                    if k.is_empty() {
                        return Err(CliError::usage(format!(
                            "empty parameter key in '{param_raw}'"
                        )));
                    }
                    params.insert(k.to_owned(), v.trim().to_owned());
                }
            }
        } else if let Some((k, v)) = param_raw.split_once('=') {
            let k = k.trim();
            if k.is_empty() {
                return Err(CliError::usage(format!(
                    "empty parameter key in '{param_raw}'"
                )));
            }
            params.insert(k.to_owned(), v.trim().to_owned());
        } else {
            return Err(CliError::usage(format!(
                "invalid parameter '{param_raw}'; expected 'key=value'"
            )));
        }
    }

    let interpolated =
        PromptEngine::interpolate(&preset.content, &params).map_err(map_prompt_error)?;

    if parsed.has_flag("dry-run") {
        return Ok(success(
            "prompt apply",
            json!({
                "name": name,
                "interpolated": interpolated,
                "kind": preset.kind.as_str(),
            }),
            interpolated,
        ));
    }

    // Execute via run command
    let mut run_args = vec!["--agent".to_owned()];
    if let Some(model) = parsed.value("model").or(preset.model.as_deref()) {
        run_args.push("--model".to_owned());
        run_args.push(model.to_owned());
    }
    if let Some(mode) = parsed.value("mode").or(preset.mode.as_deref()) {
        run_args.push("--mode".to_owned());
        run_args.push(mode.to_owned());
    }
    if let Some(session) = parsed.value("session") {
        run_args.push("--session".to_owned());
        run_args.push(session.to_owned());
    }
    run_args.push(interpolated);

    super::run::execute(&run_args)
}

fn delete(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "global", "yes", "name"],
    )?;
    let name = parsed
        .value("name")
        .or_else(|| parsed.positionals.first().map(String::as_str))
        .ok_or_else(|| CliError::usage("prompt delete requires a preset name"))?;

    let global = parsed.has_flag("global");
    let engine = engine_for(&parsed)?;
    let removed = engine.delete(name, !global).map_err(map_prompt_error)?;

    if removed {
        Ok(success(
            "prompt delete",
            json!({ "name": name, "deleted": true }),
            format!("prompt preset '{name}' removed"),
        ))
    } else {
        Err(CliError::configuration(
            format!("preset '{name}' not found on the specified tier"),
            json!({ "name": name }),
        ))
    }
}

fn map_prompt_error(err: PromptError) -> CliError {
    match err {
        PromptError::NotFound(name) => CliError::configuration(
            format!("prompt preset '{name}' not found"),
            json!({ "name": name }),
        ),
        PromptError::AlreadyExists(name) => CliError::configuration(
            format!("prompt preset '{name}' already exists; delete first to overwrite"),
            json!({ "name": name }),
        ),
        PromptError::InvalidName(name) => CliError::usage(format!("invalid preset name '{name}'")),
        PromptError::MissingParameter { param, template } => CliError::usage(format!(
            "missing required template parameter '{{{{{param}}}}}' in template: {template}"
        )),
        PromptError::Io(err) => CliError::internal(format!("prompt I/O error: {err}"), json!({})),
        PromptError::Serialization(err) => {
            CliError::internal(format!("prompt serialization error: {err}"), json!({}))
        }
    }
}
