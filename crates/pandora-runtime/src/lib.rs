#![forbid(unsafe_code)]

pub mod approvals;
pub mod config;
pub mod context_engine;
pub mod context_recovery;
pub mod evaluation_engine;
pub mod execution_controller;
pub mod executors;
pub mod harness_registry;
pub mod memory_engine;
pub mod package_admission;
pub mod parliament;
pub mod permit_store;
pub mod reference_monitor;
pub mod sessions;
pub mod shadow_council;
pub mod skill_engine;
pub mod tool_engine;

pub use approvals::{
    ApprovalError, ApprovalRequest, ApprovalStatus, ApprovalStore, PendingApproval,
};
pub use context_engine::{ContextEngine, ContextError};
pub use context_recovery::{ContextRecovery, RecoveryDecision, RecoveryInput, RecoveryStep};
pub use evaluation_engine::{EvaluationEngine, EvaluationError};
pub use execution_controller::{ExecutionController, RunStatus, RunSummary, RuntimeError};
pub use harness_registry::{HarnessRegistry, HarnessRegistryError, PackageRecord, PackageState};
pub use memory_engine::{MemoryEngine, MemoryError};
pub use parliament::Parliament;
pub use permit_store::{ConsumedPermit, PermitError, PermitStore};
pub use reference_monitor::{AuthorizationError, ReferenceMonitor};
pub use skill_engine::{
    RemovalReceipt, SkillEngine, SkillError, SkillInspection, SkillProvenance, SkillRecord,
    SkillState,
};
pub use tool_engine::{ToolContext, ToolDefinition, ToolEngine, ToolError, ToolPlan};

pub fn runtime_is_available() -> bool {
    true
}
