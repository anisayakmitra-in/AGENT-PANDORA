use crate::manifest::{ManifestError, ModelId, ProviderId, ProviderManifest, ProviderProtocol};
use pandora_types::{ExecutionId, ReasoningEffort, SessionId};
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
    reasoning_effort: Option<ReasoningEffort>,
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
            reasoning_effort: None,
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

    pub fn with_reasoning_effort(mut self, effort: ReasoningEffort) -> Self {
        self.reasoning_effort = Some(effort);
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

    pub const fn reasoning_effort(&self) -> Option<ReasoningEffort> {
        self.reasoning_effort
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
            reasoning_effort: self.reasoning_effort.map(|e| e.as_str()),
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
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'a str>,
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
                let model_name = model
                    .as_str()
                    .strip_prefix("models/")
                    .unwrap_or(model.as_str());
                let base = self.manifest.base_url().trim_end_matches('/');
                if base.ends_with(":generateContent") {
                    return base.to_owned();
                }
                return format!("{base}/models/{model_name}:generateContent");
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
            ProviderProtocol::GeminiGenerateContent => {
                let mut req = self
                    .client
                    .post(self.endpoint(request.model_id()))
                    .timeout(request.timeout());
                let key = self.api_key.expose();
                if key.starts_with("ya29.") || key.starts_with("Bearer ") {
                    let token = key.strip_prefix("Bearer ").unwrap_or(key);
                    req = req.header("Authorization", format!("Bearer {token}"));
                } else {
                    req = req.header("x-goog-api-key", key);
                }
                req.json(&GeminiRequest::from_request(&request)?).send()
            }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
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
            reasoning_effort: request
                .reasoning_effort()
                .and_then(|effort| effort.openai_reasoning_effort())
                .map(str::to_owned),
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
struct AnthropicThinking {
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    budget_tokens: Option<u32>,
}

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<AnthropicSystem>,
    messages: Vec<AnthropicMessage>,
    tools: Vec<AnthropicTool>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<AnthropicThinking>,
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
        let thinking = request.reasoning_effort().and_then(|effort| {
            effort
                .anthropic_thinking_budget()
                .map(|budget| AnthropicThinking {
                    kind: "enabled",
                    budget_tokens: Some(budget),
                })
        });
        let max_tokens = match thinking.as_ref().and_then(|t| t.budget_tokens) {
            Some(budget) if request.max_output_tokens() <= budget => budget + 1024,
            _ => request.max_output_tokens(),
        };
        Ok(Self {
            model: request.model_id().as_str().to_owned(),
            system,
            messages,
            tools: request
                .tools()
                .iter()
                .map(AnthropicTool::from_schema)
                .collect(),
            max_tokens,
            thinking,
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
    #[serde(default)]
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
    #[serde(default)]
    parts: Vec<GeminiResponsePart>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum GeminiResponsePart {
    Text {
        text: String,
        #[serde(default)]
        thought: bool,
    },
    FunctionCall {
        #[serde(rename = "functionCall")]
        function_call: GeminiFunctionCall,
    },
    #[allow(dead_code)]
    Other(serde_json::Value),
}

#[derive(Deserialize)]
struct GeminiUsage {
    #[serde(rename = "promptTokenCount", default)]
    prompt_tokens: u32,
    #[serde(rename = "candidatesTokenCount", default)]
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
    let mut thoughts = String::new();
    let mut tool_calls = Vec::new();
    for (index, part) in content.parts.into_iter().enumerate() {
        match part {
            GeminiResponsePart::Text {
                text: block,
                thought,
            } => {
                if thought {
                    if thoughts.len().saturating_add(block.len()) <= MAX_RESPONSE_TEXT_BYTES {
                        thoughts.push_str(&block);
                    }
                    continue;
                }
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
            GeminiResponsePart::Other(_) => {}
        }
    }
    if text.is_empty() && tool_calls.is_empty() && !thoughts.is_empty() {
        text = thoughts;
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
#[path = "client_tests.rs"]
mod tests;
