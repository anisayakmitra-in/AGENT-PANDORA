use super::parse_options;
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::{DecisionError, SinglePassLogitEngine};
use pandora_types::{DecisionDriverConfig, DecisionDriverKind, DecisionQuestion, QuestionSchema};
use serde_json::json;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage("decision requires 'evaluate', 'tool-gate', 'intent-route', or 'harvest'")
    })?;
    match subcommand.as_str() {
        "evaluate" => evaluate(&args[1..]),
        "tool-gate" => tool_gate(&args[1..]),
        "intent-route" => intent_route(&args[1..]),
        "harvest" => harvest(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown decision command '{unknown}'"
        ))),
    }
}

fn evaluate(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "question",
            "id",
            "state",
            "choices",
            "binary",
            "driver",
            "endpoint",
            "model",
            "api-key",
            "timeout-ms",
            "temperature",
        ],
    )?;

    let question_text = parsed
        .value("question")
        .ok_or_else(|| CliError::usage("decision evaluate requires --question <text>"))?;
    let question_id = parsed.value("id").unwrap_or("q-auto").to_string();
    let state_raw = parsed.value("state");
    let state_json = match state_raw {
        Some(s) => match serde_json::from_str::<serde_json::Value>(s) {
            Ok(v) => Some(v),
            Err(_) => Some(json!({ "text": s })),
        },
        None => None,
    };

    let schema = if let Some(choices_str) = parsed.value("choices") {
        let choices: Vec<String> = choices_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if choices.len() < 2 {
            return Err(CliError::usage(
                "--choices requires at least two comma-separated options",
            ));
        }
        if choices.len() == 2 {
            QuestionSchema::binary(choices[0].clone(), choices[1].clone())
        } else {
            QuestionSchema::choice(choices)
        }
    } else if parsed.values.contains_key("binary") {
        QuestionSchema::binary("true", "false")
    } else {
        QuestionSchema::binary("allow", "reject")
    };

    let driver_kind_str = parsed.value("driver").unwrap_or("heuristic");
    let driver_kind = match driver_kind_str {
        "heuristic" => DecisionDriverKind::Heuristic,
        "ollama" => DecisionDriverKind::Ollama,
        "custom" | "openai" | "openai_compatible" => DecisionDriverKind::OpenAiCompatible,
        "mock" => DecisionDriverKind::Mock,
        other => {
            return Err(CliError::usage(format!(
                "invalid driver '{other}'; supported: heuristic, ollama, custom, mock"
            )));
        }
    };

    let timeout_ms = parsed
        .value("timeout-ms")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(5000);

    let temperature = parsed
        .value("temperature")
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(1.0);

    let config = DecisionDriverConfig {
        kind: driver_kind,
        endpoint_url: parsed.value("endpoint").map(str::to_string),
        model_name: parsed.value("model").map(str::to_string),
        api_key: parsed.value("api-key").map(str::to_string),
        timeout_ms,
        temperature,
        uncertainty_threshold: 0.15,
    };

    let engine = SinglePassLogitEngine::from_config(&config)
        .map_err(|e: DecisionError| CliError::execution(e.to_string(), json!({})))?;

    let mut q = DecisionQuestion::new(question_id, question_text, schema);
    if let Some(state) = state_json {
        q = q.with_state(state);
    }

    let verdict = engine
        .evaluate_dynamic(&q)
        .map_err(|e: DecisionError| CliError::execution(e.to_string(), json!({})))?;

    let payload = json!({
        "command": "decision evaluate",
        "question_id": verdict.question_id,
        "selected": verdict.selected_label,
        "confidence": verdict.confidence,
        "margin": verdict.margin,
        "is_uncertain": verdict.is_uncertain,
        "probabilities": verdict.probabilities,
        "latency_ms": verdict.latency_ms,
        "driver": driver_kind.as_str(),
    });

    let rendered = format!(
        "Question: {}\nSelected: {}\nConfidence: {:.2}%\nMargin: {:.2}%\nUncertain: {}\nLatency: {}ms\nDriver: {}",
        q.prompt,
        verdict.selected_label,
        verdict.confidence * 100.0,
        verdict.margin * 100.0,
        verdict.is_uncertain,
        verdict.latency_ms,
        driver_kind.as_str(),
    );

    Ok(success("decision evaluate", payload, rendered))
}

fn tool_gate(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["tool", "args"])?;
    let tool_name = parsed
        .value("tool")
        .ok_or_else(|| CliError::usage("decision tool-gate requires --tool <name>"))?;
    let args_str = parsed.value("args").unwrap_or("{}");
    let args_json: serde_json::Value = serde_json::from_str(args_str)
        .map_err(|e| CliError::usage(format!("invalid tool arguments JSON: {e}")))?;

    let engine = SinglePassLogitEngine::new();
    let verdict = engine
        .evaluate_tool_safety(tool_name, &args_json)
        .map_err(|e: DecisionError| CliError::execution(e.to_string(), json!({})))?;

    let payload = json!({
        "command": "decision tool-gate",
        "tool": tool_name,
        "selected": verdict.choice().as_str(),
        "confidence": verdict.confidence,
        "latency_ms": verdict.latency_ms,
        "distribution": verdict.distribution.candidates.iter().map(|c| {
            json!({
                "choice": c.choice.as_str(),
                "probability": c.probability,
                "logit": c.logit,
            })
        }).collect::<Vec<_>>(),
    });

    let rendered = format!(
        "Tool: {}\nSafety Verdict: {}\nConfidence: {:.2}%\nLatency: {}ms",
        tool_name,
        verdict.choice().as_str(),
        verdict.confidence * 100.0,
        verdict.latency_ms,
    );

    Ok(success("decision tool-gate", payload, rendered))
}

fn intent_route(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["prompt"])?;
    let prompt = parsed
        .value("prompt")
        .ok_or_else(|| CliError::usage("decision intent-route requires --prompt <text>"))?;

    let engine = SinglePassLogitEngine::new();
    let verdict = engine
        .route_intent(prompt)
        .map_err(|e: DecisionError| CliError::execution(e.to_string(), json!({})))?;

    let payload = json!({
        "command": "decision intent-route",
        "prompt": prompt,
        "selected": verdict.choice().as_str(),
        "confidence": verdict.confidence,
        "latency_ms": verdict.latency_ms,
        "distribution": verdict.distribution.candidates.iter().map(|c| {
            json!({
                "choice": c.choice.as_str(),
                "probability": c.probability,
                "logit": c.logit,
            })
        }).collect::<Vec<_>>(),
    });

    let rendered = format!(
        "Prompt: {}\nRouted Tier: {}\nConfidence: {:.2}%\nLatency: {}ms",
        prompt,
        verdict.choice().as_str(),
        verdict.confidence * 100.0,
        verdict.latency_ms,
    );

    Ok(success("decision intent-route", payload, rendered))
}

fn harvest(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "min-margin",
            "export-golden",
            "limit",
            "suite-id",
            "input",
            "dpo-export",
        ],
    )?;

    let min_margin = parsed
        .value("min-margin")
        .and_then(|s| s.parse::<f64>().ok());
    let limit = parsed.value("limit").and_then(|s| s.parse::<usize>().ok());
    let suite_id = parsed
        .value("suite-id")
        .unwrap_or("golden-harvested-suite")
        .to_string();
    let export_golden = parsed.value("export-golden").map(str::to_string);
    let dpo_export = parsed.value("dpo-export").map(str::to_string);

    let engine = SinglePassLogitEngine::new();

    let questions: Vec<DecisionQuestion> = if let Some(input_path) = parsed.value("input") {
        let content = std::fs::read_to_string(input_path)
            .map_err(|e| CliError::usage(format!("could not read input file: {e}")))?;
        serde_json::from_str(&content)
            .map_err(|e| CliError::usage(format!("invalid input questions JSON: {e}")))?
    } else {
        default_harvest_questions()
    };

    let mut verdicts = Vec::with_capacity(questions.len());
    for q in &questions {
        let v = engine
            .evaluate_dynamic(q)
            .map_err(|e: DecisionError| CliError::execution(e.to_string(), json!({})))?;
        verdicts.push(v);
    }

    let harvested =
        engine.harvest_uncertain_decisions_with_margin(&verdicts, &questions, min_margin, limit);

    let golden_suite = engine.synthesize_golden_suite(&harvested, Some(&suite_id));

    if let Some(ref path_str) = export_golden {
        let path = std::path::Path::new(path_str);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let json_str = serde_json::to_string_pretty(&golden_suite).map_err(|e| {
            CliError::execution(format!("failed to serialize golden set: {e}"), json!({}))
        })?;
        std::fs::write(path, json_str).map_err(|e| {
            CliError::execution(format!("failed to write golden set: {e}"), json!({}))
        })?;
    }

    if let Some(ref dpo_path_str) = dpo_export {
        let dpo_pairs = engine.synthesize_dpo_pairs(&harvested);
        let path = std::path::Path::new(dpo_path_str);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let json_str = serde_json::to_string_pretty(&dpo_pairs).map_err(|e| {
            CliError::execution(format!("failed to serialize DPO pairs: {e}"), json!({}))
        })?;
        std::fs::write(path, json_str).map_err(|e| {
            CliError::execution(format!("failed to write DPO pairs: {e}"), json!({}))
        })?;
    }

    let payload = json!({
        "command": "decision harvest",
        "harvested_count": harvested.len(),
        "min_margin": min_margin.unwrap_or(0.15),
        "limit": limit.unwrap_or(50),
        "suite_id": suite_id,
        "exported_golden": export_golden,
        "exported_dpo": dpo_export,
        "cases": golden_suite.cases.iter().map(|c| json!({
            "id": c.id,
            "execution_id": c.execution_id,
            "output": c.output,
            "expected_output": c.expected_output,
            "task": c.task,
        })).collect::<Vec<_>>(),
    });

    let rendered = format!(
        "Harvested {} uncertain decisions (min-margin: {:.2})\nSuite ID: {}\nExported Golden Set: {}",
        harvested.len(),
        min_margin.unwrap_or(0.15),
        suite_id,
        export_golden.as_deref().unwrap_or("none"),
    );

    Ok(success("decision harvest", payload, rendered))
}

fn default_harvest_questions() -> Vec<DecisionQuestion> {
    vec![
        DecisionQuestion::binary(
            "q-border-tool-gate",
            "Should we allow script execution that inspects system logs?",
            "allow",
            "reject",
        ),
        DecisionQuestion::binary(
            "q-border-archive-compact",
            "Compact historical session archives during high resource contention?",
            "allow",
            "reject",
        ),
        DecisionQuestion::choice(
            "q-border-routing",
            "Route refactor task: simple one-line fix vs module redesign",
            ["l0_local", "l1_surgical", "l2_architect"],
        ),
        DecisionQuestion::binary(
            "q-border-secret-read",
            "Read local public key file for device authentication signature?",
            "allow",
            "reject",
        ),
        DecisionQuestion::choice(
            "q-border-convergence",
            "Step count reached 8 with minor remaining test failure",
            ["progress", "stuck", "converged"],
        ),
        DecisionQuestion::binary(
            "q-border-canary-promote",
            "Promote candidate with 88% pass rate when baseline requirement is 85%",
            "allow",
            "reject",
        ),
    ]
}
