#![forbid(unsafe_code)]

use pandora_runtime::bandit_router::{
    BanditArm, BanditModelRouter, BanditStrategy, RoutingContext,
};
use pandora_runtime::merkle_dag::{
    DagError, MerkleDagAuditLog, NodeDirection,
};
use std::collections::HashSet;

// ============================================================================
// PART 1: BANDIT MODEL ROUTER ADVERSARIAL & STRESS TESTS
// ============================================================================

#[test]
fn test_stress_bandit_0_arms_edge_case() {
    let ctx = RoutingContext::new(256, 0.5, 0.5, 3000.0);

    // Test UCB1 with 0 arms
    let mut router_ucb1 = BanditModelRouter::new(BanditStrategy::Ucb1 { c: 1.414 });
    assert_eq!(router_ucb1.arm_count(), 0);
    assert_eq!(router_ucb1.select_arm(&ctx), None);
    router_ucb1.record_reward("non_existent_arm", 1.0);
    assert_eq!(router_ucb1.total_pulls(), 0);
    assert_eq!(router_ucb1.cumulative_regret(1.0), 0.0);
    assert_eq!(BanditModelRouter::theoretical_minimax_regret(0, 100), 0.0);

    // Test Thompson Sampling with 0 arms
    let mut router_ts = BanditModelRouter::new(BanditStrategy::ThompsonSampling);
    assert_eq!(router_ts.select_arm(&ctx), None);
    router_ts.record_reward("non_existent_arm", 0.5);
    assert_eq!(router_ts.total_pulls(), 0);

    // Test Epsilon Greedy with 0 arms
    let mut router_eg = BanditModelRouter::new(BanditStrategy::EpsilonGreedy { epsilon: 0.2 });
    assert_eq!(router_eg.select_arm(&ctx), None);
}

#[test]
fn test_stress_bandit_1_arm_edge_case() {
    let ctx = RoutingContext::new(512, 0.7, 0.3, 2500.0);

    // 1 arm under UCB1
    let mut router_ucb1 = BanditModelRouter::new(BanditStrategy::Ucb1 { c: 1.414 });
    router_ucb1.register_arm(BanditArm::new("solo_arm", "model-solo", "prov-solo"));
    assert_eq!(router_ucb1.arm_count(), 1);

    for _ in 0..200 {
        let selected = router_ucb1.select_arm(&ctx);
        assert_eq!(selected.as_deref(), Some("solo_arm"));
        router_ucb1.record_reward("solo_arm", 0.75);
    }

    assert_eq!(router_ucb1.total_pulls(), 200);
    let arm = router_ucb1.get_arm("solo_arm").unwrap();
    assert_eq!(arm.pull_count(), 200);
    assert!((arm.mean_reward() - 0.75).abs() < 1e-6);
    assert_eq!(router_ucb1.cumulative_regret(0.75), 0.0);

    // 1 arm under Thompson Sampling
    let mut router_ts = BanditModelRouter::with_seed(BanditStrategy::ThompsonSampling, 42);
    router_ts.register_arm(BanditArm::new("solo_ts", "model-ts", "prov-ts"));

    for _ in 0..200 {
        let selected = router_ts.select_arm(&ctx);
        assert_eq!(selected.as_deref(), Some("solo_ts"));
        router_ts.record_reward("solo_ts", 0.80);
    }

    let arm_ts = router_ts.get_arm("solo_ts").unwrap();
    assert_eq!(arm_ts.pull_count(), 200);
    // Alpha should be 1.0 + 200 * 0.80 = 161.0, Beta = 1.0 + 200 * 0.20 = 41.0
    assert!((arm_ts.alpha() - 161.0).abs() < 1e-6);
    assert!((arm_ts.beta() - 41.0).abs() < 1e-6);
    let posterior_mean = arm_ts.alpha() / (arm_ts.alpha() + arm_ts.beta());
    assert!((posterior_mean - 0.797).abs() < 0.01);
}

#[test]
fn test_stress_bandit_1000_arms_scale() {
    let ctx = RoutingContext::new(128, 0.2, 0.9, 1000.0);
    let mut router_ucb1 = BanditModelRouter::with_seed(BanditStrategy::Ucb1 { c: 1.414 }, 777);

    // Register 1000 arms
    for i in 0..1000 {
        router_ucb1.register_arm(BanditArm::new(
            format!("arm_{i:04}"),
            format!("model_{i:04}"),
            "mock_prov",
        ));
    }
    assert_eq!(router_ucb1.arm_count(), 1000);

    // Stage 1: First 1000 rounds of UCB1: each of the 1000 arms MUST be explored exactly once
    let mut explored_arms = HashSet::new();
    for _ in 0..1000 {
        let chosen = router_ucb1.select_arm(&ctx).expect("arm chosen");
        assert!(
            explored_arms.insert(chosen.clone()),
            "arm {chosen} was re-selected before all 1000 arms were explored"
        );
        // Arm 0420 gives reward 0.95; all other arms give 0.05
        let reward = if chosen == "arm_0420" { 0.95 } else { 0.05 };
        router_ucb1.record_reward(&chosen, reward);
    }
    assert_eq!(explored_arms.len(), 1000);

    // Stage 2: In round 1001, all arms have n=1. Arm 0420 has mean 0.95 while others have 0.05.
    // With equal uncertainty bounds, arm 0420 MUST be selected first for its second pull!
    let round_1001_choice = router_ucb1.select_arm(&ctx).expect("arm chosen");
    assert_eq!(
        round_1001_choice, "arm_0420",
        "arm_0420 must be selected first for 2nd pull due to empirical mean 0.95 vs 0.05"
    );
    router_ucb1.record_reward(&round_1001_choice, 0.95);

    // Continue for remaining 999 rounds of second exploration wave (total T = 2000)
    for _ in 0..999 {
        let chosen = router_ucb1.select_arm(&ctx).expect("arm chosen");
        let reward = if chosen == "arm_0420" { 0.95 } else { 0.05 };
        router_ucb1.record_reward(&chosen, reward);
    }
    assert_eq!(router_ucb1.total_pulls(), 2000);

    // After 2000 pulls, all 1000 arms have been pulled exactly twice
    let arm_420 = router_ucb1.get_arm("arm_0420").unwrap();
    assert_eq!(arm_420.pull_count(), 2);
    assert!((arm_420.mean_reward() - 0.95).abs() < 1e-6);

    // Minimax regret bound check: cumulative regret R(T) must be strictly bounded by theoretical minimax
    let cumulative_regret = router_ucb1.cumulative_regret(0.95);
    let minimax_bound = BanditModelRouter::theoretical_minimax_regret(1000, 2000);
    assert!(
        cumulative_regret < minimax_bound,
        "cumulative regret {cumulative_regret} exceeded theoretical minimax bound {minimax_bound}"
    );

    // Stage 3: In round 2001, all arms have n=2. Arm 0420's empirical mean advantage (0.90)
    // exceeds the exploration bonus difference between n=2 and n=3, so arm 0420 is picked again!
    let round_2001_choice = router_ucb1.select_arm(&ctx).expect("arm chosen");
    assert_eq!(
        round_2001_choice, "arm_0420",
        "arm_0420 must be selected first in third wave at t=2001"
    );

    // Verify Thompson Sampling scale with 1000 arms
    let mut router_ts = BanditModelRouter::with_seed(BanditStrategy::ThompsonSampling, 999);
    for i in 0..1000 {
        router_ts.register_arm(BanditArm::new(
            format!("ts_arm_{i:04}"),
            format!("ts_model_{i:04}"),
            "ts_prov",
        ));
    }
    assert_eq!(router_ts.arm_count(), 1000);

    // Thompson Sampling selection over 1000 arms executes sub-millisecond without panic
    let ts_choice = router_ts.select_arm(&ctx).expect("TS choice");
    assert!(ts_choice.starts_with("ts_arm_"));
    router_ts.record_reward(&ts_choice, 0.90);
    assert_eq!(router_ts.total_pulls(), 1);
}

#[test]
fn test_stress_bandit_identical_rewards_zero_and_one() {
    let ctx = RoutingContext::new(256, 0.5, 0.5, 2000.0);

    // Scenario A: All arms return 0.0
    let mut router_zero = BanditModelRouter::with_seed(BanditStrategy::Ucb1 { c: 1.414 }, 101);
    for i in 0..4 {
        router_zero.register_arm(BanditArm::new(format!("zero_arm_{i}"), "m", "p"));
    }

    for _ in 0..400 {
        let chosen = router_zero.select_arm(&ctx).unwrap();
        router_zero.record_reward(&chosen, 0.0);
    }

    assert_eq!(router_zero.total_pulls(), 400);
    for i in 0..4 {
        let arm = router_zero.get_arm(&format!("zero_arm_{i}")).unwrap();
        assert_eq!(arm.mean_reward(), 0.0);
        // Due to identical 0 reward and UCB exploration bonus, pulls should be evenly balanced (100 each)
        assert!(
            (arm.pull_count() as i64 - 100).abs() <= 2,
            "zero reward arm pull count {} drifted from balanced 100",
            arm.pull_count()
        );
    }
    assert_eq!(router_zero.cumulative_regret(0.0), 0.0);

    // Scenario B: All arms return 1.0
    let mut router_one = BanditModelRouter::with_seed(BanditStrategy::Ucb1 { c: 1.414 }, 202);
    for i in 0..4 {
        router_one.register_arm(BanditArm::new(format!("one_arm_{i}"), "m", "p"));
    }

    for _ in 0..400 {
        let chosen = router_one.select_arm(&ctx).unwrap();
        router_one.record_reward(&chosen, 1.0);
    }

    assert_eq!(router_one.total_pulls(), 400);
    for i in 0..4 {
        let arm = router_one.get_arm(&format!("one_arm_{i}")).unwrap();
        assert_eq!(arm.mean_reward(), 1.0);
        assert!(
            (arm.pull_count() as i64 - 100).abs() <= 2,
            "one reward arm pull count {} drifted from balanced 100",
            arm.pull_count()
        );
    }
    assert_eq!(router_one.cumulative_regret(1.0), 0.0);

    // Scenario C: Thompson sampling with all 0.0 or 1.0
    let mut router_ts = BanditModelRouter::with_seed(BanditStrategy::ThompsonSampling, 303);
    router_ts.register_arm(BanditArm::new("ts_zero", "m", "p"));
    for _ in 0..100 {
        let chosen = router_ts.select_arm(&ctx).unwrap();
        router_ts.record_reward(&chosen, 0.0);
    }
    let arm = router_ts.get_arm("ts_zero").unwrap();
    assert_eq!(arm.alpha(), 1.0);
    assert_eq!(arm.beta(), 101.0);
}

#[test]
fn test_stress_bandit_adversarial_reward_shift() {
    let ctx = RoutingContext::new(128, 0.5, 0.5, 1000.0);
    let mut router = BanditModelRouter::with_seed(BanditStrategy::Ucb1 { c: 2.0 }, 12345);

    router.register_arm(BanditArm::new("arm_a", "m_a", "p_a"));
    router.register_arm(BanditArm::new("arm_b", "m_b", "p_b"));

    // Phase 1 (rounds 1..250): Arm A is superior (0.90), Arm B is inferior (0.10)
    for _ in 0..250 {
        let chosen = router.select_arm(&ctx).unwrap();
        let reward = if chosen == "arm_a" { 0.90 } else { 0.10 };
        router.record_reward(&chosen, reward);
    }

    let arm_a_p1 = router.get_arm("arm_a").unwrap().pull_count();
    let arm_b_p1 = router.get_arm("arm_b").unwrap().pull_count();
    assert!(
        arm_a_p1 > arm_b_p1 * 4,
        "Phase 1: Arm A ({arm_a_p1}) must strongly outperform Arm B ({arm_b_p1})"
    );

    // Phase 2 (rounds 251..1200): Adversarial Inversion! Arm A drops to 0.10, Arm B jumps to 0.90!
    for _ in 0..950 {
        let chosen = router.select_arm(&ctx).unwrap();
        let reward = if chosen == "arm_a" { 0.10 } else { 0.90 };
        router.record_reward(&chosen, reward);
    }

    let arm_a_p2 = router.get_arm("arm_a").unwrap().pull_count() - arm_a_p1;
    let arm_b_p2 = router.get_arm("arm_b").unwrap().pull_count() - arm_b_p1;

    // In Phase 2, UCB exploration should detect the shift and Arm B should receive the vast majority of pulls
    assert!(
        arm_b_p2 > arm_a_p2 * 3,
        "Phase 2: Router must adapt to reward shift! Arm B pulls ({arm_b_p2}) vs Arm A pulls ({arm_a_p2})"
    );
}

#[test]
fn test_stress_bandit_empirical_cumulative_regret_bounds() {
    let ctx = RoutingContext::new(256, 0.5, 0.5, 1500.0);

    // Test cumulative regret across multiple (K, T) combinations
    let test_configs = [
        (2, 100),
        (3, 300),
        (5, 500),
        (10, 1000),
    ];

    for &(k, t) in &test_configs {
        let mut router = BanditModelRouter::with_seed(BanditStrategy::Ucb1 { c: 1.414 }, 8888 + k as u64);

        for i in 0..k {
            router.register_arm(BanditArm::new(format!("k_arm_{i}"), "model", "prov"));
        }

        let optimal_mean = 0.85;

        for _ in 0..t {
            let chosen = router.select_arm(&ctx).unwrap();
            let arm_idx: usize = chosen.trim_start_matches("k_arm_").parse().unwrap();
            // Arm 0 is optimal (0.85), others linearly degrade
            let true_mean = if arm_idx == 0 {
                optimal_mean
            } else {
                (optimal_mean - 0.10 * (arm_idx as f64)).max(0.10)
            };
            router.record_reward(&chosen, true_mean);
        }

        let emp_regret = router.cumulative_regret(optimal_mean);
        let minimax_bound = BanditModelRouter::theoretical_minimax_regret(k, t);

        assert!(
            emp_regret < minimax_bound,
            "For K={k}, T={t}: cumulative regret {emp_regret} violated theoretical minimax bound {minimax_bound}"
        );

        // Verify sublinear growth rate: average regret per step R(T)/T decreases
        let avg_regret = emp_regret / (t as f64);
        assert!(
            avg_regret < 0.40,
            "For K={k}, T={t}: average regret {avg_regret} too high"
        );
    }
}

// ============================================================================
// PART 2: MERKLE-DAG AUDIT LOG ADVERSARIAL & STRESS TESTS
// ============================================================================

#[test]
fn test_stress_merkle_degenerate_and_scale_sizes() {
    let test_sizes = [0, 1, 2, 3, 31, 32, 33, 100, 1000];

    for &n in &test_sizes {
        let mut log = MerkleDagAuditLog::new();
        for i in 0..n {
            log.append_leaf(format!("audit_record_payload_{n}_{i}").as_bytes());
        }

        assert_eq!(log.leaf_count(), n);

        if n == 0 {
            assert_eq!(log.root_hash(), [0u8; 32]);
            assert!(log.generate_proof(0).is_none());
            continue;
        }

        if n == 1 {
            assert_eq!(log.root_hash(), log.leaves()[0]);
            let proof = log.generate_proof(0).expect("proof for n=1");
            assert_eq!(proof.proof_length(), 0);
            assert!(proof.verify());
            assert!(log.generate_proof(1).is_none());
            continue;
        }

        let root = log.root_hash();
        assert_ne!(root, [0u8; 32]);

        let expected_height = (n as f64).log2().ceil() as usize;

        // For large n (1000), test critical boundaries and sampled indices
        let indices_to_check: Vec<usize> = if n > 100 {
            let mut set = vec![0, 1, 2, n / 4, n / 2, 3 * n / 4, n - 3, n - 2, n - 1];
            for step in (10..n).step_by(27) {
                set.push(step);
            }
            set.sort();
            set.dedup();
            set
        } else {
            (0..n).collect()
        };

        for &i in &indices_to_check {
            let proof = log.generate_proof(i).unwrap_or_else(|| {
                panic!("failed to generate proof for n={n}, leaf={i}")
            });
            assert_eq!(
                proof.proof_length(),
                expected_height,
                "height mismatch for n={n}, leaf={i}: got {}, expected {}",
                proof.proof_length(),
                expected_height
            );
            assert!(
                proof.verify(),
                "proof verification failed for n={n}, leaf={i}"
            );
            assert!(
                MerkleDagAuditLog::verify_proof(&proof),
                "standalone verification failed for n={n}, leaf={i}"
            );
        }
    }
}

#[test]
fn test_stress_merkle_proof_tampering_and_out_of_range() {
    let mut log = MerkleDagAuditLog::new();
    let n = 33; // Unbalanced size spanning power of two 32
    for i in 0..n {
        log.append_leaf(format!("tamper_test_event_{i}").as_bytes());
    }

    // Out of range indices
    assert!(log.generate_proof(n).is_none());
    assert!(log.generate_proof(n + 1).is_none());
    assert!(log.generate_proof(1000).is_none());
    assert!(log.generate_proof(usize::MAX).is_none());

    // Generate valid proof for leaf 17
    let valid_proof = log.generate_proof(17).expect("proof for leaf 17");
    assert!(valid_proof.verify());

    // 1. Bit flip in leaf hash
    for byte_idx in [0, 15, 31] {
        let mut tampered = valid_proof.clone();
        tampered.leaf_hash[byte_idx] ^= 0x01;
        assert!(!tampered.verify(), "tampered leaf byte {byte_idx} must fail");
    }

    // 2. Bit flip in every sibling step
    for step_idx in 0..valid_proof.steps.len() {
        let mut tampered = valid_proof.clone();
        tampered.steps[step_idx].sibling_hash[0] ^= 0x80;
        assert!(
            !tampered.verify(),
            "tampered sibling at step {step_idx} must fail"
        );
    }

    // 3. Flipped sibling direction at every step
    for step_idx in 0..valid_proof.steps.len() {
        let mut tampered = valid_proof.clone();
        tampered.steps[step_idx].direction = match tampered.steps[step_idx].direction {
            NodeDirection::Left => NodeDirection::Right,
            NodeDirection::Right => NodeDirection::Left,
        };
        assert!(
            !tampered.verify(),
            "inverted direction at step {step_idx} must fail"
        );
    }

    // 4. Forged root hash
    let mut tampered_root = valid_proof.clone();
    tampered_root.root_hash[31] ^= 0x42;
    assert!(!tampered_root.verify(), "tampered root hash must fail");

    // 5. Truncated proof steps
    let mut truncated_proof = valid_proof.clone();
    truncated_proof.steps.pop();
    assert!(!truncated_proof.verify(), "truncated proof steps must fail");

    // 6. Cross-leaf substitution: valid proof steps for leaf 17, but leaf_hash from leaf 18
    let proof_18 = log.generate_proof(18).unwrap();
    let mut swapped_leaf = valid_proof.clone();
    swapped_leaf.leaf_hash = proof_18.leaf_hash;
    assert!(!swapped_leaf.verify(), "swapped leaf hash must fail");
}

#[test]
fn test_stress_merkle_dag_disconnected_components() {
    let mut log = MerkleDagAuditLog::new();

    // Disconnected Component 1 (Sub-agent Alpha)
    let c1_root = log.add_dag_node(b"c1_genesis", &[], 100).unwrap();
    let c1_step1 = log.add_dag_node(b"c1_step1", std::slice::from_ref(&c1_root.id), 110).unwrap();
    let c1_step2 = log.add_dag_node(b"c1_step2", std::slice::from_ref(&c1_step1.id), 120).unwrap();

    // Disconnected Component 2 (Sub-agent Beta)
    let c2_root = log.add_dag_node(b"c2_genesis", &[], 200).unwrap();
    let c2_step1 = log.add_dag_node(b"c2_step1", std::slice::from_ref(&c2_root.id), 210).unwrap();
    let c2_step2 = log.add_dag_node(b"c2_step2", std::slice::from_ref(&c2_step1.id), 220).unwrap();

    assert_eq!(log.dag_node_count(), 6);

    // Verify causal lineage for each component independently
    assert!(log.verify_causal_lineage(&c1_step2.id).unwrap());
    assert!(log.verify_causal_lineage(&c2_step2.id).unwrap());

    // Verify topological order across disconnected components
    let topo = log.topological_order().expect("topological order exists");
    assert_eq!(topo.len(), 6);

    let pos = |id: &str| topo.iter().position(|x| x == id).unwrap();

    // Component 1 causal order
    assert!(pos(&c1_root.id) < pos(&c1_step1.id));
    assert!(pos(&c1_step1.id) < pos(&c1_step2.id));

    // Component 2 causal order
    assert!(pos(&c2_root.id) < pos(&c2_step1.id));
    assert!(pos(&c2_step1.id) < pos(&c2_step2.id));

    assert!(log.is_acyclic());
}

#[test]
fn test_stress_merkle_dag_deep_chains() {
    let mut log = MerkleDagAuditLog::new();

    let depth = 500;
    let mut current_id = log.add_dag_node(b"deep_genesis", &[], 1000).unwrap().id;

    for i in 1..depth {
        let next_node = log.add_dag_node(
            format!("deep_chain_link_{i}").as_bytes(),
            std::slice::from_ref(&current_id),
            1000 + i as u64,
        ).unwrap();
        current_id = next_node.id;
    }

    assert_eq!(log.dag_node_count(), depth);

    // Iterative BFS causal lineage verification must not stack-overflow
    assert!(
        log.verify_causal_lineage(&current_id).unwrap(),
        "causal lineage failed for 500-deep chain"
    );

    // Topological order must succeed and match chain length
    let topo = log.topological_order().unwrap();
    assert_eq!(topo.len(), depth);
    assert_eq!(topo.last().unwrap(), &current_id);
    assert!(log.is_acyclic());
}

#[test]
fn test_stress_merkle_dag_diamond_and_lattice_topologies() {
    let mut log = MerkleDagAuditLog::new();

    // Diamond 1: Root -> A, B -> M1
    let root = log.add_dag_node(b"root", &[], 100).unwrap();
    let a = log.add_dag_node(b"branch_a", std::slice::from_ref(&root.id), 110).unwrap();
    let b = log.add_dag_node(b"branch_b", std::slice::from_ref(&root.id), 120).unwrap();

    // Test parent deduplication in add_dag_node
    let m1 = log.add_dag_node(
        b"merge_m1",
        &[a.id.clone(), b.id.clone(), a.id.clone(), b.id.clone()],
        130,
    ).unwrap();
    assert_eq!(m1.parent_ids.len(), 2, "duplicate parents should be deduplicated");

    // Diamond 2 off M1: M1 -> C, D, E -> M2 (3-way merge)
    let c = log.add_dag_node(b"branch_c", std::slice::from_ref(&m1.id), 140).unwrap();
    let d = log.add_dag_node(b"branch_d", std::slice::from_ref(&m1.id), 150).unwrap();
    let e = log.add_dag_node(b"branch_e", std::slice::from_ref(&m1.id), 160).unwrap();

    let m2 = log.add_dag_node(b"merge_m2", &[c.id.clone(), d.id.clone(), e.id.clone()], 170).unwrap();
    assert_eq!(m2.parent_ids.len(), 3);

    // Verify lineage back to original root
    assert!(log.verify_causal_lineage(&m2.id).unwrap());

    // Topological order validation
    let topo = log.topological_order().unwrap();
    assert_eq!(topo.len(), 8);

    let pos = |id: &str| topo.iter().position(|x| x == id).unwrap();
    assert!(pos(&root.id) < pos(&a.id));
    assert!(pos(&root.id) < pos(&b.id));
    assert!(pos(&a.id) < pos(&m1.id));
    assert!(pos(&b.id) < pos(&m1.id));
    assert!(pos(&m1.id) < pos(&c.id));
    assert!(pos(&m1.id) < pos(&d.id));
    assert!(pos(&m1.id) < pos(&e.id));
    assert!(pos(&c.id) < pos(&m2.id));
    assert!(pos(&d.id) < pos(&m2.id));
    assert!(pos(&e.id) < pos(&m2.id));

    assert!(log.is_acyclic());
}

#[test]
fn test_stress_merkle_dag_cycle_and_invalid_parent_rejection() {
    let mut log = MerkleDagAuditLog::new();

    // 1. Non-existent parent rejection
    let fake_parent = "deadbeefcafe0123456789abcdef0123456789abcdef0123456789abcdef0123".to_string();
    let err = log.add_dag_node(b"child", std::slice::from_ref(&fake_parent), 100);
    assert!(
        matches!(err, Err(DagError::ParentNotFound(ref p)) if p == &fake_parent),
        "non-existent parent must return DagError::ParentNotFound"
    );

    // 2. Genesis node
    let genesis = log.add_dag_node(b"genesis", &[], 100).unwrap();

    // 3. Idempotent addition: adding identical node returns existing node
    let genesis_repeat = log.add_dag_node(b"genesis", &[], 100).unwrap();
    assert_eq!(genesis.id, genesis_repeat.id);
    assert_eq!(log.dag_node_count(), 1);

    // 4. Lineage verification with unknown node returns ParentNotFound
    let missing_lineage = log.verify_causal_lineage("unknown_node_id");
    assert!(matches!(missing_lineage, Err(DagError::ParentNotFound(_))));
}
