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
usage: pandora <model|moira> <inspect|doctor|status|sync|calibrate> [options]

subcommands:
  inspect [--pnn <path>] [--workspace <dir>]
      Inspect active .pnn tropical neural weights, layer shapes, SHA-256 digests, and active drivers.
  doctor | status [--pnn <path>] [--workspace <dir>]
      Verify integrity and runtime readiness of .pandora/model_tpan.pnn and .pandora/model_decision.pnn.
  sync [--from <repo-or-url>] [--dest <path>] [--revision <rev>] [--file <name>] [--dry-run]
      Explicitly download and verify model_decision.pnn from Hugging Face (default: anisayakmitra-in/FATE-O-SISTERS).
  calibrate [--pairs <path>] [--pnn <path>] [--workspace <dir>] [--dry-run]
      Calibrate model_decision.pnn from DpoPair traces gated by DecisionBenchmarkSuite canary checks.
";

fn cmd_label(invoked_as: &str, action: &str) -> &'static str {
    match (invoked_as, action) {
        ("moira", "help") => "moira help",
        ("moira", "inspect") => "moira inspect",
        ("moira", "doctor") => "moira doctor",
        ("moira", "status") => "moira status",
        ("moira", "sync") => "moira sync",
        ("moira", "calibrate") => "moira calibrate",
        (_, "help") => "model help",
        (_, "inspect") => "model inspect",
        (_, "doctor") => "model doctor",
        (_, "status") => "model status",
        (_, "calibrate") => "model calibrate",
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
        "calibrate" => calibrate_command(invoked_as, rest),
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

fn calibrate_command(invoked_as: &str, args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["pairs", "pnn", "workspace", "config", "data-dir", "dry-run"],
    )?;
    let ws = resolve_workspace_root(&parsed);
    let pnn_path = parsed
        .value("pnn")
        .map(PathBuf::from)
        .unwrap_or_else(|| ws.join(".pandora/model_decision.pnn"));
    let dry_run = parsed.has_flag("dry-run");

    let base_network = if pnn_path.exists() {
        TropicalNetwork::load_from_file(&pnn_path).map_err(|e| {
            CliError::execution(
                format!("failed to load .pnn weights at {}: {e}", pnn_path.display()),
                json!({"path": pnn_path}),
            )
        })?
    } else {
        MoiraDecisionDriver::calibrated_network()
    };

    let pairs: Vec<pandora_types::DpoPair> = if let Some(pairs_path) = parsed.value("pairs") {
        let raw = fs::read_to_string(pairs_path).map_err(|e| {
            CliError::execution(
                format!("failed to read DpoPair file {pairs_path}: {e}"),
                json!({"pairs_path": pairs_path}),
            )
        })?;
        parse_dpo_pairs_content(&raw, pairs_path)?
    } else {
        let default_jsonl = ws.join(".pandora/dpo_pairs.jsonl");
        if default_jsonl.is_file()
            && let Ok(raw) = fs::read_to_string(&default_jsonl)
            && let Ok(loaded) = parse_dpo_pairs_content(&raw, &default_jsonl.display().to_string())
            && !loaded.is_empty()
        {
            loaded
        } else {
            vec![
                MoiraDecisionDriver::emit_dpo_pair_from_escalation(
                    "dpo-default-1",
                    "run_command rm -rf /",
                    "reject",
                    "allow",
                    0.08,
                ),
                MoiraDecisionDriver::emit_dpo_pair_from_escalation(
                    "dpo-default-2",
                    "view_file README.md",
                    "allow",
                    "require_confirm",
                    0.10,
                ),
            ]
        }
    };

    use pandora_runtime::decision_engine::{
        CalibrationAttestationReceipt, DecisionDriver, compute_dpo_dataset_digest,
    };
    let dpo_dataset_digest = compute_dpo_dataset_digest(&pairs);
    let suite =
        pandora_harnesses::decision_benchmark::DecisionBenchmarkSuite::default_real_task_traces();
    let baseline_driver = MoiraDecisionDriver::new(Some(base_network.clone()));
    let baseline_report = suite
        .evaluate_driver("baseline", |q| baseline_driver.evaluate_question(q))
        .map_err(|e| CliError::execution(format!("baseline benchmark failed: {e}"), json!({})))?;

    let candidate_network =
        MoiraDecisionDriver::calibrate_network_from_pairs(&base_network, &pairs, 0.05)
            .map_err(|e| CliError::execution(format!("calibration failed: {e}"), json!({})))?;
    let candidate_driver = MoiraDecisionDriver::new(Some(candidate_network.clone()));
    let candidate_report = suite
        .evaluate_driver("candidate", |q| candidate_driver.evaluate_question(q))
        .map_err(|e| CliError::execution(format!("candidate benchmark failed: {e}"), json!({})))?;

    let canary_gate = pandora_runtime::evolution::BayesianCanaryGate::default();
    let baseline_passes = baseline_report.correct_count as u32;
    let baseline_failures = baseline_report
        .trace_count
        .saturating_sub(baseline_report.correct_count) as u32;
    let baseline_canary = canary_gate.evaluate(baseline_passes, baseline_failures, 0.0);

    let candidate_passes = candidate_report.correct_count as u32;
    let candidate_failures = candidate_report
        .trace_count
        .saturating_sub(candidate_report.correct_count) as u32;
    let statistical_floor = (baseline_canary.lower_bound_95 - 1e-9).max(0.0);
    let bayesian_verdict =
        canary_gate.evaluate(candidate_passes, candidate_failures, statistical_floor);

    let non_regression_passed = candidate_report.branch_selection_accuracy + 1e-9
        >= baseline_report.branch_selection_accuracy
        && candidate_report.mean_margin + 1e-9 >= baseline_report.mean_margin;
    let canary_passed = non_regression_passed && bayesian_verdict.promoted;

    let receipt_path = attestation_receipt_path(&pnn_path);
    let (next_seq, prev_hash) = load_previous_receipt_tip(&receipt_path)?;
    let attestation_receipt = CalibrationAttestationReceipt::new(
        next_seq,
        prev_hash,
        base_network.sha256_digest.clone(),
        candidate_network.sha256_digest.clone(),
        dpo_dataset_digest.clone(),
        bayesian_verdict.lower_bound_95,
        canary_passed,
    );

    if canary_passed && !dry_run {
        ensure_pnn_parent_dir(&pnn_path)?;
        let serialized = serde_json::to_string_pretty(&candidate_network).map_err(|e| {
            CliError::execution(
                format!("failed to serialize calibrated .pnn: {e}"),
                json!({"path": pnn_path}),
            )
        })?;
        let mut atomic = atomic_write_file::AtomicWriteFile::open(&pnn_path).map_err(|e| {
            CliError::execution(
                format!(
                    "failed to open atomic .pnn target {}: {e}",
                    pnn_path.display()
                ),
                json!({"path": pnn_path}),
            )
        })?;
        use std::io::Write;
        atomic.write_all(serialized.as_bytes()).map_err(|e| {
            CliError::execution(
                format!(
                    "failed to write calibrated .pnn to {}: {e}",
                    pnn_path.display()
                ),
                json!({"path": pnn_path}),
            )
        })?;
        atomic.commit().map_err(|e| {
            CliError::execution(
                format!(
                    "failed to commit calibrated .pnn to {}: {e}",
                    pnn_path.display()
                ),
                json!({"path": pnn_path}),
            )
        })?;
        append_calibration_receipt(&receipt_path, &attestation_receipt)?;
    }

    Ok(success(
        cmd_label(invoked_as, "calibrate"),
        json!({
            "path": pnn_path,
            "pairs_count": pairs.len(),
            "dry_run": dry_run,
            "canary_passed": canary_passed,
            "baseline_accuracy": baseline_report.branch_selection_accuracy,
            "candidate_accuracy": candidate_report.branch_selection_accuracy,
            "baseline_mean_margin": baseline_report.mean_margin,
            "candidate_mean_margin": candidate_report.mean_margin,
            "baseline_digest": base_network.sha256_digest,
            "candidate_digest": candidate_network.sha256_digest,
            "dpo_dataset_digest": dpo_dataset_digest,
            "attestation_receipt": {
                "sequence": attestation_receipt.sequence,
                "prev_receipt_hash": attestation_receipt.prev_receipt_hash,
                "baseline_digest": attestation_receipt.baseline_digest,
                "candidate_digest": attestation_receipt.candidate_digest,
                "dpo_dataset_digest": attestation_receipt.dpo_dataset_digest,
                "lower_bound_95": attestation_receipt.lower_bound_95,
                "lower_bound_95_bits": attestation_receipt.lower_bound_95_bits,
                "commitment": attestation_receipt.commitment,
                "chain_hash": attestation_receipt.chain_hash,
                "promoted": attestation_receipt.promoted,
                "verified": attestation_receipt.verify(),
                "receipt_path": receipt_path,
            },
            "bayesian_canary": {
                "credible_level": canary_gate.credible_level(),
                "passes": bayesian_verdict.passes,
                "failures": bayesian_verdict.failures,
                "alpha": bayesian_verdict.alpha,
                "beta": bayesian_verdict.beta,
                "lower_bound_95": bayesian_verdict.lower_bound_95,
                "baseline_lower_bound_95": baseline_canary.lower_bound_95,
                "promoted": bayesian_verdict.promoted,
            },
        }),
        format!(
            "Calibration {} (pairs: {}, accuracy: {:.3} -> {:.3}, mean_margin: {:.3} -> {:.3}, bayesian_lb95: {:.3}, commitment: {}, dry_run: {})",
            if canary_passed {
                "passed canary gate"
            } else {
                "rejected by canary gate"
            },
            pairs.len(),
            baseline_report.branch_selection_accuracy,
            candidate_report.branch_selection_accuracy,
            baseline_report.mean_margin,
            candidate_report.mean_margin,
            bayesian_verdict.lower_bound_95,
            &attestation_receipt.commitment[..16],
            dry_run,
        ),
    ))
}

fn parse_dpo_pairs_content(
    raw: &str,
    source_label: &str,
) -> Result<Vec<pandora_types::DpoPair>, CliError> {
    let trimmed = raw.trim();
    if trimmed.starts_with('[') {
        return serde_json::from_str(trimmed).map_err(|e| {
            CliError::usage(format!("invalid DpoPair JSON array in {source_label}: {e}"))
        });
    }
    let mut pairs = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        let pair: pandora_types::DpoPair = serde_json::from_str(l).map_err(|e| {
            CliError::usage(format!(
                "invalid DpoPair JSONL at {source_label}:{}: {e}",
                idx + 1
            ))
        })?;
        pairs.push(pair);
    }
    if pairs.is_empty() {
        return Err(CliError::usage(format!(
            "no DpoPair records found in {source_label}"
        )));
    }
    Ok(pairs)
}

fn attestation_receipt_path(pnn_path: &Path) -> PathBuf {
    let mut os = pnn_path.as_os_str().to_os_string();
    os.push(".attestation.jsonl");
    PathBuf::from(os)
}

fn load_previous_receipt_tip(receipt_path: &Path) -> Result<(u64, String), CliError> {
    use pandora_runtime::decision_engine::CalibrationAttestationReceipt;
    if !receipt_path.is_file() {
        return Ok((
            0,
            CalibrationAttestationReceipt::GENESIS_PREV_HASH.to_owned(),
        ));
    }
    let raw = fs::read_to_string(receipt_path).map_err(|e| {
        CliError::execution(
            format!(
                "failed to read calibration attestation ledger {}: {e}",
                receipt_path.display()
            ),
            json!({"receipt_path": receipt_path}),
        )
    })?;
    let mut receipts = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        let r: CalibrationAttestationReceipt = serde_json::from_str(l).map_err(|e| {
            CliError::execution(
                format!(
                    "corrupted calibration attestation receipt at {}:{}: {e}",
                    receipt_path.display(),
                    idx + 1
                ),
                json!({"receipt_path": receipt_path, "line": idx + 1}),
            )
        })?;
        receipts.push(r);
    }
    if !CalibrationAttestationReceipt::verify_chain(&receipts) {
        return Err(CliError::execution(
            format!(
                "calibration attestation chain verification failed for {}",
                receipt_path.display()
            ),
            json!({"receipt_path": receipt_path}),
        ));
    }
    if let Some(last) = receipts.last() {
        Ok((last.sequence + 1, last.chain_hash.clone()))
    } else {
        Ok((
            0,
            CalibrationAttestationReceipt::GENESIS_PREV_HASH.to_owned(),
        ))
    }
}

fn append_calibration_receipt(
    receipt_path: &Path,
    receipt: &pandora_runtime::decision_engine::CalibrationAttestationReceipt,
) -> Result<(), CliError> {
    ensure_pnn_parent_dir(receipt_path)?;
    let line = serde_json::to_string(receipt).map_err(|e| {
        CliError::execution(
            format!("failed to serialize calibration attestation receipt: {e}"),
            json!({"receipt_path": receipt_path}),
        )
    })?;
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(receipt_path)
        .map_err(|e| {
            CliError::execution(
                format!(
                    "failed to open calibration attestation ledger {}: {e}",
                    receipt_path.display()
                ),
                json!({"receipt_path": receipt_path}),
            )
        })?;
    use std::io::Write;
    writeln!(f, "{line}").map_err(|e| {
        CliError::execution(
            format!(
                "failed to append calibration attestation receipt to {}: {e}",
                receipt_path.display()
            ),
            json!({"receipt_path": receipt_path}),
        )
    })?;
    Ok(())
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

fn ensure_pnn_parent_dir(pnn_path: &Path) -> Result<(), CliError> {
    if let Some(parent) = pnn_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|e| {
            CliError::execution(
                format!(
                    "failed to create .pnn parent directory {}: {e}",
                    parent.display()
                ),
                json!({"path": pnn_path}),
            )
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibrate_command_dry_run_and_atomic_promotion_work() {
        let root = std::env::temp_dir().join(format!(
            "pandora_moira_calibrate_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let pnn_path = root.join(".pandora/model_decision.pnn");

        // 1. Dry-run evaluates BayesianCanaryGate without creating the .pnn file
        let dry_res = calibrate_command(
            "model",
            &[
                "--workspace".to_string(),
                root.to_string_lossy().into_owned(),
                "--dry-run".to_string(),
            ],
        )
        .unwrap();
        assert_eq!(dry_res.data["dry_run"], true);
        assert_eq!(dry_res.data["canary_passed"], true);
        assert_eq!(dry_res.data["bayesian_canary"]["promoted"], true);
        assert_eq!(dry_res.data["bayesian_canary"]["passes"], 12);
        assert_eq!(dry_res.data["bayesian_canary"]["failures"], 0);
        assert!(
            dry_res.data["bayesian_canary"]["lower_bound_95"]
                .as_f64()
                .unwrap()
                > 0.75
        );
        assert!(!pnn_path.exists());

        // 2. Non-dry-run atomically writes verified .pnn file with valid SHA-256
        let write_res = calibrate_command(
            "moira",
            &[
                "--workspace".to_string(),
                root.to_string_lossy().into_owned(),
            ],
        )
        .unwrap();
        assert_eq!(write_res.data["dry_run"], false);
        assert_eq!(write_res.data["canary_passed"], true);
        assert!(pnn_path.is_file());
        let loaded = TropicalNetwork::load_from_file(&pnn_path).unwrap();
        assert_eq!(
            loaded.sha256_digest,
            write_res.data["candidate_digest"].as_str().unwrap()
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn ensure_pnn_parent_dir_allows_bare_relative_filename_and_creates_nested_dirs() {
        ensure_pnn_parent_dir(Path::new("model_decision.pnn"))
            .expect("bare relative --pnn filename must not invoke create_dir_all(\"\")");

        let root = std::env::temp_dir().join(format!(
            "pandora_moira_parent_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let nested = root.join("nested/sub/model_decision.pnn");
        ensure_pnn_parent_dir(&nested).expect("nested parent directory should be created");
        assert!(root.join("nested/sub").is_dir());
        let _ = fs::remove_dir_all(&root);
    }
}
