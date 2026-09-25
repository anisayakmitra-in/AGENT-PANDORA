use super::parse_options;
use super::run::configured_harnesses;
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::ToolEngine;
use serde_json::json;

/// Which tool set a read resolves against.
///
/// `Builtin` is the compiled-in set and needs no configuration. `Active` adds
/// the Wasm genes of the admitted, active Harness catalog, which is deployment
/// state and therefore does need a config. The default stays `Builtin` so the
/// existing output cannot move.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Catalog {
    Builtin,
    Active,
}

impl Catalog {
    fn parse(value: Option<&str>) -> Result<Self, CliError> {
        match value {
            None => Ok(Self::Builtin),
            Some("builtin") => Ok(Self::Builtin),
            Some("active") => Ok(Self::Active),
            Some(other) => Err(CliError::usage(format!(
                "unknown tool catalog '{other}'; pass --catalog builtin or --catalog active"
            ))),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Active => "active",
        }
    }
}

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("tool requires 'list' or 'inspect'"))?;
    match subcommand.as_str() {
        "list" => list(&args[1..]),
        "inspect" => inspect(&args[1..]),
        unknown => Err(CliError::usage(format!("unknown tool command '{unknown}'"))),
    }
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "catalog"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "tool list does not accept positional arguments",
        ));
    }
    let catalog = Catalog::parse(parsed.value("catalog"))?;
    let engine = tool_engine(&parsed, catalog)?;
    let tools = engine
        .list()
        .into_iter()
        .map(tool_value)
        .collect::<Vec<_>>();
    // The built-in shape is left exactly as it was. Only the active catalog
    // reports which set it resolved, so a default invocation cannot change.
    let mut payload = json!({"tools": tools});
    let summary = if catalog == Catalog::Active {
        let count = tools.len();
        payload["catalog"] = json!(Catalog::Active.as_str());
        format!("{count} active tools available")
    } else {
        format!("{} built-in tools available", tools.len())
    };
    Ok(success("tool list", payload, summary))
}

fn inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "catalog"])?;
    let catalog = Catalog::parse(parsed.value("catalog"))?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage("tool inspect requires exactly one tool ID"));
    }
    let tool_id = &parsed.positionals[0];
    let tool = tool_engine(&parsed, catalog)?
        .list()
        .into_iter()
        .find(|tool| tool.id().as_str() == tool_id)
        .ok_or_else(|| CliError::usage(format!("unknown tool '{tool_id}'")))?;
    let mut payload = json!({"tool": tool_value(tool)});
    if catalog == Catalog::Active {
        payload["catalog"] = json!(Catalog::Active.as_str());
    }
    Ok(success(
        "tool inspect",
        payload,
        format!("Inspected tool {tool_id}"),
    ))
}

/// Build the requested tool set.
///
/// The active path mirrors the service `runtime.tools` read model: start from the
/// built-ins, then register the Wasm genes carried by the active Harness catalog.
/// It reuses `configured_harnesses`, so admission state is enforced by the same
/// code that `run` uses rather than by a second interpretation of the package
/// store.
fn tool_engine(parsed: &super::ParsedArgs, catalog: Catalog) -> Result<ToolEngine, CliError> {
    let engine = ToolEngine::with_builtins();
    if catalog == Catalog::Builtin {
        return Ok(engine);
    }
    let config = super::load_config(parsed)?;
    super::require_config_file(&config)?;
    let harnesses = configured_harnesses(&config, None, None)?;
    engine
        .register_wasm_genes(
            harnesses
                .iter()
                .flat_map(|harness| harness.genes().iter())
                .map(|gene| gene.manifest().clone()),
        )
        .map_err(|error| match error {
            // Two active genes sharing an id at different versions is the only
            // registration failure reachable from a read, and it is a real
            // deployment inconsistency, so it is named rather than debug-dumped.
            pandora_runtime::tool_engine::ToolError::DuplicateTool => CliError::execution(
                "active Harness catalog declares the same Wasm Gene id at two versions",
                json!({}),
            ),
            other => CliError::execution(format!("{other:?}"), json!({})),
        })?;
    Ok(engine)
}

fn tool_value(tool: pandora_runtime::tool_engine::ToolDefinition) -> serde_json::Value {
    json!({
        "id": tool.id(),
        "version": tool.version(),
        "name": tool.name(),
        "capability": tool.capability().as_str(),
        "operation": tool.operation().as_str(),
        "input_schema": tool.input_schema(),
    })
}
