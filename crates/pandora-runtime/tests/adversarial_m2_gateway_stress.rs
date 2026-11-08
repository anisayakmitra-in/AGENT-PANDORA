#![forbid(unsafe_code)]

//! Adversarial Empirical Stress & Boundary Challenge Suite: Milestone 2 Gateway Controller
//!
//! Conducts empirical stress testing and boundary verification targeting:
//! 1. Circuit breaker rapid state oscillations (Closed -> Open -> HalfOpen -> Closed / Open).
//! 2. Provider failure cascade: primary -> secondary -> all providers failed, cyclic maps.
//! 3. High-throughput telemetry ingestion & extreme boundary values (NaN, Inf, negative, zero).
//! 4. Tool quarantine triggering and canary gate martingale boundary conditions / cross-talk.
//! 5. Concurrent multi-threaded telemetry ingestion and audit ledger integrity.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use pandora_runtime::gateway::{
    CircuitBreaker, CircuitBreakerState, DagTopologyTelemetry, ExecutionTelemetry, GatewayConfig,
    GatewayController, SelfHealingAction,
};
use pandora_runtime::merkle_dag::MerkleDagAuditLog;

fn test_temp_dir(prefix: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "pandora_adv_m2_{prefix}_{}_{}",
        std::process::id(),
        id
    ));
    let _ = fs::create_dir_all(&dir);
    dir
}

// =========================================================================
// SECTION 1: CIRCUIT BREAKER OSCILLATIONS & STATE TRANSITIONS
// =========================================================================

#[test]
fn test_circuit_breaker_full_lifecycle_and_rapid_oscillations() {
    let mut cb = CircuitBreaker::new("test_service", 3, 10);
    assert_eq!(cb.state, CircuitBreakerState::Closed);
    assert_eq!(cb.failure_count, 0);

    let t0 = 1000;

    // Tick 1: Failure 1 -> still Closed
    assert!(!cb.record_failure("error 1", t0));
    assert_eq!(cb.state, CircuitBreakerState::Closed);
    assert_eq!(cb.failure_count, 1);

    // Tick 2: Failure 2 -> still Closed
    assert!(!cb.record_failure("error 2", t0 + 1));
    assert_eq!(cb.state, CircuitBreakerState::Closed);
    assert_eq!(cb.failure_count, 2);

    // Tick 3: Failure 3 -> Trips to Open!
    assert!(cb.record_failure("error 3", t0 + 2));
    assert_eq!(cb.state, CircuitBreakerState::Open);
    assert_eq!(cb.failure_count, 3);
    assert_eq!(cb.last_state_change_secs, t0 + 2);

    // Tick 4: Additional failure while Open should NOT re-trip (returns false)
    assert!(!cb.record_failure("error 4", t0 + 3));
    assert_eq!(cb.state, CircuitBreakerState::Open);

    // Cooldown check: before cooldown elapses (cooldown = 10s)
    assert_eq!(cb.check_state(t0 + 5), CircuitBreakerState::Open);
    assert_eq!(cb.check_state(t0 + 11), CircuitBreakerState::Open);

    // Exactly at cooldown expiration: t0 + 2 + 10 = t0 + 12 -> transitions to HalfOpen
    assert_eq!(cb.check_state(t0 + 12), CircuitBreakerState::HalfOpen);
    assert_eq!(cb.state, CircuitBreakerState::HalfOpen);
    assert_eq!(cb.last_state_change_secs, t0 + 12);

    // Path A: Success while in HalfOpen resets to Closed and zeroes failure_count
    cb.record_success();
    assert_eq!(cb.state, CircuitBreakerState::Closed);
    assert_eq!(cb.failure_count, 0);
    assert!(cb.last_failure_reason.is_none());

    // Path B: Trip again, cooldown to HalfOpen, then single failure immediately trips back to Open
    assert!(!cb.record_failure("err1", t0 + 20));
    assert!(!cb.record_failure("err2", t0 + 21));
    assert!(cb.record_failure("err3", t0 + 22));
    assert_eq!(cb.state, CircuitBreakerState::Open);

    // Transition to HalfOpen after cooldown
    assert_eq!(cb.check_state(t0 + 33), CircuitBreakerState::HalfOpen);

    // Single failure while HalfOpen immediately trips back to Open
    assert!(cb.record_failure("half_open_failure", t0 + 34));
    assert_eq!(cb.state, CircuitBreakerState::Open);
    assert_eq!(cb.last_state_change_secs, t0 + 34);
}

#[test]
fn test_circuit_breaker_nonconsecutive_failure_accumulation_bug() {
    // Challenge hypothesis: Does `record_success()` reset `failure_count` while in `Closed` state?
    // Docstring claims: "Circuit breaker tracking consecutive failures and automatic cooldown recovery."
    // If it tracks consecutive failures, a success should reset the failure counter.
    let mut cb = CircuitBreaker::new("provider_x", 3, 30);
    assert_eq!(cb.state, CircuitBreakerState::Closed);

    // 1 failure
    cb.record_failure("intermittent failure 1", 100);
    assert_eq!(cb.failure_count, 1);

    // 1,000 successful ticks
    for _ in 0..1000 {
        cb.record_success();
    }

    // Empirical Observation: in Closed state, failure_count was NOT reset!
    // The implementation only resets failure_count when state == HalfOpen:
    // `if self.state == CircuitBreakerState::HalfOpen { self.state = CircuitBreakerState::Closed; self.failure_count = 0; }`
    // Therefore, failures accumulate cumulatively across all time rather than consecutively!
    let count_after_successes = cb.failure_count;
    assert_eq!(
        count_after_successes, 1,
        "EMPIRICAL FINDING: record_success in Closed state leaves failure_count unreset (cumulative, not consecutive)"
    );

    // Another isolated failure
    cb.record_failure("intermittent failure 2", 200);
    assert_eq!(cb.failure_count, 2);

    // Another 1,000 successful ticks
    for _ in 0..1000 {
        cb.record_success();
    }
    assert_eq!(cb.failure_count, 2);

    // Third isolated failure trips the breaker despite 2,000 successful requests!
    let tripped = cb.record_failure("intermittent failure 3", 300);
    assert!(
        tripped,
        "Circuit breaker tripped after 3 non-consecutive failures spaced by thousands of successes"
    );
    assert_eq!(cb.state, CircuitBreakerState::Open);
}

// =========================================================================
// SECTION 2: PROVIDER FAILURE CASCADE & FAILOVER REROUTING
// =========================================================================

#[test]
fn test_provider_failure_cascade_and_complete_exhaustion() {
    let dir = test_temp_dir("cascade_exhaustion");
    let audit_path = dir.join("merkle_audit.json");

    let mut fallback_map = BTreeMap::new();
    fallback_map.insert("anthropic".to_string(), "openai".to_string());
    fallback_map.insert("openai".to_string(), "fallback_local".to_string());

    let config = GatewayConfig {
        tick_interval_ms: 10,
        circuit_breaker_threshold: 2,
        cooldown_secs: 100,
        audit_store_path: audit_path,
        fallback_provider_map: fallback_map,
    };

    let gateway = GatewayController::new(config);
    assert_eq!(gateway.active_provider(), "anthropic");

    // Phase 1: Fail primary provider "anthropic" (2 ticks > 1000ms)
    let mut telem1 = gateway.collect_live_telemetry();
    telem1
        .provider_latencies
        .insert("anthropic".to_string(), 1500.0);
    telem1
        .provider_latencies
        .insert("openai".to_string(), 120.0);

    gateway.evaluate_telemetry_tick(telem1.clone()).unwrap();
    let rep1_2 = gateway.evaluate_telemetry_tick(telem1).unwrap();

    // Anthropic tripped -> failover to openai
    assert_eq!(gateway.active_provider(), "openai");
    assert!(rep1_2.self_healing_actions.iter().any(|a| matches!(
        a,
        SelfHealingAction::RerouteProvider { from_provider, to_provider, .. }
        if from_provider == "anthropic" && to_provider == "openai"
    )));

    // Phase 2: Now fail secondary provider "openai" (2 ticks > 1000ms)
    let mut telem2 = gateway.collect_live_telemetry();
    telem2
        .provider_latencies
        .insert("anthropic".to_string(), 1500.0);
    telem2
        .provider_latencies
        .insert("openai".to_string(), 2500.0);
    telem2
        .provider_latencies
        .insert("fallback_local".to_string(), 50.0);

    gateway.evaluate_telemetry_tick(telem2.clone()).unwrap();
    let rep2_2 = gateway.evaluate_telemetry_tick(telem2).unwrap();

    // OpenAI tripped -> failover to fallback_local
    assert_eq!(gateway.active_provider(), "fallback_local");
    assert!(rep2_2.self_healing_actions.iter().any(|a| matches!(
        a,
        SelfHealingAction::RerouteProvider { from_provider, to_provider, .. }
        if from_provider == "openai" && to_provider == "fallback_local"
    )));

    // Phase 3: Now fail tertiary provider "fallback_local" as well (2 ticks > 1000ms)
    // All available providers are now failing!
    let mut telem3 = gateway.collect_live_telemetry();
    telem3
        .provider_latencies
        .insert("anthropic".to_string(), 1500.0);
    telem3
        .provider_latencies
        .insert("openai".to_string(), 2500.0);
    telem3
        .provider_latencies
        .insert("fallback_local".to_string(), 3500.0);

    gateway.evaluate_telemetry_tick(telem3.clone()).unwrap();
    let rep3_2 = gateway.evaluate_telemetry_tick(telem3).unwrap();

    // All three circuit breakers are now OPEN
    let cb_states = gateway.circuit_breaker_states();
    assert_eq!(cb_states.get("anthropic"), Some(&CircuitBreakerState::Open));
    assert_eq!(cb_states.get("openai"), Some(&CircuitBreakerState::Open));
    assert_eq!(
        cb_states.get("fallback_local"),
        Some(&CircuitBreakerState::Open)
    );

    // Notice: fallback_local attempted rerouting to itself because fallback_provider_map had no entry
    let has_self_reroute = rep3_2.self_healing_actions.iter().any(|a| {
        matches!(
            a,
            SelfHealingAction::RerouteProvider { from_provider, to_provider, .. }
            if from_provider == "fallback_local" && to_provider == "fallback_local"
        )
    });
    assert!(
        has_self_reroute,
        "When terminal fallback trips, it reroutes to fallback_local"
    );

    // Verify system did NOT crash or corrupt audit ledger:
    assert!(rep3_2.all_invariants_verified);
    assert_eq!(rep3_2.sequence_number, 5);
    assert!(rep3_2.receipt.verify_hash_integrity());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_cyclic_fallback_provider_map_behavior() {
    let dir = test_temp_dir("cyclic_fallback");
    let audit_path = dir.join("merkle_audit.json");

    // Cycle: A -> B and B -> A
    let mut fallback_map = BTreeMap::new();
    fallback_map.insert("provider_a".to_string(), "provider_b".to_string());
    fallback_map.insert("provider_b".to_string(), "provider_a".to_string());

    let config = GatewayConfig {
        tick_interval_ms: 10,
        circuit_breaker_threshold: 1, // Trip on first failure
        cooldown_secs: 100,
        audit_store_path: audit_path,
        fallback_provider_map: fallback_map,
    };

    let gateway = GatewayController::new(config);
    // Active provider starts as first key in map ("provider_a")
    assert_eq!(gateway.active_provider(), "provider_a");

    // Tick 1: provider_a fails -> reroutes to provider_b
    let mut t1 = ExecutionTelemetry::default();
    t1.provider_latencies
        .insert("provider_a".to_string(), 1200.0);
    t1.provider_latencies
        .insert("provider_b".to_string(), 100.0);

    let rep1 = gateway.evaluate_telemetry_tick(t1).unwrap();
    assert_eq!(gateway.active_provider(), "provider_b");
    assert!(rep1.self_healing_actions.iter().any(|a| matches!(
        a,
        SelfHealingAction::RerouteProvider { from_provider, to_provider, .. }
        if from_provider == "provider_a" && to_provider == "provider_b"
    )));

    // Tick 2: provider_b fails -> reroutes BACK to provider_a (which is STILL OPEN!)
    let mut t2 = ExecutionTelemetry::default();
    t2.provider_latencies
        .insert("provider_b".to_string(), 1300.0);

    let rep2 = gateway.evaluate_telemetry_tick(t2).unwrap();
    assert_eq!(gateway.active_provider(), "provider_a");
    assert!(rep2.self_healing_actions.iter().any(|a| matches!(
        a,
        SelfHealingAction::RerouteProvider { from_provider, to_provider, .. }
        if from_provider == "provider_b" && to_provider == "provider_a"
    )));

    // EMPIRICAL OBSERVATION: The gateway permits rerouting back to an already-open circuit breaker
    // if the static fallback_provider_map points to it.
    let states = gateway.circuit_breaker_states();
    assert_eq!(
        states.get("provider_a"),
        Some(&CircuitBreakerState::Open),
        "Active provider is provider_a whose breaker is Open"
    );

    let _ = fs::remove_dir_all(&dir);
}

// =========================================================================
// SECTION 3: BOUNDARY & ADVERSARIAL TELEMETRY INPUTS
// =========================================================================

#[test]
fn test_telemetry_extreme_latency_inputs_nan_infinity_negative() {
    let dir = test_temp_dir("telemetry_extreme_latencies");
    let audit_path = dir.join("merkle_audit.json");

    let config = GatewayConfig {
        tick_interval_ms: 10,
        circuit_breaker_threshold: 2,
        cooldown_secs: 10,
        audit_store_path: audit_path,
        fallback_provider_map: BTreeMap::new(),
    };

    let gateway = GatewayController::new(config);

    // 1. Negative latency (-500.0 ms)
    let mut telem_neg = gateway.collect_live_telemetry();
    telem_neg
        .provider_latencies
        .insert("anthropic".to_string(), -500.0);
    let rep_neg = gateway.evaluate_telemetry_tick(telem_neg).unwrap();
    assert!(rep_neg.all_invariants_verified);
    // Negative latency does not exceed 1000ms SLA, so considered nominal
    assert_eq!(
        gateway.circuit_breaker_states().get("anthropic"),
        Some(&CircuitBreakerState::Closed)
    );

    // 2. Zero latency (0.0 ms)
    let mut telem_zero = gateway.collect_live_telemetry();
    telem_zero
        .provider_latencies
        .insert("anthropic".to_string(), 0.0);
    let rep_zero = gateway.evaluate_telemetry_tick(telem_zero).unwrap();
    assert!(rep_zero.all_invariants_verified);

    // 3. NaN latency (f64::NAN)
    let mut telem_nan = gateway.collect_live_telemetry();
    telem_nan
        .provider_latencies
        .insert("anthropic".to_string(), f64::NAN);
    let rep_nan = gateway.evaluate_telemetry_tick(telem_nan).unwrap();
    assert!(rep_nan.all_invariants_verified);
    // In IEEE 754, `NaN > 1000.0` is false, so NaN is treated as success by the circuit breaker
    assert_eq!(
        gateway.circuit_breaker_states().get("anthropic"),
        Some(&CircuitBreakerState::Closed)
    );

    // 4. Positive Infinity latency (f64::INFINITY)
    let mut telem_inf = gateway.collect_live_telemetry();
    telem_inf
        .provider_latencies
        .insert("anthropic".to_string(), f64::INFINITY);
    // Tick 1 with Inf
    gateway.evaluate_telemetry_tick(telem_inf.clone()).unwrap();
    // Tick 2 with Inf -> trips breaker
    let rep_inf2 = gateway.evaluate_telemetry_tick(telem_inf).unwrap();
    assert_eq!(
        gateway.circuit_breaker_states().get("anthropic"),
        Some(&CircuitBreakerState::Open)
    );
    assert!(rep_inf2.self_healing_actions.iter().any(|a| matches!(
        a,
        SelfHealingAction::TripCircuitBreaker { target, .. } if target == "anthropic"
    )));

    // 5. Negative Infinity latency (f64::NEG_INFINITY)
    let mut telem_neginf = gateway.collect_live_telemetry();
    telem_neginf
        .provider_latencies
        .insert("openai".to_string(), f64::NEG_INFINITY);
    let rep_neginf = gateway.evaluate_telemetry_tick(telem_neginf).unwrap();
    assert!(rep_neginf.all_invariants_verified);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_telemetry_extreme_tool_error_rates() {
    let dir = test_temp_dir("telemetry_tool_error_rates");
    let audit_path = dir.join("merkle_audit.json");

    let config = GatewayConfig {
        tick_interval_ms: 10,
        circuit_breaker_threshold: 2,
        cooldown_secs: 10,
        audit_store_path: audit_path,
        fallback_provider_map: BTreeMap::new(),
    };

    let gateway = GatewayController::new(config);

    // 1. Tool error rate = 0.29 (just below 0.30 threshold -> nominal)
    let mut t1 = gateway.collect_live_telemetry();
    t1.tool_error_rates.insert("tool_a".to_string(), 0.29);
    let rep1 = gateway.evaluate_telemetry_tick(t1).unwrap();
    assert!(!rep1.self_healing_actions.iter().any(
        |a| matches!(a, SelfHealingAction::QuarantineTool { tool_id, .. } if tool_id == "tool_a")
    ));

    // 2. Tool error rate = 0.35 (above 0.30 but <= 0.50 -> failure recorded, not immediately quarantined on tick 1)
    let mut t2 = gateway.collect_live_telemetry();
    t2.tool_error_rates.insert("tool_b".to_string(), 0.35);
    let _rep2 = gateway.evaluate_telemetry_tick(t2.clone()).unwrap();
    // Circuit breaker threshold is 2, so tick 1 does not trip yet
    assert_eq!(
        gateway.circuit_breaker_states().get("tool_b"),
        Some(&CircuitBreakerState::Closed)
    );

    // Tick 2 for tool_b: failure count reaches 2 -> circuit breaker trips -> QuarantineTool
    let rep2_2 = gateway.evaluate_telemetry_tick(t2).unwrap();
    assert_eq!(
        gateway.circuit_breaker_states().get("tool_b"),
        Some(&CircuitBreakerState::Open)
    );
    assert!(rep2_2.self_healing_actions.iter().any(
        |a| matches!(a, SelfHealingAction::QuarantineTool { tool_id, .. } if tool_id == "tool_b")
    ));

    // 3. Tool error rate = 0.55 (> 0.50 -> immediately quarantined on tick 1)
    let mut t3 = gateway.collect_live_telemetry();
    t3.tool_error_rates.insert("tool_c".to_string(), 0.55);
    let rep3 = gateway.evaluate_telemetry_tick(t3).unwrap();
    assert!(rep3.self_healing_actions.iter().any(
        |a| matches!(a, SelfHealingAction::QuarantineTool { tool_id, .. } if tool_id == "tool_c")
    ));

    // 4. Extreme error rate > 1.0 (e.g. 5.0) -> must quarantine without crashing martingale
    let mut t4 = gateway.collect_live_telemetry();
    t4.tool_error_rates.insert("tool_d".to_string(), 5.0);
    let rep4 = gateway.evaluate_telemetry_tick(t4).unwrap();
    assert!(rep4.self_healing_actions.iter().any(
        |a| matches!(a, SelfHealingAction::QuarantineTool { tool_id, .. } if tool_id == "tool_d")
    ));

    // 5. Negative error rate (-0.5) and NaN error rate
    let mut t5 = gateway.collect_live_telemetry();
    t5.tool_error_rates.insert("tool_neg".to_string(), -0.5);
    t5.tool_error_rates.insert("tool_nan".to_string(), f64::NAN);
    let rep5 = gateway.evaluate_telemetry_tick(t5).unwrap();
    assert!(!rep5
        .self_healing_actions
        .iter()
        .any(|a| matches!(a, SelfHealingAction::QuarantineTool { tool_id, .. } if tool_id == "tool_neg" || tool_id == "tool_nan")));

    let _ = fs::remove_dir_all(&dir);
}

// =========================================================================
// SECTION 4: TROPICAL DAG TOPOLOGY & CYCLE REBALANCING
// =========================================================================

#[test]
fn test_tropical_dag_makespan_and_cycle_rebalancing() {
    let dir = test_temp_dir("dag_rebalance");
    let audit_path = dir.join("merkle_audit.json");

    let config = GatewayConfig {
        tick_interval_ms: 10,
        circuit_breaker_threshold: 3,
        cooldown_secs: 10,
        audit_store_path: audit_path,
        fallback_provider_map: BTreeMap::new(),
    };

    let gateway = GatewayController::new(config);

    // 1. DAG with massive critical-path makespan (> 10,000ms SLA)
    let mut telem_long = gateway.collect_live_telemetry();
    telem_long.dag_topologies = Some(DagTopologyTelemetry {
        node_count: 3,
        edge_latencies: vec![
            ("0".to_string(), "1".to_string(), 6000.0),
            ("1".to_string(), "2".to_string(), 5000.0), // Total critical path = 11,000ms > 10,000ms
        ],
        critical_path_makespan_ms: 11000.0,
    });

    let rep_long = gateway.evaluate_telemetry_tick(telem_long).unwrap();
    let has_truncate = rep_long.self_healing_actions.iter().any(|a| {
        matches!(
            a,
            SelfHealingAction::RebalanceDag { fallback_mode, .. }
            if fallback_mode == "truncate_critical_path"
        )
    });
    assert!(
        has_truncate,
        "Makespan > 10,000ms must trigger truncate_critical_path rebalance"
    );

    // 2. DAG with a directed cycle: 0 -> 1 -> 0 with positive edge weights
    let mut telem_cycle = gateway.collect_live_telemetry();
    telem_cycle.dag_topologies = Some(DagTopologyTelemetry {
        node_count: 2,
        edge_latencies: vec![
            ("0".to_string(), "1".to_string(), 10.0),
            ("1".to_string(), "0".to_string(), 20.0), // Positive cycle!
        ],
        critical_path_makespan_ms: 30.0,
    });

    let rep_cycle = gateway.evaluate_telemetry_tick(telem_cycle).unwrap();
    let has_break_cycle = rep_cycle.self_healing_actions.iter().any(|a| {
        matches!(
            a,
            SelfHealingAction::RebalanceDag { fallback_mode, .. }
            if fallback_mode == "break_cyclic_dependency"
        )
    });
    assert!(
        has_break_cycle,
        "Cycle detection must trigger break_cyclic_dependency rebalance"
    );

    // 3. Disconnected / trivial DAGs
    let mut telem_trivial = gateway.collect_live_telemetry();
    telem_trivial.dag_topologies = Some(DagTopologyTelemetry {
        node_count: 1, // < 2 nodes -> skipped safely
        edge_latencies: vec![],
        critical_path_makespan_ms: 0.0,
    });
    let rep_triv = gateway.evaluate_telemetry_tick(telem_trivial).unwrap();
    assert!(rep_triv.all_invariants_verified);

    let _ = fs::remove_dir_all(&dir);
}

// =========================================================================
// SECTION 5: HIGH-THROUGHPUT BURST INGESTION & AUDIT HASH CHAINING
// =========================================================================

#[test]
fn test_high_throughput_burst_ingestion_and_tamper_evident_chain() {
    let dir = test_temp_dir("burst_ingestion");
    let audit_path = dir.join("merkle_audit.json");

    let config = GatewayConfig {
        tick_interval_ms: 5,
        circuit_breaker_threshold: 3,
        cooldown_secs: 5,
        audit_store_path: audit_path.clone(),
        fallback_provider_map: BTreeMap::new(),
    };

    let gateway = GatewayController::new(config);

    // Burst ingest 30 ticks
    let count = 30;
    let mut reports = Vec::with_capacity(count);
    for i in 0..count {
        let mut telem = gateway.collect_live_telemetry();
        // Vary latencies slightly
        telem
            .provider_latencies
            .insert("anthropic".to_string(), 100.0 + (i as f64) * 5.0);
        let rep = gateway.evaluate_telemetry_tick(telem).unwrap();
        assert_eq!(rep.sequence_number, i as u64);
        assert!(rep.receipt.verify_hash_integrity());
        reports.push(rep);
    }

    // Verify hash chaining: receipt[i].prev_receipt_sha256 == receipt[i-1].receipt_sha256
    for i in 1..count {
        assert_eq!(
            reports[i].receipt.prev_receipt_sha256,
            Some(reports[i - 1].receipt.receipt_sha256.clone()),
            "Receipt at index {i} must cryptographically link to receipt at {prev}",
            prev = i - 1
        );
    }

    // Verify disk audit store contains all 30 entries
    let raw = fs::read(&audit_path).unwrap();
    let store: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    let dag_entries = store["dag_entries"].as_array().unwrap();
    assert_eq!(dag_entries.len(), count);

    // Verify Merkle tree proof generation for arbitrary leaves
    let mut log = MerkleDagAuditLog::new();
    for leaf in store["leaves"].as_array().unwrap() {
        let hex_str = leaf["payload_hex"].as_str().unwrap();
        let bytes = (0..hex_str.len())
            .step_by(2)
            .map(|j| u8::from_str_radix(&hex_str[j..j + 2], 16).unwrap())
            .collect::<Vec<u8>>();
        log.append_leaf(&bytes);
    }
    for entry in dag_entries {
        let hex_str = entry["payload_hex"].as_str().unwrap();
        let bytes = (0..hex_str.len())
            .step_by(2)
            .map(|j| u8::from_str_radix(&hex_str[j..j + 2], 16).unwrap())
            .collect::<Vec<u8>>();
        let parents: Vec<String> = entry["parent_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap().to_string())
            .collect();
        let ts = entry["timestamp_secs"].as_u64().unwrap();
        log.add_dag_node(&bytes, &parents, ts).unwrap();
    }

    // Verify inclusion proofs for leaves 0, 14, and 29
    for &idx in &[0, 14, 29] {
        let proof = log.generate_proof(idx).unwrap();
        assert!(proof.verify());
    }

    let _ = fs::remove_dir_all(&dir);
}

// =========================================================================
// SECTION 6: CONCURRENCY & THREAD-SAFETY STRESS HARNESS
// =========================================================================

#[test]
fn test_concurrent_multithreaded_telemetry_ingestion() {
    let dir = test_temp_dir("concurrent_ingestion");
    let audit_path = dir.join("merkle_audit.json");

    let config = GatewayConfig {
        tick_interval_ms: 1,
        circuit_breaker_threshold: 5,
        cooldown_secs: 5,
        audit_store_path: audit_path.clone(),
        fallback_provider_map: BTreeMap::new(),
    };

    let gateway = Arc::new(GatewayController::new(config));

    // Spawn 4 threads concurrently pushing telemetry
    let num_threads = 4;
    let ticks_per_thread = 5;
    let mut handles = Vec::new();

    for t_idx in 0..num_threads {
        let gw = Arc::clone(&gateway);
        let h = thread::spawn(move || {
            let mut results = Vec::new();
            for i in 0..ticks_per_thread {
                let mut telem = gw.collect_live_telemetry();
                telem
                    .provider_latencies
                    .insert(format!("provider_{t_idx}"), 100.0 + (i as f64));
                let rep = gw.evaluate_telemetry_tick(telem);
                results.push(rep);
            }
            results
        });
        handles.push(h);
    }

    let mut all_reports = Vec::new();
    for h in handles {
        let thread_res = h.join().expect("thread panicked");
        for r in thread_res {
            let rep = r.expect("tick evaluation failed under concurrent load");
            assert!(rep.all_invariants_verified);
            all_reports.push(rep);
        }
    }

    assert_eq!(all_reports.len(), num_threads * ticks_per_thread);

    // Verify disk audit store is valid JSON and has all entries
    assert!(audit_path.exists());
    let raw = fs::read(&audit_path).unwrap();
    let store: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(
        store["leaves"].as_array().unwrap().len(),
        num_threads * ticks_per_thread
    );

    let _ = fs::remove_dir_all(&dir);
}
