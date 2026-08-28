#![forbid(unsafe_code)]

use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
};
use pandora_runtime::{RuntimeService, RuntimeServiceError, ServiceTokenStore};
use pandora_types::{
    ServiceAgentResumeRequest, ServiceAgentRunRequest, ServiceEventPageRequest, ServiceRequest,
    ServiceResponse, ServiceRunRequest, ServiceRunResumeRequest, Timestamp,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_RPC_BODY_BYTES: usize = 1_048_576;

pub struct LocalServiceConfig {
    bind_addr: SocketAddr,
    runtime: RuntimeService,
    token_store: ServiceTokenStore,
}

impl LocalServiceConfig {
    pub fn new(
        bind_addr: SocketAddr,
        runtime: RuntimeService,
        token_store: ServiceTokenStore,
    ) -> Result<Self, ServiceTransportError> {
        if !bind_addr.ip().is_loopback() {
            return Err(ServiceTransportError::NonLoopbackBindAddress);
        }

        Ok(Self {
            bind_addr,
            runtime,
            token_store,
        })
    }
}

pub struct LocalService {
    bind_addr: SocketAddr,
    state: Arc<TransportState>,
}

impl LocalService {
    pub fn new(config: LocalServiceConfig) -> Self {
        Self {
            bind_addr: config.bind_addr,
            state: Arc::new(TransportState {
                runtime: Arc::new(config.runtime),
                token_store: Arc::new(config.token_store),
            }),
        }
    }

    fn router(&self) -> Router {
        Router::new()
            .route("/v1/rpc", post(handle_rpc))
            .layer(DefaultBodyLimit::max(MAX_RPC_BODY_BYTES))
            .layer(middleware::from_fn_with_state(
                Arc::clone(&self.state),
                require_bearer,
            ))
            .with_state(Arc::clone(&self.state))
    }

    pub async fn bind(self) -> Result<BoundLocalService, ServiceTransportError> {
        let listener = tokio::net::TcpListener::bind(self.bind_addr)
            .await
            .map_err(ServiceTransportError::Bind)?;
        let local_addr = listener.local_addr().map_err(ServiceTransportError::Bind)?;

        Ok(BoundLocalService {
            listener,
            router: self.router(),
            local_addr,
        })
    }
}

pub struct BoundLocalService {
    listener: tokio::net::TcpListener,
    router: Router,
    local_addr: SocketAddr,
}

impl BoundLocalService {
    pub const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub async fn serve_until<F>(self, shutdown: F) -> Result<(), ServiceTransportError>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        axum::serve(self.listener, self.router)
            .with_graceful_shutdown(shutdown)
            .await
            .map_err(ServiceTransportError::Serve)
    }
}

#[derive(Debug)]
pub enum ServiceTransportError {
    NonLoopbackBindAddress,
    Bind(std::io::Error),
    Serve(std::io::Error),
}

impl fmt::Display for ServiceTransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonLoopbackBindAddress => {
                formatter.write_str("local service must bind to a loopback address")
            }
            Self::Bind(_) => formatter.write_str("could not bind the local service"),
            Self::Serve(_) => formatter.write_str("local service stopped unexpectedly"),
        }
    }
}

impl std::error::Error for ServiceTransportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Bind(error) | Self::Serve(error) => Some(error),
            Self::NonLoopbackBindAddress => None,
        }
    }
}

struct TransportState {
    runtime: Arc<RuntimeService>,
    token_store: Arc<ServiceTokenStore>,
}

async fn require_bearer(
    State(state): State<Arc<TransportState>>,
    request: Request,
    next: Next,
) -> Response {
    let authorized = bearer_token(request.headers())
        .is_some_and(|candidate| state.token_store.token().matches(candidate));

    if authorized {
        next.run(request).await
    } else {
        StatusCode::UNAUTHORIZED.into_response()
    }
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .filter(|candidate| !candidate.is_empty())
}

async fn handle_rpc(
    State(state): State<Arc<TransportState>>,
    body: Bytes,
) -> Json<JsonRpcResponse> {
    let request = match serde_json::from_slice::<JsonRpcRequest>(&body) {
        Ok(request) if request.is_valid() => request,
        _ => return Json(JsonRpcResponse::invalid_request()),
    };
    let id = request.id.clone();

    let response = match service_request(&request) {
        Ok(Some(request)) => match state.runtime.handle(&request, now_timestamp()) {
            Ok(response) => JsonRpcResponse::success(id, response),
            Err(error) => JsonRpcResponse::runtime_error(id, error),
        },
        Ok(None) => JsonRpcResponse::method_not_found(id),
        Err(()) => JsonRpcResponse::invalid_params(id),
    };

    Json(response)
}

fn service_request(request: &JsonRpcRequest) -> Result<Option<ServiceRequest>, ()> {
    let params = &request.params;
    let request = match request.method.as_str() {
        "runtime.health" => {
            if params.is_null() || params == &Value::Object(Default::default()) {
                ServiceRequest::health()
            } else {
                return Err(());
            }
        }
        "runtime.capabilities" => {
            if params.is_null() || params == &Value::Object(Default::default()) {
                ServiceRequest::capabilities()
            } else {
                return Err(());
            }
        }
        "runtime.providers" => {
            if params.is_null() || params == &Value::Object(Default::default()) {
                ServiceRequest::providers()
            } else {
                return Err(());
            }
        }
        "runtime.engines" => {
            if params.is_null() || params == &Value::Object(Default::default()) {
                ServiceRequest::engines()
            } else {
                return Err(());
            }
        }
        "runtime.tools" => {
            if params.is_null() || params == &Value::Object(Default::default()) {
                ServiceRequest::tools()
            } else {
                return Err(());
            }
        }
        "session.list" => {
            let params: SessionListParams = deserialize_params(params)?;
            ServiceRequest::session_list(params.limit).map_err(|_| ())?
        }
        "session.inspect" => {
            let params: SessionInspectParams = deserialize_params(params)?;
            ServiceRequest::session_inspect(params.session_id).map_err(|_| ())?
        }
        "session.events" => {
            let params: ServiceEventPageRequest = deserialize_params(params)?;
            ServiceRequest::session_events(params)
        }
        "session.memory" => {
            let params: SessionMemoryParams = deserialize_params(params)?;
            ServiceRequest::session_memory(params.session_id, params.limit).map_err(|_| ())?
        }
        "approval.list" => {
            let params: ApprovalListParams = deserialize_params(params)?;
            ServiceRequest::approval_list(params.limit).map_err(|_| ())?
        }
        "approval.inspect" => {
            let params: ApprovalInspectParams = deserialize_params(params)?;
            ServiceRequest::approval_inspect(params.approval_id).map_err(|_| ())?
        }
        "approval.resolve" => {
            let params: ApprovalResolveParams = deserialize_params(params)?;
            ServiceRequest::approval_resolve(params.approval_id, params.allow).map_err(|_| ())?
        }
        "evolution.list" => {
            let params: EvolutionListParams = deserialize_params(params)?;
            ServiceRequest::evolution_list(params.limit).map_err(|_| ())?
        }
        "evolution.inspect" => {
            let params: EvolutionInspectParams = deserialize_params(params)?;
            ServiceRequest::evolution_inspect(params.proposal_id).map_err(|_| ())?
        }
        "evolution.activations" => {
            let params: EvolutionListParams = deserialize_params(params)?;
            ServiceRequest::evolution_activations(params.limit).map_err(|_| ())?
        }
        "evolution.activate" => {
            let params: EvolutionActivateParams = deserialize_params(params)?;
            ServiceRequest::evolution_activate(params.proposal_id, params.confirmation)
                .map_err(|_| ())?
        }
        "evolution.rollback" => {
            let params: EvolutionRollbackParams = deserialize_params(params)?;
            ServiceRequest::evolution_rollback(
                params.proposal_id,
                params.confirmation,
                params.reason,
            )
            .map_err(|_| ())?
        }
        "run.execute" => {
            let params: ServiceRunRequest = deserialize_params(params)?;
            ServiceRequest::run(params)
        }
        "run.resume" => {
            let params: ServiceRunResumeRequest = deserialize_params(params)?;
            ServiceRequest::run_resume(params)
        }
        "agent.execute" => {
            let params: ServiceAgentRunRequest = deserialize_params(params)?;
            ServiceRequest::agent_run(params)
        }
        "agent.resume" => {
            let params: ServiceAgentResumeRequest = deserialize_params(params)?;
            ServiceRequest::agent_resume(params)
        }
        _ => return Ok(None),
    };

    Ok(Some(request))
}

fn deserialize_params<T>(params: &Value) -> Result<T, ()>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value(params.clone()).map_err(|_| ())
}

fn now_timestamp() -> Timestamp {
    Timestamp::from_unix_seconds(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default(),
    )
}

#[derive(Clone, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

impl JsonRpcRequest {
    fn is_valid(&self) -> bool {
        self.jsonrpc == "2.0"
            && matches!(self.id, Value::Null | Value::Number(_) | Value::String(_))
            && !self.method.trim().is_empty()
    }
}

#[derive(Deserialize)]
struct SessionListParams {
    limit: u16,
}

#[derive(Deserialize)]
struct SessionInspectParams {
    session_id: String,
}

#[derive(Deserialize)]
struct SessionMemoryParams {
    session_id: String,
    limit: u16,
}

#[derive(Deserialize)]
struct ApprovalInspectParams {
    approval_id: String,
}

#[derive(Deserialize)]
struct ApprovalListParams {
    limit: u16,
}

#[derive(Deserialize)]
struct EvolutionActivateParams {
    proposal_id: String,
    confirmation: String,
}

#[derive(Clone, Deserialize)]
struct EvolutionRollbackParams {
    proposal_id: String,
    confirmation: String,
    reason: String,
}

#[derive(Clone, Deserialize)]
struct ApprovalResolveParams {
    approval_id: String,
    allow: bool,
}

#[derive(Deserialize)]
struct EvolutionListParams {
    limit: u16,
}

#[derive(Deserialize)]
struct EvolutionInspectParams {
    proposal_id: String,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<ServiceResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    fn success(id: Value, result: ServiceResponse) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    fn invalid_request() -> Self {
        Self::error(Value::Null, -32600, "invalid request", None)
    }

    fn method_not_found(id: Value) -> Self {
        Self::error(id, -32601, "method not found", None)
    }

    fn invalid_params(id: Value) -> Self {
        Self::error(id, -32602, "invalid params", None)
    }

    fn runtime_error(id: Value, error: RuntimeServiceError) -> Self {
        Self::error(id, -32603, "runtime error", Some(error.code()))
    }

    fn error(
        id: Value,
        code: i32,
        message: &'static str,
        runtime_code: Option<&'static str>,
    ) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message,
                data: runtime_code.map(|code| JsonRpcErrorData { code }),
            }),
        }
    }
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i32,
    message: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<JsonRpcErrorData>,
}

#[derive(Serialize)]
struct JsonRpcErrorData {
    code: &'static str,
}

#[cfg(test)]
mod tests {
    use super::{LocalService, LocalServiceConfig};
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode, header};
    use pandora_runtime::{
        ApprovalStore, ArtifactCatalog, EvolutionEngine, ExecutionController, RuntimeService,
        RuntimeServiceScope, ServiceTokenStore,
    };
    use pandora_runtime::{executors::WorkspaceRoot, sessions::SessionStore};
    use pandora_types::{
        Capability, EvolutionPolicy, Operation, PolicyContext, PrincipalId, ServiceProviderSummary,
        TenantId, WorkspaceId,
    };
    use serde_json::{Value, json};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    use tower::ServiceExt;

    static NEXT_FIXTURE_ROOT: AtomicU64 = AtomicU64::new(1);

    #[tokio::test]
    async fn rpc_rejects_missing_or_wrong_bearer_tokens() {
        let fixture = Fixture::new();

        let missing = post(&fixture, None, health_request()).await;
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

        let wrong = post(&fixture, Some("wrong"), health_request()).await;
        assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn rpc_runs_a_scoped_request_through_the_runtime_facade() {
        let fixture = Fixture::new();
        let response = post(
            &fixture,
            Some(&fixture.token),
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "run.execute",
                "params": {"task": "guide"}
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["result"]["kind"], "run");
        assert_eq!(body["result"]["run"]["status"], "completed");
    }

    #[tokio::test]
    async fn rpc_exposes_agent_execution_without_bypassing_provider_configuration() {
        let fixture = Fixture::new();
        let response = post(
            &fixture,
            Some(&fixture.token),
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "agent.execute",
                "params": {"task": "Inspect this repository"}
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["error"]["data"]["code"], "agent_unavailable");
    }

    #[tokio::test]
    async fn rpc_exposes_real_evolution_inventory() {
        let fixture = Fixture::new();
        let response = post(
            &fixture,
            Some(&fixture.token),
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "evolution.list",
                "params": {"limit": 64}
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["result"]["kind"], "evolution_list");
        assert_eq!(body["result"]["proposals"], json!([]));

        let response = post(
            &fixture,
            Some(&fixture.token),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "evolution.activations",
                "params": {"limit": 64}
            }),
        )
        .await;
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["result"]["kind"], "evolution_activations");
        assert_eq!(body["result"]["activations"], json!([]));
    }

    #[tokio::test]
    async fn rpc_requires_exact_confirmation_for_evolution_mutations() {
        let fixture = Fixture::new();
        let invalid = post(
            &fixture,
            Some(&fixture.token),
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "evolution.activate",
                "params": {"proposal_id": "proposal-a", "confirmation": "proposal-b"}
            }),
        )
        .await;
        let body = to_bytes(invalid.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["error"]["code"], -32602);

        let guarded = post(
            &fixture,
            Some(&fixture.token),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "evolution.activate",
                "params": {"proposal_id": "proposal-a", "confirmation": "proposal-a"}
            }),
        )
        .await;
        let body = to_bytes(guarded.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            body["error"]["data"]["code"],
            "evolution_control_unavailable"
        );

        let rollback_without_reason = post(
            &fixture,
            Some(&fixture.token),
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "evolution.rollback",
                "params": {
                    "proposal_id": "proposal-a",
                    "confirmation": "proposal-a",
                    "reason": ""
                }
            }),
        )
        .await;
        let body = to_bytes(rollback_without_reason.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["error"]["code"], -32602);
    }

    #[tokio::test]
    async fn rpc_resolves_and_resumes_an_exact_approval() {
        let fixture = Fixture::new();
        std::fs::write(fixture._root.root.join("README.md"), "fixture\n").unwrap();
        let first = post(
            &fixture,
            Some(&fixture.token),
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "run.execute",
                "params": {"task": "patch:README.md:approved"}
            }),
        )
        .await;
        let body = to_bytes(first.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["result"]["run"]["status"], "approval_required");
        let approval_id = body["result"]["run"]["approval"]["approval_id"]
            .as_str()
            .unwrap();

        let listed = post(
            &fixture,
            Some(&fixture.token),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "approval.list",
                "params": {"limit": 32}
            }),
        )
        .await;
        let body = to_bytes(listed.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["result"]["approvals"][0]["approval_id"], approval_id);

        let resolved = post(
            &fixture,
            Some(&fixture.token),
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "approval.resolve",
                "params": {"approval_id": approval_id, "allow": true}
            }),
        )
        .await;
        let body = to_bytes(resolved.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["result"]["approval"]["status"], "approved");

        let resumed = post(
            &fixture,
            Some(&fixture.token),
            json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "run.resume",
                "params": {
                    "approval_id": approval_id,
                    "request": {"task": "patch:README.md:approved"}
                }
            }),
        )
        .await;
        let body = to_bytes(resumed.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["result"]["run"]["status"], "completed");
        assert_eq!(
            std::fs::read_to_string(fixture._root.root.join("README.md")).unwrap(),
            "approved"
        );
    }

    #[tokio::test]
    async fn rpc_returns_memory_for_a_selected_session() {
        let fixture = Fixture::new();
        let run = post(
            &fixture,
            Some(&fixture.token),
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "run.execute",
                "params": {"task": "guide"}
            }),
        )
        .await;
        let body = to_bytes(run.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        let session_id = body["result"]["run"]["session_id"].as_str().unwrap();

        let response = post(
            &fixture,
            Some(&fixture.token),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "session.memory",
                "params": {"session_id": session_id, "limit": 16}
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["result"]["kind"], "session_memory");
        assert_eq!(body["result"]["memory"]["session_id"], session_id);
        assert!(body["result"]["memory"]["records"].is_array());
    }

    #[tokio::test]
    async fn rpc_returns_the_runtime_harness_catalog() {
        let fixture = Fixture::new();
        let response = post(
            &fixture,
            Some(&fixture.token),
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "runtime.capabilities"
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["result"]["kind"], "capabilities");
        assert!(
            body["result"]["harnesses"]
                .as_array()
                .is_some_and(|harnesses| !harnesses.is_empty())
        );
    }

    #[tokio::test]
    async fn rpc_returns_redacted_provider_catalog() {
        let fixture = Fixture::new();
        let response = post(
            &fixture,
            Some(&fixture.token),
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "runtime.providers"
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["result"]["kind"], "providers");
        assert_eq!(body["result"]["providers"][0]["name"], "fixture-provider");
        assert!(
            !body["result"]["providers"][0]
                .as_object()
                .unwrap()
                .contains_key("api_key_env")
        );
    }

    #[tokio::test]
    async fn rpc_returns_the_pandora_engine_inventory() {
        let fixture = Fixture::new();
        let response = post(
            &fixture,
            Some(&fixture.token),
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "runtime.engines"
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["result"]["kind"], "engines");
        assert_eq!(body["result"]["engines"][0]["id"], "execution-controller");
        assert_eq!(
            body["result"]["engines"][1]["authority"],
            "Sole permit issuer"
        );
        let engine_ids = body["result"]["engines"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|engine| engine["id"].as_str())
            .collect::<Vec<_>>();
        for expected in [
            "adaptive-engine",
            "coding-feedback-loop",
            "efficiency-engine",
            "graph-intelligence-engine",
            "orchestration-engine",
            "self-healing-engine",
            "skill-engine",
            "observability-engine",
            "fleet-engine",
            "mutation-engine",
            "replacement-engine",
        ] {
            assert!(engine_ids.contains(&expected), "missing engine {expected}");
        }
    }

    #[tokio::test]
    async fn rpc_returns_the_builtin_tool_catalog_without_credentials() {
        let fixture = Fixture::new();
        let response = post(
            &fixture,
            Some(&fixture.token),
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "runtime.tools"
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["result"]["kind"], "tools");
        assert_eq!(body["result"]["tools"][0]["id"], "accessibility.evidence");
        assert_eq!(body["result"]["tools"][0]["capability"], "filesystem.read");
        assert!(
            !body["result"]["tools"][0]
                .as_object()
                .unwrap()
                .contains_key("credential")
        );
    }

    #[tokio::test]
    async fn rpc_rejects_bodies_above_the_service_limit() {
        let fixture = Fixture::new();
        let response = post_bytes(
            &fixture,
            Some(&fixture.token),
            vec![b'x'; crate::MAX_RPC_BODY_BYTES + 1],
        )
        .await;

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn transport_refuses_non_loopback_bind_addresses() {
        let fixture = FixtureRoot::new();

        assert!(
            LocalServiceConfig::new(
                "0.0.0.0:0".parse().unwrap(),
                runtime(&fixture.root),
                ServiceTokenStore::load_or_create(&fixture.root).unwrap(),
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn binding_an_ephemeral_port_stays_on_loopback() {
        let fixture = Fixture::new();
        let bound = fixture.service.bind().await.unwrap();

        assert!(bound.local_addr().ip().is_loopback());
        assert_ne!(bound.local_addr().port(), 0);
    }

    async fn post(fixture: &Fixture, token: Option<&str>, body: Value) -> axum::response::Response {
        post_bytes(fixture, token, serde_json::to_vec(&body).unwrap()).await
    }

    async fn post_bytes(
        fixture: &Fixture,
        token: Option<&str>,
        body: Vec<u8>,
    ) -> axum::response::Response {
        let mut request = Request::builder()
            .method("POST")
            .uri("/v1/rpc")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(token) = token {
            request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        fixture
            .service
            .router()
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap()
    }

    fn health_request() -> Value {
        json!({"jsonrpc": "2.0", "id": 1, "method": "runtime.health"})
    }

    struct Fixture {
        _root: FixtureRoot,
        service: LocalService,
        token: String,
    }

    impl Fixture {
        fn new() -> Self {
            let root = FixtureRoot::new();
            let token_store = ServiceTokenStore::load_or_create(&root).unwrap();
            let token = std::fs::read_to_string(token_store.path()).unwrap();
            let service = LocalService::new(
                LocalServiceConfig::new(
                    "127.0.0.1:0".parse().unwrap(),
                    runtime(&root.root),
                    token_store,
                )
                .unwrap(),
            );
            Self {
                _root: root,
                service,
                token,
            }
        }
    }

    struct FixtureRoot {
        root: PathBuf,
    }

    impl FixtureRoot {
        fn new() -> Self {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let sequence = NEXT_FIXTURE_ROOT.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "pandora-local-service-{}-{timestamp}-{sequence}",
                std::process::id(),
            ));
            std::fs::create_dir(&root).unwrap();
            Self { root }
        }
    }

    impl AsRef<std::path::Path> for FixtureRoot {
        fn as_ref(&self) -> &std::path::Path {
            &self.root
        }
    }

    impl Drop for FixtureRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn runtime(root: &std::path::Path) -> RuntimeService {
        let policy = PolicyContext::new(
            1,
            [
                Capability::FilesystemRead,
                Capability::FilesystemWrite,
                Capability::ProcessExecute,
                Capability::ProviderInvoke,
            ],
            [Operation::Write, Operation::Execute],
        );
        RuntimeService::new_with_providers(
            ExecutionController::with_policy(WorkspaceRoot::new(root).unwrap(), policy),
            SessionStore::open(root.join("sessions.sqlite3")).unwrap(),
            ApprovalStore::open(root.join("sessions.sqlite3")).unwrap(),
            RuntimeServiceScope::new(
                PrincipalId::new("principal-a").unwrap(),
                TenantId::new("tenant-a").unwrap(),
                WorkspaceId::new("workspace-a").unwrap(),
            ),
            vec![ServiceProviderSummary::new(
                "fixture-provider",
                "fixture-model",
                "open_ai_compatible",
                true,
                false,
                None,
            )],
        )
        .with_evolution(Arc::new(
            EvolutionEngine::open(
                root.join("evolution.sqlite3"),
                EvolutionPolicy::production(1),
            )
            .unwrap(),
        ))
        .with_artifact_catalog(Arc::new(
            ArtifactCatalog::open(root.join("artifact-catalog.sqlite3")).unwrap(),
        ))
    }
}
