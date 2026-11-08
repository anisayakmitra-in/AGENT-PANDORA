use super::{load_config, parse_options};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::{
    BanditArm, BanditModelRouter, BanditStrategy, RoutingContext, RoutingReward, SafePrng,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredModel {
    pub name: String,
    pub source: String,
    pub location: String,
    pub format: String,
    pub size_bytes: Option<u64>,
    pub size_human: String,
    pub quantization: Option<String>,
    pub architecture: Option<String>,
    pub details: Option<String>,
}

pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

pub fn extract_quantization_hint(filename: &str) -> Option<String> {
    let lower = filename.to_ascii_lowercase();
    let quants = [
        "q4_k_m", "q4_k_s", "q5_k_m", "q5_k_s", "q8_0", "q4_0", "q4_1", "q5_0", "q5_1", "q6_k",
        "q2_k", "q3_k_m", "q3_k_s", "q3_k_l", "f16", "f32", "bf16", "int4", "int8", "fp16", "fp8",
    ];
    for q in quants {
        if lower.contains(q) {
            return Some(q.to_ascii_uppercase());
        }
    }
    None
}

pub fn extract_architecture_hint(filename: &str) -> Option<String> {
    let lower = filename.to_ascii_lowercase();
    let archs = [
        ("qwen2.5-coder", "Qwen 2.5 Coder"),
        ("qwen2.5", "Qwen 2.5"),
        ("qwen2", "Qwen 2"),
        ("deepseek-coder", "DeepSeek Coder"),
        ("deepseek-r1", "DeepSeek R1"),
        ("deepseek", "DeepSeek"),
        ("llama-3.3", "Llama 3.3"),
        ("llama-3.1", "Llama 3.1"),
        ("llama-3", "Llama 3"),
        ("llama", "Llama"),
        ("mistral", "Mistral"),
        ("mixtral", "Mixtral"),
        ("hermes-3", "Nous Hermes 3"),
        ("hermes", "Nous Hermes"),
        ("phi-4", "Phi 4"),
        ("phi-3", "Phi 3"),
        ("gemma-2", "Gemma 2"),
        ("gemma", "Gemma"),
        ("starcoder", "StarCoder"),
    ];
    for (prefix, label) in archs {
        if lower.contains(prefix) {
            return Some((*label).to_owned());
        }
    }
    None
}

pub fn scan_workspace_models(workspace: &Path) -> Vec<DiscoveredModel> {
    let mut discovered = Vec::new();
    if !workspace.is_dir() {
        return discovered;
    }

    fn visit_dir(current: &Path, root: &Path, depth: usize, results: &mut Vec<DiscoveredModel>) {
        if depth > 4 {
            return;
        }
        let entries = match fs::read_dir(current) {
            Ok(entries) => entries,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = entry.file_name();
            let name_str = file_name.to_string_lossy();

            if name_str.starts_with('.') && name_str != ".models" {
                continue;
            }
            if name_str == "target"
                || name_str == "node_modules"
                || name_str == "dist"
                || name_str == "build"
                || name_str == "tmp"
            {
                continue;
            }

            if path.is_dir() {
                // Check if this directory is a Hugging Face model directory with config.json
                let config_file = path.join("config.json");
                if config_file.is_file()
                    && let Ok(content) = fs::read_to_string(&config_file)
                    && let Ok(val) = serde_json::from_str::<serde_json::Value>(&content)
                    && let Some(model_type) = val.get("model_type").and_then(|v| v.as_str())
                {
                    let rel = path.strip_prefix(root).unwrap_or(&path);
                    results.push(DiscoveredModel {
                        name: name_str.to_string(),
                        source: "workspace (hf directory)".to_owned(),
                        location: rel.display().to_string(),
                        format: "HuggingFace".to_owned(),
                        size_bytes: None,
                        size_human: "directory".to_owned(),
                        quantization: None,
                        architecture: Some(model_type.to_owned()),
                        details: Some(format!("Model type: {model_type}")),
                    });
                    continue;
                }
                visit_dir(&path, root, depth + 1, results);
            } else if path.is_file() {
                let lower = name_str.to_ascii_lowercase();
                let meta = match path.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                let size_bytes = meta.len();
                let size_human = format_bytes(size_bytes);
                let rel = path.strip_prefix(root).unwrap_or(&path);

                if lower.ends_with(".gguf") {
                    let is_gguf_magic = fs::File::open(&path)
                        .and_then(|mut f| {
                            use std::io::Read;
                            let mut buf = [0u8; 4];
                            f.read_exact(&mut buf)?;
                            Ok(buf == *b"GGUF")
                        })
                        .unwrap_or(false);

                    let quant = extract_quantization_hint(&name_str);
                    let arch = extract_architecture_hint(&name_str);
                    results.push(DiscoveredModel {
                        name: name_str.to_string(),
                        source: "workspace (gguf)".to_owned(),
                        location: rel.display().to_string(),
                        format: if is_gguf_magic {
                            "GGUF"
                        } else {
                            "GGUF (unverified)"
                        }
                        .to_owned(),
                        size_bytes: Some(size_bytes),
                        size_human,
                        quantization: quant,
                        architecture: arch,
                        details: Some(format!("GGUF file at {}", rel.display())),
                    });
                } else if lower.ends_with(".safetensors") {
                    let arch = extract_architecture_hint(&name_str);
                    results.push(DiscoveredModel {
                        name: name_str.to_string(),
                        source: "workspace (safetensors)".to_owned(),
                        location: rel.display().to_string(),
                        format: "SafeTensors".to_owned(),
                        size_bytes: Some(size_bytes),
                        size_human,
                        quantization: extract_quantization_hint(&name_str),
                        architecture: arch,
                        details: Some(format!("SafeTensors weights at {}", rel.display())),
                    });
                } else if lower.ends_with(".onnx") {
                    results.push(DiscoveredModel {
                        name: name_str.to_string(),
                        source: "workspace (onnx)".to_owned(),
                        location: rel.display().to_string(),
                        format: "ONNX".to_owned(),
                        size_bytes: Some(size_bytes),
                        size_human,
                        quantization: None,
                        architecture: extract_architecture_hint(&name_str),
                        details: Some(format!("ONNX runtime model at {}", rel.display())),
                    });
                } else if lower == "modelfile" || lower.ends_with(".modelfile") {
                    let base_model = fs::read_to_string(&path).ok().and_then(|c| {
                        for line in c.lines() {
                            let trimmed = line.trim();
                            if trimmed.to_ascii_uppercase().starts_with("FROM ") {
                                return Some(trimmed[5..].trim().to_owned());
                            }
                        }
                        None
                    });
                    results.push(DiscoveredModel {
                        name: name_str.to_string(),
                        source: "workspace (modelfile)".to_owned(),
                        location: rel.display().to_string(),
                        format: "Modelfile".to_owned(),
                        size_bytes: Some(size_bytes),
                        size_human,
                        quantization: None,
                        architecture: None,
                        details: base_model.map(|b| format!("FROM {b}")),
                    });
                }
            }
        }
    }

    visit_dir(workspace, workspace, 0, &mut discovered);
    discovered
}

pub fn scan_ollama_models() -> Vec<DiscoveredModel> {
    let mut discovered = Vec::new();

    // 1. Try local Ollama daemon API with a short timeout
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(800))
        .build();

    if let Ok(client) = client
        && let Ok(res) = client.get("http://localhost:11434/api/tags").send()
        && res.status().is_success()
        && let Ok(body) = res.json::<serde_json::Value>()
        && let Some(models) = body.get("models").and_then(|v| v.as_array())
    {
        for m in models {
            let name = m.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
            let size = m.get("size").and_then(|v| v.as_u64());
            let details = m.get("details");
            let format = details
                .and_then(|d| d.get("format"))
                .and_then(|v| v.as_str())
                .unwrap_or("gguf");
            let family = details
                .and_then(|d| d.get("family"))
                .and_then(|v| v.as_str());
            let param_size = details
                .and_then(|d| d.get("parameter_size"))
                .and_then(|v| v.as_str());
            let quant = details
                .and_then(|d| d.get("quantization_level"))
                .and_then(|v| v.as_str())
                .map(str::to_owned);

            let mut desc = Vec::new();
            if let Some(p) = param_size {
                desc.push(format!("Params: {p}"));
            }
            if let Some(f) = family {
                desc.push(format!("Family: {f}"));
            }

            discovered.push(DiscoveredModel {
                name: name.to_owned(),
                source: "ollama (running)".to_owned(),
                location: "http://localhost:11434".to_owned(),
                format: format.to_ascii_uppercase(),
                size_bytes: size,
                size_human: size
                    .map(format_bytes)
                    .unwrap_or_else(|| "unknown".to_owned()),
                quantization: quant,
                architecture: family.map(str::to_owned),
                details: if desc.is_empty() {
                    None
                } else {
                    Some(desc.join(" · "))
                },
            });
        }
    }

    // 2. If daemon wasn't reachable or had no models, check local storage manifests on disk
    if discovered.is_empty() {
        let manifest_dirs = [
            // Windows: %USERPROFILE%\.ollama\models\manifests
            std::env::var("USERPROFILE").map(|p| {
                PathBuf::from(p)
                    .join(".ollama")
                    .join("models")
                    .join("manifests")
            }),
            // Unix: ~/.ollama/models/manifests
            std::env::var("HOME").map(|p| {
                PathBuf::from(p)
                    .join(".ollama")
                    .join("models")
                    .join("manifests")
            }),
        ];

        for dir_opt in manifest_dirs {
            let Some(dir) = dir_opt.ok() else { continue };
            if !dir.is_dir() {
                continue;
            }
            // Ollama manifest tree: manifests/registry.ollama.ai/library/<name>/<tag>
            let library_dir = dir.join("registry.ollama.ai").join("library");
            if let Ok(entries) = fs::read_dir(library_dir) {
                for model_entry in entries.flatten() {
                    if model_entry.path().is_dir() {
                        let model_name = model_entry.file_name().to_string_lossy().to_string();
                        if let Ok(tags) = fs::read_dir(model_entry.path()) {
                            for tag_entry in tags.flatten() {
                                if tag_entry.path().is_file() {
                                    let tag_name =
                                        tag_entry.file_name().to_string_lossy().to_string();
                                    let full_tag = format!("{model_name}:{tag_name}");
                                    discovered.push(DiscoveredModel {
                                        name: full_tag,
                                        source: "ollama (local storage)".to_owned(),
                                        location: tag_entry.path().display().to_string(),
                                        format: "OllamaManifest".to_owned(),
                                        size_bytes: None,
                                        size_human: "local".to_owned(),
                                        quantization: None,
                                        architecture: extract_architecture_hint(&model_name),
                                        details: Some("Installed in ~/.ollama storage".to_owned()),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    discovered
}

pub fn scan_lmstudio_models() -> Vec<DiscoveredModel> {
    let mut discovered = Vec::new();
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(800))
        .build();

    if let Ok(client) = client
        && let Ok(res) = client.get("http://localhost:1234/v1/models").send()
        && res.status().is_success()
        && let Ok(body) = res.json::<serde_json::Value>()
        && let Some(data) = body.get("data").and_then(|v| v.as_array())
    {
        for m in data {
            if let Some(id) = m.get("id").and_then(|v| v.as_str()) {
                discovered.push(DiscoveredModel {
                    name: id.to_owned(),
                    source: "lmstudio (running)".to_owned(),
                    location: "http://localhost:1234".to_owned(),
                    format: "GGUF".to_owned(),
                    size_bytes: None,
                    size_human: "loaded".to_owned(),
                    quantization: extract_quantization_hint(id),
                    architecture: extract_architecture_hint(id),
                    details: Some("Loaded in LM Studio local server".to_owned()),
                });
            }
        }
    }
    discovered
}

pub fn scan_huggingface_models() -> Vec<DiscoveredModel> {
    let mut discovered = Vec::new();
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok();
    let Some(home_path) = home else {
        return discovered;
    };
    let hub_dir = PathBuf::from(home_path)
        .join(".cache")
        .join("huggingface")
        .join("hub");
    if !hub_dir.is_dir() {
        return discovered;
    }

    if let Ok(entries) = fs::read_dir(hub_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("models--") {
                let clean_name = name.trim_start_matches("models--").replace("--", "/");
                let arch = extract_architecture_hint(&clean_name);
                discovered.push(DiscoveredModel {
                    name: clean_name,
                    source: "huggingface (local cache)".to_owned(),
                    location: entry.path().display().to_string(),
                    format: "HuggingFaceHub".to_owned(),
                    size_bytes: None,
                    size_human: "cached".to_owned(),
                    quantization: extract_quantization_hint(&name),
                    architecture: arch,
                    details: Some(format!("HF snapshot in {}", entry.path().display())),
                });
            }
        }
    }

    discovered
}

pub fn inspect_all_models(workspace: &Path) -> Vec<DiscoveredModel> {
    let mut all = Vec::new();
    all.extend(scan_workspace_models(workspace));
    all.extend(scan_ollama_models());
    all.extend(scan_lmstudio_models());
    all.extend(scan_huggingface_models());
    all
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArmReport {
    pub id: String,
    pub model_id: String,
    pub provider_id: String,
    pub pull_count: u64,
    pub total_reward: f64,
    pub mean_reward: f64,
    pub alpha: f64,
    pub beta: f64,
    pub feature_weights: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegretReport {
    pub optimal_reward_mean: f64,
    pub cumulative_empirical_regret: f64,
    pub theoretical_minimax_bound: f64,
    pub bound_satisfied: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextReport {
    pub query_length: usize,
    pub task_complexity: f64,
    pub cache_affinity: f64,
    pub max_latency_ms: f64,
    pub features: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyParamsReport {
    pub c: Option<f64>,
    pub epsilon: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteReport {
    pub strategy: String,
    pub strategy_params: StrategyParamsReport,
    pub seed: u64,
    pub rounds: u64,
    pub arm_count: usize,
    pub selected_arm: String,
    pub selected_model: String,
    pub selected_provider: String,
    pub context: ContextReport,
    pub arms: Vec<ArmReport>,
    pub regret: RegretReport,
}

const ALLOWED_ROUTE_OPTIONS: &[&str] = &[
    "strategy",
    "c",
    "epsilon",
    "prompt",
    "query-length",
    "len",
    "complexity",
    "cache-affinity",
    "max-latency-ms",
    "arms",
    "rounds",
    "pulls",
    "simulate",
    "seed",
    "optimal-reward",
    "config",
    "data-dir",
    "workspace",
    "json",
];

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let first_non_flag = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .map(String::as_str);
    match first_non_flag {
        Some("route") => {
            let mut sub_args = Vec::with_capacity(args.len().saturating_sub(1));
            let mut removed = false;
            for arg in args {
                if !removed && arg == "route" {
                    removed = true;
                    continue;
                }
                sub_args.push(arg.clone());
            }
            route(&sub_args)
        }
        Some("list") | Some("inspect") | Some("scan") => {
            let mut sub_args = Vec::with_capacity(args.len().saturating_sub(1));
            let mut removed = false;
            for arg in args {
                if !removed && (arg == "list" || arg == "inspect" || arg == "scan") {
                    removed = true;
                    continue;
                }
                sub_args.push(arg.clone());
            }
            inspect_models(&sub_args)
        }
        _ => inspect_models(args),
    }
}

pub fn inspect_models(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "json"])?;
    let config = load_config(&parsed)?;
    let workspace = config.workspace_dir();

    let models = inspect_all_models(workspace);
    let count = models.len();

    let message = if models.is_empty() {
        format!(
            "No pre-installed models found in workspace ({}) or local engines (Ollama/LM Studio).\n\
             Tip: Pull a model via `ollama pull qwen2.5-coder:7b` or drop GGUF/SafeTensors files in your workspace.",
            workspace.display()
        )
    } else {
        let mut lines = vec![format!(
            "Discovered {count} pre-installed model(s) in workspace & local environment:\n"
        )];
        for (i, m) in models.iter().enumerate() {
            let quant_str = m.quantization.as_deref().unwrap_or("standard");
            let arch_str = m.architecture.as_deref().unwrap_or("general");
            lines.push(format!(
                "  [{}] {:<32} {:<16} {:<10} {:<10} {:<10} ({})",
                i + 1,
                m.name,
                arch_str,
                m.format,
                m.size_human,
                quant_str,
                m.source
            ));
            lines.push(format!("      Location: {}", m.location));
            if let Some(ref d) = m.details {
                lines.push(format!("      Details:  {d}"));
            }
            if m.source.starts_with("ollama") {
                lines.push(format!(
                    "      Bind via: pandora setup --preset ollama --model {}",
                    m.name
                ));
            }
            lines.push(String::new());
        }
        lines.join("\n")
    };

    Ok(success(
        "models",
        json!({
            "workspace": workspace,
            "count": count,
            "models": models,
        }),
        message,
    ))
}

pub fn route(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, ALLOWED_ROUTE_OPTIONS)?;

    let strategy_str = parsed
        .value("strategy")
        .unwrap_or("ucb1")
        .to_ascii_lowercase();

    let c: f64 = if let Some(val) = parsed.value("c") {
        val.parse::<f64>().map_err(|_| {
            CliError::usage("option '--c' must be a valid positive float (e.g. 1.414)")
        })?
    } else {
        std::f64::consts::SQRT_2
    };
    if c <= 0.0 {
        return Err(CliError::usage("option '--c' must be strictly positive"));
    }

    let epsilon: f64 = if let Some(val) = parsed.value("epsilon") {
        val.parse::<f64>().map_err(|_| {
            CliError::usage("option '--epsilon' must be a float between 0.0 and 1.0 (e.g. 0.1)")
        })?
    } else {
        0.1
    };
    if !(0.0..=1.0).contains(&epsilon) {
        return Err(CliError::usage(
            "option '--epsilon' must be between 0.0 and 1.0",
        ));
    }

    let strategy = match strategy_str.as_str() {
        "ucb1" | "ucb" => BanditStrategy::Ucb1 { c },
        "thompson" | "thompson-sampling" | "thompson_sampling" | "ts" => {
            BanditStrategy::ThompsonSampling
        }
        "epsilon-greedy" | "epsilon_greedy" | "eps-greedy" | "eg" => {
            BanditStrategy::EpsilonGreedy { epsilon }
        }
        unknown => {
            return Err(CliError::usage(format!(
                "unknown bandit strategy '{unknown}'; supported: ucb1, thompson, epsilon-greedy"
            )));
        }
    };

    let seed: u64 = if let Some(val) = parsed.value("seed") {
        val.parse::<u64>().map_err(|_| {
            CliError::usage("option '--seed' must be a valid 64-bit unsigned integer (e.g. 42)")
        })?
    } else {
        0x1234_5678_9abc_def0
    };

    let simulate_flag = parsed.has_flag("simulate")
        || parsed
            .value("simulate")
            .is_some_and(|v| v != "false" && v != "0");

    let rounds_opt: Option<usize> =
        if let Some(val) = parsed.value("rounds").or_else(|| parsed.value("pulls")) {
            let r = val
                .parse::<usize>()
                .map_err(|_| CliError::usage("option '--rounds' must be a positive integer"))?;
            if r == 0 {
                return Err(CliError::usage("option '--rounds' must be at least 1"));
            }
            Some(r)
        } else {
            None
        };

    let rounds = rounds_opt.unwrap_or(if simulate_flag { 100 } else { 1 });

    let prompt = parsed.value("prompt");
    let base_context = if let Some(p) = prompt {
        RoutingContext::default_for_prompt(p)
    } else {
        RoutingContext::new(1024, 0.5, 0.5, 5000.0)
    };

    let query_length = if let Some(val) =
        parsed.value("query-length").or_else(|| parsed.value("len"))
    {
        val.parse::<usize>()
            .map_err(|_| CliError::usage("option '--query-length' must be a positive integer"))?
    } else {
        base_context.query_length
    };

    let task_complexity = if let Some(val) = parsed.value("complexity") {
        val.parse::<f64>().map_err(|_| {
            CliError::usage("option '--complexity' must be a float between 0.0 and 1.0")
        })?
    } else {
        base_context.task_complexity
    };
    if !(0.0..=1.0).contains(&task_complexity) {
        return Err(CliError::usage(
            "option '--complexity' must be between 0.0 and 1.0",
        ));
    }

    let cache_affinity = if let Some(val) = parsed.value("cache-affinity") {
        val.parse::<f64>().map_err(|_| {
            CliError::usage("option '--cache-affinity' must be a float between 0.0 and 1.0")
        })?
    } else {
        base_context.cache_affinity
    };
    if !(0.0..=1.0).contains(&cache_affinity) {
        return Err(CliError::usage(
            "option '--cache-affinity' must be between 0.0 and 1.0",
        ));
    }

    let max_latency_ms = if let Some(val) = parsed.value("max-latency-ms") {
        val.parse::<f64>()
            .map_err(|_| CliError::usage("option '--max-latency-ms' must be a positive float"))?
    } else {
        base_context.max_latency_ms
    };
    if max_latency_ms <= 0.0 {
        return Err(CliError::usage(
            "option '--max-latency-ms' must be positive",
        ));
    }

    let context = RoutingContext::new(
        query_length,
        task_complexity,
        cache_affinity,
        max_latency_ms,
    );

    let optimal_reward: f64 = if let Some(val) = parsed.value("optimal-reward") {
        val.parse::<f64>().map_err(|_| {
            CliError::usage("option '--optimal-reward' must be a float between 0.0 and 1.0")
        })?
    } else {
        0.90
    };
    if !(0.0..=1.0).contains(&optimal_reward) {
        return Err(CliError::usage(
            "option '--optimal-reward' must be between 0.0 and 1.0",
        ));
    }

    let candidate_arms: Vec<BanditArm> = if let Some(arms_str) = parsed.value("arms") {
        parse_custom_arms(arms_str)?
    } else {
        default_candidate_arms()
    };

    let mut router = BanditModelRouter::with_seed(strategy.clone(), seed);
    for arm in candidate_arms {
        router.register_arm(arm);
    }

    let reward_calc = RoutingReward::default();
    let mut env_prng = SafePrng::new(seed.wrapping_add(0xfeed_beef));

    for _ in 0..rounds {
        let chosen_id = router
            .select_arm(&context)
            .ok_or_else(|| CliError::execution("no arms registered in bandit router", json!({})))?;
        let (succeeded, elapsed_ms, total_tokens) =
            simulate_arm_execution(&chosen_id, &context, &mut env_prng);
        let reward = reward_calc.calculate(
            succeeded,
            elapsed_ms,
            context.max_latency_ms,
            total_tokens,
            4096.0,
        );
        router.record_contextual_reward(&chosen_id, reward, &context);
    }

    let selected_arm_id = router
        .select_arm(&context)
        .ok_or_else(|| CliError::execution("no arm could be selected", json!({})))?;

    let empirical_regret = router.cumulative_regret(optimal_reward);
    let minimax_bound =
        BanditModelRouter::theoretical_minimax_regret(router.arm_count(), router.total_pulls());
    let bound_satisfied = empirical_regret <= minimax_bound;

    let arm_reports: Vec<ArmReport> = router
        .arms()
        .map(|arm| ArmReport {
            id: arm.id().to_string(),
            model_id: arm.model_id().to_string(),
            provider_id: arm.provider_id().to_string(),
            pull_count: arm.pull_count(),
            total_reward: arm.total_reward(),
            mean_reward: arm.mean_reward(),
            alpha: arm.alpha(),
            beta: arm.beta(),
            feature_weights: arm.feature_weights().to_vec(),
        })
        .collect();

    let selected_arm_obj = router.get_arm(&selected_arm_id);
    let selected_model = selected_arm_obj
        .map(|a| a.model_id().to_string())
        .unwrap_or_default();
    let selected_provider = selected_arm_obj
        .map(|a| a.provider_id().to_string())
        .unwrap_or_default();

    let report = RouteReport {
        strategy: match strategy {
            BanditStrategy::Ucb1 { .. } => "ucb1".to_string(),
            BanditStrategy::ThompsonSampling => "thompson".to_string(),
            BanditStrategy::EpsilonGreedy { .. } => "epsilon-greedy".to_string(),
        },
        strategy_params: StrategyParamsReport {
            c: match strategy {
                BanditStrategy::Ucb1 { c } => Some(c),
                _ => None,
            },
            epsilon: match strategy {
                BanditStrategy::EpsilonGreedy { epsilon } => Some(epsilon),
                _ => None,
            },
        },
        seed,
        rounds: router.total_pulls(),
        arm_count: router.arm_count(),
        selected_arm: selected_arm_id,
        selected_model,
        selected_provider,
        context: ContextReport {
            query_length: context.query_length,
            task_complexity: context.task_complexity,
            cache_affinity: context.cache_affinity,
            max_latency_ms: context.max_latency_ms,
            features: context.to_features().to_vec(),
        },
        arms: arm_reports,
        regret: RegretReport {
            optimal_reward_mean: optimal_reward,
            cumulative_empirical_regret: empirical_regret,
            theoretical_minimax_bound: minimax_bound,
            bound_satisfied,
        },
    };

    let human_text = format_route_report(&report);
    let json_data = serde_json::to_value(&report).unwrap_or_else(|_| json!({}));

    Ok(success("models route", json_data, human_text))
}

fn default_candidate_arms() -> Vec<BanditArm> {
    vec![
        BanditArm::new("arm-qwen", "qwen2.5-coder-7b", "ollama").with_context_dim(5),
        BanditArm::new("arm-deepseek", "deepseek-r1-8b", "ollama").with_context_dim(5),
        BanditArm::new("arm-llama", "llama-3.3-70b", "openai_compatible").with_context_dim(5),
        BanditArm::new("arm-claude", "claude-3-5-sonnet", "anthropic").with_context_dim(5),
    ]
}

fn parse_custom_arms(arms_str: &str) -> Result<Vec<BanditArm>, CliError> {
    let mut arms = Vec::new();
    for token in arms_str.split(',') {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parts: Vec<&str> = trimmed.split(':').collect();
        let (id, model, provider) = match parts.as_slice() {
            [id, model, provider] => (id.to_string(), model.to_string(), provider.to_string()),
            [model, provider] => (
                format!("arm-{model}"),
                model.to_string(),
                provider.to_string(),
            ),
            [model] => (
                format!("arm-{model}"),
                model.to_string(),
                "local".to_string(),
            ),
            _ => {
                return Err(CliError::usage(format!(
                    "invalid arm specification '{trimmed}'; expected [id:]model[:provider]"
                )));
            }
        };
        arms.push(BanditArm::new(id, model, provider).with_context_dim(5));
    }
    if arms.is_empty() {
        return Err(CliError::usage(
            "option '--arms' did not contain any valid arm definitions",
        ));
    }
    Ok(arms)
}

fn simulate_arm_execution(
    arm_id: &str,
    context: &RoutingContext,
    prng: &mut SafePrng,
) -> (bool, f64, f64) {
    let norm_len = (context.query_length as f64 / 4096.0).min(1.0);
    let complexity = context.task_complexity;
    let cache = context.cache_affinity;

    let (base_latency, base_success, cost_tokens) = match arm_id {
        id if id.contains("qwen") => (120.0, 0.94 + 0.05 * complexity, 250.0),
        id if id.contains("deepseek") => (180.0, 0.95 + 0.04 * complexity, 320.0),
        id if id.contains("llama") => (450.0, 0.90 + 0.05 * norm_len, 400.0),
        id if id.contains("claude") => (650.0, 0.98 + 0.01 * complexity, 500.0),
        _ => (300.0, 0.92, 350.0),
    };

    let latency_cache_factor = 1.0 - (0.40 * cache);
    let latency_len_factor = 1.0 + (0.50 * norm_len);
    let jitter = (prng.sample_uniform_f64() - 0.5) * 30.0;
    let elapsed_ms = (base_latency * latency_cache_factor * latency_len_factor + jitter).max(10.0);

    let roll = prng.sample_uniform_f64();
    let succeeded = roll < base_success.clamp(0.1, 0.999);

    let token_jitter = (prng.sample_uniform_f64() - 0.5) * 50.0;
    let total_tokens =
        (cost_tokens + (context.query_length as f64 * 0.15) + token_jitter).max(20.0);

    (succeeded, elapsed_ms, total_tokens)
}

pub fn format_route_report(report: &RouteReport) -> String {
    let mut out = String::new();
    out.push_str("Bandit Model Router (Multi-Armed Bandit LLM Selection)\n");
    out.push_str("======================================================\n");

    let strat_detail = match report.strategy.as_str() {
        "ucb1" => format!(
            "UCB1 (c = {:.4})",
            report.strategy_params.c.unwrap_or(std::f64::consts::SQRT_2)
        ),
        "thompson" => "Thompson Sampling (Beta Posterior)".to_string(),
        "epsilon-greedy" => format!(
            "Epsilon-Greedy (eps = {:.3})",
            report.strategy_params.epsilon.unwrap_or(0.1)
        ),
        s => s.to_string(),
    };

    out.push_str(&format!("Strategy:           {strat_detail}\n"));
    out.push_str(&format!("Seed:               0x{:016x}\n", report.seed));
    out.push_str(&format!("Simulation Rounds:  {}\n", report.rounds));
    out.push_str(&format!("Active Arms:        {}\n\n", report.arm_count));

    out.push_str("Context Features:\n");
    out.push_str(&format!(
        "  Query Length:     {} tokens (normalized: {:.3})\n",
        report.context.query_length,
        report.context.features.get(1).copied().unwrap_or(0.0)
    ));
    out.push_str(&format!(
        "  Task Complexity:  {:.3}\n",
        report.context.task_complexity
    ));
    out.push_str(&format!(
        "  Cache Affinity:   {:.3}\n",
        report.context.cache_affinity
    ));
    out.push_str(&format!(
        "  Max Latency:      {:.1} ms (normalized: {:.3})\n",
        report.context.max_latency_ms,
        report.context.features.get(4).copied().unwrap_or(0.0)
    ));

    let feats_str: Vec<String> = report
        .context
        .features
        .iter()
        .map(|f| format!("{:.3}", f))
        .collect();
    out.push_str(&format!(
        "  Feature Vector:   [{}]\n\n",
        feats_str.join(", ")
    ));

    out.push_str("Arm Performance Summary:\n");
    out.push_str(&format!(
        "  {:<2} {:<15} {:<20} {:<14} {:>7} {:>12} {:>8} {:>8}\n",
        "", "ARM ID", "MODEL", "PROVIDER", "PULLS", "MEAN REWARD", "ALPHA", "BETA"
    ));
    out.push_str(&format!(
        "  {:<2} {:<15} {:<20} {:<14} {:>7} {:>12} {:>8} {:>8}\n",
        "", "------", "-----", "--------", "-----", "-----------", "-----", "----"
    ));

    for arm in &report.arms {
        let marker = if arm.id == report.selected_arm {
            "*"
        } else {
            " "
        };
        out.push_str(&format!(
            "  {:<2} {:<15} {:<20} {:<14} {:>7} {:>12.4} {:>8.1} {:>8.1}\n",
            marker,
            arm.id,
            arm.model_id,
            arm.provider_id,
            arm.pull_count,
            arm.mean_reward,
            arm.alpha,
            arm.beta,
        ));
    }

    out.push_str(&format!(
        "\nRouting Decision:\n  Selected Arm:      {} (model: {}, provider: {})\n\n",
        report.selected_arm, report.selected_model, report.selected_provider
    ));

    out.push_str("Regret & Theoretical Bounds:\n");
    out.push_str(&format!(
        "  Optimal Mean (mu*):            {:.4}\n",
        report.regret.optimal_reward_mean
    ));
    out.push_str(&format!(
        "  Cumulative Empirical Regret:   {:.4}\n",
        report.regret.cumulative_empirical_regret
    ));
    out.push_str(&format!(
        "  Theoretical Minimax Bound:     {:.4}  [O(sqrt(K*T*ln(T)))]\n",
        report.regret.theoretical_minimax_bound
    ));
    let status_str = if report.regret.bound_satisfied {
        "SATISFIED (Empirical <= Minimax Bound)"
    } else {
        "EXCEEDED (Empirical > Minimax Bound)"
    };
    out.push_str(&format!(
        "  Regret Bound Status:           {}\n",
        status_str
    ));

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn test_format_bytes_scales() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1024 * 5), "5.0 KB");
        assert_eq!(format_bytes(1024 * 1024 * 50), "50.0 MB");
        assert_eq!(format_bytes(1024 * 1024 * 1024 * 7), "7.00 GB");
    }

    #[test]
    fn test_extract_hints() {
        assert_eq!(
            extract_quantization_hint("qwen2.5-coder-7b-q4_k_m.gguf"),
            Some("Q4_K_M".to_owned())
        );
        assert_eq!(
            extract_architecture_hint("qwen2.5-coder-7b-q4_k_m.gguf"),
            Some("Qwen 2.5 Coder".to_owned())
        );
        assert_eq!(
            extract_architecture_hint("deepseek-r1-distill-q8_0.gguf"),
            Some("DeepSeek R1".to_owned())
        );
    }

    #[test]
    fn test_scan_workspace_models_detects_files() {
        let temp_dir =
            std::env::temp_dir().join(format!("pandora_models_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);

        // 1. Create a dummy GGUF file with magic header
        let gguf_path = temp_dir.join("qwen2.5-coder-7b-q4_k_m.gguf");
        let mut f = File::create(&gguf_path).unwrap();
        f.write_all(b"GGUF\x01\x00\x00\x00dummy data").unwrap();

        // 2. Create a dummy SafeTensors file
        let st_path = temp_dir.join("model.safetensors");
        let mut f2 = File::create(&st_path).unwrap();
        f2.write_all(b"dummy safetensors").unwrap();

        // 3. Create a Modelfile
        let mf_path = temp_dir.join("Modelfile");
        let mut f3 = File::create(&mf_path).unwrap();
        f3.write_all(b"FROM qwen2.5-coder:7b\nPARAMETER temperature 0.2\n")
            .unwrap();

        let scanned = scan_workspace_models(&temp_dir);
        assert!(scanned.len() >= 3);

        let gguf_found = scanned.iter().find(|m| m.name.contains("qwen2.5-coder-7b"));
        assert!(gguf_found.is_some());
        let gguf = gguf_found.unwrap();
        assert_eq!(gguf.format, "GGUF");
        assert_eq!(gguf.quantization, Some("Q4_K_M".to_owned()));
        assert_eq!(gguf.architecture, Some("Qwen 2.5 Coder".to_owned()));

        let st_found = scanned.iter().find(|m| m.name == "model.safetensors");
        assert!(st_found.is_some());
        assert_eq!(st_found.unwrap().format, "SafeTensors");

        let mf_found = scanned.iter().find(|m| m.name == "Modelfile");
        assert!(mf_found.is_some());
        assert_eq!(
            mf_found.unwrap().details,
            Some("FROM qwen2.5-coder:7b".to_owned())
        );

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_models_route_ucb1_simulation() {
        let args = vec![
            "--strategy".to_string(),
            "ucb1".to_string(),
            "--c".to_string(),
            "1.414".to_string(),
            "--rounds".to_string(),
            "100".to_string(),
            "--seed".to_string(),
            "12345".to_string(),
        ];
        let res = route(&args).expect("ucb1 route simulation should succeed");
        assert_eq!(res.command, "models route");
        let data: RouteReport = serde_json::from_value(res.data).unwrap();
        assert_eq!(data.strategy, "ucb1");
        assert_eq!(data.rounds, 100);
        assert_eq!(data.arm_count, 4);
        assert!(!data.selected_arm.is_empty());
        assert!(data.regret.bound_satisfied);
        assert!(res.human.contains("SATISFIED"));
    }

    #[test]
    fn test_models_route_thompson_sampling_with_prompt() {
        let args = vec![
            "--strategy".to_string(),
            "thompson".to_string(),
            "--prompt".to_string(),
            "fn compute_fft(data: &[Complex<f64>]) -> Vec<Complex<f64>> { ... }".to_string(),
            "--complexity".to_string(),
            "0.9".to_string(),
            "--rounds".to_string(),
            "50".to_string(),
        ];
        let res = route(&args).expect("thompson sampling should succeed");
        let data: RouteReport = serde_json::from_value(res.data).unwrap();
        assert_eq!(data.strategy, "thompson");
        assert!((data.context.task_complexity - 0.9).abs() < 1e-4);
        assert!(data.regret.bound_satisfied);
        for arm in data.arms {
            assert!(arm.alpha >= 1.0);
            assert!(arm.beta >= 1.0);
        }
    }

    #[test]
    fn test_models_route_epsilon_greedy() {
        let args = vec![
            "--strategy".to_string(),
            "epsilon-greedy".to_string(),
            "--epsilon".to_string(),
            "0.2".to_string(),
            "--rounds".to_string(),
            "40".to_string(),
        ];
        let res = route(&args).expect("epsilon greedy should succeed");
        let data: RouteReport = serde_json::from_value(res.data).unwrap();
        assert_eq!(data.strategy, "epsilon-greedy");
        assert_eq!(data.strategy_params.epsilon, Some(0.2));
        assert_eq!(data.rounds, 40);
        assert!(data.regret.bound_satisfied);
    }

    #[test]
    fn test_models_route_custom_arms() {
        let args = vec![
            "--arms".to_string(),
            "fast:qwen2.5:ollama,heavy:deepseek-r1:ollama".to_string(),
            "--rounds".to_string(),
            "20".to_string(),
        ];
        let res = route(&args).expect("custom arms route should succeed");
        let data: RouteReport = serde_json::from_value(res.data).unwrap();
        assert_eq!(data.arm_count, 2);
        assert_eq!(data.arms[0].id, "fast");
        assert_eq!(data.arms[1].id, "heavy");
    }

    #[test]
    fn test_models_route_invalid_arguments() {
        assert!(route(&["--strategy".into(), "quantum".into()]).is_err());
        assert!(route(&["--c".into(), "-0.5".into()]).is_err());
        assert!(route(&["--epsilon".into(), "1.5".into()]).is_err());
        assert!(route(&["--rounds".into(), "0".into()]).is_err());
        assert!(route(&["--optimal-reward".into(), "2.0".into()]).is_err());
    }

    #[test]
    fn test_models_execute_backward_compatibility() {
        let res = execute(&[]).expect("inspect_models default should succeed");
        assert_eq!(res.command, "models");

        let res_route = execute(&["route".into(), "--rounds".into(), "5".into()])
            .expect("route subcommand should succeed");
        assert_eq!(res_route.command, "models route");
    }
}
