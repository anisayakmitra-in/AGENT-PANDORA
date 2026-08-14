#![forbid(unsafe_code)]

pub mod execution_controller;
pub mod executors;
pub mod parliament;
pub mod permit_store;
pub mod reference_monitor;
pub mod shadow_council;

pub use execution_controller::{ExecutionController, RunStatus, RunSummary, RuntimeError};
pub use parliament::Parliament;
pub use permit_store::{ConsumedPermit, PermitError, PermitStore};
pub use reference_monitor::{AuthorizationError, ReferenceMonitor};

pub fn runtime_is_available() -> bool {
    true
}
