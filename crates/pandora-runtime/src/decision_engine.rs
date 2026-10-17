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
            driver: Arc::new(HeuristicDecisionDriver::new()),
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
        Ok(Self {
            default_temperature: temperature,
            uncertainty_threshold: 0.15,
            driver: Arc::new(HeuristicDecisionDriver::with_temperature(temperature)),
        })
    }

    pub fn from_config(config: &DecisionDriverConfig) -> Result<Self, DecisionError> {
        let driver: Arc<dyn DecisionDriver> = match config.kind {
            DecisionDriverKind::Heuristic => Arc::new(HeuristicDecisionDriver::with_options(
                config.temperature,
                config.uncertainty_threshold,
            )),
            DecisionDriverKind::Mock => Arc::new(MockDecisionDriver::new()),
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
        let threshold = max_margin.unwrap_or(self.uncertainty_threshold);
        let unix_now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let now = Timestamp::from_unix_seconds(unix_now);
        let mut records = Vec::new();

        for (v, q) in verdicts.iter().zip(questions.iter()) {
            if v.is_uncertain || v.margin <= threshold {
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
                    Some(DpoPair {
                        id: format!("dpo-{}", r.question_id),
                        prompt: r.prompt.clone(),
                        chosen: sorted[0].0.clone(),
                        rejected: sorted[1].0.clone(),
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
}
