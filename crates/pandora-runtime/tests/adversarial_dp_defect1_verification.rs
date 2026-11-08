#![forbid(unsafe_code)]

//! Empirical Adversarial Verification Suite for Defect 1:
//! - Phase 9 Rényi Differential Privacy zero-budget tail bound discretization
//! - Double reservation commit detection via persistent historical lookup
//! - Adversary resistance against reservation lifecycle race conditions and boundary exploitation

use pandora_runtime::privacy_budget::{
    DistributedPrivacyTracker, PrivacyAccountantError, PrivacyExpenditure, RdpBudget, order_to_key,
};
use std::collections::BTreeMap;
use std::sync::{Arc, Barrier, Mutex};
use std::thread;

/// 1. EMPIRICAL TEST: Zero-budget tail bound discretization under extreme deltas
#[test]
fn test_empirical_zero_budget_tail_bound_invariants() {
    let zero_budget = RdpBudget::zero();
    assert!(zero_budget.is_empty(), "RdpBudget::zero() must be empty");
    assert_eq!(zero_budget.max_epsilon(), 0.0);

    // Test a wide spectrum of valid failure probabilities delta in (0, 1)
    let deltas = [
        1e-1, 1e-2, 1e-3, 1e-4, 1e-5, 1e-6, 1e-9, 1e-12, 1e-15, 1e-30, 1e-50, 1e-100, 1e-300,
    ];

    for &delta in &deltas {
        let eps = zero_budget
            .epsilon_at_delta(delta)
            .expect("valid delta must compute epsilon");
        assert_eq!(
            eps, 0.0,
            "zero budget must yield epsilon=0.0 at delta={delta}, but got {eps}"
        );

        let (opt_alpha, opt_eps) = zero_budget
            .optimal_order_for_delta(delta)
            .expect("valid delta must compute optimal order");
        assert_eq!(opt_eps, 0.0, "optimal eps must be 0.0 at delta={delta}");
        assert_eq!(
            opt_alpha, 2.0,
            "optimal alpha must default to 2.0 for empty budget"
        );

        let (approx_eps, approx_delta) = zero_budget
            .to_approx_dp(delta)
            .expect("to_approx_dp must succeed");
        assert_eq!(approx_eps, 0.0);
        assert_eq!(approx_delta, delta);
    }

    // Completely empty values map (no keys at all)
    let mut bare_empty = RdpBudget {
        values: BTreeMap::new(),
    };
    assert!(bare_empty.is_empty());
    assert_eq!(bare_empty.epsilon_at_delta(1e-5).unwrap(), 0.0);
    let (bare_alpha, bare_eps) = bare_empty.optimal_order_for_delta(1e-5).unwrap();
    assert_eq!(bare_alpha, 2.0);
    assert_eq!(bare_eps, 0.0);

    // Custom orders with zero values
    bare_empty.values.insert(order_to_key(1.5), 0.0);
    bare_empty.values.insert(order_to_key(3.0), 0.0);
    bare_empty.values.insert(order_to_key(12.0), 0.0);
    assert!(bare_empty.is_empty());
    assert_eq!(bare_empty.epsilon_at_delta(1e-5).unwrap(), 0.0);
    let (custom_alpha, custom_eps) = bare_empty.optimal_order_for_delta(1e-5).unwrap();
    assert_eq!(custom_alpha, 2.0);
    assert_eq!(custom_eps, 0.0);

    // Inject tiny non-zero epsilon (1e-12) to verify transition out of zero-budget
    bare_empty.values.insert(order_to_key(2.0), 1e-12);
    assert!(!bare_empty.is_empty());
    let non_zero_eps = bare_empty.epsilon_at_delta(1e-5).unwrap();
    assert!(
        non_zero_eps > 0.0,
        "non-empty budget must not return 0.0, got {non_zero_eps}"
    );
}

/// 2. EMPIRICAL TEST: Delta validation strictly takes precedence over is_empty check
#[test]
fn test_empirical_zero_budget_invalid_delta_rejection() {
    let zero_budget = RdpBudget::zero();

    // Invalid deltas must still return InvalidDelta error even though budget is zero
    let invalid_deltas = [
        0.0,
        -0.0001,
        -1.0,
        -100.0,
        1.0,
        1.0001,
        2.0,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ];

    for &bad_delta in &invalid_deltas {
        match zero_budget.epsilon_at_delta(bad_delta) {
            Err(PrivacyAccountantError::InvalidDelta(d)) => {
                if bad_delta.is_nan() {
                    assert!(d.is_nan());
                } else {
                    assert_eq!(d, bad_delta);
                }
            }
            other => panic!(
                "expected InvalidDelta for delta={bad_delta}, but got {:?}",
                other
            ),
        }

        match zero_budget.optimal_order_for_delta(bad_delta) {
            Err(PrivacyAccountantError::InvalidDelta(d)) => {
                if bad_delta.is_nan() {
                    assert!(d.is_nan());
                } else {
                    assert_eq!(d, bad_delta);
                }
            }
            other => panic!(
                "expected InvalidDelta in optimal_order for delta={bad_delta}, but got {:?}",
                other
            ),
        }
    }
}

/// 3. EMPIRICAL TEST: DistributedPrivacyTracker cold startup under tiny budget limits
#[test]
fn test_empirical_tracker_startup_with_micro_budget() {
    // Ultra-tight privacy budget: max_eps = 1e-6 (1 micro-epsilon)
    let micro_limit = 1e-6;
    let target_delta = 1e-5;
    let mut tracker = DistributedPrivacyTracker::new(micro_limit, target_delta)
        .expect("tracker creation should succeed");

    // Invariant 1: initial remaining epsilon equals full max_eps
    let remaining = tracker.remaining_epsilon().unwrap();
    assert_eq!(
        remaining, micro_limit,
        "initial remaining budget must equal micro_limit"
    );

    // Invariant 2: initial utilization is exactly 0.0
    let util = tracker.budget_utilization().unwrap();
    assert_eq!(util, 0.0, "initial utilization must be 0%");

    // Invariant 3: current approx dp has epsilon 0.0
    let (curr_eps, curr_delta) = tracker.current_approx_dp().unwrap();
    assert_eq!(curr_eps, 0.0);
    assert_eq!(curr_delta, target_delta);

    // Invariant 4: zero budget query is cleanly admitted
    let zero_q = RdpBudget::zero();
    assert!(
        tracker.can_admit_query(&zero_q).unwrap(),
        "zero budget query must be admitted under micro limit"
    );

    // Invariant 5: non-zero query that exceeds micro limit is rejected
    let real_q = RdpBudget::from_gaussian(0.1, 10.0).unwrap(); // eps ~ 0.001 >> 1e-6
    assert!(
        !tracker.can_admit_query(&real_q).unwrap(),
        "query exceeding micro budget must be rejected"
    );

    // Invariant 6: Reserving and committing zero budget does not deplete micro budget
    let res_id = tracker
        .reserve("agent_zero", "query_0", &zero_q)
        .expect("reserving zero budget query must succeed");
    assert_eq!(tracker.active_reservations.len(), 1);

    let committed_eps = tracker
        .commit_reservation(&res_id)
        .expect("committing zero budget query must succeed");
    assert_eq!(
        committed_eps, 0.0,
        "committed global epsilon should remain 0.0"
    );
    assert_eq!(tracker.active_reservations.len(), 0);
    assert_eq!(tracker.ledger.len(), 1);
    assert_eq!(tracker.committed_reservations.len(), 1);

    // Micro budget is still completely intact!
    assert_eq!(tracker.remaining_epsilon().unwrap(), micro_limit);
    assert_eq!(tracker.budget_utilization().unwrap(), 0.0);
}

/// 4. EMPIRICAL TEST: Double reservation commit and lifecycle attack matrix
#[test]
fn test_empirical_double_commit_and_lifecycle_attack_matrix() {
    let mut tracker = DistributedPrivacyTracker::new(5.0, 1e-5).unwrap();
    let query_budget = RdpBudget::from_gaussian(0.5, 4.0).unwrap();

    let res_id = tracker
        .reserve("agent_matrix", "q_lifecycle", &query_budget)
        .unwrap();

    // 1st commit: OK
    let eps1 = tracker.commit_reservation(&res_id).unwrap();
    assert!(eps1 > 0.0);
    assert!(tracker.committed_reservations.contains(&res_id));

    // 2nd commit: MUST fail with ReservationAlreadyCommitted (Defect 1 verification)
    let err2 = tracker.commit_reservation(&res_id);
    match err2 {
        Err(PrivacyAccountantError::ReservationAlreadyCommitted(id)) => {
            assert_eq!(id, res_id);
        }
        other => panic!(
            "expected ReservationAlreadyCommitted on 2nd commit, got {:?}",
            other
        ),
    }

    // 3rd commit: MUST ALSO fail with ReservationAlreadyCommitted
    let err3 = tracker.commit_reservation(&res_id);
    match err3 {
        Err(PrivacyAccountantError::ReservationAlreadyCommitted(id)) => {
            assert_eq!(id, res_id);
        }
        other => panic!(
            "expected ReservationAlreadyCommitted on 3rd commit, got {:?}",
            other
        ),
    }

    // Attempting to release an already committed reservation must fail with ReservationNotFound
    let release_after_commit = tracker.release_reservation(&res_id);
    match release_after_commit {
        Err(PrivacyAccountantError::ReservationNotFound(id)) => {
            assert_eq!(id, res_id);
        }
        other => panic!(
            "expected ReservationNotFound when releasing committed reservation, got {:?}",
            other
        ),
    }

    // Ledger count must remain strictly 1 (no double spend)
    assert_eq!(tracker.ledger.len(), 1);
}

/// 5. EMPIRICAL TEST: Release then commit lifecycle ordering
#[test]
fn test_empirical_release_then_commit_lifecycle_ordering() {
    let mut tracker = DistributedPrivacyTracker::new(5.0, 1e-5).unwrap();
    let query_budget = RdpBudget::from_gaussian(0.5, 4.0).unwrap();

    let res_id = tracker
        .reserve("agent_order", "q_order", &query_budget)
        .unwrap();

    // Release first
    tracker
        .release_reservation(&res_id)
        .expect("release must succeed");
    assert_eq!(tracker.active_reservations.len(), 0);
    assert!(!tracker.committed_reservations.contains(&res_id));

    // Attempt to commit after release -> must fail with ReservationNotFound
    let commit_after_release = tracker.commit_reservation(&res_id);
    match commit_after_release {
        Err(PrivacyAccountantError::ReservationNotFound(id)) => {
            assert_eq!(id, res_id);
        }
        other => panic!(
            "expected ReservationNotFound on commit after release, got {:?}",
            other
        ),
    }

    // 2nd release attempt must fail with ReservationNotFound
    let double_release = tracker.release_reservation(&res_id);
    match double_release {
        Err(PrivacyAccountantError::ReservationNotFound(id)) => {
            assert_eq!(id, res_id);
        }
        other => panic!(
            "expected ReservationNotFound on double release, got {:?}",
            other
        ),
    }
}

/// 6. EMPIRICAL TEST: Concurrent multi-threaded double-commit race condition.
///
/// Verifies that when N threads race to commit the EXACT SAME reservation ID,
/// exactly ONE thread succeeds and (N - 1) threads receive ReservationAlreadyCommitted.
#[test]
fn test_empirical_concurrent_double_commit_race() {
    let global_max = 10.0;
    let target_delta = 1e-5;
    let tracker = Arc::new(Mutex::new(
        DistributedPrivacyTracker::new(global_max, target_delta).unwrap(),
    ));

    let query_budget = RdpBudget::from_gaussian(0.5, 3.0).unwrap();
    let single_query_eps = query_budget.epsilon_at_delta(target_delta).unwrap();

    let res_id = {
        let mut lock = tracker.lock().unwrap();
        lock.reserve("race_agent", "q_race", &query_budget).unwrap()
    };

    let thread_count = 16;
    let barrier = Arc::new(Barrier::new(thread_count));
    let mut handles = Vec::new();

    for _ in 0..thread_count {
        let tracker_clone = Arc::clone(&tracker);
        let barrier_clone = Arc::clone(&barrier);
        let res_id_clone = res_id.clone();

        handles.push(thread::spawn(move || {
            // Synchronize all threads at the barrier so they burst simultaneously
            barrier_clone.wait();

            let mut lock = tracker_clone.lock().unwrap();
            lock.commit_reservation(&res_id_clone)
        }));
    }

    let mut ok_count = 0;
    let mut already_committed_count = 0;
    let mut other_err_count = 0;

    for h in handles {
        let res = h.join().expect("thread join must not panic");
        match res {
            Ok(_) => ok_count += 1,
            Err(PrivacyAccountantError::ReservationAlreadyCommitted(id)) => {
                assert_eq!(id, res_id);
                already_committed_count += 1;
            }
            Err(other) => {
                eprintln!("Unexpected error: {:?}", other);
                other_err_count += 1;
            }
        }
    }

    // Invariant 1: Exactly 1 thread successfully committed
    assert_eq!(
        ok_count, 1,
        "exactly 1 thread must successfully commit the reservation"
    );

    // Invariant 2: Exactly (thread_count - 1) threads got ReservationAlreadyCommitted
    assert_eq!(
        already_committed_count,
        thread_count - 1,
        "all competing threads must receive ReservationAlreadyCommitted"
    );

    // Invariant 3: Zero unexpected errors
    assert_eq!(other_err_count, 0, "no unexpected errors during race");

    // Invariant 4: Cumulative budget was only incremented ONCE
    let final_tracker = tracker.lock().unwrap();
    let (final_eps, _) = final_tracker.current_approx_dp().unwrap();
    let diff = (final_eps - single_query_eps).abs();
    assert!(
        diff < 1e-9,
        "cumulative budget must only be incremented once (expected {single_query_eps}, got {final_eps})"
    );
    assert_eq!(final_tracker.ledger.len(), 1);
    assert_eq!(final_tracker.committed_reservations.len(), 1);
}

/// 7. EMPIRICAL TEST: High-volume reservation saturation and memory bounding
#[test]
fn test_empirical_high_volume_reservation_saturation() {
    let mut tracker = DistributedPrivacyTracker::new(100.0, 1e-5).unwrap();
    let small_budget = RdpBudget::from_gaussian(0.01, 10.0).unwrap(); // tiny epsilon

    let num_reservations = 500;
    let mut ids = Vec::with_capacity(num_reservations);

    // Create 500 reservations
    for i in 0..num_reservations {
        let id = tracker
            .reserve(
                &format!("agent_{}", i % 5),
                &format!("q_{i}"),
                &small_budget,
            )
            .unwrap();
        ids.push(id);
    }

    assert_eq!(tracker.active_reservations.len(), num_reservations);
    assert_eq!(tracker.committed_reservations.len(), 0);

    // Commit all 500 reservations
    for id in &ids {
        assert!(tracker.commit_reservation(id).is_ok());
    }

    assert_eq!(tracker.active_reservations.len(), 0);
    assert_eq!(tracker.committed_reservations.len(), num_reservations);
    assert_eq!(tracker.ledger.len(), num_reservations);

    // Adversarial replay: attempt to re-commit every single ID
    for id in &ids {
        let replay_res = tracker.commit_reservation(id);
        match replay_res {
            Err(PrivacyAccountantError::ReservationAlreadyCommitted(ref bad_id)) => {
                assert_eq!(bad_id, id);
            }
            other => panic!(
                "expected ReservationAlreadyCommitted for replay of {id}, got {:?}",
                other
            ),
        }
    }

    // Ledger and committed sets are strictly unchanged after replay attempts
    assert_eq!(tracker.ledger.len(), num_reservations);
    assert_eq!(tracker.committed_reservations.len(), num_reservations);
}

/// 8. EMPIRICAL TEST: Zero-budget composition and agent allocation consistency
#[test]
fn test_empirical_zero_budget_composition_and_agent_consistency() {
    let mut tracker = DistributedPrivacyTracker::new(2.0, 1e-5).unwrap();
    let zero_budget = RdpBudget::zero();

    // 10 consecutive zero-budget expenditures directly recorded
    for i in 0..10 {
        let exp = PrivacyExpenditure {
            transaction_id: format!("zero_tx_{i}"),
            agent_id: "agent_zero_direct".to_string(),
            query_id: format!("zero_q_{i}"),
            mechanism_name: "zero_cost".to_string(),
            sensitivity: 0.0,
            noise_parameter: 1.0,
            rdp_expenditure: zero_budget.clone(),
            timestamp_ms: i as u64,
        };

        let eps = tracker.record_expenditure(exp).unwrap();
        assert_eq!(
            eps, 0.0,
            "recording zero expenditure must keep global eps at 0.0"
        );
    }

    // Global budget utilization is still 0
    assert_eq!(tracker.budget_utilization().unwrap(), 0.0);
    assert_eq!(tracker.remaining_epsilon().unwrap(), 2.0);

    // Agent's allocated budget is also zero
    let agent_budget = tracker.agent_budget("agent_zero_direct").unwrap();
    assert!(agent_budget.is_empty());
    assert_eq!(agent_budget.epsilon_at_delta(1e-5).unwrap(), 0.0);
}
