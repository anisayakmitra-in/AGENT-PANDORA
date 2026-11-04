# Mathematical Foundations & Protocol Specification: Streaming WebSocket Gateway (RFC 6455)

**Author:** ANISAYAK MITRA <266799942+anisayakmitra-in@users.noreply.github.com>  
**Version:** AGENT-PANDORA v3.0.4 (Phase 2)  
**Safety Invariant:** `#![forbid(unsafe_code)]` Strictly Enforced  

---

## 1. Executive Summary & Architectural Motivation

In AGENT-PANDORA v3.0.3, the system bridged inter-process agent communication via synchronous polling or raw TCP pipes. As the platform transitions to multi-agent swarm collaboration and real-time terminal visual inspection (Phases 2 through 5), bidirectional, low-overhead event streaming becomes foundational.

Phase 2 replaces legacy polling with an RFC 6455 compliant WebSocket Gateway supporting:
1. **Full-Duplex Framing & Uniform Entropy Masking**: Client-to-server 32-bit XOR frame masking ensuring maximal entropy $H = 8\text{ bits/byte}$ to defeat proxy cache poisoning.
2. **Backpressure Flow Control & Bounded $M/M/1/K$ Buffer Dynamics**: Mathematical modeling of event ingestion rates $\lambda$ versus socket drain rates $\mu$, guaranteeing queue boundedness $Q(t) \le K$ and deterministic upper-bounded latency $W \le K / \mu$.
3. **Cryptographic Handshake Invariants**: SHA-1 RFC 6455 key negotiation coupled with HMAC-SHA256 authenticated bearer tokens resisting chosen-message forgery.
4. **JSON-RPC 2.0 Streaming Event Bus**: Monotonic correlation identifiers and pub/sub push mechanisms for ContextVfs file notifications, agent task states, and telemetry.

---

## 2. RFC 6455 Framing & Entropy Masking Theorem

### 2.1 Frame Syntax & Mathematical Representation

An RFC 6455 frame transmits an octet sequence across a reliable byte stream (TCP). Let each octet be an element of the Galois Field $\mathbb{F}_2^8 \cong \{0, 1\}^8$.

A client frame consists of:
- Header octet 1: $\text{FIN} \in \{0, 1\}$, $\text{RSV}_{1,2,3} = (0,0,0)$, and $\text{Opcode} \in \{0x0, 0x1, 0x2, 0x8, 0x9, 0xA\}$.
- Header octet 2: $\text{MASK} = 1$, and payload length descriptor $\ell \in [0, 2^{64}-1]$.
- Masking key: $\mathbf{K} = (k_0, k_1, k_2, k_3) \in (\mathbb{F}_2^8)^4$.
- Payload bytes: $\mathbf{P} = (p_0, p_1, \dots, p_{n-1}) \in (\mathbb{F}_2^8)^n$.

The transmitted wire octets $\mathbf{C} = (c_0, c_1, \dots, c_{n-1})$ are computed as:
$$c_i = p_i \oplus k_{i \pmod 4}$$
where $\oplus$ denotes bitwise addition modulo 2 in $\mathbb{F}_2^8$.

### 2.2 Theorem 1: Uniform Entropy of Masked Frames

**Theorem:**  
*Let the payload stream $\mathbf{P}$ have an arbitrary probability distribution $P(\mathbf{P})$ (including deterministic or repetitive structures like HTTP headers or script tags). If the masking key $\mathbf{K} \sim \mathcal{U}((\mathbb{F}_2^8)^4)$ is independently drawn from a cryptographically secure uniform distribution, then every transmitted byte $c_i$ is statistically independent and exhibits maximal Shannon entropy:*
$$H(c_i) = \log_2(256) = 8\text{ bits per octet}$$

**Proof:**  
Consider an arbitrary payload octet $p_i = a \in \mathbb{F}_2^8$ with probability $P(p_i = a)$. The corresponding mask byte $k = k_{i \pmod 4}$ satisfies $P(k = b) = \frac{1}{256}$ for all $b \in \mathbb{F}_2^8$.

The probability that $c_i = z$ for any fixed $z \in \mathbb{F}_2^8$ is:
$$P(c_i = z) = \sum_{a \in \mathbb{F}_2^8} P(p_i = a) \cdot P(k = z \oplus a)$$
Since $k$ is uniformly distributed, $P(k = z \oplus a) = \frac{1}{256}$ regardless of the value of $z \oplus a$:
$$P(c_i = z) = \frac{1}{256} \sum_{a \in \mathbb{F}_2^8} P(p_i = a) = \frac{1}{256} \cdot 1 = \frac{1}{256}$$

The Shannon entropy of $c_i$ is therefore:
$$H(c_i) = -\sum_{z \in \mathbb{F}_2^8} P(c_i = z) \log_2 P(c_i = z) = -\sum_{z \in \mathbb{F}_2^8} \frac{1}{256} \log_2\left(\frac{1}{256}\right) = - \log_2\left(\frac{1}{256}\right) = 8\text{ bits}$$
$\blacksquare$

**Security Corollary:**  
Because $H(c_i) = 8\text{ bits}$ and the mutual information $I(c_i; c_{i+1}) = 0$, intermediate transparent HTTP proxy caches cannot detect recurring byte patterns. This renders proxy byte-sniffing cache-poisoning attacks mathematically impossible.

---

## 3. Backpressure Flow Control & Queue Stability

### 3.1 The $M/M/1/K$ Finite Queue Model

To prevent memory leaks under bursty event generation (e.g., intensive `ContextVfs` file modifications or swarm task broadcast floods), the WebSocket server employs a bounded priority queue with capacity $K = 1024$ frames per client connection.

Let:
- $\lambda$: Mean arrival rate of outbound event frames (frames/second).
- $\mu$: Socket transmission service rate determined by TCP window and physical network bandwidth (frames/second).
- $\rho = \frac{\lambda}{\mu}$: Traffic intensity factor.

The system is modeled as a continuous-time Markov birth-death process on state space $S = \{0, 1, 2, \dots, K\}$, where state $n$ represents the number of buffered frames.

### 3.2 Stationary Probability Distribution

The balance equations are:
$$\lambda \pi_{n-1} = \mu \pi_n \implies \pi_n = \rho^n \pi_0 \quad \text{for } 1 \le n \le K$$

Enforcing the probability normalization axiom $\sum_{n=0}^K \pi_n = 1$:
$$\pi_0 \sum_{n=0}^K \rho^n = 1 \implies \pi_0 = \frac{1 - \rho}{1 - \rho^{K+1}} \quad (\rho \ne 1)$$

Thus, the steady-state probability of having $n$ frames in the buffer is:
$$\pi_n = \frac{(1 - \rho)\rho^n}{1 - \rho^{K+1}}, \quad 0 \le n \le K$$

### 3.3 Theorem 2: Deterministic Latency Boundedness via Backpressure

**Theorem:**  
*Under an $M/M/1/K$ backpressure admission policy with queue capacity $K$, the mean sojourn latency $W$ for an accepted event frame is strictly bounded by:*
$$W \le \frac{K}{\mu}$$
*and the maximum queue memory footprint is invariant at $O(K)$.*

**Proof:**  
The drop probability (blocking probability) at capacity $K$ is:
$$P_{\text{drop}} = \pi_K = \frac{(1 - \rho)\rho^K}{1 - \rho^{K+1}}$$

The effective arrival rate admitted into the system is:
$$\lambda_{\text{eff}} = \lambda (1 - P_{\text{drop}}) = \lambda \left(1 - \frac{(1 - \rho)\rho^K}{1 - \rho^{K+1}}\right)$$

The expected number of frames in the buffer $L_q$ is:
$$L_q = \sum_{n=0}^K n \pi_n = \frac{\rho}{1 - \rho} - \frac{(K+1)\rho^{K+1}}{1 - \rho^{K+1}}$$

By Little's Law, the mean waiting time $W$ of an admitted frame satisfies:
$$W = \frac{L_q}{\lambda_{\text{eff}}}$$
As $\lambda \to \infty$ ($\rho \to \infty$), the distribution concentrates entirely at state $K$ ($\pi_K \to 1$), and the buffer reaches maximum capacity $K$. The service rate remains $\mu$. The time to drain $K$ frames is:
$$\lim_{\lambda \to \infty} W = \frac{K}{\mu}$$
For $K = 1024$ frames and a standard gigabit loopback or local socket drain rate $\mu = 10^5\text{ frames/sec}$:
$$W_{\max} = \frac{1024}{100,000} = 10.24\,\text{ms}$$
Even over a throttled $10\,\text{Mbps}$ network with $\mu = 1,000\text{ frames/sec}$:
$$W_{\max} = \frac{1024}{1,000} \approx 1.024\,\text{s}$$
The system remains strictly bounded in memory and immune to out-of-memory (OOM) faults. $\blacksquare$

---

## 4. Cryptographic Handshake & Key Derivation

### 4.1 RFC 6455 Handshake Equation

The opening handshake validates that the connecting peer is an authentic WebSocket client rather than an unwitting cross-protocol agent.

The client sends a 16-byte cryptographically random challenge encoded in Base64:
$$\text{Sec-WebSocket-Key} \in \{0, 1\}^{128}$$

The server computes the accept hash using the standard fixed salt $\text{GUID} = \texttt{"258EAFA5-E914-47DA-95CA-C5AB0DC85B11"}$:
$$\text{Sec-WebSocket-Accept} = \text{Base64}\Big(\text{SHA-1}\big(\text{Sec-WebSocket-Key} \mathbin{\Vert} \text{GUID}\big)\Big)$$

### 4.2 HMAC-SHA256 Session Authorization

In addition to the transport handshake, AGENT-PANDORA enforces cryptographically signed bearer tokens:
$$\text{Token} = \text{Payload} \mathbin{\Vert} \text{HMAC-SHA256}(K_{\text{secret}}, \text{Payload})$$
where:
$$\text{Payload} = \text{Base64URL}\big(\{\text{sub}: \text{"pandora-agent"}, \text{iat}: t_0, \text{exp}: t_0 + \Delta t\}\big)$$
The tag verification satisfies:
$$\text{Verify}(K_{\text{secret}}, \text{Payload}, \text{Tag}) = \begin{cases} 
1 & \text{if } \text{HMAC-SHA256}(K_{\text{secret}}, \text{Payload}) = \text{Tag} \\ 
0 & \text{otherwise} 
\end{cases}$$
The security parameter $\kappa = 256$ bits provides existential unforgeability under chosen-message attacks (EUF-CMA).

---

## 5. Architectural Implementation Mapping

The Phase 2 components integrate into AGENT-PANDORA across three layers:

```mermaid
flowchart TD
    subgraph Client["WebSocket Clients"]
        TUI["TUI Visualizer (Phase 4)"]
        SWARM["Swarm Peer (Phase 3)"]
        EXT["External CLI / IDE"]
    end

    subgraph Gateway["pandora-service WebSocket Gateway"]
        HS["RFC 6455 Handshake & Auth"]
        BM["Backpressure Buffer M/M/1/K"]
        MUX["JSON-RPC 2.0 Demux"]
    end

    subgraph Runtime["pandora-runtime"]
        HNSW["HNSW Graph Index (Phase 1)"]
        VFS["ContextVfs Memory"]
        BUS["Internal Event Bus"]
    end

    Client <-->|ws:// or wss://| HS
    HS --> BM
    BM --> MUX
    MUX <-->|Event Stream| BUS
    BUS <-->|Context Queries| VFS
    VFS <-->|O(log N) Vector Retrieval| HNSW
```

1. **`crates/pandora-service/src/websocket.rs`**: Safe Rust RFC 6455 frame parser, mask/unmask pipeline, and connection manager (`#![forbid(unsafe_code)]`).
2. **`crates/pandora-service/src/rpc.rs`**: JSON-RPC 2.0 dispatcher for remote methods (`pandora/vfs/search`, `pandora/status`, `pandora/memory/recall`).
3. **`crates/pandora-cli/src/commands/gateway.rs`**: `pandora gateway start --port 8443 --bind 127.0.0.1` and `pandora gateway ping`.
