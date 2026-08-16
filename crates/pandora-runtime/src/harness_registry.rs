use pandora_types::{PackageId, PackageKind, PackageManifest, hash_artifact};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PackageState {
    #[default]
    Installed,
    Admitted,
}

impl PackageState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::Admitted => "admitted",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackageRecord {
    manifest: PackageManifest,
    state: PackageState,
}

impl PackageRecord {
    pub fn manifest(&self) -> &PackageManifest {
        &self.manifest
    }

    pub fn state(&self) -> PackageState {
        self.state
    }

    pub const fn grants_runtime_authority(&self) -> bool {
        false
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HarnessRegistryError {
    ManifestMismatch,
    HashMismatch { expected: String, actual: String },
    UnsupportedKind(PackageKind),
    MissingDependency { id: String, version: String },
    DuplicateIdentity,
}

impl fmt::Display for HarnessRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ManifestMismatch => {
                formatter.write_str("package metadata does not match embedded manifest")
            }
            Self::HashMismatch { .. } => {
                formatter.write_str("package artifact hash does not match its manifest")
            }
            Self::UnsupportedKind(kind) => write!(
                formatter,
                "package kind {} is not installable",
                kind.as_str()
            ),
            Self::MissingDependency { id, version } => {
                write!(
                    formatter,
                    "required package dependency {id}@{version} is not installed"
                )
            }
            Self::DuplicateIdentity => {
                formatter.write_str("package id and version are already installed")
            }
        }
    }
}

impl std::error::Error for HarnessRegistryError {}

#[derive(Default)]
pub struct HarnessRegistry {
    packages: BTreeMap<(String, String), PackageRecord>,
}

impl HarnessRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn install(
        &mut self,
        declared: &PackageManifest,
        embedded: &PackageManifest,
        artifact: &[u8],
    ) -> Result<PackageRecord, HarnessRegistryError> {
        if declared.validate().is_err()
            || embedded.validate().is_err()
            || !declared.identity_matches(embedded)
            || declared.publisher() != embedded.publisher()
            || declared.content_hash() != embedded.content_hash()
            || declared.meta_composition() != embedded.meta_composition()
        {
            return Err(HarnessRegistryError::ManifestMismatch);
        }

        let actual = hash_artifact(artifact);
        if actual != embedded.content_hash() {
            return Err(HarnessRegistryError::HashMismatch {
                expected: embedded.content_hash().to_owned(),
                actual,
            });
        }

        if !matches!(
            embedded.kind(),
            PackageKind::Gene | PackageKind::MetaHarness
        ) {
            return Err(HarnessRegistryError::UnsupportedKind(embedded.kind()));
        }

        let key = (
            embedded.id().as_str().to_owned(),
            embedded.version().to_owned(),
        );
        if self.packages.contains_key(&key) {
            return Err(HarnessRegistryError::DuplicateIdentity);
        }

        for dependency in embedded
            .dependencies()
            .iter()
            .filter(|dependency| !dependency.optional())
        {
            let dependency_key = (
                dependency.id().as_str().to_owned(),
                dependency.version().to_owned(),
            );
            if !self.packages.contains_key(&dependency_key) {
                return Err(HarnessRegistryError::MissingDependency {
                    id: dependency.id().as_str().to_owned(),
                    version: dependency.version().to_owned(),
                });
            }
        }

        let record = PackageRecord {
            manifest: embedded.clone(),
            state: if embedded.kind() == PackageKind::MetaHarness {
                PackageState::Admitted
            } else {
                PackageState::Installed
            },
        };
        self.packages.insert(key, record.clone());
        Ok(record)
    }

    pub fn get(&self, id: &PackageId, version: &str) -> Option<&PackageRecord> {
        self.packages
            .get(&(id.as_str().to_owned(), version.to_owned()))
    }

    pub fn list(&self) -> Vec<PackageRecord> {
        self.packages.values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::{
        HarnessId, MetaComposition, PackageCompatibility, PackageDependency, PackageKind,
        PackageManifest, TrustEvidence, hash_artifact,
    };

    fn manifest(
        id: &str,
        version: &str,
        kind: PackageKind,
        dependencies: Vec<PackageDependency>,
        artifact: &[u8],
    ) -> PackageManifest {
        PackageManifest::new(
            id,
            version,
            kind,
            "publisher",
            hash_artifact(artifact),
            dependencies,
            PackageCompatibility::new("pandora>=2.0.0").unwrap(),
            "Apache-2.0",
            TrustEvidence::unsigned(),
        )
        .unwrap()
    }

    fn meta_manifest(id: &str, artifact: &[u8], domains: &[&str]) -> PackageManifest {
        PackageManifest::new_meta(
            id,
            "1.0.0",
            "publisher",
            hash_artifact(artifact),
            Vec::new(),
            PackageCompatibility::new("pandora>=2.0.0").unwrap(),
            "Apache-2.0",
            TrustEvidence::unsigned(),
            MetaComposition::new(
                domains
                    .iter()
                    .map(|domain| HarnessId::new(*domain).unwrap())
                    .collect(),
                8,
            )
            .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn identity_binding_rejects_id_version_and_kind_mismatches() {
        let artifact = b"gene artifact";
        let cases = [
            (
                manifest(
                    "publisher/declared",
                    "1.0.0",
                    PackageKind::Gene,
                    vec![],
                    artifact,
                ),
                manifest(
                    "publisher/embedded",
                    "1.0.0",
                    PackageKind::Gene,
                    vec![],
                    artifact,
                ),
            ),
            (
                manifest(
                    "publisher/gene",
                    "1.0.0",
                    PackageKind::Gene,
                    vec![],
                    artifact,
                ),
                manifest(
                    "publisher/gene",
                    "1.1.0",
                    PackageKind::Gene,
                    vec![],
                    artifact,
                ),
            ),
            (
                manifest(
                    "publisher/gene",
                    "1.0.0",
                    PackageKind::Gene,
                    vec![],
                    artifact,
                ),
                manifest(
                    "publisher/gene",
                    "1.0.0",
                    PackageKind::DomainHarness,
                    vec![],
                    artifact,
                ),
            ),
        ];

        for (declared, embedded) in cases {
            let mut registry = HarnessRegistry::new();
            assert_eq!(
                registry.install(&declared, &embedded, artifact),
                Err(HarnessRegistryError::ManifestMismatch)
            );
        }
    }

    #[test]
    fn hash_mismatch_is_rejected_before_installation() {
        let declared = manifest(
            "publisher/gene",
            "1.0.0",
            PackageKind::Gene,
            vec![],
            b"signed",
        );
        let mut registry = HarnessRegistry::new();

        assert!(matches!(
            registry.install(&declared, &declared, b"tampered"),
            Err(HarnessRegistryError::HashMismatch { .. })
        ));
        assert!(registry.list().is_empty());
    }

    #[test]
    fn unsupported_remote_kinds_fail_closed() {
        for kind in [
            PackageKind::DomainHarness,
            PackageKind::SourceHarness,
            PackageKind::Provider,
            PackageKind::Skill,
        ] {
            let artifact = b"metadata only";
            let package = manifest("publisher/package", "1.0.0", kind, vec![], artifact);
            let mut registry = HarnessRegistry::new();

            assert_eq!(
                registry.install(&package, &package, artifact),
                Err(HarnessRegistryError::UnsupportedKind(kind))
            );
            assert!(registry.list().is_empty());
        }
    }

    #[test]
    fn custom_meta_profile_is_admitted_without_runtime_authority() {
        let artifact = b"meta profile";
        let package = meta_manifest("publisher/coordination", artifact, &["coding-domain"]);
        let mut registry = HarnessRegistry::new();

        let record = registry.install(&package, &package, artifact).unwrap();

        assert_eq!(record.state(), PackageState::Admitted);
        assert!(!record.grants_runtime_authority());
        assert_eq!(
            record
                .manifest()
                .meta_composition()
                .unwrap()
                .allowed_domains()[0]
                .as_str(),
            "coding-domain"
        );
    }

    #[test]
    fn custom_meta_profile_cannot_change_composition_between_declarations() {
        let artifact = b"meta profile";
        let declared = meta_manifest("publisher/coordination", artifact, &["coding-domain"]);
        let embedded = meta_manifest("publisher/coordination", artifact, &["research-domain"]);
        let mut registry = HarnessRegistry::new();

        assert_eq!(
            registry.install(&declared, &embedded, artifact),
            Err(HarnessRegistryError::ManifestMismatch)
        );
        assert!(registry.list().is_empty());
    }

    #[test]
    fn invalid_deserialized_meta_profile_is_rejected_before_admission() {
        let artifact = b"meta profile";
        let package = meta_manifest("publisher/coordination", artifact, &["coding-domain"]);
        let mut value = serde_json::to_value(&package).unwrap();
        value["meta_composition"]["allowed_domains"] = serde_json::json!([]);
        let decoded: PackageManifest = serde_json::from_value(value).unwrap();
        let mut registry = HarnessRegistry::new();

        assert_eq!(
            registry.install(&decoded, &decoded, artifact),
            Err(HarnessRegistryError::ManifestMismatch)
        );
        assert!(registry.list().is_empty());
    }

    #[test]
    fn required_dependencies_must_be_installed_exactly() {
        let dependency = PackageDependency::new("publisher/foundation", "1.0.0", false).unwrap();
        let artifact = b"dependent gene";
        let package = manifest(
            "publisher/dependent",
            "1.0.0",
            PackageKind::Gene,
            vec![dependency],
            artifact,
        );
        let mut registry = HarnessRegistry::new();

        assert_eq!(
            registry.install(&package, &package, artifact),
            Err(HarnessRegistryError::MissingDependency {
                id: "publisher/foundation".to_owned(),
                version: "1.0.0".to_owned(),
            })
        );
    }

    #[test]
    fn duplicate_package_identity_is_rejected_without_replacing_the_record() {
        let artifact = b"gene artifact";
        let package = manifest(
            "publisher/gene",
            "1.0.0",
            PackageKind::Gene,
            vec![],
            artifact,
        );
        let mut registry = HarnessRegistry::new();
        let first = registry.install(&package, &package, artifact).unwrap();

        assert_eq!(
            registry.install(&package, &package, artifact),
            Err(HarnessRegistryError::DuplicateIdentity)
        );
        assert_eq!(registry.list(), vec![first]);
    }

    #[test]
    fn verified_gene_metadata_has_a_stable_non_authoritative_install_state() {
        let artifact = b"gene artifact";
        let package = manifest(
            "publisher/gene",
            "1.0.0",
            PackageKind::Gene,
            vec![],
            artifact,
        );
        let mut registry = HarnessRegistry::new();
        let record = registry.install(&package, &package, artifact).unwrap();

        assert_eq!(record.state(), PackageState::Installed);
        assert!(!record.grants_runtime_authority());
        assert_eq!(record.manifest().content_hash(), package.content_hash());
    }
}
