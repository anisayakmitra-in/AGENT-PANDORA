use pandora_harnesses::{
    CODING_HARNESS_ID, DATA_HARNESS_ID, DEBUGGING_HARNESS_ID, DESIGN_HARNESS_ID,
    RESEARCH_HARNESS_ID, SECURITY_HARNESS_ID,
};
use pandora_types::{GeneId, HarnessId, TaskIntent};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoutingError {
    NoDefaultHarness,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoutingReason {
    RequestedHarness,
    CodingTask,
    ResearchTask,
    DesignTask,
    SecurityTask,
    DebuggingTask,
    DataTask,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Selection {
    harness_id: HarnessId,
    gene_id: Option<GeneId>,
    reason: RoutingReason,
}

impl Selection {
    pub fn harness_id(&self) -> &HarnessId {
        &self.harness_id
    }

    pub fn gene_id(&self) -> Option<&GeneId> {
        self.gene_id.as_ref()
    }

    pub fn reason(&self) -> RoutingReason {
        self.reason
    }
}

pub struct ShadowCouncil;

impl ShadowCouncil {
    pub const fn new() -> Self {
        Self
    }

    pub fn select(&self, task: &TaskIntent) -> Result<Selection, RoutingError> {
        if let Some(harness_id) = task.requested_harness() {
            return Ok(Selection {
                harness_id: harness_id.clone(),
                gene_id: task.requested_gene().cloned(),
                reason: RoutingReason::RequestedHarness,
            });
        }

        let summary = task.summary().to_ascii_lowercase();
        let is_design_task = ["design", "accessibility"]
            .iter()
            .any(|term| summary.contains(term));
        if is_design_task {
            return Ok(Selection {
                harness_id: HarnessId::new(DESIGN_HARNESS_ID)
                    .expect("built-in harness ID is valid"),
                gene_id: task.requested_gene().cloned(),
                reason: RoutingReason::DesignTask,
            });
        }
        let is_research_task = [
            "research",
            "evidence",
            "citation",
            "source-read:",
            "source-compare:",
        ]
        .iter()
        .any(|term| summary.contains(term));
        if is_research_task {
            return Ok(Selection {
                harness_id: HarnessId::new(RESEARCH_HARNESS_ID)
                    .expect("built-in harness ID is valid"),
                gene_id: task.requested_gene().cloned(),
                reason: RoutingReason::ResearchTask,
            });
        }
        let is_security_task = ["security", "vulnerability", "threat", "cve"]
            .iter()
            .any(|term| summary.contains(term));
        if is_security_task {
            return Ok(Selection {
                harness_id: HarnessId::new(SECURITY_HARNESS_ID)
                    .expect("built-in harness ID is valid"),
                gene_id: task.requested_gene().cloned(),
                reason: RoutingReason::SecurityTask,
            });
        }
        let is_debugging_task = [
            "debugging",
            "regression",
            "stack trace",
            "backtrace",
            "flaky",
        ]
        .iter()
        .any(|term| summary.contains(term));
        if is_debugging_task {
            return Ok(Selection {
                harness_id: HarnessId::new(DEBUGGING_HARNESS_ID)
                    .expect("built-in harness ID is valid"),
                gene_id: task.requested_gene().cloned(),
                reason: RoutingReason::DebuggingTask,
            });
        }
        let is_data_task = [
            "data",
            "dataset",
            "schema",
            "lineage",
            "data quality",
            "analytics",
            "etl",
        ]
        .iter()
        .any(|term| summary.contains(term));
        if is_data_task {
            return Ok(Selection {
                harness_id: HarnessId::new(DATA_HARNESS_ID).expect("built-in harness ID is valid"),
                gene_id: task.requested_gene().cloned(),
                reason: RoutingReason::DataTask,
            });
        }
        let is_coding_task = [
            "code",
            "rust",
            "bug",
            "test",
            "compiler",
            "read:",
            "search:",
            "patch:",
            "verify",
            "review:",
            "audit",
            "deep-review:",
            "debt",
            "measure",
            "guide",
        ]
        .iter()
        .any(|term| summary.contains(term));
        if is_coding_task {
            return Ok(Selection {
                harness_id: HarnessId::new(CODING_HARNESS_ID)
                    .expect("built-in harness ID is valid"),
                gene_id: task.requested_gene().cloned(),
                reason: RoutingReason::CodingTask,
            });
        }

        Err(RoutingError::NoDefaultHarness)
    }
}

impl Default for ShadowCouncil {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use pandora_types::TaskIntent;

    use super::{RoutingError, ShadowCouncil};

    #[test]
    fn coding_task_selects_the_coding_domain() {
        let council = ShadowCouncil::new();
        let task = TaskIntent::new("Fix the Rust compiler error and run tests").unwrap();

        let selection = council.select(&task).unwrap();

        assert_eq!(selection.harness_id().as_str(), "coding-domain");
    }

    #[test]
    fn research_task_selects_the_research_domain() {
        let council = ShadowCouncil::new();
        let task = TaskIntent::new("Inventory the evidence and citations").unwrap();

        let selection = council.select(&task).unwrap();

        assert_eq!(selection.harness_id().as_str(), "research-domain");
    }

    #[test]
    fn design_task_selects_the_design_domain() {
        let council = ShadowCouncil::new();
        let task =
            TaskIntent::new("Inventory the design tokens and accessibility evidence").unwrap();

        let selection = council.select(&task).unwrap();

        assert_eq!(selection.harness_id().as_str(), "design-domain");
    }

    #[test]
    fn security_task_selects_the_security_domain() {
        let council = ShadowCouncil::new();
        let task = TaskIntent::new("security-audit").unwrap();

        let selection = council.select(&task).unwrap();

        assert_eq!(selection.harness_id().as_str(), "security-domain");
    }

    #[test]
    fn debugging_task_selects_the_debugging_domain() {
        let council = ShadowCouncil::new();
        let task = TaskIntent::new("Investigate a flaky regression with a stack trace").unwrap();

        let selection = council.select(&task).unwrap();

        assert_eq!(selection.harness_id().as_str(), "debugging-domain");
    }

    #[test]
    fn data_task_selects_the_data_domain() {
        let council = ShadowCouncil::new();
        let task = TaskIntent::new("inspect dataset schema and data lineage").unwrap();

        let selection = council.select(&task).unwrap();

        assert_eq!(selection.harness_id().as_str(), "data-domain");
    }

    #[test]
    fn unclassified_task_has_no_implicit_harness() {
        let council = ShadowCouncil::new();
        let task = TaskIntent::new("summarize the workspace").unwrap();

        assert_eq!(council.select(&task), Err(RoutingError::NoDefaultHarness));
    }
}
