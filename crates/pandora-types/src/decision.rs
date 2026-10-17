use crate::Timestamp;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSafetyChoice {
    Allow,
    RequireConfirm,
    Reject,
}

impl ToolSafetyChoice {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::RequireConfirm => "require_confirm",
            Self::Reject => "reject",
        }
    }

    pub const fn variants() -> &'static [Self] {
        &[Self::Allow, Self::RequireConfirm, Self::Reject]
    }
}

impl fmt::Display for ToolSafetyChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentTierChoice {
    L0Local,
    L1Surgical,
    L2Architect,
}

impl IntentTierChoice {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::L0Local => "l0_local",
            Self::L1Surgical => "l1_surgical",
            Self::L2Architect => "l2_architect",
        }
    }

    pub const fn variants() -> &'static [Self] {
        &[Self::L0Local, Self::L1Surgical, Self::L2Architect]
    }
}

impl fmt::Display for IntentTierChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConvergenceChoice {
    Progress,
    Stuck,
    Converged,
}

impl ConvergenceChoice {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Progress => "progress",
            Self::Stuck => "stuck",
            Self::Converged => "converged",
        }
    }

    pub const fn variants() -> &'static [Self] {
        &[Self::Progress, Self::Stuck, Self::Converged]
    }
}

impl fmt::Display for ConvergenceChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

pub trait DecisionChoice:
    Copy + Clone + fmt::Debug + Eq + PartialEq + Send + Sync + 'static
{
    fn as_str(self) -> &'static str;
    fn token(self) -> &'static str;
    fn all_choices() -> &'static [Self];
}

impl DecisionChoice for ToolSafetyChoice {
    fn as_str(self) -> &'static str {
        self.as_str()
    }
    fn token(self) -> &'static str {
        self.as_str()
    }
    fn all_choices() -> &'static [Self] {
        Self::variants()
    }
}

impl DecisionChoice for IntentTierChoice {
    fn as_str(self) -> &'static str {
        self.as_str()
    }
    fn token(self) -> &'static str {
        self.as_str()
    }
    fn all_choices() -> &'static [Self] {
        Self::variants()
    }
}

impl DecisionChoice for ConvergenceChoice {
    fn as_str(self) -> &'static str {
        self.as_str()
    }
    fn token(self) -> &'static str {
        self.as_str()
    }
    fn all_choices() -> &'static [Self] {
        Self::variants()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionCandidate<T> {
    pub choice: T,
    pub logit: f64,
    pub probability: f64,
}

impl<T> DecisionCandidate<T> {
    pub fn new(choice: T, logit: f64, probability: f64) -> Self {
        Self {
            choice,
            logit,
            probability,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionDistribution<T> {
    pub candidates: Vec<DecisionCandidate<T>>,
    pub temperature: f64,
}

impl<T> DecisionDistribution<T> {
    pub fn new(candidates: Vec<DecisionCandidate<T>>, temperature: f64) -> Self {
        Self {
            candidates,
            temperature,
        }
    }

    pub fn top_candidate(&self) -> Option<&DecisionCandidate<T>> {
        self.candidates.iter().max_by(|a, b| {
            a.probability
                .partial_cmp(&b.probability)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionVerdict<T> {
    pub selected: T,
    pub confidence: f64,
    pub distribution: DecisionDistribution<T>,
    pub latency_ms: u64,
}

impl<T: Clone> DecisionVerdict<T> {
    pub fn new(
        selected: T,
        confidence: f64,
        distribution: DecisionDistribution<T>,
        latency_ms: u64,
    ) -> Self {
        Self {
            selected,
            confidence,
            distribution,
            latency_ms,
        }
    }

    pub fn choice(&self) -> T {
        self.selected.clone()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QuestionSchema {
    Binary {
        true_label: String,
        false_label: String,
    },
    Choice {
        options: Vec<String>,
    },
    Score {
        min_label: String,
        max_label: String,
    },
}

impl QuestionSchema {
    pub fn binary(true_label: impl Into<String>, false_label: impl Into<String>) -> Self {
        Self::Binary {
            true_label: true_label.into(),
            false_label: false_label.into(),
        }
    }

    pub fn choice(options: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::Choice {
            options: options.into_iter().map(Into::into).collect(),
        }
    }

    pub fn score(min_label: impl Into<String>, max_label: impl Into<String>) -> Self {
        Self::Score {
            min_label: min_label.into(),
            max_label: max_label.into(),
        }
    }

    pub fn labels(&self) -> Vec<String> {
        match self {
            Self::Binary {
                true_label,
                false_label,
            } => vec![true_label.clone(), false_label.clone()],
            Self::Choice { options } => options.clone(),
            Self::Score {
                min_label,
                max_label,
            } => vec![min_label.clone(), max_label.clone()],
        }
    }

    pub fn is_valid(&self) -> bool {
        match self {
            Self::Binary {
                true_label,
                false_label,
            } => {
                !true_label.trim().is_empty()
                    && !false_label.trim().is_empty()
                    && true_label != false_label
            }
            Self::Choice { options } => {
                options.len() >= 2 && options.iter().all(|o| !o.trim().is_empty()) && {
                    let mut set = std::collections::BTreeSet::new();
                    options.iter().all(|o| set.insert(o))
                }
            }
            Self::Score {
                min_label,
                max_label,
            } => {
                !min_label.trim().is_empty()
                    && !max_label.trim().is_empty()
                    && min_label != max_label
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionQuestion {
    pub id: String,
    pub prompt: String,
    pub schema: QuestionSchema,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_context: Option<serde_json::Value>,
}

impl DecisionQuestion {
    pub fn new(id: impl Into<String>, prompt: impl Into<String>, schema: QuestionSchema) -> Self {
        Self {
            id: id.into(),
            prompt: prompt.into(),
            schema,
            state_context: None,
        }
    }

    pub fn binary(
        id: impl Into<String>,
        prompt: impl Into<String>,
        true_label: impl Into<String>,
        false_label: impl Into<String>,
    ) -> Self {
        Self::new(id, prompt, QuestionSchema::binary(true_label, false_label))
    }

    pub fn choice(
        id: impl Into<String>,
        prompt: impl Into<String>,
        options: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self::new(id, prompt, QuestionSchema::choice(options))
    }

    pub fn with_state(mut self, state: serde_json::Value) -> Self {
        self.state_context = Some(state);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DynamicVerdict {
    pub question_id: String,
    pub selected_label: String,
    pub confidence: f64,
    pub probabilities: BTreeMap<String, f64>,
    pub margin: f64,
    pub is_uncertain: bool,
    pub latency_ms: u64,
}

impl DynamicVerdict {
    pub fn new(
        question_id: impl Into<String>,
        selected_label: impl Into<String>,
        confidence: f64,
        probabilities: BTreeMap<String, f64>,
        margin: f64,
        is_uncertain: bool,
        latency_ms: u64,
    ) -> Self {
        Self {
            question_id: question_id.into(),
            selected_label: selected_label.into(),
            confidence,
            probabilities,
            margin,
            is_uncertain,
            latency_ms,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionDriverKind {
    Heuristic,
    Ollama,
    OpenAiCompatible,
    Mock,
}

impl DecisionDriverKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Heuristic => "heuristic",
            Self::Ollama => "ollama",
            Self::OpenAiCompatible => "openai_compatible",
            Self::Mock => "mock",
        }
    }
}

impl fmt::Display for DecisionDriverKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

const fn default_timeout_ms() -> u64 {
    5000
}

const fn default_temperature() -> f64 {
    1.0
}

const fn default_uncertainty_threshold() -> f64 {
    0.15
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionDriverConfig {
    pub kind: DecisionDriverKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    #[serde(default = "default_uncertainty_threshold")]
    pub uncertainty_threshold: f64,
}

impl Default for DecisionDriverConfig {
    fn default() -> Self {
        Self {
            kind: DecisionDriverKind::Heuristic,
            endpoint_url: None,
            model_name: None,
            api_key: None,
            timeout_ms: 5000,
            temperature: 1.0,
            uncertainty_threshold: 0.15,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionEvolutionRecord {
    pub id: String,
    pub question_id: String,
    pub prompt: String,
    pub selected_label: String,
    pub confidence: f64,
    pub margin: f64,
    pub candidate_probabilities: BTreeMap<String, f64>,
    pub recorded_at: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_enums_variants_and_display() {
        assert_eq!(ToolSafetyChoice::Allow.as_str(), "allow");
        assert_eq!(ToolSafetyChoice::RequireConfirm.as_str(), "require_confirm");
        assert_eq!(ToolSafetyChoice::Reject.as_str(), "reject");
        assert_eq!(ToolSafetyChoice::variants().len(), 3);

        assert_eq!(IntentTierChoice::L0Local.as_str(), "l0_local");
        assert_eq!(IntentTierChoice::L1Surgical.as_str(), "l1_surgical");
        assert_eq!(IntentTierChoice::L2Architect.as_str(), "l2_architect");
        assert_eq!(IntentTierChoice::variants().len(), 3);

        assert_eq!(ConvergenceChoice::Progress.as_str(), "progress");
        assert_eq!(ConvergenceChoice::Stuck.as_str(), "stuck");
        assert_eq!(ConvergenceChoice::Converged.as_str(), "converged");
        assert_eq!(ConvergenceChoice::variants().len(), 3);
    }

    #[test]
    fn decision_candidate_and_verdict_roundtrip() {
        let candidate1 = DecisionCandidate::new(ToolSafetyChoice::Allow, 5.0, 0.85);
        let candidate2 = DecisionCandidate::new(ToolSafetyChoice::RequireConfirm, 2.0, 0.10);
        let candidate3 = DecisionCandidate::new(ToolSafetyChoice::Reject, 0.5, 0.05);

        let dist = DecisionDistribution::new(vec![candidate1, candidate2, candidate3], 1.0);
        assert_eq!(
            dist.top_candidate().unwrap().choice,
            ToolSafetyChoice::Allow
        );

        let verdict = DecisionVerdict::new(ToolSafetyChoice::Allow, 0.85, dist, 12);
        assert_eq!(verdict.choice(), ToolSafetyChoice::Allow);
        assert_eq!(verdict.selected, ToolSafetyChoice::Allow);
        assert_eq!(verdict.confidence, 0.85);
        assert_eq!(verdict.latency_ms, 12);

        let json = serde_json::to_string(&verdict).unwrap();
        let decoded: DecisionVerdict<ToolSafetyChoice> = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.selected, ToolSafetyChoice::Allow);
    }

    #[test]
    fn question_schema_validation_and_labels() {
        let binary = QuestionSchema::binary("allow", "deny");
        assert!(binary.is_valid());
        assert_eq!(binary.labels(), vec!["allow", "deny"]);

        let invalid_binary = QuestionSchema::binary("same", "same");
        assert!(!invalid_binary.is_valid());

        let choice = QuestionSchema::choice(["a", "b", "c"]);
        assert!(choice.is_valid());
        assert_eq!(choice.labels(), vec!["a", "b", "c"]);

        let invalid_choice = QuestionSchema::choice(["single"]);
        assert!(!invalid_choice.is_valid());

        let duplicate_choice = QuestionSchema::choice(["a", "b", "a"]);
        assert!(!duplicate_choice.is_valid());

        let score = QuestionSchema::score("low", "high");
        assert!(score.is_valid());
        assert_eq!(score.labels(), vec!["low", "high"]);
    }

    #[test]
    fn decision_question_and_dynamic_verdict_roundtrip() {
        let question = DecisionQuestion::choice("q1", "Select optimal route", ["fast", "cheap"])
            .with_state(serde_json::json!({"traffic": "low"}));
        assert_eq!(question.id, "q1");
        assert_eq!(question.prompt, "Select optimal route");
        assert!(question.state_context.is_some());

        let q_json = serde_json::to_string(&question).unwrap();
        let q_decoded: DecisionQuestion = serde_json::from_str(&q_json).unwrap();
        assert_eq!(q_decoded.id, "q1");

        let mut probs = BTreeMap::new();
        probs.insert("fast".to_string(), 0.82);
        probs.insert("cheap".to_string(), 0.18);

        let verdict = DynamicVerdict::new("q1", "fast", 0.82, probs, 0.64, false, 5);
        assert_eq!(verdict.selected_label, "fast");
        assert_eq!(verdict.margin, 0.64);
        assert!(!verdict.is_uncertain);

        let v_json = serde_json::to_string(&verdict).unwrap();
        let v_decoded: DynamicVerdict = serde_json::from_str(&v_json).unwrap();
        assert_eq!(v_decoded.selected_label, "fast");
    }

    #[test]
    fn decision_driver_config_defaults() {
        let config = DecisionDriverConfig::default();
        assert_eq!(config.kind, DecisionDriverKind::Heuristic);
        assert_eq!(config.timeout_ms, 5000);
        assert_eq!(config.temperature, 1.0);
        assert_eq!(config.uncertainty_threshold, 0.15);
    }
}
