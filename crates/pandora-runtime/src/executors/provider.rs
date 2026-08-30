use crate::{ConsumedPermit, receipt_id::allocate_effect_receipt_id};
use pandora_provider::{ModelRequest, ModelResponse, Provider, ProviderError};
use pandora_types::{
    Capability, EffectOutcome, EffectReceipt, EffectTarget, Operation, ResourceScope,
    SecretReference, Timestamp,
};
use std::time::Instant;

pub struct ProviderResult {
    result: Result<ModelResponse, ProviderError>,
    receipts: Vec<EffectReceipt>,
    metrics: Vec<ProviderCallMetrics>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderCallMetrics {
    provider_id: String,
    model_id: String,
    elapsed_ms: u64,
    input_tokens: u32,
    output_tokens: u32,
    cached_input_tokens: u32,
    cache_write_input_tokens: u32,
    succeeded: bool,
}

impl ProviderCallMetrics {
    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    pub const fn elapsed_ms(&self) -> u64 {
        self.elapsed_ms
    }

    pub const fn input_tokens(&self) -> u32 {
        self.input_tokens
    }

    pub const fn output_tokens(&self) -> u32 {
        self.output_tokens
    }

    pub const fn cached_input_tokens(&self) -> u32 {
        self.cached_input_tokens
    }

    pub const fn cache_write_input_tokens(&self) -> u32 {
        self.cache_write_input_tokens
    }

    pub const fn succeeded(&self) -> bool {
        self.succeeded
    }
}

impl ProviderResult {
    pub fn result(&self) -> Result<&ModelResponse, &ProviderError> {
        self.result.as_ref()
    }

    pub fn into_result(self) -> Result<ModelResponse, ProviderError> {
        self.result
    }

    pub fn receipt(&self) -> &EffectReceipt {
        self.receipts
            .last()
            .expect("provider results always contain a receipt")
    }

    pub fn receipts(&self) -> &[EffectReceipt] {
        &self.receipts
    }

    pub fn metrics(&self) -> &ProviderCallMetrics {
        self.metrics
            .last()
            .expect("provider results always contain metrics")
    }

    pub fn metrics_history(&self) -> &[ProviderCallMetrics] {
        &self.metrics
    }

    pub(crate) fn prepend_receipt(&mut self, receipt: EffectReceipt) {
        self.receipts.insert(0, receipt);
    }

    pub(crate) fn prepend_metrics(&mut self, metrics: ProviderCallMetrics) {
        self.metrics.insert(0, metrics);
    }
}

pub struct ProviderExecutor;

impl ProviderExecutor {
    pub const fn new() -> Self {
        Self
    }

    pub fn complete(
        &self,
        permit: &ConsumedPermit,
        provider: &dyn Provider,
        request: ModelRequest,
        now: Timestamp,
    ) -> ProviderResult {
        let provider_id = provider.manifest().id().as_str().to_owned();
        let model_id = request.model_id().as_str().to_owned();
        let started = Instant::now();
        let result = if request_matches(permit, provider, &request) {
            provider.complete(request)
        } else {
            Err(ProviderError::InvalidRequest(
                "provider request does not match the authorization permit".to_owned(),
            ))
        };
        let outcome = match &result {
            Ok(_) => EffectOutcome::Succeeded,
            Err(error) => EffectOutcome::Failed {
                code: error_code(error).to_owned(),
            },
        };
        let (input_tokens, output_tokens, cached_input_tokens, cache_write_input_tokens) = result
            .as_ref()
            .map(|response| {
                (
                    response.usage().prompt_tokens(),
                    response.usage().completion_tokens(),
                    response.usage().cached_prompt_tokens(),
                    response.usage().cache_write_prompt_tokens(),
                )
            })
            .unwrap_or((0, 0, 0, 0));
        let succeeded = result.is_ok();
        ProviderResult {
            result,
            receipts: vec![receipt_for(permit, now, outcome)],
            metrics: vec![ProviderCallMetrics {
                provider_id,
                model_id,
                elapsed_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
                input_tokens,
                output_tokens,
                cached_input_tokens,
                cache_write_input_tokens,
                succeeded,
            }],
        }
    }
}

impl Default for ProviderExecutor {
    fn default() -> Self {
        Self::new()
    }
}

fn request_matches(
    permit: &ConsumedPermit,
    provider: &dyn Provider,
    request: &ModelRequest,
) -> bool {
    let operation = permit.request();
    let expected_credential = SecretReference::new(provider.manifest().api_key_env())
        .expect("provider manifests validate credential references");
    let payload_matches = request
        .authorization_payload_for(provider.manifest())
        .ok()
        .is_some_and(|payload| operation.payload_digest_matches(&payload));
    operation.capability() == Capability::ProviderInvoke
        && operation.operation() == Operation::Invoke
        && matches!(operation.resource_scope(), ResourceScope::None)
        && matches!(
            operation.target(),
            EffectTarget::Provider {
                provider: provider_id,
                credential,
            } if provider_id == provider.manifest().id().as_str()
                && credential == &expected_credential
        )
        && request.provider_id() == provider.manifest().id()
        && payload_matches
}

fn error_code(error: &ProviderError) -> &'static str {
    match error {
        ProviderError::InvalidManifest(_) => "invalid_manifest",
        ProviderError::CredentialUnavailable => "credential_unavailable",
        ProviderError::InvalidRequest(_) => "invalid_request",
        ProviderError::UnsupportedEndpoint => "unsupported_endpoint",
        ProviderError::Transport => "transport",
        ProviderError::HttpStatus { .. } => "http_status",
        ProviderError::ResponseTooLarge => "response_too_large",
        ProviderError::InvalidResponse => "invalid_response",
        ProviderError::InvalidToolArguments { .. } => "invalid_tool_arguments",
        ProviderError::DuplicateToolCallId { .. } => "duplicate_tool_call_id",
    }
}

fn receipt_for(permit: &ConsumedPermit, now: Timestamp, outcome: EffectOutcome) -> EffectReceipt {
    let receipt_id = allocate_effect_receipt_id("provider");
    EffectReceipt::new(
        receipt_id,
        permit.permit().permit_id().clone(),
        permit.permit().request_digest().clone(),
        now,
        outcome,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Parliament, ReferenceMonitor};
    use pandora_provider::{ChatMessage, ProviderManifest};
    use pandora_types::{
        ExecutionId, GeneId, OperationRequest, PolicyContext, PrincipalId, SecretReference,
        SessionId,
    };

    struct StubProvider {
        manifest: ProviderManifest,
    }

    impl StubProvider {
        fn new() -> Self {
            Self::with_manifest(
                ProviderManifest::new(
                    "provider-a",
                    "Provider A",
                    "https://provider.example.test/v1",
                    "model-a",
                    "PANDORA_PROVIDER_KEY",
                )
                .unwrap(),
            )
        }

        fn with_manifest(manifest: ProviderManifest) -> Self {
            Self { manifest }
        }
    }

    impl Provider for StubProvider {
        fn manifest(&self) -> &ProviderManifest {
            &self.manifest
        }

        fn complete(&self, _request: ModelRequest) -> Result<ModelResponse, ProviderError> {
            Ok(ModelResponse::new(
                "ready",
                Vec::new(),
                pandora_provider::TokenUsage::new(3, 1).with_prompt_cache(2, 1),
            ))
        }
    }

    fn request(provider: &ProviderManifest) -> OperationRequest {
        OperationRequest::new(
            ExecutionId::new("execution-1").unwrap(),
            SessionId::new("session-1").unwrap(),
            PrincipalId::new("principal-1").unwrap(),
            crate::test_support::execution_profile("provider"),
            GeneId::new("provider.invoke").unwrap(),
            None,
            Capability::ProviderInvoke,
            Operation::Invoke,
            EffectTarget::provider(
                provider.id().as_str(),
                SecretReference::new(provider.api_key_env()).unwrap(),
            ),
            ResourceScope::none(),
        )
        .unwrap()
    }

    #[test]
    fn provider_executor_requires_a_matching_permit() {
        let provider = StubProvider::new();
        let model_request = ModelRequest::new(
            provider.manifest().id().clone(),
            provider.manifest().default_model().clone(),
            vec![ChatMessage::user("hello").unwrap()],
        )
        .unwrap();
        let request = request(provider.manifest())
            .with_payload_digest(
                &model_request
                    .authorization_payload_for(provider.manifest())
                    .unwrap(),
            )
            .unwrap();
        let monitor = ReferenceMonitor::new(1, 60);
        let parliament = Parliament::new(1);
        let policy = PolicyContext::new(1, [Capability::ProviderInvoke], []);
        let permit = monitor
            .authorize(
                request.clone(),
                parliament.decide(&request, &policy),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();
        let consumed = monitor
            .store()
            .consume(permit, &request, Timestamp::from_unix_seconds(10))
            .unwrap();
        let result = ProviderExecutor::new().complete(
            &consumed,
            &provider,
            model_request,
            Timestamp::from_unix_seconds(10),
        );

        assert_eq!(result.result().unwrap().text(), "ready");
        assert_eq!(result.metrics().provider_id(), "provider-a");
        assert_eq!(result.metrics().model_id(), "model-a");
        assert_eq!(result.metrics().input_tokens(), 3);
        assert_eq!(result.metrics().output_tokens(), 1);
        assert_eq!(result.metrics().cached_input_tokens(), 2);
        assert_eq!(result.metrics().cache_write_input_tokens(), 1);
        assert!(result.metrics().succeeded());
        assert!(matches!(
            result.receipt().outcome(),
            EffectOutcome::Succeeded
        ));
    }

    #[test]
    fn provider_executor_rejects_a_permit_for_a_different_endpoint() {
        let provider = StubProvider::new();
        let model_request = ModelRequest::new(
            provider.manifest().id().clone(),
            provider.manifest().default_model().clone(),
            vec![ChatMessage::user("hello").unwrap()],
        )
        .unwrap();
        let request = request(provider.manifest())
            .with_payload_digest(
                &model_request
                    .authorization_payload_for(provider.manifest())
                    .unwrap(),
            )
            .unwrap();
        let monitor = ReferenceMonitor::new(1, 60);
        let parliament = Parliament::new(1);
        let policy = PolicyContext::new(1, [Capability::ProviderInvoke], []);
        let permit = monitor
            .authorize(
                request.clone(),
                parliament.decide(&request, &policy),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();
        let consumed = monitor
            .store()
            .consume(permit, &request, Timestamp::from_unix_seconds(10))
            .unwrap();
        let substituted_provider = StubProvider::with_manifest(
            ProviderManifest::new(
                "provider-a",
                "Provider A",
                "https://substituted.example.test/v1",
                "model-a",
                "PANDORA_PROVIDER_KEY",
            )
            .unwrap(),
        );

        let result = ProviderExecutor::new().complete(
            &consumed,
            &substituted_provider,
            model_request,
            Timestamp::from_unix_seconds(10),
        );

        assert!(matches!(
            result.result(),
            Err(ProviderError::InvalidRequest(_))
        ));
        assert_eq!(result.metrics().input_tokens(), 0);
        assert_eq!(result.metrics().output_tokens(), 0);
        assert!(!result.metrics().succeeded());
    }

    #[test]
    fn provider_executor_rejects_a_permit_for_a_different_credential_reference() {
        let provider = StubProvider::new();
        let model_request = ModelRequest::new(
            provider.manifest().id().clone(),
            provider.manifest().default_model().clone(),
            vec![ChatMessage::user("hello").unwrap()],
        )
        .unwrap();
        let request = request(provider.manifest())
            .with_payload_digest(
                &model_request
                    .authorization_payload_for(provider.manifest())
                    .unwrap(),
            )
            .unwrap();
        let monitor = ReferenceMonitor::new(1, 60);
        let parliament = Parliament::new(1);
        let policy = PolicyContext::new(1, [Capability::ProviderInvoke], []);
        let permit = monitor
            .authorize(
                request.clone(),
                parliament.decide(&request, &policy),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();
        let consumed = monitor
            .store()
            .consume(permit, &request, Timestamp::from_unix_seconds(10))
            .unwrap();
        let substituted_provider = StubProvider::with_manifest(
            ProviderManifest::new(
                "provider-a",
                "Provider A",
                "https://provider.example.test/v1",
                "model-a",
                "PANDORA_OTHER_PROVIDER_KEY",
            )
            .unwrap(),
        );

        let result = ProviderExecutor::new().complete(
            &consumed,
            &substituted_provider,
            model_request,
            Timestamp::from_unix_seconds(10),
        );

        assert!(matches!(
            result.result(),
            Err(ProviderError::InvalidRequest(_))
        ));
    }

    #[test]
    fn provider_executor_rejects_a_different_provider() {
        let provider = StubProvider::new();
        let authorized_model_request = ModelRequest::new(
            provider.manifest().id().clone(),
            provider.manifest().default_model().clone(),
            vec![ChatMessage::user("hello").unwrap()],
        )
        .unwrap();
        let request = request(provider.manifest())
            .with_payload_digest(
                &authorized_model_request
                    .authorization_payload_for(provider.manifest())
                    .unwrap(),
            )
            .unwrap();
        let monitor = ReferenceMonitor::new(1, 60);
        let parliament = Parliament::new(1);
        let policy = PolicyContext::new(1, [Capability::ProviderInvoke], []);
        let permit = monitor
            .authorize(
                request.clone(),
                parliament.decide(&request, &policy),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();
        let consumed = monitor
            .store()
            .consume(permit, &request, Timestamp::from_unix_seconds(10))
            .unwrap();
        let other = ProviderManifest::new(
            "provider-b",
            "Provider B",
            "https://provider.example.test/v1",
            "model-b",
            "PANDORA_PROVIDER_KEY_B",
        )
        .unwrap();
        let model_request = ModelRequest::new(
            other.id().clone(),
            other.default_model().clone(),
            vec![ChatMessage::user("hello").unwrap()],
        )
        .unwrap();

        let result = ProviderExecutor::new().complete(
            &consumed,
            &provider,
            model_request,
            Timestamp::from_unix_seconds(10),
        );

        assert!(matches!(
            result.result(),
            Err(ProviderError::InvalidRequest(_))
        ));
        assert!(matches!(
            result.receipt().outcome(),
            EffectOutcome::Failed { .. }
        ));
    }

    #[test]
    fn provider_executor_rejects_a_permit_without_payload_binding() {
        let provider = StubProvider::new();
        let request = request(provider.manifest());
        let monitor = ReferenceMonitor::new(1, 60);
        let parliament = Parliament::new(1);
        let policy = PolicyContext::new(1, [Capability::ProviderInvoke], []);
        let permit = monitor
            .authorize(
                request.clone(),
                parliament.decide(&request, &policy),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();
        let consumed = monitor
            .store()
            .consume(permit, &request, Timestamp::from_unix_seconds(10))
            .unwrap();
        let model_request = ModelRequest::new(
            provider.manifest().id().clone(),
            provider.manifest().default_model().clone(),
            vec![ChatMessage::user("hello").unwrap()],
        )
        .unwrap();

        let result = ProviderExecutor::new().complete(
            &consumed,
            &provider,
            model_request,
            Timestamp::from_unix_seconds(10),
        );

        assert!(matches!(
            result.result(),
            Err(ProviderError::InvalidRequest(_))
        ));
    }

    #[test]
    fn provider_executor_rejects_changed_model_payload() {
        let provider = StubProvider::new();
        let authorized_model_request = ModelRequest::new(
            provider.manifest().id().clone(),
            provider.manifest().default_model().clone(),
            vec![ChatMessage::user("authorized").unwrap()],
        )
        .unwrap();
        let request = request(provider.manifest())
            .with_payload_digest(
                &authorized_model_request
                    .authorization_payload_for(provider.manifest())
                    .unwrap(),
            )
            .unwrap();
        let monitor = ReferenceMonitor::new(1, 60);
        let parliament = Parliament::new(1);
        let policy = PolicyContext::new(1, [Capability::ProviderInvoke], []);
        let permit = monitor
            .authorize(
                request.clone(),
                parliament.decide(&request, &policy),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();
        let consumed = monitor
            .store()
            .consume(permit, &request, Timestamp::from_unix_seconds(10))
            .unwrap();
        let changed_model_request = ModelRequest::new(
            provider.manifest().id().clone(),
            provider.manifest().default_model().clone(),
            vec![ChatMessage::user("changed").unwrap()],
        )
        .unwrap();

        let result = ProviderExecutor::new().complete(
            &consumed,
            &provider,
            changed_model_request,
            Timestamp::from_unix_seconds(10),
        );

        assert!(matches!(
            result.result(),
            Err(ProviderError::InvalidRequest(_))
        ));
    }
}
