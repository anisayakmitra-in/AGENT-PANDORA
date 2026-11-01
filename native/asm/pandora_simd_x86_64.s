/*
 * pandora_simd_x86_64.s
 * Pure x86_64 Assembly implementation for AGENT-PANDORA v3.0.1.
 * Provides CPUID feature detection and AVX/SSE vector dot product.
 * Target ABI: x86_64 Windows (RCX, RDX, R8, R9 calling convention).
 */

.text
.globl pandora_simd_dot_product_128_asm
.globl pandora_cpuid_features_asm

/*
 * float pandora_simd_dot_product_128_asm(const float* a, const float* b)
 * RCX = pointer to array a (128 floats)
 * RDX = pointer to array b (128 floats)
 * Return: %xmm0 = dot product float
 */
pandora_simd_dot_product_128_asm:
    xorps %xmm0, %xmm0          /* accumulator sum0 */
    xorps %xmm1, %xmm1          /* accumulator sum1 */
    xorps %xmm2, %xmm2          /* accumulator sum2 */
    xorps %xmm3, %xmm3          /* accumulator sum3 */
    
    movq $0, %rax               /* loop offset in bytes */

.loop_unrolled:
    /* Load and multiply 16 floats (64 bytes) per iteration */
    movups (%rcx, %rax), %xmm4
    movups (%rdx, %rax), %xmm5
    mulps  %xmm5, %xmm4
    addps  %xmm4, %xmm0

    movups 16(%rcx, %rax), %xmm6
    movups 16(%rdx, %rax), %xmm7
    mulps  %xmm7, %xmm6
    addps  %xmm6, %xmm1

    movups 32(%rcx, %rax), %xmm4
    movups 32(%rdx, %rax), %xmm5
    mulps  %xmm5, %xmm4
    addps  %xmm4, %xmm2

    movups 48(%rcx, %rax), %xmm6
    movups 48(%rdx, %rax), %xmm7
    mulps  %xmm7, %xmm6
    addps  %xmm6, %xmm3

    addq $64, %rax
    cmpq $512, %rax             /* 128 floats * 4 bytes = 512 bytes */
    jl .loop_unrolled

    /* Reduce 4 vector registers into %xmm0 */
    addps %xmm1, %xmm0
    addps %xmm3, %xmm2
    addps %xmm2, %xmm0

    /* Horizontal add across 4 lanes of %xmm0 */
    movhlps %xmm0, %xmm1
    addps   %xmm1, %xmm0
    movshdup %xmm0, %xmm1
    addss   %xmm1, %xmm0

    ret

/*
 * void pandora_cpuid_features_asm(unsigned int leaf, unsigned int* eax, unsigned int* ebx, unsigned int* ecx, unsigned int* edx)
 * RCX = leaf
 * RDX = eax ptr
 * R8  = ebx ptr
 * R9  = ecx ptr
 * 40(%rsp) = edx ptr (Windows x64 shadow space: 32 bytes + 8 bytes return addr)
 */
pandora_cpuid_features_asm:
    pushq %rbx
    pushq %rdi
    pushq %rsi

    /* Save argument pointers into caller-saved registers before cpuid clobbers %rax, %rbx, %rcx, %rdx */
    movq %rdx, %r10             /* r10 = eax ptr */
    movq %r8,  %r11             /* r11 = ebx ptr */
    movq %r9,  %rdi             /* rdi = ecx ptr */
    movq 64(%rsp), %rsi         /* rsi = edx ptr (offset 64 after 3 pushes) */

    movl %ecx, %eax             /* leaf */
    xorl %ecx, %ecx             /* sub-leaf 0 */
    cpuid

    /* Store results using preserved pointers */
    testq %r10, %r10
    jz .skip_eax
    movl %eax, (%r10)
.skip_eax:
    testq %r11, %r11
    jz .skip_ebx
    movl %ebx, (%r11)
.skip_ebx:
    testq %rdi, %rdi
    jz .skip_ecx
    movl %ecx, (%rdi)
.skip_ecx:
    testq %rsi, %rsi
    jz .skip_edx
    movl %edx, (%rsi)
.skip_edx:

    popq %rsi
    popq %rdi
    popq %rbx
    ret
