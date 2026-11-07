#![forbid(unsafe_code)]

use pandora_runtime::merkle_dag::{
    DagError, MerkleDagAuditLog, NodeDirection, TAG_DAG, TAG_LEAF, TAG_NODE,
};

#[test]
fn test_empty_and_single_leaf() {
    let mut log = MerkleDagAuditLog::new();
    assert_eq!(log.leaf_count(), 0);
    assert_eq!(log.root_hash(), [0u8; 32]);

    let data = b"initial genesis execution record";
    let leaf_hash = log.append_leaf(data);
    assert_eq!(log.leaf_count(), 1);
    assert_eq!(log.root_hash(), leaf_hash);

    let proof = log.generate_proof(0).expect("proof generated");
    assert_eq!(proof.leaf_index, 0);
    assert_eq!(proof.leaf_hash, leaf_hash);
    assert_eq!(proof.proof_length(), 0);
    assert!(proof.verify());
}

#[test]
fn test_power_of_two_merkle_trees() {
    for n in [2, 4, 8, 16] {
        let mut log = MerkleDagAuditLog::new();
        for i in 0..n {
            log.append_leaf(format!("event_{i}").as_bytes());
        }
        assert_eq!(log.leaf_count(), n);

        let expected_height = (n as f64).log2().round() as usize;

        for i in 0..n {
            let proof = log.generate_proof(i).expect("proof must exist");
            assert_eq!(
                proof.proof_length(),
                expected_height,
                "proof length for n={n} leaf={i} must be {expected_height}"
            );
            assert!(
                proof.verify(),
                "proof verification failed for n={n} leaf={i}"
            );
        }
    }
}

#[test]
fn test_unbalanced_and_prime_tree_sizes() {
    for n in [3, 5, 7, 13, 29, 100] {
        let mut log = MerkleDagAuditLog::new();
        for i in 0..n {
            log.append_leaf(format!("audit_event_payload_{i}").as_bytes());
        }

        let expected_height = (n as f64).log2().ceil() as usize;

        for i in 0..n {
            let proof = log.generate_proof(i).expect("proof must exist");
            assert_eq!(
                proof.proof_length(),
                expected_height,
                "proof height mismatch for n={n}, i={i}"
            );
            assert!(proof.verify(), "proof failed for n={n}, leaf={i}");
        }
    }
}

#[test]
fn test_tamper_detection_rejection() {
    let mut log = MerkleDagAuditLog::new();
    for i in 0..8 {
        log.append_leaf(format!("state_commit_{i}").as_bytes());
    }

    let proof = log.generate_proof(3).expect("proof generated");
    assert!(proof.verify(), "unmodified proof must pass");

    // 1. Tamper with leaf hash
    let mut bad_leaf_proof = proof.clone();
    bad_leaf_proof.leaf_hash[0] ^= 0xff;
    assert!(!bad_leaf_proof.verify(), "tampered leaf hash must fail");

    // 2. Tamper with sibling hash
    let mut bad_sibling_proof = proof.clone();
    bad_sibling_proof.steps[0].sibling_hash[0] ^= 0xaa;
    assert!(
        !bad_sibling_proof.verify(),
        "tampered sibling hash must fail"
    );

    // 3. Tamper with direction
    let mut bad_dir_proof = proof.clone();
    bad_dir_proof.steps[0].direction = match bad_dir_proof.steps[0].direction {
        NodeDirection::Left => NodeDirection::Right,
        NodeDirection::Right => NodeDirection::Left,
    };
    assert!(!bad_dir_proof.verify(), "inverted direction must fail");

    // 4. Tamper with expected root
    let mut bad_root_proof = proof.clone();
    bad_root_proof.root_hash[0] ^= 0x55;
    assert!(!bad_root_proof.verify(), "tampered root hash must fail");
}

#[test]
fn test_domain_separation_integrity() {
    let raw_payload = b"arbitrary serialized payload bytes";

    let leaf_h = MerkleDagAuditLog::compute_leaf_hash(raw_payload);
    let node_h = MerkleDagAuditLog::compute_parent_hash(&leaf_h, &leaf_h);

    assert_ne!(
        leaf_h, node_h,
        "leaf and node hashes must differ via domain separation"
    );
    assert_eq!(TAG_LEAF, 0x00);
    assert_eq!(TAG_NODE, 0x01);
    assert_eq!(TAG_DAG, 0x02);
}

#[test]
fn test_merkle_dag_causal_lineage() {
    let mut log = MerkleDagAuditLog::new();

    // Genesis state
    let n1 = log
        .add_dag_node(b"genesis_commit", &[], 1000)
        .expect("genesis added");
    assert!(n1.parent_ids.is_empty());

    // Second sequential commit
    let n2 = log
        .add_dag_node(b"edit_patch_1", std::slice::from_ref(&n1.id), 1010)
        .expect("n2 added");
    assert_eq!(n2.parent_ids, vec![n1.id.clone()]);

    // Branch commit off genesis
    let n3 = log
        .add_dag_node(b"feature_branch_patch", std::slice::from_ref(&n1.id), 1020)
        .expect("n3 added");

    // Merge commit merging n2 and n3
    let n4 = log
        .add_dag_node(b"merge_commit", &[n2.id.clone(), n3.id.clone()], 1030)
        .expect("n4 added");
    assert_eq!(n4.parent_ids.len(), 2);

    // Verify causal lineage
    assert!(log.verify_causal_lineage(&n4.id).expect("lineage valid"));
    assert!(log.verify_causal_lineage(&n2.id).expect("lineage valid"));
    assert!(log.verify_causal_lineage(&n1.id).expect("lineage valid"));

    // Topological order validation
    let topo = log.topological_order().expect("topological sort succeeded");
    assert_eq!(topo.len(), 4);
    assert_eq!(topo[0], n1.id);
    assert!(log.is_acyclic());
}

#[test]
fn test_dag_unknown_parent_rejection() {
    let mut log = MerkleDagAuditLog::new();
    let err = log.add_dag_node(b"child_payload", &["non_existent_hash".to_string()], 2000);
    assert!(matches!(err, Err(DagError::ParentNotFound(_))));
}
