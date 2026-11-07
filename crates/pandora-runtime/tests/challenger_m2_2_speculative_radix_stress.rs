#![forbid(unsafe_code)]

use pandora_runtime::speculative_decoding::{
    RadixKvCache, RadixKvNode, SpeculativeEngine, TokenProbability,
};

/// 1. Empirically verify SpeculativeEngine draft verification with extreme probability vectors:
///    uniform, point mass, disjoint distributions, draft token length 0, 1, 50.
#[test]
fn challenge_speculative_extreme_probability_vectors() {
    let mut engine = SpeculativeEngine::with_seed(0xABCD_EF01_2345_6789);

    // --- Extreme Case A: Uniform Distributions ---
    // Draft and target both have identical uniform distribution over 50 tokens
    let vocab_size = 50;
    let uniform_prob = 1.0 / vocab_size as f64;
    let uniform_dist: Vec<TokenProbability> = (0..vocab_size)
        .map(|tok| TokenProbability::new(tok, uniform_prob))
        .collect();

    let draft_tokens_uniform = vec![5, 15, 25, 35, 45];
    let draft_dists_uniform = vec![uniform_dist.clone(); 5];
    let mut target_dists_uniform = vec![uniform_dist.clone(); 5];
    target_dists_uniform.push(vec![TokenProbability::new(99, 1.0)]); // Bonus step

    let res_uniform = engine.verify_draft(
        &draft_tokens_uniform,
        &draft_dists_uniform,
        &target_dists_uniform,
    );
    assert_eq!(res_uniform.accepted_tokens, draft_tokens_uniform);
    assert_eq!(res_uniform.bonus_token, Some(99));
    assert_eq!(res_uniform.total_emitted, 6);
    assert!((res_uniform.acceptance_rate - 1.0).abs() < 1e-9);

    // --- Extreme Case B: Point Mass (Dirac Delta) Distributions ---
    // Match case: Draft and target concentrated 100% on identical token
    let draft_point = vec![TokenProbability::new(777, 1.0)];
    let target_point = vec![TokenProbability::new(777, 1.0)];
    let res_point_match = engine.verify_draft(
        &[777],
        std::slice::from_ref(&draft_point),
        &[target_point, vec![TokenProbability::new(888, 1.0)]],
    );
    assert_eq!(res_point_match.accepted_tokens, vec![777]);
    assert_eq!(res_point_match.bonus_token, Some(888));
    assert_eq!(res_point_match.total_emitted, 2);

    // Mismatch case: Draft concentrated on 777, target concentrated on 888
    let target_mismatch = vec![TokenProbability::new(888, 1.0)];
    let res_point_mismatch = engine.verify_draft(
        &[777],
        &[draft_point],
        &[target_mismatch, vec![TokenProbability::new(999, 1.0)]],
    );
    assert!(res_point_mismatch.accepted_tokens.is_empty());
    assert_eq!(res_point_mismatch.bonus_token, Some(888)); // Residual must be token 888
    assert_eq!(res_point_mismatch.total_emitted, 1);
    assert!((res_point_mismatch.acceptance_rate - 0.0).abs() < 1e-9);

    // --- Extreme Case C: Disjoint Distributions (Support Intersection is Empty) ---
    // Draft support: {10, 20, 30}. Target support: {40, 50, 60}.
    let draft_disjoint = vec![
        TokenProbability::new(10, 0.5),
        TokenProbability::new(20, 0.3),
        TokenProbability::new(30, 0.2),
    ];
    let target_disjoint = vec![
        TokenProbability::new(40, 0.4),
        TokenProbability::new(50, 0.4),
        TokenProbability::new(60, 0.2),
    ];
    for tok in &[10, 20, 30] {
        let res_disjoint = engine.verify_draft(
            &[*tok],
            std::slice::from_ref(&draft_disjoint),
            std::slice::from_ref(&target_disjoint),
        );
        assert!(res_disjoint.accepted_tokens.is_empty());
        assert!(res_disjoint.bonus_token.is_some());
        let bonus = res_disjoint.bonus_token.unwrap();
        assert!([40, 50, 60].contains(&bonus), "Bonus must strictly come from target support");
        assert!(![10, 20, 30].contains(&bonus), "Bonus must never come from disjoint draft support");
    }

    // --- Extreme Case D: Draft Token Length 0 ---
    let res_len_0 = engine.verify_draft(
        &[],
        &[],
        &[vec![TokenProbability::new(42, 1.0)]],
    );
    assert!(res_len_0.accepted_tokens.is_empty());
    assert_eq!(res_len_0.bonus_token, Some(42));
    assert_eq!(res_len_0.total_emitted, 1);
    assert_eq!(res_len_0.acceptance_rate, 1.0);

    // --- Extreme Case E: Draft Token Length 1 ---
    let res_len_1 = engine.verify_draft(
        &[100],
        &[vec![TokenProbability::new(100, 1.0)]],
        &[vec![TokenProbability::new(100, 1.0)], vec![TokenProbability::new(101, 1.0)]],
    );
    assert_eq!(res_len_1.accepted_tokens, vec![100]);
    assert_eq!(res_len_1.bonus_token, Some(101));
    assert_eq!(res_len_1.total_emitted, 2);

    // --- Extreme Case F: Draft Token Length 50 ---
    let gamma_50 = 50;
    let draft_tokens_50: Vec<u32> = (1..=gamma_50 as u32).collect();
    let draft_dists_50: Vec<Vec<TokenProbability>> = draft_tokens_50
        .iter()
        .map(|&tok| vec![TokenProbability::new(tok, 1.0)])
        .collect();

    // F.1: Perfect match across all 50 tokens
    let mut target_dists_50_perfect = draft_dists_50.clone();
    target_dists_50_perfect.push(vec![TokenProbability::new(51, 1.0)]);
    let res_50_perfect = engine.verify_draft(
        &draft_tokens_50,
        &draft_dists_50,
        &target_dists_50_perfect,
    );
    assert_eq!(res_50_perfect.accepted_tokens.len(), 50);
    assert_eq!(res_50_perfect.bonus_token, Some(51));
    assert_eq!(res_50_perfect.total_emitted, 51);
    assert!((res_50_perfect.acceptance_rate - 1.0).abs() < 1e-9);

    // F.2: Divergence at token 25 (first 25 match, 26th diverges)
    let mut target_dists_50_diverge = draft_dists_50.clone();
    target_dists_50_diverge[25] = vec![TokenProbability::new(9999, 1.0)]; // Divergence at index 25 (token 26)
    let res_50_diverge = engine.verify_draft(
        &draft_tokens_50,
        &draft_dists_50,
        &target_dists_50_diverge,
    );
    assert_eq!(res_50_diverge.accepted_tokens.len(), 25);
    assert_eq!(res_50_diverge.accepted_tokens, draft_tokens_50[..25].to_vec());
    assert_eq!(res_50_diverge.bonus_token, Some(9999));
    assert_eq!(res_50_diverge.total_emitted, 26);
    assert!((res_50_diverge.acceptance_rate - 0.50).abs() < 1e-9);
}

/// 2. Challenge mathematical speedup formula:
///    S(alpha, gamma, r) = E[beta] / (gamma * r + 1) across boundary conditions:
///    alpha = 0, 1; r -> 0, r = 1; gamma = 0, 100.
#[test]
fn challenge_mathematical_speedup_boundary_conditions() {
    // --- Boundary Case 1: alpha = 0.0 (Zero Draft Acceptance) ---
    // E[beta] = (1 - 0^(gamma+1)) / (1 - 0) = 1.0 regardless of gamma.
    for g in [0, 1, 5, 10, 50, 100] {
        let e_beta = SpeculativeEngine::theoretical_expected_tokens(0.0, g);
        assert!((e_beta - 1.0).abs() < 1e-9, "E[beta] must be 1.0 when alpha = 0");

        // Speedup must be 1 / (gamma * r + 1) < 1.0 for r > 0, gamma > 0
        let speedup = SpeculativeEngine::theoretical_speedup(0.0, g, 0.1);
        let expected_s = 1.0 / (g as f64 * 0.1 + 1.0);
        assert!((speedup - expected_s).abs() < 1e-9);
    }

    // --- Boundary Case 2: alpha = 1.0 (Perfect Draft Acceptance) ---
    // E[beta] = gamma + 1. S(1, gamma, r) = (gamma + 1) / (gamma * r + 1).
    for g in [0, 1, 5, 10, 50, 100] {
        let e_beta = SpeculativeEngine::theoretical_expected_tokens(1.0, g);
        assert!((e_beta - (g + 1) as f64).abs() < 1e-9, "E[beta] must be gamma + 1 when alpha = 1.0");

        let speedup = SpeculativeEngine::theoretical_speedup(1.0, g, 0.05);
        let expected_s = (g + 1) as f64 / (g as f64 * 0.05 + 1.0);
        assert!((speedup - expected_s).abs() < 1e-9);
    }

    // --- Boundary Case 3: r -> 0 (Instantaneous Draft Model) ---
    // Denominator gamma * r + 1 -> 1.0, so S -> E[beta].
    let r_near_zero = 1e-12;
    for a in [0.0, 0.5, 0.8, 1.0] {
        for g in [1, 5, 10, 50] {
            let e_beta = SpeculativeEngine::theoretical_expected_tokens(a, g);
            let speedup = SpeculativeEngine::theoretical_speedup(a, g, r_near_zero);
            assert!((speedup - e_beta).abs() < 1e-8, "When r -> 0, S must equal E[beta]");
        }
    }

    // --- Boundary Case 4: r = 1.0 (Draft Model has Same Latency as Target) ---
    // Denominator = gamma + 1. S(alpha, gamma, 1) = E[beta] / (gamma + 1) <= 1.0.
    for a in [0.0, 0.2, 0.5, 0.8] {
        for g in [1, 5, 10, 50] {
            let speedup = SpeculativeEngine::theoretical_speedup(a, g, 1.0);
            assert!(speedup < 1.0, "When r = 1 and alpha < 1, speedup must be strictly < 1.0 (penalty)");
        }
    }
    // When alpha = 1.0 and r = 1.0: S = (gamma + 1) / (gamma + 1) = 1.0 exactly.
    for g in [0, 1, 5, 20] {
        let speedup = SpeculativeEngine::theoretical_speedup(1.0, g, 1.0);
        assert!((speedup - 1.0).abs() < 1e-9);
    }

    // --- Boundary Case 5: gamma = 0 (No Draft Tokens) ---
    // E[beta] = 1.0, denominator = 0 * r + 1 = 1.0. S = 1.0 for all alpha, r.
    for a in [0.0, 0.5, 1.0] {
        for r in [0.0, 0.1, 0.5, 1.0] {
            let speedup = SpeculativeEngine::theoretical_speedup(a, 0, r);
            assert!((speedup - 1.0).abs() < 1e-9, "When gamma = 0, speedup must be exactly 1.0");
        }
    }

    // --- Boundary Case 6: gamma = 100 (Large Draft Window) ---
    // For alpha = 0.5: E[beta] -> 1 / (1 - 0.5) = 2.0.
    let e_beta_100_half = SpeculativeEngine::theoretical_expected_tokens(0.5, 100);
    assert!((e_beta_100_half - 2.0).abs() < 1e-9);
    // Denominator for r = 0.1 is 100 * 0.1 + 1 = 11.0. Speedup = 2.0 / 11.0 ~ 0.1818 (severe regression!).
    let speedup_100_half = SpeculativeEngine::theoretical_speedup(0.5, 100, 0.1);
    assert!((speedup_100_half - (2.0 / 11.0)).abs() < 1e-4);

    // For alpha = 0.99: E[beta] = (1 - 0.99^101) / 0.01 = (1 - 0.36236) / 0.01 ~ 63.764
    let e_beta_100_high = SpeculativeEngine::theoretical_expected_tokens(0.99, 100);
    assert!((e_beta_100_high - 63.764).abs() < 1e-2);
    // With r = 0.01: denominator = 100 * 0.01 + 1 = 2.0. Speedup = 63.764 / 2.0 = 31.88x!
    let speedup_100_high = SpeculativeEngine::theoretical_speedup(0.99, 100, 0.01);
    assert!((speedup_100_high - 31.882).abs() < 1e-2);

    // Optimal draft length checks:
    let (opt_g, opt_s) = SpeculativeEngine::optimal_draft_length(0.70, 0.10, 20);
    assert!((2..=5).contains(&opt_g));
    assert!(opt_s > 1.4);

    let (opt_g2, opt_s2) = SpeculativeEngine::optimal_draft_length(0.95, 0.02, 50);
    assert!(opt_g2 > 10, "High alpha and low r should yield optimal gamma > 10");
    assert!(opt_s2 > 8.0);
}

/// 3. Empirical Monte Carlo verification of Theorem 13 (Exact Target Distribution Preservation):
///    Draft distribution Q differs drastically from target distribution P.
///    Rejection sampling must ensure that the empirical token emission distribution
///    converges to P with zero bias.
#[test]
fn challenge_rejection_sampling_distribution_preservation_monte_carlo() {
    let mut engine = SpeculativeEngine::with_seed(0xCAFE_BABE_0123_4567);

    // 3-token vocabulary: 1, 2, 3
    // Target distribution P: token 1: 0.20, token 2: 0.50, token 3: 0.30
    let target_dist = vec![
        TokenProbability::new(1, 0.20),
        TokenProbability::new(2, 0.50),
        TokenProbability::new(3, 0.30),
    ];
    // Draft distribution Q: token 1: 0.60, token 2: 0.30, token 3: 0.10 (drastically mismatched!)
    let draft_dist = vec![
        TokenProbability::new(1, 0.60),
        TokenProbability::new(2, 0.30),
        TokenProbability::new(3, 0.10),
    ];

    let num_trials = 25_000;
    let mut counts = [0usize; 4]; // index 1, 2, 3

    for _ in 0..num_trials {
        // Sample draft token from Q
        let draft_tok = engine.sample_from_distribution(&draft_dist).unwrap();

        let res = engine.verify_draft(
            &[draft_tok],
            std::slice::from_ref(&draft_dist),
            &[target_dist.clone(), target_dist.clone()],
        );

        // Record the final emitted token for position 0:
        // Either accepted draft token or bonus token sampled from positive residual
        let emitted = if !res.accepted_tokens.is_empty() {
            res.accepted_tokens[0]
        } else {
            res.bonus_token.expect("Must have bonus token upon rejection")
        };

        counts[emitted as usize] += 1;
    }

    let p1_emp = counts[1] as f64 / num_trials as f64;
    let p2_emp = counts[2] as f64 / num_trials as f64;
    let p3_emp = counts[3] as f64 / num_trials as f64;

    // Check convergence to Target distribution P = [0.20, 0.50, 0.30] within 1.5% Monte Carlo tolerance
    assert!((p1_emp - 0.20).abs() < 0.015, "P(1) empirical {p1_emp} must match target 0.20");
    assert!((p2_emp - 0.50).abs() < 0.015, "P(2) empirical {p2_emp} must match target 0.50");
    assert!((p3_emp - 0.30).abs() < 0.015, "P(3) empirical {p3_emp} must match target 0.30");
}

/// 4. Empirically verify RadixKvCache:
///    Stress-test shared prefix matching across 100 concurrent sessions with overlapping system prompts
///    and diverging user turns.
#[test]
fn challenge_radix_kv_cache_100_concurrent_sessions() {
    let mut cache = RadixKvCache::new(2048);

    // 100 sessions:
    // Shared System Prompt: 50 tokens (identical across all 100 sessions)
    let system_prompt: Vec<u32> = (1000..1050).collect();

    // 4 groups of 25 sessions each sharing a 30-token group instruction
    // Group 0: tokens 2000..2030
    // Group 1: tokens 2100..2130
    // Group 2: tokens 2200..2230
    // Group 3: tokens 2300..2330
    let group_instructions: Vec<Vec<u32>> = (0..4)
        .map(|g| (2000 + g * 100..2000 + g * 100 + 30).collect())
        .collect();

    let num_sessions = 100;
    let mut session_tokens_map = Vec::new();

    for s in 0..num_sessions {
        let group_id = s / 25;
        let mut tokens = system_prompt.clone();
        tokens.extend_from_slice(&group_instructions[group_id]);
        // Unique user turn: 20 tokens per session
        tokens.extend((5000 + s * 50..5000 + s * 50 + 20).map(|x| x as u32));
        assert_eq!(tokens.len(), 100);

        let (shared, allocated) = cache.insert(&format!("session_{s}"), &tokens);

        if s == 0 {
            // First session in cache: nothing shared
            assert_eq!(shared, 0);
            assert_eq!(allocated, 100);
        } else if s % 25 == 0 {
            // First session in subsequent group: shares 50 tokens (system prompt)
            assert_eq!(shared, 50);
            assert_eq!(allocated, 50);
        } else {
            // Subsequent session in same group: shares 80 tokens (system prompt + group instruction)
            assert_eq!(shared, 80);
            assert_eq!(allocated, 20);
        }

        session_tokens_map.push(tokens);
    }

    assert_eq!(cache.active_sessions(), 100);

    // Verify lookup_shared_prefix for all 100 sessions
    for (s, tokens) in session_tokens_map.iter().enumerate() {
        let matched = cache.lookup_shared_prefix(tokens);
        assert_eq!(matched, 100, "Full prompt of session_{s} must match resident Radix tree");
    }

    // Lookup a probe session with diverging user prompt
    let mut probe_tokens = system_prompt.clone();
    probe_tokens.extend_from_slice(&group_instructions[0]);
    probe_tokens.extend_from_slice(&[9999, 9998, 9997]); // unseen user tokens
    let probe_matched = cache.lookup_shared_prefix(&probe_tokens);
    assert_eq!(probe_matched, 80, "Probe must match 80 tokens (system + group 0)");

    // Total dense tokens = 100 * 100 = 10,000
    // Total compressed tokens:
    // 50 (system) + 4 * 30 (groups) + 100 * 20 (unique user turns) = 50 + 120 + 2000 = 2170 tokens
    let empirical_cr = cache.compression_ratio();
    let expected_cr = 10_000.0 / 2170.0;
    assert!((empirical_cr - expected_cr).abs() < 1e-4);
}

/// 5. Empirically verify RadixKvCache edge cases:
///    empty token sequences, single-token sequences, duplicate token sequences,
///    shared prefixes with partial word splits.
#[test]
fn challenge_radix_kv_cache_edge_cases_and_partial_splits() {
    let mut cache = RadixKvCache::new(1024);

    // --- Edge Case A: Empty Token Sequences ---
    let (shared_empty, alloc_empty) = cache.insert("empty_session", &[]);
    assert_eq!(shared_empty, 0);
    assert_eq!(alloc_empty, 0);
    assert_eq!(cache.lookup_shared_prefix(&[]), 0);
    assert_eq!(cache.release("empty_session"), 0);
    assert_eq!(cache.active_sessions(), 0);

    // --- Edge Case B: Single Token Sequences ---
    let (s_single1, a_single1) = cache.insert("single_1", &[42]);
    assert_eq!(s_single1, 0);
    assert_eq!(a_single1, 1);
    assert_eq!(cache.lookup_shared_prefix(&[42]), 1);

    // Single token sharing
    let (s_single2, a_single2) = cache.insert("single_2", &[42]);
    assert_eq!(s_single2, 1);
    assert_eq!(a_single2, 0);

    // Single token mismatch
    let (s_single3, a_single3) = cache.insert("single_3", &[99]);
    assert_eq!(s_single3, 0);
    assert_eq!(a_single3, 1);

    assert_eq!(cache.active_sessions(), 3);
    assert_eq!(cache.release("single_1"), 1);
    assert_eq!(cache.release("single_2"), 1);
    assert_eq!(cache.release("single_3"), 1);
    assert_eq!(cache.active_sessions(), 0);
    assert_eq!(cache.compression_ratio(), 1.0);

    // --- Edge Case C: Duplicate Token Sequences ---
    let seq = vec![7, 7, 7, 7];
    for i in 0..5 {
        let (shared, alloc) = cache.insert(&format!("dup_{i}"), &seq);
        if i == 0 {
            assert_eq!(shared, 0);
            assert_eq!(alloc, 4);
        } else {
            assert_eq!(shared, 4);
            assert_eq!(alloc, 0);
        }
    }
    assert_eq!(cache.active_sessions(), 5);
    // Release 4 out of 5
    for i in 0..4 {
        assert_eq!(cache.release(&format!("dup_{i}")), 4);
        assert_eq!(cache.lookup_shared_prefix(&seq), 4); // still resident because ref_count >= 1
    }
    // Release final session
    assert_eq!(cache.release("dup_4"), 4);
    assert_eq!(cache.active_sessions(), 0);
    assert_eq!(cache.lookup_shared_prefix(&seq), 0); // now completely pruned

    // --- Edge Case D: Complex Partial Edge Splits ---
    // Session 1: [10, 20, 30, 40, 50]
    cache.insert("s1", &[10, 20, 30, 40, 50]);
    // Session 2: [10, 20, 30, 41, 51] -> splits at index 3 ([10, 20, 30] prefix)
    let (s2_shared, s2_alloc) = cache.insert("s2", &[10, 20, 30, 41, 51]);
    assert_eq!(s2_shared, 3);
    assert_eq!(s2_alloc, 2);

    // Session 3: [10, 20, 32, 42] -> splits earlier at index 2 ([10, 20] prefix)
    let (s3_shared, s3_alloc) = cache.insert("s3", &[10, 20, 32, 42]);
    assert_eq!(s3_shared, 2);
    assert_eq!(s3_alloc, 2);

    // Session 4: [10, 25] -> splits at index 1 ([10] prefix)
    let (s4_shared, s4_alloc) = cache.insert("s4", &[10, 25]);
    assert_eq!(s4_shared, 1);
    assert_eq!(s4_alloc, 1);

    // Session 5: [15] -> completely separate root branch
    let (s5_shared, s5_alloc) = cache.insert("s5", &[15]);
    assert_eq!(s5_shared, 0);
    assert_eq!(s5_alloc, 1);

    // Verify all lookups
    assert_eq!(cache.lookup_shared_prefix(&[10, 20, 30, 40, 50]), 5);
    assert_eq!(cache.lookup_shared_prefix(&[10, 20, 30, 41, 51]), 5);
    assert_eq!(cache.lookup_shared_prefix(&[10, 20, 32, 42]), 4);
    assert_eq!(cache.lookup_shared_prefix(&[10, 25]), 2);
    assert_eq!(cache.lookup_shared_prefix(&[15]), 1);
    assert_eq!(cache.lookup_shared_prefix(&[999]), 0);

    // Clean up in non-FIFO order: s3, s1, s5, s2, s4
    assert_eq!(cache.release("s3"), 4);
    assert_eq!(cache.release("s1"), 5);
    assert_eq!(cache.release("s5"), 1);
    assert_eq!(cache.release("s2"), 5);
    assert_eq!(cache.release("s4"), 2);
    assert_eq!(cache.active_sessions(), 0);
    assert_eq!(cache.compression_ratio(), 1.0);
}

/// 6. Eviction/release cycles verifying zero memory leaks, ref counting, and complete tree cleanup.
#[test]
fn challenge_radix_kv_cache_eviction_zero_leak_and_ref_counts() {
    let mut cache = RadixKvCache::new(512);

    // Insert 100 sessions with shared prefixes and unique tails
    let base_prefix: Vec<u32> = (1..=40).collect();
    for i in 0..100 {
        let mut tokens = base_prefix.clone();
        tokens.extend((1000 + i * 10..1000 + i * 10 + 10).map(|x| x as u32));
        cache.insert(&format!("sess_{i}"), &tokens);
    }
    assert_eq!(cache.active_sessions(), 100);

    // Release all 100 sessions in arbitrary order (evens first, then odds)
    for i in (0..100).step_by(2) {
        let freed = cache.release(&format!("sess_{i}"));
        assert_eq!(freed, 50);
    }
    assert_eq!(cache.active_sessions(), 50);
    // Base prefix must still be resident because odd sessions hold references
    assert_eq!(cache.lookup_shared_prefix(&base_prefix), 40);

    for i in (1..100).step_by(2) {
        let freed = cache.release(&format!("sess_{i}"));
        assert_eq!(freed, 50);
    }
    assert_eq!(cache.active_sessions(), 0);
    assert_eq!(cache.lookup_shared_prefix(&base_prefix), 0); // Must be 0: fully pruned

    // Zero memory leak check:
    // Inserting a fresh brand-new session must start from clean state
    assert_eq!(cache.compression_ratio(), 1.0);

    let fresh_tokens = vec![9999, 8888, 7777];
    let (fresh_shared, fresh_alloc) = cache.insert("fresh_session", &fresh_tokens);
    assert_eq!(fresh_shared, 0, "No residual tokens should be shared after complete release");
    assert_eq!(fresh_alloc, 3);
    assert_eq!(cache.compression_ratio(), 1.0);

    cache.release("fresh_session");
    assert_eq!(cache.active_sessions(), 0);
    assert_eq!(cache.compression_ratio(), 1.0);
}

/// 7. Verify empirical compression ratio matches theoretical asymptotic prediction:
///    C_R -> 1 / (1 - eta) within 1%.
#[test]
fn challenge_radix_kv_cache_empirical_compression_ratio_within_1_percent() {
    // --- Test Scenario 1: eta = 0.80 -> Asymptotic limit C_R = 1 / (1 - 0.80) = 5.0x ---
    // With M = 500 sessions:
    // Shared prefix = 800 tokens, unique per session = 200 tokens (total 1000 tokens per session)
    // eta = 800 / 1000 = 0.80
    let mut cache_80 = RadixKvCache::new(2048);
    let common_prefix_80: Vec<u32> = (1..=800).collect();
    let num_sessions_80 = 500;

    for s in 0..num_sessions_80 {
        let mut tokens = common_prefix_80.clone();
        tokens.extend((100_000 + s * 200..100_000 + s * 200 + 200).map(|x| x as u32));
        cache_80.insert(&format!("sess_80_{s}"), &tokens);
    }

    let empirical_cr_80 = cache_80.compression_ratio();
    let asymptotic_cr_80 = RadixKvCache::asymptotic_compression_ratio(0.80);
    assert!((asymptotic_cr_80 - 5.0).abs() < 1e-9);

    // Exact theoretical for M=500:
    // C_R(500, 0.8) = 1 / ((1 - 0.8) + 0.8 / 500) = 1 / (0.2 + 0.0016) = 1 / 0.2016 = 4.960317...
    let theoretical_cr_80 = RadixKvCache::theoretical_compression_ratio(500, 0.80);
    assert!((empirical_cr_80 - theoretical_cr_80).abs() < 1e-4);

    // Check empirical match with asymptotic prediction within 1%:
    let relative_error_80 = (empirical_cr_80 - asymptotic_cr_80).abs() / asymptotic_cr_80;
    assert!(
        relative_error_80 < 0.01,
        "Empirical compression ratio {empirical_cr_80} must be within 1% of asymptotic 5.0x (error: {:.4}%)",
        relative_error_80 * 100.0
    );

    // --- Test Scenario 2: eta = 0.90 -> Asymptotic limit C_R = 1 / (1 - 0.90) = 10.0x ---
    // With M = 1000 sessions:
    // Shared prefix = 900 tokens, unique per session = 100 tokens (total 1000 tokens per session)
    // eta = 900 / 1000 = 0.90
    let mut cache_90 = RadixKvCache::new(2048);
    let common_prefix_90: Vec<u32> = (1..=900).collect();
    let num_sessions_90 = 1000;

    for s in 0..num_sessions_90 {
        let mut tokens = common_prefix_90.clone();
        tokens.extend((200_000 + s * 100..200_000 + s * 100 + 100).map(|x| x as u32));
        cache_90.insert(&format!("sess_90_{s}"), &tokens);
    }

    let empirical_cr_90 = cache_90.compression_ratio();
    let asymptotic_cr_90 = RadixKvCache::asymptotic_compression_ratio(0.90);
    assert!((asymptotic_cr_90 - 10.0).abs() < 1e-9);

    // Exact theoretical for M=1000:
    // C_R(1000, 0.9) = 1 / ((1 - 0.9) + 0.9 / 1000) = 1 / (0.1 + 0.0009) = 1 / 0.1009 = 9.91080...
    let theoretical_cr_90 = RadixKvCache::theoretical_compression_ratio(1000, 0.90);
    assert!((empirical_cr_90 - theoretical_cr_90).abs() < 1e-4);

    let relative_error_90 = (empirical_cr_90 - asymptotic_cr_90).abs() / asymptotic_cr_90;
    assert!(
        relative_error_90 < 0.01,
        "Empirical compression ratio {empirical_cr_90} must be within 1% of asymptotic 10.0x (error: {:.4}%)",
        relative_error_90 * 100.0
    );
}

/// 8. RadixKvNode recursive counters and invariants
#[test]
fn challenge_radix_kv_node_internal_invariants() {
    let mut root = RadixKvNode::new(vec![], 0);
    assert_eq!(root.count_nodes(), 1);
    assert_eq!(root.count_tokens(), 0);
    assert_eq!(root.total_kv_bytes(), 0);

    let child1 = RadixKvNode::new(vec![1, 2, 3], 3 * 2048);
    let child2 = RadixKvNode::new(vec![4, 5], 2 * 2048);
    root.children.insert(1, child1);
    root.children.insert(4, child2);

    assert_eq!(root.count_nodes(), 3);
    assert_eq!(root.count_tokens(), 5);
    assert_eq!(root.total_kv_bytes(), 5 * 2048);
}
