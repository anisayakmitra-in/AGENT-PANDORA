#![forbid(unsafe_code)]

use super::{load_config, parse_options, write_config};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::config::RuntimeConfig;
use pandora_runtime::{McpProtocolMode, McpStdioConfig};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    ClaudeCode,
    ClaudeDesktop,
    Cursor,
    Hermes,
}

impl AgentKind {
    #[allow(dead_code)]
    pub fn label(&self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::ClaudeDesktop => "Claude Desktop",
            Self::Cursor => "Cursor",
            Self::Hermes => "Hermes Agent",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DetectedAgent {
    pub kind: AgentKind,
    pub name: String,
    pub path: PathBuf,
    pub config_file: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportedMcpServer {
    pub id: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub url: Option<String>,
    pub env: BTreeMap<String, String>,
}

/// Detects other AI agents installed on the local system.
pub fn detect_available_agents() -> Vec<DetectedAgent> {
    let mut detected = Vec::new();
    let home = home_dir();

    // 1. Claude Code (~/.claude or ~/.claude.json)
    if let Some(ref home_path) = home {
        let claude_dir = home_path.join(".claude");
        let claude_json = home_path.join(".claude.json");
        if claude_dir.is_dir() || claude_json.is_file() {
            let config_file = if claude_json.is_file() {
                Some(claude_json.clone())
            } else if claude_dir.join("settings.json").is_file() {
                Some(claude_dir.join("settings.json"))
            } else if claude_dir.join("claude_desktop_config.json").is_file() {
                Some(claude_dir.join("claude_desktop_config.json"))
            } else {
                None
            };
            detected.push(DetectedAgent {
                kind: AgentKind::ClaudeCode,
                name: "Claude Code".to_owned(),
                path: claude_dir,
                config_file,
            });
        }
    }

    // 2. Claude Desktop
    let desktop_config = claude_desktop_path();
    if let Some(desktop_path) = desktop_config {
        if desktop_path.is_file() {
            detected.push(DetectedAgent {
                kind: AgentKind::ClaudeDesktop,
                name: "Claude Desktop".to_owned(),
                path: desktop_path
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| desktop_path.clone()),
                config_file: Some(desktop_path),
            });
        }
    }

    // 3. Cursor (~/.cursor or .cursor)
    if let Some(ref home_path) = home {
        let cursor_dir = home_path.join(".cursor");
        let cursor_mcp = cursor_dir.join("mcp.json");
        if cursor_dir.is_dir() {
            let config_file = if cursor_mcp.is_file() {
                Some(cursor_mcp)
            } else {
                None
            };
            detected.push(DetectedAgent {
                kind: AgentKind::Cursor,
                name: "Cursor".to_owned(),
                path: cursor_dir,
                config_file,
            });
        }
    }

    // 4. Hermes Agent (~/.hermes or %LOCALAPPDATA%/hermes)
    let hermes_paths = hermes_candidates();
    for hermes_dir in hermes_paths {
        if hermes_dir.is_dir() {
            let config_yaml = hermes_dir.join("config.yaml");
            let mcp_json = hermes_dir.join("mcp.json");
            let config_file = if mcp_json.is_file() {
                Some(mcp_json)
            } else if config_yaml.is_file() {
                Some(config_yaml)
            } else {
                None
            };
            detected.push(DetectedAgent {
                kind: AgentKind::Hermes,
                name: "Hermes Agent".to_owned(),
                path: hermes_dir,
                config_file,
            });
            break;
        }
    }

    detected
}

/// Resolves user home directory reliably across platforms.
pub fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

/// Resolves Claude Desktop config path across operating systems.
fn claude_desktop_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(|appdata| {
            PathBuf::from(appdata)
                .join("Claude")
                .join("claude_desktop_config.json")
        })
    }
    #[cfg(target_os = "macos")]
    {
        home_dir().map(|h| {
            h.join("Library")
                .join("Application Support")
                .join("Claude")
                .join("claude_desktop_config.json")
        })
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        home_dir().map(|h| {
            h.join(".config")
                .join("Claude")
                .join("claude_desktop_config.json")
        })
    }
}

/// Resolves candidate directories for Hermes Agent.
fn hermes_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    #[cfg(windows)]
    {
        if let Some(local_appdata) = std::env::var_os("LOCALAPPDATA") {
            candidates.push(PathBuf::from(local_appdata).join("hermes"));
        }
    }
    if let Some(home) = home_dir() {
        candidates.push(home.join(".hermes"));
    }
    candidates
}

/// Extracts MCP server definitions from a JSON or YAML file or string.
pub fn parse_mcp_servers_from_str(content: &str) -> Vec<ImportedMcpServer> {
    let mut results = Vec::new();

    // 1. Try parsing as JSON first
    if let Ok(val) = serde_json::from_str::<Value>(content) {
        let servers_obj = val
            .get("mcpServers")
            .or_else(|| val.get("mcp_servers"))
            .or_else(|| val.get("servers"))
            .and_then(|v| v.as_object());

        if let Some(servers) = servers_obj {
            for (id, spec) in servers {
                if let Some(server) = parse_single_mcp_spec(id, spec) {
                    results.push(server);
                }
            }
        } else if let Some(obj) = val.as_object() {
            // Check if top-level keys themselves look like MCP server definitions
            for (id, spec) in obj {
                if let Some(server) = parse_single_mcp_spec(id, spec) {
                    results.push(server);
                }
            }
        }
    }

    // 2. Fallback: Parse simple YAML mcp_servers / mcpServers blocks
    if results.is_empty() {
        results = parse_yaml_mcp_servers(content);
    }

    results
}

fn parse_single_mcp_spec(id: &str, spec: &Value) -> Option<ImportedMcpServer> {
    let command = spec
        .get("command")
        .and_then(|c| c.as_str())
        .map(|s| s.to_owned());
    let url = spec
        .get("url")
        .and_then(|u| u.as_str())
        .map(|s| s.to_owned());

    if command.is_none() && url.is_none() {
        return None;
    }

    let args = spec
        .get("args")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_str().map(|s| s.to_owned()))
                .collect()
        })
        .unwrap_or_default();

    let mut env = BTreeMap::new();
    if let Some(env_obj) = spec.get("env").and_then(|e| e.as_object()) {
        for (k, v) in env_obj {
            if let Some(s) = v.as_str() {
                env.insert(k.clone(), s.to_owned());
            }
        }
    }

    Some(ImportedMcpServer {
        id: id.to_owned(),
        command,
        args,
        url,
        env,
    })
}

/// Fallback line-by-line parser for simple YAML MCP blocks (e.g. in Hermes config.yaml).
fn parse_yaml_mcp_servers(content: &str) -> Vec<ImportedMcpServer> {
    let mut servers = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut in_mcp_section = false;
    let mut current_id = String::new();
    let mut current_command: Option<String> = None;
    let mut current_url: Option<String> = None;
    let mut current_args: Vec<String> = Vec::new();

    let flush = |id: &str,
                 cmd: &Option<String>,
                 url: &Option<String>,
                 args: &[String],
                 out: &mut Vec<ImportedMcpServer>| {
        if !id.is_empty() && (cmd.is_some() || url.is_some()) {
            out.push(ImportedMcpServer {
                id: id.to_owned(),
                command: cmd.clone(),
                args: args.to_vec(),
                url: url.clone(),
                env: BTreeMap::new(),
            });
        }
    };

    for line in lines {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }

        if trimmed == "mcp_servers:" || trimmed == "mcpServers:" || trimmed == "mcp:" {
            in_mcp_section = true;
            continue;
        }

        if in_mcp_section {
            // Check indentation: if no leading space, section ended
            if !line.starts_with(' ') && !line.starts_with('\t') {
                in_mcp_section = false;
                flush(
                    &current_id,
                    &current_command,
                    &current_url,
                    &current_args,
                    &mut servers,
                );
                continue;
            }

            // Server key definition, e.g. "  filesystem:"
            if (line.starts_with("  ") || line.starts_with("    "))
                && trimmed.ends_with(':')
                && !trimmed.starts_with("- ")
                && !trimmed.starts_with("command:")
                && !trimmed.starts_with("url:")
                && !trimmed.starts_with("args:")
            {
                flush(
                    &current_id,
                    &current_command,
                    &current_url,
                    &current_args,
                    &mut servers,
                );
                current_id = trimmed.trim_end_matches(':').trim().to_owned();
                current_command = None;
                current_url = None;
                current_args.clear();
            } else if trimmed.starts_with("command:") {
                let val = trimmed.trim_start_matches("command:").trim();
                current_command = Some(val.trim_matches('"').trim_matches('\'').to_owned());
            } else if trimmed.starts_with("url:") {
                let val = trimmed.trim_start_matches("url:").trim();
                current_url = Some(val.trim_matches('"').trim_matches('\'').to_owned());
            } else if trimmed.starts_with("- ") && in_mcp_section {
                let arg = trimmed.trim_start_matches("- ").trim();
                current_args.push(arg.trim_matches('"').trim_matches('\'').to_owned());
            }
        }
    }

    flush(
        &current_id,
        &current_command,
        &current_url,
        &current_args,
        &mut servers,
    );
    servers
}

/// Imports parsed MCP servers into RuntimeConfig.
pub fn apply_imported_mcp_servers(
    servers: &[ImportedMcpServer],
    config: &mut RuntimeConfig,
) -> Vec<String> {
    let mut imported = Vec::new();
    for server in servers {
        if let Some(ref prog) = server.command {
            if let Ok(stdio_cfg) = McpStdioConfig::new(
                &server.id,
                prog,
                server.args.clone(),
                McpProtocolMode::Auto,
            ) {
                config.set_mcp_server(stdio_cfg);
                imported.push(server.id.clone());
            }
        } else if let Some(ref url) = server.url {
            config.set_mcp_http_server(&server.id, url);
            imported.push(server.id.clone());
        }
    }
    imported
}

/// Executes the `pandora mcp import [source]` command.
pub fn execute_import(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "dry-run"])?;
    let dry_run = parsed.has_flag("dry-run");

    let (servers_to_import, source_description) = if let Some(target) = parsed.positionals.first() {
        let path = Path::new(target);
        if path.is_file() {
            let content = fs::read_to_string(path).map_err(|e| {
                CliError::usage(format!(
                    "cannot read configuration file {}: {e}",
                    path.display()
                ))
            })?;
            (
                parse_mcp_servers_from_str(&content),
                format!("file {}", path.display()),
            )
        } else {
            // Target is an agent name keyword: "claude", "cursor", "hermes"
            let target_lower = target.to_lowercase();
            let detected = detect_available_agents();
            let matched = detected.iter().find(|d| {
                d.name.to_lowercase().contains(&target_lower)
                    || format!("{:?}", d.kind).to_lowercase().contains(&target_lower)
            });

            if let Some(agent) = matched {
                let desc = format!("{} ({})", agent.name, agent.path.display());
                let servers = if let Some(ref cfg_file) = agent.config_file {
                    fs::read_to_string(cfg_file)
                        .map(|c| parse_mcp_servers_from_str(&c))
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                (servers, desc)
            } else {
                return Err(CliError::usage(format!(
                    "unknown agent or file '{target}'. Available: claude, cursor, hermes or file path"
                )));
            }
        }
    } else {
        // Auto-detect all agents
        let detected = detect_available_agents();
        if detected.is_empty() {
            return Ok(success(
                "mcp import",
                json!({
                    "detected_agents": [],
                    "imported_count": 0,
                    "servers": []
                }),
                "No other AI agents (Claude Code, Claude Desktop, Cursor, Hermes) detected on this computer.",
            ));
        }

        let mut sources = Vec::new();
        let mut servers = Vec::new();
        for agent in &detected {
            if let Some(ref cfg_file) = agent.config_file {
                if let Ok(content) = fs::read_to_string(cfg_file) {
                    let parsed = parse_mcp_servers_from_str(&content);
                    if !parsed.is_empty() {
                        sources.push(agent.name.clone());
                        servers.extend(parsed);
                    }
                }
            }
        }
        let desc = if sources.is_empty() {
            "detected agents (no MCP configurations found)".to_owned()
        } else {
            sources.join(", ")
        };
        (servers, desc)
    };

    if servers_to_import.is_empty() {
        return Ok(success(
            "mcp import",
            json!({
                "imported_count": 0,
                "servers": [],
                "source": source_description,
            }),
            format!("No MCP servers found to import from {source_description}."),
        ));
    }

    let imported_ids = if !dry_run {
        let mut config = load_config(&parsed)?;
        let ids = apply_imported_mcp_servers(&servers_to_import, &mut config);
        write_config(&config)?;
        ids
    } else {
        servers_to_import.iter().map(|s| s.id.clone()).collect()
    };

    let message = if dry_run {
        format!(
            "Dry run: Found {} MCP server(s) to import from {source_description}: {}",
            imported_ids.len(),
            imported_ids.join(", ")
        )
    } else {
        format!(
            "Successfully imported {} MCP server(s) from {source_description}: {}",
            imported_ids.len(),
            imported_ids.join(", ")
        )
    };

    Ok(success(
        "mcp import",
        json!({
            "imported_count": imported_ids.len(),
            "servers": imported_ids,
            "dry_run": dry_run,
            "source": source_description,
        }),
        message,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_claude_desktop_json() {
        let json_data = r#"{
            "mcpServers": {
                "filesystem": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/dir"],
                    "env": { "DEBUG": "1" }
                },
                "remote-docs": {
                    "url": "http://127.0.0.1:8000/sse"
                }
            }
        }"#;

        let servers = parse_mcp_servers_from_str(json_data);
        assert_eq!(servers.len(), 2);

        let fs_server = servers.iter().find(|s| s.id == "filesystem").unwrap();
        assert_eq!(fs_server.command.as_deref(), Some("npx"));
        assert_eq!(fs_server.args.len(), 3);
        assert_eq!(fs_server.env.get("DEBUG").map(|s| s.as_str()), Some("1"));

        let http_server = servers.iter().find(|s| s.id == "remote-docs").unwrap();
        assert_eq!(
            http_server.url.as_deref(),
            Some("http://127.0.0.1:8000/sse")
        );
        assert!(http_server.command.is_none());
    }

    #[test]
    fn parses_yaml_mcp_servers_from_hermes_format() {
        let yaml_data = r#"
_config_version: 51
database:
  journal_mode: "wal"

mcp_servers:
  local-code:
    command: python
    args:
      - "-m"
      - "code_server"
  remote-fetch:
    url: "https://mcp.example.com/api"
"#;
        let servers = parse_mcp_servers_from_str(yaml_data);
        assert_eq!(servers.len(), 2);
        let code_server = servers.iter().find(|s| s.id == "local-code").unwrap();
        assert_eq!(code_server.command.as_deref(), Some("python"));
        assert_eq!(code_server.args, vec!["-m", "code_server"]);

        let remote_server = servers.iter().find(|s| s.id == "remote-fetch").unwrap();
        assert_eq!(
            remote_server.url.as_deref(),
            Some("https://mcp.example.com/api")
        );
    }
}
