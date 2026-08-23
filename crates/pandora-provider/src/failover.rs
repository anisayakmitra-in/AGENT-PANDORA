use crate::{ModelRequest, ModelResponse, Provider, ProviderError, ProviderManifest};

pub struct FailoverProvider {
    primary: Box<dyn Provider>,
    fallback: Box<dyn Provider>,
}

impl FailoverProvider {
    pub fn new(primary: Box<dyn Provider>, fallback: Box<dyn Provider>) -> Self {
        Self { primary, fallback }
    }
}

impl Provider for FailoverProvider {
    fn manifest(&self) -> &ProviderManifest {
        self.primary.manifest()
    }

    fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        self.primary.complete(request)
    }

    fn fallback_provider(&self) -> Option<&dyn Provider> {
        Some(self.fallback.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChatMessage, TokenUsage};
    use std::sync::{Arc, Mutex};

    struct StubProvider {
        manifest: ProviderManifest,
        result: Mutex<Option<Result<ModelResponse, ProviderError>>>,
        requests: Arc<Mutex<Vec<ModelRequest>>>,
    }

    impl StubProvider {
        fn new(
            manifest: ProviderManifest,
            result: Result<ModelResponse, ProviderError>,
            requests: Arc<Mutex<Vec<ModelRequest>>>,
        ) -> Self {
            Self {
                manifest,
                result: Mutex::new(Some(result)),
                requests,
            }
        }
    }

    impl Provider for StubProvider {
        fn manifest(&self) -> &ProviderManifest {
            &self.manifest
        }

        fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
            self.requests.lock().unwrap().push(request);
            self.result
                .lock()
                .unwrap()
                .take()
                .expect("stub provider called more than once")
        }
    }

    fn manifest(id: &str, model: &str) -> ProviderManifest {
        ProviderManifest::new(
            id,
            id,
            "https://provider.example.test/v1",
            model,
            "PANDORA_PROVIDER_KEY",
        )
        .unwrap()
    }

    fn request() -> ModelRequest {
        ModelRequest::new(
            manifest("primary", "primary-model").id().clone(),
            manifest("primary", "primary-model").default_model().clone(),
            vec![ChatMessage::user("hello").unwrap()],
        )
        .unwrap()
        .with_max_output_tokens(128)
        .unwrap()
    }

    #[test]
    fn direct_completion_never_invokes_fallback_below_the_controller() {
        let primary_requests = Arc::new(Mutex::new(Vec::new()));
        let fallback_requests = Arc::new(Mutex::new(Vec::new()));
        let primary = StubProvider::new(
            manifest("primary", "primary-model"),
            Err(ProviderError::Transport),
            primary_requests,
        );
        let fallback = StubProvider::new(
            manifest("fallback", "fallback-model"),
            Ok(ModelResponse::new(
                "ready",
                Vec::new(),
                TokenUsage::new(2, 1),
            )),
            fallback_requests.clone(),
        );
        let provider = FailoverProvider::new(Box::new(primary), Box::new(fallback));
        let original = request();

        let error = provider.complete(original).unwrap_err();

        assert_eq!(error, ProviderError::Transport);
        assert!(fallback_requests.lock().unwrap().is_empty());
    }

    #[test]
    fn non_retryable_primary_failure_does_not_call_fallback() {
        let primary_requests = Arc::new(Mutex::new(Vec::new()));
        let fallback_requests = Arc::new(Mutex::new(Vec::new()));
        let error = ProviderError::InvalidRequest("bad request".to_owned());
        let primary = StubProvider::new(
            manifest("primary", "primary-model"),
            Err(error.clone()),
            primary_requests,
        );
        let fallback = StubProvider::new(
            manifest("fallback", "fallback-model"),
            Ok(ModelResponse::default()),
            fallback_requests.clone(),
        );
        let provider = FailoverProvider::new(Box::new(primary), Box::new(fallback));

        assert_eq!(provider.complete(request()).unwrap_err(), error);
        assert!(fallback_requests.lock().unwrap().is_empty());
    }

    #[test]
    fn only_recoverable_statuses_retry() {
        assert!(ProviderError::Transport.is_retryable());
        assert!(ProviderError::CredentialUnavailable.is_retryable());
        assert!(ProviderError::HttpStatus { status: 408 }.is_retryable());
        assert!(ProviderError::HttpStatus { status: 429 }.is_retryable());
        assert!(ProviderError::HttpStatus { status: 503 }.is_retryable());
        assert!(!ProviderError::HttpStatus { status: 400 }.is_retryable());
        assert!(!ProviderError::HttpStatus { status: 401 }.is_retryable());
        assert!(!ProviderError::InvalidResponse.is_retryable());
    }
}
