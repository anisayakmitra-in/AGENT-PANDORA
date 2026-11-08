//! Phase 11: Dynamic Adaptive Work-Stealing Task Scheduler with Blumofe-Leiserson Bounds.
//!
//! Provides distributed DAG scheduling with bounded-memory double-ended deques:
//! - Work-first local LIFO (`push_bottom`, `pop_bottom`) preserving spatial/temporal locality
//! - Help-first remote FIFO steal (`steal_top`, `try_steal_top`) maximizing stolen granularity
//! - Adaptive exponential backoff $\tau_k = \min(\tau_{\max}, \tau_0 \cdot 2^k)$ mitigating bus contention
//! - Theorem 21: Blumofe-Leiserson makespan bound $\mathbb{E}[T_P] \le T_1/P + O(T_\infty)$
//! - Theorem 22: Bounded memory invariant $S_P \le P \cdot S_1$ with capacity limit $C_{\max}$

#![forbid(unsafe_code)]

use crate::bandit_router::SafePrng;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// Maximum bounded task capacity per worker deque by default.
pub const DEFAULT_DEQUE_CAPACITY: usize = 1024;

/// Base backoff duration for failed steals in microseconds.
pub const BASE_STEAL_BACKOFF_US: u64 = 50;

/// Maximum backoff duration in microseconds.
pub const MAX_STEAL_BACKOFF_US: u64 = 2000;

/// Critical path contention constant c_inf = 3 / ln(4/3) ~ 10.43 (used in BL bound).
pub const BL_CRITICAL_PATH_COEFFICIENT: f64 = 3.0;

/// Errors arising within the work-stealing scheduler.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SchedulerError {
    DequeCapacityExceeded { capacity: usize },
    InvalidWorkerCount(usize),
    InvalidCapacity(usize),
    CyclicDependency(String),
    TaskNotFound(String),
    WorkerIndexOutOfBounds { index: usize, count: usize },
    ExecutionFailed(String),
}

impl fmt::Display for SchedulerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DequeCapacityExceeded { capacity } => {
                write!(f, "Worker deque capacity exceeded: maximum is {capacity}")
            }
            Self::InvalidWorkerCount(p) => {
                write!(f, "Invalid worker count {p}: must be at least 1")
            }
            Self::InvalidCapacity(c) => {
                write!(f, "Invalid deque capacity {c}: must be at least 1")
            }
            Self::CyclicDependency(msg) => {
                write!(f, "Cyclic dependency detected in task DAG: {msg}")
            }
            Self::TaskNotFound(id) => {
                write!(f, "Task '{id}' not found in DAG")
            }
            Self::WorkerIndexOutOfBounds { index, count } => {
                write!(
                    f,
                    "Worker index {index} out of bounds for worker pool of size {count}"
                )
            }
            Self::ExecutionFailed(msg) => {
                write!(f, "Task execution failed: {msg}")
            }
        }
    }
}

impl std::error::Error for SchedulerError {}

/// State of an individual task node in the execution DAG.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Ready,
    Running,
    Completed,
    Failed,
}

/// A computational unit in the task DAG.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StealableTask {
    pub id: String,
    pub name: String,
    pub weight_cost: u64,
    pub dependencies: Vec<String>,
    pub status: TaskStatus,
}

impl StealableTask {
    pub fn new(id: impl Into<String>, name: impl Into<String>, weight_cost: u64) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            weight_cost: weight_cost.max(1),
            dependencies: Vec::new(),
            status: TaskStatus::Ready,
        }
    }

    pub fn with_dependencies(mut self, deps: Vec<String>) -> Self {
        self.dependencies = deps;
        self
    }
}

/// A thread-safe bounded double-ended queue implementing the Blumofe-Leiserson discipline.
#[derive(Debug)]
pub struct BoundedTaskDeque {
    capacity: usize,
    buffer: Mutex<VecDeque<StealableTask>>,
}

impl BoundedTaskDeque {
    /// Creates a new bounded deque with the specified capacity limit C_max.
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.max(1);
        Self {
            capacity: cap,
            buffer: Mutex::new(VecDeque::with_capacity(cap)),
        }
    }

    fn lock_buffer(&self) -> MutexGuard<'_, VecDeque<StealableTask>> {
        match self.buffer.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// Local worker: pushes child task to the bottom of the deque (LIFO).
    /// Returns Err(DequeCapacityExceeded) if deque is at capacity C_max.
    pub fn push_bottom(&self, task: StealableTask) -> Result<(), SchedulerError> {
        let mut queue = self.lock_buffer();
        if queue.len() >= self.capacity {
            return Err(SchedulerError::DequeCapacityExceeded {
                capacity: self.capacity,
            });
        }
        queue.push_back(task);
        Ok(())
    }

    /// Local worker: pops task from the bottom of the deque (LIFO, data locality).
    pub fn pop_bottom(&self) -> Option<StealableTask> {
        let mut queue = self.lock_buffer();
        queue.pop_back()
    }

    /// Remote thief: steals oldest ancestor from the top of the deque (FIFO, max granularity).
    pub fn steal_top(&self) -> Option<StealableTask> {
        let mut queue = self.lock_buffer();
        queue.pop_front()
    }

    /// Remote thief non-blocking attempt: steals from the top if lock is immediately available.
    /// If contended, returns None so thief can attempt another victim without blocking.
    pub fn try_steal_top(&self) -> Option<StealableTask> {
        if let Ok(mut queue) = self.buffer.try_lock() {
            queue.pop_front()
        } else {
            None
        }
    }

    pub fn len(&self) -> usize {
        self.lock_buffer().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock_buffer().is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

/// Adaptive exponential backoff manager for starved thieves.
#[derive(Clone, Debug)]
pub struct AdaptiveBackoff {
    base_delay_us: u64,
    max_delay_us: u64,
    failure_count: u32,
}

impl Default for AdaptiveBackoff {
    fn default() -> Self {
        Self::new(BASE_STEAL_BACKOFF_US, MAX_STEAL_BACKOFF_US)
    }
}

impl AdaptiveBackoff {
    pub fn new(base_delay_us: u64, max_delay_us: u64) -> Self {
        Self {
            base_delay_us: base_delay_us.max(1),
            max_delay_us: max_delay_us.max(base_delay_us),
            failure_count: 0,
        }
    }

    /// Computes the current backoff delay: tau_k = min(tau_max, tau_0 * 2^k).
    pub fn current_delay_us(&self) -> u64 {
        let shift = self.failure_count.min(16);
        let factor = 1u64.checked_shl(shift).unwrap_or(u64::MAX);
        let computed = self.base_delay_us.saturating_mul(factor);
        computed.min(self.max_delay_us)
    }

    /// Records a failed steal attempt and increments backoff level.
    pub fn record_failure(&mut self) -> u64 {
        let delay = self.current_delay_us();
        self.failure_count = self.failure_count.saturating_add(1);
        delay
    }

    /// Resets the failure counter to 0 upon a successful steal.
    pub fn record_success(&mut self) {
        self.failure_count = 0;
    }

    pub fn failure_count(&self) -> u32 {
        self.failure_count
    }

    pub fn snooze(&mut self) {
        let delay = self.record_failure();
        std::thread::sleep(Duration::from_micros(delay));
    }
}

/// A Directed Acyclic Graph of stealable tasks.
#[derive(Clone, Debug, Default)]
pub struct StealableTaskDag {
    tasks: HashMap<String, StealableTask>,
    adjacency: HashMap<String, Vec<String>>,
    reverse_adjacency: HashMap<String, Vec<String>>,
}

impl StealableTaskDag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_task(&mut self, task: StealableTask) {
        let id = task.id.clone();
        for dep in &task.dependencies {
            self.adjacency
                .entry(dep.clone())
                .or_default()
                .push(id.clone());
            self.reverse_adjacency
                .entry(id.clone())
                .or_default()
                .push(dep.clone());
        }
        self.adjacency.entry(id.clone()).or_default();
        self.reverse_adjacency.entry(id.clone()).or_default();
        self.tasks.insert(id, task);
    }

    pub fn add_dependency(&mut self, from: &str, to: &str) -> Result<(), SchedulerError> {
        if !self.tasks.contains_key(from) {
            return Err(SchedulerError::TaskNotFound(from.to_string()));
        }
        if !self.tasks.contains_key(to) {
            return Err(SchedulerError::TaskNotFound(to.to_string()));
        }
        if from == to {
            return Err(SchedulerError::CyclicDependency(format!(
                "Self dependency on {from}"
            )));
        }

        self.adjacency
            .entry(from.to_string())
            .or_default()
            .push(to.to_string());
        self.reverse_adjacency
            .entry(to.to_string())
            .or_default()
            .push(from.to_string());
        if let Some(t) = self.tasks.get_mut(to) {
            let dep = from.to_string();
            if !t.dependencies.contains(&dep) {
                t.dependencies.push(dep);
            }
        }

        // Validate cycle freedom
        self.topological_sort()?;
        Ok(())
    }

    /// Kahn's algorithm for topological sorting and cycle detection in O(|V| + |E|).
    pub fn topological_sort(&self) -> Result<Vec<String>, SchedulerError> {
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        for id in self.tasks.keys() {
            in_degree.insert(id.clone(), 0);
        }

        for succs in self.adjacency.values() {
            for succ in succs {
                *in_degree.entry(succ.clone()).or_default() += 1;
            }
        }

        let mut queue: VecDeque<String> = in_degree
            .iter()
            .filter(|(_, deg)| **deg == 0)
            .map(|(id, _)| id.clone())
            .collect();

        let mut order = Vec::with_capacity(self.tasks.len());

        while let Some(u) = queue.pop_front() {
            order.push(u.clone());
            if let Some(succs) = self.adjacency.get(&u) {
                for succ in succs {
                    if let Some(deg) = in_degree.get_mut(succ) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push_back(succ.clone());
                        }
                    }
                }
            }
        }

        if order.len() != self.tasks.len() {
            return Err(SchedulerError::CyclicDependency(
                "Cycle detected in task dependencies".to_string(),
            ));
        }

        Ok(order)
    }

    /// Total sequential work T_1 = sum_{v in V} w(v).
    pub fn total_work(&self) -> u64 {
        self.tasks.values().map(|t| t.weight_cost).sum()
    }

    /// Critical path span T_infinity via longest path dynamic programming.
    pub fn critical_path_span(&self) -> Result<u64, SchedulerError> {
        let order = self.topological_sort()?;
        let mut dist: HashMap<String, u64> = HashMap::new();

        for id in &order {
            let w = self.tasks[id].weight_cost;
            let pred_max = self
                .reverse_adjacency
                .get(id)
                .and_then(|preds| {
                    preds
                        .iter()
                        .map(|p| dist.get(p).copied().unwrap_or(0))
                        .max()
                })
                .unwrap_or(0);
            dist.insert(id.clone(), pred_max + w);
        }

        Ok(dist.values().copied().max().unwrap_or(0))
    }

    pub fn tasks(&self) -> &HashMap<String, StealableTask> {
        &self.tasks
    }
}

/// Telemetry and execution metrics for a work-stealing scheduling run.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SchedulerTelemetry {
    pub total_work_t1: u64,
    pub critical_path_t_inf: u64,
    pub actual_makespan_tp: u64,
    pub worker_count: usize,
    pub total_steals_attempted: u64,
    pub total_steals_succeeded: u64,
    pub theoretical_bl_bound: f64,
    pub observed_speedup: f64,
    pub parallel_efficiency: f64,
    pub peak_tasks_in_flight: usize,
    pub bl_bound_satisfied: bool,
}

/// Dynamic adaptive work-stealing scheduler managing a pool of worker deques.
pub struct WorkStealingScheduler {
    pub worker_count: usize,
    pub deque_capacity: usize,
    pub deques: Vec<Arc<BoundedTaskDeque>>,
}

impl WorkStealingScheduler {
    pub fn new(worker_count: usize, deque_capacity: usize) -> Result<Self, SchedulerError> {
        if worker_count == 0 {
            return Err(SchedulerError::InvalidWorkerCount(0));
        }
        if deque_capacity == 0 {
            return Err(SchedulerError::InvalidCapacity(0));
        }

        let mut deques = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            deques.push(Arc::new(BoundedTaskDeque::new(deque_capacity)));
        }

        Ok(Self {
            worker_count,
            deque_capacity,
            deques,
        })
    }

    /// Computes Blumofe-Leiserson theoretical makespan bound: E[T_P] <= T_1/P + c_inf * T_inf.
    pub fn blumofe_leiserson_bound(t1: u64, t_inf: u64, p: usize) -> f64 {
        let p_f = p.max(1) as f64;
        ((t1 as f64) / p_f) + (BL_CRITICAL_PATH_COEFFICIENT * (t_inf as f64))
    }

    /// Uniformly selects a random victim v from {0, ..., P-1} \ {thief_id} using SafePrng.
    pub fn select_victim(thief_id: usize, worker_count: usize, prng: &mut SafePrng) -> usize {
        if worker_count <= 1 {
            return thief_id;
        }
        let rand_idx = (prng.next_u64() as usize) % (worker_count - 1);
        if rand_idx >= thief_id {
            rand_idx + 1
        } else {
            rand_idx
        }
    }

    /// Attempts to steal a task from a random victim.
    pub fn steal(&self, thief_id: usize, prng: &mut SafePrng) -> Option<StealableTask> {
        if self.worker_count <= 1 {
            return None;
        }
        let victim_id = Self::select_victim(thief_id, self.worker_count, prng);
        self.deques[victim_id].steal_top()
    }

    /// Executes a task DAG using discrete-event simulated work-stealing across P workers.
    /// Accurately tracks steals, work steps, makespan, and validates Blumofe-Leiserson bounds.
    pub fn execute_dag(
        &self,
        dag: &StealableTaskDag,
    ) -> Result<SchedulerTelemetry, SchedulerError> {
        dag.topological_sort()?; // Ensure no cycles

        let t1 = dag.total_work();
        let t_inf = dag.critical_path_span()?;
        let num_workers = self.worker_count;

        let mut remaining_deps: HashMap<String, usize> = HashMap::new();
        for (id, preds) in &dag.reverse_adjacency {
            remaining_deps.insert(id.clone(), preds.len());
        }

        // Ready tasks with in-degree 0 initially
        let mut initial_ready: Vec<String> = remaining_deps
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(id, _)| id.clone())
            .collect();
        initial_ready.sort(); // Deterministic ordering

        // Enqueue initial ready tasks onto worker deques (round-robin)
        for (i, task_id) in initial_ready.into_iter().enumerate() {
            let worker_idx = i % num_workers;
            let task = dag.tasks[&task_id].clone();
            self.deques[worker_idx].push_bottom(task)?;
        }

        let mut prngs: Vec<SafePrng> = (0..num_workers)
            .map(|i| SafePrng::new(0xdead_beef_cafe_0000 + (i as u64) + 1))
            .collect();
        let mut backoffs: Vec<AdaptiveBackoff> = vec![AdaptiveBackoff::default(); num_workers];

        // Worker slots: Option<(StealableTask, finish_timestamp)>
        let mut worker_slots: Vec<Option<(StealableTask, u64)>> = vec![None; num_workers];
        let mut completed_count = 0;
        let total_tasks = dag.tasks.len();
        let mut current_time = 0u64;

        let mut total_steals_attempted = 0u64;
        let mut total_steals_succeeded = 0u64;
        let mut peak_tasks_in_flight = 0usize;

        while completed_count < total_tasks {
            // Track active concurrency
            let active_busy = worker_slots.iter().filter(|s| s.is_some()).count();
            let queued: usize = self.deques.iter().map(|d| d.len()).sum();
            peak_tasks_in_flight = peak_tasks_in_flight.max(active_busy + queued);

            // For each idle worker, attempt to get work: locally first (LIFO), then steal (FIFO)
            for w in 0..num_workers {
                if worker_slots[w].is_none() {
                    // Step 1: Work-first local pop_bottom
                    if let Some(local_task) = self.deques[w].pop_bottom() {
                        let finish = current_time + local_task.weight_cost;
                        worker_slots[w] = Some((local_task, finish));
                        backoffs[w].record_success();
                    } else if num_workers > 1 {
                        // Step 2: Help-first remote steal_top from random victim
                        total_steals_attempted += 1;
                        if let Some(stolen_task) = self.steal(w, &mut prngs[w]) {
                            total_steals_succeeded += 1;
                            let finish = current_time + stolen_task.weight_cost;
                            worker_slots[w] = Some((stolen_task, finish));
                            backoffs[w].record_success();
                        } else {
                            backoffs[w].record_failure();
                        }
                    }
                }
            }

            // Find next earliest completion time among busy workers
            let next_event = worker_slots
                .iter()
                .filter_map(|s| s.as_ref().map(|(_, fin)| *fin))
                .min();

            match next_event {
                Some(next_time) => {
                    current_time = next_time;
                    for (w, slot) in worker_slots.iter_mut().enumerate().take(num_workers) {
                        let is_finished = match slot {
                            Some((_, finish)) => *finish == current_time,
                            None => false,
                        };
                        if is_finished {
                            let (task, _) = slot.take().unwrap();
                            let task_id = task.id;
                            completed_count += 1;

                            // Unlock successors in DAG
                            if let Some(succs) = dag.adjacency.get(&task_id) {
                                for succ in succs {
                                    if let Some(deps) =
                                        remaining_deps.get_mut(succ).filter(|d| **d > 0)
                                    {
                                        *deps -= 1;
                                        if *deps == 0 {
                                            // Push newly ready child task onto local worker w (work-first)
                                            let succ_task = dag.tasks[succ].clone();
                                            self.deques[w].push_bottom(succ_task)?;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                None => {
                    // No workers busy and no ready tasks
                    if completed_count < total_tasks {
                        return Err(SchedulerError::ExecutionFailed(format!(
                            "Deadlock: {completed_count}/{total_tasks} completed but all queues empty"
                        )));
                    }
                    break;
                }
            }
        }

        let actual_makespan_tp = current_time;
        let bl_bound = Self::blumofe_leiserson_bound(t1, t_inf, num_workers);
        let speedup = if actual_makespan_tp > 0 {
            (t1 as f64) / (actual_makespan_tp as f64)
        } else {
            1.0
        };
        let efficiency = speedup / (num_workers as f64);
        let bl_satisfied = (actual_makespan_tp as f64) <= bl_bound;

        Ok(SchedulerTelemetry {
            total_work_t1: t1,
            critical_path_t_inf: t_inf,
            actual_makespan_tp,
            worker_count: num_workers,
            total_steals_attempted,
            total_steals_succeeded,
            theoretical_bl_bound: bl_bound,
            observed_speedup: speedup,
            parallel_efficiency: efficiency,
            peak_tasks_in_flight,
            bl_bound_satisfied: bl_satisfied,
        })
    }
}
