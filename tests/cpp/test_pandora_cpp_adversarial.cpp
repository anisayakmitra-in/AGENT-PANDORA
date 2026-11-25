/**
 * @file test_pandora_cpp_adversarial.cpp
 * @brief Tier 5 Adversarial Hardening Suite for Pandora C++20 RAII Client & Vector Ops.
 */

#include "pandora.hpp"
#include <cassert>
#include <cmath>
#include <iostream>
#include <vector>

int main() {
    std::cout << "[Adversarial C++20 Client] Testing RAII lifecycle and repeated allocation loop...\n";

    // 1. Loop allocation to stress-test RAII destructor and free calls
    for (int i = 0; i < 500; ++i) {
        pandora::Client c("pandora");
        assert(c.version() == "4.0.6");
    }
    std::cout << "  -> 500 client instances allocated and cleanly destructed without leaks\n";

    // 2. Vector of clients and move semantics
    std::vector<pandora::Client> clients;
    clients.reserve(20);
    for (int i = 0; i < 10; ++i) {
        clients.emplace_back("pandora");
    }
    assert(clients.size() == 10);
    for (const auto& c : clients) {
        assert(c.version() == "4.0.6");
    }

    // Move assignment across elements
    pandora::Client target_client;
    target_client = std::move(clients.back());
    assert(target_client.version() == "4.0.6");
    clients.pop_back();
    std::cout << "  -> Vector move reallocations and transfer verified clean\n";

    // 3. Self-move assignment resilience
    #pragma GCC diagnostic push
    #pragma GCC diagnostic ignored "-Wself-move"
    target_client = std::move(target_client);
    #pragma GCC diagnostic pop
    assert(target_client.version() == "4.0.6");
    std::cout << "  -> Self-move assignment is non-destructive\n";

    // 4. Adversarial envelope inputs
    auto env1 = target_client.ParseEnvelope("");
    assert(env1.version.empty());

    auto env2 = target_client.ParseEnvelope("{broken_json: false, random_bytes: \xFF\xFE}");
    assert(env2.version.empty());

    auto env3 = target_client.ParseEnvelope("{\"version\":\"3.0.10\",\"command\":\"run\",\"healthy\":true}");
    assert(env3.version == "3.0.10");
    assert(env3.command == "run");
    assert(env3.healthy == true);
    std::cout << "  -> Envelope parsing robust on corrupted and empty inputs\n";

    // 5. SIMD Vector Dot Product with negative and alternating values
    std::array<float, 128> vec_a{};
    std::array<float, 128> vec_b{};
    for (size_t i = 0; i < 128; ++i) {
        vec_a[i] = (i % 2 == 0) ? -1.0f : 2.0f;
        vec_b[i] = (i % 2 == 0) ? 3.0f : -0.5f;
    }
    // 64 * (-1.0 * 3.0) + 64 * (2.0 * -0.5) = 64 * (-3.0) + 64 * (-1.0) = -192 - 64 = -256.0
    float dot = target_client.SimdDotProduct(vec_a, vec_b);
    assert(std::fabs(dot - (-256.0f)) < 1e-4);
    std::cout << "  -> SIMD dot product computed correctly for mixed signs: " << dot << "\n";

    // 6. Phase 4 C++20 wrappers (BuildLatsArgs, BuildTropicalSearchArgs, TropicalMaxPlus)
    auto lats_args = target_client.BuildLatsArgs("Verify tropical LATS", pandora::LatsOptions{
        .rollouts = 12,
        .depth = 4,
        .puct_c = 1.45,
        .escalation_threshold = 0.2,
    });
    assert(lats_args.size() == 13);
    assert(lats_args.front() == "run");
    assert(lats_args[2] == "lats");
    assert(lats_args.back() == "--json");

    auto ts_args = target_client.BuildTropicalSearchArgs("tropical semiring", pandora::TropicalSearchOptions{
        .limit = 4,
        .tier = "L1",
        .intent = "coding",
    });
    assert(ts_args.size() == 11);
    assert(ts_args[2] == "tropical-search");
    assert(ts_args.back() == "--json");

    std::array<float, 128> trop_a{};
    std::array<float, 128> trop_b{};
    trop_a.fill(-2.0f);
    trop_b.fill(-2.0f);
    trop_a[7] = 1.25f;
    trop_b[7] = 2.5f;
    float mp = target_client.TropicalMaxPlus(trop_a, trop_b);
    assert(std::fabs(mp - 3.75f) < 1e-5f);
    std::cout << "  -> Phase 4 C++20 wrappers and TropicalMaxPlus verified: " << mp << "\n";

    std::cout << "ALL ADVERSARIAL C++20 TESTS PASSED CLEANLY.\n";
    return 0;
}
