pub use pandora_types::{
    ConvergenceChoice, DecisionCandidate, DecisionChoice, DecisionDistribution,
    DecisionDriverConfig, DecisionDriverKind, DecisionEvolutionRecord, DecisionQuestion,
    DecisionVerdict, DpoPair, DynamicVerdict, FineTuningSample, GoldenEvaluationCase,
    GoldenSuiteDefinition, GoldenTargetRef, IntentTierChoice, Timestamp, ToolSafetyChoice,
};
use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecisionError {
    EmptyCandidates,
    InvalidTemperature(String),
    DimensionMismatch,
    CalculationError(String),
    ExecutionFailed(String),
    InvalidSchema(String),
    DriverError(String),
    NetworkError(String),
}

impl fmt::Display for DecisionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyCandidates => write!(f, "decision candidates cannot be empty"),
            Self::InvalidTemperature(msg) => write!(f, "invalid temperature: {msg}"),
            Self::DimensionMismatch => {
                write!(f, "dimension mismatch between candidates and logits")
            }
            Self::CalculationError(msg) => write!(f, "softmax calculation error: {msg}"),
            Self::ExecutionFailed(msg) => write!(f, "decision execution failed: {msg}"),
            Self::InvalidSchema(msg) => write!(f, "invalid question schema: {msg}"),
            Self::DriverError(msg) => write!(f, "decision driver error: {msg}"),
            Self::NetworkError(msg) => write!(f, "decision network error: {msg}"),
        }
    }
}

impl std::error::Error for DecisionError {}

pub trait DecisionDriver: Send + Sync + fmt::Debug {
    fn evaluate_question(
        &self,
        question: &DecisionQuestion,
    ) -> Result<DynamicVerdict, DecisionError>;
}

#[derive(Clone, Debug)]
pub struct HeuristicDecisionDriver {
    default_temperature: f64,
    uncertainty_threshold: f64,
}

impl Default for HeuristicDecisionDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl HeuristicDecisionDriver {
    pub fn new() -> Self {
        Self {
            default_temperature: 1.0,
            uncertainty_threshold: 0.15,
        }
    }

    pub fn with_temperature(temperature: f64) -> Self {
        Self {
            default_temperature: temperature,
            uncertainty_threshold: 0.15,
        }
    }

    pub fn with_options(temperature: f64, uncertainty_threshold: f64) -> Self {
        Self {
            default_temperature: temperature,
            uncertainty_threshold,
        }
    }
}

impl DecisionDriver for HeuristicDecisionDriver {
    fn evaluate_question(
        &self,
        question: &DecisionQuestion,
    ) -> Result<DynamicVerdict, DecisionError> {
        let start = Instant::now();
        if !question.schema.is_valid() {
            return Err(DecisionError::InvalidSchema(
                "invalid question schema".into(),
            ));
        }
        let labels = question.schema.labels();
        let prompt_lower = question.prompt.to_lowercase();
        let state_str = question
            .state_context
            .as_ref()
            .map(|s| s.to_string().to_lowercase())
            .unwrap_or_default();
        let full_context = format!("{prompt_lower} {state_str}");

        let mut raw_logits = Vec::with_capacity(labels.len());
        for label in &labels {
            let l_lower = label.to_lowercase();
            let mut score = 0.0;
            if full_context.contains(&l_lower) {
                score += 3.5;
            }
            if (l_lower == "allow"
                || l_lower == "true"
                || l_lower == "yes"
                || l_lower == "safe"
                || l_lower == "fast")
                && !full_context.contains("danger")
                && !full_context.contains("drop")
                && !full_context.contains("delete")
                && !full_context.contains("rm -rf")
            {
                score += 2.0;
            }
            if (l_lower == "reject"
                || l_lower == "false"
                || l_lower == "no"
                || l_lower == "dangerous")
                && (full_context.contains("danger")
                    || full_context.contains("drop")
                    || full_context.contains("delete")
                    || full_context.contains("rm -rf"))
            {
                score += 5.5;
            }
            raw_logits.push(score);
        }

        if raw_logits
            .iter()
            .all(|s: &f64| (*s - raw_logits[0]).abs() < 1e-6)
        {
            raw_logits[0] += 0.5;
        }

        let probs = SinglePassLogitEngine::compute_softmax(&raw_logits, self.default_temperature)?;
        let mut prob_map = BTreeMap::new();
        let mut sorted_probs: Vec<(String, f64)> = Vec::new();

        for (label, &p) in labels.iter().zip(probs.iter()) {
            prob_map.insert(label.clone(), p);
            sorted_probs.push((label.clone(), p));
        }

        sorted_probs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let (selected_label, confidence) = sorted_probs[0].clone();
        let second_prob = sorted_probs.get(1).map(|s| s.1).unwrap_or(0.0);
        let margin = confidence - second_prob;
        let is_uncertain = margin < self.uncertainty_threshold;
        let latency_ms = start.elapsed().as_millis() as u64;

        Ok(DynamicVerdict::new(
            &question.id,
            selected_label,
            confidence,
            prob_map,
            margin,
            is_uncertain,
            latency_ms,
        ))
    }
}

/// Deterministic 16-dimensional feature encoder for offline Tropical Decision scoring.
/// Matches `encode_decision_features` in `FATE-O-SISTERS/scripts/train_moira_colab.py`.
pub fn encode_decision_features(prompt: &str, context: &str, label: &str) -> [f32; 16] {
    let text = format!("{prompt} {context}").to_lowercase();
    let lbl = label.to_lowercase();

    const DANGER_TERMS: &[&str] = &[
        "danger", "drop", "delete", "rm -rf", "format", "wipe", "force", "bypass",
    ];
    const SAFE_TERMS: &[&str] = &[
        "read", "search", "inspect", "status", "check", "test", "list", "safe",
    ];
    const COMPLEX_TERMS: &[&str] = &[
        "refactor",
        "architect",
        "migrate",
        "distributed",
        "kernel",
        "security",
    ];

    let danger_hits = DANGER_TERMS.iter().filter(|&&t| text.contains(t)).count() as f32;
    let safe_hits = SAFE_TERMS.iter().filter(|&&t| text.contains(t)).count() as f32;
    let complex_hits = COMPLEX_TERMS.iter().filter(|&&t| text.contains(t)).count() as f32;

    let is_allow = if matches!(
        lbl.as_str(),
        "allow" | "yes" | "true" | "safe" | "fast" | "proceed"
    ) {
        1.0
    } else {
        0.0
    };
    let is_reject = if matches!(
        lbl.as_str(),
        "reject" | "no" | "false" | "dangerous" | "deny" | "block"
    ) {
        1.0
    } else {
        0.0
    };
    let is_deep = if matches!(
        lbl.as_str(),
        "deep" | "careful" | "review" | "escalate" | "thorough"
    ) {
        1.0
    } else {
        0.0
    };
    let direct_match = if text.contains(&lbl) { 1.0 } else { 0.0 };

    let length_norm = ((text.len() as f32) / 500.0).min(2.0);
    let word_count_norm = ((text.split_whitespace().count() as f32) / 100.0).min(2.0);
    let prod_flag = if text.contains("prod") || text.contains("main") {
        1.0
    } else {
        0.0
    };

    [
        danger_hits,
        safe_hits,
        complex_hits,
        is_allow,
        is_reject,
        is_deep,
        direct_match,
        length_norm,
        word_count_norm,
        (2.5 * danger_hits - 0.3 * safe_hits) * is_reject,
        (safe_hits - 2.5 * danger_hits - 0.5 * complex_hits) * is_allow,
        (2.0 * complex_hits - danger_hits) * is_deep,
        -2.5 * danger_hits * is_allow,
        -1.5 * safe_hits * is_reject,
        prod_flag,
        1.0,
    ]
}

/// Offline, zero-multiplication Tropical Polyhedral decision driver.
///
/// Loads a static `.pnn` policy file (e.g., `.pandora/model_decision.pnn` distilled
/// offline from MOIRA) if present, and falls back gracefully to
/// [`HeuristicDecisionDriver`] when no model file is present.
#[derive(Clone, Debug)]
pub struct TropicalDecisionDriver {
    network: Option<crate::tropical_nn::TropicalNetwork>,
    fallback: HeuristicDecisionDriver,
    default_temperature: f64,
    uncertainty_threshold: f64,
    dpo_output_path: Option<std::path::PathBuf>,
}

impl Default for TropicalDecisionDriver {
    fn default() -> Self {
        Self::new(None)
    }
}

impl TropicalDecisionDriver {
    /// Creates a `TropicalDecisionDriver` from an optional pre-loaded network.
    pub fn new(network: Option<crate::tropical_nn::TropicalNetwork>) -> Self {
        Self {
            network,
            fallback: HeuristicDecisionDriver::new(),
            default_temperature: 1.0,
            uncertainty_threshold: 0.15,
            dpo_output_path: None,
        }
    }

    /// Configures an optional JSONL file path (e.g. `.pandora/dpo_pairs.jsonl`) where
    /// online System-2 -> System-1 [`DpoPair`] records emitted during LATS escalations are appended.
    pub fn with_dpo_output_path(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.dpo_output_path = Some(path.into());
        self
    }

    /// Returns the configured JSONL output path for emitted [`DpoPair`] records, if any.
    pub fn dpo_output_path(&self) -> Option<&std::path::Path> {
        self.dpo_output_path.as_deref()
    }

    /// Loads a static `.pnn` decision policy from disk; falls back to heuristics if absent or invalid.
    pub fn from_pnn_path(path: &std::path::Path) -> Self {
        let network = crate::tropical_nn::TropicalNetwork::load_from_file(path).ok();
        Self::new(network)
    }

    /// Loads from the default `.pandora/model_decision.pnn` path if present.
    pub fn from_default_pandora_path() -> Self {
        Self::from_pnn_path(std::path::Path::new(".pandora/model_decision.pnn"))
    }

    /// Returns `true` if a verified `.pnn` policy network is loaded.
    pub fn has_loaded_network(&self) -> bool {
        self.network.is_some()
    }

    /// Constructs a calibrated 2-layer `TropicalNetwork` (16 -> 4 -> 1) over the
    /// 16-dimensional decision feature space (`encode_decision_features`).
    pub fn calibrated_network() -> crate::tropical_nn::TropicalNetwork {
        let mut w1 = vec![-5.0f32; 16 * 4];
        for row in 0..4 {
            let offset = row * 16;
            w1[offset + 6] = 2.5; // direct_match
            w1[offset + 9] = 2.5; // danger_hits * is_reject
            w1[offset + 10] = 2.5; // safe_hits * is_allow
            w1[offset + 11] = 2.5; // complex_hits * is_deep
        }
        let l1 = crate::tropical_nn::TropicalLayer::new(16, 4, w1, Some(vec![0.0; 4]))
            .expect("valid calibrated layer 1");
        let l2 = crate::tropical_nn::TropicalLayer::new(4, 1, vec![0.0; 4], Some(vec![0.0]))
            .expect("valid calibrated layer 2");
        crate::tropical_nn::TropicalNetwork::new(
            "moira_calibrated_decision_v312",
            "decision_policy_scoring",
            vec![l1, l2],
        )
        .expect("valid calibrated tropical decision network")
    }

    /// Creates a `TropicalDecisionDriver` initialized with [`Self::calibrated_network`]
    /// and a sharp decision temperature (`0.25`) for high-margin branch selection.
    pub fn calibrated() -> Self {
        Self {
            network: Some(Self::calibrated_network()),
            fallback: HeuristicDecisionDriver::new(),
            default_temperature: 0.25,
            uncertainty_threshold: 0.15,
            dpo_output_path: None,
        }
    }

    /// Computes normalized Shannon entropy in `[0.0, 1.0]` for MOIRA's DSR gate.
    pub fn normalized_entropy(probs: &[f64]) -> f64 {
        if probs.len() <= 1 {
            return 0.0;
        }
        if probs.iter().any(|p| !p.is_finite()) {
            return 1.0;
        }
        let max_h = (probs.len() as f64).ln();
        let mut h = 0.0;
        for &p in probs {
            if p > 1e-12 {
                h -= p * p.ln();
            }
        }
        (h / max_h).clamp(0.0, 1.0)
    }

    /// Returns a reference to the active `TropicalNetwork`, if loaded.
    pub fn network(&self) -> Option<&crate::tropical_nn::TropicalNetwork> {
        self.network.as_ref()
    }

    /// Returns the default decision temperature.
    pub fn default_temperature(&self) -> f64 {
        self.default_temperature
    }

    /// Returns the uncertainty margin threshold.
    pub fn uncertainty_threshold(&self) -> f64 {
        self.uncertainty_threshold
    }

    /// Synthesizes a `DpoPair` from a System 2 MCTS override of an uncertain reflex decision.
    pub fn emit_dpo_pair_from_escalation(
        id: impl Into<String>,
        prompt: impl Into<String>,
        chosen_mcts_action: impl Into<String>,
        rejected_reflex_action: impl Into<String>,
        margin: f64,
    ) -> DpoPair {
        DpoPair {
            id: id.into(),
            prompt: prompt.into(),
            chosen: chosen_mcts_action.into(),
            rejected: rejected_reflex_action.into(),
            margin,
        }
    }

    /// Calibrates a `TropicalNetwork` using bounded tropical subgradient margin updates
    /// over `DpoPair` preferences: for each pair $(\mathbf{x}^+, \mathbf{x}^-)$ with margin
    /// $\Delta = f(\mathbf{x}^+)_0 - f(\mathbf{x}^-)_0 < 1.0$, reinforces the winning output-layer
    /// coordinate $j^+$ by $+\eta$ and suppresses $j^-$ by $-\eta$ (clamped to $[-8.0, 8.0]$),
    /// then recomputes the SHA-256 weight digest.
    pub fn calibrate_network_from_pairs(
        network: &crate::tropical_nn::TropicalNetwork,
        pairs: &[DpoPair],
        step_size: f32,
    ) -> Result<crate::tropical_nn::TropicalNetwork, DecisionError> {
        if !step_size.is_finite() {
            return Err(DecisionError::DriverError(
                "calibration step_size must be finite".to_owned(),
            ));
        }
        let eta = step_size.clamp(0.001, 0.25);
        let mut layers = network.layers.clone();
        if layers.is_empty() {
            return Err(DecisionError::DriverError(
                "cannot calibrate an empty TropicalNetwork".to_owned(),
            ));
        }

        let last_idx = layers.len() - 1;
        let argmax_j = |h: &[f32], weights: &[f32], in_dim: usize| -> usize {
            let mut best_j = 0;
            let mut best_val = f32::NEG_INFINITY;
            for j in 0..in_dim.min(h.len()) {
                let v = weights[j] + h[j];
                if v > best_val {
                    best_val = v;
                    best_j = j;
                }
            }
            best_j
        };

        for pair in pairs {
            let x_pos = encode_decision_features(&pair.prompt, "", &pair.chosen);
            let x_neg = encode_decision_features(&pair.prompt, "", &pair.rejected);

            let mut prev_pos = x_pos.to_vec();
            let mut prev_neg = x_neg.to_vec();
            for layer in &layers[..last_idx.saturating_sub(1)] {
                prev_pos = layer
                    .forward(&prev_pos)
                    .map_err(|e| DecisionError::DriverError(e.to_string()))?;
                prev_neg = layer
                    .forward(&prev_neg)
                    .map_err(|e| DecisionError::DriverError(e.to_string()))?;
            }

            let (h_pos, h_neg) = if last_idx >= 1 {
                (
                    layers[last_idx - 1]
                        .forward(&prev_pos)
                        .map_err(|e| DecisionError::DriverError(e.to_string()))?,
                    layers[last_idx - 1]
                        .forward(&prev_neg)
                        .map_err(|e| DecisionError::DriverError(e.to_string()))?,
                )
            } else {
                (prev_pos.clone(), prev_neg.clone())
            };

            let y_pos = layers[last_idx]
                .forward(&h_pos)
                .map_err(|e| DecisionError::DriverError(e.to_string()))?;
            let y_neg = layers[last_idx]
                .forward(&h_neg)
                .map_err(|e| DecisionError::DriverError(e.to_string()))?;

            let margin =
                y_pos.first().copied().unwrap_or(0.0) - y_neg.first().copied().unwrap_or(0.0);
            if margin >= 1.0 {
                continue;
            }

            let j_pos = {
                let out_layer = &layers[last_idx];
                argmax_j(&h_pos, &out_layer.weights, out_layer.in_features)
            };
            let j_neg = {
                let out_layer = &layers[last_idx];
                argmax_j(&h_neg, &out_layer.weights, out_layer.in_features)
            };
            if j_pos != j_neg {
                let out_layer = &mut layers[last_idx];
                out_layer.weights[j_pos] = (out_layer.weights[j_pos] + eta).clamp(-8.0, 8.0);
                out_layer.weights[j_neg] = (out_layer.weights[j_neg] - eta).clamp(-8.0, 8.0);
            } else if last_idx >= 1 {
                let hidden_layer = &mut layers[last_idx - 1];
                let in_dim = hidden_layer.in_features;
                if j_pos < hidden_layer.out_features {
                    let row_start = j_pos * in_dim;
                    let row_end = row_start + in_dim;
                    if row_end <= hidden_layer.weights.len() {
                        let row_weights = &hidden_layer.weights[row_start..row_end];
                        let k_pos = argmax_j(&prev_pos, row_weights, in_dim);
                        let k_neg = argmax_j(&prev_neg, row_weights, in_dim);
                        if k_pos != k_neg {
                            hidden_layer.weights[row_start + k_pos] =
                                (hidden_layer.weights[row_start + k_pos] + eta).clamp(-8.0, 8.0);
                            hidden_layer.weights[row_start + k_neg] =
                                (hidden_layer.weights[row_start + k_neg] - eta).clamp(-8.0, 8.0);
                        }
                    }
                }
            }
        }

        crate::tropical_nn::TropicalNetwork::new(
            network.name.clone(),
            network.domain.clone(),
            layers,
        )
        .map_err(|e| DecisionError::DriverError(e.to_string()))
    }

    /// Evaluates raw tropical logits across candidate labels for a prompt and context with zero multiplications.
    pub fn forward_logits<L: AsRef<str>>(
        &self,
        prompt: &str,
        context: &str,
        action_labels: &[L],
    ) -> Result<Vec<f64>, DecisionError> {
        if action_labels.is_empty() {
            return Err(DecisionError::EmptyCandidates);
        }
        if let Some(ref net) = self.network {
            let mut raw_logits = Vec::with_capacity(action_labels.len());
            for label in action_labels {
                let features = encode_decision_features(prompt, context, label.as_ref());
                let out = net
                    .forward(&features)
                    .map_err(|e| DecisionError::DriverError(e.to_string()))?;
                let logit = out.first().copied().unwrap_or(0.0) as f64;
                raw_logits.push(logit);
            }
            Ok(raw_logits)
        } else {
            let labels: Vec<String> = action_labels
                .iter()
                .map(|l| l.as_ref().to_string())
                .collect();
            let q = DecisionQuestion::choice("priors-eval", prompt, labels);
            let verdict = self.fallback.evaluate_question(&q)?;
            let mut raw_logits = Vec::with_capacity(action_labels.len());
            for label in action_labels {
                let p = verdict
                    .probabilities
                    .get(label.as_ref())
                    .copied()
                    .unwrap_or(0.0);
                raw_logits.push(p.max(1e-9).ln());
            }
            Ok(raw_logits)
        }
    }

    /// Evaluates prior policy probabilities P(a|s) for candidate action labels
    /// using zero-multiplication tropical max-plus inference and calibrated softmax.
    pub fn evaluate_action_priors<L: AsRef<str>>(
        &self,
        prompt: &str,
        context: &str,
        action_labels: &[L],
    ) -> Result<Vec<f64>, DecisionError> {
        let raw_logits = self.forward_logits(prompt, context, action_labels)?;
        SinglePassLogitEngine::compute_softmax(&raw_logits, self.default_temperature)
    }

    /// Computes calibrated softmax priors P(a|s) using sub-microsecond tropical inference (alias for `evaluate_action_priors`).
    pub fn compute_priors<L: AsRef<str>>(
        &self,
        prompt: &str,
        context: &str,
        action_labels: &[L],
    ) -> Result<Vec<f64>, DecisionError> {
        self.evaluate_action_priors(prompt, context, action_labels)
    }

    /// Computes bounded leaf value estimation Q(s, a) in [0.0, 1.0] from tropical forward pass.
    pub fn evaluate_leaf_value(&self, prompt: &str, context: &str, action_label: &str) -> f64 {
        if let Some(ref net) = self.network {
            let features = encode_decision_features(prompt, context, action_label);
            if let Ok(out) = net.forward(&features) {
                let logit = out.first().copied().unwrap_or(0.0) as f64;
                let penalty = (features[12] + features[13]) as f64;
                let calibrated = logit + penalty;
                let val = 1.0 / (1.0 + (-calibrated).exp());
                if val.is_finite() {
                    val.clamp(0.0, 1.0)
                } else {
                    0.5
                }
            } else {
                0.5
            }
        } else {
            let q = DecisionQuestion::binary("leaf-eval", prompt, action_label, "other");
            if let Ok(verdict) = self.fallback.evaluate_question(&q) {
                if verdict.selected_label == action_label {
                    verdict.confidence.clamp(0.0, 1.0)
                } else {
                    (1.0 - verdict.confidence).clamp(0.0, 1.0)
                }
            } else {
                0.5
            }
        }
    }

    /// Result-returning variant of `evaluate_leaf_value`.
    pub fn try_evaluate_leaf_value(
        &self,
        prompt: &str,
        context: &str,
        action_label: &str,
    ) -> Result<f64, DecisionError> {
        if let Some(ref net) = self.network {
            let features = encode_decision_features(prompt, context, action_label);
            let out = net
                .forward(&features)
                .map_err(|e| DecisionError::DriverError(e.to_string()))?;
            let logit = out.first().copied().unwrap_or(0.0) as f64;
            let penalty = (features[12] + features[13]) as f64;
            let calibrated = logit + penalty;
            let val = 1.0 / (1.0 + (-calibrated).exp());
            Ok(if val.is_finite() {
                val.clamp(0.0, 1.0)
            } else {
                0.5
            })
        } else {
            let q = DecisionQuestion::binary("leaf-eval", prompt, action_label, "other");
            let verdict = self.fallback.evaluate_question(&q)?;
            if verdict.selected_label == action_label {
                Ok(verdict.confidence.clamp(0.0, 1.0))
            } else {
                Ok((1.0 - verdict.confidence).clamp(0.0, 1.0))
            }
        }
    }

    /// Evaluates a 7D [`DecisionFeatures`] leaf state $\mathbf{x}_D \in [0, 1]^7$ through the
    /// loaded `.pnn` [` crate::tropical_nn::TropicalNetwork`] (`16 -> 4 -> 1`) via logistic
    /// calibration $\sigma(f_{\text{PNN}}(\mathbf{x}_D)_0) \in [0, 1]$, falling back to a
    /// deterministic potential function when no `.pnn` network is attached.
    pub fn evaluate_features_leaf(
        &self,
        prompt: &str,
        action_label: &str,
        features: &DecisionFeatures,
    ) -> f64 {
        let features_16 = features.to_network_features_16(prompt, action_label);
        let state_potential =
            1.6 * features.progress + 0.8 * features.verify + 0.6 * features.evidence
                - 2.2 * features.failure
                - 0.9 * features.loop_ratio
                - 0.4 * features.budget;
        let state_val = (1.0 / (1.0 + (-state_potential).exp())).clamp(0.0, 1.0);
        if let Some(ref net) = self.network
            && let Ok(out) = net.forward(&features_16)
        {
            let logit = out.first().copied().unwrap_or(0.0) as f64;
            let penalty = (features_16[12] + features_16[13]) as f64;
            let calibrated = logit + penalty;
            let pnn_val = (1.0 / (1.0 + (-calibrated).exp())).clamp(0.0, 1.0);
            let combined = 0.55 * pnn_val + 0.45 * state_val;
            if combined.is_finite() {
                return combined.clamp(0.0, 1.0);
            }
        }
        state_val
    }

    /// Evaluates the discounted multi-step return over a rollout trajectory $(a_0, \dots, a_{D-1})$:
    /// $$V_{\text{LATS}}(\mathbf{x}_0) = \sum_{d=0}^{D-1} \gamma^d R(\mathbf{x}_d, a_d) + \gamma^D \sigma\!\bigl(f_{\text{PNN}}(\mathbf{x}_D)_0\bigr)$$
    pub fn evaluate_multistep_return<L: AsRef<str>>(
        &self,
        prompt: &str,
        initial_features: &DecisionFeatures,
        actions_trajectory: &[usize],
        action_labels: &[L],
        gamma: f64,
    ) -> f64 {
        let discount = if gamma.is_finite() {
            gamma.clamp(0.0, 1.0)
        } else {
            0.9
        };
        let mut state = *initial_features;
        let mut total_return = 0.0;
        let mut gamma_pow = 1.0;
        let mut last_label = action_labels.first().map(|l| l.as_ref()).unwrap_or("allow");

        for &act in actions_trajectory {
            let next_state = state.transition(act);
            let r = state.step_reward(act, &next_state);
            total_return += gamma_pow * r;
            gamma_pow *= discount;
            state = next_state;
            if let Some(lbl) = action_labels.get(act) {
                last_label = lbl.as_ref();
            }
        }

        let terminal_val = self.evaluate_features_leaf(prompt, last_label, &state);
        total_return + gamma_pow * terminal_val
    }
}

#[inline]
fn clamp_unit(v: f64) -> f64 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// 7-dimensional normalized state vector $\mathbf{x} \in [0, 1]^7$ for multi-step LATS rollouts.
///
/// Coordinates:
/// 0. `progress` ($x_0 \in [0, 1]$): estimated task completion ratio.
/// 1. `failure` ($x_1 \in [0, 1]$): current hazard / failure ratio.
/// 2. `loop_ratio` ($x_2 \in [0, 1]$): anti-thrash repetition ratio.
/// 3. `budget` ($x_3 \in [0, 1]$): fraction of turn/tool budget consumed.
/// 4. `verify` ($x_4 \in [0, 1]$): verification and safety confirmation coverage.
/// 5. `evidence` ($x_5 \in [0, 1]$): accumulated execution evidence ratio.
/// 6. `novelty` ($x_6 \in [0, 1]$): trajectory state novelty.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DecisionFeatures {
    pub progress: f64,
    pub failure: f64,
    pub loop_ratio: f64,
    pub budget: f64,
    pub verify: f64,
    pub evidence: f64,
    pub novelty: f64,
}

impl Default for DecisionFeatures {
    fn default() -> Self {
        Self {
            progress: 0.25,
            failure: 0.15,
            loop_ratio: 0.0,
            budget: 0.10,
            verify: 0.50,
            evidence: 0.25,
            novelty: 0.75,
        }
    }
}

impl DecisionFeatures {
    /// Constructs a bounded 7D state vector $\mathbf{x} \in [0, 1]^7$, clamping every coordinate to $[0, 1]$.
    pub fn new(coords: [f64; 7]) -> Self {
        Self {
            progress: clamp_unit(coords[0]),
            failure: clamp_unit(coords[1]),
            loop_ratio: clamp_unit(coords[2]),
            budget: clamp_unit(coords[3]),
            verify: clamp_unit(coords[4]),
            evidence: clamp_unit(coords[5]),
            novelty: clamp_unit(coords[6]),
        }
    }

    /// Returns the 7 coordinates as an array `[f64; 7]` in $[0, 1]^7$.
    pub fn to_array(&self) -> [f64; 7] {
        [
            self.progress,
            self.failure,
            self.loop_ratio,
            self.budget,
            self.verify,
            self.evidence,
            self.novelty,
        ]
    }

    /// Returns `true` if all 7 coordinates are finite and lie within $[0.0, 1.0]$.
    pub fn is_valid(&self) -> bool {
        self.to_array()
            .iter()
            .all(|&x| x.is_finite() && (0.0..=1.0).contains(&x))
    }

    /// Extracts initial 7D [`DecisionFeatures`] from a tool/task prompt and runtime loop counters.
    pub fn from_tool_context(
        prompt: &str,
        turn: u32,
        max_turns: u32,
        duplicate_failures: u32,
    ) -> Self {
        let base_16 = encode_decision_features(prompt, "", "allow");
        let danger_hits = base_16[0] as f64;
        let safe_hits = base_16[1] as f64;
        let complex_hits = base_16[2] as f64;
        let prod_flag = base_16[14] as f64;

        let budget_ratio = if max_turns > 0 {
            (turn as f64) / (max_turns as f64)
        } else {
            0.25
        };
        let loop_ratio = (duplicate_failures as f64) * 0.35;
        let failure = if danger_hits >= 1.0 {
            0.85 + 0.05 * danger_hits
        } else if duplicate_failures > 0 || complex_hits > 0.0 || prod_flag > 0.0 {
            0.40 + 0.15 * (duplicate_failures as f64) + 0.08 * complex_hits + 0.05 * prod_flag
        } else {
            0.15
        };
        let progress = 0.20 + 0.15 * budget_ratio + 0.05 * safe_hits;
        let verify = if danger_hits >= 1.0 {
            0.10
        } else {
            0.35 + 0.15 * safe_hits
        };
        let evidence = 0.20 + 0.10 * safe_hits + 0.10 * budget_ratio;
        let novelty = (0.85 - 0.25 * loop_ratio).max(0.10);

        Self::new([
            progress,
            failure,
            loop_ratio,
            budget_ratio,
            verify,
            evidence,
            novelty,
        ])
    }

    /// Deterministic state transition operator $T: [0, 1]^7 \times \{0, 1, 2, 3\} \to [0, 1]^7$.
    ///
    /// Action semantics:
    /// - $a = 0$ (`Proceed` / `allow`): advances progress when `failure <= 0.35`, but amplifies
    ///   failure and loop thrashing when applied prematurely in an unrepaired state (`failure > 0.35`).
    /// - $a = 1$ (`Repair` / `require_confirm`): contracts failure ($-0.35$) and loop ratio ($-0.20$)
    ///   while boosting verification ($+0.30$) and evidence ($+0.15$) at budget cost ($+0.10$).
    /// - $a = 2$ (`Escalate` / `reject`): neutralizes high-hazard states (`failure >= 0.75`) or
    ///   applies conservative verification gating.
    /// - $a \ge 3$ (`Abort`): halts progression and resets loop thrashing.
    pub fn transition(&self, action: usize) -> Self {
        match action {
            0 => {
                if self.failure <= 0.35 {
                    Self::new([
                        self.progress + 0.35 * (1.0 - self.failure),
                        self.failure * 0.90,
                        self.loop_ratio * 0.85,
                        self.budget + 0.08,
                        self.verify + 0.05,
                        self.evidence + 0.20,
                        self.novelty + 0.10,
                    ])
                } else {
                    Self::new([
                        self.progress + 0.02,
                        self.failure + 0.30,
                        self.loop_ratio + 0.25,
                        self.budget + 0.12,
                        self.verify * 0.80,
                        self.evidence + 0.02,
                        self.novelty * 0.70,
                    ])
                }
            }
            1 => Self::new([
                self.progress + 0.12,
                self.failure - 0.35,
                self.loop_ratio - 0.20,
                self.budget + 0.10,
                self.verify + 0.30,
                self.evidence + 0.15,
                self.novelty + 0.15,
            ]),
            2 => {
                if self.failure >= 0.75 {
                    Self::new([
                        self.progress,
                        self.failure - 0.65,
                        self.loop_ratio * 0.50,
                        self.budget + 0.05,
                        self.verify + 0.45,
                        self.evidence + 0.20,
                        self.novelty,
                    ])
                } else {
                    // Rejecting a recoverable (non-destructive) state stalls progress without repairing failure
                    Self::new([
                        self.progress,
                        self.failure,
                        self.loop_ratio + 0.10,
                        self.budget + 0.15,
                        self.verify,
                        self.evidence,
                        self.novelty * 0.50,
                    ])
                }
            }
            _ => Self::new([
                self.progress,
                self.failure,
                0.0,
                self.budget + 0.10,
                self.verify,
                self.evidence,
                0.0,
            ]),
        }
    }

    /// Computes bounded immediate transition reward $R(\mathbf{x}_d, a_d) \in [0.0, 1.0]$.
    pub fn step_reward(&self, action: usize, next: &Self) -> f64 {
        if self.failure >= 0.75 {
            return match action {
                2 => clamp_unit(0.90 + 0.10 * next.verify),
                1 => clamp_unit(0.35 + 0.10 * next.verify),
                0 => clamp_unit(0.05 * (1.0 - next.failure)),
                _ => clamp_unit(0.40),
            };
        }
        if action >= 2 {
            // In a non-destructive regime (failure < 0.75), Reject/Abort stalls task completion
            return clamp_unit(0.15 * (1.0 - self.failure));
        }
        let delta_progress = next.progress - self.progress;
        let failure_reduction = self.failure - next.failure;
        let verify_gain = next.verify - self.verify;
        let raw = 0.45
            + 0.85 * delta_progress
            + 0.70 * failure_reduction
            + 0.30 * verify_gain
            + 0.20 * next.evidence
            - 0.55 * next.failure
            - 0.25 * next.loop_ratio
            - 0.15 * next.budget;
        clamp_unit(raw)
    }

    /// Lifts the 7D [`DecisionFeatures`] state and prompt/action label into the 16D feature space
    /// consumed by [`crate::tropical_nn::TropicalNetwork`].
    pub fn to_network_features_16(&self, prompt: &str, label: &str) -> [f32; 16] {
        let mut f = encode_decision_features(prompt, "", label);
        let eff_danger = (f[0] + (self.failure as f32) * 2.0).max(0.0);
        let eff_safe = (f[1]
            + ((self.verify + self.progress * 0.5) as f32) * 2.0 * (1.0 - self.failure as f32))
            .max(0.0);
        let eff_complex = (f[2] + (self.loop_ratio + self.budget * 0.5) as f32).max(0.0);
        let is_allow = f[3];
        let is_reject = f[4];
        let is_deep = f[5];

        f[0] = eff_danger;
        f[1] = eff_safe;
        f[2] = eff_complex;
        f[9] = (2.5 * eff_danger - 0.3 * eff_safe) * is_reject;
        f[10] = (eff_safe - 2.5 * eff_danger - 0.5 * eff_complex) * is_allow;
        f[11] = (2.0 * eff_complex - eff_danger) * is_deep;
        f[12] = -2.5 * eff_danger * is_allow;
        f[13] = -1.5 * eff_safe * is_reject;
        f
    }
}

/// Computes a deterministic SHA-256 digest $H(\mathcal{D}_{\text{DPO}})$ over an ordered slice of [`DpoPair`] records.
pub fn compute_dpo_dataset_digest(pairs: &[DpoPair]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update((pairs.len() as u64).to_le_bytes());
    for pair in pairs {
        hasher.update((pair.id.len() as u64).to_le_bytes());
        hasher.update(pair.id.as_bytes());
        hasher.update((pair.prompt.len() as u64).to_le_bytes());
        hasher.update(pair.prompt.as_bytes());
        hasher.update((pair.chosen.len() as u64).to_le_bytes());
        hasher.update(pair.chosen.as_bytes());
        hasher.update((pair.rejected.len() as u64).to_le_bytes());
        hasher.update(pair.rejected.as_bytes());
        hasher.update(pair.margin.to_bits().to_le_bytes());
    }
    format!("{:x}", hasher.finalize())
}

/// Computes the cryptographic calibration commitment:
/// $$C_{\text{cal}} = \operatorname{SHA256}\!\bigl(H(W_{\text{base}}) \;\|\; H(W_{\text{cand}}) \;\|\; H(\mathcal{D}_{\text{DPO}}) \;\|\; \operatorname{le64}(q_{0.05})\bigr)$$
pub fn compute_calibration_commitment(
    baseline_digest: &str,
    candidate_digest: &str,
    dpo_dataset_digest: &str,
    lower_bound_95: f64,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(baseline_digest.as_bytes());
    hasher.update(candidate_digest.as_bytes());
    hasher.update(dpo_dataset_digest.as_bytes());
    hasher.update(lower_bound_95.to_bits().to_le_bytes());
    format!("{:x}", hasher.finalize())
}

/// Tamper-evident, hash-chained `.pnn` calibration attestation receipt (Phase 8).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CalibrationAttestationReceipt {
    pub sequence: u64,
    pub prev_receipt_hash: String,
    pub baseline_digest: String,
    pub candidate_digest: String,
    pub dpo_dataset_digest: String,
    pub lower_bound_95: f64,
    pub lower_bound_95_bits: u64,
    pub commitment: String,
    pub chain_hash: String,
    pub promoted: bool,
}

impl CalibrationAttestationReceipt {
    /// Genesis previous-receipt hash (`64` hex zeroes).
    pub const GENESIS_PREV_HASH: &'static str =
        "0000000000000000000000000000000000000000000000000000000000000000";

    /// Constructs a new hash-chained [`CalibrationAttestationReceipt`].
    pub fn new(
        sequence: u64,
        prev_receipt_hash: impl Into<String>,
        baseline_digest: impl Into<String>,
        candidate_digest: impl Into<String>,
        dpo_dataset_digest: impl Into<String>,
        lower_bound_95: f64,
        promoted: bool,
    ) -> Self {
        let prev_receipt_hash = prev_receipt_hash.into();
        let baseline_digest = baseline_digest.into();
        let candidate_digest = candidate_digest.into();
        let dpo_dataset_digest = dpo_dataset_digest.into();
        let lower_bound_95_bits = lower_bound_95.to_bits();
        let commitment = compute_calibration_commitment(
            &baseline_digest,
            &candidate_digest,
            &dpo_dataset_digest,
            lower_bound_95,
        );
        let chain_hash =
            Self::compute_chain_hash(sequence, &prev_receipt_hash, &commitment, promoted);
        Self {
            sequence,
            prev_receipt_hash,
            baseline_digest,
            candidate_digest,
            dpo_dataset_digest,
            lower_bound_95,
            lower_bound_95_bits,
            commitment,
            chain_hash,
            promoted,
        }
    }

    /// Computes the hash-chain digest linking `(sequence, prev_receipt_hash, commitment, promoted)`.
    pub fn compute_chain_hash(
        sequence: u64,
        prev_receipt_hash: &str,
        commitment: &str,
        promoted: bool,
    ) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(sequence.to_le_bytes());
        hasher.update(prev_receipt_hash.as_bytes());
        hasher.update(commitment.as_bytes());
        hasher.update([u8::from(promoted)]);
        format!("{:x}", hasher.finalize())
    }

    /// Verifies bit-exact integrity of `commitment`, `lower_bound_95_bits`, and `chain_hash`.
    pub fn verify(&self) -> bool {
        if !self.lower_bound_95.is_finite()
            || self.lower_bound_95.to_bits() != self.lower_bound_95_bits
        {
            return false;
        }
        if self.baseline_digest.is_empty()
            || self.candidate_digest.is_empty()
            || self.dpo_dataset_digest.is_empty()
            || self.prev_receipt_hash.len() != 64
        {
            return false;
        }
        let expected_commitment = compute_calibration_commitment(
            &self.baseline_digest,
            &self.candidate_digest,
            &self.dpo_dataset_digest,
            self.lower_bound_95,
        );
        if self.commitment != expected_commitment {
            return false;
        }
        let expected_chain = Self::compute_chain_hash(
            self.sequence,
            &self.prev_receipt_hash,
            &self.commitment,
            self.promoted,
        );
        self.chain_hash == expected_chain
    }

    /// Verifies an ordered sequence of hash-chained calibration receipts from genesis (`sequence == 0`).
    pub fn verify_chain(receipts: &[Self]) -> bool {
        for (idx, r) in receipts.iter().enumerate() {
            if !r.verify() || r.sequence != idx as u64 {
                return false;
            }
            if idx == 0 {
                if r.prev_receipt_hash != Self::GENESIS_PREV_HASH {
                    return false;
                }
            } else if r.prev_receipt_hash != receipts[idx - 1].chain_hash {
                return false;
            }
        }
        true
    }
}

/// Alias highlighting MOIRA (`FATE-O-SISTERS`) 3-6-9 decision integration.
pub type MoiraDecisionDriver = TropicalDecisionDriver;

/// Runs the deterministic `pandora-harnesses` decision benchmark suite comparing
/// `HeuristicDecisionDriver` and `MoiraDecisionDriver` on real task traces.
pub fn benchmark_moira_vs_heuristic(
    moira: &MoiraDecisionDriver,
    heuristic: &HeuristicDecisionDriver,
) -> Result<pandora_harnesses::decision_benchmark::ComparativeBenchmarkReport, DecisionError> {
    let suite =
        pandora_harnesses::decision_benchmark::DecisionBenchmarkSuite::default_real_task_traces();
    suite.compare_drivers(
        |q| heuristic.evaluate_question(q),
        |q| moira.evaluate_question(q),
    )
}

impl DecisionDriver for TropicalDecisionDriver {
    fn evaluate_question(
        &self,
        question: &DecisionQuestion,
    ) -> Result<DynamicVerdict, DecisionError> {
        let Some(ref net) = self.network else {
            return self.fallback.evaluate_question(question);
        };

        let start = Instant::now();
        if !question.schema.is_valid() {
            return Err(DecisionError::InvalidSchema(
                "invalid question schema".into(),
            ));
        }

        let labels = question.schema.labels();
        let state_str = question
            .state_context
            .as_ref()
            .map(|s| s.to_string())
            .unwrap_or_default();

        let mut raw_logits = Vec::with_capacity(labels.len());
        for label in &labels {
            let features = encode_decision_features(&question.prompt, &state_str, label);
            let out = net
                .forward(&features)
                .map_err(|e| DecisionError::DriverError(e.to_string()))?;
            let logit = out.first().copied().unwrap_or(0.0) as f64;
            raw_logits.push(logit);
        }

        let probs = SinglePassLogitEngine::compute_softmax(&raw_logits, self.default_temperature)?;
        let mut prob_map = BTreeMap::new();
        let mut sorted_probs: Vec<(String, f64)> = Vec::new();

        for (label, &p) in labels.iter().zip(probs.iter()) {
            prob_map.insert(label.clone(), p);
            sorted_probs.push((label.clone(), p));
        }

        sorted_probs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let (selected_label, confidence) = sorted_probs[0].clone();
        let second_prob = sorted_probs.get(1).map(|s| s.1).unwrap_or(0.0);
        let margin = confidence - second_prob;
        let entropy = Self::normalized_entropy(&probs);
        // MOIRA DSR Gate: flag uncertainty if margin is narrow OR normalized entropy exceeds 0.666
        let is_uncertain = margin < self.uncertainty_threshold || entropy > 0.666;
        let latency_ms = start.elapsed().as_millis() as u64;

        Ok(DynamicVerdict::new(
            &question.id,
            selected_label,
            confidence,
            prob_map,
            margin,
            is_uncertain,
            latency_ms,
        ))
    }
}

#[derive(Debug, Default)]
pub struct MockDecisionDriver {
    preset_verdicts: Mutex<VecDeque<Result<DynamicVerdict, DecisionError>>>,
}

impl MockDecisionDriver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_verdict(&self, verdict: DynamicVerdict) {
        self.preset_verdicts.lock().unwrap().push_back(Ok(verdict));
    }

    pub fn push_error(&self, error: DecisionError) {
        self.preset_verdicts.lock().unwrap().push_back(Err(error));
    }
}

impl DecisionDriver for MockDecisionDriver {
    fn evaluate_question(
        &self,
        question: &DecisionQuestion,
    ) -> Result<DynamicVerdict, DecisionError> {
        let mut queue = self.preset_verdicts.lock().unwrap();
        if let Some(res) = queue.pop_front() {
            return res;
        }
        let labels = question.schema.labels();
        let first = labels
            .first()
            .cloned()
            .unwrap_or_else(|| "default".to_string());
        let mut probs = BTreeMap::new();
        probs.insert(first.clone(), 1.0);
        Ok(DynamicVerdict::new(
            &question.id,
            first,
            1.0,
            probs,
            1.0,
            false,
            1,
        ))
    }
}

#[derive(Debug)]
pub struct OllamaDecisionDriver {
    endpoint_url: String,
    model_name: String,
    timeout: Duration,
    temperature: f64,
    uncertainty_threshold: f64,
    client: reqwest::blocking::Client,
}

impl OllamaDecisionDriver {
    pub fn new(
        endpoint_url: impl Into<String>,
        model_name: impl Into<String>,
        timeout: Duration,
        temperature: f64,
        uncertainty_threshold: f64,
    ) -> Result<Self, DecisionError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| DecisionError::NetworkError(e.to_string()))?;
        Ok(Self {
            endpoint_url: endpoint_url.into().trim_end_matches('/').to_string(),
            model_name: model_name.into(),
            timeout,
            temperature,
            uncertainty_threshold,
            client,
        })
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    pub fn format_prompt(question: &DecisionQuestion) -> String {
        let labels = question.schema.labels();
        let choices_str = labels.join(", ");
        let state_section = match &question.state_context {
            Some(val) => format!("\nContext state: {val}"),
            None => String::new(),
        };
        format!(
            "You are an ultra-fast non-generative decision engine.{state_section}\nQuestion: {}\nChoices: [{choices_str}]\nOutput ONLY exactly one choice from the available choices and nothing else.",
            question.prompt
        )
    }
}

impl DecisionDriver for OllamaDecisionDriver {
    fn evaluate_question(
        &self,
        question: &DecisionQuestion,
    ) -> Result<DynamicVerdict, DecisionError> {
        let start = Instant::now();
        if !question.schema.is_valid() {
            return Err(DecisionError::InvalidSchema(
                "invalid question schema".into(),
            ));
        }
        let labels = question.schema.labels();
        let prompt = Self::format_prompt(question);
        let url = format!("{}/api/generate", self.endpoint_url);

        let payload = serde_json::json!({
            "model": self.model_name,
            "prompt": prompt,
            "stream": false,
            "options": {
                "num_predict": 1,
                "temperature": 0.0
            }
        });

        let response = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .map_err(|e| DecisionError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            return Err(DecisionError::DriverError(format!(
                "Ollama request failed with HTTP {}",
                response.status()
            )));
        }

        let body: serde_json::Value = response.json().map_err(|e| {
            DecisionError::DriverError(format!("failed to parse Ollama JSON response: {e}"))
        })?;

        let response_text = body
            .get("response")
            .and_then(|r| r.as_str())
            .unwrap_or("")
            .trim();

        let mut selected_idx = 0;
        let mut best_match_len = 0;
        for (i, label) in labels.iter().enumerate() {
            if response_text.eq_ignore_ascii_case(label) {
                selected_idx = i;
                break;
            }
            if response_text.to_lowercase().contains(&label.to_lowercase())
                && label.len() > best_match_len
            {
                selected_idx = i;
                best_match_len = label.len();
            }
        }

        let mut logits = vec![-4.0; labels.len()];
        logits[selected_idx] = 6.0;

        let probs = SinglePassLogitEngine::compute_softmax(&logits, self.temperature)?;
        let mut prob_map = BTreeMap::new();
        let mut sorted_probs = Vec::new();

        for (label, &p) in labels.iter().zip(probs.iter()) {
            prob_map.insert(label.clone(), p);
            sorted_probs.push((label.clone(), p));
        }

        sorted_probs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let (selected_label, confidence) = sorted_probs[0].clone();
        let second_prob = sorted_probs.get(1).map(|s| s.1).unwrap_or(0.0);
        let margin = confidence - second_prob;
        let is_uncertain = margin < self.uncertainty_threshold;
        let latency_ms = start.elapsed().as_millis() as u64;

        Ok(DynamicVerdict::new(
            &question.id,
            selected_label,
            confidence,
            prob_map,
            margin,
            is_uncertain,
            latency_ms,
        ))
    }
}

#[derive(Debug)]
pub struct CustomApiDecisionDriver {
    endpoint_url: String,
    model_name: String,
    api_key: Option<String>,
    timeout: Duration,
    temperature: f64,
    uncertainty_threshold: f64,
    client: reqwest::blocking::Client,
}

impl CustomApiDecisionDriver {
    pub fn new(
        endpoint_url: impl Into<String>,
        model_name: impl Into<String>,
        api_key: Option<String>,
        timeout: Duration,
        temperature: f64,
        uncertainty_threshold: f64,
    ) -> Result<Self, DecisionError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| DecisionError::NetworkError(e.to_string()))?;
        Ok(Self {
            endpoint_url: endpoint_url.into().trim_end_matches('/').to_string(),
            model_name: model_name.into(),
            api_key,
            timeout,
            temperature,
            uncertainty_threshold,
            client,
        })
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }
}

impl DecisionDriver for CustomApiDecisionDriver {
    fn evaluate_question(
        &self,
        question: &DecisionQuestion,
    ) -> Result<DynamicVerdict, DecisionError> {
        let start = Instant::now();
        if !question.schema.is_valid() {
            return Err(DecisionError::InvalidSchema(
                "invalid question schema".into(),
            ));
        }
        let labels = question.schema.labels();
        let prompt = OllamaDecisionDriver::format_prompt(question);
        let url = if self.endpoint_url.ends_with("/chat/completions") {
            self.endpoint_url.clone()
        } else {
            format!("{}/v1/chat/completions", self.endpoint_url)
        };

        let payload = serde_json::json!({
            "model": self.model_name,
            "messages": [
                {"role": "system", "content": "You are a non-generative, schema-constrained decision engine. Output ONLY the selected label token."},
                {"role": "user", "content": prompt}
            ],
            "max_tokens": 1,
            "temperature": 0.0,
            "logprobs": true,
            "top_logprobs": 10
        });

        let mut req = self.client.post(&url).json(&payload);
        if let Some(key) = &self.api_key {
            req = req.header("Authorization", format!("Bearer {key}"));
        }

        let response = req
            .send()
            .map_err(|e| DecisionError::NetworkError(e.to_string()))?;
        if !response.status().is_success() {
            return Err(DecisionError::DriverError(format!(
                "Custom API request failed with HTTP {}",
                response.status()
            )));
        }

        let body: serde_json::Value = response.json().map_err(|e| {
            DecisionError::DriverError(format!("failed to parse custom API response: {e}"))
        })?;

        let mut logits = vec![-15.0; labels.len()];
        let mut found_logprobs = false;

        if let Some(top_lps) = body
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("logprobs"))
            .and_then(|lp| lp.get("content"))
            .and_then(|cnt| cnt.get(0))
            .and_then(|tok| tok.get("top_logprobs"))
            .and_then(|arr| arr.as_array())
        {
            for item in top_lps {
                if let (Some(tok_str), Some(lp_val)) = (
                    item.get("token").and_then(|t| t.as_str()),
                    item.get("logprob").and_then(|l| l.as_f64()),
                ) {
                    let clean_tok = tok_str.trim().to_lowercase();
                    for (i, label) in labels.iter().enumerate() {
                        if clean_tok == label.to_lowercase() {
                            logits[i] = lp_val;
                            found_logprobs = true;
                        }
                    }
                }
            }
        }

        if !found_logprobs {
            let text = body
                .get("choices")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("message"))
                .and_then(|m| m.get("content"))
                .and_then(|cnt| cnt.as_str())
                .unwrap_or("")
                .trim();
            let mut selected_idx = 0;
            for (i, label) in labels.iter().enumerate() {
                if text.eq_ignore_ascii_case(label)
                    || text.to_lowercase().contains(&label.to_lowercase())
                {
                    selected_idx = i;
                    break;
                }
            }
            logits = vec![-4.0; labels.len()];
            logits[selected_idx] = 6.0;
        }

        let probs = SinglePassLogitEngine::compute_softmax(&logits, self.temperature)?;
        let mut prob_map = BTreeMap::new();
        let mut sorted_probs = Vec::new();

        for (label, &p) in labels.iter().zip(probs.iter()) {
            prob_map.insert(label.clone(), p);
            sorted_probs.push((label.clone(), p));
        }

        sorted_probs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let (selected_label, confidence) = sorted_probs[0].clone();
        let second_prob = sorted_probs.get(1).map(|s| s.1).unwrap_or(0.0);
        let margin = confidence - second_prob;
        let is_uncertain = margin < self.uncertainty_threshold;
        let latency_ms = start.elapsed().as_millis() as u64;

        Ok(DynamicVerdict::new(
            &question.id,
            selected_label,
            confidence,
            prob_map,
            margin,
            is_uncertain,
            latency_ms,
        ))
    }
}

#[derive(Clone, Debug)]
pub struct SinglePassLogitEngine {
    default_temperature: f64,
    uncertainty_threshold: f64,
    driver: Arc<dyn DecisionDriver>,
}

pub type DecisionEngine = SinglePassLogitEngine;

/// Determines if a decision should escalate from System 1 (reflex) to System 2 (MCTS tree search).
///
/// Returns `true` if decision confidence margin falls below uncertainty threshold
/// OR normalized Shannon entropy exceeds 0.666 (or if values are non-finite).
pub fn should_escalate(margin: f64, uncertainty_threshold: f64, normalized_entropy: f64) -> bool {
    !margin.is_finite()
        || !uncertainty_threshold.is_finite()
        || !normalized_entropy.is_finite()
        || margin < uncertainty_threshold
        || normalized_entropy > 0.666
}

/// System 1 -> System 2 escalation gate with normalized Shannon entropy (Requirement R2).
pub fn should_escalate_with_entropy(
    margin: f64,
    uncertainty_threshold: f64,
    normalized_entropy: f64,
) -> bool {
    should_escalate(margin, uncertainty_threshold, normalized_entropy)
}

/// Module-level helper computing normalized Shannon entropy in `[0.0, 1.0]`.
pub fn normalized_entropy(probs: &[f64]) -> f64 {
    TropicalDecisionDriver::normalized_entropy(probs)
}

impl Default for SinglePassLogitEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SinglePassLogitEngine {
    pub fn new() -> Self {
        Self {
            default_temperature: 1.0,
            uncertainty_threshold: 0.15,
            driver: Arc::new(MoiraDecisionDriver::from_default_pandora_path()),
        }
    }

    pub fn with_driver(driver: Arc<dyn DecisionDriver>) -> Self {
        Self {
            default_temperature: 1.0,
            uncertainty_threshold: 0.15,
            driver,
        }
    }

    pub fn with_temperature(temperature: f64) -> Result<Self, DecisionError> {
        if temperature <= 0.0 || temperature.is_nan() || temperature.is_infinite() {
            return Err(DecisionError::InvalidTemperature(
                "temperature must be positive and finite".into(),
            ));
        }
        let mut driver = MoiraDecisionDriver::from_default_pandora_path();
        driver.default_temperature = temperature;
        Ok(Self {
            default_temperature: temperature,
            uncertainty_threshold: 0.15,
            driver: Arc::new(driver),
        })
    }

    /// System 1 -> System 2 escalation gate. Returns `true` when the given
    /// verdict's margin is below this engine's uncertainty threshold OR
    /// normalized Shannon entropy exceeds 0.666, indicating that the fast reflex
    /// path is unsafe and the caller should escalate to [`crate::lats_engine::LatsEngine::search`].
    pub fn should_escalate(&self, verdict: &pandora_types::DynamicVerdict) -> bool {
        if verdict.is_uncertain {
            return true;
        }
        let entropy = if verdict.probabilities.is_empty() {
            0.0
        } else {
            let probs: Vec<f64> = verdict.probabilities.values().copied().collect();
            TropicalDecisionDriver::normalized_entropy(&probs)
        };
        should_escalate(verdict.margin, self.uncertainty_threshold, entropy)
    }

    /// Escalation check based on margin alone against uncertainty threshold.
    pub fn should_escalate_margin(&self, margin: f64) -> bool {
        crate::lats_engine::should_escalate(margin, self.uncertainty_threshold)
    }

    /// Escalation check with explicit normalized entropy.
    pub fn should_escalate_with_entropy(&self, margin: f64, normalized_entropy: f64) -> bool {
        should_escalate(margin, self.uncertainty_threshold, normalized_entropy)
    }

    /// Escalation gate for typed [`pandora_types::DecisionVerdict<ToolSafetyChoice>`].
    pub fn should_escalate_tool_safety(
        &self,
        verdict: &pandora_types::DecisionVerdict<ToolSafetyChoice>,
    ) -> bool {
        let probs: Vec<f64> = verdict
            .distribution
            .candidates
            .iter()
            .map(|c| c.probability)
            .collect();
        let mut sorted = probs.clone();
        sorted.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        let margin = sorted.first().copied().unwrap_or(0.0) - sorted.get(1).copied().unwrap_or(0.0);
        let entropy = TropicalDecisionDriver::normalized_entropy(&probs);
        should_escalate(margin, self.uncertainty_threshold, entropy)
    }

    pub fn from_config(config: &DecisionDriverConfig) -> Result<Self, DecisionError> {
        let driver: Arc<dyn DecisionDriver> = match config.kind {
            DecisionDriverKind::Heuristic => Arc::new(HeuristicDecisionDriver::with_options(
                config.temperature,
                config.uncertainty_threshold,
            )),
            DecisionDriverKind::Mock => Arc::new(MockDecisionDriver::new()),
            DecisionDriverKind::Moira => {
                let pnn_path = config
                    .model_name
                    .as_deref()
                    .unwrap_or(".pandora/model_decision.pnn");
                let mut driver =
                    TropicalDecisionDriver::from_pnn_path(std::path::Path::new(pnn_path));
                driver.default_temperature = config.temperature;
                driver.uncertainty_threshold = config.uncertainty_threshold;
                Arc::new(driver)
            }
            DecisionDriverKind::Ollama => {
                let endpoint = config
                    .endpoint_url
                    .as_deref()
                    .unwrap_or("http://127.0.0.1:11434");
                let model = config.model_name.as_deref().unwrap_or("llama3");
                Arc::new(OllamaDecisionDriver::new(
                    endpoint,
                    model,
                    Duration::from_millis(config.timeout_ms),
                    config.temperature,
                    config.uncertainty_threshold,
                )?)
            }
            DecisionDriverKind::OpenAiCompatible => {
                let endpoint = config
                    .endpoint_url
                    .as_deref()
                    .unwrap_or("http://127.0.0.1:8000");
                let model = config.model_name.as_deref().unwrap_or("default");
                Arc::new(CustomApiDecisionDriver::new(
                    endpoint,
                    model,
                    config.api_key.clone(),
                    Duration::from_millis(config.timeout_ms),
                    config.temperature,
                    config.uncertainty_threshold,
                )?)
            }
        };
        Ok(Self {
            default_temperature: config.temperature,
            uncertainty_threshold: config.uncertainty_threshold,
            driver,
        })
    }

    pub fn default_temperature(&self) -> f64 {
        self.default_temperature
    }

    pub fn uncertainty_threshold(&self) -> f64 {
        self.uncertainty_threshold
    }

    pub fn driver(&self) -> &Arc<dyn DecisionDriver> {
        &self.driver
    }

    pub fn evaluate_dynamic(
        &self,
        question: &DecisionQuestion,
    ) -> Result<DynamicVerdict, DecisionError> {
        self.driver.evaluate_question(question)
    }

    pub fn evaluate_batch(
        &self,
        questions: &[DecisionQuestion],
    ) -> Result<Vec<DynamicVerdict>, DecisionError> {
        let mut results = Vec::with_capacity(questions.len());
        for q in questions {
            results.push(self.evaluate_dynamic(q)?);
        }
        Ok(results)
    }

    pub fn harvest_uncertain_decisions(
        &self,
        verdicts: &[DynamicVerdict],
        questions: &[DecisionQuestion],
    ) -> Vec<DecisionEvolutionRecord> {
        let mut records = Vec::new();
        let unix_now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let now = Timestamp::from_unix_seconds(unix_now);
        for (v, q) in verdicts.iter().zip(questions.iter()) {
            if v.is_uncertain {
                records.push(DecisionEvolutionRecord {
                    id: format!("evo-dec-{}", v.question_id),
                    question_id: v.question_id.clone(),
                    prompt: q.prompt.clone(),
                    selected_label: v.selected_label.clone(),
                    confidence: v.confidence,
                    margin: v.margin,
                    candidate_probabilities: v.probabilities.clone(),
                    recorded_at: now,
                });
            }
        }
        records
    }

    pub fn harvest_uncertain_decisions_with_margin(
        &self,
        verdicts: &[DynamicVerdict],
        questions: &[DecisionQuestion],
        max_margin: Option<f64>,
        limit: Option<usize>,
    ) -> Vec<DecisionEvolutionRecord> {
        let q_by_id: std::collections::BTreeMap<&str, &DecisionQuestion> =
            questions.iter().map(|q| (q.id.as_str(), q)).collect();
        let unix_now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let now = Timestamp::from_unix_seconds(unix_now);
        let mut records = Vec::new();

        for (idx, v) in verdicts.iter().enumerate() {
            let passes = match max_margin {
                Some(threshold) => v.margin <= threshold,
                None => v.is_uncertain || v.margin <= self.uncertainty_threshold,
            };
            if passes {
                let prompt = q_by_id
                    .get(v.question_id.as_str())
                    .map(|q| q.prompt.clone())
                    .or_else(|| questions.get(idx).map(|q| q.prompt.clone()))
                    .unwrap_or_default();
                records.push(DecisionEvolutionRecord {
                    id: format!("evo-dec-{}", v.question_id),
                    question_id: v.question_id.clone(),
                    prompt,
                    selected_label: v.selected_label.clone(),
                    confidence: v.confidence,
                    margin: v.margin,
                    candidate_probabilities: v.probabilities.clone(),
                    recorded_at: now,
                });
                if let Some(lim) = limit
                    && records.len() >= lim
                {
                    break;
                }
            }
        }
        records
    }

    pub fn synthesize_golden_suite(
        &self,
        records: &[DecisionEvolutionRecord],
        suite_id: Option<&str>,
    ) -> GoldenSuiteDefinition {
        let cases = records
            .iter()
            .enumerate()
            .map(|(i, r)| {
                GoldenEvaluationCase::new(
                    format!("case-harvest-{}", r.question_id),
                    format!("exec-harvest-{}", i + 1),
                    r.selected_label.clone(),
                    r.selected_label.clone(),
                )
                .with_target(GoldenTargetRef {
                    kind: "prompt".to_string(),
                    id: format!("target-{}", r.question_id),
                })
                .with_task(r.prompt.clone())
            })
            .collect();
        GoldenSuiteDefinition::new(suite_id, cases)
    }

    pub fn synthesize_dpo_pairs(&self, records: &[DecisionEvolutionRecord]) -> Vec<DpoPair> {
        records
            .iter()
            .filter_map(|r| {
                let mut sorted: Vec<(&String, &f64)> = r.candidate_probabilities.iter().collect();
                sorted.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
                if sorted.len() >= 2 {
                    let chosen = r.selected_label.clone();
                    let rejected = sorted
                        .iter()
                        .find(|(k, _)| **k != chosen)
                        .map(|(k, _)| (*k).clone())
                        .unwrap_or_else(|| sorted[1].0.clone());
                    Some(DpoPair {
                        id: format!("dpo-{}", r.question_id),
                        prompt: r.prompt.clone(),
                        chosen,
                        rejected,
                        margin: r.margin,
                    })
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn synthesize_fine_tuning_pairs(
        &self,
        records: &[DecisionEvolutionRecord],
    ) -> Vec<FineTuningSample> {
        records
            .iter()
            .map(|r| FineTuningSample {
                id: format!("ft-{}", r.question_id),
                prompt: r.prompt.clone(),
                completion: r.selected_label.clone(),
                weight: (1.0 - r.margin).clamp(0.1, 1.0),
            })
            .collect()
    }

    /// Computes numerically stable Softmax:
    /// P(Choice_i) = exp((z_i / T) - max_j(z_j / T)) / sum_j exp((z_j / T) - max_k(z_k / T))
    pub fn compute_softmax(logits: &[f64], temperature: f64) -> Result<Vec<f64>, DecisionError> {
        if logits.is_empty() {
            return Err(DecisionError::EmptyCandidates);
        }
        if temperature <= 0.0 || temperature.is_nan() || temperature.is_infinite() {
            return Err(DecisionError::InvalidTemperature(
                "temperature must be positive and finite".into(),
            ));
        }
        if logits.iter().any(|z| z.is_nan()) {
            return Err(DecisionError::CalculationError(
                "NaN logit encountered".into(),
            ));
        }
        if logits.iter().any(|z| z.is_infinite() && *z > 0.0) {
            return Err(DecisionError::CalculationError(
                "positive infinite logit encountered".into(),
            ));
        }

        // Scale by temperature
        let scaled: Vec<f64> = logits.iter().map(|&z| z / temperature).collect();

        // Numerically stable max extraction
        let max_logit = scaled
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, |a, b| if a > b { a } else { b });

        if max_logit.is_nan() {
            return Err(DecisionError::CalculationError(
                "NaN logit encountered".into(),
            ));
        }

        let exps: Vec<f64> = scaled.iter().map(|&z| (z - max_logit).exp()).collect();
        let sum_exp: f64 = exps.iter().sum();

        if sum_exp <= 0.0 || sum_exp.is_nan() {
            return Err(DecisionError::CalculationError(
                "sum of exponentials is non-positive or NaN".into(),
            ));
        }

        Ok(exps.iter().map(|&e| e / sum_exp).collect())
    }

    /// Evaluates candidate choices given raw logits using calibrated softmax.
    pub fn score_candidates<T: DecisionChoice>(
        &self,
        candidates: &[T],
        raw_logits: &[f64],
        temperature: Option<f64>,
    ) -> Result<DecisionVerdict<T>, DecisionError> {
        let start = Instant::now();

        if candidates.is_empty() {
            return Err(DecisionError::EmptyCandidates);
        }
        if candidates.len() != raw_logits.len() {
            return Err(DecisionError::DimensionMismatch);
        }

        let temp = temperature.unwrap_or(self.default_temperature);
        let probabilities = Self::compute_softmax(raw_logits, temp)?;

        let mut candidate_entries = Vec::with_capacity(candidates.len());
        let mut best_index = 0;
        let mut best_prob = -1.0;

        for (i, (&choice, (&logit, &prob))) in candidates
            .iter()
            .zip(raw_logits.iter().zip(probabilities.iter()))
            .enumerate()
        {
            if prob > best_prob {
                best_prob = prob;
                best_index = i;
            }
            candidate_entries.push(DecisionCandidate::new(choice, logit, prob));
        }

        let elapsed_ms = start.elapsed().as_millis() as u64;
        let distribution = DecisionDistribution::new(candidate_entries, temp);
        let selected = candidates[best_index];

        Ok(DecisionVerdict::new(
            selected,
            best_prob,
            distribution,
            elapsed_ms,
        ))
    }

    /// Fast-path Tool Safety Gate: Evaluates tool and arguments into [Allow, RequireConfirm, Reject].
    pub fn evaluate_tool_safety(
        &self,
        tool_name: &str,
        tool_args: &serde_json::Value,
    ) -> Result<DecisionVerdict<ToolSafetyChoice>, DecisionError> {
        let tool_name_lower = tool_name.to_lowercase();
        let args_str = tool_args.to_string().to_lowercase();

        // Destructive / high-risk commands -> Reject or RequireConfirm
        let is_destructive = args_str.contains("rm -rf")
            || args_str.contains("mkfs")
            || args_str.contains("dd if=")
            || args_str.contains("drop table")
            || args_str.contains("drop database")
            || args_str.contains("| sh")
            || args_str.contains("| bash")
            || args_str.contains("kill -9")
            || args_str.contains(":(){ :|:& };:")
            || args_str.contains("format c:")
            || args_str.contains("git reset --hard")
            || args_str.contains("git clean -fdx")
            || args_str.contains("rmdir /s")
            || args_str.contains("del /f")
            || args_str.contains("del /s");

        let is_mutating = tool_name_lower.contains("write")
            || tool_name_lower.contains("edit")
            || tool_name_lower.contains("replace")
            || tool_name_lower.contains("delete")
            || tool_name_lower.contains("remove")
            || tool_name_lower.contains("install")
            || tool_name_lower.contains("command")
            || tool_name_lower.contains("run");

        let is_ambiguous = (args_str.contains("force")
            || args_str.contains("bypass")
            || args_str.contains("override")
            || tool_name_lower.contains("ambiguous")
            || tool_name_lower.contains("eval"))
            && !is_destructive;

        let candidates = ToolSafetyChoice::variants();
        let logits = if is_destructive {
            // Strong bias toward Reject
            vec![-4.0, 1.0, 8.0]
        } else if is_ambiguous {
            // Ambiguous/uncertain: close margins and high entropy, triggers escalation
            vec![2.0, 2.1, 1.9]
        } else if is_mutating {
            // Require explicit confirmation
            vec![0.5, 7.0, -1.0]
        } else {
            // Safe read-only inspection -> Allow
            vec![8.0, 0.5, -4.0]
        };

        self.score_candidates(candidates, &logits, None)
    }

    /// Fast-path Intent Tier Router: Directs prompts to [L0Local, L1Surgical, L2Architect].
    pub fn route_intent(
        &self,
        prompt: &str,
    ) -> Result<DecisionVerdict<IntentTierChoice>, DecisionError> {
        let p_lower = prompt.to_lowercase();
        let word_count = prompt.split_whitespace().count();

        let is_architect = p_lower.contains("architect")
            || p_lower.contains("refactor")
            || p_lower.contains("redesign")
            || p_lower.contains("migration")
            || p_lower.contains("supersets")
            || word_count > 60;

        let is_surgical = p_lower.contains("fix")
            || p_lower.contains("patch")
            || p_lower.contains("bug")
            || p_lower.contains("test")
            || p_lower.contains("update")
            || (15..=60).contains(&word_count);

        let candidates = IntentTierChoice::variants();
        let logits = if is_architect {
            vec![-2.0, 1.0, 7.5]
        } else if is_surgical {
            vec![0.5, 7.0, 0.5]
        } else {
            vec![7.5, 1.0, -2.0]
        };

        self.score_candidates(candidates, &logits, None)
    }

    /// Fast-path Anti-Thrash Loop Detector: Checks recent action fingerprints for stuck loops vs progress.
    pub fn detect_anti_thrash(
        &self,
        action_fingerprints: &[&str],
    ) -> Result<DecisionVerdict<ConvergenceChoice>, DecisionError> {
        let candidates = ConvergenceChoice::variants();

        if action_fingerprints.len() < 3 {
            // Too few samples, assume normal progress
            return self.score_candidates(candidates, &[6.0, -2.0, 1.0], None);
        }

        let len = action_fingerprints.len();
        let mut loop_detected = false;

        // Check 1: Direct repetition (A, A, A)
        if len >= 3
            && action_fingerprints[len - 1] == action_fingerprints[len - 2]
            && action_fingerprints[len - 2] == action_fingerprints[len - 3]
        {
            loop_detected = true;
        }

        // Check 2: 2-cycle repetition (A, B, A, B)
        if len >= 4
            && action_fingerprints[len - 1] == action_fingerprints[len - 3]
            && action_fingerprints[len - 2] == action_fingerprints[len - 4]
        {
            loop_detected = true;
        }

        // Check 3: 3-cycle repetition (A, B, C, A, B, C)
        if len >= 6
            && action_fingerprints[len - 1] == action_fingerprints[len - 4]
            && action_fingerprints[len - 2] == action_fingerprints[len - 5]
            && action_fingerprints[len - 3] == action_fingerprints[len - 6]
        {
            loop_detected = true;
        }

        // Check 4: High repetition ratio in sliding window of 6
        let window = if len >= 6 {
            &action_fingerprints[len - 6..]
        } else {
            action_fingerprints
        };
        let mut counts = std::collections::HashMap::new();
        for &item in window {
            *counts.entry(item).or_insert(0) += 1;
        }
        if counts.values().any(|&c| c >= 4) {
            loop_detected = true;
        }

        let logits = if loop_detected {
            // High probability of Stuck
            vec![-2.0, 8.5, 0.5]
        } else if action_fingerprints
            .last()
            .is_some_and(|s| s.contains("complete") || s.contains("finish"))
        {
            // Converged
            vec![1.0, -2.0, 8.0]
        } else {
            // Progressing
            vec![8.0, -3.0, 1.0]
        };

        self.score_candidates(candidates, &logits, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn softmax_probabilities_sum_to_one_and_preserve_order() {
        let logits = vec![1.0, 2.0, 3.0];
        let probs = SinglePassLogitEngine::compute_softmax(&logits, 1.0).unwrap();
        let sum: f64 = probs.iter().sum();
        assert!((sum - 1.0).abs() < 1e-6);
        assert!(probs[2] > probs[1]);
        assert!(probs[1] > probs[0]);
    }

    #[test]
    fn softmax_temperature_calibration_sharpens_or_smooths() {
        let logits = vec![2.0, 4.0];

        // Low temperature (sharp)
        let sharp = SinglePassLogitEngine::compute_softmax(&logits, 0.2).unwrap();
        assert!(sharp[1] > 0.99);

        // High temperature (smooth / uniform)
        let smooth = SinglePassLogitEngine::compute_softmax(&logits, 10.0).unwrap();
        assert!((smooth[0] - smooth[1]).abs() < 0.2);
    }

    #[test]
    fn softmax_numerical_stability_with_large_logits() {
        let large_logits = vec![1000.0, 1001.0, 1002.0];
        let probs = SinglePassLogitEngine::compute_softmax(&large_logits, 1.0).unwrap();
        assert!(!probs[0].is_nan());
        assert!(!probs[1].is_nan());
        assert!(!probs[2].is_nan());
        let sum: f64 = probs.iter().sum();
        assert!((sum - 1.0).abs() < 1e-6);
    }

    #[test]
    fn tool_safety_gate_classifies_correctly() {
        let engine = SinglePassLogitEngine::new();

        let read_args = serde_json::json!({"path": "src/lib.rs"});
        let verdict_read = engine
            .evaluate_tool_safety("read_file", &read_args)
            .unwrap();
        assert_eq!(verdict_read.choice(), ToolSafetyChoice::Allow);

        let edit_args = serde_json::json!({"file": "src/lib.rs", "content": "..."});
        let verdict_edit = engine
            .evaluate_tool_safety("replace_file_content", &edit_args)
            .unwrap();
        assert_eq!(verdict_edit.choice(), ToolSafetyChoice::RequireConfirm);

        let dangerous_args = serde_json::json!({"command": "rm -rf / --no-preserve-root"});
        let verdict_dangerous = engine
            .evaluate_tool_safety("run_command", &dangerous_args)
            .unwrap();
        assert_eq!(verdict_dangerous.choice(), ToolSafetyChoice::Reject);
    }

    #[test]
    fn anti_thrash_detector_identifies_repeating_cycles() {
        let engine = SinglePassLogitEngine::new();

        let progressive = vec!["read_file", "search_code", "edit_file", "run_tests"];
        let verdict1 = engine.detect_anti_thrash(&progressive).unwrap();
        assert_eq!(verdict1.choice(), ConvergenceChoice::Progress);

        let looping = vec![
            "edit_file",
            "run_tests",
            "edit_file",
            "run_tests",
            "edit_file",
            "run_tests",
        ];
        let verdict2 = engine.detect_anti_thrash(&looping).unwrap();
        assert_eq!(verdict2.choice(), ConvergenceChoice::Stuck);
        assert!(verdict2.confidence > 0.90);

        // 3-cycle thrashing loop (edit, test, read, edit, test, read)
        let three_cycle = vec![
            "edit_file",
            "run_tests",
            "read_log",
            "edit_file",
            "run_tests",
            "read_log",
        ];
        let verdict3 = engine.detect_anti_thrash(&three_cycle).unwrap();
        assert_eq!(verdict3.choice(), ConvergenceChoice::Stuck);
        assert!(verdict3.confidence > 0.90);
    }

    #[test]
    fn softmax_rejects_nan_and_infinite_logits() {
        assert!(SinglePassLogitEngine::compute_softmax(&[f64::NAN, 1.0], 1.0).is_err());
        assert!(SinglePassLogitEngine::compute_softmax(&[f64::INFINITY, 1.0], 1.0).is_err());
    }

    #[test]
    fn intent_router_routes_complex_prompts_to_architect() {
        let engine = SinglePassLogitEngine::new();

        let simple = "show version";
        let v_simple = engine.route_intent(simple).unwrap();
        assert_eq!(v_simple.choice(), IntentTierChoice::L0Local);

        let complex =
            "Design and implement full architectural supersets including Pareto evolution and VFS";
        let v_complex = engine.route_intent(complex).unwrap();
        assert_eq!(v_complex.choice(), IntentTierChoice::L2Architect);
    }

    #[test]
    fn heuristic_driver_dynamic_eval_and_margin() {
        let engine = SinglePassLogitEngine::new();
        let question =
            DecisionQuestion::binary("sec-1", "Is it safe to run git status?", "allow", "reject");
        let verdict = engine.evaluate_dynamic(&question).unwrap();
        assert_eq!(verdict.selected_label, "allow");
        assert!(verdict.confidence > 0.60);
        assert!(!verdict.is_uncertain);

        let dangerous = DecisionQuestion::binary(
            "sec-2",
            "rm -rf / --no-preserve-root delete all files",
            "allow",
            "reject",
        );
        let v_danger = engine.evaluate_dynamic(&dangerous).unwrap();
        assert_eq!(v_danger.selected_label, "reject");
        assert!(v_danger.confidence > 0.80);
    }

    #[test]
    fn mock_driver_and_batch_eval() {
        let mock = Arc::new(MockDecisionDriver::new());
        let mut probs = BTreeMap::new();
        probs.insert("option_a".into(), 0.75);
        probs.insert("option_b".into(), 0.25);
        mock.push_verdict(DynamicVerdict::new(
            "q1", "option_a", 0.75, probs, 0.50, false, 2,
        ));

        let engine = SinglePassLogitEngine::with_driver(mock);
        let q1 = DecisionQuestion::choice("q1", "Select route", ["option_a", "option_b"]);
        let verdict = engine.evaluate_dynamic(&q1).unwrap();
        assert_eq!(verdict.selected_label, "option_a");
        assert_eq!(verdict.confidence, 0.75);

        // Fallback default from empty mock queue
        let q2 = DecisionQuestion::choice("q2", "Fallback route", ["opt_x", "opt_y"]);
        let v2 = engine.evaluate_dynamic(&q2).unwrap();
        assert_eq!(v2.selected_label, "opt_x");
    }

    #[test]
    fn harvest_uncertain_decisions_extracts_low_margin() {
        let engine = SinglePassLogitEngine::new();
        let q1 =
            DecisionQuestion::binary("q1", "ambiguous prompt without keywords", "true", "false");
        let q2 = DecisionQuestion::binary("q2", "danger drop tables rm -rf", "allow", "reject");

        let v1 = engine.evaluate_dynamic(&q1).unwrap();
        let v2 = engine.evaluate_dynamic(&q2).unwrap();

        let harvested = engine.harvest_uncertain_decisions(&[v1.clone(), v2.clone()], &[q1, q2]);
        if v1.is_uncertain {
            assert!(harvested.iter().any(|h| h.question_id == "q1"));
        }
        assert!(!v2.is_uncertain);
        assert!(!harvested.iter().any(|h| h.question_id == "q2"));
    }

    #[test]
    fn ollama_prompt_formatting_and_driver_config_instantiation() {
        let q =
            DecisionQuestion::choice("route-1", "Pick fastest protocol", ["quic", "tcp", "http"])
                .with_state(serde_json::json!({"packet_loss": 0.05}));
        let prompt = OllamaDecisionDriver::format_prompt(&q);
        assert!(prompt.contains("Pick fastest protocol"));
        assert!(prompt.contains("packet_loss"));
        assert!(prompt.contains("quic, tcp, http"));

        let config = DecisionDriverConfig {
            kind: DecisionDriverKind::Heuristic,
            endpoint_url: None,
            model_name: None,
            api_key: None,
            timeout_ms: 1000,
            temperature: 0.8,
            uncertainty_threshold: 0.20,
        };
        let engine = SinglePassLogitEngine::from_config(&config).unwrap();
        assert_eq!(engine.default_temperature(), 0.8);
        assert_eq!(engine.uncertainty_threshold(), 0.20);
    }

    #[test]
    fn moira_decision_driver_fallback_and_pnn_evaluation() {
        // 1. Missing .pnn path falls back cleanly to HeuristicDecisionDriver
        let fallback_driver =
            MoiraDecisionDriver::from_pnn_path(std::path::Path::new("nonexistent_moira.pnn"));
        assert!(!fallback_driver.has_loaded_network());
        let q = DecisionQuestion::binary(
            "safety-1",
            "Run rm -rf / on production server",
            "allow",
            "reject",
        );
        let verdict = fallback_driver.evaluate_question(&q).unwrap();
        assert_eq!(verdict.selected_label, "reject");

        // 2. Loaded TropicalNetwork evaluates without multiplications
        let loaded_driver = MoiraDecisionDriver::calibrated();
        assert!(loaded_driver.has_loaded_network());

        let loaded_verdict = loaded_driver.evaluate_question(&q).unwrap();
        assert_eq!(loaded_verdict.selected_label, "reject");
        assert!(loaded_verdict.confidence > 0.9);
    }

    #[test]
    fn moira_decision_driver_config_instantiation() {
        let config = DecisionDriverConfig {
            kind: DecisionDriverKind::Moira,
            endpoint_url: None,
            model_name: Some("nonexistent_moira.pnn".to_string()),
            api_key: None,
            timeout_ms: 1000,
            temperature: 0.9,
            uncertainty_threshold: 0.15,
        };
        let engine = SinglePassLogitEngine::from_config(&config).unwrap();
        assert_eq!(engine.default_temperature(), 0.9);
        assert_eq!(engine.uncertainty_threshold(), 0.15);
    }

    #[test]
    fn emit_dpo_pair_from_escalation_records_fields() {
        let pair = TropicalDecisionDriver::emit_dpo_pair_from_escalation(
            "dpo-esc-42",
            "run_command rm -rf /",
            "reject",
            "allow",
            0.042,
        );
        assert_eq!(pair.id, "dpo-esc-42");
        assert_eq!(pair.prompt, "run_command rm -rf /");
        assert_eq!(pair.chosen, "reject");
        assert_eq!(pair.rejected, "allow");
        assert!((pair.margin - 0.042).abs() < 1e-12);
    }

    #[test]
    fn calibrate_network_from_pairs_rejects_non_finite_step_size() {
        let net = TropicalDecisionDriver::calibrated_network();
        let pairs = vec![TropicalDecisionDriver::emit_dpo_pair_from_escalation(
            "dpo-nan",
            "read Cargo.toml",
            "allow",
            "reject",
            0.1,
        )];
        assert!(
            TropicalDecisionDriver::calibrate_network_from_pairs(&net, &pairs, f32::NAN).is_err()
        );
        assert!(
            TropicalDecisionDriver::calibrate_network_from_pairs(&net, &pairs, f32::INFINITY)
                .is_err()
        );
        assert!(
            TropicalDecisionDriver::calibrate_network_from_pairs(&net, &pairs, f32::NEG_INFINITY)
                .is_err()
        );
    }

    #[test]
    fn calibrate_network_from_pairs_updates_weights_clamps_skips_hinge_and_recomputes_sha256() {
        // 1. Asymmetric 16 -> 2 -> 1 network: row 0 favors is_allow (idx 3) + safe*allow (idx 10),
        //    row 1 favors is_reject (idx 4)
        let mut w1 = vec![-5.0f32; 16 * 2];
        w1[3] = 1.0;
        w1[10] = 1.0;
        w1[16 + 4] = 1.0;
        let l1 = crate::tropical_nn::TropicalLayer::new(16, 2, w1, Some(vec![0.0, 0.0])).unwrap();
        let l2 =
            crate::tropical_nn::TropicalLayer::new(2, 1, vec![0.0, 0.0], Some(vec![0.0])).unwrap();
        let asym_net = crate::tropical_nn::TropicalNetwork::new(
            "asym_test",
            "decision_policy_scoring",
            vec![l1, l2],
        )
        .unwrap();

        // Neutral prompt yields h_pos = [2.0, -5.0], h_neg = [1.0, 2.0], so initial margin = 0.0 < 1.0
        // and j_pos = 0 != j_neg = 1.
        let pair = TropicalDecisionDriver::emit_dpo_pair_from_escalation(
            "dpo-asym-1",
            "view README.md",
            "allow",
            "reject",
            0.05,
        );
        let calibrated = TropicalDecisionDriver::calibrate_network_from_pairs(
            &asym_net,
            std::slice::from_ref(&pair),
            0.25,
        )
        .unwrap();
        assert_eq!(calibrated.layers[1].weights, vec![0.25, -0.25]);
        assert_eq!(calibrated.sha256_digest, calibrated.compute_digest());
        assert_ne!(calibrated.sha256_digest, asym_net.sha256_digest);

        // Save and reload via TropicalNetwork::load_from_file to verify integrity check
        let tmp_pnn = std::env::temp_dir().join(format!(
            "pandora_calib_sha_{}_{}.pnn",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        calibrated.save_to_file(&tmp_pnn).unwrap();
        let reloaded = crate::tropical_nn::TropicalNetwork::load_from_file(&tmp_pnn).unwrap();
        assert_eq!(reloaded.sha256_digest, calibrated.sha256_digest);
        let _ = std::fs::remove_file(&tmp_pnn);

        // 2. Hinge skip: "read inspect status" yields safe_hits=3 -> margin >= 1.0,
        //    so calibration leaves weights untouched.
        let high_margin_pair = TropicalDecisionDriver::emit_dpo_pair_from_escalation(
            "dpo-hinge-skip",
            "read inspect status",
            "allow",
            "reject",
            0.9,
        );
        let skipped = TropicalDecisionDriver::calibrate_network_from_pairs(
            &calibrated,
            &[high_margin_pair],
            0.25,
        )
        .unwrap();
        assert_eq!(skipped.layers[1].weights, calibrated.layers[1].weights);

        // 3. Weight clamp [-8.0, 8.0] on asymmetric layer
        let mut w1_clamp = vec![-25.0f32; 16 * 2];
        w1_clamp[9] = -19.5; // danger_hits=5 => phi[9]=12.5 for reject => h_pos[0]=-7.0, h_neg[0]=-19.5
        w1_clamp[16 + 3] = 8.0; // is_allow=1 for allow => h_pos[1]=8.0, h_neg[1]=9.0
        let l1_clamp =
            crate::tropical_nn::TropicalLayer::new(16, 2, w1_clamp, Some(vec![0.0, 0.0])).unwrap();
        let l2_clamp =
            crate::tropical_nn::TropicalLayer::new(2, 1, vec![7.95, -7.95], Some(vec![0.0]))
                .unwrap();
        let clamp_net = crate::tropical_nn::TropicalNetwork::new(
            "clamp_test",
            "decision_policy_scoring",
            vec![l1_clamp, l2_clamp],
        )
        .unwrap();
        let clamp_pair = TropicalDecisionDriver::emit_dpo_pair_from_escalation(
            "dpo-clamp",
            "rm -rf wipe drop delete format",
            "reject",
            "allow",
            0.05,
        );
        let clamped =
            TropicalDecisionDriver::calibrate_network_from_pairs(&clamp_net, &[clamp_pair], 0.25)
                .unwrap();
        assert_eq!(clamped.layers[1].weights, vec![8.0, -8.0]);

        // 4. Symmetric MoiraDecisionDriver::calibrated_network() triggers Layer 1 subgradient fallback
        let sym_net = TropicalDecisionDriver::calibrated_network();
        let sym_pair = TropicalDecisionDriver::emit_dpo_pair_from_escalation(
            "dpo-sym-1",
            "refactor safe read test",
            "deep",
            "allow",
            0.05,
        );
        let sym_calibrated =
            TropicalDecisionDriver::calibrate_network_from_pairs(&sym_net, &[sym_pair], 0.20)
                .unwrap();
        assert_ne!(sym_calibrated.layers[0].weights, sym_net.layers[0].weights);
        assert_eq!(
            sym_calibrated.sha256_digest,
            sym_calibrated.compute_digest()
        );
    }
}
