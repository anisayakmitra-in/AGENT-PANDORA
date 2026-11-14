#![forbid(unsafe_code)]

use super::{load_config, parse_options};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::bandit_router::TropicalComplexityPrior;
use pandora_runtime::decision_engine::MoiraDecisionDriver;
use pandora_runtime::tropical_nn::TropicalNetwork;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEFAULT_HF_REPO: &str = "anisayakmitra-in/FATE-O-SISTERS";
const DEFAULT_HF_REVISION: &str = "main";
const DEFAULT_HF_FILENAME: &str = "model_decision.pnn";

const MOIRA_USAGE: &str = "\
usage: pandora <model|moira> <inspect|doctor|status|sync> [options]

subcommands:
  inspect [--pnn <path>] [--workspace <dir>]
      Inspect active .pnn tropical neural weights, layer shapes, SHA-256 digests, and active drivers.
  doctor | status [--pnn <path>] [--workspace <dir>]
      Verify integrity and runtime readiness of .pandora/model_tpan.pnn and .pandora/model_decision.pnn.
  sync [--from <repo-or-url>] [--dest <path>] [--revision <rev>] [--file <name>] [--dry-run]
      Explicitly download and verify model_decision.pnn from Hugging Face (default: anisayakmitra-in/FATE-O-SISTERS).
";

fn cmd_label(invoked_as: &str, action: &str) -> &'static str {
    match (invoked_as, action) {
        ("moira", "help") => "moira help",
        ("moira", "inspect") => "moira inspect",
        ("moira", "doctor") => "moira doctor",
        ("moira", "status") => "moira status",
        ("moira", "sync") => "moira sync",
        (_, "help") => "model help",
        (_, "inspect") => "model inspect",
        (_, "doctor") => "model doctor",
        (_, "status") => "model status",
        _ => "model sync",
    }
}

pub fn execute(invoked_as: &str, args: &[String]) -> Result<CommandResult, CliError> {
    if args
        .iter()
        .any(|arg| arg == "--help" || arg == "-h" || arg == "help")
    {
        return Ok(success(
            cmd_label(invoked_as, "help"),
            json!({
                "command": format!("{invoked_as} help"),
                "usage": MOIRA_USAGE,
            }),
            MOIRA_USAGE,
        ));
    }

    let subcommand = args.first().map_or("inspect", String::as_str);
    let rest = if args.is_empty() { &[][..] } else { &args[1..] };

    match subcommand {
        "inspect" => inspect_command(invoked_as, rest),
        "doctor" | "status" => doctor_command(invoked_as, subcommand, rest),
        "sync" | "pull" | "download" => sync_command(invoked_as, rest),
        "list" => super::models::execute(args),
        other => Err(CliError::usage(format!(
            "unknown {invoked_as} subcommand '{other}'.\n\n{MOIRA_USAGE}"
        ))),
    }
}

fn resolve_workspace_root(parsed: &super::ParsedArgs) -> PathBuf {
    if let Some(ws) = parsed.value("workspace") {
        return PathBuf::from(ws);
    }
    if let Ok(ws_env) = std::env::var("PANDORA_WORKSPACE") {
        let p = PathBuf::from(ws_env);
        if p.exists() {
            return p;
        }
    }
    if let Ok(cfg) = load_config(parsed) {
        let ws = cfg.workspace_dir().to_path_buf();
        if ws.exists() {
            return ws;
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn raw_file_sha256(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Some(format!("{:x}", hasher.finalize()))
}

fn summarize_network_file(path: &Path) -> Value {
    if !path.exists() {
        return json!({
            "path": path.display().to_string(),
            "exists": false,
            "verified": false,
            "status": "absent",
        });
    }

    let raw_sha256 = raw_file_sha256(path);
    let size_bytes = fs::metadata(path).map(|m| m.len()).ok();

    match TropicalNetwork::load_from_file(path) {
        Ok(net) => {
            let mut total_params = 0usize;
            let mut layer_shapes = Vec::with_capacity(net.layers.len());
            for (idx, layer) in net.layers.iter().enumerate() {
                let bias_len = layer.bias.as_ref().map_or(0, Vec::len);
                let params = layer.weights.len() + bias_len;
                total_params += params;
                layer_shapes.push(json!({
                    "index": idx,
                    "in_features": layer.in_features,
                    "out_features": layer.out_features,
                    "shape": format!("{}x{}", layer.in_features, layer.out_features),
                    "weights_count": layer.weights.len(),
                    "bias_count": bias_len,
                    "parameters": params,
                }));
            }
            json!({
                "path": path.display().to_string(),
                "exists": true,
                "verified": true,
                "status": "verified",
                "name": net.name,
                "domain": net.domain,
                "sha256_digest": net.sha256_digest,
                "raw_sha256": raw_sha256,
                "size_bytes": size_bytes,
                "layer_count": net.layers.len(),
                "layers": layer_shapes,
                "total_parameters": total_params,
            })
        }
        Err(err) => json!({
            "path": path.display().to_string(),
            "exists": true,
            "verified": false,
            "status": "invalid",
            "raw_sha256": raw_sha256,
            "size_bytes": size_bytes,
            "error": err.to_string(),
        }),
    }
}

fn inspect_command(invoked_as: &str, args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["pnn", "workspace", "config", "data-dir"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(format!(
            "{invoked_as} inspect does not accept positional arguments"
        )));
    }

    if let Some(explicit_pnn) = parsed.value("pnn") {
        let path = PathBuf::from(explicit_pnn);
        let summary = summarize_network_file(&path);
        let verified = summary
            .get("verified")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !verified {
            let err_msg = summary
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("model file does not exist or failed integrity verification");
            return Err(CliError::execution(
                format!("Failed to inspect '{}': {err_msg}", path.display()),
                summary,
            ));
        }

        let driver = MoiraDecisionDriver::from_pnn_path(&path);
        let active_driver = if driver.has_loaded_network() {
            "MoiraDecisionDriver"
        } else {
            "HeuristicDecisionDriver"
        };

        let payload = json!({
            "command": format!("{invoked_as} inspect"),
            "active_driver": active_driver,
            "model": summary,
        });
        let digest = payload["model"]["sha256_digest"]
            .as_str()
            .unwrap_or("unknown");
        return Ok(success(
            cmd_label(invoked_as, "inspect"),
            payload.clone(),
            format!(
                "Inspected {} (driver={active_driver}, sha256={digest})",
                path.display()
            ),
        ));
    }

    let ws = resolve_workspace_root(&parsed);
    let cwd = std::env::current_dir().unwrap_or_else(|_| ws.clone());

    let tpan_path = if ws.join(".pandora/model_tpan.pnn").exists() {
        ws.join(".pandora/model_tpan.pnn")
    } else if cwd.join(".pandora/model_tpan.pnn").exists() {
        cwd.join(".pandora/model_tpan.pnn")
    } else {
        ws.join(".pandora/model_tpan.pnn")
    };

    let decision_path = if ws.join(".pandora/model_decision.pnn").exists() {
        ws.join(".pandora/model_decision.pnn")
    } else if cwd.join(".pandora/model_decision.pnn").exists() {
        cwd.join(".pandora/model_decision.pnn")
    } else {
        ws.join(".pandora/model_decision.pnn")
    };

    let tpan_summary = summarize_network_file(&tpan_path);
    let decision_summary = summarize_network_file(&decision_path);

    let decision_driver = if decision_path.exists() {
        MoiraDecisionDriver::from_pnn_path(&decision_path)
    } else {
        MoiraDecisionDriver::from_default_pandora_path()
    };
    let complexity_prior = if tpan_path.exists() {
        TropicalComplexityPrior::from_pnn_path(&tpan_path)
    } else {
        TropicalComplexityPrior::from_default_path()
    };

    let active_driver = if decision_driver.has_loaded_network() {
        "MoiraDecisionDriver"
    } else {
        "HeuristicDecisionDriver"
    };

    let payload = json!({
        "command": format!("{invoked_as} inspect"),
        "active_driver": active_driver,
        "moira_network_loaded": decision_driver.has_loaded_network(),
        "complexity_prior_loaded": complexity_prior.has_network(),
        "complexity_prior_digest": complexity_prior.digest(),
        "models": {
            "tpan": tpan_summary,
            "decision": decision_summary,
        },
    });

    Ok(success(
        cmd_label(invoked_as, "inspect"),
        payload,
        format!(
            "Active decision driver: {active_driver} (TPAN complexity prior loaded: {})",
            complexity_prior.has_network()
        ),
    ))
}

fn doctor_command(
    invoked_as: &str,
    subcommand: &str,
    args: &[String],
) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["pnn", "workspace", "config", "data-dir"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(format!(
            "{invoked_as} {subcommand} does not accept positional arguments"
        )));
    }

    if let Some(explicit_pnn) = parsed.value("pnn") {
        let path = PathBuf::from(explicit_pnn);
        let summary = summarize_network_file(&path);
        let verified = summary
            .get("verified")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !verified {
            let err_msg = summary
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("model file missing or corrupt");
            return Err(CliError::execution(
                format!(
                    "Model doctor check failed for '{}': {err_msg}",
                    path.display()
                ),
                summary,
            ));
        }

        let driver = MoiraDecisionDriver::from_pnn_path(&path);
        let active_driver = if driver.has_loaded_network() {
            "MoiraDecisionDriver"
        } else {
            "HeuristicDecisionDriver"
        };

        let payload = json!({
            "command": format!("{invoked_as} {subcommand}"),
            "status": "healthy",
            "healthy": true,
            "verified": true,
            "active": true,
            "active_driver": active_driver,
            "driver": active_driver,
            "model": summary,
        });
        return Ok(success(
            cmd_label(invoked_as, subcommand),
            payload,
            format!("Model {} is healthy and verified", path.display()),
        ));
    }

    let ws = resolve_workspace_root(&parsed);
    let cwd = std::env::current_dir().unwrap_or_else(|_| ws.clone());

    let tpan_path = if ws.join(".pandora/model_tpan.pnn").exists() {
        ws.join(".pandora/model_tpan.pnn")
    } else if cwd.join(".pandora/model_tpan.pnn").exists() {
        cwd.join(".pandora/model_tpan.pnn")
    } else {
        ws.join(".pandora/model_tpan.pnn")
    };

    let decision_path = if ws.join(".pandora/model_decision.pnn").exists() {
        ws.join(".pandora/model_decision.pnn")
    } else if cwd.join(".pandora/model_decision.pnn").exists() {
        cwd.join(".pandora/model_decision.pnn")
    } else {
        ws.join(".pandora/model_decision.pnn")
    };

    let tpan_summary = summarize_network_file(&tpan_path);
    let decision_summary = summarize_network_file(&decision_path);

    let decision_driver = if decision_path.exists() {
        MoiraDecisionDriver::from_pnn_path(&decision_path)
    } else {
        MoiraDecisionDriver::from_default_pandora_path()
    };
    let complexity_prior = if tpan_path.exists() {
        TropicalComplexityPrior::from_pnn_path(&tpan_path)
    } else {
        TropicalComplexityPrior::from_default_path()
    };

    let active_driver = if decision_driver.has_loaded_network() {
        "MoiraDecisionDriver"
    } else {
        "HeuristicDecisionDriver"
    };

    let payload = json!({
        "command": format!("{invoked_as} {subcommand}"),
        "status": "healthy",
        "healthy": true,
        "verified": complexity_prior.has_network() || decision_driver.has_loaded_network(),
        "active": true,
        "active_driver": active_driver,
        "driver": active_driver,
        "offline_first": true,
        "tpan_prior_active": complexity_prior.has_network(),
        "moira_decision_active": decision_driver.has_loaded_network(),
        "models": {
            "tpan": tpan_summary,
            "decision": decision_summary,
        },
    });

    Ok(success(
        cmd_label(invoked_as, subcommand),
        payload,
        format!(
            "Model doctor: healthy (active_driver={active_driver}, tpan_prior={})",
            complexity_prior.has_network()
        ),
    ))
}

fn sync_command(invoked_as: &str, args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "from",
            "dest",
            "revision",
            "file",
            "dry-run",
            "workspace",
            "config",
            "data-dir",
        ],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(format!(
            "{invoked_as} sync does not accept positional arguments"
        )));
    }

    let source = parsed.value("from").unwrap_or(DEFAULT_HF_REPO);
    let revision = parsed.value("revision").unwrap_or(DEFAULT_HF_REVISION);
    let filename = parsed.value("file").unwrap_or(DEFAULT_HF_FILENAME);
    let dry_run = parsed.has_flag("dry-run");

    let ws = resolve_workspace_root(&parsed);
    let dest_path = parsed
        .value("dest")
        .map_or_else(|| ws.join(".pandora").join(filename), PathBuf::from);

    let resolved_url = if source.starts_with("http://") || source.starts_with("https://") {
        source.to_owned()
    } else if Path::new(source).exists() {
        format!("file://{}", Path::new(source).display())
    } else {
        format!("https://huggingface.co/{source}/resolve/{revision}/{filename}")
    };

    if dry_run {
        let payload = json!({
            "command": format!("{invoked_as} sync"),
            "dry_run": true,
            "source": source,
            "repository": DEFAULT_HF_REPO,
            "revision": revision,
            "file": filename,
            "resolved_url": resolved_url,
            "destination": dest_path.display().to_string(),
            "offline_first_runtime": true,
        });
        return Ok(success(
            cmd_label(invoked_as, "sync"),
            payload,
            format!(
                "Dry-run: would sync '{source}' ({resolved_url}) to {}",
                dest_path.display()
            ),
        ));
    }

    let raw_content = if source.starts_with("http://") || source.starts_with("https://") {
        fetch_remote_pnn(source)?
    } else if Path::new(source).exists() {
        fs::read_to_string(source).map_err(|e| {
            CliError::execution(
                format!("Failed to read local model file '{source}': {e}"),
                json!({ "source": source }),
            )
        })?
    } else {
        fetch_remote_pnn(&resolved_url)?
    };

    // Parse and verify integrity BEFORE writing to destination
    let net: TropicalNetwork = serde_json::from_str(&raw_content).map_err(|e| {
        CliError::execution(
            format!("Downloaded model failed JSON deserialization: {e}"),
            json!({ "url": resolved_url }),
        )
    })?;

    let computed_digest = net.compute_digest();
    if net.sha256_digest != computed_digest {
        return Err(CliError::execution(
            format!(
                "Downloaded model failed SHA-256 integrity check: expected {}, got {}",
                net.sha256_digest, computed_digest
            ),
            json!({
                "expected_digest": net.sha256_digest,
                "computed_digest": computed_digest,
            }),
        ));
    }

    if let Some(parent) = dest_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|e| {
            CliError::execution(
                format!(
                    "Failed to create parent directory '{}': {e}",
                    parent.display()
                ),
                json!({ "destination": dest_path.display().to_string() }),
            )
        })?;
    }

    // Write atomically via temp file in same directory
    let tmp_path = dest_path.with_extension("pnn.tmp");
    fs::write(&tmp_path, raw_content.as_bytes()).map_err(|e| {
        CliError::execution(
            format!(
                "Failed to write temporary model file '{}': {e}",
                tmp_path.display()
            ),
            json!({ "tmp_path": tmp_path.display().to_string() }),
        )
    })?;
    fs::rename(&tmp_path, &dest_path).map_err(|e| {
        let _ = fs::remove_file(&tmp_path);
        CliError::execution(
            format!(
                "Failed to atomically move model file to '{}': {e}",
                dest_path.display()
            ),
            json!({ "destination": dest_path.display().to_string() }),
        )
    })?;

    let summary = summarize_network_file(&dest_path);
    let payload = json!({
        "command": format!("{invoked_as} sync"),
        "status": "synced",
        "source": source,
        "resolved_url": resolved_url,
        "destination": dest_path.display().to_string(),
        "model": summary,
    });

    Ok(success(
        cmd_label(invoked_as, "sync"),
        payload,
        format!(
            "Synced verified model '{}' (sha256={}) to {}",
            net.name,
            net.sha256_digest,
            dest_path.display()
        ),
    ))
}

fn fetch_remote_pnn(url: &str) -> Result<String, CliError> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| {
            CliError::execution(
                format!("Failed to construct HTTP client: {e}"),
                json!({ "url": url }),
            )
        })?;

    let mut req = client.get(url);
    if let Ok(token) = std::env::var("HF_TOKEN")
        && !token.trim().is_empty()
    {
        req = req.bearer_auth(token.trim());
    }

    let response = req.send().map_err(|e| {
        CliError::execution(
            format!("HTTP request to '{url}' failed: {e}"),
            json!({ "url": url }),
        )
    })?;

    let status = response.status();
    if !status.is_success() {
        return Err(CliError::execution(
            format!("HTTP request to '{url}' returned status {status}"),
            json!({
                "url": url,
                "status_code": status.as_u16(),
            }),
        ));
    }

    response.text().map_err(|e| {
        CliError::execution(
            format!("Failed reading response body from '{url}': {e}"),
            json!({ "url": url }),
        )
    })
}
