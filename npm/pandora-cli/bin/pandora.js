#!/usr/bin/env node
"use strict";

const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { replaceFile } = require("../lib/launcher-files.js");

const repository = "anisayakmitra-in/PANDORA-AGENT";
const packageVersion = require("../package.json").version;
const MAX_RELEASE_DOWNLOAD_BYTES = 64 * 1024 * 1024;
const MAX_RELEASE_REDIRECTS = 5;

function fail(message) {
  throw new Error(`pandora launcher: ${message}`);
}

function artifactName() {
  const platform = process.platform;
  const architecture = process.arch;
  if (platform === "linux" && architecture === "x64") return "pandora-x86_64-unknown-linux-gnu";
  if (platform === "darwin" && architecture === "x64") return "pandora-x86_64-apple-darwin";
  if (platform === "darwin" && architecture === "arm64") return "pandora-aarch64-apple-darwin";
  if (platform === "win32" && architecture === "x64") return "pandora-x86_64-pc-windows-msvc.exe";
  fail(`unsupported platform or architecture: ${platform} ${architecture}`);
}

function releaseVersion() {
  const version = process.env.PANDORA_VERSION || `v${packageVersion}`;
  if (!/^v[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$/.test(version)) {
    fail("PANDORA_VERSION must be a SemVer tag such as v2.0.0-beta.1");
  }
  return version;
}

function releaseBase() {
  const base = process.env.PANDORA_RELEASE_BASE_URL ||
    `https://github.com/${repository}/releases/download`;
  const parsed = new URL(base);
  if (parsed.protocol !== "https:" || parsed.username || parsed.password || parsed.search || parsed.hash) {
    fail("PANDORA_RELEASE_BASE_URL must use HTTPS without credentials or query parameters");
  }
  return base.replace(/\/$/, "");
}

function cacheDirectory(version, artifact) {
  const root = process.env.PANDORA_CACHE_DIR ||
    (process.platform === "win32"
      ? path.join(process.env.LOCALAPPDATA || os.homedir(), "Pandora", "cache")
      : path.join(process.env.XDG_CACHE_HOME || path.join(os.homedir(), ".cache"), "pandora"));
  return path.join(root, version, artifact);
}

async function fetchBytes(url, allowedHosts) {
  let target = new URL(url);
  for (let redirects = 0; redirects <= MAX_RELEASE_REDIRECTS; redirects += 1) {
    if (target.protocol !== "https:" || target.username || target.password ||
        !allowedHosts.has(target.hostname)) {
      fail("release download redirected to an untrusted host");
    }
    const response = await fetch(target, { redirect: "manual" });
    if (response.status >= 300 && response.status < 400) {
      const location = response.headers.get("location");
      if (!location) fail("release download redirect is missing a location");
      if (redirects === MAX_RELEASE_REDIRECTS) fail("release download exceeded redirect limit");
      target = new URL(location, target);
      continue;
    }
    if (!response.ok) fail(`download failed with HTTP ${response.status}`);
    return readResponseBytes(response);
  }
  fail("release download exceeded redirect limit");
}

async function readResponseBytes(response, maxBytes = MAX_RELEASE_DOWNLOAD_BYTES) {
  const contentLength = response.headers.get("content-length");
  if (contentLength !== null) {
    if (!/^\d+$/.test(contentLength)) fail("release download returned an invalid Content-Length");
    if (Number(contentLength) > maxBytes) fail(`release download exceeds ${maxBytes} bytes`);
  }
  if (!response.body) return Buffer.alloc(0);

  const reader = response.body.getReader();
  const chunks = [];
  let size = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      if (!(value instanceof Uint8Array)) fail("release download returned an invalid body chunk");
      if (value.byteLength > maxBytes - size) {
        try {
          await reader.cancel();
        } catch {}
        fail(`release download exceeds ${maxBytes} bytes`);
      }
      chunks.push(Buffer.from(value.buffer, value.byteOffset, value.byteLength));
      size += value.byteLength;
    }
  } finally {
    reader.releaseLock();
  }
  return Buffer.concat(chunks, size);
}

function checksumManifest(text) {
  const manifest = new Map();
  for (const rawLine of text.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line || line.startsWith("#")) continue;
    const match = line.match(/^([0-9a-fA-F]{64})\s+\*?(\S+)$/);
    if (!match || manifest.has(match[2])) fail("malformed release checksum manifest");
    manifest.set(match[2], match[1].toLowerCase());
  }
  if (manifest.size === 0) fail("empty release checksum manifest");
  return manifest;
}

function verify(payload, expected) {
  return crypto.createHash("sha256").update(payload).digest("hex") === expected;
}

async function verifiedBinary() {
  const version = releaseVersion();
  const artifact = artifactName();
  const cache = cacheDirectory(version, artifact);
  const marker = `${cache}.sha256`;
  const base = releaseBase();
  const baseHost = new URL(base).hostname;
  const allowedHosts = new Set([
    baseHost,
    "github.com",
    "release-assets.githubusercontent.com",
    "objects.githubusercontent.com",
  ]);
  const checksumsUrl = `${base}/${version}/checksums.txt`;
  const artifactUrl = `${base}/${version}/${artifact}`;
  if (process.env.PANDORA_OFFLINE === "1") {
    if (!fs.existsSync(marker)) fail("no verified cached Pandora binary is available offline");
    const cachedExpected = fs.readFileSync(marker, "utf8").trim();
    if (!/^[0-9a-f]{64}$/.test(cachedExpected) || !fs.existsSync(cache) ||
        !verify(fs.readFileSync(cache), cachedExpected)) {
      fail("cached Pandora binary failed checksum verification");
    }
    return cache;
  }
  const checksums = checksumManifest((await fetchBytes(checksumsUrl, allowedHosts)).toString("utf8"));
  const expected = checksums.get(artifact);
  if (!expected) fail(`release checksum is missing for ${artifact}`);
  if (fs.existsSync(cache) && verify(fs.readFileSync(cache), expected)) return cache;
  const bytes = await fetchBytes(artifactUrl, allowedHosts);
  if (!verify(bytes, expected)) fail("release checksum verification failed");

  fs.mkdirSync(path.dirname(cache), { recursive: true });
  replaceFile(cache, bytes, 0o755);
  replaceFile(marker, Buffer.from(`${expected}\n`), 0o600);
  if (process.platform !== "win32") fs.chmodSync(cache, 0o755);
  return cache;
}

async function main() {
  const binary = await verifiedBinary();
  const result = spawnSync(binary, process.argv.slice(2), { stdio: "inherit" });
  if (result.error) throw result.error;
  process.exit(result.status === null ? 1 : result.status);
}

if (require.main === module) {
  main().catch((error) => {
    console.error(error.message);
    process.exit(1);
  });
}

module.exports = {
  MAX_RELEASE_DOWNLOAD_BYTES,
  fetchBytes,
  readResponseBytes,
};
