use super::{load_config, parse_options, require_config_file, session_scope};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::curator::{CuratorEngine, CuratorError};
use serde_json::json;
use std::time::Duration;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("curator requires 'run' or 'status'"))?;
    match subcommand.as_str() {
        "run" => run(&args[1..]),
        "status" => status(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown curator command '{unknown}'"
        ))),
    }
}

fn curator_error(error: CuratorError) -> CliError {
    CliError::internal(format!("curator operation failed: {error}"), json!({}))
}

fn run(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "interval",
            "max-passes",
            "once",
        ],
    )?;

    let config = load_config(&parsed)?;
    require_config_file(&config)?;

    let sessions_db = config.data_dir().join("sessions.sqlite3");
    let fleet_db = config.data_dir().join("fleet.sqlite3");
    let curator = CuratorEngine::new(&sessions_db, &fleet_db);

    let (principal, tenant, workspace) = session_scope();
    let root = config.workspace_dir();
    let root_str = root.to_str();

    let once = parsed.has_flag("once");
    let max_passes = parsed
        .value("max-passes")
        .map(|v| {
            v.parse::<usize>()
                .map_err(|_| CliError::usage("curator run --max-passes must be an integer"))
        })
        .transpose()?;

    if once || max_passes == Some(1) {
        let result = curator
            .perform_maintenance_pass(&principal, &tenant, &workspace, root_str)
            .map_err(curator_error)?;

        Ok(success(
            "curator run",
            serde_json::to_value(&result).unwrap_or_else(|_| json!({})),
            format!(
                "Curator pass completed in {}ms: {} sessions evaluated, {} SOPs crystallized, {} promoted, {} stale supervisors reaped, {} bytes reclaimed",
                result.duration_ms,
                result.sessions_evaluated,
                result.sops_crystallized,
                result.sops_promoted,
                result.stale_supervisors_reaped,
                result.reclaimed_bytes
            ),
        ))
    } else {
        let interval_secs = parsed
            .value("interval")
            .map(|v| {
                v.parse::<u64>().map_err(|_| {
                    CliError::usage("curator run --interval must be an integer seconds")
                })
            })
            .transpose()?
            .unwrap_or(30);

        let passes = curator
            .run_watch(
                &principal,
                &tenant,
                &workspace,
                root_str,
                Duration::from_secs(interval_secs),
                max_passes,
            )
            .map_err(curator_error)?;

        let total_crystallized: usize = passes.iter().map(|p| p.sops_crystallized).sum();
        let total_promoted: usize = passes.iter().map(|p| p.sops_promoted).sum();
        let total_reaped: usize = passes.iter().map(|p| p.stale_supervisors_reaped).sum();
        let total_reclaimed: u64 = passes.iter().map(|p| p.reclaimed_bytes).sum();

        Ok(success(
            "curator run",
            json!({
                "passes_completed": passes.len(),
                "passes": passes,
            }),
            format!(
                "Curator finished {} pass(es): {} SOPs crystallized, {} promoted, {} stale supervisors reaped, {} bytes reclaimed",
                passes.len(),
                total_crystallized,
                total_promoted,
                total_reaped,
                total_reclaimed
            ),
        ))
    }
}

fn status(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;

    let config = load_config(&parsed)?;
    require_config_file(&config)?;

    let sessions_db = config.data_dir().join("sessions.sqlite3");
    let fleet_db = config.data_dir().join("fleet.sqlite3");
    let curator = CuratorEngine::new(&sessions_db, &fleet_db);

    let (_principal, tenant, workspace) = session_scope();
    let status = curator.status(&tenant, &workspace).map_err(curator_error)?;

    Ok(success(
        "curator status",
        serde_json::to_value(&status).unwrap_or_else(|_| json!({})),
        format!(
            "Curator Status: {} session archives, {} crystallized SOPs, {} active supervisors, database size {} bytes (wal: {} bytes)",
            status.total_sessions,
            status.crystallized_sops,
            status.total_supervisors,
            status.database_size_bytes,
            status.wal_size_bytes
        ),
    ))
}
