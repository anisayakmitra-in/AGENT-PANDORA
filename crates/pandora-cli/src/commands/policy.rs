//! Policy command implementation (`pandora policy`).
//!
//! Provides inspection and validation of declarative lifecycle veto rules
//! evaluated inside the governed execution path. Hooks remain deny-only and
//! bounded; they cannot mutate requests, execute arbitrary code, resolve
//! approvals, or issue effect permits.

use super::parse_options;
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::hooks::LifecycleHooks;
use serde_json::json;
use std::fs;
use std::path::Path;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("policy requires a subcommand: 'hooks'"))?;
    match subcommand.as_str() {
        "hooks" => hooks(&args[1..]),
        "--help" | "-h" => Ok(success("policy help", json!({ "usage": usage() }), usage())),
        unknown => Err(CliError::usage(format!(
            "unknown policy subcommand '{unknown}', expected 'hooks'"
        ))),
    }
}

fn hooks(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("policy hooks requires 'list' or 'validate'"))?;
    match subcommand.as_str() {
        "list" => list(&args[1..]),
        "validate" => validate(&args[1..]),
        "--help" | "-h" => Ok(success(
            "policy hooks help",
            json!({ "usage": hooks_usage() }),
            hooks_usage(),
        )),
        unknown => Err(CliError::usage(format!(
            "unknown policy hooks subcommand '{unknown}', expected 'list' or 'validate'"
        ))),
    }
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let _parsed = parse_options(args, &[])?;
    let human = "Lifecycle Hook Policy Points and Selectors:\n\
  Hook Points:\n\
    - before_authorization: Evaluated before reference monitor authority check\n\
  Hook Selectors:\n\
    - any: Matches every operation request\n\
    - capability: Matches operation requests requiring a specific capability\n\
    - gene: Matches operation requests dispatched under a specific gene ID\n\
  Execution Posture: deny-only, bounded, declarative; arbitrary code execution disabled";

    Ok(success(
        "policy hooks list",
        json!({
            "hook_points": ["before_authorization"],
            "hook_selectors": ["any", "capability", "gene"],
            "posture": "deny_only",
            "arbitrary_code_execution": false,
            "can_issue_permits": false
        }),
        human,
    ))
}

fn validate(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["file"])?;
    let file_path = parsed
        .value("file")
        .ok_or_else(|| CliError::usage("policy hooks validate requires '--file <path>'"))?;

    let path = Path::new(file_path);
    if !path.exists() {
        return Err(CliError::usage(format!(
            "policy hooks file not found: '{file_path}'"
        )));
    }

    let content = fs::read_to_string(path).map_err(|err| {
        CliError::usage(format!(
            "failed to read policy hooks file '{file_path}': {err}"
        ))
    })?;

    let lifecycle_hooks = LifecycleHooks::from_json_str(&content).map_err(|err| {
        CliError::usage(format!("invalid policy hooks JSON in '{file_path}': {err}"))
    })?;

    let count = lifecycle_hooks.len();
    let hooks_summary: Vec<_> = lifecycle_hooks
        .hooks()
        .iter()
        .map(|h| {
            json!({
                "id": h.id(),
                "point": format!("{:?}", h.point()),
                "selector": format!("{:?}", h.selector()),
                "reason": h.reason(),
            })
        })
        .collect();

    let mut human = format!("Validated policy hooks file '{file_path}': {count} rule(s) valid.\n");
    for hook in lifecycle_hooks.hooks() {
        human.push_str(&format!(
            "  - [{}] point={:?}, selector={:?}, reason=\"{}\"\n",
            hook.id(),
            hook.point(),
            hook.selector(),
            hook.reason()
        ));
    }

    Ok(success(
        "policy hooks validate",
        json!({
            "file": file_path,
            "valid": true,
            "rule_count": count,
            "rules": hooks_summary,
        }),
        human.trim_end(),
    ))
}

fn usage() -> &'static str {
    "usage: pandora policy <hooks> [options]\n\n\
commands:\n\
  policy hooks list\n\
  policy hooks validate --file <path>"
}

fn hooks_usage() -> &'static str {
    "usage: pandora policy hooks <list|validate> [options]\n\n\
commands:\n\
  list\n\
  validate --file <path>"
}
