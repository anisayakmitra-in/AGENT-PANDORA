use super::parse_options;
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::{DecisionError, SinglePassLogitEngine};
use pandora_types::{DecisionDriverConfig, DecisionDriverKind, DecisionQuestion, QuestionSchema};
use serde_json::json;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage("decision requires 'evaluate', 'tool-gate', or 'intent-route'")
    })?;
    match subcommand.as_str() {
        "evaluate" => evaluate(&args[1..]),
        "tool-gate" => tool_gate(&args[1..]),
        "intent-route" => intent_route(&args[1..]),
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
