#![forbid(unsafe_code)]

use crate::output::{CliError, CommandResult, success};
use pandora_runtime::gepa::{
    GepaCandidate, GepaFeedbackTrace, GepaObjective, GepaOptimizer, GepaParetoFrontier,
};
use pandora_types::Signature;
use serde_json::json;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().map(String::as_str).unwrap_or("frontier");
    match subcommand {
        "frontier" | "list" => show_frontier(),
        "optimize" | "step" => run_optimization_step(),
        _ => Err(CliError::usage("gepa requires 'frontier' or 'optimize'")),
    }
}

fn sample_frontier() -> GepaParetoFrontier<Signature> {
    let mut frontier = GepaParetoFrontier::new();

    let sig_base = Signature::new(
        "CodingAgent",
        "Generate idiomatic Rust code conforming to workspace standards.",
    )
    .expect("valid")
    .with_input("task", "Coding specification")
    .expect("valid")
    .with_output("code", "Rust implementation")
    .expect("valid");

    let sig_fast = Signature::new(
        "CodingAgentFast",
        "Generate minimal, direct Rust code. Avoid extraneous abstractions.",
    )
    .expect("valid")
    .with_input("task", "Coding specification")
    .expect("valid")
    .with_output("code", "Rust implementation")
    .expect("valid");

    let sig_thorough = Signature::new(
        "CodingAgentThorough",
        "Generate comprehensive, exhaustively verified Rust code with property tests and error handling.",
    )
    .expect("valid")
    .with_input("task", "Coding specification")
    .expect("valid")
    .with_output("code", "Rust implementation")
    .expect("valid");

    // Frontier candidate 1: balanced baseline
    frontier.try_insert(GepaCandidate {
        id: "gepa-cand-001".to_owned(),
        candidate: sig_base,
        objective: GepaObjective::new(0.88, 1200, 350).expect("valid"),
        reflection: Some("Baseline prompt configuration".to_owned()),
        parent_ids: Vec::new(),
        generation: 0,
    });

    // Frontier candidate 2: low latency & low cost trade-off (pass rate 0.82, tokens 650, latency 180)
    frontier.try_insert(GepaCandidate {
        id: "gepa-cand-002".to_owned(),
        candidate: sig_fast,
        objective: GepaObjective::new(0.82, 650, 180).expect("valid"),
        reflection: Some("Optimized for latency and token cost".to_owned()),
        parent_ids: vec!["gepa-cand-001".to_owned()],
        generation: 1,
    });

    // Frontier candidate 3: high accuracy trade-off (pass rate 0.98, tokens 2100, latency 620)
    frontier.try_insert(GepaCandidate {
        id: "gepa-cand-003".to_owned(),
        candidate: sig_thorough,
        objective: GepaObjective::new(0.98, 2100, 620).expect("valid"),
        reflection: Some("Reflective prompt with invariant self-verification".to_owned()),
        parent_ids: vec!["gepa-cand-001".to_owned()],
        generation: 2,
    });

    frontier
}

fn show_frontier() -> Result<CommandResult, CliError> {
    let frontier = sample_frontier();
    let mut human = format!(
        "Genetic-Pareto (GEPA) Non-Dominated Frontier ({} candidates):\n\n",
        frontier.len()
    );
    let mut items = Vec::new();

    for c in &frontier.candidates {
        human.push_str(&format!(
            "- [{}] {} (Pass: {:.1}%, Cost: {} toks, Latency: {}ms, Gen: {})\n  Reflection: {}\n\n",
            c.id,
            c.candidate.name,
            c.objective.pass_rate * 100.0,
            c.objective.token_cost,
            c.objective.latency_ms,
            c.generation,
            c.reflection.as_deref().unwrap_or("none")
        ));

        items.push(json!({
            "id": c.id,
            "signature": c.candidate.name,
            "pass_rate": c.objective.pass_rate,
            "token_cost": c.objective.token_cost,
            "latency_ms": c.objective.latency_ms,
            "generation": c.generation,
            "reflection": c.reflection,
        }));
    }

    Ok(success(
        "gepa frontier",
        json!({
            "count": frontier.len(),
            "frontier": items,
        }),
        human.trim_end(),
    ))
}

fn run_optimization_step() -> Result<CommandResult, CliError> {
    let baseline = Signature::new(
        "ReflectiveCoder",
        "You are an AI coding assistant. Write correct code.",
    )
    .expect("valid")
    .with_input("spec", "Task specification")
    .expect("valid")
    .with_output("code", "Source code")
    .expect("valid");

    let baseline_obj = GepaObjective::new(0.80, 1000, 400).expect("valid");
    let mut optimizer = GepaOptimizer::new(baseline, baseline_obj);

    let traces = vec![
        GepaFeedbackTrace {
            task_id: "task-01".to_owned(),
            input_summary: "File writing routine".to_owned(),
            failed_output: "write_file(path, data)".to_owned(),
            diagnostic: "Missing parent directory creation before write".to_owned(),
            root_cause: "Assumed parent folder already exists".to_owned(),
        },
        GepaFeedbackTrace {
            task_id: "task-02".to_owned(),
            input_summary: "Subagent spawning".to_owned(),
            failed_output: "spawn_subagent(unbounded)".to_owned(),
            diagnostic: "Exceeded max recursion depth of 8".to_owned(),
            root_cause: "Missing explicit depth termination check".to_owned(),
        },
    ];

    let result = optimizer.step_reflective(&traces, |_candidate| {
        // Simulated probe outcome showing pass rate boost from rich reflection
        GepaObjective::new(0.95, 1150, 420)
    });

    match result {
        Ok(Some(admitted)) => {
            let human = format!(
                "GEPA Optimization Step Succeeded!\n\nNew Pareto Candidate: {}\nPass Rate: {:.1}%\nTokens: {}\nLatency: {}ms\nGeneration: {}\n\nEvolved Instructions:\n{}",
                admitted.id,
                admitted.objective.pass_rate * 100.0,
                admitted.objective.token_cost,
                admitted.objective.latency_ms,
                admitted.generation,
                admitted.candidate.instructions
            );

            Ok(success(
                "gepa optimize",
                json!({
                    "admitted_id": admitted.id,
                    "pass_rate": admitted.objective.pass_rate,
                    "token_cost": admitted.objective.token_cost,
                    "latency_ms": admitted.objective.latency_ms,
                    "generation": admitted.generation,
                    "instructions": admitted.candidate.instructions,
                }),
                human,
            ))
        }
        Ok(None) => Ok(success(
            "gepa optimize",
            json!({"admitted": false}),
            "GEPA Optimization Step: Candidate was dominated and rejected by frontier.",
        )),
        Err(e) => Err(CliError::execution(e.to_string(), json!({}))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gepa_cli_frontier_and_optimize() {
        let frontier_res = execute(&["frontier".to_owned()]).expect("frontier ok");
        assert!(frontier_res.human.contains("gepa-cand-001"));
        assert_eq!(frontier_res.data["count"], 3);

        let opt_res = execute(&["optimize".to_owned()]).expect("optimize ok");
        assert!(opt_res.human.contains("GEPA Optimization Step Succeeded!"));
        assert_eq!(opt_res.data["generation"], 1);
        assert_eq!(opt_res.data["pass_rate"], 0.95);
    }
}
