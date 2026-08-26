#![forbid(unsafe_code)]

pub mod catalog;
pub mod data;
pub mod debugging;
pub mod design;
pub mod genes;
pub mod harness;
pub mod manifest;
pub mod operations;
pub mod profile;
pub mod research;
pub mod security;
pub mod slash;

pub use catalog::HarnessCatalog;

pub fn canonical_harness_binding_digest(manifest: &pandora_types::HarnessManifest) -> String {
    let mut owned_genes = manifest
        .owned_genes()
        .iter()
        .map(|id| id.as_str())
        .collect::<Vec<_>>();
    owned_genes.sort_unstable();

    let mut canonical = format!(
        "harness-binding-v2\0id\0{}\0version\0{}\0name\0{}\0kind\0{}\0constitutional-service\0{}\0constitutional-service-version\0{}\0",
        manifest.id(),
        manifest.version(),
        manifest.name(),
        manifest.kind().as_str(),
        manifest.constitutional_service().unwrap_or_default(),
        manifest
            .constitutional_service_version()
            .unwrap_or_default(),
    );
    for gene in owned_genes {
        canonical.push_str("owned-gene\0");
        canonical.push_str(gene);
        canonical.push('\0');
    }
    if let Some(composition) = manifest.meta_composition() {
        canonical.push_str("meta-composition\0present\0max-handoffs\0");
        canonical.push_str(&composition.max_handoffs().to_string());
        canonical.push('\0');
        let mut components = composition
            .allowed_domains()
            .iter()
            .map(|id| id.as_str())
            .collect::<Vec<_>>();
        components.sort_unstable();
        for component in components {
            canonical.push_str("component-id\0");
            canonical.push_str(component);
            canonical.push('\0');
        }
    } else {
        canonical.push_str("meta-composition\0absent\0");
    }
    format!(
        "harness-{}",
        pandora_types::hash_artifact(canonical.as_bytes())
    )
}

#[cfg(test)]
mod catalog_tests {
    use super::{HarnessCatalog, canonical_harness_binding_digest};
    use pandora_types::{
        Capability, Gene, GeneError, GeneId, GeneInput, GeneKind, GeneManifest, HarnessId,
        HarnessKind, HarnessManifest, MetaComposition, OperationRequest, PackageCompatibility,
        PackageDependency, PackageKind, PackageManifest, TrustEvidence, hash_artifact,
    };

    struct PackageGene {
        manifest: GeneManifest,
    }

    impl Gene for PackageGene {
        fn manifest(&self) -> &GeneManifest {
            &self.manifest
        }

        fn plan(&self, _input: &GeneInput) -> Result<Vec<OperationRequest>, GeneError> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn harness_binding_digest_covers_domain_identity_and_sorts_owned_genes() {
        let manifest = HarnessManifest::new(
            "example/domain",
            "1.2.3",
            "Example Domain",
            HarnessKind::Domain,
            None,
            vec![
                GeneId::new("workspace.search").unwrap(),
                GeneId::new("workspace.read").unwrap(),
            ],
        )
        .unwrap();
        let reordered = HarnessManifest::new(
            "example/domain",
            "1.2.3",
            "Example Domain",
            HarnessKind::Domain,
            None,
            vec![
                GeneId::new("workspace.read").unwrap(),
                GeneId::new("workspace.search").unwrap(),
            ],
        )
        .unwrap();
        let renamed = HarnessManifest::new(
            "example/domain",
            "1.2.3",
            "Renamed Domain",
            HarnessKind::Domain,
            None,
            manifest.owned_genes().to_vec(),
        )
        .unwrap();

        assert_eq!(
            canonical_harness_binding_digest(&manifest),
            canonical_harness_binding_digest(&reordered)
        );
        assert_ne!(
            canonical_harness_binding_digest(&manifest),
            canonical_harness_binding_digest(&renamed)
        );
    }

    #[test]
    fn harness_binding_digest_covers_source_service_binding() {
        let first = HarnessManifest::new_source(
            "memory-source",
            "1.0.0",
            "Memory Source",
            "memory",
            "1.0.0",
            Vec::new(),
        )
        .unwrap();
        let second = HarnessManifest::new_source(
            "memory-source",
            "1.0.0",
            "Memory Source",
            "memory",
            "1.1.0",
            Vec::new(),
        )
        .unwrap();

        assert_ne!(
            canonical_harness_binding_digest(&first),
            canonical_harness_binding_digest(&second)
        );
    }

    #[test]
    fn harness_binding_digest_covers_meta_limit_and_component_ids() {
        let first = HarnessManifest::new_meta(
            "coordination-meta",
            "1.0.0",
            "Coordination Meta",
            MetaComposition::new(
                vec![
                    HarnessId::new("research-domain").unwrap(),
                    HarnessId::new("coding-domain").unwrap(),
                ],
                2,
            )
            .unwrap(),
        )
        .unwrap();
        let reordered = HarnessManifest::new_meta(
            "coordination-meta",
            "1.0.0",
            "Coordination Meta",
            MetaComposition::new(
                vec![
                    HarnessId::new("coding-domain").unwrap(),
                    HarnessId::new("research-domain").unwrap(),
                ],
                2,
            )
            .unwrap(),
        )
        .unwrap();
        let different_limit = HarnessManifest::new_meta(
            "coordination-meta",
            "1.0.0",
            "Coordination Meta",
            MetaComposition::new(
                vec![
                    HarnessId::new("coding-domain").unwrap(),
                    HarnessId::new("research-domain").unwrap(),
                ],
                3,
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            canonical_harness_binding_digest(&first),
            canonical_harness_binding_digest(&reordered)
        );
        assert_ne!(
            canonical_harness_binding_digest(&first),
            canonical_harness_binding_digest(&different_limit)
        );
    }

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
            "MIT",
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
    fn declarative_domain_profile_binds_an_exact_package_gene() {
        let artifact = b"domain profile";
        let package = PackageManifest::new(
            "example/domain",
            "1.0.0",
            PackageKind::DomainHarness,
            "publisher",
            hash_artifact(artifact),
            vec![PackageDependency::new("owner/transform", "1.2.3", false).unwrap()],
            PackageCompatibility::new("pandora>=2.0.0").unwrap(),
            "MIT",
            TrustEvidence::unsigned(),
        )
        .unwrap();
        let gene = PackageGene {
            manifest: GeneManifest::new(
                "owner/transform",
                "1.2.3",
                GeneKind::Tool,
                vec![Capability::WasmExecute],
            )
            .unwrap(),
        };

        let catalog = HarnessCatalog::builtins()
            .with_declarative_domain_genes(&package, vec![Box::new(gene)])
            .unwrap();
        let harness = catalog
            .find(&HarnessId::new("example/domain").unwrap())
            .unwrap();

        assert_eq!(harness.genes().len(), 1);
        assert_eq!(
            harness.genes()[0].manifest().id().as_str(),
            "owner/transform"
        );
        assert_eq!(harness.genes()[0].manifest().version(), "1.2.3");
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
            "MIT",
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

pub use design::{
    DesignAction, DesignGene, DesignGeneRole, DesignRequest, design_static_output, is_design_gene,
};
pub use genes::{
    CodingAction, CodingGene, CodingGeneRole, CodingRequest, PlanningContext, coding_static_output,
};
pub use harness::{
    CodingHarness, CoordinationMetaHarness, CoreSourceHarness, DebuggingHarness, DesignHarness,
    OperationsHarness, ResearchHarness, SecurityHarness,
};
pub use manifest::{
    CODING_HARNESS_ID, CODING_HARNESS_VERSION, COORDINATION_META_HARNESS_ID,
    COORDINATION_META_HARNESS_VERSION, CORE_SOURCE_HARNESS_ID, CORE_SOURCE_HARNESS_VERSION,
    DATA_HARNESS_ID, DATA_HARNESS_VERSION, DEBUGGING_HARNESS_ID, DEBUGGING_HARNESS_VERSION,
    DESIGN_HARNESS_ID, DESIGN_HARNESS_VERSION, OPERATIONS_HARNESS_ID, OPERATIONS_HARNESS_VERSION,
    RESEARCH_HARNESS_ID, RESEARCH_HARNESS_VERSION, SECURITY_HARNESS_ID, SECURITY_HARNESS_VERSION,
};
pub use operations::{
    OperationsAction, OperationsGene, OperationsGeneRole, OperationsRequest, is_operations_gene,
    operations_static_output,
};
pub use profile::{
    DeclarativeDomainHarness, DeclarativeMetaHarness, DomainProfileError, MetaProfileError,
};
pub use slash::{
    SlashCommand, SlashCommandCatalog, SlashCommandError, SlashCommandKind, canonical_gene_command,
    canonical_harness_command, canonical_profile_gene_command, canonical_profile_harness_command,
};

pub use data::{DataAction, DataGene, DataGeneRole, DataRequest, data_static_output, is_data_gene};
pub use debugging::{
    DebuggingAction, DebuggingGene, DebuggingGeneRole, DebuggingRequest, debugging_static_output,
    is_debugging_gene,
};
pub use research::{
    ResearchAction, ResearchGene, ResearchGeneRole, ResearchRequest, is_research_gene,
    research_static_output,
};
pub use security::{
    SecurityAction, SecurityGene, SecurityGeneRole, SecurityRequest, is_security_gene,
    security_static_output,
};

pub fn builtin_genes() -> Vec<Box<dyn pandora_types::Gene>> {
    let mut genes = CodingGene::all();
    genes.extend(ResearchGene::all());
    genes.extend(DesignGene::all());
    genes.extend(OperationsGene::all());
    genes.extend(SecurityGene::all());
    genes.extend(DebuggingGene::all());
    genes.extend(DataGene::all());
    genes
}

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
        assert!(
            composition
                .allowed_domains()
                .iter()
                .any(|id| id.as_str() == "research-domain")
        );
        assert!(
            composition
                .allowed_domains()
                .iter()
                .any(|id| id.as_str() == "design-domain")
        );
        assert!(
            composition
                .allowed_domains()
                .iter()
                .any(|id| id.as_str() == "operations-domain")
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
    fn research_domain_exposes_only_bounded_read_workflows() {
        let research = builtin_harnesses()
            .into_iter()
            .find(|harness| harness.manifest().id().as_str() == "research-domain")
            .expect("the built-in catalog should include the Research Domain Harness");
        let genes = research
            .genes()
            .iter()
            .map(|gene| gene.manifest())
            .collect::<Vec<_>>();

        assert_eq!(research.manifest().kind(), HarnessKind::Domain);
        assert_eq!(genes.len(), 6);
        for id in [
            "evidence.inventory",
            "evidence.search",
            "source.read",
            "source.compare",
            "citation.inventory",
            "research.guide",
        ] {
            assert!(genes.iter().any(|gene| gene.id().as_str() == id));
        }
        assert!(genes.iter().all(|gene| {
            gene.capabilities().is_empty()
                || gene.capabilities() == [pandora_types::Capability::FilesystemRead]
        }));
    }

    #[test]
    fn design_domain_exposes_only_bounded_read_workflows() {
        let design = builtin_harnesses()
            .into_iter()
            .find(|harness| harness.manifest().id().as_str() == "design-domain")
            .expect("the built-in catalog should include the Design Domain Harness");
        let genes = design
            .genes()
            .iter()
            .map(|gene| gene.manifest())
            .collect::<Vec<_>>();

        assert_eq!(design.manifest().kind(), HarnessKind::Domain);
        assert_eq!(genes.len(), 6);
        for id in [
            "design.inventory",
            "design.tokens",
            "design.inspect",
            "design.compare",
            "accessibility.evidence",
            "design.guide",
        ] {
            assert!(genes.iter().any(|gene| gene.id().as_str() == id));
        }
        assert!(genes.iter().all(|gene| {
            gene.capabilities().is_empty()
                || gene.capabilities() == [pandora_types::Capability::FilesystemRead]
        }));
    }

    #[test]
    fn operations_domain_exposes_only_bounded_read_workflows() {
        let operations = builtin_harnesses()
            .into_iter()
            .find(|harness| harness.manifest().id().as_str() == "operations-domain")
            .expect("the built-in catalog should include the Operations Domain Harness");
        let genes = operations
            .genes()
            .iter()
            .map(|gene| gene.manifest())
            .collect::<Vec<_>>();

        assert_eq!(operations.manifest().kind(), HarnessKind::Domain);
        assert_eq!(genes.len(), 6);
        for id in [
            "operations.inventory",
            "operations.search",
            "config.inspect",
            "config.compare",
            "deployment.evidence",
            "operations.guide",
        ] {
            assert!(genes.iter().any(|gene| gene.id().as_str() == id));
        }
        assert!(genes.iter().all(|gene| {
            gene.capabilities().is_empty()
                || gene.capabilities() == [pandora_types::Capability::FilesystemRead]
        }));
    }

    #[test]
    fn security_domain_exposes_only_bounded_read_workflows() {
        let security = builtin_harnesses()
            .into_iter()
            .find(|harness| harness.manifest().id().as_str() == "security-domain")
            .expect("the built-in catalog should include the Security Domain Harness");
        let genes = security
            .genes()
            .iter()
            .map(|gene| gene.manifest())
            .collect::<Vec<_>>();

        assert_eq!(security.manifest().kind(), HarnessKind::Domain);
        assert_eq!(genes.len(), 18);
        for id in [
            "security.assess",
            "security.audit",
            "security.scan",
            "security.deep-scan",
            "security.diff-scan",
            "security.dependencies",
            "security.threat-model",
            "security.discovery",
            "security.triage",
            "security.attack-path",
            "security.validation",
            "security.fix",
            "security.verify-fix",
            "security.writeup",
            "security.track",
            "security.hardening",
            "security.policy",
            "security.guide",
        ] {
            assert!(genes.iter().any(|gene| gene.id().as_str() == id));
        }
        assert!(genes.iter().all(|gene| {
            gene.capabilities().is_empty()
                || gene.capabilities() == [pandora_types::Capability::FilesystemRead]
        }));
    }

    #[test]
    fn debugging_domain_exposes_only_bounded_read_workflows() {
        let debugging = builtin_harnesses()
            .into_iter()
            .find(|harness| harness.manifest().id().as_str() == "debugging-domain")
            .expect("the built-in catalog should include the Debugging Domain Harness");
        let genes = debugging
            .genes()
            .iter()
            .map(|gene| gene.manifest())
            .collect::<Vec<_>>();

        assert_eq!(debugging.manifest().kind(), HarnessKind::Domain);
        assert_eq!(genes.len(), 6);
        for id in [
            "debugging.inventory",
            "debugging.failures",
            "debugging.tests",
            "debugging.regressions",
            "debugging.diagnostics",
            "debugging.guide",
        ] {
            assert!(genes.iter().any(|gene| gene.id().as_str() == id));
        }
        assert!(genes.iter().all(|gene| {
            gene.capabilities().is_empty()
                || gene.capabilities() == [pandora_types::Capability::FilesystemRead]
        }));
    }

    #[test]
    fn data_domain_exposes_only_bounded_read_workflows() {
        let data = builtin_harnesses()
            .into_iter()
            .find(|harness| harness.manifest().id().as_str() == "data-domain")
            .expect("the built-in catalog should include the Data Domain Harness");
        let genes = data
            .genes()
            .iter()
            .map(|gene| gene.manifest())
            .collect::<Vec<_>>();

        assert_eq!(data.manifest().kind(), HarnessKind::Domain);
        assert_eq!(genes.len(), 6);
        for id in [
            "data.inventory",
            "data.schema",
            "data.quality",
            "data.lineage",
            "data.analysis",
            "data.guide",
        ] {
            assert!(genes.iter().any(|gene| gene.id().as_str() == id));
        }
        assert!(genes.iter().all(|gene| {
            gene.capabilities().is_empty()
                || gene.capabilities() == [pandora_types::Capability::FilesystemRead]
        }));
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
            "/test",
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
    fn slash_catalog_covers_the_research_harness_and_every_gene() {
        let harnesses = HarnessCatalog::builtins();
        let commands = SlashCommandCatalog::from_harnesses(harnesses.iter()).unwrap();

        for command in [
            "/research",
            "/evidence-inventory",
            "/evidence-search",
            "/source-read",
            "/source-compare",
            "/citation-inventory",
            "/research-guide",
        ] {
            assert!(commands.resolve(command).is_some(), "missing {command}");
        }
        for gene in harnesses
            .find(&pandora_types::HarnessId::new("research-domain").unwrap())
            .unwrap()
            .genes()
        {
            let command = canonical_gene_command("research-domain", gene.manifest().id().as_str());
            assert!(commands.resolve(&command).is_some(), "missing {command}");
        }
    }

    #[test]
    fn slash_catalog_covers_the_design_harness_and_every_gene() {
        let harnesses = HarnessCatalog::builtins();
        let commands = SlashCommandCatalog::from_harnesses(harnesses.iter()).unwrap();

        for command in [
            "/design",
            "/design-inventory",
            "/design-tokens",
            "/design-inspect",
            "/design-compare",
            "/accessibility-evidence",
            "/design-guide",
        ] {
            assert!(commands.resolve(command).is_some(), "missing {command}");
        }
        for gene in harnesses
            .find(&pandora_types::HarnessId::new("design-domain").unwrap())
            .unwrap()
            .genes()
        {
            let command = canonical_gene_command("design-domain", gene.manifest().id().as_str());
            assert!(commands.resolve(&command).is_some(), "missing {command}");
        }
    }

    #[test]
    fn slash_catalog_covers_the_operations_harness_and_every_gene() {
        let harnesses = HarnessCatalog::builtins();
        let commands = SlashCommandCatalog::from_harnesses(harnesses.iter()).unwrap();

        for command in [
            "/operations",
            "/operations-inventory",
            "/operations-search",
            "/config-inspect",
            "/config-compare",
            "/deployment-evidence",
            "/operations-guide",
        ] {
            assert!(commands.resolve(command).is_some(), "missing {command}");
        }
        for gene in harnesses
            .find(&pandora_types::HarnessId::new("operations-domain").unwrap())
            .unwrap()
            .genes()
        {
            let command =
                canonical_gene_command("operations-domain", gene.manifest().id().as_str());
            assert!(commands.resolve(&command).is_some(), "missing {command}");
        }
    }

    #[test]
    fn slash_catalog_covers_the_security_harness_and_every_gene() {
        let harnesses = HarnessCatalog::builtins();
        let commands = SlashCommandCatalog::from_harnesses(harnesses.iter()).unwrap();

        for command in [
            "/security",
            "/security-audit",
            "/security-scan",
            "/security-dependencies",
            "/security-threat-model",
            "/security-discovery",
            "/security-triage",
            "/security-attack-path",
            "/security-validation",
            "/security-fix",
            "/security-verify-fix",
            "/security-writeup",
            "/security-track",
            "/security-hardening",
            "/security-policy",
            "/security-guide",
        ] {
            assert!(commands.resolve(command).is_some(), "missing {command}");
        }
        for gene in harnesses
            .find(&pandora_types::HarnessId::new("security-domain").unwrap())
            .unwrap()
            .genes()
        {
            let command = canonical_gene_command("security-domain", gene.manifest().id().as_str());
            assert!(commands.resolve(&command).is_some(), "missing {command}");
        }
    }

    #[test]
    fn slash_catalog_covers_the_debugging_harness_and_every_gene() {
        let harnesses = HarnessCatalog::builtins();
        let commands = SlashCommandCatalog::from_harnesses(harnesses.iter()).unwrap();

        for command in [
            "/debugging",
            "/debugging-inventory",
            "/debugging-failures",
            "/debugging-tests",
            "/debugging-regressions",
            "/debugging-diagnostics",
            "/debugging-guide",
        ] {
            assert!(commands.resolve(command).is_some(), "missing {command}");
        }
        for gene in harnesses
            .find(&pandora_types::HarnessId::new("debugging-domain").unwrap())
            .unwrap()
            .genes()
        {
            let command = canonical_gene_command("debugging-domain", gene.manifest().id().as_str());
            assert!(commands.resolve(&command).is_some(), "missing {command}");
        }
    }

    #[test]
    fn slash_catalog_covers_the_data_harness_and_every_gene() {
        let harnesses = HarnessCatalog::builtins();
        let commands = SlashCommandCatalog::from_harnesses(harnesses.iter()).unwrap();

        for command in [
            "/data",
            "/data-inventory",
            "/data-schema",
            "/data-quality",
            "/data-lineage",
            "/data-analysis",
            "/data-guide",
        ] {
            assert!(commands.resolve(command).is_some(), "missing {command}");
        }
        for gene in harnesses
            .find(&pandora_types::HarnessId::new("data-domain").unwrap())
            .unwrap()
            .genes()
        {
            let command = canonical_gene_command("data-domain", gene.manifest().id().as_str());
            assert!(commands.resolve(&command).is_some(), "missing {command}");
        }
    }

    #[test]
    fn custom_domain_profiles_can_bind_research_genes() {
        let artifact = b"custom research domain";
        let package = pandora_types::PackageManifest::new(
            "owner/custom-research",
            "1.0.0",
            pandora_types::PackageKind::DomainHarness,
            "publisher",
            pandora_types::hash_artifact(artifact),
            vec![pandora_types::PackageDependency::new("evidence.search", "0.1.0", false).unwrap()],
            pandora_types::PackageCompatibility::new("pandora>=2.0.0").unwrap(),
            "MIT",
            pandora_types::TrustEvidence::unsigned(),
        )
        .unwrap();

        let catalog = HarnessCatalog::builtins()
            .with_declarative_domain(&package)
            .unwrap();
        let custom = catalog
            .find(&pandora_types::HarnessId::new("owner/custom-research").unwrap())
            .unwrap();

        assert_eq!(custom.genes().len(), 1);
        assert_eq!(
            custom.genes()[0].manifest().id().as_str(),
            "evidence.search"
        );
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
            "MIT",
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
