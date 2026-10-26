#![forbid(unsafe_code)]

use crate::output::{CliError, CommandResult, success};
use pandora_types::{Signature, SignatureFieldKind};
use serde_json::json;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().map(String::as_str).unwrap_or("show");
    match subcommand {
        "show" | "list" => show_signatures(),
        "inspect" => {
            let name = args.get(1).map(String::as_str).unwrap_or("CodeReviewer");
            inspect_signature(name)
        }
        "cot" => {
            let name = args.get(1).map(String::as_str).unwrap_or("CodeReviewer");
            show_cot(name)
        }
        _ => Err(CliError::usage(
            "dsrs requires 'show', 'inspect <name>', or 'cot <name>'",
        )),
    }
}

fn sample_signatures() -> Vec<Signature> {
    vec![
        Signature::new(
            "CodeReviewer",
            "You are an expert Rust systems programmer. Review the code for safety, correctness, and performance.",
        )
        .expect("valid")
        .with_input("code", "Rust source code to review")
        .expect("valid")
        .with_input("context", "Execution environment and constraints")
        .expect("valid")
        .with_output("verdict", "Approve or Reject")
        .expect("valid")
        .with_output("feedback", "Specific constructive feedback with diff recommendations")
        .expect("valid"),

        Signature::new(
            "ReActModule",
            "Interleave reasoning thought steps with concrete tool invocations.",
        )
        .expect("valid")
        .with_input("task", "Goal to achieve")
        .expect("valid")
        .with_input("observations", "Prior tool execution results")
        .expect("valid")
        .with_output("thought", "Reasoning step planning the next action")
        .expect("valid")
        .with_output("action", "Exact tool and arguments to execute")
        .expect("valid"),

        Signature::new(
            "SqlSynthesizer",
            "Synthesize safe, optimized BigQuery SQL for analytical tasks.",
        )
        .expect("valid")
        .with_input("question", "Analytical question")
        .expect("valid")
        .with_input("schema", "Database schema tables and column descriptions")
        .expect("valid")
        .with_output("sql", "Executable SQL query")
        .expect("valid"),
    ]
}

fn show_signatures() -> Result<CommandResult, CliError> {
    let sigs = sample_signatures();
    let mut data_list = Vec::new();
    let mut human = String::from("Declarative Structured Reasoning (DSRs) Signatures:\n\n");

    for sig in &sigs {
        let input_names: Vec<&str> = sig.inputs().map(|f| f.name.as_str()).collect();
        let output_names: Vec<&str> = sig.outputs().map(|f| f.name.as_str()).collect();
        human.push_str(&format!(
            "- {} ({} -> {})\n  Instructions: {}\n\n",
            sig.name,
            input_names.join(", "),
            output_names.join(", "),
            sig.instructions
        ));

        data_list.push(json!({
            "name": sig.name,
            "instructions": sig.instructions,
            "inputs": input_names,
            "outputs": output_names,
        }));
    }

    Ok(success(
        "dsrs show",
        json!({
            "count": sigs.len(),
            "signatures": data_list,
        }),
        human.trim_end(),
    ))
}

fn inspect_signature(name: &str) -> Result<CommandResult, CliError> {
    let sigs = sample_signatures();
    let sig = sigs
        .into_iter()
        .find(|s| s.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| CliError::usage(format!("unknown DSRs signature '{name}'")))?;

    let mut human = format!("DSRs Signature: {}\n", sig.name);
    human.push_str(&format!("Instructions: {}\n\nFields:\n", sig.instructions));

    let mut fields_json = Vec::new();
    for f in &sig.fields {
        let kind_str = match f.kind {
            SignatureFieldKind::Input => "Input",
            SignatureFieldKind::Output => "Output",
        };
        human.push_str(&format!("  [{kind_str}] {}: {}\n", f.prefix, f.description));
        fields_json.push(json!({
            "name": f.name,
            "kind": kind_str,
            "prefix": f.prefix,
            "description": f.description,
        }));
    }

    Ok(success(
        "dsrs inspect",
        json!({
            "signature": sig.name,
            "instructions": sig.instructions,
            "fields": fields_json,
        }),
        human.trim_end(),
    ))
}

fn show_cot(name: &str) -> Result<CommandResult, CliError> {
    let sigs = sample_signatures();
    let sig = sigs
        .into_iter()
        .find(|s| s.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| CliError::usage(format!("unknown DSRs signature '{name}'")))?;

    let cot_sig = sig.into_chain_of_thought();
    let mut human = format!(
        "Chain-of-Thought (CoT) DSRs Signature: {}\n\nFields:\n",
        cot_sig.name
    );
    for f in &cot_sig.fields {
        let kind_str = match f.kind {
            SignatureFieldKind::Input => "Input",
            SignatureFieldKind::Output => "Output",
        };
        human.push_str(&format!("  [{kind_str}] {}: {}\n", f.prefix, f.description));
    }

    Ok(success(
        "dsrs cot",
        json!({
            "signature": cot_sig.name,
            "has_rationale": cot_sig.fields.iter().any(|f| f.name == "rationale"),
        }),
        human.trim_end(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dsrs_cli_show_and_inspect() {
        let show_res = execute(&["show".to_owned()]).expect("show ok");
        assert!(show_res.human.contains("CodeReviewer"));
        assert_eq!(show_res.data["count"], 3);

        let inspect_res =
            execute(&["inspect".to_owned(), "SqlSynthesizer".to_owned()]).expect("inspect ok");
        assert!(inspect_res.human.contains("Executable SQL query"));
        assert_eq!(inspect_res.data["signature"], "SqlSynthesizer");

        let cot_res = execute(&["cot".to_owned(), "CodeReviewer".to_owned()]).expect("cot ok");
        assert!(cot_res.human.contains("Rationale:"));
        assert_eq!(cot_res.data["has_rationale"], true);
    }
}
