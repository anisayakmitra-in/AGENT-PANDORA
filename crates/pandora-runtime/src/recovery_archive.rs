use argon2::{Algorithm, Argon2, Params, Version};
use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt;
use std::path::{Component, Path};
use zeroize::{Zeroize, Zeroizing};

const FORMAT_VERSION: u32 = 1;
const MAX_ARCHIVE_BYTES: usize = 128 * 1024 * 1024;
const MAX_ENTRY_BYTES: usize = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 2_048;
const MIN_PASSPHRASE_BYTES: usize = 16;
const MAX_PASSPHRASE_BYTES: usize = 4 * 1024;
const SALT_BYTES: usize = 16;
const NONCE_BYTES: usize = 24;
const KEY_BYTES: usize = 32;
const AAD: &[u8] = b"pandora-recovery-archive-v1";

#[derive(Debug)]
pub enum RecoveryArchiveError {
    InvalidPassphrase,
    InvalidPath,
    DuplicatePath,
    TooLarge,
    TooManyEntries,
    InvalidEnvelope,
    AuthenticationFailed,
    IntegrityFailed,
    Serialization,
    Random,
}

impl fmt::Display for RecoveryArchiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidPassphrase => "recovery passphrase is invalid",
            Self::InvalidPath => "recovery archive contains an invalid path",
            Self::DuplicatePath => "recovery archive contains duplicate paths",
            Self::TooLarge => "recovery archive exceeds the size limit",
            Self::TooManyEntries => "recovery archive contains too many entries",
            Self::InvalidEnvelope => "recovery archive envelope is invalid",
            Self::AuthenticationFailed => "recovery archive authentication failed",
            Self::IntegrityFailed => "recovery archive integrity check failed",
            Self::Serialization => "recovery archive could not be encoded",
            Self::Random => "recovery archive randomness is unavailable",
        })
    }
}

impl std::error::Error for RecoveryArchiveError {}

pub struct RecoveryEntry {
    path: String,
    bytes: Zeroizing<Vec<u8>>,
}

impl RecoveryEntry {
    pub fn new(path: impl Into<String>, bytes: Vec<u8>) -> Result<Self, RecoveryArchiveError> {
        let path = path.into();
        validate_path(&path)?;
        if bytes.len() > MAX_ENTRY_BYTES {
            return Err(RecoveryArchiveError::TooLarge);
        }
        Ok(Self {
            path,
            bytes: Zeroizing::new(bytes),
        })
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

pub struct RecoveryBundle {
    created_at: u64,
    entries: Vec<RecoveryEntry>,
}

impl RecoveryBundle {
    pub const fn created_at(&self) -> u64 {
        self.created_at
    }

    pub fn entries(&self) -> &[RecoveryEntry] {
        &self.entries
    }

    pub fn into_entries(self) -> Vec<RecoveryEntry> {
        self.entries
    }
}

pub struct RecoveryArchive;

impl RecoveryArchive {
    pub fn seal(
        entries: Vec<RecoveryEntry>,
        passphrase: &str,
        created_at: u64,
    ) -> Result<Vec<u8>, RecoveryArchiveError> {
        validate_passphrase(passphrase)?;
        validate_entries(&entries)?;
        let records = entries
            .iter()
            .map(|entry| EntryRecord {
                path: entry.path.clone(),
                sha256: digest(entry.bytes()),
                data: STANDARD_NO_PAD.encode(entry.bytes()),
            })
            .collect();
        let plaintext = Zeroizing::new(
            serde_json::to_vec(&ArchiveContents {
                format_version: FORMAT_VERSION,
                created_at,
                entries: records,
            })
            .map_err(|_| RecoveryArchiveError::Serialization)?,
        );
        if plaintext.len() > MAX_ARCHIVE_BYTES {
            return Err(RecoveryArchiveError::TooLarge);
        }
        let mut salt = [0_u8; SALT_BYTES];
        let mut nonce = [0_u8; NONCE_BYTES];
        getrandom::fill(&mut salt).map_err(|_| RecoveryArchiveError::Random)?;
        getrandom::fill(&mut nonce).map_err(|_| RecoveryArchiveError::Random)?;
        let mut key = Zeroizing::new([0_u8; KEY_BYTES]);
        derive_key(passphrase, &salt, &mut key)?;
        let cipher = XChaCha20Poly1305::new_from_slice(key.as_ref())
            .map_err(|_| RecoveryArchiveError::InvalidPassphrase)?;
        let ciphertext = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &plaintext,
                    aad: AAD,
                },
            )
            .map_err(|_| RecoveryArchiveError::AuthenticationFailed)?;
        serde_json::to_vec(&ArchiveEnvelope {
            format_version: FORMAT_VERSION,
            algorithm: "xchacha20poly1305".to_owned(),
            kdf: "argon2id-v19-m65536-t3-p1".to_owned(),
            salt: STANDARD_NO_PAD.encode(salt),
            nonce: STANDARD_NO_PAD.encode(nonce),
            ciphertext: STANDARD_NO_PAD.encode(ciphertext),
        })
        .map_err(|_| RecoveryArchiveError::Serialization)
    }

    pub fn open(encoded: &[u8], passphrase: &str) -> Result<RecoveryBundle, RecoveryArchiveError> {
        validate_passphrase(passphrase)?;
        if encoded.len() > MAX_ARCHIVE_BYTES {
            return Err(RecoveryArchiveError::TooLarge);
        }
        let envelope: ArchiveEnvelope =
            serde_json::from_slice(encoded).map_err(|_| RecoveryArchiveError::InvalidEnvelope)?;
        if envelope.format_version != FORMAT_VERSION
            || envelope.algorithm != "xchacha20poly1305"
            || envelope.kdf != "argon2id-v19-m65536-t3-p1"
        {
            return Err(RecoveryArchiveError::InvalidEnvelope);
        }
        let salt = decode_fixed::<SALT_BYTES>(&envelope.salt)?;
        let nonce = decode_fixed::<NONCE_BYTES>(&envelope.nonce)?;
        let ciphertext = STANDARD_NO_PAD
            .decode(envelope.ciphertext)
            .map_err(|_| RecoveryArchiveError::InvalidEnvelope)?;
        if ciphertext.len() > MAX_ARCHIVE_BYTES {
            return Err(RecoveryArchiveError::TooLarge);
        }
        let mut key = Zeroizing::new([0_u8; KEY_BYTES]);
        derive_key(passphrase, &salt, &mut key)?;
        let cipher = XChaCha20Poly1305::new_from_slice(key.as_ref())
            .map_err(|_| RecoveryArchiveError::InvalidPassphrase)?;
        let plaintext = Zeroizing::new(
            cipher
                .decrypt(
                    XNonce::from_slice(&nonce),
                    Payload {
                        msg: &ciphertext,
                        aad: AAD,
                    },
                )
                .map_err(|_| RecoveryArchiveError::AuthenticationFailed)?,
        );
        let contents: ArchiveContents = serde_json::from_slice(&plaintext)
            .map_err(|_| RecoveryArchiveError::InvalidEnvelope)?;
        if contents.format_version != FORMAT_VERSION || contents.entries.len() > MAX_ENTRIES {
            return Err(RecoveryArchiveError::InvalidEnvelope);
        }
        let mut paths = BTreeSet::new();
        let mut entries = Vec::with_capacity(contents.entries.len());
        for record in contents.entries {
            validate_path(&record.path)?;
            if !paths.insert(record.path.clone()) {
                return Err(RecoveryArchiveError::DuplicatePath);
            }
            let mut bytes = STANDARD_NO_PAD
                .decode(record.data)
                .map_err(|_| RecoveryArchiveError::InvalidEnvelope)?;
            if bytes.len() > MAX_ENTRY_BYTES || digest(&bytes) != record.sha256 {
                bytes.zeroize();
                return Err(RecoveryArchiveError::IntegrityFailed);
            }
            entries.push(RecoveryEntry::new(record.path, bytes)?);
        }
        validate_entries(&entries)?;
        Ok(RecoveryBundle {
            created_at: contents.created_at,
            entries,
        })
    }
}

#[derive(Serialize, Deserialize)]
struct ArchiveEnvelope {
    format_version: u32,
    algorithm: String,
    kdf: String,
    salt: String,
    nonce: String,
    ciphertext: String,
}

#[derive(Serialize, Deserialize)]
struct ArchiveContents {
    format_version: u32,
    created_at: u64,
    entries: Vec<EntryRecord>,
}

#[derive(Serialize, Deserialize)]
struct EntryRecord {
    path: String,
    sha256: String,
    data: String,
}

fn validate_entries(entries: &[RecoveryEntry]) -> Result<(), RecoveryArchiveError> {
    if entries.len() > MAX_ENTRIES {
        return Err(RecoveryArchiveError::TooManyEntries);
    }
    let mut paths = BTreeSet::new();
    let mut total = 0_usize;
    for entry in entries {
        validate_path(entry.path())?;
        if !paths.insert(entry.path()) {
            return Err(RecoveryArchiveError::DuplicatePath);
        }
        total = total
            .checked_add(entry.bytes().len())
            .ok_or(RecoveryArchiveError::TooLarge)?;
    }
    if total > MAX_ARCHIVE_BYTES {
        return Err(RecoveryArchiveError::TooLarge);
    }
    Ok(())
}

fn validate_path(value: &str) -> Result<(), RecoveryArchiveError> {
    let path = Path::new(value);
    if value.is_empty()
        || value.len() > 512
        || path.is_absolute()
        || !matches!(
            path.components().next(),
            Some(Component::Normal(first)) if first == "config" || first == "data"
        )
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(RecoveryArchiveError::InvalidPath);
    }
    Ok(())
}

fn derive_key(
    passphrase: &str,
    salt: &[u8],
    key: &mut [u8; KEY_BYTES],
) -> Result<(), RecoveryArchiveError> {
    validate_passphrase(passphrase)?;
    let params = Params::new(64 * 1024, 3, 1, Some(KEY_BYTES))
        .map_err(|_| RecoveryArchiveError::InvalidPassphrase)?;
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(passphrase.as_bytes(), salt, key)
        .map_err(|_| RecoveryArchiveError::InvalidPassphrase)
}

fn validate_passphrase(passphrase: &str) -> Result<(), RecoveryArchiveError> {
    if (MIN_PASSPHRASE_BYTES..=MAX_PASSPHRASE_BYTES).contains(&passphrase.len()) {
        Ok(())
    } else {
        Err(RecoveryArchiveError::InvalidPassphrase)
    }
}

fn decode_fixed<const N: usize>(value: &str) -> Result<[u8; N], RecoveryArchiveError> {
    STANDARD_NO_PAD
        .decode(value)
        .map_err(|_| RecoveryArchiveError::InvalidEnvelope)?
        .try_into()
        .map_err(|_| RecoveryArchiveError::InvalidEnvelope)
}

fn digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{}", encode_hex(&digest))
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypted_archive_round_trips_and_rejects_tampering() {
        let encoded = RecoveryArchive::seal(
            vec![RecoveryEntry::new("data/sessions.sqlite3", b"state".to_vec()).unwrap()],
            "correct horse battery staple",
            10,
        )
        .unwrap();
        assert!(!encoded.windows(5).any(|window| window == b"state"));
        let opened = RecoveryArchive::open(&encoded, "correct horse battery staple").unwrap();
        assert_eq!(opened.entries()[0].bytes(), b"state");
        let mut envelope: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        let ciphertext = envelope["ciphertext"].as_str().unwrap();
        let mut changed = ciphertext.as_bytes().to_vec();
        changed[0] = if changed[0] == b'A' { b'B' } else { b'A' };
        envelope["ciphertext"] = serde_json::Value::String(String::from_utf8(changed).unwrap());
        assert!(
            RecoveryArchive::open(
                &serde_json::to_vec(&envelope).unwrap(),
                "correct horse battery staple"
            )
            .is_err()
        );
    }

    #[test]
    fn archive_paths_cannot_escape_restore_roots() {
        assert!(RecoveryEntry::new("data/../secret", Vec::new()).is_err());
        assert!(RecoveryEntry::new("/data/secret", Vec::new()).is_err());
    }
}
