use super::{parse_options, run};
use crate::output::{CliError, CommandResult, success};
use pandora_harnesses::builtin_harnesses;
use serde_json::json;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("harness requires 'list', 'inspect', or 'run'"))?;
    match subcommand.as_str() {
        "list" => list(&args[1..]),
        "inspect" => inspect(&args[1..]),
        "run" => run_harness(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown harness command '{unknown}'"
        ))),
    }
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "harness list does not accept positional arguments",
        ));
    }
    let harnesses = builtin_harnesses();
    let values = harnesses
        .iter()
        .map(|harness| harness_value(harness.as_ref()))
        .collect::<Vec<_>>();
    Ok(success(
        "harness list",
        json!({"harnesses": values}),
        format!("{} harnesses available", harnesses.len()),
    ))
}

fn inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "harness inspect requires the harness name 'coding'",
        ));
    }
    let requested_id = match parsed.positionals[0].as_str() {
        "coding" | "coding-domain" => "coding-domain",
        other => return Err(CliError::usage(format!("unknown harness '{other}'"))),
    };
    let harnesses = builtin_harnesses();
    let harness = harnesses
        .iter()
        .find(|harness| harness.manifest().id().as_str() == requested_id)
        .expect("catalogued harness should be available");
    Ok(success(
        "harness inspect",
        json!({"harness": harness_value(harness.as_ref())}),
        format!(
            "{} {}",
            harness.manifest().name(),
            harness.manifest().version()
        ),
    ))
}

fn run_harness(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "session", "gene", "task"],
    )?;
    if parsed.positionals.len() != 1
        || !matches!(parsed.positionals[0].as_str(), "coding" | "coding-domain")
    {
        return Err(CliError::usage(
            "harness run requires the harness name 'coding' or 'coding-domain'",
        ));
    }
    let gene = parsed
        .value("gene")
        .ok_or_else(|| CliError::usage("harness run requires '--gene <id>'"))?;
    let task = parsed
        .value("task")
        .ok_or_else(|| CliError::usage("harness run requires '--task <task>'"))?;
    let mut run_args = vec![
        task.to_owned(),
        "--harness".to_owned(),
        "coding-domain".to_owned(),
        "--gene".to_owned(),
        gene.to_owned(),
    ];
    for option in ["config", "data-dir", "workspace", "session"] {
        if let Some(value) = parsed.value(option) {
            run_args.push(format!("--{option}"));
            run_args.push(value.to_owned());
        }
    }
    let result = run::execute(&run_args)?;
    Ok(success("harness run", result.data, result.human))
}

fn harness_value(harness: &dyn pandora_types::Harness) -> serde_json::Value {
    let manifest = harness.manifest();
    let genes = harness
        .genes()
        .iter()
        .map(|gene| {
            json!({
                "id": gene.manifest().id(),
                "version": gene.manifest().version(),
                "kind": gene.manifest().kind().as_str(),
                "capabilities": gene
                    .manifest()
                    .capabilities()
                    .iter()
                    .map(|capability| capability.as_str())
                    .collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "id": manifest.id(),
        "version": manifest.version(),
        "name": manifest.name(),
        "kind": manifest.kind().as_str(),
        "genes": genes,
    })
}
