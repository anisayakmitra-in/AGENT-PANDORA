#![forbid(unsafe_code)]

use pandora_runtime::{
    AsciiGridRenderer, BftConsensus, BloomFilter, ConsensusDecision, ForceDirectedLayout,
    GossipNode, Point2D, SwarmError, TaskDag, ViewportCamera,
};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(500);

struct Fixture {
    root: PathBuf,
    config: PathBuf,
    data: PathBuf,
    workspace: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock available")
            .as_nanos();
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let temp_root = std::env::temp_dir()
            .canonicalize()
            .expect("canonical temp dir");
        let root = temp_root.join(format!(
            "pandora-challenger-m1-2-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).expect("workspace should be created");
        Self {
            config: root.join("config.json"),
            data: root.join("data"),
            workspace,
            root,
        }
    }

    fn setup(&self) {
        let output = Command::new(env!("CARGO_BIN_EXE_pandora"))
            .args(["setup", "--config"])
            .arg(&self.config)
            .args(["--data-dir"])
            .arg(&self.data)
            .env("PANDORA_CONFIG", &self.config)
            .env("PANDORA_DATA_DIR", &self.data)
            .env("PANDORA_WORKSPACE", &self.workspace)
            .output()
            .expect("setup command should start");
        assert!(
            output.status.success(),
            "setup failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(self.config.exists(), "config file should exist");
    }

    fn run(&self, args: &[&str]) -> JsonResponse {
        let mut full_args = args.to_vec();
        if !full_args.contains(&"--json") {
            full_args.push("--json");
        }
        let output = Command::new(env!("CARGO_BIN_EXE_pandora"))
            .args(&full_args)
            .env("PANDORA_CONFIG", &self.config)
            .env("PANDORA_DATA_DIR", &self.data)
            .env("PANDORA_WORKSPACE", &self.workspace)
            .env("PANDORA_MASTER_KEY", "test-master-key-stress")
            .output()
            .expect("Pandora command should execute");
        JsonResponse::from_output(output)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct JsonResponse {
    status: ExitStatus,
    text: String,
    value: Value,
}

impl JsonResponse {
    fn from_output(output: Output) -> Self {
        let text = String::from_utf8(output.stdout).expect("valid utf-8 stdout");
        let value = serde_json::from_str(&text).unwrap_or(Value::Null);
        Self {
            status: output.status,
            text,
            value,
        }
    }

    fn success(self, command: &str) -> Value {
        assert!(self.status.success(), "command failed: {}", self.text);
        assert_eq!(self.value["version"], "0.1");
        assert_eq!(self.value["command"], command);
        self.value
    }

    fn failure(self) -> (ExitStatus, String) {
        assert!(!self.status.success(), "command unexpectedly succeeded: {}", self.text);
        (self.status, self.text)
    }
}

// ============================================================================
// 1. EMPIRICAL CHALLENGES: BFT BOUNDARY CONDITIONS & EQUIVOCATION
// ============================================================================

#[test]
fn challenge_bft_boundary_conditions_and_equivocation() {
    // 1. Theorem 4: N >= 3f + 1
    // N=4, f=1: Valid (4 >= 3(1)+1). Q = 2(1)+1 = 3.
    let bft_4_1 = BftConsensus::new(4, 1).expect("N=4, f=1 must be valid");
    assert_eq!(bft_4_1.quorum_threshold, 3);
    assert_eq!(bft_4_1.total_nodes, 4);
    assert_eq!(bft_4_1.byzantine_tolerance, 1);

    // N=3, f=1: Invalid (3 < 3(1)+1=4). Must return error.
    let err_3_1 = BftConsensus::new(3, 1);
    assert_eq!(
        err_3_1.unwrap_err(),
        SwarmError::InvalidBftParameters { n: 3, f: 1 }
    );

    // N=7, f=2: Valid (7 >= 3(2)+1=7). Q = 2(2)+1 = 5.
    let mut bft_7_2 = BftConsensus::new(7, 2).expect("N=7, f=2 must be valid");
    assert_eq!(bft_7_2.quorum_threshold, 5);

    // N=6, f=2: Invalid (6 < 3(2)+1=7). Must return error.
    let err_6_2 = BftConsensus::new(6, 2);
    assert_eq!(
        err_6_2.unwrap_err(),
        SwarmError::InvalidBftParameters { n: 6, f: 2 }
    );

    // N=1, f=0: Minimal boundary (1 >= 3(0)+1=1). Q = 1.
    let bft_1_0 = BftConsensus::new(1, 0).expect("N=1, f=0 must be valid");
    assert_eq!(bft_1_0.quorum_threshold, 1);

    // 2. Voting lifecycle and consensus threshold dynamics on N=7, f=2 (Q=5)
    let payload_hash = [0xabu8; 32];
    bft_7_2
        .propose("proposer-0", "proposal-7-2", payload_hash, 1000)
        .expect("proposal should succeed");

    // Duplicate propose must be rejected
    assert_eq!(
        bft_7_2.propose("proposer-0", "proposal-7-2", payload_hash, 1001),
        Err(SwarmError::ProposalAlreadyExists("proposal-7-2".to_string()))
    );

    // Cast 4 approval votes -> Still Pending (4 < 5)
    for i in 1..=4 {
        let dec = bft_7_2
            .cast_vote(&format!("voter-{i}"), "proposal-7-2", true, vec![])
            .unwrap();
        assert_eq!(dec, ConsensusDecision::Pending);
    }

    // Cast 5th approval vote -> Becomes Approved (5 >= 5)
    let dec_commit = bft_7_2
        .cast_vote("voter-5", "proposal-7-2", true, vec![])
        .unwrap();
    assert_eq!(dec_commit, ConsensusDecision::Approved);

    // Idempotent re-vote from same voter with same stance preserves Approved
    let dec_repeat = bft_7_2
        .cast_vote("voter-5", "proposal-7-2", true, vec![])
        .unwrap();
    assert_eq!(dec_repeat, ConsensusDecision::Approved);

    // 3. Byzantine Equivocation Challenge: Contradictory vote from same voter
    // voter-5 previously voted true; now attempts to vote false -> EquivocationDetected
    let equiv_err = bft_7_2.cast_vote("voter-5", "proposal-7-2", false, vec![]);
    assert_eq!(
        equiv_err.unwrap_err(),
        SwarmError::EquivocationDetected {
            voter: "voter-5".to_string(),
            proposal: "proposal-7-2".to_string(),
        }
    );

    // 4. Rejection threshold dynamics: In N=7, f=2, if rejections > total - Q = 7 - 5 = 2 (i.e. >= 3 rejections)
    let mut bft_reject = BftConsensus::new(7, 2).unwrap();
    bft_reject
        .propose("proposer-0", "proposal-reject", payload_hash, 2000)
        .unwrap();

    // 2 approvals, 2 rejections -> Pending (remaining 3 uncast votes could still reach 5 approvals)
    bft_reject.cast_vote("v1", "proposal-reject", true, vec![]).unwrap();
    bft_reject.cast_vote("v2", "proposal-reject", true, vec![]).unwrap();
    bft_reject.cast_vote("v3", "proposal-reject", false, vec![]).unwrap();
    let dec_p = bft_reject.cast_vote("v4", "proposal-reject", false, vec![]).unwrap();
    assert_eq!(dec_p, ConsensusDecision::Pending);

    // 3rd rejection -> Rejections = 3 > 7 - 5 = 2. Max possible approvals is now 7 - 3 = 4 < 5.
    // Must immediately transition to Rejected!
    let dec_rej = bft_reject.cast_vote("v5", "proposal-reject", false, vec![]).unwrap();
    assert_eq!(dec_rej, ConsensusDecision::Rejected);
}

#[test]
fn challenge_bft_cli_quorum_check_propose_and_vote() {
    let fixture = Fixture::new();
    fixture.setup();

    // 1. CLI Quorum check: N=4, f=1 (valid)
    let resp = fixture.run(&["fleet", "quorum", "check", "--nodes", "4", "--byzantine", "1"]);
    let val = resp.success("fleet quorum check");
    assert_eq!(val["valid"], true);
    assert_eq!(val["quorum_threshold"], 3);
    assert_eq!(val["min_required_nodes"], 4);

    // 2. CLI Quorum check: N=3, f=1 (invalid)
    let resp_inv = fixture.run(&["fleet", "quorum", "check", "--nodes", "3", "--byzantine", "1"]);
    let val_inv = resp_inv.success("fleet quorum check");
    assert_eq!(val_inv["valid"], false);
    assert_eq!(val_inv["min_required_nodes"], 4);

    // 3. CLI Quorum check: N=7, f=2 (valid)
    let resp_7_2 = fixture.run(&["fleet", "quorum", "check", "--nodes", "7", "--byzantine", "2"]);
    let val_7_2 = resp_7_2.success("fleet quorum check");
    assert_eq!(val_7_2["valid"], true);
    assert_eq!(val_7_2["quorum_threshold"], 5);

    // 4. CLI Propose rejection when invalid BFT parameters given
    let resp_fail_prop = fixture.run(&[
        "fleet", "quorum", "propose",
        "--nodes", "3",
        "--byzantine", "1",
        "--proposal-id", "prop-invalid",
    ]);
    let (_, stderr) = resp_fail_prop.failure();
    assert!(stderr.contains("invalid BFT parameters") || stderr.contains("must satisfy N >= 3f + 1"));

    // 5. CLI Propose valid proposal
    let resp_prop = fixture.run(&[
        "fleet", "quorum", "propose",
        "--nodes", "7",
        "--byzantine", "2",
        "--proposal-id", "prop-fleet-72",
        "--payload", "upgrade-payload-123",
    ]);
    let val_prop = resp_prop.success("fleet quorum propose");
    assert_eq!(val_prop["proposal_id"], "prop-fleet-72");
    assert_eq!(val_prop["quorum_threshold"], 5);
    assert_eq!(val_prop["decision"], "Pending");

    // 6. CLI Vote reaching quorum threshold
    let resp_vote = fixture.run(&[
        "fleet", "quorum", "vote",
        "--nodes", "7",
        "--byzantine", "2",
        "--proposal-id", "prop-fleet-72",
        "--payload", "upgrade-payload-123",
        "--votes", "v1:true,v2:true,v3:true,v4:true,v5:true",
    ]);
    let val_vote = resp_vote.success("fleet quorum vote");
    assert_eq!(val_vote["decision"], "Approved");
    assert_eq!(val_vote["votes_cast"], 5);
    assert_eq!(val_vote["quorum_threshold"], 5);
}

// ============================================================================
// 2. EMPIRICAL CHALLENGES: BLOOM FILTER STRESS & P2P GOSSIP DEDUPLICATION
// ============================================================================

#[test]
fn challenge_bloom_filter_stress_fp_rate_and_deduplication() {
    let num_bits = 4096;
    let hash_functions = 7;
    let mut filter = BloomFilter::new(num_bits, hash_functions);

    let inserted_count = 300;
    // Insert 300 items
    for i in 0..inserted_count {
        let key = format!("mesh-message-key-{i}");
        filter.insert(key.as_bytes());
    }

    // 1. Zero False Negatives Invariant: Every inserted item MUST return true
    for i in 0..inserted_count {
        let key = format!("mesh-message-key-{i}");
        assert!(
            filter.contains(key.as_bytes()),
            "False negative detected for item '{key}'!"
        );
    }

    // 2. Empirical False Positive Rate vs Theoretical Bound
    // Theoretical rate for m=4096, k=7, n=300:
    let theoretical_fp = filter.theoretical_false_positive_rate(inserted_count);
    assert!(
        theoretical_fp < 0.02,
        "Theoretical FP rate {theoretical_fp} should be < 2%"
    );

    // Test 10,000 non-inserted random items
    let test_queries = 10_000;
    let mut empirical_fp_count = 0;
    for i in 0..test_queries {
        let query_key = format!("completely-random-uninserted-probe-string-{i}");
        if filter.contains(query_key.as_bytes()) {
            empirical_fp_count += 1;
        }
    }

    let empirical_fp_rate = (empirical_fp_count as f64) / (test_queries as f64);
    // Empirical FP should be close to theoretical FP and well bounded
    assert!(
        empirical_fp_rate < 0.05,
        "Empirical FP rate {empirical_fp_rate} exceeded 5% tolerance (count={empirical_fp_count}/{test_queries})"
    );

    // 3. Clear operation resets all bits
    filter.clear();
    for i in 0..inserted_count {
        let key = format!("mesh-message-key-{i}");
        assert!(
            !filter.contains(key.as_bytes()),
            "Filter should be empty after clear!"
        );
    }
}

#[test]
fn challenge_peer_gossip_duplicate_drops_and_vector_clock() {
    let mut node_a = GossipNode::new("node-alpha", 3);
    let mut node_b = GossipNode::new("node-beta", 3);
    node_a.add_peer("node-beta");
    node_a.add_peer("node-gamma");
    node_b.add_peer("node-alpha");
    node_b.add_peer("node-delta");

    // 1. Node A publishes a message
    let (msg1, targets) = node_a.publish(b"test-gossip-payload-1".to_vec(), 5);
    assert_eq!(targets.len(), 2);
    assert!(targets.contains(&"node-beta".to_string()));
    assert_eq!(msg1.hop_count, 0);
    assert_eq!(msg1.max_hops, 5);

    // 2. Node A attempting to receive its own published message is immediately dropped (Bloom filter deduplication)
    let re_recv_a = node_a.receive(msg1.clone());
    assert!(re_recv_a.is_none(), "Node A must discard its own message as duplicate");

    // 3. Node B receives the message for the first time -> accepted and forward targets selected (excluding origin node-alpha)
    let targets_b = node_b.receive(msg1.clone());
    assert_eq!(targets_b, Some(vec!["node-delta".to_string()]));


    // 4. Node B receives the EXACT SAME message a second and third time -> MUST be dropped
    for _ in 0..5 {
        let dup_b = node_b.receive(msg1.clone());
        assert!(
            dup_b.is_none(),
            "Node B must discard subsequent receipts of message as duplicate"
        );
    }

    // 5. Causal Vector Clock ordering
    // Before exchange: Node A has clock with alpha=1. Node B merged alpha=1 and incremented beta=1.
    assert_eq!(node_b.clock.get("node-alpha"), 1);
    assert_eq!(node_b.clock.get("node-beta"), 1);

    // msg1 clock causally precedes node_b's clock
    assert!(msg1.vector_clock.causally_precedes(&node_b.clock));
    assert!(!node_b.clock.causally_precedes(&msg1.vector_clock));

    // 6. Max hops expiry: Message with hop_count >= max_hops is accepted locally but NOT forwarded
    let mut expired_msg = msg1.clone();
    expired_msg.id = "different-expired-msg-id".to_string();
    expired_msg.hop_count = 5;
    expired_msg.max_hops = 5;
    let forward_expired = node_b.receive(expired_msg);
    assert!(
        forward_expired.is_none(),
        "Expired message (hop_count >= max_hops) must not be forwarded"
    );
}

#[test]
fn challenge_peer_gossip_cli_broadcast_receive_filter() {
    let fixture = Fixture::new();
    fixture.setup();

    // 1. Broadcast command
    let resp_bcast = fixture.run(&[
        "peer", "gossip", "broadcast",
        "--node-id", "stress-node-1",
        "--payload", "critical-state-sync",
        "--fanout", "4",
        "--peers", "peer-a,peer-b,peer-c",
    ]);
    let val_bcast = resp_bcast.success("peer gossip broadcast");
    let msg_id = val_bcast["message_id"].as_str().unwrap().to_string();
    assert_eq!(val_bcast["origin_node"], "stress-node-1");
    assert_eq!(val_bcast["targets"].as_array().unwrap().len(), 3);

    // 2. Receive command (first receipt: accepted)
    let resp_recv1 = fixture.run(&[
        "peer", "gossip", "receive",
        "--node-id", "stress-node-2",
        "--message-id", &msg_id,
        "--origin", "stress-node-1",
        "--payload", "critical-state-sync",
    ]);
    let val_recv1 = resp_recv1.success("peer gossip receive");
    assert_eq!(val_recv1["accepted"], true);

    // 3. Filter query command
    let resp_filt = fixture.run(&[
        "peer", "gossip", "filter",
        "--action", "contains",
        "--item", "nonexistent-item-key",
        "--num-bits", "2048",
        "--hash-functions", "7",
    ]);
    let val_filt = resp_filt.success("peer gossip filter");
    assert_eq!(val_filt["contains"], false);
    assert!(val_filt["theoretical_fp_rate"].as_f64().unwrap() < 0.01);
}

// ============================================================================
// 3. EMPIRICAL CHALLENGES: DAG SCHEDULING TOPOLOGIES & CYCLIC REJECTION
// ============================================================================

#[test]
fn challenge_dag_scheduling_topologies_and_invariants() {
    // 1. Empty DAG Boundary
    let empty_dag = TaskDag::new();
    assert_eq!(empty_dag.topological_sort().unwrap(), Vec::<String>::new());
    assert_eq!(empty_dag.total_work(), 0);
    assert_eq!(empty_dag.critical_path_span().unwrap(), 0);
    assert_eq!(empty_dag.simulate_work_stealing(4).unwrap(), 0);

    // 2. Single Task DAG Boundary
    let mut single_dag = TaskDag::new();
    single_dag.add_task("solo", 42).unwrap();
    assert_eq!(single_dag.topological_sort().unwrap(), vec!["solo"]);
    assert_eq!(single_dag.total_work(), 42);
    assert_eq!(single_dag.critical_path_span().unwrap(), 42);
    assert_eq!(single_dag.simulate_work_stealing(1).unwrap(), 42);
    assert_eq!(single_dag.simulate_work_stealing(8).unwrap(), 42);

    // 3. Linear Chain DAG: A -> B -> C -> D (No concurrency possible)
    let mut chain = TaskDag::new();
    chain.add_task("A", 10).unwrap();
    chain.add_task("B", 20).unwrap();
    chain.add_task("C", 30).unwrap();
    chain.add_task("D", 40).unwrap();
    chain.add_dependency("A", "B").unwrap();
    chain.add_dependency("B", "C").unwrap();
    chain.add_dependency("C", "D").unwrap();

    let chain_topo = chain.topological_sort().unwrap();
    assert_eq!(chain_topo, vec!["A", "B", "C", "D"]);
    assert_eq!(chain.total_work(), 100);
    assert_eq!(chain.critical_path_span().unwrap(), 100);
    // Makespan must equal 100 regardless of number of workers (serial bottleneck)
    assert_eq!(chain.simulate_work_stealing(1).unwrap(), 100);
    assert_eq!(chain.simulate_work_stealing(2).unwrap(), 100);
    assert_eq!(chain.simulate_work_stealing(8).unwrap(), 100);

    // 4. Diamond DAG: Init(10) -> A(25), B(35) -> Finalize(15)
    let mut diamond = TaskDag::new();
    diamond.add_task("init", 10).unwrap();
    diamond.add_task("A", 25).unwrap();
    diamond.add_task("B", 35).unwrap();
    diamond.add_task("finalize", 15).unwrap();
    diamond.add_dependency("init", "A").unwrap();
    diamond.add_dependency("init", "B").unwrap();
    diamond.add_dependency("A", "finalize").unwrap();
    diamond.add_dependency("B", "finalize").unwrap();

    assert_eq!(diamond.total_work(), 85);
    // Critical path: init(10) + B(35) + finalize(15) = 60
    assert_eq!(diamond.critical_path_span().unwrap(), 60);

    // 1 worker: serial execution T_1 = 85
    let ms_1 = diamond.simulate_work_stealing(1).unwrap();
    assert_eq!(ms_1, 85);

    // 2 workers: A and B run concurrently.
    // Time 0..10: init.
    // Time 10..35: A runs on W1 (finishes at 35).
    // Time 10..45: B runs on W2 (finishes at 45).
    // Time 45..60: finalize runs on W1. Total = 60.
    let ms_2 = diamond.simulate_work_stealing(2).unwrap();
    assert_eq!(ms_2, 60);

    // Speedup with 2 workers: 85 / 60 = 1.4166x
    let speedup_2 = (85.0) / (ms_2 as f64);
    assert!((speedup_2 - 1.4166).abs() < 0.01);

    // 4 workers: bounded by critical path span T_infinity = 60
    let ms_4 = diamond.simulate_work_stealing(4).unwrap();
    assert_eq!(ms_4, 60);

    // 5. Wide Parallel DAG: fork(10) -> 8 tasks of weight 20 -> join(10)
    let mut wide = TaskDag::new();
    wide.add_task("fork", 10).unwrap();
    wide.add_task("join", 10).unwrap();
    for i in 0..8 {
        let name = format!("par-{i}");
        wide.add_task(&name, 20).unwrap();
        wide.add_dependency("fork", &name).unwrap();
        wide.add_dependency(&name, "join").unwrap();
    }
    assert_eq!(wide.total_work(), 10 + 8 * 20 + 10); // 180
    assert_eq!(wide.critical_path_span().unwrap(), 40); // 10 + 20 + 10

    // With 1 worker: 180
    assert_eq!(wide.simulate_work_stealing(1).unwrap(), 180);
    // With 8 workers: all 8 tasks parallel -> makespan = 40 (critical path minimum)
    assert_eq!(wide.simulate_work_stealing(8).unwrap(), 40);
}

#[test]
fn challenge_dag_cyclic_task_sets_rejected() {
    // 1. Simple 2-cycle: A -> B -> A
    let mut dag_2 = TaskDag::new();
    dag_2.add_task("A", 10).unwrap();
    dag_2.add_task("B", 10).unwrap();
    dag_2.add_dependency("A", "B").unwrap();
    let err_2 = dag_2.add_dependency("B", "A");
    assert!(
        matches!(err_2, Err(SwarmError::DagCycleDetected(_))),
        "2-cycle must be rejected with DagCycleDetected"
    );

    // 2. 3-cycle: A -> B -> C -> A
    let mut dag_3 = TaskDag::new();
    dag_3.add_task("A", 10).unwrap();
    dag_3.add_task("B", 10).unwrap();
    dag_3.add_task("C", 10).unwrap();
    dag_3.add_dependency("A", "B").unwrap();
    dag_3.add_dependency("B", "C").unwrap();
    let err_3 = dag_3.add_dependency("C", "A");
    assert!(
        matches!(err_3, Err(SwarmError::DagCycleDetected(_))),
        "3-cycle must be rejected with DagCycleDetected"
    );

    // 3. Self-loop: A -> A
    let mut dag_self = TaskDag::new();
    dag_self.add_task("A", 10).unwrap();
    let err_self = dag_self.add_dependency("A", "A");
    assert!(
        matches!(err_self, Err(SwarmError::InvalidDependency { .. })),
        "Self-dependency must be rejected with InvalidDependency"
    );

    // 4. Duplicate task creation must be rejected
    assert_eq!(
        dag_self.add_task("A", 20),
        Err(SwarmError::TaskAlreadyExists("A".to_string()))
    );
}

#[test]
fn challenge_dag_cli_simulation_presets_and_custom() {
    let fixture = Fixture::new();
    fixture.setup();

    // 1. Preset pipeline
    let resp_pipe = fixture.run(&["fleet", "dag", "--preset", "pipeline", "--workers", "2"]);
    let val_pipe = resp_pipe.success("fleet dag");
    assert_eq!(val_pipe["tasks_count"], 3);
    assert_eq!(val_pipe["total_work_t1"], 45); // 10 + 20 + 15
    assert_eq!(val_pipe["critical_path_t_inf"], 45);

    // 2. Custom DAG via CLI: tasks and dependencies
    let resp_custom = fixture.run(&[
        "fleet", "dag",
        "--tasks", "build:30,test_unit:15,test_int:25,package:10",
        "--dependencies", "build->test_unit,build->test_int,test_unit->package,test_int->package",
        "--workers", "2",
    ]);
    let val_custom = resp_custom.success("fleet dag");
    assert_eq!(val_custom["tasks_count"], 4);
    assert_eq!(val_custom["total_work_t1"], 80); // 30 + 15 + 25 + 10
    assert_eq!(val_custom["critical_path_t_inf"], 65); // build(30) + test_int(25) + package(10)

    // 3. Custom DAG with cyclic dependency must fail with error
    let resp_cyclic = fixture.run(&[
        "fleet", "dag",
        "--tasks", "task1:10,task2:20",
        "--dependencies", "task1->task2,task2->task1",
    ]);
    let (_, stderr) = resp_cyclic.failure();
    assert!(stderr.contains("Cycle detected") || stderr.contains("failed to add dependency"));
}

// ============================================================================
// 4. EMPIRICAL CHALLENGES: VISUAL GRAPH LYAPUNOV STABILITY & ASCII RASTERIZER
// ============================================================================

#[test]
fn challenge_graph_visual_empty_and_disconnected_components() {
    // 1. Empty Graph: No nodes, no edges
    let mut empty_layout = ForceDirectedLayout::new(60.0, 20.0);
    assert_eq!(empty_layout.potential_energy(), 0.0);
    let empty_iters = empty_layout.simulate_until_equilibrium(50, 0.01);
    assert_eq!(empty_iters, 1);
    let camera = ViewportCamera::new();
    let rendered_empty = AsciiGridRenderer::render(&empty_layout, &camera, 60, 20);
    assert_eq!(rendered_empty.len(), 20);
    for line in rendered_empty {
        assert_eq!(line.len(), 60);
        assert!(line.chars().all(|c| c == ' '));
    }

    // 2. Disconnected Components: C1 (n1 - n2) and C2 (n3 - n4)
    let mut disc_layout = ForceDirectedLayout::new(80.0, 40.0);
    disc_layout.add_node("n1", "Comp1A", 20.0, 20.0);
    disc_layout.add_node("n2", "Comp1B", 25.0, 20.0);
    disc_layout.add_edge("n1", "n2");

    disc_layout.add_node("n3", "Comp2A", 60.0, 20.0);
    disc_layout.add_node("n4", "Comp2B", 65.0, 20.0);
    disc_layout.add_edge("n3", "n4");

    assert_eq!(disc_layout.nodes.len(), 4);
    assert_eq!(disc_layout.edges.len(), 2);
    let initial_energy = disc_layout.potential_energy();
    assert!(initial_energy > 0.0);

    // Simulation must converge without crashing or NaNs
    let iters = disc_layout.simulate_until_equilibrium(100, 0.05);
    assert!(iters <= 100);
    for node in disc_layout.nodes.values() {
        assert!(!node.pos.x.is_nan());
        assert!(!node.pos.y.is_nan());
        assert!(node.pos.x >= 2.0 && node.pos.x <= 78.0);
        assert!(node.pos.y >= 2.0 && node.pos.y <= 38.0);
    }
}

#[test]
fn challenge_graph_visual_large_node_count_and_lyapunov_convergence() {
    let mut layout = ForceDirectedLayout::new(100.0, 50.0);
    let num_nodes = 30;

    // Create a circular ring of 30 nodes with mutual springs
    for i in 0..num_nodes {
        let angle = (i as f64) * 2.0 * std::f64::consts::PI / (num_nodes as f64);
        let x = 50.0 + 20.0 * angle.cos();
        let y = 25.0 + 10.0 * angle.sin();
        layout.add_node(&format!("v_{i}"), &format!("node_{i}"), x, y);
    }
    for i in 0..num_nodes {
        let next = (i + 1) % num_nodes;
        layout.add_edge(&format!("v_{i}"), &format!("v_{next}"));
    }

    // Verify Lyapunov Kinetic Energy Dissipation
    let dt = 0.1;
    let initial_ke = layout.step(dt);
    println!("Step 0 KE: {initial_ke}");
    for step in 1..=100 {
        let ke = layout.step(dt);
        if step % 10 == 0 || step <= 10 {
            println!("Step {step} KE: {ke}");
        }
    }


    // Due to velocity damping gamma = 0.85, kinetic energy dissipates towards equilibrium
    assert!(
        layout.step(dt) < 5.0,
        "Kinetic energy should dissipate towards equilibrium"
    );

    // Ensure all 30 nodes and edges render cleanly into ASCII grid
    // With aspect_ratio = 2.0, zoom = 0.5 maps [0, 100] world x to [0, 100] screen columns
    let mut camera = ViewportCamera::new();
    camera.focus_on(Point2D::new(50.0, 25.0));
    camera.zoom = 0.5;
    let rendered = AsciiGridRenderer::render(&layout, &camera, 100, 50);
    assert_eq!(rendered.len(), 50);
    let rendered_text = rendered.join("\n");
    assert!(rendered_text.contains("[node_0]") || rendered_text.contains("[node_1]"));
    assert!(rendered_text.contains(".")); // Edge Bresenham dots
}

#[test]
fn challenge_graph_visual_extreme_dimensions_and_camera_steering() {
    let mut layout = ForceDirectedLayout::new(40.0, 20.0);
    layout.add_node("a", "Alpha", 10.0, 10.0);
    layout.add_node("b", "Beta", 30.0, 10.0);
    layout.add_edge("a", "b");

    let mut camera = ViewportCamera::new();

    // 1. Extreme zoom in: zoom = 10.0
    camera.zoom = 10.0;
    let rendered_zoom = AsciiGridRenderer::render(&layout, &camera, 40, 20);
    assert_eq!(rendered_zoom.len(), 20);

    // 2. Extreme zoom out: zoom = 0.2
    camera.zoom = 0.2;
    let rendered_zoom_out = AsciiGridRenderer::render(&layout, &camera, 40, 20);
    assert_eq!(rendered_zoom_out.len(), 20);

    // 3. Extreme pan off-screen: pan_x = 1000.0
    camera.pan(1000.0, 1000.0);
    let rendered_panned = AsciiGridRenderer::render(&layout, &camera, 40, 20);
    assert_eq!(rendered_panned.len(), 20);

    // 4. Tiny grid dimension boundary: 10 x 5
    let rendered_tiny = AsciiGridRenderer::render(&layout, &camera, 10, 5);
    assert_eq!(rendered_tiny.len(), 5);
    assert_eq!(rendered_tiny[0].len(), 10);
}

#[test]
fn challenge_graph_visual_cli_rendering_subcommands() {
    let fixture = Fixture::new();
    fixture.setup();

    // 1. Knowledge graph visual
    let resp_k = fixture.run(&["graph", "visual", "--kind", "knowledge", "--width", "50", "--height", "15"]);
    let val_k = resp_k.success("graph visual");
    assert_eq!(val_k["kind"], "knowledge");
    assert!(val_k["nodes_count"].as_u64().unwrap() >= 4);
    assert!(val_k["rendered"].as_str().unwrap().contains("[Kernel]") || val_k["rendered"].as_str().unwrap().contains("[VFS]"));

    // 2. Architecture graph visual
    let resp_arch = fixture.run(&["graph", "visual", "--kind", "architecture", "--width", "50", "--height", "15"]);
    let val_arch = resp_arch.success("graph visual");
    assert_eq!(val_arch["kind"], "architecture");
    assert!(val_arch["nodes_count"].as_u64().unwrap() >= 4);

    // 3. Review graph visual
    let resp_rev = fixture.run(&["graph", "visual", "--kind", "review", "--width", "50", "--height", "15"]);
    let val_rev = resp_rev.success("graph visual");
    assert_eq!(val_rev["kind"], "review");

    // 4. Code graph visual
    let resp_code = fixture.run(&["graph", "code", "visual", "--width", "60", "--height", "20"]);
    let val_code = resp_code.success("graph visual");
    assert_eq!(val_code["kind"], "code");
    assert!(val_code["rendered"].as_str().unwrap().contains("cli"));
}
