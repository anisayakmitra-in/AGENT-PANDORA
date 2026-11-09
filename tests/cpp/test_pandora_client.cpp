/**
 * @file test_pandora_client.cpp
 * @brief Verification suite for Pandora C++20 RAII Client & Vector Ops.
 */

#include "pandora.hpp"
#include <cassert>
#include <cmath>
#include <iostream>

int main() {
    std::cout << "[C++20 Client Test] Verifying version...\n";
    pandora::Client client;
    assert(client.version() == "3.0.9");
    std::cout << "  -> Client version: " << client.version() << "\n";

    std::cout << "[C++20 Client Test] Verifying envelope parsing...\n";
    auto env = client.ParseEnvelope(
        "{\"version\":\"3.0.9\",\"command\":\"workspace.verify\",\"healthy\":true}"
    );
    assert(env.version == "3.0.9");
    assert(env.command == "workspace.verify");
    assert(env.healthy == true);
    std::cout << "  -> Parsed envelope: " << env.command << ", healthy=" << env.healthy << "\n";

    std::cout << "[C++20 Client Test] Verifying RAII Move Semantics...\n";
    pandora::Client moved_client = std::move(client);
    assert(moved_client.version() == "3.0.9");
    std::cout << "  -> Move constructor and RAII transfer passed\n";

    std::cout << "[C++20 Client Test] Verifying SIMD vector dot product...\n";
    std::array<float, 128> vec_a{};
    std::array<float, 128> vec_b{};
    for (size_t i = 0; i < 128; ++i) {
        vec_a[i] = 1.5f;
        vec_b[i] = 2.0f;
    }
    float dot = moved_client.SimdDotProduct(vec_a, vec_b);
    assert(std::fabs(dot - 384.0f) < 1e-4);
    std::cout << "  -> SIMD dot product (128-d, 1.5 * 2.0 * 128): " << dot << "\n";

    float dot_avx2 = moved_client.SimdDotProductAvx2(vec_a, vec_b);
    assert(std::fabs(dot_avx2 - 384.0f) < 1e-4);
    std::cout << "  -> AVX2 SIMD dot product (128-d): " << dot_avx2 << "\n";

    float dot_scalar = moved_client.SimdDotProductScalar(vec_a, vec_b);
    assert(std::fabs(dot_scalar - 384.0f) < 1e-4);
    std::cout << "  -> Scalar reference dot product: " << dot_scalar << "\n";

    std::cout << "[C++20 Client Test] Checking CPUID AVX2 detection...\n";
    bool avx2 = moved_client.HasAvx2();
    std::cout << "  -> Host AVX2 support: " << (avx2 ? "true" : "false") << "\n";

    std::cout << "ALL C++20 TESTS PASSED CLEANLY.\n";
    return 0;
}
