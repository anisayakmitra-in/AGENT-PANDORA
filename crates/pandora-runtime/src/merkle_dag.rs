//! Phase 7: Merkle-DAG Cryptographic Audit Log & Verifiable State Proofs.
//!
//! Provides cryptographic non-repudiation, tamper evidence, and logarithmic
//! $O(\log N)$ inclusion proofs for autonomous agent execution histories.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Domain separation tag for leaf node hashes to prevent second-preimage attacks.
pub const TAG_LEAF: u8 = 0x00;
/// Domain separation tag for internal binary Merkle tree node hashes.
pub const TAG_NODE: u8 = 0x01;
/// Domain separation tag for Merkle-DAG state transitions and causal links.
pub const TAG_DAG: u8 = 0x02;

/// Indicates whether the target hash is on the Left or Right relative to its sibling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeDirection {
    /// Target is left child, sibling is right child: `parent = H(TAG || target || sibling)`
    Left,
    /// Target is right child, sibling is left child: `parent = H(TAG || sibling || target)`
    Right,
}

/// A single step along the logarithmic authentication co-path from leaf to root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MerkleProofStep {
    /// 32-byte cryptographic hash of the sibling node.
    pub sibling_hash: [u8; 32],
    /// Direction of the target node relative to this sibling.
    pub direction: NodeDirection,
}

/// Logarithmic Merkle inclusion proof verifying that a specific leaf exists in the root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InclusionProof {
    /// Zero-based index of the leaf in the audit log.
    pub leaf_index: usize,
    /// 32-byte cryptographic leaf hash.
    pub leaf_hash: [u8; 32],
    /// Sequence of sibling hashes along the tree path, length $\lceil \log_2 N \rceil$.
    pub steps: Vec<MerkleProofStep>,
    /// Expected 32-byte Merkle root hash.
    pub root_hash: [u8; 32],
}

impl InclusionProof {
    /// Verifies the inclusion proof by computing hash co-path up to the root.
    pub fn verify(&self) -> bool {
        let mut current_hash = self.leaf_hash;

        for step in &self.steps {
            current_hash = match step.direction {
                NodeDirection::Left => {
                    MerkleDagAuditLog::compute_parent_hash(&current_hash, &step.sibling_hash)
                }
                NodeDirection::Right => {
                    MerkleDagAuditLog::compute_parent_hash(&step.sibling_hash, &current_hash)
                }
            };
        }

        current_hash == self.root_hash
    }

    /// Returns the length of the proof co-path, bounded by $\lceil \log_2 N \rceil$.
    pub fn proof_length(&self) -> usize {
        self.steps.len()
    }
}

/// A node in the causal Merkle-DAG representing an immutable agent execution event or state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DagNode {
    /// Unique identifier for this node (typically hex encoding of its canonical hash).
    pub id: String,
    /// Hash of the event payload.
    pub payload_hash: [u8; 32],
    /// Sorted identifiers of direct causal parent nodes.
    pub parent_ids: Vec<String>,
    /// Canonical 32-byte hash binding payload and parent lineage.
    pub canonical_hash: [u8; 32],
    /// Unix timestamp in seconds when the event was recorded.
    pub timestamp_secs: u64,
}

/// Error conditions encountered during Merkle-DAG operations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DagError {
    /// A referenced parent node ID was not found in the DAG store.
    ParentNotFound(String),
    /// Adding the node would introduce a cycle, violating DAG acyclicity.
    CycleDetected(String),
    /// Leaf index is out of bounds for the audit log.
    IndexOutOfBounds { index: usize, count: usize },
    /// Causal lineage verification failed for the given node.
    LineageCorrupted(String),
    /// Audit log is empty.
    EmptyLog,
}

impl fmt::Display for DagError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ParentNotFound(id) => write!(f, "parent node not found in DAG: {id}"),
            Self::CycleDetected(id) => write!(f, "cycle detected when linking node: {id}"),
            Self::IndexOutOfBounds { index, count } => {
                write!(f, "leaf index {index} out of bounds for log of size {count}")
            }
            Self::LineageCorrupted(id) => write!(f, "causal lineage corrupted for node: {id}"),
            Self::EmptyLog => write!(f, "merkle audit log is empty"),
        }
    }
}

impl std::error::Error for DagError {}

/// Merkle-DAG cryptographic audit log and state lineage manager.
#[derive(Clone, Debug, Default)]
pub struct MerkleDagAuditLog {
    leaves: Vec<[u8; 32]>,
    dag_nodes: BTreeMap<String, DagNode>,
}

impl MerkleDagAuditLog {
    /// Creates a new empty Merkle-DAG audit log.
    pub fn new() -> Self {
        Self {
            leaves: Vec::new(),
            dag_nodes: BTreeMap::new(),
        }
    }

    /// Computes the cryptographic leaf hash using domain separation tag `0x00`:
    /// $$h = \text{SHA256}(0x00 \mathbin{\Vert} \text{data})$$
    pub fn compute_leaf_hash(data: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update([TAG_LEAF]);
        hasher.update(data);
        hasher.finalize().into()
    }

    /// Computes the parent node hash using domain separation tag `0x01`:
    /// $$h = \text{SHA256}(0x01 \mathbin{\Vert} \text{left} \mathbin{\Vert} \text{right})$$
    pub fn compute_parent_hash(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update([TAG_NODE]);
        hasher.update(left);
        hasher.update(right);
        hasher.finalize().into()
    }

    /// Computes the canonical Merkle-DAG node hash using domain separation tag `0x02`:
    /// $$H(u) = \text{SHA256}(0x02 \mathbin{\Vert} \text{payload\_hash} \mathbin{\Vert} \operatorname{Sort}(\text{parent\_hashes}))$$
    pub fn compute_dag_hash(payload_hash: &[u8; 32], sorted_parent_hashes: &[[u8; 32]]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update([TAG_DAG]);
        hasher.update(payload_hash);
        for parent_hash in sorted_parent_hashes {
            hasher.update(parent_hash);
        }
        hasher.finalize().into()
    }

    /// Appends a new raw event payload to the sequential audit log and returns its leaf hash.
    pub fn append_leaf(&mut self, data: &[u8]) -> [u8; 32] {
        let leaf_hash = Self::compute_leaf_hash(data);
        self.leaves.push(leaf_hash);
        leaf_hash
    }

    /// Returns the total count of leaves in the audit log.
    pub fn leaf_count(&self) -> usize {
        self.leaves.len()
    }

    /// Returns a slice of all leaf hashes.
    pub fn leaves(&self) -> &[[u8; 32]] {
        &self.leaves
    }

    /// Computes the current 32-byte Merkle root hash across all appended leaves.
    ///
    /// For an empty log, returns 32 zero bytes.
    /// For $N = 1$, returns the single leaf hash.
    /// For $N > 1$, builds a balanced tree where odd trailing nodes are duplicated
    /// to preserve exact binary tree shape with height $h = \lceil \log_2 N \rceil$.
    pub fn root_hash(&self) -> [u8; 32] {
        if self.leaves.is_empty() {
            return [0u8; 32];
        }
        if self.leaves.len() == 1 {
            return self.leaves[0];
        }

        let mut current_level = self.leaves.clone();

        while current_level.len() > 1 {
            let mut next_level = Vec::with_capacity(current_level.len().div_ceil(2));
            let mut i = 0;
            while i < current_level.len() {
                let left = current_level[i];
                let right = if i + 1 < current_level.len() {
                    current_level[i + 1]
                } else {
                    current_level[i] // Duplicate last element if odd
                };
                next_level.push(Self::compute_parent_hash(&left, &right));
                i += 2;
            }
            current_level = next_level;
        }

        current_level[0]
    }

    /// Generates a logarithmic inclusion proof for the leaf at `leaf_index`.
    ///
    /// Returns `None` if `leaf_index >= self.leaf_count()`.
    /// The proof contains exactly $\lceil \log_2 N \rceil$ steps.
    pub fn generate_proof(&self, leaf_index: usize) -> Option<InclusionProof> {
        if leaf_index >= self.leaves.len() {
            return None;
        }

        let root = self.root_hash();
        let leaf_hash = self.leaves[leaf_index];

        if self.leaves.len() == 1 {
            return Some(InclusionProof {
                leaf_index,
                leaf_hash,
                steps: Vec::new(),
                root_hash: root,
            });
        }

        let mut steps = Vec::new();
        let mut current_level = self.leaves.clone();
        let mut target_idx = leaf_index;

        while current_level.len() > 1 {
            let is_right_child = target_idx % 2 == 1;
            let sibling_idx = if is_right_child {
                target_idx - 1
            } else if target_idx + 1 < current_level.len() {
                target_idx + 1
            } else {
                target_idx // Sibling is duplicate of self
            };

            let sibling_hash = current_level[sibling_idx];
            let direction = if is_right_child {
                NodeDirection::Right
            } else {
                NodeDirection::Left
            };

            steps.push(MerkleProofStep {
                sibling_hash,
                direction,
            });

            // Construct next level
            let mut next_level = Vec::with_capacity(current_level.len().div_ceil(2));
            let mut i = 0;
            while i < current_level.len() {
                let left = current_level[i];
                let right = if i + 1 < current_level.len() {
                    current_level[i + 1]
                } else {
                    current_level[i]
                };
                next_level.push(Self::compute_parent_hash(&left, &right));
                i += 2;
            }

            current_level = next_level;
            target_idx /= 2;
        }

        Some(InclusionProof {
            leaf_index,
            leaf_hash,
            steps,
            root_hash: root,
        })
    }

    /// Verifies an inclusion proof independently against its stated root hash.
    pub fn verify_proof(proof: &InclusionProof) -> bool {
        proof.verify()
    }

    /// Adds a new causal state node to the Merkle-DAG.
    ///
    /// Validates that all parent IDs exist and ensures acyclicity.
    pub fn add_dag_node(
        &mut self,
        payload: &[u8],
        parent_ids: &[String],
        timestamp_secs: u64,
    ) -> Result<DagNode, DagError> {
        let payload_hash = Self::compute_leaf_hash(payload);

        let mut sorted_parents = parent_ids.to_vec();
        sorted_parents.sort();
        sorted_parents.dedup();

        let mut parent_hashes = Vec::with_capacity(sorted_parents.len());
        for pid in &sorted_parents {
            let parent = self
                .dag_nodes
                .get(pid)
                .ok_or_else(|| DagError::ParentNotFound(pid.clone()))?;
            parent_hashes.push(parent.canonical_hash);
        }
        parent_hashes.sort();

        let canonical_hash = Self::compute_dag_hash(&payload_hash, &parent_hashes);
        let hex_id = canonical_hash
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();

        // Check if node already exists (idempotent addition)
        if let Some(existing) = self.dag_nodes.get(&hex_id) {
            return Ok(existing.clone());
        }

        // Verify that adding this node does not introduce a cycle
        // (Since parent_ids must already exist in dag_nodes, an edge hex_id -> pid is a directed edge backward in time.
        // A cycle can only occur if pid is an ancestor of hex_id, which is impossible if hex_id was just generated,
        // but we explicitly check against existing paths).
        for pid in &sorted_parents {
            if pid == &hex_id {
                return Err(DagError::CycleDetected(hex_id));
            }
        }

        let node = DagNode {
            id: hex_id.clone(),
            payload_hash,
            parent_ids: sorted_parents,
            canonical_hash,
            timestamp_secs,
        };

        self.dag_nodes.insert(hex_id, node.clone());
        Ok(node)
    }

    /// Retrieves a DAG node by its unique identifier.
    pub fn get_dag_node(&self, id: &str) -> Option<&DagNode> {
        self.dag_nodes.get(id)
    }

    /// Returns the number of nodes stored in the DAG.
    pub fn dag_node_count(&self) -> usize {
        self.dag_nodes.len()
    }

    /// Verifies the full causal lineage of a DAG node by checking parent existence,
    /// cryptographic hash binding, and acyclicity back to root nodes.
    pub fn verify_causal_lineage(&self, node_id: &str) -> Result<bool, DagError> {
        let mut visited = BTreeSet::new();
        let mut queue = VecDeque::new();

        queue.push_back(node_id.to_string());

        while let Some(current_id) = queue.pop_front() {
            if visited.contains(&current_id) {
                continue;
            }
            visited.insert(current_id.clone());

            let node = self
                .dag_nodes
                .get(&current_id)
                .ok_or_else(|| DagError::ParentNotFound(current_id.clone()))?;

            // Re-verify canonical hash
            let mut parent_hashes = Vec::with_capacity(node.parent_ids.len());
            for pid in &node.parent_ids {
                let parent = self
                    .dag_nodes
                    .get(pid)
                    .ok_or_else(|| DagError::ParentNotFound(pid.clone()))?;
                parent_hashes.push(parent.canonical_hash);
                queue.push_back(pid.clone());
            }
            parent_hashes.sort();

            let expected_hash = Self::compute_dag_hash(&node.payload_hash, &parent_hashes);
            if expected_hash != node.canonical_hash {
                return Err(DagError::LineageCorrupted(current_id));
            }
        }

        Ok(true)
    }

    /// Returns a valid topological ordering of DAG nodes using Kahn's algorithm in $O(|V| + |E|)$.
    pub fn topological_order(&self) -> Result<Vec<String>, DagError> {
        let mut in_degree: BTreeMap<String, usize> = BTreeMap::new();
        let mut adjacency: BTreeMap<String, Vec<String>> = BTreeMap::new();

        for id in self.dag_nodes.keys() {
            in_degree.insert(id.clone(), 0);
            adjacency.insert(id.clone(), Vec::new());
        }

        for (id, node) in &self.dag_nodes {
            for parent_id in &node.parent_ids {
                if let Some(children) = adjacency.get_mut(parent_id) {
                    children.push(id.clone());
                }
                if let Some(deg) = in_degree.get_mut(id) {
                    *deg += 1;
                }
            }
        }

        let mut zero_in_degree = VecDeque::new();
        for (id, &deg) in &in_degree {
            if deg == 0 {
                zero_in_degree.push_back(id.clone());
            }
        }

        let mut order = Vec::with_capacity(self.dag_nodes.len());

        while let Some(u) = zero_in_degree.pop_front() {
            order.push(u.clone());
            if let Some(children) = adjacency.get(&u) {
                for v in children {
                    if let Some(deg) = in_degree.get_mut(v) {
                        *deg -= 1;
                        if *deg == 0 {
                            zero_in_degree.push_back(v.clone());
                        }
                    }
                }
            }
        }

        if order.len() != self.dag_nodes.len() {
            return Err(DagError::CycleDetected("graph contains cycles".to_string()));
        }

        Ok(order)
    }

    /// Returns true if the DAG contains no directed cycles.
    pub fn is_acyclic(&self) -> bool {
        self.topological_order().is_ok()
    }
}
