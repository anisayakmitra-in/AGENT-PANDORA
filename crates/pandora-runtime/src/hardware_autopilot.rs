use pandora_types::{
    GpuBackend, GpuDevice, HostHardwareProfile, ModelFootprintEstimation, Quantization,
};
use std::env;
#[cfg(target_os = "linux")]
use std::fs;

pub struct HardwareAutopilot;

impl HardwareAutopilot {
    /// Probe host CPU, system memory, and GPU accelerators.
    pub fn probe_host() -> HostHardwareProfile {
        let cpu_cores = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(8);

        let total_ram_bytes = detect_total_ram();
        let free_ram_bytes = detect_free_ram(total_ram_bytes);

        let mut gpus = Vec::new();

        // 1. Check for manual environment override first (useful for containers/testing)
        if let Some(gb) = env::var("PANDORA_HOST_VRAM_GB")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
        {
            let vram_bytes = (gb * 1024.0 * 1024.0 * 1024.0) as u64;
            gpus.push(GpuDevice::new(
                "Configured Accelerator",
                GpuBackend::Cuda,
                vram_bytes,
                vram_bytes,
            ));
        }

        // 2. Platform-specific device probing if no override
        if gpus.is_empty() {
            #[cfg(target_os = "macos")]
            {
                // Apple Silicon uses Unified Memory where CPU and GPU share high-bandwidth RAM
                let metal_vram = (total_ram_bytes as f64 * 0.75) as u64; // ~75% usable as Metal VRAM
                gpus.push(GpuDevice::new(
                    "Apple Silicon Unified Memory",
                    GpuBackend::Metal,
                    metal_vram,
                    (free_ram_bytes as f64 * 0.75) as u64,
                ));
            }

            #[cfg(target_os = "windows")]
            {
                // Check if NVIDIA driver / CUDA is present via env
                if env::var("CUDA_PATH").is_ok()
                    || env::var("CUDA_HOME").is_ok()
                    || env::var("CUDA_VISIBLE_DEVICES").is_ok()
                {
                    // Default baseline NVIDIA card if CUDA environment is set
                    let vram = 16 * 1024 * 1024 * 1024; // 16 GB baseline
                    gpus.push(GpuDevice::new(
                        "NVIDIA CUDA Accelerator",
                        GpuBackend::Cuda,
                        vram,
                        vram,
                    ));
                }
            }

            #[cfg(target_os = "linux")]
            {
                if fs::metadata("/proc/driver/nvidia/version").is_ok()
                    || env::var("CUDA_VISIBLE_DEVICES").is_ok()
                {
                    let vram = 16 * 1024 * 1024 * 1024;
                    gpus.push(GpuDevice::new(
                        "NVIDIA CUDA Device",
                        GpuBackend::Cuda,
                        vram,
                        vram,
                    ));
                }
            }
        }

        let primary_backend = if let Some(gpu) = gpus.first() {
            gpu.backend
        } else {
            GpuBackend::CpuOnly
        };

        let total_usable_vram_bytes = gpus.iter().map(|g| g.total_vram_bytes).sum();

        HostHardwareProfile {
            cpu_cores,
            total_ram_bytes,
            free_ram_bytes,
            gpus,
            primary_backend,
            total_usable_vram_bytes,
        }
    }

    /// Evaluate memory footprint and compute fit score for a local model.
    pub fn evaluate_fit(
        profile: &HostHardwareProfile,
        model_id: &str,
        parameters_billion: f64,
        quantization: Quantization,
        context_window: usize,
    ) -> ModelFootprintEstimation {
        // 1. Weight memory: parameters * bytes_per_weight
        let weights_memory_bytes =
            (parameters_billion * 1_000_000_000.0 * quantization.bytes_per_weight()) as u64;

        // 2. KV cache memory estimate: ~0.5 MB per 1024 tokens context per 1B parameters for FP16 KV
        let kv_cache_memory_bytes =
            ((context_window as f64 / 1024.0) * parameters_billion * 512.0 * 1024.0) as u64;

        // 3. Headroom for runtime activations and buffers (~1 GiB)
        let headroom_bytes: u64 = 1_073_741_824;

        let total_required_bytes = weights_memory_bytes
            .saturating_add(kv_cache_memory_bytes)
            .saturating_add(headroom_bytes);

        let fits_in_vram = profile.total_usable_vram_bytes >= total_required_bytes;
        let fits_in_ram = profile.total_ram_bytes >= total_required_bytes;

        let fit_score = if profile.total_usable_vram_bytes > 0 {
            profile.total_usable_vram_bytes as f64 / total_required_bytes as f64
        } else {
            profile.total_ram_bytes as f64 / total_required_bytes as f64
        };

        let recommended_backend = if fits_in_vram {
            profile.primary_backend
        } else {
            GpuBackend::CpuOnly
        };

        let fit_verdict = if fit_score >= 1.05 {
            "Optimal (Full GPU VRAM Offload)".to_string()
        } else if fit_score >= 0.85 {
            "Tight (Partial GPU Offload / Small KV Cache)".to_string()
        } else if fits_in_ram {
            "CPU Offload Only (High Latency)".to_string()
        } else {
            "Insufficient Memory (OOM Risk)".to_string()
        };

        ModelFootprintEstimation {
            model_id: model_id.to_string(),
            parameters_billion,
            quantization,
            context_window,
            weights_memory_bytes,
            kv_cache_memory_bytes,
            headroom_bytes,
            total_required_bytes,
            fits_in_vram,
            fits_in_ram,
            recommended_backend,
            fit_score,
            fit_verdict,
        }
    }

    /// List recommendations across standard local model presets.
    pub fn preset_recommendations(profile: &HostHardwareProfile) -> Vec<ModelFootprintEstimation> {
        let presets = [
            ("qwen2.5:3b", 3.0),
            ("qwen2.5:7b", 7.6),
            ("deepseek-r1:8b", 8.0),
            ("llama3.1:8b", 8.0),
            ("phi-4:14b", 14.0),
            ("qwen2.5:14b", 14.7),
            ("deepseek-r1:14b", 14.7),
            ("qwen2.5:32b", 32.5),
            ("llama3.1:70b", 70.6),
        ];

        presets
            .iter()
            .map(|(name, params)| {
                Self::evaluate_fit(profile, name, *params, Quantization::Q4KM, 8192)
            })
            .collect()
    }
}

fn detect_total_ram() -> u64 {
    if let Some(gb) = env::var("PANDORA_HOST_RAM_GB")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
    {
        return (gb * 1024.0 * 1024.0 * 1024.0) as u64;
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(meminfo) = fs::read_to_string("/proc/meminfo") {
            for line in meminfo.lines() {
                if let Some(rest) = line.strip_prefix("MemTotal:") {
                    let parts: Vec<&str> = rest.split_whitespace().collect();
                    if let Some(first) = parts.first() {
                        if let Ok(kb) = first.parse::<u64>() {
                            return kb * 1024;
                        }
                    }
                }
            }
        }
    }

    // Default safe fallback if probing not available: 16 GiB
    16 * 1024 * 1024 * 1024
}

fn detect_free_ram(total_ram: u64) -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(meminfo) = fs::read_to_string("/proc/meminfo") {
            for line in meminfo.lines() {
                if let Some(rest) = line.strip_prefix("MemAvailable:") {
                    let parts: Vec<&str> = rest.split_whitespace().collect();
                    if let Some(first) = parts.first() {
                        if let Ok(kb) = first.parse::<u64>() {
                            return kb * 1024;
                        }
                    }
                }
            }
        }
    }

    // Default safe fallback: ~50% of total RAM
    total_ram / 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hardware_autopilot_fit_evaluation() {
        let profile = HostHardwareProfile {
            cpu_cores: 16,
            total_ram_bytes: 32 * 1024 * 1024 * 1024,
            free_ram_bytes: 16 * 1024 * 1024 * 1024,
            gpus: vec![GpuDevice::new(
                "NVIDIA RTX 4090",
                GpuBackend::Cuda,
                24 * 1024 * 1024 * 1024,
                24 * 1024 * 1024 * 1024,
            )],
            primary_backend: GpuBackend::Cuda,
            total_usable_vram_bytes: 24 * 1024 * 1024 * 1024,
        };

        // 7B model at Q4_K_M easily fits in 24GB VRAM
        let fit_7b =
            HardwareAutopilot::evaluate_fit(&profile, "qwen2.5:7b", 7.6, Quantization::Q4KM, 8192);
        assert!(fit_7b.fits_in_vram);
        assert!(fit_7b.fit_score > 1.5);
        assert_eq!(fit_7b.recommended_backend, GpuBackend::Cuda);

        // 70B model requires > 40GB VRAM, does not fit in 24GB VRAM
        let fit_70b = HardwareAutopilot::evaluate_fit(
            &profile,
            "llama3.1:70b",
            70.6,
            Quantization::Q4KM,
            8192,
        );
        assert!(!fit_70b.fits_in_vram);
        assert!(fit_70b.fit_score < 1.0);
    }
}
