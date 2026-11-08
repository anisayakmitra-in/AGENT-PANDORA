/**
 * @file pandora.hpp
 * @brief Modern C++20 Header-Only Client for AGENT-PANDORA v3.0.6.
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
