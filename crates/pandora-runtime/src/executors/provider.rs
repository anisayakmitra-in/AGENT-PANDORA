use crate::ConsumedPermit;
use pandora_provider::{ModelRequest, ModelResponse, Provider, ProviderError};
use pandora_types::{
    Capability, EffectOutcome, EffectReceipt, EffectTarget, Operation, ReceiptId, ResourceScope,
    Timestamp,
};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_RECEIPT_ID: AtomicU64 = AtomicU64::new(1);

pub struct ProviderResult {
    result: Result<ModelResponse, ProviderError>,
    receipt: EffectReceipt,
}

impl ProviderResult {
    pub fn result(&self) -> Result<&ModelResponse, &ProviderError> {
        self.result.as_ref()
    }

    pub fn into_result(self) -> Result<ModelResponse, ProviderError> {
        self.result
    }

    pub fn receipt(&self) -> &EffectReceipt {
        &self.receipt
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
        ProviderResult {
            result,
            receipt: receipt_for(permit, now, outcome),
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
    operation.capability() == Capability::ProviderInvoke
        && operation.operation() == Operation::Invoke
        && matches!(operation.resource_scope(), ResourceScope::None)
        && matches!(
            operation.target(),
            EffectTarget::Provider { provider: provider_id, .. }
                if provider_id == provider.manifest().id().as_str()
        )
        && request.provider_id() == provider.manifest().id()
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
    let receipt_id = ReceiptId::new(format!(
        "receipt-provider-{}",
        NEXT_RECEIPT_ID.fetch_add(1, Ordering::Relaxed)
    ))
    .expect("generated receipt ID is valid");
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
            Self {
                manifest: ProviderManifest::new(
                    "provider-a",
                    "Provider A",
                    "https://provider.example.test/v1",
                    "model-a",
                    "PANDORA_PROVIDER_KEY",
                )
                .unwrap(),
            }
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
                pandora_provider::TokenUsage::new(1, 1),
            ))
        }
    }

    fn request(provider: &ProviderManifest) -> OperationRequest {
        OperationRequest::new(
            ExecutionId::new("execution-1").unwrap(),
            SessionId::new("session-1").unwrap(),
            PrincipalId::new("principal-1").unwrap(),
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

        assert_eq!(result.result().unwrap().text(), "ready");
        assert!(matches!(
            result.receipt().outcome(),
            EffectOutcome::Succeeded
        ));
    }

    #[test]
    fn provider_executor_rejects_a_different_provider() {
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
}
