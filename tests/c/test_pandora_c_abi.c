/**
 * @file test_pandora_c_abi.c
 * @brief Verification suite for Pandora C ABI & Assembly SIMD integration.
 */

#include "pandora.h"
#include <assert.h>
#include <math.h>
#include <stdio.h>
#include <string.h>

int main(void) {
    printf("[C ABI Test] Checking AGENT-PANDORA version...\n");
    const char* version = pandora_version();
    assert(version != NULL);
    assert(strcmp(version, "3.0.9") == 0);
    printf("  -> Verified version: %s\n", version);

    printf("[C ABI Test] Checking client allocation and lifecycle...\n");
    pandora_client_t* client = NULL;
    int err = pandora_client_new("bin/pandora.exe", &client);
    assert(err == 0);
    assert(client != NULL);
    pandora_client_free(client);
    printf("  -> Verified client lifecycle clean\n");

    printf("[C ABI Test] Checking envelope parsing...\n");
    pandora_envelope_t envelope;
    const char* sample_json = "{\"version\":\"3.0.9\",\"command\":\"doctor\",\"healthy\":true}";
    err = pandora_parse_envelope(sample_json, &envelope);
    assert(err == 0);
    assert(strcmp(envelope.version, "3.0.9") == 0);
    assert(strcmp(envelope.command, "doctor") == 0);
    assert(envelope.healthy == true);
    printf("  -> Verified envelope parser: cmd=%s, healthy=%d\n", envelope.command, envelope.healthy);

    printf("[C ABI Test] Checking x86_64 Assembly SIMD dot product...\n");
    float a[128] = {0};
    float b[128] = {0};
    a[0] = 1.0f;
    b[1] = 1.0f;
    float dot_ortho = pandora_simd_dot_product_128(a, b);
    assert(fabsf(dot_ortho) < 1e-6);

    for (int i = 0; i < 128; i++) {
        a[i] = 1.0f;
        b[i] = 2.0f;
    }
    float dot_parallel = pandora_simd_dot_product_128(a, b);
    assert(fabsf(dot_parallel - 256.0f) < 1e-4);
    printf("  -> Verified 128-d SIMD dot product: %f\n", dot_parallel);

    float dot_scalar = pandora_simd_dot_product_128_scalar(a, b);
    assert(fabsf(dot_scalar - 256.0f) < 1e-4);
    printf("  -> Verified 128-d scalar dot product: %f\n", dot_scalar);

    float dot_avx2 = pandora_simd_dot_product_128_avx2(a, b);
    assert(fabsf(dot_avx2 - 256.0f) < 1e-4);
    printf("  -> Verified 128-d AVX2 SIMD dot product: %f\n", dot_avx2);

    printf("[C ABI Test] Checking CPUID hardware features...\n");
    bool has_avx2 = pandora_cpuid_has_avx2();
    printf("  -> Host CPUID AVX2 support: %s\n", has_avx2 ? "true" : "false");

    printf("ALL C ABI TESTS PASSED CLEANLY.\n");
    return 0;
}
