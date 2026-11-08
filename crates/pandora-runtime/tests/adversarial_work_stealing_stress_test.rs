//! Phase 11: Adversarial Empirical Integration & Stress Test Suite
//! Dynamic Adaptive Work-Stealing Task Scheduler with Blumofe-Leiserson Bounds.
//!
//! Evaluates:
//! 1. Adversarial Deep Linear Chains: T_inf ~ T_1, proving speedup <= 1.0 bound regardless of P
//! 2. Wide Embarrassingly Parallel Trees/Forks: T_inf << T_1, near-linear speedup scaling
//! 3. Adversarial Cyclic Topologies: Comprehensive rejection matrix
//! 4. Disconnected DAG Components & Isolated Sub-Tasks: Correct completeness and makespan
//! 5. Deque Capacity Limit & Memory Bounds: DequeCapacityExceeded error and S_P <= P * C_max
//! 6. Adaptive Exponential Backoff Dynamics: Exponential delay progression and reset
//! 7. True Concurrent Multi-Threaded Stealing: Heavy thread contention under pure safe Rust
//! 8. Randomized DAG Fuzzing: Verification of Blumofe-Leiserson Theorem 21 across diverse random topologies
//! 9. Boundary Conditions & Input Validation: 0 workers, 0 capacity, empty DAG, single task, oversubscribed workers

#![forbid(unsafe_code)]

use pandora_runtime::bandit_router::SafePrng;
use pandora_runtime::work_stealing::{
    AdaptiveBackoff, BoundedTaskDeque, SchedulerError, StealableTask, StealableTaskDag,
    WorkStealingScheduler,
};
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

// ============================================================================
// PART 1: ADVERSARIAL DEEP LINEAR DEPENDENCY CHAINS
// ============================================================================

#[test]
fn test_adversarial_deep_linear_dependency_chain_speedup_ceiling() {
    // Construct a deep linear chain: v_0 -> v_1 -> ... -> v_127 (128 tasks)
    // T_1 = 128 * 10 = 1280
    // T_inf = 128 * 10 = 1280
    // Theoretical maximum parallelism P_max = T_1 / T_inf = 1.0.
    // Speedup MUST be strictly bounded by 1.0 regardless of processor count P.
    let chain_len = 128;
    let node_weight = 10;
    let mut dag = StealableTaskDag::new();

    for i in 0..chain_len {
        dag.add_task(StealableTask::new(
            format!("chain-node-{i}"),
            format!("Linear Chain Step {i}"),
            node_weight,
        ));
    }

    for i in 0..(chain_len - 1) {
        dag.add_dependency(&format!("chain-node-{i}"), &format!("chain-node-{}", i + 1))
            .expect("Sequential edges must be cycle-free");
    }

    let t1 = dag.total_work();
    assert_eq!(t1, (chain_len as u64) * node_weight);

    let t_inf = dag
        .critical_path_span()
        .expect("Critical path must compute cleanly");
    assert_eq!(t_inf, t1);

    // Evaluate across worker pool sizes: P in {1, 2, 4, 8, 16}
    for p in [1, 2, 4, 8, 16] {
        let scheduler = WorkStealingScheduler::new(p, 256).expect("Scheduler instantiation failed");
        let telemetry = scheduler.execute_dag(&dag).expect("DAG execution failed");

        assert_eq!(telemetry.total_work_t1, t1);
        assert_eq!(telemetry.critical_path_t_inf, t_inf);
        assert_eq!(telemetry.worker_count, p);

        // Lower bound check: T_P >= T_inf
        assert!(
            telemetry.actual_makespan_tp >= t_inf,
            "Makespan {} cannot be faster than critical path span {}",
            telemetry.actual_makespan_tp,
            t_inf
        );

        // Linear chain invariant: Makespan must equal sequential work T_1 (no speedup possible)
        assert_eq!(
            telemetry.actual_makespan_tp, t1,
            "Linear chain makespan on P={} workers must equal sequential work T_1={}",
            p, t1
        );

        // Speedup upper bound: S_P <= 1.0 (speedup cannot exceed 1.0 on a pure chain)
        assert!(
            telemetry.observed_speedup <= 1.0 + 1e-6,
            "Observed speedup {} on deep linear chain exceeded 1.0",
            telemetry.observed_speedup
        );

        // Theorem 21 Blumofe-Leiserson bound: T_P <= T_1/P + c_inf * T_inf
        assert!(
            telemetry.bl_bound_satisfied,
            "Blumofe-Leiserson bound violated: T_{p}={} exceeds theoretical bound {}",
            telemetry.actual_makespan_tp, telemetry.theoretical_bl_bound
        );

        // Peak tasks in flight invariant: on a linear chain, at most 1 task is ready at any time
        assert!(
            telemetry.peak_tasks_in_flight <= 2,
            "Linear chain peak in-flight tasks {} exceeded expected bound (<= 2)",
            telemetry.peak_tasks_in_flight
        );

        // Under P > 1, starved workers should attempt steals against the sole busy worker
        if p > 1 {
            assert!(
                telemetry.total_steals_attempted > 0,
                "Idle workers must attempt steals on P={}",
                p
            );
        }
    }
}

// ============================================================================
// PART 2: WIDE EMBARRASSINGLY PARALLEL FORK-JOIN GRAPHS
// ============================================================================

#[test]
fn test_adversarial_wide_embarrassingly_parallel_fork_join_scaling() {
    // Construct a wide fork-join DAG:
    // Root (weight 2) -> 256 independent parallel branches (weight 25 each) -> Join (weight 2)
    // T_1 = 2 + 256 * 25 + 2 = 6404
    // T_inf = 2 + 25 + 2 = 29
    // Available parallelism P_max = 6404 / 29 ~ 220.8
    let branch_count = 256;
    let branch_weight = 25;
    let mut dag = StealableTaskDag::new();

    dag.add_task(StealableTask::new("root", "Root Dispatcher", 2));
    dag.add_task(StealableTask::new("join", "Join Collector", 2));

    for i in 0..branch_count {
        let branch_id = format!("branch-{i}");
        dag.add_task(StealableTask::new(
            &branch_id,
            "Parallel Worker",
            branch_weight,
        ));
        dag.add_dependency("root", &branch_id).unwrap();
        dag.add_dependency(&branch_id, "join").unwrap();
    }

    let t1 = dag.total_work();
    assert_eq!(t1, 2 + (branch_count as u64) * branch_weight + 2);

    let t_inf = dag.critical_path_span().unwrap();
    assert_eq!(t_inf, 2 + branch_weight + 2);

    let mut previous_makespan = u64::MAX;

    let mut violations = Vec::new();

    // Evaluate across worker pool sizes: P in {1, 2, 4, 8, 16}
    for p in [1, 2, 4, 8, 16] {
        let scheduler = WorkStealingScheduler::new(p, 512).unwrap();
        let telemetry = scheduler.execute_dag(&dag).unwrap();

        assert_eq!(telemetry.total_work_t1, t1);
        assert_eq!(telemetry.critical_path_t_inf, t_inf);

        println!(
            "P={:2}: T_P={:4}, Bound={:6.1}, T1/P={:6.1}, Speedup={:5.2}, Eff={:5.2}, StealsAtt={:4}, StealsSucc={:4}, BL_Satisfied={}",
            p,
            telemetry.actual_makespan_tp,
            telemetry.theoretical_bl_bound,
            (t1 as f64) / (p as f64),
            telemetry.observed_speedup,
            telemetry.parallel_efficiency,
            telemetry.total_steals_attempted,
            telemetry.total_steals_succeeded,
            telemetry.bl_bound_satisfied
        );

        if !telemetry.bl_bound_satisfied {
            violations.push(format!(
                "P={:2}: T_P={} exceeded BL bound {:.1} (T1/P={:.1}, Speedup={:.2}, Eff={:.2})",
                p,
                telemetry.actual_makespan_tp,
                telemetry.theoretical_bl_bound,
                (t1 as f64) / (p as f64),
                telemetry.observed_speedup,
                telemetry.parallel_efficiency
            ));
        }

        // Monotonic scaling check: makespan should decrease as P increases
        if telemetry.actual_makespan_tp >= previous_makespan {
            violations.push(format!(
                "P={:2}: Scaling stalled! Makespan {} did not improve over previous makespan {}",
                p, telemetry.actual_makespan_tp, previous_makespan
            ));
        }
        previous_makespan = telemetry.actual_makespan_tp;

        // Lower bound: T_P >= T_inf
        assert!(
            telemetry.actual_makespan_tp >= t_inf,
            "Makespan {} cannot beat critical path span {}",
            telemetry.actual_makespan_tp,
            t_inf
        );

        if p == 1 {
            assert_eq!(telemetry.actual_makespan_tp, t1);
            assert_eq!(telemetry.total_steals_attempted, 0);
            assert!((telemetry.observed_speedup - 1.0).abs() < 1e-4);
        }
    }

    assert!(
        violations.is_empty(),
        "Blumofe-Leiserson empirical violations detected across wide fork-join topology:\n{}",
        violations.join("\n")
    );
}

// ============================================================================
// PART 3: ADVERSARIAL CYCLIC TOPOLOGIES REJECTION MATRIX
// ============================================================================

#[test]
fn test_adversarial_cyclic_topologies_rejection_matrix() {
    // Attack 1: Self-loop via add_dependency
    {
        let mut dag = StealableTaskDag::new();
        dag.add_task(StealableTask::new("solo", "Self-dependent", 10));
        let err = dag.add_dependency("solo", "solo");
        assert!(
            matches!(err, Err(SchedulerError::CyclicDependency(_))),
            "Self-dependency must be rejected with CyclicDependency"
        );
    }

    // Attack 2: 2-node direct mutual cycle
    {
        let mut dag = StealableTaskDag::new();
        dag.add_task(StealableTask::new("a", "Node A", 10));
        dag.add_task(StealableTask::new("b", "Node B", 10));
        assert!(dag.add_dependency("a", "b").is_ok());
        let err = dag.add_dependency("b", "a");
        assert!(
            matches!(err, Err(SchedulerError::CyclicDependency(_))),
            "2-node mutual cycle must be rejected"
        );
    }

    // Attack 3: 5-node indirect cycle (A -> B -> C -> D -> E -> A)
    {
        let mut dag = StealableTaskDag::new();
        let nodes = ["a", "b", "c", "d", "e"];
        for n in &nodes {
            dag.add_task(StealableTask::new(*n, format!("Node {n}"), 10));
        }
        for i in 0..4 {
            assert!(dag.add_dependency(nodes[i], nodes[i + 1]).is_ok());
        }
        let err = dag.add_dependency("e", "a");
        assert!(
            matches!(err, Err(SchedulerError::CyclicDependency(_))),
            "5-node indirect cycle must be rejected"
        );
    }

    // Attack 4: Embedded cycle within a larger 20-node DAG
    {
        let mut dag = StealableTaskDag::new();
        for i in 0..20 {
            dag.add_task(StealableTask::new(format!("n{i}"), "Step", 5));
        }
        for i in 0..19 {
            assert!(
                dag.add_dependency(&format!("n{i}"), &format!("n{}", i + 1))
                    .is_ok()
            );
        }
        // Add back-edge from n15 to n5 (creating a cycle)
        let err = dag.add_dependency("n15", "n5");
        assert!(
            matches!(err, Err(SchedulerError::CyclicDependency(_))),
            "Embedded back-edge cycle must be rejected"
        );
    }

    // Attack 5: Disconnected cycle (Valid component + isolated cycle)
    {
        let mut dag = StealableTaskDag::new();
        // Valid pipeline
        dag.add_task(StealableTask::new("valid-1", "V1", 10));
        dag.add_task(StealableTask::new("valid-2", "V2", 10));
        assert!(dag.add_dependency("valid-1", "valid-2").is_ok());

        // Isolated cyclic pair
        dag.add_task(StealableTask::new("cyc-1", "C1", 10));
        dag.add_task(StealableTask::new("cyc-2", "C2", 10));
        assert!(dag.add_dependency("cyc-1", "cyc-2").is_ok());
        let err = dag.add_dependency("cyc-2", "cyc-1");
        assert!(
            matches!(err, Err(SchedulerError::CyclicDependency(_))),
            "Cycle in disconnected component must be rejected"
        );
    }

    // Attack 6: Pre-populated cyclic dependencies injected directly via StealableTask
    // bypassing add_dependency, then validated via topological_sort, critical_path_span, and execute_dag
    {
        let mut dag = StealableTaskDag::new();
        let task_x = StealableTask::new("x", "X", 10).with_dependencies(vec!["z".to_string()]);
        let task_y = StealableTask::new("y", "Y", 10).with_dependencies(vec!["x".to_string()]);
        let task_z = StealableTask::new("z", "Z", 10).with_dependencies(vec!["y".to_string()]);

        dag.add_task(task_x);
        dag.add_task(task_y);
        dag.add_task(task_z);

        // topological_sort must detect the pre-populated cycle
        let sort_result = dag.topological_sort();
        assert!(
            matches!(sort_result, Err(SchedulerError::CyclicDependency(_))),
            "topological_sort must catch pre-populated cycle"
        );

        // critical_path_span must catch pre-populated cycle
        let span_result = dag.critical_path_span();
        assert!(
            matches!(span_result, Err(SchedulerError::CyclicDependency(_))),
            "critical_path_span must catch pre-populated cycle"
        );

        // execute_dag must catch pre-populated cycle and refuse execution
        let scheduler = WorkStealingScheduler::new(4, 64).unwrap();
        let exec_result = scheduler.execute_dag(&dag);
        assert!(
            matches!(exec_result, Err(SchedulerError::CyclicDependency(_))),
            "execute_dag must catch pre-populated cycle"
        );
    }
}

// ============================================================================
// PART 4: DISCONNECTED DAG COMPONENTS & ISOLATED SUB-TASKS
// ============================================================================

#[test]
fn test_adversarial_disconnected_dag_components_and_isolated_tasks() {
    let mut dag = StealableTaskDag::new();

    // Component 1: Linear chain A (4 tasks, weight 10 each = 40)
    for i in 0..4 {
        dag.add_task(StealableTask::new(format!("pipe_a_{i}"), "Pipeline A", 10));
    }
    for i in 0..3 {
        dag.add_dependency(&format!("pipe_a_{i}"), &format!("pipe_a_{}", i + 1))
            .unwrap();
    }

    // Component 2: Linear chain B (3 tasks, weight 20 each = 60)
    for i in 0..3 {
        dag.add_task(StealableTask::new(format!("pipe_b_{i}"), "Pipeline B", 20));
    }
    for i in 0..2 {
        dag.add_dependency(&format!("pipe_b_{i}"), &format!("pipe_b_{}", i + 1))
            .unwrap();
    }

    // Component 3: Diamond fork-join C (4 tasks: root 10, left 15, right 20, join 10 = 55)
    dag.add_task(StealableTask::new("c_root", "C Root", 10));
    dag.add_task(StealableTask::new("c_left", "C Left", 15));
    dag.add_task(StealableTask::new("c_right", "C Right", 20));
    dag.add_task(StealableTask::new("c_join", "C Join", 10));
    dag.add_dependency("c_root", "c_left").unwrap();
    dag.add_dependency("c_root", "c_right").unwrap();
    dag.add_dependency("c_left", "c_join").unwrap();
    dag.add_dependency("c_right", "c_join").unwrap();

    // Component 4, 5, 6: Completely isolated tasks
    dag.add_task(StealableTask::new("iso_1", "Isolated 1", 30));
    dag.add_task(StealableTask::new("iso_2", "Isolated 2", 25));
    dag.add_task(StealableTask::new("iso_3", "Isolated 3", 45));

    // Total tasks = 4 + 3 + 4 + 3 = 14 tasks
    let total_tasks_count = dag.tasks().len();
    assert_eq!(total_tasks_count, 14);

    let t1 = dag.total_work();
    // T1 = 40 + 60 + 55 + 30 + 25 + 45 = 255
    assert_eq!(t1, 255);

    let t_inf = dag.critical_path_span().unwrap();
    // Longest single path is pipe_b (60)
    assert_eq!(t_inf, 60);

    for p in [1, 2, 4, 8] {
        let scheduler = WorkStealingScheduler::new(p, 128).unwrap();
        let telemetry = scheduler.execute_dag(&dag).unwrap();

        assert_eq!(telemetry.total_work_t1, t1);
        assert_eq!(telemetry.critical_path_t_inf, t_inf);

        // Invariant: all 14 tasks must complete without deadlock or abandonment
        assert!(telemetry.actual_makespan_tp >= t_inf);
        assert!(telemetry.bl_bound_satisfied);

        if p == 1 {
            assert_eq!(telemetry.actual_makespan_tp, t1);
        } else {
            // Speedup should be achieved across multiple disjoint components
            assert!(telemetry.observed_speedup > 1.0);
        }
    }
}

// ============================================================================
// PART 5: DEQUE CAPACITY OVERFLOW & BOUNDED MEMORY DEFENSE
// ============================================================================

#[test]
fn test_adversarial_deque_capacity_overflow_bounded_memory_defense() {
    // Test A: Standalone BoundedTaskDeque exact boundary checks
    let cap = 5;
    let deque = BoundedTaskDeque::new(cap);
    assert_eq!(deque.capacity(), cap);
    assert_eq!(deque.len(), 0);

    // Push up to capacity limit
    for i in 0..cap {
        let task = StealableTask::new(format!("t{i}"), "Task", 10);
        assert!(deque.push_bottom(task).is_ok());
    }
    assert_eq!(deque.len(), cap);

    // Overflow push must be rejected gracefully
    let overflow_task = StealableTask::new("overflow", "Overflow", 10);
    let err = deque.push_bottom(overflow_task);
    assert_eq!(
        err,
        Err(SchedulerError::DequeCapacityExceeded { capacity: cap })
    );
    assert_eq!(deque.len(), cap);

    // Popping frees a slot
    let popped = deque.pop_bottom().unwrap();
    assert_eq!(popped.id, "t4"); // LIFO bottom
    assert_eq!(deque.len(), 4);

    // Now push succeeds
    let retry_task = StealableTask::new("retry", "Retry", 10);
    assert!(deque.push_bottom(retry_task).is_ok());
    assert_eq!(deque.len(), cap);

    // Remote steal FIFO from top
    let stolen = deque.steal_top().unwrap();
    assert_eq!(stolen.id, "t0"); // FIFO top

    // Test B: Scheduler DAG execution with fan-out exceeding deque capacity
    // Root spawns 16 children, but worker deques have capacity limit C_max = 6.
    // When root finishes, worker 0 attempts to push 16 children onto its deque.
    // The scheduler must cleanly halt with DequeCapacityExceeded error.
    let small_cap = 6;
    let fanout = 16;
    let mut dag = StealableTaskDag::new();

    dag.add_task(StealableTask::new("root", "Root", 1));
    for i in 0..fanout {
        let child_id = format!("child_{i}");
        dag.add_task(StealableTask::new(&child_id, "Child", 10));
        dag.add_dependency("root", &child_id).unwrap();
    }

    let scheduler = WorkStealingScheduler::new(2, small_cap).unwrap();
    let result = scheduler.execute_dag(&dag);

    assert_eq!(
        result.err(),
        Some(SchedulerError::DequeCapacityExceeded {
            capacity: small_cap
        }),
        "Scheduler must halt gracefully with DequeCapacityExceeded when fanout exceeds capacity"
    );

    // Test C: Initial ready task burst exceeding capacity
    // 16 independent tasks distributed to 2 workers (8 each), with deque capacity 4
    let mut burst_dag = StealableTaskDag::new();
    for i in 0..16 {
        burst_dag.add_task(StealableTask::new(format!("burst_{i}"), "Burst", 5));
    }
    let burst_scheduler = WorkStealingScheduler::new(2, 4).unwrap();
    let burst_result = burst_scheduler.execute_dag(&burst_dag);
    assert_eq!(
        burst_result.err(),
        Some(SchedulerError::DequeCapacityExceeded { capacity: 4 }),
        "Initial burst exceeding per-worker capacity must be rejected"
    );
}

// ============================================================================
// PART 6: ADAPTIVE EXPONENTIAL BACKOFF DYNAMICS & CONTENTION MODELING
// ============================================================================

#[test]
fn test_adversarial_adaptive_exponential_backoff_stress() {
    let base_us = 50;
    let max_us = 2000;
    let mut backoff = AdaptiveBackoff::new(base_us, max_us);

    // Verify progression: 50 -> 100 -> 200 -> 400 -> 800 -> 1600 -> 2000 (clamped)
    let expected_sequence = [50, 100, 200, 400, 800, 1600, 2000, 2000, 2000];
    for (step, &expected) in expected_sequence.iter().enumerate() {
        assert_eq!(
            backoff.current_delay_us(),
            expected,
            "Backoff mismatch at step {step}"
        );
        let returned = backoff.record_failure();
        assert_eq!(
            returned, expected,
            "record_failure return value mismatch at step {step}"
        );
    }

    // Stress: 10,000 consecutive failures must never overflow or exceed max_us
    for _ in 0..10_000 {
        let delay = backoff.record_failure();
        assert_eq!(delay, max_us);
    }
    assert_eq!(backoff.current_delay_us(), max_us);
    assert_eq!(backoff.failure_count(), 10_009);

    // Immediate reset on success
    backoff.record_success();
    assert_eq!(backoff.failure_count(), 0);
    assert_eq!(backoff.current_delay_us(), base_us);

    // Boundary check: base == max
    let mut flat_backoff = AdaptiveBackoff::new(500, 500);
    assert_eq!(flat_backoff.current_delay_us(), 500);
    flat_backoff.record_failure();
    assert_eq!(flat_backoff.current_delay_us(), 500);

    // Boundary check: base > max (constructor clamps max >= base)
    let inverted = AdaptiveBackoff::new(1000, 100);
    assert_eq!(inverted.current_delay_us(), 1000);
}

// ============================================================================
// PART 7: TRUE CONCURRENT MULTI-THREADED STEALING UNDER SAFE RUST
// ============================================================================

#[test]
fn test_adversarial_concurrent_multithreaded_stealing_and_backoff() {
    // Spin up actual concurrent OS threads pounding on BoundedTaskDeque:
    // - 2 Producer threads pushing tasks to the bottom
    // - 2 Local worker threads popping from the bottom (LIFO)
    // - 4 Thief threads concurrently stealing from the top (FIFO) with try_steal_top and steal_top
    // Every task must be processed exactly once (no drops, no duplicates).
    let capacity = 4096;
    let deque = Arc::new(BoundedTaskDeque::new(capacity));
    let tasks_per_producer = 500;
    let total_producers = 2;
    let total_expected_tasks = total_producers * tasks_per_producer;

    let processed_tasks = Arc::new(Mutex::new(HashSet::new()));
    let producers_active = Arc::new(AtomicUsize::new(total_producers));
    let duplicate_count = Arc::new(AtomicUsize::new(0));

    // Spawn Producers
    let mut producer_handles = Vec::new();
    for p_id in 0..total_producers {
        let d = Arc::clone(&deque);
        let active = Arc::clone(&producers_active);
        let handle = thread::spawn(move || {
            for i in 0..tasks_per_producer {
                let task_id = format!("task_p{p_id}_{i}");
                let task = StealableTask::new(&task_id, "Payload", 1);
                // Spin-retry if deque happens to be momentarily full
                while d.push_bottom(task.clone()).is_err() {
                    thread::yield_now();
                }
            }
            active.fetch_sub(1, Ordering::SeqCst);
        });
        producer_handles.push(handle);
    }

    // Spawn Local Workers (pop_bottom)
    let mut consumer_handles = Vec::new();
    for _ in 0..2 {
        let d = Arc::clone(&deque);
        let p_active = Arc::clone(&producers_active);
        let processed = Arc::clone(&processed_tasks);
        let dupes = Arc::clone(&duplicate_count);

        let handle = thread::spawn(move || {
            while p_active.load(Ordering::SeqCst) > 0 || !d.is_empty() {
                if let Some(task) = d.pop_bottom() {
                    let mut lock = processed.lock().unwrap();
                    if !lock.insert(task.id) {
                        dupes.fetch_add(1, Ordering::SeqCst);
                    }
                } else {
                    thread::yield_now();
                }
            }
        });
        consumer_handles.push(handle);
    }

    // Spawn Remote Thieves (steal_top & try_steal_top with AdaptiveBackoff)
    let mut thief_handles = Vec::new();
    for _ in 0..4 {
        let d = Arc::clone(&deque);
        let p_active = Arc::clone(&producers_active);
        let processed = Arc::clone(&processed_tasks);
        let dupes = Arc::clone(&duplicate_count);

        let handle = thread::spawn(move || {
            let mut backoff = AdaptiveBackoff::new(10, 500);
            while p_active.load(Ordering::SeqCst) > 0 || !d.is_empty() {
                // Alternate between non-blocking try_steal_top and steal_top
                let stolen = if backoff.failure_count().is_multiple_of(2) {
                    d.try_steal_top()
                } else {
                    d.steal_top()
                };

                match stolen {
                    Some(task) => {
                        backoff.record_success();
                        let mut lock = processed.lock().unwrap();
                        if !lock.insert(task.id) {
                            dupes.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                    None => {
                        backoff.snooze();
                    }
                }
            }
        });
        thief_handles.push(handle);
    }

    // Join all producers
    for h in producer_handles {
        h.join().expect("Producer thread panicked");
    }

    // Join consumers & thieves
    for h in consumer_handles {
        h.join().expect("Consumer thread panicked");
    }
    for h in thief_handles {
        h.join().expect("Thief thread panicked");
    }

    // Verification of thread safety invariants
    let total_processed = processed_tasks.lock().unwrap().len();
    assert_eq!(
        duplicate_count.load(Ordering::SeqCst),
        0,
        "Concurrency race condition: duplicate tasks processed!"
    );
    assert_eq!(
        total_processed, total_expected_tasks,
        "Missing tasks under thread contention: processed {} of expected {}",
        total_processed, total_expected_tasks
    );
    assert!(
        deque.is_empty(),
        "Deque should be completely drained at termination"
    );
}

// ============================================================================
// PART 8: RANDOMIZED DAG FUZZING (BLUMOFE-LEISERSON INVARIANT VERIFICATION)
// ============================================================================

#[test]
fn test_adversarial_randomized_dag_fuzzing_blumofe_leiserson_invariants() {
    let mut prng = SafePrng::new(0xca11_ab1e_f00d);
    let iterations = 15;

    for iter in 0..iterations {
        let mut dag = StealableTaskDag::new();
        let num_layers = 4 + (prng.next_u64() % 4) as usize; // 4 to 7 layers
        let nodes_per_layer = 3 + (prng.next_u64() % 4) as usize; // 3 to 6 per layer

        let mut layer_nodes: Vec<Vec<String>> = Vec::new();
        let mut node_idx = 0;

        for _l in 0..num_layers {
            let mut current_layer = Vec::new();
            for _ in 0..nodes_per_layer {
                let id = format!("fuzz_{iter}_{node_idx}");
                let weight = 5 + (prng.next_u64() % 50); // weight 5 to 54
                dag.add_task(StealableTask::new(&id, "Fuzz Task", weight));
                current_layer.push(id);
                node_idx += 1;
            }
            layer_nodes.push(current_layer);
        }

        // Add forward edges between layers to preserve acyclicity
        for l in 0..(num_layers - 1) {
            for from_node in &layer_nodes[l] {
                // Ensure at least one edge to next layer
                let target_idx = (prng.next_u64() as usize) % layer_nodes[l + 1].len();
                let to_node = &layer_nodes[l + 1][target_idx];
                let _ = dag.add_dependency(from_node, to_node);

                // Probabilistically add extra cross edges
                if prng.next_u64().is_multiple_of(2) && l + 2 < num_layers {
                    let deep_target = (prng.next_u64() as usize) % layer_nodes[l + 2].len();
                    let _ = dag.add_dependency(from_node, &layer_nodes[l + 2][deep_target]);
                }
            }
        }

        let t1 = dag.total_work();
        let t_inf = dag.critical_path_span().unwrap();
        assert!(t1 >= t_inf);

        for p in [1, 2, 4, 8] {
            let scheduler = WorkStealingScheduler::new(p, 256).unwrap();
            let telemetry = scheduler.execute_dag(&dag).unwrap();

            // Theorem 21 Invariant: T_P <= T_1/P + 3.0 * T_inf
            assert!(
                telemetry.bl_bound_satisfied,
                "Fuzz iter {iter} P={p}: BL bound violated! T_P={} > Bound={}",
                telemetry.actual_makespan_tp, telemetry.theoretical_bl_bound
            );

            // Lower bound: T_P >= T_inf
            assert!(
                telemetry.actual_makespan_tp >= t_inf,
                "Fuzz iter {iter} P={p}: makespan {} < critical path span {}",
                telemetry.actual_makespan_tp,
                t_inf
            );

            // Theorem 22 Bounded Memory Invariant: peak tasks in flight <= P * C_max + P
            assert!(
                telemetry.peak_tasks_in_flight <= p * 256 + p,
                "Fuzz iter {iter} P={p}: peak tasks in flight {} exceeded bounded memory limit",
                telemetry.peak_tasks_in_flight
            );
        }
    }
}

// ============================================================================
// PART 9: BOUNDARY CONDITIONS & ERROR HANDLING
// ============================================================================

#[test]
fn test_adversarial_boundary_conditions_and_error_handling() {
    // 1. Invalid Worker Count = 0
    let err_w0 = WorkStealingScheduler::new(0, 64);
    assert_eq!(err_w0.err(), Some(SchedulerError::InvalidWorkerCount(0)));

    // 2. Invalid Capacity = 0
    let err_c0 = WorkStealingScheduler::new(4, 0);
    assert_eq!(err_c0.err(), Some(SchedulerError::InvalidCapacity(0)));

    // 3. Dependency between non-existent tasks
    let mut empty_dag = StealableTaskDag::new();
    let err_missing = empty_dag.add_dependency("non_existent_1", "non_existent_2");
    assert_eq!(
        err_missing.err(),
        Some(SchedulerError::TaskNotFound("non_existent_1".to_string()))
    );

    // 4. Empty DAG (0 tasks)
    let scheduler = WorkStealingScheduler::new(4, 64).unwrap();
    let empty_telemetry = scheduler.execute_dag(&empty_dag).unwrap();
    assert_eq!(empty_telemetry.total_work_t1, 0);
    assert_eq!(empty_telemetry.critical_path_t_inf, 0);
    assert_eq!(empty_telemetry.actual_makespan_tp, 0);
    assert!(empty_telemetry.bl_bound_satisfied);

    // 5. Single Task DAG (1 task)
    let mut solo_dag = StealableTaskDag::new();
    solo_dag.add_task(StealableTask::new("solo", "Solo", 42));
    for p in [1, 4, 16] {
        let solo_scheduler = WorkStealingScheduler::new(p, 64).unwrap();
        let telemetry = solo_scheduler.execute_dag(&solo_dag).unwrap();
        assert_eq!(telemetry.total_work_t1, 42);
        assert_eq!(telemetry.critical_path_t_inf, 42);
        assert_eq!(telemetry.actual_makespan_tp, 42);
        assert!((telemetry.observed_speedup - 1.0).abs() < 1e-4);
        assert!(telemetry.bl_bound_satisfied);
    }

    // 6. Oversubscribed Worker Pool: P = 64 workers for 3 tasks
    let mut small_dag = StealableTaskDag::new();
    small_dag.add_task(StealableTask::new("s1", "S1", 10));
    small_dag.add_task(StealableTask::new("s2", "S2", 10));
    small_dag.add_task(StealableTask::new("s3", "S3", 10));
    let oversubscribed_scheduler = WorkStealingScheduler::new(64, 64).unwrap();
    let over_telemetry = oversubscribed_scheduler.execute_dag(&small_dag).unwrap();
    assert_eq!(over_telemetry.total_work_t1, 30);
    assert!(over_telemetry.bl_bound_satisfied);

    // 7. Extreme Workload Heterogeneity (1 Monster Task + 30 Tiny Tasks)
    let mut hetero_dag = StealableTaskDag::new();
    hetero_dag.add_task(StealableTask::new("monster", "Heavy Computation", 10_000));
    for i in 0..30 {
        let tid = format!("tiny_{i}");
        hetero_dag.add_task(StealableTask::new(&tid, "Tiny", 2));
    }
    let hetero_scheduler = WorkStealingScheduler::new(4, 64).unwrap();
    let hetero_telemetry = hetero_scheduler.execute_dag(&hetero_dag).unwrap();
    assert_eq!(hetero_telemetry.total_work_t1, 10_060);
    assert_eq!(hetero_telemetry.critical_path_t_inf, 10_000);
    // Makespan is bottlenecked by the monster task
    assert!(hetero_telemetry.actual_makespan_tp >= 10_000);
    assert!(hetero_telemetry.bl_bound_satisfied);
}
