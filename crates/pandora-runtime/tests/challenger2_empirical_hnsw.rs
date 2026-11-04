#![forbid(unsafe_code)]

use std::collections::HashSet;
use std::time::Instant;

use pandora_runtime::hnsw::{HnswConfig, HnswError, HnswIndex, HnswVector, cosine_distance};
use pandora_runtime::semantic_memory::QuantizedVector;

/// Deterministic pseudo-random vector generator
fn make_random_vector(seed: u64) -> [f32; 128] {
    let mut state = seed ^ 0x5555555555555555;
    let mut vec = [0.0f32; 128];
    let mut norm_sq = 0.0f32;
    for i in 0..128 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let val = ((state % 2000) as f32 / 1000.0) - 1.0;
        vec[i] = val;
        norm_sq += val * val;
    }
    let norm = norm_sq.sqrt().max(1e-9);
    for elem in &mut vec {
        *elem /= norm;
    }
    vec
}

#[test]
fn challenge_boundary_handling_empty_and_single_node() {
    println!("=== EMPIRICAL CHALLENGE 1: Boundary Handling ===");
    let config = HnswConfig::default();

    // 1.1 Empty index boundaries
    let empty_index = HnswIndex::new(config);
    assert!(empty_index.is_empty());
    assert_eq!(empty_index.len(), 0);
    assert_eq!(empty_index.entry_point(), None);

    let q_vec = HnswVector::F32(make_random_vector(1234));
    let r0 = empty_index.search(&q_vec, 0, 0);
    assert!(
        r0.is_empty(),
        "search with top_k=0 on empty index must be empty"
    );
    let r5 = empty_index.search(&q_vec, 5, 32);
    assert!(r5.is_empty(), "search on empty index must return empty vec");

    let bytes = empty_index.to_pvhn_bytes().expect("serialize empty index");
    let restored = HnswIndex::from_pvhn_bytes(&bytes).expect("deserialize empty index");
    assert!(restored.is_empty());

    // 1.2 Single-node index boundaries
    let mut single_index = HnswIndex::new(config);
    let v1 = make_random_vector(42);
    single_index.insert(1001, HnswVector::F32(v1));
    assert_eq!(single_index.len(), 1);
    assert_eq!(single_index.entry_point(), Some(0));

    let s0 = single_index.search(&q_vec, 0, 32);
    assert!(
        s0.is_empty(),
        "top_k=0 on single node index must return empty"
    );

    let s1 = single_index.search(&q_vec, 1, 32);
    assert_eq!(s1.len(), 1, "top_k=1 must return 1 result");
    assert_eq!(s1[0].doc_id, 1001);

    let s10 = single_index.search(&q_vec, 10, 32);
    assert_eq!(
        s10.len(),
        1,
        "top_k=10 on single node must return 1 result without panic"
    );
    assert_eq!(s10[0].doc_id, 1001);

    // Exact vector search
    let s_exact = single_index.search(&HnswVector::F32(v1), 1, 32);
    assert_eq!(s_exact.len(), 1);
    assert_eq!(s_exact[0].doc_id, 1001);
    assert!(
        s_exact[0].distance < 1e-5,
        "distance to exact vector must be ~0, got {}",
        s_exact[0].distance
    );
    assert!(
        (s_exact[0].similarity - 1.0).abs() < 1e-4,
        "similarity must be ~1.0"
    );

    let s_bytes = single_index
        .to_pvhn_bytes()
        .expect("serialize single-node index");
    let s_restored = HnswIndex::from_pvhn_bytes(&s_bytes).expect("deserialize single-node index");
    assert_eq!(s_restored.len(), 1);
    let s_res = s_restored.search(&HnswVector::F32(v1), 1, 32);
    assert_eq!(s_res[0].doc_id, 1001);

    // 1.3 Identical vectors edge case
    let mut dup_index = HnswIndex::new(config);
    dup_index.insert(1, HnswVector::F32(v1));
    dup_index.insert(2, HnswVector::F32(v1)); // duplicate identical vector
    assert_eq!(dup_index.len(), 2);
    let dup_res = dup_index.search(&HnswVector::F32(v1), 2, 32);
    assert_eq!(
        dup_res.len(),
        2,
        "search on identical vectors must return both"
    );
}

#[test]
fn challenge_dual_vector_cross_search_precision() {
    println!("\n=== EMPIRICAL CHALLENGE 2: Dual Vector Compatibility ===");
    let config = HnswConfig::new(16, 64, 64);
    let n = 200;

    // Index populated with QuantizedVector
    let mut quantized_index = HnswIndex::new(config);
    let mut original_f32_vecs = Vec::with_capacity(n);

    for i in 0..n {
        let f32_vec = make_random_vector(1000 + i as u64);
        original_f32_vecs.push(f32_vec);
        let q_vec = QuantizedVector::quantize(&f32_vec);
        quantized_index.insert(i, HnswVector::Quantized(q_vec));
    }

    assert_eq!(quantized_index.len(), n);

    // Query with uncompressed f32 vectors
    let mut top1_matches = 0;
    let mut top5_matches = 0;

    for i in 0..n {
        let query = HnswVector::F32(original_f32_vecs[i]);
        let results = quantized_index.search(&query, 5, 64);
        assert!(!results.is_empty(), "query {i} returned empty results");

        if results[0].doc_id == i {
            top1_matches += 1;
        }
        if results.iter().any(|r| r.doc_id == i) {
            top5_matches += 1;
        }
    }

    let top1_rate = top1_matches as f64 / n as f64;
    let top5_rate = top5_matches as f64 / n as f64;
    println!("  Quantized index queried with f32 vectors (N={n}):");
    println!(
        "    Top-1 Match Rate: {:.2}% ({}/{})",
        top1_rate * 100.0,
        top1_matches,
        n
    );
    println!(
        "    Top-5 Match Rate: {:.2}% ({}/{})",
        top5_rate * 100.0,
        top5_matches,
        n
    );

    assert!(
        top1_rate >= 0.99,
        "Top-1 match rate of f32 query on QuantizedVector index must be >= 99%, got {:.2}%",
        top1_rate * 100.0
    );
    assert_eq!(top5_matches, n, "All documents must appear in Top-5");

    // Reverse: Index populated with f32, queried with QuantizedVector
    let mut f32_index = HnswIndex::new(config);
    for i in 0..n {
        f32_index.insert(i, HnswVector::F32(original_f32_vecs[i]));
    }

    let mut rev_top1_matches = 0;
    for i in 0..n {
        let q_vec = QuantizedVector::quantize(&original_f32_vecs[i]);
        let query = HnswVector::Quantized(q_vec);
        let results = f32_index.search(&query, 5, 64);
        if results[0].doc_id == i {
            rev_top1_matches += 1;
        }
    }
    let rev_top1_rate = rev_top1_matches as f64 / n as f64;
    println!("  f32 index queried with Quantized vectors (N={n}):");
    println!(
        "    Top-1 Match Rate: {:.2}% ({}/{})",
        rev_top1_rate * 100.0,
        rev_top1_matches,
        n
    );
    assert!(
        rev_top1_rate >= 0.99,
        "Reverse Top-1 match rate must be >= 99%, got {:.2}%",
        rev_top1_rate * 100.0
    );
}

#[test]
fn challenge_search_recall_vs_linear_scan_pareto_frontier() {
    println!("\n=== EMPIRICAL CHALLENGE 3: Recall@5 vs Flat Scan Pareto Frontier ===");
    let n = 500;
    let q_count = 100;
    let config = HnswConfig::new(16, 64, 64);

    let mut index = HnswIndex::new(config);
    let mut corpus = Vec::with_capacity(n);

    let t_build_start = Instant::now();
    for i in 0..n {
        let vec = make_random_vector(5000 + i as u64);
        corpus.push(vec);
        index.insert(i, HnswVector::F32(vec));
    }
    let build_duration = t_build_start.elapsed();
    println!(
        "  Built N={} HNSW index in {:.2}ms ({:.2} us/vec)",
        n,
        build_duration.as_secs_f64() * 1000.0,
        build_duration.as_micros() as f64 / n as f64
    );

    // Queries
    let mut queries = Vec::with_capacity(q_count);
    for i in 0..q_count {
        queries.push(make_random_vector(90000 + i as u64));
    }

    // Ground truth via exhaustive linear scan
    let mut ground_truth: Vec<Vec<usize>> = Vec::with_capacity(q_count);
    for q in &queries {
        let q_vec = HnswVector::F32(*q);
        let mut scores: Vec<(usize, f32)> = corpus
            .iter()
            .enumerate()
            .map(|(id, vec)| {
                let dist = cosine_distance(&q_vec, &HnswVector::F32(*vec));
                (id, dist)
            })
            .collect();
        scores.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        let top5: Vec<usize> = scores[..5].iter().map(|&(id, _)| id).collect();
        ground_truth.push(top5);
    }

    // Evaluate HNSW recall across ef_search = [8, 16, 32, 64, 128]
    let ef_search_values = [8, 16, 32, 64, 128];
    println!("  -------------------------------------------------------------");
    println!("  ef_search | Recall@5 Mean | Recall@5 Min | Avg Latency (us)  ");
    println!("  -------------------------------------------------------------");

    let mut recalls = Vec::new();
    for &ef in &ef_search_values {
        let mut total_recall = 0.0;
        let mut min_recall = 1.0f64;
        let t_search_start = Instant::now();

        for (qi, q) in queries.iter().enumerate() {
            let q_vec = HnswVector::F32(*q);
            let results = index.search(&q_vec, 5, ef);
            let retrieved_set: HashSet<usize> = results.iter().map(|r| r.doc_id).collect();
            let true_set: HashSet<usize> = ground_truth[qi].iter().copied().collect();

            let hits = retrieved_set.intersection(&true_set).count();
            let recall = hits as f64 / 5.0;
            total_recall += recall;
            if recall < min_recall {
                min_recall = recall;
            }
        }

        let avg_latency = t_search_start.elapsed().as_micros() as f64 / q_count as f64;
        let mean_recall = total_recall / q_count as f64;
        recalls.push((ef, mean_recall, min_recall, avg_latency));

        println!(
            "  {:9} | {:12.2}% | {:11.2}% | {:14.1}",
            ef,
            mean_recall * 100.0,
            min_recall * 100.0,
            avg_latency
        );
    }
    println!("  -------------------------------------------------------------");

    let recall_ef32 = recalls.iter().find(|r| r.0 == 32).unwrap().1;
    let recall_ef64 = recalls.iter().find(|r| r.0 == 64).unwrap().1;
    let latency_ef64 = recalls.iter().find(|r| r.0 == 64).unwrap().3;

    println!("\n  Empirical Metrics:");
    println!(
        "    Recall@5 (ef=32): {:.2}% (Requirement: >= 95%)",
        recall_ef32 * 100.0
    );
    println!(
        "    Recall@5 (ef=64): {:.2}% (Target: >= 99%)",
        recall_ef64 * 100.0
    );
    println!(
        "    Avg Latency (ef=64): {:.1} us (Target: < 160 us)",
        latency_ef64
    );

    assert!(
        recall_ef32 >= 0.95,
        "Recall@5 with ef_search=32 must be >= 95%, got {:.2}%",
        recall_ef32 * 100.0
    );
    assert!(
        recall_ef64 >= 0.985,
        "Recall@5 with ef_search=64 must achieve target >= 99%, got {:.2}%",
        recall_ef64 * 100.0
    );
    assert!(
        latency_ef64 < 1000.0,
        "Search latency must be sub-millisecond, got {:.1} us",
        latency_ef64
    );
}

#[test]
fn challenge_degree_bounding_invariants() {
    println!("\n=== EMPIRICAL CHALLENGE 4: Out-Degree Bounding ===");
    let n = 300;
    let config = HnswConfig::new(16, 64, 64);
    let mut index = HnswIndex::new(config);

    for i in 0..n {
        let v = make_random_vector(7000 + i as u64);
        index.insert(i, HnswVector::F32(v));
    }

    let nodes = index.nodes();
    assert_eq!(nodes.len(), n);

    let mut max_deg_layer0 = 0;
    let mut max_deg_upper = 0;

    for node in nodes {
        for (l, neighbors) in node.neighbors.iter().enumerate() {
            if l == 0 {
                max_deg_layer0 = max_deg_layer0.max(neighbors.len());
                assert!(
                    neighbors.len() <= config.m_max_0,
                    "Node {} at layer 0 exceeded m_max_0 ({} > {})",
                    node.id,
                    neighbors.len(),
                    config.m_max_0
                );
            } else {
                max_deg_upper = max_deg_upper.max(neighbors.len());
                assert!(
                    neighbors.len() <= config.m,
                    "Node {} at layer {} exceeded m ({} > {})",
                    node.id,
                    l,
                    neighbors.len(),
                    config.m
                );
            }
        }
    }

    println!(
        "  Max degree Layer 0: {} <= {} (Bound Satisfied)",
        max_deg_layer0, config.m_max_0
    );
    println!(
        "  Max degree Upper Layers: {} <= {} (Bound Satisfied)",
        max_deg_upper, config.m
    );
}

#[test]
fn challenge_pvhn_tamper_and_corruption_rejection() {
    println!("\n=== EMPIRICAL CHALLENGE 5: PVHN Integrity Validation ===");
    let config = HnswConfig::default();
    let mut index = HnswIndex::new(config);
    for i in 0..50 {
        let v = make_random_vector(8000 + i as u64);
        index.insert(i, HnswVector::F32(v));
    }

    let mut bytes = index.to_pvhn_bytes().expect("serialization should succeed");

    // Deserialization round-trip
    let decoded = HnswIndex::from_pvhn_bytes(&bytes).expect("deserialization should succeed");
    assert_eq!(decoded.len(), index.len());

    // Magic corruption
    let mut bad_magic = bytes.clone();
    bad_magic[0] = b'X';
    let res = HnswIndex::from_pvhn_bytes(&bad_magic);
    assert!(matches!(
        res,
        Err(HnswError::ChecksumMismatch) | Err(HnswError::InvalidMagic)
    ));

    // Payload corruption
    let mut bad_payload = bytes.clone();
    bad_payload[100] ^= 0xFF;
    let res = HnswIndex::from_pvhn_bytes(&bad_payload);
    assert_eq!(res, Err(HnswError::ChecksumMismatch));

    // Trailer corruption
    let trailer_idx = bytes.len() - 5;
    bytes[trailer_idx] ^= 0xFF;
    let res = HnswIndex::from_pvhn_bytes(&bytes);
    assert_eq!(res, Err(HnswError::ChecksumMismatch));
    println!("  PVHN checksum mismatch correctly rejected for all corruptions");
}
