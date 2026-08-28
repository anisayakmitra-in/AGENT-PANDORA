use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use pandora_harnesses::{builtin_genes, builtin_harnesses, replaceable_builtin_harness_kind};
use pandora_types::{
    HarnessId, HarnessKind, PackageId, PackageKind, PackageManifest, TrustLevel, hash_artifact,
};
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
    HashMismatch {
        expected: String,
        actual: String,
    },
    IncompatibleRuntime {
        required: String,
        actual: String,
    },
    UnverifiedTrustClaim,
    SignatureRequired,
    InvalidSignatureEncoding,
    InvalidSignature,
    OfficialTrustUnsupported,
    UnsupportedKind(PackageKind),
    MissingDependency {
        id: String,
        version: String,
    },
    DomainDependencyNotGene {
        id: String,
        version: String,
        kind: PackageKind,
    },
    DomainHarnessRequiresGene,
    MetaDomainMissing {
        id: String,
    },
    MetaDomainNotDomain {
        id: String,
        kind: PackageKind,
    },
    AmbiguousMetaDomain {
        id: String,
    },
    ReservedHarnessId {
        id: String,
    },
    BuiltInReplacementRequiresVerifiedSignature {
        id: String,
    },
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
            Self::IncompatibleRuntime { required, actual } => write!(
                formatter,
                "package requires {required}, but this Pandora runtime is {actual}"
            ),
            Self::UnverifiedTrustClaim => {
                formatter.write_str("local package admission cannot verify a claimed trust level")
            }
            Self::SignatureRequired => formatter
                .write_str("verified package admission requires a public key and signature"),
            Self::InvalidSignatureEncoding => {
                formatter.write_str("package signature evidence is not valid fixed-width hex")
            }
            Self::InvalidSignature => {
                formatter.write_str("package signature does not match its identity and artifact")
            }
            Self::OfficialTrustUnsupported => {
                formatter.write_str("official package trust requires a configured publisher root")
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
            Self::DomainDependencyNotGene { id, version, kind } => write!(
                formatter,
                "Domain Harness dependency {id}@{version} is {}, not a Gene",
                kind.as_str()
            ),
            Self::DomainHarnessRequiresGene => {
                formatter.write_str("Domain Harness packages require a required Gene dependency")
            }
            Self::MetaDomainMissing { id } => {
                write!(formatter, "Meta Harness domain {id} is not admitted")
            }
            Self::MetaDomainNotDomain { id, kind } => {
                write!(
                    formatter,
                    "Meta Harness member {id} is {}, not a Domain Harness",
                    kind.as_str()
                )
            }
            Self::AmbiguousMetaDomain { id } => {
                write!(
                    formatter,
                    "Meta Harness domain {id} resolves to multiple profiles"
                )
            }
            Self::ReservedHarnessId { id } => {
                write!(formatter, "{id} is reserved by a built-in Harness")
            }
            Self::BuiltInReplacementRequiresVerifiedSignature { id } => write!(
                formatter,
                "optional built-in Harness replacement {id} requires a verified signature"
            ),
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
        if declared.validate().is_err() || embedded.validate().is_err() || declared != embedded {
            return Err(HarnessRegistryError::ManifestMismatch);
        }

        let actual = hash_artifact(artifact);
        if actual != embedded.content_hash() {
            return Err(HarnessRegistryError::HashMismatch {
                expected: embedded.content_hash().to_owned(),
                actual,
            });
        }
        if !embedded
            .compatibility()
            .matches_runtime(env!("CARGO_PKG_VERSION"))
            .map_err(|_| HarnessRegistryError::ManifestMismatch)?
        {
            return Err(HarnessRegistryError::IncompatibleRuntime {
                required: embedded.compatibility().runtime().to_owned(),
                actual: env!("CARGO_PKG_VERSION").to_owned(),
            });
        }
        match embedded.trust().level() {
            TrustLevel::Unverified => {}
            TrustLevel::Verified => verify_package_signature(embedded)?,
            TrustLevel::Official => return Err(HarnessRegistryError::OfficialTrustUnsupported),
        }

        if let Some(built_in) = builtin_harnesses()
            .into_iter()
            .find(|harness| harness.manifest().id().as_str() == embedded.id().as_str())
        {
            let replacement_kind = replaceable_builtin_harness_kind(embedded.id().as_str());
            if replacement_kind != Some(built_in.manifest().kind())
                || replacement_kind.map(PackageKind::from) != Some(embedded.kind())
            {
                return Err(HarnessRegistryError::ReservedHarnessId {
                    id: embedded.id().as_str().to_owned(),
                });
            }
            if embedded.trust().level() != TrustLevel::Verified {
                return Err(
                    HarnessRegistryError::BuiltInReplacementRequiresVerifiedSignature {
                        id: embedded.id().as_str().to_owned(),
                    },
                );
            }
        }
        if !matches!(
            embedded.kind(),
            PackageKind::Gene | PackageKind::DomainHarness | PackageKind::MetaHarness
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

        let mut has_required_gene = false;
        for dependency in embedded.dependencies() {
            let dependency_key = (
                dependency.id().as_str().to_owned(),
                dependency.version().to_owned(),
            );
            let dependency_kind = self
                .packages
                .get(&dependency_key)
                .map(|record| record.manifest().kind())
                .or_else(|| {
                    builtin_gene_available(dependency.id().as_str(), dependency.version())
                        .then_some(PackageKind::Gene)
                });
            match dependency_kind {
                Some(kind) => {
                    if embedded.kind() == PackageKind::DomainHarness && kind != PackageKind::Gene {
                        return Err(HarnessRegistryError::DomainDependencyNotGene {
                            id: dependency.id().as_str().to_owned(),
                            version: dependency.version().to_owned(),
                            kind,
                        });
                    }
                    if embedded.kind() == PackageKind::DomainHarness && !dependency.optional() {
                        has_required_gene = true;
                    }
                }
                None if !dependency.optional() => {
                    return Err(HarnessRegistryError::MissingDependency {
                        id: dependency.id().as_str().to_owned(),
                        version: dependency.version().to_owned(),
                    });
                }
                None => {}
            }
        }

        if embedded.kind() == PackageKind::DomainHarness && !has_required_gene {
            return Err(HarnessRegistryError::DomainHarnessRequiresGene);
        }
        if embedded.kind() == PackageKind::MetaHarness {
            for domain in embedded
                .meta_composition()
                .expect("validated Meta Harness package has a composition")
                .allowed_domains()
            {
                self.require_domain(domain)?;
            }
        }

        let record = PackageRecord {
            manifest: embedded.clone(),
            state: match embedded.kind() {
                PackageKind::MetaHarness | PackageKind::DomainHarness => PackageState::Admitted,
                _ => PackageState::Installed,
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

    fn require_domain(&self, domain: &HarnessId) -> Result<(), HarnessRegistryError> {
        let installed = self
            .packages
            .values()
            .filter(|record| record.manifest().id().as_str() == domain.as_str())
            .collect::<Vec<_>>();
        let built_in = builtin_harnesses()
            .into_iter()
            .find(|harness| harness.manifest().id() == domain)
            .map(|harness| harness.manifest().kind() == HarnessKind::Domain)
            .unwrap_or(false);

        if built_in {
            return Ok(());
        }
        if installed.is_empty() {
            return Err(HarnessRegistryError::MetaDomainMissing {
                id: domain.as_str().to_owned(),
            });
        }
        if let Some(record) = installed
            .iter()
            .find(|record| record.manifest().kind() != PackageKind::DomainHarness)
        {
            return Err(HarnessRegistryError::MetaDomainNotDomain {
                id: domain.as_str().to_owned(),
                kind: record.manifest().kind(),
            });
        }
        Ok(())
    }
}

fn builtin_gene_available(id: &str, version: &str) -> bool {
    builtin_genes()
        .iter()
        .any(|gene| gene.manifest().id().as_str() == id && gene.manifest().version() == version)
}

fn verify_package_signature(manifest: &PackageManifest) -> Result<(), HarnessRegistryError> {
    let Some(public_key) = manifest.trust().public_key() else {
        return Err(HarnessRegistryError::SignatureRequired);
    };
    let Some(signature) = manifest.trust().signature() else {
        return Err(HarnessRegistryError::SignatureRequired);
    };
    let public_key = decode_signature_bytes::<32>(public_key)?;
    let signature = decode_signature_bytes::<64>(signature)?;
    let public_key = VerifyingKey::from_bytes(&public_key)
        .map_err(|_| HarnessRegistryError::InvalidSignatureEncoding)?;
    let signature = Signature::from_bytes(&signature);
    public_key
        .verify(manifest.signing_message().as_bytes(), &signature)
        .map_err(|_| HarnessRegistryError::InvalidSignature)
}

fn decode_signature_bytes<const N: usize>(value: &str) -> Result<[u8; N], HarnessRegistryError> {
    if value.len() == N * 2 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return decode_hex(value);
    }
    let encoded = value.strip_prefix("base64:").unwrap_or(value);
    let decoded = BASE64
        .decode(encoded)
        .map_err(|_| HarnessRegistryError::InvalidSignatureEncoding)?;
    decoded
        .try_into()
        .map_err(|_| HarnessRegistryError::InvalidSignatureEncoding)
}

fn decode_hex<const N: usize>(value: &str) -> Result<[u8; N], HarnessRegistryError> {
    if value.len() != N * 2 {
        return Err(HarnessRegistryError::InvalidSignatureEncoding);
    }
    let mut bytes = [0_u8; N];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_digit(pair[0])?;
        let low = hex_digit(pair[1])?;
        bytes[index] = (high << 4) | low;
    }
    Ok(bytes)
}

fn hex_digit(value: u8) -> Result<u8, HarnessRegistryError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err(HarnessRegistryError::InvalidSignatureEncoding),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD as BASE64;
    use ed25519_dalek::{Signer, SigningKey};
    use pandora_types::{
        DomainRoutingProfile, HarnessId, MetaComposition, PackageCompatibility, PackageDependency,
        PackageKind, PackageManifest, TrustEvidence, TrustLevel, hash_artifact,
    };

    const CURRENT_RUNTIME_REQUIREMENT: &str = concat!("pandora>=", env!("CARGO_PKG_VERSION"));

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
            PackageCompatibility::new(CURRENT_RUNTIME_REQUIREMENT).unwrap(),
            "MIT",
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
            PackageCompatibility::new(CURRENT_RUNTIME_REQUIREMENT).unwrap(),
            "MIT",
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

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn valid_signed_package_is_admitted_after_signature_verification() {
        let artifact = b"signed gene artifact";
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let content_hash = hash_artifact(artifact);
        let unsigned = PackageManifest::new(
            "publisher/gene",
            "1.0.0",
            PackageKind::Gene,
            "publisher",
            content_hash.clone(),
            Vec::new(),
            PackageCompatibility::new(CURRENT_RUNTIME_REQUIREMENT).unwrap(),
            "MIT",
            TrustEvidence::unsigned(),
        )
        .unwrap();
        let signature = signing_key.sign(unsigned.signing_message().as_bytes());
        let package = PackageManifest::new(
            "publisher/gene",
            "1.0.0",
            PackageKind::Gene,
            "publisher",
            content_hash,
            Vec::new(),
            PackageCompatibility::new(CURRENT_RUNTIME_REQUIREMENT).unwrap(),
            "MIT",
            TrustEvidence::new(
                TrustLevel::Verified,
                Some(hex(&signature.to_bytes())),
                Some(hex(&signing_key.verifying_key().to_bytes())),
            )
            .unwrap(),
        )
        .unwrap();
        let mut registry = HarnessRegistry::new();

        assert!(registry.install(&package, &package, artifact).is_ok());
    }

    #[test]
    fn palace_base64_signature_evidence_is_verified_without_reencoding() {
        let artifact = b"signed registry gene artifact";
        let signing_key = SigningKey::from_bytes(&[11_u8; 32]);
        let content_hash = hash_artifact(artifact);
        let unsigned = PackageManifest::new(
            "publisher/gene",
            "1.0.0",
            PackageKind::Gene,
            "publisher",
            content_hash.clone(),
            Vec::new(),
            PackageCompatibility::new(CURRENT_RUNTIME_REQUIREMENT).unwrap(),
            "MIT",
            TrustEvidence::unsigned(),
        )
        .unwrap();
        let signature = signing_key.sign(unsigned.signing_message().as_bytes());
        let encoded_signature = BASE64.encode(signature.to_bytes());
        let encoded_key = format!(
            "base64:{}",
            BASE64.encode(signing_key.verifying_key().to_bytes())
        );
        let package = PackageManifest::new(
            "publisher/gene",
            "1.0.0",
            PackageKind::Gene,
            "publisher",
            content_hash,
            Vec::new(),
            PackageCompatibility::new(CURRENT_RUNTIME_REQUIREMENT).unwrap(),
            "MIT",
            TrustEvidence::new(
                TrustLevel::Verified,
                Some(encoded_signature.clone()),
                Some(encoded_key.clone()),
            )
            .unwrap(),
        )
        .unwrap();
        let mut registry = HarnessRegistry::new();

        let installed = registry.install(&package, &package, artifact).unwrap();

        assert_eq!(
            installed.manifest().trust().signature(),
            Some(encoded_signature.as_str())
        );
        assert_eq!(
            installed.manifest().trust().public_key(),
            Some(encoded_key.as_str())
        );
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
    fn manifest_binding_rejects_dependency_license_and_trust_mismatches() {
        let artifact = b"gene artifact";
        let declared = manifest(
            "publisher/gene",
            "1.0.0",
            PackageKind::Gene,
            Vec::new(),
            artifact,
        );
        let cases = [
            PackageManifest::new(
                "publisher/gene",
                "1.0.0",
                PackageKind::Gene,
                "publisher",
                hash_artifact(artifact),
                vec![PackageDependency::new("workspace.read", "0.1.0", false).unwrap()],
                PackageCompatibility::new(CURRENT_RUNTIME_REQUIREMENT).unwrap(),
                "MIT",
                TrustEvidence::unsigned(),
            )
            .unwrap(),
            PackageManifest::new(
                "publisher/gene",
                "1.0.0",
                PackageKind::Gene,
                "publisher",
                hash_artifact(artifact),
                Vec::new(),
                PackageCompatibility::new(CURRENT_RUNTIME_REQUIREMENT).unwrap(),
                "BSD-3-Clause",
                TrustEvidence::unsigned(),
            )
            .unwrap(),
            PackageManifest::new(
                "publisher/gene",
                "1.0.0",
                PackageKind::Gene,
                "publisher",
                hash_artifact(artifact),
                Vec::new(),
                PackageCompatibility::new(CURRENT_RUNTIME_REQUIREMENT).unwrap(),
                "MIT",
                TrustEvidence::new(
                    TrustLevel::Verified,
                    Some("signature".to_owned()),
                    Some("public-key".to_owned()),
                )
                .unwrap(),
            )
            .unwrap(),
        ];

        for embedded in cases {
            let mut registry = HarnessRegistry::new();
            assert_eq!(
                registry.install(&declared, &embedded, artifact),
                Err(HarnessRegistryError::ManifestMismatch)
            );
            assert!(registry.list().is_empty());
        }
    }

    #[test]
    fn invalid_signed_package_is_rejected() {
        let artifact = b"gene artifact";
        let package = PackageManifest::new(
            "publisher/gene",
            "1.0.0",
            PackageKind::Gene,
            "publisher",
            hash_artifact(artifact),
            Vec::new(),
            PackageCompatibility::new(CURRENT_RUNTIME_REQUIREMENT).unwrap(),
            "MIT",
            TrustEvidence::new(
                TrustLevel::Verified,
                Some("signature".to_owned()),
                Some("public-key".to_owned()),
            )
            .unwrap(),
        )
        .unwrap();
        let mut registry = HarnessRegistry::new();

        assert_eq!(
            registry.install(&package, &package, artifact),
            Err(HarnessRegistryError::InvalidSignatureEncoding)
        );
        assert!(registry.list().is_empty());
    }

    #[test]
    fn signed_package_requires_complete_signature_evidence() {
        let artifact = b"gene artifact";
        let package = PackageManifest::new(
            "publisher/gene",
            "1.0.0",
            PackageKind::Gene,
            "publisher",
            hash_artifact(artifact),
            Vec::new(),
            PackageCompatibility::new(CURRENT_RUNTIME_REQUIREMENT).unwrap(),
            "MIT",
            TrustEvidence::new(TrustLevel::Verified, None, None).unwrap(),
        )
        .unwrap();
        let mut registry = HarnessRegistry::new();

        assert_eq!(
            registry.install(&package, &package, artifact),
            Err(HarnessRegistryError::SignatureRequired)
        );
        assert!(registry.list().is_empty());
    }

    #[test]
    fn signed_package_rejects_a_signature_for_different_content() {
        let artifact = b"gene artifact";
        let signing_key = SigningKey::from_bytes(&[9_u8; 32]);
        let package = PackageManifest::new(
            "publisher/gene",
            "1.0.0",
            PackageKind::Gene,
            "publisher",
            hash_artifact(artifact),
            Vec::new(),
            PackageCompatibility::new(CURRENT_RUNTIME_REQUIREMENT).unwrap(),
            "MIT",
            TrustEvidence::new(
                TrustLevel::Verified,
                Some(hex(&[0_u8; 64])),
                Some(hex(&signing_key.verifying_key().to_bytes())),
            )
            .unwrap(),
        )
        .unwrap();
        let mut registry = HarnessRegistry::new();

        assert_eq!(
            registry.install(&package, &package, artifact),
            Err(HarnessRegistryError::InvalidSignature)
        );
        assert!(registry.list().is_empty());
    }

    #[test]
    fn official_package_trust_requires_a_publisher_root() {
        let artifact = b"gene artifact";
        let package = PackageManifest::new(
            "publisher/gene",
            "1.0.0",
            PackageKind::Gene,
            "publisher",
            hash_artifact(artifact),
            Vec::new(),
            PackageCompatibility::new(CURRENT_RUNTIME_REQUIREMENT).unwrap(),
            "MIT",
            TrustEvidence::new(TrustLevel::Official, None, None).unwrap(),
        )
        .unwrap();
        let mut registry = HarnessRegistry::new();

        assert_eq!(
            registry.install(&package, &package, artifact),
            Err(HarnessRegistryError::OfficialTrustUnsupported)
        );
        assert!(registry.list().is_empty());
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
    fn incompatible_runtime_is_rejected_before_admission() {
        let artifact = b"gene artifact";
        let package = PackageManifest::new(
            "publisher/gene",
            "1.0.0",
            PackageKind::Gene,
            "publisher",
            hash_artifact(artifact),
            Vec::new(),
            PackageCompatibility::new("pandora>=3.0.0").unwrap(),
            "MIT",
            TrustEvidence::unsigned(),
        )
        .unwrap();
        let mut registry = HarnessRegistry::new();

        assert_eq!(
            registry.install(&package, &package, artifact),
            Err(HarnessRegistryError::IncompatibleRuntime {
                required: "pandora>=3.0.0".to_owned(),
                actual: env!("CARGO_PKG_VERSION").to_owned(),
            })
        );
        assert!(registry.list().is_empty());
    }

    #[test]
    fn unsupported_remote_kinds_fail_closed() {
        for kind in [
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
    fn custom_meta_profile_rejects_an_unknown_domain() {
        let artifact = b"meta profile";
        let package = meta_manifest("publisher/coordination", artifact, &["publisher/domain"]);
        let mut registry = HarnessRegistry::new();

        assert_eq!(
            registry.install(&package, &package, artifact),
            Err(HarnessRegistryError::MetaDomainMissing {
                id: "publisher/domain".to_owned(),
            })
        );
    }

    #[test]
    fn custom_meta_profile_rejects_a_non_domain_member() {
        let gene_artifact = b"gene artifact";
        let gene = manifest(
            "publisher/gene",
            "1.0.0",
            PackageKind::Gene,
            vec![],
            gene_artifact,
        );
        let meta_artifact = b"meta profile";
        let meta = meta_manifest("publisher/coordination", meta_artifact, &["publisher/gene"]);
        let mut registry = HarnessRegistry::new();
        registry.install(&gene, &gene, gene_artifact).unwrap();

        assert_eq!(
            registry.install(&meta, &meta, meta_artifact),
            Err(HarnessRegistryError::MetaDomainNotDomain {
                id: "publisher/gene".to_owned(),
                kind: PackageKind::Gene,
            })
        );
    }

    #[test]
    fn custom_meta_profile_accepts_one_admitted_domain() {
        let gene_artifact = b"gene artifact";
        let gene = manifest(
            "publisher/gene",
            "1.0.0",
            PackageKind::Gene,
            vec![],
            gene_artifact,
        );
        let domain_artifact = b"domain profile";
        let domain = manifest(
            "publisher/domain",
            "1.0.0",
            PackageKind::DomainHarness,
            vec![PackageDependency::new("publisher/gene", "1.0.0", false).unwrap()],
            domain_artifact,
        );
        let meta_artifact = b"meta profile";
        let meta = meta_manifest(
            "publisher/coordination",
            meta_artifact,
            &["publisher/domain"],
        );
        let mut registry = HarnessRegistry::new();
        registry.install(&gene, &gene, gene_artifact).unwrap();
        registry.install(&domain, &domain, domain_artifact).unwrap();

        assert_eq!(
            registry
                .install(&meta, &meta, meta_artifact)
                .unwrap()
                .state(),
            PackageState::Admitted
        );
    }

    #[test]
    fn custom_meta_profile_allows_side_by_side_domain_versions_for_lifecycle_binding() {
        let first_artifact = b"first domain profile";
        let first = manifest(
            "publisher/domain",
            "1.0.0",
            PackageKind::DomainHarness,
            vec![PackageDependency::new("workspace.read", "0.1.0", false).unwrap()],
            first_artifact,
        );
        let second_artifact = b"second domain profile";
        let second = manifest(
            "publisher/domain",
            "2.0.0",
            PackageKind::DomainHarness,
            vec![PackageDependency::new("workspace.read", "0.1.0", false).unwrap()],
            second_artifact,
        );
        let meta_artifact = b"meta profile";
        let meta = meta_manifest(
            "publisher/coordination",
            meta_artifact,
            &["publisher/domain"],
        );
        let mut registry = HarnessRegistry::new();
        registry.install(&first, &first, first_artifact).unwrap();
        registry.install(&second, &second, second_artifact).unwrap();

        let record = registry.install(&meta, &meta, meta_artifact).unwrap();
        assert_eq!(record.state(), PackageState::Admitted);
        assert!(!record.grants_runtime_authority());
    }

    #[test]
    fn unsigned_packages_cannot_replace_optional_built_in_harnesses() {
        let domain_artifact = b"domain profile";
        let domain = manifest(
            "coding-domain",
            "1.0.0",
            PackageKind::DomainHarness,
            vec![PackageDependency::new("workspace.read", "0.1.0", false).unwrap()],
            domain_artifact,
        );
        let meta_artifact = b"meta profile";
        let meta = meta_manifest("coordination-meta", meta_artifact, &["coding-domain"]);
        let mut registry = HarnessRegistry::new();

        assert_eq!(
            registry.install(&domain, &domain, domain_artifact),
            Err(
                HarnessRegistryError::BuiltInReplacementRequiresVerifiedSignature {
                    id: "coding-domain".to_owned(),
                }
            )
        );
        assert_eq!(
            registry.install(&meta, &meta, meta_artifact),
            Err(
                HarnessRegistryError::BuiltInReplacementRequiresVerifiedSignature {
                    id: "coordination-meta".to_owned(),
                }
            )
        );
    }

    #[test]
    fn signed_optional_domain_replacement_is_admitted_with_bound_routing() {
        let artifact = b"signed coding replacement";
        let signing_key = SigningKey::from_bytes(&[17_u8; 32]);
        let dependencies = vec![PackageDependency::new("workspace.read", "0.1.0", false).unwrap()];
        let routing = DomainRoutingProfile::new(vec!["firmware development".to_owned()]).unwrap();
        let unsigned = PackageManifest::new(
            "coding-domain",
            "2.0.0",
            PackageKind::DomainHarness,
            "publisher",
            hash_artifact(artifact),
            dependencies.clone(),
            PackageCompatibility::new(CURRENT_RUNTIME_REQUIREMENT).unwrap(),
            "MIT",
            TrustEvidence::unsigned(),
        )
        .unwrap()
        .with_domain_routing(routing.clone())
        .unwrap();
        let signature = signing_key.sign(unsigned.signing_message().as_bytes());
        let package = PackageManifest::new(
            "coding-domain",
            "2.0.0",
            PackageKind::DomainHarness,
            "publisher",
            hash_artifact(artifact),
            dependencies,
            PackageCompatibility::new(CURRENT_RUNTIME_REQUIREMENT).unwrap(),
            "MIT",
            TrustEvidence::new(
                TrustLevel::Verified,
                Some(hex(&signature.to_bytes())),
                Some(hex(&signing_key.verifying_key().to_bytes())),
            )
            .unwrap(),
        )
        .unwrap()
        .with_domain_routing(routing)
        .unwrap();
        let mut registry = HarnessRegistry::new();

        let record = registry.install(&package, &package, artifact).unwrap();

        assert_eq!(record.state(), PackageState::Admitted);
        assert!(!record.grants_runtime_authority());
    }

    #[test]
    fn constitutional_source_harness_remains_immutable() {
        let artifact = b"source replacement";
        let package = manifest(
            "core-source",
            "2.0.0",
            PackageKind::SourceHarness,
            Vec::new(),
            artifact,
        );
        let mut registry = HarnessRegistry::new();

        assert_eq!(
            registry.install(&package, &package, artifact),
            Err(HarnessRegistryError::ReservedHarnessId {
                id: "core-source".to_owned(),
            })
        );
    }

    #[test]
    fn domain_profile_is_admitted_after_its_gene_dependencies_are_installed() {
        let gene_artifact = b"gene artifact";
        let gene = manifest(
            "publisher/gene",
            "1.0.0",
            PackageKind::Gene,
            vec![],
            gene_artifact,
        );
        let profile_artifact = b"domain profile";
        let profile = manifest(
            "publisher/domain",
            "1.0.0",
            PackageKind::DomainHarness,
            vec![PackageDependency::new("publisher/gene", "1.0.0", false).unwrap()],
            profile_artifact,
        );
        let mut registry = HarnessRegistry::new();
        registry.install(&gene, &gene, gene_artifact).unwrap();

        let record = registry
            .install(&profile, &profile, profile_artifact)
            .unwrap();

        assert_eq!(record.state(), PackageState::Admitted);
        assert!(!record.grants_runtime_authority());
        assert_eq!(record.manifest().kind(), PackageKind::DomainHarness);
    }

    #[test]
    fn domain_profile_can_reference_a_built_in_gene_without_a_package_record() {
        let artifact = b"domain profile";
        let profile = manifest(
            "publisher/domain",
            "1.0.0",
            PackageKind::DomainHarness,
            vec![PackageDependency::new("workspace.read", "0.1.0", false).unwrap()],
            artifact,
        );
        let mut registry = HarnessRegistry::new();

        let record = registry.install(&profile, &profile, artifact).unwrap();

        assert_eq!(record.state(), PackageState::Admitted);
        assert!(!record.grants_runtime_authority());
    }

    #[test]
    fn domain_profile_requires_a_required_gene_dependency() {
        let artifact = b"domain profile";
        let profile = manifest(
            "publisher/domain",
            "1.0.0",
            PackageKind::DomainHarness,
            vec![],
            artifact,
        );
        let mut registry = HarnessRegistry::new();

        assert_eq!(
            registry.install(&profile, &profile, artifact),
            Err(HarnessRegistryError::DomainHarnessRequiresGene)
        );
        assert!(registry.list().is_empty());
    }

    #[test]
    fn domain_profile_rejects_a_non_gene_dependency() {
        let meta_artifact = b"meta profile";
        let meta = meta_manifest("publisher/meta", meta_artifact, &["coding-domain"]);
        let profile_artifact = b"domain profile";
        let profile = manifest(
            "publisher/domain",
            "1.0.0",
            PackageKind::DomainHarness,
            vec![PackageDependency::new("publisher/meta", "1.0.0", false).unwrap()],
            profile_artifact,
        );
        let mut registry = HarnessRegistry::new();
        registry.install(&meta, &meta, meta_artifact).unwrap();

        assert_eq!(
            registry.install(&profile, &profile, profile_artifact),
            Err(HarnessRegistryError::DomainDependencyNotGene {
                id: "publisher/meta".to_owned(),
                version: "1.0.0".to_owned(),
                kind: PackageKind::MetaHarness,
            })
        );
        assert_eq!(registry.list().len(), 1);
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
