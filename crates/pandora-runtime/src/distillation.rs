//! Cross-session `.pnn` distillation, Ed25519-signed `MemoryCapsule` & `CalibrationAttestationReceipt`
//! emission, Phase 13 Sinkhorn/Wasserstein drift gating, Phase 14 Martingale canary gating,
//! and fail-closed attestation verification.

#![forbid(unsafe_code)]

use crate::decision_engine::{
    CalibrationAttestationReceipt, DecisionDriver, MoiraDecisionDriver, compute_dpo_dataset_digest,
};
use crate::evolution::BayesianCanaryGate;
use crate::martingale_gate::{CanaryGateVerdict, MartingaleCanaryGate, MartingaleGateSnapshot};
use crate::mesh_sync::{MeshSyncEngine, PeerIdentity, SignedCapsulePackage};
use crate::sessions::SessionStore;
use crate::tropical_nn::TropicalNetwork;
use crate::wasserstein_drift::{EmpiricalDistribution, SinkhornDriftDetector, SinkhornDriftReport};
use ed25519_dalek::SigningKey;
use pandora_harnesses::decision_benchmark::DecisionBenchmarkSuite;
use pandora_types::{
    DistillationTelemetrySnapshot, DpoPair, MartingaleTelemetrySummary, MemoryCapsule,
    MemoryIntent, SopDefinition, SopStep, Timestamp, WassersteinTelemetrySummary,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DistillationError {
    NoDpoSources,
    EmptyDataset,
    InvalidSourcePath(String),
    InvalidDpoPair {
        source: String,
        line: usize,
        reason: String,
    },
    InvalidConfig(String),
    InvalidKeyFile(String),
    Io(String),
    Serialization(String),
    Decision(String),
    Tropical(String),
    Wasserstein(String),
    Martingale(String),
    Mesh(String),
    Session(String),
    VerificationFailed(String),
    Contract(String),
}

impl fmt::Display for DistillationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoDpoSources => write!(f, "no DPO pair sources discovered"),
            Self::EmptyDataset => write!(f, "aggregated DPO dataset contains zero pairs"),
            Self::InvalidSourcePath(msg) => write!(f, "invalid DPO source path: {msg}"),
            Self::InvalidDpoPair {
                source,
                line,
                reason,
            } => write!(f, "invalid DpoPair at {source}:{line}: {reason}"),
            Self::InvalidConfig(msg) => write!(f, "invalid distillation configuration: {msg}"),
            Self::InvalidKeyFile(msg) => write!(f, "invalid Ed25519 key file: {msg}"),
            Self::Io(msg) => write!(f, "distillation I/O error: {msg}"),
            Self::Serialization(msg) => write!(f, "distillation serialization error: {msg}"),
            Self::Decision(msg) => write!(f, "decision calibration error: {msg}"),
            Self::Tropical(msg) => write!(f, "tropical network error: {msg}"),
            Self::Wasserstein(msg) => write!(f, "Phase 13 Wasserstein drift error: {msg}"),
            Self::Martingale(msg) => write!(f, "Phase 14 Martingale gate error: {msg}"),
            Self::Mesh(msg) => write!(f, "MemoryCapsule signing/validation error: {msg}"),
            Self::Session(msg) => write!(f, "session store error: {msg}"),
            Self::VerificationFailed(msg) => write!(f, "attestation verification failed: {msg}"),
            Self::Contract(msg) => write!(f, "telemetry contract error: {msg}"),
        }
    }
}

impl std::error::Error for DistillationError {}

/// Aggregated, deterministically deduplicated cross-session `DpoPair` dataset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AggregatedDpoDataset {
    pub pairs: Vec<DpoPair>,
    pub source_paths: Vec<PathBuf>,
    pub per_source_counts: BTreeMap<String, usize>,
    pub duplicates_removed: usize,
    pub dpo_dataset_digest: String,
}

/// Configuration for cross-session `.pnn` + `MemoryCapsule` distillation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrossSessionDistillConfig {
    pub workspace: PathBuf,
    pub data_dir: Option<PathBuf>,
    pub session_paths: Vec<PathBuf>,
    pub pnn_path: PathBuf,
    pub capsule_out_path: Option<PathBuf>,
    pub key_file: Option<PathBuf>,
    pub telemetry_path: Option<PathBuf>,
    pub drift_threshold: f64,
    pub sinkhorn_epsilon: f64,
    pub martingale_alpha: f64,
    pub martingale_sigma: f64,
    pub learning_rate: f32,
    pub dry_run: bool,
    pub author: Option<String>,
    pub store_in_session_db: bool,
}

impl CrossSessionDistillConfig {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        let workspace = workspace.into();
        let pnn_path = workspace.join(".pandora").join("model_decision.pnn");
        Self {
            workspace,
            data_dir: None,
            session_paths: Vec::new(),
            pnn_path,
            capsule_out_path: None,
            key_file: None,
            telemetry_path: None,
            drift_threshold: 0.50,
            sinkhorn_epsilon: 0.10,
            martingale_alpha: 0.05,
            martingale_sigma: 0.50,
            learning_rate: 0.05,
            dry_run: false,
            author: None,
            store_in_session_db: false,
        }
    }

    pub fn with_data_dir(mut self, data_dir: impl Into<PathBuf>) -> Self {
        self.data_dir = Some(data_dir.into());
        self
    }

    pub fn with_session_paths(mut self, paths: Vec<PathBuf>) -> Self {
        self.session_paths = paths;
        self
    }

    pub fn with_pnn_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.pnn_path = path.into();
        self
    }

    pub fn with_capsule_out_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.capsule_out_path = Some(path.into());
        self
    }

    pub fn with_key_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.key_file = Some(path.into());
        self
    }

    pub fn with_telemetry_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.telemetry_path = Some(path.into());
        self
    }

    pub fn with_drift_threshold(mut self, threshold: f64) -> Self {
        self.drift_threshold = threshold;
        self
    }

    pub fn with_martingale_alpha(mut self, alpha: f64) -> Self {
        self.martingale_alpha = alpha;
        self
    }

    pub fn with_learning_rate(mut self, lr: f32) -> Self {
        self.learning_rate = lr;
        self
    }

    pub fn with_dry_run(mut self, dry_run: bool) -> Self {
        self.dry_run = dry_run;
        self
    }

    pub fn with_author(mut self, author: impl Into<String>) -> Self {
        self.author = Some(author.into());
        self
    }

    pub fn with_store_in_session_db(mut self, enabled: bool) -> Self {
        self.store_in_session_db = enabled;
        self
    }
}

/// Complete output of cross-session distillation, gating, signing, and persistence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrossSessionDistillResult {
    pub pnn_path: PathBuf,
    pub receipt_path: PathBuf,
    pub capsule_path: PathBuf,
    pub telemetry_path: Option<PathBuf>,
    pub source_paths: Vec<PathBuf>,
    pub per_source_counts: BTreeMap<String, usize>,
    pub pairs_count: usize,
    pub duplicates_removed: usize,
    pub dpo_dataset_digest: String,
    pub baseline_digest: String,
    pub candidate_digest: String,
    pub baseline_accuracy: f64,
    pub candidate_accuracy: f64,
    pub baseline_mean_margin: f64,
    pub candidate_mean_margin: f64,
    pub bayesian_lower_bound_95: f64,
    pub bayesian_promoted: bool,
    pub non_regression_passed: bool,
    pub drift_report: SinkhornDriftReport,
    pub martingale_snapshot: MartingaleGateSnapshot,
    pub promoted: bool,
    pub dry_run: bool,
    pub capsule_package: SignedCapsulePackage,
    pub receipt: CalibrationAttestationReceipt,
    pub telemetry_snapshot: DistillationTelemetrySnapshot,
}

/// Configuration for fail-closed `.pnn`, `CalibrationAttestationReceipt`, and `MemoryCapsule` verification.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttestationVerifyConfig {
    pub pnn_path: PathBuf,
    pub receipt_path: Option<PathBuf>,
    pub capsule_path: Option<PathBuf>,
    pub expected_pubkey_hex: Option<String>,
    pub require_signature: bool,
    pub require_capsule: bool,
}

impl AttestationVerifyConfig {
    pub fn new(pnn_path: impl Into<PathBuf>) -> Self {
        Self {
            pnn_path: pnn_path.into(),
            receipt_path: None,
            capsule_path: None,
            expected_pubkey_hex: None,
            require_signature: false,
            require_capsule: false,
        }
    }

    pub fn with_receipt_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.receipt_path = Some(path.into());
        self
    }

    pub fn with_capsule_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.capsule_path = Some(path.into());
        self
    }

    pub fn with_expected_pubkey_hex(mut self, pubkey_hex: impl Into<String>) -> Self {
        self.expected_pubkey_hex = Some(pubkey_hex.into());
        self
    }

    pub fn with_require_signature(mut self, require: bool) -> Self {
        self.require_signature = require;
        self
    }

    pub fn with_require_capsule(mut self, require: bool) -> Self {
        self.require_capsule = require;
        self
    }
}

/// Structured report produced by [`verify_pnn_and_capsule_attestation`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AttestationVerificationReport {
    pub valid: bool,
    pub pnn_path: PathBuf,
    pub receipt_path: PathBuf,
    pub capsule_path: Option<PathBuf>,
    pub chain_length: usize,
    pub latest_sequence: u64,
    pub latest_commitment: String,
    pub latest_chain_hash: String,
    pub pnn_digest: String,
    pub expected_candidate_digest: String,
    pub pnn_digest_matches: bool,
    pub signature_verified: bool,
    pub signer_public_key: Option<String>,
    pub capsule_digest: Option<String>,
    pub capsule_verified: bool,
    pub capsule_digest_matches: Option<bool>,
    pub promoted: bool,
    pub sinkhorn_divergence: Option<f64>,
    pub wasserstein_2_distance: Option<f64>,
    pub martingale_e_value: Option<f64>,
    pub martingale_verdict: Option<String>,
}

/// Default sidecar receipt ledger path `<pnn>.attestation.jsonl`.
pub fn attestation_receipt_path(pnn_path: &Path) -> PathBuf {
    let mut os = pnn_path.as_os_str().to_os_string();
    os.push(".attestation.jsonl");
    PathBuf::from(os)
}

/// Default sidecar signed capsule path `<pnn>.capsule.json`.
pub fn default_capsule_path(pnn_path: &Path) -> PathBuf {
    let mut os = pnn_path.as_os_str().to_os_string();
    os.push(".capsule.json");
    PathBuf::from(os)
}

/// Default persisted cockpit telemetry snapshot path `<data_dir>/cockpit_telemetry.json`.
pub fn default_telemetry_path(data_dir: &Path) -> PathBuf {
    data_dir.join("cockpit_telemetry.json")
}

/// Loads a [`DistillationTelemetrySnapshot`] from disk and validates finiteness.
pub fn load_telemetry_snapshot(
    path: &Path,
) -> Result<DistillationTelemetrySnapshot, DistillationError> {
    DistillationTelemetrySnapshot::load_from_file(path)
        .map_err(|e| DistillationError::Contract(e.to_string()))
}

/// Atomically persists a [`DistillationTelemetrySnapshot`] to disk via `atomic-write-file`.
pub fn save_telemetry_snapshot(
    path: &Path,
    snap: &DistillationTelemetrySnapshot,
) -> Result<(), DistillationError> {
    snap.validate()
        .map_err(|e| DistillationError::Contract(e.to_string()))?;
    let bytes = serde_json::to_vec_pretty(snap)
        .map_err(|e| DistillationError::Serialization(e.to_string()))?;
    atomic_write_bytes(path, &bytes)
}

/// Atomically writes bytes to `path`, creating parent directories as needed.
pub fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> Result<(), DistillationError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|e| DistillationError::Io(e.to_string()))?;
    }
    let mut file = atomic_write_file::AtomicWriteFile::open(path)
        .map_err(|e| DistillationError::Io(e.to_string()))?;
    file.write_all(bytes)
        .map_err(|e| DistillationError::Io(e.to_string()))?;
    file.commit()
        .map_err(|e| DistillationError::Io(e.to_string()))?;
    Ok(())
}

/// Discovers existing DPO pair sources across workspace `.pandora/dpo_pairs.jsonl`,
/// `<data_dir>/dpo_pairs.jsonl`, `<data_dir>/sessions/*/dpo_pairs.jsonl`, and explicit paths.
pub fn discover_dpo_sources(
    workspace: &Path,
    data_dir: Option<&Path>,
    explicit_paths: &[PathBuf],
) -> Vec<PathBuf> {
    let mut discovered = Vec::new();
    let mut seen_canonical = HashSet::new();

    let mut push_if_file = |candidate: PathBuf| {
        if candidate.is_file() {
            let key = fs::canonicalize(&candidate).unwrap_or_else(|_| candidate.clone());
            if seen_canonical.insert(key) {
                discovered.push(candidate);
            }
        }
    };

    push_if_file(workspace.join(".pandora").join("dpo_pairs.jsonl"));

    if let Some(dir) = data_dir {
        push_if_file(dir.join("dpo_pairs.jsonl"));
        let sessions_dir = dir.join("sessions");
        if sessions_dir.is_dir()
            && let Ok(entries) = fs::read_dir(&sessions_dir)
        {
            let mut subdirs: Vec<PathBuf> = entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect();
            subdirs.sort();
            for subdir in subdirs {
                push_if_file(subdir.join("dpo_pairs.jsonl"));
                push_if_file(subdir.join(".pandora").join("dpo_pairs.jsonl"));
            }
        }
    }

    for path in explicit_paths {
        if path.is_file() {
            push_if_file(path.clone());
        } else if path.is_dir() {
            push_if_file(path.join(".pandora").join("dpo_pairs.jsonl"));
            push_if_file(path.join("dpo_pairs.jsonl"));
        }
    }

    discovered
}

/// Parses, validates, deterministically deduplicates, and digests `DpoPair` records across files.
pub fn aggregate_dpo_pairs(paths: &[PathBuf]) -> Result<AggregatedDpoDataset, DistillationError> {
    if paths.is_empty() {
        return Err(DistillationError::NoDpoSources);
    }

    let mut sorted_paths = paths.to_vec();
    sorted_paths.sort();

    let mut per_source_counts = BTreeMap::new();
    let mut deduped: Vec<DpoPair> = Vec::new();
    let mut seen_keys: HashSet<(String, String, String, String)> = HashSet::new();
    let mut duplicates_removed = 0usize;

    for path in &sorted_paths {
        let source_label = path.display().to_string();
        let raw = fs::read_to_string(path).map_err(|e| {
            DistillationError::Io(format!("failed reading DPO source {source_label}: {e}"))
        })?;
        let parsed = parse_dpo_pairs_content(&raw, &source_label)?;
        per_source_counts.insert(source_label, parsed.len());

        for pair in parsed {
            let key = (
                pair.id.clone(),
                pair.prompt.clone(),
                pair.chosen.clone(),
                pair.rejected.clone(),
            );
            if seen_keys.insert(key) {
                deduped.push(pair);
            } else {
                duplicates_removed += 1;
            }
        }
    }

    if deduped.is_empty() {
        return Err(DistillationError::EmptyDataset);
    }

    deduped.sort_by(|a, b| {
        (&a.id, &a.prompt, &a.chosen, &a.rejected, a.margin.to_bits()).cmp(&(
            &b.id,
            &b.prompt,
            &b.chosen,
            &b.rejected,
            b.margin.to_bits(),
        ))
    });

    let dpo_dataset_digest = compute_dpo_dataset_digest(&deduped);

    Ok(AggregatedDpoDataset {
        pairs: deduped,
        source_paths: sorted_paths,
        per_source_counts,
        duplicates_removed,
        dpo_dataset_digest,
    })
}

fn parse_dpo_pairs_content(
    raw: &str,
    source_label: &str,
) -> Result<Vec<DpoPair>, DistillationError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let pairs: Vec<DpoPair> = if trimmed.starts_with('[') {
        let parsed: Vec<DpoPair> = serde_json::from_str(trimmed).map_err(|e| {
            DistillationError::Serialization(format!(
                "invalid DpoPair JSON array in {source_label}: {e}"
            ))
        })?;
        for (idx, pair) in parsed.iter().enumerate() {
            validate_dpo_pair(pair, source_label, idx + 1)?;
        }
        parsed
    } else {
        let mut out = Vec::new();
        for (idx, line) in raw.lines().enumerate() {
            let l = line.trim();
            if l.is_empty() {
                continue;
            }
            let pair: DpoPair =
                serde_json::from_str(l).map_err(|e| DistillationError::InvalidDpoPair {
                    source: source_label.to_owned(),
                    line: idx + 1,
                    reason: e.to_string(),
                })?;
            validate_dpo_pair(&pair, source_label, idx + 1)?;
            out.push(pair);
        }
        out
    };

    Ok(pairs)
}

fn validate_dpo_pair(
    pair: &DpoPair,
    source_label: &str,
    line: usize,
) -> Result<(), DistillationError> {
    if pair.prompt.trim().is_empty() {
        return Err(DistillationError::InvalidDpoPair {
            source: source_label.to_owned(),
            line,
            reason: "prompt must not be empty".to_owned(),
        });
    }
    if pair.chosen.trim().is_empty() {
        return Err(DistillationError::InvalidDpoPair {
            source: source_label.to_owned(),
            line,
            reason: "chosen action must not be empty".to_owned(),
        });
    }
    if pair.rejected.trim().is_empty() {
        return Err(DistillationError::InvalidDpoPair {
            source: source_label.to_owned(),
            line,
            reason: "rejected action must not be empty".to_owned(),
        });
    }
    if !pair.margin.is_finite() {
        return Err(DistillationError::InvalidDpoPair {
            source: source_label.to_owned(),
            line,
            reason: "margin must be finite".to_owned(),
        });
    }
    Ok(())
}

/// Runs cross-session DPO aggregation, `.pnn` calibration, Phase 13 Sinkhorn drift detection,
/// Phase 14 Martingale canary gating, Bayesian canary gating, Ed25519 `MemoryCapsule` +
/// `CalibrationAttestationReceipt` signing, and atomic checkpoint/telemetry persistence.
pub fn distill_cross_session_checkpoint(
    config: CrossSessionDistillConfig,
) -> Result<CrossSessionDistillResult, DistillationError> {
    if !config.drift_threshold.is_finite() || config.drift_threshold < 0.0 {
        return Err(DistillationError::InvalidConfig(
            "drift_threshold must be finite and >= 0".to_owned(),
        ));
    }
    if !config.sinkhorn_epsilon.is_finite() || config.sinkhorn_epsilon <= 0.0 {
        return Err(DistillationError::InvalidConfig(
            "sinkhorn_epsilon must be finite and > 0".to_owned(),
        ));
    }
    if !config.martingale_alpha.is_finite()
        || config.martingale_alpha <= 0.0
        || config.martingale_alpha >= 1.0
    {
        return Err(DistillationError::InvalidConfig(
            "martingale_alpha must lie in (0, 1)".to_owned(),
        ));
    }
    if !config.learning_rate.is_finite() || config.learning_rate <= 0.0 {
        return Err(DistillationError::InvalidConfig(
            "learning_rate must be finite and > 0".to_owned(),
        ));
    }
    for explicit in &config.session_paths {
        if !explicit.exists() {
            return Err(DistillationError::InvalidSourcePath(format!(
                "{} does not exist",
                explicit.display()
            )));
        }
    }

    let sources = discover_dpo_sources(
        &config.workspace,
        config.data_dir.as_deref(),
        &config.session_paths,
    );
    let dataset = aggregate_dpo_pairs(&sources)?;

    let baseline_network = if config.pnn_path.is_file() {
        TropicalNetwork::load_from_file(&config.pnn_path)
            .map_err(|e| DistillationError::Tropical(e.to_string()))?
    } else {
        MoiraDecisionDriver::calibrated_network()
    };

    let candidate_network = MoiraDecisionDriver::calibrate_network_from_pairs(
        &baseline_network,
        &dataset.pairs,
        config.learning_rate,
    )
    .map_err(|e| DistillationError::Decision(e.to_string()))?;

    let suite = DecisionBenchmarkSuite::default_real_task_traces();
    let baseline_driver = MoiraDecisionDriver::new(Some(baseline_network.clone()));
    let candidate_driver = MoiraDecisionDriver::new(Some(candidate_network.clone()));

    let SuiteEvaluationOutputs {
        report: base_report,
        embeddings: base_points,
        losses: base_losses,
    } = evaluate_suite_with_embeddings(&suite, "baseline", &baseline_driver)?;
    let SuiteEvaluationOutputs {
        report: cand_report,
        embeddings: cand_points,
        losses: cand_losses,
    } = evaluate_suite_with_embeddings(&suite, "candidate", &candidate_driver)?;

    // Phase 13: Sinkhorn-Knopp Entropic Optimal Transport Semantic Drift Detection
    let mu = EmpiricalDistribution::uniform(base_points)
        .map_err(|e| DistillationError::Wasserstein(e.to_string()))?;
    let nu = EmpiricalDistribution::uniform(cand_points)
        .map_err(|e| DistillationError::Wasserstein(e.to_string()))?;
    let detector =
        SinkhornDriftDetector::new(config.sinkhorn_epsilon, 200, 1e-6, config.drift_threshold)
            .map_err(|e| DistillationError::Wasserstein(e.to_string()))?;
    let drift_report = detector
        .evaluate_drift(&mu, &nu)
        .map_err(|e| DistillationError::Wasserstein(e.to_string()))?;
    if !drift_report.sinkhorn_divergence.is_finite() || drift_report.sinkhorn_divergence < 0.0 {
        return Err(DistillationError::Wasserstein(
            "Sinkhorn divergence must be finite and non-negative".to_owned(),
        ));
    }

    // Phase 14: Martingale Anytime-Valid Canary Gate
    let mut martingale_gate =
        MartingaleCanaryGate::new(config.martingale_alpha, config.martingale_sigma, 1)
            .map_err(|e| DistillationError::Martingale(e.to_string()))?;
    let mut martingale_snapshot = martingale_gate
        .snapshot()
        .map_err(|e| DistillationError::Martingale(e.to_string()))?;
    for (&base_loss, &cand_loss) in base_losses.iter().zip(cand_losses.iter()) {
        let x_t = (cand_loss - base_loss).clamp(-1.0, 1.0);
        martingale_snapshot = martingale_gate
            .observe(x_t)
            .map_err(|e| DistillationError::Martingale(e.to_string()))?;
    }

    // Bayesian Canary Gate + Non-Regression Check
    let canary_gate = BayesianCanaryGate::default();
    let base_passes = base_report.correct_count as u32;
    let base_failures = base_report
        .trace_count
        .saturating_sub(base_report.correct_count) as u32;
    let base_canary = canary_gate.evaluate(base_passes, base_failures, 0.0);

    let cand_passes = cand_report.correct_count as u32;
    let cand_failures = cand_report
        .trace_count
        .saturating_sub(cand_report.correct_count) as u32;
    let floor = (base_canary.lower_bound_95 - 1e-9).max(0.0);
    let cand_canary = canary_gate.evaluate(cand_passes, cand_failures, floor);

    let non_regression_passed = cand_report.branch_selection_accuracy + 1e-9
        >= base_report.branch_selection_accuracy
        && cand_report.mean_margin + 1e-9 >= base_report.mean_margin;
    let drift_passed = !drift_report.drift_detected;
    let martingale_passed = martingale_snapshot.verdict != CanaryGateVerdict::RollbackRegression
        && martingale_snapshot.e_value < martingale_snapshot.ville_rejection_threshold;

    let promoted =
        non_regression_passed && cand_canary.promoted && drift_passed && martingale_passed;

    // Resolve Ed25519 PeerIdentity (deterministic reuse if key_file/data_dir present, fail-closed on corrupt key)
    let identity = resolve_peer_identity(
        config.key_file.as_deref(),
        config.data_dir.as_deref(),
        config.dry_run,
    )?;
    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let capsule_package = build_signed_distillation_capsule(
        &identity,
        config.author.as_deref(),
        &dataset,
        &baseline_network,
        &candidate_network,
        &drift_report,
        &martingale_snapshot,
        cand_canary.lower_bound_95,
        promoted,
        &config.pnn_path,
        now_unix,
    )?;

    let receipt_path = attestation_receipt_path(&config.pnn_path);
    let existing_receipts = load_receipt_ledger_if_exists(&receipt_path)?;
    let (next_seq, prev_hash) = if let Some(last) = existing_receipts.last() {
        (last.sequence + 1, last.chain_hash.clone())
    } else {
        (
            0,
            CalibrationAttestationReceipt::GENESIS_PREV_HASH.to_owned(),
        )
    };

    let mut receipt = CalibrationAttestationReceipt::new(
        next_seq,
        prev_hash,
        baseline_network.sha256_digest.clone(),
        candidate_network.sha256_digest.clone(),
        dataset.dpo_dataset_digest.clone(),
        cand_canary.lower_bound_95,
        promoted,
    )
    .with_distillation_metadata(
        Some(capsule_package.capsule.sha256_digest.clone()),
        Some(drift_report.sinkhorn_divergence),
        Some(drift_report.wasserstein_2_distance),
        Some(martingale_snapshot.e_value),
        Some(martingale_snapshot.verdict.as_str().to_owned()),
    );
    receipt.sign_ed25519(&identity.signing_key);

    let capsule_path = config
        .capsule_out_path
        .clone()
        .unwrap_or_else(|| default_capsule_path(&config.pnn_path));
    let telemetry_path = config
        .telemetry_path
        .clone()
        .or_else(|| config.data_dir.as_deref().map(default_telemetry_path));

    let prev_telemetry = telemetry_path
        .as_deref()
        .filter(|p| p.is_file())
        .and_then(|p| load_telemetry_snapshot(p).ok())
        .unwrap_or_default();

    let wasserstein_summary = WassersteinTelemetrySummary::new(
        drift_report.sinkhorn_divergence,
        drift_report.wasserstein_2_distance,
        drift_report.birkhoff_contraction_factor,
        drift_report.theoretical_entropic_bias_bound,
        drift_report.drift_threshold,
        drift_report.drift_detected,
        drift_report.iterations,
        drift_report.final_dual_residual,
        drift_report.marginal_conservation_error,
    )
    .map_err(|e| DistillationError::Contract(e.to_string()))?;

    let martingale_summary = sanitize_martingale_summary(&martingale_snapshot)?;

    let active_pnn_digest = if promoted {
        Some(candidate_network.sha256_digest.clone())
    } else {
        prev_telemetry
            .active_pnn_digest
            .clone()
            .or_else(|| Some(baseline_network.sha256_digest.clone()))
    };
    let next_reloads = if promoted
        && !config.dry_run
        && candidate_network.sha256_digest != baseline_network.sha256_digest
    {
        prev_telemetry.pnn_hot_reloads.saturating_add(1)
    } else {
        prev_telemetry.pnn_hot_reloads
    };

    let telemetry_snapshot = DistillationTelemetrySnapshot::new()
        .with_runtime_counters(
            next_reloads,
            prev_telemetry.lats_memo_hits,
            active_pnn_digest,
        )
        .with_distillation_artifacts(
            dataset.pairs.len(),
            if promoted {
                Some(capsule_package.capsule.capsule_id.clone())
            } else {
                prev_telemetry.last_capsule_id
            },
            if promoted {
                Some(capsule_package.capsule.sha256_digest.clone())
            } else {
                prev_telemetry.last_capsule_digest
            },
            if promoted {
                Some(receipt.commitment.clone())
            } else {
                prev_telemetry.last_attestation_commitment
            },
            promoted && receipt.verify_ed25519(None),
        )
        .with_phase_metrics(
            Some(wasserstein_summary),
            Some(martingale_summary),
            now_unix,
        );
    telemetry_snapshot
        .validate()
        .map_err(|e| DistillationError::Contract(e.to_string()))?;

    if promoted && !config.dry_run {
        let pnn_bytes = serde_json::to_vec_pretty(&candidate_network)
            .map_err(|e| DistillationError::Serialization(e.to_string()))?;
        atomic_write_bytes(&config.pnn_path, &pnn_bytes)?;

        let capsule_bytes = serde_json::to_vec_pretty(&capsule_package)
            .map_err(|e| DistillationError::Serialization(e.to_string()))?;
        atomic_write_bytes(&capsule_path, &capsule_bytes)?;

        if config.store_in_session_db
            && let Some(ref data_dir) = config.data_dir
        {
            let store = SessionStore::open(data_dir.join("sessions.sqlite3"))
                .map_err(|e| DistillationError::Session(e.to_string()))?;
            store
                .store_mesh_capsule(&capsule_package)
                .map_err(|e| DistillationError::Session(e.to_string()))?;
        }

        let mut all_receipts = existing_receipts;
        all_receipts.push(receipt.clone());
        let mut ledger_buf = String::new();
        for item in &all_receipts {
            let line = serde_json::to_string(item)
                .map_err(|e| DistillationError::Serialization(e.to_string()))?;
            ledger_buf.push_str(&line);
            ledger_buf.push('\n');
        }
        atomic_write_bytes(&receipt_path, ledger_buf.as_bytes())?;

        if let Some(ref tpath) = telemetry_path {
            save_telemetry_snapshot(tpath, &telemetry_snapshot)?;
        }
    }

    Ok(CrossSessionDistillResult {
        pnn_path: config.pnn_path,
        receipt_path,
        capsule_path,
        telemetry_path,
        source_paths: dataset.source_paths,
        per_source_counts: dataset.per_source_counts,
        pairs_count: dataset.pairs.len(),
        duplicates_removed: dataset.duplicates_removed,
        dpo_dataset_digest: dataset.dpo_dataset_digest,
        baseline_digest: baseline_network.sha256_digest,
        candidate_digest: candidate_network.sha256_digest,
        baseline_accuracy: base_report.branch_selection_accuracy,
        candidate_accuracy: cand_report.branch_selection_accuracy,
        baseline_mean_margin: base_report.mean_margin,
        candidate_mean_margin: cand_report.mean_margin,
        bayesian_lower_bound_95: cand_canary.lower_bound_95,
        bayesian_promoted: cand_canary.promoted,
        non_regression_passed,
        drift_report,
        martingale_snapshot,
        promoted,
        dry_run: config.dry_run,
        capsule_package,
        receipt,
        telemetry_snapshot,
    })
}

/// Fail-closed verification of a `.pnn` checkpoint, its hash-chained & Ed25519-signed
/// `CalibrationAttestationReceipt` ledger, and an optional/linked `SignedCapsulePackage`.
pub fn verify_pnn_and_capsule_attestation(
    config: AttestationVerifyConfig,
) -> Result<AttestationVerificationReport, DistillationError> {
    let receipt_path = config
        .receipt_path
        .clone()
        .unwrap_or_else(|| attestation_receipt_path(&config.pnn_path));

    if !receipt_path.is_file() {
        return Err(DistillationError::VerificationFailed(format!(
            "calibration attestation ledger not found at {}",
            receipt_path.display()
        )));
    }

    let receipts = load_receipt_ledger_strict(&receipt_path)?;
    if receipts.is_empty() {
        return Err(DistillationError::VerificationFailed(format!(
            "calibration attestation ledger is empty at {}",
            receipt_path.display()
        )));
    }

    if !CalibrationAttestationReceipt::verify_chain(&receipts) {
        return Err(DistillationError::VerificationFailed(format!(
            "calibration attestation hash-chain verification failed at {}",
            receipt_path.display()
        )));
    }

    let has_any_signature = receipts
        .iter()
        .any(|r| r.signature.is_some() || r.signer_public_key.is_some());
    let signature_verified =
        if config.require_signature || has_any_signature || config.expected_pubkey_hex.is_some() {
            if !CalibrationAttestationReceipt::verify_signed_chain(
                &receipts,
                config.expected_pubkey_hex.as_deref(),
            ) {
                return Err(DistillationError::VerificationFailed(format!(
                    "Ed25519 signature verification failed for attestation ledger {}",
                    receipt_path.display()
                )));
            }
            true
        } else {
            false
        };

    if !config.pnn_path.is_file() {
        return Err(DistillationError::VerificationFailed(format!(
            ".pnn checkpoint not found at {}",
            config.pnn_path.display()
        )));
    }
    let net = TropicalNetwork::load_from_file(&config.pnn_path).map_err(|e| {
        DistillationError::VerificationFailed(format!(
            "failed .pnn integrity check at {}: {e}",
            config.pnn_path.display()
        ))
    })?;

    let latest = receipts
        .last()
        .ok_or_else(|| DistillationError::VerificationFailed("empty receipt chain".to_owned()))?;

    let pnn_digest_matches = net.sha256_digest == latest.candidate_digest;
    if !pnn_digest_matches {
        return Err(DistillationError::VerificationFailed(format!(
            ".pnn digest mismatch: active={} != attested={}",
            net.sha256_digest, latest.candidate_digest
        )));
    }

    let effective_capsule_path = config.capsule_path.clone().or_else(|| {
        let candidate = default_capsule_path(&config.pnn_path);
        candidate.is_file().then_some(candidate)
    });

    let (capsule_verified, capsule_digest_matches, verified_capsule_digest) =
        if let Some(ref cpath) = effective_capsule_path {
            if !cpath.is_file() {
                return Err(DistillationError::VerificationFailed(format!(
                    "MemoryCapsule package not found at {}",
                    cpath.display()
                )));
            }
            let raw = fs::read_to_string(cpath).map_err(|e| {
                DistillationError::VerificationFailed(format!(
                    "failed reading MemoryCapsule at {}: {e}",
                    cpath.display()
                ))
            })?;
            let package: SignedCapsulePackage = serde_json::from_str(&raw).map_err(|e| {
                DistillationError::VerificationFailed(format!(
                    "malformed SignedCapsulePackage at {}: {e}",
                    cpath.display()
                ))
            })?;
            MeshSyncEngine::validate_capsule_package(&package, &HashMap::new()).map_err(|e| {
                DistillationError::VerificationFailed(format!(
                    "MemoryCapsule cryptographic/DAG validation failed at {}: {e}",
                    cpath.display()
                ))
            })?;
            let Some(ref expected_cap_digest) = latest.capsule_digest else {
                return Err(DistillationError::VerificationFailed(
                    "latest attestation receipt does not bind a capsule_digest".to_owned(),
                ));
            };
            if &package.capsule.sha256_digest != expected_cap_digest {
                return Err(DistillationError::VerificationFailed(format!(
                    "MemoryCapsule digest mismatch: capsule={} != attested={}",
                    package.capsule.sha256_digest, expected_cap_digest
                )));
            }
            (true, Some(true), Some(package.capsule.sha256_digest))
        } else if config.require_capsule {
            return Err(DistillationError::VerificationFailed(
                "MemoryCapsule verification required but no capsule file found".to_owned(),
            ));
        } else {
            (false, None, latest.capsule_digest.clone())
        };

    Ok(AttestationVerificationReport {
        valid: true,
        pnn_path: config.pnn_path,
        receipt_path,
        capsule_path: effective_capsule_path,
        chain_length: receipts.len(),
        latest_sequence: latest.sequence,
        latest_commitment: latest.commitment.clone(),
        latest_chain_hash: latest.chain_hash.clone(),
        pnn_digest: net.sha256_digest,
        expected_candidate_digest: latest.candidate_digest.clone(),
        pnn_digest_matches,
        signature_verified,
        signer_public_key: latest.signer_public_key.clone(),
        capsule_digest: verified_capsule_digest,
        capsule_verified,
        capsule_digest_matches,
        promoted: latest.promoted,
        sinkhorn_divergence: latest.sinkhorn_divergence,
        wasserstein_2_distance: latest.wasserstein_2_distance,
        martingale_e_value: latest.martingale_e_value,
        martingale_verdict: latest.martingale_verdict.clone(),
    })
}

struct SuiteEvaluationOutputs {
    report: pandora_harnesses::decision_benchmark::DriverBenchmarkMetrics,
    embeddings: Vec<Vec<f64>>,
    losses: Vec<f64>,
}

fn evaluate_suite_with_embeddings(
    suite: &DecisionBenchmarkSuite,
    driver_name: &str,
    driver: &MoiraDecisionDriver,
) -> Result<SuiteEvaluationOutputs, DistillationError> {
    let report = suite
        .evaluate_driver(driver_name, |q| driver.evaluate_question(q))
        .map_err(|e| DistillationError::Decision(e.to_string()))?;

    let mut embeddings = Vec::with_capacity(suite.traces.len());
    let mut losses = Vec::with_capacity(suite.traces.len());

    for (trace, rec) in suite.traces.iter().zip(report.records.iter()) {
        let verdict = driver
            .evaluate_question(&trace.question)
            .map_err(|e| DistillationError::Decision(e.to_string()))?;
        let expected_prob = verdict
            .probabilities
            .get(&trace.expected_label)
            .copied()
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);
        let probs: Vec<f64> = verdict.probabilities.values().copied().collect();
        let entropy = MoiraDecisionDriver::normalized_entropy(&probs);
        let correct_flag = if rec.is_correct { 1.0 } else { 0.0 };

        embeddings.push(vec![
            verdict.confidence.clamp(0.0, 1.0),
            verdict.margin.clamp(0.0, 1.0),
            expected_prob,
            entropy.clamp(0.0, 1.0),
            correct_flag,
        ]);

        let loss =
            (0.5 * (1.0 - expected_prob) + if rec.is_correct { 0.0 } else { 0.5 }).clamp(0.0, 1.0);
        losses.push(loss);
    }

    Ok(SuiteEvaluationOutputs {
        report,
        embeddings,
        losses,
    })
}

fn resolve_peer_identity(
    key_file: Option<&Path>,
    data_dir: Option<&Path>,
    dry_run: bool,
) -> Result<PeerIdentity, DistillationError> {
    let target_key_path = key_file
        .map(PathBuf::from)
        .or_else(|| data_dir.map(|d| d.join("mesh").join("peer_identity.key")));

    if let Some(ref path) = target_key_path {
        if path.exists() {
            let raw = fs::read_to_string(path).map_err(|e| DistillationError::Io(e.to_string()))?;
            let bytes = decode_key_32(raw.trim()).ok_or_else(|| {
                DistillationError::InvalidKeyFile(format!(
                    "key file {} does not contain 32-byte hex Ed25519 seed",
                    path.display()
                ))
            })?;
            let signing_key = SigningKey::from_bytes(&bytes);
            let verifying_key = signing_key.verifying_key();
            let pub_hex = encode_hex(&verifying_key.to_bytes());
            let peer_id = format!("peer-{}", &pub_hex[..16]);
            return Ok(PeerIdentity {
                signing_key,
                verifying_key,
                peer_id,
            });
        }
        let identity = PeerIdentity::new_random();
        if !dry_run {
            let hex_private = encode_hex(&identity.signing_key.to_bytes());
            atomic_write_bytes(path, hex_private.as_bytes())?;
        }
        return Ok(identity);
    }

    Ok(PeerIdentity::new_random())
}

#[allow(clippy::too_many_arguments)]
fn build_signed_distillation_capsule(
    identity: &PeerIdentity,
    author_override: Option<&str>,
    dataset: &AggregatedDpoDataset,
    baseline: &TropicalNetwork,
    candidate: &TropicalNetwork,
    drift_report: &SinkhornDriftReport,
    martingale: &MartingaleGateSnapshot,
    bayesian_lb95: f64,
    promoted: bool,
    pnn_path: &Path,
    now_unix: u64,
) -> Result<SignedCapsulePackage, DistillationError> {
    let short_dpo = &dataset.dpo_dataset_digest[..12.min(dataset.dpo_dataset_digest.len())];
    let short_cand = &candidate.sha256_digest[..12.min(candidate.sha256_digest.len())];
    let sop_id = format!("sop-distill-{short_dpo}");
    let sop_name = format!("cross-session-pnn-distill-{short_cand}");

    let step1 = SopStep::new(
        "aggregate-dpo",
        "Aggregate cross-session DPO pairs",
        format!(
            "aggregate {} pairs across {} sources (digest={})",
            dataset.pairs.len(),
            dataset.source_paths.len(),
            dataset.dpo_dataset_digest
        ),
        vec![],
    )
    .map_err(|e| DistillationError::Mesh(e.to_string()))?;

    let step2 = SopStep::new(
        "calibrate-pnn",
        "Calibrate TropicalNetwork checkpoint",
        format!(
            "calibrate baseline={} -> candidate={} (bayesian_lb95={:.6})",
            baseline.sha256_digest, candidate.sha256_digest, bayesian_lb95
        ),
        vec!["aggregate-dpo".to_owned()],
    )
    .map_err(|e| DistillationError::Mesh(e.to_string()))?;

    let step3 = SopStep::new(
        "gate-verification",
        "Evaluate Phase 13 Wasserstein & Phase 14 Martingale gates",
        format!(
            "sinkhorn={:.6}, w2={:.6}, drift={}, martingale_e={:.6}, verdict={}, promoted={}",
            drift_report.sinkhorn_divergence,
            drift_report.wasserstein_2_distance,
            drift_report.drift_detected,
            martingale.e_value,
            martingale.verdict.as_str(),
            promoted
        ),
        vec!["calibrate-pnn".to_owned()],
    )
    .map_err(|e| DistillationError::Mesh(e.to_string()))?;

    let sop = SopDefinition::new(
        sop_id,
        sop_name,
        MemoryIntent::Procedure,
        vec![step1, step2, step3],
        Some(format!(
            "pandora fleet verify-attestation --pnn {}",
            pnn_path.display()
        )),
    )
    .map_err(|e| DistillationError::Mesh(e.to_string()))?;

    let author = author_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&identity.peer_id);

    let capsule = MemoryCapsule::new(
        format!("capsule-distill-{short_cand}-{now_unix}"),
        sop,
        author,
        Timestamp::from_unix_seconds(now_unix),
    )
    .map_err(|e| DistillationError::Mesh(e.to_string()))?;

    let author_public_key = identity.public_key_hex();
    let author_signature = identity.sign_digest(&capsule.sha256_digest);
    let package = SignedCapsulePackage {
        capsule,
        author_public_key,
        author_signature,
        parent_capsule_ids: Vec::new(),
        gossip_hops: 0,
    };

    MeshSyncEngine::validate_capsule_package(&package, &HashMap::new())
        .map_err(|e| DistillationError::Mesh(e.to_string()))?;

    Ok(package)
}

fn load_receipt_ledger_if_exists(
    receipt_path: &Path,
) -> Result<Vec<CalibrationAttestationReceipt>, DistillationError> {
    if !receipt_path.is_file() {
        return Ok(Vec::new());
    }
    let receipts = load_receipt_ledger_strict(receipt_path)?;
    if !receipts.is_empty() && !CalibrationAttestationReceipt::verify_chain(&receipts) {
        return Err(DistillationError::VerificationFailed(format!(
            "existing attestation ledger failed hash-chain verification at {}",
            receipt_path.display()
        )));
    }
    for r in &receipts {
        if (r.signature.is_some() || r.signer_public_key.is_some()) && !r.verify_ed25519(None) {
            return Err(DistillationError::VerificationFailed(format!(
                "existing signed receipt seq={} failed Ed25519 verification at {}",
                r.sequence,
                receipt_path.display()
            )));
        }
    }
    Ok(receipts)
}

fn load_receipt_ledger_strict(
    receipt_path: &Path,
) -> Result<Vec<CalibrationAttestationReceipt>, DistillationError> {
    let raw = fs::read_to_string(receipt_path).map_err(|e| {
        DistillationError::Io(format!(
            "failed reading calibration attestation ledger {}: {e}",
            receipt_path.display()
        ))
    })?;
    let mut receipts = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        let receipt: CalibrationAttestationReceipt = serde_json::from_str(l).map_err(|e| {
            DistillationError::VerificationFailed(format!(
                "malformed calibration attestation receipt at {}:{}: {e}",
                receipt_path.display(),
                idx + 1
            ))
        })?;
        receipts.push(receipt);
    }
    Ok(receipts)
}

fn sanitize_martingale_summary(
    snap: &MartingaleGateSnapshot,
) -> Result<MartingaleTelemetrySummary, DistillationError> {
    let radius = if snap.confidence_radius.is_finite() {
        snap.confidence_radius
    } else {
        1.0
    };
    let lower = if snap.lower_confidence_bound.is_finite() {
        snap.lower_confidence_bound
    } else {
        snap.empirical_mean - radius
    };
    let upper = if snap.upper_confidence_bound.is_finite() {
        snap.upper_confidence_bound
    } else {
        snap.empirical_mean + radius
    };
    MartingaleTelemetrySummary::new(
        snap.step_t,
        snap.e_value,
        snap.peak_e_value,
        snap.anytime_p_value,
        snap.ville_rejection_threshold,
        snap.empirical_mean,
        radius,
        lower,
        upper,
        snap.betting_fraction_lambda,
        snap.verdict.as_str(),
    )
    .map_err(|e| DistillationError::Contract(e.to_string()))
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use fmt::Write;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

fn decode_key_32(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    let bytes = hex.as_bytes();
    for i in 0..32 {
        let hi = match bytes[i * 2] {
            b'0'..=b'9' => bytes[i * 2] - b'0',
            b'a'..=b'f' => bytes[i * 2] - b'a' + 10,
            b'A'..=b'F' => bytes[i * 2] - b'A' + 10,
            _ => return None,
        };
        let lo = match bytes[i * 2 + 1] {
            b'0'..=b'9' => bytes[i * 2 + 1] - b'0',
            b'a'..=b'f' => bytes[i * 2 + 1] - b'a' + 10,
            b'A'..=b'F' => bytes[i * 2 + 1] - b'A' + 10,
            _ => return None,
        };
        out[i] = (hi << 4) | lo;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_test_dir(prefix: &str) -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "pandora_m2_distill_{prefix}_{}_{}",
            std::process::id(),
            id
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_jsonl_pairs(path: &Path, pairs: &[DpoPair]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut content = String::new();
        for p in pairs {
            content.push_str(&serde_json::to_string(p).unwrap());
            content.push('\n');
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn receipt_ed25519_signing_preserves_legacy_compat_and_rejects_all_tampering() {
        let id = PeerIdentity::new_random();
        let other = PeerIdentity::new_random();

        // Legacy unsigned receipt deserializes and passes verify() / verify_chain()
        let legacy = CalibrationAttestationReceipt::new(
            0,
            CalibrationAttestationReceipt::GENESIS_PREV_HASH,
            "base-digest",
            "cand-digest",
            "dpo-digest",
            0.75,
            true,
        );
        let legacy_json = serde_json::json!({
            "sequence": legacy.sequence,
            "prev_receipt_hash": legacy.prev_receipt_hash,
            "baseline_digest": legacy.baseline_digest,
            "candidate_digest": legacy.candidate_digest,
            "dpo_dataset_digest": legacy.dpo_dataset_digest,
            "lower_bound_95": legacy.lower_bound_95,
            "lower_bound_95_bits": legacy.lower_bound_95_bits,
            "commitment": legacy.commitment,
            "chain_hash": legacy.chain_hash,
            "promoted": legacy.promoted,
        })
        .to_string();
        let parsed_legacy: CalibrationAttestationReceipt =
            serde_json::from_str(&legacy_json).unwrap();
        assert!(parsed_legacy.verify());
        assert!(CalibrationAttestationReceipt::verify_chain(
            std::slice::from_ref(&parsed_legacy)
        ));
        assert!(!parsed_legacy.verify_ed25519(None));

        // Signed receipt with capsule + Phase 13/14 fields
        let mut r0 = CalibrationAttestationReceipt::new(
            0,
            CalibrationAttestationReceipt::GENESIS_PREV_HASH,
            "base-0",
            "cand-0",
            "dpo-0",
            0.81,
            true,
        )
        .with_distillation_metadata(
            Some("sha256:cap0".to_owned()),
            Some(0.012),
            Some(0.11),
            Some(0.95),
            Some("continue_monitoring".to_owned()),
        );
        r0.sign_ed25519(&id.signing_key);
        assert!(r0.verify());
        assert!(r0.verify_ed25519(Some(&id.public_key_hex())));
        assert!(!r0.verify_ed25519(Some(&other.public_key_hex())));

        let mut r1 = CalibrationAttestationReceipt::new(
            1,
            r0.chain_hash.clone(),
            "cand-0",
            "cand-1",
            "dpo-1",
            0.84,
            true,
        )
        .with_distillation_metadata(
            Some("sha256:cap1".to_owned()),
            Some(0.010),
            Some(0.10),
            Some(0.88),
            Some("promote_canary".to_owned()),
        );
        r1.sign_ed25519(&id.signing_key);
        assert!(CalibrationAttestationReceipt::verify_signed_chain(
            &[r0.clone(), r1.clone()],
            Some(&id.public_key_hex())
        ));
        assert!(!CalibrationAttestationReceipt::verify_signed_chain(
            &[],
            None
        ));

        // Tamper capsule_digest -> rejected
        let mut tampered_cap = r0.clone();
        tampered_cap.capsule_digest = Some("sha256:forged".to_owned());
        assert!(!tampered_cap.verify_ed25519(None));

        // Tamper Phase 13/14 fields -> rejected
        let mut tampered_w2 = r0.clone();
        tampered_w2.wasserstein_2_distance = Some(0.0);
        assert!(!tampered_w2.verify_ed25519(None));

        let mut tampered_e = r0.clone();
        tampered_e.martingale_e_value = Some(999.0);
        assert!(!tampered_e.verify_ed25519(None));

        let mut tampered_verdict = r0.clone();
        tampered_verdict.martingale_verdict = Some("rollback_regression".to_owned());
        assert!(!tampered_verdict.verify_ed25519(None));

        // Recompute commitment & chain_hash without key -> rejected
        let mut forged = CalibrationAttestationReceipt::new(
            0,
            CalibrationAttestationReceipt::GENESIS_PREV_HASH,
            "base-0",
            "cand-evil",
            "dpo-0",
            0.81,
            true,
        );
        forged.signer_public_key = r0.signer_public_key.clone();
        forged.signature = r0.signature.clone();
        assert!(forged.verify()); // unkeyed SHA-256 chain still consistent
        assert!(!forged.verify_ed25519(None)); // Ed25519 strictly rejects

        // Broken chain sequence -> rejected
        let mut reordered = vec![r1, r0];
        assert!(!CalibrationAttestationReceipt::verify_signed_chain(
            &reordered, None
        ));
        reordered.clear();
    }

    #[test]
    fn discover_and_aggregate_dpo_pairs_deduplicates_deterministically() {
        let root = temp_test_dir("discover");
        let ws = root.join("workspace");
        let data_dir = root.join("data");
        let extra_session = root.join("external_session");
        let direct_file = root.join("custom_pairs.json");

        let p1 = MoiraDecisionDriver::emit_dpo_pair_from_escalation(
            "pair-1",
            "Execute rm -rf /var/lib/postgresql",
            "reject",
            "allow",
            0.12,
        );
        let p2 = MoiraDecisionDriver::emit_dpo_pair_from_escalation(
            "pair-2",
            "Read workspace Cargo.toml status",
            "allow",
            "reject",
            0.15,
        );
        let p3 = MoiraDecisionDriver::emit_dpo_pair_from_escalation(
            "pair-3",
            "Refactor distributed kernel security boundaries",
            "deep",
            "fast",
            0.10,
        );

        write_jsonl_pairs(
            &ws.join(".pandora/dpo_pairs.jsonl"),
            std::slice::from_ref(&p1),
        );
        write_jsonl_pairs(
            &data_dir.join("sessions/sess-a/dpo_pairs.jsonl"),
            &[p1.clone(), p2.clone()],
        );
        write_jsonl_pairs(
            &extra_session.join(".pandora/dpo_pairs.jsonl"),
            &[p2.clone(), p3.clone()],
        );
        fs::write(
            &direct_file,
            serde_json::to_string_pretty(&vec![p3]).unwrap(),
        )
        .unwrap();

        let sources = discover_dpo_sources(
            &ws,
            Some(&data_dir),
            &[extra_session.clone(), direct_file.clone()],
        );
        assert_eq!(sources.len(), 4);

        let dataset_a = aggregate_dpo_pairs(&sources).unwrap();
        let mut reversed = sources.clone();
        reversed.reverse();
        let dataset_b = aggregate_dpo_pairs(&reversed).unwrap();

        assert_eq!(dataset_a.pairs.len(), 3);
        assert_eq!(dataset_a.duplicates_removed, 3);
        assert_eq!(dataset_a.dpo_dataset_digest, dataset_b.dpo_dataset_digest);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn distill_and_verify_fail_closed_on_tampered_artifacts() {
        let root = temp_test_dir("distill_verify");
        let ws = root.join("workspace");
        let data_dir = root.join("data");
        let key_file = data_dir.join("mesh/distiller.key");
        let pnn_path = ws.join(".pandora/model_decision.pnn");

        let p1 = MoiraDecisionDriver::emit_dpo_pair_from_escalation(
            "dpo-1",
            "Execute destructive rm -rf /var/lib/postgresql on production database host",
            "reject",
            "allow",
            0.10,
        );
        let p2 = MoiraDecisionDriver::emit_dpo_pair_from_escalation(
            "dpo-2",
            "Read and inspect Cargo.toml workspace manifest for version verification",
            "allow",
            "reject",
            0.12,
        );
        let p3 = MoiraDecisionDriver::emit_dpo_pair_from_escalation(
            "dpo-3",
            "Select execution tier to refactor distributed consensus kernel and security boundaries",
            "deep",
            "fast",
            0.11,
        );

        write_jsonl_pairs(&ws.join(".pandora/dpo_pairs.jsonl"), &[p1]);
        write_jsonl_pairs(&data_dir.join("sessions/s1/dpo_pairs.jsonl"), &[p2, p3]);

        let cfg = CrossSessionDistillConfig::new(&ws)
            .with_data_dir(&data_dir)
            .with_key_file(&key_file)
            .with_pnn_path(&pnn_path)
            .with_store_in_session_db(true);

        let res = distill_cross_session_checkpoint(cfg).unwrap();
        assert!(res.promoted);
        assert!(res.pnn_path.is_file());
        assert!(res.receipt_path.is_file());
        assert!(res.capsule_path.is_file());
        assert!(res.telemetry_path.as_ref().unwrap().is_file());
        assert!(res.drift_report.sinkhorn_divergence >= 0.0);
        assert!(!res.drift_report.drift_detected);
        assert_ne!(
            res.martingale_snapshot.verdict,
            CanaryGateVerdict::RollbackRegression
        );
        assert!(res.telemetry_snapshot.last_attestation_verified);
        assert_eq!(res.telemetry_snapshot.dpo_pairs_total, 3);

        // Verify in SessionStore as well
        let store = SessionStore::open(data_dir.join("sessions.sqlite3")).unwrap();
        let stored_capsules = store.list_mesh_capsules().unwrap();
        assert_eq!(stored_capsules.len(), 1);
        assert_eq!(
            stored_capsules[0].capsule.sha256_digest,
            res.capsule_package.capsule.sha256_digest
        );

        let pubkey_hex = res.receipt.signer_public_key.clone().unwrap();

        // 1. Valid verification succeeds
        let ok_report = verify_pnn_and_capsule_attestation(
            AttestationVerifyConfig::new(&pnn_path)
                .with_capsule_path(&res.capsule_path)
                .with_expected_pubkey_hex(&pubkey_hex)
                .with_require_signature(true)
                .with_require_capsule(true),
        )
        .unwrap();
        assert!(ok_report.valid);
        assert!(ok_report.pnn_digest_matches);
        assert!(ok_report.signature_verified);
        assert!(ok_report.capsule_verified);

        // 2. Wrong public key fails closed
        let wrong_id = PeerIdentity::new_random();
        assert!(
            verify_pnn_and_capsule_attestation(
                AttestationVerifyConfig::new(&pnn_path)
                    .with_expected_pubkey_hex(wrong_id.public_key_hex())
                    .with_require_signature(true),
            )
            .is_err()
        );

        // 3. Tampered .capsule.json fails closed
        let orig_capsule = fs::read_to_string(&res.capsule_path).unwrap();
        let mut cap_pkg: SignedCapsulePackage = serde_json::from_str(&orig_capsule).unwrap();
        cap_pkg.capsule.author = "attacker".to_owned();
        fs::write(
            &res.capsule_path,
            serde_json::to_string_pretty(&cap_pkg).unwrap(),
        )
        .unwrap();
        assert!(
            verify_pnn_and_capsule_attestation(
                AttestationVerifyConfig::new(&pnn_path)
                    .with_capsule_path(&res.capsule_path)
                    .with_require_signature(true),
            )
            .is_err()
        );
        fs::write(&res.capsule_path, orig_capsule).unwrap();

        // 4. Tampered .pnn fails closed
        let orig_pnn = fs::read_to_string(&pnn_path).unwrap();
        let mut net: TropicalNetwork = serde_json::from_str(&orig_pnn).unwrap();
        net.layers[0].weights[0] += 1.0;
        fs::write(&pnn_path, serde_json::to_string_pretty(&net).unwrap()).unwrap();
        assert!(
            verify_pnn_and_capsule_attestation(
                AttestationVerifyConfig::new(&pnn_path).with_require_signature(true),
            )
            .is_err()
        );
        fs::write(&pnn_path, orig_pnn).unwrap();

        // 5. Tampered / unsigned receipt fails closed
        let orig_receipts = fs::read_to_string(&res.receipt_path).unwrap();
        let mut tampered_r = res.receipt.clone();
        tampered_r.signature = None;
        fs::write(
            &res.receipt_path,
            format!("{}\n", serde_json::to_string(&tampered_r).unwrap()),
        )
        .unwrap();
        assert!(
            verify_pnn_and_capsule_attestation(
                AttestationVerifyConfig::new(&pnn_path).with_require_signature(true),
            )
            .is_err()
        );
        fs::write(&res.receipt_path, orig_receipts).unwrap();

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn distill_blocks_promotion_on_poisoned_pairs_and_strict_drift_threshold() {
        let root = temp_test_dir("reject");
        let ws = root.join("workspace");
        let data_dir = root.join("data");
        let pnn_path = ws.join(".pandora/model_decision.pnn");

        // Poisoned pairs trying to invert safety & routing decisions aggressively
        let mut poisoned = Vec::new();
        for i in 0..16 {
            poisoned.push(MoiraDecisionDriver::emit_dpo_pair_from_escalation(
                format!("poison-{i}"),
                "Execute destructive rm -rf /var/lib/postgresql on production database host",
                "allow",
                "reject",
                0.01,
            ));
        }
        write_jsonl_pairs(&ws.join(".pandora/dpo_pairs.jsonl"), &poisoned);

        let cfg = CrossSessionDistillConfig::new(&ws)
            .with_data_dir(&data_dir)
            .with_pnn_path(&pnn_path)
            .with_learning_rate(0.25)
            .with_drift_threshold(0.0001);

        let res = distill_cross_session_checkpoint(cfg).unwrap();
        assert!(!res.promoted);
        assert!(!pnn_path.exists());
        assert!(!res.receipt_path.exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn distill_multi_step_chain_dry_run_and_telemetry_preservation() {
        let root = temp_test_dir("multi_step");
        let ws = root.join("workspace");
        let data_dir = root.join("data");
        let pnn_path = ws.join(".pandora/model_decision.pnn");
        let tpath = default_telemetry_path(&data_dir);

        let initial_snap = DistillationTelemetrySnapshot::new().with_runtime_counters(4, 29, None);
        save_telemetry_snapshot(&tpath, &initial_snap).unwrap();

        let p1 = MoiraDecisionDriver::emit_dpo_pair_from_escalation(
            "m-1",
            "Read and inspect Cargo.toml workspace manifest for version verification",
            "allow",
            "reject",
            0.12,
        );
        write_jsonl_pairs(&ws.join(".pandora/dpo_pairs.jsonl"), &[p1]);

        // Dry run must NOT persist files
        let dry_res = distill_cross_session_checkpoint(
            CrossSessionDistillConfig::new(&ws)
                .with_data_dir(&data_dir)
                .with_pnn_path(&pnn_path)
                .with_dry_run(true),
        )
        .unwrap();
        assert!(dry_res.promoted);
        assert!(dry_res.dry_run);
        assert!(!pnn_path.exists());
        assert!(!dry_res.receipt_path.exists());
        assert!(!dry_res.capsule_path.exists());

        // Step 0 commit
        let r0 = distill_cross_session_checkpoint(
            CrossSessionDistillConfig::new(&ws)
                .with_data_dir(&data_dir)
                .with_pnn_path(&pnn_path),
        )
        .unwrap();
        assert!(r0.promoted);
        assert_eq!(r0.receipt.sequence, 0);
        assert_eq!(r0.telemetry_snapshot.lats_memo_hits, 29);

        // Step 1 incremental session distillation
        let p2 = MoiraDecisionDriver::emit_dpo_pair_from_escalation(
            "m-2",
            "Execute destructive rm -rf /var/lib/postgresql on production database host",
            "reject",
            "allow",
            0.09,
        );
        write_jsonl_pairs(&data_dir.join("sessions/s2/dpo_pairs.jsonl"), &[p2]);

        let r1 = distill_cross_session_checkpoint(
            CrossSessionDistillConfig::new(&ws)
                .with_data_dir(&data_dir)
                .with_pnn_path(&pnn_path),
        )
        .unwrap();
        assert!(r1.promoted);
        assert_eq!(r1.receipt.sequence, 1);
        assert_eq!(r1.receipt.prev_receipt_hash, r0.receipt.chain_hash);
        assert_eq!(r1.telemetry_snapshot.lats_memo_hits, 29);

        let report = verify_pnn_and_capsule_attestation(
            AttestationVerifyConfig::new(&pnn_path)
                .with_expected_pubkey_hex(r1.receipt.signer_public_key.as_deref().unwrap())
                .with_require_signature(true)
                .with_require_capsule(true),
        )
        .unwrap();
        assert_eq!(report.chain_length, 2);
        assert_eq!(report.latest_sequence, 1);
        assert!(report.capsule_verified);

        // Missing .pnn fails closed
        let orig_pnn = fs::read(&pnn_path).unwrap();
        fs::remove_file(&pnn_path).unwrap();
        assert!(
            verify_pnn_and_capsule_attestation(
                AttestationVerifyConfig::new(&pnn_path).with_require_signature(true),
            )
            .is_err()
        );
        fs::write(&pnn_path, orig_pnn).unwrap();

        // Broken hash chain (swapping line order) fails closed
        let ledger = fs::read_to_string(&r1.receipt_path).unwrap();
        let lines: Vec<&str> = ledger.lines().collect();
        assert_eq!(lines.len(), 2);
        fs::write(&r1.receipt_path, format!("{}\n{}\n", lines[1], lines[0])).unwrap();
        assert!(
            verify_pnn_and_capsule_attestation(
                AttestationVerifyConfig::new(&pnn_path).with_require_signature(true),
            )
            .is_err()
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn aggregate_and_distill_reject_invalid_sources_and_corrupt_key() {
        let root = temp_test_dir("invalid_inputs");
        let ws = root.join("workspace");
        fs::create_dir_all(ws.join(".pandora")).unwrap();

        // Empty sources
        assert!(matches!(
            aggregate_dpo_pairs(&[]),
            Err(DistillationError::NoDpoSources)
        ));

        // Invalid DPO pair (empty prompt)
        let bad_file = root.join("bad.jsonl");
        fs::write(
            &bad_file,
            r#"{"id":"x","prompt":"   ","chosen":"allow","rejected":"reject","margin":0.5}"#,
        )
        .unwrap();
        assert!(matches!(
            aggregate_dpo_pairs(&[bad_file]),
            Err(DistillationError::InvalidDpoPair { .. })
        ));

        // Corrupt key file fails closed without overwriting
        let good_pair = MoiraDecisionDriver::emit_dpo_pair_from_escalation(
            "ok-1",
            "Read Cargo.toml",
            "allow",
            "reject",
            0.1,
        );
        write_jsonl_pairs(&ws.join(".pandora/dpo_pairs.jsonl"), &[good_pair]);
        let bad_key = root.join("bad.key");
        fs::write(&bad_key, "not-valid-hex").unwrap();
        let err = distill_cross_session_checkpoint(
            CrossSessionDistillConfig::new(&ws).with_key_file(&bad_key),
        )
        .unwrap_err();
        assert!(matches!(err, DistillationError::InvalidKeyFile(_)));
        assert_eq!(fs::read_to_string(&bad_key).unwrap(), "not-valid-hex");

        // Nonexistent explicit session path fails closed
        let err2 = distill_cross_session_checkpoint(
            CrossSessionDistillConfig::new(&ws)
                .with_session_paths(vec![root.join("does_not_exist")]),
        )
        .unwrap_err();
        assert!(matches!(err2, DistillationError::InvalidSourcePath(_)));

        let _ = fs::remove_dir_all(&root);
    }
}
