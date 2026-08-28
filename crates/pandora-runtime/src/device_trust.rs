use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use zeroize::Zeroize;

const PRIVATE_KEY_BYTES: usize = 32;
const PRIVATE_KEY_HEX_BYTES: usize = PRIVATE_KEY_BYTES * 2;
const SIGNATURE_BYTES: usize = 64;
const TOKEN_HEX_BYTES: usize = 64;
const NONCE_HEX_BYTES: usize = 32;

#[derive(Debug)]
pub enum DeviceKeyError {
    Io,
    UnsafePath,
    InvalidKey,
    InvalidProof,
    Random,
}

impl fmt::Display for DeviceKeyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Io => "could not access the device key",
            Self::UnsafePath => "device key path is unsafe",
            Self::InvalidKey => "device key is invalid",
            Self::InvalidProof => "device proof input is invalid",
            Self::Random => "device key randomness is unavailable",
        })
    }
}

impl std::error::Error for DeviceKeyError {}

pub struct DeviceKeyStore {
    path: PathBuf,
    signing_key: SigningKey,
}

pub struct DeviceProofRequest<'a> {
    token: &'a str,
    timestamp: u64,
    nonce: &'a str,
    method: &'a str,
    path: &'a str,
    body: &'a [u8],
}

impl<'a> DeviceProofRequest<'a> {
    pub const fn new(
        token: &'a str,
        timestamp: u64,
        nonce: &'a str,
        method: &'a str,
        path: &'a str,
        body: &'a [u8],
    ) -> Self {
        Self {
            token,
            timestamp,
            nonce,
            method,
            path,
            body,
        }
    }
}

impl DeviceKeyStore {
    pub fn load_or_create(path: impl AsRef<Path>) -> Result<Self, DeviceKeyError> {
        let path = path.as_ref().to_path_buf();
        let signing_key = match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink()
                    || !metadata.is_file()
                    || metadata.len() != PRIVATE_KEY_HEX_BYTES as u64
                {
                    return Err(DeviceKeyError::UnsafePath);
                }
                let encoded = fs::read(&path).map_err(|_| DeviceKeyError::Io)?;
                let mut private = decode_fixed::<PRIVATE_KEY_BYTES>(&encoded)
                    .ok_or(DeviceKeyError::InvalidKey)?;
                let key = SigningKey::from_bytes(&private);
                private.zeroize();
                key
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).map_err(|_| DeviceKeyError::Io)?;
                }
                let mut private = [0_u8; PRIVATE_KEY_BYTES];
                getrandom::fill(&mut private).map_err(|_| DeviceKeyError::Random)?;
                let signing_key = SigningKey::from_bytes(&private);
                let encoded = encode_hex(&private);
                private.zeroize();
                let mut file = atomic_write_file::AtomicWriteFile::open(&path)
                    .map_err(|_| DeviceKeyError::Io)?;
                set_private_permissions(file.as_file())?;
                file.write_all(encoded.as_bytes())
                    .map_err(|_| DeviceKeyError::Io)?;
                file.commit().map_err(|_| DeviceKeyError::Io)?;
                signing_key
            }
            Err(_) => return Err(DeviceKeyError::Io),
        };
        Ok(Self { path, signing_key })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn public_key(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }

    pub fn device_id(&self) -> String {
        let digest = Sha256::digest(self.public_key());
        format!("device-{}", encode_hex(&digest[..16]))
    }

    pub fn sign(&self, request: &DeviceProofRequest<'_>) -> Result<String, DeviceKeyError> {
        let message = device_proof_message(request)?;
        Ok(encode_hex(&self.signing_key.sign(&message).to_bytes()))
    }
}

pub fn verify_device_proof(
    public_key: &[u8; 32],
    request: &DeviceProofRequest<'_>,
    signature: &str,
) -> bool {
    let Ok(verifying_key) = VerifyingKey::from_bytes(public_key) else {
        return false;
    };
    let Ok(message) = device_proof_message(request) else {
        return false;
    };
    let Some(signature) = decode_fixed::<SIGNATURE_BYTES>(signature.as_bytes()) else {
        return false;
    };
    verifying_key
        .verify(&message, &Signature::from_bytes(&signature))
        .is_ok()
}

pub fn device_proof_message(request: &DeviceProofRequest<'_>) -> Result<Vec<u8>, DeviceKeyError> {
    let DeviceProofRequest {
        token,
        timestamp,
        nonce,
        method,
        path,
        body,
    } = request;
    if token.len() != TOKEN_HEX_BYTES
        || nonce.len() != NONCE_HEX_BYTES
        || !is_lower_hex(token.as_bytes())
        || !is_lower_hex(nonce.as_bytes())
        || method.is_empty()
        || method.len() > 16
        || !method.bytes().all(|byte| byte.is_ascii_uppercase())
        || path.is_empty()
        || path.len() > 256
        || !path.starts_with('/')
        || path.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(DeviceKeyError::InvalidProof);
    }
    let token_digest = Sha256::digest(token.as_bytes());
    let body_digest = Sha256::digest(body);
    let mut message = b"pandora-device-proof-v2\0".to_vec();
    message.extend_from_slice(timestamp.to_string().as_bytes());
    message.push(0);
    message.extend_from_slice(nonce.as_bytes());
    message.push(0);
    message.extend_from_slice(method.as_bytes());
    message.push(0);
    message.extend_from_slice(path.as_bytes());
    message.push(0);
    message.extend_from_slice(&token_digest);
    message.extend_from_slice(&body_digest);
    Ok(message)
}

fn decode_fixed<const N: usize>(value: &[u8]) -> Option<[u8; N]> {
    if value.len() != N * 2 || !is_lower_hex(value) {
        return None;
    }
    let mut decoded = [0_u8; N];
    for (index, byte) in decoded.iter_mut().enumerate() {
        *byte = decode_hex(value[index * 2])? << 4 | decode_hex(value[index * 2 + 1])?;
    }
    Some(decoded)
}

fn decode_hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn is_lower_hex(value: &[u8]) -> bool {
    value
        .iter()
        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
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

fn set_private_permissions(file: &fs::File) -> Result<(), DeviceKeyError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| DeviceKeyError::Io)?;
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
    fn proof_binds_every_request_dimension() {
        let root = fixture_root();
        let key = DeviceKeyStore::load_or_create(root.join("device-key")).unwrap();
        let token = "a".repeat(64);
        let nonce = "b".repeat(32);
        let request = DeviceProofRequest::new(&token, 10, &nonce, "POST", "/v1/rpc", b"request");
        let signature = key.sign(&request).unwrap();
        assert!(verify_device_proof(&key.public_key(), &request, &signature,));
        let changed_time =
            DeviceProofRequest::new(&token, 11, &nonce, "POST", "/v1/rpc", b"request");
        assert!(!verify_device_proof(
            &key.public_key(),
            &changed_time,
            &signature,
        ));
        let changed_body =
            DeviceProofRequest::new(&token, 10, &nonce, "POST", "/v1/rpc", b"changed");
        assert!(!verify_device_proof(
            &key.public_key(),
            &changed_body,
            &signature,
        ));
        let _ = fs::remove_dir_all(root);
    }

    fn fixture_root() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "pandora-device-key-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
