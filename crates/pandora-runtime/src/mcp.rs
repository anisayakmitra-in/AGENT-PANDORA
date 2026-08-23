use crate::mcp_catalog::{
    McpCatalogError, McpCatalogReservation, McpCatalogRevision, McpCatalogTool, digest_bytes,
};
use crate::{ConsumedPermit, ToolDefinition, ToolEngine, ToolError, ToolPlan};
use pandora_types::{
    Capability, EffectOutcome, EffectReceipt, EffectTarget, Operation, ReceiptId, ResourceScope,
    Timestamp,
};
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

pub const MCP_PROTOCOL_VERSION: &str = "2026-07-28";
pub const MCP_LEGACY_PROTOCOL_VERSION: &str = "2025-11-25";
const MCP_CLIENT_NAME: &str = "pandora-agent";
const MAX_TOOLS: usize = 128;
const MAX_CONTENT_ITEMS: usize = 128;
static NEXT_RECEIPT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpProtocolMode {
    ModernOnly,
    LegacyOnly,
    Auto,
}

impl McpProtocolMode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ModernOnly => "modern_only",
            Self::LegacyOnly => "legacy_only",
            Self::Auto => "auto",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpWireEra {
    Modern,
    Legacy,
}

impl McpWireEra {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Modern => "modern",
            Self::Legacy => "legacy",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct McpLimits {
    timeout: Duration,
    max_frame_bytes: usize,
    max_stderr_bytes: usize,
}

impl Default for McpLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(10),
            max_frame_bytes: 1024 * 1024,
            max_stderr_bytes: 64 * 1024,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpStdioConfig {
    server_id: String,
    program: String,
    arguments: Vec<String>,
    mode: McpProtocolMode,
    limits: McpLimits,
}

impl McpStdioConfig {
    pub fn new(
        server_id: impl Into<String>,
        program: impl Into<String>,
        arguments: Vec<String>,
        mode: McpProtocolMode,
    ) -> Result<Self, McpError> {
        let server_id = server_id.into();
        let program = program.into();
        validate_name(&server_id, 64)?;
        if !Path::new(&program).is_absolute()
            || program.len() > 4096
            || program.chars().any(char::is_control)
            || arguments.len() > 64
            || arguments
                .iter()
                .any(|argument| argument.len() > 4096 || argument.chars().any(char::is_control))
        {
            return Err(McpError::InvalidConfig);
        }
        Ok(Self {
            server_id,
            program,
            arguments,
            mode,
            limits: McpLimits::default(),
        })
    }

    pub fn server_id(&self) -> &str {
        &self.server_id
    }

    pub fn program(&self) -> &str {
        &self.program
    }

    pub const fn mode(&self) -> McpProtocolMode {
        self.mode
    }

    pub(crate) fn authorization_payload(&self, purpose: SpawnPurpose) -> Result<Vec<u8>, McpError> {
        serde_json::to_vec(&json!({
            "server": self.server_id,
            "program": self.program,
            "arguments": self.arguments,
            "mode": self.mode.as_str(),
            "purpose": purpose.as_str(),
        }))
        .map_err(|_| McpError::RequestRejected)
    }

    pub(crate) fn catalog_config_digest(&self) -> Result<String, McpError> {
        let payload = serde_json::to_vec(&json!({
            "server": self.server_id,
            "program": self.program,
            "arguments": self.arguments,
            "mode": self.mode.as_str(),
        }))
        .map_err(|_| McpError::RequestRejected)?;
        Ok(digest_bytes(&payload))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SpawnPurpose {
    Modern,
    ModernProbe,
    Legacy,
}

impl SpawnPurpose {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Modern => "modern",
            Self::ModernProbe => "modern_probe",
            Self::Legacy => "legacy",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum McpError {
    InvalidConfig,
    PermissionDenied,
    SpawnFailed,
    Io,
    FrameTooLarge,
    StderrTooLarge,
    TimedOut,
    UnexpectedEof,
    InvalidUtf8,
    MalformedFrame,
    MalformedMessage,
    UnexpectedServerMessage,
    UnexpectedResponseId,
    DuplicateResponseId,
    Rpc {
        code: i64,
        message: String,
        data: Option<Value>,
    },
    UnsupportedProtocolVersion,
    ToolsCapabilityMissing,
    UnsupportedPagination,
    InvalidToolList,
    InvalidToolName,
    UnsupportedSchema,
    DuplicateTool,
    DuplicateServer,
    UnknownTool,
    InvalidArguments,
    InvalidResult,
    ProcessExited,
    RequestRejected,
    PolicyDenied,
    ApprovalRequired,
    AuthorizationFailed,
    PermitFailed,
    ToolEngine,
}

impl McpError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidConfig => "invalid_config",
            Self::PermissionDenied => "permission_denied",
            Self::SpawnFailed => "spawn_failed",
            Self::Io => "mcp_io",
            Self::FrameTooLarge => "frame_too_large",
            Self::StderrTooLarge => "stderr_too_large",
            Self::TimedOut => "timed_out",
            Self::UnexpectedEof => "unexpected_eof",
            Self::InvalidUtf8 => "invalid_utf8",
            Self::MalformedFrame => "malformed_frame",
            Self::MalformedMessage => "malformed_message",
            Self::UnexpectedServerMessage => "unexpected_server_message",
            Self::UnexpectedResponseId => "unexpected_response_id",
            Self::DuplicateResponseId => "duplicate_response_id",
            Self::Rpc { .. } => "rpc_error",
            Self::UnsupportedProtocolVersion => "unsupported_protocol_version",
            Self::ToolsCapabilityMissing => "tools_capability_missing",
            Self::UnsupportedPagination => "unsupported_pagination",
            Self::InvalidToolList => "invalid_tool_list",
            Self::InvalidToolName => "invalid_tool_name",
            Self::UnsupportedSchema => "unsupported_schema",
            Self::DuplicateTool => "duplicate_tool",
            Self::DuplicateServer => "duplicate_server",
            Self::UnknownTool => "unknown_tool",
            Self::InvalidArguments => "invalid_arguments",
            Self::InvalidResult => "invalid_result",
            Self::ProcessExited => "process_exited",
            Self::RequestRejected => "request_rejected",
            Self::PolicyDenied => "policy_denied",
            Self::ApprovalRequired => "approval_required",
            Self::AuthorizationFailed => "authorization_failed",
            Self::PermitFailed => "permit_failed",
            Self::ToolEngine => "tool_engine",
        }
    }

    fn fatal(&self) -> bool {
        !matches!(
            self,
            Self::PermissionDenied
                | Self::Rpc { .. }
                | Self::DuplicateServer
                | Self::UnknownTool
                | Self::InvalidArguments
                | Self::RequestRejected
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ImportedTool {
    definition: ToolDefinition,
    remote_name: String,
}

fn protocol_request(method: &str, id: u64, mut params: Value) -> Value {
    let params = params
        .as_object_mut()
        .expect("MCP request params are constructed as an object");
    params.insert("_meta".to_owned(), request_metadata());
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    })
}

fn legacy_request(method: &str, id: u64, params: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    })
}

fn legacy_initialize_request(id: u64) -> Value {
    legacy_request(
        "initialize",
        id,
        json!({
            "protocolVersion": MCP_LEGACY_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {
                "name": MCP_CLIENT_NAME,
                "version": env!("CARGO_PKG_VERSION"),
            },
        }),
    )
}

fn legacy_initialized_notification() -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {},
    })
}

fn request_metadata() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": MCP_PROTOCOL_VERSION,
        "io.modelcontextprotocol/clientInfo": {
            "name": MCP_CLIENT_NAME,
            "version": env!("CARGO_PKG_VERSION"),
        },
        "io.modelcontextprotocol/clientCapabilities": {},
    })
}

fn read_frame(reader: &mut impl BufRead, limit: usize) -> Result<String, McpError> {
    let read_limit = limit.checked_add(2).ok_or(McpError::FrameTooLarge)?;
    let mut frame = Vec::with_capacity(limit.min(4096));
    let mut limited = std::io::Read::take(reader, read_limit as u64);
    limited
        .read_until(b'\n', &mut frame)
        .map_err(|_| McpError::Io)?;
    if frame.is_empty() {
        return Err(McpError::UnexpectedEof);
    }
    if frame.last() != Some(&b'\n') {
        return if frame.len() > limit {
            Err(McpError::FrameTooLarge)
        } else {
            Err(McpError::MalformedFrame)
        };
    }
    frame.pop();
    if frame.last() == Some(&b'\r') {
        frame.pop();
    }
    if frame.len() > limit {
        return Err(McpError::FrameTooLarge);
    }
    String::from_utf8(frame).map_err(|_| McpError::InvalidUtf8)
}

fn decode_response(
    response: Value,
    expected_id: u64,
    seen: &mut HashSet<u64>,
) -> Result<Value, McpError> {
    let object = response.as_object().ok_or(McpError::MalformedMessage)?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(McpError::MalformedMessage);
    }
    if object.contains_key("method") {
        return Err(McpError::UnexpectedServerMessage);
    }
    let id = object
        .get("id")
        .and_then(Value::as_u64)
        .ok_or(McpError::MalformedMessage)?;
    if seen.contains(&id) {
        return Err(McpError::DuplicateResponseId);
    }
    if id != expected_id {
        return Err(McpError::UnexpectedResponseId);
    }
    seen.insert(id);
    match (object.get("result"), object.get("error")) {
        (Some(result), None) => Ok(result.clone()),
        (None, Some(error)) => {
            let error = error.as_object().ok_or(McpError::MalformedMessage)?;
            let code = error
                .get("code")
                .and_then(Value::as_i64)
                .ok_or(McpError::MalformedMessage)?;
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .ok_or(McpError::MalformedMessage)?;
            if message.is_empty() || message.len() > 4096 || message.chars().any(char::is_control) {
                return Err(McpError::MalformedMessage);
            }
            Err(McpError::Rpc {
                code,
                message: message.to_owned(),
                data: error.get("data").cloned(),
            })
        }
        _ => Err(McpError::MalformedMessage),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProbeErrorClass {
    ContinueModern,
    Legacy,
    Fail,
}

fn classify_probe_error(error: &McpError) -> ProbeErrorClass {
    let McpError::Rpc {
        code,
        data,
        message: _,
    } = error
    else {
        return ProbeErrorClass::Fail;
    };
    let Some(data) = data.as_ref().and_then(Value::as_object) else {
        return if *code == -32601 {
            ProbeErrorClass::ContinueModern
        } else {
            ProbeErrorClass::Fail
        };
    };
    let supported = data
        .get("supported")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    let concrete_legacy = supported.contains(&MCP_LEGACY_PROTOCOL_VERSION)
        && !supported.contains(&MCP_PROTOCOL_VERSION);
    if data.get("requested").and_then(Value::as_str) == Some(MCP_PROTOCOL_VERSION)
        && concrete_legacy
    {
        return ProbeErrorClass::Legacy;
    }
    if data.get("requiredMethod").and_then(Value::as_str) == Some("initialize") && concrete_legacy {
        return ProbeErrorClass::Legacy;
    }
    if *code == -32601 {
        ProbeErrorClass::ContinueModern
    } else {
        ProbeErrorClass::Fail
    }
}

fn validate_discovery(result: &Value) -> Result<(), McpError> {
    let result = result.as_object().ok_or(McpError::MalformedMessage)?;
    if result.get("resultType").and_then(Value::as_str) != Some("complete") {
        return Err(McpError::InvalidResult);
    }
    let versions = result
        .get("supportedVersions")
        .and_then(Value::as_array)
        .ok_or(McpError::MalformedMessage)?;
    if !versions
        .iter()
        .any(|version| version.as_str() == Some(MCP_PROTOCOL_VERSION))
    {
        return Err(McpError::UnsupportedProtocolVersion);
    }
    if !result
        .get("capabilities")
        .and_then(Value::as_object)
        .is_some_and(|capabilities| capabilities.get("tools").is_some_and(Value::is_object))
    {
        return Err(McpError::ToolsCapabilityMissing);
    }
    Ok(())
}

fn validate_legacy_initialize(result: &Value) -> Result<(), McpError> {
    let result = result.as_object().ok_or(McpError::MalformedMessage)?;
    if result.get("protocolVersion").and_then(Value::as_str) != Some(MCP_LEGACY_PROTOCOL_VERSION) {
        return Err(McpError::UnsupportedProtocolVersion);
    }
    if !result
        .get("capabilities")
        .and_then(Value::as_object)
        .is_some_and(|capabilities| capabilities.get("tools").is_some_and(Value::is_object))
    {
        return Err(McpError::ToolsCapabilityMissing);
    }
    Ok(())
}

fn definitions_from_list_for_era(
    server_id: &str,
    era: McpWireEra,
    result: &Value,
) -> Result<Vec<ImportedTool>, McpError> {
    validate_name(server_id, 64)?;
    let result = result.as_object().ok_or(McpError::InvalidToolList)?;
    match era {
        McpWireEra::Modern
            if result.get("resultType").and_then(Value::as_str) != Some("complete") =>
        {
            return Err(McpError::InvalidResult);
        }
        McpWireEra::Legacy if result.contains_key("resultType") => {
            return Err(McpError::InvalidResult);
        }
        _ => {}
    }
    if result
        .get("nextCursor")
        .is_some_and(|cursor| !cursor.is_null())
    {
        return Err(McpError::UnsupportedPagination);
    }
    let tools = result
        .get("tools")
        .and_then(Value::as_array)
        .ok_or(McpError::InvalidToolList)?;
    if tools.len() > MAX_TOOLS {
        return Err(McpError::InvalidToolList);
    }
    let mut remote_names = HashSet::with_capacity(tools.len());
    let mut imported = Vec::with_capacity(tools.len());
    for tool in tools {
        let tool = tool.as_object().ok_or(McpError::InvalidToolList)?;
        let remote_name = tool
            .get("name")
            .and_then(Value::as_str)
            .ok_or(McpError::InvalidToolName)?;
        validate_name(remote_name, 128)?;
        if !remote_names.insert(remote_name.to_owned()) {
            return Err(McpError::DuplicateTool);
        }
        if tool.contains_key("outputSchema") || tool.contains_key("execution") {
            return Err(McpError::UnsupportedSchema);
        }
        let schema = tool.get("inputSchema").ok_or(McpError::UnsupportedSchema)?;
        validate_schema_subset(schema)?;
        let local_id = format!("mcp.{server_id}.{remote_name}");
        let description = tool
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or(remote_name);
        let definition = ToolDefinition::new(
            local_id,
            "mcp",
            description,
            schema.clone(),
            Capability::McpInvoke,
            Operation::Invoke,
        )
        .map_err(map_tool_error)?;
        imported.push(ImportedTool {
            definition,
            remote_name: remote_name.to_owned(),
        });
    }
    Ok(imported)
}

fn validate_schema_subset(schema: &Value) -> Result<(), McpError> {
    let schema = schema.as_object().ok_or(McpError::UnsupportedSchema)?;
    let allowed_root = ["type", "properties", "required", "additionalProperties"];
    if schema
        .keys()
        .any(|key| !allowed_root.contains(&key.as_str()))
        || schema.get("type").and_then(Value::as_str) != Some("object")
    {
        return Err(McpError::UnsupportedSchema);
    }
    if schema
        .get("additionalProperties")
        .is_some_and(|value| !value.is_boolean())
    {
        return Err(McpError::UnsupportedSchema);
    }
    let empty = Map::new();
    let properties = match schema.get("properties") {
        Some(value) => value.as_object().ok_or(McpError::UnsupportedSchema)?,
        None => &empty,
    };
    for (name, property) in properties {
        if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
            return Err(McpError::UnsupportedSchema);
        }
        let property = property.as_object().ok_or(McpError::UnsupportedSchema)?;
        if property
            .keys()
            .any(|key| !matches!(key.as_str(), "type" | "description"))
        {
            return Err(McpError::UnsupportedSchema);
        }
        let property_type = property
            .get("type")
            .and_then(Value::as_str)
            .ok_or(McpError::UnsupportedSchema)?;
        if !matches!(
            property_type,
            "array" | "boolean" | "integer" | "null" | "number" | "object" | "string"
        ) || property
            .get("description")
            .is_some_and(|value| !value.is_string())
        {
            return Err(McpError::UnsupportedSchema);
        }
    }
    if let Some(required) = schema.get("required") {
        let required = required.as_array().ok_or(McpError::UnsupportedSchema)?;
        let mut names = HashSet::with_capacity(required.len());
        if required.iter().any(|value| {
            let Some(name) = value.as_str() else {
                return true;
            };
            !properties.contains_key(name) || !names.insert(name)
        }) {
            return Err(McpError::UnsupportedSchema);
        }
    }
    Ok(())
}

fn validate_name(value: &str, max_len: usize) -> Result<(), McpError> {
    if value.is_empty()
        || value.len() > max_len
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(McpError::InvalidToolName);
    }
    Ok(())
}

pub(crate) fn map_tool_error(error: ToolError) -> McpError {
    match error {
        ToolError::DuplicateTool => McpError::DuplicateTool,
        ToolError::UnknownTool(_) => McpError::UnknownTool,
        ToolError::InvalidArguments(_) => McpError::InvalidArguments,
        ToolError::InvalidSchema(_) => McpError::UnsupportedSchema,
        ToolError::Request(_) | ToolError::InvalidIdempotencyKey => McpError::RequestRejected,
        ToolError::UnsupportedTool(_) | ToolError::IdempotencyConflict => McpError::ToolEngine,
    }
}

fn validate_tool_result(era: McpWireEra, result: Value) -> Result<McpToolResult, McpError> {
    let object = result.as_object().ok_or(McpError::InvalidResult)?;
    match era {
        McpWireEra::Modern
            if object.get("resultType").and_then(Value::as_str) != Some("complete") =>
        {
            return Err(McpError::InvalidResult);
        }
        McpWireEra::Legacy if object.contains_key("resultType") => {
            return Err(McpError::InvalidResult);
        }
        _ => {}
    }
    let content = object
        .get("content")
        .and_then(Value::as_array)
        .ok_or(McpError::InvalidResult)?;
    if content.len() > MAX_CONTENT_ITEMS
        || object
            .get("isError")
            .is_some_and(|value| !value.is_boolean())
    {
        return Err(McpError::InvalidResult);
    }
    for item in content {
        let item = item.as_object().ok_or(McpError::InvalidResult)?;
        let valid = match item.get("type").and_then(Value::as_str) {
            Some("text") => item.get("text").is_some_and(Value::is_string),
            Some("image") | Some("audio") => {
                item.get("data").is_some_and(Value::is_string)
                    && item.get("mimeType").is_some_and(Value::is_string)
            }
            Some("resource_link") => {
                item.get("uri").is_some_and(Value::is_string)
                    && item.get("name").is_some_and(Value::is_string)
            }
            _ => false,
        };
        if !valid {
            return Err(McpError::InvalidResult);
        }
    }
    let is_error = object
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(McpToolResult {
        value: result,
        is_error,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpToolResult {
    value: Value,
    is_error: bool,
}

impl McpToolResult {
    pub fn value(&self) -> &Value {
        &self.value
    }

    pub const fn is_error(&self) -> bool {
        self.is_error
    }
}

struct McpProcess {
    child: Child,
    stdin: Option<BufWriter<ChildStdin>>,
    stdout: Receiver<Result<String, McpError>>,
    stderr: Receiver<Result<(), McpError>>,
    seen_response_ids: HashSet<u64>,
    next_request_id: u64,
    limits: McpLimits,
}

impl McpProcess {
    fn spawn(config: &McpStdioConfig) -> Result<Self, McpError> {
        let mut command = Command::new(&config.program);
        command
            .args(&config.arguments)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().map_err(|_| McpError::SpawnFailed)?;
        let stdin = child.stdin.take().ok_or(McpError::SpawnFailed)?;
        let stdout = child.stdout.take().ok_or(McpError::SpawnFailed)?;
        let stderr = child.stderr.take().ok_or(McpError::SpawnFailed)?;
        let (stdout_sender, stdout_receiver) = mpsc::channel();
        let frame_limit = config.limits.max_frame_bytes;
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let result = read_frame(&mut reader, frame_limit);
                let terminal = result.is_err();
                if stdout_sender.send(result).is_err() || terminal {
                    break;
                }
            }
        });
        let (stderr_sender, stderr_receiver) = mpsc::channel();
        let stderr_limit = config.limits.max_stderr_bytes;
        thread::spawn(move || {
            let result = read_stderr(stderr, stderr_limit);
            let _ = stderr_sender.send(result);
        });
        Ok(Self {
            child,
            stdin: Some(BufWriter::new(stdin)),
            stdout: stdout_receiver,
            stderr: stderr_receiver,
            seen_response_ids: HashSet::new(),
            next_request_id: 1,
            limits: config.limits.clone(),
        })
    }

    fn request(&mut self, era: McpWireEra, method: &str, params: Value) -> Result<Value, McpError> {
        let id = self.allocate_request_id()?;
        let request = match era {
            McpWireEra::Modern => protocol_request(method, id, params),
            McpWireEra::Legacy => legacy_request(method, id, params),
        };
        self.send_request(request, id)
    }

    fn initialize_legacy(&mut self) -> Result<Value, McpError> {
        let id = self.allocate_request_id()?;
        self.send_request(legacy_initialize_request(id), id)
    }

    fn notify_initialized(&mut self) -> Result<(), McpError> {
        self.write_message(&legacy_initialized_notification())
    }

    fn allocate_request_id(&mut self) -> Result<u64, McpError> {
        let id = self.next_request_id;
        self.next_request_id = id.checked_add(1).ok_or(McpError::RequestRejected)?;
        Ok(id)
    }

    fn send_request(&mut self, request: Value, expected_id: u64) -> Result<Value, McpError> {
        self.write_message(&request)?;
        let started = Instant::now();
        loop {
            match self.stdout.try_recv() {
                Ok(Ok(frame)) => {
                    let message =
                        serde_json::from_str(&frame).map_err(|_| McpError::MalformedMessage)?;
                    return decode_response(message, expected_id, &mut self.seen_response_ids);
                }
                Ok(Err(error)) => return Err(error),
                Err(TryRecvError::Disconnected) => return Err(McpError::UnexpectedEof),
                Err(TryRecvError::Empty) => {}
            }
            match self.stderr.try_recv() {
                Ok(Err(error)) => return Err(error),
                Ok(Ok(())) | Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {}
            }
            if started.elapsed() >= self.limits.timeout {
                return Err(McpError::TimedOut);
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn write_message(&mut self, message: &Value) -> Result<(), McpError> {
        let bytes = serde_json::to_vec(message).map_err(|_| McpError::RequestRejected)?;
        if bytes.len() > self.limits.max_frame_bytes {
            return Err(McpError::FrameTooLarge);
        }
        let stdin = self.stdin.as_mut().ok_or(McpError::ProcessExited)?;
        stdin.write_all(&bytes).map_err(|_| McpError::Io)?;
        stdin.write_all(b"\n").map_err(|_| McpError::Io)?;
        stdin.flush().map_err(|_| McpError::Io)
    }

    fn terminate(&mut self) {
        self.stdin.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    fn id(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for McpProcess {
    fn drop(&mut self) {
        self.terminate();
    }
}

fn read_stderr(stderr: impl Read, limit: usize) -> Result<(), McpError> {
    let read_limit = limit.checked_add(1).ok_or(McpError::StderrTooLarge)?;
    let mut bytes = Vec::with_capacity(limit.min(4096));
    stderr
        .take(read_limit as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| McpError::Io)?;
    if bytes.len() > limit {
        return Err(McpError::StderrTooLarge);
    }
    Ok(())
}

pub struct McpServer {
    server_id: String,
    era: McpWireEra,
    process: McpProcess,
    remote_names: HashMap<String, String>,
    tool_engine: ToolEngine,
    imported_ids: Vec<pandora_types::GeneId>,
    catalog_revision: McpCatalogRevision,
    catalog_reservation: Option<McpCatalogReservation>,
    active: bool,
}

impl fmt::Debug for McpServer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpServer")
            .field("server_id", &self.server_id)
            .field("era", &self.era)
            .field("catalog_revision", &self.catalog_revision)
            .field("active", &self.active)
            .finish_non_exhaustive()
    }
}

impl McpServer {
    pub fn server_id(&self) -> &str {
        &self.server_id
    }

    pub const fn era(&self) -> McpWireEra {
        self.era
    }

    pub fn catalog_revision(&self) -> &McpCatalogRevision {
        &self.catalog_revision
    }

    pub(crate) fn remote_tool(&self, local_tool: &str) -> Result<&str, McpError> {
        self.remote_names
            .get(local_tool)
            .map(String::as_str)
            .ok_or(McpError::UnknownTool)
    }

    pub(crate) fn invocation_payload(
        &self,
        local_tool: &str,
        arguments: &Value,
    ) -> Result<Vec<u8>, McpError> {
        let remote_tool = self.remote_tool(local_tool)?;
        let tool = self
            .catalog_revision
            .tool(local_tool)
            .ok_or(McpError::UnknownTool)?;
        if tool.remote_name() != remote_tool {
            return Err(McpError::RequestRejected);
        }
        serde_json::to_vec(&json!({
            "server": self.catalog_revision.server_id(),
            "generation": self.catalog_revision.generation(),
            "protocol_era": self.catalog_revision.protocol_era().as_str(),
            "process_id": self.catalog_revision.process_id(),
            "config_digest": self.catalog_revision.config_digest(),
            "catalog_digest": self.catalog_revision.catalog_digest(),
            "local_tool": local_tool,
            "remote_tool": remote_tool,
            "schema_digest": tool.schema_digest(),
            "arguments": arguments,
        }))
        .map_err(|_| McpError::RequestRejected)
    }

    fn terminate(&mut self) {
        if self.active {
            self.active = false;
            self.process.terminate();
            self.tool_engine.unregister_batch(&self.imported_ids);
            self.catalog_reservation.take();
        }
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[derive(Debug)]
pub(crate) enum McpStartOutcome {
    Connected(Box<McpServer>),
    LegacyIdentified,
}

pub(crate) struct McpExecutorResult<T> {
    result: Result<T, McpError>,
    receipt: EffectReceipt,
}

impl<T> McpExecutorResult<T> {
    #[cfg(test)]
    pub(crate) fn result(&self) -> Result<&T, &McpError> {
        self.result.as_ref()
    }

    pub(crate) fn into_parts(self) -> (Result<T, McpError>, EffectReceipt) {
        (self.result, self.receipt)
    }
}

pub(crate) struct McpExecutor;

impl McpExecutor {
    pub(crate) fn start_modern(
        permit: &ConsumedPermit,
        tool_engine: ToolEngine,
        config: McpStdioConfig,
        catalog_reservation: &McpCatalogReservation,
        purpose: SpawnPurpose,
        allow_legacy: bool,
        now: Timestamp,
    ) -> McpExecutorResult<McpStartOutcome> {
        let result = Self::start_modern_inner(
            permit,
            tool_engine,
            config,
            catalog_reservation,
            purpose,
            allow_legacy,
        );
        executor_result(permit, now, result)
    }

    fn start_modern_inner(
        permit: &ConsumedPermit,
        tool_engine: ToolEngine,
        config: McpStdioConfig,
        catalog_reservation: &McpCatalogReservation,
        purpose: SpawnPurpose,
        allow_legacy: bool,
    ) -> Result<McpStartOutcome, McpError> {
        recheck_spawn(permit, &config, purpose)?;
        let mut process = McpProcess::spawn(&config)?;
        let discovery = process.request(McpWireEra::Modern, "server/discover", json!({}));
        match discovery {
            Ok(result) => {
                if let Err(error) = validate_discovery(&result) {
                    process.terminate();
                    return Err(error);
                }
            }
            Err(error) => match classify_probe_error(&error) {
                ProbeErrorClass::ContinueModern => {}
                ProbeErrorClass::Legacy if allow_legacy => {
                    process.terminate();
                    return Ok(McpStartOutcome::LegacyIdentified);
                }
                ProbeErrorClass::Legacy | ProbeErrorClass::Fail => {
                    process.terminate();
                    return Err(error);
                }
            },
        }
        let listed = match process.request(McpWireEra::Modern, "tools/list", json!({})) {
            Ok(result) => result,
            Err(error) => {
                process.terminate();
                return Err(error);
            }
        };
        import_server(
            tool_engine,
            config.server_id,
            McpWireEra::Modern,
            process,
            &listed,
            catalog_reservation,
        )
        .map(|server| McpStartOutcome::Connected(Box::new(server)))
    }

    pub(crate) fn start_legacy(
        permit: &ConsumedPermit,
        tool_engine: ToolEngine,
        config: McpStdioConfig,
        catalog_reservation: &McpCatalogReservation,
        now: Timestamp,
    ) -> McpExecutorResult<McpServer> {
        let result = Self::start_legacy_inner(permit, tool_engine, config, catalog_reservation);
        executor_result(permit, now, result)
    }

    fn start_legacy_inner(
        permit: &ConsumedPermit,
        tool_engine: ToolEngine,
        config: McpStdioConfig,
        catalog_reservation: &McpCatalogReservation,
    ) -> Result<McpServer, McpError> {
        recheck_spawn(permit, &config, SpawnPurpose::Legacy)?;
        let mut process = McpProcess::spawn(&config)?;
        let initialized = match process.initialize_legacy() {
            Ok(result) => result,
            Err(error) => {
                process.terminate();
                return Err(error);
            }
        };
        if let Err(error) = validate_legacy_initialize(&initialized) {
            process.terminate();
            return Err(error);
        }
        if let Err(error) = process.notify_initialized() {
            process.terminate();
            return Err(error);
        }
        let listed = match process.request(McpWireEra::Legacy, "tools/list", json!({})) {
            Ok(result) => result,
            Err(error) => {
                process.terminate();
                return Err(error);
            }
        };
        import_server(
            tool_engine,
            config.server_id,
            McpWireEra::Legacy,
            process,
            &listed,
            catalog_reservation,
        )
    }

    pub(crate) fn invoke(
        permit: &ConsumedPermit,
        server: &mut McpServer,
        plan: &ToolPlan,
        now: Timestamp,
    ) -> McpExecutorResult<McpToolResult> {
        let result = Self::invoke_inner(permit, server, plan);
        if result.as_ref().is_err_and(McpError::fatal) {
            server.terminate();
        }
        executor_result(permit, now, result)
    }

    fn invoke_inner(
        permit: &ConsumedPermit,
        server: &mut McpServer,
        plan: &ToolPlan,
    ) -> Result<McpToolResult, McpError> {
        let remote_name = server.remote_tool(plan.tool_id().as_str())?.to_owned();
        recheck_invocation(permit, server, plan, &remote_name)?;
        let result = server.process.request(
            server.era,
            "tools/call",
            json!({
                "name": remote_name,
                "arguments": plan.arguments(),
            }),
        )?;
        validate_tool_result(server.era, result)
    }
}

fn import_server(
    tool_engine: ToolEngine,
    server_id: String,
    era: McpWireEra,
    mut process: McpProcess,
    listed: &Value,
    catalog_reservation: &McpCatalogReservation,
) -> Result<McpServer, McpError> {
    let imported = match definitions_from_list_for_era(&server_id, era, listed) {
        Ok(imported) => imported,
        Err(error) => {
            process.terminate();
            return Err(error);
        }
    };
    let imported_ids = imported
        .iter()
        .map(|tool| tool.definition.id().clone())
        .collect::<Vec<_>>();
    let remote_names = imported
        .iter()
        .map(|tool| {
            (
                tool.definition.id().as_str().to_owned(),
                tool.remote_name.clone(),
            )
        })
        .collect::<HashMap<_, _>>();
    let catalog_tools = imported
        .iter()
        .map(|tool| {
            McpCatalogTool::new(
                tool.definition.id().as_str(),
                &tool.remote_name,
                tool.definition.input_schema(),
            )
            .map_err(map_catalog_error)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let definitions = imported
        .into_iter()
        .map(|tool| tool.definition)
        .collect::<Vec<_>>();
    if let Err(error) = tool_engine.register_batch(definitions) {
        process.terminate();
        return Err(map_tool_error(error));
    }
    let catalog_revision = match catalog_reservation.activate(era, process.id(), catalog_tools) {
        Ok(revision) => revision,
        Err(error) => {
            tool_engine.unregister_batch(&imported_ids);
            process.terminate();
            return Err(map_catalog_error(error));
        }
    };
    Ok(McpServer {
        server_id,
        era,
        process,
        remote_names,
        tool_engine,
        imported_ids,
        catalog_revision,
        catalog_reservation: Some(catalog_reservation.clone()),
        active: true,
    })
}

pub(crate) fn map_catalog_error(error: McpCatalogError) -> McpError {
    match error {
        McpCatalogError::AlreadyActive => McpError::DuplicateServer,
        McpCatalogError::AlreadyActivated
        | McpCatalogError::InvalidIdentity
        | McpCatalogError::GenerationExhausted
        | McpCatalogError::ReservationLost => McpError::RequestRejected,
    }
}

fn recheck_spawn(
    permit: &ConsumedPermit,
    config: &McpStdioConfig,
    purpose: SpawnPurpose,
) -> Result<(), McpError> {
    let request = permit.request();
    let payload = config.authorization_payload(purpose)?;
    if request.capability() != Capability::ProcessExecute
        || request.operation() != Operation::Execute
        || request.target() != &EffectTarget::process(config.program.clone())
        || request.resource_scope() != &ResourceScope::none()
        || !request.payload_digest_matches(&payload)
    {
        return Err(McpError::PermissionDenied);
    }
    Ok(())
}

fn recheck_invocation(
    permit: &ConsumedPermit,
    server: &McpServer,
    plan: &ToolPlan,
    remote_name: &str,
) -> Result<(), McpError> {
    let request = permit.request();
    let payload = server.invocation_payload(plan.tool_id().as_str(), plan.arguments())?;
    if request != plan.request()
        || request.capability() != Capability::McpInvoke
        || request.operation() != Operation::Invoke
        || request.target() != &EffectTarget::mcp(&server.server_id, remote_name)
        || request.resource_scope() != &ResourceScope::none()
        || !request.payload_digest_matches(&payload)
    {
        return Err(McpError::PermissionDenied);
    }
    Ok(())
}

fn executor_result<T>(
    permit: &ConsumedPermit,
    now: Timestamp,
    result: Result<T, McpError>,
) -> McpExecutorResult<T> {
    let outcome = match &result {
        Ok(_) => EffectOutcome::Succeeded,
        Err(error) => EffectOutcome::Failed {
            code: error.code().to_owned(),
        },
    };
    let receipt_id = ReceiptId::new(format!(
        "receipt-mcp-{}",
        NEXT_RECEIPT_ID.fetch_add(1, Ordering::Relaxed)
    ))
    .expect("generated receipt ID is valid");
    McpExecutorResult {
        result,
        receipt: EffectReceipt::new(
            receipt_id,
            permit.permit().permit_id().clone(),
            permit.permit().request_digest().clone(),
            now,
            outcome,
        ),
    }
}

pub struct McpStart {
    server: McpServer,
    selected_era: McpWireEra,
    downgraded: bool,
    receipts: Vec<EffectReceipt>,
    events: Vec<pandora_types::RuntimeEvent>,
}

impl fmt::Debug for McpStart {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpStart")
            .field("selected_era", &self.selected_era)
            .field("downgraded", &self.downgraded)
            .field("receipts", &self.receipts)
            .field("events", &self.events)
            .finish_non_exhaustive()
    }
}

impl McpStart {
    pub(crate) fn new(
        server: McpServer,
        selected_era: McpWireEra,
        downgraded: bool,
        receipts: Vec<EffectReceipt>,
        events: Vec<pandora_types::RuntimeEvent>,
    ) -> Self {
        Self {
            server,
            selected_era,
            downgraded,
            receipts,
            events,
        }
    }

    pub const fn selected_era(&self) -> McpWireEra {
        self.selected_era
    }

    pub const fn downgraded(&self) -> bool {
        self.downgraded
    }

    pub fn receipts(&self) -> &[EffectReceipt] {
        &self.receipts
    }

    pub fn events(&self) -> &[pandora_types::RuntimeEvent] {
        &self.events
    }

    pub fn into_server(self) -> McpServer {
        self.server
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpFailure {
    error: McpError,
    receipts: Vec<EffectReceipt>,
    events: Vec<pandora_types::RuntimeEvent>,
}

impl McpFailure {
    pub(crate) fn new(
        error: McpError,
        receipts: Vec<EffectReceipt>,
        events: Vec<pandora_types::RuntimeEvent>,
    ) -> Self {
        Self {
            error,
            receipts,
            events,
        }
    }

    pub fn error(&self) -> &McpError {
        &self.error
    }

    pub fn receipts(&self) -> &[EffectReceipt] {
        &self.receipts
    }

    pub fn events(&self) -> &[pandora_types::RuntimeEvent] {
        &self.events
    }

    pub fn event_types(&self) -> Vec<pandora_types::EventType> {
        self.events.iter().map(|event| event.event_type()).collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpInvocation {
    result: McpToolResult,
    receipts: Vec<EffectReceipt>,
    events: Vec<pandora_types::RuntimeEvent>,
}

impl McpInvocation {
    pub(crate) fn new(
        result: McpToolResult,
        receipts: Vec<EffectReceipt>,
        events: Vec<pandora_types::RuntimeEvent>,
    ) -> Self {
        Self {
            result,
            receipts,
            events,
        }
    }

    pub fn result(&self) -> &McpToolResult {
        &self.result
    }

    pub fn receipts(&self) -> &[EffectReceipt] {
        &self.receipts
    }

    pub fn events(&self) -> &[pandora_types::RuntimeEvent] {
        &self.events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executors::WorkspaceRoot;
    use crate::{ExecutionController, Parliament, ReferenceMonitor, ToolContext, ToolEngine};
    use pandora_types::{
        Capability, EffectTarget, EventPayload, EventType, ExecutionId, GeneId, Operation,
        OperationRequest, PolicyContext, PrincipalId, ResourceScope, Session, SessionId, TenantId,
        Timestamp, WorkspaceId,
    };
    use serde_json::json;
    use std::collections::HashSet;
    use std::io::Cursor;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::OnceLock;
    use std::time::Duration;

    #[test]
    fn protocol_requests_carry_modern_metadata() {
        let request = protocol_request("tools/list", 7, json!({}));

        assert_eq!(request["jsonrpc"], "2.0");
        assert_eq!(request["id"], 7);
        assert_eq!(request["method"], "tools/list");
        assert_eq!(
            request["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"],
            "2026-07-28"
        );
        assert_eq!(
            request["params"]["_meta"]["io.modelcontextprotocol/clientInfo"]["name"],
            "pandora-agent"
        );
        assert!(
            request["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"].is_object()
        );
    }

    #[test]
    fn protocol_discovery_requires_the_pinned_version_and_tools() {
        assert_eq!(
            validate_discovery(&json!({
                "resultType": "complete",
                "supportedVersions": ["2026-07-28"],
                "capabilities": {"tools": {}}
            })),
            Ok(())
        );
        assert_eq!(
            validate_discovery(&json!({
                "resultType": "complete",
                "supportedVersions": ["2025-11-25"],
                "capabilities": {"tools": {}}
            })),
            Err(McpError::UnsupportedProtocolVersion)
        );
        assert_eq!(
            validate_discovery(&json!({
                "resultType": "complete",
                "supportedVersions": ["2026-07-28"],
                "capabilities": {}
            })),
            Err(McpError::ToolsCapabilityMissing)
        );
        assert_eq!(
            validate_discovery(&json!({
                "resultType": "complete",
                "supportedVersions": ["2025-11-25", "2026-07-28"],
                "capabilities": {"tools": {}}
            })),
            Ok(())
        );
    }

    #[test]
    fn protocol_frames_are_bounded_utf8_and_newline_delimited() {
        let mut exact = Cursor::new(b"1234\n".to_vec());
        let mut oversized = Cursor::new(b"12345\n".to_vec());
        let mut non_utf8 = Cursor::new(vec![0xff, b'\n']);
        let mut unterminated = Cursor::new(b"1234".to_vec());

        assert_eq!(read_frame(&mut exact, 4), Ok("1234".to_owned()));
        assert_eq!(read_frame(&mut oversized, 4), Err(McpError::FrameTooLarge));
        assert_eq!(read_frame(&mut non_utf8, 4), Err(McpError::InvalidUtf8));
        assert_eq!(
            read_frame(&mut unterminated, 4),
            Err(McpError::MalformedFrame)
        );
    }

    #[test]
    fn protocol_response_ids_must_match_once() {
        let mut seen = HashSet::new();
        let response = json!({"jsonrpc": "2.0", "id": 3, "result": {}});

        assert_eq!(
            decode_response(response.clone(), 3, &mut seen),
            Ok(json!({}))
        );
        assert_eq!(
            decode_response(response, 4, &mut seen),
            Err(McpError::DuplicateResponseId)
        );
        assert_eq!(
            decode_response(
                json!({"jsonrpc": "2.0", "id": 9, "result": {}}),
                4,
                &mut seen
            ),
            Err(McpError::UnexpectedResponseId)
        );
        assert_eq!(
            decode_response(
                json!({"jsonrpc": "2.0", "method": "sampling/createMessage"}),
                4,
                &mut seen
            ),
            Err(McpError::UnexpectedServerMessage)
        );
    }

    #[test]
    fn protocol_tool_import_rejects_unsupported_schema_constructs_atomically() {
        let result = definitions_from_list_for_era(
            "local",
            McpWireEra::Modern,
            &json!({
                "resultType": "complete",
                "tools": [
                    {
                        "name": "valid",
                        "description": "Valid tool",
                        "inputSchema": {
                            "type": "object",
                            "properties": {"value": {"type": "string"}},
                            "required": ["value"],
                            "additionalProperties": false
                        }
                    },
                    {
                        "name": "unsupported",
                        "description": "Unsupported tool",
                        "inputSchema": {
                            "type": "object",
                            "oneOf": [{"required": ["value"]}]
                        }
                    }
                ]
            }),
        );

        assert_eq!(result, Err(McpError::UnsupportedSchema));
    }

    #[test]
    fn protocol_rpc_errors_preserve_structured_fallback_evidence() {
        let mut seen = HashSet::new();
        let error = decode_response(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "error": {
                    "code": -32022,
                    "message": "Unsupported protocol version",
                    "data": {
                        "requested": "2026-07-28",
                        "supported": ["2025-11-25"]
                    }
                }
            }),
            1,
            &mut seen,
        )
        .unwrap_err();

        assert_eq!(
            error,
            McpError::Rpc {
                code: -32022,
                message: "Unsupported protocol version".to_owned(),
                data: Some(json!({
                    "requested": "2026-07-28",
                    "supported": ["2025-11-25"]
                })),
            }
        );
        assert_eq!(classify_probe_error(&error), ProbeErrorClass::Legacy);
        assert_eq!(
            classify_probe_error(&McpError::Rpc {
                code: -32601,
                message: "Method not found".to_owned(),
                data: None,
            }),
            ProbeErrorClass::ContinueModern
        );
        assert_eq!(
            classify_probe_error(&McpError::Rpc {
                code: -32601,
                message: "initialize required".to_owned(),
                data: Some(json!({
                    "requiredMethod": "initialize",
                    "supported": ["2025-11-25"]
                })),
            }),
            ProbeErrorClass::Legacy
        );
        assert_eq!(
            classify_probe_error(&McpError::Rpc {
                code: -32602,
                message: "legacy initialization required".to_owned(),
                data: Some(json!({
                    "requiredMethod": "initialize",
                    "supported": ["2025-11-25"]
                })),
            }),
            ProbeErrorClass::Legacy
        );
        assert_eq!(
            classify_probe_error(&McpError::Rpc {
                code: -32603,
                message: "internal".to_owned(),
                data: None,
            }),
            ProbeErrorClass::Fail
        );
    }

    #[test]
    fn executor_rechecks_the_exact_spawn_payload_before_starting() {
        let allowed = config("modern", McpProtocolMode::ModernOnly);
        let substituted = config("exit", McpProtocolMode::ModernOnly);
        let request = OperationRequest::new(
            ExecutionId::new("execution-1").unwrap(),
            SessionId::new("session-1").unwrap(),
            PrincipalId::new("principal-1").unwrap(),
            crate::test_support::execution_profile("mcp_stdio"),
            GeneId::new("mcp.local.spawn").unwrap(),
            None,
            Capability::ProcessExecute,
            Operation::Execute,
            EffectTarget::process(allowed.program()),
            ResourceScope::none(),
        )
        .unwrap()
        .with_payload_digest(&allowed.authorization_payload(SpawnPurpose::Modern).unwrap())
        .unwrap();
        let policy = PolicyContext::new(1, [Capability::ProcessExecute], []);
        let monitor = ReferenceMonitor::new_with_policy(policy.clone(), 60);
        let permit = monitor
            .authorize(
                request.clone(),
                Parliament::new(1).decide(&request, &policy),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();
        let consumed = monitor
            .store()
            .consume(permit, &request, Timestamp::from_unix_seconds(10))
            .unwrap();
        let catalogs = crate::mcp_catalog::McpCatalogSupervisor::new();
        let reservation = catalogs
            .reserve("local", allowed.catalog_config_digest().unwrap())
            .unwrap();

        let result = McpExecutor::start_modern(
            &consumed,
            ToolEngine::new(),
            substituted,
            &reservation,
            SpawnPurpose::Modern,
            false,
            Timestamp::from_unix_seconds(10),
        );

        assert_eq!(result.result().unwrap_err(), &McpError::PermissionDenied);
    }

    #[test]
    fn controller_denies_or_requires_approval_before_spawn() {
        let fixture = Fixture::new();
        let engine = ToolEngine::new();
        let denied = ExecutionController::new(fixture.root.clone()).start_mcp(
            &engine,
            config("modern", McpProtocolMode::ModernOnly),
            &fixture.session(),
            Timestamp::from_unix_seconds(10),
        );
        let denied = denied.unwrap_err();
        assert_eq!(denied.error(), &McpError::PolicyDenied);
        assert_eq!(
            denied.event_types(),
            vec![EventType::EffectRequested, EventType::PolicyDenied]
        );

        let policy = PolicyContext::new(
            1,
            [Capability::ProcessExecute, Capability::McpInvoke],
            [Operation::Execute],
        );
        let approval = ExecutionController::with_policy(fixture.root.clone(), policy).start_mcp(
            &engine,
            config("modern", McpProtocolMode::ModernOnly),
            &fixture.session(),
            Timestamp::from_unix_seconds(10),
        );
        assert_eq!(approval.unwrap_err().error(), &McpError::ApprovalRequired);
    }

    #[test]
    fn controller_supports_modern_legacy_and_auto_modern_with_one_shared_call_path() {
        for (mode, fixture_mode, expected_era) in [
            (McpProtocolMode::ModernOnly, "modern", McpWireEra::Modern),
            (McpProtocolMode::LegacyOnly, "legacy", McpWireEra::Legacy),
            (McpProtocolMode::Auto, "modern", McpWireEra::Modern),
        ] {
            governed_round_trip(mode, fixture_mode, expected_era);
        }
    }

    #[test]
    fn catalog_revision_blocks_duplicate_server_until_the_owner_drops() {
        let fixture = Fixture::new();
        let log = fixture.path.join("catalog-lifecycle.log");
        let engine = ToolEngine::new();
        let controller = controller(&fixture);
        let first = controller
            .start_mcp(
                &engine,
                config_with_log("modern", McpProtocolMode::ModernOnly, &log),
                &fixture.session(),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap()
            .into_server();
        assert_eq!(first.catalog_revision().generation(), 1);
        assert!(first.catalog_revision().process_id() > 0);
        assert_eq!(first.catalog_revision().catalog_digest().len(), 64);
        let request_count = std::fs::read_to_string(&log).unwrap().lines().count();

        let duplicate = controller
            .start_mcp(
                &engine,
                config_with_log("modern", McpProtocolMode::ModernOnly, &log),
                &fixture.session(),
                Timestamp::from_unix_seconds(11),
            )
            .unwrap_err();
        assert_eq!(duplicate.error(), &McpError::DuplicateServer);
        assert_eq!(
            std::fs::read_to_string(&log).unwrap().lines().count(),
            request_count
        );

        drop(first);
        let second = controller
            .start_mcp(
                &engine,
                config_with_log("modern", McpProtocolMode::ModernOnly, &log),
                &fixture.session(),
                Timestamp::from_unix_seconds(12),
            )
            .unwrap()
            .into_server();
        assert_eq!(second.catalog_revision().generation(), 2);
    }

    #[test]
    fn failed_atomic_import_leaves_no_catalog_or_tools() {
        let fixture = Fixture::new();
        let engine = ToolEngine::new();
        let controller = controller(&fixture);

        let failure = controller
            .start_mcp(
                &engine,
                config("unsupported-schema", McpProtocolMode::ModernOnly),
                &fixture.session(),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap_err();
        assert_eq!(failure.error(), &McpError::UnsupportedSchema);
        assert!(engine.list().is_empty());

        let server = controller
            .start_mcp(
                &engine,
                config("modern", McpProtocolMode::ModernOnly),
                &fixture.session(),
                Timestamp::from_unix_seconds(11),
            )
            .unwrap()
            .into_server();
        assert_eq!(server.catalog_revision().generation(), 2);
    }

    #[test]
    fn fatal_server_termination_releases_the_catalog_before_drop() {
        let fixture = Fixture::new();
        let engine = ToolEngine::new();
        let controller = controller(&fixture);
        let mut terminated = controller
            .start_mcp(
                &engine,
                config("invalid-result", McpProtocolMode::ModernOnly),
                &fixture.session(),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap()
            .into_server();
        let failure = controller
            .invoke_mcp(
                &engine,
                &mut terminated,
                "mcp.local.echo",
                json!({"value": "hello"}),
                "fatal-call-1",
                &fixture.session(),
                Timestamp::from_unix_seconds(11),
            )
            .unwrap_err();
        assert_eq!(failure.error(), &McpError::InvalidResult);
        assert!(engine.list().is_empty());

        let replacement = controller
            .start_mcp(
                &engine,
                config("modern", McpProtocolMode::ModernOnly),
                &fixture.session(),
                Timestamp::from_unix_seconds(12),
            )
            .unwrap()
            .into_server();
        assert_eq!(replacement.catalog_revision().generation(), 2);
    }

    #[test]
    fn invocation_request_binds_revision_catalog_and_schema_digests() {
        let fixture = Fixture::new();
        let engine = ToolEngine::new();
        let controller = controller(&fixture);
        let server = controller
            .start_mcp(
                &engine,
                config("modern", McpProtocolMode::ModernOnly),
                &fixture.session(),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap()
            .into_server();
        let arguments = json!({"value": "hello"});
        let payload = server
            .invocation_payload("mcp.local.echo", &arguments)
            .unwrap();
        let decoded: Value = serde_json::from_slice(&payload).unwrap();
        let tool = server.catalog_revision().tool("mcp.local.echo").unwrap();
        assert_eq!(decoded["server"], "local");
        assert_eq!(decoded["generation"], 1);
        assert_eq!(
            decoded["catalog_digest"],
            server.catalog_revision().catalog_digest()
        );
        assert_eq!(decoded["local_tool"], "mcp.local.echo");
        assert_eq!(decoded["remote_tool"], "echo");
        assert_eq!(decoded["schema_digest"], tool.schema_digest());
        assert_eq!(decoded["arguments"], arguments);

        let context = ToolContext::new(
            ExecutionId::new("execution-catalog-1").unwrap(),
            fixture.session().id().clone(),
            fixture.session().principal_id().clone(),
            crate::test_support::execution_profile("mcp_stdio"),
            None,
        );
        let plan = engine
            .plan_with_payload(
                "mcp.local.echo",
                &context,
                arguments.clone(),
                "catalog-call-1",
                EffectTarget::mcp("local", "echo"),
                ResourceScope::none(),
                &payload,
            )
            .unwrap();
        assert!(plan.request().payload_digest_matches(&payload));
        assert!(
            !plan
                .request()
                .payload_digest_matches(&serde_json::to_vec(&arguments).unwrap())
        );
    }

    #[test]
    fn stale_catalog_revision_is_rejected_before_rpc() {
        let fixture = Fixture::new();
        let log = fixture.path.join("stale-catalog.log");
        let engine = ToolEngine::new();
        let controller = controller(&fixture);
        let first = controller
            .start_mcp(
                &engine,
                config_with_log("modern", McpProtocolMode::ModernOnly, &log),
                &fixture.session(),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap()
            .into_server();
        let arguments = json!({"value": "hello"});
        let payload = first
            .invocation_payload("mcp.local.echo", &arguments)
            .unwrap();
        let context = ToolContext::new(
            ExecutionId::new("execution-stale-1").unwrap(),
            fixture.session().id().clone(),
            fixture.session().principal_id().clone(),
            crate::test_support::execution_profile("mcp_stdio"),
            None,
        );
        let plan = engine
            .plan_with_payload(
                "mcp.local.echo",
                &context,
                arguments,
                "stale-call-1",
                EffectTarget::mcp("local", "echo"),
                ResourceScope::none(),
                &payload,
            )
            .unwrap();
        let policy = PolicyContext::new(1, [Capability::McpInvoke], []);
        let monitor = ReferenceMonitor::new_with_policy(policy.clone(), 60);
        let permit = monitor
            .authorize(
                plan.request().clone(),
                Parliament::new(1).decide(plan.request(), &policy),
                Timestamp::from_unix_seconds(11),
            )
            .unwrap();
        let consumed = monitor
            .store()
            .consume(permit, plan.request(), Timestamp::from_unix_seconds(11))
            .unwrap();
        drop(first);

        let mut second = controller
            .start_mcp(
                &engine,
                config_with_log("modern", McpProtocolMode::ModernOnly, &log),
                &fixture.session(),
                Timestamp::from_unix_seconds(12),
            )
            .unwrap()
            .into_server();
        assert_eq!(second.catalog_revision().generation(), 2);
        let request_count = std::fs::read_to_string(&log).unwrap().lines().count();
        let execution = McpExecutor::invoke(
            &consumed,
            &mut second,
            &plan,
            Timestamp::from_unix_seconds(13),
        );
        assert_eq!(execution.result().unwrap_err(), &McpError::PermissionDenied);
        assert_eq!(
            std::fs::read_to_string(&log).unwrap().lines().count(),
            request_count
        );
    }

    #[test]
    fn method_not_found_remains_modern_and_uses_modern_tool_metadata() {
        let fixture = Fixture::new();
        let engine = ToolEngine::new();
        let controller = controller(&fixture);
        let started = controller
            .start_mcp(
                &engine,
                config("method-not-found", McpProtocolMode::Auto),
                &fixture.session(),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();
        assert_eq!(started.selected_era(), McpWireEra::Modern);
        assert!(!started.downgraded());
    }

    #[test]
    fn auto_restarts_for_explicit_legacy_and_audits_the_downgrade() {
        for legacy_signal in ["explicit-legacy", "initialize-required"] {
            let fixture = Fixture::new();
            let log = fixture.path.join(format!("{legacy_signal}.log"));
            let engine = ToolEngine::new();
            let started = controller(&fixture)
                .start_mcp(
                    &engine,
                    config_with_log(legacy_signal, McpProtocolMode::Auto, &log),
                    &fixture.session(),
                    Timestamp::from_unix_seconds(10),
                )
                .unwrap();
            assert_eq!(started.selected_era(), McpWireEra::Legacy);
            assert!(started.downgraded());
            assert_eq!(started.receipts().len(), 2);
            assert!(started.events().iter().any(|event| matches!(
                event.payload(),
                EventPayload::McpEra {
                    era,
                    downgraded: true,
                    ..
                } if era == "legacy"
            )));
            let lines = std::fs::read_to_string(&log).unwrap();
            let lines = lines.lines().collect::<Vec<_>>();
            assert_eq!(lines.len(), 4);
            assert!(lines[0].ends_with("server/discover"));
            assert!(lines[1].ends_with("initialize"));
            assert!(lines[2].ends_with("notifications/initialized"));
            assert!(lines[3].ends_with("tools/list"));
            assert_ne!(
                lines[0].split_whitespace().next(),
                lines[1].split_whitespace().next()
            );
        }
    }

    #[test]
    fn auto_never_downgrades_ambiguous_or_fatal_probe_failures() {
        for (mode, expected) in [
            ("hang", McpError::TimedOut),
            ("exit", McpError::UnexpectedEof),
            ("non-utf8", McpError::InvalidUtf8),
            ("oversized", McpError::FrameTooLarge),
            ("multiline", McpError::MalformedMessage),
            ("bad-id", McpError::UnexpectedResponseId),
            ("malformed", McpError::MalformedMessage),
            ("unexpected-method", McpError::UnexpectedServerMessage),
            (
                "generic-error",
                McpError::Rpc {
                    code: -32603,
                    message: "internal".to_owned(),
                    data: None,
                },
            ),
        ] {
            let fixture = Fixture::new();
            let log = fixture.path.join(format!("{mode}.log"));
            let engine = ToolEngine::new();
            let mut config = config_with_log(mode, McpProtocolMode::Auto, &log);
            config.limits.timeout = if mode == "hang" {
                Duration::from_millis(40)
            } else {
                Duration::from_secs(1)
            };
            if mode == "oversized" {
                config.limits.max_frame_bytes = 64;
            }
            let failure = controller(&fixture)
                .start_mcp(
                    &engine,
                    config,
                    &fixture.session(),
                    Timestamp::from_unix_seconds(10),
                )
                .unwrap_err();
            assert_eq!(failure.error(), &expected, "mode {mode}");
            assert!(engine.list().is_empty());
            let lines = std::fs::read_to_string(&log).unwrap_or_default();
            let pids = lines
                .lines()
                .filter_map(|line| line.split_whitespace().next())
                .collect::<HashSet<_>>();
            assert!(pids.len() <= 1, "mode {mode} spawned a fallback child");
        }
    }

    #[test]
    fn version_capability_schema_and_result_fail_closed() {
        for (mode, expected) in [
            ("wrong-version", McpError::UnsupportedProtocolVersion),
            ("missing-tools", McpError::ToolsCapabilityMissing),
            ("unsupported-schema", McpError::UnsupportedSchema),
        ] {
            let fixture = Fixture::new();
            let engine = ToolEngine::new();
            let failure = controller(&fixture)
                .start_mcp(
                    &engine,
                    config(mode, McpProtocolMode::ModernOnly),
                    &fixture.session(),
                    Timestamp::from_unix_seconds(10),
                )
                .unwrap_err();
            assert_eq!(failure.error(), &expected, "mode {mode}");
            assert!(engine.list().is_empty());
        }
        let fixture = Fixture::new();
        let engine = ToolEngine::new();
        let mut started = controller(&fixture)
            .start_mcp(
                &engine,
                config("invalid-result", McpProtocolMode::ModernOnly),
                &fixture.session(),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap()
            .into_server();
        let failure = controller(&fixture)
            .invoke_mcp(
                &engine,
                &mut started,
                "mcp.local.echo",
                json!({"value": "hello"}),
                "call-1",
                &fixture.session(),
                Timestamp::from_unix_seconds(11),
            )
            .unwrap_err();
        assert_eq!(failure.error(), &McpError::InvalidResult);
        assert!(engine.list().is_empty());
    }

    fn governed_round_trip(mode: McpProtocolMode, fixture_mode: &str, expected_era: McpWireEra) {
        let fixture = Fixture::new();
        let engine = ToolEngine::new();
        let controller = controller(&fixture);
        let started = controller
            .start_mcp(
                &engine,
                config(fixture_mode, mode),
                &fixture.session(),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();
        assert_eq!(started.selected_era(), expected_era);
        assert!(!started.downgraded());
        assert_eq!(started.receipts().len(), 1);
        assert!(started.events().iter().any(|event| matches!(
            event.payload(),
            EventPayload::McpEra { era, .. }
                if era == match expected_era {
                    McpWireEra::Modern => "modern",
                    McpWireEra::Legacy => "legacy",
                }
        )));
        let imported = engine
            .list()
            .into_iter()
            .find(|tool| tool.id().as_str() == "mcp.local.echo")
            .unwrap();
        assert_eq!(imported.version(), "mcp");
        let mut server = started.into_server();
        let invalid = controller.invoke_mcp(
            &engine,
            &mut server,
            "mcp.local.echo",
            json!({"value": 4}),
            "call-invalid",
            &fixture.session(),
            Timestamp::from_unix_seconds(11),
        );
        assert_eq!(invalid.unwrap_err().error(), &McpError::InvalidArguments);
        let invocation = controller
            .invoke_mcp(
                &engine,
                &mut server,
                "mcp.local.echo",
                json!({"value": "hello"}),
                "call-1",
                &fixture.session(),
                Timestamp::from_unix_seconds(12),
            )
            .unwrap();
        assert_eq!(invocation.result().value()["content"][0]["text"], "echoed");
        assert_eq!(invocation.receipts().len(), 1);
        drop(server);
        assert!(engine.list().is_empty());
    }

    fn config(fixture_mode: &str, mode: McpProtocolMode) -> McpStdioConfig {
        McpStdioConfig::new(
            "local",
            fixture_program().to_string_lossy().into_owned(),
            vec![fixture_mode.to_owned()],
            mode,
        )
        .unwrap()
    }

    fn config_with_log(
        fixture_mode: &str,
        mode: McpProtocolMode,
        log: &std::path::Path,
    ) -> McpStdioConfig {
        McpStdioConfig::new(
            "local",
            fixture_program().to_string_lossy().into_owned(),
            vec![fixture_mode.to_owned(), log.to_string_lossy().into_owned()],
            mode,
        )
        .unwrap()
    }

    fn fixture_program() -> &'static PathBuf {
        static PROGRAM: OnceLock<PathBuf> = OnceLock::new();
        PROGRAM.get_or_init(|| {
            let directory = crate::test_support::new_temp_dir("pandora-mcp-fixture").unwrap();
            let output = directory.join(format!("mcp-server{}", std::env::consts::EXE_SUFFIX));
            let source =
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mcp_server.rs");
            let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
            let status = Command::new(rustc)
                .arg("--edition=2024")
                .arg(source)
                .arg("-o")
                .arg(&output)
                .status()
                .unwrap();
            assert!(status.success());
            output
        })
    }

    struct Fixture {
        path: PathBuf,
        root: WorkspaceRoot,
    }

    impl Fixture {
        fn new() -> Self {
            let path = crate::test_support::new_temp_dir("pandora-mcp-test").unwrap();
            let root = WorkspaceRoot::new(&path).unwrap();
            Self { path, root }
        }

        fn session(&self) -> Session {
            Session::new(
                SessionId::new("session-1").unwrap(),
                PrincipalId::new("principal-1").unwrap(),
                TenantId::new("tenant-1").unwrap(),
                WorkspaceId::new("workspace-1").unwrap(),
                Timestamp::from_unix_seconds(1),
            )
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn controller(fixture: &Fixture) -> ExecutionController {
        ExecutionController::with_policy(
            fixture.root.clone(),
            PolicyContext::new(1, [Capability::ProcessExecute, Capability::McpInvoke], []),
        )
    }
}
