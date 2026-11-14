//! Phase 26: Tropical Polyhedral Neural Network (TPAN) Test Suite.

#![forbid(unsafe_code)]

use pandora_runtime::tropical_nn::{TropicalError, TropicalLayer, TropicalNetwork};
use std::fs;

#[test]
fn test_tropical_layer_forward_exact_math() {
    // 2 inputs, 2 outputs
    // W = [[1.0, 3.0],
    //      [-2.0, 0.5]]
    // bias = [0.1, -0.2]
    // x = [2.0, 1.0]
    //
    // Output 0: max(1.0 + 2.0, 3.0 + 1.0) + 0.1 = max(3.0, 4.0) + 0.1 = 4.1
    // Output 1: max(-2.0 + 2.0, 0.5 + 1.0) - 0.2 = max(0.0, 1.5) - 0.2 = 1.3
    let layer = TropicalLayer::new(2, 2, vec![1.0, 3.0, -2.0, 0.5], Some(vec![0.1, -0.2]))
        .expect("layer constructs successfully");

    let input = vec![2.0, 1.0];
    let output = layer.forward(&input).expect("forward pass succeeds");

    assert_eq!(output.len(), 2);
    assert!((output[0] - 4.1).abs() < 1e-6);
    assert!((output[1] - 1.3).abs() < 1e-6);
}

#[test]
fn test_tropical_layer_dimension_validation() {
    // Expected 2x2 = 4 weights, provided 3
    let err = TropicalLayer::new(2, 2, vec![1.0, 2.0, 3.0], None).unwrap_err();
    assert_eq!(
        err,
        TropicalError::DimensionMismatch {
            expected: 4,
            actual: 3
        }
    );

    let layer = TropicalLayer::new(3, 2, vec![0.0; 6], None).unwrap();
    // Input has length 2, expected 3
    let forward_err = layer.forward(&[1.0, 2.0]).unwrap_err();
    assert_eq!(
        forward_err,
        TropicalError::DimensionMismatch {
            expected: 3,
            actual: 2
        }
    );
}

#[test]
fn test_tropical_multi_layer_chaining() {
    // Layer 1: 3 in -> 2 out
    // Layer 2: 2 in -> 1 out
    let layer1 = TropicalLayer::new(3, 2, vec![1.0, 0.0, 2.0, -1.0, 4.0, 1.0], None).unwrap();

    let layer2 = TropicalLayer::new(2, 1, vec![0.5, -0.5], Some(vec![1.0])).unwrap();

    let net = TropicalNetwork::new("test_tpan", "ast_scoring", vec![layer1, layer2])
        .expect("network constructs successfully");

    let x = vec![1.0, 2.0, 0.5];
    // Layer 1:
    // Out 0: max(1+1, 0+2, 2+0.5) = max(2.0, 2.0, 2.5) = 2.5
    // Out 1: max(-1+1, 4+2, 1+0.5) = max(0.0, 6.0, 1.5) = 6.0
    // Layer 2:
    // Out 0: max(0.5 + 2.5, -0.5 + 6.0) + 1.0 = max(3.0, 5.5) + 1.0 = 6.5
    let result = net.forward(&x).expect("network forward succeeds");
    assert_eq!(result.len(), 1);
    assert!((result[0] - 6.5).abs() < 1e-6);
}

#[test]
fn test_tropical_network_serialization_and_tamper_detection() {
    let layer = TropicalLayer::new(2, 1, vec![1.0, -1.0], Some(vec![0.5])).unwrap();
    let net = TropicalNetwork::new("tpan_tamper_test", "benchmark", vec![layer]).unwrap();

    let unique = format!(
        "tpan_model_{}.json",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let path = std::env::temp_dir().join(unique);

    net.save_to_file(&path).expect("save succeeds");
    assert!(path.exists());

    // Clean load
    let loaded = TropicalNetwork::load_from_file(&path).expect("load succeeds");
    assert_eq!(net, loaded);

    // Tamper with file
    let mut raw = fs::read_to_string(&path).unwrap();
    raw = raw.replace("1.0", "999.0");
    fs::write(&path, raw).unwrap();

    let tamper_err = TropicalNetwork::load_from_file(&path).unwrap_err();
    match tamper_err {
        TropicalError::IntegrityFailure { .. } => {}
        other => panic!("expected IntegrityFailure, got {other:?}"),
    }

    let _ = fs::remove_file(&path);
}

#[test]
fn test_train_synthetic_tpan_and_export_default_model() {
    let model = TropicalNetwork::train_synthetic_tpan(16, 64, 1, 30)
        .expect("synthetic TPAN training succeeds");
    assert_eq!(model.layers.len(), 3);
    assert_eq!(model.layers[0].in_features, 16);
    assert_eq!(model.layers[0].out_features, 64);
    assert_eq!(model.layers[1].in_features, 64);
    assert_eq!(model.layers[1].out_features, 32);
    assert_eq!(model.layers[2].in_features, 32);
    assert_eq!(model.layers[2].out_features, 1);

    let sample_x = vec![0.5f32; 16];
    let output = model.forward(&sample_x).expect("forward pass succeeds");
    assert_eq!(output.len(), 1);

    // Save default model artifact to .pandora/model_tpan.pnn
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root exists");
    let model_path = workspace_root.join(".pandora").join("model_tpan.pnn");
    model
        .save_to_file(&model_path)
        .expect("save model artifact succeeds");

    // Load back and verify integrity
    let reloaded = TropicalNetwork::load_from_file(&model_path)
        .expect("reloading model succeeds with verified digest");
    assert_eq!(reloaded.layers.len(), 3);
    assert_eq!(reloaded.sha256_digest, model.sha256_digest);
}
