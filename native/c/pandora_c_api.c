/**
 * @file pandora_c_api.c
 * @brief Implementation of C ABI for AGENT-PANDORA v3.1.3.
 */

#include "pandora.h"
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifdef __cplusplus
extern "C" {
#endif
#if defined(__x86_64__) || defined(_M_X64)
#if defined(_WIN32)
float pandora_simd_dot_product_128_asm(const float* a, const float* b);
float pandora_simd_dot_product_128_avx2_asm(const float* a, const float* b);
void pandora_cpuid_features_asm(unsigned int leaf, unsigned int* eax, unsigned int* ebx, unsigned int* ecx, unsigned int* edx);
#define pandora_simd_dot_product_128_raw pandora_simd_dot_product_128_asm
#define pandora_simd_dot_product_128_avx2_raw pandora_simd_dot_product_128_avx2_asm
#define pandora_cpuid_features_raw pandora_cpuid_features_asm
#else
float pandora_simd_dot_product_128_sysv_asm(const float* a, const float* b);
float pandora_simd_dot_product_128_avx2_sysv_asm(const float* a, const float* b);
void pandora_cpuid_features_sysv_asm(unsigned int leaf, unsigned int* eax, unsigned int* ebx, unsigned int* ecx, unsigned int* edx);
#define pandora_simd_dot_product_128_raw pandora_simd_dot_product_128_sysv_asm
#define pandora_simd_dot_product_128_avx2_raw pandora_simd_dot_product_128_avx2_sysv_asm
#define pandora_cpuid_features_raw pandora_cpuid_features_sysv_asm
#endif
#endif
#ifdef __cplusplus
}
#endif

struct pandora_client {
    char launcher_path[512];
};

const char* pandora_version(void) {
    return PANDORA_VERSION;
}

int pandora_client_new(const char* launcher_path, pandora_client_t** out_client) {
    if (!out_client) return -1;
    pandora_client_t* client = (pandora_client_t*)malloc(sizeof(pandora_client_t));
    if (!client) return -2;

    if (launcher_path) {
        strncpy(client->launcher_path, launcher_path, sizeof(client->launcher_path) - 1);
        client->launcher_path[sizeof(client->launcher_path) - 1] = '\0';
    } else {
        strcpy(client->launcher_path, "pandora");
    }

    *out_client = client;
    return 0;
}

void pandora_client_free(pandora_client_t* client) {
    if (client) {
        free(client);
    }
}

int pandora_parse_envelope(const char* json_str, pandora_envelope_t* out_envelope) {
    if (!json_str || !out_envelope) return -1;
    memset(out_envelope, 0, sizeof(pandora_envelope_t));

    // Basic extraction of version and command keys
    const char* ver_pos = strstr(json_str, "\"version\":");
    if (ver_pos) {
        sscanf(ver_pos, "\"version\":\"%31[^\"]\"", out_envelope->version);
    }

    const char* cmd_pos = strstr(json_str, "\"command\":");
    if (cmd_pos) {
        sscanf(cmd_pos, "\"command\":\"%63[^\"]\"", out_envelope->command);
    }

    if (strstr(json_str, "\"healthy\":true")) {
        out_envelope->healthy = true;
    }

    strncpy(out_envelope->raw_json, json_str, sizeof(out_envelope->raw_json) - 1);
    out_envelope->raw_json[sizeof(out_envelope->raw_json) - 1] = '\0';

    return 0;
}

float pandora_simd_dot_product_128_scalar(const float* a, const float* b) {
    if (!a || !b) return 0.0f;
    float sum = 0.0f;
    for (int i = 0; i < PANDORA_DENSE_DIMENSION; i++) {
        sum += a[i] * b[i];
    }
    return sum;
}

float pandora_simd_dot_product_128_avx2(const float* a, const float* b) {
    if (!a || !b) return 0.0f;
#if defined(__x86_64__) || defined(_M_X64)
    return pandora_simd_dot_product_128_avx2_raw(a, b);
#else
    return pandora_simd_dot_product_128_scalar(a, b);
#endif
}

#if !defined(__aarch64__) && !defined(_M_ARM64)
bool pandora_cpu_has_neon(void) {
    return false;
}

float pandora_simd_dot_product_128_neon(const float* a, const float* b) {
    if (!a || !b) return 0.0f;
    return pandora_simd_dot_product_128_scalar(a, b);
}
#endif

float pandora_simd_dot_product_128(const float* a, const float* b) {
    if (!a || !b) return 0.0f;
#if defined(__aarch64__) || defined(_M_ARM64)
    if (pandora_cpu_has_neon()) {
        return pandora_simd_dot_product_128_neon(a, b);
    }
    return pandora_simd_dot_product_128_scalar(a, b);
#elif defined(__x86_64__) || defined(_M_X64)
    if (pandora_cpuid_has_avx2()) {
        return pandora_simd_dot_product_128_avx2_raw(a, b);
    }
    return pandora_simd_dot_product_128_raw(a, b);
#else
    return pandora_simd_dot_product_128_scalar(a, b);
#endif
}

bool pandora_cpuid_has_avx2(void) {
#if (defined(__GNUC__) || defined(__clang__)) && (defined(__x86_64__) || defined(_M_X64) || defined(__i386__))
    return __builtin_cpu_supports("avx2");
#elif defined(__x86_64__) || defined(_M_X64)
    unsigned int eax = 0, ebx = 0, ecx = 0, edx = 0;
    // Leaf 7, Subleaf 0: EBX bit 5 = AVX2
    pandora_cpuid_features_raw(7, &eax, &ebx, &ecx, &edx);
    return (ebx & (1 << 5)) != 0;
#else
    return false;
#endif
}

static int pandora_push_arg(const char** out_argv, size_t max_args, size_t* idx, const char* arg) {
    if (*idx >= max_args) {
        return -2;
    }
    out_argv[*idx] = arg;
    *idx += 1;
    return 0;
}

static int pandora_push_fmt_u32(
    const char** out_argv,
    size_t max_args,
    size_t* idx,
    char* scratch,
    size_t scratch_len,
    size_t* scratch_used,
    uint32_t value
) {
    if (!scratch || *scratch_used >= scratch_len) {
        return -2;
    }
    int written = snprintf(scratch + *scratch_used, scratch_len - *scratch_used, "%u", value);
    if (written < 0 || (size_t)written >= scratch_len - *scratch_used) {
        return -2;
    }
    const char* ptr = scratch + *scratch_used;
    *scratch_used += (size_t)written + 1;
    return pandora_push_arg(out_argv, max_args, idx, ptr);
}

static int pandora_push_fmt_f64(
    const char** out_argv,
    size_t max_args,
    size_t* idx,
    char* scratch,
    size_t scratch_len,
    size_t* scratch_used,
    double value
) {
    if (!isfinite(value)) {
        return -1;
    }
    if (!scratch || *scratch_used >= scratch_len) {
        return -2;
    }
    int written = snprintf(scratch + *scratch_used, scratch_len - *scratch_used, "%.6g", value);
    if (written < 0 || (size_t)written >= scratch_len - *scratch_used) {
        return -2;
    }
    const char* ptr = scratch + *scratch_used;
    *scratch_used += (size_t)written + 1;
    return pandora_push_arg(out_argv, max_args, idx, ptr);
}

int pandora_build_lats_args(
    const char* task,
    const pandora_lats_options_t* options,
    const char** out_argv,
    size_t max_args,
    size_t* out_argc,
    char* scratch,
    size_t scratch_len
) {
    if (!task || task[0] == '\0' || !out_argv || !out_argc) {
        return -1;
    }
    size_t argc = 0;
    size_t used = 0;
    int rc = 0;

    if ((rc = pandora_push_arg(out_argv, max_args, &argc, "run")) != 0) return rc;
    if ((rc = pandora_push_arg(out_argv, max_args, &argc, "--strategy")) != 0) return rc;
    if ((rc = pandora_push_arg(out_argv, max_args, &argc, "lats")) != 0) return rc;

    if (options) {
        if (options->rollouts > 0) {
            if ((rc = pandora_push_arg(out_argv, max_args, &argc, "--lats-rollouts")) != 0) return rc;
            if ((rc = pandora_push_fmt_u32(out_argv, max_args, &argc, scratch, scratch_len, &used, options->rollouts)) != 0) return rc;
        }
        if (options->depth > 0) {
            if ((rc = pandora_push_arg(out_argv, max_args, &argc, "--lats-depth")) != 0) return rc;
            if ((rc = pandora_push_fmt_u32(out_argv, max_args, &argc, scratch, scratch_len, &used, options->depth)) != 0) return rc;
        }
        if (options->puct_c != 0.0) {
            if (!isfinite(options->puct_c) || options->puct_c < 0.0) return -1;
            if ((rc = pandora_push_arg(out_argv, max_args, &argc, "--puct-c")) != 0) return rc;
            if ((rc = pandora_push_fmt_f64(out_argv, max_args, &argc, scratch, scratch_len, &used, options->puct_c)) != 0) return rc;
        }
        if (options->escalation_threshold != 0.0) {
            if (!isfinite(options->escalation_threshold) || options->escalation_threshold < 0.0) return -1;
            if ((rc = pandora_push_arg(out_argv, max_args, &argc, "--escalation-threshold")) != 0) return rc;
            if ((rc = pandora_push_fmt_f64(out_argv, max_args, &argc, scratch, scratch_len, &used, options->escalation_threshold)) != 0) return rc;
        }
    }

    if ((rc = pandora_push_arg(out_argv, max_args, &argc, task)) != 0) return rc;
    if ((rc = pandora_push_arg(out_argv, max_args, &argc, "--json")) != 0) return rc;

    *out_argc = argc;
    return 0;
}

int pandora_build_tropical_search_args(
    const char* query,
    const pandora_tropical_search_options_t* options,
    const char** out_argv,
    size_t max_args,
    size_t* out_argc,
    char* scratch,
    size_t scratch_len
) {
    if (!query || query[0] == '\0' || !out_argv || !out_argc) {
        return -1;
    }
    size_t argc = 0;
    size_t used = 0;
    int rc = 0;

    if ((rc = pandora_push_arg(out_argv, max_args, &argc, "memory")) != 0) return rc;
    if ((rc = pandora_push_arg(out_argv, max_args, &argc, "vfs")) != 0) return rc;
    if ((rc = pandora_push_arg(out_argv, max_args, &argc, "tropical-search")) != 0) return rc;
    if ((rc = pandora_push_arg(out_argv, max_args, &argc, query)) != 0) return rc;

    uint32_t limit = (options && options->limit > 0) ? options->limit : 5u;
    if ((rc = pandora_push_arg(out_argv, max_args, &argc, "--limit")) != 0) return rc;
    if ((rc = pandora_push_fmt_u32(out_argv, max_args, &argc, scratch, scratch_len, &used, limit)) != 0) return rc;

    if (options) {
        if (options->tier && options->tier[0] != '\0') {
            if ((rc = pandora_push_arg(out_argv, max_args, &argc, "--tier")) != 0) return rc;
            if ((rc = pandora_push_arg(out_argv, max_args, &argc, options->tier)) != 0) return rc;
        }
        if (options->intent && options->intent[0] != '\0') {
            if ((rc = pandora_push_arg(out_argv, max_args, &argc, "--intent")) != 0) return rc;
            if ((rc = pandora_push_arg(out_argv, max_args, &argc, options->intent)) != 0) return rc;
        }
    }

    if ((rc = pandora_push_arg(out_argv, max_args, &argc, "--json")) != 0) return rc;

    *out_argc = argc;
    return 0;
}

float pandora_tropical_max_plus_128(const float* a, const float* b) {
    if (!a || !b) return 0.0f;
    float best = -INFINITY;
    for (int i = 0; i < PANDORA_DENSE_DIMENSION; i++) {
        float sum = a[i] + b[i];
        if (!isnan(sum) && sum > best) {
            best = sum;
        }
    }
    return best;
}

int pandora_build_verify_attestation_args(
    const char* pnn_path,
    const char** out_argv,
    size_t max_args,
    size_t* out_argc
) {
    if (!out_argv || !out_argc) {
        return -1;
    }
    size_t argc = 0;
    int rc = 0;
    if ((rc = pandora_push_arg(out_argv, max_args, &argc, "model")) != 0) return rc;
    if ((rc = pandora_push_arg(out_argv, max_args, &argc, "verify-attestation")) != 0) return rc;
    if (pnn_path && pnn_path[0] != '\0') {
        if ((rc = pandora_push_arg(out_argv, max_args, &argc, "--pnn")) != 0) return rc;
        if ((rc = pandora_push_arg(out_argv, max_args, &argc, pnn_path)) != 0) return rc;
    }
    if ((rc = pandora_push_arg(out_argv, max_args, &argc, "--json")) != 0) return rc;
    *out_argc = argc;
    return 0;
}
