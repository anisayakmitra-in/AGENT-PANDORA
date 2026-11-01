use super::parse_options;
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::lsp_engine::LspEngine;
use pandora_types::{LspDiagnosticSeverity, LspServerKind};
use serde_json::json;
use std::path::{Path, PathBuf};

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("lsp requires 'status', 'servers', 'definition', 'references', 'type-definition', or 'diagnostics'"))?;

    match subcommand.as_str() {
        "status" => status(&args[1..]),
        "servers" => servers(&args[1..]),
        "definition" => definition(&args[1..]),
        "references" => references(&args[1..]),
        "type-definition" => type_definition(&args[1..]),
        "diagnostics" => diagnostics(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown lsp subcommand '{unknown}'. Available subcommands: status, servers, definition, references, type-definition, diagnostics"
        ))),
    }
}

fn resolve_workspace(parsed: &super::ParsedArgs) -> PathBuf {
    parsed
        .value("workspace")
        .map(PathBuf::from)
        .or_else(|| std::env::var("PANDORA_WORKSPACE").ok().map(PathBuf::from))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn parse_server(parsed: &super::ParsedArgs) -> Result<Option<LspServerKind>, CliError> {
    if let Some(s) = parsed.value("server") {
        let kind: LspServerKind = s.parse().map_err(CliError::usage)?;
        Ok(Some(kind))
    } else {
        Ok(None)
    }
}

fn status(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["server", "workspace"])?;
    let workspace = resolve_workspace(&parsed);
    let engine = LspEngine::new(&workspace);

    let specific_server = parse_server(&parsed)?;

    let statuses = if let Some(kind) = specific_server {
        vec![engine.server_status(&kind)]
    } else {
        engine.all_server_statuses()
    };

    let mut lines = Vec::new();
    lines.push(format!(
        "Language Server Statuses (root: {}):",
        workspace.display()
    ));
    for s in &statuses {
        let state = if s.running {
            "running"
        } else if s.available {
            "available"
        } else {
            "not installed"
        };
        lines.push(format!("  - {}: {} (binary: {})", s.name, state, s.binary));
    }

    let summary = lines.join("\n");
    let payload = json!({
        "workspace": workspace.to_string_lossy(),
        "servers": statuses,
    });

    Ok(success("lsp status", payload, summary))
}

fn servers(args: &[String]) -> Result<CommandResult, CliError> {
    let _parsed = parse_options(args, &[])?;

    let mut server_info = Vec::new();
    let mut lines = Vec::new();
    lines.push("Supported Language Servers:".to_string());

    for kind in LspServerKind::all_standard() {
        let info = json!({
            "kind": kind.as_str(),
            "name": kind.display_name(),
            "default_binary": kind.default_binary(),
            "candidate_binaries": kind.candidate_binaries(),
            "default_args": kind.default_args(),
            "trigger_files": kind.trigger_files(),
            "file_extensions": kind.file_extensions(),
        });
        lines.push(format!(
            "  - {} (id: '{}'): binary: {}, triggers: {:?}, extensions: {:?}",
            kind.display_name(),
            kind.as_str(),
            kind.default_binary(),
            kind.trigger_files(),
            kind.file_extensions(),
        ));
        server_info.push(info);
    }

    let summary = lines.join("\n");
    let payload = json!({ "servers": server_info });
    Ok(success("lsp servers", payload, summary))
}

fn definition(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["file", "line", "col", "server", "workspace"])?;
    let file = parsed
        .value("file")
        .ok_or_else(|| CliError::usage("lsp definition requires '--file <path>'"))?;
    let line: u32 = parsed
        .value("line")
        .ok_or_else(|| CliError::usage("lsp definition requires '--line <line_number>'"))?
        .parse()
        .map_err(|e| CliError::usage(format!("invalid line number: {e}")))?;
    let col: u32 = parsed
        .value("col")
        .ok_or_else(|| CliError::usage("lsp definition requires '--col <column_number>'"))?
        .parse()
        .map_err(|e| CliError::usage(format!("invalid column number: {e}")))?;

    let server = parse_server(&parsed)?;
    let workspace = resolve_workspace(&parsed);
    let engine = LspEngine::new(&workspace);

    let result = engine
        .goto_definition(Path::new(file), line, col, server)
        .map_err(|e| CliError::execution(e.to_string(), serde_json::Value::Null))?;

    let summary = if result.locations.is_empty() {
        format!("No definition found for symbol at {file}:{line}:{col}")
    } else {
        let mut lines = Vec::new();
        lines.push(format!("Found {} definition(s):", result.locations.len()));
        for loc in &result.locations {
            let target_path = loc.relative_path.as_deref().unwrap_or(&loc.uri);
            let (target_line, target_col) = loc.range.start.to_one_indexed();
            lines.push(format!("  - {target_path}:{target_line}:{target_col}"));
        }
        lines.join("\n")
    };

    let payload = json!(result);
    Ok(success("lsp definition", payload, summary))
}

fn references(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "file",
            "line",
            "col",
            "include-declaration",
            "server",
            "workspace",
        ],
    )?;
    let file = parsed
        .value("file")
        .ok_or_else(|| CliError::usage("lsp references requires '--file <path>'"))?;
    let line: u32 = parsed
        .value("line")
        .ok_or_else(|| CliError::usage("lsp references requires '--line <line_number>'"))?
        .parse()
        .map_err(|e| CliError::usage(format!("invalid line number: {e}")))?;
    let col: u32 = parsed
        .value("col")
        .ok_or_else(|| CliError::usage("lsp references requires '--col <column_number>'"))?
        .parse()
        .map_err(|e| CliError::usage(format!("invalid column number: {e}")))?;

    let include_decl = parsed.has_flag("include-declaration");
    let server = parse_server(&parsed)?;
    let workspace = resolve_workspace(&parsed);
    let engine = LspEngine::new(&workspace);

    let result = engine
        .find_references(Path::new(file), line, col, include_decl, server)
        .map_err(|e| CliError::execution(e.to_string(), serde_json::Value::Null))?;

    let summary = if result.references.is_empty() {
        format!("No references found for symbol at {file}:{line}:{col}")
    } else {
        let mut lines = Vec::new();
        lines.push(format!("Found {} reference(s):", result.references.len()));
        for loc in &result.references {
            let target_path = loc.relative_path.as_deref().unwrap_or(&loc.uri);
            let (target_line, target_col) = loc.range.start.to_one_indexed();
            lines.push(format!("  - {target_path}:{target_line}:{target_col}"));
        }
        lines.join("\n")
    };

    let payload = json!(result);
    Ok(success("lsp references", payload, summary))
}

fn type_definition(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["file", "line", "col", "server", "workspace"])?;
    let file = parsed
        .value("file")
        .ok_or_else(|| CliError::usage("lsp type-definition requires '--file <path>'"))?;
    let line: u32 = parsed
        .value("line")
        .ok_or_else(|| CliError::usage("lsp type-definition requires '--line <line_number>'"))?
        .parse()
        .map_err(|e| CliError::usage(format!("invalid line number: {e}")))?;
    let col: u32 = parsed
        .value("col")
        .ok_or_else(|| CliError::usage("lsp type-definition requires '--col <column_number>'"))?
        .parse()
        .map_err(|e| CliError::usage(format!("invalid column number: {e}")))?;

    let server = parse_server(&parsed)?;
    let workspace = resolve_workspace(&parsed);
    let engine = LspEngine::new(&workspace);

    let result = engine
        .type_definition(Path::new(file), line, col, server)
        .map_err(|e| CliError::execution(e.to_string(), serde_json::Value::Null))?;

    let summary = if result.locations.is_empty() {
        format!("No type definition found for symbol at {file}:{line}:{col}")
    } else {
        let mut lines = Vec::new();
        lines.push(format!(
            "Found {} type definition(s):",
            result.locations.len()
        ));
        for loc in &result.locations {
            let target_path = loc.relative_path.as_deref().unwrap_or(&loc.uri);
            let (target_line, target_col) = loc.range.start.to_one_indexed();
            lines.push(format!("  - {target_path}:{target_line}:{target_col}"));
        }
        lines.join("\n")
    };

    let payload = json!(result);
    Ok(success("lsp type-definition", payload, summary))
}

fn diagnostics(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["file", "severity", "server", "workspace"])?;
    let file = parsed.value("file").map(Path::new);
    let severity = if let Some(s) = parsed.value("severity") {
        if s.eq_ignore_ascii_case("all") {
            None
        } else {
            let sev: LspDiagnosticSeverity = s.parse().map_err(CliError::usage)?;
            Some(sev)
        }
    } else {
        None
    };

    let server = parse_server(&parsed)?;
    let workspace = resolve_workspace(&parsed);
    let engine = LspEngine::new(&workspace);

    let result = engine
        .diagnostics(file, severity, server)
        .map_err(|e| CliError::execution(e.to_string(), serde_json::Value::Null))?;

    let summary = if result.diagnostics.is_empty() {
        "No compiler diagnostics reported (clean check)".to_string()
    } else {
        let mut lines = Vec::new();
        lines.push(format!(
            "Diagnostics: {} total ({} error(s), {} warning(s)):",
            result.total, result.errors, result.warnings
        ));
        for d in &result.diagnostics {
            let target_path = d.relative_path.as_deref().unwrap_or(&d.uri);
            let (line, col) = d.range.start.to_one_indexed();
            let code_str = d
                .code
                .as_deref()
                .map(|c| format!(" [{c}]"))
                .unwrap_or_default();
            lines.push(format!(
                "  - [{}] {}:{}:{}{}: {}",
                d.severity.as_str().to_uppercase(),
                target_path,
                line,
                col,
                code_str,
                d.message
            ));
        }
        lines.join("\n")
    };

    let payload = json!(result);
    Ok(success("lsp diagnostics", payload, summary))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lsp_servers_command() {
        let result = execute(&["servers".to_string()]).expect("lsp servers command should succeed");
        assert_eq!(result.command, "lsp servers");
        let servers = result
            .data
            .get("servers")
            .and_then(|s| s.as_array())
            .expect("servers array");
        assert_eq!(servers.len(), 4);
    }

    #[test]
    fn test_lsp_status_command() {
        let result = execute(&["status".to_string()]).expect("lsp status command should succeed");
        assert_eq!(result.command, "lsp status");
        assert!(result.human.contains("Language Server Statuses"));
    }

    #[test]
    fn test_lsp_subcommand_dispatch_error() {
        let err = match execute(&["invalid-subcommand".to_string()]) {
            Err(e) => e,
            Ok(_) => panic!("expected error"),
        };
        assert!(err.message.contains("unknown lsp subcommand"));
    }
}
