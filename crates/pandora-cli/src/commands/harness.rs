use super::{parse_options, run};
use crate::output::{CliError, CommandResult, success};
use pandora_harnesses::{CODING_HARNESS_ID, HarnessCatalog};
use pandora_types::HarnessKind;
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
    let harnesses = HarnessCatalog::builtins();
    let values = harnesses.iter().map(harness_value).collect::<Vec<_>>();
    let harness_count = values.len();
    let package_records = super::package::store(&parsed)?
        .list()
        .map_err(super::package::store_error)?;
    Ok(success(
        "harness list",
        json!({
            "harnesses": values,
            "package_records": package_records
                .iter()
                .map(super::package::package_value)
                .collect::<Vec<_>>(),
        }),
        format!(
            "{} harnesses available; {} package record(s)",
            harness_count,
            package_records.len()
        ),
    ))
}

fn inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "harness-version"],
    )?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "harness inspect requires exactly one harness ID",
        ));
    }
    let requested_id = match parsed.positionals[0].as_str() {
        "coding" => CODING_HARNESS_ID,
        requested_id => requested_id,
    };
    let harness_id = pandora_types::HarnessId::new(requested_id.to_owned())
        .map_err(|_| CliError::usage(format!("unknown harness '{requested_id}'")))?;
    let builtins = HarnessCatalog::builtins();
    let harnesses = if let Some(harness) = builtins.find(&harness_id) {
        if let Some(version) = parsed.value("harness-version")
            && version != harness.manifest().version()
        {
            return Err(CliError::usage(format!(
                "built-in Harness '{}' is version {}, not {}",
                requested_id,
                harness.manifest().version(),
                version
            )));
        }
        builtins
    } else {
        let config = super::load_config(&parsed)?;
        super::run::configured_harnesses(
            &config,
            Some(requested_id),
            parsed.value("harness-version"),
        )?
    };
    let harness = harnesses.find(&harness_id).ok_or_else(|| {
        CliError::execution(
            "the requested Harness profile is unavailable",
            json!({"harness_id": requested_id}),
        )
    })?;
    Ok(success(
        "harness inspect",
        json!({"harness": harness_value(harness)}),
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
        &[
            "config",
            "data-dir",
            "workspace",
            "session",
            "gene",
            "task",
            "harness-version",
        ],
    )?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "harness run requires exactly one harness ID",
        ));
    }
    let requested_id = match parsed.positionals[0].as_str() {
        "coding" => CODING_HARNESS_ID,
        requested_id => requested_id,
    };
    let harnesses = HarnessCatalog::builtins();
    let harness_id = pandora_types::HarnessId::new(requested_id.to_owned())
        .map_err(|_| CliError::usage(format!("unknown harness '{requested_id}'")))?;
    if let Some(harness) = harnesses.find(&harness_id) {
        if !harness.is_runnable() {
            return Err(CliError::execution(
                format!("harness '{requested_id}' is not runnable"),
                json!({
                    "harness_id": harness.manifest().id(),
                    "kind": harness.manifest().kind().as_str(),
                }),
            ));
        }
    } else if parsed.value("harness-version").is_none() {
        return Err(CliError::usage(
            "custom Domain Harnesses require '--harness-version <version>'",
        ));
    }
    let canonical_id = harness_id.as_str().to_owned();
    let gene = parsed
        .value("gene")
        .ok_or_else(|| CliError::usage("harness run requires '--gene <id>'"))?;
    let task = parsed
        .value("task")
        .ok_or_else(|| CliError::usage("harness run requires '--task <task>'"))?;
    let mut run_args = vec![
        task.to_owned(),
        "--harness".to_owned(),
        canonical_id,
        "--gene".to_owned(),
        gene.to_owned(),
    ];
    for option in ["config", "data-dir", "workspace", "session"] {
        if let Some(value) = parsed.value(option) {
            run_args.push(format!("--{option}"));
            run_args.push(value.to_owned());
        }
    }
    if let Some(value) = parsed.value("harness-version") {
        run_args.push("--harness-version".to_owned());
        run_args.push(value.to_owned());
    }
    let result = run::execute(&run_args)?;
    Ok(success("harness run", result.data, result.human))
}

fn harness_value(harness: &dyn pandora_types::Harness) -> serde_json::Value {
    let manifest = harness.manifest();
    let meta_composition = manifest.meta_composition().map(|composition| {
        json!({
            "allowed_domains": composition
                .allowed_domains()
                .iter()
                .map(|domain| domain.as_str())
                .collect::<Vec<_>>(),
            "max_handoffs": composition.max_handoffs(),
        })
    });
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
        "execution": {
            "runnable": harness.is_runnable(),
            "mode": execution_mode(manifest.kind()),
        },
        "constitutional_service": manifest.constitutional_service(),
        "constitutional_service_version": manifest.constitutional_service_version(),
        "meta_composition": meta_composition,
        "genes": genes,
    })
}

fn execution_mode(kind: HarnessKind) -> &'static str {
    match kind {
        HarnessKind::Source => "system_augmentation",
        HarnessKind::Meta => "composition_only",
        HarnessKind::Domain => "domain_execution",
    }
}
