use super::{load_config, parse_options, require_config_file, session_scope, timestamp};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::{ApprovalError, ApprovalStore, PendingApproval};
use serde_json::json;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("approval requires 'list', 'inspect', or 'resolve'"))?;
    match subcommand.as_str() {
        "list" => list(&args[1..]),
        "inspect" => inspect(&args[1..]),
        "resolve" => resolve(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown approval command '{unknown}'"
        ))),
    }
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = open_store(&config)?;
    let principal = session_scope().0;
    let approvals = store
        .list(&principal)
        .map_err(approval_error)?
        .into_iter()
        .map(|approval| approval_value(&approval))
        .collect::<Vec<_>>();
    let count = approvals.len();
    Ok(success(
        "approval list",
        json!({"approvals": approvals}),
        format!("{count} approval(s)"),
    ))
}

fn inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "approval inspect requires exactly one approval ID",
        ));
    }
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = open_store(&config)?;
    let principal = session_scope().0;
    let approval = store
        .inspect(&parsed.positionals[0], &principal)
        .map_err(approval_error)?;
    Ok(success(
        "approval inspect",
        json!({"approval": approval_value(&approval)}),
        format!(
            "Approval {} is {}",
            approval.id(),
            approval.status_at(timestamp()).as_str()
        ),
    ))
}

fn resolve(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "allow", "deny"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "approval resolve requires exactly one approval ID",
        ));
    }
    let allow = match (
        parsed.value("allow").is_some(),
        parsed.value("deny").is_some(),
    ) {
        (true, false) => true,
        (false, true) => false,
        _ => {
            return Err(CliError::usage(
                "approval resolve requires exactly one of '--allow' or '--deny'",
            ));
        }
    };
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let store = open_store(&config)?;
    let principal = session_scope().0;
    let approval = store
        .resolve(
            &parsed.positionals[0],
            &principal,
            &principal,
            allow,
            timestamp(),
        )
        .map_err(approval_error)?;
    Ok(success(
        "approval resolve",
        json!({"approval": approval_value(&approval)}),
        format!(
            "Approval {} resolved as {}",
            approval.id(),
            approval.status_at(timestamp()).as_str()
        ),
    ))
}

fn open_store(config: &pandora_runtime::config::RuntimeConfig) -> Result<ApprovalStore, CliError> {
    ApprovalStore::open(config.data_dir().join("sessions.sqlite3")).map_err(approval_error)
}

fn approval_value(approval: &PendingApproval) -> serde_json::Value {
    json!({
        "id": approval.id(),
        "session_id": approval.session_id(),
        "execution_id": approval.execution_id(),
        "principal_id": approval.principal_id(),
        "gene_id": approval.gene_id(),
        "request_digest": approval.request_digest(),
        "request_summary": approval.request_summary(),
        "policy_version": approval.policy_version(),
        "expires_at": approval.expires_at().as_unix_seconds(),
        "status": approval.status_at(timestamp()).as_str(),
        "approver_id": approval.approver_id(),
        "created_at": approval.created_at().as_unix_seconds(),
    })
}

fn approval_error(error: ApprovalError) -> CliError {
    match error {
        ApprovalError::Expired | ApprovalError::Terminal => {
            CliError::approval(error.to_string(), json!({}))
        }
        ApprovalError::ScopeMismatch | ApprovalError::DigestMismatch => {
            CliError::policy(error.to_string(), json!({}))
        }
        other => CliError::internal(other.to_string(), json!({})),
    }
}
