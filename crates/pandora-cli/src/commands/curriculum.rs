//! Curriculum command implementation (`pandora curriculum`).
//!
//! Exposes the Voyager-style automatic curriculum synthesis engine (Phase 14.2),
//! converting harvested decision uncertainty records or decision evaluation questions
//! into progressive challenge cases (Easy -> Medium -> Hard) structured as a
//! [`GoldenSuiteDefinition`].

use super::parse_options;
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::{
    CurriculumDifficulty, DecisionError, SinglePassLogitEngine, VoyagerCurriculum,
};
use pandora_types::{DecisionEvolutionRecord, DecisionQuestion, GoldenSuiteDefinition};
use serde_json::json;
use std::fs;
use std::path::Path;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("curriculum requires 'generate', 'inspect', or 'list'"))?;
    match subcommand.as_str() {
        "generate" => generate(&args[1..]),
        "inspect" => inspect(&args[1..]),
        "list" => list(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown curriculum command '{unknown}', expected 'generate', 'inspect', or 'list'"
        ))),
    }
}

fn generate(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "input",
            "out",
            "export-golden",
            "suite-id",
            "difficulty",
            "min-margin",
            "limit",
        ],
    )?;

    let min_margin = if let Some(raw) = parsed.value("min-margin") {
        let v = raw
            .parse::<f64>()
            .map_err(|_| CliError::usage(format!("invalid float for --min-margin: '{raw}'")))?;
        if !(0.0..=1.0).contains(&v) {
            return Err(CliError::usage("--min-margin must be between 0.0 and 1.0"));
        }
        Some(v)
    } else {
        Some(1.0)
    };

    let limit = if let Some(raw) = parsed.value("limit") {
        let l = raw
            .parse::<usize>()
            .map_err(|_| CliError::usage(format!("invalid integer for --limit: '{raw}'")))?;
        if l == 0 {
            return Err(CliError::usage("--limit must be at least 1"));
        }
        Some(l)
    } else {
        None
    };

    let suite_id = parsed
        .value("suite-id")
        .unwrap_or("voyager-curriculum")
        .to_string();

    let difficulty_filter = match parsed.value("difficulty") {
        Some("easy") => Some(CurriculumDifficulty::Easy),
        Some("medium") => Some(CurriculumDifficulty::Medium),
        Some("hard") => Some(CurriculumDifficulty::Hard),
        Some("all") | None => None,
        Some(unknown) => {
            return Err(CliError::usage(format!(
                "invalid difficulty '{unknown}', expected 'easy', 'medium', 'hard', or 'all'"
            )));
        }
    };

    let export_path = parsed
        .value("out")
        .or_else(|| parsed.value("export-golden"))
        .map(str::to_string);

    let engine = SinglePassLogitEngine::new();

    let records: Vec<DecisionEvolutionRecord> = if let Some(input_path) = parsed.value("input") {
        let content = fs::read_to_string(input_path).map_err(|e| {
            CliError::usage(format!("could not read input file '{input_path}': {e}"))
        })?;

        // Attempt 1: Parse directly as DecisionEvolutionRecord array
        if let Ok(recs) = serde_json::from_str::<Vec<DecisionEvolutionRecord>>(&content) {
            recs
        } else if let Ok(questions) = serde_json::from_str::<Vec<DecisionQuestion>>(&content) {
            // Attempt 2: Parse as DecisionQuestion array, evaluate and harvest
            harvest_from_questions(&engine, &questions, min_margin, limit)?
        } else if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
            // Attempt 3: Extract from JSON object with "records" or "harvested"
            if let Some(records_val) = val.get("records").or_else(|| val.get("harvested")) {
                serde_json::from_value::<Vec<DecisionEvolutionRecord>>(records_val.clone())
                    .map_err(|e| {
                        CliError::usage(format!("invalid decision records in input JSON: {e}"))
                    })?
            } else {
                return Err(CliError::usage(
                    "input JSON must be an array of DecisionEvolutionRecord or DecisionQuestion, or an object containing 'records'",
                ));
            }
        } else {
            return Err(CliError::usage("input file contains invalid JSON"));
        }
    } else {
        // Default questions covering tool gating, routing, secret inspection, and canary boundaries
        let questions = default_curriculum_questions();
        harvest_from_questions(&engine, &questions, min_margin, limit)?
    };

    let suite =
        VoyagerCurriculum::from_records_with_options(&records, Some(&suite_id), difficulty_filter);

    let summary = VoyagerCurriculum::summarize(&suite);

    if let Some(ref path_str) = export_path {
        let path = Path::new(path_str);
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let json_str = serde_json::to_string_pretty(&suite).map_err(|e| {
            CliError::execution(
                format!("failed to serialize curriculum suite: {e}"),
                json!({}),
            )
        })?;
        fs::write(path, json_str).map_err(|e| {
            CliError::execution(format!("failed to write curriculum suite: {e}"), json!({}))
        })?;
    }

    let payload = json!({
        "command": "curriculum generate",
        "suite_id": suite_id,
        "total_cases": summary.total_cases,
        "easy_cases": summary.easy_count,
        "medium_cases": summary.medium_count,
        "hard_cases": summary.hard_count,
        "difficulty_filter": parsed.value("difficulty").unwrap_or("all"),
        "output_file": export_path,
        "cases": suite.cases.iter().map(|c| json!({
            "id": c.id,
            "task": c.task,
            "target": c.target,
            "output": c.output,
            "expected_output": c.expected_output,
        })).collect::<Vec<_>>(),
    });

    let rendered = format!(
        "Voyager Curriculum Suite: {}\nTotal Challenge Cases: {} (Easy: {}, Medium: {}, Hard: {})\nDifficulty Filter: {}\nExported Golden Suite: {}",
        suite_id,
        summary.total_cases,
        summary.easy_count,
        summary.medium_count,
        summary.hard_count,
        parsed.value("difficulty").unwrap_or("all"),
        export_path.as_deref().unwrap_or("none"),
    );

    Ok(success("curriculum generate", payload, rendered))
}

fn inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["input", "file"])?;
    let path_str = parsed
        .value("input")
        .or_else(|| parsed.value("file"))
        .ok_or_else(|| CliError::usage("curriculum inspect requires '--input <path>'"))?;

    let content = fs::read_to_string(path_str).map_err(|e| {
        CliError::usage(format!("could not read curriculum file '{path_str}': {e}"))
    })?;

    let suite: GoldenSuiteDefinition = serde_json::from_str(&content)
        .map_err(|e| CliError::usage(format!("invalid GoldenSuiteDefinition JSON: {e}")))?;

    let summary = VoyagerCurriculum::summarize(&suite);

    let payload = json!({
        "command": "curriculum inspect",
        "suite_id": suite.suite_id.as_deref().unwrap_or("unknown"),
        "total_cases": summary.total_cases,
        "easy_cases": summary.easy_count,
        "medium_cases": summary.medium_count,
        "hard_cases": summary.hard_count,
        "cases": suite.cases.iter().map(|c| json!({
            "id": c.id,
            "execution_id": c.execution_id,
            "task": c.task,
            "target": c.target,
            "expected_output": c.expected_output,
        })).collect::<Vec<_>>(),
    });

    let rendered = format!(
        "Curriculum Suite: {}\nTotal Cases: {}\nDifficulty Breakdown: Easy: {}, Medium: {}, Hard: {}\nFile: {}",
        suite.suite_id.as_deref().unwrap_or("unknown"),
        summary.total_cases,
        summary.easy_count,
        summary.medium_count,
        summary.hard_count,
        path_str,
    );

    Ok(success("curriculum inspect", payload, rendered))
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let _ = parse_options(args, &[])?;

    let tiers = vec![
        json!({
            "tier": "easy",
            "margin_threshold": "margin >= 0.10",
            "description": "High-confidence decision boundary challenges with clear candidate separation",
        }),
        json!({
            "tier": "medium",
            "margin_threshold": "0.03 <= margin < 0.10",
            "description": "Moderate uncertainty boundary challenges requiring multi-step verification",
        }),
        json!({
            "tier": "hard",
            "margin_threshold": "margin < 0.03",
            "description": "Near-indistinguishable logit entropy requiring deep reflexion diagnostics or MCTS search",
        }),
    ];

    let payload = json!({
        "command": "curriculum list",
        "tiers": tiers,
    });

    let rendered = "Voyager Curriculum Difficulty Tiers:\n  easy   - margin >= 0.10 (High confidence boundary)\n  medium - 0.03 <= margin < 0.10 (Moderate uncertainty boundary)\n  hard   - margin < 0.03 (Near-indistinguishable logit entropy)";

    Ok(success("curriculum list", payload, rendered))
}

fn harvest_from_questions(
    engine: &SinglePassLogitEngine,
    questions: &[DecisionQuestion],
    min_margin: Option<f64>,
    limit: Option<usize>,
) -> Result<Vec<DecisionEvolutionRecord>, CliError> {
    let mut verdicts = Vec::with_capacity(questions.len());
    for q in questions {
        let v = engine
            .evaluate_dynamic(q)
            .map_err(|e: DecisionError| CliError::execution(e.to_string(), json!({})))?;
        verdicts.push(v);
    }
    Ok(engine.harvest_uncertain_decisions_with_margin(&verdicts, questions, min_margin, limit))
}

fn default_curriculum_questions() -> Vec<DecisionQuestion> {
    vec![
        DecisionQuestion::binary(
            "q-curr-easy-gate",
            "safely allow inspecting read-only git status",
            "allow",
            "reject",
        ),
        DecisionQuestion::choice(
            "q-curr-medium-routing",
            "balance load among 10 candidate workers",
            ["w1", "w2", "w3", "w4", "w5", "w6", "w7", "w8", "w9", "w10"],
        ),
        DecisionQuestion::choice(
            "q-curr-hard-entropy",
            "near identical logit distribution across 25 micro-partitions",
            (1..=25)
                .map(|i| format!("partition_{i}"))
                .collect::<Vec<_>>(),
        ),
        DecisionQuestion::binary(
            "q-curr-archive-compact",
            "Compact historical session archives during resource constraint?",
            "allow",
            "reject",
        ),
        DecisionQuestion::choice(
            "q-curr-refactor-tier",
            "Route bugfix task: surgical local edit vs architecture redesign",
            ["l0_local", "l1_surgical", "l2_architect"],
        ),
        DecisionQuestion::binary(
            "q-curr-canary-promote",
            "Promote candidate with 88% pass rate when baseline requirement is 85%",
            "allow",
            "reject",
        ),
    ]
}
