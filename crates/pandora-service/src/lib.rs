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
    ServiceEventPageRequest, ServiceRequest, ServiceResponse, ServiceRunRequest, Timestamp,
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
        "run.execute" => {
            let params: ServiceRunRequest = deserialize_params(params)?;
            ServiceRequest::run(params)
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
        ExecutionController, RuntimeService, RuntimeServiceScope, ServiceTokenStore,
    };
    use pandora_runtime::{executors::WorkspaceRoot, sessions::SessionStore};
    use pandora_types::{PrincipalId, TenantId, WorkspaceId};
    use serde_json::{Value, json};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tower::ServiceExt;

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
            let root = std::env::temp_dir().join(format!(
                "pandora-local-service-{}-{timestamp}",
                std::process::id()
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
        RuntimeService::new(
            ExecutionController::new(WorkspaceRoot::new(root).unwrap()),
            SessionStore::open(root.join("sessions.sqlite3")).unwrap(),
            RuntimeServiceScope::new(
                PrincipalId::new("principal-a").unwrap(),
                TenantId::new("tenant-a").unwrap(),
                WorkspaceId::new("workspace-a").unwrap(),
            ),
        )
    }
}
