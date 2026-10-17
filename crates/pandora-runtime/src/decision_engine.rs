use pandora_types::{
    ConvergenceChoice, DecisionCandidate, DecisionChoice, DecisionDistribution, DecisionVerdict,
    IntentTierChoice, ToolSafetyChoice,
};
use std::fmt;
use std::time::Instant;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecisionError {
    EmptyCandidates,
    InvalidTemperature(String),
    DimensionMismatch,
    CalculationError(String),
    ExecutionFailed(String),
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
        }
    }
}

impl std::error::Error for DecisionError {}

#[derive(Clone, Debug)]
pub struct SinglePassLogitEngine {
    default_temperature: f64,
}

pub type DecisionEngine = SinglePassLogitEngine;

impl Default for SinglePassLogitEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SinglePassLogitEngine {
    pub fn new() -> Self {
        Self {
            default_temperature: 1.0,
        }
    }

    pub fn with_temperature(temperature: f64) -> Result<Self, DecisionError> {
        if temperature <= 0.0 || temperature.is_nan() || temperature.is_infinite() {
            return Err(DecisionError::InvalidTemperature(
                "temperature must be positive and finite".into(),
            ));
        }
        Ok(Self {
            default_temperature: temperature,
        })
    }

    pub fn default_temperature(&self) -> f64 {
        self.default_temperature
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

        let candidates = ToolSafetyChoice::variants();
        let logits = if is_destructive {
            // Strong bias toward Reject
            vec![-4.0, 1.0, 8.0]
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
}
