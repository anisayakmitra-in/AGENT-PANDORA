//! Subsystem 2: Tripartite Constitutional Governance & Capability Bus CLI.
//!
//! Subcommands:
//! - `status`: Inspect tripartite governance operational status, memory quota, and kill latencies.
//! - `audit`: Verify constitutional effect permit authorization.

use super::parse_options;
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::subsystems::governance::GovernanceSubsystem;
use serde_json::json;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("governance requires 'status' or 'audit'"))?;

    match subcommand.as_str() {
        "status" => status(&args[1..]),
        "audit" => audit(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown governance subcommand '{unknown}'. Available: 'status', 'audit'"
        ))),
    }
}

fn status(args: &[String]) -> Result<CommandResult, CliError> {
    let _parsed = parse_options(args, &["config", "data-dir", "workspace", "json"])?;
    let subsystem = GovernanceSubsystem::new();
    let status = subsystem.status();

    let display = format!(
        "Governance Subsystem [v4.0.5 AETHER]\n\
         Status: {}\n\
         Authorities: {}\n\
         Memory Quota: {} MB\n\
         Kill Latency Bound: {} ms\n\
         Theorems Enforced: {:?}",
        status.status,
        status.active_authorities.join(", "),
        status.max_memory_quota_mb,
        status.max_kill_latency_bound_ms,
        status.theorems_enforced
    );

    Ok(success("governance status", json!(status), display))
}

fn audit(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["permit", "config", "data-dir", "workspace", "json"])?;
    let subsystem = GovernanceSubsystem::new();

    let permit = parsed.value("permit").unwrap_or("filesystem:read");
    let authorized = subsystem.authorize_permit(permit);

    let display = format!(
        "Governance Constitutional Audit\n\
         Permit Request: {}\n\
         Decision: {}\n\
         Constitutional Guarantee: Theorem 74 (Zero Capability Leakage)",
        permit,
        if authorized { "AUTHORIZED" } else { "DENIED" }
    );

    Ok(success(
        "governance audit",
        json!({
            "permit": permit,
            "authorized": authorized,
            "theorem": 74
        }),
        display,
    ))
}
