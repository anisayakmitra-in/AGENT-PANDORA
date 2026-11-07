use super::parse_options;
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::{BatchVectorDispatcher, HardwareAutopilot, Precision, RooflineModel};
use pandora_types::{HostHardwareProfile, ModelFootprintEstimation, Quantization};
use serde_json::json;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("hardware requires 'probe', 'fit', 'presets', 'roofline', or 'benchmark'"))?;

    match subcommand.as_str() {
        "probe" => probe(&args[1..]),
        "fit" => fit(&args[1..]),
        "presets" => presets(&args[1..]),
        "roofline" => roofline(&args[1..]),
        "benchmark" => benchmark(&args[1..]),
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

fn roofline(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "batch", "batch-size", "corpus", "corpus-size", "precision",
            "parallel-fraction", "simd-lanes",
        ],
    )?;
    let batch_size: usize = parsed
        .value("batch")
        .or_else(|| parsed.value("batch-size"))
        .map(|v| v.parse().map_err(|_| CliError::usage("--batch must be an integer")))
        .transpose()?
        .unwrap_or(64);
    let corpus_size: usize = parsed
        .value("corpus")
        .or_else(|| parsed.value("corpus-size"))
        .map(|v| v.parse().map_err(|_| CliError::usage("--corpus must be an integer")))
        .transpose()?
        .unwrap_or(1024);
    let precision_str = parsed.value("precision").unwrap_or("f32");
    let precision = match precision_str.to_ascii_lowercase().as_str() {
        "int8" | "i8" => Precision::Int8,
        _ => Precision::F32,
    };
    let parallel_fraction: f64 = parsed
        .value("parallel-fraction")
        .map(|v| v.parse().map_err(|_| CliError::usage("--parallel-fraction must be a number")))
        .transpose()?
        .unwrap_or(0.95);
    let simd_lanes: usize = parsed
        .value("simd-lanes")
        .map(|v| v.parse().map_err(|_| CliError::usage("--simd-lanes must be an integer")))
        .transpose()?
        .unwrap_or(8);

    let operational_intensity =
        RooflineModel::calculate_intensity(batch_size, corpus_size, precision);
    let amdahl_speedup = RooflineModel::amdahl_speedup(parallel_fraction, simd_lanes);

    let summary = format!(
        "Roofline Model Analysis:\n  Batch Size (B): {batch_size}\n  Corpus Size (N): {corpus_size}\n  Precision: {:?} (b = {} byte(s))\n  Operational Intensity I(B, N): {:.4} FLOPs/byte\n  Parallel Fraction (f): {:.2}\n  SIMD Lanes (p): {simd_lanes}\n  Amdahl Theoretical Speedup S(f, p): {:.2}x",
        precision,
        precision.bytes_per_element(),
        operational_intensity,
        parallel_fraction,
        amdahl_speedup
    );

    Ok(success(
        "hardware roofline",
        json!({
            "batch_size": batch_size,
            "corpus_size": corpus_size,
            "precision": format!("{:?}", precision),
            "bytes_per_element": precision.bytes_per_element(),
            "operational_intensity": operational_intensity,
            "parallel_fraction": parallel_fraction,
            "simd_lanes": simd_lanes,
            "amdahl_speedup": amdahl_speedup,
        }),
        summary,
    ))
}

fn benchmark(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["dim", "batch", "batch-size", "corpus", "corpus-size", "iterations", "top-k"],
    )?;
    let dim: usize = parsed
        .value("dim")
        .map(|v| v.parse().map_err(|_| CliError::usage("--dim must be an integer")))
        .transpose()?
        .unwrap_or(256);
    let batch_size: usize = parsed
        .value("batch")
        .or_else(|| parsed.value("batch-size"))
        .map(|v| v.parse().map_err(|_| CliError::usage("--batch must be an integer")))
        .transpose()?
        .unwrap_or(32);
    let corpus_size: usize = parsed
        .value("corpus")
        .or_else(|| parsed.value("corpus-size"))
        .map(|v| v.parse().map_err(|_| CliError::usage("--corpus must be an integer")))
        .transpose()?
        .unwrap_or(64);
    let iterations: usize = parsed
        .value("iterations")
        .map(|v| v.parse().map_err(|_| CliError::usage("--iterations must be an integer")))
        .transpose()?
        .unwrap_or(10);
    let k: usize = parsed
        .value("top-k")
        .map(|v| v.parse().map_err(|_| CliError::usage("--top-k must be an integer")))
        .transpose()?
        .unwrap_or(5);

    // Deterministically generate vectors for test reproducibility
    let mut queries_f32 = Vec::with_capacity(batch_size);
    let mut queries_int8 = Vec::with_capacity(batch_size);
    for b in 0..batch_size {
        let mut row_f32 = Vec::with_capacity(dim);
        let mut row_int8 = Vec::with_capacity(dim);
        for d in 0..dim {
            let val = (((b * 31 + d * 17 + 7) % 200) as f32 - 100.0) / 100.0;
            row_f32.push(val);
            row_int8.push((val * 127.0).clamp(-127.0, 127.0) as i8);
        }
        queries_f32.push(row_f32);
        queries_int8.push(row_int8);
    }

    let mut corpus_f32 = Vec::with_capacity(corpus_size);
    let mut corpus_int8 = Vec::with_capacity(corpus_size);
    for c in 0..corpus_size {
        let mut row_f32 = Vec::with_capacity(dim);
        let mut row_int8 = Vec::with_capacity(dim);
        for d in 0..dim {
            let val = (((c * 43 + d * 13 + 19) % 200) as f32 - 100.0) / 100.0;
            row_f32.push(val);
            row_int8.push((val * 127.0).clamp(-127.0, 127.0) as i8);
        }
        corpus_f32.push(row_f32);
        corpus_int8.push(row_int8);
    }

    // Benchmark f32 batch cosine similarity
    let start_f32 = std::time::Instant::now();
    let mut f32_matrix = Vec::new();
    for _ in 0..iterations {
        f32_matrix = BatchVectorDispatcher::batch_cosine_similarity_matrix(&queries_f32, &corpus_f32);
    }
    let f32_duration = start_f32.elapsed();
    let f32_duration_us = f32_duration.as_micros().max(1);

    // Benchmark int8 batch dot product
    let start_int8 = std::time::Instant::now();
    let mut int8_matrix = Vec::new();
    for _ in 0..iterations {
        int8_matrix = BatchVectorDispatcher::batch_dot_product_matrix_int8(&queries_int8, &corpus_int8);
    }
    let int8_duration = start_int8.elapsed();
    let int8_duration_us = int8_duration.as_micros().max(1);

    let speedup = (f32_duration_us as f64) / (int8_duration_us as f64);
    let total_ops = (batch_size * corpus_size * dim * iterations) as f64;
    let f32_gflops = (total_ops * 2.0 / (f32_duration.as_secs_f64().max(1e-9))) / 1e9;
    let int8_gops = (total_ops * 2.0 / (int8_duration.as_secs_f64().max(1e-9))) / 1e9;

    let sample_top_k = if !f32_matrix.is_empty() {
        BatchVectorDispatcher::top_k(&f32_matrix[0], k)
    } else {
        Vec::new()
    };

    let summary = format!(
        "Hardware Vector Benchmark Telemetry:\n  Dimensions: {dim}\n  Batch Size: {batch_size}, Corpus Size: {corpus_size}\n  Iterations: {iterations}\n  F32 Cosine Similarity: {} us ({:.2} GFLOP/s)\n  Int8 Dot Product: {} us ({:.2} GOP/s)\n  Int8 Speedup: {:.2}x\n  Sample Query 0 Top-{} matches: {:?}",
        f32_duration_us,
        f32_gflops,
        int8_duration_us,
        int8_gops,
        speedup,
        sample_top_k.len(),
        sample_top_k
    );

    Ok(success(
        "hardware benchmark",
        json!({
            "dim": dim,
            "batch_size": batch_size,
            "corpus_size": corpus_size,
            "iterations": iterations,
            "f32_duration_us": f32_duration_us,
            "int8_duration_us": int8_duration_us,
            "f32_gflops": f32_gflops,
            "int8_gops": int8_gops,
            "speedup": speedup,
            "matrix_rows": f32_matrix.len(),
            "matrix_cols": if f32_matrix.is_empty() { 0 } else { f32_matrix[0].len() },
            "int8_rows": int8_matrix.len(),
            "sample_top_k": sample_top_k,
        }),
        summary,
    ))
}
