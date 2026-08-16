#![forbid(unsafe_code)]

pub mod adaptation;
pub mod capability;
pub mod context;
pub mod effect;
pub mod efficiency;
pub mod evaluation;
pub mod events;
pub mod evolution;
pub mod gene;
pub mod governance;
pub mod harness;
pub mod ids;
pub mod memory;
pub mod observability;
pub mod orchestration;
pub mod package;
pub mod session;
pub mod skill;

pub use adaptation::{
    AdaptationCandidate, AdaptationContractError, AdaptationDecision, AdaptationPolicy,
    AdaptationReceipt, AdaptationRequest, AdaptationTarget,
};
pub use capability::{Capability, Operation};
pub use context::{
    ContextAssembly, ContextCacheKey, ContextClassification, ContextContractError, ContextEntry,
    ContextFragment, ContextReceipt, ContextRequest, ContextSource, ContextTrust,
};
pub use effect::{
    EffectOutcome, EffectPermit, EffectReceipt, EffectTarget, OperationRequest, RequestError,
    ResourceScope, SecretReference, Timestamp,
};
pub use efficiency::{
    EfficiencyContractError, EfficiencyObjective, EfficiencySample, EfficiencySummary,
};
pub use evaluation::{
    EvaluationContractError, EvaluationKind, EvaluationRequest, EvaluationResult, EvaluationStatus,
};
pub use events::{EventContext, EventPayload, EventType, RuntimeEvent};
pub use evolution::{
    ArtifactSignature, CanaryResult, EvolutionContractError, EvolutionMode, EvolutionPolicy,
    EvolutionSource, EvolutionState, HoldoutEvaluation, MutationProposal, ParliamentApproval,
    ReflexionArtifact, ReplacementReceipt, RollbackReceipt,
};
pub use gene::{Gene, GeneError, GeneInput, GeneKind, GeneManifest};
pub use governance::{ParliamentDecision, PolicyContext};
pub use harness::{Harness, HarnessKind, HarnessManifest, MetaComposition};
pub use ids::{
    ArtifactId, EventId, ExecutionId, GeneId, HarnessId, IdError, MemoryId, PackageId, PermitId,
    PlanId, PrincipalId, ProposalId, ReceiptId, RequestDigest, RoleId, RunLoopId, SessionId,
    TenantId, WorkspaceId,
};
pub use memory::{
    MemoryApproval, MemoryAuditAction, MemoryAuditEntry, MemoryContractError, MemoryKind,
    MemoryRecord, MemoryScope, MemoryTier,
};
pub use observability::{
    ObservabilityContractError, ObservabilitySample, ObservabilitySnapshot, SpanView, TraceView,
};
pub use orchestration::{
    DomainAgentProfile, DomainProfileMode, Handoff, IterationOutcome, LoopDecision,
    LoopTermination, OrchestrationContractError, OrchestrationPlan, OrchestrationRole,
    RoleAssignment, RunLoopConfig, RunLoopSnapshot, RunLoopState, Usage,
};
pub use package::{
    PackageCompatibility, PackageDependency, PackageKind, PackageManifest, PackageManifestError,
    TrustEvidence, TrustLevel, hash_artifact,
};
pub use session::{Session, TaskIntent};
pub use skill::{SkillId, SkillManifest, SkillManifestError};
