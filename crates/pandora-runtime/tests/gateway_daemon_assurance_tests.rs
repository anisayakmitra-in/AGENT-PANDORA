#![forbid(unsafe_code)]

//! Integration Test Suite: Live Gateway Controller, 12-Engine Assurance & Daemon Self-Healing
//!
//! Validates:
//! 1. Ingestion of live telemetry and evaluation across all 12 engines (Theorems 17–40).
//! 2. Autonomous circuit-breaking, tool quarantining, and provider rerouting upon invariant degradation.
//! 3. Signed, hash-chained assurance receipts committed to the append-only Merkle-DAG audit ledger.
//! 4. Causal DAG lineage verification and logarithmic Merkle inclusion proofs.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use pandora_runtime::gateway::{
    CircuitBreakerState, GatewayConfig, GatewayController, SelfHealingAction,
};
use pandora_runtime::merkle_dag::MerkleDagAuditLog;

fn test_dir(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pandora_{prefix}_{}_{}",
        std::process::id(),
        fastrand()
    ));
    let _ = fs::create_dir_all(&dir);
    dir
}

fn fastrand() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(42)
}

#[test]
fn test_twelve_engine_telemetry_ingestion_and_assurance_receipt() {
    let dir = test_dir("gw_12_engine");
    let audit_path = dir.join("merkle_audit.json");

    let config = GatewayConfig {
        tick_interval_ms: 50,
        circuit_breaker_threshold: 3,
        cooldown_secs: 10,
        audit_store_path: audit_path.clone(),
        fallback_provider_map: BTreeMap::new(),
    };

    let gateway = GatewayController::new(config);
    let telemetry = gateway.collect_live_telemetry();
    let report = gateway.evaluate_telemetry_tick(telemetry).unwrap();

    assert_eq!(report.sequence_number, 0);
    assert!(report.all_invariants_verified);
    assert_eq!(report.receipt.phases_certified, 12);
    assert_eq!(report.receipt.certificates.len(), 12);

    // Verify all phases 9 to 20 are represented and verified
    for (i, cert) in report.receipt.certificates.iter().enumerate() {
        assert_eq!(cert.phase as usize, 9 + i);
        assert!(
            cert.invariant_verified,
            "phase {} failed: {}",
            cert.phase, cert.metric_summary
        );
    }

    // Verify cryptographic integrity and signature
    assert!(report.receipt.verify_hash_integrity());
    assert!(report.receipt.signature.is_some());
    assert!(
        report
            .receipt
            .signature
            .as_ref()
            .unwrap()
            .starts_with("sig:sha256:")
    );

    // Verify audit log commitment
    assert!(audit_path.exists());
    assert_eq!(report.audit_leaf_index, 0);
    assert!(!report.audit_root_hash.is_empty());
    assert!(!report.audit_node_id.is_empty());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_circuit_breaking_and_autonomous_rerouting_on_invariant_degradation() {
    let dir = test_dir("gw_self_healing");
    let audit_path = dir.join("merkle_audit.json");

    let mut fallback_map = BTreeMap::new();
    fallback_map.insert("anthropic".to_string(), "openai".to_string());
    fallback_map.insert("openai".to_string(), "fallback_local".to_string());

    let config = GatewayConfig {
        tick_interval_ms: 50,
        circuit_breaker_threshold: 2,
        cooldown_secs: 15,
        audit_store_path: audit_path.clone(),
        fallback_provider_map: fallback_map,
    };

    let gateway = GatewayController::new(config);
    assert_eq!(gateway.active_provider(), "anthropic");

    // Telemetry with degraded latency on active provider "anthropic" (1850ms > 1000ms SLA)
    // and failing tool "untrusted_shell" (error rate 0.75 > 0.30/0.50)
    let mut degraded_telemetry = gateway.collect_live_telemetry();
    degraded_telemetry
        .provider_latencies
        .insert("anthropic".to_string(), 1850.0);
    degraded_telemetry
        .tool_error_rates
        .insert("untrusted_shell".to_string(), 0.75);

    // Tick 1: Failure recorded (failure count 1 < threshold 2 for breaker)
    let report1 = gateway
        .evaluate_telemetry_tick(degraded_telemetry.clone())
        .unwrap();
    // High tool error rate > 0.50 immediately triggers quarantine
    let has_quarantine = report1.self_healing_actions.iter().any(|a| {
        matches!(a, SelfHealingAction::QuarantineTool { tool_id, .. } if tool_id == "untrusted_shell")
    });
    assert!(has_quarantine);

    // Breaker for anthropic still not open
    assert_eq!(
        gateway.circuit_breaker_states().get("anthropic"),
        Some(&CircuitBreakerState::Closed)
    );

    // Tick 2: Failure count 2 >= threshold 2 -> Breaker trips to Open and reroutes to "openai"
    let report2 = gateway.evaluate_telemetry_tick(degraded_telemetry).unwrap();

    let has_trip = report2.self_healing_actions.iter().any(|a| {
        matches!(a, SelfHealingAction::TripCircuitBreaker { target, .. } if target == "anthropic")
    });
    let has_reroute = report2.self_healing_actions.iter().any(|a| {
        matches!(a, SelfHealingAction::RerouteProvider { from_provider, to_provider, .. }
            if from_provider == "anthropic" && to_provider == "openai")
    });

    assert!(has_trip, "expected TripCircuitBreaker for anthropic");
    assert!(
        has_reroute,
        "expected RerouteProvider from anthropic to openai"
    );
    assert_eq!(gateway.active_provider(), "openai");
    assert_eq!(
        gateway.circuit_breaker_states().get("anthropic"),
        Some(&CircuitBreakerState::Open)
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_signed_hash_chained_receipts_and_audit_ledger_causal_lineage() {
    let dir = test_dir("gw_chain_audit");
    let audit_path = dir.join("merkle_audit.json");

    let config = GatewayConfig {
        tick_interval_ms: 10,
        circuit_breaker_threshold: 3,
        cooldown_secs: 30,
        audit_store_path: audit_path.clone(),
        fallback_provider_map: BTreeMap::new(),
    };

    let gateway = GatewayController::new(config);

    // Produce 3 successive ticks
    let t1 = gateway
        .evaluate_telemetry_tick(gateway.collect_live_telemetry())
        .unwrap();
    let t2 = gateway
        .evaluate_telemetry_tick(gateway.collect_live_telemetry())
        .unwrap();
    let t3 = gateway
        .evaluate_telemetry_tick(gateway.collect_live_telemetry())
        .unwrap();

    // Verify hash-chaining across receipts
    assert_eq!(t1.sequence_number, 0);
    assert_eq!(t1.receipt.prev_receipt_sha256, None);
    assert!(t1.receipt.verify_hash_integrity());

    assert_eq!(t2.sequence_number, 1);
    assert_eq!(
        t2.receipt.prev_receipt_sha256,
        Some(t1.receipt.receipt_sha256.clone())
    );
    assert!(t2.receipt.verify_hash_integrity());

    assert_eq!(t3.sequence_number, 2);
    assert_eq!(
        t3.receipt.prev_receipt_sha256,
        Some(t2.receipt.receipt_sha256.clone())
    );
    assert!(t3.receipt.verify_hash_integrity());

    // Verify audit log file on disk
    let raw_store_bytes = fs::read(&audit_path).unwrap();
    let store_file: serde_json::Value = serde_json::from_slice(&raw_store_bytes).unwrap();

    assert_eq!(store_file["version"], 1);
    let leaves = store_file["leaves"].as_array().unwrap();
    let dag_entries = store_file["dag_entries"].as_array().unwrap();

    assert_eq!(leaves.len(), 3);
    assert_eq!(dag_entries.len(), 3);

    // Reconstruct MerkleDagAuditLog and verify causal lineage
    let mut log = MerkleDagAuditLog::new();
    for leaf in leaves {
        let hex_str = leaf["payload_hex"].as_str().unwrap();
        let payload = (0..hex_str.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex_str[i..i + 2], 16).unwrap())
            .collect::<Vec<u8>>();
        log.append_leaf(&payload);
    }
    for entry in dag_entries {
        let hex_str = entry["payload_hex"].as_str().unwrap();
        let payload = (0..hex_str.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex_str[i..i + 2], 16).unwrap())
            .collect::<Vec<u8>>();
        let parents = entry["parent_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap().to_string())
            .collect::<Vec<String>>();
        let ts = entry["timestamp_secs"].as_u64().unwrap();
        log.add_dag_node(&payload, &parents, ts).unwrap();
    }

    // Verify topological acyclicity and causal lineage of node 3
    assert!(log.is_acyclic());
    assert!(log.verify_causal_lineage(&t3.audit_node_id).unwrap());
    assert!(log.verify_causal_lineage(&t2.audit_node_id).unwrap());
    assert!(log.verify_causal_lineage(&t1.audit_node_id).unwrap());

    // Verify inclusion proof for leaf 1
    let proof = log.generate_proof(1).unwrap();
    assert_eq!(proof.leaf_index, 1);
    assert!(proof.verify());
    assert!(MerkleDagAuditLog::verify_proof(&proof));

    let _ = fs::remove_dir_all(&dir);
}
