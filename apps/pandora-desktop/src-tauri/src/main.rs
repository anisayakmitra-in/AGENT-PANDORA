#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use ed25519_dalek::{Signer, SigningKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use tauri::{Manager, State, WindowEvent};
use url::{Host, Url};
#[cfg(target_os = "macos")]
use window_vibrancy::{
    apply_liquid_glass, apply_vibrancy, LiquidGlassOptions, NSGlassEffectViewStyle,
    NSVisualEffectMaterial, NSVisualEffectState,
};
use zeroize::{Zeroize, Zeroizing};

const NATIVE_ENDPOINT: &str = "tauri://pandora";
const TOKEN_LENGTH: usize = 64;
const DEVICE_KEY_LENGTH: usize = 64;

#[derive(Default)]
struct ServiceState(Mutex<Option<RunningService>>);

struct RunningService {
    child: Child,
    endpoint: String,
    token: String,
    device_id: String,
    device_key: SigningKey,
}

#[derive(Deserialize)]
struct ServiceReadiness {
    endpoint: String,
    token_path: String,
    device_key_path: String,
    device_id: String,
}

#[derive(Serialize)]
struct NativeServiceStatus {
    endpoint: &'static str,
}

#[tauri::command]
fn start_local_service(state: State<'_, ServiceState>) -> Result<NativeServiceStatus, String> {
    let mut running = state
        .0
        .lock()
        .map_err(|_| "service state is unavailable".to_owned())?;
    if running.is_some() {
        return Ok(NativeServiceStatus {
            endpoint: NATIVE_ENDPOINT,
        });
    }

    let program = cli_program()?;
    let mut child = Command::new(program)
        .args(["service", "start"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "could not start the Pandora CLI service".to_owned())?;
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Pandora service did not expose readiness output".to_owned());
        }
    };
    let mut line = String::new();
    if BufReader::new(stdout).read_line(&mut line).is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return Err("could not read Pandora service readiness".to_owned());
    }
    let readiness: ServiceReadiness = match serde_json::from_str(&line) {
        Ok(readiness) => readiness,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Pandora service returned invalid readiness output".to_owned());
        }
    };
    if let Err(error) = validate_loopback_endpoint(&readiness.endpoint) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let token = match read_token(Path::new(&readiness.token_path)) {
        Ok(token) => token,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    if !is_valid_device_id(&readiness.device_id) {
        let _ = child.kill();
        let _ = child.wait();
        return Err("Pandora service returned an invalid device identity".to_owned());
    }
    let device_key = match read_device_key(Path::new(&readiness.device_key_path)) {
        Ok(key) => key,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    if device_id_for_key(&device_key) != readiness.device_id {
        let _ = child.kill();
        let _ = child.wait();
        return Err("Pandora service device key does not match its identity".to_owned());
    }
    *running = Some(RunningService {
        child,
        endpoint: readiness.endpoint,
        token,
        device_id: readiness.device_id,
        device_key,
    });
    Ok(NativeServiceStatus {
        endpoint: NATIVE_ENDPOINT,
    })
}

#[tauri::command]
fn stop_local_service(state: State<'_, ServiceState>) -> Result<(), String> {
    let mut running = state
        .0
        .lock()
        .map_err(|_| "service state is unavailable".to_owned())?;
    if let Some(mut service) = running.take() {
        let _ = service.child.kill();
        let _ = service.child.wait();
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderConfiguration {
    name: String,
    protocol: String,
    base_url: String,
    model: String,
    api_key_environment: String,
    api_key: String,
}

#[derive(Deserialize)]
struct ProviderIdentity {
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct McpConfiguration {
    server_id: String,
    program: String,
    arguments_json: String,
    mode: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeConfigurationResult {
    message: String,
    restart_required: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegistryConfiguration {
    name: String,
    base_url: String,
    token_environment: String,
    token: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeRegistryResult {
    message: String,
    data: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegistryPackageInstall {
    package_id: String,
    version: String,
    registry_profile: String,
    registry_url: String,
    token: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitHubPackageInstall {
    repository_url: String,
    commit: String,
    manifest_path: String,
    artifact_path: String,
    token: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalPackageAdmission {
    manifest_path: String,
    artifact_path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackageIdentity {
    package_id: String,
    version: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackageRemoval {
    package_id: String,
    version: String,
    confirmation: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackageRollback {
    package_id: String,
    confirmation: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NativePackageResult {
    message: String,
    restart_required: bool,
    data: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalSkillInstall {
    source_path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillMutation {
    skill_id: String,
    action: String,
    confirmation: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeSkillResult {
    message: String,
    restart_required: bool,
    data: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MemoryScopeInput {
    session_id: String,
    provider: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MemoryIdentity {
    session_id: String,
    provider: String,
    memory_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MemoryForget {
    session_id: String,
    provider: String,
    memory_id: String,
    confirmation: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MemoryCompactionScope {
    session_id: String,
    provider: String,
    revoked_before_or_at: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MemoryCompaction {
    session_id: String,
    provider: String,
    revoked_before_or_at: u64,
    confirmation: String,
}

#[derive(Serialize)]
struct NativeMemoryResult {
    message: String,
    data: Value,
}

#[derive(Serialize)]
struct NativeStorageLifecycleResult {
    message: String,
    data: Value,
}

#[derive(Serialize)]
struct NativeFleetOperationsResult {
    message: String,
    data: Value,
}

#[tauri::command]
fn configure_provider(
    mut input: ProviderConfiguration,
) -> Result<NativeConfigurationResult, String> {
    let mut api_key = Zeroizing::new(std::mem::take(&mut input.api_key));
    validate_identifier(&input.name, "provider profile")?;
    if !matches!(
        input.protocol.as_str(),
        "open_ai_compatible" | "anthropic_messages" | "gemini_generate_content"
    ) {
        return Err("provider protocol is unsupported".to_owned());
    }
    validate_provider_url(&input.base_url)?;
    validate_text_field(&input.model, "provider model", 256)?;
    validate_environment_name(&input.api_key_environment)?;
    if api_key.len() >= 64 * 1024 {
        return Err("API key exceeds Pandora's secret size limit".to_owned());
    }

    if !api_key.is_empty() {
        let secret_args = vec![
            "secret".to_owned(),
            "set".to_owned(),
            input.api_key_environment.clone(),
            "--value-stdin".to_owned(),
            "--json".to_owned(),
        ];
        run_cli_with_secret(&secret_args, &mut api_key, "storing the encrypted API key")?;
    }

    let provider_args = vec![
        "provider".to_owned(),
        "set".to_owned(),
        "--name".to_owned(),
        input.name.clone(),
        "--protocol".to_owned(),
        input.protocol,
        "--provider-url".to_owned(),
        input.base_url,
        "--model".to_owned(),
        input.model,
        "--api-key-env".to_owned(),
        input.api_key_environment,
        "--json".to_owned(),
    ];
    run_cli(&provider_args, "saving the provider profile")?;
    Ok(NativeConfigurationResult {
        message: format!("Provider {} configured.", input.name),
        restart_required: true,
    })
}

#[tauri::command]
fn activate_provider(input: ProviderIdentity) -> Result<NativeConfigurationResult, String> {
    validate_identifier(&input.name, "provider profile")?;
    let args = vec![
        "provider".to_owned(),
        "use".to_owned(),
        input.name.clone(),
        "--json".to_owned(),
    ];
    run_cli(&args, "activating the provider profile")?;
    Ok(NativeConfigurationResult {
        message: format!("Provider {} selected.", input.name),
        restart_required: true,
    })
}

#[tauri::command]
fn configure_mcp(input: McpConfiguration) -> Result<NativeConfigurationResult, String> {
    validate_identifier(&input.server_id, "MCP server")?;
    if !matches!(input.mode.as_str(), "auto" | "modern-only" | "legacy-only") {
        return Err("MCP protocol mode is unsupported".to_owned());
    }
    let program = Path::new(&input.program);
    if !program.is_absolute() {
        return Err("MCP program path must be absolute".to_owned());
    }
    let metadata = std::fs::symlink_metadata(program)
        .map_err(|_| "MCP program path does not exist or is not readable".to_owned())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("MCP program path must identify a regular file, not a symlink".to_owned());
    }
    let arguments: Vec<String> = serde_json::from_str(&input.arguments_json)
        .map_err(|_| "MCP arguments must be a JSON array of strings".to_owned())?;
    if arguments.len() > 64 || arguments.iter().any(|argument| argument.len() > 4096) {
        return Err("MCP arguments exceed the local configuration limit".to_owned());
    }

    let mcp_args = vec![
        "mcp".to_owned(),
        "set".to_owned(),
        input.server_id.clone(),
        "--program".to_owned(),
        input.program,
        "--arguments-json".to_owned(),
        serde_json::to_string(&arguments)
            .map_err(|_| "could not encode MCP arguments".to_owned())?,
        "--mode".to_owned(),
        input.mode,
        "--json".to_owned(),
    ];
    run_cli(&mcp_args, "saving the MCP server")?;
    Ok(NativeConfigurationResult {
        message: format!("MCP server {} configured.", input.server_id),
        restart_required: true,
    })
}

#[tauri::command]
fn list_registry_profiles() -> Result<NativeRegistryResult, String> {
    let args = vec![
        "registry".to_owned(),
        "list".to_owned(),
        "--json".to_owned(),
    ];
    let data = run_cli_json(&args, "listing registry profiles")?;
    let count = data
        .get("registries")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or_default();
    Ok(NativeRegistryResult {
        message: format!("{count} registry profile(s) configured."),
        data,
    })
}

#[tauri::command]
fn configure_registry_profile(
    mut input: RegistryConfiguration,
) -> Result<NativeConfigurationResult, String> {
    validate_identifier(&input.name, "registry profile")?;
    validate_registry_url(&input.base_url)?;
    if !input.token_environment.is_empty() {
        validate_environment_name(&input.token_environment)?;
    }
    let mut token = Zeroizing::new(std::mem::take(&mut input.token));
    if token.len() >= 64 * 1024 || token.contains('\0') {
        return Err("registry token exceeds Pandora's secret size limit".to_owned());
    }
    if !token.is_empty() && input.token_environment.is_empty() {
        return Err("a registry token requires a secret reference".to_owned());
    }
    if !token.is_empty() {
        let secret_args = vec![
            "secret".to_owned(),
            "set".to_owned(),
            input.token_environment.clone(),
            "--value-stdin".to_owned(),
            "--json".to_owned(),
        ];
        run_cli_with_secret(
            &secret_args,
            &mut token,
            "storing the encrypted registry token",
        )?;
    }
    let mut args = vec![
        "registry".to_owned(),
        "set".to_owned(),
        "--name".to_owned(),
        input.name.clone(),
        "--registry-url".to_owned(),
        input.base_url,
    ];
    if !input.token_environment.is_empty() {
        args.extend(["--token-env".to_owned(), input.token_environment]);
    }
    args.push("--json".to_owned());
    run_cli(&args, "saving the registry profile")?;
    Ok(NativeConfigurationResult {
        message: format!("Registry {} configured.", input.name),
        restart_required: false,
    })
}

#[tauri::command]
fn list_local_packages() -> Result<NativePackageResult, String> {
    let args = vec!["package".to_owned(), "list".to_owned(), "--json".to_owned()];
    let data = run_cli_json(&args, "listing local packages")?;
    Ok(NativePackageResult {
        message: package_count_message(&data),
        restart_required: false,
        data,
    })
}

#[tauri::command]
fn list_package_transparency() -> Result<NativePackageResult, String> {
    let args = vec![
        "package".to_owned(),
        "transparency".to_owned(),
        "list".to_owned(),
        "--limit".to_owned(),
        "64".to_owned(),
        "--json".to_owned(),
    ];
    let data = run_cli_json(&args, "loading package trust transparency evidence")?;
    let count = data
        .get("count")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    Ok(NativePackageResult {
        message: format!("Loaded {count} append-only package transparency event(s)."),
        restart_required: false,
        data,
    })
}

#[tauri::command]
fn fleet_operations_dashboard() -> Result<NativeFleetOperationsResult, String> {
    let args = vec![
        "fleet".to_owned(),
        "dashboard".to_owned(),
        "--stale-after".to_owned(),
        "30".to_owned(),
        "--json".to_owned(),
    ];
    let data = run_cli_json(&args, "loading the local Fleet operations snapshot")?;
    let health = data
        .get("health")
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    Ok(NativeFleetOperationsResult {
        message: format!("Local Fleet operations are {health}."),
        data,
    })
}

#[tauri::command]
fn list_local_skills() -> Result<NativeSkillResult, String> {
    let args = vec!["skill".to_owned(), "list".to_owned(), "--json".to_owned()];
    let data = run_cli_json(&args, "listing local skills")?;
    Ok(NativeSkillResult {
        message: "Loaded local Skills.".to_owned(),
        restart_required: false,
        data,
    })
}

#[tauri::command]
fn install_local_skill(input: LocalSkillInstall) -> Result<NativeSkillResult, String> {
    validate_text_field(&input.source_path, "Skill source", 4096)?;
    let source = validate_local_skill_directory(Path::new(&input.source_path))?;
    let args = vec![
        "skill".to_owned(),
        "install".to_owned(),
        source.to_string_lossy().into_owned(),
        "--json".to_owned(),
    ];
    let data = run_cli_json(&args, "installing the local Skill")?;
    Ok(NativeSkillResult {
        message: "Skill installed disabled.".to_owned(),
        restart_required: true,
        data,
    })
}

#[tauri::command]
fn mutate_local_skill(input: SkillMutation) -> Result<NativeSkillResult, String> {
    validate_skill_mutation(&input)?;
    let mut args = vec![
        "skill".to_owned(),
        input.action.clone(),
        input.skill_id.clone(),
    ];
    if input.action == "remove" {
        args.push("--yes".to_owned());
    }
    args.push("--json".to_owned());
    let data = run_cli_json(&args, "changing the local Skill lifecycle")?;
    let state = match input.action.as_str() {
        "restore" => "restored disabled",
        "remove" => "removed and retained for restore",
        action => action,
    };
    Ok(NativeSkillResult {
        message: format!("Skill {} is now {state}.", input.skill_id),
        restart_required: true,
        data,
    })
}

#[tauri::command]
fn install_registry_package(
    mut input: RegistryPackageInstall,
) -> Result<NativePackageResult, String> {
    validate_package_id(&input.package_id)?;
    let version = optional_package_version(&input.version)?;
    if input.registry_profile.is_empty() {
        validate_registry_url(&input.registry_url)?;
    } else {
        validate_identifier(&input.registry_profile, "registry profile")?;
        if !input.registry_url.is_empty() {
            return Err("choose a registry profile or a registry URL, not both".to_owned());
        }
    }
    let mut token = Zeroizing::new(std::mem::take(&mut input.token));
    if token.len() >= 64 * 1024 || token.contains('\0') {
        return Err("registry token exceeds Pandora's secret size limit".to_owned());
    }

    let mut args = vec![
        "package".to_owned(),
        "install".to_owned(),
        input.package_id.clone(),
    ];
    if let Some(version) = version {
        args.push(version);
    }
    if input.registry_profile.is_empty() {
        args.extend(["--registry".to_owned(), input.registry_url]);
    } else {
        args.extend(["--registry-profile".to_owned(), input.registry_profile]);
    }
    args.push("--json".to_owned());
    let data = if token.is_empty() {
        run_cli_json(&args, "installing the registry package")?
    } else {
        const TOKEN_ENV: &str = "PANDORA_DESKTOP_REGISTRY_TOKEN";
        args.extend(["--token-env".to_owned(), TOKEN_ENV.to_owned()]);
        run_cli_json_with_secret_environment(
            &args,
            TOKEN_ENV,
            &mut token,
            "installing the registry package",
        )?
    };
    token.zeroize();
    Ok(NativePackageResult {
        message: format!("Package {} admitted from the registry.", input.package_id),
        restart_required: true,
        data,
    })
}

#[tauri::command]
fn install_github_package(mut input: GitHubPackageInstall) -> Result<NativePackageResult, String> {
    validate_github_repository_url(&input.repository_url)?;
    validate_github_commit(&input.commit)?;
    validate_github_repository_path(&input.manifest_path, "manifest")?;
    validate_github_repository_path(&input.artifact_path, "artifact")?;
    if input.manifest_path == input.artifact_path {
        return Err("GitHub manifest and artifact paths must be different".to_owned());
    }
    let mut token = Zeroizing::new(std::mem::take(&mut input.token));
    if token.len() >= 64 * 1024 || token.contains('\0') {
        return Err("GitHub token exceeds Pandora's secret size limit".to_owned());
    }
    let mut args = vec![
        "package".to_owned(),
        "install-github".to_owned(),
        "--repository".to_owned(),
        input.repository_url,
        "--commit".to_owned(),
        input.commit,
        "--manifest".to_owned(),
        input.manifest_path,
        "--artifact".to_owned(),
        input.artifact_path,
        "--json".to_owned(),
    ];
    let data = if token.is_empty() {
        run_cli_json(&args, "installing the GitHub package")?
    } else {
        const TOKEN_ENV: &str = "PANDORA_DESKTOP_GITHUB_TOKEN";
        args.extend(["--token-env".to_owned(), TOKEN_ENV.to_owned()]);
        run_cli_json_with_secret_environment(
            &args,
            TOKEN_ENV,
            &mut token,
            "installing the GitHub package",
        )?
    };
    token.zeroize();
    Ok(NativePackageResult {
        message: "Package admitted from the pinned GitHub source.".to_owned(),
        restart_required: true,
        data,
    })
}

#[tauri::command]
fn admit_local_package(input: LocalPackageAdmission) -> Result<NativePackageResult, String> {
    let manifest = validate_regular_absolute_path(&input.manifest_path, "package manifest")?;
    let artifact = validate_regular_absolute_path(&input.artifact_path, "package artifact")?;
    let args = vec![
        "package".to_owned(),
        "admit".to_owned(),
        "--manifest".to_owned(),
        manifest.to_string_lossy().into_owned(),
        "--artifact".to_owned(),
        artifact.to_string_lossy().into_owned(),
        "--json".to_owned(),
    ];
    let data = run_cli_json(&args, "admitting the local package")?;
    Ok(NativePackageResult {
        message: "Local package admitted after manifest, artifact, dependency, and trust checks."
            .to_owned(),
        restart_required: true,
        data,
    })
}

#[tauri::command]
fn preview_package_removal(input: PackageIdentity) -> Result<NativePackageResult, String> {
    validate_package_identity(&input.package_id, &input.version)?;
    let args = vec![
        "package".to_owned(),
        "remove".to_owned(),
        input.package_id.clone(),
        input.version.clone(),
        "--dry-run".to_owned(),
        "--json".to_owned(),
    ];
    let data = run_cli_json(&args, "previewing package removal")?;
    Ok(NativePackageResult {
        message: format!(
            "Removal preview recorded for {}@{}; no package changed.",
            input.package_id, input.version
        ),
        restart_required: false,
        data,
    })
}

#[tauri::command]
fn remove_local_package(input: PackageRemoval) -> Result<NativePackageResult, String> {
    validate_package_identity(&input.package_id, &input.version)?;
    let expected = format!("{}@{}", input.package_id, input.version);
    if input.confirmation != expected {
        return Err(format!(
            "type {expected} to confirm this exact package removal"
        ));
    }
    let args = vec![
        "package".to_owned(),
        "remove".to_owned(),
        input.package_id.clone(),
        input.version.clone(),
        "--yes".to_owned(),
        "--json".to_owned(),
    ];
    let data = run_cli_json(&args, "removing the local package")?;
    Ok(NativePackageResult {
        message: format!("Package {expected} removed after dependency and binding checks."),
        restart_required: true,
        data,
    })
}

fn package_lifecycle_command(
    operation: &str,
    input: PackageIdentity,
    confirmed: bool,
) -> Result<NativePackageResult, String> {
    validate_package_identity(&input.package_id, &input.version)?;
    let mut args = vec![
        "package".to_owned(),
        operation.to_owned(),
        input.package_id.clone(),
        input.version.clone(),
    ];
    args.push(if confirmed { "--yes" } else { "--dry-run" }.to_owned());
    args.push("--json".to_owned());
    let data = run_cli_json(&args, &format!("{operation} package lifecycle"))?;
    Ok(NativePackageResult {
        message: if confirmed {
            format!(
                "Package {}@{} {} without changing Pandora's constitutional authority.",
                input.package_id,
                input.version,
                if operation == "enable" {
                    "enabled"
                } else {
                    "disabled"
                }
            )
        } else {
            format!(
                "{} preview recorded for {}@{}; no lifecycle binding changed.",
                if operation == "enable" {
                    "Activation"
                } else {
                    "Disable"
                },
                input.package_id,
                input.version
            )
        },
        restart_required: confirmed,
        data,
    })
}

#[tauri::command]
fn preview_package_enable(input: PackageIdentity) -> Result<NativePackageResult, String> {
    package_lifecycle_command("enable", input, false)
}

#[tauri::command]
fn enable_local_package(input: PackageRemoval) -> Result<NativePackageResult, String> {
    let expected = format!("{}@{}", input.package_id, input.version);
    if input.confirmation != expected {
        return Err(format!("type {expected} to confirm this exact activation"));
    }
    package_lifecycle_command(
        "enable",
        PackageIdentity {
            package_id: input.package_id,
            version: input.version,
        },
        true,
    )
}

#[tauri::command]
fn preview_package_disable(input: PackageIdentity) -> Result<NativePackageResult, String> {
    package_lifecycle_command("disable", input, false)
}

#[tauri::command]
fn disable_local_package(input: PackageRemoval) -> Result<NativePackageResult, String> {
    let expected = format!("{}@{}", input.package_id, input.version);
    if input.confirmation != expected {
        return Err(format!("type {expected} to confirm this exact disable"));
    }
    package_lifecycle_command(
        "disable",
        PackageIdentity {
            package_id: input.package_id,
            version: input.version,
        },
        true,
    )
}

#[tauri::command]
fn preview_package_rollback(input: PackageRollback) -> Result<NativePackageResult, String> {
    validate_package_id(&input.package_id)?;
    let args = vec![
        "package".to_owned(),
        "rollback".to_owned(),
        input.package_id.clone(),
        "--dry-run".to_owned(),
        "--json".to_owned(),
    ];
    let data = run_cli_json(&args, "previewing the package rollback")?;
    Ok(NativePackageResult {
        message: format!(
            "Rollback preview recorded for {}; no lifecycle binding changed.",
            input.package_id
        ),
        restart_required: false,
        data,
    })
}

#[tauri::command]
fn rollback_local_package(input: PackageRollback) -> Result<NativePackageResult, String> {
    validate_package_id(&input.package_id)?;
    if input.confirmation != input.package_id {
        return Err(format!(
            "type {} to confirm rollback to the retained exact version",
            input.package_id
        ));
    }
    let args = vec![
        "package".to_owned(),
        "rollback".to_owned(),
        input.package_id.clone(),
        "--yes".to_owned(),
        "--json".to_owned(),
    ];
    let data = run_cli_json(&args, "rolling back the package binding")?;
    Ok(NativePackageResult {
        message: format!(
            "Package {} restored to its retained exact version.",
            input.package_id
        ),
        restart_required: true,
        data,
    })
}

#[tauri::command]
fn lock_local_packages() -> Result<NativePackageResult, String> {
    let args = vec!["package".to_owned(), "lock".to_owned(), "--json".to_owned()];
    let data = run_cli_json(&args, "writing the package lock")?;
    Ok(NativePackageResult {
        message: "Deterministic package lock written for the current workspace.".to_owned(),
        restart_required: false,
        data,
    })
}

fn validate_memory_scope(session_id: &str, provider: &str) -> Result<(), String> {
    validate_identifier(session_id, "memory session")?;
    validate_identifier(provider, "memory provider")
}

fn validate_memory_id(memory_id: &str) -> Result<(), String> {
    validate_text_field(memory_id, "memory ID", 256)?;
    if memory_id.trim() != memory_id || memory_id.bytes().any(|byte| byte.is_ascii_whitespace()) {
        return Err("memory ID must be one bounded token without whitespace".to_owned());
    }
    Ok(())
}

fn validate_memory_forget(input: &MemoryForget) -> Result<(), String> {
    validate_memory_scope(&input.session_id, &input.provider)?;
    validate_memory_id(&input.memory_id)?;
    if input.confirmation != input.memory_id {
        return Err(format!(
            "type {} to confirm this exact memory revocation",
            input.memory_id
        ));
    }
    Ok(())
}

fn validate_memory_compaction(input: &MemoryCompaction) -> Result<(), String> {
    validate_memory_scope(&input.session_id, &input.provider)?;
    let expected = format!("COMPACT {}", input.revoked_before_or_at);
    if input.confirmation != expected {
        return Err(format!(
            "type {expected} to confirm logical compaction for this exact retention boundary"
        ));
    }
    Ok(())
}

#[tauri::command]
fn inspect_memory_audit(input: MemoryScopeInput) -> Result<NativeMemoryResult, String> {
    validate_memory_scope(&input.session_id, &input.provider)?;
    let args = vec![
        "memory".to_owned(),
        "audit".to_owned(),
        "--session".to_owned(),
        input.session_id,
        "--provider".to_owned(),
        input.provider,
        "--json".to_owned(),
    ];
    let data = run_cli_json(&args, "inspecting memory audit evidence")?;
    let count = data
        .get("count")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    Ok(NativeMemoryResult {
        message: format!("Loaded {count} durable memory audit record(s)."),
        data,
    })
}

#[tauri::command]
fn inspect_memory_provenance(input: MemoryIdentity) -> Result<NativeMemoryResult, String> {
    validate_memory_scope(&input.session_id, &input.provider)?;
    validate_memory_id(&input.memory_id)?;
    let memory_id = input.memory_id.clone();
    let args = vec![
        "memory".to_owned(),
        "provenance".to_owned(),
        "--session".to_owned(),
        input.session_id,
        "--provider".to_owned(),
        input.provider,
        input.memory_id,
        "--json".to_owned(),
    ];
    let data = run_cli_json(&args, "inspecting memory provenance")?;
    Ok(NativeMemoryResult {
        message: format!("Loaded bounded provenance for {memory_id}."),
        data,
    })
}

#[tauri::command]
fn preview_memory_forget(input: MemoryIdentity) -> Result<NativeMemoryResult, String> {
    validate_memory_scope(&input.session_id, &input.provider)?;
    validate_memory_id(&input.memory_id)?;
    let memory_id = input.memory_id.clone();
    let args = vec![
        "memory".to_owned(),
        "forget".to_owned(),
        "--session".to_owned(),
        input.session_id,
        "--provider".to_owned(),
        input.provider,
        input.memory_id,
        "--json".to_owned(),
    ];
    let data = run_cli_json(&args, "previewing memory revocation")?;
    Ok(NativeMemoryResult {
        message: format!("Previewed durable revocation for {memory_id}; no memory changed."),
        data,
    })
}

#[tauri::command]
fn forget_memory(input: MemoryForget) -> Result<NativeMemoryResult, String> {
    validate_memory_forget(&input)?;
    let memory_id = input.memory_id.clone();
    let args = vec![
        "memory".to_owned(),
        "forget".to_owned(),
        "--session".to_owned(),
        input.session_id,
        "--provider".to_owned(),
        input.provider,
        input.memory_id,
        "--yes".to_owned(),
        "--json".to_owned(),
    ];
    let data = run_cli_json(&args, "revoking memory")?;
    Ok(NativeMemoryResult {
        message: format!("Memory {memory_id} revoked with a durable tombstone."),
        data,
    })
}

#[tauri::command]
fn preview_memory_compaction(input: MemoryCompactionScope) -> Result<NativeMemoryResult, String> {
    validate_memory_scope(&input.session_id, &input.provider)?;
    let boundary = input.revoked_before_or_at;
    let args = vec![
        "memory".to_owned(),
        "compact".to_owned(),
        "--session".to_owned(),
        input.session_id,
        "--provider".to_owned(),
        input.provider,
        "--before".to_owned(),
        boundary.to_string(),
        "--json".to_owned(),
    ];
    let data = run_cli_json(&args, "previewing memory retention compaction")?;
    let count = data
        .get("compactable_records")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    Ok(NativeMemoryResult {
        message: format!(
            "Previewed {count} revoked logical memory record(s) at or before {boundary}; no records changed."
        ),
        data,
    })
}

#[tauri::command]
fn compact_memory(input: MemoryCompaction) -> Result<NativeMemoryResult, String> {
    validate_memory_compaction(&input)?;
    let boundary = input.revoked_before_or_at;
    let args = vec![
        "memory".to_owned(),
        "compact".to_owned(),
        "--session".to_owned(),
        input.session_id,
        "--provider".to_owned(),
        input.provider,
        "--before".to_owned(),
        boundary.to_string(),
        "--yes".to_owned(),
        "--json".to_owned(),
    ];
    let data = run_cli_json(&args, "compacting revoked logical memory records")?;
    let count = data
        .get("compacted_records")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    Ok(NativeMemoryResult {
        message: format!(
            "Compacted {count} revoked logical memory record(s); tombstones and audit evidence remain."
        ),
        data,
    })
}

#[tauri::command]
fn list_storage_lifecycle_evidence() -> Result<NativeStorageLifecycleResult, String> {
    let args = vec![
        "backup".to_owned(),
        "lifecycle".to_owned(),
        "list".to_owned(),
        "--limit".to_owned(),
        "64".to_owned(),
        "--json".to_owned(),
    ];
    let data = run_cli_json(&args, "loading storage lifecycle evidence")?;
    let count = data
        .get("count")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    Ok(NativeStorageLifecycleResult {
        message: format!("Loaded {count} append-only storage lifecycle receipt(s)."),
        data,
    })
}

fn validate_identifier(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        || !value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
    {
        return Err(format!(
            "{label} ID must start with a letter or number and contain only letters, numbers, dots, underscores, or hyphens"
        ));
    }
    Ok(())
}

fn validate_text_field(value: &str, label: &str, maximum: usize) -> Result<(), String> {
    if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        return Err(format!("{label} is empty or invalid"));
    }
    Ok(())
}

fn validate_environment_name(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_uppercase() || *byte == b'_')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(
            "secret reference must be an uppercase environment name such as PANDORA_CUSTOM_API_KEY"
                .to_owned(),
        );
    }
    Ok(())
}

fn validate_package_id(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 256
        || value.split('/').any(|part| {
            part.is_empty()
                || matches!(part, "." | "..")
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        })
    {
        return Err(
            "package ID must contain bounded slash-separated names without path traversal"
                .to_owned(),
        );
    }
    Ok(())
}

fn optional_package_version(value: &str) -> Result<Option<String>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    validate_text_field(value, "package version", 128)?;
    if value.bytes().any(|byte| byte.is_ascii_whitespace()) {
        return Err("package version must be one exact SemVer value".to_owned());
    }
    Ok(Some(value.to_owned()))
}

fn validate_package_identity(id: &str, version: &str) -> Result<(), String> {
    validate_package_id(id)?;
    if optional_package_version(version)?.is_none() {
        return Err("package version is required".to_owned());
    }
    Ok(())
}

fn validate_registry_url(value: &str) -> Result<(), String> {
    validate_secure_url(value, "registry")
}

fn validate_github_repository_url(value: &str) -> Result<(), String> {
    if value.len() > 2048 {
        return Err("GitHub repository URL exceeds the local limit".to_owned());
    }
    let url = Url::parse(value).map_err(|_| "GitHub repository URL is invalid".to_owned())?;
    if url.scheme() != "https"
        || !url
            .host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case("github.com"))
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("GitHub repository must be https://github.com/<owner>/<repository>".to_owned());
    }
    let parts = url.path().trim_matches('/').split('/').collect::<Vec<_>>();
    if parts.len() != 2 {
        return Err("GitHub repository must be https://github.com/<owner>/<repository>".to_owned());
    }
    let repository = parts[1].strip_suffix(".git").unwrap_or(parts[1]);
    let owner_valid = !parts[0].is_empty()
        && parts[0].len() <= 39
        && parts[0]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    let repository_valid = !repository.is_empty()
        && repository.len() <= 100
        && !matches!(repository, "." | "..")
        && repository
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if !owner_valid || !repository_valid {
        return Err("GitHub owner or repository name is invalid".to_owned());
    }
    Ok(())
}

fn validate_github_commit(value: &str) -> Result<(), String> {
    if value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("GitHub source requires one full 40-character commit SHA".to_owned());
    }
    Ok(())
}

fn validate_github_repository_path(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 1024
        || value.starts_with('/')
        || value.ends_with('/')
        || value.contains('\\')
        || value.chars().any(char::is_control)
        || value.split('/').any(|segment| {
            segment.is_empty()
                || matches!(segment, "." | "..")
                || !segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        })
    {
        return Err(format!(
            "GitHub {label} path must be a bounded repository-relative file path"
        ));
    }
    Ok(())
}

fn validate_provider_url(value: &str) -> Result<(), String> {
    validate_secure_url(value, "provider")
}

fn validate_secure_url(value: &str, label: &str) -> Result<(), String> {
    if value.len() > 2048 {
        return Err(format!("{label} URL exceeds the local configuration limit"));
    }
    let url = Url::parse(value).map_err(|_| format!("{label} URL is invalid"))?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(format!(
            "{label} URL must not contain credentials, query data, or a fragment"
        ));
    }
    let loopback = match url.host() {
        Some(Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    };
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        return Err(format!(
            "{label} URL must use HTTPS; HTTP is allowed only for loopback services"
        ));
    }
    Ok(())
}

fn validate_regular_absolute_path(value: &str, label: &str) -> Result<std::path::PathBuf, String> {
    if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(format!("{label} path is empty or invalid"));
    }
    let path = std::path::PathBuf::from(value);
    if !path.is_absolute() {
        return Err(format!("{label} path must be absolute"));
    }
    let metadata = std::fs::symlink_metadata(&path)
        .map_err(|_| format!("{label} path does not exist or is not readable"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("{label} must be a regular file, not a symlink"));
    }
    Ok(path)
}

fn validate_local_skill_directory(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("Skill source must be an absolute directory path".to_owned());
    }
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| "Skill source does not exist or is not readable".to_owned())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("Skill source must be a regular directory, not a symlink".to_owned());
    }
    std::fs::canonicalize(path).map_err(|_| "Skill source could not be resolved".to_owned())
}

fn validate_skill_mutation(input: &SkillMutation) -> Result<(), String> {
    validate_identifier(&input.skill_id, "Skill")?;
    if !matches!(
        input.action.as_str(),
        "enable" | "disable" | "suspend" | "remove" | "restore"
    ) {
        return Err("Skill lifecycle action is unsupported".to_owned());
    }
    if input.action == "remove" {
        if input.confirmation != input.skill_id {
            return Err("Skill removal confirmation must match the exact Skill ID".to_owned());
        }
    } else if !input.confirmation.is_empty() {
        return Err("Skill confirmation is accepted only for removal".to_owned());
    }
    Ok(())
}

fn cli_binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "pandora.exe"
    } else {
        "pandora"
    }
}

fn validate_cli_program_path(path: &Path, label: &str) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!("{label} must be an absolute path"));
    }
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| format!("{label} does not exist or is not readable"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "{label} must identify a regular file, not a symlink"
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(format!("{label} is not executable"));
        }
    }
    Ok(path.to_path_buf())
}

fn bundled_cli_program() -> Result<PathBuf, String> {
    let executable = std::env::current_exe()
        .map_err(|_| "could not resolve the Pandora desktop executable".to_owned())?;
    let directory = executable
        .parent()
        .ok_or_else(|| "Pandora desktop executable has no parent directory".to_owned())?;
    Ok(directory.join(cli_binary_name()))
}

fn cli_program() -> Result<std::ffi::OsString, String> {
    if let Some(override_path) = std::env::var_os("PANDORA_CLI_PATH") {
        let path = PathBuf::from(override_path);
        return validate_cli_program_path(&path, "PANDORA_CLI_PATH").map(PathBuf::into_os_string);
    }
    let bundled = bundled_cli_program()?;
    match validate_cli_program_path(&bundled, "bundled Pandora CLI") {
        Ok(path) => Ok(path.into_os_string()),
        Err(_) if cfg!(debug_assertions) => Ok("pandora".into()),
        Err(error) => Err(error),
    }
}

fn run_cli(args: &[String], action: &str) -> Result<(), String> {
    let output = Command::new(cli_program()?)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|_| format!("could not launch the Pandora CLI while {action}"))?;
    validate_cli_output(output, action)
}

fn run_cli_json(args: &[String], action: &str) -> Result<Value, String> {
    let output = Command::new(cli_program()?)
        .args(args)
        .env_remove("PANDORA_REGISTRY_TOKEN")
        .env_remove("PANDORA_DESKTOP_REGISTRY_TOKEN")
        .env_remove("PANDORA_GITHUB_TOKEN")
        .env_remove("PANDORA_DESKTOP_GITHUB_TOKEN")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|_| format!("could not launch the Pandora CLI while {action}"))?;
    parse_cli_json(output, action)
}

fn run_cli_json_with_secret_environment(
    args: &[String],
    environment: &str,
    secret: &mut String,
    action: &str,
) -> Result<Value, String> {
    let output = Command::new(cli_program()?)
        .args(args)
        .env_remove("PANDORA_REGISTRY_TOKEN")
        .env_remove("PANDORA_DESKTOP_REGISTRY_TOKEN")
        .env_remove("PANDORA_GITHUB_TOKEN")
        .env_remove("PANDORA_DESKTOP_GITHUB_TOKEN")
        .env(environment, secret.as_str())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    secret.zeroize();
    let output = output.map_err(|_| format!("could not launch the Pandora CLI while {action}"))?;
    parse_cli_json(output, action)
}

fn parse_cli_json(output: std::process::Output, action: &str) -> Result<Value, String> {
    if !output.status.success() {
        return validate_cli_output(output, action).map(|()| Value::Null);
    }
    const MAX_CLI_JSON_BYTES: usize = 1024 * 1024;
    if output.stdout.len() > MAX_CLI_JSON_BYTES {
        return Err(format!("Pandora CLI returned too much data while {action}"));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|_| format!("Pandora CLI returned invalid JSON while {action}"))
}

fn package_count_message(data: &Value) -> String {
    let count = data
        .get("packages")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or_default();
    format!("{count} local package(s) available.")
}

fn run_cli_with_secret(args: &[String], secret: &mut String, action: &str) -> Result<(), String> {
    let child = Command::new(cli_program()?)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(_) => {
            secret.zeroize();
            return Err(format!("could not launch the Pandora CLI while {action}"));
        }
    };
    let write_result = child
        .stdin
        .take()
        .ok_or_else(|| "Pandora CLI did not open secret input".to_owned())
        .and_then(|mut stdin| {
            stdin
                .write_all(secret.as_bytes())
                .map_err(|_| "could not send the API key to Pandora's encrypted vault".to_owned())
        });
    secret.zeroize();
    if let Err(error) = write_result {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let output = child
        .wait_with_output()
        .map_err(|_| format!("Pandora CLI did not finish while {action}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!("Pandora CLI failed while {action}"))
    }
}

fn validate_cli_output(output: std::process::Output, action: &str) -> Result<(), String> {
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    let detail: String = detail
        .chars()
        .filter(|character| !character.is_control() || *character == '\n')
        .take(600)
        .collect();
    if detail.is_empty() {
        Err(format!("Pandora CLI failed while {action}"))
    } else {
        Err(format!("Pandora CLI failed while {action}: {detail}"))
    }
}

#[tauri::command]
fn pandora_rpc(
    state: State<'_, ServiceState>,
    method: String,
    params: Value,
) -> Result<Value, String> {
    if !matches!(
        method.as_str(),
        "runtime.health"
            | "runtime.capabilities"
            | "runtime.providers"
            | "runtime.engines"
            | "runtime.tools"
            | "orchestration.list"
            | "orchestration.inspect"
            | "orchestration.cancel"
            | "orchestration.resume"
            | "session.list"
            | "session.inspect"
            | "session.events"
            | "session.memory"
            | "approval.list"
            | "approval.inspect"
            | "approval.resolve"
            | "evolution.list"
            | "evolution.inspect"
            | "evolution.activations"
            | "evolution.activate"
            | "evolution.rollback"
            | "evolution.rollout.transition"
            | "run.execute"
            | "run.resume"
            | "agent.execute"
            | "agent.resume"
    ) {
        return Err("unsupported Pandora service method".to_owned());
    }
    let running = state
        .0
        .lock()
        .map_err(|_| "service state is unavailable".to_owned())?;
    let service = running
        .as_ref()
        .ok_or_else(|| "Pandora service is not running".to_owned())?;
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "system time is unavailable".to_owned())?
        .as_secs();
    let mut nonce_bytes = [0_u8; 16];
    getrandom::fill(&mut nonce_bytes)
        .map_err(|_| "device proof randomness is unavailable".to_owned())?;
    let nonce = encode_hex(&nonce_bytes);
    let request_body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "pandora-desktop",
        "method": method,
        "params": params,
    }))
    .map_err(|_| "could not encode the Pandora request".to_owned())?;
    let signature = sign_device_proof(
        &service.device_key,
        &service.token,
        timestamp,
        &nonce,
        "POST",
        "/v1/rpc",
        &request_body,
    )?;
    let response = reqwest::blocking::Client::builder()
        .no_proxy()
        .build()
        .map_err(|_| "could not configure the local service client".to_owned())?
        .post(&service.endpoint)
        .bearer_auth(&service.token)
        .header("x-pandora-device-id", &service.device_id)
        .header("x-pandora-timestamp", timestamp.to_string())
        .header("x-pandora-nonce", nonce)
        .header("x-pandora-signature", signature)
        .header("content-type", "application/json")
        .body(request_body)
        .send()
        .map_err(|_| "could not reach the local Pandora service".to_owned())?;
    if !response.status().is_success() {
        return Err(format!(
            "Pandora service returned HTTP {}",
            response.status()
        ));
    }
    response
        .json::<Value>()
        .map_err(|_| "Pandora service returned invalid JSON".to_owned())
}

fn is_valid_device_id(value: &str) -> bool {
    value.starts_with("device-")
        && value.len() == 39
        && value
            .bytes()
            .skip(7)
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn validate_loopback_endpoint(endpoint: &str) -> Result<(), String> {
    let url = Url::parse(endpoint)
        .map_err(|_| "Pandora service returned an invalid endpoint".to_owned())?;
    let host = url
        .host_str()
        .ok_or_else(|| "Pandora service endpoint has no host".to_owned())?;
    if !matches!(host, "127.0.0.1" | "localhost" | "[::1]" | "::1") {
        return Err("Pandora service endpoint is not loopback".to_owned());
    }
    Ok(())
}

fn read_token(path: &Path) -> Result<String, String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| "could not read the service token".to_owned())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("service token path is unsafe".to_owned());
    }
    let token = std::fs::read_to_string(path)
        .map_err(|_| "could not read the service token".to_owned())?
        .trim()
        .to_owned();
    if token.len() != TOKEN_LENGTH
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("service token has an invalid format".to_owned());
    }
    Ok(token)
}

fn read_device_key(path: &Path) -> Result<SigningKey, String> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| "could not read the device key".to_owned())?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() != DEVICE_KEY_LENGTH as u64
    {
        return Err("device key path is unsafe".to_owned());
    }
    let encoded = std::fs::read(path).map_err(|_| "could not read the device key".to_owned())?;
    let mut seed = decode_hex_32(&encoded).ok_or_else(|| "device key is invalid".to_owned())?;
    let key = SigningKey::from_bytes(&seed);
    seed.zeroize();
    Ok(key)
}

fn device_id_for_key(key: &SigningKey) -> String {
    let digest = Sha256::digest(key.verifying_key().to_bytes());
    format!("device-{}", encode_hex(&digest[..16]))
}

fn sign_device_proof(
    key: &SigningKey,
    token: &str,
    timestamp: u64,
    nonce: &str,
    method: &str,
    path: &str,
    body: &[u8],
) -> Result<String, String> {
    if token.len() != TOKEN_LENGTH || nonce.len() != 32 {
        return Err("device proof input is invalid".to_owned());
    }
    let token_digest = Sha256::digest(token.as_bytes());
    let body_digest = Sha256::digest(body);
    let mut message = b"pandora-device-proof-v2\0".to_vec();
    message.extend_from_slice(timestamp.to_string().as_bytes());
    message.push(0);
    message.extend_from_slice(nonce.as_bytes());
    message.push(0);
    message.extend_from_slice(method.as_bytes());
    message.push(0);
    message.extend_from_slice(path.as_bytes());
    message.push(0);
    message.extend_from_slice(&token_digest);
    message.extend_from_slice(&body_digest);
    Ok(encode_hex(&key.sign(&message).to_bytes()))
}

fn decode_hex_32(value: &[u8]) -> Option<[u8; 32]> {
    if value.len() != DEVICE_KEY_LENGTH {
        return None;
    }
    let mut decoded = [0_u8; 32];
    for (index, byte) in decoded.iter_mut().enumerate() {
        *byte = decode_hex(value[index * 2])? << 4 | decode_hex(value[index * 2 + 1])?;
    }
    Some(decoded)
}

fn decode_hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[cfg(target_os = "macos")]
fn install_platform_material(app: &mut tauri::App) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let options = LiquidGlassOptions::new(NSGlassEffectViewStyle::Clear)
        .radius(26.0)
        .opaque(false);
    if apply_liquid_glass(&window, options).is_err() {
        let _ = apply_vibrancy(
            &window,
            NSVisualEffectMaterial::UnderWindowBackground,
            Some(NSVisualEffectState::FollowsWindowActiveState),
            Some(26.0),
        );
    }
}

#[cfg(not(target_os = "macos"))]
fn install_platform_material(_app: &mut tauri::App) {}

fn main() {
    install_desktop_crash_reporter();
    tauri::Builder::default()
        .setup(|app| {
            install_platform_material(app);
            Ok(())
        })
        .manage(ServiceState::default())
        .invoke_handler(tauri::generate_handler![
            start_local_service,
            stop_local_service,
            configure_provider,
            activate_provider,
            configure_mcp,
            list_registry_profiles,
            configure_registry_profile,
            list_local_packages,
            list_package_transparency,
            fleet_operations_dashboard,
            list_local_skills,
            install_local_skill,
            mutate_local_skill,
            install_registry_package,
            install_github_package,
            admit_local_package,
            preview_package_enable,
            enable_local_package,
            preview_package_disable,
            disable_local_package,
            preview_package_rollback,
            rollback_local_package,
            preview_package_removal,
            remove_local_package,
            lock_local_packages,
            inspect_memory_audit,
            inspect_memory_provenance,
            preview_memory_forget,
            forget_memory,
            preview_memory_compaction,
            compact_memory,
            list_storage_lifecycle_evidence,
            pandora_rpc
        ])
        .on_window_event(|window, event| {
            if matches!(event, WindowEvent::Destroyed) {
                if let Some(state) = window.try_state::<ServiceState>() {
                    let _ = stop_local_service(state);
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Pandora desktop");
}

fn install_desktop_crash_reporter() {
    let data_directory = std::env::var_os("PANDORA_DATA_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("LOCALAPPDATA")
                .map(std::path::PathBuf::from)
                .map(|path| path.join("Pandora"))
        })
        .or_else(|| {
            std::env::var_os("XDG_DATA_HOME")
                .map(std::path::PathBuf::from)
                .map(|path| path.join("pandora"))
        })
        .or_else(|| {
            std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .map(|path| {
                    if cfg!(target_os = "macos") {
                        path.join("Library")
                            .join("Application Support")
                            .join("Pandora")
                    } else {
                        path.join(".local").join("share").join("pandora")
                    }
                })
        });
    let Some(data_directory) = data_directory else {
        return;
    };
    std::panic::set_hook(Box::new(move |info| {
        let occurred_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        let location_digest = info.location().map(|location| {
            let mut hasher = Sha256::new();
            hasher.update(b"pandora-desktop-crash-location-v1\0");
            hasher.update(location.file().as_bytes());
            hasher.update(location.line().to_be_bytes());
            hasher.update(location.column().to_be_bytes());
            encode_hex(&hasher.finalize())
        });
        let directory = data_directory.join("operations").join("crashes");
        if std::fs::create_dir_all(&directory).is_err() {
            return;
        }
        let path = directory.join(format!(
            "desktop-crash-{occurred_at}-{}.json",
            std::process::id()
        ));
        let report = json!({
            "schema_version": 1,
            "occurred_at": occurred_at,
            "component": "pandora-desktop",
            "version": env!("CARGO_PKG_VERSION"),
            "location_digest": location_digest,
            "panic_payload_recorded": false,
        });
        let Ok(bytes) = serde_json::to_vec_pretty(&report) else {
            return;
        };
        let Ok(mut options) = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
        else {
            return;
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = options.set_permissions(std::fs::Permissions::from_mode(0o600));
        }
        let _ = std::io::Write::write_all(&mut options, &bytes);
    }));
}

#[cfg(test)]
mod configuration_tests {
    use super::{
        optional_package_version, validate_environment_name, validate_github_commit,
        validate_github_repository_path, validate_github_repository_url, validate_identifier,
        validate_local_skill_directory, validate_memory_compaction, validate_memory_forget,
        validate_memory_id, validate_package_id, validate_provider_url, validate_registry_url,
        validate_skill_mutation, MemoryCompaction, MemoryForget, SkillMutation,
    };
    use std::path::Path;

    #[test]
    fn provider_urls_require_https_except_for_loopback() {
        assert!(validate_provider_url("https://models.example.test/v1").is_ok());
        assert!(validate_provider_url("http://127.0.0.1:11434/v1").is_ok());
        assert!(validate_provider_url("http://[::1]:11434/v1").is_ok());
        assert!(validate_provider_url("http://models.example.test/v1").is_err());
        assert!(validate_provider_url("https://token@models.example.test/v1").is_err());
        assert!(validate_provider_url("https://models.example.test/v1?key=secret").is_err());
    }

    #[test]
    fn configuration_identifiers_and_secret_references_are_bounded() {
        assert!(validate_identifier("local-tools.v2", "MCP server").is_ok());
        assert!(validate_identifier("../escape", "MCP server").is_err());
        assert!(validate_environment_name("PANDORA_CUSTOM_API_KEY").is_ok());
        assert!(validate_environment_name("Pandora-Key").is_err());
    }

    #[test]
    fn skill_lifecycle_is_allowlisted_and_removal_requires_exact_confirmation() {
        let enable = SkillMutation {
            skill_id: "local-skill".to_owned(),
            action: "enable".to_owned(),
            confirmation: String::new(),
        };
        assert!(validate_skill_mutation(&enable).is_ok());
        assert!(validate_local_skill_directory(Path::new("relative-skill")).is_err());

        let wrong_action = SkillMutation {
            action: "execute".to_owned(),
            ..enable
        };
        assert!(validate_skill_mutation(&wrong_action).is_err());

        let remove = SkillMutation {
            skill_id: "local-skill".to_owned(),
            action: "remove".to_owned(),
            confirmation: "different-skill".to_owned(),
        };
        assert!(validate_skill_mutation(&remove).is_err());
        assert!(validate_skill_mutation(&SkillMutation {
            confirmation: "local-skill".to_owned(),
            ..remove
        })
        .is_ok());
    }

    #[test]
    fn package_inputs_preserve_exact_identity_and_secure_registry_urls() {
        assert!(validate_package_id("owner/coding-gene").is_ok());
        assert!(validate_package_id("../coding-gene").is_err());
        assert!(validate_package_id("owner//coding-gene").is_err());
        assert_eq!(
            optional_package_version("2.0.0-beta.7").unwrap(),
            Some("2.0.0-beta.7".to_owned())
        );
        assert!(optional_package_version("2.0.0 beta").is_err());
        assert!(validate_registry_url("https://registry.example.test").is_ok());
        assert!(validate_registry_url("http://127.0.0.1:8080").is_ok());
        assert!(validate_registry_url("http://registry.example.test").is_err());
        assert!(validate_registry_url("https://user@registry.example.test").is_err());
    }

    #[test]
    fn memory_governance_inputs_are_bounded_and_confirmation_is_exact() {
        assert!(validate_memory_id("lesson-release-review").is_ok());
        assert!(validate_memory_id(" lesson-release-review").is_err());
        assert!(validate_memory_id("lesson release review").is_err());
        assert!(validate_memory_id(&"x".repeat(257)).is_err());

        let valid = MemoryForget {
            session_id: "session-1".to_owned(),
            provider: "openai-compatible".to_owned(),
            memory_id: "lesson-release-review".to_owned(),
            confirmation: "lesson-release-review".to_owned(),
        };
        assert!(validate_memory_forget(&valid).is_ok());

        let wrong_confirmation = MemoryForget {
            confirmation: "lesson-release".to_owned(),
            ..valid
        };
        assert!(validate_memory_forget(&wrong_confirmation).is_err());

        let compaction = MemoryCompaction {
            session_id: "session-1".to_owned(),
            provider: "openai-compatible".to_owned(),
            revoked_before_or_at: 4_102_444_800,
            confirmation: "COMPACT 4102444800".to_owned(),
        };
        assert!(validate_memory_compaction(&compaction).is_ok());
        assert!(validate_memory_compaction(&MemoryCompaction {
            confirmation: "COMPACT all".to_owned(),
            ..compaction
        })
        .is_err());
    }

    #[test]
    fn github_package_inputs_require_pinned_bounded_sources() {
        let commit = "0123456789abcdef0123456789abcdef01234567";
        assert!(validate_github_repository_url("https://github.com/owner/repository").is_ok());
        assert!(validate_github_repository_url("https://github.com/owner/repository.git").is_ok());
        assert!(validate_github_repository_url("https://example.com/owner/repository").is_err());
        assert!(
            validate_github_repository_url("https://github.com/owner/repository/tree/main")
                .is_err()
        );
        assert!(validate_github_commit(commit).is_ok());
        assert!(validate_github_commit("main").is_err());
        assert!(validate_github_repository_path("packages/gene.json", "manifest").is_ok());
        assert!(validate_github_repository_path("../gene.json", "manifest").is_err());
    }
}

#[cfg(test)]
mod desktop_packaging_tests {
    use super::{cli_binary_name, validate_cli_program_path};
    use serde_json::Value;
    use std::path::Path;

    #[test]
    fn cli_override_requires_an_absolute_regular_file() {
        assert!(validate_cli_program_path(Path::new("pandora"), "CLI override").is_err());
    }

    #[test]
    fn bundle_configuration_declares_the_pandora_sidecar() {
        let config: Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config["bundle"]["externalBin"][0], "binaries/pandora");
        assert_eq!(
            cli_binary_name(),
            if cfg!(windows) {
                "pandora.exe"
            } else {
                "pandora"
            }
        );
    }

    #[test]
    fn macos_configuration_keeps_native_window_controls_and_transparency() {
        let config: Value = serde_json::from_str(include_str!("../tauri.macos.conf.json")).unwrap();
        assert_eq!(config["app"]["macOSPrivateApi"], true);
        let window = &config["app"]["windows"][0];
        assert_eq!(window["transparent"], true);
        assert_eq!(window["titleBarStyle"], "Overlay");
        assert_eq!(window["hiddenTitle"], true);
        assert_eq!(window["decorations"], true);
    }
}
