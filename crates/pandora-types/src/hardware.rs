use serde::{Deserialize, Serialize};

/// Accelerator backend detected or configured for local inference.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuBackend {
    Cuda,
    Metal,
    Vulkan,
    Rocm,
    CpuOnly,
}

impl GpuBackend {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cuda => "cuda",
            Self::Metal => "metal",
            Self::Vulkan => "vulkan",
            Self::Rocm => "rocm",
            Self::CpuOnly => "cpu_only",
        }
    }
}

/// A GPU device detected on the host system.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GpuDevice {
    pub name: String,
    pub backend: GpuBackend,
    pub total_vram_bytes: u64,
    pub free_vram_bytes: u64,
}

impl GpuDevice {
    pub fn new(
        name: impl Into<String>,
        backend: GpuBackend,
        total_vram_bytes: u64,
        free_vram_bytes: u64,
    ) -> Self {
        Self {
            name: name.into(),
            backend,
            total_vram_bytes,
            free_vram_bytes,
        }
    }
}

/// System hardware capabilities profile.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HostHardwareProfile {
    pub cpu_cores: usize,
    pub total_ram_bytes: u64,
    pub free_ram_bytes: u64,
    pub gpus: Vec<GpuDevice>,
    pub primary_backend: GpuBackend,
    pub total_usable_vram_bytes: u64,
}

/// GGUF and LLM weight quantization formats.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Quantization {
    Q4KM,
    Q5KM,
    Q80,
    Fp16,
    Fp32,
}

impl Quantization {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().replace(['-', '_'], "").as_str() {
            "q4km" | "q4k" | "q4" => Some(Self::Q4KM),
            "q5km" | "q5k" | "q5" => Some(Self::Q5KM),
            "q80" | "q8" => Some(Self::Q80),
            "fp16" | "f16" => Some(Self::Fp16),
            "fp32" | "f32" => Some(Self::Fp32),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Q4KM => "q4_k_m",
            Self::Q5KM => "q5_k_m",
            Self::Q80 => "q8_0",
            Self::Fp16 => "fp16",
            Self::Fp32 => "fp32",
        }
    }

    pub fn bytes_per_weight(&self) -> f64 {
        match self {
            Self::Q4KM => 0.5625, // ~4.5 bits/weight
            Self::Q5KM => 0.6875, // ~5.5 bits/weight
            Self::Q80 => 1.0625,  // ~8.5 bits/weight
            Self::Fp16 => 2.0,    // 16 bits
            Self::Fp32 => 4.0,    // 32 bits
        }
    }
}

/// Sizing and fit evaluation for running a model locally on host hardware.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ModelFootprintEstimation {
    pub model_id: String,
    pub parameters_billion: f64,
    pub quantization: Quantization,
    pub context_window: usize,
    pub weights_memory_bytes: u64,
    pub kv_cache_memory_bytes: u64,
    pub headroom_bytes: u64,
    pub total_required_bytes: u64,
    pub fits_in_vram: bool,
    pub fits_in_ram: bool,
    pub recommended_backend: GpuBackend,
    pub fit_score: f64,
    pub fit_verdict: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quantization_parsing_and_rates() {
        assert_eq!(Quantization::parse("q4_k_m"), Some(Quantization::Q4KM));
        assert_eq!(Quantization::parse("q8_0"), Some(Quantization::Q80));
        assert_eq!(Quantization::parse("fp16"), Some(Quantization::Fp16));
        assert!(Quantization::Q4KM.bytes_per_weight() < Quantization::Q80.bytes_per_weight());
    }
}
