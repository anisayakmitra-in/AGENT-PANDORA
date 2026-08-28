use argon2::{Algorithm, Argon2, Params, Version};
use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use pandora_types::{TenantId, WorkspaceId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use zeroize::{Zeroize, Zeroizing};

const VAULT_FORMAT_VERSION: u32 = 1;
const VAULT_DIRECTORY: &str = "secret-vaults";
const MAX_VAULT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_SECRET_COUNT: usize = 128;
const MAX_SECRET_BYTES: usize = 64 * 1024;
const MIN_PASSPHRASE_BYTES: usize = 16;
const MAX_PASSPHRASE_BYTES: usize = 4 * 1024;
const SALT_BYTES: usize = 16;
const NONCE_BYTES: usize = 24;
const KEY_BYTES: usize = 32;

#[derive(Debug)]
pub enum SecretVaultError {
    Io,
    UnsafePath,
    InvalidPassphrase,
    InvalidName,
    SecretTooLarge,
    VaultFull,
    InvalidEnvelope,
    AuthenticationFailed,
    ScopeMismatch,
    Serialization,
    Random,
}

impl fmt::Display for SecretVaultError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Io => "could not access the encrypted secret vault",
            Self::UnsafePath => "encrypted secret vault path is unsafe",
            Self::InvalidPassphrase => "secret vault passphrase is invalid",
            Self::InvalidName => "secret name is invalid",
            Self::SecretTooLarge => "secret exceeds the size limit",
            Self::VaultFull => "secret vault entry limit reached",
            Self::InvalidEnvelope => "encrypted secret vault is invalid",
            Self::AuthenticationFailed => "secret vault authentication failed",
            Self::ScopeMismatch => "secret vault scope does not match",
            Self::Serialization => "secret vault could not be encoded",
            Self::Random => "secret vault randomness is unavailable",
        })
    }
}

impl std::error::Error for SecretVaultError {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SecretVaultEntry {
    name: String,
    created_at: u64,
    updated_at: u64,
}

impl SecretVaultEntry {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn created_at(&self) -> u64 {
        self.created_at
    }

    pub const fn updated_at(&self) -> u64 {
        self.updated_at
    }
}

pub struct VaultSecret(Zeroizing<String>);

impl VaultSecret {
    pub fn expose(&self) -> &str {
        self.0.as_str()
    }

    pub fn into_string(self) -> String {
        self.0.to_string()
    }
}

impl fmt::Debug for VaultSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted]")
    }
}

#[derive(Deserialize, Serialize)]
struct VaultEnvelope {
    format_version: u32,
    algorithm: String,
    kdf: String,
    salt: String,
    nonce: String,
    ciphertext: String,
}

#[derive(Default, Deserialize, Serialize)]
struct VaultContents {
    tenant_id: String,
    workspace_id: String,
    secrets: BTreeMap<String, StoredSecret>,
}

#[derive(Deserialize, Serialize)]
struct StoredSecret {
    value: String,
    created_at: u64,
    updated_at: u64,
}

impl Drop for StoredSecret {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}

pub struct SecretVault {
    path: PathBuf,
    tenant_id: TenantId,
    workspace_id: WorkspaceId,
    passphrase: Zeroizing<String>,
    contents: VaultContents,
}

impl SecretVault {
    pub fn open(
        data_dir: impl AsRef<Path>,
        tenant_id: TenantId,
        workspace_id: WorkspaceId,
        passphrase: impl Into<String>,
    ) -> Result<Self, SecretVaultError> {
        let passphrase = Zeroizing::new(passphrase.into());
        validate_passphrase(&passphrase)?;
        let directory = data_dir.as_ref().join(VAULT_DIRECTORY);
        fs::create_dir_all(&directory).map_err(|_| SecretVaultError::Io)?;
        reject_symlink_or_non_directory(&directory)?;
        let path = directory.join(format!("{}.vault", scope_digest(&tenant_id, &workspace_id)));
        let contents = if path.exists() {
            read_contents(&path, &tenant_id, &workspace_id, &passphrase)?
        } else {
            VaultContents {
                tenant_id: tenant_id.as_str().to_owned(),
                workspace_id: workspace_id.as_str().to_owned(),
                secrets: BTreeMap::new(),
            }
        };
        Ok(Self {
            path,
            tenant_id,
            workspace_id,
            passphrase,
            contents,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn list(&self) -> Vec<SecretVaultEntry> {
        self.contents
            .secrets
            .iter()
            .map(|(name, secret)| SecretVaultEntry {
                name: name.clone(),
                created_at: secret.created_at,
                updated_at: secret.updated_at,
            })
            .collect()
    }

    pub fn get(&self, name: &str) -> Result<Option<VaultSecret>, SecretVaultError> {
        validate_name(name)?;
        Ok(self
            .contents
            .secrets
            .get(name)
            .map(|secret| VaultSecret(Zeroizing::new(secret.value.clone()))))
    }

    pub fn put(
        &mut self,
        name: impl Into<String>,
        value: impl Into<String>,
        now: u64,
    ) -> Result<SecretVaultEntry, SecretVaultError> {
        let name = name.into();
        validate_name(&name)?;
        let mut value = value.into();
        if value.trim().is_empty() || value.len() > MAX_SECRET_BYTES {
            value.zeroize();
            return Err(SecretVaultError::SecretTooLarge);
        }
        if !self.contents.secrets.contains_key(&name)
            && self.contents.secrets.len() >= MAX_SECRET_COUNT
        {
            value.zeroize();
            return Err(SecretVaultError::VaultFull);
        }
        let created_at = self
            .contents
            .secrets
            .get(&name)
            .map_or(now, |secret| secret.created_at);
        self.contents.secrets.insert(
            name.clone(),
            StoredSecret {
                value,
                created_at,
                updated_at: now,
            },
        );
        self.persist()?;
        Ok(SecretVaultEntry {
            name,
            created_at,
            updated_at: now,
        })
    }

    pub fn remove(&mut self, name: &str) -> Result<bool, SecretVaultError> {
        validate_name(name)?;
        let removed = self.contents.secrets.remove(name).is_some();
        if removed {
            self.persist()?;
        }
        Ok(removed)
    }

    fn persist(&self) -> Result<(), SecretVaultError> {
        let plaintext = Zeroizing::new(
            serde_json::to_vec(&self.contents).map_err(|_| SecretVaultError::Serialization)?,
        );
        let mut salt = [0_u8; SALT_BYTES];
        let mut nonce = [0_u8; NONCE_BYTES];
        getrandom::fill(&mut salt).map_err(|_| SecretVaultError::Random)?;
        getrandom::fill(&mut nonce).map_err(|_| SecretVaultError::Random)?;
        let mut key = Zeroizing::new([0_u8; KEY_BYTES]);
        derive_key(&self.passphrase, &salt, &mut key)?;
        let cipher = XChaCha20Poly1305::new_from_slice(key.as_ref())
            .map_err(|_| SecretVaultError::InvalidPassphrase)?;
        let aad = scope_aad(&self.tenant_id, &self.workspace_id);
        let ciphertext = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext.as_slice(),
                    aad: &aad,
                },
            )
            .map_err(|_| SecretVaultError::AuthenticationFailed)?;
        let envelope = VaultEnvelope {
            format_version: VAULT_FORMAT_VERSION,
            algorithm: "xchacha20poly1305".to_owned(),
            kdf: "argon2id-v19-m65536-t3-p1".to_owned(),
            salt: STANDARD_NO_PAD.encode(salt),
            nonce: STANDARD_NO_PAD.encode(nonce),
            ciphertext: STANDARD_NO_PAD.encode(ciphertext),
        };
        let mut encoded =
            serde_json::to_vec(&envelope).map_err(|_| SecretVaultError::Serialization)?;
        encoded.push(b'\n');
        let mut file = atomic_write_file::AtomicWriteFile::open(&self.path)
            .map_err(|_| SecretVaultError::Io)?;
        set_private_permissions(file.as_file())?;
        file.write_all(&encoded).map_err(|_| SecretVaultError::Io)?;
        file.commit().map_err(|_| SecretVaultError::Io)
    }
}

fn read_contents(
    path: &Path,
    tenant_id: &TenantId,
    workspace_id: &WorkspaceId,
    passphrase: &str,
) -> Result<VaultContents, SecretVaultError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| SecretVaultError::Io)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > MAX_VAULT_BYTES
    {
        return Err(SecretVaultError::UnsafePath);
    }
    let bytes = fs::read(path).map_err(|_| SecretVaultError::Io)?;
    let envelope: VaultEnvelope =
        serde_json::from_slice(&bytes).map_err(|_| SecretVaultError::InvalidEnvelope)?;
    if envelope.format_version != VAULT_FORMAT_VERSION
        || envelope.algorithm != "xchacha20poly1305"
        || envelope.kdf != "argon2id-v19-m65536-t3-p1"
    {
        return Err(SecretVaultError::InvalidEnvelope);
    }
    let salt = decode_fixed::<SALT_BYTES>(&envelope.salt)?;
    let nonce = decode_fixed::<NONCE_BYTES>(&envelope.nonce)?;
    let ciphertext = STANDARD_NO_PAD
        .decode(envelope.ciphertext)
        .map_err(|_| SecretVaultError::InvalidEnvelope)?;
    if ciphertext.len() > MAX_VAULT_BYTES as usize {
        return Err(SecretVaultError::InvalidEnvelope);
    }
    let mut key = Zeroizing::new([0_u8; KEY_BYTES]);
    derive_key(passphrase, &salt, &mut key)?;
    let cipher = XChaCha20Poly1305::new_from_slice(key.as_ref())
        .map_err(|_| SecretVaultError::InvalidPassphrase)?;
    let aad = scope_aad(tenant_id, workspace_id);
    let plaintext = Zeroizing::new(
        cipher
            .decrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| SecretVaultError::AuthenticationFailed)?,
    );
    let contents: VaultContents =
        serde_json::from_slice(&plaintext).map_err(|_| SecretVaultError::InvalidEnvelope)?;
    if contents.tenant_id != tenant_id.as_str() || contents.workspace_id != workspace_id.as_str() {
        return Err(SecretVaultError::ScopeMismatch);
    }
    if contents.secrets.len() > MAX_SECRET_COUNT
        || contents.secrets.iter().any(|(name, secret)| {
            validate_name(name).is_err() || secret.value.len() > MAX_SECRET_BYTES
        })
    {
        return Err(SecretVaultError::InvalidEnvelope);
    }
    Ok(contents)
}

fn derive_key(
    passphrase: &str,
    salt: &[u8],
    key: &mut [u8; KEY_BYTES],
) -> Result<(), SecretVaultError> {
    validate_passphrase(passphrase)?;
    let params = Params::new(64 * 1024, 3, 1, Some(KEY_BYTES))
        .map_err(|_| SecretVaultError::InvalidPassphrase)?;
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(passphrase.as_bytes(), salt, key)
        .map_err(|_| SecretVaultError::InvalidPassphrase)
}

fn decode_fixed<const N: usize>(value: &str) -> Result<[u8; N], SecretVaultError> {
    let decoded = STANDARD_NO_PAD
        .decode(value)
        .map_err(|_| SecretVaultError::InvalidEnvelope)?;
    decoded
        .try_into()
        .map_err(|_| SecretVaultError::InvalidEnvelope)
}

fn validate_passphrase(passphrase: &str) -> Result<(), SecretVaultError> {
    if (MIN_PASSPHRASE_BYTES..=MAX_PASSPHRASE_BYTES).contains(&passphrase.len()) {
        Ok(())
    } else {
        Err(SecretVaultError::InvalidPassphrase)
    }
}

fn validate_name(name: &str) -> Result<(), SecretVaultError> {
    if !name.is_empty()
        && name.len() <= 128
        && name.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_uppercase() || (index > 0 && byte.is_ascii_digit())
        })
    {
        Ok(())
    } else {
        Err(SecretVaultError::InvalidName)
    }
}

fn scope_digest(tenant_id: &TenantId, workspace_id: &WorkspaceId) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"pandora-secret-vault-scope-v1\0");
    hasher.update(tenant_id.as_str().as_bytes());
    hasher.update([0]);
    hasher.update(workspace_id.as_str().as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn scope_aad(tenant_id: &TenantId, workspace_id: &WorkspaceId) -> Vec<u8> {
    let mut aad = b"pandora-secret-vault-v1\0".to_vec();
    aad.extend_from_slice(tenant_id.as_str().as_bytes());
    aad.push(0);
    aad.extend_from_slice(workspace_id.as_str().as_bytes());
    aad
}

fn reject_symlink_or_non_directory(path: &Path) -> Result<(), SecretVaultError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| SecretVaultError::Io)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        Err(SecretVaultError::UnsafePath)
    } else {
        Ok(())
    }
}

fn set_private_permissions(file: &fs::File) -> Result<(), SecretVaultError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| SecretVaultError::Io)?;
    }
    #[cfg(not(unix))]
    let _ = file;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn vault_encrypts_values_and_reopens() {
        let fixture = Fixture::new();
        let tenant = TenantId::new("tenant-a").unwrap();
        let workspace = WorkspaceId::new("workspace-a").unwrap();
        let mut vault = SecretVault::open(
            &fixture.root,
            tenant.clone(),
            workspace.clone(),
            "correct horse battery staple",
        )
        .unwrap();
        vault.put("PANDORA_PROVIDER_KEY", "sk-private", 10).unwrap();
        let stored = fs::read_to_string(vault.path()).unwrap();
        assert!(!stored.contains("sk-private"));
        drop(vault);
        let reopened = SecretVault::open(
            &fixture.root,
            tenant,
            workspace,
            "correct horse battery staple",
        )
        .unwrap();
        assert_eq!(
            reopened
                .get("PANDORA_PROVIDER_KEY")
                .unwrap()
                .unwrap()
                .expose(),
            "sk-private"
        );
    }

    #[test]
    fn vault_fails_closed_for_wrong_passphrase() {
        let fixture = Fixture::new();
        let tenant = TenantId::new("tenant-a").unwrap();
        let workspace = WorkspaceId::new("workspace-a").unwrap();
        let mut vault = SecretVault::open(
            &fixture.root,
            tenant.clone(),
            workspace.clone(),
            "correct horse battery staple",
        )
        .unwrap();
        vault.put("PANDORA_PROVIDER_KEY", "sk-private", 10).unwrap();
        assert!(matches!(
            SecretVault::open(
                &fixture.root,
                tenant,
                workspace,
                "wrong horse battery staple"
            ),
            Err(SecretVaultError::AuthenticationFailed)
        ));
    }

    #[test]
    fn vault_rejects_tampering() {
        let fixture = Fixture::new();
        let tenant = TenantId::new("tenant-a").unwrap();
        let workspace = WorkspaceId::new("workspace-a").unwrap();
        let mut vault = SecretVault::open(
            &fixture.root,
            tenant.clone(),
            workspace.clone(),
            "correct horse battery staple",
        )
        .unwrap();
        vault.put("PANDORA_PROVIDER_KEY", "sk-private", 10).unwrap();
        let path = vault.path().to_path_buf();
        drop(vault);
        let mut envelope: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let ciphertext = envelope["ciphertext"].as_str().unwrap();
        let mut replacement = ciphertext.as_bytes().to_vec();
        replacement[0] = if replacement[0] == b'A' { b'B' } else { b'A' };
        envelope["ciphertext"] = serde_json::Value::String(String::from_utf8(replacement).unwrap());
        fs::write(&path, serde_json::to_vec(&envelope).unwrap()).unwrap();
        assert!(matches!(
            SecretVault::open(
                &fixture.root,
                tenant,
                workspace,
                "correct horse battery staple"
            ),
            Err(SecretVaultError::AuthenticationFailed)
        ));
    }

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "pandora-secret-vault-{}-{suffix}",
                std::process::id()
            ));
            fs::create_dir_all(&root).unwrap();
            Self { root }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
