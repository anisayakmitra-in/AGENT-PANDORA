use super::{parse_options, run};
use crate::output::{CliError, CommandResult, success};
use pandora_harnesses::CodingHarness;
use pandora_types::Harness;
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
    let coding = CodingHarness::new();
    Ok(success(
        "harness list",
        json!({"harnesses": [harness_value(&coding)]}),
        "1 harness available".to_owned(),
    ))
}

fn inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "harness inspect requires the harness name 'coding'",
        ));
    }
    if parsed.positionals[0] != "coding" && parsed.positionals[0] != "coding-domain" {
        return Err(CliError::usage("unknown harness 'coding'"));
    }
    let coding = CodingHarness::new();
    Ok(success(
        "harness inspect",
        json!({"harness": harness_value(&coding)}),
        format!(
            "{} {}",
            coding.manifest().name(),
            coding.manifest().version()
        ),
    ))
}

fn run_harness(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "session", "gene", "task"],
    )?;
    if parsed.positionals.len() != 1 || parsed.positionals[0] != "coding" {
        return Err(CliError::usage(
            "harness run requires the harness name 'coding'",
        ));
    }
    let gene = parsed
        .value("gene")
        .ok_or_else(|| CliError::usage("harness run requires '--gene <id>'"))?;
    let task = parsed
        .value("task")
        .ok_or_else(|| CliError::usage("harness run requires '--task <task>'"))?;
    let mut run_args = vec![task.to_owned(), "--gene".to_owned(), gene.to_owned()];
    for option in ["config", "data-dir", "workspace", "session"] {
        if let Some(value) = parsed.value(option) {
            run_args.push(format!("--{option}"));
            run_args.push(value.to_owned());
        }
    }
    let result = run::execute(&run_args)?;
    Ok(success("harness run", result.data, result.human))
}

fn harness_value(coding: &CodingHarness) -> serde_json::Value {
    let manifest = coding.manifest();
    let genes = coding
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
