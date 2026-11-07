# Mathematical Foundations & Architecture: Speculative Token Decoding & Prefix-Tree KV Cache Compression

**Author:** ANISAYAK MITRA <266799942+anisayakmitra-in@users.noreply.github.com>  
**Version:** AGENT-PANDORA v3.0.4 (Phase 8)  
**Safety Invariant:** `#![forbid(unsafe_code)]` Strictly Enforced  

---

## 1. Executive Summary & Latency / Memory Bottlenecks

In autonomous agent systems, LLM inference suffers from two fundamental hardware bottlenecks:
1. **Autoregressive Memory-Bandwidth Stall**: Generating $N$ tokens autoregressively requires $N$ sequential forward passes through multi-billion parameter neural networks, transferring gigabytes of weights from High-Bandwidth Memory (HBM) to compute ALUs for every single token generated ($I \approx 0.05\text{ FLOP/byte}$).
2. **Key-Value (KV) Cache Memory Bloat**: Multi-turn agent loops retain long system prompts, tool schemas, and environment contexts across $M$ concurrent sessions. Storing duplicate dense KV tensors for identical prefix tokens exhausts GPU/host RAM ($O(M \cdot L)$ memory growth).

Phase 8 breaks both bottlenecks by combining:
- **Speculative Draft Decoding**: A lightweight draft model proposes $\gamma$ candidate tokens, verified concurrently in a single batched forward pass by the target model via modified rejection sampling.
- **Exact Distribution Invariance (Theorem 14)**: Mathematically guarantees zero quality loss—the sampled output distribution is identically equal to native target model sampling ($P_{\text{spec}}(x) \equiv P(x)$).
- **Theoretical Speedup Bound (Theorem 15)**: Expected acceptance $\mathbb{E}[\beta] = \frac{1 - \alpha^{\gamma+1}}{1 - \alpha}$ yielding speedup $S = \frac{\mathbb{E}[\beta]}{\gamma r + 1}$.
- **Radix Trie KV Cache Compression (Theorem 16)**: Eliminates redundant prompt KV allocations with asymptotic compression ratio $C_R \to \frac{1}{1 - \eta}$.

---

## 2. Theorem 14: Exact Distribution Equivalence of Speculative Rejection Sampling

### 2.1 Theorem Statement

**Theorem 14 (Exact Distribution Preservation):**  
*Let $P(x)$ denote the categorical probability distribution of the target model over vocabulary $\mathcal{V}$, and $Q(x)$ denote the proposal distribution of the draft model ($Q(x) > 0$ for all $x \in \mathcal{V}$).  
Suppose a draft token $\tilde{x} \sim Q$ is drawn and accepted with probability:*
$$\alpha(\tilde{x}) = \min\left(1, \frac{P(\tilde{x})}{Q(\tilde{x})}\right)$$
*If rejected, a bonus token $x'$ is sampled from the normalized positive residual distribution:*
$$P'(x) = \frac{\max(0, P(x) - Q(x))}{\sum_{z \in \mathcal{V}} \max(0, P(z) - Q(z))}$$
*Then the marginal probability distribution of the emitted token $X_{\text{spec}}$ is identical to $P(x)$ for all $x \in \mathcal{V}$:*
$$P_{\text{spec}}(x) \equiv P(x)$$
*guaranteeing that speculative decoding produces outputs statistically indistinguishable from the target model with zero degradation in reasoning capability.*

---

### 2.2 Proof

**Step 1: Total Acceptance Probability.**  
The probability that the draft token is accepted under the criterion $\alpha(\tilde{x})$ is:
$$\mathbb{P}(\text{Accept}) = \sum_{x \in \mathcal{V}} Q(x) \alpha(x) = \sum_{x \in \mathcal{V}} Q(x) \min\left(1, \frac{P(x)}{Q(x)}\right) = \sum_{x \in \mathcal{V}} \min(Q(x), P(x))$$

Let $\Omega = \sum_{x \in \mathcal{V}} \min(Q(x), P(x))$ denote the overlapping probability mass between draft and target distributions.  
The probability of rejection is:
$$\mathbb{P}(\text{Reject}) = 1 - \mathbb{P}(\text{Accept}) = 1 - \Omega = 1 - \sum_{x \in \mathcal{V}} \min(Q(x), P(x))$$

**Step 2: Normalization of Residual Distribution $P'(x)$.**  
Using the identity $P(x) - \min(Q(x), P(x)) = \max(0, P(x) - Q(x))$:
$$\sum_{x \in \mathcal{V}} \max(0, P(x) - Q(x)) = \sum_{x \in \mathcal{V}} \left(P(x) - \min(Q(x), P(x))\right) = \sum_{x \in \mathcal{V}} P(x) - \sum_{x \in \mathcal{V}} \min(Q(x), P(x)) = 1 - \Omega$$

Therefore, the normalizing constant of the residual distribution $P'(x)$ is exactly $1 - \Omega$:
$$P'(x) = \frac{\max(0, P(x) - Q(x))}{1 - \Omega}$$

**Step 3: Marginal Distribution Derivation.**  
The emitted token $X_{\text{spec}}$ equals $x$ either through acceptance of the draft token or through sampling from $P'$ upon rejection. By the Law of Total Probability:
$$P_{\text{spec}}(x) = \mathbb{P}(X_{\text{spec}} = x \mid \text{Accept}) \mathbb{P}(\text{Accept}) + \mathbb{P}(X_{\text{spec}} = x \mid \text{Reject}) \mathbb{P}(\text{Reject})$$

Evaluating the first term:
$$\mathbb{P}(X_{\text{spec}} = x \mid \text{Accept}) \mathbb{P}(\text{Accept}) = Q(x) \alpha(x) = Q(x) \min\left(1, \frac{P(x)}{Q(x)}\right) = \min(Q(x), P(x))$$

Evaluating the second term:
$$\mathbb{P}(X_{\text{spec}} = x \mid \text{Reject}) \mathbb{P}(\text{Reject}) = P'(x) \cdot (1 - \Omega) = \frac{\max(0, P(x) - Q(x))}{1 - \Omega} \cdot (1 - \Omega) = \max(0, P(x) - Q(x))$$

Summing both terms:
$$P_{\text{spec}}(x) = \min(Q(x), P(x)) + \max(0, P(x) - Q(x))$$

Using the algebraic identity $\min(a, b) + \max(0, a - b) \equiv a$:
- If $P(x) \le Q(x)$: $\min(Q(x), P(x)) = P(x)$ and $\max(0, P(x) - Q(x)) = 0 \implies P_{\text{spec}}(x) = P(x)$.
- If $P(x) > Q(x)$: $\min(Q(x), P(x)) = Q(x)$ and $\max(0, P(x) - Q(x)) = P(x) - Q(x) \implies P_{\text{spec}}(x) = Q(x) + (P(x) - Q(x)) = P(x)$.

Hence, for all $x \in \mathcal{V}$:
$$P_{\text{spec}}(x) = P(x)$$
$\blacksquare$

---

## 3. Theorem 15: Expected Accepted Tokens & Speculative Speedup

### 3.1 Theorem Statement

**Theorem 15 (Expected Speedup Bound):**  
*Let $\gamma \ge 1$ denote the draft window length, and let $\alpha \in [0, 1]$ denote the mean token acceptance rate.  
If the target model verification requires latency $t_t$ and the draft model generates each token in latency $t_d = r \cdot t_t$ (where $r = t_d / t_t \in (0, 1)$ is the latency ratio), then:*
1. *The expected number of accepted tokens $\mathbb{E}[\beta]$ per speculative cycle is:*
   $$\mathbb{E}[\beta] = \sum_{i=0}^\gamma \alpha^i = \begin{cases} \frac{1 - \alpha^{\gamma + 1}}{1 - \alpha} & \text{if } \alpha < 1 \\ \gamma + 1 & \text{if } \alpha = 1 \end{cases}$$
2. *The end-to-end theoretical inference speedup ratio $S(\alpha, \gamma, r)$ satisfies:*
   $$S(\alpha, \gamma, r) = \frac{\mathbb{E}[\beta] \cdot t_t}{\gamma \cdot t_d + t_t} = \frac{\frac{1 - \alpha^{\gamma + 1}}{1 - \alpha}}{\gamma r + 1}$$

---

### 3.2 Proof

**Step 1: Expected Tokens Emitted per Cycle.**  
In each speculative cycle, $\gamma$ draft tokens are proposed.  
Token $i \in \{1, \dots, \gamma\}$ is verified only if all preceding tokens $1, \dots, i-1$ were accepted.  
Assuming independent per-token acceptance with expectation $\alpha$:
- Token 1 is accepted with probability $\alpha$.
- Token 2 is accepted with probability $\alpha^2$.
- In general, token $k$ is accepted with probability $\alpha^k$.

In addition, each cycle emits at least one bonus token:
- If rejection occurs at position $k \le \gamma$, a bonus token is sampled from $P'_k$ (contribution = 1).
- If all $\gamma$ tokens are accepted, a bonus token is sampled from the target distribution at position $\gamma + 1$ (contribution = 1).

Thus, the total tokens emitted is $\beta = k_{\text{accepted}} + 1$.  
The expected number of emitted tokens is:
$$\mathbb{E}[\beta] = 1 + \sum_{i=1}^\gamma \alpha^i = \sum_{i=0}^\gamma \alpha^i$$
Applying the finite geometric series sum formula for $\alpha \ne 1$:
$$\sum_{i=0}^\gamma \alpha^i = \frac{1 - \alpha^{\gamma + 1}}{1 - \alpha}$$
For $\alpha = 1$:
$$\lim_{\alpha \to 1} \frac{1 - \alpha^{\gamma + 1}}{1 - \alpha} = \lim_{\alpha \to 1} \frac{-(\gamma + 1)\alpha^\gamma}{-1} = \gamma + 1$$

**Step 2: Wall-Clock Latency Formulation.**  
To generate the same number of tokens $\mathbb{E}[\beta]$ under standard autoregressive target decoding requires:
$$T_{\text{baseline}} = \mathbb{E}[\beta] \cdot t_t$$

Under speculative decoding:
1. The draft model executes $\gamma$ sequential autoregressive steps: $\gamma \cdot t_d$.
2. The target model verifies all $\gamma$ tokens in parallel in a single batched forward pass: $t_t$.
Total cycle latency:
$$T_{\text{spec}} = \gamma \cdot t_d + t_t = (\gamma r + 1) t_t$$

**Step 3: Speedup Ratio.**  
The speedup $S$ is the ratio of baseline latency to speculative latency:
$$S(\alpha, \gamma, r) = \frac{T_{\text{baseline}}}{T_{\text{spec}}} = \frac{\mathbb{E}[\beta] \cdot t_t}{(\gamma r + 1) t_t} = \frac{\frac{1 - \alpha^{\gamma + 1}}{1 - \alpha}}{\gamma r + 1}$$

**Numerical Illustration:**  
For empirical parameters $\alpha = 0.80$, $\gamma = 5$, and $r = 0.10$ (draft model is $10\times$ faster than target model):
$$\mathbb{E}[\beta] = \frac{1 - 0.8^6}{1 - 0.8} = \frac{1 - 0.262144}{0.2} = \frac{0.737856}{0.2} = 3.68928\text{ tokens}$$
$$\text{Denominator} = 5 \times 0.10 + 1 = 1.50$$
$$S(0.8, 5, 0.1) = \frac{3.68928}{1.50} \approx 2.46\times\text{ latency reduction}$$
$\blacksquare$

---

## 4. Theorem 16: Radix Trie KV Cache Compression Ratio

### 4.1 Theorem Statement

**Theorem 16 (Prefix-Tree KV Cache Compression):**  
*Let $M \ge 1$ be the number of concurrent agent sessions, each processing a total prompt sequence length $L_{\text{total}}$. Suppose each session shares a common prompt prefix (e.g. system instructions, tool definitions) of length $L_{\text{prefix}} = \eta \cdot L_{\text{total}}$ where $\eta \in [0, 1)$ is the shared fraction.  
Then:*
1. *The compression ratio $C_R(M, \eta)$ achieved by the Radix Trie KV Cache is:*
   $$C_R(M, \eta) = \frac{1}{(1 - \eta) + \frac{\eta}{M}}$$
2. *As the number of concurrent sessions grows ($M \to \infty$), the asymptotic memory compression bound is:*
   $$\lim_{M \to \infty} C_R(M, \eta) = \frac{1}{1 - \eta}$$
*achieving an asymptotic memory footprint reduction of $100 \cdot \eta\%$.*

---

### 4.2 Proof

**Step 1: Dense KV Cache Memory Footprint.**  
Let $C_{\text{kv}}$ denote the memory in bytes required to store Key and Value activation vectors for a single token:
$$C_{\text{kv}} = 2 \times N_{\text{layers}} \times N_{\text{heads}} \times D_{\text{head}} \times b_{\text{precision}}$$
Under dense uncompressed allocation, each session maintains a separate KV buffer of length $L_{\text{total}}$.  
Total dense memory across $M$ sessions:
$$\text{Memory}_{\text{dense}}(M) = M \cdot L_{\text{total}} \cdot C_{\text{kv}}$$

**Step 2: Radix Trie KV Cache Memory Footprint.**  
In the Radix Trie, the shared prompt prefix of length $\eta L_{\text{total}}$ is stored exactly once in the root path with reference count $M$.  
Each of the $M$ sessions branches off the common prefix to allocate only its distinct private suffix of length $(1 - \eta) L_{\text{total}}$.  
Total compressed memory:
$$\text{Memory}_{\text{radix}}(M) = \left( \eta L_{\text{total}} + M (1 - \eta) L_{\text{total}} \right) C_{\text{kv}}$$

**Step 3: Compression Ratio Evaluation.**  
The compression ratio $C_R$ is:
$$C_R(M, \eta) = \frac{\text{Memory}_{\text{dense}}(M)}{\text{Memory}_{\text{radix}}(M)} = \frac{M \cdot L_{\text{total}} \cdot C_{\text{kv}}}{\left(\eta L_{\text{total}} + M (1 - \eta) L_{\text{total}}\right) C_{\text{kv}}} = \frac{M}{\eta + M (1 - \eta)}$$

Dividing numerator and denominator by $M$:
$$C_R(M, \eta) = \frac{1}{\frac{\eta}{M} + (1 - \eta)} = \frac{1}{(1 - \eta) + \frac{\eta}{M}}$$

**Step 4: Asymptotic Limit.**  
Taking the limit as $M \to \infty$:
$$\lim_{M \to \infty} C_R(M, \eta) = \frac{1}{(1 - \eta) + 0} = \frac{1}{1 - \eta}$$

**Engineering Implication:**  
In typical autonomous coding agent workflows, system prompt instructions, standard library documentation, and tool declarations constitute $\eta = 0.80$ (80% of prompt tokens).  
As $M \to \infty$:
$$C_R \to \frac{1}{1 - 0.80} = \frac{1}{0.20} = 5.0\times\text{ memory compression}$$
This allows $5\times$ more concurrent subagents to run within identical GPU VRAM constraints.
$\blacksquare$

---

## 5. Engineering Implementation & Test Alignment

- **Runtime Module**: `crates/pandora-runtime/src/speculative_decoding.rs`
- **Re-exports**: `SpeculativeEngine`, `RadixKvCache`, `RadixKvNode`, `DraftVerificationResult`, `TokenProbability` in `crates/pandora-runtime/src/lib.rs`.
- **Integration Tests**: `crates/pandora-runtime/tests/speculative_decoding_tests.rs`:
  - `test_perfect_draft_acceptance_and_bonus`: Confirms $\alpha = 1.0$ yields $\gamma + 1$ emitted tokens.
  - `test_rejection_sampling_distribution_shift`: Verifies that rejection triggers bonus token sampling strictly from residual $P'(x)$.
  - `test_theoretical_speedup_calculations`: Validates exact correspondence with Theorem 15 formulas.
  - `test_radix_kv_cache_prefix_insertion_and_lookup`: Confirms prefix detection and edge splitting in Radix Trie.
  - `test_radix_kv_cache_compression_ratio_matches_theory`: Confirms empirical compression ratio matches Theorem 16 formula to $10^{-4}$ precision.
  - `test_radix_kv_cache_release_and_cleanup`: Confirms reference-counted pruning upon session termination.
