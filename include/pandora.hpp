/**
 * @file pandora.hpp
 * @brief Modern C++20 Header-Only Client for AGENT-PANDORA v4.1.9.
 *
 * Implements RAII resource management, structured envelope handling,
 * and vector dot product acceleration.
 */

#pragma once

#include "pandora.h"
#include <array>
#include <cstdlib>
#include <memory>
#include <stdexcept>
#include <string>
#include <string_view>
#include <vector>

namespace pandora {

inline constexpr std::string_view kVersion = PANDORA_VERSION;
inline constexpr size_t kDenseDimension = PANDORA_DENSE_DIMENSION;

class Exception : public std::runtime_error {
public:
    explicit Exception(const std::string& message) : std::runtime_error(message) {}
};

struct Envelope {
    std::string version;
    std::string command;
    bool healthy{false};
    std::string raw_json;
};

struct LatsOptions {
    uint32_t rollouts{10};
    uint32_t depth{4};
    double puct_c{1.4};
    double escalation_threshold{0.15};
};

struct TropicalSearchOptions {
    uint32_t limit{5};
    std::string tier{};
    std::string intent{};
};

class Client {
public:
    explicit Client(std::string launcher_path = "pandora")
        : launcher_path_(std::move(launcher_path)), raw_client_(nullptr) {
        pandora_client_t* raw = nullptr;
        if (pandora_client_new(launcher_path_.c_str(), &raw) != 0) {
            throw Exception("Failed to initialize Pandora native C client");
        }
        raw_client_ = raw;
    }

    ~Client() {
        if (raw_client_) {
            pandora_client_free(raw_client_);
            raw_client_ = nullptr;
        }
    }

    // Move-only RAII semantics
    Client(const Client&) = delete;
    Client& operator=(const Client&) = delete;

    Client(Client&& other) noexcept
        : launcher_path_(std::move(other.launcher_path_)), raw_client_(other.raw_client_) {
        other.raw_client_ = nullptr;
    }

    Client& operator=(Client&& other) noexcept {
        if (this != &other) {
            if (raw_client_) {
                pandora_client_free(raw_client_);
            }
            launcher_path_ = std::move(other.launcher_path_);
            raw_client_ = other.raw_client_;
            other.raw_client_ = nullptr;
        }
        return *this;
    }

    [[nodiscard]] std::string_view version() const noexcept {
        return kVersion;
    }

    [[nodiscard]] Envelope ParseEnvelope(const std::string& json_str) const {
        pandora_envelope_t c_env{};
        if (pandora_parse_envelope(json_str.c_str(), &c_env) != 0) {
            throw Exception("Failed to parse JSON envelope: " + json_str);
        }
        return Envelope{
            .version = c_env.version,
            .command = c_env.command,
            .healthy = c_env.healthy,
            .raw_json = c_env.raw_json,
        };
    }

    [[nodiscard]] float SimdDotProduct(
        const std::array<float, kDenseDimension>& a,
        const std::array<float, kDenseDimension>& b
    ) const noexcept {
        return pandora_simd_dot_product_128(a.data(), b.data());
    }

    [[nodiscard]] float SimdDotProductAvx2(
        const std::array<float, kDenseDimension>& a,
        const std::array<float, kDenseDimension>& b
    ) const noexcept {
        return pandora_simd_dot_product_128_avx2(a.data(), b.data());
    }

    [[nodiscard]] float SimdDotProductNeon(
        const std::array<float, kDenseDimension>& a,
        const std::array<float, kDenseDimension>& b
    ) const noexcept {
        return pandora_simd_dot_product_128_neon(a.data(), b.data());
    }

    [[nodiscard]] float SimdDotProductScalar(
        const std::array<float, kDenseDimension>& a,
        const std::array<float, kDenseDimension>& b
    ) const noexcept {
        return pandora_simd_dot_product_128_scalar(a.data(), b.data());
    }

    [[nodiscard]] float TropicalMaxPlus(
        const std::array<float, kDenseDimension>& a,
        const std::array<float, kDenseDimension>& b
    ) const noexcept {
        return pandora_tropical_max_plus_128(a.data(), b.data());
    }

    [[nodiscard]] std::vector<std::string> BuildLatsArgs(
        const std::string& task,
        const LatsOptions& options = {}
    ) const {
        pandora_lats_options_t c_opts{
            .rollouts = options.rollouts,
            .depth = options.depth,
            .puct_c = options.puct_c,
            .escalation_threshold = options.escalation_threshold,
        };
        std::array<const char*, 16> argv{};
        std::array<char, 256> scratch{};
        size_t argc = 0;
        if (pandora_build_lats_args(
                task.c_str(),
                &c_opts,
                argv.data(),
                argv.size(),
                &argc,
                scratch.data(),
                scratch.size()) != 0) {
            throw Exception("Failed to build LATS CLI arguments");
        }
        std::vector<std::string> out;
        out.reserve(argc);
        for (size_t i = 0; i < argc; ++i) {
            out.emplace_back(argv[i]);
        }
        return out;
    }

    [[nodiscard]] std::vector<std::string> BuildTropicalSearchArgs(
        const std::string& query,
        const TropicalSearchOptions& options = {}
    ) const {
        pandora_tropical_search_options_t c_opts{
            .limit = options.limit,
            .tier = options.tier.empty() ? nullptr : options.tier.c_str(),
            .intent = options.intent.empty() ? nullptr : options.intent.c_str(),
        };
        std::array<const char*, 16> argv{};
        std::array<char, 128> scratch{};
        size_t argc = 0;
        if (pandora_build_tropical_search_args(
                query.c_str(),
                &c_opts,
                argv.data(),
                argv.size(),
                &argc,
                scratch.data(),
                scratch.size()) != 0) {
            throw Exception("Failed to build tropical-search CLI arguments");
        }
        std::vector<std::string> out;
        out.reserve(argc);
        for (size_t i = 0; i < argc; ++i) {
            out.emplace_back(argv[i]);
        }
        return out;
    }

    /**
     * Builds CLI argument list for `pandora model verify-attestation --json`.
     */
    [[nodiscard]] std::vector<std::string> BuildVerifyAttestationArgs(
        const std::string& pnn_path = "") const {
        std::array<const char*, 8> argv{};
        size_t argc = 0;
        const char* pnn_cstr = pnn_path.empty() ? nullptr : pnn_path.c_str();
        if (pandora_build_verify_attestation_args(
                pnn_cstr,
                argv.data(),
                argv.size(),
                &argc) != 0) {
            throw Exception("Failed to build verify-attestation CLI arguments");
        }
        std::vector<std::string> out;
        out.reserve(argc);
        for (size_t i = 0; i < argc; ++i) {
            out.emplace_back(argv[i]);
        }
        return out;
    }

    [[nodiscard]] bool HasAvx2() const noexcept {
        return pandora_cpuid_has_avx2();
    }

    [[nodiscard]] bool HasNeon() const noexcept {
        return pandora_cpu_has_neon();
    }

private:
    std::string launcher_path_;
    pandora_client_t* raw_client_;
};

} // namespace pandora
