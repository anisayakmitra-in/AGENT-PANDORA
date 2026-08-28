use super::provider::{configured_provider_for, provider_credential_available};
use super::run::{active_skill_context, configured_service_runtime};
use super::{load_config, parse_options, require_config_file, session_scope, session_store};
use crate::output::{CliError, CommandResult, already_printed};
use pandora_provider::Provider;
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
            Capability::NetworkConnect,
            Capability::ProviderInvoke,
            Capability::WasmExecute,
        ],
        [Operation::Write, Operation::Execute, Operation::Connect],
    );
    let (harnesses, wasm) = configured_service_runtime(config)?;
    let controller = ExecutionController::with_policy_and_harnesses(workspace, policy, harnesses)
        .with_wasm_executor(wasm);
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
    let mut agent_providers: Vec<Arc<dyn Provider>> = Vec::new();
    for name in config.provider_names() {
        let profile = config
            .provider_profile(&name)
            .expect("configured provider name should resolve");
        if !provider_credential_available(config, profile.api_key_env())? {
            continue;
        }
        agent_providers.push(Arc::from(configured_provider_for(
            config,
            profile.model(),
            "desktop agent mode",
            Some(&name),
        )?));
    }
    if agent_providers.is_empty() {
        return Ok(runtime);
    }
    let default_provider = config
        .active_provider()
        .filter(|active| {
            agent_providers
                .iter()
                .any(|provider| provider.manifest().id().as_str() == *active)
        })
        .map(str::to_owned)
        .unwrap_or_else(|| agent_providers[0].manifest().id().as_str().to_owned());
    runtime
        .with_agent_providers(
            agent_providers,
            default_provider,
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

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_runtime::PackageStore;
    use pandora_runtime::config::ConfigOverrides;
    use pandora_types::{
        GeneId, HarnessId, MetaComposition, PackageCompatibility, PackageDependency, PackageId,
        PackageKind, PackageManifest, ServiceRequest, ServiceResponse, ServiceRunRequest,
        ServiceRunResumeRequest, Timestamp, TrustEvidence, hash_artifact,
    };
    use std::collections::BTreeMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_config(root: &Path) -> RuntimeConfig {
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).expect("test workspace should exist");
        RuntimeConfig::from_sources(
            &ConfigOverrides::default(),
            &BTreeMap::new(),
            &root.join("config.json"),
            root.join("data"),
            workspace,
        )
        .expect("test runtime configuration should load")
    }

    fn package_manifest(
        id: &str,
        kind: PackageKind,
        artifact: &[u8],
        dependencies: Vec<PackageDependency>,
    ) -> PackageManifest {
        package_manifest_with_version(id, "1.0.0", kind, artifact, dependencies)
    }

    fn package_manifest_with_version(
        id: &str,
        version: &str,
        kind: PackageKind,
        artifact: &[u8],
        dependencies: Vec<PackageDependency>,
    ) -> PackageManifest {
        PackageManifest::new(
            id,
            version,
            kind,
            "test-publisher",
            hash_artifact(artifact),
            dependencies,
            PackageCompatibility::new(concat!("pandora>=", env!("CARGO_PKG_VERSION"))).unwrap(),
            "MIT",
            TrustEvidence::unsigned(),
        )
        .expect("test package manifest should be valid")
    }

    #[test]
    fn desktop_service_loads_enabled_package_compositions_and_executes_wasm() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be available")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "pandora-service-packages-{}-{suffix}",
            std::process::id()
        ));
        let config = test_config(&root);
        let store = PackageStore::open(config.data_dir().join("packages.sqlite3"))
            .expect("package store should open");
        let wasm = wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (func (export "pandora_alloc") (param i32) (result i32) i32.const 0)
                (func (export "pandora_run") (param i32 i32) (result i64)
                    local.get 0
                    i64.extend_i32_u
                    i64.const 32
                    i64.shl
                    local.get 1
                    i64.extend_i32_u
                    i64.or))"#,
        )
        .expect("WASM fixture should compile");
        let gene = package_manifest("example/service-echo", PackageKind::Gene, &wasm, Vec::new());
        store
            .admit(&gene, &gene, &wasm)
            .expect("WASM Gene should be admitted");

        let dependency = PackageDependency::new("example/service-echo", "1.0.0", false).unwrap();
        let primary_artifact = b"primary service domain";
        let primary = package_manifest(
            "example/service-domain",
            PackageKind::DomainHarness,
            primary_artifact,
            vec![dependency.clone()],
        );
        store
            .admit(&primary, &primary, primary_artifact)
            .expect("primary Domain Harness should be admitted");
        let shared_artifact = b"shared service domain";
        let shared = package_manifest(
            "example/shared-domain",
            PackageKind::DomainHarness,
            shared_artifact,
            vec![dependency],
        );
        store
            .admit(&shared, &shared, shared_artifact)
            .expect("shared Domain Harness should be admitted");
        let meta_artifact = b"service meta composition";
        let meta = PackageManifest::new_meta(
            "example/service-meta",
            "1.0.0",
            "test-publisher",
            hash_artifact(meta_artifact),
            Vec::new(),
            PackageCompatibility::new(concat!("pandora>=", env!("CARGO_PKG_VERSION"))).unwrap(),
            "MIT",
            TrustEvidence::unsigned(),
            MetaComposition::new(
                vec![
                    HarnessId::new("example/service-domain").unwrap(),
                    HarnessId::new("example/shared-domain").unwrap(),
                ],
                2,
            )
            .unwrap(),
        )
        .expect("Meta Harness manifest should be valid");
        store
            .admit(&meta, &meta, meta_artifact)
            .expect("Meta Harness should be admitted");

        for id in [
            "example/service-echo",
            "example/service-domain",
            "example/shared-domain",
            "example/service-meta",
        ] {
            store
                .enable(&PackageId::new(id).unwrap(), "1.0.0")
                .expect("package should enable in dependency order");
        }
        drop(store);

        let service = build_runtime_service(&config).expect("desktop runtime should start");
        let capabilities = service
            .handle(
                &ServiceRequest::capabilities(),
                Timestamp::from_unix_seconds(10),
            )
            .expect("capabilities should be available");
        let ServiceResponse::Capabilities { harnesses, .. } = capabilities else {
            panic!("service should return Harness capabilities");
        };
        for id in ["example/service-domain", "example/shared-domain"] {
            let harness = harnesses
                .iter()
                .find(|harness| harness.id().as_str() == id)
                .expect("enabled Domain Harness should be runtime visible");
            assert!(harness.runnable());
            assert_eq!(harness.version(), "1.0.0");
            assert_eq!(harness.gene_ids()[0].as_str(), "example/service-echo");
        }
        assert!(
            harnesses
                .iter()
                .find(|harness| harness.id().as_str() == "example/service-meta")
                .is_some_and(|harness| !harness.runnable())
        );

        let tools = service
            .handle(&ServiceRequest::tools(), Timestamp::from_unix_seconds(10))
            .expect("tools should be available");
        let ServiceResponse::Tools { tools, .. } = tools else {
            panic!("service should return tool capabilities");
        };
        let packaged_tool = tools
            .iter()
            .find(|tool| tool.name().contains("example/service-echo@1.0.0"))
            .expect("enabled WASM Gene should be exposed as an agent tool");
        assert!(
            packaged_tool
                .id()
                .as_str()
                .starts_with("package.example_service-echo.")
        );
        assert_eq!(packaged_tool.capability(), "wasm.execute");
        assert_eq!(packaged_tool.operation(), "execute");

        let task = r#"{"value":42}"#;
        let request = ServiceRunRequest::new(
            task,
            Some(HarnessId::new("example/service-domain").unwrap()),
            Some(GeneId::new("example/service-echo").unwrap()),
        )
        .expect("service run request should be valid");
        let first = service
            .handle(
                &ServiceRequest::run(request.clone()),
                Timestamp::from_unix_seconds(11),
            )
            .expect("WASM service run should reach approval");
        let ServiceResponse::Run { run, .. } = first else {
            panic!("service should return a run result");
        };
        assert_eq!(run.status(), "approval_required");
        let approval_id = run
            .approval()
            .expect("WASM execution should require exact approval")
            .approval_id()
            .to_owned();
        service
            .handle(
                &ServiceRequest::approval_resolve(&approval_id, true).unwrap(),
                Timestamp::from_unix_seconds(12),
            )
            .expect("exact approval should resolve");
        let resumed = service
            .handle(
                &ServiceRequest::run_resume(
                    ServiceRunResumeRequest::new(approval_id, request).unwrap(),
                ),
                Timestamp::from_unix_seconds(13),
            )
            .expect("approved WASM run should resume");
        let ServiceResponse::Run { run, .. } = resumed else {
            panic!("service should return the resumed run");
        };
        assert_eq!(run.status(), "completed");
        assert_eq!(run.output(), task);
        assert_eq!(run.receipt_count(), 1);

        drop(service);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn desktop_service_catalog_applies_exact_update_and_rollback_after_restart() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be available")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "pandora-service-rollback-{}-{suffix}",
            std::process::id()
        ));
        let config = test_config(&root);
        let package_id = PackageId::new("example/versioned-domain").unwrap();
        let dependency = PackageDependency::new("workspace.read", "0.1.0", false).unwrap();
        let first_artifact = b"versioned service domain one";
        let first = package_manifest_with_version(
            package_id.as_str(),
            "1.0.0",
            PackageKind::DomainHarness,
            first_artifact,
            vec![dependency.clone()],
        );
        let second_artifact = b"versioned service domain two";
        let second = package_manifest_with_version(
            package_id.as_str(),
            "2.0.0",
            PackageKind::DomainHarness,
            second_artifact,
            vec![dependency],
        );
        let store = PackageStore::open(config.data_dir().join("packages.sqlite3"))
            .expect("package store should open");
        store
            .admit(&first, &first, first_artifact)
            .expect("first Domain Harness version should be admitted");
        store
            .admit(&second, &second, second_artifact)
            .expect("second Domain Harness version should be admitted");
        store
            .enable(&package_id, "1.0.0")
            .expect("first Domain Harness version should enable");
        drop(store);

        for (step, expected) in ["1.0.0", "2.0.0", "1.0.0"].into_iter().enumerate() {
            let service = build_runtime_service(&config).expect("desktop runtime should restart");
            let capabilities = service
                .handle(
                    &ServiceRequest::capabilities(),
                    Timestamp::from_unix_seconds(20),
                )
                .expect("capabilities should be available");
            let ServiceResponse::Capabilities { harnesses, .. } = capabilities else {
                panic!("service should return Harness capabilities");
            };
            let loaded = harnesses
                .iter()
                .find(|harness| harness.id().as_str() == package_id.as_str())
                .expect("active exact Domain Harness should be runtime visible");
            assert_eq!(loaded.version(), expected);
            assert_eq!(loaded.gene_ids()[0].as_str(), "workspace.read");
            drop(service);

            let store = PackageStore::open(config.data_dir().join("packages.sqlite3"))
                .expect("package store should reopen");
            match step {
                0 => {
                    store
                        .enable(&package_id, "2.0.0")
                        .expect("second exact version should enable");
                }
                1 => {
                    let binding = store
                        .rollback(&package_id)
                        .expect("active package should roll back");
                    assert_eq!(binding.active_version(), Some("1.0.0"));
                    assert_eq!(binding.previous_version(), Some("2.0.0"));
                }
                _ => break,
            }
        }

        let _ = fs::remove_dir_all(root);
    }
}
