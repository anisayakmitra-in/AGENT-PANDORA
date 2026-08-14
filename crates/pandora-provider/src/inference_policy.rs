use crate::manifest::{ManifestError, ModelId, ProviderId};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheClass {
    Public,
    Internal,
    Sensitive,
    Secret,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KvCacheOwnership {
    Unknown,
    External,
    Runtime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackendCapabilities {
    kv_cache_ownership: KvCacheOwnership,
    continuous_batching: bool,
    paged_attention: bool,
}

impl BackendCapabilities {
    pub const fn unknown() -> Self {
        Self {
            kv_cache_ownership: KvCacheOwnership::Unknown,
            continuous_batching: false,
            paged_attention: false,
        }
    }

    pub const fn hosted() -> Self {
        Self {
            kv_cache_ownership: KvCacheOwnership::External,
            continuous_batching: false,
            paged_attention: false,
        }
    }

    pub const fn local_managed() -> Self {
        Self {
            kv_cache_ownership: KvCacheOwnership::Runtime,
            continuous_batching: true,
            paged_attention: true,
        }
    }

    pub const fn kv_cache_ownership(self) -> KvCacheOwnership {
        self.kv_cache_ownership
    }

    pub const fn continuous_batching_available(self) -> bool {
        self.continuous_batching
    }

    pub const fn paged_attention_available(self) -> bool {
        self.paged_attention
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InferencePolicyError {
    InvalidManifest(ManifestError),
    InvalidMetric(&'static str),
    InvalidLimit,
    CostCeiling,
    InvalidErrorCode,
}

impl fmt::Display for InferencePolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidManifest(error) => error.fmt(formatter),
            Self::InvalidMetric(field) => write!(formatter, "invalid inference metric: {field}"),
            Self::InvalidLimit => formatter.write_str("inference cost ceiling must be positive"),
            Self::CostCeiling => formatter.write_str("inference observation exceeds cost ceiling"),
            Self::InvalidErrorCode => formatter.write_str("inference error code is invalid"),
        }
    }
}

impl std::error::Error for InferencePolicyError {}

impl From<ManifestError> for InferencePolicyError {
    fn from(error: ManifestError) -> Self {
        Self::InvalidManifest(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InferenceObservation {
    provider_id: ProviderId,
    model_id: ModelId,
    fallback_provider: Option<String>,
    cache_class: CacheClass,
    prompt_cache_requested: bool,
    semantic_cache_requested: bool,
    prefill_latency_ms: u64,
    decode_latency_ms: u64,
    input_tokens: u64,
    output_tokens: u64,
    cost_micros: u64,
    quality_bps: u16,
    error_code: Option<String>,
    backend: BackendCapabilities,
}

impl InferenceObservation {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        provider_id: impl Into<String>,
        model_id: impl Into<String>,
        fallback_provider: Option<String>,
        cache_class: CacheClass,
        prompt_cache_requested: bool,
        semantic_cache_requested: bool,
        prefill_latency_ms: u64,
        decode_latency_ms: u64,
        input_tokens: u64,
        output_tokens: u64,
        cost_micros: u64,
        quality_bps: u16,
    ) -> Result<Self, InferencePolicyError> {
        if quality_bps > 10_000 {
            return Err(InferencePolicyError::InvalidMetric("quality"));
        }
        let fallback_provider = fallback_provider
            .map(|value| validate_label("fallback provider", value))
            .transpose()?;
        Ok(Self {
            provider_id: ProviderId::new(provider_id)?,
            model_id: ModelId::new(model_id)?,
            fallback_provider,
            cache_class,
            prompt_cache_requested,
            semantic_cache_requested,
            prefill_latency_ms,
            decode_latency_ms,
            input_tokens,
            output_tokens,
            cost_micros,
            quality_bps,
            error_code: None,
            backend: BackendCapabilities::unknown(),
        })
    }

    pub fn with_fallback(mut self, provider: impl Into<String>) -> Self {
        self.fallback_provider = Some(provider.into());
        self
    }

    pub fn with_backend(mut self, backend: BackendCapabilities) -> Self {
        self.backend = backend;
        self
    }

    pub fn with_error_code(
        mut self,
        code: impl Into<String>,
    ) -> Result<Self, InferencePolicyError> {
        let code = validate_label("error code", code.into())?;
        self.error_code = Some(code);
        Ok(self)
    }

    pub fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }

    pub fn model_id(&self) -> &ModelId {
        &self.model_id
    }

    pub fn fallback_provider(&self) -> Option<&str> {
        self.fallback_provider.as_deref()
    }

    pub const fn cache_class(&self) -> CacheClass {
        self.cache_class
    }

    pub const fn prompt_cache_requested(&self) -> bool {
        self.prompt_cache_requested
    }

    pub const fn semantic_cache_requested(&self) -> bool {
        self.semantic_cache_requested
    }

    pub const fn prefill_latency_ms(&self) -> u64 {
        self.prefill_latency_ms
    }

    pub const fn decode_latency_ms(&self) -> u64 {
        self.decode_latency_ms
    }

    pub const fn input_tokens(&self) -> u64 {
        self.input_tokens
    }

    pub const fn output_tokens(&self) -> u64 {
        self.output_tokens
    }

    pub const fn cost_micros(&self) -> u64 {
        self.cost_micros
    }

    pub const fn quality_bps(&self) -> u16 {
        self.quality_bps
    }

    pub fn error_code(&self) -> Option<&str> {
        self.error_code.as_deref()
    }

    pub const fn backend(&self) -> BackendCapabilities {
        self.backend
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InferencePolicy {
    max_cost_micros: u64,
}

impl InferencePolicy {
    pub fn new(max_cost_micros: u64) -> Result<Self, InferencePolicyError> {
        if max_cost_micros == 0 {
            return Err(InferencePolicyError::InvalidLimit);
        }
        Ok(Self { max_cost_micros })
    }

    pub const fn max_cost_micros(&self) -> u64 {
        self.max_cost_micros
    }

    pub fn record(
        &self,
        observation: InferenceObservation,
    ) -> Result<InferenceRecord, InferencePolicyError> {
        if observation.cost_micros() > self.max_cost_micros {
            return Err(InferencePolicyError::CostCeiling);
        }
        let prompt_cache_eligible = observation.prompt_cache_requested()
            && matches!(
                observation.cache_class(),
                CacheClass::Public | CacheClass::Internal
            );
        let semantic_cache_eligible = observation.semantic_cache_requested()
            && observation.cache_class() == CacheClass::Public;
        Ok(InferenceRecord {
            observation,
            prompt_cache_eligible,
            semantic_cache_eligible,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InferenceRecord {
    observation: InferenceObservation,
    prompt_cache_eligible: bool,
    semantic_cache_eligible: bool,
}

impl InferenceRecord {
    pub fn provider_id(&self) -> &ProviderId {
        self.observation.provider_id()
    }

    pub fn model_id(&self) -> &ModelId {
        self.observation.model_id()
    }

    pub fn fallback_provider(&self) -> Option<&str> {
        self.observation.fallback_provider()
    }

    pub const fn prompt_cache_eligible(&self) -> bool {
        self.prompt_cache_eligible
    }

    pub const fn semantic_cache_eligible(&self) -> bool {
        self.semantic_cache_eligible
    }

    pub const fn prefill_latency_ms(&self) -> u64 {
        self.observation.prefill_latency_ms()
    }

    pub const fn decode_latency_ms(&self) -> u64 {
        self.observation.decode_latency_ms()
    }

    pub const fn input_tokens(&self) -> u64 {
        self.observation.input_tokens()
    }

    pub const fn output_tokens(&self) -> u64 {
        self.observation.output_tokens()
    }

    pub const fn cost_micros(&self) -> u64 {
        self.observation.cost_micros()
    }

    pub const fn quality_bps(&self) -> u16 {
        self.observation.quality_bps()
    }

    pub fn error_code(&self) -> Option<&str> {
        self.observation.error_code()
    }

    pub const fn backend(&self) -> BackendCapabilities {
        self.observation.backend()
    }
}

fn validate_label(field: &'static str, value: String) -> Result<String, InferencePolicyError> {
    let value = value.trim().to_owned();
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        return Err(InferencePolicyError::InvalidMetric(field));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(class: CacheClass) -> InferenceObservation {
        InferenceObservation::new(
            "openai", "model-a", None, class, true, true, 12, 30, 100, 50, 50, 9_000,
        )
        .unwrap()
    }

    #[test]
    fn sensitive_content_disables_prompt_and_semantic_caches() {
        let record = InferencePolicy::new(1_000)
            .unwrap()
            .record(observation(CacheClass::Sensitive))
            .unwrap();

        assert!(!record.prompt_cache_eligible());
        assert!(!record.semantic_cache_eligible());
    }

    #[test]
    fn public_content_can_use_requested_caches() {
        let record = InferencePolicy::new(1_000)
            .unwrap()
            .record(observation(CacheClass::Public))
            .unwrap();

        assert!(record.prompt_cache_eligible());
        assert!(record.semantic_cache_eligible());
        assert_eq!(record.prefill_latency_ms(), 12);
        assert_eq!(record.decode_latency_ms(), 30);
        assert_eq!(record.cost_micros(), 50);
    }

    #[test]
    fn fallback_and_backend_capabilities_are_observations_not_controls() {
        let observation = observation(CacheClass::Internal)
            .with_fallback("backup")
            .with_backend(BackendCapabilities::local_managed());
        let record = InferencePolicy::new(1_000)
            .unwrap()
            .record(observation)
            .unwrap();

        assert_eq!(record.fallback_provider(), Some("backup"));
        assert_eq!(
            record.backend().kv_cache_ownership(),
            KvCacheOwnership::Runtime
        );
        assert!(record.backend().continuous_batching_available());
    }

    #[test]
    fn cost_ceiling_rejects_an_observation() {
        assert_eq!(
            InferencePolicy::new(10)
                .unwrap()
                .record(observation(CacheClass::Public)),
            Err(InferencePolicyError::CostCeiling)
        );
    }
}
