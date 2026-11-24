#![forbid(unsafe_code)]

use std::collections::HashSet;
use std::hint::black_box;
use std::time::Instant;

use pandora_runtime::{
    DecisionFeatures, HnswConfig, HnswIndex, HnswVector, LatsMemoEntry, LatsMemoKey,
    LatsPrefixMemoCache, TropicalProjector, TropicalVector, cosine_distance,
};

fn deterministic_features(idx: usize) -> DecisionFeatures {
    let mut coords = [0.0f64; 7];
    for (dim, slot) in coords.iter_mut().enumerate() {
        let raw = ((idx.wrapping_mul(131) ^ dim.wrapping_mul(37)) % 1000) as f64 / 1000.0;
        *slot = raw;
    }
    DecisionFeatures::new(coords)
}

fn deterministic_tropical_vector(seed: usize) -> TropicalVector {
    const DOMAINS: [&str; 32] = [
        "tropical",
        "rust",
        "kubernetes",
        "ed25519",
        "postgres",
        "mcts",
        "sinkhorn",
        "martingale",
        "hnsw",
        "bm25",
        "wasm",
        "tokio",
        "merkle",
        "distill",
        "conformal",
        "compaction",
        "sandbox",
        "ledger",
        "canary",
        "shard",
        "latency",
        "cache",
        "memo",
        "telemetry",
        "capsule",
        "voucher",
        "beacon",
        "router",
        "parser",
        "planner",
        "tensor",
        "kernel",
    ];
    let a = DOMAINS[seed % DOMAINS.len()];
    let b = DOMAINS[(seed / 32 + seed * 5) % DOMAINS.len()];
    let c = format!("tok_{}", seed % 97);
    TropicalProjector::project(&format!("{a} {b} {c}"))
}

fn bench_lats_prefix_memo_cache() {
    println!("=== LatsPrefixMemoCache Throughput Benchmark ===");

    const QUANT_ITERS: usize = 100_000;
    let features: Vec<DecisionFeatures> = (0..512).map(deterministic_features).collect();
    let digest = "pnn-bench-digest-v315";

    // Warmup quantization
    for f in features.iter().take(64) {
        black_box(LatsMemoKey::from_features("read_file", f, digest));
    }

    let start_quant = Instant::now();
    for i in 0..QUANT_ITERS {
        let f = &features[i % features.len()];
        black_box(LatsMemoKey::from_features("read_file", f, digest));
    }
    let quant_elapsed = start_quant.elapsed();
    let quant_ops_sec = QUANT_ITERS as f64 / quant_elapsed.as_secs_f64();
    let quant_ns_op = quant_elapsed.as_nanos() as f64 / QUANT_ITERS as f64;
    println!(
        "  key_quantization:   {:>8} ops in {:>7.2} ms | {:>11.0} ops/sec ({:.0} ops/s) | {:>6.1} ns/op",
        QUANT_ITERS,
        quant_elapsed.as_secs_f64() * 1000.0,
        quant_ops_sec,
        quant_ops_sec,
        quant_ns_op
    );

    // Populate 256-entry cache with distinct keys
    let mut cache = LatsPrefixMemoCache::new(256);
    let mut hit_keys = Vec::with_capacity(256);
    for i in 0..256 {
        let mut f_coords = [0.0f64; 7];
        f_coords[0] = (i % 16) as f64 / 15.0;
        f_coords[1] = ((i / 16) % 16) as f64 / 15.0;
        let key = LatsMemoKey::from_features("edit_file", &DecisionFeatures::new(f_coords), digest);
        cache.insert(
            key.clone(),
            LatsMemoEntry {
                best_action: Some(i % 4),
                best_value: 0.75 + (i as f64) * 0.0005,
                nodes_created: 12,
                max_depth_reached: 3,
            },
        );
        hit_keys.push(key);
    }

    // Cache hit benchmark
    const LOOKUP_ITERS: usize = 200_000;
    for k in hit_keys.iter().take(32) {
        black_box(cache.get(k));
    }
    let hits_before = cache.hits();
    let start_hits = Instant::now();
    for i in 0..LOOKUP_ITERS {
        black_box(cache.get(&hit_keys[i & 255]));
    }
    let hits_elapsed = start_hits.elapsed();
    let hit_delta = cache.hits() - hits_before;
    println!(
        "  cache_hits:         {:>8} ops in {:>7.2} ms | {:>11.0} ops/s | {:>6.1} ns/op | hit_rate={:.2}%",
        LOOKUP_ITERS,
        hits_elapsed.as_secs_f64() * 1000.0,
        LOOKUP_ITERS as f64 / hits_elapsed.as_secs_f64(),
        hits_elapsed.as_nanos() as f64 / LOOKUP_ITERS as f64,
        (hit_delta as f64 / LOOKUP_ITERS as f64) * 100.0
    );

    // Cache miss benchmark
    let miss_keys: Vec<LatsMemoKey> = (0..256)
        .map(|i| {
            LatsMemoKey::from_features(
                "run_command_miss",
                &deterministic_features(i + 10_000),
                digest,
            )
        })
        .collect();
    let misses_before = cache.misses();
    let start_misses = Instant::now();
    for i in 0..LOOKUP_ITERS {
        black_box(cache.get(&miss_keys[i & 255]));
    }
    let misses_elapsed = start_misses.elapsed();
    let miss_delta = cache.misses() - misses_before;
    println!(
        "  cache_misses:       {:>8} ops in {:>7.2} ms | {:>11.0} ops/s | {:>6.1} ns/op | misses={}",
        LOOKUP_ITERS,
        misses_elapsed.as_secs_f64() * 1000.0,
        LOOKUP_ITERS as f64 / misses_elapsed.as_secs_f64(),
        misses_elapsed.as_nanos() as f64 / LOOKUP_ITERS as f64,
        miss_delta
    );

    // Bounded FIFO eviction throughput benchmark
    const EVICT_ITERS: usize = 100_000;
    let mut evict_cache = LatsPrefixMemoCache::new(256);
    let evict_keys: Vec<LatsMemoKey> = (0..2048)
        .map(|i| LatsMemoKey {
            tool_name: format!("tool_{}", i % 16),
            quantized_features: [
                (i & 0xF) as u8,
                ((i >> 4) & 0xF) as u8,
                ((i >> 8) & 0xF) as u8,
                ((i >> 1) & 0xF) as u8,
                ((i >> 2) & 0xF) as u8,
                ((i >> 3) & 0xF) as u8,
                ((i >> 5) & 0xF) as u8,
            ],
            pnn_digest: digest.to_string(),
        })
        .collect();
    let start_evict = Instant::now();
    for i in 0..EVICT_ITERS {
        let key = evict_keys[i % evict_keys.len()].clone();
        evict_cache.insert(
            key,
            LatsMemoEntry {
                best_action: Some(i % 5),
                best_value: 0.5,
                nodes_created: 8,
                max_depth_reached: 2,
            },
        );
    }
    let evict_elapsed = start_evict.elapsed();
    assert_eq!(evict_cache.len(), 256);
    println!(
        "  bounded_fifo_evict: {:>8} ops in {:>7.2} ms | {:>11.0} ops/s | {:>6.1} ns/op | cap_len={}",
        EVICT_ITERS,
        evict_elapsed.as_secs_f64() * 1000.0,
        EVICT_ITERS as f64 / evict_elapsed.as_secs_f64(),
        evict_elapsed.as_nanos() as f64 / EVICT_ITERS as f64,
        evict_cache.len()
    );
}

fn bench_tropical_hnsw() {
    println!("\n=== Tropical HNSW Throughput & Compaction Benchmark ===");

    const CORPUS_SIZE: usize = 500;
    const DELETE_COUNT: usize = 120;
    const UPSERT_COUNT: usize = 80;
    const QUERY_COUNT: usize = 200;
    const TOP_K: usize = 10;
    const EF_SEARCH: usize = 64;

    let cfg = HnswConfig::default();
    let mut index = HnswIndex::new(cfg).with_seed(0x5151_3150);
    let mut live_vectors: Vec<(usize, HnswVector)> = Vec::with_capacity(CORPUS_SIZE);

    for doc_id in 0..CORPUS_SIZE {
        let tv = HnswVector::Tropical(deterministic_tropical_vector(doc_id));
        live_vectors.push((doc_id, tv));
    }

    // 1. Bulk build
    let start_build = Instant::now();
    for (doc_id, vec) in &live_vectors {
        index.insert(*doc_id, vec.clone());
    }
    let build_elapsed = start_build.elapsed();
    println!(
        "  tropical_bulk_build:   {:>5} docs in {:>7.2} ms | {:>9.0} inserts/s | {:>7.1} us/op",
        CORPUS_SIZE,
        build_elapsed.as_secs_f64() * 1000.0,
        CORPUS_SIZE as f64 / build_elapsed.as_secs_f64(),
        build_elapsed.as_micros() as f64 / CORPUS_SIZE as f64
    );

    // 2. Incremental deletes + upserts
    let start_mut = Instant::now();
    for doc_id in 0..DELETE_COUNT {
        black_box(index.mark_deleted(doc_id));
    }
    for offset in 0..UPSERT_COUNT {
        let doc_id = DELETE_COUNT + offset;
        let updated_vec = HnswVector::Tropical(deterministic_tropical_vector(10_000 + offset));
        black_box(index.upsert(doc_id, updated_vec.clone()));
        live_vectors[doc_id] = (doc_id, updated_vec);
    }
    let mut_elapsed = start_mut.elapsed();
    let total_mut_ops = DELETE_COUNT + UPSERT_COUNT;
    println!(
        "  incremental_del_ups:   {:>5} ops  in {:>7.2} ms | {:>9.0} ops/s     | {:>7.1} us/op | tombstones={}",
        total_mut_ops,
        mut_elapsed.as_secs_f64() * 1000.0,
        total_mut_ops as f64 / mut_elapsed.as_secs_f64(),
        mut_elapsed.as_micros() as f64 / total_mut_ops as f64,
        index.tombstone_count()
    );

    // 3. Compaction
    let start_compact = Instant::now();
    let report = index.compact();
    let compact_elapsed = start_compact.elapsed();
    println!(
        "  compact_with_repair:   {:>5} -> {:>3} nodes in {:>6.2} ms | removed={} | repaired={} | reconnected={}",
        report.nodes_before,
        report.nodes_after,
        compact_elapsed.as_secs_f64() * 1000.0,
        report.tombstones_removed,
        report.edges_repaired,
        report.nodes_reconnected
    );

    // 4. Post-compaction search throughput & recall@10 vs exact tropical scan
    let remaining_live: Vec<(usize, HnswVector)> = live_vectors
        .into_iter()
        .filter(|(doc_id, _)| *doc_id >= DELETE_COUNT)
        .collect();
    let queries: Vec<HnswVector> = (0..QUERY_COUNT)
        .map(|q| HnswVector::Tropical(deterministic_tropical_vector(50_000 + q * 7)))
        .collect();

    for q in queries.iter().take(16) {
        black_box(index.search(q, TOP_K, EF_SEARCH));
    }

    let mut all_hnsw_results = Vec::with_capacity(QUERY_COUNT);
    let start_search = Instant::now();
    for q in &queries {
        let hnsw_res = index.search(q, TOP_K, EF_SEARCH);
        all_hnsw_results.push(black_box(hnsw_res));
    }
    let search_elapsed = start_search.elapsed();

    let mut total_recall = 0.0f64;
    for (q, hnsw_res) in queries.iter().zip(all_hnsw_results.iter()) {
        let hnsw_ids: HashSet<usize> = hnsw_res.iter().map(|r| r.doc_id).collect();
        let mut exact: Vec<(usize, f32)> = remaining_live
            .iter()
            .map(|(doc_id, vec)| (*doc_id, cosine_distance(q, vec)))
            .collect();
        exact.sort_by(|a, b| a.1.total_cmp(&b.1));
        let truth_ids: HashSet<usize> = exact.iter().take(TOP_K).map(|(id, _)| *id).collect();
        let hits = hnsw_ids.intersection(&truth_ids).count();
        total_recall += hits as f64 / TOP_K as f64;
    }
    let avg_recall = total_recall / QUERY_COUNT as f64;
    println!(
        "  post_compact_search:   {:>5} qrys in {:>7.2} ms | {:>9.0} qps       | {:>7.1} us/qry | recall@{}={:.3}",
        QUERY_COUNT,
        search_elapsed.as_secs_f64() * 1000.0,
        QUERY_COUNT as f64 / search_elapsed.as_secs_f64(),
        search_elapsed.as_micros() as f64 / QUERY_COUNT as f64,
        TOP_K,
        avg_recall
    );
}

fn main() {
    println!("[r3_throughput_bench] Starting R3 throughput benchmarks (LATS memo cache + HNSW)...");
    bench_lats_prefix_memo_cache();
    bench_tropical_hnsw();
}
