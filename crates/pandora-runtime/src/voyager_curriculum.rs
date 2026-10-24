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
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

/// Summary statistics for a synthesized curriculum suite.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CurriculumSummary {
    pub total_cases: usize,
    pub easy_count: usize,
    pub medium_count: usize,
    pub hard_count: usize,
}

/// Synthesizes a progressive curriculum suite from already-harvested records.
pub struct VoyagerCurriculum;

impl VoyagerCurriculum {
    /// Builds a suite where each harvested record becomes a challenge case
    /// tagged with a difficulty derived from its decision margin.
    pub fn from_records(records: &[DecisionEvolutionRecord]) -> GoldenSuiteDefinition {
        Self::from_records_with_options(records, None, None)
    }

    /// Builds a suite with optional custom suite ID and optional difficulty filter.
    pub fn from_records_with_options(
        records: &[DecisionEvolutionRecord],
        suite_id: Option<&str>,
        difficulty_filter: Option<CurriculumDifficulty>,
    ) -> GoldenSuiteDefinition {
        let cases: Vec<GoldenEvaluationCase> = records
            .iter()
            .enumerate()
            .filter_map(|(i, r)| {
                let difficulty = CurriculumDifficulty::from_margin(r.margin);
                if let Some(target_diff) = difficulty_filter
                    && difficulty != target_diff
                {
                    return None;
                }
                Some(
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
                    }),
                )
            })
            .collect();
        GoldenSuiteDefinition::new(Some(suite_id.unwrap_or("voyager-curriculum")), cases)
    }

    /// Computes summary statistics over challenge case difficulties in a suite.
    pub fn summarize(suite: &GoldenSuiteDefinition) -> CurriculumSummary {
        let mut easy_count = 0;
        let mut medium_count = 0;
        let mut hard_count = 0;
        for case in &suite.cases {
            if case.id.contains("easy") {
                easy_count += 1;
            } else if case.id.contains("medium") {
                medium_count += 1;
            } else if case.id.contains("hard") {
                hard_count += 1;
            }
        }
        CurriculumSummary {
            total_cases: suite.cases.len(),
            easy_count,
            medium_count,
            hard_count,
        }
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

    #[test]
    fn from_records_with_options_filters_difficulty_and_summarizes() {
        let records = vec![
            pandora_types::DecisionEvolutionRecord {
                id: "r1".into(),
                question_id: "q1".into(),
                prompt: "p1".into(),
                selected_label: "allow".into(),
                confidence: 0.9,
                margin: 0.5, // easy
                candidate_probabilities: std::collections::BTreeMap::new(),
                recorded_at: pandora_types::Timestamp::from_unix_seconds(0),
            },
            pandora_types::DecisionEvolutionRecord {
                id: "r2".into(),
                question_id: "q2".into(),
                prompt: "p2".into(),
                selected_label: "reject".into(),
                confidence: 0.55,
                margin: 0.05, // medium
                candidate_probabilities: std::collections::BTreeMap::new(),
                recorded_at: pandora_types::Timestamp::from_unix_seconds(0),
            },
            pandora_types::DecisionEvolutionRecord {
                id: "r3".into(),
                question_id: "q3".into(),
                prompt: "p3".into(),
                selected_label: "stuck".into(),
                confidence: 0.51,
                margin: 0.01, // hard
                candidate_probabilities: std::collections::BTreeMap::new(),
                recorded_at: pandora_types::Timestamp::from_unix_seconds(0),
            },
        ];

        let suite_all =
            VoyagerCurriculum::from_records_with_options(&records, Some("custom-suite"), None);
        assert_eq!(suite_all.suite_id.as_deref(), Some("custom-suite"));
        assert_eq!(suite_all.cases.len(), 3);

        let summary = VoyagerCurriculum::summarize(&suite_all);
        assert_eq!(summary.total_cases, 3);
        assert_eq!(summary.easy_count, 1);
        assert_eq!(summary.medium_count, 1);
        assert_eq!(summary.hard_count, 1);

        let suite_hard = VoyagerCurriculum::from_records_with_options(
            &records,
            Some("hard-only"),
            Some(CurriculumDifficulty::Hard),
        );
        assert_eq!(suite_hard.cases.len(), 1);
        assert!(suite_hard.cases[0].id.contains("hard"));
    }
}
