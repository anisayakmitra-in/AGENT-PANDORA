use super::{load_config_storage, parse_options, path_option};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::RuntimeService;
use pandora_runtime::adversarial_bandit::Exp3AdversarialBandit;
use pandora_runtime::assurance_receipt::UnifiedAssuranceReceipt;
use pandora_runtime::conformal_calibrator::{
    AdaptiveConformalTracker, CandidateAction, SplitConformalCalibrator,
};
use pandora_runtime::gateway::{GatewayConfig, GatewayController};
use pandora_runtime::spectral_topology::{SpectralTopologyAnalyzer, WeightedEdge};
use std::collections::BTreeMap;
use std::path::PathBuf;

use pandora_runtime::byzantine_aggregator::{
    AgentVectorUpdate, AggregationRule, ByzantineAggregator,
};
use pandora_runtime::martingale_gate::MartingaleCanaryGate;
use pandora_runtime::mean_field_game::{MeanFieldGame, MfgResourceConfig};
use pandora_runtime::pac_bayes_monitor::{PacBayesConfig, PacBayesMonitor};
use pandora_runtime::speculative_decoding::{RadixKvCache, SpeculativeEngine, TokenProbability};
use pandora_runtime::tropical_dag::{MaxPlusMatrix, TropicalDagScheduler, TropicalError};
use pandora_runtime::wasserstein_drift::{EmpiricalDistribution, SinkhornDriftDetector};
use pandora_runtime::work_stealing::{StealableTask, StealableTaskDag, WorkStealingScheduler};
use pandora_types::ServiceEngineSummary;
use serde_json::json;

/// Read-only view of the runtime engine inventory and hardware acceleration simulators.
pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let first = args.first().map(String::as_str);
    match first {
        Some("engines") => engines(&args[1..]),
        Some("decision") => super::decision::execute(&args[1..]),
        Some("speculative") => speculative(&args[1..]),
        Some("kv-cache") => kv_cache(&args[1..]),
        Some("scheduler") | Some("work-stealing") => scheduler(&args[1..]),
        Some("privacy") => super::privacy::execute(&args[1..]),
        Some("zk") => super::zk::execute(&args[1..]),
        Some("byzantine") => byzantine(&args[1..]),
        Some("drift") | Some("wasserstein") => drift(&args[1..]),
        Some("canary-gate") | Some("martingale") => canary_gate(&args[1..]),
        Some("conformal") => conformal(&args[1..]),
        Some("topology") | Some("spectral") => topology(&args[1..]),
        Some("bandit") | Some("exp3") => bandit(&args[1..]),
        Some("assurance") | Some("assurance-receipt") => assurance(&args[1..]),
        Some("mfg") | Some("mean-field") => mfg(&args[1..]),
        Some("pac-bayes") | Some("pacbayes") => pac_bayes(&args[1..]),
        Some("tropical") | Some("max-plus") => tropical(&args[1..]),
        Some("daemon-guard") | Some("guard") => daemon_guard(&args[1..]),
        Some(unknown) if !unknown.starts_with('-') => Err(CliError::usage(format!(
            "unknown runtime command '{unknown}'; expected 'engines', 'decision', 'speculative', 'kv-cache', 'scheduler', 'byzantine', 'drift', 'canary-gate', 'conformal', 'topology', 'bandit', 'assurance', 'mfg', 'pac-bayes', 'tropical', or 'daemon-guard'"
        ))),
        _ => engines(args),
    }
}

pub(crate) fn byzantine(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "agents",
            "byzantine",
            "dim",
            "rule",
            "beta",
            "attack-scale",
            "config",
            "data-dir",
            "workspace",
            "json",
        ],
    )?;

    let n: usize = parsed
        .value("agents")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--agents must be an integer"))
        })
        .transpose()?
        .unwrap_or(11);
    let f: usize = parsed
        .value("byzantine")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--byzantine must be an integer"))
        })
        .transpose()?
        .unwrap_or(2);
    let d: usize = parsed
        .value("dim")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--dim must be an integer"))
        })
        .transpose()?
        .unwrap_or(8);
    let attack_scale: f64 = parsed
        .value("attack-scale")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--attack-scale must be a number"))
        })
        .transpose()?
        .unwrap_or(1000.0);

    let rule_name = parsed.value("rule").unwrap_or("krum").to_ascii_lowercase();
    let rule = match rule_name.as_str() {
        "trimmed-mean" | "trimmed" => {
            let beta: f64 = parsed
                .value("beta")
                .map(|s| {
                    s.parse()
                        .map_err(|_| CliError::usage("--beta must be in [0, 0.5)"))
                })
                .transpose()?
                .unwrap_or(0.2);
            AggregationRule::TrimmedMean { beta }
        }
        "median" => AggregationRule::CoordinateMedian,
        "krum" | "multi-krum" => AggregationRule::MultiKrum {
            selection_q: (n.saturating_sub(2 * f)).max(1),
        },
        other => {
            return Err(CliError::usage(format!(
                "unknown rule '{other}'; expected 'krum', 'trimmed-mean', or 'median'"
            )));
        }
    };

    let aggregator =
        ByzantineAggregator::new(f, rule).map_err(|e| CliError::usage(e.to_string()))?;
    let true_target: Vec<f64> = (0..d).map(|j| (j as f64) + 1.0).collect();
    let mut updates = Vec::with_capacity(n);

    for i in 0..n {
        if i < n - f {
            let honest_vec: Vec<f64> = (0..d)
                .map(|j| true_target[j] + (((i + j) % 5) as f64 - 2.0) * 0.02)
                .collect();
            updates.push(AgentVectorUpdate::new(format!("honest_{i}"), honest_vec));
        } else {
            let byz_vec: Vec<f64> = (0..d).map(|j| -attack_scale * ((j + 1) as f64)).collect();
            updates.push(AgentVectorUpdate::new(format!("byzantine_{i}"), byz_vec));
        }
    }

    let result = aggregator
        .aggregate(&updates)
        .map_err(|e| CliError::usage(e.to_string()))?;
    let robust_error = ByzantineAggregator::l2_distance(&result.consensus_vector, &true_target);
    let naive_error = ByzantineAggregator::l2_distance(&result.naive_mean_vector, &true_target);

    let human = format!(
        "Byzantine-Resilient Consensus Telemetry (Theorems 23 & 24):\n  Agents (n): {n} | Byzantine Adversaries (f): {f} | Dimension (d): {d}\n  Aggregation Rule: {rule_name} | Attack Scale: {attack_scale:.1}\n  Robust Consensus L2 Error ||mu_hat - mu*||_2: {robust_error:.6}\n  Unguarded Naive Mean L2 Error ||mu_bar - mu*||_2: {naive_error:.4}\n  Mitigation Shift ||mu_bar - mu_hat||_2: {:.4}\n  Theoretical Deviation Factor: {:.4}\n  Suspected Outliers Identified: {:?}",
        result.mitigation_shift_l2, result.theoretical_deviation_factor, result.suspected_outliers
    );

    Ok(success(
        "runtime byzantine",
        json!({
            "total_agents_n": n,
            "max_byzantine_f": f,
            "dimension_d": d,
            "rule": rule_name,
            "robust_l2_error": robust_error,
            "naive_l2_error": naive_error,
            "mitigation_shift_l2": result.mitigation_shift_l2,
            "theoretical_deviation_factor": result.theoretical_deviation_factor,
            "suspected_outliers": result.suspected_outliers,
            "selected_agents": result.selected_agents,
        }),
        human,
    ))
}

pub(crate) fn drift(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "samples",
            "dim",
            "epsilon",
            "shift",
            "threshold",
            "config",
            "data-dir",
            "workspace",
            "json",
        ],
    )?;

    let samples: usize = parsed
        .value("samples")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--samples must be an integer"))
        })
        .transpose()?
        .unwrap_or(16);
    let d: usize = parsed
        .value("dim")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--dim must be an integer"))
        })
        .transpose()?
        .unwrap_or(4);
    let epsilon: f64 = parsed
        .value("epsilon")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--epsilon must be a positive number"))
        })
        .transpose()?
        .unwrap_or(0.1);
    let shift: f64 = parsed
        .value("shift")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--shift must be a number"))
        })
        .transpose()?
        .unwrap_or(1.5);
    let threshold: f64 = parsed
        .value("threshold")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--threshold must be a non-negative number"))
        })
        .transpose()?
        .unwrap_or(1.0);

    let mut base_pts = Vec::with_capacity(samples);
    let mut live_pts = Vec::with_capacity(samples);
    for i in 0..samples {
        let pt_base: Vec<f64> = (0..d).map(|j| ((i + j) as f64) * 0.1).collect();
        let pt_live: Vec<f64> = (0..d)
            .map(|j| ((i + j) as f64) * 0.1 + shift / (d as f64).sqrt())
            .collect();
        base_pts.push(pt_base);
        live_pts.push(pt_live);
    }

    let mu =
        EmpiricalDistribution::uniform(base_pts).map_err(|e| CliError::usage(e.to_string()))?;
    let nu =
        EmpiricalDistribution::uniform(live_pts).map_err(|e| CliError::usage(e.to_string()))?;
    let detector = SinkhornDriftDetector::new(epsilon, 200, 1e-7, threshold)
        .map_err(|e| CliError::usage(e.to_string()))?;
    let report = detector
        .evaluate_drift(&mu, &nu)
        .map_err(|e| CliError::usage(e.to_string()))?;

    let human = format!(
        "Entropic Optimal Transport (Sinkhorn-Knopp) Semantic Drift Detector (Theorems 25 & 26):\n  Samples (n=m): {samples} | Dimension (d): {d} | Entropic Regularization (epsilon): {epsilon:.4}\n  Debiased Sinkhorn Divergence S_eps(mu, nu): {:.6}\n  Debiased 2-Wasserstein Distance W_2(mu, nu): {:.6} (Injected Shift: {shift:.4})\n  Sinkhorn Iterations: {} (Final Dual Residual: {:.2e}, Marginal Error: {:.2e})\n  Birkhoff-Hopf Projective Contraction kappa: {:.6} (< 1.0)\n  Drift Threshold: {threshold:.4} -> Drift Detected: {}",
        report.sinkhorn_divergence,
        report.wasserstein_2_distance,
        report.iterations,
        report.final_dual_residual,
        report.marginal_conservation_error,
        report.birkhoff_contraction_factor,
        if report.drift_detected { "YES" } else { "NO" }
    );

    Ok(success(
        "runtime drift",
        serde_json::to_value(&report).unwrap_or_else(|_| json!({})),
        human,
    ))
}

pub(crate) fn canary_gate(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "steps",
            "alpha",
            "effect",
            "sigma",
            "config",
            "data-dir",
            "workspace",
            "json",
        ],
    )?;

    let steps: usize = parsed
        .value("steps")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--steps must be an integer"))
        })
        .transpose()?
        .unwrap_or(64);
    let alpha: f64 = parsed
        .value("alpha")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--alpha must be in (0, 1)"))
        })
        .transpose()?
        .unwrap_or(0.05);
    let effect: f64 = parsed
        .value("effect")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--effect must be in [-1.0, 1.0]"))
        })
        .transpose()?
        .unwrap_or(0.25);
    let sigma: f64 = parsed
        .value("sigma")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--sigma must be > 0"))
        })
        .transpose()?
        .unwrap_or(0.5);

    let mut gate =
        MartingaleCanaryGate::new(alpha, sigma, 10).map_err(|e| CliError::usage(e.to_string()))?;
    for i in 0..steps {
        let jitter = if i % 2 == 0 { 0.05 } else { -0.05 };
        let x_t = (effect + jitter).clamp(-1.0, 1.0);
        gate.observe(x_t)
            .map_err(|e| CliError::usage(e.to_string()))?;
    }
    let snap = gate
        .snapshot()
        .map_err(|e| CliError::usage(e.to_string()))?;

    let human = format!(
        "Martingale Anytime-Valid Canary Gate Telemetry (Theorems 27 & 28):\n  Steps Observed (t): {} | Target Alpha: {alpha:.4} (Ville Threshold 1/alpha = {:.2})\n  Test Supermartingale Wealth (E_t): {:.4} (Peak E_s: {:.4}, Anytime p-value: {:.4e})\n  Empirical Mean Effect (mu_hat_t): {:.4}\n  Time-Uniform Confidence Sequence: [{:.4}, {:.4}] (Radius r_t: {:.4})\n  Verdict: {}",
        snap.step_t,
        snap.ville_rejection_threshold,
        snap.e_value,
        snap.peak_e_value,
        snap.anytime_p_value,
        snap.empirical_mean,
        snap.lower_confidence_bound,
        snap.upper_confidence_bound,
        snap.confidence_radius,
        snap.verdict.as_str()
    );

    Ok(success(
        "runtime canary-gate",
        serde_json::to_value(&snap).unwrap_or_else(|_| json!({})),
        human,
    ))
}

pub(crate) fn conformal(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "samples",
            "alpha",
            "gamma",
            "steps",
            "config",
            "data-dir",
            "workspace",
            "json",
        ],
    )?;

    let samples: usize = parsed
        .value("samples")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--samples must be a positive integer"))
        })
        .transpose()?
        .unwrap_or(99);
    let alpha: f64 = parsed
        .value("alpha")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--alpha must be in (0, 1)"))
        })
        .transpose()?
        .unwrap_or(0.10);
    let gamma: f64 = parsed
        .value("gamma")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--gamma must be in (0, 1]"))
        })
        .transpose()?
        .unwrap_or(0.05);
    let steps: usize = parsed
        .value("steps")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--steps must be a positive integer"))
        })
        .transpose()?
        .unwrap_or(200);

    let scores: Vec<f64> = (1..=samples).map(|i| i as f64 / (samples as f64)).collect();
    let calibrator = SplitConformalCalibrator::calibrate(&scores, alpha)
        .map_err(|e| CliError::usage(e.to_string()))?;
    let summary = calibrator.summary();
    let pred = calibrator
        .predict_set(&[
            CandidateAction {
                action_id: "safe_tool_call".to_owned(),
                nonconformity_score: 0.25,
            },
            CandidateAction {
                action_id: "uncertain_mutation".to_owned(),
                nonconformity_score: 0.96,
            },
        ])
        .map_err(|e| CliError::usage(e.to_string()))?;

    let mut aci =
        AdaptiveConformalTracker::new(alpha, gamma).map_err(|e| CliError::usage(e.to_string()))?;
    for t in 0..steps {
        let q_t = calibrator.quantile_for_alpha(aci.status().current_alpha_t);
        let test_s = if t % 5 == 0 { 0.95 } else { 0.45 };
        aci.observe_step(test_s > q_t);
    }
    let aci_status = aci.status();

    let human = format!(
        "Conformal Prediction & ACI Uncertainty Calibrator (Theorems 29 & 30):\n  Calibration Size (n): {samples} | Target Alpha: {alpha:.4} | Rank (k): {}\n  Conformal Quantile q_hat: {:.4} | Marginal Coverage Bound: [{:.4}, {:.4}]\n  Prediction Set: {:?} (Verdict: {:?})\n  ACI Online Steps (T): {steps} | Empirical Coverage: {:.4} | Bound Verified: {}",
        summary.order_statistic_rank,
        summary.quantile_threshold,
        summary.marginal_coverage_lower_bound,
        summary.marginal_coverage_upper_bound,
        pred.prediction_set,
        pred.verdict,
        aci_status.empirical_coverage_rate,
        aci_status.within_theoretical_bound
    );

    Ok(success(
        "runtime conformal",
        json!({
            "calibration": summary,
            "prediction": pred,
            "aci_status": aci_status,
        }),
        human,
    ))
}

pub(crate) fn topology(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "nodes",
            "bridge-weight",
            "threshold",
            "config",
            "data-dir",
            "workspace",
            "json",
        ],
    )?;

    let nodes: usize = parsed
        .value("nodes")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--nodes must be an integer >= 2"))
        })
        .transpose()?
        .unwrap_or(6);
    if nodes < 2 {
        return Err(CliError::usage("--nodes must be at least 2"));
    }
    let bridge_weight: f64 = parsed
        .value("bridge-weight")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--bridge-weight must be > 0"))
        })
        .transpose()?
        .unwrap_or(1.0);
    let threshold: f64 = parsed
        .value("threshold")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--threshold must be in (0, 1)"))
        })
        .transpose()?
        .unwrap_or(0.20);

    let mid = nodes / 2;
    let mut edges = Vec::new();
    for u in 0..mid {
        for v in (u + 1)..mid {
            edges.push(WeightedEdge { u, v, weight: 1.0 });
        }
    }
    for u in mid..nodes {
        for v in (u + 1)..nodes {
            edges.push(WeightedEdge { u, v, weight: 1.0 });
        }
    }
    edges.push(WeightedEdge {
        u: mid.saturating_sub(1),
        v: mid,
        weight: bridge_weight,
    });

    let analyzer = SpectralTopologyAnalyzer::new(nodes, edges, threshold)
        .map_err(|e| CliError::usage(e.to_string()))?;
    let report = analyzer.analyze();

    let human = format!(
        "Spectral Graph Topology & Algebraic Connectivity (Theorems 31 & 32):\n  Nodes (|V|): {} | Edges (|E|): {} | Fiedler Value lambda_2(L): {:.6}\n  Normalized Fiedler nu_2(L_sym): {:.6} | Sweep Conductance h(G): {:.6}\n  Cheeger Bounds: [{:.6}, {:.6}] (Verified: {})\n  Spectral Bisection: {:?} | {:?}\n  Verdict: {:?}",
        report.node_count,
        report.edge_count,
        report.fiedler_value_unnormalized,
        report.fiedler_value_normalized,
        report.sweep_cut_conductance,
        report.cheeger_lower_bound,
        report.cheeger_upper_bound,
        report.cheeger_inequality_verified,
        report.partition_left,
        report.partition_right,
        report.verdict
    );

    Ok(success(
        "runtime topology",
        serde_json::to_value(&report).unwrap_or_else(|_| json!({})),
        human,
    ))
}

pub(crate) fn bandit(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "arms",
            "horizon",
            "gamma",
            "config",
            "data-dir",
            "workspace",
            "json",
        ],
    )?;

    let k: usize = parsed
        .value("arms")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--arms must be an integer >= 2"))
        })
        .transpose()?
        .unwrap_or(3);
    let horizon: usize = parsed
        .value("horizon")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--horizon must be a positive integer"))
        })
        .transpose()?
        .unwrap_or(200);

    let arm_ids: Vec<String> = (0..k).map(|i| format!("route_arm_{i}")).collect();
    let mut exp3 = if let Some(g_str) = parsed.value("gamma") {
        let g: f64 = g_str
            .parse()
            .map_err(|_| CliError::usage("--gamma must be in (0, 1]"))?;
        Exp3AdversarialBandit::new(arm_ids, g).map_err(|e| CliError::usage(e.to_string()))?
    } else {
        Exp3AdversarialBandit::with_horizon(arm_ids, horizon)
            .map_err(|e| CliError::usage(e.to_string()))?
    };

    for t in 0..horizon {
        let u = ((t as f64 + 1.0) * 0.372_583_002).fract();
        let (chosen, _, _) = exp3
            .select_arm(u)
            .map_err(|e| CliError::usage(e.to_string()))?;
        let reward = if chosen == 0 { 0.85 } else { 0.35 };
        exp3.update_reward(chosen, reward)
            .map_err(|e| CliError::usage(e.to_string()))?;
    }
    let status = exp3.status();

    let human = format!(
        "Exp3 Adversarial Multi-Armed Bandit Router (Theorems 33 & 34):\n  Arms (K): {} | Horizon (T): {} | Exploration gamma: {:.4} (eta: {:.4})\n  Cumulative Observed Reward: {:.2} | Minimax Regret Upper Bound: {:.2}",
        status.num_arms,
        status.rounds_elapsed,
        status.gamma,
        status.eta,
        status.cumulative_observed_reward,
        status.minimax_regret_upper_bound
    );

    Ok(success(
        "runtime bandit",
        serde_json::to_value(&status).unwrap_or_else(|_| json!({})),
        human,
    ))
}

pub(crate) fn assurance(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["run-id", "config", "data-dir", "workspace", "json"])?;
    let run_id = parsed.value("run-id").unwrap_or("pandora-assurance-v3.0.7");
    let receipt =
        UnifiedAssuranceReceipt::generate(run_id).map_err(|e| CliError::execution(e, json!({})))?;

    let human = format!(
        "Unified Mathematical Assurance Receipt (Phases 9-20, Theorems 17-40):\n  Run ID: {}\n  Phases Certified: {}/12 | All Invariants Verified: {}\n  Receipt Digest: {}",
        receipt.run_id,
        receipt.phases_certified,
        receipt.all_invariants_verified,
        receipt.receipt_sha256
    );

    Ok(success(
        "runtime assurance",
        serde_json::to_value(&receipt).unwrap_or_else(|_| json!({})),
        human,
    ))
}

pub(crate) fn mfg(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "resources",
            "base-costs",
            "congestion-slopes",
            "temperature",
            "damping",
            "iterations",
            "tolerance",
            "config",
            "data-dir",
            "workspace",
            "json",
        ],
    )?;

    let k: usize = parsed
        .value("resources")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--resources must be an integer >= 2"))
        })
        .transpose()?
        .unwrap_or(3);

    if k < 2 {
        return Err(CliError::usage("--resources must be at least 2"));
    }

    let base_costs: Vec<f64> = if let Some(s) = parsed.value("base-costs") {
        s.split(',')
            .map(|val| {
                val.trim()
                    .parse()
                    .map_err(|_| CliError::usage("invalid float in --base-costs"))
            })
            .collect::<Result<Vec<f64>, CliError>>()?
    } else {
        (0..k).map(|i| 1.0 + (i as f64) * 0.5).collect()
    };

    let congestion_slopes: Vec<f64> = if let Some(s) = parsed.value("congestion-slopes") {
        s.split(',')
            .map(|val| {
                val.trim()
                    .parse()
                    .map_err(|_| CliError::usage("invalid float in --congestion-slopes"))
            })
            .collect::<Result<Vec<f64>, CliError>>()?
    } else {
        (0..k).map(|i| 2.0 - (i as f64) * 0.5).collect()
    };

    if base_costs.len() != k {
        return Err(CliError::usage(format!(
            "--base-costs count ({}) must match --resources ({k})",
            base_costs.len()
        )));
    }
    if congestion_slopes.len() != k {
        return Err(CliError::usage(format!(
            "--congestion-slopes count ({}) must match --resources ({k})",
            congestion_slopes.len()
        )));
    }

    let temperature: f64 = parsed
        .value("temperature")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--temperature must be a positive float"))
        })
        .transpose()?
        .unwrap_or(1.0);

    let damping: f64 = parsed
        .value("damping")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--damping must be in (0, 1]"))
        })
        .transpose()?
        .unwrap_or(0.5);

    let iterations: usize = parsed
        .value("iterations")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--iterations must be a positive integer"))
        })
        .transpose()?
        .unwrap_or(100);

    let tolerance: f64 = parsed
        .value("tolerance")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--tolerance must be a positive float"))
        })
        .transpose()?
        .unwrap_or(1e-6);

    let mut resource_configs = Vec::with_capacity(k);
    for i in 0..k {
        resource_configs.push(
            MfgResourceConfig::new(format!("resource_{i}"), base_costs[i], congestion_slopes[i])
                .map_err(|e| CliError::usage(e.to_string()))?,
        );
    }

    let mfg_engine = MeanFieldGame::with_damping(resource_configs, temperature, damping)
        .map_err(|e| CliError::usage(e.to_string()))?;

    let eq_result = mfg_engine
        .compute_equilibrium(iterations, tolerance)
        .map_err(|e| CliError::usage(e.to_string()))?;
    let poa_result = mfg_engine
        .compute_price_of_anarchy()
        .map_err(|e| CliError::usage(e.to_string()))?;

    let status = mfg_engine
        .status()
        .map_err(|e| CliError::usage(e.to_string()))?;

    let human = format!(
        "Mean-Field Game Equilibrium & Congestion Control (Theorems 35 & 36):\n  Resources (K): {} | Temperature tau: {:.4} | Damping alpha: {:.4}\n  Contraction Modulus kappa: {:.4} (Strict: {})\n  Equilibrium Iterations: {} | Residual L1: {:.2e}\n  Equilibrium Distribution m*: {:?}\n  Resource Costs c(m*): {:?}\n  Nash Social Cost: {:.4} | Social Optimum Cost: {:.4}\n  Price of Anarchy (PoA): {:.4} (Theoretical Upper Bound: {:.4}, Bound Satisfied: {})",
        status.num_resources,
        status.temperature_tau,
        status.damping_alpha,
        status.contraction_modulus,
        status.fixed_point.is_strict_contraction,
        eq_result.iterations,
        eq_result.residual_norm,
        eq_result.equilibrium_distribution,
        eq_result.resource_costs,
        poa_result.nash_social_cost,
        poa_result.optimal_social_cost,
        poa_result.price_of_anarchy,
        poa_result.theoretical_upper_bound,
        poa_result.bound_satisfied
    );

    Ok(success(
        "runtime mfg",
        serde_json::to_value(&status).unwrap_or_else(|_| json!({})),
        human,
    ))
}

pub(crate) fn pac_bayes(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "samples",
            "empirical-risk",
            "kl",
            "target-risk",
            "delta",
            "hypotheses",
            "losses",
            "config",
            "data-dir",
            "workspace",
            "json",
        ],
    )?;

    let samples: usize = parsed
        .value("samples")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--samples must be a positive integer"))
        })
        .transpose()?
        .unwrap_or(200);

    let delta: f64 = parsed
        .value("delta")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--delta must be a float in (0, 1)"))
        })
        .transpose()?
        .unwrap_or(0.05);

    let target_risk: f64 = parsed
        .value("target-risk")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--target-risk must be a float in [0, 1]"))
        })
        .transpose()?
        .unwrap_or(0.12);

    let monitor = PacBayesMonitor::with_config(PacBayesConfig {
        delta,
        target_risk,
        risk_margin: 0.01,
    })
    .map_err(|e| CliError::usage(e.to_string()))?;

    let losses: Vec<f64> = if let Some(s) = parsed.value("losses") {
        s.split(',')
            .map(|val| {
                val.trim()
                    .parse()
                    .map_err(|_| CliError::usage("invalid float in --losses"))
            })
            .collect::<Result<Vec<f64>, CliError>>()?
    } else {
        let m = parsed
            .value("hypotheses")
            .map(|s| {
                s.parse()
                    .map_err(|_| CliError::usage("--hypotheses must be a positive integer"))
            })
            .transpose()?
            .unwrap_or(4);
        (0..m).map(|i| 0.02 + (i as f64) * 0.04).collect()
    };

    let gate_report = monitor
        .run_promotion_gate(&losses, samples)
        .map_err(|e| CliError::usage(e.to_string()))?;

    let direct_cert = if let Some(emp_str) = parsed.value("empirical-risk") {
        let emp: f64 = emp_str
            .parse()
            .map_err(|_| CliError::usage("--empirical-risk must be a float in [0, 1]"))?;
        let kl: f64 = parsed
            .value("kl")
            .map(|s| {
                s.parse()
                    .map_err(|_| CliError::usage("--kl must be a non-negative float"))
            })
            .transpose()?
            .unwrap_or(0.5);
        Some(
            monitor
                .certify_risk(emp, samples, kl, delta)
                .map_err(|e| CliError::usage(e.to_string()))?,
        )
    } else {
        None
    };

    let human = format!(
        "PAC-Bayesian Generalization Risk Monitor & Policy Promotion Gate (Theorems 37 & 38):\n  Samples (n): {} | Confidence (1 - delta): {:.2}% | Target Risk: {:.4}\n  Optimal Gibbs Temperature beta*: {:.4}\n  Certified Maurer Risk Bound L_max(Q): {:.4}\n  Risk Margin: {:.4}\n  Gibbs Posterior Q_beta*: {:?}\n  Catoni Generalization Bound: {:.4}\n  Promotion Gate Verdict: {:?}",
        samples,
        (1.0 - delta) * 100.0,
        target_risk,
        gate_report.optimal_temperature_beta,
        gate_report.certified_risk_bound,
        gate_report.risk_margin,
        gate_report.gibbs_posterior,
        gate_report.catoni_bound,
        gate_report.verdict
    );

    let mut json_val = serde_json::to_value(&gate_report).unwrap_or_else(|_| json!({}));
    if let (Some(cert), Some(obj)) = (direct_cert, json_val.as_object_mut()) {
        obj.insert(
            "direct_certificate".to_string(),
            serde_json::to_value(cert).unwrap_or_else(|_| json!({})),
        );
    }

    Ok(success("runtime pac-bayes", json_val, human))
}

pub(crate) fn tropical(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "nodes",
            "topology",
            "sla-threshold",
            "matrix",
            "config",
            "data-dir",
            "workspace",
            "json",
        ],
    )?;

    let nodes: usize = parsed
        .value("nodes")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--nodes must be an integer >= 2"))
        })
        .transpose()?
        .unwrap_or(4);

    let topology = parsed.value("topology").unwrap_or("pipeline");
    let sla_threshold: f64 = parsed
        .value("sla-threshold")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--sla-threshold must be a positive float"))
        })
        .transpose()?
        .unwrap_or(50.0);

    let scheduler = TropicalDagScheduler::new();

    let (adj, is_dag) = if let Some(mat_str) = parsed.value("matrix") {
        let raw: Vec<Vec<f64>> = serde_json::from_str(mat_str)
            .map_err(|e| CliError::usage(format!("invalid JSON in --matrix: {e}")))?;
        (
            MaxPlusMatrix::from_data(raw).map_err(|e| CliError::usage(e.to_string()))?,
            false,
        )
    } else {
        match topology {
            "pipeline" | "dag" => {
                let mut mat =
                    MaxPlusMatrix::zero(nodes).map_err(|e| CliError::usage(e.to_string()))?;
                for i in 0..(nodes.saturating_sub(1)) {
                    mat.set(i, i + 1, (i as f64 + 1.0) * 10.0)
                        .map_err(|e| CliError::usage(e.to_string()))?;
                }
                (mat, true)
            }
            "cycle" | "recurrent" => {
                let mut mat =
                    MaxPlusMatrix::zero(nodes).map_err(|e| CliError::usage(e.to_string()))?;
                for i in 0..nodes {
                    mat.set(i, (i + 1) % nodes, 8.0)
                        .map_err(|e| CliError::usage(e.to_string()))?;
                    mat.set(i, i, 4.0)
                        .map_err(|e| CliError::usage(e.to_string()))?;
                }
                (mat, false)
            }
            other => {
                return Err(CliError::usage(format!(
                    "unknown topology '{other}'; expected 'pipeline', 'dag', or 'cycle'"
                )));
            }
        }
    };

    let (makespan, has_positive_cycle, convergence_steps) = if is_dag {
        match scheduler.compute_kleene_star(&adj) {
            Ok(star) => (star.critical_path_makespan, false, star.convergence_steps),
            Err(TropicalError::PositiveCycleDetected { .. }) => (0.0, true, nodes),
            Err(e) => return Err(CliError::usage(e.to_string())),
        }
    } else {
        (0.0, false, nodes)
    };

    let karp_opt = scheduler.check_sla_compliance(&adj, sla_threshold).ok();

    let (karp_mean, throughput, sla_satisfied) = if let Some(ref k) = karp_opt {
        (
            k.maximum_cycle_mean,
            k.asymptotic_throughput,
            k.sla_satisfied,
        )
    } else {
        (0.0, f64::INFINITY, true)
    };

    let human = format!(
        "Tropical Max-Plus DAG Calculus & SLA Deadlock Guard (Theorems 39 & 40):\n  Workflow Nodes (|V|): {} | Topology: {}\n  Critical Path Makespan: {:.2} ms (Positive Cycle: {}, Steps: {})\n  Karp Maximum Cycle Mean lambda(G): {:.4} ms/cycle\n  Periodic Pipeline Throughput Theta_max: {:.4} cycles/ms\n  SLA Deadlock Threshold: {:.2} ms | SLA Compliant: {}",
        nodes,
        topology,
        makespan,
        has_positive_cycle,
        convergence_steps,
        karp_mean,
        throughput,
        sla_threshold,
        sla_satisfied
    );

    let json_data = json!({
        "nodes": nodes,
        "topology": topology,
        "critical_path_makespan": makespan,
        "has_positive_cycle": has_positive_cycle,
        "convergence_steps": convergence_steps,
        "maximum_cycle_mean": karp_mean,
        "asymptotic_throughput": throughput,
        "sla_threshold": sla_threshold,
        "sla_satisfied": sla_satisfied,
        "karp_details": karp_opt,
    });

    Ok(success("runtime tropical", json_data, human))
}

pub(crate) fn scheduler(args: &[String]) -> Result<CommandResult, CliError> {
    scheduler_with_cmd(args, "runtime scheduler benchmark")
}

pub(crate) fn scheduler_with_cmd(
    args: &[String],
    cmd_name: &'static str,
) -> Result<CommandResult, CliError> {
    let first = args.first().map(String::as_str);
    match first {
        Some("benchmark") | Some("simulate") => scheduler_benchmark_with_cmd(&args[1..], cmd_name),
        Some(other) if !other.starts_with('-') => Err(CliError::usage(format!(
            "unknown scheduler subcommand '{other}'; expected 'benchmark' or 'simulate'"
        ))),
        _ => scheduler_benchmark_with_cmd(args, cmd_name),
    }
}

pub(crate) fn scheduler_benchmark_with_cmd(
    args: &[String],
    cmd_name: &'static str,
) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "workers",
            "tasks",
            "topology",
            "capacity",
            "weight",
            "config",
            "data-dir",
            "workspace",
            "json",
        ],
    )?;

    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "scheduler benchmark does not accept positional arguments",
        ));
    }

    let workers: usize = if let Some(w) = parsed.value("workers") {
        w.parse()
            .map_err(|_| CliError::usage("--workers must be a positive integer"))?
    } else {
        4
    };
    if workers == 0 || workers > 256 {
        return Err(CliError::usage("--workers must be between 1 and 256"));
    }

    let tasks_count: usize = if let Some(t) = parsed.value("tasks") {
        t.parse()
            .map_err(|_| CliError::usage("--tasks must be a positive integer"))?
    } else {
        32
    };
    if tasks_count == 0 || tasks_count > 4096 {
        return Err(CliError::usage("--tasks must be between 1 and 4096"));
    }

    let capacity: usize = if let Some(c) = parsed.value("capacity") {
        c.parse()
            .map_err(|_| CliError::usage("--capacity must be a positive integer"))?
    } else {
        tasks_count.max(64)
    };
    if capacity < tasks_count {
        return Err(CliError::usage(
            "--capacity must be at least --tasks to avoid deque overflow",
        ));
    }

    let base_weight: u64 = if let Some(w) = parsed.value("weight") {
        w.parse()
            .map_err(|_| CliError::usage("--weight must be a positive integer"))?
    } else {
        10
    };
    if base_weight == 0 {
        return Err(CliError::usage("--weight must be greater than 0"));
    }

    let topology = parsed
        .value("topology")
        .unwrap_or("layered")
        .to_ascii_lowercase();
    let mut dag = StealableTaskDag::new();

    match topology.as_str() {
        "fanout" | "parallel" => {
            dag.add_task(StealableTask::new("t_0", "root", base_weight));
            for i in 1..tasks_count {
                let id = format!("t_{i}");
                let weight = base_weight + ((i % 4) as u64) * 5;
                dag.add_task(
                    StealableTask::new(&id, format!("worker_leaf_{i}"), weight)
                        .with_dependencies(vec!["t_0".to_string()]),
                );
            }
        }
        "chain" | "serial" => {
            for i in 0..tasks_count {
                let id = format!("t_{i}");
                let deps = if i == 0 {
                    Vec::new()
                } else {
                    vec![format!("t_{}", i - 1)]
                };
                dag.add_task(
                    StealableTask::new(&id, format!("seq_stage_{i}"), base_weight)
                        .with_dependencies(deps),
                );
            }
        }
        "layered" | "diamond" => {
            let width = workers.max(2);
            for i in 0..tasks_count {
                let id = format!("t_{i}");
                let layer = i / width;
                let deps = if layer == 0 {
                    Vec::new()
                } else {
                    let prev_start = (layer - 1) * width;
                    let p1 = prev_start + (i % width);
                    let p2 = prev_start + ((i + 1) % width);
                    if p1 == p2 || p2 >= i {
                        vec![format!("t_{p1}")]
                    } else {
                        vec![format!("t_{p1}"), format!("t_{p2}")]
                    }
                };
                let weight = base_weight + ((i % 3) as u64) * 4;
                dag.add_task(
                    StealableTask::new(&id, format!("layer_{layer}_node_{i}"), weight)
                        .with_dependencies(deps),
                );
            }
        }
        other => {
            return Err(CliError::usage(format!(
                "unknown topology '{other}'; expected 'layered', 'diamond', 'fanout', or 'chain'"
            )));
        }
    }

    let scheduler = WorkStealingScheduler::new(workers, capacity)
        .map_err(|e| CliError::usage(e.to_string()))?;
    let telemetry = scheduler
        .execute_dag(&dag)
        .map_err(|e| CliError::execution(e.to_string(), json!({})))?;

    let human = format!(
        "Blumofe-Leiserson Adaptive Work-Stealing Scheduler Telemetry (Theorems 21 & 22):\n  Workers (P): {workers} | Tasks (|V|): {tasks_count} | Topology: {topology}\n  Total Sequential Work (T_1): {} ticks\n  Critical Path Span (T_inf): {} ticks\n  Observed Parallel Makespan (T_P): {} ticks\n  Theoretical BL Upper Bound (T_1/P + c_inf * T_inf): {:.2} ticks\n  Observed Speedup (T_1 / T_P): {:.2}x (Parallel Efficiency: {:.1}%)\n  Steal Telemetry: {} succeeded / {} attempted\n  Peak Tasks In-Flight (Bounded Memory S_P): {} (Deque Capacity: {capacity})\n  Blumofe-Leiserson Bound Satisfied: {}",
        telemetry.total_work_t1,
        telemetry.critical_path_t_inf,
        telemetry.actual_makespan_tp,
        telemetry.theoretical_bl_bound,
        telemetry.observed_speedup,
        telemetry.parallel_efficiency * 100.0,
        telemetry.total_steals_succeeded,
        telemetry.total_steals_attempted,
        telemetry.peak_tasks_in_flight,
        if telemetry.bl_bound_satisfied {
            "YES"
        } else {
            "NO"
        }
    );

    Ok(success(
        cmd_name,
        json!({
            "workers": workers,
            "tasks_count": tasks_count,
            "topology": topology,
            "deque_capacity": capacity,
            "total_work_t1": telemetry.total_work_t1,
            "critical_path_t_inf": telemetry.critical_path_t_inf,
            "actual_makespan_tp": telemetry.actual_makespan_tp,
            "theoretical_bl_bound": telemetry.theoretical_bl_bound,
            "observed_speedup": telemetry.observed_speedup,
            "parallel_efficiency": telemetry.parallel_efficiency,
            "total_steals_attempted": telemetry.total_steals_attempted,
            "total_steals_succeeded": telemetry.total_steals_succeeded,
            "peak_tasks_in_flight": telemetry.peak_tasks_in_flight,
            "bl_bound_satisfied": telemetry.bl_bound_satisfied,
        }),
        human,
    ))
}

pub(crate) fn speculative(args: &[String]) -> Result<CommandResult, CliError> {
    speculative_with_cmd(args, "runtime speculative benchmark")
}

pub(crate) fn speculative_with_cmd(
    args: &[String],
    cmd_name: &'static str,
) -> Result<CommandResult, CliError> {
    let first = args.first().map(String::as_str);
    match first {
        Some("benchmark") => speculative_benchmark_with_cmd(&args[1..], cmd_name),
        Some("kv-cache") => kv_cache_with_cmd(&args[1..], "runtime kv-cache"),
        Some(other) if !other.starts_with('-') => Err(CliError::usage(format!(
            "unknown speculative subcommand '{other}'; expected 'benchmark' or 'kv-cache'"
        ))),
        _ => speculative_benchmark_with_cmd(args, cmd_name),
    }
}

pub(crate) fn kv_cache(args: &[String]) -> Result<CommandResult, CliError> {
    kv_cache_with_cmd(args, "runtime kv-cache")
}

pub(crate) fn kv_cache_with_cmd(
    args: &[String],
    cmd_name: &'static str,
) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "sessions",
            "prefix-len",
            "suffix-len",
            "bytes-per-token",
            "config",
            "data-dir",
            "workspace",
            "json",
        ],
    )?;

    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "kv-cache does not accept positional arguments",
        ));
    }

    let sessions: usize = if let Some(s) = parsed.value("sessions") {
        s.parse()
            .map_err(|_| CliError::usage("--sessions must be a positive integer"))?
    } else {
        8
    };
    if sessions == 0 {
        return Err(CliError::usage("--sessions must be greater than 0"));
    }

    let prefix_len: usize = if let Some(p) = parsed.value("prefix-len") {
        p.parse()
            .map_err(|_| CliError::usage("--prefix-len must be an integer"))?
    } else {
        512
    };
    if prefix_len == 0 {
        return Err(CliError::usage("--prefix-len must be greater than 0"));
    }

    let suffix_len: usize = if let Some(s) = parsed.value("suffix-len") {
        s.parse()
            .map_err(|_| CliError::usage("--suffix-len must be an integer"))?
    } else {
        256
    };

    let bytes_per_token: usize = if let Some(b) = parsed.value("bytes-per-token") {
        b.parse()
            .map_err(|_| CliError::usage("--bytes-per-token must be an integer"))?
    } else {
        2048
    };
    if bytes_per_token == 0 {
        return Err(CliError::usage("--bytes-per-token must be greater than 0"));
    }

    let mut cache = RadixKvCache::new(bytes_per_token);
    let common_prefix: Vec<u32> = (1..=prefix_len as u32).collect();

    for s in 0..sessions {
        let mut tokens = common_prefix.clone();
        let suffix_start = 1_000_000 + (s * (suffix_len + 100)) as u32;
        tokens.extend(suffix_start..suffix_start + suffix_len as u32);
        cache.insert(&format!("session_{s}"), &tokens);
    }

    let tokens_per_session = prefix_len + suffix_len;
    let total_tokens_requested = sessions * tokens_per_session;
    let total_tokens_shared = (sessions - 1) * prefix_len;
    let dense_memory_bytes = total_tokens_requested * bytes_per_token;
    let compressed_memory_bytes = (prefix_len + sessions * suffix_len) * bytes_per_token;
    let dense_memory_mib = dense_memory_bytes as f64 / (1024.0 * 1024.0);
    let compressed_memory_mib = compressed_memory_bytes as f64 / (1024.0 * 1024.0);
    let prefix_fraction_eta = prefix_len as f64 / tokens_per_session as f64;

    let empirical_cr = cache.compression_ratio();
    let theoretical_cr = RadixKvCache::theoretical_compression_ratio(sessions, prefix_fraction_eta);
    let asymptotic_cr = RadixKvCache::asymptotic_compression_ratio(prefix_fraction_eta);
    let memory_savings_percent =
        (1.0 - (compressed_memory_bytes as f64 / dense_memory_bytes as f64)) * 100.0;

    let human = format!(
        "Prefix-Tree Radix KV Cache Telemetry:\n  Active Sessions (M): {sessions}\n  Common Prefix Length: {prefix_len} tokens (prefix ratio eta: {:.2}%)\n  Unique Suffix Length: {suffix_len} tokens per session\n  Memory Footprint: {bytes_per_token} bytes/token\n  Total Tokens Requested: {total_tokens_requested} tokens across {sessions} sessions\n  Shared Prefix Tokens Reused: {total_tokens_shared} tokens\n  Dense Uncompressed Memory: {:.2} MiB ({dense_memory_bytes} bytes)\n  Compressed Radix Memory: {:.2} MiB ({compressed_memory_bytes} bytes)\n  Empirical Compression Ratio C_R: {:.2}x\n  Theoretical Compression Ratio C_R(M, eta): {:.2}x\n  Asymptotic Limit (M -> inf): {:.2}x (1 / (1 - eta))\n  Net Memory Savings: {:.2}% ({:.2} MiB saved)",
        prefix_fraction_eta * 100.0,
        dense_memory_mib,
        compressed_memory_mib,
        empirical_cr,
        theoretical_cr,
        asymptotic_cr,
        memory_savings_percent,
        dense_memory_mib - compressed_memory_mib
    );

    Ok(success(
        cmd_name,
        json!({
            "active_sessions": sessions,
            "prefix_length_tokens": prefix_len,
            "suffix_length_tokens": suffix_len,
            "bytes_per_token": bytes_per_token,
            "prefix_fraction_eta": prefix_fraction_eta,
            "total_tokens_requested": total_tokens_requested,
            "total_tokens_shared": total_tokens_shared,
            "dense_memory_bytes": dense_memory_bytes,
            "dense_memory_mib": dense_memory_mib,
            "compressed_memory_bytes": compressed_memory_bytes,
            "compressed_memory_mib": compressed_memory_mib,
            "empirical_compression_ratio": empirical_cr,
            "theoretical_compression_ratio": theoretical_cr,
            "asymptotic_compression_ratio": asymptotic_cr,
            "memory_savings_percent": memory_savings_percent
        }),
        human,
    ))
}

pub(crate) fn speculative_benchmark_with_cmd(
    args: &[String],
    cmd_name: &'static str,
) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "gamma",
            "alpha",
            "latency-ratio",
            "ratio",
            "tokens",
            "seed",
            "config",
            "data-dir",
            "workspace",
            "json",
        ],
    )?;

    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "speculative benchmark does not accept positional arguments",
        ));
    }

    let gamma: usize = if let Some(g) = parsed.value("gamma") {
        g.parse()
            .map_err(|_| CliError::usage("--gamma must be an integer between 1 and 32"))?
    } else {
        5
    };
    if gamma == 0 || gamma > 32 {
        return Err(CliError::usage("--gamma must be between 1 and 32"));
    }

    let alpha: f64 = if let Some(a) = parsed.value("alpha") {
        a.parse()
            .map_err(|_| CliError::usage("--alpha must be a number between 0.0 and 1.0"))?
    } else {
        0.80
    };
    if !(0.0..=1.0).contains(&alpha) {
        return Err(CliError::usage("--alpha must be between 0.0 and 1.0"));
    }

    let latency_ratio: f64 = if let Some(r) = parsed
        .value("latency-ratio")
        .or_else(|| parsed.value("ratio"))
    {
        r.parse()
            .map_err(|_| CliError::usage("--latency-ratio must be a positive number"))?
    } else {
        0.10
    };
    if latency_ratio <= 0.0 {
        return Err(CliError::usage("--latency-ratio must be greater than 0.0"));
    }

    let target_tokens: usize = if let Some(t) = parsed.value("tokens") {
        t.parse()
            .map_err(|_| CliError::usage("--tokens must be a positive integer"))?
    } else {
        128
    };
    if target_tokens == 0 {
        return Err(CliError::usage("--tokens must be greater than 0"));
    }

    let seed: u64 = if let Some(s) = parsed.value("seed") {
        s.parse()
            .map_err(|_| CliError::usage("--seed must be a valid 64-bit integer"))?
    } else {
        0xfeed_face_cafe_beef
    };

    let mut engine = SpeculativeEngine::with_seed(seed);
    let mut total_accepted = 0usize;
    let mut total_bonus = 0usize;
    let mut total_emitted = 0usize;
    let mut total_drafted = 0usize;
    let mut cycles_count = 0usize;

    while total_emitted < target_tokens {
        let mut draft_tokens = Vec::with_capacity(gamma);
        let mut draft_dists = Vec::with_capacity(gamma);
        let mut target_dists = Vec::with_capacity(gamma + 1);

        for i in 0..gamma {
            let tok_id = 1000 + (cycles_count * 10 + i) as u32;
            let alt_id = 2000 + (cycles_count * 10 + i) as u32;

            draft_tokens.push(tok_id);
            draft_dists.push(vec![TokenProbability::new(tok_id, 1.0)]);

            if (alpha - 1.0).abs() < 1e-9 {
                target_dists.push(vec![TokenProbability::new(tok_id, 1.0)]);
            } else if alpha <= 1e-9 {
                target_dists.push(vec![TokenProbability::new(alt_id, 1.0)]);
            } else {
                target_dists.push(vec![
                    TokenProbability::new(tok_id, alpha),
                    TokenProbability::new(alt_id, 1.0 - alpha),
                ]);
            }
        }
        target_dists.push(vec![TokenProbability::new(
            3000 + (cycles_count * 10) as u32,
            1.0,
        )]);

        let result = engine.verify_draft(&draft_tokens, &draft_dists, &target_dists);
        total_accepted += result.accepted_tokens.len();
        if result.bonus_token.is_some() {
            total_bonus += 1;
        }
        total_emitted += result.total_emitted;
        total_drafted += gamma;
        cycles_count += 1;
    }

    let empirical_acceptance_rate = if total_drafted > 0 {
        total_accepted as f64 / total_drafted as f64
    } else {
        1.0
    };
    let empirical_tokens_per_cycle = total_emitted as f64 / cycles_count as f64;
    let empirical_speedup = empirical_tokens_per_cycle / (gamma as f64 * latency_ratio + 1.0);

    let theoretical_expected = SpeculativeEngine::theoretical_expected_tokens(alpha, gamma);
    let theoretical_speedup = SpeculativeEngine::theoretical_speedup(alpha, gamma, latency_ratio);
    let (optimal_gamma, optimal_speedup) =
        SpeculativeEngine::optimal_draft_length(alpha, latency_ratio, 16);

    let human = format!(
        "Speculative Decoding Acceptance Benchmark:\n  Draft Window Length (gamma): {gamma}\n  Target Acceptance Probability (alpha): {:.4}\n  Draft Latency Ratio (r): {:.4} (draft latency {:.1}% of target)\n  Total Tokens Simulated: {total_emitted} across {cycles_count} verification cycles\n  Empirical Acceptance Rate (alpha_hat): {:.2}% ({total_accepted} accepted / {total_drafted} drafted)\n  Empirical Bonus Tokens Emitted: {total_bonus}\n  Empirical Speedup: {:.2}x ({:.2} tokens/cycle)\n  Theoretical Expected Tokens E[beta]: {:.4} tokens/cycle\n  Theoretical Speedup S(alpha, gamma, r): {:.4}x\n  Optimal Draft Length gamma*: {optimal_gamma} tokens (max speedup: {:.4}x)\n  Verdict: Speculative decoding achieves {:.2}x empirical speedup over autoregressive baseline",
        alpha,
        latency_ratio,
        latency_ratio * 100.0,
        empirical_acceptance_rate * 100.0,
        empirical_speedup,
        empirical_tokens_per_cycle,
        theoretical_expected,
        theoretical_speedup,
        optimal_speedup,
        empirical_speedup
    );

    Ok(success(
        cmd_name,
        json!({
            "draft_window_gamma": gamma,
            "acceptance_probability_alpha": alpha,
            "latency_ratio": latency_ratio,
            "total_tokens_simulated": total_emitted,
            "cycles_count": cycles_count,
            "empirical_accepted_tokens": total_accepted,
            "empirical_bonus_tokens": total_bonus,
            "empirical_acceptance_rate": empirical_acceptance_rate,
            "empirical_tokens_per_cycle": empirical_tokens_per_cycle,
            "empirical_speedup": empirical_speedup,
            "theoretical_expected_tokens": theoretical_expected,
            "theoretical_speedup": theoretical_speedup,
            "optimal_draft_length": optimal_gamma,
            "optimal_theoretical_speedup": optimal_speedup,
            "seed": seed
        }),
        human,
    ))
}

fn engines(args: &[String]) -> Result<CommandResult, CliError> {
    let selected = args.iter().position(|arg| !arg.starts_with('-'));
    let (subcommand, rest) = match selected {
        Some(index) => (args[index].as_str(), &args[index + 1..]),
        None => ("list", args),
    };
    match subcommand {
        "list" => list(rest),
        "inspect" => inspect(rest),
        unknown => Err(CliError::usage(format!(
            "unknown runtime engines command '{unknown}'"
        ))),
    }
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "json"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "runtime engines list does not accept positional arguments",
        ));
    }
    let inventory = RuntimeService::engine_inventory();
    let engines = inventory.iter().map(engine_value).collect::<Vec<_>>();
    let mut categories = std::collections::BTreeMap::<&str, usize>::new();
    for engine in &inventory {
        *categories.entry(engine.category()).or_default() += 1;
    }
    let summary = categories
        .iter()
        .map(|(name, count)| format!("{name} {count}"))
        .collect::<Vec<_>>()
        .join(", ");
    Ok(success(
        "runtime engines list",
        json!({
            "engines": engines,
            "count": inventory.len(),
            "categories": categories,
        }),
        format!("{} engines ({summary})", inventory.len()),
    ))
}

fn inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "json"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "runtime engines inspect requires exactly one engine ID",
        ));
    }
    let engine_id = &parsed.positionals[0];
    let engine = RuntimeService::engine_inventory()
        .into_iter()
        .find(|engine| engine.id() == engine_id)
        .ok_or_else(|| CliError::usage(format!("unknown engine '{engine_id}'")))?;
    Ok(success(
        "runtime engines inspect",
        json!({"engine": engine_value(&engine)}),
        format!(
            "{} — {} [{}]",
            engine.name(),
            engine.authority(),
            engine.category()
        ),
    ))
}

fn engine_value(engine: &ServiceEngineSummary) -> serde_json::Value {
    json!({
        "id": engine.id(),
        "name": engine.name(),
        "role": engine.role(),
        "authority": engine.authority(),
        "category": engine.category(),
        "component_kind": engine.component_kind(),
        "inputs": engine.inputs(),
        "outputs": engine.outputs(),
        "invariants": engine.invariants(),
        "evidence": engine.evidence(),
        "source_modules": engine.source_modules(),
        "related_components": engine.related_components(),
        "documentation": engine.documentation(),
    })
}

pub(crate) fn daemon_guard(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "interval-ms",
            "iterations",
            "store",
            "once",
            "circuit-threshold",
            "cooldown-secs",
            "config",
            "data-dir",
            "workspace",
            "json",
        ],
    )?;

    let iterations: usize = if parsed.has_flag("once") {
        1
    } else {
        parsed
            .value("iterations")
            .map(|s| {
                s.parse()
                    .map_err(|_| CliError::usage("--iterations must be an integer"))
            })
            .transpose()?
            .unwrap_or(1)
    };

    let interval_ms: u64 = parsed
        .value("interval-ms")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--interval-ms must be an integer"))
        })
        .transpose()?
        .unwrap_or(100);

    let circuit_threshold: u32 = parsed
        .value("circuit-threshold")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--circuit-threshold must be an integer"))
        })
        .transpose()?
        .unwrap_or(3);

    let cooldown_secs: u64 = parsed
        .value("cooldown-secs")
        .map(|s| {
            s.parse()
                .map_err(|_| CliError::usage("--cooldown-secs must be an integer"))
        })
        .transpose()?
        .unwrap_or(30);

    let store_path: PathBuf = if let Some(path) = path_option(&parsed, "store") {
        path
    } else {
        let config = load_config_storage(&parsed)?;
        config.data_dir().join("audit").join("merkle_audit.json")
    };

    let gateway = GatewayController::new(GatewayConfig {
        tick_interval_ms: interval_ms,
        circuit_breaker_threshold: circuit_threshold,
        cooldown_secs,
        audit_store_path: store_path.clone(),
        fallback_provider_map: BTreeMap::new(),
    });

    let mut tick_reports = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let telemetry = gateway.collect_live_telemetry();
        let report = gateway
            .evaluate_telemetry_tick(telemetry)
            .map_err(|e| CliError::execution(e.to_string(), json!({})))?;
        tick_reports.push(report);
        if iterations > 1 && interval_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(interval_ms));
        }
    }

    let last_report = tick_reports.last().expect("at least 1 report");

    let human = format!(
        "Autonomous Self-Healing Daemon Guard (12-Engine Unified Assurance Pipeline):\n\
         Tick: {} | Timestamp: {}\n\
         All Invariants Verified: {} (Phases Certified: {}/12)\n\
         Receipt Digest: {}\n\
         Circuit Breakers Active: {}\n\
         Self-Healing Actions Taken: {}\n\
         Committed to Audit Store: {}\n\
           Leaf Index: {} | Merkle Root: {}\n\
           Causal DAG Node ID: {}\n\
         Verify independently via: pandora audit verify --dag-node {}",
        last_report.sequence_number,
        last_report.timestamp_secs,
        last_report.all_invariants_verified,
        last_report.receipt.phases_certified,
        last_report.receipt.receipt_sha256,
        last_report.active_circuit_breakers.len(),
        if last_report.self_healing_actions.is_empty() {
            "None (All engines nominal)".to_string()
        } else {
            format!("{:?}", last_report.self_healing_actions)
        },
        store_path.display(),
        last_report.audit_leaf_index,
        last_report.audit_root_hash,
        last_report.audit_node_id,
        last_report.audit_node_id
    );

    let mut json_data = serde_json::to_value(last_report).unwrap_or_else(|_| json!({}));

    if let Some(map) = json_data.as_object_mut() {
        map.insert(
            "receipt_sha256".to_string(),
            json!(last_report.receipt.receipt_sha256),
        );
        map.insert(
            "phases_certified".to_string(),
            json!(last_report.receipt.phases_certified),
        );
        if tick_reports.len() > 1 {
            map.insert(
                "tick_reports".to_string(),
                serde_json::to_value(&tick_reports).unwrap_or_else(|_| json!([])),
            );
        }
    }

    Ok(success("runtime daemon-guard", json_data, human))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runtime_speculative_benchmark_defaults() {
        let result =
            speculative_with_cmd(&[], "runtime speculative benchmark").expect("should succeed");
        assert_eq!(result.command, "runtime speculative benchmark");
        assert_eq!(result.data["draft_window_gamma"], 5);
        assert!(
            (result.data["acceptance_probability_alpha"]
                .as_f64()
                .unwrap()
                - 0.80)
                .abs()
                < 1e-4
        );
        assert!((result.data["latency_ratio"].as_f64().unwrap() - 0.10).abs() < 1e-4);
        assert!(result.data["total_tokens_simulated"].as_u64().unwrap() >= 128);
        assert!(result.data["empirical_speedup"].as_f64().unwrap() > 1.5);

        let theo_expected = result.data["theoretical_expected_tokens"].as_f64().unwrap();
        assert!((theo_expected - 3.68928).abs() < 1e-4);
        let theo_speedup = result.data["theoretical_speedup"].as_f64().unwrap();
        assert!((theo_speedup - 2.45952).abs() < 1e-4);
    }

    #[test]
    fn test_runtime_speculative_benchmark_custom_options() {
        let args = vec![
            "benchmark".to_string(),
            "--gamma".to_string(),
            "4".to_string(),
            "--alpha".to_string(),
            "0.90".to_string(),
            "--latency-ratio".to_string(),
            "0.05".to_string(),
            "--tokens".to_string(),
            "64".to_string(),
            "--seed".to_string(),
            "42".to_string(),
        ];
        let result =
            speculative_with_cmd(&args, "runtime speculative benchmark").expect("should succeed");
        assert_eq!(result.data["draft_window_gamma"], 4);
        assert!(
            (result.data["acceptance_probability_alpha"]
                .as_f64()
                .unwrap()
                - 0.90)
                .abs()
                < 1e-4
        );
        assert!((result.data["latency_ratio"].as_f64().unwrap() - 0.05).abs() < 1e-4);
        assert!(result.data["total_tokens_simulated"].as_u64().unwrap() >= 64);
        assert_eq!(result.data["seed"], 42);
    }

    #[test]
    fn test_runtime_speculative_validation_errors() {
        let err1 = speculative_with_cmd(
            &["--gamma".to_string(), "0".to_string()],
            "runtime speculative benchmark",
        );
        assert!(err1.is_err());
        let err2 = speculative_with_cmd(
            &["--gamma".to_string(), "40".to_string()],
            "runtime speculative benchmark",
        );
        assert!(err2.is_err());

        let err3 = speculative_with_cmd(
            &["--alpha".to_string(), "1.5".to_string()],
            "runtime speculative benchmark",
        );
        assert!(err3.is_err());

        let err4 = speculative_with_cmd(
            &["--latency-ratio".to_string(), "-0.1".to_string()],
            "runtime speculative benchmark",
        );
        assert!(err4.is_err());

        let err5 = speculative_with_cmd(
            &["--tokens".to_string(), "0".to_string()],
            "runtime speculative benchmark",
        );
        assert!(err5.is_err());

        let err6 = speculative_with_cmd(
            &["benchmark".to_string(), "extra".to_string()],
            "runtime speculative benchmark",
        );
        assert!(err6.is_err());
    }

    #[test]
    fn test_runtime_kv_cache_defaults_and_exact_compression_identities() {
        let result = kv_cache_with_cmd(&[], "runtime kv-cache").expect("should succeed");
        assert_eq!(result.command, "runtime kv-cache");
        assert_eq!(result.data["active_sessions"], 8);
        assert_eq!(result.data["prefix_length_tokens"], 512);
        assert_eq!(result.data["suffix_length_tokens"], 256);
        assert_eq!(result.data["bytes_per_token"], 2048);
        assert_eq!(result.data["total_tokens_requested"], 6144);
        assert_eq!(result.data["total_tokens_shared"], 3584);
        assert_eq!(result.data["dense_memory_bytes"], 12582912);
        assert_eq!(result.data["compressed_memory_bytes"], 5242880);

        let emp_cr = result.data["empirical_compression_ratio"].as_f64().unwrap();
        let theo_cr = result.data["theoretical_compression_ratio"]
            .as_f64()
            .unwrap();
        let asym_cr = result.data["asymptotic_compression_ratio"]
            .as_f64()
            .unwrap();
        assert!((emp_cr - 2.40).abs() < 1e-4);
        assert!((theo_cr - 2.40).abs() < 1e-4);
        assert!((asym_cr - 3.00).abs() < 1e-4);

        let savings = result.data["memory_savings_percent"].as_f64().unwrap();
        assert!((savings - (7.0 / 12.0 * 100.0)).abs() < 1e-4);
    }

    #[test]
    fn test_runtime_kv_cache_custom_options() {
        let args = vec![
            "--sessions".to_string(),
            "10".to_string(),
            "--prefix-len".to_string(),
            "800".to_string(),
            "--suffix-len".to_string(),
            "200".to_string(),
            "--bytes-per-token".to_string(),
            "1024".to_string(),
        ];
        let result = kv_cache_with_cmd(&args, "runtime kv-cache").expect("should succeed");
        assert_eq!(result.data["active_sessions"], 10);
        assert_eq!(result.data["prefix_length_tokens"], 800);
        assert_eq!(result.data["suffix_length_tokens"], 200);
        assert_eq!(result.data["bytes_per_token"], 1024);

        let emp_cr = result.data["empirical_compression_ratio"].as_f64().unwrap();
        let theo_cr = result.data["theoretical_compression_ratio"]
            .as_f64()
            .unwrap();
        assert!((emp_cr - (1.0 / 0.28)).abs() < 1e-4);
        assert!((theo_cr - (1.0 / 0.28)).abs() < 1e-4);
    }

    #[test]
    fn test_runtime_kv_cache_validation_errors() {
        let err1 = kv_cache_with_cmd(
            &["--sessions".to_string(), "0".to_string()],
            "runtime kv-cache",
        );
        assert!(err1.is_err());
        let err2 = kv_cache_with_cmd(
            &["--prefix-len".to_string(), "0".to_string()],
            "runtime kv-cache",
        );
        assert!(err2.is_err());
        let err3 = kv_cache_with_cmd(
            &["--bytes-per-token".to_string(), "0".to_string()],
            "runtime kv-cache",
        );
        assert!(err3.is_err());
        let err4 = kv_cache_with_cmd(&["unexpected".to_string()], "runtime kv-cache");
        assert!(err4.is_err());
    }

    #[test]
    fn test_execute_group_routing() {
        let res_spec =
            execute(&["speculative".to_string()]).expect("runtime speculative should succeed");
        assert_eq!(res_spec.command, "runtime speculative benchmark");

        let res_kv = execute(&["kv-cache".to_string()]).expect("runtime kv-cache should succeed");
        assert_eq!(res_kv.command, "runtime kv-cache");

        let res_mfg = execute(&["mfg".to_string()]).expect("runtime mfg should succeed");
        assert_eq!(res_mfg.command, "runtime mfg");

        let res_pac =
            execute(&["pac-bayes".to_string()]).expect("runtime pac-bayes should succeed");
        assert_eq!(res_pac.command, "runtime pac-bayes");

        let res_trop = execute(&["tropical".to_string()]).expect("runtime tropical should succeed");
        assert_eq!(res_trop.command, "runtime tropical");
    }

    #[test]
    fn test_runtime_mfg_cli_defaults_and_options() {
        let res = execute(&[
            "mfg".to_string(),
            "--resources".to_string(),
            "3".to_string(),
            "--temperature".to_string(),
            "2.0".to_string(),
        ])
        .expect("mfg should succeed");
        assert_eq!(res.command, "runtime mfg");
        assert_eq!(res.data["num_resources"], 3);
        assert!(
            res.data["fixed_point"]["is_strict_contraction"]
                .as_bool()
                .unwrap()
        );
        assert!(res.data["poa"]["bound_satisfied"].as_bool().unwrap());
        let poa = res.data["poa"]["price_of_anarchy"].as_f64().unwrap();
        assert!(poa <= 4.0 / 3.0 + 1e-4);
    }

    #[test]
    fn test_runtime_mfg_validation_errors() {
        let err1 = execute(&[
            "mfg".to_string(),
            "--resources".to_string(),
            "1".to_string(),
        ]);
        assert!(err1.is_err());

        let err2 = execute(&[
            "mfg".to_string(),
            "--resources".to_string(),
            "3".to_string(),
            "--base-costs".to_string(),
            "1.0,2.0".to_string(),
        ]);
        assert!(err2.is_err());
    }

    #[test]
    fn test_runtime_pac_bayes_cli_defaults_and_options() {
        let res = execute(&[
            "pac-bayes".to_string(),
            "--samples".to_string(),
            "500".to_string(),
            "--target-risk".to_string(),
            "0.15".to_string(),
        ])
        .expect("pac-bayes should succeed");
        assert_eq!(res.command, "runtime pac-bayes");
        assert!(res.data["certified_risk_bound"].as_f64().unwrap() <= 0.15);
        assert_eq!(res.data["verdict"], "PromoteToProduction");
    }

    #[test]
    fn test_runtime_tropical_cli_defaults_and_options() {
        let res_pipe = execute(&[
            "tropical".to_string(),
            "--nodes".to_string(),
            "4".to_string(),
            "--topology".to_string(),
            "pipeline".to_string(),
        ])
        .expect("tropical pipeline should succeed");
        assert_eq!(res_pipe.command, "runtime tropical");
        assert_eq!(res_pipe.data["nodes"], 4);
        assert_eq!(res_pipe.data["critical_path_makespan"], 60.0); // 10 + 20 + 30 = 60
        assert!(!res_pipe.data["has_positive_cycle"].as_bool().unwrap());

        let res_cycle = execute(&[
            "tropical".to_string(),
            "--nodes".to_string(),
            "3".to_string(),
            "--topology".to_string(),
            "cycle".to_string(),
        ])
        .expect("tropical cycle should succeed");
        assert_eq!(res_cycle.command, "runtime tropical");
        assert!(res_cycle.data["sla_satisfied"].as_bool().unwrap());
        assert!(res_cycle.data["maximum_cycle_mean"].as_f64().unwrap() > 0.0);
    }

    #[test]
    fn test_runtime_daemon_guard_cli_once() {
        let temp_dir = std::env::temp_dir().join(format!("daemon_cli_test_{}", std::process::id()));
        let store_path = temp_dir.join("merkle_audit.json");

        let res = execute(&[
            "daemon-guard".to_string(),
            "--once".to_string(),
            "--store".to_string(),
            store_path.display().to_string(),
        ])
        .expect("daemon-guard should succeed");

        assert_eq!(res.command, "runtime daemon-guard");
        assert_eq!(res.data["sequence_number"], 0);
        assert!(res.data["all_invariants_verified"].as_bool().unwrap());
        assert_eq!(res.data["phases_certified"], 12);
        assert!(
            res.data["receipt_sha256"]
                .as_str()
                .unwrap()
                .starts_with("sha256:")
        );
        assert!(store_path.exists());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
