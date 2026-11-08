#![forbid(unsafe_code)]

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(7000);

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
            .expect("clock available")
            .as_nanos();
        let seq = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let temp_root = std::env::temp_dir()
            .canonicalize()
            .expect("canonical temp dir");
        let root = temp_root.join(format!(
            "pandora-math-stress-{}-{timestamp}-{seq}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        let data = root.join("data");
        fs::create_dir_all(&workspace).expect("create workspace");
        fs::create_dir_all(&data).expect("create data dir");
        Self {
            config: root.join("config.json"),
            data,
            workspace,
            root,
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_pandora"));
        cmd.args(args)
            .env("PANDORA_CONFIG", &self.config)
            .env("PANDORA_DATA_DIR", &self.data)
            .env("PANDORA_WORKSPACE", &self.workspace);
        cmd
    }

    fn run_success(&self, args: &[&str]) -> Value {
        let output = self.command(args).output().expect("command execution");
        assert!(
            output.status.success(),
            "command '{:?}' failed with stderr: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str(&stdout).expect("stdout valid json envelope")
    }

    fn run_err(&self, args: &[&str]) -> (std::process::ExitStatus, String, String) {
        let output = self.command(args).output().expect("command execution");
        assert!(
            !output.status.success(),
            "command '{:?}' was expected to fail, but succeeded with stdout: {}",
            args,
            String::from_utf8_lossy(&output.stdout)
        );
        (
            output.status,
            String::from_utf8_lossy(&output.stdout).to_string(),
            String::from_utf8_lossy(&output.stderr).to_string(),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

// ============================================================================
// 1. MULTI-ARMED BANDIT MATHEMATICAL REGRET BOUND STRESS TESTS
// ============================================================================

#[test]
fn test_math_mab_regret_bounds_across_strategies_seeds_and_rounds() {
    let fixture = Fixture::new();

    let strategies = ["ucb1", "thompson", "epsilon-greedy"];
    let seeds = [42u64, 100, 1337, 0x1234_5678, 0xdead_beef];
    let round_counts = [1usize, 2, 5, 20, 50, 100];

    for strategy in strategies {
        for seed in seeds {
            for rounds in round_counts {
                let rounds_str = rounds.to_string();
                let seed_str = seed.to_string();

                let mut args = vec![
                    "models",
                    "route",
                    "--strategy",
                    strategy,
                    "--rounds",
                    &rounds_str,
                    "--seed",
                    &seed_str,
                    "--json",
                ];

                if strategy == "ucb1" {
                    args.push("--c");
                    args.push("1.414");
                } else if strategy == "epsilon-greedy" {
                    args.push("--epsilon");
                    args.push("0.1");
                }

                let val = fixture.run_success(&args);

                let k = val["arm_count"].as_u64().unwrap() as f64;
                let t = val["rounds"].as_u64().unwrap() as f64;
                let empirical_regret = val["regret"]["cumulative_empirical_regret"]
                    .as_f64()
                    .unwrap();
                let minimax_bound = val["regret"]["theoretical_minimax_bound"].as_f64().unwrap();
                let bound_satisfied = val["regret"]["bound_satisfied"].as_bool().unwrap();

                // Regret non-negativity invariant: R(T) >= 0
                assert!(
                    empirical_regret >= 0.0,
                    "Empirical regret must be non-negative: {empirical_regret}"
                );

                // Minimax formula verification: 2*sqrt(8*K*T*ln(T)) + K*(1 + pi^2/3)
                let ln_t = t.ln().max(1.0);
                let expected_bound = if k == 0.0 || t <= 1.0 {
                    0.0
                } else {
                    2.0 * (8.0 * k * t * ln_t).sqrt()
                        + (k * (1.0 + std::f64::consts::PI.powi(2) / 3.0))
                };

                assert!(
                    (minimax_bound - expected_bound).abs() < 1e-6,
                    "Theoretical minimax bound mismatch: reported {minimax_bound}, expected {expected_bound}"
                );

                // Minimax regret bound invariant: R(T) <= theoretical_minimax_bound
                assert!(
                    bound_satisfied,
                    "Regret bound violated for strategy {strategy}, seed {seed}, rounds {rounds}: empirical {empirical_regret} > bound {minimax_bound}"
                );
                assert!(
                    empirical_regret <= minimax_bound + 1e-9,
                    "Empirical regret exceeds theoretical minimax bound"
                );
            }
        }
    }
}

#[test]
fn test_math_mab_custom_arms_and_extreme_contexts() {
    let fixture = Fixture::new();

    // Test custom arms with varying K: 2 arms, 5 arms, 8 arms
    let custom_arms_cases = [
        "arm-a:model-a:provider-a,arm-b:model-b:provider-b",
        "a1:m1:p1,a2:m2:p2,a3:m3:p3,a4:m4:p4,a5:m5:p5",
        "m1:x:y,m2:x:y,m3:x:y,m4:x:y,m5:x:y,m6:x:y,m7:x:y,m8:x:y",
    ];

    for arms_spec in custom_arms_cases {
        let expected_k = arms_spec.split(',').count();
        let val = fixture.run_success(&[
            "models", "route", "--arms", arms_spec, "--rounds", "30", "--seed", "999", "--json",
        ]);

        assert_eq!(val["arm_count"].as_u64().unwrap() as usize, expected_k);
        let bound_satisfied = val["regret"]["bound_satisfied"].as_bool().unwrap();
        assert!(bound_satisfied);
    }

    // Extreme context vectors: query length 1 vs 10000, complexity 0.0 vs 1.0
    let context_cases = [
        (
            "--len",
            "1",
            "--complexity",
            "0.0",
            "--cache-affinity",
            "0.0",
            "--max-latency-ms",
            "100.0",
        ),
        (
            "--len",
            "10000",
            "--complexity",
            "1.0",
            "--cache-affinity",
            "1.0",
            "--max-latency-ms",
            "60000.0",
        ),
    ];

    for (len_flag, len_val, comp_flag, comp_val, cache_flag, cache_val, lat_flag, lat_val) in
        context_cases
    {
        let val = fixture.run_success(&[
            "models", "route", len_flag, len_val, comp_flag, comp_val, cache_flag, cache_val,
            lat_flag, lat_val, "--rounds", "25", "--json",
        ]);

        let features = val["context"]["features"].as_array().unwrap();
        assert_eq!(features.len(), 5);
        for feat in features {
            let f = feat.as_f64().unwrap();
            assert!((0.0..=1.0).contains(&f), "Feature outside [0, 1]: {f}");
        }
        assert!(val["regret"]["bound_satisfied"].as_bool().unwrap());
    }
}

// ============================================================================
// 2. MERKLE-DAG INCLUSION PROOFS & TAMPER DETECTION STRESS TESTS
// ============================================================================

#[test]
fn test_math_merkle_dag_proof_length_log2_invariant_and_tamper_detection() {
    let fixture = Fixture::new();
    // Test across various tree sizes: powers of 2 and odd/unbalanced counts
    let leaf_counts = [1usize, 2, 3, 4, 5, 7, 8, 9, 13, 16, 25, 32];

    for &n in &leaf_counts {
        // Create fresh store for each n
        let current_store = fixture.root.join(format!("store_n_{n}.json"));
        let current_store_str = current_store.to_str().unwrap();

        for i in 0..n {
            let payload = format!("state_event_{n}_{i}");
            fixture.run_success(&[
                "audit",
                "record",
                "--payload",
                &payload,
                "--store",
                current_store_str,
                "--json",
            ]);
        }

        // Verify tree inspection
        let tree_val =
            fixture.run_success(&["audit", "tree", "--store", current_store_str, "--json"]);
        assert_eq!(tree_val["leaf_count"].as_u64().unwrap() as usize, n);

        let expected_depth = if n == 0 {
            0
        } else {
            (n as f64).log2().ceil() as usize
        };
        assert_eq!(
            tree_val["tree_depth"].as_u64().unwrap() as usize,
            expected_depth
        );

        // Generate and verify inclusion proofs for all leaves if n <= 8, or sample leaves
        let test_indices: Vec<usize> = if n <= 8 {
            (0..n).collect()
        } else {
            vec![0, 1, n / 2, n - 2, n - 1]
        };

        for &idx in &test_indices {
            let idx_str = idx.to_string();
            let proof_val = fixture.run_success(&[
                "audit",
                "proof",
                "--index",
                &idx_str,
                "--store",
                current_store_str,
                "--json",
            ]);

            // Proof length mathematical invariant: |pi(i)| = ceil(log2(N))
            let proof_len = proof_val["proof_length"].as_u64().unwrap() as usize;
            assert_eq!(
                proof_len, expected_depth,
                "Proof length invariant failed for N={n}, index={idx}: got {proof_len}, expected {expected_depth}"
            );
            assert!(proof_val["verified"].as_bool().unwrap());

            // Export proof to disk for adversarial mutation tests
            let proof_file = fixture.root.join(format!("proof_{n}_{idx}.json"));
            let proof_file_str = proof_file.to_str().unwrap();

            fixture.run_success(&[
                "audit",
                "proof",
                "--index",
                &idx_str,
                "--store",
                current_store_str,
                "--output",
                proof_file_str,
                "--json",
            ]);

            // Verify original proof file passes
            let verify_ok =
                fixture.run_success(&["audit", "verify", "--proof-file", proof_file_str, "--json"]);
            assert!(verify_ok["verified"].as_bool().unwrap());
            assert_eq!(verify_ok["tamper_status"].as_str().unwrap(), "UNMODIFIED");

            // --- ADVERSARIAL TAMPER ORACLE ---
            let original_proof_raw = fs::read_to_string(&proof_file).unwrap();
            let proof_obj: Value = serde_json::from_str(&original_proof_raw).unwrap();

            // 1. Bit-flip in leaf_hash: must be rejected with 100% certainty (0 false positives)
            {
                let mut tampered_obj = proof_obj.clone();
                let leaf_arr = tampered_obj["leaf_hash"].as_array_mut().unwrap();
                let orig_byte = leaf_arr[0].as_u64().unwrap();
                leaf_arr[0] = Value::from(orig_byte ^ 0x01); // flip lowest bit

                let tampered_file = fixture.root.join(format!("proof_leaf_flip_{n}_{idx}.json"));
                fs::write(
                    &tampered_file,
                    serde_json::to_string(&tampered_obj).unwrap(),
                )
                .unwrap();

                let verify_fail = fixture.run_success(&[
                    "audit",
                    "verify",
                    "--proof-file",
                    tampered_file.to_str().unwrap(),
                    "--json",
                ]);
                assert!(
                    !verify_fail["verified"].as_bool().unwrap(),
                    "Leaf hash tamper was not detected!"
                );
                assert_eq!(
                    verify_fail["tamper_status"].as_str().unwrap(),
                    "CORRUPTED_OR_TAMPERED"
                );
            }

            // 2. Bit-flip in sibling_hash (for proofs with depth >= 1)
            if proof_len > 0 {
                let mut tampered_obj = proof_obj.clone();
                let steps = tampered_obj["steps"].as_array_mut().unwrap();
                let sibling_arr = steps[0]["sibling_hash"].as_array_mut().unwrap();
                let orig_byte = sibling_arr[0].as_u64().unwrap();
                sibling_arr[0] = Value::from(orig_byte ^ 0x80); // flip highest bit

                let tampered_file = fixture.root.join(format!("proof_sib_flip_{n}_{idx}.json"));
                fs::write(
                    &tampered_file,
                    serde_json::to_string(&tampered_obj).unwrap(),
                )
                .unwrap();

                let verify_fail = fixture.run_success(&[
                    "audit",
                    "verify",
                    "--proof-file",
                    tampered_file.to_str().unwrap(),
                    "--json",
                ]);
                assert!(
                    !verify_fail["verified"].as_bool().unwrap(),
                    "Sibling hash tamper was not detected!"
                );
                assert_eq!(
                    verify_fail["tamper_status"].as_str().unwrap(),
                    "CORRUPTED_OR_TAMPERED"
                );
            }

            // 3. Bit-flip in root_hash
            {
                let mut tampered_obj = proof_obj.clone();
                let root_arr = tampered_obj["root_hash"].as_array_mut().unwrap();
                let orig_byte = root_arr[0].as_u64().unwrap();
                root_arr[0] = Value::from(orig_byte ^ 0x40);

                let tampered_file = fixture.root.join(format!("proof_root_flip_{n}_{idx}.json"));
                fs::write(
                    &tampered_file,
                    serde_json::to_string(&tampered_obj).unwrap(),
                )
                .unwrap();

                let verify_fail = fixture.run_success(&[
                    "audit",
                    "verify",
                    "--proof-file",
                    tampered_file.to_str().unwrap(),
                    "--json",
                ]);
                assert!(
                    !verify_fail["verified"].as_bool().unwrap(),
                    "Root hash tamper was not detected!"
                );
                assert_eq!(
                    verify_fail["tamper_status"].as_str().unwrap(),
                    "CORRUPTED_OR_TAMPERED"
                );
            }
        }
    }
}

#[test]
fn test_math_merkle_dag_diamond_lineage_and_acyclicity() {
    let fixture = Fixture::new();
    let store_path = fixture.root.join("dag_diamond.json");
    let store_str = store_path.to_str().unwrap();

    // Create Diamond DAG:
    // Genesis -> Node A, Node B -> Node C (merges A and B) -> Node D (merges C and Genesis)
    let genesis_record = fixture.run_success(&[
        "audit",
        "record",
        "--payload",
        "genesis_block",
        "--parent",
        "",
        "--store",
        store_str,
        "--json",
    ]);
    let gen_id = genesis_record["dag_node"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let node_a = fixture.run_success(&[
        "audit",
        "record",
        "--payload",
        "branch_a",
        "--parent",
        &gen_id,
        "--store",
        store_str,
        "--json",
    ]);
    let a_id = node_a["dag_node"]["id"].as_str().unwrap().to_string();

    let node_b = fixture.run_success(&[
        "audit",
        "record",
        "--payload",
        "branch_b",
        "--parent",
        &gen_id,
        "--store",
        store_str,
        "--json",
    ]);
    let b_id = node_b["dag_node"]["id"].as_str().unwrap().to_string();

    let parents_ab = format!("{a_id},{b_id}");
    let node_c = fixture.run_success(&[
        "audit",
        "record",
        "--payload",
        "merge_c",
        "--parent",
        &parents_ab,
        "--store",
        store_str,
        "--json",
    ]);
    let c_id = node_c["dag_node"]["id"].as_str().unwrap().to_string();

    let parents_cg = format!("{c_id},{gen_id}");
    let node_d = fixture.run_success(&[
        "audit",
        "record",
        "--payload",
        "final_merge_d",
        "--parent",
        &parents_cg,
        "--store",
        store_str,
        "--json",
    ]);
    let d_id = node_d["dag_node"]["id"].as_str().unwrap().to_string();

    // Verify causal lineage of final node D back to genesis
    let verify_lineage = fixture.run_success(&[
        "audit",
        "verify",
        "--dag-node",
        &d_id,
        "--store",
        store_str,
        "--json",
    ]);
    assert!(verify_lineage["verified"].as_bool().unwrap());
    assert_eq!(verify_lineage["node_id"].as_str().unwrap(), d_id);
    assert_eq!(
        verify_lineage["mode"].as_str().unwrap(),
        "dag_causal_lineage"
    );
    assert_eq!(
        verify_lineage["tamper_status"].as_str().unwrap(),
        "UNMODIFIED"
    );

    // Verify tree DAG properties
    let tree_info = fixture.run_success(&["audit", "tree", "--store", store_str, "--json"]);
    assert_eq!(tree_info["dag_node_count"].as_u64().unwrap(), 5);
    assert!(tree_info["is_acyclic"].as_bool().unwrap());
    assert_eq!(tree_info["topological_order"].as_array().unwrap().len(), 5);
}

// ============================================================================
// 3. SPECULATIVE DECODING & RADIX KV CACHE MATHEMATICAL STRESS TESTS
// ============================================================================

#[test]
fn test_math_speculative_decoding_theoretical_bounds_and_empirical_speedup() {
    let fixture = Fixture::new();

    // Test Boundary 1: alpha = 0.0 (draft tokens never accepted)
    // E[beta] = (1 - 0^(gamma+1)) / (1 - 0) = 1.0 token/cycle (only 1 bonus token)
    // Theoretical speedup S = 1 / (gamma * r + 1) < 1.0
    {
        let val = fixture.run_success(&[
            "runtime",
            "speculative",
            "benchmark",
            "--gamma",
            "4",
            "--alpha",
            "0.0",
            "--ratio",
            "0.1",
            "--tokens",
            "100",
            "--seed",
            "12345",
            "--json",
        ]);

        let theoretical_expected = val["theoretical_expected_tokens"].as_f64().unwrap();
        let theoretical_speedup = val["theoretical_speedup"].as_f64().unwrap();
        let empirical_speedup = val["empirical_speedup"].as_f64().unwrap();
        let empirical_acceptance_rate = val["empirical_acceptance_rate"].as_f64().unwrap();
        let empirical_tokens_per_cycle = val["empirical_tokens_per_cycle"].as_f64().unwrap();

        assert!((theoretical_expected - 1.0).abs() < 1e-9);
        assert!((theoretical_speedup - (1.0 / (4.0 * 0.1 + 1.0))).abs() < 1e-9);
        assert_eq!(empirical_acceptance_rate, 0.0);
        assert!((empirical_tokens_per_cycle - 1.0).abs() < 1e-9);
        assert!((empirical_speedup - theoretical_speedup).abs() < 1e-9);
    }

    // Test Boundary 2: alpha = 1.0 (draft tokens 100% accepted)
    // E[beta] = gamma + 1
    // Theoretical speedup S = (gamma + 1) / (gamma * r + 1)
    {
        let val = fixture.run_success(&[
            "runtime",
            "speculative",
            "benchmark",
            "--gamma",
            "5",
            "--alpha",
            "1.0",
            "--ratio",
            "0.1",
            "--tokens",
            "120",
            "--seed",
            "54321",
            "--json",
        ]);

        let theoretical_expected = val["theoretical_expected_tokens"].as_f64().unwrap();
        let theoretical_speedup = val["theoretical_speedup"].as_f64().unwrap();
        let empirical_speedup = val["empirical_speedup"].as_f64().unwrap();
        let empirical_acceptance_rate = val["empirical_acceptance_rate"].as_f64().unwrap();
        let empirical_tokens_per_cycle = val["empirical_tokens_per_cycle"].as_f64().unwrap();

        assert!((theoretical_expected - 6.0).abs() < 1e-9);
        let expected_s = 6.0 / (5.0 * 0.1 + 1.0);
        assert!((theoretical_speedup - expected_s).abs() < 1e-9);
        assert_eq!(empirical_acceptance_rate, 1.0);
        assert!((empirical_tokens_per_cycle - 6.0).abs() < 1e-9);
        assert!((empirical_speedup - expected_s).abs() < 1e-9);
    }

    // Test Parameter Grid: verify mathematical formulas for E[beta], S(alpha, gamma, r), and gamma*
    let alpha_grid: [f64; 3] = [0.5, 0.75, 0.9];
    let gamma_grid = [2usize, 4, 8];
    let ratio_grid: [f64; 3] = [0.05, 0.15, 0.3];

    for &alpha in &alpha_grid {
        for &gamma in &gamma_grid {
            for &ratio in &ratio_grid {
                let alpha_s = alpha.to_string();
                let gamma_s = gamma.to_string();
                let ratio_s = ratio.to_string();

                let val = fixture.run_success(&[
                    "runtime",
                    "speculative",
                    "benchmark",
                    "--gamma",
                    &gamma_s,
                    "--alpha",
                    &alpha_s,
                    "--ratio",
                    &ratio_s,
                    "--tokens",
                    "200",
                    "--seed",
                    "77777",
                    "--json",
                ]);

                let th_exp = val["theoretical_expected_tokens"].as_f64().unwrap();
                let th_speedup = val["theoretical_speedup"].as_f64().unwrap();
                let opt_gamma = val["optimal_draft_length"].as_u64().unwrap() as usize;
                let opt_speedup = val["optimal_theoretical_speedup"].as_f64().unwrap();

                // Check expected tokens formula: (1 - alpha^(gamma+1)) / (1 - alpha)
                let expected_tokens_math = (1.0 - alpha.powi(gamma as i32 + 1)) / (1.0 - alpha);
                assert!((th_exp - expected_tokens_math).abs() < 1e-6);

                // Check speedup formula: E[beta] / (gamma * ratio + 1)
                let expected_speedup_math = expected_tokens_math / (gamma as f64 * ratio + 1.0);
                assert!((th_speedup - expected_speedup_math).abs() < 1e-6);

                // Check that optimal gamma yields >= speedup of current gamma
                assert!(opt_speedup >= th_speedup - 1e-9);
                assert!((1..=16).contains(&opt_gamma));
            }
        }
    }
}

#[test]
fn test_math_radix_kv_cache_compression_ratio_exact_equivalence() {
    let fixture = Fixture::new();

    // Test grid of sessions M, prefix len Lp, suffix len Ls
    let test_cases = [
        // (sessions, prefix_len, suffix_len)
        (1usize, 512usize, 256usize), // M=1: baseline, C_R must be 1.0
        (4, 512, 256),
        (8, 1024, 128),
        (16, 2048, 512),
        (32, 256, 64),
        (8, 512, 0), // Ls=0: identical prompts, C_R must equal M = 8
    ];

    for (m, lp, ls) in test_cases {
        let m_s = m.to_string();
        let lp_s = lp.to_string();
        let ls_s = ls.to_string();

        let val = fixture.run_success(&[
            "runtime",
            "kv-cache",
            "--sessions",
            &m_s,
            "--prefix-len",
            &lp_s,
            "--suffix-len",
            &ls_s,
            "--bytes-per-token",
            "2048",
            "--json",
        ]);

        let empirical_cr = val["empirical_compression_ratio"].as_f64().unwrap();
        let theoretical_cr = val["theoretical_compression_ratio"].as_f64().unwrap();
        let asymptotic_cr = val["asymptotic_compression_ratio"].as_f64().unwrap();
        let prefix_eta = val["prefix_fraction_eta"].as_f64().unwrap();
        let dense_bytes = val["dense_memory_bytes"].as_u64().unwrap();
        let compressed_bytes = val["compressed_memory_bytes"].as_u64().unwrap();

        let expected_eta = lp as f64 / (lp + ls) as f64;
        assert!((prefix_eta - expected_eta).abs() < 1e-9);

        // Theoretical formula in pandora-runtime clamps eta to 0.999 to prevent div-by-zero in asymptotic limit
        let clamped_eta = expected_eta.clamp(0.0, 0.999);
        let expected_th_cr = 1.0 / ((1.0 - clamped_eta) + (clamped_eta / m as f64));
        assert!(
            (theoretical_cr - expected_th_cr).abs() < 1e-6,
            "Theoretical CR formula mismatch for M={m}, Lp={lp}, Ls={ls}"
        );

        // Exact equality: empirical CR = dense_bytes / compressed_bytes
        let expected_emp_cr = dense_bytes as f64 / compressed_bytes as f64;
        assert!((empirical_cr - expected_emp_cr).abs() < 1e-9);

        // For common prefix sharing with Ls > 0, empirical and theoretical CR match within 1e-4
        if ls > 0 {
            assert!(
                (empirical_cr - theoretical_cr).abs() < 1e-4,
                "Empirical CR ({empirical_cr}) != Theoretical CR ({theoretical_cr}) for M={m}, Lp={lp}, Ls={ls}"
            );
        } else {
            // When Ls = 0, empirical CR is exact M, while theoretical is clamped to 0.999
            assert!((empirical_cr - m as f64).abs() < 1e-9);
            assert!((empirical_cr - theoretical_cr).abs() < 0.1);
        }

        // Asymptotic bound: C_R(M, eta) <= asymptotic_cr = 1 / (1 - eta)
        assert!(
            theoretical_cr <= asymptotic_cr + 1e-6,
            "Theoretical CR exceeded asymptotic limit"
        );

        // If M=1, CR must be exactly 1.0
        if m == 1 {
            assert!((empirical_cr - 1.0).abs() < 1e-9);
        }

        // If Ls=0, all sessions share 100% of tokens, CR must equal M
        if ls == 0 {
            assert!((empirical_cr - m as f64).abs() < 1e-9);
        }
    }
}

// ============================================================================
// 4. SUBCOMMAND ALIAS PARITY & ZERO PANIC VALIDATION
// ============================================================================

#[test]
fn test_math_subcommand_aliases_parity_and_panic_resistance() {
    let fixture = Fixture::new();

    // 1. Alias Parity: `pandora backup audit tree` == `pandora audit tree`
    let audit_tree = fixture.run_success(&["audit", "tree", "--json"]);
    let backup_audit_tree = fixture.run_success(&["backup", "audit", "tree", "--json"]);
    assert_eq!(audit_tree["leaf_count"], backup_audit_tree["leaf_count"]);
    assert_eq!(audit_tree["root_hash"], backup_audit_tree["root_hash"]);

    // 2. Alias Parity: `pandora hardware speculative benchmark` == `pandora runtime speculative benchmark`
    let run_spec = fixture.run_success(&[
        "runtime",
        "speculative",
        "benchmark",
        "--gamma",
        "3",
        "--alpha",
        "0.7",
        "--seed",
        "888",
        "--json",
    ]);
    let hw_spec = fixture.run_success(&[
        "hardware",
        "speculative",
        "benchmark",
        "--gamma",
        "3",
        "--alpha",
        "0.7",
        "--seed",
        "888",
        "--json",
    ]);
    assert_eq!(run_spec["empirical_speedup"], hw_spec["empirical_speedup"]);
    assert_eq!(
        run_spec["theoretical_speedup"],
        hw_spec["theoretical_speedup"]
    );

    // 3. Alias Parity: `pandora hardware kv-cache` == `pandora runtime kv-cache`
    let run_kv = fixture.run_success(&["runtime", "kv-cache", "--sessions", "4", "--json"]);
    let hw_kv = fixture.run_success(&["hardware", "kv-cache", "--sessions", "4", "--json"]);
    assert_eq!(
        run_kv["empirical_compression_ratio"],
        hw_kv["empirical_compression_ratio"]
    );

    // 4. Zero Panic Resistance: Adversarial inputs must return clean errors without crashing
    let invalid_invocations: Vec<&[&str]> = vec![
        &["models", "route", "--strategy", "invalid_strategy_xyz"],
        &["models", "route", "--c", "-5.0"],
        &["models", "route", "--epsilon", "1.5"],
        &["models", "route", "--rounds", "0"],
        &["models", "route", "--query-length", "-10"],
        &["models", "route", "--complexity", "2.0"],
        &["audit", "proof", "--index", "999999"],
        &["audit", "record"], // missing payload/file
        &["audit", "record", "--payload", "foo", "--file", "bar"], // mutually exclusive
        &["audit", "verify"], // missing proof/node
        &["runtime", "speculative", "benchmark", "--gamma", "0"],
        &["runtime", "speculative", "benchmark", "--gamma", "50"],
        &["runtime", "speculative", "benchmark", "--alpha", "-0.5"],
        &["runtime", "speculative", "benchmark", "--ratio", "-0.1"],
        &["runtime", "kv-cache", "--sessions", "0"],
    ];

    for args in invalid_invocations {
        let (status, stdout, stderr) = fixture.run_err(args);
        assert!(!status.success());
        // Verify no panic backtrace in stderr
        assert!(
            !stderr.contains("panicked at"),
            "Command '{:?}' panicked! Stderr: {stderr}",
            args
        );
        assert!(
            !stdout.contains("panicked at"),
            "Command '{:?}' panicked! Stdout: {stdout}",
            args
        );
    }
}

#[test]
fn test_math_merkle_exhaustive_single_bit_flip_tamper_oracle() {
    let fixture = Fixture::new();
    let store_path = fixture.root.join("exhaustive_bit_flip.json");
    let store_str = store_path.to_str().unwrap();

    let n = 16;
    for i in 0..n {
        let payload = format!("state_event_{i}");
        fixture.run_success(&[
            "audit",
            "record",
            "--payload",
            &payload,
            "--store",
            store_str,
            "--json",
        ]);
    }

    let proof_file = fixture.root.join("proof_target.json");
    let proof_file_str = proof_file.to_str().unwrap();
    fixture.run_success(&[
        "audit",
        "proof",
        "--index",
        "7",
        "--store",
        store_str,
        "--output",
        proof_file_str,
        "--json",
    ]);

    let original_raw = fs::read_to_string(&proof_file).unwrap();
    let proof_obj: Value = serde_json::from_str(&original_raw).unwrap();

    // Exhaustively flip all 256 bits of the leaf hash (32 bytes * 8 bits)
    // Every single mutation MUST be rejected (0 false positives)
    let mut total_bit_flips_tested = 0usize;
    let mut total_bit_flips_rejected = 0usize;

    for byte_idx in 0..32 {
        for bit_idx in 0..8 {
            let mut mutated = proof_obj.clone();
            let leaf_arr = mutated["leaf_hash"].as_array_mut().unwrap();
            let orig = leaf_arr[byte_idx].as_u64().unwrap() as u8;
            leaf_arr[byte_idx] = Value::from(orig ^ (1 << bit_idx));

            let mutated_file = fixture
                .root
                .join(format!("proof_mut_{byte_idx}_{bit_idx}.json"));
            fs::write(&mutated_file, serde_json::to_string(&mutated).unwrap()).unwrap();

            let verify_res = fixture.run_success(&[
                "audit",
                "verify",
                "--proof-file",
                mutated_file.to_str().unwrap(),
                "--json",
            ]);

            total_bit_flips_tested += 1;
            if !verify_res["verified"].as_bool().unwrap() {
                total_bit_flips_rejected += 1;
            }
            let _ = fs::remove_file(mutated_file);
        }
    }

    assert_eq!(total_bit_flips_tested, 256);
    assert_eq!(
        total_bit_flips_rejected, 256,
        "Every single bit flip must be rejected by SHA-256 Merkle proof verification! Got {total_bit_flips_rejected}/256"
    );
}

#[test]
fn test_math_mab_large_horizon_sublinear_regret() {
    let fixture = Fixture::new();

    // Test large horizon T=500 and T=1000 to verify sublinear regret growth
    for strategy in ["ucb1", "thompson"] {
        let val_500 = fixture.run_success(&[
            "models",
            "route",
            "--strategy",
            strategy,
            "--rounds",
            "500",
            "--seed",
            "314159",
            "--json",
        ]);
        let r_500 = val_500["regret"]["cumulative_empirical_regret"]
            .as_f64()
            .unwrap();
        let bound_500 = val_500["regret"]["theoretical_minimax_bound"]
            .as_f64()
            .unwrap();
        assert!(val_500["regret"]["bound_satisfied"].as_bool().unwrap());
        assert!(r_500 <= bound_500);

        let val_1000 = fixture.run_success(&[
            "models",
            "route",
            "--strategy",
            strategy,
            "--rounds",
            "1000",
            "--seed",
            "314159",
            "--json",
        ]);
        let r_1000 = val_1000["regret"]["cumulative_empirical_regret"]
            .as_f64()
            .unwrap();
        let bound_1000 = val_1000["regret"]["theoretical_minimax_bound"]
            .as_f64()
            .unwrap();
        assert!(val_1000["regret"]["bound_satisfied"].as_bool().unwrap());
        assert!(r_1000 <= bound_1000);

        // Sublinear average regret invariant: R(T)/T decreases as T increases
        let avg_regret_500 = r_500 / 500.0;
        let avg_regret_1000 = r_1000 / 1000.0;
        assert!(
            avg_regret_1000 <= avg_regret_500 + 0.05,
            "Average regret must be asymptotically non-increasing: avg(500)={avg_regret_500}, avg(1000)={avg_regret_1000}"
        );
    }
}

#[test]
fn test_math_speculative_extreme_latency_slowdown() {
    let fixture = Fixture::new();

    // When draft model is severely slower than target (latency ratio r = 5.0 and r = 10.0),
    // speculative decoding should exhibit clear empirical slowdown (speedup < 0.25x)
    for &ratio in &[5.0, 10.0] {
        let ratio_s = ratio.to_string();
        let val = fixture.run_success(&[
            "runtime",
            "speculative",
            "benchmark",
            "--gamma",
            "4",
            "--alpha",
            "0.7",
            "--ratio",
            &ratio_s,
            "--tokens",
            "100",
            "--seed",
            "9999",
            "--json",
        ]);

        let th_speedup = val["theoretical_speedup"].as_f64().unwrap();
        let emp_speedup = val["empirical_speedup"].as_f64().unwrap();

        // Speedup must be well below 1.0 (actual slowdown)
        assert!(th_speedup < 0.25);
        assert!(emp_speedup < 0.25);
        // Empirical and theoretical speedup should be close
        assert!((th_speedup - emp_speedup).abs() < 0.1);
    }
}
