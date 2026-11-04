//! Autonomous Multi-Agent Swarm Mesh & P2P Gossip Engine
//!
//! Grounded in mathematical foundations:
//! - Epidemic gossip logarithmic dissemination (Theorem 3, t* = O(log N))
//! - Byzantine Fault Tolerant (BFT) quorum consensus (Theorem 4, N >= 3f + 1, Q = 2f + 1)
//! - Probabilistic message deduplication with Kirsch-Mitzenmacher Bloom filtering
//! - Distributed task DAG scheduling with Blumofe-Leiserson work-stealing bounds

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt;

/// Errors arising within the Swarm Mesh subsystems.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwarmError {
    InvalidBftParameters { n: usize, f: usize },
    EquivocationDetected { voter: String, proposal: String },
    ProposalNotFound(String),
    ProposalAlreadyExists(String),
    DagCycleDetected(String),
    TaskNotFound(String),
    TaskAlreadyExists(String),
    InvalidDependency { from: String, to: String },
}

impl fmt::Display for SwarmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBftParameters { n, f: byzantine } => {
                write!(
                    f,
                    "Invalid BFT parameters: N={n} must satisfy N >= 3f + 1 for f={byzantine}"
                )
            }
            Self::EquivocationDetected { voter, proposal } => {
                write!(
                    f,
                    "Byzantine equivocation detected: voter '{voter}' double-voted on proposal '{proposal}'"
                )
            }
            Self::ProposalNotFound(id) => write!(f, "Consensus proposal '{id}' not found"),
            Self::ProposalAlreadyExists(id) => {
                write!(f, "Consensus proposal '{id}' already exists")
            }
            Self::DagCycleDetected(msg) => write!(f, "Task DAG cycle detected: {msg}"),
            Self::TaskNotFound(id) => write!(f, "Task '{id}' not found in DAG"),
            Self::TaskAlreadyExists(id) => write!(f, "Task '{id}' already exists in DAG"),
            Self::InvalidDependency { from, to } => {
                write!(f, "Invalid dependency from '{from}' to '{to}'")
            }
        }
    }
}

impl std::error::Error for SwarmError {}

// ============================================================================
// 1. Vector Clock: Causal Partial Ordering
// ============================================================================

/// Vector Clock maintaining causal ordering across distributed agents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct VectorClock {
    clock: BTreeMap<String, u64>,
}

impl VectorClock {
    pub fn new() -> Self {
        Self {
            clock: BTreeMap::new(),
        }
    }

    pub fn increment(&mut self, node: &str) {
        *self.clock.entry(node.to_string()).or_default() += 1;
    }

    pub fn get(&self, node: &str) -> u64 {
        self.clock.get(node).copied().unwrap_or(0)
    }

    pub fn merge(&mut self, other: &VectorClock) {
        for (node, &other_tick) in &other.clock {
            let my_tick = self.clock.entry(node.clone()).or_default();
            *my_tick = (*my_tick).max(other_tick);
        }
    }

    /// Determines if `self` causally precedes `other` (`self < other`).
    pub fn causally_precedes(&self, other: &VectorClock) -> bool {
        let mut strictly_less = false;

        // All keys in self must be <= other
        for (node, &val) in &self.clock {
            let other_val = other.get(node);
            if val > other_val {
                return false;
            }
            if val < other_val {
                strictly_less = true;
            }
        }

        // Keys in other not in self are treated as self having 0
        for (node, &other_val) in &other.clock {
            if !self.clock.contains_key(node) && other_val > 0 {
                strictly_less = true;
            }
        }

        strictly_less
    }

    /// Determines if two vector clocks are concurrent (neither dominates).
    pub fn is_concurrent_with(&self, other: &VectorClock) -> bool {
        !self.causally_precedes(other) && !other.causally_precedes(self) && self != other
    }
}

// ============================================================================
// 2. Kirsch-Mitzenmacher Bloom Filter: Message Deduplication
// ============================================================================

/// Probabilistic Bloom filter using Kirsch-Mitzenmacher double-hashing
/// to enforce message deduplication with provable false positive rate.
#[derive(Debug, Clone)]
pub struct BloomFilter {
    bits: Vec<u64>,
    num_bits: usize,
    hash_functions: usize,
}

impl BloomFilter {
    pub fn new(num_bits: usize, hash_functions: usize) -> Self {
        let clamped_bits = num_bits.max(64);
        let u64_count = clamped_bits.div_ceil(64);
        Self {
            bits: vec![0u64; u64_count],
            num_bits: u64_count * 64,
            hash_functions: hash_functions.max(1),
        }
    }

    /// Double hashing: g_i(x) = h_1(x) + i * h_2(x) mod m
    fn hash_pair(item: &[u8]) -> (u64, u64) {
        let mut hasher = Sha256::new();
        hasher.update(item);
        let digest = hasher.finalize();

        let h1 = u64::from_be_bytes(digest[0..8].try_into().unwrap());
        let mut h2 = u64::from_be_bytes(digest[8..16].try_into().unwrap());
        if h2 == 0 {
            h2 = 1;
        }
        (h1, h2)
    }

    pub fn insert(&mut self, item: &[u8]) {
        let (h1, h2) = Self::hash_pair(item);
        for i in 0..self.hash_functions {
            let combined = h1.wrapping_add((i as u64).wrapping_mul(h2));
            let bit_idx = (combined as usize) % self.num_bits;
            self.bits[bit_idx / 64] |= 1u64 << (bit_idx % 64);
        }
    }

    pub fn contains(&self, item: &[u8]) -> bool {
        let (h1, h2) = Self::hash_pair(item);
        for i in 0..self.hash_functions {
            let combined = h1.wrapping_add((i as u64).wrapping_mul(h2));
            let bit_idx = (combined as usize) % self.num_bits;
            if (self.bits[bit_idx / 64] & (1u64 << (bit_idx % 64))) == 0 {
                return false;
            }
        }
        true
    }

    /// Theoretical false positive rate for `n` inserted items:
    /// epsilon = (1 - e^(-kn/m))^k
    pub fn theoretical_false_positive_rate(&self, n: usize) -> f64 {
        if self.num_bits == 0 || n == 0 {
            return 0.0;
        }
        let k = self.hash_functions as f64;
        let m = self.num_bits as f64;
        let exponent = -k * (n as f64) / m;
        let p_zero = (-exponent).exp(); // e^(kn/m)
        let inner = 1.0 - (1.0 / p_zero);
        inner.powf(k)
    }

    pub fn clear(&mut self) {
        self.bits.fill(0);
    }
}

// ============================================================================
// 3. Epidemic Gossip Engine
// ============================================================================

/// P2P Gossip Broadcast Envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GossipMessage {
    pub id: String,
    pub origin_node: String,
    pub payload: Vec<u8>,
    pub vector_clock: VectorClock,
    pub hop_count: u32,
    pub max_hops: u32,
}

/// Swarm Node participating in the epidemic P2P mesh.
#[derive(Debug)]
pub struct GossipNode {
    pub node_id: String,
    pub clock: VectorClock,
    pub filter: BloomFilter,
    pub known_peers: HashSet<String>,
    pub fanout: usize,
    pub message_store: Vec<GossipMessage>,
}

impl GossipNode {
    pub fn new(node_id: &str, fanout: usize) -> Self {
        Self {
            node_id: node_id.to_string(),
            clock: VectorClock::new(),
            filter: BloomFilter::new(2048, 7),
            known_peers: HashSet::new(),
            fanout: fanout.max(1),
            message_store: Vec::new(),
        }
    }

    pub fn add_peer(&mut self, peer_id: &str) {
        if peer_id != self.node_id {
            self.known_peers.insert(peer_id.to_string());
        }
    }

    pub fn remove_peer(&mut self, peer_id: &str) {
        self.known_peers.remove(peer_id);
    }

    pub fn publish(&mut self, payload: Vec<u8>, max_hops: u32) -> (GossipMessage, Vec<String>) {
        self.clock.increment(&self.node_id);
        let mut hasher = Sha256::new();
        hasher.update(self.node_id.as_bytes());
        hasher.update(&payload);
        hasher.update(self.clock.get(&self.node_id).to_be_bytes());
        let id = format!("{:x}", hasher.finalize());

        self.filter.insert(id.as_bytes());

        let msg = GossipMessage {
            id,
            origin_node: self.node_id.clone(),
            payload,
            vector_clock: self.clock.clone(),
            hop_count: 0,
            max_hops,
        };

        self.message_store.push(msg.clone());
        let targets = self.select_gossip_targets(&[]);
        (msg, targets)
    }

    /// Process received gossip frame. Returns targets to forward to, or None if discarded.
    pub fn receive(&mut self, mut msg: GossipMessage) -> Option<Vec<String>> {
        // Deduplication via Bloom Filter
        if self.filter.contains(msg.id.as_bytes()) {
            return None;
        }

        self.filter.insert(msg.id.as_bytes());
        self.clock.merge(&msg.vector_clock);
        self.clock.increment(&self.node_id);

        msg.hop_count += 1;
        let should_forward = msg.hop_count < msg.max_hops;

        let targets = if should_forward {
            self.select_gossip_targets(&[&msg.origin_node])
        } else {
            Vec::new()
        };

        self.message_store.push(msg);

        if should_forward && !targets.is_empty() {
            Some(targets)
        } else {
            None
        }
    }

    fn select_gossip_targets(&self, exclude: &[&str]) -> Vec<String> {
        let mut candidates: Vec<String> = self
            .known_peers
            .iter()
            .filter(|p| !exclude.contains(&p.as_str()))
            .cloned()
            .collect();
        // Deterministic pseudo-random sorting based on node_id and count
        candidates.sort();
        candidates.into_iter().take(self.fanout).collect()
    }
}

// ============================================================================
// 4. Byzantine Fault Tolerant (BFT) Quorum Consensus
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConsensusDecision {
    Pending,
    Approved,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal {
    pub proposal_id: String,
    pub proposer_id: String,
    pub payload_hash: [u8; 32],
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vote {
    pub voter_id: String,
    pub approve: bool,
    pub signature: Vec<u8>,
}

/// BFT Quorum Consensus state machine enforcing N >= 3f + 1 (Theorem 4).
#[derive(Debug)]
pub struct BftConsensus {
    pub total_nodes: usize,
    pub byzantine_tolerance: usize,
    pub quorum_threshold: usize,
    pub proposals: HashMap<String, Proposal>,
    pub votes: HashMap<String, HashMap<String, Vote>>,
}

impl BftConsensus {
    pub fn new(total_nodes: usize, byzantine_tolerance: usize) -> Result<Self, SwarmError> {
        // Enforce Theorem 4: N >= 3f + 1
        if total_nodes < 3 * byzantine_tolerance + 1 {
            return Err(SwarmError::InvalidBftParameters {
                n: total_nodes,
                f: byzantine_tolerance,
            });
        }

        // Supermajority quorum threshold: Q = 2f + 1
        let quorum_threshold = 2 * byzantine_tolerance + 1;

        Ok(Self {
            total_nodes,
            byzantine_tolerance,
            quorum_threshold,
            proposals: HashMap::new(),
            votes: HashMap::new(),
        })
    }

    pub fn propose(
        &mut self,
        proposer_id: &str,
        proposal_id: &str,
        payload_hash: [u8; 32],
        timestamp: u64,
    ) -> Result<(), SwarmError> {
        if self.proposals.contains_key(proposal_id) {
            return Err(SwarmError::ProposalAlreadyExists(proposal_id.to_string()));
        }

        self.proposals.insert(
            proposal_id.to_string(),
            Proposal {
                proposal_id: proposal_id.to_string(),
                proposer_id: proposer_id.to_string(),
                payload_hash,
                created_at: timestamp,
            },
        );
        self.votes.insert(proposal_id.to_string(), HashMap::new());
        Ok(())
    }

    pub fn cast_vote(
        &mut self,
        voter_id: &str,
        proposal_id: &str,
        approve: bool,
        signature: Vec<u8>,
    ) -> Result<ConsensusDecision, SwarmError> {
        if !self.proposals.contains_key(proposal_id) {
            return Err(SwarmError::ProposalNotFound(proposal_id.to_string()));
        }

        let proposal_votes = self
            .votes
            .get_mut(proposal_id)
            .expect("proposal votes map must exist");

        // Equivocation check: detect contradictory voting from same node
        if let Some(existing) = proposal_votes.get(voter_id) {
            if existing.approve != approve {
                return Err(SwarmError::EquivocationDetected {
                    voter: voter_id.to_string(),
                    proposal: proposal_id.to_string(),
                });
            }
            // Idempotent re-cast of identical vote
            return Ok(self.tally(proposal_id));
        }

        proposal_votes.insert(
            voter_id.to_string(),
            Vote {
                voter_id: voter_id.to_string(),
                approve,
                signature,
            },
        );

        Ok(self.tally(proposal_id))
    }

    pub fn tally(&self, proposal_id: &str) -> ConsensusDecision {
        let proposal_votes = match self.votes.get(proposal_id) {
            Some(v) => v,
            None => return ConsensusDecision::Pending,
        };

        let mut approvals = 0;
        let mut rejections = 0;

        for vote in proposal_votes.values() {
            if vote.approve {
                approvals += 1;
            } else {
                rejections += 1;
            }
        }

        // BFT decision rule: Supermajority Q = 2f + 1 approvals commits
        if approvals >= self.quorum_threshold {
            ConsensusDecision::Approved
        } else if rejections > self.total_nodes - self.quorum_threshold {
            // Insufficient remaining nodes to ever reach Q approvals
            ConsensusDecision::Rejected
        } else {
            ConsensusDecision::Pending
        }
    }
}

// ============================================================================
// 5. Distributed Task DAG & Work-Stealing Scheduling
// ============================================================================

#[derive(Debug, Clone)]
pub struct TaskNode {
    pub id: String,
    pub weight: u64, // execution cost in abstract time units
}

/// Task Dependency DAG with Kahn topological cycle verification (Theorem 5).
#[derive(Debug, Clone, Default)]
pub struct TaskDag {
    tasks: HashMap<String, TaskNode>,
    adjacency: HashMap<String, Vec<String>>, // pred -> succs
    reverse_adjacency: HashMap<String, Vec<String>>, // succ -> preds
}

impl TaskDag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_task(&mut self, id: &str, weight: u64) -> Result<(), SwarmError> {
        if self.tasks.contains_key(id) {
            return Err(SwarmError::TaskAlreadyExists(id.to_string()));
        }
        self.tasks.insert(
            id.to_string(),
            TaskNode {
                id: id.to_string(),
                weight: weight.max(1),
            },
        );
        self.adjacency.entry(id.to_string()).or_default();
        self.reverse_adjacency.entry(id.to_string()).or_default();
        Ok(())
    }

    pub fn add_dependency(&mut self, pred: &str, succ: &str) -> Result<(), SwarmError> {
        if !self.tasks.contains_key(pred) {
            return Err(SwarmError::TaskNotFound(pred.to_string()));
        }
        if !self.tasks.contains_key(succ) {
            return Err(SwarmError::TaskNotFound(succ.to_string()));
        }
        if pred == succ {
            return Err(SwarmError::InvalidDependency {
                from: pred.to_string(),
                to: succ.to_string(),
            });
        }

        self.adjacency
            .entry(pred.to_string())
            .or_default()
            .push(succ.to_string());
        self.reverse_adjacency
            .entry(succ.to_string())
            .or_default()
            .push(pred.to_string());

        // Validate that no cycle was introduced
        self.topological_sort()?;
        Ok(())
    }

    /// Kahn's Algorithm for topological ordering and cycle detection in O(|V| + |E|).
    pub fn topological_sort(&self) -> Result<Vec<String>, SwarmError> {
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        for id in self.tasks.keys() {
            in_degree.insert(id.clone(), 0);
        }

        for succs in self.adjacency.values() {
            for succ in succs {
                *in_degree.entry(succ.clone()).or_default() += 1;
            }
        }

        let mut queue: VecDeque<String> = in_degree
            .iter()
            .filter(|(_, deg)| **deg == 0)
            .map(|(id, _)| id.clone())
            .collect();

        let mut order = Vec::with_capacity(self.tasks.len());

        while let Some(u) = queue.pop_front() {
            order.push(u.clone());
            if let Some(succs) = self.adjacency.get(&u) {
                for succ in succs {
                    if let Some(deg) = in_degree.get_mut(succ) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push_back(succ.clone());
                        }
                    }
                }
            }
        }

        if order.len() != self.tasks.len() {
            return Err(SwarmError::DagCycleDetected(
                "Cycle detected in task dependencies".to_string(),
            ));
        }

        Ok(order)
    }

    /// Total sequential work T_1 = sum_{v in V} w(v).
    pub fn total_work(&self) -> u64 {
        self.tasks.values().map(|t| t.weight).sum()
    }

    /// Span (critical path length) T_infinity via longest path dynamic programming.
    pub fn critical_path_span(&self) -> Result<u64, SwarmError> {
        let order = self.topological_sort()?;
        let mut dist: HashMap<String, u64> = HashMap::new();

        for id in &order {
            let w = self.tasks[id].weight;
            let pred_max = self
                .reverse_adjacency
                .get(id)
                .and_then(|preds| {
                    preds
                        .iter()
                        .map(|p| dist.get(p).copied().unwrap_or(0))
                        .max()
                })
                .unwrap_or(0);
            dist.insert(id.clone(), pred_max + w);
        }

        Ok(dist.values().copied().max().unwrap_or(0))
    }

    /// Simulates multi-agent work-stealing execution with P workers.
    /// Returns makespan T_P satisfying Blumofe-Leiserson bound E[T_P] <= T_1/P + O(T_infinity).
    pub fn simulate_work_stealing(&self, workers: usize) -> Result<u64, SwarmError> {
        let num_workers = workers.max(1);
        self.topological_sort()?; // Ensure no cycles

        let mut remaining_deps: HashMap<String, usize> = HashMap::new();
        for (id, preds) in &self.reverse_adjacency {
            remaining_deps.insert(id.clone(), preds.len());
        }

        let mut ready_queue: VecDeque<String> = remaining_deps
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(id, _)| id.clone())
            .collect();

        // Worker assignments: (task_id, finish_time)
        let mut worker_assignment: Vec<Option<(String, u64)>> = vec![None; num_workers];
        let mut completed_tasks = 0;
        let total_tasks = self.tasks.len();
        let mut current_time = 0u64;

        while completed_tasks < total_tasks {
            // Assign ready tasks to available worker slots
            for slot in &mut worker_assignment {
                if slot.is_none() {
                    let Some(task_id) = ready_queue.pop_front() else {
                        break;
                    };
                    let duration = self.tasks[&task_id].weight;
                    *slot = Some((task_id, current_time + duration));
                }
            }

            // Find earliest finish time among busy workers
            let next_event = worker_assignment
                .iter()
                .filter_map(|slot| slot.as_ref().map(|(_, finish)| *finish))
                .min();

            match next_event {
                Some(next_time) => {
                    current_time = next_time;
                    // Complete tasks that finished at current_time
                    for slot in &mut worker_assignment {
                        if let Some((task_id, finish)) = slot
                            && *finish == current_time
                        {
                            completed_tasks += 1;
                            if let Some(succs) = self.adjacency.get(task_id) {
                                for succ in succs {
                                    if let Some(deps) = remaining_deps.get_mut(succ)
                                        && *deps > 0
                                    {
                                        *deps -= 1;
                                        if *deps == 0 {
                                            ready_queue.push_back(succ.clone());
                                        }
                                    }
                                }
                            }
                            *slot = None;
                        }
                    }
                }
                None => {
                    break;
                }
            }
        }

        Ok(current_time)
    }
}
