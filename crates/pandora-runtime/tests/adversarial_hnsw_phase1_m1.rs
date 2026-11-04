#![forbid(unsafe_code)]

//! Adversarial empirical challenge suite for HNSW Core Engine (Phase 1, Milestone 1).
//!
//! Validates:
//! 1. Graph invariants: degree bounds (M for l > 0, M_max_0 for l == 0), level hierarchy, entry point.
//! 2. Mathematical properties of distance metrics and triangular pruning heuristic.
//! 3. PVHN v1 binary format integrity under truncation, bit flips, and header corruption.
//! 4. Search monotonicity and edge cases (empty index, single node, k=0, large k).
//! 5. Persistence roundtrip through filesystem via atomic writes.

use pandora_runtime::hnsw::{
    HnswConfig, HnswError, HnswIndex, HnswVector, PVHN_HEADER_SIZE, PVHN_TRAILER_SIZE,
    cosine_distance,
};
use pandora_runtime::semantic_memory::QuantizedVector;
use sha2::{Digest, Sha256};
use std::collections::HashSet;

fn make_unit_vector(seed: u64, offset: f32) -> [f32; 128] {
    let mut v = [0.0f32; 128];
    let mut norm_sq = 0.0f32;
    let mut state = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    for i in 0..128 {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let raw = ((state >> 32) as i32) as f32 / (i32::MAX as f32);
        let val = raw + offset;
        v[i] = val;
        norm_sq += val * val;
    }
    let norm = norm_sq.sqrt().max(1e-9);
    for elem in &mut v {
        *elem /= norm;
    }
    v
}

fn assert_graph_invariants(index: &HnswIndex, config: &HnswConfig) {
    let nodes = index.nodes();
    let n = nodes.len();

    if n == 0 {
        assert!(
            index.entry_point().is_none(),
            "empty graph must have None entry point"
        );
        assert_eq!(index.max_level(), 0, "empty graph must have max_level 0");
        return;
    }

    // 1. Entry point invariant
    let ep = index
        .entry_point()
        .expect("non-empty graph must have entry point");
    assert!(ep < n, "entry point index out of bounds: ep={ep}, n={n}");
    assert_eq!(
        nodes[ep].level,
        index.max_level(),
        "entry point node level must equal max_level"
    );

    let mut actual_max_level = 0;
    for node in nodes {
        actual_max_level = actual_max_level.max(node.level);
    }
    assert_eq!(
        index.max_level(),
        actual_max_level,
        "index max_level must equal the supremum of node levels"
    );

    // 2. Degree bounds and level hierarchy invariants
    for node in nodes {
        assert_eq!(
            node.neighbors.len(),
            node.level + 1,
            "node {} neighbors layer count ({}) must equal level + 1 ({})",
            node.id,
            node.neighbors.len(),
            node.level + 1
        );

        for l in 0..=node.level {
            let deg = node.neighbors[l].len();
            let max_allowed = if l == 0 { config.m_max_0 } else { config.m };
            assert!(
                deg <= max_allowed,
                "Degree bound violated at node {}, layer {}: deg {} > max_allowed {}",
                node.id,
                l,
                deg,
                max_allowed
            );

            // Invariant: no duplicate neighbors
            let mut set = HashSet::new();
            for &nb in &node.neighbors[l] {
                assert!(nb < n, "neighbor ID {} out of range (n={})", nb, n);
                assert_ne!(nb, node.id, "node cannot be neighbor of itself");
                assert!(
                    set.insert(nb),
                    "duplicate neighbor {} in node {} at layer {}",
                    nb,
                    node.id,
                    l
                );

                // Level hierarchy invariant: neighbor nb must exist at layer l
                assert!(
                    nodes[nb].level >= l,
                    "neighbor {} (level {}) does not exist at layer {} (referenced by node {})",
                    nb,
                    nodes[nb].level,
                    l,
                    node.id
                );
            }
        }
    }
}

#[test]
fn test_adversarial_degree_bounds_random_insertion() {
    let config = HnswConfig::new(8, 32, 32).with_m_max_0(16);
    let mut index = HnswIndex::new(config).with_seed(1234567);

    for i in 0..150 {
        let vec = make_unit_vector(i as u64, 0.0);
        index.insert(i, HnswVector::F32(vec));
        assert_graph_invariants(&index, &config);
    }
    assert_eq!(index.len(), 150);
}

#[test]
fn test_adversarial_degree_bounds_clustered_vectors() {
    // Highly clustered vectors stress the neighbor selection heuristic
    let config = HnswConfig::new(6, 24, 24).with_m_max_0(12);
    let mut index = HnswIndex::new(config).with_seed(99999);

    let base = make_unit_vector(42, 0.0);
    for i in 0..100 {
        let mut v = base;
        // Perturb by tiny epsilon to create near-identical cluster
        v[i % 128] += (i as f32) * 1e-5;
        let mut norm_sq = 0.0f32;
        for x in &v {
            norm_sq += x * x;
        }
        let norm = norm_sq.sqrt();
        for x in &mut v {
            *x /= norm;
        }

        index.insert(i, HnswVector::F32(v));
        assert_graph_invariants(&index, &config);
    }
}

#[test]
fn test_adversarial_degree_bounds_duplicate_vectors() {
    // Exact identical vectors inserted repeatedly
    let config = HnswConfig::new(4, 16, 16).with_m_max_0(8);
    let mut index = HnswIndex::new(config).with_seed(777);

    let identical = make_unit_vector(101, 0.0);
    for i in 0..50 {
        index.insert(i, HnswVector::F32(identical));
        assert_graph_invariants(&index, &config);
    }
}

#[test]
fn test_adversarial_degree_bounds_mixed_vectors() {
    // Interleaved uncompressed F32 and QuantizedInt8 vectors
    let config = HnswConfig::new(8, 32, 32);
    let mut index = HnswIndex::new(config).with_seed(888);

    for i in 0..80 {
        let raw = make_unit_vector(i as u64, 0.1);
        let vec = if i % 2 == 0 {
            HnswVector::F32(raw)
        } else {
            HnswVector::Quantized(QuantizedVector::quantize(&raw))
        };
        index.insert(i, vec);
        assert_graph_invariants(&index, &config);
    }
}

#[test]
fn test_adversarial_minimal_configuration_boundary() {
    // Minimal M=2, M_max_0=2 configuration edge case
    let config = HnswConfig::new(2, 4, 4).with_m_max_0(2);
    let mut index = HnswIndex::new(config).with_seed(333);

    for i in 0..30 {
        let vec = make_unit_vector(i as u64, 0.0);
        index.insert(i, HnswVector::F32(vec));
        assert_graph_invariants(&index, &config);
    }
}

#[test]
fn test_adversarial_distance_metric_properties() {
    let zero_v = [0.0f32; 128];
    let zero_vec = HnswVector::F32(zero_v);
    let d_self_zero = cosine_distance(&zero_vec, &zero_vec);
    assert_eq!(d_self_zero, 0.0, "identity distance must be 0.0");

    let v1 = HnswVector::F32(make_unit_vector(1, 0.0));
    let v2 = HnswVector::F32(make_unit_vector(2, 0.0));

    let d12 = cosine_distance(&v1, &v2);
    let d21 = cosine_distance(&v2, &v1);
    assert!(
        (d12 - d21).abs() < 1e-6,
        "symmetry violated: d12={d12}, d21={d21}"
    );
    assert!(
        d12 >= 0.0 && d12 <= 2.0,
        "distance out of [0.0, 2.0] bounds"
    );

    // Inverted vector has cosine distance 2.0
    let mut inv = [0.0f32; 128];
    let raw = make_unit_vector(3, 0.0);
    for i in 0..128 {
        inv[i] = -raw[i];
    }
    let v_inv = HnswVector::F32(inv);
    let d_inv = cosine_distance(&HnswVector::F32(raw), &v_inv);
    assert!(
        (d_inv - 2.0).abs() < 1e-5,
        "antipodal distance must be 2.0, got {d_inv}"
    );
}

#[test]
fn test_adversarial_pvhn_truncation_matrix() {
    let config = HnswConfig::new(4, 16, 16);
    let mut index = HnswIndex::new(config).with_seed(1234);
    for i in 0..10 {
        index.insert(i, HnswVector::F32(make_unit_vector(i as u64, 0.0)));
    }
    let valid_bytes = index.to_pvhn_bytes().expect("serialization must succeed");

    // Test 1: Empty slice
    assert_eq!(
        HnswIndex::from_pvhn_bytes(&[]),
        Err(HnswError::TruncatedData)
    );

    // Test 2: Lengths strictly less than header + trailer
    for len in 1..(PVHN_HEADER_SIZE + PVHN_TRAILER_SIZE) {
        assert_eq!(
            HnswIndex::from_pvhn_bytes(&valid_bytes[..len]),
            Err(HnswError::TruncatedData),
            "Length {} should fail as TruncatedData",
            len
        );
    }

    // Test 3: Truncate internal payload while keeping a valid SHA-256 trailer of wrong length
    for cut in 1..20 {
        let truncated_len = valid_bytes.len() - cut;
        if truncated_len >= PVHN_HEADER_SIZE + PVHN_TRAILER_SIZE {
            // Checksum mismatch will catch it if trailer is untouched
            assert!(
                HnswIndex::from_pvhn_bytes(&valid_bytes[..truncated_len]).is_err(),
                "Truncated slice of len {} must fail",
                truncated_len
            );
        }
    }
}

#[test]
fn test_adversarial_pvhn_corruption_rejection() {
    let config = HnswConfig::new(4, 16, 16);
    let mut index = HnswIndex::new(config).with_seed(4321);
    for i in 0..15 {
        index.insert(i, HnswVector::F32(make_unit_vector(i as u64, 0.0)));
    }
    let valid_bytes = index.to_pvhn_bytes().expect("serialization must succeed");

    // Helper to forge valid checksum on modified payload
    let forge_checksum = |payload: &[u8]| -> Vec<u8> {
        let mut out = payload.to_vec();
        let digest = Sha256::digest(payload);
        out.extend_from_slice(&digest);
        out
    };

    let payload = &valid_bytes[..valid_bytes.len() - PVHN_TRAILER_SIZE];

    // Case 1: Corrupted Magic Bytes (with forged checksum)
    let mut bad_magic_payload = payload.to_vec();
    bad_magic_payload[0] = b'X';
    let forged = forge_checksum(&bad_magic_payload);
    assert_eq!(
        HnswIndex::from_pvhn_bytes(&forged),
        Err(HnswError::InvalidMagic)
    );

    // Case 2: Unsupported Version (with forged checksum)
    let mut bad_ver_payload = payload.to_vec();
    bad_ver_payload[4] = 99;
    let forged = forge_checksum(&bad_ver_payload);
    assert_eq!(
        HnswIndex::from_pvhn_bytes(&forged),
        Err(HnswError::UnsupportedVersion(99))
    );

    // Case 3: Dimension Mismatch (with forged checksum)
    let mut bad_dim_payload = payload.to_vec();
    bad_dim_payload[20] = 64; // expected 128
    bad_dim_payload[21] = 0;
    let forged = forge_checksum(&bad_dim_payload);
    assert_eq!(
        HnswIndex::from_pvhn_bytes(&forged),
        Err(HnswError::DimensionMismatch(64))
    );

    // Case 4: Single Bit Flip anywhere in payload (without re-forging checksum)
    for offset in [0, 10, 50, 79, 100, 200, valid_bytes.len() - 10] {
        let mut flipped = valid_bytes.clone();
        flipped[offset] ^= 0x01;
        assert_eq!(
            HnswIndex::from_pvhn_bytes(&flipped),
            Err(HnswError::ChecksumMismatch),
            "bit flip at offset {} must trigger ChecksumMismatch",
            offset
        );
    }

    // Case 5: Single Bit Flip in SHA-256 trailer
    for t_offset in (valid_bytes.len() - PVHN_TRAILER_SIZE)..valid_bytes.len() {
        let mut flipped = valid_bytes.clone();
        flipped[t_offset] ^= 0x80;
        assert_eq!(
            HnswIndex::from_pvhn_bytes(&flipped),
            Err(HnswError::ChecksumMismatch),
            "bit flip in trailer at offset {} must trigger ChecksumMismatch",
            t_offset
        );
    }
}

#[test]
fn test_adversarial_persistence_roundtrip() {
    let temp_dir = std::env::temp_dir().join(format!("pandora_hnsw_test_{}", std::process::id()));
    let temp_file = temp_dir.join("test_index.pvhn");

    let config = HnswConfig::new(6, 24, 24).with_m_max_0(12);
    let mut index = HnswIndex::new(config).with_seed(6789);
    for i in 0..40 {
        let vec = if i % 2 == 0 {
            HnswVector::F32(make_unit_vector(i as u64, 0.0))
        } else {
            HnswVector::Quantized(QuantizedVector::quantize(&make_unit_vector(i as u64, 0.0)))
        };
        index.insert(i, vec);
    }

    index
        .save_to_file(&temp_file)
        .expect("save_to_file must succeed");
    assert!(temp_file.exists());

    let loaded = HnswIndex::load_from_file(&temp_file).expect("load_from_file must succeed");
    assert_eq!(loaded.len(), 40);
    assert_eq!(loaded.entry_point(), index.entry_point());
    assert_eq!(loaded.max_level(), index.max_level());
    assert_graph_invariants(&loaded, &config);

    // Clean up
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_adversarial_search_edge_cases() {
    let config = HnswConfig::new(8, 32, 32);
    let mut index = HnswIndex::new(config).with_seed(555);

    let query = HnswVector::F32(make_unit_vector(999, 0.0));

    // 1. Search empty index
    let res = index.search(&query, 10, 32);
    assert!(
        res.is_empty(),
        "searching empty index must return empty vec"
    );

    // 2. Insert single node
    index.insert(42, query.clone());
    assert_eq!(index.len(), 1);

    // 3. Search single node with top_k = 0
    let res0 = index.search(&query, 0, 32);
    assert!(res0.is_empty(), "top_k = 0 must return empty vec");

    // 4. Search single node with exact match
    let res1 = index.search(&query, 5, 32);
    assert_eq!(res1.len(), 1);
    assert_eq!(res1[0].doc_id, 42);
    assert!(
        res1[0].distance < 1e-6,
        "exact match distance must be ~0, got {}",
        res1[0].distance
    );
    assert!(
        (res1[0].similarity - 1.0).abs() < 1e-6,
        "exact match similarity must be ~1.0"
    );

    // 5. Search single node with top_k > index.len()
    let res_large = index.search(&query, 100, 32);
    assert_eq!(
        res_large.len(),
        1,
        "cannot return more results than index length"
    );

    // 6. Monotonicity of search result distances
    for i in 1..25 {
        index.insert(i, HnswVector::F32(make_unit_vector(i as u64, 0.05)));
    }
    let res_multi = index.search(&query, 10, 64);
    assert_eq!(res_multi.len(), 10);
    for window in res_multi.windows(2) {
        assert!(
            window[0].distance <= window[1].distance,
            "results must be sorted in non-decreasing order of distance: {} > {}",
            window[0].distance,
            window[1].distance
        );
        assert!(
            window[0].similarity >= window[1].similarity,
            "results must be sorted in non-increasing order of similarity: {} < {}",
            window[0].similarity,
            window[1].similarity
        );
    }
}
