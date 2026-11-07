#![forbid(unsafe_code)]

use pandora_runtime::speculative_decoding::{
    RadixKvCache, SpeculativeEngine, TokenProbability,
};

#[test]
fn test_perfect_draft_acceptance_and_bonus() {
    let mut engine = SpeculativeEngine::with_seed(1234);

    let draft_tokens = vec![10, 20, 30];
    let gamma = draft_tokens.len();

    // Draft and target share identical distribution
    let draft_dists = vec![
        vec![TokenProbability::new(10, 0.9), TokenProbability::new(99, 0.1)],
        vec![TokenProbability::new(20, 0.95), TokenProbability::new(88, 0.05)],
        vec![TokenProbability::new(30, 0.8), TokenProbability::new(77, 0.2)],
    ];
    let mut target_dists = draft_dists.clone();
    // Bonus token distribution at position gamma
    target_dists.push(vec![TokenProbability::new(40, 1.0)]);

    let result = engine.verify_draft(&draft_tokens, &draft_dists, &target_dists);

    assert_eq!(result.accepted_tokens, draft_tokens);
    assert_eq!(result.bonus_token, Some(40));
    assert_eq!(result.total_emitted, gamma + 1);
    assert!((result.acceptance_rate - 1.0).abs() < 1e-6);
}

#[test]
fn test_rejection_sampling_distribution_shift() {
    let mut engine = SpeculativeEngine::with_seed(5678);

    let draft_tokens = vec![100];
    // Draft model heavily favored token 100
    let draft_dists = vec![vec![
        TokenProbability::new(100, 0.99),
        TokenProbability::new(200, 0.01),
    ]];
    // Target model strongly prefers token 200 (rejection of token 100 is almost certain)
    let target_dists = vec![
        vec![TokenProbability::new(100, 0.001), TokenProbability::new(200, 0.999)],
        vec![TokenProbability::new(300, 1.0)],
    ];

    let result = engine.verify_draft(&draft_tokens, &draft_dists, &target_dists);

    // Draft token 100 must be rejected
    assert!(result.accepted_tokens.is_empty());
    // Bonus token must be sampled from the residual, which concentrates on token 200
    assert_eq!(result.bonus_token, Some(200));
    assert_eq!(result.total_emitted, 1);
}

#[test]
fn test_theoretical_speedup_calculations() {
    // Theorem 14 validation:
    // With alpha = 0.8, gamma = 5: E[beta] = (1 - 0.8^6) / (1 - 0.8) = (1 - 0.262144) / 0.2 = 3.68928
    let expected_beta = SpeculativeEngine::theoretical_expected_tokens(0.8, 5);
    assert!((expected_beta - 3.68928).abs() < 1e-5);

    // Speedup with latency ratio r = 0.1:
    // S = 3.68928 / (5 * 0.1 + 1) = 3.68928 / 1.5 = 2.45952
    let speedup = SpeculativeEngine::theoretical_speedup(0.8, 5, 0.1);
    assert!((speedup - 2.45952).abs() < 1e-5);

    // When alpha = 1.0 (perfect acceptance), E[beta] = gamma + 1 = 6.0
    let perfect_beta = SpeculativeEngine::theoretical_expected_tokens(1.0, 5);
    assert!((perfect_beta - 6.0).abs() < 1e-5);

    // Optimal draft length search
    let (opt_gamma, max_speedup) = SpeculativeEngine::optimal_draft_length(0.85, 0.08, 10);
    assert!((3..=8).contains(&opt_gamma));
    assert!(max_speedup > 2.5);
}

#[test]
fn test_radix_kv_cache_prefix_insertion_and_lookup() {
    let mut cache = RadixKvCache::new(2048);

    let prefix_system_prompt = vec![1000, 1001, 1002, 1003];
    let mut session1_tokens = prefix_system_prompt.clone();
    session1_tokens.extend_from_slice(&[2001, 2002]); // Session 1 prompt

    let mut session2_tokens = prefix_system_prompt.clone();
    session2_tokens.extend_from_slice(&[3001, 3002]); // Session 2 prompt

    // First session insertion: nothing shared yet
    let (shared_s1, new_s1) = cache.insert("session_1", &session1_tokens);
    assert_eq!(shared_s1, 0);
    assert_eq!(new_s1, 6);

    // Second session insertion: system prompt is 100% shared!
    let (shared_s2, new_s2) = cache.insert("session_2", &session2_tokens);
    assert_eq!(shared_s2, 4);
    assert_eq!(new_s2, 2);

    // Lookup shared prefix length for third session
    let mut session3_tokens = prefix_system_prompt.clone();
    session3_tokens.extend_from_slice(&[4001]);
    let lookup_len = cache.lookup_shared_prefix(&session3_tokens);
    assert_eq!(lookup_len, 4);

    assert_eq!(cache.active_sessions(), 2);
}

#[test]
fn test_radix_kv_cache_compression_ratio_matches_theory() {
    let mut cache = RadixKvCache::new(1024);

    // Common system instruction: 80 tokens (eta = 0.80)
    let common_prefix: Vec<u32> = (1..=80).collect();
    let num_sessions = 5;

    for s in 0..num_sessions {
        let mut tokens = common_prefix.clone();
        // 20 unique tokens per session
        tokens.extend((100 * (s + 1)..100 * (s + 1) + 20).map(|x| x as u32));
        cache.insert(&format!("sess_{s}"), &tokens);
    }

    // Dense tokens: 5 * 100 = 500 tokens
    // Compressed tree tokens: 80 (root edge) + 5 * 20 (branches) = 180 tokens
    // Empirical ratio: 500 / 180 = 2.777...
    let empirical_cr = cache.compression_ratio();
    assert!((empirical_cr - (500.0 / 180.0)).abs() < 1e-4);

    // Theoretical ratio: C_R(5, 0.80) = 1 / ((1 - 0.8) + 0.8 / 5) = 1 / (0.2 + 0.16) = 1 / 0.36 = 2.777...
    let theoretical_cr = RadixKvCache::theoretical_compression_ratio(5, 0.80);
    assert!((empirical_cr - theoretical_cr).abs() < 1e-4);

    // Asymptotic compression ratio as M -> inf: 1 / (1 - 0.80) = 5.0x
    let asymptotic_cr = RadixKvCache::asymptotic_compression_ratio(0.80);
    assert!((asymptotic_cr - 5.0).abs() < 1e-4);
}

#[test]
fn test_radix_kv_cache_release_and_cleanup() {
    let mut cache = RadixKvCache::new(512);

    let tokens = vec![1, 2, 3, 4, 5];
    cache.insert("temp_session", &tokens);
    assert_eq!(cache.active_sessions(), 1);

    let freed = cache.release("temp_session");
    assert_eq!(freed, 5);
    assert_eq!(cache.active_sessions(), 0);

    // Non-existent session release returns 0
    assert_eq!(cache.release("ghost_session"), 0);
}
