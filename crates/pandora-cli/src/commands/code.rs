#![forbid(unsafe_code)]

//! Phase 31: Grammar-Constrained Token-Level BNF Code Synthesis CLI
//!
//! Subcommands for grammar-masked neural-symbolic synthesis, vocabulary admissibility
//! masking, and pushdown automata invariant verification (Theorems 68–70).

use super::{load_config, parse_options};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::grammar_mask::{GrammarKind, GrammarMaskEngine};
use serde_json::json;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let first = args.first().map(String::as_str);
    let subcommand = match first {
        Some(s) if !s.starts_with('-') => s,
        _ => {
            return Err(CliError::usage(
                "code requires 'synthesize', 'grammar', or 'speculate'",
            ));
        }
    };

    match subcommand {
        "synthesize" => synthesize(&args[1..]),
        "grammar" => grammar(&args[1..]),
        "speculate" => speculate(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown code subcommand '{unknown}'. Available subcommands: synthesize, grammar, speculate"
        ))),
    }
}

fn parse_grammar_kind(name: &str) -> Result<GrammarKind, CliError> {
    match name.to_lowercase().as_str() {
        "json" => Ok(GrammarKind::Json),
        "sql" => Ok(GrammarKind::SqlSubset),
        "rust" => Ok(GrammarKind::RustSubset),
        "python" => Ok(GrammarKind::PythonSubset),
        other => Err(CliError::usage(format!(
            "unsupported grammar '{other}'. Supported grammars: json, sql, rust, python"
        ))),
    }
}

fn parse_grammar_opt(parsed: &crate::commands::ParsedArgs) -> Result<GrammarKind, CliError> {
    let grammar_str = parsed
        .value("kind")
        .or_else(|| {
            parsed.value("grammar").and_then(|v| {
                if v.is_empty() {
                    parsed.positionals.first().map(String::as_str)
                } else {
                    Some(v)
                }
            })
        })
        .or_else(|| parsed.positionals.first().map(String::as_str))
        .unwrap_or("json");

    parse_grammar_kind(grammar_str)
}

fn synthesize(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "grammar",
            "kind",
            "input",
        ],
    )?;
    let _cfg = load_config(&parsed)?;

    let kind = parse_grammar_opt(&parsed)?;

    let mut engine = GrammarMaskEngine::new(kind);

    let sample_stream = match kind {
        GrammarKind::Json => vec![
            "{",
            "\"name\"",
            ":",
            "\"pandora\"",
            ",",
            "\"version\"",
            ":",
            "\"4.0.3\"",
            "}",
        ],
        GrammarKind::SqlSubset => vec!["SELECT", "id", ",", "name", "FROM", "agents", ";"],
        GrammarKind::RustSubset => vec!["let", "x", "=", "42", ";"],
        GrammarKind::PythonSubset => vec!["return", "42"],
    };

    let report = engine
        .synthesize_guided(&sample_stream)
        .map_err(|e| CliError::execution(format!("synthesis failed: {e}"), json!({})))?;

    let text = format!(
        "Grammar-Constrained Synthesis [{}]\n\
         Synthesized Output:\n  {}\n\
         Tokens Accepted: {}\n\
         Is Terminal: {}\n\
         Theorem 68 (Pr[ParseError] = 0): {}\n\
         Theorem 69 (Context-Free Invariance): {}\n\
         Theorem 70 (Roofline Compliant): {}\n",
        kind.as_str(),
        report.synthesized_output,
        report.tokens_accepted,
        report.is_valid_terminal,
        report.theorem_68_zero_parse_error,
        report.theorem_69_invariance_preserved,
        report.theorem_70_roofline_compliant
    );

    Ok(success(
        "code synthesize",
        json!({
            "grammar": kind.as_str(),
            "report": report
        }),
        text,
    ))
}

fn grammar(args: &[String]) -> Result<CommandResult, CliError> {
    let sub = args.first().map(String::as_str);
    let (action, rest) = match sub {
        Some(a) if !a.starts_with('-') => (a, &args[1..]),
        _ => ("inspect", args),
    };

    let parsed = parse_options(
        rest,
        &[
            "config",
            "data-dir",
            "workspace",
            "grammar",
            "kind",
            "token",
        ],
    )?;
    let kind = parse_grammar_opt(&parsed)?;

    match action {
        "inspect" | "mask" => {
            let engine = GrammarMaskEngine::new(kind);
            let sample_vocab = match kind {
                GrammarKind::Json => vec!["{", "}", ":", ",", "\"key\"", "123", "let", "SELECT"],
                GrammarKind::SqlSubset => vec!["SELECT", "INSERT", "FROM", "*", "where", ";", "{"],
                GrammarKind::RustSubset => vec!["let", "fn", "return", "42", ";", "SELECT"],
                GrammarKind::PythonSubset => vec!["def", "return", "pass", "42", ";", "{"],
            };

            let mask = engine.compute_mask(&sample_vocab);

            let text = format!(
                "Grammar Token Mask [{}]\n\
                 Total Vocab Evaluated: {}\n\
                 Admissible Tokens Count: {}\n\
                 Admissible Tokens: {:?}\n\
                 Stack Depth: {}\n\
                 Elapsed Microseconds: {} us\n\
                 Theorem 68 Soundness: {}\n",
                kind.as_str(),
                mask.total_vocab_size,
                mask.admissible_tokens_count,
                mask.admissible_tokens,
                mask.stack_depth,
                mask.elapsed_micros,
                mask.theorem_68_soundness
            );

            Ok(success(
                "code grammar mask",
                json!({
                    "grammar": kind.as_str(),
                    "mask": mask
                }),
                text,
            ))
        }
        "validate" => {
            let token = parsed.value("token").unwrap_or("{");
            let engine = GrammarMaskEngine::new(kind);
            let admissible = engine.is_token_admissible(token);

            let text = format!(
                "Token Admissibility Check\n\
                 Grammar: {}\n\
                 Token: '{}'\n\
                 Admissible: {}\n",
                kind.as_str(),
                token,
                admissible
            );

            Ok(success(
                "code grammar validate",
                json!({
                    "grammar": kind.as_str(),
                    "token": token,
                    "admissible": admissible
                }),
                text,
            ))
        }
        unknown => Err(CliError::usage(format!(
            "unknown grammar action '{unknown}'. Available: inspect, mask, validate"
        ))),
    }
}

fn speculate(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "depth",
            "branching",
            "prompt",
        ],
    )?;
    let _cfg = load_config(&parsed)?;

    let depth: usize = parsed
        .value("depth")
        .and_then(|s| s.parse().ok())
        .unwrap_or(4);
    let branching: usize = parsed
        .value("branching")
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);
    let prompt = parsed.value("prompt").unwrap_or("synthesize_kernel");

    let engine = pandora_runtime::speculative_tree::SpeculativeTreeEngine::new(depth, branching);
    let draft = vec![0.85, 0.72, 0.68, 0.91, 0.84, 0.76];
    let target = vec![0.88, 0.75, 0.62, 0.93, 0.87, 0.74];

    let (nodes, report) = engine.evaluate_tree(&draft, &target);

    let text = format!(
        "Distributed Speculative Tree Decoding:\n\
         Prompt: {prompt}\n\
         Tree Depth: {depth}\n\
         Branching Factor: {branching}\n\
         Draft Nodes Evaluated: {}\n\
         Tokens Accepted: {}\n\
         Speedup Factor: {:.2}x\n\
         Theorem 89 (Distributional Equivalence): {}\n\
         Theorem 90 (Speedup Bound >= 2.0x): {}\n\
         Theorem 91 (Latency Amortized <= 15%): {}\n",
        report.total_nodes_evaluated,
        report.tokens_accepted,
        report.speedup_factor,
        report.theorem_89_distributional_equivalence,
        report.theorem_90_speedup_bound_met,
        report.theorem_91_latency_amortized
    );

    Ok(success(
        "code speculate",
        json!({
            "prompt": prompt,
            "depth": depth,
            "branching": branching,
            "nodes_count": nodes.len(),
            "report": report,
            "theorems": [89, 90, 91]
        }),
        text,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_code_cli_synthesize_json_and_sql() {
        let json_res = execute(&[
            "synthesize".to_string(),
            "--grammar".to_string(),
            "json".to_string(),
        ])
        .expect("json synthesis succeeds");
        assert_eq!(json_res.command, "code synthesize");
        assert_eq!(json_res.data["grammar"], "json");
        assert!(
            json_res.data["report"]["is_valid_terminal"]
                .as_bool()
                .unwrap()
        );
        assert!(
            json_res.data["report"]["theorem_68_zero_parse_error"]
                .as_bool()
                .unwrap()
        );

        let sql_res = execute(&[
            "synthesize".to_string(),
            "--grammar".to_string(),
            "sql".to_string(),
        ])
        .expect("sql synthesis succeeds");
        assert_eq!(sql_res.command, "code synthesize");
        assert_eq!(sql_res.data["grammar"], "sql");
        assert!(
            sql_res.data["report"]["is_valid_terminal"]
                .as_bool()
                .unwrap()
        );
    }

    #[test]
    fn test_code_cli_grammar_mask_and_validate() {
        let mask_res = execute(&[
            "grammar".to_string(),
            "mask".to_string(),
            "--grammar".to_string(),
            "json".to_string(),
        ])
        .expect("mask succeeds");
        assert_eq!(mask_res.command, "code grammar mask");
        assert_eq!(mask_res.data["grammar"], "json");

        let val_res = execute(&[
            "grammar".to_string(),
            "validate".to_string(),
            "--grammar".to_string(),
            "json".to_string(),
            "--token".to_string(),
            "{".to_string(),
        ])
        .expect("validate succeeds");
        assert_eq!(val_res.command, "code grammar validate");
        assert!(val_res.data["admissible"].as_bool().unwrap());
    }
}
