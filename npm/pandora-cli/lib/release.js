"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.resolveArtifactName = resolveArtifactName;
exports.normalizeReleaseVersion = normalizeReleaseVersion;
exports.resolveReleaseVersion = resolveReleaseVersion;
function resolveArtifactName(platform = process.platform, architecture = process.arch) {
    if (platform === "linux" && architecture === "x64") {
        return "pandora-x86_64-unknown-linux-gnu";
    }
    if (platform === "darwin" && architecture === "x64") {
        return "pandora-x86_64-apple-darwin";
    }
    if (platform === "darwin" && architecture === "arm64") {
        return "pandora-aarch64-apple-darwin";
    }
    if (platform === "win32" && architecture === "x64") {
        return "pandora-x86_64-pc-windows-msvc.exe";
    }
    throw new Error(`unsupported platform or architecture: ${platform} ${architecture}`);
}
function normalizeReleaseVersion(version) {
    if (!/^v[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$/.test(version)) {
        throw new Error("PANDORA_VERSION must be a SemVer tag such as v2.0.0-beta.1");
    }
    return version;
}
function resolveReleaseVersion(packageVersion, override = process.env.PANDORA_VERSION) {
    return normalizeReleaseVersion(override || `v${packageVersion}`);
}
