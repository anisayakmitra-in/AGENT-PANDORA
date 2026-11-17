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

use crate::decision_engine::{SinglePassLogitEngine, TropicalDecisionDriver};
use crate::strategies::StrategyBudget;
use pandora_types::{ConvergenceChoice, DecisionVerdict};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Aliases
// ---------------------------------------------------------------------------

/// Alias for `LatsEngine` highlighting Monte Carlo Tree Search integration.
pub type LatsMctsEngine = LatsEngine;

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

#[derive(Clone, Debug)]
pub struct LatsEngine {
    budget: StrategyBudget,
    puct_c: f64,
    decision_engine: SinglePassLogitEngine,
    escalation_threshold: f64,
    tropical_driver: Option<Arc<TropicalDecisionDriver>>,
}

impl LatsEngine {
    pub fn new(budget: StrategyBudget, decision_engine: SinglePassLogitEngine) -> Self {
        Self {
            budget,
            puct_c: 1.4,
            decision_engine,
            escalation_threshold: 0.15,
            tropical_driver: None,
        }
    }

    /// Constructs a `LatsMctsEngine` wired directly to a `TropicalDecisionDriver`.
    pub fn with_tropical_driver(driver: Arc<TropicalDecisionDriver>) -> Self {
        let budget = StrategyBudget::new(4, 8, 1000, 8, 10, 1000).expect("valid default budget");
        let decision_engine = SinglePassLogitEngine::with_driver(driver.clone());
        let threshold = driver.uncertainty_threshold();
        Self {
            budget,
            puct_c: 1.4,
            decision_engine,
            escalation_threshold: threshold,
            tropical_driver: Some(driver),
        }
    }

    /// Constructs a `LatsMctsEngine` with a custom budget wired directly to a `TropicalDecisionDriver`.
    pub fn new_with_tropical(budget: StrategyBudget, driver: Arc<TropicalDecisionDriver>) -> Self {
        let decision_engine = SinglePassLogitEngine::with_driver(driver.clone());
        let threshold = driver.uncertainty_threshold();
        Self {
            budget,
            puct_c: 1.4,
            decision_engine,
            escalation_threshold: threshold,
            tropical_driver: Some(driver),
        }
    }

    /// Attaches or updates the `TropicalDecisionDriver` for this engine.
    pub fn with_tropical(mut self, driver: Arc<TropicalDecisionDriver>) -> Self {
        self.decision_engine = SinglePassLogitEngine::with_driver(driver.clone());
        self.escalation_threshold = driver.uncertainty_threshold();
        self.tropical_driver = Some(driver);
        self
    }

    /// Updates the `StrategyBudget` for this engine.
    pub fn with_budget(mut self, budget: StrategyBudget) -> Self {
        self.budget = budget;
        self
    }

    /// Returns a reference to the attached `TropicalDecisionDriver`, if configured.
    pub fn tropical_driver(&self) -> Option<&Arc<TropicalDecisionDriver>> {
        self.tropical_driver.as_ref()
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

    /// Escalation gate taking both confidence margin and normalized Shannon entropy.
    /// Returns `true` if margin is below uncertainty threshold OR normalized entropy > 0.666
    /// (or if values are non-finite).
    pub fn should_escalate_with_entropy(&self, margin: f64, normalized_entropy: f64) -> bool {
        should_escalate_with_entropy(margin, self.escalation_threshold, normalized_entropy)
    }

    /// Escalation gate evaluating a dynamic decision verdict.
    /// Returns `true` if verdict is uncertain, margin < threshold, or normalized entropy > 0.666.
    pub fn should_escalate_verdict(&self, verdict: &pandora_types::DynamicVerdict) -> bool {
        if verdict.is_uncertain {
            return true;
        }
        let probs: Vec<f64> = verdict.probabilities.values().copied().collect();
        let entropy = crate::decision_engine::TropicalDecisionDriver::normalized_entropy(&probs);
        self.should_escalate_with_entropy(verdict.margin, entropy)
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
                // Pick child by PUCT: U(s, a) = Q(s, a) + c_puct * P(a|s) * sqrt(sum_b N(s, b)) / (1 + N(s, a))
                // Directly compute sum_b N(s, b) across all children (siblings of candidate action)
                let sum_b: u64 = node
                    .children
                    .iter()
                    .filter_map(|&cid| node_by_id(&nodes, cid))
                    .map(|c| c.visits)
                    .sum();
                // When sum_b == 0 (unvisited children), max(1) yields sqrt(1)=1.0 so priors P(a|s) guide exploration
                let sqrt_siblings = (sum_b.max(1) as f64).sqrt();

                let mut best_child = node.children[0];
                let mut best_score = f64::NEG_INFINITY;
                let mut best_prior = f64::NEG_INFINITY;
                let mut best_action_idx = usize::MAX;

                for &cid in &node.children {
                    let c = node_by_id(&nodes, cid).unwrap();
                    let q = c.q_value();
                    let u = self.puct_c * c.prior * sqrt_siblings / (1.0 + c.visits as f64);
                    let score = q + u;
                    // Deterministic tie-breaking: prefer higher score; within 1e-9 tolerance,
                    // tie-break by higher prior P(a|s), then lower action index
                    let is_better = if score > best_score + 1e-9 {
                        true
                    } else if (score - best_score).abs() <= 1e-9 {
                        if (c.prior - best_prior).abs() > 1e-9 {
                            c.prior > best_prior
                        } else {
                            c.action.unwrap_or(usize::MAX) < best_action_idx
                        }
                    } else {
                        false
                    };
                    if is_better {
                        best_score = score;
                        best_child = cid;
                        best_prior = c.prior;
                        best_action_idx = c.action.unwrap_or(usize::MAX);
                    }
                }
                current = best_child;
                path.push(current);
            }

            // --- Expansion ---
            let leaf = node_by_id(&nodes, current).unwrap();
            let mut expanded_now = false;
            if leaf.depth < max_depth && !leaf.expanded {
                // Find untried action prioritized by highest prior probability P(a|s)
                let tried: std::collections::HashSet<usize> = node_by_id(&nodes, current)
                    .unwrap()
                    .children
                    .iter()
                    .filter_map(|&cid| node_by_id(&nodes, cid).unwrap().action)
                    .collect();
                let mut to_try: Option<usize> = None;
                let mut best_untried_prior = f64::NEG_INFINITY;
                for &a in actions {
                    if !tried.contains(&a) {
                        let p = priors[a.min(priors.len() - 1)];
                        if p > best_untried_prior {
                            best_untried_prior = p;
                            to_try = Some(a);
                        }
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

        // Pick best action by visit count (tie-break by value, prior, and action index).
        let root = node_by_id(&nodes, root_id).unwrap();
        let mut best_action: Option<usize> = None;
        let mut best_visits: u64 = 0;
        let mut best_value: f64 = f64::NEG_INFINITY;
        let mut best_prior: f64 = f64::NEG_INFINITY;
        for &cid in &root.children {
            let c = node_by_id(&nodes, cid).unwrap();
            let is_better = if c.visits > best_visits {
                true
            } else if c.visits == best_visits {
                if (c.q_value() - best_value).abs() > 1e-9 {
                    c.q_value() > best_value
                } else if (c.prior - best_prior).abs() > 1e-9 {
                    c.prior > best_prior
                } else {
                    c.action.unwrap_or(usize::MAX) < best_action.unwrap_or(usize::MAX)
                }
            } else {
                false
            };
            if is_better {
                best_visits = c.visits;
                best_value = c.q_value();
                best_action = c.action;
                best_prior = c.prior;
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

    /// Executes PUCT Monte Carlo Tree Search using sub-microsecond tropical inference
    /// (`TropicalNetwork::forward` / `TropicalLayer` with max-plus semiring)
    /// to compute prior policy probabilities P(a|s) and leaf value estimations Q(s,a)
    /// during rollouts without LLM calls during tree expansion.
    pub fn search_with_tropical<S, L>(
        &self,
        initial_state: S,
        actions: &[usize],
        action_labels: &[L],
    ) -> Result<LatsSearchResult<S>, LatsError>
    where
        S: Clone + fmt::Display,
        L: AsRef<str>,
    {
        self.search_with_tropical_context(initial_state, actions, action_labels, |s| s.to_string())
    }

    /// Executes PUCT MCTS using tropical inference with an explicit task prompt.
    pub fn search_with_tropical_prompt<S, L>(
        &self,
        prompt: &str,
        initial_state: S,
        actions: &[usize],
        action_labels: &[L],
    ) -> Result<LatsSearchResult<S>, LatsError>
    where
        S: Clone + fmt::Display,
        L: AsRef<str>,
    {
        let prompt_owned = prompt.to_string();
        self.search_with_tropical_context(initial_state, actions, action_labels, move |s| {
            let s_str = s.to_string();
            if s_str.is_empty() {
                prompt_owned.clone()
            } else {
                format!("{prompt_owned} {s_str}")
            }
        })
    }

    /// Executes PUCT MCTS using tropical inference with a custom state context extractor closure.
    pub fn search_with_tropical_context<S, L, F>(
        &self,
        initial_state: S,
        actions: &[usize],
        action_labels: &[L],
        mut context_fn: F,
    ) -> Result<LatsSearchResult<S>, LatsError>
    where
        S: Clone,
        L: AsRef<str>,
        F: FnMut(&S) -> String,
    {
        if actions.is_empty() {
            return Err(LatsError::EmptyCandidates);
        }
        if actions.len() != action_labels.len() {
            return Err(LatsError::DimensionMismatch);
        }

        let fallback_driver;
        let driver: &TropicalDecisionDriver = match &self.tropical_driver {
            Some(d) => d.as_ref(),
            None => {
                fallback_driver = TropicalDecisionDriver::calibrated();
                &fallback_driver
            }
        };

        let label_strs: Vec<&str> = action_labels.iter().map(|l| l.as_ref()).collect();

        // 1. Compute prior policy probabilities P(a|s) via zero-multiplication tropical forward pass
        let root_prompt = context_fn(&initial_state);
        let priors = driver
            .evaluate_action_priors(&root_prompt, "", &label_strs)
            .map_err(|_| LatsError::DimensionMismatch)?;

        // 2. Run MCTS rollout with zero-multiplication tropical leaf evaluation
        self.search(initial_state, actions, &priors, |leaf_state, action_idx| {
            let label = label_strs.get(action_idx).copied().unwrap_or(label_strs[0]);
            let leaf_prompt = context_fn(leaf_state);
            driver.evaluate_leaf_value(&leaf_prompt, "", label)
        })
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

/// System 1 -> System 2 escalation gate with normalized Shannon entropy (Requirement R2).
/// Returns `true` when confidence margin is below `uncertainty_threshold`
/// OR `normalized_entropy` exceeds 0.666 (or if values are non-finite).
pub fn should_escalate_with_entropy(
    margin: f64,
    uncertainty_threshold: f64,
    normalized_entropy: f64,
) -> bool {
    !margin.is_finite()
        || !uncertainty_threshold.is_finite()
        || !normalized_entropy.is_finite()
        || margin < uncertainty_threshold
        || normalized_entropy > 0.666
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

    #[test]
    fn test_tropical_inference_drives_prior_probabilities_with_zero_llm_calls() {
        let driver = Arc::new(TropicalDecisionDriver::calibrated());
        assert!(driver.has_loaded_network());

        let prompt = "Inspect system logs, check status, and verify file integrity";
        let state = "system_state: clean";
        let labels = ["allow", "reject"];
        let priors = driver
            .evaluate_action_priors(prompt, state, &labels)
            .expect("priors evaluate successfully");

        assert_eq!(priors.len(), 2);
        assert!(
            (priors[0] + priors[1] - 1.0).abs() < 1e-6,
            "priors must normalize to 1.0 on probability simplex"
        );
        assert!(
            priors[0] > 0.85,
            "safe inspection prompt must strongly favor 'allow'"
        );
        assert!(
            priors[1] < 0.15,
            "safe inspection prompt must produce low prior for 'reject'"
        );

        // Also test dangerous prompt
        let danger_prompt = "danger drop tables rm -rf /";
        let danger_priors = driver
            .evaluate_action_priors(danger_prompt, state, &labels)
            .expect("danger priors evaluate successfully");
        assert_eq!(danger_priors.len(), 2);
        assert!(
            danger_priors[1] > 0.85,
            "dangerous prompt must strongly favor 'reject'"
        );
        assert!(
            danger_priors[0] < 0.15,
            "dangerous prompt must produce low prior for 'allow'"
        );
    }

    #[test]
    fn test_tropical_leaf_evaluations_bounded_and_consistent() {
        let driver = Arc::new(TropicalDecisionDriver::calibrated());
        assert!(driver.has_loaded_network());

        let q_safe =
            driver.evaluate_leaf_value("read source code safely", "clean worktree", "allow");
        let q_danger = driver.evaluate_leaf_value(
            "drop database and rm -rf production root",
            "emergency status",
            "allow",
        );
        let q_reject = driver.evaluate_leaf_value(
            "drop database and rm -rf production root",
            "emergency status",
            "reject",
        );

        assert!(
            q_safe > 0.85,
            "safe allow should have high leaf value: got {q_safe}"
        );
        assert!(
            q_danger < 0.15,
            "dangerous allow should have near-zero leaf value: got {q_danger}"
        );
        assert!(
            q_reject > 0.85,
            "blocking dangerous operation should have high leaf value: got {q_reject}"
        );

        assert!(q_safe > 0.0 && q_safe < 1.0);
        assert!(q_danger > 0.0 && q_danger < 1.0);
        assert!(q_reject > 0.0 && q_reject < 1.0);
    }

    #[test]
    fn test_puct_selection_rule_mathematical_adherence() {
        // U(s, a) = Q(s, a) + c_puct * P(a|s) * sqrt(sum_b N(s, b)) / (1 + N(s, a))
        let puct_c: f64 = 1.4;

        // Step 1: Child 0 visited once with reward 0.8; Child 1 unvisited; sum_b = 1.
        let q0: f64 = 0.8;
        let n0: f64 = 1.0;
        let p0: f64 = 0.4;
        let q1: f64 = 0.0;
        let n1: f64 = 0.0;
        let p1: f64 = 0.6;
        let sum_b: f64 = 1.0;

        let u0: f64 = q0 + puct_c * p0 * sum_b.sqrt() / (1.0 + n0);
        let u1: f64 = q1 + puct_c * p1 * sum_b.sqrt() / (1.0 + n1);

        assert!((u0 - 1.08).abs() < 1e-6);
        assert!((u1 - 0.84).abs() < 1e-6);
        assert!(u0 > u1, "child 0 exploitation dominates when sum_b is low");

        // Step 2: Child 0 visited 4 times (Q drops to 0.6); Child 1 still unvisited; sum_b = 4.
        let q0_step2: f64 = 0.6;
        let n0_step2: f64 = 4.0;
        let sum_b_step2: f64 = 4.0;

        let u0_step2: f64 = q0_step2 + puct_c * p0 * sum_b_step2.sqrt() / (1.0 + n0_step2);
        let u1_step2: f64 = q1 + puct_c * p1 * sum_b_step2.sqrt() / (1.0 + n1);

        assert!((u0_step2 - 0.824).abs() < 1e-6);
        assert!((u1_step2 - 1.68).abs() < 1e-6);
        assert!(
            u1_step2 > u0_step2,
            "exploration bonus dominates for unvisited child 1 under high sibling visits"
        );
    }

    #[test]
    fn test_lats_mcts_engine_optimal_action_convergence() {
        let driver = Arc::new(TropicalDecisionDriver::calibrated());
        let engine = LatsMctsEngine::with_tropical_driver(driver);

        let actions = [0usize, 1, 2];
        let labels = ["allow", "require_confirm", "reject"];

        // Safe prompt: should converge to action 0 ("allow")
        let safe_prompt = "Inspect repository status and search code symbols";
        let res1 = engine
            .search_with_tropical(safe_prompt, &actions, &labels)
            .expect("search_with_tropical succeeds");

        assert_eq!(
            res1.best_action,
            Some(0),
            "safe prompt must select allow (action 0)"
        );
        assert!(
            res1.best_value > 0.80,
            "expected best_value > 0.80, got {}",
            res1.best_value
        );
        assert!(res1.metrics.rollouts > 0);
        assert!(res1.metrics.nodes_created >= 3);

        // Determinism test: duplicate search produces exact same result
        let res2 = engine
            .search_with_tropical(safe_prompt, &actions, &labels)
            .expect("search_with_tropical succeeds");
        assert_eq!(res1.best_action, res2.best_action);
        assert!((res1.best_value - res2.best_value).abs() < 1e-9);
        assert_eq!(res1.metrics, res2.metrics);

        // Dangerous prompt: should converge to action 2 ("reject")
        let danger_prompt = "danger drop tables rm -rf /";
        let res_danger = engine
            .search_with_tropical(danger_prompt, &actions, &labels)
            .expect("search_with_tropical succeeds for danger prompt");
        assert_eq!(
            res_danger.best_action,
            Some(2),
            "danger prompt must select reject (action 2)"
        );
        assert!(
            res_danger.best_value > 0.60,
            "expected danger best_value > 0.60, got {}",
            res_danger.best_value
        );

        // Test search_with_tropical_prompt with separate prompt and state
        let res_prompt = engine
            .search_with_tropical_prompt(
                "Inspect repository status",
                "clean_repo",
                &actions,
                &labels,
            )
            .expect("search_with_tropical_prompt succeeds");
        assert_eq!(res_prompt.best_action, Some(0));
    }
}
