#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

/// 4-Level Task Hierarchy used by the PRP Pipeline.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrpHierarchyLevel {
    Phase,
    Milestone,
    Task,
    Subtask,
}

impl PrpHierarchyLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Phase => "phase",
            Self::Milestone => "milestone",
            Self::Task => "task",
            Self::Subtask => "subtask",
        }
    }
}

/// The 4-Level Progressive Validation Gates from PRP Pipeline engineering.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrpValidationGate {
    /// Level 1: Syntax / compilation gate (e.g. `cargo check`, linter).
    Syntax,
    /// Level 2: Unit tests gate (e.g. `cargo test --lib`).
    Unit,
    /// Level 3: Integration / E2E tests gate (e.g. `cargo test --test integration`).
    Integration,
    /// Level 4: Manual / Receipt validation gate (e.g. human sign-off, deterministic proof receipts).
    ReceiptCheck,
}

impl PrpValidationGate {
    pub const ALL: [Self; 4] = [
        Self::Syntax,
        Self::Unit,
        Self::Integration,
        Self::ReceiptCheck,
    ];

    pub const fn gate_number(self) -> u8 {
        match self {
            Self::Syntax => 1,
            Self::Unit => 2,
            Self::Integration => 3,
            Self::ReceiptCheck => 4,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Syntax => "Gate 1: Syntax",
            Self::Unit => "Gate 2: Unit",
            Self::Integration => "Gate 3: Integration",
            Self::ReceiptCheck => "Gate 4: ReceiptCheck",
        }
    }
}

/// The outcome of executing a single validation gate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PrpGateResult {
    pub gate: PrpValidationGate,
    pub passed: bool,
    pub duration_ms: u64,
    pub details: String,
}

/// A Product Requirement Prompt (PRP) document representing a structured,
/// context-dense task specification with invariant contracts and validation gates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PrpDocument {
    pub id: String,
    pub title: String,
    pub level: PrpHierarchyLevel,
    pub phase: String,
    pub milestone: String,
    pub task: String,
    pub subtask: Option<String>,
    pub context_summary: String,
    pub pre_flight_invariants: Vec<String>,
    pub acceptance_criteria: Vec<String>,
    pub required_gates: Vec<PrpValidationGate>,
    pub rollback_plan: String,
}

impl PrpDocument {
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        phase: impl Into<String>,
        milestone: impl Into<String>,
        task: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            level: PrpHierarchyLevel::Task,
            phase: phase.into(),
            milestone: milestone.into(),
            task: task.into(),
            subtask: None,
            context_summary: String::new(),
            pre_flight_invariants: Vec::new(),
            acceptance_criteria: Vec::new(),
            required_gates: PrpValidationGate::ALL.to_vec(),
            rollback_plan: "Revert workspace to snapshot commit".to_string(),
        }
    }

    pub fn scaffold_default(id: &str, title: &str) -> Self {
        let mut doc = Self::new(
            id,
            title,
            "Phase 16",
            "M1 Core Architecture",
            "T1 Implementation",
        );
        doc.context_summary =
            "Implementation of autonomous self-improvement components".to_string();
        doc.pre_flight_invariants = vec![
            "Working tree is clean".to_string(),
            "#![forbid(unsafe_code)] is enforced".to_string(),
            "All unit and integration tests compile without warnings".to_string(),
        ];
        doc.acceptance_criteria = vec![
            "Zero clippy warnings on cargo clippy --workspace --all-targets --locked".to_string(),
            "All test suites pass 100%".to_string(),
            "Shell completions and CLI documentation stay in sync".to_string(),
        ];
        doc
    }
}

/// The report generated after evaluating a PRP document through its validation gates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PrpValidationReport {
    pub prp_id: String,
    pub all_passed: bool,
    pub gate_results: Vec<PrpGateResult>,
    pub summary: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prp_scaffold_default() {
        let prp = PrpDocument::scaffold_default("PRP-001", "Evolution Synthesis");
        assert_eq!(prp.required_gates.len(), 4);
        assert_eq!(prp.pre_flight_invariants.len(), 3);
        assert_eq!(prp.level, PrpHierarchyLevel::Task);
    }
}
