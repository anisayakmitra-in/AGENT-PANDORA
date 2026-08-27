use super::{load_config, parse_options, require_config_file, session_scope, session_store};
use crate::output::{CliError, CommandResult, already_printed};
use pandora_harnesses::HarnessCatalog;
use pandora_runtime::config::RuntimeConfig;
use pandora_runtime::executors::WorkspaceRoot;
use pandora_runtime::{
    ExecutionController, RuntimeService, RuntimeServiceScope, ServiceTokenStore,
};
use pandora_service::{LocalService, LocalServiceConfig};
use pandora_types::{Capability, Operation, PolicyContext, ServiceProviderSummary};
use serde_json::json;
use std::io::{self, Write};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "port"])?;
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
    let token_store = ServiceTokenStore::load_or_create(config.data_dir())
        .map_err(|_| CliError::internal("could not prepare the local service token", json!({})))?;
    let token_path = token_store.path().to_path_buf();
    let service = LocalService::new(
        LocalServiceConfig::new(
            SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
            runtime,
            token_store,
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
        write_readiness(bound.local_addr(), &token_path)?;
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
                std::env::var_os(profile.api_key_env()).is_some_and(|value| !value.is_empty()),
                profile.fallback_provider().map(str::to_owned),
            )
        })
        .collect();
    Ok(RuntimeService::new_with_providers(
        controller,
        sessions,
        RuntimeServiceScope::new(principal, tenant, workspace),
        providers,
    ))
}

fn write_readiness(address: SocketAddr, token_path: &Path) -> Result<(), CliError> {
    let readiness = json!({
        "endpoint": format!("http://{address}/v1/rpc"),
        "token_path": token_path,
    });
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, &readiness)
        .map_err(|_| CliError::internal("could not write local service readiness", json!({})))?;
    stdout
        .write_all(b"\n")
        .and_then(|()| stdout.flush())
        .map_err(|_| CliError::internal("could not write local service readiness", json!({})))
}
