use crate::MAX_STORED_ARTIFACT_BYTES;
use pandora_types::{
    PackageCompatibility, PackageDependency, PackageId, PackageKind, PackageManifest,
    TrustEvidence, TrustLevel,
};
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::redirect::Policy;
use reqwest::{StatusCode, Url};
use semver::Version;
use serde::Deserialize;
use std::fmt;
use std::io::Read;
use std::net::IpAddr;
use std::time::Duration;

const MAX_METADATA_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub enum PackageRegistryError {
    InvalidRegistryUrl,
    UnsupportedRegistryTransport,
    InvalidToken,
    RequestFailed(&'static str),
    HttpStatus {
        operation: &'static str,
        status: StatusCode,
    },
    MetadataTooLarge,
    ArtifactTooLarge,
    InvalidMetadata(&'static str),
    UnsupportedKind(&'static str),
    CapabilityDependenciesUnsupported,
    IncompatiblePlatform,
}

impl fmt::Display for PackageRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRegistryUrl => formatter.write_str("registry URL is invalid"),
            Self::UnsupportedRegistryTransport => formatter
                .write_str("registry URL must use HTTPS or loopback HTTP"),
            Self::InvalidToken => formatter.write_str("registry token is not a valid HTTP value"),
            Self::RequestFailed(operation) => write!(formatter, "registry {operation} failed"),
            Self::HttpStatus { operation, status } => {
                write!(formatter, "registry {operation} returned HTTP {status}")
            }
            Self::MetadataTooLarge => formatter.write_str("registry metadata exceeds the limit"),
            Self::ArtifactTooLarge => formatter.write_str("registry artifact exceeds the limit"),
            Self::InvalidMetadata(reason) => write!(formatter, "registry metadata is invalid: {reason}"),
            Self::UnsupportedKind(kind) => {
                write!(formatter, "registry package kind {kind} is not remotely installable")
            }
            Self::CapabilityDependenciesUnsupported => formatter.write_str(
                "registry capability requirements cannot be converted into exact package dependencies",
            ),
            Self::IncompatiblePlatform => {
                formatter.write_str("registry package does not support this platform")
            }
        }
    }
}

impl std::error::Error for PackageRegistryError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistryPackageDownload {
    manifest: PackageManifest,
    artifact: Vec<u8>,
}

impl RegistryPackageDownload {
    pub fn manifest(&self) -> &PackageManifest {
        &self.manifest
    }

    pub fn artifact(&self) -> &[u8] {
        &self.artifact
    }

    pub fn into_parts(self) -> (PackageManifest, Vec<u8>) {
        (self.manifest, self.artifact)
    }
}

pub struct PackageRegistryClient {
    base_url: Url,
    token: Option<String>,
    client: Client,
}

impl PackageRegistryClient {
    pub fn new(base_url: &str, token: Option<String>) -> Result<Self, PackageRegistryError> {
        let base_url = validate_base_url(base_url)?;
        if token.as_deref().is_some_and(|value| {
            value.is_empty() || value.trim() != value || value.chars().any(char::is_control)
        }) {
            return Err(PackageRegistryError::InvalidToken);
        }
        let client = Client::builder()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|_| PackageRegistryError::InvalidRegistryUrl)?;
        Ok(Self {
            base_url,
            token,
            client,
        })
    }

    pub fn discover(
        &self,
        id: &PackageId,
        version: Option<&str>,
    ) -> Result<PackageManifest, PackageRegistryError> {
        let package = self.fetch_metadata(id, version)?;
        package.to_manifest(id, version)
    }

    pub fn download_exact(
        &self,
        id: &PackageId,
        version: &str,
    ) -> Result<RegistryPackageDownload, PackageRegistryError> {
        let package = self.fetch_metadata(id, Some(version))?;
        let manifest = package.to_manifest(id, Some(version))?;
        let artifact = self.fetch_artifact(id, manifest.version())?;
        Ok(RegistryPackageDownload { manifest, artifact })
    }

    fn fetch_metadata(
        &self,
        id: &PackageId,
        version: Option<&str>,
    ) -> Result<RegistryPackage, PackageRegistryError> {
        let url = self.package_url(id, version, false)?;
        let response = self.send(self.client.get(url), "metadata")?;
        let bytes = read_limited(response, MAX_METADATA_BYTES).map_err(|error| match error {
            ReadLimitError::TooLarge => PackageRegistryError::MetadataTooLarge,
            ReadLimitError::Io => PackageRegistryError::RequestFailed("metadata read"),
        })?;
        serde_json::from_slice(&bytes)
            .map_err(|_| PackageRegistryError::InvalidMetadata("response is not valid JSON"))
    }

    fn fetch_artifact(
        &self,
        id: &PackageId,
        version: &str,
    ) -> Result<Vec<u8>, PackageRegistryError> {
        let url = self.package_url(id, Some(version), true)?;
        let response = self.send(self.client.get(url), "artifact download")?;
        read_limited(response, MAX_STORED_ARTIFACT_BYTES).map_err(|error| match error {
            ReadLimitError::TooLarge => PackageRegistryError::ArtifactTooLarge,
            ReadLimitError::Io => PackageRegistryError::RequestFailed("artifact read"),
        })
    }

    fn package_url(
        &self,
        id: &PackageId,
        version: Option<&str>,
        download: bool,
    ) -> Result<Url, PackageRegistryError> {
        validate_remote_id(id.as_str())?;
        let mut path = format!("api/v1/packages/{}", encode_package_id(id.as_str()));
        if let Some(version) = version {
            Version::parse(version)
                .map_err(|_| PackageRegistryError::InvalidMetadata("version is not SemVer"))?;
            path.push_str("/versions/");
            path.push_str(&encode_version(version));
        }
        if download {
            path.push_str("/download");
        }
        self.base_url
            .join(&path)
            .map_err(|_| PackageRegistryError::InvalidRegistryUrl)
    }

    fn send(
        &self,
        mut request: RequestBuilder,
        operation: &'static str,
    ) -> Result<Response, PackageRegistryError> {
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        let response = request
            .send()
            .map_err(|_| PackageRegistryError::RequestFailed(operation))?;
        if !response.status().is_success() {
            return Err(PackageRegistryError::HttpStatus {
                operation,
                status: response.status(),
            });
        }
        Ok(response)
    }
}

fn validate_remote_id(value: &str) -> Result<(), PackageRegistryError> {
    let valid = !value.is_empty()
        && value.len() <= 256
        && value.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        });
    if valid {
        Ok(())
    } else {
        Err(PackageRegistryError::InvalidMetadata(
            "package ID is invalid",
        ))
    }
}

fn encode_package_id(value: &str) -> String {
    value.replace('/', "%2F")
}

fn encode_version(value: &str) -> String {
    value.replace('+', "%2B")
}

fn validate_base_url(value: &str) -> Result<Url, PackageRegistryError> {
    let mut url = Url::parse(value).map_err(|_| PackageRegistryError::InvalidRegistryUrl)?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
    {
        return Err(PackageRegistryError::InvalidRegistryUrl);
    }
    let allowed = match url.scheme() {
        "https" => true,
        "http" => is_loopback_host(&url),
        _ => false,
    };
    if !allowed {
        return Err(PackageRegistryError::UnsupportedRegistryTransport);
    }
    url.set_path("");
    Ok(url)
}

fn is_loopback_host(url: &Url) -> bool {
    match url.host_str() {
        Some("localhost") => true,
        Some(host) => host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback()),
        None => false,
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RemoteArtifactKind {
    Gene,
    DomainHarness,
    MetaHarness,
    SourceHarness,
    Package,
    Provider,
    Skill,
    MemorySchema,
    RuntimeExtension,
    CapabilityPack,
    Template,
    Persona,
    Policy,
    Benchmark,
    Dataset,
    Plugin,
    Connector,
    Sdk,
    Distribution,
}

impl RemoteArtifactKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Gene => "gene",
            Self::DomainHarness => "domain_harness",
            Self::MetaHarness => "meta_harness",
            Self::SourceHarness => "source_harness",
            Self::Package => "package",
            Self::Provider => "provider",
            Self::Skill => "skill",
            Self::MemorySchema => "memory_schema",
            Self::RuntimeExtension => "runtime_extension",
            Self::CapabilityPack => "capability_pack",
            Self::Template => "template",
            Self::Persona => "persona",
            Self::Policy => "policy",
            Self::Benchmark => "benchmark",
            Self::Dataset => "dataset",
            Self::Plugin => "plugin",
            Self::Connector => "connector",
            Self::Sdk => "sdk",
            Self::Distribution => "distribution",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RemoteTrustLevel {
    Experimental,
    Community,
    Verified,
    Official,
    Enterprise,
    Certified,
}

impl RemoteTrustLevel {
    const fn requires_signature(self) -> bool {
        matches!(
            self,
            Self::Verified | Self::Official | Self::Enterprise | Self::Certified
        )
    }
}

#[derive(Debug, Deserialize)]
struct RegistryTrust {
    level: RemoteTrustLevel,
    signature: Option<String>,
    public_key: Option<String>,
    content_hash: Option<String>,
    publisher: String,
}

#[derive(Debug, Deserialize)]
struct RegistryCapabilities {
    #[serde(default)]
    requires: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RegistryCompatibility {
    #[serde(default)]
    runtimes: Vec<String>,
    #[serde(default)]
    platforms: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RegistryPackage {
    id: String,
    version: String,
    kind: RemoteArtifactKind,
    license: String,
    trust: RegistryTrust,
    capabilities: RegistryCapabilities,
    #[serde(default)]
    dependencies: Vec<PackageDependency>,
    compatibility: RegistryCompatibility,
    artifact_url: Option<String>,
    #[serde(default)]
    yanked: bool,
}

impl RegistryPackage {
    fn to_manifest(
        &self,
        requested_id: &PackageId,
        requested_version: Option<&str>,
    ) -> Result<PackageManifest, PackageRegistryError> {
        if self.id != requested_id.as_str() {
            return Err(PackageRegistryError::InvalidMetadata(
                "package ID does not match request",
            ));
        }
        if requested_version.is_some_and(|version| version != self.version) {
            return Err(PackageRegistryError::InvalidMetadata(
                "package version does not match request",
            ));
        }
        if self.yanked {
            return Err(PackageRegistryError::InvalidMetadata("package is yanked"));
        }
        if self.artifact_url.is_none() {
            return Err(PackageRegistryError::InvalidMetadata(
                "package has no published artifact",
            ));
        }
        let kind = match self.kind {
            RemoteArtifactKind::Gene => PackageKind::Gene,
            RemoteArtifactKind::Provider => PackageKind::Provider,
            RemoteArtifactKind::Skill => PackageKind::Skill,
            _ => return Err(PackageRegistryError::UnsupportedKind(self.kind.as_str())),
        };
        if !self.capabilities.requires.is_empty() {
            return Err(PackageRegistryError::CapabilityDependenciesUnsupported);
        }
        if !self.compatibility.platforms.is_empty()
            && !self
                .compatibility
                .platforms
                .iter()
                .any(|platform| platform == std::env::consts::OS)
        {
            return Err(PackageRegistryError::IncompatiblePlatform);
        }
        let mut runtime_requirements = self
            .compatibility
            .runtimes
            .iter()
            .filter(|runtime| runtime.starts_with("pandora"));
        let runtime_requirement =
            runtime_requirements
                .next()
                .ok_or(PackageRegistryError::InvalidMetadata(
                    "Pandora runtime requirement is missing",
                ))?;
        if runtime_requirements.next().is_some() {
            return Err(PackageRegistryError::InvalidMetadata(
                "Pandora runtime requirement is ambiguous",
            ));
        }
        let compatibility =
            PackageCompatibility::new(runtime_requirement.clone()).map_err(|_| {
                PackageRegistryError::InvalidMetadata("Pandora runtime requirement is invalid")
            })?;
        let content_hash =
            self.trust
                .content_hash
                .clone()
                .ok_or(PackageRegistryError::InvalidMetadata(
                    "content hash is missing",
                ))?;
        if !is_canonical_content_hash(&content_hash) {
            return Err(PackageRegistryError::InvalidMetadata(
                "content hash is not canonical",
            ));
        }
        let trust = match (&self.trust.signature, &self.trust.public_key) {
            (Some(signature), Some(public_key)) => TrustEvidence::new(
                TrustLevel::Official,
                Some(signature.clone()),
                Some(public_key.clone()),
            )
            .map_err(|_| PackageRegistryError::InvalidMetadata("trust evidence is invalid"))?,
            (None, None) if !self.trust.level.requires_signature() => TrustEvidence::unsigned(),
            (None, None) | (Some(_), None) | (None, Some(_)) => {
                return Err(PackageRegistryError::InvalidMetadata(
                    "trust evidence is incomplete",
                ));
            }
        };
        PackageManifest::new(
            self.id.clone(),
            self.version.clone(),
            kind,
            self.trust.publisher.clone(),
            content_hash,
            self.dependencies.clone(),
            compatibility,
            self.license.clone(),
            trust,
        )
        .map_err(|_| PackageRegistryError::InvalidMetadata("canonical manifest is invalid"))
    }
}

fn is_canonical_content_hash(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    })
}

#[derive(Debug)]
enum ReadLimitError {
    Io,
    TooLarge,
}

fn read_limited(mut response: Response, limit: usize) -> Result<Vec<u8>, ReadLimitError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(ReadLimitError::TooLarge);
    }
    read_limited_reader(&mut response, limit)
}

fn read_limited_reader<R: Read>(reader: &mut R, limit: usize) -> Result<Vec<u8>, ReadLimitError> {
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ReadLimitError::Io)?;
    if bytes.len() > limit {
        return Err(ReadLimitError::TooLarge);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn bounded_reader_stops_after_the_limit_plus_one_byte() {
        let mut source = Cursor::new(vec![0_u8; 32]);

        assert!(matches!(
            read_limited_reader(&mut source, 8),
            Err(ReadLimitError::TooLarge)
        ));
        assert_eq!(source.position(), 9);
    }

    #[test]
    fn remote_manifest_rejects_a_noncanonical_uppercase_digest() {
        let package: RegistryPackage = serde_json::from_value(serde_json::json!({
            "id": "owner/package",
            "version": "1.0.0",
            "kind": "gene",
            "license": "MIT",
            "trust": {
                "level": "community",
                "signature": null,
                "public_key": null,
                "content_hash": format!("sha256:{}", "A".repeat(64)),
                "publisher": "owner"
            },
            "capabilities": {"requires": []},
            "compatibility": {
                "runtimes": [concat!("pandora>=", env!("CARGO_PKG_VERSION"))],
                "platforms": []
            },
            "artifact_url": "https://example.com/package.tar",
            "yanked": false
        }))
        .unwrap();

        assert!(matches!(
            package.to_manifest(&PackageId::new("owner/package").unwrap(), Some("1.0.0")),
            Err(PackageRegistryError::InvalidMetadata(
                "content hash is not canonical"
            ))
        ));
    }

    #[test]
    fn remote_manifest_rejects_ambiguous_pandora_runtime_requirements() {
        let package: RegistryPackage = serde_json::from_value(serde_json::json!({
            "id": "owner/package",
            "version": "1.0.0",
            "kind": "gene",
            "license": "MIT",
            "trust": {
                "level": "community",
                "signature": null,
                "public_key": null,
                "content_hash": format!("sha256:{}", "a".repeat(64)),
                "publisher": "owner"
            },
            "capabilities": {"requires": []},
            "compatibility": {
                "runtimes": [
                    "pandora",
                    concat!("pandora>=", env!("CARGO_PKG_VERSION"))
                ],
                "platforms": []
            },
            "artifact_url": "https://example.com/package.tar",
            "yanked": false
        }))
        .unwrap();

        assert!(matches!(
            package.to_manifest(&PackageId::new("owner/package").unwrap(), Some("1.0.0")),
            Err(PackageRegistryError::InvalidMetadata(
                "Pandora runtime requirement is ambiguous"
            ))
        ));
    }

    #[test]
    fn registry_token_rejects_surrounding_whitespace() {
        assert!(matches!(
            PackageRegistryClient::new(
                "https://registry.example",
                Some(" registry-secret".to_owned())
            ),
            Err(PackageRegistryError::InvalidToken)
        ));
    }
}
