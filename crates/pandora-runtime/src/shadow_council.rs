use pandora_types::{GeneId, HarnessId, TaskIntent};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoutingReason {
    RequestedHarness,
    CodingTask,
    DefaultDomain,
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

    pub fn select(&self, task: &TaskIntent) -> Selection {
        if let Some(harness_id) = task.requested_harness() {
            return Selection {
                harness_id: harness_id.clone(),
                gene_id: task.requested_gene().cloned(),
                reason: RoutingReason::RequestedHarness,
            };
        }

        let summary = task.summary().to_ascii_lowercase();
        let is_coding_task = [
            "code", "rust", "bug", "test", "compiler", "read:", "search:", "patch:", "verify",
            "review:",
        ]
        .iter()
        .any(|term| summary.contains(term));
        let (harness_id, reason) = if is_coding_task {
            ("coding-domain", RoutingReason::CodingTask)
        } else {
            ("general-domain", RoutingReason::DefaultDomain)
        };

        Selection {
            harness_id: HarnessId::new(harness_id).expect("built-in harness ID is valid"),
            gene_id: task.requested_gene().cloned(),
            reason,
        }
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

    use super::ShadowCouncil;

    #[test]
    fn coding_task_selects_the_coding_domain() {
        let council = ShadowCouncil::new();
        let task = TaskIntent::new("Fix the Rust compiler error and run tests").unwrap();

        let selection = council.select(&task);

        assert_eq!(selection.harness_id().as_str(), "coding-domain");
    }
}
