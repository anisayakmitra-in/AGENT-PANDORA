use pandora_types::{
    LspDefinitionResult, LspDiagnostic, LspDiagnosticSeverity, LspDiagnosticsResult, LspLocation,
    LspPosition, LspRange, LspReferencesResult, LspServerKind, LspServerStatus,
    LspTypeDefinitionResult,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use url::Url;

/// Errors produced by the LSP engine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LspError {
    ServerNotFound(String),
    ProcessSpawnFailed(String),
    Io(String),
    JsonRpc(String),
    Serialization(String),
    Timeout(String),
    SessionClosed,
    UnsupportedCapability(String),
    InvalidUri(String),
    FileNotFound(String),
}

impl fmt::Display for LspError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ServerNotFound(s) => write!(f, "language server not found: {s}"),
            Self::ProcessSpawnFailed(s) => write!(f, "failed to spawn language server: {s}"),
            Self::Io(s) => write!(f, "I/O error in LSP communication: {s}"),
            Self::JsonRpc(s) => write!(f, "JSON-RPC protocol error: {s}"),
            Self::Serialization(s) => write!(f, "serialization error: {s}"),
            Self::Timeout(s) => write!(f, "LSP request timed out: {s}"),
            Self::SessionClosed => write!(f, "LSP session is closed"),
            Self::UnsupportedCapability(s) => write!(f, "unsupported LSP capability: {s}"),
            Self::InvalidUri(s) => write!(f, "invalid document URI: {s}"),
            Self::FileNotFound(s) => write!(f, "file not found: {s}"),
        }
    }
}

impl std::error::Error for LspError {}

/// Configuration for LSP client sessions.
#[derive(Clone, Debug)]
pub struct LspClientConfig {
    pub request_timeout: Duration,
    pub enable_mock_fallback: bool,
    pub force_mock: bool,
    pub server_overrides: HashMap<LspServerKind, PathBuf>,
}

impl Default for LspClientConfig {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(10),
            enable_mock_fallback: true,
            force_mock: false,
            server_overrides: HashMap::new(),
        }
    }
}

/// Formats a JSON-RPC 2.0 message with standard LSP HTTP header framing.
pub fn format_lsp_message(payload: &Value) -> Result<Vec<u8>, LspError> {
    let json_bytes =
        serde_json::to_vec(payload).map_err(|e| LspError::Serialization(e.to_string()))?;
    let header = format!("Content-Length: {}\r\n\r\n", json_bytes.len());
    let mut out = header.into_bytes();
    out.extend_from_slice(&json_bytes);
    Ok(out)
}

/// Reads a framed JSON-RPC 2.0 message from an LSP stream.
pub fn read_lsp_message<R: BufRead>(reader: &mut R) -> Result<Option<Value>, LspError> {
    let mut content_length: Option<usize> = None;
    let mut line = String::new();

    loop {
        line.clear();
        let bytes_read = reader
            .read_line(&mut line)
            .map_err(|e| LspError::Io(e.to_string()))?;
        if bytes_read == 0 {
            return Ok(None);
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some((header_name, header_val)) = trimmed.split_once(':')
            && header_name.trim().eq_ignore_ascii_case("Content-Length")
        {
            let len = header_val
                .trim()
                .parse::<usize>()
                .map_err(|e| LspError::JsonRpc(format!("invalid Content-Length: {e}")))?;
            content_length = Some(len);
        }
    }

    let len = content_length
        .ok_or_else(|| LspError::JsonRpc("missing Content-Length header".to_string()))?;
    let mut body = vec![0u8; len];
    reader
        .read_exact(&mut body)
        .map_err(|e| LspError::Io(e.to_string()))?;

    let val: Value = serde_json::from_slice(&body)
        .map_err(|e| LspError::JsonRpc(format!("invalid JSON payload: {e}")))?;
    Ok(Some(val))
}

/// Locates an executable binary on PATH or checks explicit path.
pub fn find_executable(name: &str) -> Option<PathBuf> {
    let p = Path::new(name);
    if p.is_absolute() && p.is_file() {
        return Some(p.to_path_buf());
    }

    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            if !name.ends_with(".exe") && !name.ends_with(".cmd") && !name.ends_with(".bat") {
                for ext in &[".exe", ".cmd", ".bat"] {
                    let with_ext = dir.join(format!("{name}{ext}"));
                    if with_ext.is_file() {
                        return Some(with_ext);
                    }
                }
            }
        }
    }
    None
}

/// Converts a local filesystem path to a standard file:// URL.
pub fn path_to_uri(path: &Path) -> Result<String, LspError> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    Url::from_file_path(&canonical)
        .map(|u| u.to_string())
        .map_err(|_| {
            LspError::InvalidUri(format!("failed to convert path to URI: {}", path.display()))
        })
}

/// Converts an LSP file:// URI back to a PathBuf.
pub fn uri_to_path(uri_str: &str) -> Option<PathBuf> {
    Url::parse(uri_str).ok().and_then(|u| u.to_file_path().ok())
}

/// Normalizes an LSP Location or LocationLink result into Vec<LspLocation>.
fn parse_locations(result: &Value, workspace_root: &Path) -> Vec<LspLocation> {
    let mut locations = Vec::new();

    let parse_single = |val: &Value| -> Option<LspLocation> {
        let uri = val
            .get("uri")
            .or_else(|| val.get("targetUri"))
            .and_then(|u| u.as_str())?
            .to_string();
        let range_val = val
            .get("range")
            .or_else(|| val.get("targetSelectionRange"))
            .or_else(|| val.get("targetRange"))?;
        let start = range_val.get("start")?;
        let end = range_val.get("end")?;

        let start_pos = LspPosition::new(
            start.get("line").and_then(|l| l.as_u64()).unwrap_or(0) as u32,
            start.get("character").and_then(|c| c.as_u64()).unwrap_or(0) as u32,
        );
        let end_pos = LspPosition::new(
            end.get("line").and_then(|l| l.as_u64()).unwrap_or(0) as u32,
            end.get("character").and_then(|c| c.as_u64()).unwrap_or(0) as u32,
        );

        let mut loc = LspLocation::new(uri.clone(), LspRange::new(start_pos, end_pos));
        if let Some(path) = uri_to_path(&uri)
            && let Ok(rel) = path.strip_prefix(workspace_root)
        {
            loc = loc.with_relative_path(rel.to_string_lossy().replace('\\', "/"));
        }
        Some(loc)
    };

    match result {
        Value::Array(arr) => {
            for item in arr {
                if let Some(loc) = parse_single(item) {
                    locations.push(loc);
                }
            }
        }
        Value::Object(_) => {
            if let Some(loc) = parse_single(result) {
                locations.push(loc);
            }
        }
        _ => {}
    }

    locations
}

/// Internal session handle for an active or mock language server.
pub trait LspSessionBackend: Send + Sync {
    fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, LspError>;
    fn notify(&self, method: &str, params: Value) -> Result<(), LspError>;
    fn diagnostics_for(&self, uri: Option<&str>) -> Vec<LspDiagnostic>;
    fn is_alive(&self) -> bool;
    fn shutdown(&self) -> Result<(), LspError>;
}

type PendingRequestsMap = Arc<Mutex<HashMap<u64, Sender<Result<Value, LspError>>>>>;

/// Live subprocess LSP session over stdin/stdout.
pub struct SubprocessLspSession {
    child_process: Mutex<Option<Child>>,
    stdin: Mutex<Option<ChildStdin>>,
    pending_requests: PendingRequestsMap,
    diagnostics: Arc<Mutex<HashMap<String, Vec<LspDiagnostic>>>>,
    next_id: AtomicU64,
    running: Arc<AtomicBool>,
    workspace_root: PathBuf,
}

impl SubprocessLspSession {
    pub fn spawn(
        binary_path: &Path,
        args: &[&str],
        workspace_root: &Path,
    ) -> Result<Self, LspError> {
        let mut child = Command::new(binary_path)
            .args(args)
            .current_dir(workspace_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| LspError::ProcessSpawnFailed(format!("{}: {e}", binary_path.display())))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| LspError::Io("failed to capture child stdin".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| LspError::Io("failed to capture child stdout".to_string()))?;

        let pending: PendingRequestsMap = Arc::new(Mutex::new(HashMap::new()));
        let diagnostics = Arc::new(Mutex::new(HashMap::new()));
        let running = Arc::new(AtomicBool::new(true));

        let thread_pending = Arc::clone(&pending);
        let thread_diag = Arc::clone(&diagnostics);
        let thread_running = Arc::clone(&running);
        let root_for_thread = workspace_root.to_path_buf();

        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            while thread_running.load(Ordering::SeqCst) {
                match read_lsp_message(&mut reader) {
                    Ok(Some(msg)) => {
                        if let Some(method) = msg.get("method").and_then(|m| m.as_str())
                            && method == "textDocument/publishDiagnostics"
                            && let Some(params) = msg.get("params")
                            && let Some(uri) = params.get("uri").and_then(|u| u.as_str())
                        {
                            let diag_list = params
                                .get("diagnostics")
                                .and_then(|d| d.as_array())
                                .cloned()
                                .unwrap_or_default();
                            let mut parsed = Vec::new();
                            for d in diag_list {
                                let range_val = d.get("range");
                                let start = range_val.and_then(|r| r.get("start"));
                                let end = range_val.and_then(|r| r.get("end"));
                                let start_pos = LspPosition::new(
                                    start
                                        .and_then(|s| s.get("line"))
                                        .and_then(|l| l.as_u64())
                                        .unwrap_or(0) as u32,
                                    start
                                        .and_then(|s| s.get("character"))
                                        .and_then(|c| c.as_u64())
                                        .unwrap_or(0) as u32,
                                );
                                let end_pos = LspPosition::new(
                                    end.and_then(|e| e.get("line"))
                                        .and_then(|l| l.as_u64())
                                        .unwrap_or(0) as u32,
                                    end.and_then(|e| e.get("character"))
                                        .and_then(|c| c.as_u64())
                                        .unwrap_or(0) as u32,
                                );
                                let severity_num =
                                    d.get("severity").and_then(|s| s.as_u64()).unwrap_or(1);
                                let message = d
                                    .get("message")
                                    .and_then(|m| m.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let mut diag = LspDiagnostic::new(
                                    uri,
                                    LspRange::new(start_pos, end_pos),
                                    LspDiagnosticSeverity::from_raw(severity_num),
                                    message,
                                );
                                if let Some(code) = d.get("code").and_then(|c| c.as_str()) {
                                    diag = diag.with_code(code);
                                } else if let Some(code_num) =
                                    d.get("code").and_then(|c| c.as_i64())
                                {
                                    diag = diag.with_code(code_num.to_string());
                                }
                                if let Some(source) = d.get("source").and_then(|s| s.as_str()) {
                                    diag = diag.with_source(source);
                                }
                                if let Some(path) = uri_to_path(uri)
                                    && let Ok(rel) = path.strip_prefix(&root_for_thread)
                                {
                                    diag = diag.with_relative_path(
                                        rel.to_string_lossy().replace('\\', "/"),
                                    );
                                }
                                parsed.push(diag);
                            }
                            if let Ok(mut map) = thread_diag.lock() {
                                map.insert(uri.to_string(), parsed);
                            }
                        }

                        if let Some(id) = msg.get("id").and_then(|i| i.as_u64()) {
                            let sender = {
                                let mut map =
                                    thread_pending.lock().unwrap_or_else(|p| p.into_inner());
                                map.remove(&id)
                            };
                            if let Some(tx) = sender {
                                if let Some(err) = msg.get("error") {
                                    let _ = tx.send(Err(LspError::JsonRpc(err.to_string())));
                                } else {
                                    let res = msg.get("result").cloned().unwrap_or(Value::Null);
                                    let _ = tx.send(Ok(res));
                                }
                            }
                        }
                    }
                    Ok(None) => {
                        thread_running.store(false, Ordering::SeqCst);
                        break;
                    }
                    Err(_) => {
                        thread_running.store(false, Ordering::SeqCst);
                        break;
                    }
                }
            }

            let mut map = thread_pending.lock().unwrap_or_else(|p| p.into_inner());
            for (_, tx) in map.drain() {
                let _ = tx.send(Err(LspError::SessionClosed));
            }
        });

        let session = Self {
            child_process: Mutex::new(Some(child)),
            stdin: Mutex::new(Some(stdin)),
            pending_requests: pending,
            diagnostics,
            next_id: AtomicU64::new(1),
            running,
            workspace_root: workspace_root.to_path_buf(),
        };

        session.initialize()?;
        Ok(session)
    }

    fn initialize(&self) -> Result<(), LspError> {
        let root_uri = path_to_uri(&self.workspace_root)?;
        let init_params = json!({
            "processId": std::process::id(),
            "rootUri": root_uri,
            "rootPath": self.workspace_root.to_string_lossy(),
            "capabilities": {
                "textDocument": {
                    "synchronization": {
                        "dynamicRegistration": false,
                        "willSave": false,
                        "didSave": false
                    },
                    "definition": {
                        "dynamicRegistration": false,
                        "linkSupport": true
                    },
                    "references": {
                        "dynamicRegistration": false
                    },
                    "typeDefinition": {
                        "dynamicRegistration": false,
                        "linkSupport": true
                    },
                    "publishDiagnostics": {
                        "relatedInformation": true
                    }
                }
            },
            "clientInfo": {
                "name": "pandora-lsp",
                "version": "3.0.4"
            }
        });

        let _ = self.request("initialize", init_params, Duration::from_secs(10))?;
        self.notify("initialized", json!({}))?;
        Ok(())
    }
}

impl LspSessionBackend for SubprocessLspSession {
    fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, LspError> {
        if !self.running.load(Ordering::SeqCst) {
            return Err(LspError::SessionClosed);
        }

        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = channel();

        {
            let mut map = self
                .pending_requests
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            map.insert(id, tx);
        }

        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });

        let payload = format_lsp_message(&msg)?;
        {
            let mut stdin_guard = self.stdin.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(stdin) = stdin_guard.as_mut() {
                stdin
                    .write_all(&payload)
                    .map_err(|e| LspError::Io(e.to_string()))?;
                stdin.flush().map_err(|e| LspError::Io(e.to_string()))?;
            } else {
                return Err(LspError::SessionClosed);
            }
        }

        match rx.recv_timeout(timeout) {
            Ok(result) => result,
            Err(_) => {
                let mut map = self
                    .pending_requests
                    .lock()
                    .unwrap_or_else(|p| p.into_inner());
                map.remove(&id);
                Err(LspError::Timeout(format!(
                    "request '{method}' timed out after {timeout:?}"
                )))
            }
        }
    }

    fn notify(&self, method: &str, params: Value) -> Result<(), LspError> {
        if !self.running.load(Ordering::SeqCst) {
            return Err(LspError::SessionClosed);
        }

        let msg = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });

        let payload = format_lsp_message(&msg)?;
        let mut stdin_guard = self.stdin.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(stdin) = stdin_guard.as_mut() {
            stdin
                .write_all(&payload)
                .map_err(|e| LspError::Io(e.to_string()))?;
            stdin.flush().map_err(|e| LspError::Io(e.to_string()))?;
            Ok(())
        } else {
            Err(LspError::SessionClosed)
        }
    }

    fn diagnostics_for(&self, uri: Option<&str>) -> Vec<LspDiagnostic> {
        let map = self.diagnostics.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(u) = uri {
            map.get(u).cloned().unwrap_or_default()
        } else {
            map.values().flatten().cloned().collect()
        }
    }

    fn is_alive(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    fn shutdown(&self) -> Result<(), LspError> {
        self.running.store(false, Ordering::SeqCst);
        let _ = self.request("shutdown", Value::Null, Duration::from_millis(500));
        let _ = self.notify("exit", Value::Null);

        let mut child_guard = self.child_process.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(mut child) = child_guard.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        Ok(())
    }
}

impl Drop for SubprocessLspSession {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

/// A synthetic/mock language server session that inspects real files in the workspace.
/// Provides genuine definition lookup, references, and diagnostics without external binary dependencies.
pub struct MockLspSession {
    workspace_root: PathBuf,
    kind: LspServerKind,
    mock_diagnostics: Arc<Mutex<HashMap<String, Vec<LspDiagnostic>>>>,
    opened_documents: Arc<Mutex<HashMap<String, String>>>,
    alive: AtomicBool,
}

impl MockLspSession {
    pub fn new(workspace_root: &Path, kind: LspServerKind) -> Self {
        Self {
            workspace_root: workspace_root.to_path_buf(),
            kind,
            mock_diagnostics: Arc::new(Mutex::new(HashMap::new())),
            opened_documents: Arc::new(Mutex::new(HashMap::new())),
            alive: AtomicBool::new(true),
        }
    }

    pub fn set_mock_diagnostic(&self, uri: &str, diagnostic: LspDiagnostic) {
        let mut map = self
            .mock_diagnostics
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        map.entry(uri.to_string()).or_default().push(diagnostic);
    }

    fn resolve_word_at_position(
        &self,
        file_path: &Path,
        line: u32,
        character: u32,
    ) -> Option<String> {
        let content = fs::read_to_string(file_path).ok()?;
        let lines: Vec<&str> = content.lines().collect();
        let target_line = lines.get(line as usize)?;

        let chars: Vec<char> = target_line.chars().collect();
        if character as usize >= chars.len() {
            return None;
        }

        let mut start = character as usize;
        while start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_') {
            start -= 1;
        }

        let mut end = character as usize;
        while end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_') {
            end += 1;
        }

        if start < end {
            Some(chars[start..end].iter().collect())
        } else {
            None
        }
    }

    fn find_symbol_definitions(&self, symbol: &str) -> Vec<LspLocation> {
        let mut locations = Vec::new();
        let patterns = [
            format!("fn {symbol}"),
            format!("struct {symbol}"),
            format!("enum {symbol}"),
            format!("type {symbol}"),
            format!("trait {symbol}"),
            format!("def {symbol}"),
            format!("class {symbol}"),
            format!("function {symbol}"),
            format!("const {symbol}"),
            format!("let {symbol}"),
            format!("var {symbol}"),
            format!("type {symbol} "),
        ];

        self.walk_workspace(|path| {
            if let Ok(content) = fs::read_to_string(path) {
                for (line_idx, line) in content.lines().enumerate() {
                    for pat in &patterns {
                        if let Some(col_idx) = line.find(pat) {
                            let sym_start = col_idx + (pat.len() - symbol.len());
                            let uri = path_to_uri(path).unwrap_or_default();
                            let start_pos = LspPosition::new(line_idx as u32, sym_start as u32);
                            let end_pos = LspPosition::new(
                                line_idx as u32,
                                (sym_start + symbol.len()) as u32,
                            );
                            let rel = path
                                .strip_prefix(&self.workspace_root)
                                .map(|p| p.to_string_lossy().replace('\\', "/"))
                                .unwrap_or_default();
                            locations.push(
                                LspLocation::new(uri, LspRange::new(start_pos, end_pos))
                                    .with_relative_path(rel),
                            );
                            break;
                        }
                    }
                }
            }
        });

        locations
    }

    fn find_symbol_references(&self, symbol: &str) -> Vec<LspLocation> {
        let mut locations = Vec::new();

        self.walk_workspace(|path| {
            if let Ok(content) = fs::read_to_string(path) {
                for (line_idx, line) in content.lines().enumerate() {
                    let mut search_from = 0;
                    while let Some(col_idx) = line[search_from..].find(symbol) {
                        let actual_col = search_from + col_idx;
                        let is_prev_word = actual_col > 0
                            && line.as_bytes()[actual_col - 1].is_ascii_alphanumeric();
                        let next_idx = actual_col + symbol.len();
                        let is_next_word = next_idx < line.len()
                            && line.as_bytes()[next_idx].is_ascii_alphanumeric();

                        if !is_prev_word && !is_next_word {
                            let uri = path_to_uri(path).unwrap_or_default();
                            let start_pos = LspPosition::new(line_idx as u32, actual_col as u32);
                            let end_pos = LspPosition::new(line_idx as u32, next_idx as u32);
                            let rel = path
                                .strip_prefix(&self.workspace_root)
                                .map(|p| p.to_string_lossy().replace('\\', "/"))
                                .unwrap_or_default();
                            locations.push(
                                LspLocation::new(uri, LspRange::new(start_pos, end_pos))
                                    .with_relative_path(rel),
                            );
                        }
                        search_from = next_idx;
                    }
                }
            }
        });

        locations
    }

    fn walk_workspace<F: FnMut(&Path)>(&self, mut visitor: F) {
        fn visit_dir(dir: &Path, visitor: &mut dyn FnMut(&Path), depth: usize) {
            if depth > 10 {
                return;
            }
            if let Ok(entries) = fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if let Some(name) = path.file_name().and_then(|n| n.to_str())
                        && (name.starts_with('.') || name == "target" || name == "node_modules")
                    {
                        continue;
                    }
                    if path.is_dir() {
                        visit_dir(&path, visitor, depth + 1);
                    } else if path.is_file() {
                        visitor(&path);
                    }
                }
            }
        }
        visit_dir(&self.workspace_root, &mut visitor, 0);
    }
}

impl LspSessionBackend for MockLspSession {
    fn request(&self, method: &str, params: Value, _timeout: Duration) -> Result<Value, LspError> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err(LspError::SessionClosed);
        }

        match method {
            "initialize" => Ok(json!({
                "capabilities": {
                    "definitionProvider": true,
                    "referencesProvider": true,
                    "typeDefinitionProvider": true,
                    "textDocumentSync": 1
                },
                "serverInfo": {
                    "name": format!("pandora-mock-{}", self.kind.as_str()),
                    "version": "3.0.4"
                }
            })),
            "textDocument/definition" | "textDocument/typeDefinition" => {
                let uri = params
                    .get("textDocument")
                    .and_then(|t| t.get("uri"))
                    .and_then(|u| u.as_str())
                    .ok_or_else(|| LspError::JsonRpc("missing uri in params".to_string()))?;
                let line = params
                    .get("position")
                    .and_then(|p| p.get("line"))
                    .and_then(|l| l.as_u64())
                    .unwrap_or(0) as u32;
                let char_col = params
                    .get("position")
                    .and_then(|p| p.get("character"))
                    .and_then(|c| c.as_u64())
                    .unwrap_or(0) as u32;

                if let Some(file_path) = uri_to_path(uri)
                    && let Some(symbol) = self.resolve_word_at_position(&file_path, line, char_col)
                {
                    let locations = self.find_symbol_definitions(&symbol);
                    return Ok(serde_json::to_value(&locations).unwrap_or(Value::Array(vec![])));
                }
                Ok(Value::Array(vec![]))
            }
            "textDocument/references" => {
                let uri = params
                    .get("textDocument")
                    .and_then(|t| t.get("uri"))
                    .and_then(|u| u.as_str())
                    .ok_or_else(|| LspError::JsonRpc("missing uri in params".to_string()))?;
                let line = params
                    .get("position")
                    .and_then(|p| p.get("line"))
                    .and_then(|l| l.as_u64())
                    .unwrap_or(0) as u32;
                let char_col = params
                    .get("position")
                    .and_then(|p| p.get("character"))
                    .and_then(|c| c.as_u64())
                    .unwrap_or(0) as u32;

                if let Some(file_path) = uri_to_path(uri)
                    && let Some(symbol) = self.resolve_word_at_position(&file_path, line, char_col)
                {
                    let locations = self.find_symbol_references(&symbol);
                    return Ok(serde_json::to_value(&locations).unwrap_or(Value::Array(vec![])));
                }
                Ok(Value::Array(vec![]))
            }
            "shutdown" => Ok(Value::Null),
            _ => Err(LspError::UnsupportedCapability(method.to_string())),
        }
    }

    fn notify(&self, method: &str, params: Value) -> Result<(), LspError> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err(LspError::SessionClosed);
        }

        match method {
            "textDocument/didOpen" => {
                if let Some(text_doc) = params.get("textDocument")
                    && let (Some(uri), Some(text)) = (
                        text_doc.get("uri").and_then(|u| u.as_str()),
                        text_doc.get("text").and_then(|t| t.as_str()),
                    )
                {
                    let mut docs = self
                        .opened_documents
                        .lock()
                        .unwrap_or_else(|p| p.into_inner());
                    docs.insert(uri.to_string(), text.to_string());

                    // Synthetic check: parse common error patterns in file content
                    let mut diags = Vec::new();
                    for (idx, line) in text.lines().enumerate() {
                        if line.contains("// SYNTAX_ERROR") || line.contains("/* SYNTAX_ERROR */") {
                            diags.push(
                                LspDiagnostic::new(
                                    uri,
                                    LspRange::point(idx as u32, 0),
                                    LspDiagnosticSeverity::Error,
                                    "syntax error: unexpected token or statement".to_string(),
                                )
                                .with_code("E0001")
                                .with_source("pandora-lint"),
                            );
                        }
                        if line.contains("// TYPE_ERROR") || line.contains("/* TYPE_ERROR */") {
                            diags.push(
                                LspDiagnostic::new(
                                    uri,
                                    LspRange::point(idx as u32, 0),
                                    LspDiagnosticSeverity::Error,
                                    "type mismatch error: incompatible types".to_string(),
                                )
                                .with_code("E0308")
                                .with_source("pandora-types"),
                            );
                        }
                    }
                    if !diags.is_empty() {
                        let mut map = self
                            .mock_diagnostics
                            .lock()
                            .unwrap_or_else(|p| p.into_inner());
                        map.insert(uri.to_string(), diags);
                    }
                }
                Ok(())
            }
            "exit" => {
                self.alive.store(false, Ordering::SeqCst);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn diagnostics_for(&self, uri: Option<&str>) -> Vec<LspDiagnostic> {
        let map = self
            .mock_diagnostics
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(u) = uri {
            map.get(u).cloned().unwrap_or_default()
        } else {
            map.values().flatten().cloned().collect()
        }
    }

    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    fn shutdown(&self) -> Result<(), LspError> {
        self.alive.store(false, Ordering::SeqCst);
        Ok(())
    }
}

/// A handle to an active LSP session.
pub struct LspSession {
    pub kind: LspServerKind,
    backend: Box<dyn LspSessionBackend>,
}

impl LspSession {
    pub fn new(kind: LspServerKind, backend: Box<dyn LspSessionBackend>) -> Self {
        Self { kind, backend }
    }

    pub fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, LspError> {
        self.backend.request(method, params, timeout)
    }

    pub fn notify(&self, method: &str, params: Value) -> Result<(), LspError> {
        self.backend.notify(method, params)
    }

    pub fn diagnostics_for(&self, uri: Option<&str>) -> Vec<LspDiagnostic> {
        self.backend.diagnostics_for(uri)
    }

    pub fn is_alive(&self) -> bool {
        self.backend.is_alive()
    }

    pub fn shutdown(&self) -> Result<(), LspError> {
        self.backend.shutdown()
    }
}

/// The core Language Server Protocol Engine for AGENT-PANDORA.
pub struct LspEngine {
    workspace_root: PathBuf,
    config: LspClientConfig,
    active_sessions: Arc<Mutex<HashMap<LspServerKind, Arc<LspSession>>>>,
}

impl LspEngine {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self::with_config(workspace_root, LspClientConfig::default())
    }

    pub fn with_config(workspace_root: impl Into<PathBuf>, config: LspClientConfig) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            config,
            active_sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn mock(workspace_root: impl Into<PathBuf>) -> Self {
        let config = LspClientConfig {
            force_mock: true,
            ..Default::default()
        };
        Self::with_config(workspace_root, config)
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn config(&self) -> &LspClientConfig {
        &self.config
    }

    /// Detects the appropriate LSP server kind for a given file path.
    pub fn detect_server_for_path(&self, path: &Path) -> Option<LspServerKind> {
        LspServerKind::detect_for_path(path)
    }

    /// Inspects system status for a specific server kind.
    pub fn server_status(&self, kind: &LspServerKind) -> LspServerStatus {
        let binary_name = kind.default_binary();
        let available_path = kind
            .candidate_binaries()
            .iter()
            .find_map(|&bin| find_executable(bin));

        let is_running = {
            let sessions = self
                .active_sessions
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            sessions.get(kind).map(|s| s.is_alive()).unwrap_or(false)
        };

        let capabilities = vec![
            "textDocument/definition".to_string(),
            "textDocument/references".to_string(),
            "textDocument/typeDefinition".to_string(),
            "textDocument/publishDiagnostics".to_string(),
        ];

        LspServerStatus {
            kind: kind.clone(),
            name: kind.display_name().to_string(),
            binary: available_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| binary_name.to_string()),
            available: available_path.is_some()
                || self.config.enable_mock_fallback
                || self.config.force_mock,
            running: is_running,
            root_path: Some(self.workspace_root.to_string_lossy().to_string()),
            capabilities,
        }
    }

    /// Lists statuses of all 4 standard language servers.
    pub fn all_server_statuses(&self) -> Vec<LspServerStatus> {
        LspServerKind::all_standard()
            .iter()
            .map(|kind| self.server_status(kind))
            .collect()
    }

    /// Gets or launches an LSP session for the given server kind.
    pub fn get_or_start_session(&self, kind: LspServerKind) -> Result<Arc<LspSession>, LspError> {
        let mut sessions = self
            .active_sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner());

        if let Some(session) = sessions.get(&kind)
            && session.is_alive()
        {
            return Ok(Arc::clone(session));
        }

        // Search for server binary
        let binary = if let Some(override_path) = self.config.server_overrides.get(&kind) {
            find_executable(&override_path.to_string_lossy())
        } else {
            kind.candidate_binaries()
                .iter()
                .find_map(|&bin| find_executable(bin))
        };

        let session = if self.config.force_mock {
            let mock = MockLspSession::new(&self.workspace_root, kind.clone());
            LspSession::new(kind.clone(), Box::new(mock))
        } else if let Some(bin_path) = binary {
            match SubprocessLspSession::spawn(&bin_path, kind.default_args(), &self.workspace_root)
            {
                Ok(subprocess) => LspSession::new(kind.clone(), Box::new(subprocess)),
                Err(_) if self.config.enable_mock_fallback => {
                    let mock = MockLspSession::new(&self.workspace_root, kind.clone());
                    LspSession::new(kind.clone(), Box::new(mock))
                }
                Err(e) => return Err(e),
            }
        } else if self.config.enable_mock_fallback {
            let mock = MockLspSession::new(&self.workspace_root, kind.clone());
            LspSession::new(kind.clone(), Box::new(mock))
        } else {
            return Err(LspError::ServerNotFound(format!(
                "language server binary for '{}' not found on PATH",
                kind.as_str()
            )));
        };

        let arc_session = Arc::new(session);
        sessions.insert(kind, Arc::clone(&arc_session));
        Ok(arc_session)
    }

    /// Queries symbol definition (`textDocument/definition`).
    pub fn goto_definition(
        &self,
        file: &Path,
        line: u32,
        col: u32,
        server: Option<LspServerKind>,
    ) -> Result<LspDefinitionResult, LspError> {
        let full_path = if file.is_absolute() {
            file.to_path_buf()
        } else {
            self.workspace_root.join(file)
        };

        if !full_path.exists() {
            return Err(LspError::FileNotFound(full_path.display().to_string()));
        }

        let kind = server
            .or_else(|| self.detect_server_for_path(&full_path))
            .unwrap_or(LspServerKind::RustAnalyzer);

        let session = self.get_or_start_session(kind)?;
        let uri = path_to_uri(&full_path)?;
        let lsp_pos = LspPosition::from_one_indexed(line, col);

        // Notify didOpen if needed
        if let Ok(content) = fs::read_to_string(&full_path) {
            let _ = session.notify(
                "textDocument/didOpen",
                json!({
                    "textDocument": {
                        "uri": uri,
                        "languageId": "plaintext",
                        "version": 1,
                        "text": content
                    }
                }),
            );
        }

        let params = json!({
            "textDocument": { "uri": uri },
            "position": {
                "line": lsp_pos.line,
                "character": lsp_pos.character
            }
        });

        let resp = session.request(
            "textDocument/definition",
            params,
            self.config.request_timeout,
        )?;
        let locations = parse_locations(&resp, &self.workspace_root);
        Ok(LspDefinitionResult { locations })
    }

    /// Queries symbol references (`textDocument/references`).
    pub fn find_references(
        &self,
        file: &Path,
        line: u32,
        col: u32,
        include_declaration: bool,
        server: Option<LspServerKind>,
    ) -> Result<LspReferencesResult, LspError> {
        let full_path = if file.is_absolute() {
            file.to_path_buf()
        } else {
            self.workspace_root.join(file)
        };

        if !full_path.exists() {
            return Err(LspError::FileNotFound(full_path.display().to_string()));
        }

        let kind = server
            .or_else(|| self.detect_server_for_path(&full_path))
            .unwrap_or(LspServerKind::RustAnalyzer);

        let session = self.get_or_start_session(kind)?;
        let uri = path_to_uri(&full_path)?;
        let lsp_pos = LspPosition::from_one_indexed(line, col);

        if let Ok(content) = fs::read_to_string(&full_path) {
            let _ = session.notify(
                "textDocument/didOpen",
                json!({
                    "textDocument": {
                        "uri": uri,
                        "languageId": "plaintext",
                        "version": 1,
                        "text": content
                    }
                }),
            );
        }

        let params = json!({
            "textDocument": { "uri": uri },
            "position": {
                "line": lsp_pos.line,
                "character": lsp_pos.character
            },
            "context": {
                "includeDeclaration": include_declaration
            }
        });

        let resp = session.request(
            "textDocument/references",
            params,
            self.config.request_timeout,
        )?;
        let references = parse_locations(&resp, &self.workspace_root);
        Ok(LspReferencesResult { references })
    }

    /// Queries type definition (`textDocument/typeDefinition`).
    pub fn type_definition(
        &self,
        file: &Path,
        line: u32,
        col: u32,
        server: Option<LspServerKind>,
    ) -> Result<LspTypeDefinitionResult, LspError> {
        let full_path = if file.is_absolute() {
            file.to_path_buf()
        } else {
            self.workspace_root.join(file)
        };

        if !full_path.exists() {
            return Err(LspError::FileNotFound(full_path.display().to_string()));
        }

        let kind = server
            .or_else(|| self.detect_server_for_path(&full_path))
            .unwrap_or(LspServerKind::RustAnalyzer);

        let session = self.get_or_start_session(kind)?;
        let uri = path_to_uri(&full_path)?;
        let lsp_pos = LspPosition::from_one_indexed(line, col);

        if let Ok(content) = fs::read_to_string(&full_path) {
            let _ = session.notify(
                "textDocument/didOpen",
                json!({
                    "textDocument": {
                        "uri": uri,
                        "languageId": "plaintext",
                        "version": 1,
                        "text": content
                    }
                }),
            );
        }

        let params = json!({
            "textDocument": { "uri": uri },
            "position": {
                "line": lsp_pos.line,
                "character": lsp_pos.character
            }
        });

        let resp = session.request(
            "textDocument/typeDefinition",
            params,
            self.config.request_timeout,
        )?;
        let locations = parse_locations(&resp, &self.workspace_root);
        Ok(LspTypeDefinitionResult { locations })
    }

    /// Extracts real-time compiler diagnostics (`textDocument/publishDiagnostics`).
    pub fn diagnostics(
        &self,
        file: Option<&Path>,
        min_severity: Option<LspDiagnosticSeverity>,
        server: Option<LspServerKind>,
    ) -> Result<LspDiagnosticsResult, LspError> {
        let target_uri = match file {
            Some(f) => {
                let full = if f.is_absolute() {
                    f.to_path_buf()
                } else {
                    self.workspace_root.join(f)
                };
                if full.exists() {
                    let uri = path_to_uri(&full)?;
                    // Trigger didOpen
                    if let Ok(content) = fs::read_to_string(&full) {
                        let kind = server
                            .clone()
                            .or_else(|| self.detect_server_for_path(&full))
                            .unwrap_or(LspServerKind::RustAnalyzer);
                        if let Ok(session) = self.get_or_start_session(kind) {
                            let _ = session.notify(
                                "textDocument/didOpen",
                                json!({
                                    "textDocument": {
                                        "uri": uri,
                                        "languageId": "plaintext",
                                        "version": 1,
                                        "text": content
                                    }
                                }),
                            );
                        }
                    }
                    Some(uri)
                } else {
                    return Err(LspError::FileNotFound(full.display().to_string()));
                }
            }
            None => None,
        };

        let mut all_diags = Vec::new();
        let sessions = self
            .active_sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner());

        for session in sessions.values() {
            let diags = session.diagnostics_for(target_uri.as_deref());
            all_diags.extend(diags);
        }

        // If target_uri was specified and no sessions are active, try to start one
        if all_diags.is_empty() && target_uri.is_some() {
            let kind = server.unwrap_or(LspServerKind::RustAnalyzer);
            drop(sessions);
            if let Ok(session) = self.get_or_start_session(kind) {
                let diags = session.diagnostics_for(target_uri.as_deref());
                all_diags.extend(diags);
            }
        }

        if let Some(severity) = min_severity {
            all_diags.retain(|d| d.severity <= severity);
        }

        Ok(LspDiagnosticsResult::from_diagnostics(all_diags))
    }

    /// Gracefully terminates all active language server sessions.
    pub fn shutdown_all(&self) {
        let mut sessions = self
            .active_sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        for session in sessions.values() {
            let _ = session.shutdown();
        }
        sessions.clear();
    }
}

impl Drop for LspEngine {
    fn drop(&mut self) {
        self.shutdown_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lsp_framing_roundtrip() {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 42,
            "method": "textDocument/definition",
            "params": {"position": {"line": 10, "character": 5}}
        });

        let framed = format_lsp_message(&payload).expect("framing should succeed");
        let header_prefix = b"Content-Length: ";
        assert!(framed.starts_with(header_prefix));

        let mut cursor = std::io::Cursor::new(framed);
        let parsed = read_lsp_message(&mut cursor)
            .expect("read should succeed")
            .expect("should yield message");

        assert_eq!(parsed["id"], 42);
        assert_eq!(parsed["method"], "textDocument/definition");
    }

    #[test]
    fn test_mock_lsp_session_definition_and_references() {
        let temp_dir =
            std::env::temp_dir().join(format!("pandora_lsp_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);

        let test_file = temp_dir.join("main.rs");
        let code = r#"
fn calculate_total(amount: u32) -> u32 {
    amount * 2
}

fn main() {
    let result = calculate_total(21);
    println!("{}", result);
}
"#;
        fs::write(&test_file, code).expect("write test file");

        let engine = LspEngine::mock(&temp_dir);
        let def = engine
            .goto_definition(&test_file, 7, 20, Some(LspServerKind::RustAnalyzer))
            .expect("goto_definition");

        assert!(
            !def.locations.is_empty(),
            "should find definition of calculate_total"
        );
        assert_eq!(def.locations[0].range.start.line, 1);

        let refs = engine
            .find_references(&test_file, 2, 5, true, Some(LspServerKind::RustAnalyzer))
            .expect("find_references");

        assert!(
            refs.references.len() >= 2,
            "should find calculate_total at definition and callsite"
        );

        let type_def = engine
            .type_definition(&test_file, 8, 20, Some(LspServerKind::RustAnalyzer))
            .expect("type_definition");
        assert!(
            !type_def.locations.is_empty(),
            "should find type definition"
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_mock_lsp_diagnostics_detection() {
        let temp_dir =
            std::env::temp_dir().join(format!("pandora_lsp_diag_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);

        let test_file = temp_dir.join("broken.rs");
        let code = r#"
fn bad() {
    // SYNTAX_ERROR
    invalid syntax here!
}
"#;
        fs::write(&test_file, code).expect("write test file");

        let engine = LspEngine::mock(&temp_dir);
        let diags = engine
            .diagnostics(Some(&test_file), None, Some(LspServerKind::RustAnalyzer))
            .expect("diagnostics");

        assert!(diags.has_errors());
        assert_eq!(diags.errors, 1);
        assert!(diags.diagnostics[0].message.contains("syntax error"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_lsp_server_detection_and_statuses() {
        assert_eq!(
            LspServerKind::detect_for_extension("rs"),
            Some(LspServerKind::RustAnalyzer)
        );
        assert_eq!(
            LspServerKind::detect_for_extension("ts"),
            Some(LspServerKind::TsServer)
        );
        assert_eq!(
            LspServerKind::detect_for_extension("py"),
            Some(LspServerKind::Pyright)
        );
        assert_eq!(
            LspServerKind::detect_for_extension("go"),
            Some(LspServerKind::Gopls)
        );

        let engine = LspEngine::mock(std::env::temp_dir());
        let statuses = engine.all_server_statuses();
        assert_eq!(statuses.len(), 4);
        for status in statuses {
            assert!(status.available);
            assert_eq!(status.capabilities.len(), 4);
        }
    }
}
