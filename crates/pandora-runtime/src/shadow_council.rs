use pandora_harnesses::{
    CODING_HARNESS_ID, DATA_HARNESS_ID, DEBUGGING_HARNESS_ID, DESIGN_HARNESS_ID, HarnessCatalog,
    RESEARCH_HARNESS_ID, SECURITY_HARNESS_ID,
};
use pandora_types::{GeneId, HarnessId, TaskIntent};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RoutingError {
    NoDefaultHarness,
    AmbiguousHarnesses { ids: Vec<HarnessId> },
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
    DeclaredDomainRoute,
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
        self.select_with_catalog(task, &HarnessCatalog::builtins())
    }

    pub fn select_with_catalog(
        &self,
        task: &TaskIntent,
        catalog: &HarnessCatalog,
    ) -> Result<Selection, RoutingError> {
        if let Some(harness_id) = task.requested_harness() {
            return Ok(Selection {
                harness_id: harness_id.clone(),
                gene_id: task.requested_gene().cloned(),
                reason: RoutingReason::RequestedHarness,
            });
        }

        let summary = task.summary().to_ascii_lowercase();
        let mut candidates = BTreeMap::<HarnessId, (usize, RoutingReason)>::new();
        for (id, reason, hints) in builtin_routes() {
            record_candidate(&mut candidates, id, reason, hints.copied(), &summary);
        }
        for (id, routing) in catalog.domain_routing() {
            record_candidate(
                &mut candidates,
                id.as_str(),
                RoutingReason::DeclaredDomainRoute,
                routing.hints().iter().map(String::as_str),
                &summary,
            );
        }
        let Some(best_score) = candidates.values().map(|(score, _)| *score).max() else {
            return Err(RoutingError::NoDefaultHarness);
        };
        let winners = candidates
            .into_iter()
            .filter(|(_, (score, _))| *score == best_score)
            .collect::<Vec<_>>();
        if winners.len() > 1 {
            return Err(RoutingError::AmbiguousHarnesses {
                ids: winners.into_iter().map(|(id, _)| id).collect(),
            });
        }
        let (harness_id, (_, reason)) = winners
            .into_iter()
            .next()
            .expect("a best score has at least one candidate");
        Ok(Selection {
            harness_id,
            gene_id: task.requested_gene().cloned(),
            reason,
        })
    }
}

fn builtin_routes() -> impl Iterator<
    Item = (
        &'static str,
        RoutingReason,
        std::slice::Iter<'static, &'static str>,
    ),
> {
    const ROUTES: &[(&str, RoutingReason, &[&str])] = &[
        (
            DESIGN_HARNESS_ID,
            RoutingReason::DesignTask,
            &["design", "accessibility"],
        ),
        (
            RESEARCH_HARNESS_ID,
            RoutingReason::ResearchTask,
            &[
                "research",
                "evidence",
                "citation",
                "source-read:",
                "source-compare:",
            ],
        ),
        (
            SECURITY_HARNESS_ID,
            RoutingReason::SecurityTask,
            &["security", "vulnerability", "threat", "cve"],
        ),
        (
            DEBUGGING_HARNESS_ID,
            RoutingReason::DebuggingTask,
            &[
                "debugging",
                "regression",
                "stack trace",
                "backtrace",
                "flaky",
            ],
        ),
        (
            DATA_HARNESS_ID,
            RoutingReason::DataTask,
            &[
                "data",
                "dataset",
                "schema",
                "lineage",
                "data quality",
                "analytics",
                "etl",
            ],
        ),
        (
            CODING_HARNESS_ID,
            RoutingReason::CodingTask,
            &[
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
            ],
        ),
    ];
    ROUTES
        .iter()
        .map(|(id, reason, hints)| (*id, *reason, hints.iter()))
}

fn record_candidate<'a>(
    candidates: &mut BTreeMap<HarnessId, (usize, RoutingReason)>,
    id: &str,
    reason: RoutingReason,
    hints: impl IntoIterator<Item = &'a str>,
    summary: &str,
) {
    let score = hints
        .into_iter()
        .filter(|hint| summary.contains(hint))
        .map(str::len)
        .max();
    let Some(score) = score else {
        return;
    };
    let id = HarnessId::new(id).expect("route IDs are validated Harness IDs");
    match candidates.get(&id) {
        Some((current, _)) if *current >= score => {}
        _ => {
            candidates.insert(id, (score, reason));
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
    use pandora_harnesses::HarnessCatalog;
    use pandora_types::{
        DomainRoutingProfile, HarnessId, PackageCompatibility, PackageDependency, PackageKind,
        PackageManifest, TaskIntent, TrustEvidence, hash_artifact,
    };

    use super::{RoutingError, ShadowCouncil};

    fn domain_package(id: &str, hints: Option<&[&str]>) -> PackageManifest {
        let package = PackageManifest::new(
            id,
            "1.0.0",
            PackageKind::DomainHarness,
            "publisher",
            hash_artifact(id.as_bytes()),
            vec![PackageDependency::new("workspace.read", "0.1.0", false).unwrap()],
            PackageCompatibility::new("pandora>=2.0.0").unwrap(),
            "MIT",
            TrustEvidence::unsigned(),
        )
        .unwrap();
        match hints {
            Some(hints) => package
                .with_domain_routing(
                    DomainRoutingProfile::new(
                        hints.iter().map(|hint| (*hint).to_owned()).collect(),
                    )
                    .unwrap(),
                )
                .unwrap(),
            None => package,
        }
    }

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

    #[test]
    fn installed_domain_routes_image_generation_without_a_built_in_category() {
        let package = domain_package(
            "creator/image-domain",
            Some(&["image generation", "diffusion model"]),
        );
        let catalog = HarnessCatalog::builtins()
            .with_declarative_domain(&package)
            .unwrap();
        let task = TaskIntent::new("Generate an image with a diffusion model").unwrap();

        let selection = ShadowCouncil::new()
            .select_with_catalog(&task, &catalog)
            .unwrap();

        assert_eq!(selection.harness_id().as_str(), "creator/image-domain");
    }

    #[test]
    fn specific_vlsi_route_outranks_the_generic_design_route() {
        let package = domain_package(
            "silicon/vlsi-domain",
            Some(&["vlsi design", "verilog", "place and route"]),
        );
        let catalog = HarnessCatalog::builtins()
            .with_declarative_domain(&package)
            .unwrap();
        let task = TaskIntent::new("Review this VLSI design and Verilog").unwrap();

        let selection = ShadowCouncil::new()
            .select_with_catalog(&task, &catalog)
            .unwrap();

        assert_eq!(selection.harness_id().as_str(), "silicon/vlsi-domain");
    }

    #[test]
    fn custom_domain_without_routing_is_explicit_selection_only() {
        let package = domain_package("creator/video-domain", None);
        let catalog = HarnessCatalog::builtins()
            .with_declarative_domain(&package)
            .unwrap();
        let task = TaskIntent::new("render a cinematic sequence").unwrap();

        assert_eq!(
            ShadowCouncil::new().select_with_catalog(&task, &catalog),
            Err(RoutingError::NoDefaultHarness)
        );
    }

    #[test]
    fn equally_specific_custom_routes_fail_closed() {
        let first = domain_package("creator/image-domain", Some(&["image generation"]));
        let second = domain_package("studio/image-domain", Some(&["image generation"]));
        let catalog = HarnessCatalog::builtins()
            .with_declarative_domain(&first)
            .unwrap()
            .with_declarative_domain(&second)
            .unwrap();
        let task = TaskIntent::new("image generation for a product shot").unwrap();

        assert_eq!(
            ShadowCouncil::new().select_with_catalog(&task, &catalog),
            Err(RoutingError::AmbiguousHarnesses {
                ids: vec![
                    HarnessId::new("creator/image-domain").unwrap(),
                    HarnessId::new("studio/image-domain").unwrap(),
                ],
            })
        );
    }

    #[test]
    fn explicit_harness_selection_overrides_auto_route() {
        let package = domain_package("creator/image-domain", Some(&["image generation"]));
        let catalog = HarnessCatalog::builtins()
            .with_declarative_domain(&package)
            .unwrap();
        let task = TaskIntent::new("image generation")
            .unwrap()
            .with_harness(HarnessId::new("coding-domain").unwrap());

        let selection = ShadowCouncil::new()
            .select_with_catalog(&task, &catalog)
            .unwrap();

        assert_eq!(selection.harness_id().as_str(), "coding-domain");
    }
}
