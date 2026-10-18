//! Voyager-style automatic curriculum generator (Phase 14.2).
//!
//! Turns harvested uncertain decisions into progressive challenge cases
//! (Easy -> Medium -> Hard) targeting the areas of maximum uncertainty. The
//! output is a [`GoldenSuiteDefinition`] consumable by the governed evaluation
//! adapter.

use crate::decision_engine::SinglePassLogitEngine;
use pandora_types::{
    DecisionEvolutionRecord, DecisionQuestion, GoldenEvaluationCase, GoldenSuiteDefinition,
    GoldenTargetRef,
};

/// Difficulty tier assigned to a challenge case, derived from decision margin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurriculumDifficulty {
    Easy,
    Medium,
    Hard,
}

impl CurriculumDifficulty {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Easy => "easy",
            Self::Medium => "medium",
            Self::Hard => "hard",
        }
    }

    /// Maps a decision margin to a difficulty tier. Lower margin = harder.
    pub fn from_margin(margin: f64) -> Self {
        if margin >= 0.1 {
            Self::Easy
        } else if margin >= 0.03 {
            Self::Medium
        } else {
            Self::Hard
        }
    }
}

/// Synthesizes a progressive curriculum suite from already-harvested records.
pub struct VoyagerCurriculum;

impl VoyagerCurriculum {
    /// Builds a suite where each harvested record becomes a challenge case
    /// tagged with a difficulty derived from its decision margin.
    pub fn from_records(records: &[DecisionEvolutionRecord]) -> GoldenSuiteDefinition {
        let cases = records
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let difficulty = CurriculumDifficulty::from_margin(r.margin);
                GoldenEvaluationCase::new(
                    format!("curriculum-{}-{}", difficulty.as_str(), i + 1),
                    format!("exec-curriculum-{}", i + 1),
                    r.selected_label.clone(),
                    r.selected_label.clone(),
                )
                .with_task(format!(
                    "challenge case (difficulty={}) for question {}",
                    difficulty.as_str(),
                    r.question_id
                ))
                .with_target(GoldenTargetRef {
                    kind: "prompt".to_string(),
                    id: format!("target-{}", r.question_id),
                })
            })
            .collect();
        GoldenSuiteDefinition::new(Some("voyager-curriculum"), cases)
    }

    /// Convenience: harvests uncertain decisions via the engine, then builds.
    pub fn harvest_and_build(
        engine: &SinglePassLogitEngine,
        verdicts: &[pandora_types::DynamicVerdict],
        questions: &[DecisionQuestion],
        min_margin: Option<f64>,
        limit: Option<usize>,
    ) -> GoldenSuiteDefinition {
        let records =
            engine.harvest_uncertain_decisions_with_margin(verdicts, questions, min_margin, limit);
        Self::from_records(&records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_records_assigns_difficulty() {
        let records = vec![pandora_types::DecisionEvolutionRecord {
            id: "r1".into(),
            question_id: "q1".into(),
            prompt: "p".into(),
            selected_label: "progress".into(),
            confidence: 0.9,
            margin: 0.01,
            candidate_probabilities: std::collections::BTreeMap::new(),
            recorded_at: pandora_types::Timestamp::from_unix_seconds(0),
        }];
        let suite = VoyagerCurriculum::from_records(&records);
        assert_eq!(suite.cases.len(), 1);
        assert!(suite.cases[0].id.contains("hard"));
    }

    #[test]
    fn empty_records_yields_empty_suite() {
        let suite = VoyagerCurriculum::from_records(&[]);
        assert!(suite.cases.is_empty());
    }

    #[test]
    fn difficulty_mapping() {
        assert_eq!(
            CurriculumDifficulty::from_margin(0.5),
            CurriculumDifficulty::Easy
        );
        assert_eq!(
            CurriculumDifficulty::from_margin(0.05),
            CurriculumDifficulty::Medium
        );
        assert_eq!(
            CurriculumDifficulty::from_margin(0.001),
            CurriculumDifficulty::Hard
        );
    }
}
