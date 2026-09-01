use crate::MAX_STORED_ARTIFACT_BYTES;
use pandora_types::PackageManifest;
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::redirect::Policy;
use reqwest::{StatusCode, Url};
use std::fmt;
use std::io::Read;
use std::time::Duration;
use zeroize::Zeroizing;

const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_REPOSITORY_PATH_BYTES: usize = 1024;

#[derive(Debug)]
pub enum GitHubPackageError {
    InvalidRepositoryUrl,
    InvalidCommit,
    InvalidPath(&'static str),
    InvalidToken,
    RequestFailed(&'static str),
    HttpStatus {
        operation: &'static str,
        status: StatusCode,
    },
    ManifestTooLarge,
    ArtifactTooLarge,
    InvalidManifest,
}

impl fmt::Display for GitHubPackageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRepositoryUrl => formatter
                .write_str("GitHub repository must be https://github.com/<owner>/<repository>"),
            Self::InvalidCommit => {
                formatter.write_str("GitHub source requires one full 40-character commit SHA")
            }
            Self::InvalidPath(label) => write!(
                formatter,
                "GitHub {label} path must be a bounded repository-relative file path"
            ),
            Self::InvalidToken => formatter.write_str("GitHub token is not a valid HTTP value"),
            Self::RequestFailed(operation) => write!(formatter, "GitHub {operation} failed"),
            Self::HttpStatus { operation, status } => {
                write!(formatter, "GitHub {operation} returned HTTP {status}")
            }
            Self::ManifestTooLarge => formatter.write_str("GitHub manifest exceeds the limit"),
            Self::ArtifactTooLarge => formatter.write_str("GitHub artifact exceeds the limit"),
            Self::InvalidManifest => {
                formatter.write_str("GitHub package manifest is not valid Pandora package JSON")
            }
        }
    }
}

impl std::error::Error for GitHubPackageError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitHubPackageDownload {
    manifest: PackageManifest,
    artifact: Vec<u8>,
}

impl GitHubPackageDownload {
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

#[derive(Debug)]
struct GitHubRepository {
    owner: String,
    name: String,
}

pub struct GitHubPackageClient {
    repository: GitHubRepository,
    commit: String,
    token: Option<Zeroizing<String>>,
    raw_base: Url,
    client: Client,
}

impl GitHubPackageClient {
    pub fn new(
        repository_url: &str,
        commit: &str,
        token: Option<String>,
    ) -> Result<Self, GitHubPackageError> {
        let repository = validate_repository_url(repository_url)?;
        let commit = validate_commit(commit)?;
        let token = validate_token(token)?;
        let raw_base = Url::parse("https://raw.githubusercontent.com/")
            .map_err(|_| GitHubPackageError::InvalidRepositoryUrl)?;
        let client = Client::builder()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .user_agent(concat!("pandora/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| GitHubPackageError::InvalidRepositoryUrl)?;
        Ok(Self {
            repository,
            commit,
            token,
            raw_base,
            client,
        })
    }

    pub fn download(
        &self,
        manifest_path: &str,
        artifact_path: &str,
    ) -> Result<GitHubPackageDownload, GitHubPackageError> {
        let manifest_path = validate_repository_path(manifest_path, "manifest")?;
        let artifact_path = validate_repository_path(artifact_path, "artifact")?;
        if manifest_path == artifact_path {
            return Err(GitHubPackageError::InvalidPath("artifact"));
        }
        let manifest_bytes = self.fetch(
            &manifest_path,
            MAX_MANIFEST_BYTES,
            "manifest download",
            GitHubPackageError::ManifestTooLarge,
        )?;
        let artifact = self.fetch(
            &artifact_path,
            MAX_STORED_ARTIFACT_BYTES,
            "artifact download",
            GitHubPackageError::ArtifactTooLarge,
        )?;
        let manifest: PackageManifest = serde_json::from_slice(&manifest_bytes)
            .map_err(|_| GitHubPackageError::InvalidManifest)?;
        Ok(GitHubPackageDownload { manifest, artifact })
    }

    fn fetch(
        &self,
        path: &[String],
        limit: usize,
        operation: &'static str,
        too_large: GitHubPackageError,
    ) -> Result<Vec<u8>, GitHubPackageError> {
        let url = self.raw_url(path)?;
        let response = self.send(self.client.get(url), operation)?;
        read_limited(response, limit).map_err(|error| match error {
            ReadLimitError::TooLarge => too_large,
            ReadLimitError::Io => GitHubPackageError::RequestFailed(operation),
        })
    }

    fn raw_url(&self, path: &[String]) -> Result<Url, GitHubPackageError> {
        let mut url = self.raw_base.clone();
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| GitHubPackageError::InvalidRepositoryUrl)?;
        segments.clear();
        segments.push(&self.repository.owner);
        segments.push(&self.repository.name);
        segments.push(&self.commit);
        for segment in path {
            segments.push(segment);
        }
        drop(segments);
        Ok(url)
    }

    fn send(
        &self,
        mut request: RequestBuilder,
        operation: &'static str,
    ) -> Result<Response, GitHubPackageError> {
        if let Some(token) = &self.token {
            request = request.bearer_auth(token.as_str());
        }
        let response = request
            .send()
            .map_err(|_| GitHubPackageError::RequestFailed(operation))?;
        if !response.status().is_success() {
            return Err(GitHubPackageError::HttpStatus {
                operation,
                status: response.status(),
            });
        }
        Ok(response)
    }

    #[cfg(test)]
    fn with_raw_base(mut self, raw_base: &str) -> Result<Self, GitHubPackageError> {
        self.raw_base =
            Url::parse(raw_base).map_err(|_| GitHubPackageError::InvalidRepositoryUrl)?;
        Ok(self)
    }
}

fn validate_repository_url(value: &str) -> Result<GitHubRepository, GitHubPackageError> {
    let url = Url::parse(value).map_err(|_| GitHubPackageError::InvalidRepositoryUrl)?;
    if url.scheme() != "https"
        || !url
            .host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case("github.com"))
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(GitHubPackageError::InvalidRepositoryUrl);
    }
    let parts = url.path().trim_matches('/').split('/').collect::<Vec<_>>();
    if parts.len() != 2 {
        return Err(GitHubPackageError::InvalidRepositoryUrl);
    }
    let owner = parts[0];
    let name = parts[1].strip_suffix(".git").unwrap_or(parts[1]);
    if !valid_owner(owner) || !valid_repository_name(name) {
        return Err(GitHubPackageError::InvalidRepositoryUrl);
    }
    Ok(GitHubRepository {
        owner: owner.to_owned(),
        name: name.to_owned(),
    })
}

fn valid_owner(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 39
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn valid_repository_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && !matches!(value, "." | "..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn validate_commit(value: &str) -> Result<String, GitHubPackageError> {
    if value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(GitHubPackageError::InvalidCommit);
    }
    Ok(value.to_ascii_lowercase())
}

fn validate_token(token: Option<String>) -> Result<Option<Zeroizing<String>>, GitHubPackageError> {
    match token {
        Some(value)
            if value.is_empty()
                || value.trim() != value
                || value.len() >= 64 * 1024
                || value.chars().any(char::is_control) =>
        {
            Err(GitHubPackageError::InvalidToken)
        }
        Some(value) => Ok(Some(Zeroizing::new(value))),
        None => Ok(None),
    }
}

fn validate_repository_path(
    value: &str,
    label: &'static str,
) -> Result<Vec<String>, GitHubPackageError> {
    if value.is_empty()
        || value.len() > MAX_REPOSITORY_PATH_BYTES
        || value.starts_with('/')
        || value.ends_with('/')
        || value.contains('\\')
        || value.chars().any(char::is_control)
    {
        return Err(GitHubPackageError::InvalidPath(label));
    }
    let segments = value.split('/').map(str::to_owned).collect::<Vec<_>>();
    if segments.iter().any(|segment| {
        segment.is_empty()
            || matches!(segment.as_str(), "." | "..")
            || !segment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    }) {
        return Err(GitHubPackageError::InvalidPath(label));
    }
    Ok(segments)
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
    let mut bytes = Vec::new();
    response
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
    use pandora_types::{PackageCompatibility, PackageKind, TrustEvidence, hash_artifact};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn github_source_requires_an_exact_repository_and_commit() {
        assert!(GitHubPackageClient::new("https://github.com/owner/repo", COMMIT, None).is_ok());
        assert!(
            GitHubPackageClient::new("https://github.com/owner/repo.git", COMMIT, None).is_ok()
        );
        assert!(matches!(
            GitHubPackageClient::new("https://gitlab.com/owner/repo", COMMIT, None),
            Err(GitHubPackageError::InvalidRepositoryUrl)
        ));
        assert!(matches!(
            GitHubPackageClient::new("https://github.com/owner/repo", "main", None),
            Err(GitHubPackageError::InvalidCommit)
        ));
    }

    #[test]
    fn github_paths_reject_traversal_and_ambiguous_segments() {
        assert!(validate_repository_path("packages/gene.json", "manifest").is_ok());
        assert!(validate_repository_path("../gene.json", "manifest").is_err());
        assert!(validate_repository_path("packages//gene.json", "manifest").is_err());
        assert!(validate_repository_path("packages\\gene.json", "manifest").is_err());
    }

    #[test]
    fn github_source_fetches_pinned_files_and_uses_normal_package_admission() {
        let artifact = wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (func (export "pandora_alloc") (param i32) (result i32)
                    i32.const 0)
                (func (export "pandora_run") (param i32 i32) (result i64)
                    i64.const 0))"#,
        )
        .unwrap();
        let manifest = PackageManifest::new(
            "owner/gene",
            "1.0.0",
            PackageKind::Gene,
            "owner",
            hash_artifact(&artifact),
            Vec::new(),
            PackageCompatibility::new(concat!("pandora>=", env!("CARGO_PKG_VERSION"))).unwrap(),
            "MIT",
            TrustEvidence::unsigned(),
        )
        .unwrap();
        let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let served_artifact = artifact.clone();
        let server = thread::spawn(move || {
            for (expected_path, body, content_type) in [
                (
                    format!("/owner/repo/{COMMIT}/pandora-package.json"),
                    manifest_bytes,
                    "application/json",
                ),
                (
                    format!("/owner/repo/{COMMIT}/dist/gene.wasm"),
                    served_artifact,
                    "application/octet-stream",
                ),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    let mut chunk = [0_u8; 1024];
                    let read = stream.read(&mut chunk).unwrap();
                    assert_ne!(read, 0);
                    request.extend_from_slice(&chunk[..read]);
                }
                let headers = String::from_utf8_lossy(&request);
                assert!(headers.starts_with(&format!("GET {expected_path} HTTP/1.1\r\n")));
                assert!(
                    headers
                        .to_ascii_lowercase()
                        .contains("authorization: bearer test-token\r\n")
                );
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            }
        });

        let client = GitHubPackageClient::new(
            "https://github.com/owner/repo",
            COMMIT,
            Some("test-token".to_owned()),
        )
        .unwrap()
        .with_raw_base(&format!("http://{address}/"))
        .unwrap();
        let download = client
            .download("pandora-package.json", "dist/gene.wasm")
            .unwrap();

        assert_eq!(download.manifest().id().as_str(), "owner/gene");
        assert_eq!(download.artifact(), artifact);
        server.join().unwrap();
    }
}
