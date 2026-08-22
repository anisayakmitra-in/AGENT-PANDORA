use crate::harness::{HarnessKind, HarnessManifest, MetaComposition};
use crate::ids::{IdError, PackageId};
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

const MAX_PACKAGE_TEXT_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageKind {
    Gene,
    DomainHarness,
    MetaHarness,
    SourceHarness,
    Package,
    Provider,
    Skill,
}

impl PackageKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Gene => "gene",
            Self::DomainHarness => "domain_harness",
            Self::MetaHarness => "meta_harness",
            Self::SourceHarness => "source_harness",
            Self::Package => "package",
            Self::Provider => "provider",
            Self::Skill => "skill",
        }
    }

    pub fn parse(value: &str) -> Result<Self, PackageManifestError> {
        match value {
            "gene" => Ok(Self::Gene),
            "domain_harness" => Ok(Self::DomainHarness),
            "meta_harness" => Ok(Self::MetaHarness),
            "source_harness" => Ok(Self::SourceHarness),
            "package" => Ok(Self::Package),
            "provider" => Ok(Self::Provider),
            "skill" => Ok(Self::Skill),
            _ => Err(PackageManifestError::InvalidKind),
        }
    }
}

impl From<HarnessKind> for PackageKind {
    fn from(kind: HarnessKind) -> Self {
        match kind {
            HarnessKind::Source => Self::SourceHarness,
            HarnessKind::Meta => Self::MetaHarness,
            HarnessKind::Domain => Self::DomainHarness,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PackageManifestError {
    InvalidId(IdError),
    EmptyField(&'static str),
    FieldTooLong(&'static str),
    InvalidVersion,
    InvalidRuntimeCompatibility,
    InvalidPackageId,
    InvalidHash,
    InvalidKind,
    ControlCharacter(&'static str),
    MissingMetaComposition,
    UnexpectedMetaComposition,
    InvalidMetaComposition,
}

impl fmt::Display for PackageManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId(error) => error.fmt(formatter),
            Self::EmptyField(field) => write!(formatter, "{field} cannot be empty"),
            Self::FieldTooLong(field) => write!(formatter, "{field} is too long"),
            Self::InvalidVersion => formatter.write_str("version must be valid SemVer"),
            Self::InvalidRuntimeCompatibility => formatter.write_str(
                "runtime compatibility must be a non-wildcard Pandora SemVer requirement",
            ),
            Self::InvalidPackageId => {
                formatter.write_str("package id contains an invalid path component")
            }
            Self::InvalidHash => formatter.write_str("package content hash is not a sha256 digest"),
            Self::InvalidKind => formatter.write_str("package kind is not recognized"),
            Self::ControlCharacter(field) => {
                write!(formatter, "{field} contains a control character")
            }
            Self::MissingMetaComposition => {
                formatter.write_str("Meta Harness packages require a composition")
            }
            Self::UnexpectedMetaComposition => {
                formatter.write_str("only Meta Harness packages may declare a composition")
            }
            Self::InvalidMetaComposition => {
                formatter.write_str("Meta Harness composition is invalid")
            }
        }
    }
}

impl std::error::Error for PackageManifestError {}

impl From<IdError> for PackageManifestError {
    fn from(error: IdError) -> Self {
        Self::InvalidId(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PackageDependency {
    id: PackageId,
    version: String,
    optional: bool,
}

impl PackageDependency {
    pub fn new(
        id: impl Into<String>,
        version: impl Into<String>,
        optional: bool,
    ) -> Result<Self, PackageManifestError> {
        Ok(Self {
            id: package_id(id)?,
            version: validate_version(version.into())?,
            optional,
        })
    }

    pub fn id(&self) -> &PackageId {
        &self.id
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn optional(&self) -> bool {
        self.optional
    }

    fn validate(&self) -> Result<(), PackageManifestError> {
        package_id(self.id.as_str())?;
        if validate_version(self.version.clone())? != self.version {
            return Err(PackageManifestError::InvalidVersion);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PackageCompatibility {
    runtime: String,
}

impl PackageCompatibility {
    pub fn new(runtime: impl Into<String>) -> Result<Self, PackageManifestError> {
        let runtime = validate_text("runtime compatibility", runtime.into())?;
        runtime_requirement(&runtime)?;
        Ok(Self { runtime })
    }

    pub fn runtime(&self) -> &str {
        &self.runtime
    }

    pub fn matches_runtime(&self, runtime_version: &str) -> Result<bool, PackageManifestError> {
        let version =
            Version::parse(runtime_version).map_err(|_| PackageManifestError::InvalidVersion)?;
        Ok(runtime_requirement(&self.runtime)?.matches(&version))
    }

    fn validate(&self) -> Result<(), PackageManifestError> {
        let runtime = validate_text("runtime compatibility", self.runtime.clone())?;
        if runtime != self.runtime {
            return Err(PackageManifestError::InvalidRuntimeCompatibility);
        }
        runtime_requirement(&runtime)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    Unverified,
    Verified,
    Official,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TrustEvidence {
    level: TrustLevel,
    signature: Option<String>,
    public_key: Option<String>,
}

impl TrustEvidence {
    pub fn new(
        level: TrustLevel,
        signature: Option<String>,
        public_key: Option<String>,
    ) -> Result<Self, PackageManifestError> {
        let evidence = Self {
            level,
            signature,
            public_key,
        };
        evidence.validate()?;
        Ok(evidence)
    }

    fn validate(&self) -> Result<(), PackageManifestError> {
        for (field, value) in [
            ("signature", self.signature.as_deref()),
            ("public_key", self.public_key.as_deref()),
        ] {
            if value.is_some_and(|value| value.chars().any(char::is_control)) {
                return Err(PackageManifestError::ControlCharacter(field));
            }
        }
        Ok(())
    }

    pub fn unsigned() -> Self {
        Self {
            level: TrustLevel::Unverified,
            signature: None,
            public_key: None,
        }
    }

    pub fn level(&self) -> TrustLevel {
        self.level
    }

    pub fn signature(&self) -> Option<&str> {
        self.signature.as_deref()
    }

    pub fn public_key(&self) -> Option<&str> {
        self.public_key.as_deref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PackageManifest {
    id: PackageId,
    version: String,
    kind: PackageKind,
    publisher: String,
    content_hash: String,
    dependencies: Vec<PackageDependency>,
    compatibility: PackageCompatibility,
    license: String,
    trust: TrustEvidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    meta_composition: Option<MetaComposition>,
}

impl PackageManifest {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        version: impl Into<String>,
        kind: PackageKind,
        publisher: impl Into<String>,
        content_hash: impl Into<String>,
        dependencies: Vec<PackageDependency>,
        compatibility: PackageCompatibility,
        license: impl Into<String>,
        trust: TrustEvidence,
    ) -> Result<Self, PackageManifestError> {
        Self::build(
            package_id(id)?,
            validate_version(version.into())?,
            kind,
            validate_text("publisher", publisher.into())?,
            validate_hash(content_hash.into())?,
            dependencies,
            compatibility,
            validate_text("license", license.into())?,
            trust,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_meta(
        id: impl Into<String>,
        version: impl Into<String>,
        publisher: impl Into<String>,
        content_hash: impl Into<String>,
        dependencies: Vec<PackageDependency>,
        compatibility: PackageCompatibility,
        license: impl Into<String>,
        trust: TrustEvidence,
        composition: MetaComposition,
    ) -> Result<Self, PackageManifestError> {
        Self::build(
            package_id(id)?,
            validate_version(version.into())?,
            PackageKind::MetaHarness,
            validate_text("publisher", publisher.into())?,
            validate_hash(content_hash.into())?,
            dependencies,
            compatibility,
            validate_text("license", license.into())?,
            trust,
            Some(composition),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        id: PackageId,
        version: String,
        kind: PackageKind,
        publisher: String,
        content_hash: String,
        dependencies: Vec<PackageDependency>,
        compatibility: PackageCompatibility,
        license: String,
        trust: TrustEvidence,
        meta_composition: Option<MetaComposition>,
    ) -> Result<Self, PackageManifestError> {
        let package = Self {
            id,
            version,
            kind,
            publisher,
            content_hash,
            dependencies,
            compatibility,
            license,
            trust,
            meta_composition,
        };
        package.validate()?;
        Ok(package)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_harness(
        manifest: &HarnessManifest,
        publisher: impl Into<String>,
        content_hash: impl Into<String>,
        dependencies: Vec<PackageDependency>,
        compatibility: PackageCompatibility,
        license: impl Into<String>,
        trust: TrustEvidence,
    ) -> Result<Self, PackageManifestError> {
        if let Some(composition) = manifest.meta_composition().cloned() {
            return Self::new_meta(
                manifest.id().as_str(),
                manifest.version(),
                publisher,
                content_hash,
                dependencies,
                compatibility,
                license,
                trust,
                composition,
            );
        }
        Self::new(
            manifest.id().as_str(),
            manifest.version(),
            manifest.kind().into(),
            publisher,
            content_hash,
            dependencies,
            compatibility,
            license,
            trust,
        )
    }

    pub fn id(&self) -> &PackageId {
        &self.id
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn kind(&self) -> PackageKind {
        self.kind
    }

    pub fn publisher(&self) -> &str {
        &self.publisher
    }

    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    pub fn dependencies(&self) -> &[PackageDependency] {
        &self.dependencies
    }

    pub fn compatibility(&self) -> &PackageCompatibility {
        &self.compatibility
    }

    pub fn license(&self) -> &str {
        &self.license
    }

    pub fn trust(&self) -> &TrustEvidence {
        &self.trust
    }

    pub fn meta_composition(&self) -> Option<&MetaComposition> {
        self.meta_composition.as_ref()
    }

    pub fn signing_message(&self) -> String {
        format!(
            "{}:{}:{}:{}",
            self.id.as_str(),
            self.version,
            self.publisher,
            self.content_hash
        )
    }

    pub fn validate(&self) -> Result<(), PackageManifestError> {
        package_id(self.id.as_str())?;
        if validate_version(self.version.clone())? != self.version {
            return Err(PackageManifestError::InvalidVersion);
        }
        validate_text("publisher", self.publisher.clone())?;
        validate_hash(self.content_hash.clone())?;
        for dependency in &self.dependencies {
            dependency.validate()?;
        }
        self.compatibility.validate()?;
        validate_text("license", self.license.clone())?;
        self.trust.validate()?;
        match (self.kind, self.meta_composition.as_ref()) {
            (PackageKind::MetaHarness, None) => Err(PackageManifestError::MissingMetaComposition),
            (PackageKind::MetaHarness, Some(composition)) => composition
                .validate()
                .map_err(|_| PackageManifestError::InvalidMetaComposition),
            (_, Some(_)) => Err(PackageManifestError::UnexpectedMetaComposition),
            _ => Ok(()),
        }
    }

    pub fn identity_matches(&self, other: &Self) -> bool {
        self.id == other.id && self.version == other.version && self.kind == other.kind
    }
}

pub const PACKAGE_LOCK_FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PackageLockError {
    InvalidFormat,
    InvalidManifest,
    DuplicateIdentity,
    NonCanonicalOrder,
}

impl fmt::Display for PackageLockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFormat => formatter.write_str("package lock format is not supported"),
            Self::InvalidManifest => {
                formatter.write_str("package lock contains an invalid manifest")
            }
            Self::DuplicateIdentity => {
                formatter.write_str("package lock contains a duplicate identity")
            }
            Self::NonCanonicalOrder => {
                formatter.write_str("package lock is not canonically ordered")
            }
        }
    }
}

impl std::error::Error for PackageLockError {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PackageLock {
    format_version: u32,
    packages: Vec<PackageManifest>,
}

impl PackageLock {
    pub fn new(mut packages: Vec<PackageManifest>) -> Result<Self, PackageLockError> {
        for manifest in &packages {
            manifest
                .validate()
                .map_err(|_| PackageLockError::InvalidManifest)?;
        }
        packages.sort_by(compare_package_identity);
        let lock = Self {
            format_version: PACKAGE_LOCK_FORMAT_VERSION,
            packages,
        };
        lock.validate()?;
        Ok(lock)
    }

    pub const fn format_version(&self) -> u32 {
        self.format_version
    }

    pub fn packages(&self) -> &[PackageManifest] {
        &self.packages
    }

    pub fn validate(&self) -> Result<(), PackageLockError> {
        if self.format_version != PACKAGE_LOCK_FORMAT_VERSION {
            return Err(PackageLockError::InvalidFormat);
        }
        for manifest in &self.packages {
            manifest
                .validate()
                .map_err(|_| PackageLockError::InvalidManifest)?;
        }
        for pair in self.packages.windows(2) {
            match compare_package_identity(&pair[0], &pair[1]) {
                std::cmp::Ordering::Less => {}
                std::cmp::Ordering::Equal => return Err(PackageLockError::DuplicateIdentity),
                std::cmp::Ordering::Greater => return Err(PackageLockError::NonCanonicalOrder),
            }
        }
        Ok(())
    }
}

fn compare_package_identity(left: &PackageManifest, right: &PackageManifest) -> std::cmp::Ordering {
    left.id()
        .as_str()
        .cmp(right.id().as_str())
        .then_with(|| left.version().cmp(right.version()))
}

pub fn hash_artifact(artifact: &[u8]) -> String {
    let digest = Sha256::digest(artifact);
    format!("sha256:{digest:x}")
}

fn package_id(value: impl Into<String>) -> Result<PackageId, PackageManifestError> {
    let id = PackageId::new(value)?;
    if id.as_str().split('/').any(|part| {
        part.is_empty()
            || part == "."
            || part == ".."
            || part.contains('\\')
            || part.chars().any(char::is_control)
    }) {
        return Err(PackageManifestError::InvalidPackageId);
    }
    Ok(id)
}

fn validate_text(field: &'static str, value: String) -> Result<String, PackageManifestError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(PackageManifestError::EmptyField(field));
    }
    if value.len() > MAX_PACKAGE_TEXT_BYTES {
        return Err(PackageManifestError::FieldTooLong(field));
    }
    if value.chars().any(char::is_control) {
        return Err(PackageManifestError::ControlCharacter(field));
    }
    Ok(trimmed.to_owned())
}

fn validate_version(value: String) -> Result<String, PackageManifestError> {
    let value = validate_text("version", value)?;
    Version::parse(&value)
        .map(|_| value)
        .map_err(|_| PackageManifestError::InvalidVersion)
}

fn validate_hash(value: String) -> Result<String, PackageManifestError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(PackageManifestError::InvalidHash);
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(PackageManifestError::InvalidHash);
    }
    Ok(value)
}

fn runtime_requirement(value: &str) -> Result<VersionReq, PackageManifestError> {
    let Some(requirement) = value.strip_prefix("pandora") else {
        return Err(PackageManifestError::InvalidRuntimeCompatibility);
    };
    let requirement = requirement.trim();
    if requirement.is_empty() || requirement == "*" {
        return Err(PackageManifestError::InvalidRuntimeCompatibility);
    }
    VersionReq::parse(requirement).map_err(|_| PackageManifestError::InvalidRuntimeCompatibility)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HarnessId;
    use crate::harness::{HarnessKind, HarnessManifest, MetaComposition};

    #[test]
    fn package_kinds_use_the_closed_external_vocabulary() {
        assert_eq!(
            PackageKind::parse("domain_harness").unwrap(),
            PackageKind::DomainHarness
        );
        assert_eq!(PackageKind::DomainHarness.as_str(), "domain_harness");
        assert_eq!(
            PackageKind::parse("domain-harness"),
            Err(PackageManifestError::InvalidKind)
        );
    }

    #[test]
    fn harness_adapter_preserves_canonical_identity_and_kind() {
        let harness = HarnessManifest::new(
            "coding-domain",
            "0.1.0",
            "Coding Domain",
            HarnessKind::Domain,
            None,
            Vec::new(),
        )
        .unwrap();
        let package = PackageManifest::from_harness(
            &harness,
            "pandora",
            hash_artifact(b"coding"),
            Vec::new(),
            PackageCompatibility::new("pandora>=2.0.0").unwrap(),
            "Apache-2.0",
            TrustEvidence::unsigned(),
        )
        .unwrap();

        assert_eq!(package.id().as_str(), "coding-domain");
        assert_eq!(package.version(), "0.1.0");
        assert_eq!(package.kind(), PackageKind::DomainHarness);
    }

    #[test]
    fn meta_package_requires_and_round_trips_composition() {
        let composition =
            MetaComposition::new(vec![HarnessId::new("coding-domain").unwrap()], 8).unwrap();
        let package = PackageManifest::new_meta(
            "publisher/coordination",
            "1.0.0",
            "publisher",
            hash_artifact(b"meta profile"),
            Vec::new(),
            PackageCompatibility::new("pandora>=2.0.0").unwrap(),
            "Apache-2.0",
            TrustEvidence::unsigned(),
            composition.clone(),
        )
        .unwrap();

        let encoded = serde_json::to_vec(&package).unwrap();
        let decoded: PackageManifest = serde_json::from_slice(&encoded).unwrap();

        assert_eq!(decoded.kind(), PackageKind::MetaHarness);
        assert_eq!(decoded.meta_composition(), Some(&composition));
    }

    #[test]
    fn ordinary_constructor_rejects_meta_without_composition() {
        assert_eq!(
            PackageManifest::new(
                "publisher/coordination",
                "1.0.0",
                PackageKind::MetaHarness,
                "publisher",
                hash_artifact(b"meta profile"),
                Vec::new(),
                PackageCompatibility::new("pandora>=2.0.0").unwrap(),
                "Apache-2.0",
                TrustEvidence::unsigned(),
            ),
            Err(PackageManifestError::MissingMetaComposition)
        );
    }

    #[test]
    fn package_and_dependency_versions_require_semver() {
        assert_eq!(
            PackageDependency::new("publisher/gene", "1.0", false),
            Err(PackageManifestError::InvalidVersion)
        );

        let artifact = b"domain";
        assert_eq!(
            PackageManifest::new(
                "publisher/domain",
                "release-1",
                PackageKind::DomainHarness,
                "publisher",
                hash_artifact(artifact),
                vec![],
                PackageCompatibility::new("pandora>=2.0.0").unwrap(),
                "Apache-2.0",
                TrustEvidence::unsigned(),
            ),
            Err(PackageManifestError::InvalidVersion)
        );
    }

    #[test]
    fn runtime_compatibility_requires_a_pandora_semver_requirement() {
        assert_eq!(
            PackageCompatibility::new("other>=2.0.0"),
            Err(PackageManifestError::InvalidRuntimeCompatibility)
        );
        assert_eq!(
            PackageCompatibility::new("pandora*"),
            Err(PackageManifestError::InvalidRuntimeCompatibility)
        );

        let compatibility = PackageCompatibility::new("pandora>=2.0.0-alpha.0, <3.0.0").unwrap();
        assert!(compatibility.matches_runtime("2.0.0-alpha.6").unwrap());
        assert!(!compatibility.matches_runtime("3.0.0").unwrap());
        assert!(
            !PackageCompatibility::new("pandora>=2.0.0")
                .unwrap()
                .matches_runtime("2.0.0-alpha.6")
                .unwrap()
        );
    }

    #[test]
    fn package_versions_preserve_prerelease_and_build_metadata() {
        let artifact = b"gene";
        let manifest = PackageManifest::new(
            "publisher/gene",
            "1.0.0-beta.1+build.5",
            PackageKind::Gene,
            "publisher",
            hash_artifact(artifact),
            vec![PackageDependency::new("publisher/base", "2.0.0-rc.2+build.9", false).unwrap()],
            PackageCompatibility::new("pandora>=2.0.0").unwrap(),
            "Apache-2.0",
            TrustEvidence::unsigned(),
        )
        .unwrap();

        assert_eq!(manifest.version(), "1.0.0-beta.1+build.5");
        assert_eq!(manifest.dependencies()[0].version(), "2.0.0-rc.2+build.9");
    }

    #[test]
    fn deserialized_manifest_cannot_bypass_constructor_validation() {
        let artifact = b"gene";
        let manifest = PackageManifest::new(
            "publisher/gene",
            "1.0.0",
            PackageKind::Gene,
            "publisher",
            hash_artifact(artifact),
            Vec::new(),
            PackageCompatibility::new("pandora>=2.0.0").unwrap(),
            "Apache-2.0",
            TrustEvidence::unsigned(),
        )
        .unwrap();

        let mut invalid_version = serde_json::to_value(&manifest).unwrap();
        invalid_version["version"] = serde_json::Value::String("release-1".to_owned());
        let invalid_version: PackageManifest = serde_json::from_value(invalid_version).unwrap();
        assert_eq!(
            invalid_version.validate(),
            Err(PackageManifestError::InvalidVersion)
        );

        let mut invalid_dependency = serde_json::to_value(&manifest).unwrap();
        invalid_dependency["dependencies"] = serde_json::json!([{
            "id": "publisher/dependency",
            "version": "1.0",
            "optional": false,
        }]);
        let invalid_dependency: PackageManifest =
            serde_json::from_value(invalid_dependency).unwrap();
        assert_eq!(
            invalid_dependency.validate(),
            Err(PackageManifestError::InvalidVersion)
        );

        let mut invalid_compatibility = serde_json::to_value(&manifest).unwrap();
        invalid_compatibility["compatibility"]["runtime"] =
            serde_json::Value::String("\u{0000}".to_owned());
        let invalid_compatibility: PackageManifest =
            serde_json::from_value(invalid_compatibility).unwrap();
        assert_eq!(
            invalid_compatibility.validate(),
            Err(PackageManifestError::ControlCharacter(
                "runtime compatibility"
            ))
        );
    }

    #[test]
    fn package_lock_orders_exact_manifests_and_preserves_evidence() {
        let alpha_artifact = b"alpha";
        let alpha = PackageManifest::new(
            "publisher/alpha",
            "1.0.0-beta.1+build.5",
            PackageKind::Gene,
            "publisher",
            hash_artifact(alpha_artifact),
            Vec::new(),
            PackageCompatibility::new("pandora>=2.0.0").unwrap(),
            "Apache-2.0",
            TrustEvidence::unsigned(),
        )
        .unwrap();
        let zeta_artifact = b"zeta";
        let zeta = PackageManifest::new(
            "publisher/zeta",
            "2.0.0",
            PackageKind::Gene,
            "publisher",
            hash_artifact(zeta_artifact),
            vec![PackageDependency::new("publisher/alpha", "1.0.0-beta.1+build.5", false).unwrap()],
            PackageCompatibility::new("pandora>=2.0.0").unwrap(),
            "MIT",
            TrustEvidence::unsigned(),
        )
        .unwrap();

        let lock = PackageLock::new(vec![zeta, alpha.clone()]).unwrap();

        assert_eq!(lock.format_version(), PACKAGE_LOCK_FORMAT_VERSION);
        assert_eq!(lock.packages().len(), 2);
        assert_eq!(lock.packages()[0], alpha);
        assert_eq!(lock.packages()[1].id().as_str(), "publisher/zeta");
        assert_eq!(
            lock.packages()[1].dependencies()[0].version(),
            "1.0.0-beta.1+build.5"
        );
        assert!(lock.validate().is_ok());
    }

    #[test]
    fn deserialized_package_lock_rejects_noncanonical_and_duplicate_records() {
        let artifact = b"gene";
        let manifest = PackageManifest::new(
            "publisher/gene",
            "1.0.0",
            PackageKind::Gene,
            "publisher",
            hash_artifact(artifact),
            Vec::new(),
            PackageCompatibility::new("pandora>=2.0.0").unwrap(),
            "Apache-2.0",
            TrustEvidence::unsigned(),
        )
        .unwrap();
        let lock = PackageLock::new(vec![manifest]).unwrap();
        let mut encoded = serde_json::to_value(&lock).unwrap();
        encoded["packages"] = serde_json::json!([
            encoded["packages"][0].clone(),
            encoded["packages"][0].clone()
        ]);
        let duplicate: PackageLock = serde_json::from_value(encoded).unwrap();

        assert_eq!(
            duplicate.validate(),
            Err(PackageLockError::DuplicateIdentity)
        );
    }
}
