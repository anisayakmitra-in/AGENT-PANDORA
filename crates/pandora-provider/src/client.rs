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
    prompt_cache_ttl: Option<PromptCacheTtl>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum PromptCacheTtl {
    #[serde(rename = "5m")]
    FiveMinutes,
    #[serde(rename = "1h")]
    OneHour,
}

impl PromptCacheTtl {
    const fn as_str(self) -> &'static str {
        match self {
            Self::FiveMinutes => "5m",
            Self::OneHour => "1h",
        }
    }
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
            prompt_cache_ttl: None,
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

    pub fn with_prompt_cache(mut self, ttl: PromptCacheTtl) -> Self {
        self.prompt_cache_ttl = Some(ttl);
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

    pub const fn prompt_cache_ttl(&self) -> Option<PromptCacheTtl> {
        self.prompt_cache_ttl
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
            prompt_cache_ttl: self.prompt_cache_ttl,
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
    prompt_cache_ttl: Option<PromptCacheTtl>,
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
    cached_prompt_tokens: u32,
    cache_write_prompt_tokens: u32,
}

impl TokenUsage {
    pub fn new(prompt_tokens: u32, completion_tokens: u32) -> Self {
        Self {
            prompt_tokens,
            completion_tokens,
            cached_prompt_tokens: 0,
            cache_write_prompt_tokens: 0,
        }
    }

    pub fn with_prompt_cache(
        mut self,
        cached_prompt_tokens: u32,
        cache_write_prompt_tokens: u32,
    ) -> Self {
        self.cached_prompt_tokens = cached_prompt_tokens;
        self.cache_write_prompt_tokens = cache_write_prompt_tokens;
        self
    }

    pub fn prompt_tokens(&self) -> u32 {
        self.prompt_tokens
    }

    pub fn completion_tokens(&self) -> u32 {
        self.completion_tokens
    }

    pub fn cached_prompt_tokens(&self) -> u32 {
        self.cached_prompt_tokens
    }

    pub fn cache_write_prompt_tokens(&self) -> u32 {
        self.cache_write_prompt_tokens
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

    fn fallback_provider(&self) -> Option<&dyn Provider> {
        None
    }
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

    fn endpoint(&self, model: &ModelId) -> String {
        let path = match self.manifest.protocol() {
            ProviderProtocol::OpenAiCompatible => "chat/completions",
            ProviderProtocol::AnthropicMessages => "messages",
            ProviderProtocol::GeminiGenerateContent => {
                return format!(
                    "{}/models/{}:generateContent",
                    self.manifest.base_url().trim_end_matches('/'),
                    model.as_str()
                );
            }
        };
        format!("{}/{path}", self.manifest.base_url().trim_end_matches('/'))
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
        let response = match self.manifest.protocol() {
            ProviderProtocol::OpenAiCompatible => self
                .client
                .post(self.endpoint(request.model_id()))
                .timeout(request.timeout())
                .bearer_auth(self.api_key.expose())
                .json(&OpenAiRequest::from_request(&request))
                .send(),
            ProviderProtocol::AnthropicMessages => self
                .client
                .post(self.endpoint(request.model_id()))
                .timeout(request.timeout())
                .header("x-api-key", self.api_key.expose())
                .header("anthropic-version", "2023-06-01")
                .json(&AnthropicRequest::from_request(&request)?)
                .send(),
            ProviderProtocol::GeminiGenerateContent => self
                .client
                .post(self.endpoint(request.model_id()))
                .timeout(request.timeout())
                .header("x-goog-api-key", self.api_key.expose())
                .json(&GeminiRequest::from_request(&request)?)
                .send(),
        }
        .map_err(|_| ProviderError::Transport)?;
        let status = response.status().as_u16();
        let bytes = read_limited(response)?;
        if !(200..300).contains(&status) {
            return Err(ProviderError::HttpStatus { status });
        }
        match self.manifest.protocol() {
            ProviderProtocol::OpenAiCompatible => parse_response(&bytes),
            ProviderProtocol::AnthropicMessages => parse_anthropic_response(&bytes),
            ProviderProtocol::GeminiGenerateContent => parse_gemini_response(&bytes),
        }
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
    #[serde(default)]
    prompt_tokens_details: Option<OpenAiPromptTokenDetails>,
}

#[derive(Default, Deserialize)]
struct OpenAiPromptTokenDetails {
    #[serde(default)]
    cached_tokens: u32,
}

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<AnthropicSystem>,
    messages: Vec<AnthropicMessage>,
    tools: Vec<AnthropicTool>,
    max_tokens: u32,
}

impl AnthropicRequest {
    fn from_request(request: &ModelRequest) -> Result<Self, ProviderError> {
        let system = request
            .messages()
            .iter()
            .filter(|message| message.role() == MessageRole::System)
            .map(ChatMessage::content)
            .collect::<Vec<_>>();
        let messages = request
            .messages()
            .iter()
            .filter(|message| message.role() != MessageRole::System)
            .map(AnthropicMessage::from_message)
            .collect::<Result<Vec<_>, ProviderError>>()?;
        if messages.is_empty() {
            return Err(ProviderError::InvalidRequest(
                "Anthropic requests require at least one conversation message".to_owned(),
            ));
        }
        let system = (!system.is_empty()).then(|| system.join("\n\n"));
        let system = system.map(|text| match request.prompt_cache_ttl() {
            Some(ttl) => AnthropicSystem::Blocks(vec![AnthropicSystemBlock {
                kind: "text",
                text,
                cache_control: AnthropicCacheControl {
                    kind: "ephemeral",
                    ttl: ttl.as_str(),
                },
            }]),
            None => AnthropicSystem::Text(text),
        });
        Ok(Self {
            model: request.model_id().as_str().to_owned(),
            system,
            messages,
            tools: request
                .tools()
                .iter()
                .map(AnthropicTool::from_schema)
                .collect(),
            max_tokens: request.max_output_tokens(),
        })
    }
}

#[derive(Serialize)]
#[serde(untagged)]
enum AnthropicSystem {
    Text(String),
    Blocks(Vec<AnthropicSystemBlock>),
}

#[derive(Serialize)]
struct AnthropicSystemBlock {
    #[serde(rename = "type")]
    kind: &'static str,
    text: String,
    cache_control: AnthropicCacheControl,
}

#[derive(Serialize)]
struct AnthropicCacheControl {
    #[serde(rename = "type")]
    kind: &'static str,
    ttl: &'static str,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: AnthropicRole,
    content: AnthropicContent,
}

impl AnthropicMessage {
    fn from_message(message: &ChatMessage) -> Result<Self, ProviderError> {
        match message.role() {
            MessageRole::System => Err(ProviderError::InvalidRequest(
                "system messages must use the Anthropic system field".to_owned(),
            )),
            MessageRole::User => Ok(Self {
                role: AnthropicRole::User,
                content: AnthropicContent::Text(message.content().to_owned()),
            }),
            MessageRole::Assistant => {
                let calls = message.tool_calls()?;
                if calls.is_empty() {
                    return Ok(Self {
                        role: AnthropicRole::Assistant,
                        content: AnthropicContent::Text(message.content().to_owned()),
                    });
                }
                let mut blocks = Vec::with_capacity(calls.len() + 1);
                if !message.content().is_empty() {
                    blocks.push(AnthropicContentBlock::Text {
                        text: message.content().to_owned(),
                    });
                }
                blocks.extend(
                    calls
                        .into_iter()
                        .map(|call| AnthropicContentBlock::ToolUse {
                            id: call.id().to_owned(),
                            name: call.name().to_owned(),
                            input: call.arguments().clone(),
                        }),
                );
                Ok(Self {
                    role: AnthropicRole::Assistant,
                    content: AnthropicContent::Blocks(blocks),
                })
            }
            MessageRole::Tool => {
                let tool_use_id = message.tool_call_id().ok_or_else(|| {
                    ProviderError::InvalidRequest(
                        "Anthropic tool results require a tool call ID".to_owned(),
                    )
                })?;
                Ok(Self {
                    role: AnthropicRole::User,
                    content: AnthropicContent::Blocks(vec![AnthropicContentBlock::ToolResult {
                        tool_use_id: tool_use_id.to_owned(),
                        content: message.content().to_owned(),
                    }]),
                })
            }
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum AnthropicRole {
    User,
    Assistant,
}

#[derive(Serialize)]
#[serde(untagged)]
enum AnthropicContent {
    Text(String),
    Blocks(Vec<AnthropicContentBlock>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

#[derive(Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: Value,
}

impl AnthropicTool {
    fn from_schema(schema: &ToolSchema) -> Self {
        Self {
            name: schema.name().to_owned(),
            description: schema.description.clone(),
            input_schema: schema.input_schema().clone(),
        }
    }
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicResponseBlock>,
    usage: Option<AnthropicUsage>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicResponseBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
}

#[derive(Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: u32,
    #[serde(default)]
    cache_read_input_tokens: u32,
}

#[derive(Serialize)]
struct GeminiRequest {
    #[serde(rename = "systemInstruction", skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiContent>,
    contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<GeminiTool>,
    #[serde(rename = "generationConfig")]
    generation_config: GeminiGenerationConfig,
}

impl GeminiRequest {
    fn from_request(request: &ModelRequest) -> Result<Self, ProviderError> {
        let system = request
            .messages()
            .iter()
            .filter(|message| message.role() == MessageRole::System)
            .map(ChatMessage::content)
            .collect::<Vec<_>>();
        let mut contents = Vec::new();
        for message in request
            .messages()
            .iter()
            .filter(|message| message.role() != MessageRole::System)
        {
            let content = match message.role() {
                MessageRole::System => unreachable!("system messages are filtered above"),
                MessageRole::User => GeminiContent {
                    role: Some(GeminiRole::User),
                    parts: vec![GeminiPart::Text {
                        text: message.content().to_owned(),
                    }],
                },
                MessageRole::Assistant => {
                    let calls = message.tool_calls()?;
                    let mut parts = Vec::with_capacity(calls.len() + 1);
                    if !message.content().is_empty() {
                        parts.push(GeminiPart::Text {
                            text: message.content().to_owned(),
                        });
                    }
                    parts.extend(calls.into_iter().map(|call| GeminiPart::FunctionCall {
                        function_call: GeminiFunctionCall {
                            name: call.name().to_owned(),
                            args: call.arguments().clone(),
                        },
                    }));
                    if parts.is_empty() {
                        return Err(ProviderError::InvalidRequest(
                            "assistant messages must contain text or a tool call".to_owned(),
                        ));
                    }
                    GeminiContent {
                        role: Some(GeminiRole::Model),
                        parts,
                    }
                }
                MessageRole::Tool => {
                    let tool_call_id = message.tool_call_id().ok_or_else(|| {
                        ProviderError::InvalidRequest(
                            "Gemini tool results require a tool call ID".to_owned(),
                        )
                    })?;
                    let name = request
                        .messages()
                        .iter()
                        .rev()
                        .find_map(|candidate| {
                            candidate
                                .tool_calls
                                .iter()
                                .find(|call| call.id == tool_call_id)
                                .map(|call| call.name.clone())
                        })
                        .ok_or_else(|| {
                            ProviderError::InvalidRequest(
                                "Gemini tool result has no matching tool call".to_owned(),
                            )
                        })?;
                    GeminiContent {
                        role: Some(GeminiRole::User),
                        parts: vec![GeminiPart::FunctionResponse {
                            function_response: GeminiFunctionResponse {
                                name,
                                response: serde_json::json!({
                                    "content": message.content()
                                }),
                            },
                        }],
                    }
                }
            };
            contents.push(content);
        }
        if contents.is_empty() {
            return Err(ProviderError::InvalidRequest(
                "Gemini requests require at least one conversation message".to_owned(),
            ));
        }
        let system_instruction = (!system.is_empty()).then(|| GeminiContent {
            role: None,
            parts: vec![GeminiPart::Text {
                text: system.join("\n\n"),
            }],
        });
        Ok(Self {
            system_instruction,
            contents,
            tools: if request.tools().is_empty() {
                Vec::new()
            } else {
                vec![GeminiTool {
                    function_declarations: request
                        .tools()
                        .iter()
                        .map(GeminiFunctionDeclaration::from_schema)
                        .collect(),
                }]
            },
            generation_config: GeminiGenerationConfig {
                max_output_tokens: request.max_output_tokens(),
            },
        })
    }
}

#[derive(Serialize)]
struct GeminiContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<GeminiRole>,
    parts: Vec<GeminiPart>,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum GeminiRole {
    User,
    Model,
}

#[derive(Serialize)]
#[serde(untagged)]
enum GeminiPart {
    Text {
        text: String,
    },
    FunctionCall {
        #[serde(rename = "functionCall")]
        function_call: GeminiFunctionCall,
    },
    FunctionResponse {
        #[serde(rename = "functionResponse")]
        function_response: GeminiFunctionResponse,
    },
}

#[derive(Clone, Deserialize, Serialize)]
struct GeminiFunctionCall {
    name: String,
    args: Value,
}

#[derive(Serialize)]
struct GeminiFunctionResponse {
    name: String,
    response: Value,
}

#[derive(Serialize)]
struct GeminiTool {
    #[serde(rename = "functionDeclarations")]
    function_declarations: Vec<GeminiFunctionDeclaration>,
}

#[derive(Serialize)]
struct GeminiFunctionDeclaration {
    name: String,
    description: String,
    parameters: Value,
}

impl GeminiFunctionDeclaration {
    fn from_schema(schema: &ToolSchema) -> Self {
        Self {
            name: schema.name().to_owned(),
            description: schema.description().to_owned(),
            parameters: schema.input_schema().clone(),
        }
    }
}

#[derive(Serialize)]
struct GeminiGenerationConfig {
    #[serde(rename = "maxOutputTokens")]
    max_output_tokens: u32,
}

#[derive(Deserialize)]
struct GeminiResponse {
    candidates: Vec<GeminiCandidate>,
    #[serde(rename = "usageMetadata")]
    usage: Option<GeminiUsage>,
}

#[derive(Deserialize)]
struct GeminiCandidate {
    content: Option<GeminiResponseContent>,
}

#[derive(Deserialize)]
struct GeminiResponseContent {
    parts: Vec<GeminiResponsePart>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum GeminiResponsePart {
    Text {
        text: String,
    },
    FunctionCall {
        #[serde(rename = "functionCall")]
        function_call: GeminiFunctionCall,
    },
}

#[derive(Deserialize)]
struct GeminiUsage {
    #[serde(rename = "promptTokenCount")]
    prompt_tokens: u32,
    #[serde(rename = "candidatesTokenCount")]
    completion_tokens: u32,
    #[serde(rename = "cachedContentTokenCount", default)]
    cached_prompt_tokens: u32,
}

fn parse_gemini_response(bytes: &[u8]) -> Result<ModelResponse, ProviderError> {
    if bytes.len() > MAX_RESPONSE_BYTES as usize {
        return Err(ProviderError::ResponseTooLarge);
    }
    let response: GeminiResponse =
        serde_json::from_slice(bytes).map_err(|_| ProviderError::InvalidResponse)?;
    let content = response
        .candidates
        .into_iter()
        .next()
        .and_then(|candidate| candidate.content)
        .ok_or(ProviderError::InvalidResponse)?;
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for (index, part) in content.parts.into_iter().enumerate() {
        match part {
            GeminiResponsePart::Text { text: block } => {
                if text.len().saturating_add(block.len()) > MAX_RESPONSE_TEXT_BYTES {
                    return Err(ProviderError::ResponseTooLarge);
                }
                text.push_str(&block);
            }
            GeminiResponsePart::FunctionCall { function_call } => {
                let id = format!("gemini-call-{index}");
                let call =
                    ToolCall::new(id, function_call.name, function_call.args).map_err(|error| {
                        match error {
                            ProviderError::InvalidToolArguments { call_id } => {
                                ProviderError::InvalidToolArguments { call_id }
                            }
                            _ => ProviderError::InvalidResponse,
                        }
                    })?;
                tool_calls.push(call);
            }
        }
    }
    let usage = response
        .usage
        .map(|usage| {
            TokenUsage::new(usage.prompt_tokens, usage.completion_tokens)
                .with_prompt_cache(usage.cached_prompt_tokens, 0)
        })
        .unwrap_or_default();
    Ok(ModelResponse::new(text, tool_calls, usage))
}

fn parse_anthropic_response(bytes: &[u8]) -> Result<ModelResponse, ProviderError> {
    if bytes.len() > MAX_RESPONSE_BYTES as usize {
        return Err(ProviderError::ResponseTooLarge);
    }
    let response: AnthropicResponse =
        serde_json::from_slice(bytes).map_err(|_| ProviderError::InvalidResponse)?;
    let mut text = String::new();
    let mut seen_ids = BTreeSet::new();
    let mut tool_calls = Vec::new();
    for block in response.content {
        match block {
            AnthropicResponseBlock::Text { text: block } => {
                if text.len().saturating_add(block.len()) > MAX_RESPONSE_TEXT_BYTES {
                    return Err(ProviderError::ResponseTooLarge);
                }
                text.push_str(&block);
            }
            AnthropicResponseBlock::ToolUse { id, name, input } => {
                if validate_tool_call_id(&id).is_err() {
                    return Err(ProviderError::InvalidResponse);
                }
                if !seen_ids.insert(id.clone()) {
                    return Err(ProviderError::DuplicateToolCallId { call_id: id });
                }
                let call = ToolCall::new(id, name, input).map_err(|error| match error {
                    ProviderError::InvalidToolArguments { call_id } => {
                        ProviderError::InvalidToolArguments { call_id }
                    }
                    _ => ProviderError::InvalidResponse,
                })?;
                tool_calls.push(call);
            }
        }
    }
    let usage = response
        .usage
        .map(|usage| {
            let prompt_tokens = usage
                .input_tokens
                .saturating_add(usage.cache_creation_input_tokens)
                .saturating_add(usage.cache_read_input_tokens);
            TokenUsage::new(prompt_tokens, usage.output_tokens).with_prompt_cache(
                usage.cache_read_input_tokens,
                usage.cache_creation_input_tokens,
            )
        })
        .unwrap_or_default();
    Ok(ModelResponse::new(text, tool_calls, usage))
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
        .map(|usage| {
            TokenUsage::new(usage.prompt_tokens, usage.completion_tokens).with_prompt_cache(
                usage
                    .prompt_tokens_details
                    .map(|details| details.cached_tokens)
                    .unwrap_or(0),
                0,
            )
        })
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
    fn anthropic_request_translates_system_tools_and_tool_results() {
        let manifest = ProviderManifest::new_with_protocol(
            "anthropic",
            "Anthropic",
            ProviderProtocol::AnthropicMessages,
            "https://api.anthropic.com/v1",
            "claude-sonnet-4-20250514",
            "PANDORA_ANTHROPIC_API_KEY",
        )
        .unwrap();
        let call =
            ToolCall::new("toolu-1", "workspace.read", json!({"path": "README.md"})).unwrap();
        let request = ModelRequest::new(
            manifest.id().clone(),
            manifest.default_model().clone(),
            vec![
                ChatMessage::system("Follow policy.").unwrap(),
                ChatMessage::user("Read README.").unwrap(),
                ChatMessage::assistant_tool_calls(&[call]).unwrap(),
                ChatMessage::tool_result("toolu-1", "fixture").unwrap(),
            ],
        )
        .unwrap()
        .with_tools(vec![
            ToolSchema::new(
                "workspace.read",
                "Read a workspace file",
                json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "required": ["path"]
                }),
            )
            .unwrap(),
        ])
        .unwrap();

        let body = serde_json::to_value(AnthropicRequest::from_request(&request).unwrap()).unwrap();

        assert_eq!(body["system"], "Follow policy.");
        assert_eq!(
            body["messages"][0],
            json!({"role":"user","content":"Read README."})
        );
        assert_eq!(
            body["messages"][1],
            json!({
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "id": "toolu-1",
                    "name": "workspace.read",
                    "input": {"path": "README.md"}
                }]
            })
        );
        assert_eq!(
            body["messages"][2],
            json!({
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "toolu-1",
                    "content": "fixture"
                }]
            })
        );
        assert_eq!(body["tools"][0]["name"], "workspace.read");
        assert_eq!(body["tools"][0]["input_schema"]["required"][0], "path");
    }

    #[test]
    fn anthropic_prompt_cache_marks_only_the_stable_system_prefix() {
        let manifest = ProviderManifest::new_with_protocol(
            "anthropic",
            "Anthropic",
            ProviderProtocol::AnthropicMessages,
            "https://api.anthropic.com/v1",
            "claude-sonnet-4-20250514",
            "PANDORA_ANTHROPIC_API_KEY",
        )
        .unwrap();
        let request = ModelRequest::new(
            manifest.id().clone(),
            manifest.default_model().clone(),
            vec![
                ChatMessage::system("Stable governed context.").unwrap(),
                ChatMessage::user("Dynamic task.").unwrap(),
            ],
        )
        .unwrap()
        .with_prompt_cache(PromptCacheTtl::FiveMinutes);

        let body = serde_json::to_value(AnthropicRequest::from_request(&request).unwrap()).unwrap();

        assert_eq!(
            body["system"],
            json!([{
                "type": "text",
                "text": "Stable governed context.",
                "cache_control": {"type": "ephemeral", "ttl": "5m"}
            }])
        );
        assert_eq!(
            body["messages"][0],
            json!({"role": "user", "content": "Dynamic task."})
        );
        assert!(body["messages"][0].get("cache_control").is_none());
    }

    #[test]
    fn anthropic_response_normalizes_text_tools_and_usage() {
        let response = parse_anthropic_response(
            br#"{
                "content": [
                    {"type":"text","text":"Checking."},
                    {"type":"tool_use","id":"toolu-1","name":"workspace.read","input":{"path":"README.md"}}
                ],
                "usage":{
                    "input_tokens":7,
                    "output_tokens":3,
                    "cache_creation_input_tokens":11,
                    "cache_read_input_tokens":13
                }
            }"#,
        )
        .unwrap();

        assert_eq!(response.text(), "Checking.");
        assert_eq!(response.tool_calls().len(), 1);
        assert_eq!(response.tool_calls()[0].id(), "toolu-1");
        assert_eq!(response.tool_calls()[0].name(), "workspace.read");
        assert_eq!(response.usage().prompt_tokens(), 31);
        assert_eq!(response.usage().completion_tokens(), 3);
        assert_eq!(response.usage().cached_prompt_tokens(), 13);
        assert_eq!(response.usage().cache_write_prompt_tokens(), 11);
    }

    #[test]
    fn gemini_request_translates_system_tools_and_tool_results() {
        let manifest = ProviderManifest::new_with_protocol(
            "gemini",
            "Gemini",
            ProviderProtocol::GeminiGenerateContent,
            "https://generativelanguage.googleapis.com/v1beta",
            "gemini-2.5-pro",
            "PANDORA_GEMINI_API_KEY",
        )
        .unwrap();
        let call = ToolCall::new("call-1", "workspace.read", json!({"path": "README.md"})).unwrap();
        let request = ModelRequest::new(
            manifest.id().clone(),
            manifest.default_model().clone(),
            vec![
                ChatMessage::system("Follow policy.").unwrap(),
                ChatMessage::user("Read README.").unwrap(),
                ChatMessage::assistant_tool_calls(&[call]).unwrap(),
                ChatMessage::tool_result("call-1", "fixture").unwrap(),
            ],
        )
        .unwrap()
        .with_tools(vec![
            ToolSchema::new(
                "workspace.read",
                "Read a workspace file",
                json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "required": ["path"]
                }),
            )
            .unwrap(),
        ])
        .unwrap();

        let body = serde_json::to_value(GeminiRequest::from_request(&request).unwrap()).unwrap();

        assert_eq!(
            body["systemInstruction"],
            json!({"parts":[{"text":"Follow policy."}]})
        );
        assert_eq!(
            body["contents"][0],
            json!({
                "role":"user",
                "parts":[{"text":"Read README."}]
            })
        );
        assert_eq!(
            body["contents"][1],
            json!({
                "role":"model",
                "parts":[{"functionCall":{
                    "name":"workspace.read",
                    "args":{"path":"README.md"}
                }}]
            })
        );
        assert_eq!(
            body["contents"][2],
            json!({
                "role":"user",
                "parts":[{"functionResponse":{
                    "name":"workspace.read",
                    "response":{"content":"fixture"}
                }}]
            })
        );
        assert_eq!(
            body["tools"][0]["functionDeclarations"][0]["name"],
            "workspace.read"
        );
        assert_eq!(body["generationConfig"]["maxOutputTokens"], 1024);
    }

    #[test]
    fn gemini_response_normalizes_text_tools_and_usage() {
        let response = parse_gemini_response(
            br#"{
                "candidates":[{"content":{"parts":[
                    {"text":"Checking."},
                    {"functionCall":{"name":"workspace.read","args":{"path":"README.md"}}}
                ]}}],
                "usageMetadata":{"promptTokenCount":7,"candidatesTokenCount":3}
            }"#,
        )
        .unwrap();

        assert_eq!(response.text(), "Checking.");
        assert_eq!(response.tool_calls().len(), 1);
        assert_eq!(response.tool_calls()[0].id(), "gemini-call-1");
        assert_eq!(response.tool_calls()[0].name(), "workspace.read");
        assert_eq!(response.usage().prompt_tokens(), 7);
        assert_eq!(response.usage().completion_tokens(), 3);
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

    #[test]
    fn anthropic_provider_uses_native_endpoint_headers_and_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let header_end = loop {
                let mut chunk = [0_u8; 1_024];
                let bytes_read = stream.read(&mut chunk).unwrap();
                request.extend_from_slice(&chunk[..bytes_read]);
                if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    break position + 4;
                }
            };
            let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
            let content_length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap();
            while request.len() < header_end + content_length {
                let mut chunk = [0_u8; 1_024];
                let bytes_read = stream.read(&mut chunk).unwrap();
                request.extend_from_slice(&chunk[..bytes_read]);
            }
            let request_line = headers.lines().next().unwrap();
            let body = String::from_utf8_lossy(&request[header_end..header_end + content_length]);
            assert!(request_line.contains("post /v1/messages http/1.1"));
            assert!(headers.contains("x-api-key: anthropic-secret"));
            assert!(headers.contains("anthropic-version: 2023-06-01"));
            assert!(!headers.contains("authorization: bearer"));
            assert!(body.contains("\"model\":\"claude-sonnet-4-20250514\""));
            let response = br#"{"content":[{"type":"text","text":"ready"}],"usage":{"input_tokens":2,"output_tokens":1}}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            )
            .unwrap();
            stream.write_all(response).unwrap();
        });
        let manifest = ProviderManifest::new_with_protocol(
            "anthropic",
            "Anthropic",
            ProviderProtocol::AnthropicMessages,
            format!("http://{address}/v1"),
            "claude-sonnet-4-20250514",
            "PANDORA_ANTHROPIC_API_KEY",
        )
        .unwrap();
        let request = ModelRequest::new(
            manifest.id().clone(),
            manifest.default_model().clone(),
            vec![ChatMessage::user("hello").unwrap()],
        )
        .unwrap();
        let provider = HttpProvider::new(manifest, "anthropic-secret").unwrap();

        let response = provider.complete(request).unwrap();

        server.join().unwrap();
        assert_eq!(response.text(), "ready");
        assert_eq!(response.usage().total_tokens(), 3);
    }

    #[test]
    fn gemini_provider_uses_native_endpoint_headers_and_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let header_end = loop {
                let mut chunk = [0_u8; 1_024];
                let bytes_read = stream.read(&mut chunk).unwrap();
                request.extend_from_slice(&chunk[..bytes_read]);
                if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    break position + 4;
                }
            };
            let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
            let content_length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap();
            while request.len() < header_end + content_length {
                let mut chunk = [0_u8; 1_024];
                let bytes_read = stream.read(&mut chunk).unwrap();
                request.extend_from_slice(&chunk[..bytes_read]);
            }
            let request_line = headers.lines().next().unwrap();
            let body = String::from_utf8_lossy(&request[header_end..header_end + content_length]);
            assert!(
                request_line
                    .contains("post /v1beta/models/gemini-2.5-pro:generatecontent http/1.1")
            );
            assert!(headers.contains("x-goog-api-key: gemini-secret"));
            assert!(!headers.contains("authorization: bearer"));
            assert!(body.contains("\"maxOutputTokens\":1024"));
            let response = br#"{
                "candidates":[{"content":{"parts":[{"text":"ready"}]}}],
                "usageMetadata":{"promptTokenCount":2,"candidatesTokenCount":1}
            }"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            )
            .unwrap();
            stream.write_all(response).unwrap();
        });
        let manifest = ProviderManifest::new_with_protocol(
            "gemini",
            "Gemini",
            ProviderProtocol::GeminiGenerateContent,
            format!("http://{address}/v1beta"),
            "gemini-2.5-pro",
            "PANDORA_GEMINI_API_KEY",
        )
        .unwrap();
        let request = ModelRequest::new(
            manifest.id().clone(),
            manifest.default_model().clone(),
            vec![ChatMessage::user("hello").unwrap()],
        )
        .unwrap();
        let provider = HttpProvider::new(manifest, "gemini-secret").unwrap();

        let response = provider.complete(request).unwrap();

        server.join().unwrap();
        assert_eq!(response.text(), "ready");
        assert_eq!(response.usage().total_tokens(), 3);
    }
}
