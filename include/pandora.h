/**
 * @file pandora.h
 * @brief Stable C ABI Header for AGENT-PANDORA v3.0.6.
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

#define PANDORA_VERSION "3.0.6"
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

#ifdef __cplusplus
}
#endif

#endif /* PANDORA_H */
