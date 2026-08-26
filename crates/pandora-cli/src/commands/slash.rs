use super::{ParsedArgs, harness, package, parse_options, run};
use crate::output::{CliError, CommandResult, success};
use pandora_harnesses::{HarnessCatalog, SlashCommand, SlashCommandCatalog, SlashCommandKind};
use pandora_runtime::PackageState;
use pandora_types::PackageKind;
use serde_json::json;

const COMMON_OPTIONS: &[&str] = &[
    "config",
    "data-dir",
    "workspace",
    "session",
    "provider",
    "model",
    "task-class",
    "approval",
    "optimize",
];

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("slash requires 'list' or 'resolve'"))?;
    match subcommand.as_str() {
        "list" => list(&args[1..]),
        "resolve" => resolve(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown slash command '{unknown}'"
        ))),
    }
}

pub fn execute_direct(command: &str, args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, COMMON_OPTIONS)?;
    let catalog = catalog(&parsed)?;
    let target = catalog
        .resolve(command)
        .ok_or_else(|| CliError::usage(format!("unknown slash command '{command}'")))?;
    match target.kind() {
        SlashCommandKind::Harness => inspect_harness(target, &parsed),
        SlashCommandKind::Gene => run_gene(target, &parsed),
    }
}

pub(crate) fn execute_interactive(
    line: &str,
    parsed: &ParsedArgs,
    session_id: Option<&str>,
    approval_id: Option<&str>,
) -> Result<CommandResult, CliError> {
    let (command, args) = interactive_invocation(line, parsed, session_id, approval_id)?;
    execute_direct(&command, &args)
}

fn interactive_invocation(
    line: &str,
    parsed: &ParsedArgs,
    session_id: Option<&str>,
    approval_id: Option<&str>,
) -> Result<(String, Vec<String>), CliError> {
    let mut values = split_interactive_line(line)?.into_iter();
    let command = values
        .next()
        .filter(|command| command.starts_with('/'))
        .ok_or_else(|| CliError::usage("interactive slash command is missing"))?
        .to_owned();
    let mut args = values.collect::<Vec<_>>();
    append_common_options(
        &mut args,
        parsed,
        &["config", "data-dir", "workspace", "provider", "model"],
    );
    if let Some(session_id) = session_id {
        append_option(&mut args, "session", session_id);
    }
    if let Some(approval_id) = approval_id {
        append_option(&mut args, "approval", approval_id);
    }
    Ok((command, args))
}

fn split_interactive_line(line: &str) -> Result<Vec<String>, CliError> {
    let mut values = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;
    let mut characters = line.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '\\' if quoted && matches!(characters.peek(), Some('"' | '\\')) => {
                current.push(characters.next().expect("peeked character exists"));
                started = true;
            }
            character if character.is_whitespace() && !quoted => {
                if started {
                    values.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            character => {
                current.push(character);
                started = true;
            }
        }
    }
    if quoted {
        return Err(CliError::usage(
            "interactive slash command has an unclosed quote",
        ));
    }
    if started {
        values.push(current);
    }
    Ok(values)
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "slash list does not accept positional arguments",
        ));
    }
    let catalog = catalog(&parsed)?;
    let commands = catalog
        .list()
        .into_iter()
        .map(command_value)
        .collect::<Vec<_>>();
    Ok(success(
        "slash list",
        json!({"commands": commands}),
        format!("{} slash command(s) available", commands.len()),
    ))
}

fn resolve(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "slash resolve requires exactly one slash command",
        ));
    }
    let catalog = catalog(&parsed)?;
    let command = catalog.resolve(&parsed.positionals[0]).ok_or_else(|| {
        CliError::usage(format!("unknown slash command '{}'", parsed.positionals[0]))
    })?;
    Ok(success(
        "slash resolve",
        json!({"target": command_value(command)}),
        format!(
            "{} resolves to {} {}",
            command.command(),
            command.kind().as_str(),
            command.harness_id()
        ),
    ))
}

fn catalog(parsed: &ParsedArgs) -> Result<SlashCommandCatalog, CliError> {
    let harnesses = HarnessCatalog::builtins();
    let mut catalog = SlashCommandCatalog::from_harnesses(harnesses.iter())
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let config = super::load_config(parsed)?;
    let records = package::store(parsed)?
        .list()
        .map_err(package::store_error)?;
    for record in records {
        if record.state() == PackageState::Admitted
            && matches!(
                record.manifest().kind(),
                PackageKind::DomainHarness | PackageKind::MetaHarness
            )
        {
            let harnesses = run::configured_harnesses(
                &config,
                Some(record.manifest().id().as_str()),
                Some(record.manifest().version()),
            )?;
            let harness_id =
                pandora_types::HarnessId::new(record.manifest().id().as_str().to_owned())
                    .map_err(|_| CliError::internal("admitted Harness ID is invalid", json!({})))?;
            let harness = harnesses.find(&harness_id).ok_or_else(|| {
                CliError::internal("admitted Harness profile is unavailable", json!({}))
            })?;
            catalog
                .add_profile_harness(harness)
                .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
        }
    }
    Ok(catalog)
}

fn inspect_harness(command: &SlashCommand, parsed: &ParsedArgs) -> Result<CommandResult, CliError> {
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(format!(
            "{} does not accept positional arguments",
            command.command()
        )));
    }
    let mut args = vec![
        "inspect".to_owned(),
        command.harness_id().as_str().to_owned(),
    ];
    append_option(&mut args, "harness-version", command.harness_version());
    append_common_options(&mut args, parsed, &["config", "data-dir", "workspace"]);
    harness::execute(&args)
}

fn run_gene(command: &SlashCommand, parsed: &ParsedArgs) -> Result<CommandResult, CliError> {
    let gene_id = command
        .gene_id()
        .expect("Gene slash command has a Gene target");
    let task = task_for(gene_id.as_str(), &parsed.positionals)?;
    let mut args = vec![
        task,
        "--harness".to_owned(),
        command.harness_id().as_str().to_owned(),
        "--harness-version".to_owned(),
        command.harness_version().to_owned(),
        "--gene".to_owned(),
        gene_id.as_str().to_owned(),
    ];
    append_common_options(&mut args, parsed, COMMON_OPTIONS);
    run::execute(&args)
}

fn task_for(gene_id: &str, values: &[String]) -> Result<String, CliError> {
    match gene_id {
        "workspace.read" => one_argument("read", values),
        "workspace.search" => joined_argument("search", values),
        "patch.apply" => patch_task(values),
        "verification.run" => no_argument("verify", values),
        "tests.run" => no_argument("test", values),
        "format.check" => no_argument("format", values),
        "lint.check" => no_argument("lint", values),
        "build.check" => no_argument("build", values),
        "workspace.status" => no_argument("status", values),
        "workspace.diff" => no_argument("diff", values),
        "change.review" => one_argument("review", values),
        "daedalus.audit" => no_argument("audit", values),
        "argus.review" => one_argument("deep-review", values),
        "ariadne.debt" => no_argument("debt", values),
        "hephaestus.measure" => no_argument("measure", values),
        "athena.guide" => no_argument("guide", values),
        "evidence.inventory" => no_argument("evidence-inventory", values),
        "evidence.search" => joined_argument("evidence-search", values),
        "source.read" => one_argument("source-read", values),
        "source.compare" => {
            if values.len() != 2 {
                return Err(CliError::usage(
                    "/source-compare requires exactly two paths",
                ));
            }
            Ok(format!("source-compare:{}|{}", values[0], values[1]))
        }
        "citation.inventory" => no_argument("citation-inventory", values),
        "research.guide" => no_argument("research-guide", values),
        "design.inventory" => no_argument("design-inventory", values),
        "design.tokens" => no_argument("design-tokens", values),
        "design.inspect" => one_argument("design-inspect", values),
        "design.compare" => {
            if values.len() != 2 {
                return Err(CliError::usage(
                    "/design-compare requires exactly two paths",
                ));
            }
            Ok(format!("design-compare:{}|{}", values[0], values[1]))
        }
        "accessibility.evidence" => no_argument("accessibility-evidence", values),
        "design.guide" => no_argument("design-guide", values),
        "operations.inventory" => no_argument("operations-inventory", values),
        "operations.search" => joined_argument("operations-search", values),
        "config.inspect" => one_argument("config-inspect", values),
        "config.compare" => {
            if values.len() != 2 {
                return Err(CliError::usage(
                    "/config-compare requires exactly two paths",
                ));
            }
            Ok(format!("config-compare:{}|{}", values[0], values[1]))
        }
        "deployment.evidence" => no_argument("deployment-evidence", values),
        "operations.guide" => no_argument("operations-guide", values),
        _ if values.len() == 1 => Ok(values[0].clone()),
        _ => Err(CliError::usage(format!(
            "Gene '{gene_id}' requires exactly one JSON argument"
        ))),
    }
}

fn one_argument(action: &str, values: &[String]) -> Result<String, CliError> {
    if values.len() != 1 {
        return Err(CliError::usage(format!(
            "/{action} requires exactly one argument"
        )));
    }
    Ok(format!("{action}:{}", values[0]))
}

fn joined_argument(action: &str, values: &[String]) -> Result<String, CliError> {
    if values.is_empty() {
        return Err(CliError::usage(format!(
            "/{action} requires a non-empty query"
        )));
    }
    Ok(format!("{action}:{}", values.join(" ")))
}

fn patch_task(values: &[String]) -> Result<String, CliError> {
    let Some((path, content)) = values.split_first() else {
        return Err(CliError::usage("/patch requires a path and content"));
    };
    if content.is_empty() {
        return Err(CliError::usage("/patch requires a path and content"));
    }
    Ok(format!("patch:{path}:{}", content.join(" ")))
}

fn no_argument(action: &str, values: &[String]) -> Result<String, CliError> {
    if !values.is_empty() {
        return Err(CliError::usage(format!(
            "/{action} does not accept positional arguments"
        )));
    }
    Ok(action.to_owned())
}

fn append_common_options(args: &mut Vec<String>, parsed: &ParsedArgs, allowed: &[&str]) {
    for option in allowed {
        if let Some(value) = parsed.value(option) {
            append_option(args, option, value);
        }
    }
}

fn append_option(args: &mut Vec<String>, name: &str, value: &str) {
    args.push(format!("--{name}"));
    args.push(value.to_owned());
}

fn command_value(command: &SlashCommand) -> serde_json::Value {
    json!({
        "command": command.command(),
        "kind": command.kind().as_str(),
        "harness_id": command.harness_id(),
        "harness_version": command.harness_version(),
        "gene_id": command.gene_id(),
        "alias": command.is_alias(),
    })
}

#[cfg(test)]
mod tests {
    use super::{interactive_invocation, split_interactive_line};
    use crate::commands::ParsedArgs;
    use std::collections::BTreeMap;

    #[test]
    fn interactive_invocation_preserves_exact_command_scope() {
        let parsed = ParsedArgs {
            values: BTreeMap::from([
                ("workspace".to_owned(), "workspace-root".to_owned()),
                ("provider".to_owned(), "coding".to_owned()),
            ]),
            positionals: Vec::new(),
        };

        let (command, args) = interactive_invocation(
            "/gene:owner%2Fdomain@1.0.0:workspace.search rust async",
            &parsed,
            Some("session-1"),
            Some("approval-1"),
        )
        .unwrap();

        assert_eq!(command, "/gene:owner%2Fdomain@1.0.0:workspace.search");
        assert_eq!(
            args,
            [
                "rust",
                "async",
                "--workspace",
                "workspace-root",
                "--provider",
                "coding",
                "--session",
                "session-1",
                "--approval",
                "approval-1",
            ]
        );
    }

    #[test]
    fn interactive_arguments_preserve_quoted_paths() {
        assert_eq!(
            split_interactive_line(r#"/read "My Project/README.md""#).unwrap(),
            ["/read", "My Project/README.md"]
        );
        assert!(split_interactive_line(r#"/read "unfinished"#).is_err());
    }
}
