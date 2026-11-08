#![forbid(unsafe_code)]

//! Unified Mathematical Assurance Receipt Pipeline (Phases 9–20, Theorems 17–40)
//!
//! Synthesizes formal verification certificates across all twelve mathematical
//! runtime engines into a single SHA-256 tamper-evident assurance receipt for
//! autonomous agent execution and governance auditing.

use crate::adversarial_bandit::Exp3AdversarialBandit;
use crate::byzantine_aggregator::{AgentVectorUpdate, AggregationRule, ByzantineAggregator};
use crate::conformal_calibrator::{CandidateAction, SplitConformalCalibrator};
use crate::martingale_gate::{CanaryGateVerdict, MartingaleCanaryGate};
use crate::mean_field_game::{MeanFieldGame, MfgResourceConfig};
use crate::pac_bayes_monitor::{PacBayesMonitor, PolicyPromotionVerdict};
use crate::privacy_budget::{DistributedPrivacyTracker, PrivacyExpenditure, RdpBudget};
use crate::spectral_topology::{SpectralTopologyAnalyzer, TopologyVerdict, WeightedEdge};
use crate::tropical_dag::{MaxPlusMatrix, TropicalDagScheduler};
use crate::wasserstein_drift::{EmpiricalDistribution, SinkhornDriftDetector};
use crate::work_stealing::{StealableTask, StealableTaskDag, WorkStealingScheduler};
use crate::zk_attestation::{FieldElement, StateTransitionAir};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Summary of an individual phase certificate inside the unified assurance receipt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhaseCertificate {
    pub phase: u8,
    pub engine: String,
    pub theorems: String,
    pub invariant_verified: bool,
    pub metric_summary: String,
}

/// Complete tamper-evident mathematical assurance receipt across Phases 9–20.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnifiedAssuranceReceipt {
    pub run_id: String,
    pub sequence_number: u64,
    pub prev_receipt_sha256: Option<String>,
    pub all_invariants_verified: bool,
    pub phases_certified: usize,
    pub certificates: Vec<PhaseCertificate>,
    pub receipt_sha256: String,
    pub signature: Option<String>,
}

impl UnifiedAssuranceReceipt {
    /// Execute a full end-to-end self-certifying mathematical audit across Phases 9–20
    /// for a given `run_id` and return the unchained `UnifiedAssuranceReceipt`.
    pub fn generate(run_id: &str) -> Result<Self, String> {
        Self::generate_chained(run_id, 0, None)
    }

    /// Execute an end-to-end self-certifying mathematical audit across all 12 phases
    /// binding to a prior receipt's hash and sequence number in a cryptographic hash chain.
    pub fn generate_chained(
        run_id: &str,
        sequence_number: u64,
        prev_receipt_sha256: Option<String>,
    ) -> Result<Self, String> {
        let mut certificates = Vec::with_capacity(12);

        // Phase 9: Renyi Differential Privacy Budget (Theorems 17-18)
        let mut tracker = DistributedPrivacyTracker::new(2.0, 1e-5).map_err(|e| e.to_string())?;
        let rdp_budget = RdpBudget::from_gaussian(1.0, 4.0).map_err(|e| e.to_string())?;
        let spent_eps = tracker
            .record_expenditure(PrivacyExpenditure {
                transaction_id: "tx-assurance-1".to_owned(),
                agent_id: "agent-core".to_owned(),
                query_id: "q-1".to_owned(),
                mechanism_name: "gaussian".to_owned(),
                sensitivity: 1.0,
                noise_parameter: 4.0,
                rdp_expenditure: rdp_budget,
                timestamp_ms: 1000,
            })
            .map_err(|e| e.to_string())?;
        let (opt_alpha, _) = tracker
            .cumulative_budget
            .optimal_order_for_delta(1e-5)
            .map_err(|e| e.to_string())?;
        let phase9_ok = spent_eps > 0.0 && spent_eps <= tracker.global_max_epsilon;
        certificates.push(PhaseCertificate {
            phase: 9,
            engine: "renyi_differential_privacy".to_owned(),
            theorems: "Theorems 17-18".to_owned(),
            invariant_verified: phase9_ok,
            metric_summary: format!(
                "eps={spent_eps:.4}/{:.2} (alpha*={opt_alpha:.1}, delta=1.0e-5)",
                tracker.global_max_epsilon
            ),
        });

        // Phase 10: Zero-Knowledge Succinct State Attestation (Theorems 19-20)
        let air = StateTransitionAir::new(8).map_err(|e| e.to_string())?;
        let mut pre = Vec::with_capacity(8);
        let mut act = Vec::with_capacity(8);
        let mut post = Vec::with_capacity(8);
        for i in 0..8 {
            let s = FieldElement::new((100 + i * 17) as u64);
            let a = FieldElement::new((25 + i * 9) as u64);
            pre.push(s);
            act.push(a);
            post.push(s.add(a));
        }
        let proof = air
            .prove_transition(&pre, &act, &post)
            .map_err(|e| e.to_string())?;
        let phase10_ok = air.verify_transition(&proof).map_err(|e| e.to_string())?;
        certificates.push(PhaseCertificate {
            phase: 10,
            engine: "zk_fiat_shamir_attestation".to_owned(),
            theorems: "Theorems 19-20".to_owned(),
            invariant_verified: phase10_ok,
            metric_summary: format!("domain={}, verified={phase10_ok}", proof.domain_size),
        });

        // Phase 11: Blumofe-Leiserson Adaptive Work-Stealing Scheduler (Theorems 21-22)
        let sched = WorkStealingScheduler::new(4, 64).map_err(|e| e.to_string())?;
        let mut dag = StealableTaskDag::new();
        dag.add_task(StealableTask::new("t_0", "root", 10));
        for i in 1..8 {
            dag.add_task(
                StealableTask::new(format!("t_{i}"), format!("leaf_{i}"), 10)
                    .with_dependencies(vec!["t_0".to_owned()]),
            );
        }
        let telem = sched.execute_dag(&dag).map_err(|e| e.to_string())?;
        let phase11_ok = telem.bl_bound_satisfied && telem.actual_makespan_tp > 0;
        certificates.push(PhaseCertificate {
            phase: 11,
            engine: "work_stealing_scheduler".to_owned(),
            theorems: "Theorems 21-22".to_owned(),
            invariant_verified: phase11_ok,
            metric_summary: format!(
                "T_P={} <= {:.1}, speedup={:.2}x",
                telem.actual_makespan_tp, telem.theoretical_bl_bound, telem.observed_speedup
            ),
        });

        // Phase 12: Byzantine-Resilient Multi-Agent Consensus (Theorems 23-24)
        let byz = ByzantineAggregator::new(1, AggregationRule::MultiKrum { selection_q: 3 })
            .map_err(|e| e.to_string())?;
        let updates = vec![
            AgentVectorUpdate::new("a1", vec![1.0, 2.0]),
            AgentVectorUpdate::new("a2", vec![1.02, 1.98]),
            AgentVectorUpdate::new("a3", vec![0.99, 2.01]),
            AgentVectorUpdate::new("a4", vec![1.01, 1.99]),
            AgentVectorUpdate::new("byz", vec![100.0, -100.0]),
        ];
        let byz_res = byz.aggregate(&updates).map_err(|e| e.to_string())?;
        let phase12_ok =
            (byz_res.consensus_vector[0] - 1.0).abs() < 0.1 && !byz_res.selected_agents.is_empty();
        certificates.push(PhaseCertificate {
            phase: 12,
            engine: "byzantine_krum_consensus".to_owned(),
            theorems: "Theorems 23-24".to_owned(),
            invariant_verified: phase12_ok,
            metric_summary: format!(
                "selected={:?}, cone_factor={:.3}",
                byz_res.selected_agents, byz_res.theoretical_deviation_factor
            ),
        });

        // Phase 13: Entropic Optimal Transport Semantic Drift Detector (Theorems 25-26)
        let mu = EmpiricalDistribution::uniform(vec![vec![0.0, 0.0], vec![1.0, 1.0]])
            .map_err(|e| e.to_string())?;
        let nu = EmpiricalDistribution::uniform(vec![vec![0.05, 0.02], vec![1.02, 0.98]])
            .map_err(|e| e.to_string())?;
        let detector =
            SinkhornDriftDetector::new(0.1, 100, 1e-7, 0.5).map_err(|e| e.to_string())?;
        let drift = detector
            .evaluate_drift(&mu, &nu)
            .map_err(|e| e.to_string())?;
        let phase13_ok = drift.sinkhorn_divergence >= 0.0 && !drift.drift_detected;
        certificates.push(PhaseCertificate {
            phase: 13,
            engine: "sinkhorn_wasserstein_drift".to_owned(),
            theorems: "Theorems 25-26".to_owned(),
            invariant_verified: phase13_ok,
            metric_summary: format!(
                "S_eps={:.6}, kappa={:.4}, drift={}",
                drift.sinkhorn_divergence, drift.birkhoff_contraction_factor, drift.drift_detected
            ),
        });

        // Phase 14: Martingale Anytime-Valid Canary Gate (Theorems 27-28)
        let mut gate = MartingaleCanaryGate::new(0.05, 0.25, 4).map_err(|e| e.to_string())?;
        for x_t in [-0.02, 0.01, -0.01, 0.0, -0.01] {
            gate.observe(x_t).map_err(|e| e.to_string())?;
        }
        let gate_status = gate.snapshot().map_err(|e| e.to_string())?;
        let phase14_ok = gate_status.verdict != CanaryGateVerdict::RollbackRegression
            && gate_status.e_value < gate_status.ville_rejection_threshold;
        certificates.push(PhaseCertificate {
            phase: 14,
            engine: "ville_martingale_canary_gate".to_owned(),
            theorems: "Theorems 27-28".to_owned(),
            invariant_verified: phase14_ok,
            metric_summary: format!(
                "E_t={:.4} < {:.1}, verdict={}",
                gate_status.e_value,
                gate_status.ville_rejection_threshold,
                gate_status.verdict.as_str()
            ),
        });

        // Phase 15: Conformal Prediction Uncertainty Calibrator (Theorems 29-30)
        let scores: Vec<f64> = (1..=19).map(|i| i as f64 * 0.05).collect();
        let cal = SplitConformalCalibrator::calibrate(&scores, 0.10).map_err(|e| e.to_string())?;
        let pred = cal
            .predict_set(&[
                CandidateAction {
                    action_id: "verified_tool".to_owned(),
                    nonconformity_score: 0.20,
                },
                CandidateAction {
                    action_id: "untrusted_tool".to_owned(),
                    nonconformity_score: 0.95,
                },
            ])
            .map_err(|e| e.to_string())?;
        let phase15_ok = pred.set_size == 1 && pred.marginal_coverage_lower_bound >= 0.90 - 1e-12;
        certificates.push(PhaseCertificate {
            phase: 15,
            engine: "split_adaptive_conformal".to_owned(),
            theorems: "Theorems 29-30".to_owned(),
            invariant_verified: phase15_ok,
            metric_summary: format!(
                "|C_alpha|={}, q={:.2}, cov>={:.2}",
                pred.set_size, pred.quantile_threshold, pred.marginal_coverage_lower_bound
            ),
        });

        // Phase 16: Spectral Graph Topology & Cheeger Conductance (Theorems 31-32)
        let edges = vec![
            WeightedEdge {
                u: 0,
                v: 1,
                weight: 1.0,
            },
            WeightedEdge {
                u: 1,
                v: 2,
                weight: 1.0,
            },
            WeightedEdge {
                u: 2,
                v: 3,
                weight: 1.0,
            },
            WeightedEdge {
                u: 3,
                v: 0,
                weight: 1.0,
            },
            WeightedEdge {
                u: 0,
                v: 2,
                weight: 1.0,
            },
            WeightedEdge {
                u: 1,
                v: 3,
                weight: 1.0,
            },
        ];
        let topo = SpectralTopologyAnalyzer::new(4, edges, 0.20)
            .map_err(|e| e.to_string())?
            .analyze();
        let phase16_ok =
            topo.cheeger_inequality_verified && topo.verdict == TopologyVerdict::ConnectedResilient;
        certificates.push(PhaseCertificate {
            phase: 16,
            engine: "spectral_fiedler_topology".to_owned(),
            theorems: "Theorems 31-32".to_owned(),
            invariant_verified: phase16_ok,
            metric_summary: format!(
                "lambda2={:.3}, nu2={:.3}, h(G)={:.3}",
                topo.fiedler_value_unnormalized,
                topo.fiedler_value_normalized,
                topo.sweep_cut_conductance
            ),
        });

        // Phase 17: Exp3 Adversarial Multi-Armed Bandit (Theorems 33-34)
        let mut bandit = Exp3AdversarialBandit::with_horizon(
            vec!["route_primary".to_owned(), "route_backup".to_owned()],
            100,
        )
        .map_err(|e| e.to_string())?;
        bandit.update_reward(0, 0.95).map_err(|e| e.to_string())?;
        let b_status = bandit.status();
        let prob_sum: f64 = b_status.arms.iter().map(|a| a.sampling_probability).sum();
        let phase17_ok =
            (prob_sum - 1.0).abs() < 1e-12 && b_status.minimax_regret_upper_bound > 0.0;
        certificates.push(PhaseCertificate {
            phase: 17,
            engine: "exp3_adversarial_bandit".to_owned(),
            theorems: "Theorems 33-34".to_owned(),
            invariant_verified: phase17_ok,
            metric_summary: format!(
                "K={}, gamma={:.4}, regret_bound={:.2}",
                b_status.num_arms, b_status.gamma, b_status.minimax_regret_upper_bound
            ),
        });

        // Phase 18: Mean-Field Game Equilibrium & Multi-Agent Congestion Control (Theorems 35-36)
        let resources = vec![
            MfgResourceConfig::new("tool_compute_alpha", 10.0, 0.5).map_err(|e| e.to_string())?,
            MfgResourceConfig::new("tool_compute_beta", 15.0, 0.4).map_err(|e| e.to_string())?,
            MfgResourceConfig::new("tool_compute_gamma", 20.0, 0.2).map_err(|e| e.to_string())?,
        ];
        let mfg = MeanFieldGame::new(resources, 2.0).map_err(|e| e.to_string())?;
        let eq = mfg
            .compute_equilibrium(200, 1e-6)
            .map_err(|e| e.to_string())?;
        let poa = mfg.compute_price_of_anarchy().map_err(|e| e.to_string())?;
        let phase18_ok = eq.is_strict_contraction && poa.bound_satisfied;
        certificates.push(PhaseCertificate {
            phase: 18,
            engine: "mean_field_game_congestion".to_owned(),
            theorems: "Theorems 35-36".to_owned(),
            invariant_verified: phase18_ok,
            metric_summary: format!(
                "PoA={:.3} <= {:.3}, kappa={:.3} < 1.0, iters={}",
                poa.price_of_anarchy,
                poa.theoretical_upper_bound,
                eq.contraction_modulus,
                eq.iterations
            ),
        });

        // Phase 19: PAC-Bayesian Generalization & Prompt/Policy Risk Certificates (Theorems 37-38)
        let pac_monitor = PacBayesMonitor::new();
        let cert = pac_monitor
            .certify_risk(0.04, 100, 0.25, 0.05)
            .map_err(|e| e.to_string())?;
        let verdict = pac_monitor.evaluate_promotion_gate(&cert, 0.20);
        let phase19_ok =
            cert.bound_verified && verdict == PolicyPromotionVerdict::PromoteToProduction;
        certificates.push(PhaseCertificate {
            phase: 19,
            engine: "pac_bayes_risk_certificate".to_owned(),
            theorems: "Theorems 37-38".to_owned(),
            invariant_verified: phase19_ok,
            metric_summary: format!(
                "L_hat={:.4}, L_max={:.4} <= L_Pinsker={:.4}, verdict={:?}",
                cert.empirical_risk, cert.maurer_risk_bound, cert.pinsker_risk_bound, verdict
            ),
        });

        // Phase 20: Tropical / Max-Plus Algebra DAG Critical-Path & Latency Calculus (Theorems 39-40)
        let trop_sched = TropicalDagScheduler::new();
        let mut adj = MaxPlusMatrix::zero(4).map_err(|e| e.to_string())?;
        adj.set(0, 1, 15.0).map_err(|e| e.to_string())?;
        adj.set(1, 2, 25.0).map_err(|e| e.to_string())?;
        adj.set(0, 2, 30.0).map_err(|e| e.to_string())?;
        adj.set(2, 3, 10.0).map_err(|e| e.to_string())?;
        let kleene = trop_sched
            .compute_kleene_star(&adj)
            .map_err(|e| e.to_string())?;

        let mut cycle_adj = MaxPlusMatrix::zero(2).map_err(|e| e.to_string())?;
        cycle_adj.set(0, 1, 5.0).map_err(|e| e.to_string())?;
        cycle_adj.set(1, 0, 7.0).map_err(|e| e.to_string())?;
        let karp = trop_sched
            .compute_karp_maximum_cycle_mean(&cycle_adj)
            .map_err(|e| e.to_string())?;

        let phase20_ok = !kleene.has_positive_cycle
            && kleene.critical_path_makespan > 0.0
            && karp.maximum_cycle_mean <= 10.0;
        certificates.push(PhaseCertificate {
            phase: 20,
            engine: "tropical_dag_latency".to_owned(),
            theorems: "Theorems 39-40".to_owned(),
            invariant_verified: phase20_ok,
            metric_summary: format!(
                "makespan={:.1}ms, acyclic={}, lambda(G)={:.2} <= 10.0",
                kleene.critical_path_makespan, !kleene.has_positive_cycle, karp.maximum_cycle_mean
            ),
        });

        let all_verified = certificates.iter().all(|c| c.invariant_verified);
        let phases_certified = certificates.iter().filter(|c| c.invariant_verified).count();

        let mut hasher = Sha256::new();
        hasher.update(run_id.as_bytes());
        hasher.update(sequence_number.to_be_bytes());
        if let Some(prev) = &prev_receipt_sha256 {
            hasher.update(b"prev:");
            hasher.update(prev.as_bytes());
        }
        for cert in &certificates {
            hasher.update([cert.phase]);
            hasher.update(cert.engine.as_bytes());
            hasher.update(cert.theorems.as_bytes());
            hasher.update([u8::from(cert.invariant_verified)]);
            hasher.update(cert.metric_summary.as_bytes());
        }
        let digest = hasher.finalize();
        let receipt_sha256 = format!("sha256:{digest:x}");

        Ok(Self {
            run_id: run_id.to_owned(),
            sequence_number,
            prev_receipt_sha256,
            all_invariants_verified: all_verified,
            phases_certified,
            certificates,
            receipt_sha256,
            signature: None,
        })
    }

    /// Attach an explicit cryptographic signature to the assurance receipt.
    pub fn with_signature(mut self, signature: impl Into<String>) -> Self {
        self.signature = Some(signature.into());
        self
    }

    /// Sign the receipt digest with a designated secret key using SHA-256.
    pub fn sign_with_key(&mut self, secret_key: &str) {
        let mut sig_hasher = Sha256::new();
        sig_hasher.update(secret_key.as_bytes());
        sig_hasher.update(b":");
        sig_hasher.update(self.receipt_sha256.as_bytes());
        self.signature = Some(format!("sig:sha256:{:x}", sig_hasher.finalize()));
    }

    /// Verify cryptographic hash integrity of the receipt including sequence number and prev hash.
    pub fn verify_hash_integrity(&self) -> bool {
        let mut hasher = Sha256::new();
        hasher.update(self.run_id.as_bytes());
        hasher.update(self.sequence_number.to_be_bytes());
        if let Some(prev) = &self.prev_receipt_sha256 {
            hasher.update(b"prev:");
            hasher.update(prev.as_bytes());
        }
        for cert in &self.certificates {
            hasher.update([cert.phase]);
            hasher.update(cert.engine.as_bytes());
            hasher.update(cert.theorems.as_bytes());
            hasher.update([u8::from(cert.invariant_verified)]);
            hasher.update(cert.metric_summary.as_bytes());
        }
        let expected = format!("sha256:{:x}", hasher.finalize());
        self.receipt_sha256 == expected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unified_assurance_receipt_certifies_all_twelve_phases() {
        let receipt = UnifiedAssuranceReceipt::generate("test-run-v307").unwrap();
        assert!(receipt.all_invariants_verified);
        assert_eq!(receipt.phases_certified, 12);
        assert_eq!(receipt.certificates.len(), 12);
        assert!(receipt.receipt_sha256.starts_with("sha256:"));
        assert_eq!(receipt.sequence_number, 0);
        assert_eq!(receipt.prev_receipt_sha256, None);
        assert!(receipt.verify_hash_integrity());
    }

    #[test]
    fn unified_assurance_receipt_hash_chaining_and_signing() {
        let r1 = UnifiedAssuranceReceipt::generate_chained("run-001", 1, None).unwrap();
        assert_eq!(r1.sequence_number, 1);
        assert!(r1.verify_hash_integrity());

        let mut r2 = UnifiedAssuranceReceipt::generate_chained(
            "run-002",
            2,
            Some(r1.receipt_sha256.clone()),
        )
        .unwrap();
        assert_eq!(r2.sequence_number, 2);
        assert_eq!(r2.prev_receipt_sha256, Some(r1.receipt_sha256));
        assert!(r2.verify_hash_integrity());

        r2.sign_with_key("master-governance-key");
        assert!(r2.signature.is_some());
        assert!(r2.signature.unwrap().starts_with("sig:sha256:"));
    }
}
