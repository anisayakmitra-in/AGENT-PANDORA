use super::{parse_options, run};
use crate::output::{CliError, CommandResult, success};
use pandora_harnesses::{
    CODING_HARNESS_ID, DATA_HARNESS_ID, DEBUGGING_HARNESS_ID, DESIGN_HARNESS_ID, HarnessCatalog,
    OPERATIONS_HARNESS_ID, RESEARCH_HARNESS_ID, SECURITY_HARNESS_ID,
};
use pandora_runtime::{PackageRecord, PackageState};
use pandora_types::{HarnessKind, PackageKind};
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
    let admitted_profiles = package_records
        .iter()
        .filter_map(admitted_profile_value)
        .collect::<Vec<_>>();
    let profile_count = admitted_profiles.len();
    Ok(success(
        "harness list",
        json!({
            "harnesses": values,
            "admitted_profiles": admitted_profiles,
            "package_records": package_records
                .iter()
                .map(super::package::package_value)
                .collect::<Vec<_>>(),
        }),
        format!(
            "{harness_count} built-in Harness(es) available; {profile_count} admitted profile(s); {} package record(s)",
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
        "research" => RESEARCH_HARNESS_ID,
        "design" => DESIGN_HARNESS_ID,
        "operations" => OPERATIONS_HARNESS_ID,
        "security" => SECURITY_HARNESS_ID,
        "debugging" => DEBUGGING_HARNESS_ID,
        "data" => DATA_HARNESS_ID,
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
        "research" => RESEARCH_HARNESS_ID,
        "design" => DESIGN_HARNESS_ID,
        "operations" => OPERATIONS_HARNESS_ID,
        "security" => SECURITY_HARNESS_ID,
        "debugging" => DEBUGGING_HARNESS_ID,
        "data" => DATA_HARNESS_ID,
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
        "meta_composition": meta_composition_value(manifest.meta_composition()),
        "genes": harness
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
            .collect::<Vec<_>>(),
    })
}

fn admitted_profile_value(record: &PackageRecord) -> Option<serde_json::Value> {
    if record.state() != PackageState::Admitted {
        return None;
    }
    let manifest = record.manifest();
    let kind = match manifest.kind() {
        PackageKind::DomainHarness => HarnessKind::Domain,
        PackageKind::MetaHarness => HarnessKind::Meta,
        _ => return None,
    };
    Some(json!({
        "id": manifest.id(),
        "version": manifest.version(),
        "kind": kind.as_str(),
        "package_kind": manifest.kind().as_str(),
        "execution": {
            "runnable": kind == HarnessKind::Domain,
            "mode": execution_mode(kind),
        },
        "meta_composition": meta_composition_value(manifest.meta_composition()),
        "state": record.state().as_str(),
        "runtime_authority": record.grants_runtime_authority(),
    }))
}

fn meta_composition_value(
    composition: Option<&pandora_types::MetaComposition>,
) -> Option<serde_json::Value> {
    composition.map(|composition| {
        json!({
            "allowed_domains": composition
                .allowed_domains()
                .iter()
                .map(|domain| domain.as_str())
                .collect::<Vec<_>>(),
            "max_handoffs": composition.max_handoffs(),
        })
    })
}

fn execution_mode(kind: HarnessKind) -> &'static str {
    match kind {
        HarnessKind::Source => "system_augmentation",
        HarnessKind::Meta => "composition_only",
        HarnessKind::Domain => "domain_execution",
    }
}
