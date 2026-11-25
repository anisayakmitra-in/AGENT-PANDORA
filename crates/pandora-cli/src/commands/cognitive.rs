//! Subsystem 1: Cognitive Micro-Kernel & Neuro-Symbolic Inference Engine CLI.
//!
//! Subcommands:
//! - `status`: Inspect Cognitive Subsystem status, BNF grammar masking, and speculative decoding speedup.
//! - `speculate`: Run speculative tree rejection sampling evaluation.
//! - `synthesize`: Synthesize tokens constrained by pushdown automata grammar rules.
//! - `audit`: Perform metacognitive entropy and semantic drift supervision.

use super::parse_options;
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::subsystems::cognitive::CognitiveSubsystem;
use serde_json::json;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage("cognitive requires 'status', 'speculate', 'synthesize', or 'audit'")
    })?;

    match subcommand.as_str() {
        "status" => status(&args[1..]),
        "speculate" => speculate(&args[1..]),
        "synthesize" => synthesize(&args[1..]),
        "audit" => audit(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown cognitive subcommand '{unknown}'. Available: 'status', 'speculate', 'synthesize', 'audit'"
        ))),
    }
}

fn status(args: &[String]) -> Result<CommandResult, CliError> {
    let _parsed = parse_options(args, &["config", "data-dir", "workspace", "json"])?;
    let mut subsystem = CognitiveSubsystem::new();
    let status = subsystem.status();

    let display = format!(
        "Cognitive Subsystem [v4.0.5 AETHER]\n\
         Status: {}\n\
         Grammars Supported: {}\n\
         Speculative Speedup: {:.2}x\n\
         PAC-Bayes Risk Bound: {:.4}\n\
         Theorems Enforced: {:?}",
        status.status,
        status.grammars_supported.join(", "),
        status.speculative_default_speedup,
        status.metacognitive_pac_bayes_risk,
        status.theorems_enforced
    );

    Ok(success("cognitive status", json!(status), display))
}

fn speculate(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["draft", "target", "config", "data-dir", "workspace", "json"],
    )?;
    let subsystem = CognitiveSubsystem::new();

    let draft: Vec<f64> = if let Some(d) = parsed.value("draft") {
        d.split(',')
            .filter_map(|s| s.trim().parse::<f64>().ok())
            .collect()
    } else {
        vec![0.8, 0.7, 0.6, 0.9]
    };

    let target: Vec<f64> = if let Some(t) = parsed.value("target") {
        t.split(',')
            .filter_map(|s| s.trim().parse::<f64>().ok())
            .collect()
    } else {
        vec![0.85, 0.75, 0.65, 0.95]
    };

    let (nodes, report) = subsystem.evaluate_speculative(&draft, &target);

    let display = format!(
        "Speculative Tree Decoding Report\n\
         Nodes Evaluated: {}\n\
         Tokens Accepted: {}\n\
         Speedup Factor: {:.2}x\n\
         Distribution Match: {}",
        report.total_nodes_evaluated,
        report.tokens_accepted,
        report.speedup_factor,
        report.theorem_89_distributional_equivalence
    );

    Ok(success(
        "cognitive speculate",
        json!({
            "report": report,
            "accepted_nodes": nodes.len(),
        }),
        display,
    ))
}

fn synthesize(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["tokens", "config", "data-dir", "workspace", "json"])?;
    let mut subsystem = CognitiveSubsystem::new();

    let default_tokens = vec!["{", "\"name\"", ":", "\"pandora\"", "}"];
    let tokens_storage: Vec<String>;
    let tokens_refs: Vec<&str> = if let Some(raw) = parsed.value("tokens") {
        tokens_storage = raw.split(',').map(|s| s.trim().to_string()).collect();
        tokens_storage.iter().map(|s| s.as_str()).collect()
    } else {
        default_tokens
    };

    let report = subsystem
        .synthesize_guided(&tokens_refs)
        .map_err(|e| CliError::execution(format!("synthesis failed: {e}"), json!({"error": e})))?;

    let display = format!(
        "Neuro-Symbolic Grammar Synthesis\n\
         Grammar: {:?}\n\
         Tokens Accepted: {}\n\
         Is Valid Terminal: {}\n\
         Zero Parse Error (Theorem 68): {}",
        report.grammar,
        report.tokens_accepted,
        report.is_valid_terminal,
        report.theorem_68_zero_parse_error
    );

    Ok(success("cognitive synthesize", json!(report), display))
}

fn audit(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["logits", "drift", "config", "data-dir", "workspace", "json"],
    )?;
    let mut subsystem = CognitiveSubsystem::new();

    let logits: Vec<f64> = if let Some(l) = parsed.value("logits") {
        l.split(',')
            .filter_map(|s| s.trim().parse::<f64>().ok())
            .collect()
    } else {
        vec![2.5, 1.2, 0.1, 0.05]
    };

    let drift: f64 = parsed
        .value("drift")
        .and_then(|d| d.parse::<f64>().ok())
        .unwrap_or(0.04);

    let report = subsystem.audit_step(&logits, drift);

    let display = format!(
        "Metacognitive Step Audit\n\
         Steps Audited: {}\n\
         Mean Entropy: {:.4} nats\n\
         Peak Entropy: {:.4} nats\n\
         PAC-Bayes Risk: {:.4}\n\
         Backtracks Triggered: {}",
        report.steps_audited,
        report.mean_entropy_nats,
        report.peak_entropy_nats,
        report.theorem_82_pac_bayes_risk,
        report.backtracks_triggered
    );

    Ok(success("cognitive audit", json!(report), display))
}
