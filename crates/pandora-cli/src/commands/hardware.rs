use super::parse_options;
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::HardwareAutopilot;
use pandora_types::{HostHardwareProfile, ModelFootprintEstimation, Quantization};
use serde_json::json;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("hardware requires 'probe', 'fit', or 'presets'"))?;

    match subcommand.as_str() {
        "probe" => probe(&args[1..]),
        "fit" => fit(&args[1..]),
        "presets" => presets(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown hardware command '{unknown}'"
        ))),
    }
}

fn probe(args: &[String]) -> Result<CommandResult, CliError> {
    let _ = parse_options(args, &[])?;
    let profile = HardwareAutopilot::probe_host();
    let text = format_probe(&profile);
    Ok(success(
        "hardware probe",
        serde_json::to_value(&profile).unwrap_or_else(|_| json!({})),
        text,
    ))
}

fn fit(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["model", "params", "quant", "context"])?;
    let profile = HardwareAutopilot::probe_host();

    let model = parsed.value("model").unwrap_or("custom-model");
    let params: f64 = if let Some(p) = parsed.value("params") {
        p.parse().map_err(|_| {
            CliError::usage("hardware fit --params must be a valid number (e.g. 7.0)")
        })?
    } else {
        7.0
    };

    let quant_str = parsed.value("quant").unwrap_or("q4_k_m");
    let quant = Quantization::parse(quant_str).ok_or_else(|| {
        CliError::usage(format!(
            "unknown quantization '{quant_str}'; supported: q4_k_m, q5_k_m, q8_0, fp16, fp32"
        ))
    })?;

    let context: usize = if let Some(c) = parsed.value("context") {
        c.parse()
            .map_err(|_| CliError::usage("hardware fit --context must be an integer tokens"))?
    } else {
        8192
    };

    let estimation = HardwareAutopilot::evaluate_fit(&profile, model, params, quant, context);
    let text = format_fit(&estimation);

    Ok(success(
        "hardware fit",
        serde_json::to_value(&estimation).unwrap_or_else(|_| json!({})),
        text,
    ))
}

fn presets(args: &[String]) -> Result<CommandResult, CliError> {
    let _ = parse_options(args, &[])?;
    let profile = HardwareAutopilot::probe_host();
    let recommendations = HardwareAutopilot::preset_recommendations(&profile);
    let text = format_presets(&recommendations);

    Ok(success(
        "hardware presets",
        json!({ "recommendations": recommendations }),
        text,
    ))
}

fn format_probe(profile: &HostHardwareProfile) -> String {
    let mut out = format!(
        "Host Hardware Profile:\n  CPU Cores: {}\n  System RAM: {:.2} GiB (Free: {:.2} GiB)\n  Primary Accelerator: {}\n  Total Usable VRAM: {:.2} GiB\n",
        profile.cpu_cores,
        profile.total_ram_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
        profile.free_ram_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
        profile.primary_backend.as_str(),
        profile.total_usable_vram_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
    );
    if profile.gpus.is_empty() {
        out.push_str("  Accelerators: None detected (CPU offload fallback)\n");
    } else {
        out.push_str("  Accelerators:\n");
        for (i, gpu) in profile.gpus.iter().enumerate() {
            out.push_str(&format!(
                "    [{}] {} (backend: {}): {:.2} GiB VRAM\n",
                i,
                gpu.name,
                gpu.backend.as_str(),
                gpu.total_vram_bytes as f64 / (1024.0 * 1024.0 * 1024.0)
            ));
        }
    }
    out
}

fn format_fit(fit: &ModelFootprintEstimation) -> String {
    format!(
        "Model Sizing Evaluation: {}\n  Parameters: {:.1}B\n  Quantization: {}\n  Context Window: {} tokens\n  Weights Memory: {:.2} GiB\n  KV Cache Memory: {:.2} GiB\n  Runtime Headroom: {:.2} GiB\n  Total Memory Required: {:.2} GiB\n  Fits in GPU VRAM: {}\n  Fits in System RAM: {}\n  Recommended Backend: {}\n  Fit Score: {:.2}\n  Verdict: {}",
        fit.model_id,
        fit.parameters_billion,
        fit.quantization.as_str(),
        fit.context_window,
        fit.weights_memory_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
        fit.kv_cache_memory_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
        fit.headroom_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
        fit.total_required_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
        if fit.fits_in_vram { "Yes" } else { "No" },
        if fit.fits_in_ram { "Yes" } else { "No" },
        fit.recommended_backend.as_str(),
        fit.fit_score,
        fit.fit_verdict,
    )
}

fn format_presets(presets: &[ModelFootprintEstimation]) -> String {
    let mut out = String::from("Local Model Hardware Fit Recommendations:\n\n");
    out.push_str(&format!(
        "{:<18} {:<10} {:<12} {:<14} {}\n",
        "MODEL", "PARAMS", "REQ (GiB)", "GPU OFFLOAD", "VERDICT"
    ));
    out.push_str(&format!(
        "{:-<18} {:-<10} {:-<12} {:-<14} {:-<35}\n",
        "", "", "", "", ""
    ));
    for p in presets {
        let req_gib = p.total_required_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
        let offload = if p.fits_in_vram {
            "Full VRAM"
        } else if p.fits_in_ram {
            "CPU Only"
        } else {
            "OOM"
        };
        out.push_str(&format!(
            "{:<18} {:<10.1} {:<12.2} {:<14} {}\n",
            p.model_id, p.parameters_billion, req_gib, offload, p.fit_verdict
        ));
    }
    out
}
