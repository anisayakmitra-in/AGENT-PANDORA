//! Unit & Integration tests for Google Colab Remote GPU Bridge under `#![forbid(unsafe_code)]`.

#![forbid(unsafe_code)]

use pandora_runtime::colab_bridge::{ColabClient, ColabSession};
use std::fs;

#[test]
fn test_colab_session_serialization_roundtrip() {
    let session = ColabSession {
        url: "https://pandora-test-gpu.trycloudflare.com".to_string(),
        token: "pandora-colab-secret-test-token-12345".to_string(),
        connected_at: 1731580000,
        gpu_device: "NVIDIA Tesla T4".to_string(),
        vram_total_mb: 15109,
        cuda_version: "12.2".to_string(),
        bridge_version: "v3.1.0".to_string(),
    };

    let serialized = serde_json::to_string_pretty(&session).expect("serialization succeeds");
    let deserialized: ColabSession =
        serde_json::from_str(&serialized).expect("deserialization succeeds");

    assert_eq!(session, deserialized);
    assert_eq!(deserialized.gpu_device, "NVIDIA Tesla T4");
    assert_eq!(deserialized.vram_total_mb, 15109);
}

#[test]
fn test_colab_session_filesystem_persistence() {
    let unique = format!(
        "pandora_colab_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let temp_dir = std::env::temp_dir().join(unique);
    let session_path = temp_dir.join("sub").join("session.json");

    assert!(
        ColabClient::load_session(&session_path)
            .expect("load succeeds")
            .is_none()
    );

    let session = ColabSession {
        url: "https://worker-a100.trycloudflare.com".to_string(),
        token: "tok-abc-def".to_string(),
        connected_at: 1731581111,
        gpu_device: "NVIDIA A100-SXM4-40GB".to_string(),
        vram_total_mb: 40960,
        cuda_version: "12.4".to_string(),
        bridge_version: "v3.1.0".to_string(),
    };

    ColabClient::save_session(&session, &session_path).expect("save succeeds");
    assert!(session_path.exists());

    let loaded = ColabClient::load_session(&session_path)
        .expect("load succeeds")
        .expect("session present");
    assert_eq!(loaded, session);

    ColabClient::clear_session(&session_path).expect("clear succeeds");
    assert!(!session_path.exists());
    assert!(
        ColabClient::load_session(&session_path)
            .expect("load succeeds")
            .is_none()
    );
    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_colab_client_url_sanitization() {
    let client = ColabClient::new("  https://my-colab.trycloudflare.com///  ", "  token-123  ");
    assert_eq!(client.base_url(), "https://my-colab.trycloudflare.com");
    assert_eq!(client.token(), "token-123");
}
