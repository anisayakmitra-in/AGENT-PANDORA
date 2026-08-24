use super::{load_config, parse_options, path_option};
use crate::output::{CliError, CommandResult, success};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use pandora_types::hash_artifact;
use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAX_UPDATE_BYTES: u64 = 256 * 1024 * 1024;
const OFFICIAL_RELEASE_BASE_URL: &str =
    "https://github.com/anisayakmitra-in/PANDORA-AGENT/releases/download";
const RELEASE_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
enum UpdateError {
    Io,
    MissingArtifact,
    InvalidChecksum,
    ChecksumMismatch,
    InvalidSignatureEncoding,
    InvalidSignature,
    TooLarge,
    UnsafeTarget,
    NoRollback,
    InvalidRelease,
    UnsupportedPlatform,
    InvalidChecksumManifest,
    MissingReleaseChecksum,
    ReleaseDownload,
}

#[derive(Debug)]
struct VerifiedArtifact {
    bytes: Vec<u8>,
    signature_verified: bool,
}

#[derive(Debug)]
struct DownloadedRelease {
    artifact_name: String,
    verified: VerifiedArtifact,
}

impl UpdateError {
    fn reason(&self) -> &'static str {
        match self {
            Self::Io => "io_error",
            Self::MissingArtifact => "missing_artifact",
            Self::InvalidChecksum => "invalid_checksum",
            Self::ChecksumMismatch => "checksum_mismatch",
            Self::InvalidSignatureEncoding => "invalid_signature_encoding",
            Self::InvalidSignature => "invalid_signature",
            Self::TooLarge => "artifact_too_large",
            Self::UnsafeTarget => "unsafe_target",
            Self::NoRollback => "no_rollback_available",
            Self::InvalidRelease => "invalid_release",
            Self::UnsupportedPlatform => "unsupported_platform",
            Self::InvalidChecksumManifest => "invalid_checksum_manifest",
            Self::MissingReleaseChecksum => "missing_release_checksum",
            Self::ReleaseDownload => "release_download_failed",
        }
    }
}

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "artifact",
            "sha256",
            "public-key",
            "signature",
            "target",
            "config",
            "data-dir",
            "workspace",
            "dry-run",
            "rollback",
            "release",
        ],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "update does not accept positional arguments",
        ));
    }
    let dry_run = parsed.value("dry-run").is_some();
    if parsed.value("rollback").is_some() {
        if parsed.value("artifact").is_some()
            || parsed.value("sha256").is_some()
            || parsed.value("public-key").is_some()
            || parsed.value("signature").is_some()
            || parsed.value("release").is_some()
        {
            return Err(CliError::usage(
                "update --rollback cannot be combined with artifact verification options",
            ));
        }
        let config = load_config(&parsed)
            .map_err(|error| CliError::update(error.message, json!({"reason": "configuration"})))?;
        let target = target_path(&config, path_option(&parsed, "target"));
        return rollback(&target, dry_run).map_err(|error| update_error(error, &target));
    }

    if let Some(release) = parsed.value("release") {
        if parsed.value("artifact").is_some()
            || parsed.value("sha256").is_some()
            || parsed.value("public-key").is_some()
            || parsed.value("signature").is_some()
        {
            return Err(CliError::usage(
                "update --release cannot be combined with local artifact verification options",
            ));
        }
        let downloaded = download_release_artifact(
            OFFICIAL_RELEASE_BASE_URL,
            release,
            std::env::consts::OS,
            std::env::consts::ARCH,
            download_https,
        )
        .map_err(|error| update_error(error, Path::new(release)))?;
        if dry_run {
            return Ok(success(
                "update",
                json!({
                    "verified": true,
                    "release": release,
                    "artifact": downloaded.artifact_name,
                    "signature_verified": downloaded.verified.signature_verified,
                    "dry_run": true,
                }),
                "Official release artifact verified; no files changed".to_owned(),
            ));
        }
        let config = load_config(&parsed)
            .map_err(|error| CliError::update(error.message, json!({"reason": "configuration"})))?;
        let target = target_path(&config, path_option(&parsed, "target"));
        install_verified(&downloaded.verified.bytes, &target)
            .map_err(|error| update_error(error, &target))?;
        return Ok(success(
            "update",
            json!({
                "verified": true,
                "release": release,
                "artifact": downloaded.artifact_name,
                "target": target,
                "signature_verified": downloaded.verified.signature_verified,
                "dry_run": false,
            }),
            format!("Verified release update staged at {}", target.display()),
        ));
    }

    let artifact = parsed.value("artifact").map(PathBuf::from).ok_or_else(|| {
        CliError::usage("update requires '--release <tag>', '--artifact <path>', or '--rollback'")
    })?;
    let expected = parsed
        .value("sha256")
        .ok_or_else(|| CliError::usage("update requires '--sha256 <digest>'"))?;
    let verified = verify_artifact(
        &artifact,
        expected,
        parsed.value("public-key"),
        parsed.value("signature"),
    )
    .map_err(|error| update_error(error, &artifact))?;
    if dry_run {
        return Ok(success(
            "update",
            json!({
                "verified": true,
                "artifact": artifact,
                "signature_verified": verified.signature_verified,
                "dry_run": true,
            }),
            "Update artifact verified; no files changed".to_owned(),
        ));
    }
    let config = load_config(&parsed)
        .map_err(|error| CliError::update(error.message, json!({"reason": "configuration"})))?;
    let target = target_path(&config, path_option(&parsed, "target"));
    install_verified(&verified.bytes, &target).map_err(|error| update_error(error, &target))?;
    Ok(success(
        "update",
        json!({
            "verified": true,
            "artifact": artifact,
            "target": target,
            "signature_verified": verified.signature_verified,
            "dry_run": false,
        }),
        format!("Verified update staged at {}", target.display()),
    ))
}

fn verify_artifact(
    artifact: &Path,
    expected: &str,
    public_key: Option<&str>,
    signature: Option<&str>,
) -> Result<VerifiedArtifact, UpdateError> {
    if !expected.starts_with("sha256:") || expected.len() != 71 {
        return Err(UpdateError::InvalidChecksum);
    }
    if (public_key.is_some()) != (signature.is_some()) {
        return Err(UpdateError::InvalidSignatureEncoding);
    }
    let metadata = fs::metadata(artifact).map_err(|_| UpdateError::MissingArtifact)?;
    if !metadata.is_file() {
        return Err(UpdateError::MissingArtifact);
    }
    if metadata.len() > MAX_UPDATE_BYTES {
        return Err(UpdateError::TooLarge);
    }
    let bytes = fs::read(artifact).map_err(|_| UpdateError::Io)?;
    verify_artifact_bytes(bytes, expected, public_key, signature)
}

fn verify_artifact_bytes(
    bytes: Vec<u8>,
    expected: &str,
    public_key: Option<&str>,
    signature: Option<&str>,
) -> Result<VerifiedArtifact, UpdateError> {
    if !hash_artifact(&bytes).eq_ignore_ascii_case(expected) {
        return Err(UpdateError::ChecksumMismatch);
    }
    match (public_key, signature) {
        (Some(public_key), Some(signature)) => {
            let key_bytes = decode_fixed::<32>(public_key)?;
            let signature_bytes = decode_fixed::<64>(signature)?;
            let key = VerifyingKey::from_bytes(&key_bytes)
                .map_err(|_| UpdateError::InvalidSignatureEncoding)?;
            let signature = Signature::from_bytes(&signature_bytes);
            key.verify(&bytes, &signature)
                .map_err(|_| UpdateError::InvalidSignature)?;
            Ok(VerifiedArtifact {
                bytes,
                signature_verified: true,
            })
        }
        (None, None) => Ok(VerifiedArtifact {
            bytes,
            signature_verified: false,
        }),
        _ => Err(UpdateError::InvalidSignatureEncoding),
    }
}

fn download_release_artifact<F>(
    base_url: &str,
    release: &str,
    operating_system: &str,
    architecture: &str,
    mut download: F,
) -> Result<DownloadedRelease, UpdateError>
where
    F: FnMut(&str) -> Result<Vec<u8>, UpdateError>,
{
    let artifact_name = release_artifact_name(operating_system, architecture)?;
    let checksums_url = release_asset_url(base_url, release, "checksums.txt")?;
    let checksum_manifest = download(&checksums_url)?;
    let checksum = release_checksum(&checksum_manifest, &artifact_name)?;
    let artifact_url = release_asset_url(base_url, release, &artifact_name)?;
    let bytes = download(&artifact_url)?;
    let verified = verify_artifact_bytes(bytes, &format!("sha256:{checksum}"), None, None)?;
    Ok(DownloadedRelease {
        artifact_name,
        verified,
    })
}

fn release_artifact_name(
    operating_system: &str,
    architecture: &str,
) -> Result<String, UpdateError> {
    match (operating_system, architecture) {
        ("linux", "x86_64") => Ok("pandora-x86_64-unknown-linux-gnu".to_owned()),
        ("macos", "x86_64") => Ok("pandora-x86_64-apple-darwin".to_owned()),
        ("macos", "aarch64") => Ok("pandora-aarch64-apple-darwin".to_owned()),
        ("windows", "x86_64") => Ok("pandora-x86_64-pc-windows-msvc.exe".to_owned()),
        _ => Err(UpdateError::UnsupportedPlatform),
    }
}

fn release_asset_url(base_url: &str, release: &str, asset: &str) -> Result<String, UpdateError> {
    let version = release
        .strip_prefix('v')
        .ok_or(UpdateError::InvalidRelease)?;
    if semver::Version::parse(version).is_err()
        || asset.is_empty()
        || asset.contains('/')
        || asset.contains('\\')
    {
        return Err(UpdateError::InvalidRelease);
    }
    let mut url = reqwest::Url::parse(base_url).map_err(|_| UpdateError::InvalidRelease)?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(UpdateError::InvalidRelease);
    }
    url.path_segments_mut()
        .map_err(|_| UpdateError::InvalidRelease)?
        .push(release)
        .push(asset);
    Ok(url.into())
}

fn release_checksum(manifest: &[u8], artifact_name: &str) -> Result<String, UpdateError> {
    let text = std::str::from_utf8(manifest).map_err(|_| UpdateError::InvalidChecksumManifest)?;
    let mut checksum = None;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let mut fields = line.split_whitespace();
        let digest = fields.next().ok_or(UpdateError::InvalidChecksumManifest)?;
        let name = fields.next().ok_or(UpdateError::InvalidChecksumManifest)?;
        if fields.next().is_some() || !is_sha256_digest(digest) {
            return Err(UpdateError::InvalidChecksumManifest);
        }
        let name = name.trim_start_matches('*');
        if (name == artifact_name || name.strip_prefix("dist/") == Some(artifact_name))
            && checksum.replace(digest.to_ascii_lowercase()).is_some()
        {
            return Err(UpdateError::InvalidChecksumManifest);
        }
    }
    checksum.ok_or(UpdateError::MissingReleaseChecksum)
}

fn is_sha256_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn download_https(url: &str) -> Result<Vec<u8>, UpdateError> {
    let parsed = reqwest::Url::parse(url).map_err(|_| UpdateError::ReleaseDownload)?;
    if parsed.scheme() != "https" || !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(UpdateError::ReleaseDownload);
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(RELEASE_DOWNLOAD_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|_| UpdateError::ReleaseDownload)?;
    let mut response = client
        .get(parsed)
        .send()
        .map_err(|_| UpdateError::ReleaseDownload)?;
    if !response.status().is_success()
        || response.url().scheme() != "https"
        || !response.url().username().is_empty()
        || response.url().password().is_some()
    {
        return Err(UpdateError::ReleaseDownload);
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_UPDATE_BYTES)
    {
        return Err(UpdateError::TooLarge);
    }
    read_limited(&mut response, MAX_UPDATE_BYTES)
}

fn read_limited<R: Read>(reader: &mut R, limit: u64) -> Result<Vec<u8>, UpdateError> {
    let mut bytes = Vec::new();
    reader
        .take(limit.checked_add(1).ok_or(UpdateError::TooLarge)?)
        .read_to_end(&mut bytes)
        .map_err(|_| UpdateError::ReleaseDownload)?;
    if bytes.len() as u64 > limit {
        return Err(UpdateError::TooLarge);
    }
    Ok(bytes)
}

fn decode_fixed<const N: usize>(value: &str) -> Result<[u8; N], UpdateError> {
    if value.len() != N * 2 {
        return Err(UpdateError::InvalidSignatureEncoding);
    }
    let mut output = [0_u8; N];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_digit(pair[0]).ok_or(UpdateError::InvalidSignatureEncoding)?;
        let low = hex_digit(pair[1]).ok_or(UpdateError::InvalidSignatureEncoding)?;
        output[index] = (high << 4) | low;
    }
    Ok(output)
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn target_path(
    config: &pandora_runtime::config::RuntimeConfig,
    target: Option<PathBuf>,
) -> PathBuf {
    target.unwrap_or_else(|| config.data_dir().join("updates/current/pandora"))
}

fn previous_path(target: &Path) -> Result<PathBuf, UpdateError> {
    let parent = target.parent().ok_or(UpdateError::UnsafeTarget)?;
    let name = target.file_name().ok_or(UpdateError::UnsafeTarget)?;
    Ok(parent.join(format!(".{}.previous", name.to_string_lossy())))
}

fn install_verified(bytes: &[u8], target: &Path) -> Result<(), UpdateError> {
    reject_symlink(target)?;
    let previous = previous_path(target)?;
    reject_symlink(&previous)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|_| UpdateError::Io)?;
    }
    let had_current = target.exists();
    if had_current {
        if previous.exists() {
            fs::remove_file(&previous).map_err(|_| UpdateError::Io)?;
        }
        fs::rename(target, &previous).map_err(|_| UpdateError::Io)?;
    }
    let temporary = target.with_extension(format!("new-{}", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|_| UpdateError::Io)?;
        file.write_all(bytes).map_err(|_| UpdateError::Io)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            file.set_permissions(fs::Permissions::from_mode(0o755))
                .map_err(|_| UpdateError::Io)?;
        }
        file.sync_all().map_err(|_| UpdateError::Io)?;
        fs::rename(&temporary, target).map_err(|_| UpdateError::Io)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
        if had_current {
            let _ = fs::rename(&previous, target);
        }
    }
    result
}

fn rollback(target: &Path, dry_run: bool) -> Result<CommandResult, UpdateError> {
    let previous = previous_path(target)?;
    reject_symlink(target)?;
    reject_symlink(&previous)?;
    if !previous.is_file() {
        return Err(UpdateError::NoRollback);
    }
    if dry_run {
        return Ok(success(
            "update rollback",
            json!({"target": target, "previous": previous, "dry_run": true}),
            "Rollback is ready; no files changed".to_owned(),
        ));
    }
    if target.exists() {
        fs::remove_file(target).map_err(|_| UpdateError::Io)?;
    }
    fs::rename(&previous, target).map_err(|_| UpdateError::Io)?;
    Ok(success(
        "update rollback",
        json!({"target": target, "restored": true, "dry_run": false}),
        format!("Rolled back update at {}", target.display()),
    ))
}

fn reject_symlink(path: &Path) -> Result<(), UpdateError> {
    if fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(UpdateError::UnsafeTarget);
    }
    Ok(())
}

fn update_error(error: UpdateError, path: &Path) -> CliError {
    CliError::update(
        match error {
            UpdateError::ChecksumMismatch => "update checksum does not match the artifact",
            UpdateError::InvalidChecksum => "update checksum must be a sha256 digest",
            UpdateError::InvalidSignatureEncoding => "update signature uses invalid hex encoding",
            UpdateError::InvalidSignature => "update signature verification failed",
            UpdateError::MissingArtifact => "update artifact is missing or not a file",
            UpdateError::TooLarge => "update artifact exceeds the size limit",
            UpdateError::UnsafeTarget => "update target is unsafe",
            UpdateError::NoRollback => "no verified previous update is available",
            UpdateError::Io => "update filesystem operation failed",
            UpdateError::InvalidRelease => "release tag or official release URL is invalid",
            UpdateError::UnsupportedPlatform => {
                "no official update artifact exists for this platform"
            }
            UpdateError::InvalidChecksumManifest => "release checksum manifest is invalid",
            UpdateError::MissingReleaseChecksum => "release checksum is missing for this platform",
            UpdateError::ReleaseDownload => "official release download failed",
        },
        json!({"reason": error.reason(), "path": path}),
    )
}

#[cfg(test)]
mod tests {
    use super::{UpdateError, download_release_artifact, verify_artifact};
    use ed25519_dalek::{Signer, SigningKey};
    use std::fs;

    fn encode_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn verifies_detached_ed25519_signature() {
        let artifact_path =
            std::env::temp_dir().join(format!("pandora-update-signature-{}", std::process::id()));
        let artifact = b"verified update artifact";
        fs::write(&artifact_path, artifact).expect("artifact should be written");
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let signature = signing_key.sign(artifact);

        let verified = verify_artifact(
            &artifact_path,
            &pandora_types::hash_artifact(artifact),
            Some(&encode_hex(&signing_key.verifying_key().to_bytes())),
            Some(&encode_hex(&signature.to_bytes())),
        )
        .expect("signature should verify");

        assert_eq!(verified.bytes, artifact);
        assert!(verified.signature_verified);
        let _ = fs::remove_file(artifact_path);
    }

    #[test]
    fn rejects_a_changed_artifact_after_verification_input_is_read() {
        let artifact_path =
            std::env::temp_dir().join(format!("pandora-update-checksum-{}", std::process::id()));
        let artifact = b"original artifact";
        fs::write(&artifact_path, artifact).expect("artifact should be written");
        let expected = pandora_types::hash_artifact(artifact);
        fs::write(&artifact_path, b"changed artifact").expect("artifact should be changed");

        let error = verify_artifact(&artifact_path, &expected, None, None)
            .expect_err("changed artifact should fail verification");
        assert!(matches!(error, UpdateError::ChecksumMismatch));
        let _ = fs::remove_file(artifact_path);
    }

    #[test]
    fn downloads_the_requested_platform_artifact_and_verifies_its_checksum() {
        let artifact = b"release artifact";
        let expected = pandora_types::hash_artifact(artifact);
        let manifest = format!("{}  pandora-x86_64-unknown-linux-gnu\n", &expected[7..]);
        let mut requested = Vec::new();

        let downloaded = download_release_artifact(
            "https://github.com/anisayakmitra-in/PANDORA-AGENT/releases/download",
            "v2.0.0-alpha.6",
            "linux",
            "x86_64",
            |url| {
                requested.push(url.to_owned());
                if url.ends_with("/checksums.txt") {
                    Ok(manifest.as_bytes().to_vec())
                } else if url.ends_with("/pandora-x86_64-unknown-linux-gnu") {
                    Ok(artifact.to_vec())
                } else {
                    Err(UpdateError::Io)
                }
            },
        )
        .expect("the release artifact should verify");

        assert_eq!(downloaded.artifact_name, "pandora-x86_64-unknown-linux-gnu");
        assert_eq!(downloaded.verified.bytes, artifact);
        assert_eq!(
            requested,
            vec![
                "https://github.com/anisayakmitra-in/PANDORA-AGENT/releases/download/v2.0.0-alpha.6/checksums.txt",
                "https://github.com/anisayakmitra-in/PANDORA-AGENT/releases/download/v2.0.0-alpha.6/pandora-x86_64-unknown-linux-gnu",
            ]
        );
    }

    #[test]
    fn rejects_a_release_artifact_that_does_not_match_its_checksum() {
        let manifest = "0000000000000000000000000000000000000000000000000000000000000000  pandora-x86_64-unknown-linux-gnu\n";

        let error = download_release_artifact(
            "https://github.com/anisayakmitra-in/PANDORA-AGENT/releases/download",
            "v2.0.0-alpha.6",
            "linux",
            "x86_64",
            |url| {
                if url.ends_with("/checksums.txt") {
                    Ok(manifest.as_bytes().to_vec())
                } else {
                    Ok(b"untrusted release artifact".to_vec())
                }
            },
        )
        .expect_err("mismatched release bytes should be rejected");

        assert!(matches!(error, UpdateError::ChecksumMismatch));
    }

    #[test]
    fn accepts_a_historic_dist_prefixed_checksum_entry() {
        let artifact = b"historic release artifact";
        let expected = pandora_types::hash_artifact(artifact);
        let manifest = format!(
            "{}  dist/pandora-x86_64-unknown-linux-gnu\n",
            &expected[7..]
        );

        let downloaded = download_release_artifact(
            "https://github.com/anisayakmitra-in/PANDORA-AGENT/releases/download",
            "v2.0.0-alpha.6",
            "linux",
            "x86_64",
            |url| {
                if url.ends_with("/checksums.txt") {
                    Ok(manifest.as_bytes().to_vec())
                } else {
                    Ok(artifact.to_vec())
                }
            },
        )
        .expect("historic release checksum entries should remain supported");

        assert_eq!(downloaded.verified.bytes, artifact);
    }
}
