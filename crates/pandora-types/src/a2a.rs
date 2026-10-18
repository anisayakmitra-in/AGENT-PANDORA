use serde::{Deserialize, Serialize};

/// Agent Card representing agent identity, protocol capabilities, and endpoints for A2A discovery.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct A2aAgentCard {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub capabilities: Vec<String>,
    pub supported_models: Vec<String>,
    pub endpoints: Vec<String>,
    pub auth_mode: String,
}

impl A2aAgentCard {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        version: impl Into<String>,
        description: impl Into<String>,
        capabilities: Vec<String>,
        supported_models: Vec<String>,
        endpoints: Vec<String>,
        auth_mode: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            version: version.into(),
            description: description.into(),
            capabilities,
            supported_models,
            endpoints,
            auth_mode: auth_mode.into(),
        }
    }
}

/// Execution status of an Agent-to-Agent delegated task.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum A2aTaskStatus {
    Pending,
    Executing,
    Completed,
    Failed,
    Cancelled,
}

impl A2aTaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Executing => "executing",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl std::str::FromStr for A2aTaskStatus {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_lowercase().as_str() {
            "pending" => Ok(Self::Pending),
            "executing" => Ok(Self::Executing),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(()),
        }
    }
}

/// Bounded task delegation request sent from one agent to another over A2A.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct A2aTaskRequest {
    pub task_id: String,
    pub from_agent: String,
    pub to_agent: String,
    pub intent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    pub timeout_seconds: u64,
    pub created_at: u64,
}

/// Result returned from an A2A task delegation execution.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct A2aTaskResponse {
    pub task_id: String,
    pub status: A2aTaskStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<u64>,
}

/// Standard JSON-RPC 2.0 request envelope for A2A protocol messaging.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct A2aRpcRequest {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// Standard JSON-RPC 2.0 error representation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct A2aRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl A2aRpcError {
    pub fn method_not_found(method: &str) -> Self {
        Self {
            code: -32601,
            message: format!("Method '{method}' not found"),
            data: None,
        }
    }

    pub fn invalid_params(msg: impl Into<String>) -> Self {
        Self {
            code: -32602,
            message: msg.into(),
            data: None,
        }
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Self {
            code: -32603,
            message: msg.into(),
            data: None,
        }
    }
}

/// Standard JSON-RPC 2.0 response envelope for A2A protocol messaging.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct A2aRpcResponse {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<A2aRpcError>,
}

impl A2aRpcResponse {
    pub fn success(id: serde_json::Value, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: serde_json::Value, error: A2aRpcError) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(error),
        }
    }
}
