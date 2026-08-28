#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use ed25519_dalek::{Signer, SigningKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use tauri::{Manager, State, WindowEvent};
use url::Url;
use zeroize::Zeroize;

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

    let program = std::env::var_os("PANDORA_CLI_PATH").unwrap_or_else(|| "pandora".into());
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

fn main() {
    install_desktop_crash_reporter();
    tauri::Builder::default()
        .manage(ServiceState::default())
        .invoke_handler(tauri::generate_handler![
            start_local_service,
            stop_local_service,
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
