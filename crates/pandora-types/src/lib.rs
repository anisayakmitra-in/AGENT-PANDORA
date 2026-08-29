#![forbid(unsafe_code)]

pub mod adaptation;
pub mod capability;
pub mod containment;
pub mod context;
pub mod effect;
pub mod efficiency;
pub mod evaluation;
pub mod events;
pub mod evolution;
pub mod execution_profile;
pub mod gene;
pub mod governance;
pub mod harness;
pub mod ids;
pub mod jobs;
pub mod memory;
pub mod observability;
pub mod orchestration;
pub mod package;
pub mod population;
pub mod rollout;
pub mod service;
pub mod session;
pub mod skill;
pub mod subagent;
pub mod workspace_orchestration;

pub use adaptation::{
    AdaptationCandidate, AdaptationContractError, AdaptationDecision, AdaptationPolicy,
    AdaptationReceipt, AdaptationRequest, AdaptationTarget,
};
pub use capability::{Capability, Operation};
pub use containment::{
    CONTAINMENT_EVIDENCE_VERSION, ContainmentBoundary, ContainmentBoundaryKind,
    ContainmentContractError, ContainmentControl, ContainmentEvidence, ContainmentLevel,
    ContainmentLimitation, ContainmentSnapshot, ExecutorIdentity, ExecutorWorkerClass,
};
pub use context::{
    CONTEXT_PROJECTION_VERSION, ContextAssembly, ContextCacheDisposition, ContextCacheKey,
    ContextClassification, ContextContractError, ContextEntry, ContextFragment,
    ContextFragmentManifest, ContextManifest, ContextOrigin, ContextOriginKind, ContextReceipt,
    ContextRequest, ContextSource, ContextTrust,
};
pub use effect::{
    EffectOutcome, EffectPermit, EffectReceipt, EffectTarget, OperationRequest, RequestError,
    ResourceScope, SecretReference, Timestamp,
};
pub use efficiency::{
    EfficiencyContractError, EfficiencyObjective, EfficiencySample, EfficiencySummary,
};
pub use evaluation::{
    EvaluationContractError, EvaluationKind, EvaluationReceipt, EvaluationRequest,
    EvaluationResult, EvaluationStatus,
};
pub use events::{EventContext, EventPayload, EventType, RuntimeEvent};
pub use evolution::{
    ArtifactSignature, CanaryResult, EvolutionContractError, EvolutionMode, EvolutionPolicy,
    EvolutionSource, EvolutionState, HoldoutEvaluation, MutationProposal, ParliamentApproval,
    ReflexionArtifact, ReplacementReceipt, ResearchArtifactKind, RollbackReceipt,
};
pub use execution_profile::{
    EXECUTION_PROFILE_VERSION, ExecutionProfile, ExecutionProfileBinding,
    ExecutionProfileBindingKind, ExecutionProfileContractError, ExecutionProfileDigest,
};
pub use gene::{Gene, GeneError, GeneInput, GeneKind, GeneManifest};
pub use governance::{ParliamentDecision, PolicyContext};
pub use harness::{Harness, HarnessKind, HarnessManifest, MetaComposition};
pub use ids::{
    ArtifactId, EventId, ExecutionId, FailureId, GeneId, HarnessId, IdError, JobId, JobWorkerId,
    MemoryId, OrchestrationRunId, PackageId, PermitId, PlanId, PopulationId, PrincipalId,
    ProposalId, ReceiptId, RepositoryId, RequestDigest, RoleId, RunLoopId, SessionId, SubagentId,
    TenantId, WorkspaceId,
};
pub use jobs::{JobCommand, JobContractError, JobRequest, JobStatus, MAX_JOB_ARGUMENT_BYTES};
pub use memory::{
    MAX_MEMORY_SYNTHESIS_EVIDENCE, MemoryApproval, MemoryAuditAction, MemoryAuditEntry,
    MemoryContractError, MemoryKind, MemoryOrigin, MemoryRecord, MemoryScope, MemoryTier,
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
    DomainRoutingProfile, MAX_DOMAIN_ROUTE_HINT_BYTES, MAX_DOMAIN_ROUTE_HINTS,
    PACKAGE_LOCK_FORMAT_VERSION, PackageCompatibility, PackageDependency, PackageKind, PackageLock,
    PackageLockError, PackageManifest, PackageManifestError, TrustEvidence, TrustLevel,
    hash_artifact,
};
pub use population::{
    CandidateDisposition, CandidateOutcome, CandidatePopulation, FailureCorpus, FailureEvidence,
    FailurePartition, GenerationReceipt, GenerationStats, LineageAttempt, LineageDirection,
    LineageLesson, LineageLimits, LineageMemory, LineageNode, LineageQuery, LineageView,
    MutationBatch, MutationLimits, MutationPrecheckReceipt, POPULATION_PROTOCOL_VERSION,
    PopulationCandidate, PopulationContractError, PopulationEvaluation, PopulationMutationRequest,
    PopulationPolicy, PopulationScope, PrecheckDisposition, PrecheckFailure,
};
pub use rollout::{
    MAX_ROLLOUT_RECORDS, ROLLOUT_PROJECTION_VERSION, Rollout, RolloutContractError, RolloutDigest,
    RolloutEffectOutcome, RolloutEventEvidence, RolloutEvidence, RolloutRecord, RolloutRecordKind,
    RolloutScope,
};
pub use service::{
    LOCAL_SERVICE_PROTOCOL_VERSION, MAX_SERVICE_EVENT_PAGE, MAX_SERVICE_SESSION_PAGE,
    ServiceAgentResumeRequest, ServiceAgentRunRequest, ServiceAgentRunResult,
    ServiceApprovalSummary, ServiceArtifactActivation, ServiceContextAttachment,
    ServiceContractError, ServiceEngineSummary, ServiceEventPage, ServiceEventPageRequest,
    ServiceEvolutionApproval, ServiceEvolutionCanary, ServiceEvolutionCandidate,
    ServiceEvolutionEvaluation, ServiceEvolutionPreview, ServiceEvolutionSummary,
    ServiceHarnessSummary, ServiceHealth, ServiceMemoryPage, ServiceMemoryRecord,
    ServiceOrchestrationRoleSummary, ServiceOrchestrationRunSummary, ServiceProviderSummary,
    ServiceRequest, ServiceResponse, ServiceRunRequest, ServiceRunResult, ServiceRunResumeRequest,
    ServiceSessionDetail, ServiceSessionSummary, ServiceToolSummary,
};
pub use session::{Session, TaskIntent};
pub use skill::{SkillId, SkillManifest, SkillManifestError};
pub use subagent::{
    MAX_SUBAGENT_DELEGATION_DEPTH, MAX_SUBAGENT_DURATION_SECONDS, MAX_SUBAGENT_RESULT_BYTES,
    MAX_SUBAGENT_TASK_BYTES, MAX_SUBAGENT_TOKENS, MAX_SUBAGENT_TOOL_CALLS, MAX_SUBAGENT_TURNS,
    SubagentBudgets, SubagentContractError, SubagentHarnessBinding, SubagentRequest,
    SubagentStatus, SubagentWorktreeState,
};
pub use workspace_orchestration::{
    GovernedOrchestrationPlan, OrchestrationRoleReceipt, RepositoryBinding, RoleRepositoryBinding,
    WorkspaceOrchestrationError,
};
