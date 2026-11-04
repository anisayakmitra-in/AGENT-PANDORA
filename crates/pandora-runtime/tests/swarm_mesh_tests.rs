//! Comprehensive Verification of Autonomous Swarm Mesh & P2P Gossip Engine
//!
//! Grounded in mathematical foundations:
//! - Theorem 3: Epidemic gossip logarithmic dissemination rounds O(log N)
//! - Theorem 4: BFT Quorum Consensus invariants (N >= 3f + 1, Q = 2f + 1)
//! - Probabilistic message deduplication with Bloom filter false positive bounds
//! - Theorem 5: Task DAG scheduling with work-stealing bounds

use pandora_runtime::swarm_mesh::{
    BftConsensus, BloomFilter, ConsensusDecision, GossipNode, SwarmError, TaskDag, VectorClock,
};
use std::collections::{HashMap, HashSet, VecDeque};

#[test]
fn test_vector_clock_causality_and_concurrency() {
    let mut vc_a = VectorClock::new();
    let mut vc_b = VectorClock::new();

    vc_a.increment("node-1");
    assert_eq!(vc_a.get("node-1"), 1);
    assert_eq!(vc_a.get("node-2"), 0);

    // Initial state: vc_b is at 0, vc_a is at 1 => vc_b < vc_a
    assert!(vc_b.causally_precedes(&vc_a));
    assert!(!vc_a.causally_precedes(&vc_b));
    assert!(!vc_a.is_concurrent_with(&vc_b));

    // vc_b increments node-2
    vc_b.increment("node-2");
    // Now vc_a has (node-1: 1), vc_b has (node-2: 1) => concurrent!
    assert!(vc_a.is_concurrent_with(&vc_b));
    assert!(vc_b.is_concurrent_with(&vc_a));

    // Merge vc_b into vc_a
    vc_a.merge(&vc_b);
    assert_eq!(vc_a.get("node-1"), 1);
    assert_eq!(vc_a.get("node-2"), 1);

    // Now vc_b < vc_a
    assert!(vc_b.causally_precedes(&vc_a));
    assert!(!vc_a.causally_precedes(&vc_b));
    assert!(!vc_a.is_concurrent_with(&vc_b));
}

#[test]
fn test_bloom_filter_deduplication_and_rate_bounds() {
    let mut filter = BloomFilter::new(4096, 7);
    let n = 200;

    // Insert 200 distinct message IDs
    for i in 0..n {
        let msg_id = format!("msg-id-batch-alpha-{i}");
        filter.insert(msg_id.as_bytes());
    }

    // Zero false negative invariant: all inserted keys MUST test positive
    for i in 0..n {
        let msg_id = format!("msg-id-batch-alpha-{i}");
        assert!(
            filter.contains(msg_id.as_bytes()),
            "Bloom filter must have zero false negatives"
        );
    }

    // Check false positive rate on 2,000 unseen keys
    let unseen_count = 2000;
    let mut false_positives = 0;
    for i in 0..unseen_count {
        let unseen = format!("completely-unseen-key-beta-{i}");
        if filter.contains(unseen.as_bytes()) {
            false_positives += 1;
        }
    }

    let empirical_rate = (false_positives as f64) / (unseen_count as f64);
    let theoretical_rate = filter.theoretical_false_positive_rate(n);

    // Theoretical rate for m=4096, k=7, n=200 is approx 0.003 (0.3%)
    assert!(
        theoretical_rate < 0.05,
        "Theoretical false positive rate must be < 5%: got {theoretical_rate}"
    );
    assert!(
        empirical_rate < 0.05,
        "Empirical false positive rate must be < 5%: got {empirical_rate}"
    );
}

#[test]
fn test_epidemic_gossip_dissemination_convergence() {
    // Simulate a 16-node swarm network
    let node_count = 16;
    let fanout = 3;
    let max_hops = 10;

    let mut nodes: HashMap<String, GossipNode> = HashMap::new();
    for i in 0..node_count {
        let id = format!("agent-{i}");
        nodes.insert(id.clone(), GossipNode::new(&id, fanout));
    }

    // Connect nodes in a ring topology with cross-links
    let node_ids: Vec<String> = (0..node_count).map(|i| format!("agent-{i}")).collect();
    for i in 0..node_count {
        let next = (i + 1) % node_count;
        let prev = (i + node_count - 1) % node_count;
        let cross = (i + node_count / 2) % node_count;

        let node = nodes.get_mut(&node_ids[i]).unwrap();
        node.add_peer(&node_ids[next]);
        node.add_peer(&node_ids[prev]);
        node.add_peer(&node_ids[cross]);
    }

    // Origin node 0 publishes a message
    let origin_id = &node_ids[0];
    let payload = b"swarm-task-distribution-v1".to_vec();
    let (msg, initial_targets) = nodes.get_mut(origin_id).unwrap().publish(payload, max_hops);

    // Discrete round simulation queue: (target_node, message)
    let mut round_queue: VecDeque<(String, pandora_runtime::swarm_mesh::GossipMessage)> =
        VecDeque::new();
    for target in initial_targets {
        round_queue.push_back((target, msg.clone()));
    }

    let mut rounds = 0;
    let mut informed_nodes: HashSet<String> = HashSet::new();
    informed_nodes.insert(origin_id.clone());

    while !round_queue.is_empty() && rounds < 20 {
        rounds += 1;
        let batch_size = round_queue.len();
        for _ in 0..batch_size {
            let (target, packet) = round_queue.pop_front().unwrap();
            informed_nodes.insert(target.clone());
            if let Some(target_node) = nodes.get_mut(&target)
                && let Some(next_targets) = target_node.receive(packet.clone())
            {
                for nxt in next_targets {
                    round_queue.push_back((nxt, packet.clone()));
                }
            }
        }
    }

    // Dissemination must inform all 16 agents
    assert_eq!(
        informed_nodes.len(),
        node_count,
        "Gossip epidemic must reach all nodes"
    );
    // Logarithmic round bound: rounds <= 2 * ln(16) / beta <= 8 rounds
    assert!(
        rounds <= 8,
        "Gossip convergence rounds must scale logarithmically: took {rounds} rounds"
    );
}

#[test]
fn test_bft_quorum_consensus_theorem_and_equivocation() {
    // 1. Parameter validity: N >= 3f + 1
    // For f = 1: N >= 4. N = 3 must fail.
    let invalid = BftConsensus::new(3, 1);
    assert!(matches!(
        invalid,
        Err(SwarmError::InvalidBftParameters { n: 3, f: 1 })
    ));

    // Valid 4-node swarm with f = 1 Byzantine tolerance
    let mut consensus = BftConsensus::new(4, 1).expect("N=4, f=1 is valid");
    assert_eq!(consensus.quorum_threshold, 3); // Q = 2f + 1 = 3

    let proposal_id = "patch-proposal-hash-789";
    let payload_hash = [0x5Au8; 32];
    consensus
        .propose("agent-proposer", proposal_id, payload_hash, 1_700_000_000)
        .expect("proposal must succeed");

    // Proposing duplicate ID fails
    let dup_err = consensus.propose("agent-other", proposal_id, payload_hash, 1_700_000_001);
    assert!(matches!(dup_err, Err(SwarmError::ProposalAlreadyExists(_))));

    // Vote 1: Approved by agent-1
    let d1 = consensus
        .cast_vote("agent-1", proposal_id, true, vec![0x01])
        .unwrap();
    assert_eq!(d1, ConsensusDecision::Pending);

    // Vote 2: Rejected by Byzantine agent-2
    let d2 = consensus
        .cast_vote("agent-2", proposal_id, false, vec![0x02])
        .unwrap();
    assert_eq!(d2, ConsensusDecision::Pending);

    // Byzantine equivocation attempt: agent-2 tries to change vote to true
    let equiv_err = consensus.cast_vote("agent-2", proposal_id, true, vec![0x02]);
    assert!(matches!(
        equiv_err,
        Err(SwarmError::EquivocationDetected { .. })
    ));

    // Vote 3: Approved by agent-3 (now 2 approvals, 1 rejection out of 4)
    let d3 = consensus
        .cast_vote("agent-3", proposal_id, true, vec![0x03])
        .unwrap();
    assert_eq!(d3, ConsensusDecision::Pending);

    // Vote 4: Approved by agent-4 (now 3 approvals >= Q=3 => APPROVED!)
    let d4 = consensus
        .cast_vote("agent-4", proposal_id, true, vec![0x04])
        .unwrap();
    assert_eq!(d4, ConsensusDecision::Approved);
}

#[test]
fn test_task_dag_kahn_topological_sort_and_cycle_rejection() {
    let mut dag = TaskDag::new();

    dag.add_task("task-a", 10).unwrap();
    dag.add_task("task-b", 20).unwrap();
    dag.add_task("task-c", 15).unwrap();
    dag.add_task("task-d", 5).unwrap();

    // Adding existing task fails
    assert!(matches!(
        dag.add_task("task-a", 10),
        Err(SwarmError::TaskAlreadyExists(_))
    ));

    // Valid dependencies: A -> B, A -> C, B -> D, C -> D
    dag.add_dependency("task-a", "task-b").unwrap();
    dag.add_dependency("task-a", "task-c").unwrap();
    dag.add_dependency("task-b", "task-d").unwrap();
    dag.add_dependency("task-c", "task-d").unwrap();

    let order = dag.topological_sort().expect("DAG must be cycle-free");
    assert_eq!(order.len(), 4);
    assert_eq!(order[0], "task-a");
    assert_eq!(order[3], "task-d");

    // Cycle introduction attempt: D -> A
    let cycle_err = dag.add_dependency("task-d", "task-a");
    assert!(matches!(cycle_err, Err(SwarmError::DagCycleDetected(_))));
}

#[test]
fn test_task_dag_work_stealing_makespan_bounds() {
    let mut dag = TaskDag::new();

    // Fork-join diamond graph:
    // A (10) -> {B (20), C (20)} -> D (10)
    dag.add_task("A", 10).unwrap();
    dag.add_task("B", 20).unwrap();
    dag.add_task("C", 20).unwrap();
    dag.add_task("D", 10).unwrap();

    dag.add_dependency("A", "B").unwrap();
    dag.add_dependency("A", "C").unwrap();
    dag.add_dependency("B", "D").unwrap();
    dag.add_dependency("C", "D").unwrap();

    // Total work T_1 = 10 + 20 + 20 + 10 = 60
    let t1 = dag.total_work();
    assert_eq!(t1, 60);

    // Critical path span T_infinity = 10 + 20 + 10 = 40
    let t_inf = dag.critical_path_span().unwrap();
    assert_eq!(t_inf, 40);

    // Sequential makespan with 1 worker: T_1 = 60
    let m1 = dag.simulate_work_stealing(1).unwrap();
    assert_eq!(m1, 60);

    // Parallel makespan with 2 workers:
    // Worker 1 runs A (0..10), then B (10..30)
    // Worker 2 idle (0..10), then runs C (10..30)
    // Worker 1 or 2 runs D (30..40)
    // Makespan T_2 = 40 (exact critical path span!)
    let m2 = dag.simulate_work_stealing(2).unwrap();
    assert_eq!(m2, 40);

    // Verifying Blumofe-Leiserson bound: T_P >= max(T_1/P, T_infinity)
    assert!(m2 >= t_inf);
    assert!(m2 >= t1 / 2);
}
