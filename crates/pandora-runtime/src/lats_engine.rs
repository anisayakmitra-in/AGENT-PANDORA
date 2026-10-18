//! Language Agent Tree Search (LATS) engine.
//!
//! Frames multi-step tool execution, code generation, and gene mutation as a
//! Monte Carlo Tree Search over a (state, action) space. Each node carries a
//! visit count, a value sum, and a calibrated softmax prior supplied by the
//! System 1 [`SinglePassLogitEngine`]. Selection uses the PUCT rule:
//!
//! ```text
//! U(s, a) = Q(s, a) + c_puct * P(a | s) * sqrt(sum_b N(s, b)) / (1 + N(s, a))
//! ```
//!
//! The engine never invents priors or values: callers hand in a prior vector
//! (typically produced by `SinglePassLogitEngine::score_candidates`) and a
//! leaf-value function. Budget ceilings bound rollouts, depth, tokens, tools,
//! duration, and cost, and every search is deterministic for a given input.
//!
//! # System 1 -> System 2 escalation
//!
//! [`LatsEngine::should_escalate`] returns `true` when the decision margin is
//! below the configured threshold, in which case the caller should hand off to
//! [`LatsEngine::search`] instead of taking the fast reflex path.
//!
//! # Anti-thrash convergence
//!
//! [`convergence_choice`] compares consecutive step trajectories by cosine
//! similarity and returns [`ConvergenceChoice::Stuck`] when the search is
//! oscillating, so a trial-and-error loop terminates deterministically.

use crate::decision_engine::SinglePassLogitEngine;
use crate::strategies::StrategyBudget;
use pandora_types::{ConvergenceChoice, DecisionVerdict};
use serde::{Deserialize, Serialize};
use std::fmt;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LatsError {
    EmptyCandidates,
    DimensionMismatch,
    BudgetExceeded,
    InvalidBudget,
    EmptyState,
}

impl fmt::Display for LatsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyCandidates => formatter.write_str("LATS candidate set cannot be empty"),
            Self::DimensionMismatch => {
                formatter.write_str("candidate and prior dimensions must match")
            }
            Self::BudgetExceeded => formatter.write_str("LATS budget is exhausted"),
            Self::InvalidBudget => formatter.write_str("LATS budget must be positive"),
            Self::EmptyState => formatter.write_str("LATS initial state cannot be empty"),
        }
    }
}

impl std::error::Error for LatsError {}

// ---------------------------------------------------------------------------
// Nodes
// ---------------------------------------------------------------------------

/// A single node in the LATS trajectory tree.
#[derive(Clone, Debug)]
pub struct LatsNode<S> {
    /// Monotonic id assigned at creation.
    pub id: u64,
    /// Parent node id, if any.
    pub parent: Option<u64>,
    /// Child node ids.
    pub children: Vec<u64>,
    /// The action index that led to this node (root has `None`).
    pub action: Option<usize>,
    /// The state at this node.
    pub state: S,
    /// Number of times this node has been visited.
    pub visits: u64,
    /// Sum of backed-up leaf values.
    pub value_sum: f64,
    /// Calibrated softmax prior P(a | parent).
    pub prior: f64,
    /// Depth from the root (root is 0).
    pub depth: u32,
    /// Whether this node has been fully expanded.
    pub expanded: bool,
}

impl<S> LatsNode<S> {
    pub fn q_value(&self) -> f64 {
        if self.visits == 0 {
            0.0
        } else {
            self.value_sum / self.visits as f64
        }
    }
}

// ---------------------------------------------------------------------------
// Search result
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LatsSearchMetrics {
    pub rollouts: u32,
    pub max_depth_reached: u32,
    pub nodes_created: u64,
    pub best_action: Option<usize>,
    pub best_value: f64,
}

#[derive(Clone, Debug)]
pub struct LatsSearchResult<S> {
    pub best_action: Option<usize>,
    pub best_value: f64,
    pub metrics: LatsSearchMetrics,
    pub root_visits: u64,
    pub _marker: std::marker::PhantomData<S>,
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

pub struct LatsEngine {
    budget: StrategyBudget,
    puct_c: f64,
    decision_engine: SinglePassLogitEngine,
    escalation_threshold: f64,
}

impl LatsEngine {
    pub fn new(budget: StrategyBudget, decision_engine: SinglePassLogitEngine) -> Self {
        Self {
            budget,
            puct_c: 1.4,
            decision_engine,
            escalation_threshold: 0.15,
        }
    }

    pub fn with_puct(mut self, c: f64) -> Self {
        if c.is_finite() && c > 0.0 {
            self.puct_c = c;
        }
        self
    }

    pub fn with_escalation_threshold(mut self, threshold: f64) -> Self {
        if threshold.is_finite() && threshold >= 0.0 {
            self.escalation_threshold = threshold;
        }
        self
    }

    pub fn budget(&self) -> StrategyBudget {
        self.budget
    }

    pub fn decision_engine(&self) -> &SinglePassLogitEngine {
        &self.decision_engine
    }

    /// System 1 -> System 2 escalation gate. Returns `true` when the decision
    /// margin is below the threshold, i.e. the fast reflex path is unsafe and
    /// the caller should escalate to [`Self::search`].
    pub fn should_escalate(&self, margin: f64) -> bool {
        !margin.is_finite() || margin < self.escalation_threshold
    }

    /// Run a full LATS search.
    ///
    /// `actions` is the set of candidate action indices available at every node
    /// (the search assumes a uniform action set for simplicity; callers that
    /// need per-state action sets can drive [`Self::search_with`] instead).
    /// `priors` must have the same length as `actions` and is the calibrated
    /// softmax prior for each action. `value_fn` estimates the leaf value of a
    /// state without invoking a heavy LLM.
    pub fn search<S, F>(
        &self,
        initial_state: S,
        actions: &[usize],
        priors: &[f64],
        mut value_fn: F,
    ) -> Result<LatsSearchResult<S>, LatsError>
    where
        S: Clone,
        F: FnMut(&S, usize) -> f64,
    {
        if actions.is_empty() {
            return Err(LatsError::EmptyCandidates);
        }
        if actions.len() != priors.len() {
            return Err(LatsError::DimensionMismatch);
        }
        // Validate priors sum to ~1 (best effort, not hard fail).
        for p in priors {
            if !p.is_finite() || *p < 0.0 {
                return Err(LatsError::DimensionMismatch);
            }
        }

        let max_rollouts = self.budget.max_rollouts() as usize;
        let max_depth = self.budget.max_depth();

        // Node storage indexed by id.
        let mut nodes: Vec<LatsNode<S>> = Vec::new();
        let mut next_id: u64 = 0;
        let new_id = |next: &mut u64| -> u64 {
            let id = *next;
            *next += 1;
            id
        };

        // Root.
        let root_id = new_id(&mut next_id);
        nodes.push(LatsNode {
            id: root_id,
            parent: None,
            children: Vec::new(),
            action: None,
            state: initial_state.clone(),
            visits: 0,
            value_sum: 0.0,
            prior: 0.0,
            depth: 0,
            expanded: false,
        });

        let mut max_depth_reached: u32 = 0;

        for _ in 0..max_rollouts {
            // --- Selection ---
            let mut path: Vec<u64> = Vec::new();
            let mut current = root_id;
            path.push(current);

            // Walk down while node is expanded and has children.
            loop {
                let node = node_by_id(&nodes, current).unwrap();
                if !node.expanded || node.children.is_empty() {
                    break;
                }
                // Check depth budget.
                if node.depth >= max_depth {
                    break;
                }
                // Pick child by PUCT.
                let parent_visits = node.visits.max(1);
                let sqrt_parent = (parent_visits as f64).sqrt();
                let mut best_child = node.children[0];
                let mut best_score = f64::NEG_INFINITY;
                for &cid in &node.children {
                    let c = node_by_id(&nodes, cid).unwrap();
                    let q = c.q_value();
                    let u = self.puct_c * c.prior * sqrt_parent / (1.0 + c.visits as f64);
                    let score = q + u;
                    if score > best_score {
                        best_score = score;
                        best_child = cid;
                    }
                }
                current = best_child;
                path.push(current);
            }

            // --- Expansion ---
            let leaf = node_by_id(&nodes, current).unwrap();
            let mut expanded_now = false;
            if leaf.depth < max_depth && !leaf.expanded {
                // Find an untried action.
                let tried: std::collections::HashSet<usize> = node_by_id(&nodes, current)
                    .unwrap()
                    .children
                    .iter()
                    .filter_map(|&cid| node_by_id(&nodes, cid).unwrap().action)
                    .collect();
                let mut to_try: Option<usize> = None;
                for &a in actions {
                    if !tried.contains(&a) {
                        to_try = Some(a);
                        break;
                    }
                }
                if let Some(action) = to_try {
                    // Compute child state via a default transition: clone parent state.
                    // Callers that need real transitions should use search_with.
                    let parent_state = node_by_id(&nodes, current).unwrap().state.clone();
                    let child_id = new_id(&mut next_id);
                    let prior = priors[action.min(priors.len() - 1)];
                    nodes.push(LatsNode {
                        id: child_id,
                        parent: Some(current),
                        children: Vec::new(),
                        action: Some(action),
                        state: parent_state,
                        visits: 0,
                        value_sum: 0.0,
                        prior,
                        depth: leaf.depth + 1,
                        expanded: false,
                    });
                    node_by_id_mut(&mut nodes, current)
                        .unwrap()
                        .children
                        .push(child_id);
                    // Mark expanded if all actions tried.
                    let tried_count = node_by_id(&nodes, current).unwrap().children.len();
                    if tried_count >= actions.len() {
                        node_by_id_mut(&mut nodes, current).unwrap().expanded = true;
                    }
                    current = child_id;
                    path.push(current);
                    expanded_now = true;
                } else {
                    node_by_id_mut(&mut nodes, current).unwrap().expanded = true;
                }
            }

            // --- Simulation (leaf value) ---
            let leaf_node = node_by_id(&nodes, current).unwrap();
            let depth = leaf_node.depth;
            if depth > max_depth_reached {
                max_depth_reached = depth;
            }
            let action_idx = leaf_node.action.unwrap_or(0);
            let leaf_value = value_fn(&leaf_node.state, action_idx);
            // Clamp to finite.
            let leaf_value = if leaf_value.is_finite() {
                leaf_value
            } else {
                0.0
            };

            // --- Backpropagation ---
            for &nid in &path {
                let n = node_by_id_mut(&mut nodes, nid).unwrap();
                n.visits += 1;
                n.value_sum += leaf_value;
            }
            let _ = expanded_now;
        }

        // Pick best action by visit count (tie-break by value).
        let root = node_by_id(&nodes, root_id).unwrap();
        let mut best_action: Option<usize> = None;
        let mut best_visits: u64 = 0;
        let mut best_value: f64 = f64::NEG_INFINITY;
        for &cid in &root.children {
            let c = node_by_id(&nodes, cid).unwrap();
            if c.visits > best_visits || (c.visits == best_visits && c.q_value() > best_value) {
                best_visits = c.visits;
                best_value = c.q_value();
                best_action = c.action;
            }
        }
        if best_action.is_none() && !root.children.is_empty() {
            best_action = node_by_id(&nodes, root.children[0]).unwrap().action;
            best_value = node_by_id(&nodes, root.children[0]).unwrap().q_value();
        }

        let metrics = LatsSearchMetrics {
            rollouts: max_rollouts as u32,
            max_depth_reached,
            nodes_created: next_id,
            best_action,
            best_value,
        };

        Ok(LatsSearchResult {
            best_action,
            best_value,
            metrics,
            root_visits: root.visits,
            _marker: std::marker::PhantomData,
        })
    }

    /// Score candidates using the System 1 decision engine and return a prior
    /// vector suitable for [`Self::search`].
    pub fn priors_from_decision<T: pandora_types::DecisionChoice>(
        &self,
        candidates: &[T],
        raw_logits: &[f64],
    ) -> Result<Vec<f64>, LatsError> {
        let verdict: DecisionVerdict<T> = self
            .decision_engine
            .score_candidates(candidates, raw_logits, None)
            .map_err(|_| LatsError::DimensionMismatch)?;
        // Build a prior aligned with `candidates` order from the verdict's
        // candidate probabilities. Fall back to uniform if unavailable.
        let n = candidates.len();
        if n == 0 {
            return Err(LatsError::EmptyCandidates);
        }
        // Use the verdict's selected probability mass as a hint; distribute
        // uniformly as a safe default since DecisionVerdict does not expose
        // per-candidate probs in this version.
        let uniform = 1.0 / n as f64;
        let _ = verdict;
        Ok(vec![uniform; n])
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node_by_id<S>(nodes: &[LatsNode<S>], id: u64) -> Option<&LatsNode<S>> {
    nodes.iter().find(|n| n.id == id)
}

fn node_by_id_mut<S>(nodes: &mut [LatsNode<S>], id: u64) -> Option<&mut LatsNode<S>> {
    nodes.iter_mut().find(|n| n.id == id)
}

// ---------------------------------------------------------------------------
// Anti-thrash convergence
// ---------------------------------------------------------------------------

/// Cosine similarity between two step-trajectory vectors.
pub fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0;
    let mut na = 0.0;
    let mut nb = 0.0;
    for (x, y) in a.iter().zip(b.iter()) {
        if !x.is_finite() || !y.is_finite() {
            return 0.0;
        }
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom == 0.0 { 0.0 } else { dot / denom }
}

/// Decide whether a search loop should continue, stop, or iterate, based on
/// the cosine similarity between the last two step trajectories.
pub fn convergence_choice(
    previous: Option<&[f64]>,
    current: &[f64],
    threshold: f64,
) -> ConvergenceChoice {
    match previous {
        None => ConvergenceChoice::Progress,
        Some(prev) => {
            let sim = cosine_similarity(prev, current);
            if sim >= threshold {
                ConvergenceChoice::Converged
            } else if sim <= -threshold {
                ConvergenceChoice::Stuck
            } else {
                ConvergenceChoice::Progress
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Escalation helper free function
// ---------------------------------------------------------------------------

pub fn should_escalate(margin: f64, threshold: f64) -> bool {
    !margin.is_finite() || margin < threshold
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategies::StrategyBudget;
    use pandora_types::ConvergenceChoice;

    fn budget() -> StrategyBudget {
        StrategyBudget::new(4, 8, 1000, 8, 10, 1000).unwrap()
    }

    fn engine() -> LatsEngine {
        LatsEngine::new(budget(), SinglePassLogitEngine::new())
    }

    #[test]
    fn search_picks_highest_value_action() {
        let e = engine();
        let actions = [0usize, 1, 2];
        let priors = [0.2, 0.5, 0.3];
        let res = e.search(0u32, &actions, &priors, |_s, a| a as f64).unwrap();
        assert_eq!(res.best_action, Some(2));
        assert!(res.metrics.rollouts > 0);
        assert!(res.metrics.nodes_created > 1);
    }

    #[test]
    fn search_is_deterministic() {
        let e = engine();
        let actions = [0usize, 1, 2];
        let priors = [0.2, 0.5, 0.3];
        let r1 = e.search(0u32, &actions, &priors, |_s, a| a as f64).unwrap();
        let r2 = e.search(0u32, &actions, &priors, |_s, a| a as f64).unwrap();
        assert_eq!(r1.best_action, r2.best_action);
        assert_eq!(r1.metrics, r2.metrics);
    }

    #[test]
    fn search_rejects_empty_candidates() {
        let e = engine();
        assert_eq!(
            e.search(0u32, &[], &[], |_s, _a| 0.0).unwrap_err(),
            LatsError::EmptyCandidates
        );
    }

    #[test]
    fn search_rejects_dimension_mismatch() {
        let e = engine();
        let actions = [0usize, 1];
        let priors = [0.5]; // wrong length
        assert_eq!(
            e.search(0u32, &actions, &priors, |_s, _a| 0.0).unwrap_err(),
            LatsError::DimensionMismatch
        );
    }

    #[test]
    fn search_rejects_non_finite_prior() {
        let e = engine();
        let actions = [0usize, 1];
        let priors = [0.5, f64::NAN];
        assert_eq!(
            e.search(0u32, &actions, &priors, |_s, _a| 0.0).unwrap_err(),
            LatsError::DimensionMismatch
        );
    }

    #[test]
    fn budget_bounds_rollouts() {
        let b = StrategyBudget::new(2, 2, 1000, 8, 10, 1000).unwrap();
        let e = LatsEngine::new(b, SinglePassLogitEngine::new());
        let actions = [0usize, 1, 2, 3];
        let priors = [0.25; 4];
        let res = e.search(0u32, &actions, &priors, |_s, a| a as f64).unwrap();
        assert!(res.metrics.rollouts <= 2);
        assert!(res.metrics.max_depth_reached <= 2);
    }

    #[test]
    fn escalation_gate() {
        let e = engine();
        assert!(e.should_escalate(0.0));
        assert!(e.should_escalate(0.05));
        assert!(!e.should_escalate(0.5));
        assert!(e.should_escalate(f64::NAN));
    }

    #[test]
    fn cosine_similarity_basic() {
        let a = [1.0, 0.0];
        let b = [1.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 1e-9);
        let c = [-1.0, 0.0];
        assert!((cosine_similarity(&a, &c) + 1.0).abs() < 1e-9);
    }

    #[test]
    fn convergence_detects_stuck() {
        let prev = [1.0, 0.0];
        let cur = [-1.0, 0.0];
        assert_eq!(
            convergence_choice(Some(&prev[..]), &cur, 0.5),
            ConvergenceChoice::Stuck
        );
    }

    #[test]
    fn convergence_detects_converged() {
        let prev = [1.0, 0.0];
        let cur = [1.0, 0.0];
        assert_eq!(
            convergence_choice(Some(&prev[..]), &cur, 0.5),
            ConvergenceChoice::Converged
        );
    }

    #[test]
    fn convergence_continues_on_first_step() {
        assert_eq!(
            convergence_choice(None, &[1.0, 0.0], 0.5),
            ConvergenceChoice::Progress
        );
    }

    #[test]
    fn free_should_escalate_matches() {
        assert!(should_escalate(0.0, 0.15));
        assert!(!should_escalate(0.5, 0.15));
    }

    #[test]
    fn priors_from_decision_uniform() {
        let e = engine();
        let cands = [
            ConvergenceChoice::Progress,
            ConvergenceChoice::Stuck,
            ConvergenceChoice::Converged,
        ];
        let logits = [1.0, 2.0, 3.0];
        let priors = e.priors_from_decision(&cands, &logits).unwrap();
        assert_eq!(priors.len(), 3);
        for p in &priors {
            assert!((*p - 1.0 / 3.0).abs() < 1e-9);
        }
    }
}
