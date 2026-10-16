use crate::ids::ArtifactId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRef {
    pub artifact_id: ArtifactId,
    pub path: PathBuf,
    pub sha256: [u8; 32],
    pub mime_type: String,
    pub size_bytes: u64,
}

impl ArtifactRef {
    pub fn new(
        artifact_id: ArtifactId,
        path: impl Into<PathBuf>,
        sha256: [u8; 32],
        mime_type: impl Into<String>,
        size_bytes: u64,
    ) -> Self {
        Self {
            artifact_id,
            path: path.into(),
            sha256,
            mime_type: mime_type.into(),
            size_bytes,
        }
    }

    pub fn compute(
        artifact_id: ArtifactId,
        path: impl Into<PathBuf>,
        bytes: &[u8],
        mime_type: impl Into<String>,
    ) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let digest: [u8; 32] = hasher.finalize().into();
        Self {
            artifact_id,
            path: path.into(),
            sha256: digest,
            mime_type: mime_type.into(),
            size_bytes: bytes.len() as u64,
        }
    }

    pub fn hex_digest(&self) -> String {
        self.sha256
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    }

    pub fn verify_bytes(&self, bytes: &[u8]) -> bool {
        if bytes.len() as u64 != self.size_bytes {
            return false;
        }
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let digest: [u8; 32] = hasher.finalize().into();
        digest == self.sha256
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_ref_computes_sha256_and_verifies() {
        let id = ArtifactId::new("art-123").unwrap();
        let payload = b"hello pandora diamond graph artifact";
        let artifact = ArtifactRef::compute(
            id.clone(),
            "build/artifact.bin",
            payload,
            "application/octet-stream",
        );

        assert_eq!(artifact.artifact_id, id);
        assert_eq!(artifact.size_bytes, payload.len() as u64);
        assert!(artifact.verify_bytes(payload));
        assert!(!artifact.verify_bytes(b"tampered content"));

        let serialized = serde_json::to_string(&artifact).unwrap();
        let deserialized: ArtifactRef = serde_json::from_str(&serialized).unwrap();
        assert_eq!(artifact, deserialized);
    }
}
