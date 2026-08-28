use super::provider::{configured_provider_for, provider_credential_available};
use super::run::active_skill_context;
use super::{load_config, parse_options, require_config_file, session_scope, session_store};
use crate::output::{CliError, CommandResult, already_printed};
use pandora_harnesses::HarnessCatalog;
use pandora_runtime::config::RuntimeConfig;
use pandora_runtime::executors::WorkspaceRoot;
use pandora_runtime::{
    AccessRole, ApprovalStore, ArtifactCatalog, DeviceKeyStore, EvolutionEngine,
    ExecutionController, FleetEngine, IdentityEnrollmentRequest, IdentityStore, OrchestrationStore,
    RuntimeService, RuntimeServiceScope,
};
use pandora_service::{LocalService, LocalServiceConfig};
use pandora_types::{
    Capability, EvolutionPolicy, Operation, PolicyContext, ServiceProviderSummary,
};
use serde_json::json;
use std::fs;
use std::io::{self, Write};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;

const SERVICE_AGENT_MAX_TURNS: u32 = 8;
const SERVICE_AGENT_MAX_TOOL_CALLS: u32 = 16;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "port",
            "token-file",
            "device-key-file",
        ],
    )?;
    if parsed.positionals.as_slice() != ["start"] {
        return Err(CliError::usage("service requires 'start'"));
    }
    let port = parsed
        .value("port")
        .map(|value| {
            value
                .parse::<u16>()
                .map_err(|_| CliError::usage("--port must be a valid TCP port"))
        })
        .transpose()?
        .unwrap_or(0);
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let runtime = build_runtime_service(&config)?;
    let identities = IdentityStore::open(config.data_dir().join("identities.sqlite3"))
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let (token_path, device_key_path, device_id) =
        prepare_service_identity(&config, &identities, &parsed)?;
    let service = LocalService::new(
        LocalServiceConfig::with_identities(
            SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
            runtime,
            identities,
        )
        .map_err(|_| CliError::internal("could not configure the local service", json!({})))?,
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()
        .map_err(|_| CliError::internal("could not start the local service runtime", json!({})))?;

    runtime.block_on(async move {
        let bound = service
            .bind()
            .await
            .map_err(|_| CliError::internal("could not bind the local service", json!({})))?;
        write_readiness(
            bound.local_addr(),
            &token_path,
            &device_key_path,
            &device_id,
        )?;
        bound
            .serve_until(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
            .map_err(|_| CliError::internal("local service stopped unexpectedly", json!({})))
    })?;

    Ok(already_printed("service start"))
}

fn build_runtime_service(config: &RuntimeConfig) -> Result<RuntimeService, CliError> {
    let workspace = WorkspaceRoot::new(config.workspace_dir()).map_err(|_| {
        CliError::configuration(
            "workspace path is invalid",
            json!({"workspace": config.workspace_dir()}),
        )
    })?;
    let policy = PolicyContext::new(
        1,
        [
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
            Capability::ProcessExecute,
            Capability::ProviderInvoke,
        ],
        [Operation::Write, Operation::Execute],
    );
    let controller = ExecutionController::with_policy_and_harnesses(
        workspace,
        policy,
        HarnessCatalog::builtins(),
    );
    let sessions = session_store(config)?;
    let approvals = ApprovalStore::open(config.data_dir().join("sessions.sqlite3"))
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let (principal, tenant, workspace) = session_scope();
    let providers = config
        .provider_names()
        .into_iter()
        .filter_map(|name| config.provider_profile(&name))
        .map(|profile| {
            ServiceProviderSummary::new(
                profile.name(),
                profile.model(),
                profile.protocol().as_str(),
                config.active_provider() == Some(profile.name()),
                provider_credential_available(config, profile.api_key_env()).unwrap_or(false),
                profile.fallback_provider().map(str::to_owned),
            )
        })
        .collect();
    let evolution = EvolutionEngine::open(
        config.data_dir().join("evolution.sqlite3"),
        EvolutionPolicy::production(1),
    )
    .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let artifact_catalog =
        ArtifactCatalog::open(config.data_dir().join("artifact-catalog.sqlite3"))
            .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let runtime = RuntimeService::new_with_providers(
        controller,
        sessions,
        approvals,
        RuntimeServiceScope::new(principal, tenant, workspace),
        providers,
    )
    .with_fleet(
        FleetEngine::open(config.data_dir().join("fleet.sqlite3"))
            .map_err(|error| CliError::internal(error.to_string(), json!({})))?,
        "pandora-service",
    )
    .map_err(|error| CliError::internal(error.to_string(), json!({})))?
    .with_orchestration(
        OrchestrationStore::open(config.data_dir().join("orchestration.sqlite3"))
            .map_err(|error| CliError::internal(error.to_string(), json!({})))?,
    )
    .with_evolution(Arc::new(evolution))
    .with_artifact_catalog(Arc::new(artifact_catalog))
    .with_evolution_control(config.data_dir());
    let Some(model) = config.provider_model() else {
        return Ok(runtime);
    };
    let credential_environment = config
        .active_provider()
        .and_then(|name| config.provider_profile(name))
        .map(|profile| profile.api_key_env())
        .or_else(|| config.provider_api_key_env());
    let credential_configured = credential_environment
        .map(|name| provider_credential_available(config, name))
        .transpose()?
        .unwrap_or(false);
    if !credential_configured || config.provider_url().is_none() {
        return Ok(runtime);
    }
    let provider = configured_provider_for(config, model, "desktop agent mode", None)?;
    runtime
        .with_agent(
            Arc::from(provider),
            SERVICE_AGENT_MAX_TURNS,
            SERVICE_AGENT_MAX_TOOL_CALLS,
            config.data_dir().join("context-cache.json"),
            active_skill_context(config)?,
        )
        .map_err(|error| CliError::internal(error.to_string(), json!({})))
}

fn prepare_service_identity(
    config: &RuntimeConfig,
    identities: &IdentityStore,
    parsed: &super::ParsedArgs,
) -> Result<(std::path::PathBuf, std::path::PathBuf, String), CliError> {
    match (parsed.value("token-file"), parsed.value("device-key-file")) {
        (Some(path), Some(key_path)) => {
            let token_path = std::path::PathBuf::from(path);
            let device_key_path = std::path::PathBuf::from(key_path);
            let device_key = DeviceKeyStore::load_or_create(&device_key_path)
                .map_err(|error| CliError::configuration(error.to_string(), json!({})))?;
            let device_id = device_key.device_id();
            let token = read_token(&token_path)?;
            if identities
                .authenticate(&token, &device_id)
                .map_err(|error| CliError::configuration(error.to_string(), json!({})))?
                .is_none()
            {
                return Err(CliError::configuration(
                    "service identity credential was rejected",
                    json!({"token_path": token_path}),
                ));
            }
            Ok((token_path, device_key_path, device_id))
        }
        (None, None) => prepare_default_identity(config, identities),
        _ => Err(CliError::usage(
            "service requires both '--token-file' and '--device-key-file' when either is supplied",
        )),
    }
}

fn prepare_default_identity(
    config: &RuntimeConfig,
    identities: &IdentityStore,
) -> Result<(std::path::PathBuf, std::path::PathBuf, String), CliError> {
    fs::create_dir_all(config.data_dir()).map_err(|_| {
        CliError::configuration("could not prepare Pandora data directory", json!({}))
    })?;
    let token_path = config.data_dir().join("service-token");
    let device_key_path = config.data_dir().join("service-device.key");
    let device_key = DeviceKeyStore::load_or_create(&device_key_path)
        .map_err(|error| CliError::configuration(error.to_string(), json!({})))?;
    let device_id = device_key.device_id();
    if token_path.is_file() {
        let token = read_token(&token_path)?;
        if identities
            .authenticate(&token, &device_id)
            .map_err(|error| CliError::configuration(error.to_string(), json!({})))?
            .is_some()
        {
            return Ok((token_path, device_key_path, device_id));
        }
    }
    if !identities
        .list()
        .map_err(|error| CliError::configuration(error.to_string(), json!({})))?
        .is_empty()
    {
        return Err(CliError::configuration(
            "default service identity is missing or revoked; supply an enrolled token and device",
            json!({"token_path": token_path}),
        ));
    }
    let (principal, tenant, workspace) = session_scope();
    let enrollment = identities
        .enroll(
            IdentityEnrollmentRequest::new(
                principal,
                tenant,
                workspace,
                AccessRole::Administrator,
                super::timestamp().as_unix_seconds(),
            ),
            &device_id,
            device_key.public_key(),
        )
        .map_err(|error| CliError::configuration(error.to_string(), json!({})))?;
    enrollment
        .write_token_file(&token_path)
        .map_err(|error| CliError::configuration(error.to_string(), json!({})))?;
    Ok((token_path, device_key_path, device_id))
}

fn read_token(path: &Path) -> Result<String, CliError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| {
        CliError::configuration("could not read identity credential", json!({"path": path}))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != 64 {
        return Err(CliError::configuration(
            "identity credential path is unsafe",
            json!({"path": path}),
        ));
    }
    fs::read_to_string(path)
        .map(|token| token.trim().to_owned())
        .map_err(|_| CliError::configuration("could not read identity credential", json!({})))
}

fn write_readiness(
    address: SocketAddr,
    token_path: &Path,
    device_key_path: &Path,
    device_id: &str,
) -> Result<(), CliError> {
    let readiness = json!({
        "endpoint": format!("http://{address}/v1/rpc"),
        "token_path": token_path,
        "device_key_path": device_key_path,
        "device_id": device_id,
    });
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, &readiness)
        .map_err(|_| CliError::internal("could not write local service readiness", json!({})))?;
    stdout
        .write_all(b"\n")
        .and_then(|()| stdout.flush())
        .map_err(|_| CliError::internal("could not write local service readiness", json!({})))
}
