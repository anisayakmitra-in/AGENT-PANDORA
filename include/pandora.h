/**
 * @file pandora.h
 * @brief Stable C ABI Header for AGENT-PANDORA v4.0.10.
 *
 * Provides universal C FFI exports, JSON-RPC envelope structures,
 * and hardware SIMD vector operations.
 */

#ifndef PANDORA_H
#define PANDORA_H

#ifdef __cplusplus
extern "C" {
#endif

#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>

#define PANDORA_VERSION "4.0.10"
#define PANDORA_DENSE_DIMENSION 128

typedef struct pandora_client pandora_client_t;

typedef struct pandora_envelope {
    char version[32];
    char command[64];
    bool healthy;
    int exit_code;
    char raw_json[4096];
} pandora_envelope_t;

typedef struct pandora_error {
    char version[32];
    char code[64];
    char message[256];
} pandora_error_t;

/**
 * Returns the active SemVer release version of AGENT-PANDORA.
 */
const char* pandora_version(void);

/**
 * Allocates and initializes a new Pandora client instance.
 *
 * @param launcher_path Optional path to native CLI binary or launcher.
 * @param out_client Pointer receiving the allocated client.
 * @return 0 on success, non-zero on error.
 */
int pandora_client_new(const char* launcher_path, pandora_client_t** out_client);

/**
 * Frees a Pandora client instance.
 */
void pandora_client_free(pandora_client_t* client);

/**
 * Parses a JSON string envelope into a structured pandora_envelope_t.
 */
int pandora_parse_envelope(const char* json_str, pandora_envelope_t* out_envelope);

/**
 * Hardware-accelerated 128-dimensional dense vector dot product.
 * Dynamically selects NEON on aarch64, AVX2 on x86_64 when supported, otherwise falling back to SSE or scalar.
 */
float pandora_simd_dot_product_128(const float* a, const float* b);

/**
 * Explicit 256-bit AVX2 hardware SIMD 128-dimensional dense vector dot product.
 */
float pandora_simd_dot_product_128_avx2(const float* a, const float* b);

/**
 * Explicit 128-bit ARM64 NEON hardware SIMD 128-dimensional dense vector dot product.
 */
float pandora_simd_dot_product_128_neon(const float* a, const float* b);

/**
 * Scalar reference 128-dimensional dense vector dot product.
 */
float pandora_simd_dot_product_128_scalar(const float* a, const float* b);

/**
 * Checks whether the host CPU supports AVX2 instructions.
 */
bool pandora_cpuid_has_avx2(void);

/**
 * Checks whether the host CPU supports ARM64 NEON instructions.
 */
bool pandora_cpu_has_neon(void);

/* -------------------------------------------------------------------------
 * Additive Phase 4 ABI Helpers (strictly backward-compatible with v3.1.3)
 * ------------------------------------------------------------------------- */

/**
 * Options for constructing `pandora run --strategy lats` CLI argument vectors.
 * Pass 0 / 0.0 for optional numeric fields to omit the flag and use CLI defaults (negative or non-finite floats return -1).
 */
typedef struct pandora_lats_options {
    uint32_t rollouts;
    uint32_t depth;
    double puct_c;
    double escalation_threshold;
} pandora_lats_options_t;

/**
 * Options for constructing `pandora memory vfs tropical-search` CLI argument vectors.
 * Pass 0 or NULL to omit optional flags.
 */
typedef struct pandora_tropical_search_options {
    uint32_t limit;
    const char* tier;
    const char* intent;
} pandora_tropical_search_options_t;

/**
 * Builds a `--json` CLI argv vector for `pandora run --strategy lats`.
 *
 * @param task Non-empty task prompt string.
 * @param options Optional LATS strategy parameters (NULL uses CLI defaults).
 * @param out_argv Caller-allocated array receiving pointers into static literals / `scratch`.
 * @param max_args Capacity of `out_argv`.
 * @param out_argc Receives the number of populated arguments.
 * @param scratch Caller-allocated buffer for formatted numeric option strings.
 * @param scratch_len Capacity of `scratch` in bytes.
 * @return 0 on success, -1 on invalid/NULL inputs, -2 if buffers are too small.
 */
int pandora_build_lats_args(
    const char* task,
    const pandora_lats_options_t* options,
    const char** out_argv,
    size_t max_args,
    size_t* out_argc,
    char* scratch,
    size_t scratch_len
);

/**
 * Builds a `--json` CLI argv vector for `pandora memory vfs tropical-search`.
 *
 * @param query Non-empty search query string.
 * @param options Optional tropical search filters (NULL uses CLI defaults).
 * @param out_argv Caller-allocated array receiving pointers into static literals / `scratch`.
 * @param max_args Capacity of `out_argv`.
 * @param out_argc Receives the number of populated arguments.
 * @param scratch Caller-allocated buffer for formatted numeric option strings.
 * @param scratch_len Capacity of `scratch` in bytes.
 * @return 0 on success, -1 on invalid/NULL inputs, -2 if buffers are too small.
 */
int pandora_build_tropical_search_args(
    const char* query,
    const pandora_tropical_search_options_t* options,
    const char** out_argv,
    size_t max_args,
    size_t* out_argc,
    char* scratch,
    size_t scratch_len
);

/**
 * Zero-multiplication 128-dimensional Tropical Max-Plus inner product:
 *   max_{i=0..127} (a[i] + b[i])
 * Returns 0.0f if either pointer is NULL, and skips NaN coordinates.
 */
float pandora_tropical_max_plus_128(const float* a, const float* b);

/**
 * Builds a `--json` CLI argv vector for `pandora model verify-attestation`.
 *
 * @param pnn_path Optional `.pnn` file path (NULL or empty omits `--pnn` and uses workspace default).
 * @param out_argv Caller-allocated array receiving pointers into static literals / `pnn_path`.
 * @param max_args Capacity of `out_argv` (at least 6).
 * @param out_argc Receives the number of populated arguments.
 * @return 0 on success, -1 on NULL output pointers, -2 if `out_argv` is too small.
 */
int pandora_build_verify_attestation_args(
    const char* pnn_path,
    const char** out_argv,
    size_t max_args,
    size_t* out_argc
);

#ifdef __cplusplus
}
#endif

#endif /* PANDORA_H */
