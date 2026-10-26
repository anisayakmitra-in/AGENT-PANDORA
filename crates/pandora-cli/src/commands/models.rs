use super::{load_config, parse_options};
use crate::output::{CliError, CommandResult, success};
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

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
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
}
