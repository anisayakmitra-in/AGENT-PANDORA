#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use tauri::{Manager, State, WindowEvent};
use url::Url;

const NATIVE_ENDPOINT: &str = "tauri://pandora";
const TOKEN_LENGTH: usize = 64;

#[derive(Default)]
struct ServiceState(Mutex<Option<RunningService>>);

struct RunningService {
    child: Child,
    endpoint: String,
    token: String,
}

#[derive(Deserialize)]
struct ServiceReadiness {
    endpoint: String,
    token_path: String,
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
    *running = Some(RunningService {
        child,
        endpoint: readiness.endpoint,
        token,
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
        "runtime.health" | "runtime.capabilities" | "runtime.providers" | "runtime.engines" | "runtime.tools" | "session.list" | "session.inspect" | "session.events" | "session.memory" | "approval.list" | "approval.inspect" | "approval.resolve" | "run.execute" | "run.resume"
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
    let response = reqwest::blocking::Client::builder()
        .no_proxy()
        .build()
        .map_err(|_| "could not configure the local service client".to_owned())?
        .post(&service.endpoint)
        .bearer_auth(&service.token)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": "pandora-desktop",
            "method": method,
            "params": params,
        }))
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

fn main() {
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
