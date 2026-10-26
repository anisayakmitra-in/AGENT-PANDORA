#![forbid(unsafe_code)]

use super::parse_options;
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::PrpEngine;
use pandora_types::PrpDocument;
use serde_json::json;
use std::fs;

pub(crate) fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .map(String::as_str)
        .ok_or_else(|| CliError::usage("prp requires 'inspect', 'scaffold', or 'validate'"))?;

    match subcommand {
        "inspect" => inspect(&args[1..]),
        "scaffold" => scaffold(&args[1..]),
        "validate" => validate(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown prp command '{unknown}', expected 'inspect', 'scaffold', or 'validate'"
        ))),
    }
}

fn inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["id", "input"])?;
    let doc = if let Some(path) = parsed.value("input") {
        let content = fs::read_to_string(path).map_err(|err| {
            CliError::execution(
                format!("failed to read PRP document '{path}': {err}"),
                json!({}),
            )
        })?;
        serde_json::from_str::<PrpDocument>(&content).map_err(|err| {
            CliError::execution(
                format!("invalid PRP document JSON in '{path}': {err}"),
                json!({}),
            )
        })?
    } else {
        let id = parsed.value("id").unwrap_or("PRP-PHASE16-RSI");
        PrpDocument::scaffold_default(id, "Recursive Self-Improvement & Evolution Core")
    };

    let mut human = format!(
        "Product Requirement Prompt (PRP) '{}' [Level: {}]:\n",
        doc.id,
        doc.level.as_str()
    );
    human.push_str(&format!("  Title: {}\n", doc.title));
    human.push_str(&format!(
        "  Hierarchy: Phase='{}' > Milestone='{}' > Task='{}'\n",
        doc.phase, doc.milestone, doc.task
    ));
    human.push_str("\nPre-Flight Invariants:\n");
    for inv in &doc.pre_flight_invariants {
        human.push_str(&format!("  * [INVARIANT] {inv}\n"));
    }
    human.push_str("\nProgressive Validation Gates:\n");
    for gate in &doc.required_gates {
        human.push_str(&format!("  * [{}] {}\n", gate.gate_number(), gate.as_str()));
    }
    human.push_str("\nAcceptance Criteria:\n");
    for crit in &doc.acceptance_criteria {
        human.push_str(&format!("  - {crit}\n"));
    }
    human.push_str(&format!("\nRollback Plan: {}\n", doc.rollback_plan));

    Ok(success("prp inspect", json!({ "prp": doc }), human))
}

fn scaffold(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["id", "title", "out"])?;
    let id = parsed.value("id").unwrap_or("PRP-001");
    let title = parsed.value("title").unwrap_or("New Task Specification");
    let out_path = parsed.value("out");

    let doc = PrpDocument::scaffold_default(id, title);

    if let Some(path) = out_path {
        let serialized = serde_json::to_string_pretty(&doc).map_err(|err| {
            CliError::execution(
                format!("failed to serialize PRP document: {err}"),
                json!({}),
            )
        })?;
        fs::write(path, serialized).map_err(|err| {
            CliError::execution(format!("failed to write PRP to '{path}': {err}"), json!({}))
        })?;
        Ok(success(
            "prp scaffold",
            json!({ "prp": doc, "out": path }),
            format!("Scaffolded PRP '{}' to '{}'", doc.id, path),
        ))
    } else {
        let serialized = serde_json::to_string_pretty(&doc).unwrap_or_default();
        Ok(success("prp scaffold", json!({ "prp": doc }), serialized))
    }
}

fn validate(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["id", "input", "fail-gate"])?;
    let fail_gate = parsed.value("fail-gate");

    let doc = if let Some(path) = parsed.value("input") {
        let content = fs::read_to_string(path).map_err(|err| {
            CliError::execution(
                format!("failed to read PRP document '{path}': {err}"),
                json!({}),
            )
        })?;
        serde_json::from_str::<PrpDocument>(&content).map_err(|err| {
            CliError::execution(
                format!("invalid PRP document JSON in '{path}': {err}"),
                json!({}),
            )
        })?
    } else {
        let id = parsed.value("id").unwrap_or("PRP-PHASE16-RSI");
        PrpDocument::scaffold_default(id, "Recursive Self-Improvement & Evolution Core")
    };

    PrpEngine::verify_pre_flight_invariants(&doc).map_err(|err| {
        CliError::execution(
            format!("pre-flight invariant assertion failed: {err}"),
            json!({}),
        )
    })?;

    let syntax_ok = fail_gate != Some("syntax");
    let unit_ok = fail_gate != Some("unit");
    let integration_ok = fail_gate != Some("integration");
    let receipt_ok = fail_gate != Some("receipt");

    let report = PrpEngine::validate_prp(&doc, syntax_ok, unit_ok, integration_ok, receipt_ok);

    let mut human = format!(
        "Progressive Validation Report for PRP '{}':\n",
        report.prp_id
    );
    for res in &report.gate_results {
        let status = if res.passed { "PASS" } else { "FAIL" };
        human.push_str(&format!(
            "  [{}] {}: {} ({} ms)\n",
            status,
            res.gate.as_str(),
            res.details,
            res.duration_ms
        ));
    }
    human.push_str(&format!("\nSummary: {}\n", report.summary));

    Ok(success(
        "prp validate",
        json!({
            "report": report
        }),
        human,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prp_inspect_default() {
        let res = inspect(&[]).unwrap();
        assert_eq!(res.command, "prp inspect");
        assert!(res.human.contains("PRP-PHASE16-RSI"));
    }

    #[test]
    fn test_prp_validate_all_pass() {
        let res = validate(&[]).unwrap();
        assert_eq!(res.command, "prp validate");
        assert!(res.human.contains("[PASS]"));
        assert!(
            res.human
                .contains("successfully cleared all 4 progressive validation gates")
        );
    }

    #[test]
    fn test_prp_validate_gate_failure() {
        let res = validate(&["--fail-gate".to_string(), "unit".to_string()]).unwrap();
        assert_eq!(res.command, "prp validate");
        assert!(res.human.contains("[FAIL] Gate 2: Unit"));
    }
}
