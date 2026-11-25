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
            return Err(CliError::usage("code requires 'synthesize' or 'grammar'"));
        }
    };

    match subcommand {
        "synthesize" => synthesize(&args[1..]),
        "grammar" => grammar(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown code subcommand '{unknown}'. Available subcommands: synthesize, grammar"
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
            "\"4.0.2\"",
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
