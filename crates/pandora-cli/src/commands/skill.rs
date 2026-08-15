use super::{load_config, parse_options};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::skill_engine::{SkillEngine, SkillError};
use serde_json::json;
use std::fs;
use std::path::PathBuf;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage(
            "skill requires 'list', 'inspect', 'enable', 'disable', 'suspend', 'remove', or 'restore'",
        )
    })?;
    match subcommand.as_str() {
        "list" => list(&args[1..]),
        "inspect" => inspect(&args[1..]),
        "enable" => transition(&args[1..], "enable", SkillEngine::enable),
        "disable" => transition(&args[1..], "disable", SkillEngine::disable),
        "suspend" => transition(&args[1..], "suspend", SkillEngine::suspend),
        "remove" => remove(&args[1..]),
        "restore" => restore(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown skill command '{unknown}'"
        ))),
    }
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "root"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "skill list does not accept positional arguments",
        ));
    }
    let engine = engine(&parsed)?;
    let skills = engine
        .list()
        .map_err(skill_error)?
        .into_iter()
        .map(skill_value)
        .collect::<Vec<_>>();
    let count = skills.len();
    Ok(success(
        "skill list",
        json!({"skills": skills}),
        format!("{count} skill(s) discovered"),
    ))
}

fn transition(
    args: &[String],
    action: &str,
    apply: fn(&SkillEngine, &str) -> Result<pandora_runtime::skill_engine::SkillRecord, SkillError>,
) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "root"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(format!(
            "skill {action} requires exactly one skill ID"
        )));
    }
    let engine = engine(&parsed)?;
    let skill = apply(&engine, &parsed.positionals[0]).map_err(skill_error)?;
    let state = skill.state().as_str();
    let command = match action {
        "enable" => "skill enable",
        "disable" => "skill disable",
        "suspend" => "skill suspend",
        _ => unreachable!("skill transition action is fixed by the command router"),
    };
    Ok(success(
        command,
        json!({"skill": skill_value(skill)}),
        format!("Skill {} is now {state}", parsed.positionals[0]),
    ))
}

fn inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "root"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "skill inspect requires exactly one skill ID",
        ));
    }
    let engine = engine(&parsed)?;
    let inspection = engine
        .inspect(&parsed.positionals[0])
        .map_err(skill_error)?;
    let manifest = inspection.manifest();
    let scripts = inspection
        .scripts()
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    Ok(success(
        "skill inspect",
        json!({
            "skill": {
                "id": manifest.id().as_str(),
                "version": manifest.version(),
                "name": manifest.name(),
                "description": manifest.description(),
                "publisher": manifest.publisher(),
                "resources": inspection.resources(),
                "state": inspection.state().as_str(),
                "root": inspection.root(),
                "provenance": inspection.provenance().source(),
                "body": inspection.body(),
                "scripts": scripts,
            }
        }),
        format!("Inspected skill {}", manifest.id()),
    ))
}

fn remove(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "root", "dry-run", "yes"],
    )?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "skill remove requires exactly one skill ID",
        ));
    }
    let dry_run = parsed.value("dry-run").is_some();
    let confirmed = parsed.value("yes").is_some();
    if dry_run && confirmed {
        return Err(CliError::usage(
            "skill remove accepts only one of '--dry-run' or '--yes'",
        ));
    }
    if !dry_run && !confirmed {
        return Err(CliError::usage(
            "skill remove requires '--yes' or '--dry-run'",
        ));
    }
    let engine = engine(&parsed)?;
    let id = &parsed.positionals[0];
    let inspection = engine.inspect(id).map_err(skill_error)?;
    if dry_run {
        return Ok(success(
            "skill remove",
            json!({
                "dry_run": true,
                "skill": {
                    "id": id,
                    "state": inspection.state().as_str(),
                    "root": inspection.root(),
                },
            }),
            format!("Skill {id} is ready to be removed; no files changed"),
        ));
    }
    engine.remove(id).map_err(skill_error)?;
    Ok(success(
        "skill remove",
        json!({
            "dry_run": false,
            "skill": {"id": id, "state": "removed"},
        }),
        format!("Skill {id} removed and retained for restore"),
    ))
}

fn restore(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "root"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "skill restore requires exactly one skill ID",
        ));
    }
    let engine = engine(&parsed)?;
    let skill = engine
        .restore(&parsed.positionals[0])
        .map_err(skill_error)?;
    Ok(success(
        "skill restore",
        json!({"skill": skill_value(skill)}),
        format!("Skill {} restored disabled", parsed.positionals[0]),
    ))
}

fn engine(parsed: &super::ParsedArgs) -> Result<SkillEngine, CliError> {
    let config = load_config(parsed)?;
    let root = parsed
        .value("root")
        .map(PathBuf::from)
        .unwrap_or_else(|| config.data_dir().join("skills"));
    fs::create_dir_all(&root).map_err(|_| {
        CliError::configuration(
            "could not create the skill directory",
            json!({"root": root}),
        )
    })?;
    SkillEngine::discover(root).map_err(skill_error)
}

fn skill_value(skill: pandora_runtime::skill_engine::SkillRecord) -> serde_json::Value {
    let manifest = skill.manifest();
    json!({
        "id": manifest.id().as_str(),
        "version": manifest.version(),
        "name": manifest.name(),
        "description": manifest.description(),
        "publisher": manifest.publisher(),
        "resources": manifest.resources(),
        "state": skill.state().as_str(),
        "root": skill.root(),
        "provenance": skill.provenance().source(),
    })
}

fn skill_error(error: SkillError) -> CliError {
    CliError::execution(error.to_string(), json!({}))
}
