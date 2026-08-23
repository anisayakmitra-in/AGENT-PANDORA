#![forbid(unsafe_code)]

pub mod adaptive_engine;
pub mod agent_loop;
pub mod approvals;
pub mod coding_feedback;
pub mod config;
pub mod context_engine;
pub mod context_recovery;
pub mod efficiency_engine;
pub mod efficiency_store;
pub mod evaluation_engine;
pub mod evolution;
pub mod execution_controller;
pub mod executors;
pub mod harness_registry;
pub mod hooks;
pub mod human_review;
pub mod job_store;
pub mod mcp;
pub mod memory_engine;
pub mod mutation;
pub mod observability;
pub mod orchestration_engine;
pub mod package_admission;
pub mod package_store;
pub mod parliament;
pub mod permit_store;
pub mod reference_monitor;
pub mod registry_client;
pub mod replacement;
pub mod run_loop;
pub mod service;
pub mod service_token;
pub mod sessions;
pub mod shadow_council;
pub mod skill_engine;
pub mod strategies;
pub mod tool_engine;

#[cfg(test)]
mod test_support;

pub use adaptive_engine::{AdaptationResult, AdaptiveEngine, AdaptiveError};
pub use agent_loop::{
    AgentApprovalContext, AgentLoop, AgentLoopError, AgentRunRequest, AgentRunSummary,
    MAX_AGENT_TOOL_CALLS, MAX_AGENT_TURNS,
};
pub use approvals::{
    ApprovalError, ApprovalRequest, ApprovalStatus, ApprovalStore, PendingApproval,
};
pub use coding_feedback::{
    CodingFeedbackError, CodingFeedbackInput, CodingFeedbackLoop, CodingFeedbackResult,
};
pub use context_engine::{ContextCacheStats, ContextEngine, ContextError};
pub use context_recovery::{ContextRecovery, RecoveryDecision, RecoveryInput, RecoveryStep};
pub use efficiency_engine::{DEFAULT_MAX_SAMPLES_PER_TARGET, EfficiencyEngine, EfficiencyError};
pub use efficiency_store::{EfficiencyStore, EfficiencyStoreError};
pub use evaluation_engine::{EvaluationEngine, EvaluationError};
pub use evolution::{EvolutionEngine, EvolutionError};
pub use execution_controller::{ExecutionController, RunStatus, RunSummary, RuntimeError};
pub use executors::{
    GitWorktreeExecutor, WorktreeChange, WorktreeCommand, WorktreeError, WorktreeResult,
};
pub use harness_registry::{HarnessRegistry, HarnessRegistryError, PackageRecord, PackageState};
pub use hooks::{HookPoint, HookSelector, LifecycleHook, LifecycleHooks};
pub use human_review::{
    HumanMode, HumanReviewEngine, ReviewDecision, ReviewError, ReviewReceipt, ReviewRecord,
    ReviewState, ReviewSubject,
};
pub use job_store::{JobRecord, JobStore, JobStoreError, MAX_JOB_RESULT_BYTES};
pub use mcp::{
    MCP_LEGACY_PROTOCOL_VERSION, MCP_PROTOCOL_VERSION, McpError, McpFailure, McpInvocation,
    McpProtocolMode, McpServer, McpStart, McpStdioConfig, McpToolResult, McpWireEra,
};
pub use memory_engine::{MemoryEngine, MemoryError};
pub use mutation::{MutationEngine, MutationError};
pub use observability::{ObservabilityEngine, ObservabilityError};
pub use orchestration_engine::{
    DomainProfileRun, OrchestrationEngine, OrchestrationError, OrchestrationRun,
};
pub use package_store::{MAX_STORED_ARTIFACT_BYTES, PackageStore, PackageStoreError};
pub use parliament::Parliament;
pub use permit_store::{ConsumedPermit, PermitError, PermitStore};
pub use reference_monitor::{AuthorizationError, ReferenceMonitor};
pub use registry_client::{PackageRegistryClient, PackageRegistryError};
pub use replacement::{ReplacementEngine, ReplacementError};
pub use run_loop::{RunLoop, RunLoopError};
pub use service::{RuntimeService, RuntimeServiceError, RuntimeServiceScope};
pub use service_token::{ServiceToken, ServiceTokenError, ServiceTokenStore};
pub use skill_engine::{
    RemovalReceipt, SkillEngine, SkillError, SkillInspection, SkillProvenance, SkillRecord,
    SkillState,
};
pub use strategies::{StrategyBudget, StrategyError, StrategyProfile};
pub use tool_engine::{
    ToolContext, ToolDefinition, ToolEngine, ToolError, ToolInvocation, ToolPlan,
};
