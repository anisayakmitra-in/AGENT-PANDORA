/**
 * @file pandora_c_api.c
 * @brief Implementation of C ABI for AGENT-PANDORA v3.0.1.
 */

#include "pandora.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifdef __cplusplus
extern "C" {
#endif
float pandora_simd_dot_product_128_asm(const float* a, const float* b);
void pandora_cpuid_features_asm(unsigned int leaf, unsigned int* eax, unsigned int* ebx, unsigned int* ecx, unsigned int* edx);
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

float pandora_simd_dot_product_128(const float* a, const float* b) {
    if (!a || !b) return 0.0f;
    return pandora_simd_dot_product_128_asm(a, b);
}

bool pandora_cpuid_has_avx2(void) {
    unsigned int eax = 0, ebx = 0, ecx = 0, edx = 0;
    // Leaf 7, Subleaf 0: EBX bit 5 = AVX2
    pandora_cpuid_features_asm(7, &eax, &ebx, &ecx, &edx);
    return (ebx & (1 << 5)) != 0;
}
