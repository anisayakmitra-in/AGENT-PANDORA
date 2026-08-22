use crate::manifest::{ManifestError, ModelId, ProviderId, ProviderManifest, ProviderProtocol};
use pandora_types::{ExecutionId, SessionId};
use reqwest::blocking::{Client, Response};
use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fmt;
use std::io::Read;
use std::net::IpAddr;
use std::time::Duration;

const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 1_024;
const MAX_OUTPUT_TOKENS: u32 = 131_072;
const MAX_MESSAGES: usize = 128;
const MAX_MESSAGE_BYTES: usize = 1_048_576;
const MAX_TOOLS: usize = 128;
const MAX_TOOL_CALL_ID_BYTES: usize = 256;
const MAX_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_RESPONSE_TEXT_BYTES: usize = 4 * 1024 * 1024;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderError {
    InvalidManifest(ManifestError),
    CredentialUnavailable,
    InvalidRequest(String),
    UnsupportedEndpoint,
    Transport,
    HttpStatus { status: u16 },
    ResponseTooLarge,
    InvalidResponse,
    InvalidToolArguments { call_id: String },
    DuplicateToolCallId { call_id: String },
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidManifest(error) => error.fmt(formatter),
            Self::CredentialUnavailable => formatter.write_str("provider credential unavailable"),
            Self::InvalidRequest(reason) => write!(formatter, "invalid provider request: {reason}"),
            Self::UnsupportedEndpoint => formatter.write_str("provider endpoint is not allowed"),
            Self::Transport => formatter.write_str("provider transport failed"),
            Self::HttpStatus { status } => {
                write!(formatter, "provider returned HTTP status {status}")
            }
            Self::ResponseTooLarge => {
                formatter.write_str("provider response exceeds the size limit")
            }
            Self::InvalidResponse => formatter.write_str("provider returned an invalid response"),
            Self::InvalidToolArguments { .. } => {
                formatter.write_str("provider returned invalid tool arguments")
            }
            Self::DuplicateToolCallId { .. } => {
                formatter.write_str("provider returned duplicate tool-call identifiers")
            }
        }
    }
}

impl ProviderError {
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::CredentialUnavailable | Self::Transport => true,
            Self::HttpStatus { status } => {
                matches!(*status, 408 | 429 | 500..=599)
            }
            _ => false,
        }
    }
}

impl std::error::Error for ProviderError {}

fn validate_tool_call_id(value: &str) -> Result<(), ProviderError> {
    if value.len() > MAX_TOOL_CALL_ID_BYTES
        || value.trim().is_empty()
        || value.chars().any(char::is_control)
    {
        return Err(ProviderError::InvalidRequest(
            "tool call ID is invalid".to_owned(),
        ));
    }
    Ok(())
}

impl From<ManifestError> for ProviderError {
    fn from(error: ManifestError) -> Self {
        Self::InvalidManifest(error)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ChatMessage {
    role: MessageRole,
    content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<ChatToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ChatToolCall {
    id: String,
    name: String,
    arguments: String,
}

impl ChatMessage {
    pub fn new(role: MessageRole, content: impl Into<String>) -> Result<Self, ProviderError> {
        let content = content.into();
        if content.is_empty() {
            return Err(ProviderError::InvalidRequest(
                "message content cannot be empty".to_owned(),
            ));
        }
        if content.len() > MAX_MESSAGE_BYTES {
            return Err(ProviderError::InvalidRequest(
                "message content exceeds the size limit".to_owned(),
            ));
        }
        Ok(Self {
            role,
            content,
            tool_calls: Vec::new(),
            tool_call_id: None,
        })
    }

    pub fn system(content: impl Into<String>) -> Result<Self, ProviderError> {
        Self::new(MessageRole::System, content)
    }

    pub fn user(content: impl Into<String>) -> Result<Self, ProviderError> {
        Self::new(MessageRole::User, content)
    }

    pub fn assistant(content: impl Into<String>) -> Result<Self, ProviderError> {
        Self::new(MessageRole::Assistant, content)
    }

    pub fn tool(content: impl Into<String>) -> Result<Self, ProviderError> {
        Self::new(MessageRole::Tool, content)
    }

    pub fn assistant_tool_calls(tool_calls: &[ToolCall]) -> Result<Self, ProviderError> {
        if tool_calls.is_empty() {
            return Err(ProviderError::InvalidRequest(
                "assistant tool-call message cannot be empty".to_owned(),
            ));
        }
        let tool_calls = tool_calls
            .iter()
            .map(|call| {
                let arguments = serde_json::to_string(call.arguments()).map_err(|_| {
                    ProviderError::InvalidRequest("tool arguments could not be encoded".to_owned())
                })?;
                Ok(ChatToolCall {
                    id: call.id().to_owned(),
                    name: call.name().to_owned(),
                    arguments,
                })
            })
            .collect::<Result<Vec<_>, ProviderError>>()?;
        Ok(Self {
            role: MessageRole::Assistant,
            content: String::new(),
            tool_calls,
            tool_call_id: None,
        })
    }

    pub fn tool_result(
        call_id: impl Into<String>,
        content: impl Into<String>,
    ) -> Result<Self, ProviderError> {
        let call_id = call_id.into();
        validate_tool_call_id(&call_id)?;
        let content = content.into();
        let content = if content.is_empty() {
            "[empty tool result]".to_owned()
        } else {
            content
        };
        let mut message = Self::tool(content)?;
        message.tool_call_id = Some(call_id);
        Ok(message)
    }

    pub fn role(&self) -> MessageRole {
        self.role
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    pub fn tool_call_id(&self) -> Option<&str> {
        self.tool_call_id.as_deref()
    }

    pub fn tool_calls(&self) -> Result<Vec<ToolCall>, ProviderError> {
        self.tool_calls
            .iter()
            .map(|call| {
                let arguments = serde_json::from_str::<Value>(&call.arguments).map_err(|_| {
                    ProviderError::InvalidToolArguments {
                        call_id: call.id.clone(),
                    }
                })?;
                ToolCall::new(call.id.clone(), call.name.clone(), arguments)
            })
            .collect()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolSchema {
    name: String,
    description: String,
    input_schema: Value,
}

impl ToolSchema {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
    ) -> Result<Self, ProviderError> {
        let name = name.into();
        let description = description.into();
        if name.is_empty() || name.len() > 128 {
            return Err(ProviderError::InvalidRequest(
                "tool name is empty or too long".to_owned(),
            ));
        }
        if !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(ProviderError::InvalidRequest(
                "tool name contains unsupported characters".to_owned(),
            ));
        }
        if description.len() > 1_024 {
            return Err(ProviderError::InvalidRequest(
                "tool description exceeds the size limit".to_owned(),
            ));
        }
        if !input_schema.is_object() {
            return Err(ProviderError::InvalidRequest(
                "tool input schema must be an object".to_owned(),
            ));
        }
        Ok(Self {
            name,
            description,
            input_schema,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn input_schema(&self) -> &Value {
        &self.input_schema
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct TraceMetadata {
    execution_id: Option<ExecutionId>,
    session_id: Option<SessionId>,
}

impl TraceMetadata {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_execution_id(mut self, execution_id: ExecutionId) -> Self {
        self.execution_id = Some(execution_id);
        self
    }

    pub fn with_session_id(mut self, session_id: SessionId) -> Self {
        self.session_id = Some(session_id);
        self
    }

    pub fn execution_id(&self) -> Option<&ExecutionId> {
        self.execution_id.as_ref()
    }

    pub fn session_id(&self) -> Option<&SessionId> {
        self.session_id.as_ref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelRequest {
    provider_id: ProviderId,
    model_id: ModelId,
    messages: Vec<ChatMessage>,
    tools: Vec<ToolSchema>,
    max_output_tokens: u32,
    timeout: Duration,
    trace: TraceMetadata,
}

impl ModelRequest {
    pub fn new(
        provider_id: ProviderId,
        model_id: ModelId,
        messages: Vec<ChatMessage>,
    ) -> Result<Self, ProviderError> {
        let request = Self {
            provider_id,
            model_id,
            messages,
            tools: Vec::new(),
            max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
            timeout: DEFAULT_TIMEOUT,
            trace: TraceMetadata::default(),
        };
        request.validate()?;
        Ok(request)
    }

    pub fn with_tools(mut self, tools: Vec<ToolSchema>) -> Result<Self, ProviderError> {
        self.tools = tools;
        self.validate()?;
        Ok(self)
    }

    pub fn with_max_output_tokens(mut self, max_output_tokens: u32) -> Result<Self, ProviderError> {
        self.max_output_tokens = max_output_tokens;
        self.validate()?;
        Ok(self)
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Result<Self, ProviderError> {
        self.timeout = timeout;
        self.validate()?;
        Ok(self)
    }

    pub fn with_trace_metadata(mut self, trace: TraceMetadata) -> Self {
        self.trace = trace;
        self
    }

    pub fn for_provider(mut self, provider_id: ProviderId, model_id: ModelId) -> Self {
        self.provider_id = provider_id;
        self.model_id = model_id;
        self
    }

    pub fn validate(&self) -> Result<(), ProviderError> {
        if self.messages.is_empty() || self.messages.len() > MAX_MESSAGES {
            return Err(ProviderError::InvalidRequest(
                "message count is outside the allowed range".to_owned(),
            ));
        }
        let message_bytes = self
            .messages
            .iter()
            .map(|message| message.content.len())
            .sum::<usize>();
        if message_bytes > MAX_MESSAGE_BYTES {
            return Err(ProviderError::InvalidRequest(
                "total message content exceeds the size limit".to_owned(),
            ));
        }
        if self.tools.len() > MAX_TOOLS {
            return Err(ProviderError::InvalidRequest(
                "tool count exceeds the size limit".to_owned(),
            ));
        }
        let mut names = BTreeSet::new();
        for tool in &self.tools {
            if !names.insert(tool.name()) {
                return Err(ProviderError::InvalidRequest(
                    "duplicate tool name".to_owned(),
                ));
            }
        }
        if self.max_output_tokens == 0 || self.max_output_tokens > MAX_OUTPUT_TOKENS {
            return Err(ProviderError::InvalidRequest(
                "output token budget is outside the allowed range".to_owned(),
            ));
        }
        if self.timeout.is_zero() || self.timeout > MAX_TIMEOUT {
            return Err(ProviderError::InvalidRequest(
                "request timeout is outside the allowed range".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }

    pub fn model_id(&self) -> &ModelId {
        &self.model_id
    }

    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    pub fn tools(&self) -> &[ToolSchema] {
        &self.tools
    }

    pub fn max_output_tokens(&self) -> u32 {
        self.max_output_tokens
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    pub fn trace(&self) -> &TraceMetadata {
        &self.trace
    }

    pub fn authorization_payload(&self) -> Result<Vec<u8>, ProviderError> {
        Self::encode_authorization_payload(&self.canonical_authorization_request())
    }

    pub fn authorization_payload_for(
        &self,
        manifest: &ProviderManifest,
    ) -> Result<Vec<u8>, ProviderError> {
        if self.provider_id != *manifest.id() {
            return Err(ProviderError::InvalidRequest(
                "provider request is not bound to the provider manifest".to_owned(),
            ));
        }
        Self::encode_authorization_payload(&CanonicalProviderInvocation {
            provider_id: manifest.id().as_str(),
            protocol: manifest.protocol(),
            base_url: manifest.base_url(),
            credential_reference: manifest.api_key_env(),
            request: self.canonical_authorization_request(),
        })
    }

    fn canonical_authorization_request(&self) -> CanonicalModelRequest<'_> {
        CanonicalModelRequest {
            provider_id: self.provider_id.as_str(),
            model_id: self.model_id.as_str(),
            messages: &self.messages,
            tools: &self.tools,
            max_output_tokens: self.max_output_tokens,
            timeout_millis: self.timeout.as_millis(),
            trace_execution_id: self.trace.execution_id().map(|id| id.as_str()),
            trace_session_id: self.trace.session_id().map(|id| id.as_str()),
        }
    }

    fn encode_authorization_payload<T: Serialize>(value: &T) -> Result<Vec<u8>, ProviderError> {
        serde_json::to_vec(value).map_err(|_| {
            ProviderError::InvalidRequest(
                "provider request could not be encoded for authorization".to_owned(),
            )
        })
    }
}

#[derive(Serialize)]
struct CanonicalModelRequest<'a> {
    provider_id: &'a str,
    model_id: &'a str,
    messages: &'a [ChatMessage],
    tools: &'a [ToolSchema],
    max_output_tokens: u32,
    timeout_millis: u128,
    trace_execution_id: Option<&'a str>,
    trace_session_id: Option<&'a str>,
}

#[derive(Serialize)]
struct CanonicalProviderInvocation<'a> {
    provider_id: &'a str,
    protocol: ProviderProtocol,
    base_url: &'a str,
    credential_reference: &'a str,
    request: CanonicalModelRequest<'a>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolCall {
    id: String,
    name: String,
    arguments: Value,
}

impl ToolCall {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: Value,
    ) -> Result<Self, ProviderError> {
        let id = id.into();
        let name = name.into();
        validate_tool_call_id(&id)?;
        if name.trim().is_empty() || name.chars().any(char::is_control) {
            return Err(ProviderError::InvalidRequest(
                "tool name is invalid".to_owned(),
            ));
        }
        if !arguments.is_object() {
            return Err(ProviderError::InvalidToolArguments { call_id: id });
        }
        Ok(Self {
            id,
            name,
            arguments,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn arguments(&self) -> &Value {
        &self.arguments
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TokenUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

impl TokenUsage {
    pub fn new(prompt_tokens: u32, completion_tokens: u32) -> Self {
        Self {
            prompt_tokens,
            completion_tokens,
        }
    }

    pub fn prompt_tokens(&self) -> u32 {
        self.prompt_tokens
    }

    pub fn completion_tokens(&self) -> u32 {
        self.completion_tokens
    }

    pub fn total_tokens(&self) -> u32 {
        self.prompt_tokens.saturating_add(self.completion_tokens)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModelResponse {
    text: String,
    tool_calls: Vec<ToolCall>,
    usage: TokenUsage,
}

impl ModelResponse {
    pub fn new(text: impl Into<String>, tool_calls: Vec<ToolCall>, usage: TokenUsage) -> Self {
        Self {
            text: text.into(),
            tool_calls,
            usage,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn tool_calls(&self) -> &[ToolCall] {
        &self.tool_calls
    }

    pub fn usage(&self) -> &TokenUsage {
        &self.usage
    }
}

pub trait Provider: Send + Sync {
    fn manifest(&self) -> &ProviderManifest;
    fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError>;
}

struct SecretValue(String);

impl SecretValue {
    fn new(value: String) -> Result<Self, ProviderError> {
        if value.trim().is_empty() || value.chars().any(char::is_control) {
            return Err(ProviderError::CredentialUnavailable);
        }
        Ok(Self(value))
    }

    fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted]")
    }
}

pub struct HttpProvider {
    manifest: ProviderManifest,
    client: Client,
    api_key: SecretValue,
}

impl fmt::Debug for HttpProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpProvider")
            .field("manifest", &self.manifest)
            .field("api_key", &self.api_key)
            .finish()
    }
}

impl HttpProvider {
    pub fn new(
        manifest: ProviderManifest,
        api_key: impl Into<String>,
    ) -> Result<Self, ProviderError> {
        validate_endpoint(manifest.base_url())?;
        let api_key = SecretValue::new(api_key.into())?;
        let client = Client::builder()
            .redirect(Policy::none())
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .map_err(|_| ProviderError::Transport)?;
        Ok(Self {
            manifest,
            client,
            api_key,
        })
    }

    pub fn from_environment(manifest: ProviderManifest) -> Result<Self, ProviderError> {
        let api_key = std::env::var(manifest.api_key_env())
            .map_err(|_| ProviderError::CredentialUnavailable)?;
        Self::new(manifest, api_key)
    }

    fn endpoint(&self) -> String {
        format!(
            "{}/chat/completions",
            self.manifest.base_url().trim_end_matches('/')
        )
    }
}

impl Provider for HttpProvider {
    fn manifest(&self) -> &ProviderManifest {
        &self.manifest
    }

    fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        request.validate()?;
        if request.provider_id() != self.manifest.id() {
            return Err(ProviderError::InvalidRequest(
                "request provider does not match the client".to_owned(),
            ));
        }
        let body = OpenAiRequest::from_request(&request);
        let response = self
            .client
            .post(self.endpoint())
            .timeout(request.timeout())
            .bearer_auth(self.api_key.expose())
            .json(&body)
            .send()
            .map_err(|_| ProviderError::Transport)?;
        let status = response.status().as_u16();
        let bytes = read_limited(response)?;
        if !(200..300).contains(&status) {
            return Err(ProviderError::HttpStatus { status });
        }
        parse_response(&bytes)
    }
}

fn validate_endpoint(base_url: &str) -> Result<(), ProviderError> {
    let url = reqwest::Url::parse(base_url).map_err(|_| ProviderError::UnsupportedEndpoint)?;
    if url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ProviderError::UnsupportedEndpoint);
    }
    match url.scheme() {
        "https" => Ok(()),
        "http" if is_loopback_host(url.host_str()) => Ok(()),
        _ => Err(ProviderError::UnsupportedEndpoint),
    }
}

fn is_loopback_host(host: Option<&str>) -> bool {
    let Some(host) = host else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .map(|address| address.is_loopback())
            .unwrap_or(false)
}

fn read_limited(mut response: Response) -> Result<Vec<u8>, ProviderError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES)
    {
        return Err(ProviderError::ResponseTooLarge);
    }
    let mut bytes = Vec::with_capacity(
        response
            .content_length()
            .unwrap_or(0)
            .min(MAX_RESPONSE_BYTES) as usize,
    );
    response
        .by_ref()
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ProviderError::Transport)?;
    if bytes.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(ProviderError::ResponseTooLarge);
    }
    Ok(bytes)
}

#[derive(Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<OpenAiRequestMessage>,
    tools: Vec<OpenAiTool>,
    max_tokens: u32,
}

impl OpenAiRequest {
    fn from_request(request: &ModelRequest) -> Self {
        Self {
            model: request.model_id().as_str().to_owned(),
            messages: request
                .messages()
                .iter()
                .map(OpenAiRequestMessage::from_message)
                .collect(),
            tools: request
                .tools()
                .iter()
                .map(OpenAiTool::from_schema)
                .collect(),
            max_tokens: request.max_output_tokens(),
        }
    }
}

#[derive(Serialize)]
struct OpenAiRequestMessage {
    role: MessageRole,
    content: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<OpenAiRequestToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

impl OpenAiRequestMessage {
    fn from_message(message: &ChatMessage) -> Self {
        Self {
            role: message.role,
            content: if message.tool_calls.is_empty() {
                Some(message.content.clone())
            } else {
                None
            },
            tool_calls: message
                .tool_calls
                .iter()
                .map(|call| OpenAiRequestToolCall {
                    id: call.id.clone(),
                    kind: "function",
                    function: OpenAiRequestFunction {
                        name: call.name.clone(),
                        arguments: call.arguments.clone(),
                    },
                })
                .collect(),
            tool_call_id: message.tool_call_id.clone(),
        }
    }
}

#[derive(Serialize)]
struct OpenAiRequestToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: &'static str,
    function: OpenAiRequestFunction,
}

#[derive(Serialize)]
struct OpenAiRequestFunction {
    name: String,
    arguments: String,
}

#[derive(Serialize)]
struct OpenAiTool {
    #[serde(rename = "type")]
    kind: &'static str,
    function: OpenAiFunction,
}

impl OpenAiTool {
    fn from_schema(schema: &ToolSchema) -> Self {
        Self {
            kind: "function",
            function: OpenAiFunction {
                name: schema.name.clone(),
                description: schema.description.clone(),
                parameters: schema.input_schema.clone(),
            },
        }
    }
}

#[derive(Serialize)]
struct OpenAiFunction {
    name: String,
    description: String,
    parameters: Value,
}

#[derive(Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
    usage: Option<OpenAiUsage>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
}

#[derive(Deserialize)]
struct OpenAiMessage {
    content: Option<String>,
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

#[derive(Deserialize)]
struct OpenAiToolCall {
    id: String,
    function: OpenAiFunctionCall,
}

#[derive(Deserialize)]
struct OpenAiFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Deserialize)]
struct OpenAiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

fn parse_response(bytes: &[u8]) -> Result<ModelResponse, ProviderError> {
    if bytes.len() > MAX_RESPONSE_BYTES as usize {
        return Err(ProviderError::ResponseTooLarge);
    }
    let response: OpenAiResponse =
        serde_json::from_slice(bytes).map_err(|_| ProviderError::InvalidResponse)?;
    let choice = response
        .choices
        .into_iter()
        .next()
        .ok_or(ProviderError::InvalidResponse)?;
    let text = choice.message.content.unwrap_or_default();
    if text.len() > MAX_RESPONSE_TEXT_BYTES {
        return Err(ProviderError::ResponseTooLarge);
    }
    let mut seen_ids = BTreeSet::new();
    let mut tool_calls = Vec::new();
    for call in choice.message.tool_calls.unwrap_or_default() {
        if validate_tool_call_id(&call.id).is_err() {
            return Err(ProviderError::InvalidResponse);
        }
        if !seen_ids.insert(call.id.clone()) {
            return Err(ProviderError::DuplicateToolCallId { call_id: call.id });
        }
        let arguments = serde_json::from_str(&call.function.arguments).map_err(|_| {
            ProviderError::InvalidToolArguments {
                call_id: call.id.clone(),
            }
        })?;
        let tool_call =
            ToolCall::new(call.id, call.function.name, arguments).map_err(|error| match error {
                ProviderError::InvalidToolArguments { call_id } => {
                    ProviderError::InvalidToolArguments { call_id }
                }
                _ => ProviderError::InvalidResponse,
            })?;
        tool_calls.push(tool_call);
    }
    let usage = response
        .usage
        .map(|usage| TokenUsage::new(usage.prompt_tokens, usage.completion_tokens))
        .unwrap_or_default();
    Ok(ModelResponse {
        text,
        tool_calls,
        usage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::ProviderManifest;
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn manifest() -> ProviderManifest {
        ProviderManifest::new(
            "openai-compatible",
            "OpenAI-compatible",
            "https://api.example.test/v1",
            "model-a",
            "PANDORA_PROVIDER_KEY",
        )
        .unwrap()
    }

    #[test]
    fn request_is_bounded_and_binds_provider_and_model() {
        let manifest = manifest();
        let request = ModelRequest::new(
            manifest.id().clone(),
            manifest.default_model().clone(),
            vec![ChatMessage::user("hello").unwrap()],
        )
        .unwrap()
        .with_max_output_tokens(128)
        .unwrap();

        assert!(request.validate().is_ok());
        assert_eq!(request.provider_id(), manifest.id());
        assert_eq!(request.model_id(), manifest.default_model());
    }

    #[test]
    fn duplicate_tool_names_are_rejected_before_network_use() {
        let manifest = manifest();
        let tool = ToolSchema::new("read_file", "Read a file", json!({"type":"object"})).unwrap();
        let request = ModelRequest::new(
            manifest.id().clone(),
            manifest.default_model().clone(),
            vec![ChatMessage::user("read").unwrap()],
        )
        .unwrap()
        .with_tools(vec![tool.clone(), tool]);

        assert_eq!(
            request.unwrap_err(),
            ProviderError::InvalidRequest("duplicate tool name".to_owned())
        );
    }

    #[test]
    fn response_parser_rejects_malformed_tool_arguments_without_echoing_them() {
        let error = parse_response(
            br#"{
                "choices":[{"message":{"content":null,"tool_calls":[
                    {"id":"call-1","type":"function","function":{"name":"read_file","arguments":"not-json"}}
                ]}}]
            }"#,
        )
        .unwrap_err();

        assert_eq!(
            error,
            ProviderError::InvalidToolArguments {
                call_id: "call-1".to_owned()
            }
        );
        assert!(!error.to_string().contains("not-json"));
    }

    #[test]
    fn tool_call_identifiers_are_bounded_before_transcript_creation() {
        let identifier = "x".repeat(257);

        assert!(ToolCall::new("x".repeat(256), "workspace.read", json!({})).is_ok());
        assert!(ToolCall::new(identifier.clone(), "workspace.read", json!({})).is_err());
        assert!(ChatMessage::tool_result(identifier.clone(), "fixture").is_err());

        let response = json!({
            "choices": [{
                "message": {
                    "content": null,
                    "tool_calls": [{
                        "id": identifier,
                        "type": "function",
                        "function": {
                            "name": "workspace.read",
                            "arguments": "{}"
                        }
                    }]
                }
            }]
        });

        assert_eq!(
            parse_response(&serde_json::to_vec(&response).unwrap()).unwrap_err(),
            ProviderError::InvalidResponse
        );
    }

    #[test]
    fn tool_call_continuation_serializes_assistant_and_tool_messages() {
        let response = parse_response(
            br#"{
                "choices":[{"message":{"content":null,"tool_calls":[
                    {"id":"call-1","type":"function","function":{"name":"workspace.read","arguments":"{\"path\":\"README.md\"}"}}
                ]}}]
            }"#,
        )
        .unwrap();
        let call = &response.tool_calls()[0];
        let assistant = ChatMessage::assistant_tool_calls(response.tool_calls()).unwrap();
        let tool = ChatMessage::tool_result(call.id(), "fixture").unwrap();
        let manifest = manifest();
        let request = ModelRequest::new(
            manifest.id().clone(),
            manifest.default_model().clone(),
            vec![ChatMessage::user("read README").unwrap(), assistant, tool],
        )
        .unwrap();

        let body = serde_json::to_value(OpenAiRequest::from_request(&request)).unwrap();
        assert_eq!(
            body["messages"][1],
            json!({
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call-1",
                    "type": "function",
                    "function": {
                        "name": "workspace.read",
                        "arguments": "{\"path\":\"README.md\"}"
                    }
                }]
            })
        );
        assert_eq!(
            body["messages"][2],
            json!({"role": "tool", "content": "fixture", "tool_call_id": "call-1"})
        );
    }

    #[test]
    fn persisted_chat_messages_round_trip_tool_calls() {
        let response = parse_response(
            br#"{
                "choices":[{"message":{"content":null,"tool_calls":[
                    {"id":"call-1","type":"function","function":{"name":"workspace.read","arguments":"{\"path\":\"README.md\"}"}}
                ]}}]
            }"#,
        )
        .unwrap();
        let messages = vec![
            ChatMessage::user("read README").unwrap(),
            ChatMessage::assistant_tool_calls(response.tool_calls()).unwrap(),
            ChatMessage::tool_result("call-1", "fixture").unwrap(),
        ];

        let encoded = serde_json::to_vec(&messages).unwrap();
        let decoded: Vec<ChatMessage> = serde_json::from_slice(&encoded).unwrap();

        assert_eq!(decoded, messages);
    }

    #[test]
    fn empty_tool_results_use_a_bounded_placeholder() {
        let message = ChatMessage::tool_result("call-1", "").unwrap();

        assert_eq!(message.role(), MessageRole::Tool);
        assert_eq!(message.content(), "[empty tool result]");
    }

    #[test]
    fn provider_debug_output_does_not_include_the_api_key() {
        let provider = HttpProvider::new(manifest(), "sk-live-secret").unwrap();

        assert!(!format!("{provider:?}").contains("sk-live-secret"));
    }

    #[test]
    fn loopback_provider_completes_without_leaking_the_credential() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            loop {
                let mut chunk = [0_u8; 1_024];
                let bytes_read = stream.read(&mut chunk).unwrap();
                if bytes_read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..bytes_read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8_lossy(&request);
            let request = request.to_ascii_lowercase();
            assert!(request.contains("authorization: bearer sk-live-secret"));
            assert!(request.contains("\"model\":\"model-a\""));
            let body = br#"{"choices":[{"message":{"content":"ready"}}],"usage":{"prompt_tokens":2,"completion_tokens":1}}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
        });
        let provider_manifest = ProviderManifest::new(
            "openai-compatible",
            "OpenAI-compatible",
            format!("http://{address}/v1"),
            "model-a",
            "PANDORA_PROVIDER_KEY",
        )
        .unwrap();
        let request = ModelRequest::new(
            provider_manifest.id().clone(),
            provider_manifest.default_model().clone(),
            vec![ChatMessage::user("hello").unwrap()],
        )
        .unwrap();
        let provider = HttpProvider::new(provider_manifest, "sk-live-secret").unwrap();

        let response = provider.complete(request).unwrap();

        server.join().unwrap();
        assert_eq!(response.text(), "ready");
        assert_eq!(response.usage().total_tokens(), 3);
    }
}
