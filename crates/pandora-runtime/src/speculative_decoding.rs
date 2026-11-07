//! Phase 8: Speculative Token Decoding & Prefix-Tree KV Cache Compression.
//!
//! Provides mathematically sound speculative draft verification preserving exact target
//! model distribution (Theorem 13), theoretical speedup modeling (Theorem 14), and
//! prefix-tree Radix KV cache compression with $O(1)$ memory prefix sharing (Theorem 15).

#![forbid(unsafe_code)]

use crate::bandit_router::SafePrng;
use std::collections::BTreeMap;

/// A discrete token ID paired with its categorical probability $P(x_i)$.
#[derive(Clone, Debug, PartialEq)]
pub struct TokenProbability {
    /// Numerical token identifier.
    pub token_id: u32,
    /// Probability mass assigned to this token, $p \in [0.0, 1.0]$.
    pub prob: f64,
}

impl TokenProbability {
    /// Creates a new token probability pair.
    pub fn new(token_id: u32, prob: f64) -> Self {
        Self {
            token_id,
            prob: prob.clamp(0.0, 1.0),
        }
    }
}

/// The outcome of speculative draft verification by the target model.
#[derive(Clone, Debug, PartialEq)]
pub struct DraftVerificationResult {
    /// Tokens accepted by the rejection sampling criterion.
    pub accepted_tokens: Vec<u32>,
    /// Bonus token sampled either from target distribution (on complete draft acceptance)
    /// or from the residual distribution $P'(x) = \frac{\max(0, P - Q)}{1 - \sum \min(P, Q)}$ (on rejection).
    pub bonus_token: Option<u32>,
    /// Total tokens emitted this inference cycle: $|\text{accepted}| + \mathbb{I}\{\text{bonus}\}$.
    pub total_emitted: usize,
    /// Empirical acceptance rate $\hat{\alpha} = \frac{|\text{accepted}|}{\gamma}$.
    pub acceptance_rate: f64,
}

/// Speculative decoding engine executing rejection sampling with exact target distribution preservation.
#[derive(Clone, Debug)]
pub struct SpeculativeEngine {
    prng: SafePrng,
}

impl Default for SpeculativeEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SpeculativeEngine {
    /// Creates a speculative decoding engine with default seed.
    pub fn new() -> Self {
        Self::with_seed(0xfeed_face_cafe_beef)
    }

    /// Creates a speculative decoding engine with a deterministic seed.
    pub fn with_seed(seed: u64) -> Self {
        Self {
            prng: SafePrng::new(seed),
        }
    }

    /// Samples a token from a normalized categorical probability distribution.
    pub fn sample_from_distribution(&mut self, dist: &[TokenProbability]) -> Option<u32> {
        if dist.is_empty() {
            return None;
        }

        let total_mass: f64 = dist.iter().map(|tp| tp.prob).sum();
        if total_mass <= 1e-15 {
            return Some(dist[0].token_id);
        }

        let mut roll = self.prng.sample_uniform_f64() * total_mass;
        for tp in dist {
            if roll <= tp.prob {
                return Some(tp.token_id);
            }
            roll -= tp.prob;
        }

        dist.last().map(|tp| tp.token_id)
    }

    /// Verifies speculative draft tokens against target model distributions using rejection sampling.
    ///
    /// Preserves exact target distribution equivalence (Theorem 13):
    /// - For each draft token $\tilde{x}_i$:
    ///   - $q = Q(\tilde{x}_i)$, $p = P(\tilde{x}_i)$
    ///   - Acceptance probability $\alpha_i = \min\left(1, \frac{p}{q}\right)$
    ///   - If $U \le \alpha_i$: accept $\tilde{x}_i$.
    ///   - If $U > \alpha_i$: reject $\tilde{x}_i$, sample bonus token from residual
    ///     $P'(x) = \frac{\max(0, P(x) - Q(x))}{\sum_z \max(0, P(z) - Q(z))}$, and terminate.
    /// - If all draft tokens are accepted, bonus token is sampled from the target distribution at position $\gamma$.
    pub fn verify_draft(
        &mut self,
        draft_tokens: &[u32],
        draft_dists: &[Vec<TokenProbability>],
        target_dists: &[Vec<TokenProbability>],
    ) -> DraftVerificationResult {
        let gamma = draft_tokens.len();
        let mut accepted = Vec::new();
        let mut bonus_token = None;

        for (i, &tok) in draft_tokens.iter().enumerate() {
            let q = draft_dists
                .get(i)
                .and_then(|d| d.iter().find(|tp| tp.token_id == tok).map(|tp| tp.prob))
                .unwrap_or(1e-6);

            let p = target_dists
                .get(i)
                .and_then(|d| d.iter().find(|tp| tp.token_id == tok).map(|tp| tp.prob))
                .unwrap_or(0.0);

            let alpha = if q <= 1e-15 { 1.0 } else { (p / q).min(1.0) };

            let u = self.prng.sample_uniform_f64();
            if u <= alpha {
                accepted.push(tok);
            } else {
                // Rejection: sample bonus token from positive residual P'(x) = max(0, P(x) - Q(x))
                if let (Some(t_dist), Some(d_dist)) = (target_dists.get(i), draft_dists.get(i)) {
                    let mut residual = Vec::new();
                    for tp_t in t_dist {
                        let q_prob = d_dist
                            .iter()
                            .find(|tp_d| tp_d.token_id == tp_t.token_id)
                            .map(|tp_d| tp_d.prob)
                            .unwrap_or(0.0);
                        let diff = (tp_t.prob - q_prob).max(0.0);
                        if diff > 1e-9 {
                            residual.push(TokenProbability::new(tp_t.token_id, diff));
                        }
                    }
                    bonus_token = self.sample_from_distribution(&residual);
                }
                break;
            }
        }

        // If all gamma tokens were accepted, sample bonus token from target model at step gamma
        if accepted.len() == gamma && target_dists.len() > gamma {
            bonus_token = self.sample_from_distribution(&target_dists[gamma]);
        }

        let acceptance_rate = if gamma > 0 {
            accepted.len() as f64 / gamma as f64
        } else {
            1.0
        };

        let total_emitted = accepted.len() + if bonus_token.is_some() { 1 } else { 0 };

        DraftVerificationResult {
            accepted_tokens: accepted,
            bonus_token,
            total_emitted,
            acceptance_rate,
        }
    }

    /// Evaluates the theoretical expected accepted tokens (Theorem 14):
    /// $$\mathbb{E}[\beta] = \sum_{i=0}^\gamma \alpha^i = \frac{1 - \alpha^{\gamma+1}}{1 - \alpha}$$
    pub fn theoretical_expected_tokens(alpha: f64, gamma: usize) -> f64 {
        let a = alpha.clamp(0.0, 1.0);
        if (a - 1.0).abs() < 1e-9 {
            (gamma + 1) as f64
        } else {
            (1.0 - a.powi(gamma as i32 + 1)) / (1.0 - a)
        }
    }

    /// Evaluates the theoretical speculative speedup ratio (Theorem 14):
    /// $$S(\alpha, \gamma, r) = \frac{\mathbb{E}[\beta]}{\gamma r + 1}$$
    /// where $r = t_{\text{draft}} / t_{\text{target}}$ is the draft latency ratio.
    pub fn theoretical_speedup(alpha: f64, gamma: usize, latency_ratio: f64) -> f64 {
        let expected_beta = Self::theoretical_expected_tokens(alpha, gamma);
        let denominator = (gamma as f64) * latency_ratio + 1.0;
        if denominator <= 0.0 {
            1.0
        } else {
            expected_beta / denominator
        }
    }

    /// Finds the optimal draft window length $\gamma^*$ maximizing speedup $S(\alpha, \gamma, r)$.
    pub fn optimal_draft_length(alpha: f64, latency_ratio: f64, max_gamma: usize) -> (usize, f64) {
        let mut best_gamma = 1;
        let mut best_speedup = Self::theoretical_speedup(alpha, 1, latency_ratio);

        for g in 2..=max_gamma {
            let s = Self::theoretical_speedup(alpha, g, latency_ratio);
            if s > best_speedup {
                best_speedup = s;
                best_gamma = g;
            }
        }

        (best_gamma, best_speedup)
    }
}

/// A node in the Radix Trie prefix tree for compressed KV cache sharing.
#[derive(Clone, Debug, PartialEq)]
pub struct RadixKvNode {
    /// Token sequence segment represented by this edge.
    pub prefix: Vec<u32>,
    /// Number of active sessions referencing this node.
    pub ref_count: usize,
    /// Size of cached KV activations for this prefix slice in bytes.
    pub kv_bytes: usize,
    /// Child edges mapped by the first token of their prefix.
    pub children: BTreeMap<u32, RadixKvNode>,
}

impl RadixKvNode {
    /// Creates a new Radix KV node with the specified token prefix and KV byte size.
    pub fn new(prefix: Vec<u32>, kv_bytes: usize) -> Self {
        Self {
            prefix,
            ref_count: 1,
            kv_bytes,
            children: BTreeMap::new(),
        }
    }

    /// Counts the total number of unique nodes in this subtree.
    pub fn count_nodes(&self) -> usize {
        1 + self
            .children
            .values()
            .map(|c| c.count_nodes())
            .sum::<usize>()
    }

    /// Counts the total number of tokens stored across all edges in this subtree.
    pub fn count_tokens(&self) -> usize {
        self.prefix.len()
            + self
                .children
                .values()
                .map(|c| c.count_tokens())
                .sum::<usize>()
    }

    /// Sums total allocated KV cache bytes across this subtree.
    pub fn total_kv_bytes(&self) -> usize {
        self.kv_bytes
            + self
                .children
                .values()
                .map(|c| c.total_kv_bytes())
                .sum::<usize>()
    }
}

/// Prefix-Tree (Radix Trie) KV Cache manager with $O(1)$ memory sharing across common prompt prefixes.
#[derive(Clone, Debug)]
pub struct RadixKvCache {
    root: RadixKvNode,
    bytes_per_token: usize,
    session_tokens: BTreeMap<String, Vec<u32>>,
    total_tokens_requested: usize,
    total_tokens_shared: usize,
}

impl Default for RadixKvCache {
    fn default() -> Self {
        Self::new(2048) // Default 2048 bytes per token (e.g. 16 layers * 2(K,V) * 64 heads)
    }
}

impl RadixKvCache {
    /// Creates a new Radix KV cache specifying the memory footprint per token in bytes.
    pub fn new(bytes_per_token: usize) -> Self {
        Self {
            root: RadixKvNode {
                prefix: Vec::new(),
                ref_count: 1,
                kv_bytes: 0,
                children: BTreeMap::new(),
            },
            bytes_per_token,
            session_tokens: BTreeMap::new(),
            total_tokens_requested: 0,
            total_tokens_shared: 0,
        }
    }

    /// Returns the bytes allocated per token.
    pub const fn bytes_per_token(&self) -> usize {
        self.bytes_per_token
    }

    /// Returns the total active sessions currently cached.
    pub fn active_sessions(&self) -> usize {
        self.session_tokens.len()
    }

    /// Looks up the length of the longest matching prefix already resident in cache.
    pub fn lookup_shared_prefix(&self, tokens: &[u32]) -> usize {
        let mut matched = 0;
        let mut current = &self.root;
        let mut remaining = tokens;

        while !remaining.is_empty() {
            let first = remaining[0];
            if let Some(child) = current.children.get(&first) {
                let common_len = remaining
                    .iter()
                    .zip(child.prefix.iter())
                    .take_while(|(a, b)| a == b)
                    .count();

                if common_len == child.prefix.len() {
                    // Full child prefix matched, continue traversal
                    matched += common_len;
                    remaining = &remaining[common_len..];
                    current = child;
                } else {
                    // Partial match on edge
                    matched += common_len;
                    break;
                }
            } else {
                break;
            }
        }

        matched
    }

    /// Inserts a sequence of prompt tokens for a session, reusing shared prefix KV blocks.
    ///
    /// Returns `(shared_prefix_tokens, newly_allocated_tokens)`.
    pub fn insert(&mut self, session_id: &str, tokens: &[u32]) -> (usize, usize) {
        if tokens.is_empty() {
            return (0, 0);
        }

        let shared_len = self.lookup_shared_prefix(tokens);
        let newly_allocated = tokens.len().saturating_sub(shared_len);

        self.total_tokens_requested += tokens.len();
        self.total_tokens_shared += shared_len;
        self.session_tokens
            .insert(session_id.to_string(), tokens.to_vec());

        Self::insert_into_node(&mut self.root, tokens, self.bytes_per_token);

        (shared_len, newly_allocated)
    }

    fn insert_into_node(current: &mut RadixKvNode, tokens: &[u32], bytes_per_token: usize) {
        if tokens.is_empty() {
            return;
        }

        let first = tokens[0];

        if let Some(child) = current.children.get_mut(&first) {
            let common_len = tokens
                .iter()
                .zip(child.prefix.iter())
                .take_while(|(a, b)| a == b)
                .count();

            if common_len == child.prefix.len() {
                // Whole child prefix matches, increment ref_count and descend
                child.ref_count += 1;
                Self::insert_into_node(child, &tokens[common_len..], bytes_per_token);
            } else {
                // Branch split needed: child prefix splits at common_len
                let original_prefix = child.prefix.clone();
                let original_children = std::mem::take(&mut child.children);
                let original_ref_count = child.ref_count;

                // Truncate child to common prefix
                child.prefix = original_prefix[..common_len].to_vec();
                child.ref_count += 1;
                child.kv_bytes = common_len * bytes_per_token;

                // Split edge: branch for existing child suffix
                let split_suffix = original_prefix[common_len..].to_vec();
                let split_first = split_suffix[0];
                let split_node = RadixKvNode {
                    prefix: split_suffix.clone(),
                    ref_count: original_ref_count,
                    kv_bytes: split_suffix.len() * bytes_per_token,
                    children: original_children,
                };
                child.children.insert(split_first, split_node);

                // Split edge: branch for new incoming tokens suffix
                let new_suffix = tokens[common_len..].to_vec();
                if !new_suffix.is_empty() {
                    let new_first = new_suffix[0];
                    let new_node =
                        RadixKvNode::new(new_suffix.clone(), new_suffix.len() * bytes_per_token);
                    child.children.insert(new_first, new_node);
                }
            }
        } else {
            // No existing branch starts with this token: create new child node
            let new_node = RadixKvNode::new(tokens.to_vec(), tokens.len() * bytes_per_token);
            current.children.insert(first, new_node);
        }
    }

    /// Releases a session's KV cache allocation, pruning nodes whose reference count drops to 0.
    ///
    /// Returns the number of tokens freed.
    pub fn release(&mut self, session_id: &str) -> usize {
        let Some(tokens) = self.session_tokens.remove(session_id) else {
            return 0;
        };

        Self::decrement_node(&mut self.root, &tokens);
        tokens.len()
    }

    fn decrement_node(current: &mut RadixKvNode, tokens: &[u32]) {
        if tokens.is_empty() {
            return;
        }

        let first = tokens[0];
        let mut prune = false;

        if let Some(child) = current.children.get_mut(&first) {
            let common_len = tokens
                .iter()
                .zip(child.prefix.iter())
                .take_while(|(a, b)| a == b)
                .count();

            if child.ref_count > 1 {
                child.ref_count -= 1;
            } else {
                prune = true;
            }

            if !prune && common_len < tokens.len() {
                Self::decrement_node(child, &tokens[common_len..]);
            }
        }

        if prune {
            current.children.remove(&first);
        }
    }

    /// Computes the empirical memory compression ratio:
    /// $$C_R = \frac{\text{Dense Uncompressed Space}}{\text{Compressed Radix Space}}$$
    pub fn compression_ratio(&self) -> f64 {
        let dense_tokens: usize = self.session_tokens.values().map(|v| v.len()).sum();
        let compressed_tokens = self.root.count_tokens();

        if compressed_tokens == 0 {
            1.0
        } else {
            (dense_tokens as f64) / (compressed_tokens as f64)
        }
    }

    /// Returns theoretical Radix compression ratio for $M$ sessions and prefix fraction $\eta$ (Theorem 15):
    /// $$C_R(M, \eta) = \frac{1}{(1 - \eta) + \frac{\eta}{M}}$$
    pub fn theoretical_compression_ratio(sessions: usize, prefix_fraction: f64) -> f64 {
        if sessions == 0 {
            return 1.0;
        }
        let m = sessions as f64;
        let eta = prefix_fraction.clamp(0.0, 0.999);
        1.0 / ((1.0 - eta) + (eta / m))
    }

    /// Returns asymptotic Radix compression ratio as $M \to \infty$ (Theorem 15):
    /// $$\lim_{M \to \infty} C_R(M, \eta) = \frac{1}{1 - \eta}$$
    pub fn asymptotic_compression_ratio(prefix_fraction: f64) -> f64 {
        let eta = prefix_fraction.clamp(0.0, 0.999);
        1.0 / (1.0 - eta)
    }
}
