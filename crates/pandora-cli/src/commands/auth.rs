use super::{load_config, parse_options, timestamp};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::{AccessRole, DeviceKeyStore, IdentityEnrollmentRequest, IdentityStore};
use pandora_types::{PrincipalId, TenantId, WorkspaceId};
use serde_json::json;
use std::fs;
use std::path::PathBuf;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("auth requires 'enroll', 'list', or 'revoke'"))?;
    match subcommand.as_str() {
        "enroll" => enroll(&args[1..]),
        "list" => list(&args[1..]),
        "revoke" => revoke(&args[1..]),
        unknown => Err(CliError::usage(format!("unknown auth command '{unknown}'"))),
    }
}

fn enroll(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "principal",
            "tenant",
            "workspace-id",
            "role",
            "device-key-file",
            "token-file",
        ],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage("auth enroll accepts only named options"));
    }
    let principal = PrincipalId::new(required(&parsed, "principal")?)
        .map_err(|_| CliError::usage("--principal is invalid"))?;
    let tenant = TenantId::new(required(&parsed, "tenant")?)
        .map_err(|_| CliError::usage("--tenant is invalid"))?;
    let workspace = WorkspaceId::new(required(&parsed, "workspace-id")?)
        .map_err(|_| CliError::usage("--workspace-id is invalid"))?;
    let role = AccessRole::parse(required(&parsed, "role")?)
        .map_err(|_| CliError::usage("--role must be viewer, operator, or administrator"))?;
    let config = load_config(&parsed)?;
    fs::create_dir_all(config.data_dir()).map_err(|_| {
        CliError::configuration("could not prepare Pandora data directory", json!({}))
    })?;
    let device_key_path = parsed.value("device-key-file").map_or_else(
        || config.data_dir().join("devices").join("service-device.key"),
        PathBuf::from,
    );
    let device_key = DeviceKeyStore::load_or_create(&device_key_path)
        .map_err(|error| CliError::configuration(error.to_string(), json!({})))?;
    let store = identity_store(&config)?;
    let enrollment = store
        .enroll(
            IdentityEnrollmentRequest::new(
                principal,
                tenant,
                workspace,
                role,
                timestamp().as_unix_seconds(),
            ),
            device_key.device_id(),
            device_key.public_key(),
        )
        .map_err(identity_error)?;
    let token_path = parsed.value("token-file").map_or_else(
        || {
            config
                .data_dir()
                .join("service-tokens")
                .join(format!("{}.token", enrollment.identity().id()))
        },
        PathBuf::from,
    );
    if let Some(parent) = token_path.parent() {
        fs::create_dir_all(parent).map_err(|_| {
            CliError::configuration("could not prepare identity token directory", json!({}))
        })?;
    }
    if let Err(error) = enrollment.write_token_file(&token_path) {
        let _ = store.revoke(enrollment.identity().id(), timestamp().as_unix_seconds());
        return Err(identity_error(error));
    }
    Ok(success(
        "auth enroll",
        json!({
            "identity": enrollment.identity(),
            "token_path": token_path,
            "device_key_path": device_key.path(),
            "token_exposed": false,
            "private_key_exposed": false,
        }),
        format!(
            "Identity {} enrolled; credential written to {} and device key protected at {}",
            enrollment.identity().id(),
            token_path.display(),
            device_key.path().display()
        ),
    ))
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage("auth list accepts no positional arguments"));
    }
    let config = load_config(&parsed)?;
    let identities = identity_store(&config)?.list().map_err(identity_error)?;
    Ok(success(
        "auth list",
        json!({
            "identities": identities,
            "count": identities.len(),
            "credentials_exposed": false,
        }),
        format!("{} service identity record(s)", identities.len()),
    ))
}

fn revoke(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "yes"])?;
    if parsed.positionals.len() != 1 || parsed.value("yes").is_none() {
        return Err(CliError::usage(
            "auth revoke requires '<identity-id> --yes'",
        ));
    }
    let config = load_config(&parsed)?;
    identity_store(&config)?
        .revoke(&parsed.positionals[0], timestamp().as_unix_seconds())
        .map_err(identity_error)?;
    Ok(success(
        "auth revoke",
        json!({"identity_id": parsed.positionals[0], "revoked": true}),
        format!("Identity {} revoked", parsed.positionals[0]),
    ))
}

fn identity_store(
    config: &pandora_runtime::config::RuntimeConfig,
) -> Result<IdentityStore, CliError> {
    IdentityStore::open(config.data_dir().join("identities.sqlite3")).map_err(identity_error)
}

fn required<'a>(parsed: &'a super::ParsedArgs, name: &str) -> Result<&'a str, CliError> {
    parsed
        .value(name)
        .ok_or_else(|| CliError::usage(format!("auth enroll requires '--{name} <value>'")))
}

fn identity_error(error: pandora_runtime::IdentityStoreError) -> CliError {
    CliError::configuration(error.to_string(), json!({}))
}
