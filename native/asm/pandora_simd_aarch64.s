/*
 * pandora_simd_aarch64.s
 * Pure ARM64 (AArch64) NEON Vector Assembly Implementation for AGENT-PANDORA v3.0.10.
 *
 * Architecture Parity:
 * Matches AVX2 256-bit engine throughput on Linux AArch64 and Apple Silicon macOS (Mach-O).
 * Calling Convention: AAPCS64 (x0 = const float* a, x1 = const float* b).
 * Return: s0 = dot product float (scalar IEEE-754 single-precision float).
 *
 * Microarchitectural & Theoretical Mathematical Justification:
 * -------------------------------------------------------------
 * 1. Pipeline Latency Hiding:
 *    On modern ARM64 cores (Cortex-A78, Neoverse V1/V2, Apple Silicon Firestorm/Avalanche/M-series),
 *    the floating-point multiply-accumulate instruction (FMLA.4S) has a pipeline latency
 *    L_FMLA in [3, 4] cycles and a dual-to-quad vector execution issue width W_issue in [2, 4] ops/cycle.
 *    A single accumulator loop exhibits Read-After-Write (RAW) data hazards, stalling the pipeline
 *    for (L_FMLA - 1) ~ 3 cycles between dependent instructions, bounding throughput to:
 *        Theta_single <= 1 / L_FMLA <= 1 / 3 ~= 0.33 FMLA/cycle.
 *    By unrolling 4x with 4 independent accumulator registers (v0, v1, v2, v3):
 *        v0 <- FMLA(v0, v4, v16)
 *        v1 <- FMLA(v1, v5, v17)
 *        v2 <- FMLA(v2, v6, v18)
 *        v3 <- FMLA(v3, v7, v19)
 *    the dependency distance is Delta_dep = 4 >= L_FMLA. All RAW stalls are eliminated (0 bubbles),
 *    attaining full superscalar vector saturation:
 *        Theta_unrolled = min(W_issue, 4) FMLA/cycle.
 *
 * 2. Register File Partitioning & Zero-Spill Invariant:
 *    Under AAPCS64:
 *      - Parameter/Return & Caller-Saved Scratch: v0-v7 (8 registers)
 *      - Callee-Saved Registers: v8-v15 (lower 64 bits d8-d15 preserved across calls)
 *      - Caller-Saved Scratch: v16-v31 (16 registers)
 *    Our 4x unrolled scheme uses:
 *      - 4 accumulators: v0, v1, v2, v3 (subset of v0-v7)
 *      - 4 operand registers for vector a: v4, v5, v6, v7 (subset of v0-v7)
 *      - 4 operand registers for vector b: v16, v17, v18, v19 (subset of v16-v31)
 *    Total vector registers utilized: 4 + 4 + 4 = 12 registers.
 *    All 12 registers reside strictly within scratch sets {v0..v7} U {v16..v31}.
 *    Zero callee-saved registers (v8-v15) are touched.
 *    Zero stack memory spills or frame allocations (0 stack cycles, leaf function).
 *
 * 3. L1D Cache Line Burst Alignment:
 *    Each unrolled iteration processes 16 single-precision floats (16 * 4 bytes = 64 bytes)
 *    per operand vector. Standard ARM64 L1D cache line width is 64 bytes.
 *    Paired quadword loads (LDP q4, q5... LDP q6, q7...) stream exactly one full L1D cache line
 *    per array per iteration via 32-byte quadword bus transactions.
 *    For dimension 128 (512 bytes per array), the loop terminates in exactly:
 *        N_iter = 128 / 16 = 8 iterations.
 *    Amortized branch overhead is only 1 branch per 16 floats (0.0625 branches/float).
 *
 * Dual Symbol Export:
 * Supports Linux AArch64 (ELF, unadorned symbol) and macOS Apple Silicon (Mach-O, leading underscore).
 */

.text
.p2align 4

.globl pandora_simd_dot_product_128_neon
.globl _pandora_simd_dot_product_128_neon
.globl pandora_cpu_has_neon
.globl _pandora_cpu_has_neon

/*
 * float pandora_simd_dot_product_128_neon(const float* a, const float* b)
 *
 * Inputs:
 *   x0: Pointer to array a (128 single-precision IEEE-754 floats = 512 bytes)
 *   x1: Pointer to array b (128 single-precision IEEE-754 floats = 512 bytes)
 * Output:
 *   s0: Scalar float dot product result
 */
pandora_simd_dot_product_128_neon:
_pandora_simd_dot_product_128_neon:
    // Initialize 4 independent 128-bit vector accumulators to zero
    movi    v0.4s, #0
    movi    v1.4s, #0
    movi    v2.4s, #0
    movi    v3.4s, #0

    // Loop counter: 128 floats / 16 floats per iteration = 8 iterations
    mov     w2, #8

.Lneon_loop:
    // Load 16 floats (64 bytes = 1 L1D cache line) from vector a into v4-v7
    // Using paired quadword loads with post-index pointer progression
    ldp     q4, q5, [x0], #32
    ldp     q6, q7, [x0], #32

    // Load 16 floats (64 bytes = 1 L1D cache line) from vector b into v16-v19
    ldp     q16, q17, [x1], #32
    ldp     q18, q19, [x1], #32

    // 4-way unrolled Fused Multiply-Accumulate: vD = vD + (vN * vM)
    // Dispatched across independent pipelines with 0 RAW stall bubbles
    fmla    v0.4s, v4.4s, v16.4s
    fmla    v1.4s, v5.4s, v17.4s
    fmla    v2.4s, v6.4s, v18.4s
    fmla    v3.4s, v7.4s, v19.4s

    subs    w2, w2, #1
    b.ne    .Lneon_loop

    // Reduce 4 independent vector accumulators into v0.4s
    fadd    v0.4s, v0.4s, v1.4s
    fadd    v2.4s, v2.4s, v3.4s
    fadd    v0.4s, v0.4s, v2.4s

    // Horizontal pairwise reduction of 4 lanes in v0.4s down to scalar float in s0
    // Step 1: Pairwise add across lanes: [s0+s1, s2+s3, s0+s1, s2+s3]
    faddp   v0.4s, v0.4s, v0.4s
    // Step 2: Pairwise add lower 2 lanes into scalar destination register s0:
    //         s0 = (s0 + s1) + (s2 + s3)
    faddp   s0, v0.2s

    ret

/*
 * bool pandora_cpu_has_neon(void)
 *
 * NEON (Advanced SIMD) is mandatory across the ARMv8-A / ARMv9-A architecture
 * baseline and all Apple Silicon processors. Returns 1 (true) in w0.
 */
.p2align 2
pandora_cpu_has_neon:
_pandora_cpu_has_neon:
    mov     w0, #1
    ret
