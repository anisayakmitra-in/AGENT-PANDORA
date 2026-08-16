#![forbid(unsafe_code)]

pub mod adaptive_engine;
pub mod agent_loop;
pub mod approvals;
pub mod config;
pub mod context_engine;
pub mod context_recovery;
pub mod evaluation_engine;
pub mod evolution;
pub mod execution_controller;
pub mod executors;
pub mod harness_registry;
pub mod human_review;
pub mod memory_engine;
pub mod mutation;
pub mod observability;
pub mod orchestration_engine;
pub mod package_admission;
pub mod package_store;
pub mod parliament;
pub mod permit_store;
pub mod reference_monitor;
pub mod replacement;
pub mod run_loop;
pub mod sessions;
pub mod shadow_council;
pub mod skill_engine;
pub mod strategies;
pub mod tool_engine;

pub use adaptive_engine::{AdaptationResult, AdaptiveEngine, AdaptiveError};
pub use agent_loop::{
    AgentApprovalContext, AgentLoop, AgentLoopError, AgentRunRequest, AgentRunSummary,
    MAX_AGENT_TOOL_CALLS, MAX_AGENT_TURNS,
};
pub use approvals::{
    ApprovalError, ApprovalRequest, ApprovalStatus, ApprovalStore, PendingApproval,
};
pub use context_engine::{ContextEngine, ContextError};
pub use context_recovery::{ContextRecovery, RecoveryDecision, RecoveryInput, RecoveryStep};
pub use evaluation_engine::{EvaluationEngine, EvaluationError};
pub use evolution::{EvolutionEngine, EvolutionError};
pub use execution_controller::{ExecutionController, RunStatus, RunSummary, RuntimeError};
pub use harness_registry::{HarnessRegistry, HarnessRegistryError, PackageRecord, PackageState};
pub use human_review::{
    HumanMode, HumanReviewEngine, ReviewDecision, ReviewError, ReviewReceipt, ReviewRecord,
    ReviewState, ReviewSubject,
};
pub use memory_engine::{MemoryEngine, MemoryError};
pub use mutation::{MutationEngine, MutationError};
pub use observability::{ObservabilityEngine, ObservabilityError};
pub use orchestration_engine::{OrchestrationEngine, OrchestrationError, OrchestrationRun};
pub use package_store::{MAX_STORED_ARTIFACT_BYTES, PackageStore, PackageStoreError};
pub use parliament::Parliament;
pub use permit_store::{ConsumedPermit, PermitError, PermitStore};
pub use reference_monitor::{AuthorizationError, ReferenceMonitor};
pub use replacement::{ReplacementEngine, ReplacementError};
pub use run_loop::{RunLoop, RunLoopError};
pub use skill_engine::{
    RemovalReceipt, SkillEngine, SkillError, SkillInspection, SkillProvenance, SkillRecord,
    SkillState,
};
pub use strategies::{StrategyBudget, StrategyError, StrategyProfile};
pub use tool_engine::{
    ToolContext, ToolDefinition, ToolEngine, ToolError, ToolInvocation, ToolPlan,
};
