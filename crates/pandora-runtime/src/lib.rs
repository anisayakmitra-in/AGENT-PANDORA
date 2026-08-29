#![forbid(unsafe_code)]

pub mod adaptive_engine;
pub mod agent_loop;
pub mod approvals;
pub mod artifact_catalog;
pub mod coding_feedback;
pub mod composition_ledger;
pub mod config;
pub mod containment;
pub mod context_engine;
pub mod context_recovery;
pub mod device_trust;
pub mod efficiency_engine;
pub mod efficiency_store;
pub mod evaluation_engine;
pub mod evaluation_schedule;
pub mod evaluation_suite;
pub mod evolution;
pub mod execution_controller;
mod execution_profile;
pub mod executors;
pub mod fleet;
pub mod github_client;
pub mod graph_intelligence;
pub mod harness_registry;
pub mod hooks;
pub mod human_review;
pub mod identity;
pub mod job_store;
pub mod mcp;
pub mod mcp_catalog;
pub mod memory_engine;
pub mod mutation;
pub mod observability;
pub mod operations;
pub mod orchestration_engine;
pub mod orchestration_store;
pub mod package_admission;
pub mod package_store;
pub mod parliament;
pub mod permit_store;
pub mod recovery_archive;
pub mod reference_monitor;
pub mod registry_client;
pub mod replacement;
pub mod research_artifact;
pub mod rollout_reducer;
pub mod run_loop;
pub mod secret_vault;
pub mod self_healing;
pub mod service;
pub mod service_token;
pub mod sessions;
pub mod shadow_council;
pub mod skill_engine;
pub mod strategies;
pub mod subagent;
pub mod subagent_store;
pub mod tool_engine;
pub mod wasm;

#[cfg(test)]
mod test_support;

pub use adaptive_engine::{AdaptationResult, AdaptiveEngine, AdaptiveError};
pub use agent_loop::{
    AgentApprovalContext, AgentCheckpoint, AgentCheckpointKind, AgentControlStop, AgentLoop,
    AgentLoopError, AgentRunControl, AgentRunRequest, AgentRunSummary, MAX_AGENT_TOOL_CALLS,
    MAX_AGENT_TURNS, SubagentRunControl,
};
pub use approvals::{
    ApprovalError, ApprovalRequest, ApprovalStatus, ApprovalStore, PendingApproval,
};
pub use artifact_catalog::{ArtifactActivation, ArtifactCatalog, ArtifactCatalogError};
pub use coding_feedback::{
    CodingFeedbackError, CodingFeedbackInput, CodingFeedbackLoop, CodingFeedbackResult,
};
pub use composition_ledger::{
    COMPOSITION_LEDGER_VERSION, CompositionBinding, CompositionLedger, CompositionLedgerError,
    CompositionSource, MAX_COMPOSITION_BINDINGS,
};
pub use containment::shipped_executor_containment;
pub use context_engine::{ContextCacheStats, ContextEngine, ContextError};
pub use context_recovery::{ContextRecovery, RecoveryDecision, RecoveryInput, RecoveryStep};
pub use device_trust::{
    DeviceKeyError, DeviceKeyStore, DeviceProofRequest, device_proof_message, verify_device_proof,
};
pub use efficiency_engine::{DEFAULT_MAX_SAMPLES_PER_TARGET, EfficiencyEngine, EfficiencyError};
pub use efficiency_store::{EfficiencyStore, EfficiencyStoreError};
pub use evaluation_engine::{
    EvaluationEngine, EvaluationError, GoldenCase, GoldenCaseResult, GoldenSetReport, HoldoutCase,
    HoldoutCaseResult, HoldoutSetReport, MAX_GOLDEN_CASE_ID_BYTES, MAX_GOLDEN_CASES,
    MAX_GOLDEN_EXPECTED_OUTPUT_BYTES, MAX_HOLDOUT_CASE_ID_BYTES, MAX_HOLDOUT_CASES,
    MAX_HOLDOUT_OUTPUT_BYTES,
};
pub use evaluation_schedule::{
    EvaluationSchedule, EvaluationScheduleError, EvaluationScheduleRun,
    EvaluationScheduleRunStatus, EvaluationScheduleStore, MAX_CLAIM_BATCH,
    MAX_EVALUATION_SUITE_BYTES, MAX_SCHEDULES, SCHEDULE_LEASE_SECONDS,
};
pub use evaluation_suite::{
    EvaluationSuite, EvaluationSuiteError, EvaluationSuiteStore, MAX_EVALUATION_DEFINITION_BYTES,
    MAX_EVALUATION_SUITE_ID_BYTES, MAX_EVALUATION_SUITES,
};
pub use evolution::{EvolutionEngine, EvolutionError, EvolutionRecord};
pub use execution_controller::{
    ExecutionController, RunStatus, RunSummary, RuntimeError, WorktreeExecutionContext,
};
pub use execution_profile::ExecutionProfileAssemblyError;
pub use executors::{
    GitWorktreeExecutor, ProviderCallMetrics, WorktreeChange, WorktreeCommand, WorktreeError,
    WorktreeResult,
};
pub use fleet::{
    FLEET_SCHEMA_VERSION, FleetBudget, FleetEngine, FleetError, FleetLease, FleetLeaseState,
    FleetNode, FleetNodeState, FleetQuiescenceGuard, FleetSupervisor, FleetSupervisorState,
    MAX_FLEET_CAPABILITIES, MAX_FLEET_LEASES, MAX_FLEET_NODES,
};
pub use github_client::{GitHubPackageClient, GitHubPackageError};
pub use graph_intelligence::{
    GraphEdge, GraphError, GraphInput, GraphIntelligenceEngine, GraphKind, GraphNode,
    GraphNodeKind, GraphScope, GraphSnapshot, GraphStore, GraphStoreError, MAX_GRAPH_EDGES,
    MAX_GRAPH_INPUT_BYTES, MAX_GRAPH_INPUTS, MAX_GRAPH_NODES, MAX_GRAPH_SNAPSHOT_BYTES,
};
pub use harness_registry::{
    HarnessRegistry, HarnessRegistryError, PackageRecord, PackageState, PublisherTrustRoot,
    PublisherTrustRoots,
};
pub use hooks::{HookPoint, HookSelector, LifecycleHook, LifecycleHooks};
pub use human_review::{
    HumanMode, HumanReviewEngine, ReviewDecision, ReviewError, ReviewReceipt, ReviewRecord,
    ReviewState, ReviewSubject,
};
pub use identity::{
    AccessRole, IdentityEnrollment, IdentityEnrollmentRequest, IdentityStore, IdentityStoreError,
    ServiceIdentity,
};
pub use job_store::{JobRecord, JobStore, JobStoreError, MAX_JOB_RESULT_BYTES};
pub use mcp::{
    MCP_LEGACY_PROTOCOL_VERSION, MCP_PROTOCOL_VERSION, McpError, McpFailure, McpInvocation,
    McpProtocolMode, McpServer, McpStart, McpStdioConfig, McpToolResult, McpWireEra,
};
pub use mcp_catalog::{McpCatalogRevision, McpCatalogTool};
pub use memory_engine::{
    MAX_SYNTHESIS_SOURCE_RECORDS, MemoryEngine, MemoryError, MemorySynthesisProposal,
    MemorySynthesisSnapshot,
};
pub use mutation::{MutationEngine, MutationError};
pub use observability::{
    DEFAULT_MAX_OBSERVABILITY_SAMPLES, ObservabilityEngine, ObservabilityError,
};
pub use operations::{OperationalEvent, OperationalRecorder, OperationalStatus};
pub use orchestration_engine::{
    DomainProfileRun, OrchestrationEngine, OrchestrationError, OrchestrationRun,
    OrchestrationRunSnapshot,
};
pub use orchestration_store::{
    OrchestrationRunRecord, OrchestrationRunStatus, OrchestrationStore, OrchestrationStoreError,
};
pub use package_store::{
    MAX_PUBLISHER_TRUST_ROOTS, MAX_STORED_ARTIFACT_BYTES, PackageBinding, PackageStore,
    PackageStoreError, PublisherTrustRootRecord,
};
pub use parliament::Parliament;
pub use permit_store::{ConsumedPermit, PermitError, PermitStore};
pub use recovery_archive::{RecoveryArchive, RecoveryArchiveError, RecoveryBundle, RecoveryEntry};
pub use reference_monitor::{AuthorizationError, ReferenceMonitor};
pub use registry_client::{PackageRegistryClient, PackageRegistryError};
pub use replacement::{ReplacementEngine, ReplacementError};
pub use research_artifact::{
    MAX_RESEARCH_ARTIFACT_BYTES, ResearchArtifactError, ResearchArtifactRecord,
    ResearchArtifactStore, ResearchCandidateRecord,
};
pub use rollout_reducer::{RolloutReducer, RolloutReducerError};
pub use run_loop::{RunLoop, RunLoopError};
pub use secret_vault::{SecretVault, SecretVaultEntry, SecretVaultError, VaultSecret};
pub use self_healing::{SelfHealingEngine, SelfHealingError};
pub use service::{RuntimeService, RuntimeServiceError, RuntimeServiceScope};
pub use service_token::{ServiceToken, ServiceTokenError, ServiceTokenStore};
pub use sessions::{RolloutSummary, SessionEventPage, SessionSnapshot, SessionStore};
pub use skill_engine::{
    RemovalReceipt, SkillEngine, SkillError, SkillInspection, SkillProvenance, SkillRecord,
    SkillState,
};
pub use strategies::population::{
    PopulationParentPlan, PopulationPlan, PopulationStrategy, PopulationStrategyError,
};
pub use strategies::{StrategyBudget, StrategyError, StrategyProfile};
pub use subagent::{
    SubagentCleanupContext, SubagentCoordinator, SubagentCoordinatorError, SubagentSpawnContext,
};
pub use subagent_store::{
    ClaimedSubagent, SubagentPreparation, SubagentRecord, SubagentScope, SubagentStore,
    SubagentStoreError,
};
pub use tool_engine::{
    ToolContext, ToolDefinition, ToolEngine, ToolError, ToolInvocation, ToolPlan,
};
pub use wasm::{
    DEFAULT_WASM_FUEL, MAX_WASM_INPUT_BYTES, MAX_WASM_MEMORY_BYTES, MAX_WASM_OUTPUT_BYTES,
    WasmError, WasmExecutor, WasmGene, WasmGeneRequest, WasmResult,
};
