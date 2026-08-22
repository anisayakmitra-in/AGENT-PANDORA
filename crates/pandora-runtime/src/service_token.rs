use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use subtle::ConstantTimeEq;

const SERVICE_TOKEN_FILE_NAME: &str = "service-token";
const SERVICE_TOKEN_BYTES: usize = 32;
const SERVICE_TOKEN_HEX_LENGTH: usize = SERVICE_TOKEN_BYTES * 2;

pub struct ServiceToken([u8; SERVICE_TOKEN_BYTES]);

impl ServiceToken {
    pub fn matches(&self, candidate: &str) -> bool {
        let Some(candidate) = decode_token(candidate.as_bytes()) else {
            return false;
        };
        self.0.ct_eq(&candidate).into()
    }
}

pub struct ServiceTokenStore {
    token: ServiceToken,
    path: PathBuf,
}

impl ServiceTokenStore {
    pub fn load_or_create(data_dir: impl AsRef<Path>) -> Result<Self, ServiceTokenError> {
        let data_dir = data_dir.as_ref();
        fs::create_dir_all(data_dir)?;
        let path = data_dir.join(SERVICE_TOKEN_FILE_NAME);
        let token = match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(ServiceTokenError::UnsafePath);
                }
                ServiceToken(read_token(&path)?)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let mut token = [0; SERVICE_TOKEN_BYTES];
                getrandom::fill(&mut token)?;
                write_token(&path, &token)?;
                ServiceToken(token)
            }
            Err(error) => return Err(ServiceTokenError::Io(error)),
        };
        Ok(Self { token, path })
    }

    pub fn token(&self) -> &ServiceToken {
        &self.token
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Debug)]
pub enum ServiceTokenError {
    Io(io::Error),
    Random(getrandom::Error),
    UnsafePath,
    InvalidToken,
}

impl fmt::Display for ServiceTokenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(_) => formatter.write_str("could not access the local service token"),
            Self::Random(_) => formatter.write_str("could not generate the local service token"),
            Self::UnsafePath => formatter.write_str("local service token path is unsafe"),
            Self::InvalidToken => formatter.write_str("local service token is invalid"),
        }
    }
}

impl std::error::Error for ServiceTokenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Random(error) => Some(error),
            Self::UnsafePath | Self::InvalidToken => None,
        }
    }
}

impl From<io::Error> for ServiceTokenError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<getrandom::Error> for ServiceTokenError {
    fn from(error: getrandom::Error) -> Self {
        Self::Random(error)
    }
}

fn read_token(path: &Path) -> Result<[u8; SERVICE_TOKEN_BYTES], ServiceTokenError> {
    let bytes = fs::read(path)?;
    decode_token(&bytes).ok_or(ServiceTokenError::InvalidToken)
}

fn write_token(path: &Path, token: &[u8; SERVICE_TOKEN_BYTES]) -> Result<(), ServiceTokenError> {
    let encoded = encode_token(token);
    let mut file = atomic_write_file::AtomicWriteFile::open(path)?;
    set_private_permissions(file.as_file())?;
    file.write_all(encoded.as_bytes())?;
    file.commit()?;
    Ok(())
}

fn decode_token(value: &[u8]) -> Option<[u8; SERVICE_TOKEN_BYTES]> {
    if value.len() != SERVICE_TOKEN_HEX_LENGTH {
        return None;
    }
    let mut token = [0; SERVICE_TOKEN_BYTES];
    for (index, byte) in token.iter_mut().enumerate() {
        let high = decode_hex_digit(value[index * 2])?;
        let low = decode_hex_digit(value[index * 2 + 1])?;
        *byte = (high << 4) | low;
    }
    Some(token)
}

const fn decode_hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn encode_token(token: &[u8; SERVICE_TOKEN_BYTES]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(SERVICE_TOKEN_HEX_LENGTH);
    for byte in token {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn set_private_permissions(file: &std::fs::File) -> Result<(), ServiceTokenError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = file;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ServiceTokenError, ServiceTokenStore};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn token_store_reuses_an_existing_token_without_exposing_it() {
        let fixture = Fixture::new();
        let first = ServiceTokenStore::load_or_create(&fixture.root).unwrap();
        let stored = std::fs::read_to_string(first.path()).unwrap();
        let second = ServiceTokenStore::load_or_create(&fixture.root).unwrap();

        assert!(first.token().matches(&stored));
        assert!(second.token().matches(&stored));
    }

    #[test]
    fn token_store_rejects_an_unsafe_token_path() {
        let fixture = Fixture::new();
        std::fs::create_dir(fixture.root.join("service-token")).unwrap();

        assert!(matches!(
            ServiceTokenStore::load_or_create(&fixture.root),
            Err(ServiceTokenError::UnsafePath)
        ));
    }

    #[test]
    fn token_store_rejects_noncanonical_existing_token_contents() {
        let fixture = Fixture::new();
        std::fs::write(fixture.root.join("service-token"), "A".repeat(64)).unwrap();

        assert!(matches!(
            ServiceTokenStore::load_or_create(&fixture.root),
            Err(ServiceTokenError::InvalidToken)
        ));
    }

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "pandora-service-token-{}-{timestamp}",
                std::process::id()
            ));
            std::fs::create_dir(&root).unwrap();
            Self { root }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}
