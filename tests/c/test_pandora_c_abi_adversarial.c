/**
 * @file test_pandora_c_abi_adversarial.c
 * @brief Tier 5 Adversarial Hardening Suite for Pandora C ABI & Assembly SIMD.
 */

#include "pandora.h"
#include <assert.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(void) {
    printf("[Adversarial C ABI] Checking NULL pointer safety...\n");

    // 1. NULL pointer in pandora_client_new
    int ret1 = pandora_client_new(NULL, NULL);
    assert(ret1 == -1);

    pandora_client_t* client = NULL;
    int ret2 = pandora_client_new(NULL, &client);
    assert(ret2 == 0);
    assert(client != NULL);

    // 2. NULL pointer in pandora_client_free
    pandora_client_free(NULL); // Must be a safe no-op
    pandora_client_free(client);
    printf("  -> Verified NULL client allocations and safe free\n");

    // 3. NULL and malformed envelopes
    pandora_envelope_t env;
    assert(pandora_parse_envelope(NULL, &env) == -1);
    assert(pandora_parse_envelope("{}", NULL) == -1);
    assert(pandora_parse_envelope(NULL, NULL) == -1);

    // Malformed JSON and arbitrary corrupt payloads
    const char* corrupted_json = "<<<INVALID JSON>>> !!! @@ ### \0 TRUNCATED";
    int ret_corrupt = pandora_parse_envelope(corrupted_json, &env);
    assert(ret_corrupt == 0);

    // Massive oversized string buffer overflow resistance
    char massive[8192];
    memset(massive, 'A', sizeof(massive) - 1);
    massive[sizeof(massive) - 1] = '\0';
    int ret_overflow = pandora_parse_envelope(massive, &env);
    assert(ret_overflow == 0);
    assert(env.raw_json[sizeof(env.raw_json) - 1] == '\0');
    printf("  -> Verified envelope parser robust against NULL, malformed, and oversized inputs\n");

    // 4. NULL pointers in SIMD dot product
    float a[128] = {0};
    float b[128] = {0};
    assert(pandora_simd_dot_product_128(NULL, b) == 0.0f);
    assert(pandora_simd_dot_product_128(a, NULL) == 0.0f);
    assert(pandora_simd_dot_product_128(NULL, NULL) == 0.0f);

    // 5. Negative and extreme numbers in SIMD dot product
    for (int i = 0; i < 128; i++) {
        a[i] = -2.5f;
        b[i] = 4.0f;
    }
    // 128 * (-2.5 * 4.0) = 128 * (-10.0) = -1280.0
    float dot_neg = pandora_simd_dot_product_128(a, b);
    assert(fabsf(dot_neg - (-1280.0f)) < 1e-3f);
    printf("  -> Verified 128-d SIMD dot product on negative values: %f\n", dot_neg);

    // 6. Alternating zero and non-zero
    for (int i = 0; i < 128; i++) {
        a[i] = (i % 2 == 0) ? 1.0f : 0.0f;
        b[i] = (i % 2 == 0) ? 0.0f : 1.0f;
    }
    float dot_zero = pandora_simd_dot_product_128(a, b);
    assert(fabsf(dot_zero) < 1e-6f);
    printf("  -> Verified 128-d SIMD dot product on orthogonal sparse vectors: %f\n", dot_zero);

    // 7. CPUID
    bool avx2 = pandora_cpuid_has_avx2();
    printf("  -> Verified CPUID AVX2 detection: %s\n", avx2 ? "supported" : "not supported");

    printf("ALL ADVERSARIAL C ABI TESTS PASSED CLEANLY.\n");
    return 0;
}
