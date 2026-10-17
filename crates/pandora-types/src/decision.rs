use serde::{Deserialize, Serialize};
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
}
