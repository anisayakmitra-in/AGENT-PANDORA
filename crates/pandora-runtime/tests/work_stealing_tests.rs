#![forbid(unsafe_code)]

use pandora_runtime::bandit_router::SafePrng;
use pandora_runtime::work_stealing::{
    AdaptiveBackoff, BoundedTaskDeque, SchedulerError, StealableTask, StealableTaskDag,
    WorkStealingScheduler,
};

#[test]
fn test_bounded_deque_lifo_bottom_fifo_top_discipline() {
    let deque = BoundedTaskDeque::new(10);
    assert_eq!(deque.len(), 0);
    assert!(deque.is_empty());

    let task_a = StealableTask::new("task-a", "Root Task", 100);
    let task_b = StealableTask::new("task-b", "Child 1", 50);
    let task_c = StealableTask::new("task-c", "Child 2", 25);

    // Push tasks to bottom: [task_a (top), task_b, task_c (bottom)]
    deque.push_bottom(task_a.clone()).unwrap();
    deque.push_bottom(task_b.clone()).unwrap();
    deque.push_bottom(task_c.clone()).unwrap();
    assert_eq!(deque.len(), 3);

    // Local worker pops bottom -> should be LIFO (task_c)
    let popped = deque.pop_bottom().unwrap();
    assert_eq!(popped.id, "task-c");
    assert_eq!(deque.len(), 2);

    // Remote thief steals top -> should be FIFO (task_a)
    let stolen = deque.steal_top().unwrap();
    assert_eq!(stolen.id, "task-a");
    assert_eq!(deque.len(), 1);

    // Local worker pops remaining task -> task_b
    let last = deque.pop_bottom().unwrap();
    assert_eq!(last.id, "task-b");
    assert_eq!(deque.len(), 0);
    assert!(deque.is_empty());

    // Both pop and steal return None when empty
    assert!(deque.pop_bottom().is_none());
    assert!(deque.steal_top().is_none());
    assert!(deque.try_steal_top().is_none());
}

#[test]
fn test_bounded_deque_capacity_enforcement() {
    let capacity = 3;
    let deque = BoundedTaskDeque::new(capacity);

    for i in 0..capacity {
        let task = StealableTask::new(format!("task-{i}"), "Worker Task", 10);
        assert!(deque.push_bottom(task).is_ok());
    }
    assert_eq!(deque.len(), capacity);

    // Push beyond capacity MUST fail with DequeCapacityExceeded
    let overflow_task = StealableTask::new("overflow", "Overflow", 10);
    let result = deque.push_bottom(overflow_task);
    assert_eq!(
        result,
        Err(SchedulerError::DequeCapacityExceeded { capacity: 3 })
    );

    // Popping one element allows another push
    let _ = deque.pop_bottom();
    assert_eq!(deque.len(), 2);
    let retry_task = StealableTask::new("retry", "Retry", 10);
    assert!(deque.push_bottom(retry_task).is_ok());
    assert_eq!(deque.len(), 3);
}

#[test]
fn test_adaptive_exponential_backoff_progression() {
    let base_us = 50;
    let max_us = 2000;
    let mut backoff = AdaptiveBackoff::new(base_us, max_us);

    assert_eq!(backoff.current_delay_us(), 50);
    assert_eq!(backoff.failure_count(), 0);

    // Sequence of failures: 50 -> 100 -> 200 -> 400 -> 800 -> 1600 -> 2000 (clamped)
    let d0 = backoff.record_failure();
    assert_eq!(d0, 50);
    assert_eq!(backoff.current_delay_us(), 100);

    let d1 = backoff.record_failure();
    assert_eq!(d1, 100);
    assert_eq!(backoff.current_delay_us(), 200);

    let d2 = backoff.record_failure();
    assert_eq!(d2, 200);
    assert_eq!(backoff.current_delay_us(), 400);

    let d3 = backoff.record_failure();
    assert_eq!(d3, 400);
    assert_eq!(backoff.current_delay_us(), 800);

    let d4 = backoff.record_failure();
    assert_eq!(d4, 800);
    assert_eq!(backoff.current_delay_us(), 1600);

    let d5 = backoff.record_failure();
    assert_eq!(d5, 1600);
    assert_eq!(backoff.current_delay_us(), 2000); // Clamped at max_us

    let d6 = backoff.record_failure();
    assert_eq!(d6, 2000);
    assert_eq!(backoff.current_delay_us(), 2000);

    // Success resets backoff immediately to base_us
    backoff.record_success();
    assert_eq!(backoff.failure_count(), 0);
    assert_eq!(backoff.current_delay_us(), 50);
}

#[test]
fn test_randomized_victim_selection_self_exclusion_and_uniformity() {
    let mut prng = SafePrng::new(0x1337_c0de);
    let worker_count = 8;
    let thief_id = 3;

    let mut selection_counts = vec![0usize; worker_count];
    let iterations = 10_000;

    for _ in 0..iterations {
        let victim = WorkStealingScheduler::select_victim(thief_id, worker_count, &mut prng);
        // Self-exclusion invariant: thief NEVER selects itself
        assert_ne!(
            victim, thief_id,
            "Thief {thief_id} must never select itself as victim"
        );
        assert!(victim < worker_count);
        selection_counts[victim] += 1;
    }

    assert_eq!(selection_counts[thief_id], 0);

    // Check statistical uniformity across the other 7 workers
    // Expected frequency per peer = 10000 / 7 ~ 1428.57
    let expected = (iterations as f64) / ((worker_count - 1) as f64);
    for (idx, &count) in selection_counts.iter().enumerate() {
        if idx != thief_id {
            let diff = ((count as f64) - expected).abs();
            assert!(
                diff < expected * 0.15,
                "Worker {idx} count {count} deviated excessively from expected {expected}"
            );
        }
    }
}

#[test]
fn test_dag_cycle_detection() {
    let mut dag = StealableTaskDag::new();
    dag.add_task(StealableTask::new("t1", "Task 1", 10));
    dag.add_task(StealableTask::new("t2", "Task 2", 10));
    dag.add_task(StealableTask::new("t3", "Task 3", 10));

    assert!(dag.add_dependency("t1", "t2").is_ok());
    assert!(dag.add_dependency("t2", "t3").is_ok());

    // Creating cycle t3 -> t1 MUST fail
    let err = dag.add_dependency("t3", "t1");
    assert!(matches!(err, Err(SchedulerError::CyclicDependency(_))));
}

#[test]
fn test_blumofe_leiserson_makespan_bound_verification() {
    // Construct a balanced fork-join computation DAG:
    // Root -> (Branch A1 -> A2, Branch B1 -> B2, Branch C1 -> C2, Branch D1 -> D2) -> Join
    let mut dag = StealableTaskDag::new();

    dag.add_task(StealableTask::new("root", "Root Initialization", 20));

    dag.add_task(StealableTask::new("a1", "Branch A Stage 1", 30));
    dag.add_task(StealableTask::new("a2", "Branch A Stage 2", 40));

    dag.add_task(StealableTask::new("b1", "Branch B Stage 1", 25));
    dag.add_task(StealableTask::new("b2", "Branch B Stage 2", 35));

    dag.add_task(StealableTask::new("c1", "Branch C Stage 1", 50));
    dag.add_task(StealableTask::new("c2", "Branch C Stage 2", 60));

    dag.add_task(StealableTask::new("d1", "Branch D Stage 1", 20));
    dag.add_task(StealableTask::new("d2", "Branch D Stage 2", 30));

    dag.add_task(StealableTask::new("join", "Join & Aggregate", 15));

    dag.add_dependency("root", "a1").unwrap();
    dag.add_dependency("a1", "a2").unwrap();
    dag.add_dependency("a2", "join").unwrap();

    dag.add_dependency("root", "b1").unwrap();
    dag.add_dependency("b1", "b2").unwrap();
    dag.add_dependency("b2", "join").unwrap();

    dag.add_dependency("root", "c1").unwrap();
    dag.add_dependency("c1", "c2").unwrap();
    dag.add_dependency("c2", "join").unwrap();

    dag.add_dependency("root", "d1").unwrap();
    dag.add_dependency("d1", "d2").unwrap();
    dag.add_dependency("d2", "join").unwrap();

    let t1 = dag.total_work();
    // T1 = 20 + (30+40) + (25+35) + (50+60) + (20+30) + 15 = 20 + 70 + 60 + 110 + 50 + 15 = 325
    assert_eq!(t1, 325);

    let t_inf = dag.critical_path_span().unwrap();
    // Critical path: root (20) -> c1 (50) -> c2 (60) -> join (15) = 145
    assert_eq!(t_inf, 145);

    // Test across worker configurations: P = 1, 2, 4
    for p in [1, 2, 4] {
        let scheduler = WorkStealingScheduler::new(p, 64).unwrap();
        let telemetry = scheduler.execute_dag(&dag).unwrap();

        assert_eq!(telemetry.total_work_t1, t1);
        assert_eq!(telemetry.critical_path_t_inf, t_inf);
        assert_eq!(telemetry.worker_count, p);

        // Theorem 21 Invariant: T_P <= T_1/P + O(T_inf)
        assert!(
            telemetry.bl_bound_satisfied,
            "Blumofe-Leiserson bound violated: T_{p}={} exceeds bound {}",
            telemetry.actual_makespan_tp, telemetry.theoretical_bl_bound
        );

        // Lower bound check: T_P >= max(T_1 / P, T_inf)
        assert!(
            telemetry.actual_makespan_tp >= t_inf,
            "Makespan cannot be faster than critical path span T_inf"
        );

        if p == 1 {
            assert_eq!(telemetry.actual_makespan_tp, t1);
            assert_eq!(telemetry.total_steals_attempted, 0);
            assert!((telemetry.observed_speedup - 1.0).abs() < 1e-4);
        } else {
            // Speedup is strictly greater than 1 for P > 1 on parallel branch workload
            assert!(telemetry.observed_speedup > 1.0);
        }
    }
}

#[test]
fn test_bounded_memory_invariant_tracking() {
    let mut dag = StealableTaskDag::new();
    let num_tasks = 20;

    // Independent task array
    for i in 0..num_tasks {
        dag.add_task(StealableTask::new(format!("task-{i}"), "Independent", 10));
    }

    let p = 4;
    let deque_cap = 16;
    let scheduler = WorkStealingScheduler::new(p, deque_cap).unwrap();
    let telemetry = scheduler.execute_dag(&dag).unwrap();

    // Theorem 22: peak tasks in flight is bounded by total deque capacity
    assert!(
        telemetry.peak_tasks_in_flight <= p * deque_cap + p,
        "Memory invariant violated: peak tasks {} exceeded bound {}",
        telemetry.peak_tasks_in_flight,
        p * deque_cap + p
    );
}
