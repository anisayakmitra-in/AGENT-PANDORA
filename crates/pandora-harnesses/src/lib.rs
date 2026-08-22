#![forbid(unsafe_code)]

pub mod catalog;
pub mod genes;
pub mod harness;
pub mod manifest;
pub mod profile;
pub mod slash;

pub use catalog::HarnessCatalog;

#[cfg(test)]
mod catalog_tests {
    use super::HarnessCatalog;
    use pandora_types::{
        HarnessId, HarnessKind, MetaComposition, PackageCompatibility, PackageDependency,
        PackageKind, PackageManifest, TrustEvidence, hash_artifact,
    };

    #[test]
    fn built_in_catalog_resolves_harnesses_by_id() {
        let catalog = HarnessCatalog::builtins();
        let coding = catalog
            .find(&HarnessId::new("coding-domain").unwrap())
            .expect("coding harness should be in the built-in catalog");

        assert_eq!(coding.manifest().id().as_str(), "coding-domain");
    }

    #[test]
    fn declarative_domain_profile_uses_only_matching_built_in_genes() {
        let artifact = b"domain profile";
        let package = PackageManifest::new(
            "example/domain",
            "1.0.0",
            PackageKind::DomainHarness,
            "publisher",
            hash_artifact(artifact),
            vec![PackageDependency::new("workspace.read", "0.1.0", false).unwrap()],
            PackageCompatibility::new("pandora>=2.0.0").unwrap(),
            "Apache-2.0",
            TrustEvidence::unsigned(),
        )
        .unwrap();

        let catalog = HarnessCatalog::builtins()
            .with_declarative_domain(&package)
            .expect("profile should bind to the built-in read Gene");
        let harness = catalog
            .find(&HarnessId::new("example/domain").unwrap())
            .expect("profile should be discoverable");

        assert!(harness.is_runnable());
        assert_eq!(harness.genes().len(), 1);
        assert_eq!(
            harness.genes()[0].manifest().id().as_str(),
            "workspace.read"
        );
    }

    #[test]
    fn declarative_meta_profile_keeps_composition_metadata_without_execution() {
        let artifact = b"meta profile";
        let package = PackageManifest::new_meta(
            "example/meta",
            "1.0.0",
            "publisher",
            hash_artifact(artifact),
            Vec::new(),
            PackageCompatibility::new("pandora>=2.0.0").unwrap(),
            "Apache-2.0",
            TrustEvidence::unsigned(),
            MetaComposition::new(vec![HarnessId::new("coding-domain").unwrap()], 4).unwrap(),
        )
        .unwrap();

        let catalog = HarnessCatalog::builtins()
            .with_declarative_meta(&package)
            .expect("validated Meta profile should be discoverable");
        let harness = catalog
            .find(&HarnessId::new("example/meta").unwrap())
            .expect("profile should be discoverable");

        assert_eq!(harness.manifest().kind(), HarnessKind::Meta);
        assert!(!harness.is_runnable());
        assert_eq!(
            harness
                .manifest()
                .meta_composition()
                .unwrap()
                .max_handoffs(),
            4
        );
        assert!(harness.genes().is_empty());
    }
}

pub use genes::{
    CodingAction, CodingGene, CodingGeneRole, CodingRequest, PlanningContext, coding_static_output,
};
pub use harness::{CodingHarness, CoordinationMetaHarness, CoreSourceHarness};
pub use manifest::{
    CODING_HARNESS_ID, CODING_HARNESS_VERSION, COORDINATION_META_HARNESS_ID,
    COORDINATION_META_HARNESS_VERSION, CORE_SOURCE_HARNESS_ID, CORE_SOURCE_HARNESS_VERSION,
};
pub use profile::{
    DeclarativeDomainHarness, DeclarativeMetaHarness, DomainProfileError, MetaProfileError,
};
pub use slash::{
    SlashCommand, SlashCommandCatalog, SlashCommandError, SlashCommandKind, canonical_gene_command,
    canonical_harness_command, canonical_profile_gene_command, canonical_profile_harness_command,
};

pub fn builtin_harnesses() -> Vec<Box<dyn pandora_types::Harness>> {
    HarnessCatalog::builtins().into_harnesses()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::HarnessKind;

    #[test]
    fn builtin_catalog_includes_bound_core_source_harness() {
        let core = builtin_harnesses()
            .into_iter()
            .find(|harness| harness.manifest().id().as_str() == "core-source")
            .expect("the built-in catalog should include the core source harness");

        assert_eq!(core.manifest().kind(), HarnessKind::Source);
        assert_eq!(
            core.manifest().constitutional_service(),
            Some("pandora-runtime")
        );
        assert_eq!(
            core.manifest().constitutional_service_version(),
            Some(env!("CARGO_PKG_VERSION"))
        );
        assert!(core.genes().is_empty());
    }

    #[test]
    fn builtin_catalog_includes_a_declared_coordination_meta_harness() {
        let meta = builtin_harnesses()
            .into_iter()
            .find(|harness| harness.manifest().id().as_str() == "coordination-meta")
            .expect("the built-in catalog should include the coordination Meta Harness");

        assert_eq!(meta.manifest().kind(), HarnessKind::Meta);
        assert!(meta.genes().is_empty());
        let composition = meta
            .manifest()
            .meta_composition()
            .expect("Meta Harness must declare its composition");
        assert!(
            composition
                .allowed_domains()
                .iter()
                .any(|id| id.as_str() == "coding-domain")
        );
    }

    #[test]
    fn coding_domain_exposes_the_governed_analysis_workflows() {
        let coding = builtin_harnesses()
            .into_iter()
            .find(|harness| harness.manifest().id().as_str() == "coding-domain")
            .expect("the built-in catalog should include the Coding Domain Harness");
        let genes = coding
            .genes()
            .iter()
            .map(|gene| (gene.manifest().id().as_str(), gene.manifest().kind()))
            .collect::<Vec<_>>();

        for id in [
            "daedalus.audit",
            "argus.review",
            "ariadne.debt",
            "hephaestus.measure",
            "athena.guide",
        ] {
            assert!(
                genes.iter().any(|(gene_id, kind)| {
                    *gene_id == id && *kind == pandora_types::GeneKind::Workflow
                }),
                "missing governed workflow Gene {id}"
            );
        }
    }

    #[test]
    fn slash_catalog_covers_the_coding_harness_and_every_gene() {
        let harnesses = HarnessCatalog::builtins();
        let commands = SlashCommandCatalog::from_harnesses(harnesses.iter()).unwrap();

        assert!(commands.resolve("/coding").is_some());
        for command in [
            "/read",
            "/search",
            "/patch",
            "/verify",
            "/review",
            "/audit",
            "/argus-review",
            "/debt",
            "/measure",
            "/guide",
        ] {
            assert!(commands.resolve(command).is_some(), "missing {command}");
        }
        for gene in harnesses
            .find(&pandora_types::HarnessId::new("coding-domain").unwrap())
            .unwrap()
            .genes()
        {
            let command = canonical_gene_command("coding-domain", gene.manifest().id().as_str());
            assert!(commands.resolve(&command).is_some(), "missing {command}");
        }
    }

    #[test]
    fn custom_harness_commands_are_namespaced_and_cannot_claim_core_aliases() {
        let artifact = b"custom domain";
        let package = pandora_types::PackageManifest::new(
            "owner/custom-domain",
            "1.0.0",
            pandora_types::PackageKind::DomainHarness,
            "publisher",
            pandora_types::hash_artifact(artifact),
            vec![
                pandora_types::PackageDependency::new("workspace.read", "0.1.0", false).unwrap(),
                pandora_types::PackageDependency::new("unavailable.gene", "1.0.0", true).unwrap(),
            ],
            pandora_types::PackageCompatibility::new("pandora>=2.0.0").unwrap(),
            "Apache-2.0",
            pandora_types::TrustEvidence::unsigned(),
        )
        .unwrap();
        let mut commands =
            SlashCommandCatalog::from_harnesses(HarnessCatalog::builtins().iter()).unwrap();

        commands.add_profile(&package).unwrap();

        assert!(
            commands
                .resolve("/harness:owner%2Fcustom-domain@1.0.0")
                .is_some()
        );
        assert!(
            commands
                .resolve("/gene:owner%2Fcustom-domain@1.0.0:workspace.read")
                .is_some()
        );
        assert!(
            commands
                .resolve("/gene:owner%2Fcustom-domain@1.0.0:unavailable.gene")
                .is_none()
        );
        assert_eq!(
            commands.resolve("/coding").unwrap().harness_id().as_str(),
            "coding-domain"
        );
    }
}
