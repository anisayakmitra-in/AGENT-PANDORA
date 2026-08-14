use crate::harness::{HarnessKind, HarnessManifest};
use crate::ids::{IdError, PackageId};
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
    InvalidPackageId,
    InvalidHash,
    InvalidKind,
    ControlCharacter(&'static str),
}

impl fmt::Display for PackageManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId(error) => error.fmt(formatter),
            Self::EmptyField(field) => write!(formatter, "{field} cannot be empty"),
            Self::FieldTooLong(field) => write!(formatter, "{field} is too long"),
            Self::InvalidPackageId => {
                formatter.write_str("package id contains an invalid path component")
            }
            Self::InvalidHash => formatter.write_str("package content hash is not a sha256 digest"),
            Self::InvalidKind => formatter.write_str("package kind is not recognized"),
            Self::ControlCharacter(field) => {
                write!(formatter, "{field} contains a control character")
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
            version: validate_text("dependency version", version.into())?,
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
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PackageCompatibility {
    runtime: String,
}

impl PackageCompatibility {
    pub fn new(runtime: impl Into<String>) -> Result<Self, PackageManifestError> {
        Ok(Self {
            runtime: validate_text("runtime compatibility", runtime.into())?,
        })
    }

    pub fn runtime(&self) -> &str {
        &self.runtime
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
        for (field, value) in [
            ("signature", signature.as_deref()),
            ("public_key", public_key.as_deref()),
        ] {
            if value.is_some_and(|value| value.chars().any(char::is_control)) {
                return Err(PackageManifestError::ControlCharacter(field));
            }
        }
        Ok(Self {
            level,
            signature,
            public_key,
        })
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
        Ok(Self {
            id: package_id(id)?,
            version: validate_text("version", version.into())?,
            kind,
            publisher: validate_text("publisher", publisher.into())?,
            content_hash: validate_hash(content_hash.into())?,
            dependencies,
            compatibility,
            license: validate_text("license", license.into())?,
            trust,
        })
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

    pub fn identity_matches(&self, other: &Self) -> bool {
        self.id == other.id && self.version == other.version && self.kind == other.kind
    }
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

fn validate_hash(value: String) -> Result<String, PackageManifestError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(PackageManifestError::InvalidHash);
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(PackageManifestError::InvalidHash);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::{HarnessKind, HarnessManifest};

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
}
