#![forbid(unsafe_code)]

pub mod capability;
pub mod context;
pub mod effect;
pub mod evaluation;
pub mod events;
pub mod gene;
pub mod governance;
pub mod harness;
pub mod ids;
pub mod memory;
pub mod observability;
pub mod package;
pub mod session;
pub mod skill;

pub use capability::{Capability, Operation};
pub use context::{
    ContextAssembly, ContextCacheKey, ContextClassification, ContextContractError, ContextEntry,
    ContextFragment, ContextReceipt, ContextRequest, ContextSource, ContextTrust,
};
pub use effect::{
    EffectOutcome, EffectPermit, EffectReceipt, EffectTarget, OperationRequest, RequestError,
    ResourceScope, SecretReference, Timestamp,
};
pub use evaluation::{
    EvaluationContractError, EvaluationKind, EvaluationRequest, EvaluationResult, EvaluationStatus,
};
pub use events::{EventContext, EventPayload, EventType, RuntimeEvent};
pub use gene::{Gene, GeneError, GeneInput, GeneKind, GeneManifest};
pub use governance::{ParliamentDecision, PolicyContext};
pub use harness::{Harness, HarnessKind, HarnessManifest, SourceHarnessManifest};
pub use ids::{
    ArtifactId, EventId, ExecutionId, GeneId, HarnessId, IdError, MemoryId, PackageId, PermitId,
    PrincipalId, ReceiptId, RequestDigest, SessionId, TenantId, WorkspaceId,
};
pub use memory::{
    MemoryApproval, MemoryAuditAction, MemoryAuditEntry, MemoryContractError, MemoryKind,
    MemoryRecord, MemoryScope, MemoryTier,
};
pub use observability::{
    ObservabilityContractError, ObservabilitySample, ObservabilitySnapshot, SpanView, TraceView,
};
pub use package::{
    PackageCompatibility, PackageDependency, PackageKind, PackageManifest, PackageManifestError,
    TrustEvidence, TrustLevel, hash_artifact,
};
pub use session::{Session, TaskIntent};
pub use skill::{SkillId, SkillManifest, SkillManifestError};
