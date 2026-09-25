use super::parse_options;
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::RuntimeService;
use pandora_types::ServiceEngineSummary;
use serde_json::json;

/// Read-only view of the runtime engine inventory.
///
/// The inventory is a static description of compiled components, so this
/// command opens no store, needs no configuration, and can never grant
/// authority. It is the CLI counterpart of the service's `runtime.engines`
/// read model, and both read the same core list.
pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let selected = args.iter().position(|arg| !arg.starts_with('-'));
    let (group, rest) = match selected {
        Some(index) => (args[index].as_str(), &args[index + 1..]),
        None => ("engines", args),
    };
    match group {
        "engines" => engines(rest),
        unknown => Err(CliError::usage(format!(
            "unknown runtime command '{unknown}'"
        ))),
    }
}

fn engines(args: &[String]) -> Result<CommandResult, CliError> {
    // The first non-flag argument selects the subcommand, so `runtime engines
    // --json` lists rather than complaining about a missing verb. Reading the
    // whole inventory is the only sensible default for a read-only listing.
    let selected = args.iter().position(|arg| !arg.starts_with('-'));
    let (subcommand, rest) = match selected {
        Some(index) => (args[index].as_str(), &args[index + 1..]),
        None => ("list", args),
    };
    match subcommand {
        "list" => list(rest),
        "inspect" => inspect(rest),
        unknown => Err(CliError::usage(format!(
            "unknown runtime engines command '{unknown}'"
        ))),
    }
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "runtime engines list does not accept positional arguments",
        ));
    }
    let inventory = RuntimeService::engine_inventory();
    let engines = inventory.iter().map(engine_value).collect::<Vec<_>>();
    let mut categories = std::collections::BTreeMap::<&str, usize>::new();
    for engine in &inventory {
        *categories.entry(engine.category()).or_default() += 1;
    }
    let summary = categories
        .iter()
        .map(|(name, count)| format!("{name} {count}"))
        .collect::<Vec<_>>()
        .join(", ");
    Ok(success(
        "runtime engines list",
        json!({
            "engines": engines,
            "count": inventory.len(),
            "categories": categories,
        }),
        format!("{} engines ({summary})", inventory.len()),
    ))
}

fn inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "runtime engines inspect requires exactly one engine ID",
        ));
    }
    let engine_id = &parsed.positionals[0];
    let engine = RuntimeService::engine_inventory()
        .into_iter()
        .find(|engine| engine.id() == engine_id)
        .ok_or_else(|| CliError::usage(format!("unknown engine '{engine_id}'")))?;
    Ok(success(
        "runtime engines inspect",
        json!({"engine": engine_value(&engine)}),
        format!(
            "{} — {} [{}]",
            engine.name(),
            engine.authority(),
            engine.category()
        ),
    ))
}

fn engine_value(engine: &ServiceEngineSummary) -> serde_json::Value {
    json!({
        "id": engine.id(),
        "name": engine.name(),
        "role": engine.role(),
        "authority": engine.authority(),
        "category": engine.category(),
        "component_kind": engine.component_kind(),
        "inputs": engine.inputs(),
        "outputs": engine.outputs(),
        "invariants": engine.invariants(),
        "evidence": engine.evidence(),
        "source_modules": engine.source_modules(),
        "related_components": engine.related_components(),
        "documentation": engine.documentation(),
    })
}
