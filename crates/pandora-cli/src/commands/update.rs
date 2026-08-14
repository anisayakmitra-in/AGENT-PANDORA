use super::{load_config, parse_options, path_option};
use crate::output::{CliError, CommandResult, success};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use pandora_types::hash_artifact;
use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const MAX_UPDATE_BYTES: u64 = 256 * 1024 * 1024;

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
}

#[derive(Debug)]
struct VerifiedArtifact {
    bytes: Vec<u8>,
    signature_verified: bool,
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

    let artifact = parsed
        .value("artifact")
        .map(PathBuf::from)
        .ok_or_else(|| CliError::usage("update requires '--artifact <path>' or '--rollback'"))?;
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
        },
        json!({"reason": error.reason(), "path": path}),
    )
}

#[cfg(test)]
mod tests {
    use super::{UpdateError, verify_artifact};
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
}
