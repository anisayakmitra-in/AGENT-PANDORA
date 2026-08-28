use pandora_types::{PrincipalId, TenantId, WorkspaceId};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const TOKEN_BYTES: usize = 32;
const TOKEN_HEX_BYTES: usize = TOKEN_BYTES * 2;
const ID_BYTES: usize = 16;
const MAX_DEVICE_ID_BYTES: usize = 128;
const DEVICE_PUBLIC_KEY_BYTES: usize = 32;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessRole {
    Viewer,
    Operator,
    Administrator,
}

impl AccessRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Operator => "operator",
            Self::Administrator => "administrator",
        }
    }

    pub fn parse(value: &str) -> Result<Self, IdentityStoreError> {
        match value {
            "viewer" => Ok(Self::Viewer),
            "operator" => Ok(Self::Operator),
            "administrator" => Ok(Self::Administrator),
            _ => Err(IdentityStoreError::CorruptRecord),
        }
    }
}

pub struct IdentityEnrollmentRequest {
    principal_id: PrincipalId,
    tenant_id: TenantId,
    workspace_id: WorkspaceId,
    role: AccessRole,
    now: u64,
}

impl IdentityEnrollmentRequest {
    pub fn new(
        principal_id: PrincipalId,
        tenant_id: TenantId,
        workspace_id: WorkspaceId,
        role: AccessRole,
        now: u64,
    ) -> Self {
        Self {
            principal_id,
            tenant_id,
            workspace_id,
            role,
            now,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceIdentity {
    id: String,
    principal_id: PrincipalId,
    tenant_id: TenantId,
    workspace_id: WorkspaceId,
    role: AccessRole,
    device_id: String,
    #[serde(skip)]
    device_public_key: [u8; DEVICE_PUBLIC_KEY_BYTES],
    created_at: u64,
    revoked_at: Option<u64>,
}

impl ServiceIdentity {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn principal_id(&self) -> &PrincipalId {
        &self.principal_id
    }

    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    pub fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }

    pub const fn role(&self) -> AccessRole {
        self.role
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    pub const fn device_public_key(&self) -> &[u8; DEVICE_PUBLIC_KEY_BYTES] {
        &self.device_public_key
    }

    pub const fn created_at(&self) -> u64 {
        self.created_at
    }

    pub const fn revoked_at(&self) -> Option<u64> {
        self.revoked_at
    }
}

pub struct IdentityEnrollment {
    identity: ServiceIdentity,
    token: String,
}

impl IdentityEnrollment {
    pub fn identity(&self) -> &ServiceIdentity {
        &self.identity
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn write_token_file(&self, path: impl AsRef<Path>) -> Result<(), IdentityStoreError> {
        let path = path.as_ref();
        if let Ok(metadata) = fs::symlink_metadata(path)
            && (metadata.file_type().is_symlink() || !metadata.is_file())
        {
            return Err(IdentityStoreError::UnsafePath);
        }
        let mut file = atomic_write_file::AtomicWriteFile::open(path)
            .map_err(|_| IdentityStoreError::Database)?;
        set_private_permissions(file.as_file())?;
        file.write_all(self.token.as_bytes())
            .map_err(|_| IdentityStoreError::Database)?;
        file.commit().map_err(|_| IdentityStoreError::Database)
    }
}

impl fmt::Debug for IdentityEnrollment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IdentityEnrollment")
            .field("identity", &self.identity)
            .field("token", &"[redacted]")
            .finish()
    }
}

#[derive(Debug)]
pub enum IdentityStoreError {
    Database,
    Random,
    InvalidDevice,
    InvalidToken,
    InvalidIdentifier,
    Duplicate,
    NotFound,
    CorruptRecord,
    UnsafePath,
}

impl fmt::Display for IdentityStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Database => "identity store is unavailable",
            Self::Random => "identity credential randomness is unavailable",
            Self::InvalidDevice => "device identifier is invalid",
            Self::InvalidToken => "identity credential is invalid",
            Self::InvalidIdentifier => "identity scope is invalid",
            Self::Duplicate => "identity already exists",
            Self::NotFound => "identity was not found",
            Self::CorruptRecord => "identity store contains an invalid record",
            Self::UnsafePath => "identity credential path is unsafe",
        })
    }
}

impl std::error::Error for IdentityStoreError {}

#[derive(Clone)]
pub struct IdentityStore {
    path: PathBuf,
}

impl IdentityStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, IdentityStoreError> {
        let path = path.as_ref().to_path_buf();
        let connection = Connection::open(&path).map_err(|_| IdentityStoreError::Database)?;
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE IF NOT EXISTS service_identities (
                     id TEXT PRIMARY KEY,
                     principal_id TEXT NOT NULL,
                     tenant_id TEXT NOT NULL,
                     workspace_id TEXT NOT NULL,
                     role TEXT NOT NULL,
                     device_id TEXT NOT NULL,
                     device_public_key BLOB NOT NULL,
                     token_digest BLOB NOT NULL UNIQUE,
                     created_at INTEGER NOT NULL,
                     revoked_at INTEGER
                 );
                 CREATE INDEX IF NOT EXISTS service_identity_scope
                 ON service_identities (tenant_id, workspace_id, principal_id);",
            )
            .map_err(|_| IdentityStoreError::Database)?;
        Ok(Self { path })
    }

    pub fn enroll(
        &self,
        request: IdentityEnrollmentRequest,
        device_id: impl Into<String>,
        device_public_key: [u8; DEVICE_PUBLIC_KEY_BYTES],
    ) -> Result<IdentityEnrollment, IdentityStoreError> {
        let IdentityEnrollmentRequest {
            principal_id,
            tenant_id,
            workspace_id,
            role,
            now,
        } = request;
        let device_id = device_id.into();
        validate_device_id(&device_id)?;
        let mut token_bytes = [0_u8; TOKEN_BYTES];
        let mut id_bytes = [0_u8; ID_BYTES];
        getrandom::fill(&mut token_bytes).map_err(|_| IdentityStoreError::Random)?;
        getrandom::fill(&mut id_bytes).map_err(|_| IdentityStoreError::Random)?;
        let token = encode_hex(&token_bytes);
        let id = format!("identity-{}", encode_hex(&id_bytes));
        let digest = token_digest(&token)?;
        let created_at = i64::try_from(now).map_err(|_| IdentityStoreError::Database)?;
        let connection = Connection::open(&self.path).map_err(|_| IdentityStoreError::Database)?;
        connection
            .execute(
                "INSERT INTO service_identities (
                    id, principal_id, tenant_id, workspace_id, role, device_id,
                    device_public_key, token_digest, created_at, revoked_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL)",
                params![
                    id,
                    principal_id.as_str(),
                    tenant_id.as_str(),
                    workspace_id.as_str(),
                    role.as_str(),
                    device_id,
                    device_public_key.as_slice(),
                    digest.as_slice(),
                    created_at,
                ],
            )
            .map_err(|error| {
                if error.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation) {
                    IdentityStoreError::Duplicate
                } else {
                    IdentityStoreError::Database
                }
            })?;
        Ok(IdentityEnrollment {
            identity: ServiceIdentity {
                id,
                principal_id,
                tenant_id,
                workspace_id,
                role,
                device_id,
                device_public_key,
                created_at: now,
                revoked_at: None,
            },
            token,
        })
    }

    pub fn authenticate(
        &self,
        token: &str,
        device_id: &str,
    ) -> Result<Option<ServiceIdentity>, IdentityStoreError> {
        validate_device_id(device_id)?;
        let digest = token_digest(token)?;
        let connection = Connection::open(&self.path).map_err(|_| IdentityStoreError::Database)?;
        let raw = connection
            .query_row(
                "SELECT id, principal_id, tenant_id, workspace_id, role, device_id,
                        device_public_key, created_at, revoked_at
                 FROM service_identities
                 WHERE token_digest = ?1 AND revoked_at IS NULL",
                [digest.as_slice()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Vec<u8>>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, Option<i64>>(8)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| IdentityStoreError::Database)?;
        let Some((
            id,
            principal,
            tenant,
            workspace,
            role,
            bound_device,
            public_key,
            created,
            revoked,
        )) = raw
        else {
            return Ok(None);
        };
        if bound_device != device_id {
            return Ok(None);
        }
        Ok(Some(ServiceIdentity {
            id,
            principal_id: PrincipalId::new(principal)
                .map_err(|_| IdentityStoreError::CorruptRecord)?,
            tenant_id: TenantId::new(tenant).map_err(|_| IdentityStoreError::CorruptRecord)?,
            workspace_id: WorkspaceId::new(workspace)
                .map_err(|_| IdentityStoreError::CorruptRecord)?,
            role: AccessRole::parse(&role)?,
            device_id: bound_device,
            device_public_key: public_key
                .try_into()
                .map_err(|_| IdentityStoreError::CorruptRecord)?,
            created_at: u64::try_from(created).map_err(|_| IdentityStoreError::CorruptRecord)?,
            revoked_at: revoked
                .map(u64::try_from)
                .transpose()
                .map_err(|_| IdentityStoreError::CorruptRecord)?,
        }))
    }

    pub fn list(&self) -> Result<Vec<ServiceIdentity>, IdentityStoreError> {
        let connection = Connection::open(&self.path).map_err(|_| IdentityStoreError::Database)?;
        let mut statement = connection
            .prepare(
                "SELECT id, principal_id, tenant_id, workspace_id, role, device_id,
                        device_public_key, created_at, revoked_at
                 FROM service_identities
                 ORDER BY created_at ASC, id ASC",
            )
            .map_err(|_| IdentityStoreError::Database)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Vec<u8>>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, Option<i64>>(8)?,
                ))
            })
            .map_err(|_| IdentityStoreError::Database)?;
        rows.map(|row| {
            let (id, principal, tenant, workspace, role, device, public_key, created, revoked) =
                row.map_err(|_| IdentityStoreError::Database)?;
            Ok(ServiceIdentity {
                id,
                principal_id: PrincipalId::new(principal)
                    .map_err(|_| IdentityStoreError::CorruptRecord)?,
                tenant_id: TenantId::new(tenant).map_err(|_| IdentityStoreError::CorruptRecord)?,
                workspace_id: WorkspaceId::new(workspace)
                    .map_err(|_| IdentityStoreError::CorruptRecord)?,
                role: AccessRole::parse(&role)?,
                device_id: device,
                device_public_key: public_key
                    .try_into()
                    .map_err(|_| IdentityStoreError::CorruptRecord)?,
                created_at: u64::try_from(created)
                    .map_err(|_| IdentityStoreError::CorruptRecord)?,
                revoked_at: revoked
                    .map(u64::try_from)
                    .transpose()
                    .map_err(|_| IdentityStoreError::CorruptRecord)?,
            })
        })
        .collect()
    }

    pub fn revoke(&self, identity_id: &str, now: u64) -> Result<(), IdentityStoreError> {
        if !identity_id.starts_with("identity-") || identity_id.len() != 9 + ID_BYTES * 2 {
            return Err(IdentityStoreError::InvalidIdentifier);
        }
        let connection = Connection::open(&self.path).map_err(|_| IdentityStoreError::Database)?;
        let changed = connection
            .execute(
                "UPDATE service_identities
                 SET revoked_at = ?1
                 WHERE id = ?2 AND revoked_at IS NULL",
                params![
                    i64::try_from(now).map_err(|_| IdentityStoreError::Database)?,
                    identity_id
                ],
            )
            .map_err(|_| IdentityStoreError::Database)?;
        if changed == 1 {
            Ok(())
        } else {
            Err(IdentityStoreError::NotFound)
        }
    }
}

fn validate_device_id(value: &str) -> Result<(), IdentityStoreError> {
    if !value.is_empty()
        && value.len() <= MAX_DEVICE_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        Ok(())
    } else {
        Err(IdentityStoreError::InvalidDevice)
    }
}

fn token_digest(token: &str) -> Result<[u8; 32], IdentityStoreError> {
    if token.len() != TOKEN_HEX_BYTES
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(IdentityStoreError::InvalidToken);
    }
    let mut hasher = Sha256::new();
    hasher.update(b"pandora-service-identity-token-v1\0");
    hasher.update(token.as_bytes());
    Ok(hasher.finalize().into())
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

fn set_private_permissions(file: &fs::File) -> Result<(), IdentityStoreError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| IdentityStoreError::Database)?;
    }
    #[cfg(not(unix))]
    let _ = file;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DeviceKeyStore;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn credentials_are_hash_only_device_bound_and_revocable() {
        let fixture = Fixture::new();
        let store = IdentityStore::open(fixture.root.join("identities.sqlite3")).unwrap();
        let device = DeviceKeyStore::load_or_create(fixture.root.join("device.key")).unwrap();
        let enrollment = store
            .enroll(
                IdentityEnrollmentRequest::new(
                    PrincipalId::new("alice").unwrap(),
                    TenantId::new("tenant-a").unwrap(),
                    WorkspaceId::new("workspace-a").unwrap(),
                    AccessRole::Operator,
                    10,
                ),
                device.device_id(),
                device.public_key(),
            )
            .unwrap();
        let database = std::fs::read(fixture.root.join("identities.sqlite3")).unwrap();
        assert!(
            !database
                .windows(enrollment.token().len())
                .any(|window| window == enrollment.token().as_bytes())
        );
        assert!(
            store
                .authenticate(enrollment.token(), "device-b")
                .unwrap()
                .is_none()
        );
        let identity = store
            .authenticate(enrollment.token(), &device.device_id())
            .unwrap()
            .unwrap();
        assert_eq!(identity.principal_id().as_str(), "alice");
        assert_eq!(identity.role(), AccessRole::Operator);
        assert_eq!(identity.device_public_key(), &device.public_key());
        store.revoke(identity.id(), 20).unwrap();
        assert!(
            store
                .authenticate(enrollment.token(), &device.device_id())
                .unwrap()
                .is_none()
        );
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
                "pandora-identities-{}-{suffix}",
                std::process::id()
            ));
            std::fs::create_dir_all(&root).unwrap();
            Self { root }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}
