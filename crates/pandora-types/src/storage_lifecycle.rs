use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;

pub const STORAGE_LIFECYCLE_POLICY_VERSION: u32 = 1;
pub const MAX_STORAGE_LIFECYCLE_PROVIDER_FIELDS: usize = 5;

const MAX_IDENTIFIER_BYTES: usize = 160;
const MAX_RESOURCE_ID_BYTES: usize = 512;
const MAX_PROVIDER_FIELD_BYTES: usize = 2_048;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageLifecycleProvider {
    LocalFilesystem,
    AwsS3,
    AzureBlob,
    GcpCloudStorage,
}

impl StorageLifecycleProvider {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LocalFilesystem => "local_filesystem",
            Self::AwsS3 => "aws_s3",
            Self::AzureBlob => "azure_blob",
            Self::GcpCloudStorage => "gcp_cloud_storage",
        }
    }

    pub fn parse(value: &str) -> Result<Self, StorageLifecycleContractError> {
        match value {
            "local_filesystem" => Ok(Self::LocalFilesystem),
            "aws_s3" => Ok(Self::AwsS3),
            "azure_blob" => Ok(Self::AzureBlob),
            "gcp_cloud_storage" => Ok(Self::GcpCloudStorage),
            _ => Err(StorageLifecycleContractError::InvalidProvider),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageLifecycleAction {
    BackupExpired,
    SnapshotRemoved,
    EncryptionKeyDestroyed,
}

impl StorageLifecycleAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BackupExpired => "backup_expired",
            Self::SnapshotRemoved => "snapshot_removed",
            Self::EncryptionKeyDestroyed => "encryption_key_destroyed",
        }
    }

    pub fn parse(value: &str) -> Result<Self, StorageLifecycleContractError> {
        match value {
            "backup_expired" => Ok(Self::BackupExpired),
            "snapshot_removed" => Ok(Self::SnapshotRemoved),
            "encryption_key_destroyed" => Ok(Self::EncryptionKeyDestroyed),
            _ => Err(StorageLifecycleContractError::InvalidAction),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StorageLifecycleManifest {
    policy_version: u32,
    evidence_id: String,
    policy_id: String,
    provider: StorageLifecycleProvider,
    action: StorageLifecycleAction,
    resource_id: String,
    provider_fields: BTreeMap<String, String>,
    external_evidence_digest: String,
    actor: String,
    performed_at: u64,
}

impl StorageLifecycleManifest {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        evidence_id: impl Into<String>,
        policy_id: impl Into<String>,
        provider: StorageLifecycleProvider,
        action: StorageLifecycleAction,
        resource_id: impl Into<String>,
        provider_fields: BTreeMap<String, String>,
        external_evidence_digest: impl Into<String>,
        actor: impl Into<String>,
        performed_at: u64,
    ) -> Result<Self, StorageLifecycleContractError> {
        let manifest = Self {
            policy_version: STORAGE_LIFECYCLE_POLICY_VERSION,
            evidence_id: evidence_id.into(),
            policy_id: policy_id.into(),
            provider,
            action,
            resource_id: resource_id.into(),
            provider_fields,
            external_evidence_digest: external_evidence_digest.into(),
            actor: actor.into(),
            performed_at,
        };
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), StorageLifecycleContractError> {
        if self.policy_version != STORAGE_LIFECYCLE_POLICY_VERSION {
            return Err(StorageLifecycleContractError::UnsupportedPolicyVersion);
        }
        validate_identifier("evidence_id", &self.evidence_id, MAX_IDENTIFIER_BYTES)?;
        validate_identifier("policy_id", &self.policy_id, MAX_IDENTIFIER_BYTES)?;
        validate_identifier("resource_id", &self.resource_id, MAX_RESOURCE_ID_BYTES)?;
        validate_identifier("actor", &self.actor, MAX_IDENTIFIER_BYTES)?;
        validate_digest(&self.external_evidence_digest)?;
        if self.performed_at == 0 || i64::try_from(self.performed_at).is_err() {
            return Err(StorageLifecycleContractError::InvalidTimestamp);
        }
        let expected = expected_provider_fields(self.provider, self.action);
        if self.provider_fields.len() > MAX_STORAGE_LIFECYCLE_PROVIDER_FIELDS {
            return Err(StorageLifecycleContractError::TooManyProviderFields);
        }
        for field in expected {
            let value = self.provider_fields.get(*field).ok_or_else(|| {
                StorageLifecycleContractError::MissingProviderField((*field).to_owned())
            })?;
            validate_provider_field(field, value)?;
        }
        for field in self.provider_fields.keys() {
            if !expected.contains(&field.as_str()) {
                return Err(StorageLifecycleContractError::UnexpectedProviderField(
                    field.clone(),
                ));
            }
        }
        Ok(())
    }

    pub const fn policy_version(&self) -> u32 {
        self.policy_version
    }

    pub fn evidence_id(&self) -> &str {
        &self.evidence_id
    }

    pub fn policy_id(&self) -> &str {
        &self.policy_id
    }

    pub const fn provider(&self) -> StorageLifecycleProvider {
        self.provider
    }

    pub const fn action(&self) -> StorageLifecycleAction {
        self.action
    }

    pub fn resource_id(&self) -> &str {
        &self.resource_id
    }

    pub fn provider_fields(&self) -> &BTreeMap<String, String> {
        &self.provider_fields
    }

    pub fn external_evidence_digest(&self) -> &str {
        &self.external_evidence_digest
    }

    pub fn actor(&self) -> &str {
        &self.actor
    }

    pub const fn performed_at(&self) -> u64 {
        self.performed_at
    }

    pub fn manifest_digest(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"storage-lifecycle-manifest-v1\0");
        hasher.update(self.policy_version.to_be_bytes());
        for value in [
            self.evidence_id.as_str(),
            self.policy_id.as_str(),
            self.provider.as_str(),
            self.action.as_str(),
            self.resource_id.as_str(),
            self.external_evidence_digest.as_str(),
            self.actor.as_str(),
        ] {
            hash_field(&mut hasher, value);
        }
        hasher.update(self.performed_at.to_be_bytes());
        for (key, value) in &self.provider_fields {
            hash_field(&mut hasher, key);
            hash_field(&mut hasher, value);
        }
        format!("sha256:{:x}", hasher.finalize())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StorageLifecycleContractError {
    UnsupportedPolicyVersion,
    InvalidProvider,
    InvalidAction,
    EmptyField(&'static str),
    FieldTooLong(&'static str),
    ControlCharacter(&'static str),
    InvalidIdentifier(&'static str),
    InvalidDigest,
    InvalidTimestamp,
    TooManyProviderFields,
    MissingProviderField(String),
    UnexpectedProviderField(String),
    EmptyProviderField(String),
    ProviderFieldTooLong(String),
    ProviderFieldControlCharacter(String),
}

impl fmt::Display for StorageLifecycleContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPolicyVersion => {
                formatter.write_str("storage lifecycle policy version is unsupported")
            }
            Self::InvalidProvider => formatter.write_str("storage lifecycle provider is invalid"),
            Self::InvalidAction => formatter.write_str("storage lifecycle action is invalid"),
            Self::EmptyField(field) => write!(formatter, "{field} cannot be empty"),
            Self::FieldTooLong(field) => write!(formatter, "{field} is too long"),
            Self::ControlCharacter(field) => {
                write!(formatter, "{field} contains a control character")
            }
            Self::InvalidIdentifier(field) => write!(formatter, "{field} is not canonical"),
            Self::InvalidDigest => {
                formatter.write_str("external_evidence_digest must be a lowercase sha256 digest")
            }
            Self::InvalidTimestamp => {
                formatter.write_str("performed_at must be positive Unix seconds")
            }
            Self::TooManyProviderFields => {
                formatter.write_str("storage lifecycle manifest has too many provider fields")
            }
            Self::MissingProviderField(field) => {
                write!(formatter, "provider field '{field}' is required")
            }
            Self::UnexpectedProviderField(field) => {
                write!(formatter, "provider field '{field}' is not allowed")
            }
            Self::EmptyProviderField(field) => {
                write!(formatter, "provider field '{field}' cannot be empty")
            }
            Self::ProviderFieldTooLong(field) => {
                write!(formatter, "provider field '{field}' is too long")
            }
            Self::ProviderFieldControlCharacter(field) => {
                write!(
                    formatter,
                    "provider field '{field}' contains a control character"
                )
            }
        }
    }
}

impl std::error::Error for StorageLifecycleContractError {}

fn expected_provider_fields(
    provider: StorageLifecycleProvider,
    action: StorageLifecycleAction,
) -> &'static [&'static str] {
    match (provider, action) {
        (StorageLifecycleProvider::LocalFilesystem, StorageLifecycleAction::BackupExpired) => {
            &["deletion_event_id", "file_sha256", "path"]
        }
        (StorageLifecycleProvider::LocalFilesystem, StorageLifecycleAction::SnapshotRemoved) => {
            &["deletion_event_id", "snapshot_id"]
        }
        (
            StorageLifecycleProvider::LocalFilesystem,
            StorageLifecycleAction::EncryptionKeyDestroyed,
        ) => &["destruction_event_id", "key_id"],
        (StorageLifecycleProvider::AwsS3, StorageLifecycleAction::BackupExpired) => {
            &["bucket", "deletion_marker_id", "object_key", "version_id"]
        }
        (StorageLifecycleProvider::AwsS3, StorageLifecycleAction::SnapshotRemoved) => {
            &["deletion_event_id", "snapshot_arn"]
        }
        (StorageLifecycleProvider::AwsS3, StorageLifecycleAction::EncryptionKeyDestroyed) => {
            &["deletion_event_id", "key_arn"]
        }
        (StorageLifecycleProvider::AzureBlob, StorageLifecycleAction::BackupExpired) => &[
            "account",
            "blob",
            "container",
            "deletion_event_id",
            "version_id",
        ],
        (StorageLifecycleProvider::AzureBlob, StorageLifecycleAction::SnapshotRemoved) => {
            &["deletion_event_id", "snapshot_id"]
        }
        (StorageLifecycleProvider::AzureBlob, StorageLifecycleAction::EncryptionKeyDestroyed) => {
            &["key_name", "key_version", "purge_event_id", "vault_uri"]
        }
        (StorageLifecycleProvider::GcpCloudStorage, StorageLifecycleAction::BackupExpired) => {
            &["bucket", "generation", "object_name", "operation_id"]
        }
        (StorageLifecycleProvider::GcpCloudStorage, StorageLifecycleAction::SnapshotRemoved) => {
            &["operation_id", "snapshot_resource"]
        }
        (
            StorageLifecycleProvider::GcpCloudStorage,
            StorageLifecycleAction::EncryptionKeyDestroyed,
        ) => &["destroy_event_id", "key_version_resource"],
    }
}

fn validate_identifier(
    field: &'static str,
    value: &str,
    max_bytes: usize,
) -> Result<(), StorageLifecycleContractError> {
    validate_text(field, value, max_bytes)?;
    if !value
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphanumeric)
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'_' | b'-' | b'.' | b'/' | b':' | b'@' | b'+')
        })
    {
        return Err(StorageLifecycleContractError::InvalidIdentifier(field));
    }
    Ok(())
}

fn validate_text(
    field: &'static str,
    value: &str,
    max_bytes: usize,
) -> Result<(), StorageLifecycleContractError> {
    if value.is_empty() || value.trim() != value {
        return Err(StorageLifecycleContractError::EmptyField(field));
    }
    if value.len() > max_bytes {
        return Err(StorageLifecycleContractError::FieldTooLong(field));
    }
    if value.chars().any(char::is_control) {
        return Err(StorageLifecycleContractError::ControlCharacter(field));
    }
    Ok(())
}

fn validate_provider_field(field: &str, value: &str) -> Result<(), StorageLifecycleContractError> {
    if value.is_empty() || value.trim() != value {
        return Err(StorageLifecycleContractError::EmptyProviderField(
            field.to_owned(),
        ));
    }
    if value.len() > MAX_PROVIDER_FIELD_BYTES {
        return Err(StorageLifecycleContractError::ProviderFieldTooLong(
            field.to_owned(),
        ));
    }
    if value.chars().any(char::is_control) {
        return Err(StorageLifecycleContractError::ProviderFieldControlCharacter(field.to_owned()));
    }
    if field == "file_sha256" {
        validate_digest(value)?;
    }
    Ok(())
}

fn validate_digest(value: &str) -> Result<(), StorageLifecycleContractError> {
    if value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        Ok(())
    } else {
        Err(StorageLifecycleContractError::InvalidDigest)
    }
}

fn hash_field(hasher: &mut Sha256, value: &str) {
    hasher.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
    hasher.update(value.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(values: &[(&str, &str)]) -> BTreeMap<String, String> {
        values
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn manifest(
        provider: StorageLifecycleProvider,
        action: StorageLifecycleAction,
        provider_fields: BTreeMap<String, String>,
    ) -> StorageLifecycleManifest {
        StorageLifecycleManifest::new(
            "evidence:daily-2026-09-01",
            "retention:daily-30d",
            provider,
            action,
            "resource:daily-2026-08-01",
            provider_fields,
            format!("sha256:{}", "1".repeat(64)),
            "operator:alice",
            1_788_192_000,
        )
        .unwrap()
    }

    #[test]
    fn provider_action_matrix_requires_exact_evidence_fields() {
        let cases = [
            (
                StorageLifecycleProvider::LocalFilesystem,
                StorageLifecycleAction::BackupExpired,
                fields(&[
                    ("deletion_event_id", "event-1"),
                    ("file_sha256", &format!("sha256:{}", "2".repeat(64))),
                    ("path", "D:/backups/archive.json"),
                ]),
            ),
            (
                StorageLifecycleProvider::AwsS3,
                StorageLifecycleAction::SnapshotRemoved,
                fields(&[
                    ("deletion_event_id", "cloudtrail-1"),
                    ("snapshot_arn", "arn:aws:ec2:region:account:snapshot/snap-1"),
                ]),
            ),
            (
                StorageLifecycleProvider::AzureBlob,
                StorageLifecycleAction::EncryptionKeyDestroyed,
                fields(&[
                    ("key_name", "backup-key"),
                    ("key_version", "version-1"),
                    ("purge_event_id", "activity-1"),
                    ("vault_uri", "https://vault.example/"),
                ]),
            ),
            (
                StorageLifecycleProvider::GcpCloudStorage,
                StorageLifecycleAction::BackupExpired,
                fields(&[
                    ("bucket", "backup-bucket"),
                    ("generation", "123"),
                    ("object_name", "daily/archive.json"),
                    ("operation_id", "audit-1"),
                ]),
            ),
        ];
        for (provider, action, provider_fields) in cases {
            assert_eq!(
                manifest(provider, action, provider_fields).policy_version(),
                1
            );
        }
    }

    #[test]
    fn manifest_rejects_missing_and_unexpected_provider_fields() {
        let missing = StorageLifecycleManifest::new(
            "evidence:1",
            "policy:1",
            StorageLifecycleProvider::AwsS3,
            StorageLifecycleAction::BackupExpired,
            "resource:1",
            fields(&[("bucket", "bucket")]),
            format!("sha256:{}", "1".repeat(64)),
            "operator:alice",
            1,
        )
        .unwrap_err();
        assert!(matches!(
            missing,
            StorageLifecycleContractError::MissingProviderField(_)
        ));

        let mut unexpected = fields(&[("deletion_event_id", "event-1"), ("snapshot_id", "snap-1")]);
        unexpected.insert("secret_key".to_owned(), "not-allowed".to_owned());
        let error = StorageLifecycleManifest::new(
            "evidence:2",
            "policy:1",
            StorageLifecycleProvider::LocalFilesystem,
            StorageLifecycleAction::SnapshotRemoved,
            "resource:2",
            unexpected,
            format!("sha256:{}", "1".repeat(64)),
            "operator:alice",
            1,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            StorageLifecycleContractError::UnexpectedProviderField(_)
        ));
    }

    #[test]
    fn manifest_digest_is_stable_and_sensitive_to_evidence() {
        let first = manifest(
            StorageLifecycleProvider::LocalFilesystem,
            StorageLifecycleAction::EncryptionKeyDestroyed,
            fields(&[("destruction_event_id", "event-1"), ("key_id", "key-1")]),
        );
        let second = first.clone();
        assert_eq!(first.manifest_digest(), second.manifest_digest());
        assert!(first.manifest_digest().starts_with("sha256:"));
    }

    #[test]
    fn deserialized_manifest_still_requires_current_policy() {
        let value = serde_json::json!({
            "policy_version": 2,
            "evidence_id": "evidence:1",
            "policy_id": "policy:1",
            "provider": "local_filesystem",
            "action": "snapshot_removed",
            "resource_id": "resource:1",
            "provider_fields": {
                "deletion_event_id": "event-1",
                "snapshot_id": "snapshot-1"
            },
            "external_evidence_digest": format!("sha256:{}", "1".repeat(64)),
            "actor": "operator:alice",
            "performed_at": 1
        });
        let decoded: StorageLifecycleManifest = serde_json::from_value(value).unwrap();
        assert_eq!(
            decoded.validate(),
            Err(StorageLifecycleContractError::UnsupportedPolicyVersion)
        );
    }
}
