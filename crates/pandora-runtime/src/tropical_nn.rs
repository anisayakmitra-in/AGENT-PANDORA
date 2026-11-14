//! Phase 26: Tropical Polyhedral Neural Network (TPAN) Engine.
//!
//! Implements discrete tropical max-plus semi-ring forward evaluation:
//!     y_i = \max_j (W_{ij} + x_j)
//!
//! Key properties:
//! - Strictly `#![forbid(unsafe_code)]` with zero unsafe blocks.
//! - ZERO floating-point multiplications in the hot forward pass (pure additions and max operations).
//! - Deterministic, piecewise-linear polyhedral decision boundaries.
//! - Native serialization format (`.pnn` / JSON) with SHA-256 integrity checks.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::path::Path;

/// Tropical semi-ring neural network error.
#[derive(Debug, PartialEq, Eq)]
pub enum TropicalError {
    /// Dimension mismatch between input vector and weight matrix.
    DimensionMismatch { expected: usize, actual: usize },
    /// Layer configuration is empty or invalid.
    InvalidArchitecture(String),
    /// Integrity validation failed (SHA-256 digest mismatch).
    IntegrityFailure { expected: String, actual: String },
    /// I/O error during weight loading.
    Io(String),
    /// Deserialization failure.
    Serialization(String),
}

impl fmt::Display for TropicalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DimensionMismatch { expected, actual } => {
                write!(
                    f,
                    "Tropical dimension mismatch: expected {expected}, got {actual}"
                )
            }
            Self::InvalidArchitecture(msg) => write!(f, "Invalid tropical architecture: {msg}"),
            Self::IntegrityFailure { expected, actual } => {
                write!(
                    f,
                    "Tropical model integrity failure: expected SHA-256 {expected}, got {actual}"
                )
            }
            Self::Io(msg) => write!(f, "Tropical model I/O error: {msg}"),
            Self::Serialization(msg) => write!(f, "Tropical serialization error: {msg}"),
        }
    }
}

impl std::error::Error for TropicalError {}

/// A single Tropical Max-Plus linear layer: y_i = \max_j (W_{ij} + x_j) + b_i
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TropicalLayer {
    /// Input dimension.
    pub in_features: usize,
    /// Output dimension.
    pub out_features: usize,
    /// Tropical weight matrix W of shape [out_features, in_features] flattened row-major.
    pub weights: Vec<f32>,
    /// Optional additive bias vector of length out_features.
    pub bias: Option<Vec<f32>>,
}

impl TropicalLayer {
    /// Constructs a new tropical layer.
    pub fn new(
        in_features: usize,
        out_features: usize,
        weights: Vec<f32>,
        bias: Option<Vec<f32>>,
    ) -> Result<Self, TropicalError> {
        if in_features == 0 || out_features == 0 {
            return Err(TropicalError::InvalidArchitecture(
                "Features must be non-zero".to_string(),
            ));
        }
        let expected_weights = in_features * out_features;
        if weights.len() != expected_weights {
            return Err(TropicalError::DimensionMismatch {
                expected: expected_weights,
                actual: weights.len(),
            });
        }
        if let Some(ref b) = bias {
            if b.len() != out_features {
                return Err(TropicalError::DimensionMismatch {
                    expected: out_features,
                    actual: b.len(),
                });
            }
        }
        Ok(Self {
            in_features,
            out_features,
            weights,
            bias,
        })
    }

    /// Evaluates the tropical max-plus forward pass on an input vector.
    ///
    /// For each output unit i:
    ///     y_i = \max_{j=0}^{in-1} (W_{i,j} + x_j) + b_i
    ///
    /// Notice: ZERO multiplications are performed!
    pub fn forward(&self, input: &[f32]) -> Result<Vec<f32>, TropicalError> {
        if input.len() != self.in_features {
            return Err(TropicalError::DimensionMismatch {
                expected: self.in_features,
                actual: input.len(),
            });
        }

        let mut output = Vec::with_capacity(self.out_features);

        for i in 0..self.out_features {
            let row_offset = i * self.in_features;
            let mut max_val = f32::NEG_INFINITY;

            for j in 0..self.in_features {
                let w = self.weights[row_offset + j];
                let val = w + input[j];
                if val > max_val {
                    max_val = val;
                }
            }

            if let Some(ref b) = self.bias {
                max_val += b[i];
            }

            output.push(max_val);
        }

        Ok(output)
    }
}

/// A multi-layer Tropical Polyhedral AST Network (TPAN).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TropicalNetwork {
    /// Model name identifier.
    pub name: String,
    /// Target semantic domain (e.g. "ast_patch_scoring" or "complexity_routing").
    pub domain: String,
    /// Sequential tropical layers.
    pub layers: Vec<TropicalLayer>,
    /// SHA-256 tamper-evident digest of weight tensors.
    pub sha256_digest: String,
}

impl TropicalNetwork {
    /// Constructs and validates a tropical network, computing its SHA-256 digest.
    pub fn new(
        name: impl Into<String>,
        domain: impl Into<String>,
        layers: Vec<TropicalLayer>,
    ) -> Result<Self, TropicalError> {
        if layers.is_empty() {
            return Err(TropicalError::InvalidArchitecture(
                "Network must contain at least one layer".to_string(),
            ));
        }

        // Validate dimensional chain consistency: layer[k].out_features == layer[k+1].in_features
        for k in 0..layers.len() - 1 {
            if layers[k].out_features != layers[k + 1].in_features {
                return Err(TropicalError::DimensionMismatch {
                    expected: layers[k].out_features,
                    actual: layers[k + 1].in_features,
                });
            }
        }

        let mut net = Self {
            name: name.into(),
            domain: domain.into(),
            layers,
            sha256_digest: String::new(),
        };

        net.sha256_digest = net.compute_digest();
        Ok(net)
    }

    /// Computes the deterministic SHA-256 digest of layer weights.
    pub fn compute_digest(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.name.as_bytes());
        hasher.update(self.domain.as_bytes());
        for layer in &self.layers {
            hasher.update((layer.in_features as u64).to_le_bytes());
            hasher.update((layer.out_features as u64).to_le_bytes());
            for w in &layer.weights {
                hasher.update(w.to_bits().to_le_bytes());
            }
            if let Some(ref bias) = layer.bias {
                for b in bias {
                    hasher.update(b.to_bits().to_le_bytes());
                }
            }
        }
        format!("{:x}", hasher.finalize())
    }

    /// Full sequential forward evaluation across all tropical layers.
    pub fn forward(&self, input: &[f32]) -> Result<Vec<f32>, TropicalError> {
        let mut current = input.to_vec();
        for layer in &self.layers {
            current = layer.forward(&current)?;
        }
        Ok(current)
    }

    /// Saves the network to a JSON/PNN file.
    pub fn save_to_file(&self, path: &Path) -> Result<(), TropicalError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| TropicalError::Io(e.to_string()))?;
        }
        let serialized = serde_json::to_string_pretty(self)
            .map_err(|e| TropicalError::Serialization(e.to_string()))?;
        fs::write(path, serialized).map_err(|e| TropicalError::Io(e.to_string()))
    }

    /// Loads and verifies a network from file, validating the SHA-256 integrity digest.
    pub fn load_from_file(path: &Path) -> Result<Self, TropicalError> {
        let content = fs::read_to_string(path).map_err(|e| TropicalError::Io(e.to_string()))?;
        let net: TropicalNetwork = serde_json::from_str(&content)
            .map_err(|e| TropicalError::Serialization(e.to_string()))?;

        let actual_digest = net.compute_digest();
        if net.sha256_digest != actual_digest {
            return Err(TropicalError::IntegrityFailure {
                expected: net.sha256_digest,
                actual: actual_digest,
            });
        }

        Ok(net)
    }
}
