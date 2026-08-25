use super::{load_config, parse_options, timestamp};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::{FleetBudget, FleetEngine, FleetError, FleetLease, FleetNode};
use serde_json::{Value, json};

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("fleet requires 'list', 'register', 'dispatch', 'lease', 'release', 'expire', 'quarantine', 'revoke', or 'kill'"))?;
    match subcommand.as_str() {
        "list" => list(&args[1..]),
        "register" => register(&args[1..]),
        "dispatch" => dispatch(&args[1..]),
        "lease" => lease(&args[1..]),
        "release" => release(&args[1..]),
        "expire" => expire(&args[1..]),
        "quarantine" => transition(&args[1..], "quarantine", FleetEngine::quarantine_node),
        "revoke" => transition(&args[1..], "revoke", FleetEngine::revoke_node),
        "kill" => transition(&args[1..], "kill", FleetEngine::kill_node),
        unknown => Err(CliError::usage(format!(
            "unknown fleet command '{unknown}'"
        ))),
    }
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "fleet list does not accept positional arguments",
        ));
    }
    let fleet = engine(&parsed)?;
    let nodes = fleet.list_nodes().map_err(fleet_error)?;
    let leases = fleet.list_leases().map_err(fleet_error)?;
    Ok(success(
        "fleet list",
        json!({
            "nodes": nodes.iter().map(node_value).collect::<Vec<_>>(),
            "leases": leases.iter().map(lease_value).collect::<Vec<_>>(),
        }),
        format!(
            "Fleet has {} node(s) and {} lease(s)",
            nodes.len(),
            leases.len()
        ),
    ))
}

fn register(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "version",
            "worker-class",
            "capabilities-json",
        ],
    )?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "fleet register requires exactly one node ID",
        ));
    }
    let version = required(&parsed, "version")?;
    let worker_class = required(&parsed, "worker-class")?;
    let capabilities = serde_json::from_str::<Vec<String>>(required(&parsed, "capabilities-json")?)
        .map_err(|_| CliError::usage("--capabilities-json must be a JSON array of strings"))?;
    let node = FleetNode::new(
        parsed.positionals[0].clone(),
        version,
        worker_class,
        capabilities,
        timestamp().as_unix_seconds(),
    )
    .map_err(fleet_error)?;
    let registered = engine(&parsed)?.register_node(&node).map_err(fleet_error)?;
    Ok(success(
        "fleet register",
        json!({"node": node_value(&registered)}),
        format!("Registered Fleet node {}", registered.id()),
    ))
}

fn dispatch(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "fleet dispatch requires exactly one capability",
        ));
    }
    let node = engine(&parsed)?
        .dispatch_node(&parsed.positionals[0])
        .map_err(fleet_error)?;
    Ok(success(
        "fleet dispatch",
        json!({"capability": parsed.positionals[0], "node": node_value(&node)}),
        format!("Selected Fleet node {}", node.id()),
    ))
}

fn lease(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "node",
            "execution",
            "max-tokens",
            "max-tools",
            "max-duration",
            "max-cost",
            "duration",
            "now",
        ],
    )?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage("fleet lease requires exactly one lease ID"));
    }
    let fleet = engine(&parsed)?;
    let lease = fleet
        .acquire_lease(
            parsed.positionals[0].clone(),
            required(&parsed, "node")?,
            required(&parsed, "execution")?,
            FleetBudget::new(
                number(&parsed, "max-tokens")?,
                number(&parsed, "max-tools")?,
                number(&parsed, "max-duration")?,
                number(&parsed, "max-cost")?,
            ),
            parsed.value("now").map_or_else(
                || Ok(timestamp().as_unix_seconds()),
                |value| {
                    value
                        .parse()
                        .map_err(|_| CliError::usage("--now must be an unsigned integer"))
                },
            )?,
            number(&parsed, "duration")?,
        )
        .map_err(fleet_error)?;
    Ok(success(
        "fleet lease",
        json!({"lease": lease_value(&lease)}),
        format!("Issued Fleet lease {}", lease.id()),
    ))
}

fn release(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "fleet release requires exactly one lease ID",
        ));
    }
    let lease = engine(&parsed)?
        .release_lease(&parsed.positionals[0])
        .map_err(fleet_error)?;
    Ok(success(
        "fleet release",
        json!({"lease": lease_value(&lease)}),
        format!("Released Fleet lease {}", lease.id()),
    ))
}

fn expire(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "now"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "fleet expire does not accept positional arguments",
        ));
    }
    let now = parsed.value("now").map_or_else(
        || Ok(timestamp().as_unix_seconds()),
        |value| {
            value
                .parse()
                .map_err(|_| CliError::usage("--now must be an unsigned integer"))
        },
    )?;
    let count = engine(&parsed)?.expire_leases(now).map_err(fleet_error)?;
    Ok(success(
        "fleet expire",
        json!({"expired": count, "now": now}),
        format!("Expired {count} Fleet lease(s)"),
    ))
}

fn transition(
    args: &[String],
    action: &'static str,
    apply: fn(&FleetEngine, &str) -> Result<(), FleetError>,
) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "yes"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(format!(
            "fleet {action} requires exactly one node ID"
        )));
    }
    if !parsed.values.contains_key("yes") {
        return Err(CliError::usage(format!("fleet {action} requires '--yes'")));
    }
    apply(&engine(&parsed)?, &parsed.positionals[0]).map_err(fleet_error)?;
    Ok(success(
        "fleet transition",
        json!({"action": action, "node_id": parsed.positionals[0]}),
        format!("Fleet node {} is now {action}d", parsed.positionals[0]),
    ))
}

fn engine(parsed: &super::ParsedArgs) -> Result<FleetEngine, CliError> {
    let config = load_config(parsed)?;
    FleetEngine::open(config.data_dir().join("fleet.sqlite3")).map_err(fleet_error)
}

fn required<'a>(parsed: &'a super::ParsedArgs, name: &str) -> Result<&'a str, CliError> {
    parsed
        .value(name)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| CliError::usage(format!("fleet requires '--{name} <value>'")))
}

fn number(parsed: &super::ParsedArgs, name: &str) -> Result<u64, CliError> {
    required(parsed, name)?
        .parse()
        .map_err(|_| CliError::usage(format!("--{name} must be an unsigned integer")))
}

fn node_value(node: &FleetNode) -> Value {
    json!({
        "id": node.id(),
        "implementation_version": node.implementation_version(),
        "worker_class": node.worker_class(),
        "capabilities": node.capabilities(),
        "state": node.state().as_str(),
        "registered_at": node.registered_at(),
    })
}

fn lease_value(lease: &FleetLease) -> Value {
    json!({
        "id": lease.id(),
        "node_id": lease.node_id(),
        "execution_id": lease.execution_id(),
        "budget": {
            "max_tokens": lease.budget().max_tokens(),
            "max_tools": lease.budget().max_tools(),
            "max_duration_seconds": lease.budget().max_duration_seconds(),
            "max_cost_micros": lease.budget().max_cost_micros(),
        },
        "issued_at": lease.issued_at(),
        "expires_at": lease.expires_at(),
        "state": lease.state().as_str(),
    })
}

fn fleet_error(error: FleetError) -> CliError {
    CliError::execution(error.to_string(), json!({}))
}
