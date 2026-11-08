/*
 * pandora_simd_x86_64.s
 * Pure x86_64 Assembly implementation for AGENT-PANDORA v3.0.7.
 * Provides CPUID feature detection and AVX/SSE vector dot product.
 * Target ABI: x86_64 Windows (RCX, RDX, R8, R9 calling convention).
 */

.text
.globl pandora_simd_dot_product_128_asm
.globl pandora_simd_dot_product_128_avx2_asm
.globl pandora_cpuid_features_asm
.globl pandora_simd_dot_product_128_sysv_asm
.globl pandora_simd_dot_product_128_avx2_sysv_asm
.globl pandora_cpuid_features_sysv_asm

/*
 * SysV AMD64 ABI wrappers for Linux and macOS (arguments passed in %rdi, %rsi)
 */
pandora_simd_dot_product_128_sysv_asm:
    movq %rdi, %rcx
    movq %rsi, %rdx
    jmp pandora_simd_dot_product_128_asm

pandora_simd_dot_product_128_avx2_sysv_asm:
    movq %rdi, %rcx
    movq %rsi, %rdx
    jmp pandora_simd_dot_product_128_avx2_asm

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
 * float pandora_simd_dot_product_128_avx2_asm(const float* a, const float* b)
 * 256-bit AVX2 SIMD dot product unrolled 4x (32 floats / 128 bytes per iteration).
 * RCX = pointer to array a (128 floats = 512 bytes)
 * RDX = pointer to array b (128 floats = 512 bytes)
 * Return: %xmm0 = dot product float
 */
pandora_simd_dot_product_128_avx2_asm:
    vxorps %ymm0, %ymm0, %ymm0  /* accumulator ymm0 */
    vxorps %ymm1, %ymm1, %ymm1  /* accumulator ymm1 */
    vxorps %ymm2, %ymm2, %ymm2  /* accumulator ymm2 */
    vxorps %ymm3, %ymm3, %ymm3  /* accumulator ymm3 */

    movq $0, %rax               /* loop offset in bytes */

.loop_avx2:
    /* Load and multiply 32 floats (128 bytes) per iteration unrolled 4x */
    vmovups (%rcx, %rax), %ymm4
    vmovups (%rdx, %rax), %ymm5
    vmulps  %ymm5, %ymm4, %ymm4
    vaddps  %ymm4, %ymm0, %ymm0

    vmovups 32(%rcx, %rax), %ymm6
    vmovups 32(%rdx, %rax), %ymm7
    vmulps  %ymm7, %ymm6, %ymm6
    vaddps  %ymm6, %ymm1, %ymm1

    vmovups 64(%rcx, %rax), %ymm4
    vmovups 64(%rdx, %rax), %ymm5
    vmulps  %ymm5, %ymm4, %ymm4
    vaddps  %ymm4, %ymm2, %ymm2

    vmovups 96(%rcx, %rax), %ymm6
    vmovups 96(%rdx, %rax), %ymm7
    vmulps  %ymm7, %ymm6, %ymm6
    vaddps  %ymm6, %ymm3, %ymm3

    addq $128, %rax
    cmpq $512, %rax             /* 128 floats * 4 bytes = 512 bytes */
    jl .loop_avx2

    /* Accumulate 4 YMM registers into %ymm0 */
    vaddps %ymm1, %ymm0, %ymm0
    vaddps %ymm3, %ymm2, %ymm2
    vaddps %ymm2, %ymm0, %ymm0

    /* Extract upper 128 bits of %ymm0 to %xmm1 and add to lower 128 bits */
    vextractf128 $1, %ymm0, %xmm1
    vaddps %xmm1, %xmm0, %xmm0

    /* Horizontal add across the 4 single-precision floats in %xmm0 */
    vshufps $0x4e, %xmm0, %xmm0, %xmm1
    vaddps  %xmm1, %xmm0, %xmm0
    vshufps $0xb1, %xmm0, %xmm0, %xmm1
    vaddss  %xmm1, %xmm0, %xmm0

    vzeroupper
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

/*
 * void pandora_cpuid_features_sysv_asm(unsigned int leaf, unsigned int* eax, unsigned int* ebx, unsigned int* ecx, unsigned int* edx)
 * SysV AMD64 ABI: %edi = leaf, %rsi = eax, %rdx = ebx, %rcx = ecx, %r8 = edx
 */
pandora_cpuid_features_sysv_asm:
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15

    movq %rsi, %r12             /* r12 = eax ptr */
    movq %rdx, %r13             /* r13 = ebx ptr */
    movq %rcx, %r14             /* r14 = ecx ptr */
    movq %r8,  %r15             /* r15 = edx ptr */

    movl %edi, %eax             /* leaf */
    xorl %ecx, %ecx             /* sub-leaf 0 */
    cpuid

    testq %r12, %r12
    jz .skip_sysv_eax
    movl %eax, (%r12)
.skip_sysv_eax:
    testq %r13, %r13
    jz .skip_sysv_ebx
    movl %ebx, (%r13)
.skip_sysv_ebx:
    testq %r14, %r14
    jz .skip_sysv_ecx
    movl %ecx, (%r14)
.skip_sysv_ecx:
    testq %r15, %r15
    jz .skip_sysv_edx
    movl %edx, (%r15)
.skip_sysv_edx:

    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret
