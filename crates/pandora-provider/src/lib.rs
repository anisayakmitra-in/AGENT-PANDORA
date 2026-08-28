#![forbid(unsafe_code)]

pub mod client;
pub mod failover;
pub mod inference_policy;
pub mod manifest;
pub mod structured_output;

pub use client::{
    ChatMessage, HttpProvider, MessageRole, ModelRequest, ModelResponse, PromptCacheTtl, Provider,
    ProviderError, TokenUsage, ToolCall, ToolSchema, TraceMetadata,
};
pub use failover::FailoverProvider;
pub use inference_policy::{
    BackendCapabilities, CacheClass, InferenceObservation, InferencePolicy, InferenceRecord,
    KvCacheOwnership,
};
pub use manifest::{ManifestError, ModelId, ProviderId, ProviderManifest, ProviderProtocol};
pub use structured_output::{
    FallbackPolicy, RepairError, Repairer, SchemaError, StructuredOutputError,
    StructuredOutputErrorKind, ValidatedOutput, ValidationOutcome, parse_and_validate,
};
