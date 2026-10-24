use crate::sessions::{SessionError, SessionStore};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use pandora_types::{MemoryCapsule, TenantId, Timestamp, WorkspaceId};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::path::Path;

#[derive(Debug)]
pub enum MeshError {
    Session(SessionError),
    DigestMismatch,
    InvalidSignature(String),
    InvalidPublicKey(String),
    DagCycle(String),
    PeerNotFound(String),
    Io(String),
    Serialization(String),
}

impl fmt::Display for MeshError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Session(err) => write!(formatter, "session store error: {err}"),
            Self::DigestMismatch => {
                formatter.write_str("capsule SHA-256 digest verification failed")
            }
            Self::InvalidSignature(msg) => {
                write!(formatter, "author Ed25519 signature invalid: {msg}")
            }
            Self::InvalidPublicKey(msg) => write!(formatter, "invalid Ed25519 public key: {msg}"),
            Self::DagCycle(msg) => write!(formatter, "capsule DAG acyclicity check failed: {msg}"),
            Self::PeerNotFound(id) => write!(formatter, "peer '{id}' not found"),
            Self::Io(msg) => write!(formatter, "I/O error: {msg}"),
            Self::Serialization(msg) => write!(formatter, "serialization error: {msg}"),
        }
    }
}

impl std::error::Error for MeshError {}

impl From<SessionError> for MeshError {
    fn from(error: SessionError) -> Self {
        Self::Session(error)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeshPeerEntry {
    pub peer_id: String,
    pub address: String,
    pub public_key: String,
    pub status: String,
    pub last_seen: u64,
    pub trust_score: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeshHandshake {
    pub peer_id: String,
    pub public_key: String,
    pub listen_addr: String,
    pub version: String,
    pub capabilities: Vec<String>,
    pub timestamp: u64,
    pub signature: String,
}

impl MeshHandshake {
    pub fn payload_bytes(&self) -> Vec<u8> {
        format!(
            "pandora-mesh-handshake:v1:{}:{}:{}:{}",
            self.peer_id, self.public_key, self.listen_addr, self.timestamp
        )
        .into_bytes()
    }

    pub fn verify(&self) -> Result<bool, MeshError> {
        let pk_bytes = decode_hex_32(&self.public_key)
            .ok_or_else(|| MeshError::InvalidPublicKey("invalid hex public key".to_string()))?;
        let verifying_key = VerifyingKey::from_bytes(&pk_bytes)
            .map_err(|e| MeshError::InvalidPublicKey(e.to_string()))?;

        let sig_bytes = decode_hex_64(&self.signature)
            .ok_or_else(|| MeshError::InvalidSignature("invalid hex signature".to_string()))?;
        let signature = Signature::from_bytes(&sig_bytes);

        verifying_key
            .verify_strict(&self.payload_bytes(), &signature)
            .map(|_| true)
            .map_err(|e| MeshError::InvalidSignature(e.to_string()))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SignedCapsulePackage {
    pub capsule: MemoryCapsule,
    pub author_public_key: String,
    pub author_signature: String,
    #[serde(default)]
    pub parent_capsule_ids: Vec<String>,
    #[serde(default)]
    pub gossip_hops: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CapsuleValidationVerdict {
    pub valid: bool,
    pub sha256_verified: bool,
    pub signature_verified: bool,
    pub dag_acyclic: bool,
    pub capsule_id: String,
    pub author: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeshSyncResult {
    pub admitted: Vec<String>,
    pub rejected: Vec<(String, String)>,
    pub promoted: Vec<String>,
    pub peer_count: usize,
}

pub struct PeerIdentity {
    pub signing_key: SigningKey,
    pub verifying_key: VerifyingKey,
    pub peer_id: String,
}

impl PeerIdentity {
    pub fn new_random() -> Self {
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).expect("random seed available");
        let signing_key = SigningKey::from_bytes(&seed);
        let verifying_key = signing_key.verifying_key();
        let pub_hex = encode_hex(&verifying_key.to_bytes());
        let peer_id = format!("peer-{}", &pub_hex[..16]);
        Self {
            signing_key,
            verifying_key,
            peer_id,
        }
    }

    pub fn load_or_generate(path: &Path) -> Result<Self, MeshError> {
        if path.exists() {
            let data = fs::read_to_string(path).map_err(|e| MeshError::Io(e.to_string()))?;
            let trimmed = data.trim();
            if let Some(bytes) = decode_hex_32(trimmed) {
                let signing_key = SigningKey::from_bytes(&bytes);
                let verifying_key = signing_key.verifying_key();
                let pub_hex = encode_hex(&verifying_key.to_bytes());
                let peer_id = format!("peer-{}", &pub_hex[..16]);
                return Ok(Self {
                    signing_key,
                    verifying_key,
                    peer_id,
                });
            }
        }

        let identity = Self::new_random();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let hex_private = encode_hex(&identity.signing_key.to_bytes());
        let _ = fs::write(path, hex_private);
        Ok(identity)
    }

    pub fn public_key_hex(&self) -> String {
        encode_hex(&self.verifying_key.to_bytes())
    }

    pub fn sign_digest(&self, digest: &str) -> String {
        let sig = self.signing_key.sign(digest.as_bytes());
        encode_hex(&sig.to_bytes())
    }

    pub fn create_handshake(&self, listen_addr: &str, capabilities: Vec<String>) -> MeshHandshake {
        let timestamp = Timestamp::from_unix_seconds(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        )
        .as_unix_seconds();

        let mut hs = MeshHandshake {
            peer_id: self.peer_id.clone(),
            public_key: self.public_key_hex(),
            listen_addr: listen_addr.to_string(),
            version: "2.0.0".to_string(),
            capabilities,
            timestamp,
            signature: String::new(),
        };

        let sig = self.signing_key.sign(&hs.payload_bytes());
        hs.signature = encode_hex(&sig.to_bytes());
        hs
    }
}

pub struct MeshSyncEngine;

#[allow(clippy::collapsible_if)]
impl MeshSyncEngine {
    /// Validates an incoming signed capsule package against the strict 3-point security pipeline:
    /// 1. SHA-256 digest integrity check
    /// 2. Author Ed25519 signature check
    /// 3. DAG acyclicity check
    pub fn validate_capsule_package(
        package: &SignedCapsulePackage,
        known_ancestors: &HashMap<String, Vec<String>>,
    ) -> Result<CapsuleValidationVerdict, MeshError> {
        let capsule_id = package.capsule.capsule_id.clone();

        // 1. SHA-256 Digest Verification
        if !package.capsule.verify_digest() {
            return Err(MeshError::DigestMismatch);
        }

        // 2. Author Ed25519 Signature Verification
        let pk_bytes = decode_hex_32(&package.author_public_key).ok_or_else(|| {
            MeshError::InvalidPublicKey("invalid author public key hex".to_string())
        })?;
        let verifying_key = VerifyingKey::from_bytes(&pk_bytes)
            .map_err(|e| MeshError::InvalidPublicKey(e.to_string()))?;

        let sig_bytes = decode_hex_64(&package.author_signature).ok_or_else(|| {
            MeshError::InvalidSignature("invalid author signature hex".to_string())
        })?;
        let signature = Signature::from_bytes(&sig_bytes);

        verifying_key
            .verify_strict(package.capsule.sha256_digest.as_bytes(), &signature)
            .map_err(|e| MeshError::InvalidSignature(e.to_string()))?;

        // 3. DAG Acyclicity Check
        // 3a. SOP step dependency graph validation (checks step acyclicity internally)
        package
            .capsule
            .sop
            .validate()
            .map_err(|e| MeshError::DagCycle(e.to_string()))?;

        // 3b. Capsule-level ancestry graph acyclicity check
        if package.parent_capsule_ids.contains(&capsule_id) {
            return Err(MeshError::DagCycle(format!(
                "self-referential parent cycle in capsule {capsule_id}"
            )));
        }

        // Detect any cycle in the reachable ancestry graph using DFS recursion-stack tracking
        fn check_ancestry_cycle(
            current: &str,
            target_capsule: &str,
            known_ancestors: &HashMap<String, Vec<String>>,
            visited: &mut HashSet<String>,
            in_stack: &mut HashSet<String>,
        ) -> Result<(), MeshError> {
            if current == target_capsule {
                return Err(MeshError::DagCycle(format!(
                    "cycle detected in capsule lineage reaching {target_capsule}"
                )));
            }
            if in_stack.contains(current) {
                return Err(MeshError::DagCycle(format!(
                    "cycle detected in capsule ancestry lineage at node {current}"
                )));
            }
            if visited.contains(current) {
                return Ok(());
            }

            visited.insert(current.to_string());
            in_stack.insert(current.to_string());

            if let Some(parents) = known_ancestors.get(current) {
                for parent in parents {
                    check_ancestry_cycle(
                        parent,
                        target_capsule,
                        known_ancestors,
                        visited,
                        in_stack,
                    )?;
                }
            }

            in_stack.remove(current);
            Ok(())
        }

        let mut visited = HashSet::new();
        let mut in_stack = HashSet::new();
        for parent in &package.parent_capsule_ids {
            check_ancestry_cycle(
                parent,
                &capsule_id,
                known_ancestors,
                &mut visited,
                &mut in_stack,
            )?;
        }

        Ok(CapsuleValidationVerdict {
            valid: true,
            sha256_verified: true,
            signature_verified: true,
            dag_acyclic: true,
            capsule_id,
            author: package.capsule.author.clone(),
        })
    }

    /// Broadcasts an SOP as a signed `.capsule.json` package across mesh gossip staging.
    pub fn broadcast_sop(
        store: &SessionStore,
        identity: &PeerIdentity,
        sop_id: &str,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
        data_dir: &Path,
        timestamp: Timestamp,
    ) -> Result<SignedCapsulePackage, MeshError> {
        let stored_sop = store
            .load_sop(tenant_id, workspace_id, sop_id)?
            .ok_or_else(|| MeshError::PeerNotFound(format!("SOP '{sop_id}' not found in store")))?;

        let capsule = MemoryCapsule::new(
            format!("capsule-{sop_id}"),
            stored_sop.sop.clone(),
            identity.peer_id.clone(),
            timestamp,
        )
        .map_err(|e| MeshError::Serialization(e.to_string()))?;

        let author_signature = identity.sign_digest(&capsule.sha256_digest);
        let author_public_key = identity.public_key_hex();

        let package = SignedCapsulePackage {
            capsule,
            author_public_key,
            author_signature,
            parent_capsule_ids: vec![],
            gossip_hops: 0,
        };

        // 1. Store in SQLite mesh_capsules
        store.store_mesh_capsule(&package)?;

        // 2. Write to mesh staging directory
        let mesh_dir = data_dir.join("mesh").join("capsules");
        let _ = fs::create_dir_all(&mesh_dir);
        let package_file = mesh_dir.join(format!("{}.capsule.json", package.capsule.capsule_id));
        let package_json = serde_json::to_string_pretty(&package)
            .map_err(|e| MeshError::Serialization(e.to_string()))?;
        fs::write(package_file, package_json).map_err(|e| MeshError::Io(e.to_string()))?;

        Ok(package)
    }

    /// Synchronizes incoming mesh capsules, runs 3-point validation, admits to local SQLite,
    /// and optionally auto-promotes qualifying SOPs to active L3 storage.
    pub fn sync_capsules(
        store: &SessionStore,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
        data_dir: &Path,
        auto_promote: bool,
        timestamp: Timestamp,
    ) -> Result<MeshSyncResult, MeshError> {
        let mut admitted = Vec::new();
        let mut rejected = Vec::new();
        let mut promoted = Vec::new();

        let mut known_ancestors = HashMap::new();
        for existing in store.list_mesh_capsules()? {
            known_ancestors.insert(existing.capsule.capsule_id, existing.parent_capsule_ids);
        }

        let mesh_dir = data_dir.join("mesh").join("capsules");
        if mesh_dir.exists() {
            if let Ok(entries) = fs::read_dir(&mesh_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|ext| ext.to_str()) == Some("json")
                        || path.to_string_lossy().ends_with(".capsule.json")
                    {
                        if let Ok(content) = fs::read_to_string(&path) {
                            let filename = path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string();
                            match serde_json::from_str::<SignedCapsulePackage>(&content) {
                                Ok(package) => {
                                    match Self::validate_capsule_package(&package, &known_ancestors)
                                    {
                                        Ok(verdict) => {
                                            let cap_id = verdict.capsule_id.clone();
                                            let _ = store.store_mesh_capsule(&package);
                                            known_ancestors.insert(
                                                cap_id.clone(),
                                                package.parent_capsule_ids.clone(),
                                            );
                                            admitted.push(cap_id.clone());

                                            if auto_promote {
                                                if let Ok(stored) = store.store_sop(
                                                    tenant_id,
                                                    workspace_id,
                                                    &package.capsule.sop,
                                                    timestamp,
                                                ) {
                                                    promoted.push(stored.sop.id);
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            rejected
                                                .push((package.capsule.capsule_id, e.to_string()));
                                        }
                                    }
                                }
                                Err(e) => {
                                    rejected.push((filename, format!("invalid capsule JSON: {e}")));
                                }
                            }
                        }
                    }
                }
            }
        }

        let peer_count = store.list_mesh_peers()?.len();

        Ok(MeshSyncResult {
            admitted,
            rejected,
            promoted,
            peer_count,
        })
    }

    /// Lists known mesh peers with discovery and trust posture.
    pub fn list_peers(
        store: &SessionStore,
        data_dir: &Path,
    ) -> Result<Vec<MeshPeerEntry>, MeshError> {
        let mut peers = store.list_mesh_peers()?;

        if peers.is_empty() {
            // Seed peer fallback
            let seed_alpha = MeshPeerEntry {
                peer_id: "peer-seed-alpha".to_string(),
                address: "127.0.0.1:48123".to_string(),
                public_key: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                    .to_string(),
                status: "seed".to_string(),
                last_seen: 1729000000,
                trust_score: 0.99,
            };
            let seed_beta = MeshPeerEntry {
                peer_id: "peer-seed-beta".to_string(),
                address: "127.0.0.1:48124".to_string(),
                public_key: "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210"
                    .to_string(),
                status: "active".to_string(),
                last_seen: 1729000100,
                trust_score: 0.95,
            };
            let _ = store.store_mesh_peer(&seed_alpha);
            let _ = store.store_mesh_peer(&seed_beta);
            peers.push(seed_alpha);
            peers.push(seed_beta);
        }

        // Also check optional mesh_peers.json in data_dir
        let seed_file = data_dir.join("mesh_peers.json");
        if seed_file.exists() {
            if let Ok(content) = fs::read_to_string(seed_file) {
                if let Ok(extra_peers) = serde_json::from_str::<Vec<MeshPeerEntry>>(&content) {
                    for ep in extra_peers {
                        if !peers.iter().any(|p| p.peer_id == ep.peer_id) {
                            let _ = store.store_mesh_peer(&ep);
                            peers.push(ep);
                        }
                    }
                }
            }
        }

        Ok(peers)
    }

    /// Broadcasts an Ed25519 identity handshake announcement over UDP broadcast or unicast.
    pub fn broadcast_peer_announcement(
        identity: &PeerIdentity,
        listen_addr: &str,
        target_addr: &str,
    ) -> Result<MeshHandshake, MeshError> {
        let handshake = identity.create_handshake(listen_addr, vec!["capsule_gossip".to_string()]);
        let payload =
            serde_json::to_vec(&handshake).map_err(|e| MeshError::Serialization(e.to_string()))?;

        if let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") {
            let _ = socket.set_broadcast(true);
            let _ = socket.send_to(&payload, target_addr);
        }
        Ok(handshake)
    }

    /// Listens briefly for incoming UDP peer announcements and records verified peers to SQLite.
    pub fn receive_peer_announcements(
        store: &SessionStore,
        bind_addr: &str,
        timeout: std::time::Duration,
    ) -> Result<Vec<MeshPeerEntry>, MeshError> {
        let socket = match std::net::UdpSocket::bind(bind_addr) {
            Ok(s) => s,
            Err(e) => return Err(MeshError::Io(format!("UDP bind error on {bind_addr}: {e}"))),
        };
        let _ = socket.set_read_timeout(Some(timeout));
        let mut buf = [0u8; 4096];
        let mut discovered = Vec::new();

        while let Ok((amt, src)) = socket.recv_from(&mut buf) {
            if let Ok(handshake) = serde_json::from_slice::<MeshHandshake>(&buf[..amt]) {
                if handshake.verify().unwrap_or(false) {
                    let entry = MeshPeerEntry {
                        peer_id: handshake.peer_id.clone(),
                        address: if handshake.listen_addr.is_empty() {
                            src.to_string()
                        } else {
                            handshake.listen_addr.clone()
                        },
                        public_key: handshake.public_key.clone(),
                        status: "discovered".to_string(),
                        last_seen: handshake.timestamp,
                        trust_score: 0.90,
                    };
                    let _ = store.store_mesh_peer(&entry);
                    discovered.push(entry);
                }
            }
        }
        Ok(discovered)
    }
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

fn decode_hex_32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut buf = [0u8; 32];
    for (i, byte) in buf.iter_mut().enumerate() {
        let hi = decode_hex_nibble(s.as_bytes()[i * 2])?;
        let lo = decode_hex_nibble(s.as_bytes()[i * 2 + 1])?;
        *byte = (hi << 4) | lo;
    }
    Some(buf)
}

fn decode_hex_64(s: &str) -> Option<[u8; 64]> {
    if s.len() != 128 {
        return None;
    }
    let mut buf = [0u8; 64];
    for (i, byte) in buf.iter_mut().enumerate() {
        let hi = decode_hex_nibble(s.as_bytes()[i * 2])?;
        let lo = decode_hex_nibble(s.as_bytes()[i * 2 + 1])?;
        *byte = (hi << 4) | lo;
    }
    Some(buf)
}

fn decode_hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::{MemoryIntent, SopDefinition, SopStep};

    #[test]
    fn test_peer_identity_and_handshake() {
        let id = PeerIdentity::new_random();
        let hs = id.create_handshake("127.0.0.1:4000", vec!["gossip".to_string()]);
        assert_eq!(hs.peer_id, id.peer_id);
        assert!(hs.verify().unwrap());
    }

    #[test]
    fn test_signed_capsule_3_point_validation() {
        let id = PeerIdentity::new_random();
        let step = SopStep::new("s1", "Step 1", "echo 1", vec![]).unwrap();
        let sop = SopDefinition::new(
            "sop-test-mesh",
            "Mesh SOP",
            MemoryIntent::Procedure,
            vec![step],
            None,
        )
        .unwrap();

        let capsule = MemoryCapsule::new(
            "capsule-test-1",
            sop,
            id.peer_id.clone(),
            Timestamp::from_unix_seconds(100),
        )
        .unwrap();

        let author_signature = id.sign_digest(&capsule.sha256_digest);
        let author_public_key = id.public_key_hex();

        let package = SignedCapsulePackage {
            capsule,
            author_public_key,
            author_signature,
            parent_capsule_ids: vec![],
            gossip_hops: 0,
        };

        let verdict = MeshSyncEngine::validate_capsule_package(&package, &HashMap::new()).unwrap();
        assert!(verdict.valid);
        assert!(verdict.sha256_verified);
        assert!(verdict.signature_verified);
        assert!(verdict.dag_acyclic);
    }

    #[test]
    fn test_dag_cycle_detection_in_ancestor_lineage() {
        let id = PeerIdentity::new_random();
        let step = SopStep::new("s1", "Step 1", "echo 1", vec![]).unwrap();
        let sop = SopDefinition::new(
            "sop-test-mesh-cycle",
            "Mesh SOP Cycle",
            MemoryIntent::Procedure,
            vec![step],
            None,
        )
        .unwrap();

        let capsule = MemoryCapsule::new(
            "capsule-leaf",
            sop,
            id.peer_id.clone(),
            Timestamp::from_unix_seconds(100),
        )
        .unwrap();

        let package = SignedCapsulePackage {
            capsule: capsule.clone(),
            author_public_key: id.public_key_hex(),
            author_signature: id.sign_digest(&capsule.sha256_digest),
            parent_capsule_ids: vec!["node_1".to_string()],
            gossip_hops: 0,
        };

        // Construct cyclic ancestor graph: node_1 -> node_2 -> node_3 -> node_1
        let mut cyclic_ancestors = HashMap::new();
        cyclic_ancestors.insert("node_1".to_string(), vec!["node_2".to_string()]);
        cyclic_ancestors.insert("node_2".to_string(), vec!["node_3".to_string()]);
        cyclic_ancestors.insert("node_3".to_string(), vec!["node_1".to_string()]);

        let result = MeshSyncEngine::validate_capsule_package(&package, &cyclic_ancestors);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), MeshError::DagCycle(_)));

        // Test diamond DAG (valid, non-cyclic multi-parent graph):
        // leaf -> [node_b, node_c]
        // node_b -> [node_d]
        // node_c -> [node_d]
        // node_d -> []
        let mut diamond_ancestors = HashMap::new();
        diamond_ancestors.insert("node_b".to_string(), vec!["node_d".to_string()]);
        diamond_ancestors.insert("node_c".to_string(), vec!["node_d".to_string()]);
        diamond_ancestors.insert("node_d".to_string(), vec![]);

        let diamond_package = SignedCapsulePackage {
            capsule: capsule.clone(),
            author_public_key: id.public_key_hex(),
            author_signature: id.sign_digest(&capsule.sha256_digest),
            parent_capsule_ids: vec!["node_b".to_string(), "node_c".to_string()],
            gossip_hops: 0,
        };

        let diamond_verdict =
            MeshSyncEngine::validate_capsule_package(&diamond_package, &diamond_ancestors).unwrap();
        assert!(diamond_verdict.valid);
        assert!(diamond_verdict.dag_acyclic);
    }
}
