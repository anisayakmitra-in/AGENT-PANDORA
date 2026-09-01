#!/usr/bin/env node
import {
  chmodSync,
  constants,
  copyFileSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  realpathSync,
  writeFileSync,
} from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  findBundle,
  regularFile,
  resolveBundleRoot,
  sha256,
  sidecarName,
  validateSidecarTarget,
} from "./verify-bundle-lifecycle.mjs";

const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const commitPattern = /^[0-9a-f]{40}$/;
const versionPattern = /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-(?:alpha|beta|rc)\.(0|[1-9][0-9]*))?$/;
const platforms = {
  "linux-x64": {
    runtime: "linux",
    target: "x86_64-unknown-linux-gnu",
    extension: ".deb",
  },
  "macos-x64": {
    runtime: "darwin",
    target: "x86_64-apple-darwin",
    extension: ".dmg",
  },
  "macos-arm64": {
    runtime: "darwin",
    target: "aarch64-apple-darwin",
    extension: ".dmg",
  },
  "windows-x64": {
    runtime: "win32",
    target: "x86_64-pc-windows-msvc",
    extension: ".msi",
  },
};

function regularDirectory(path, description) {
  const metadata = lstatSync(path);
  if (metadata.isSymbolicLink() || !metadata.isDirectory()) {
    throw new Error(`${description} must be a directory and not a symlink`);
  }
  return realpathSync(path);
}

function createEmptyOutput(path) {
  const requested = resolve(path);
  mkdirSync(requested, { recursive: true });
  const output = regularDirectory(requested, "native test package output");
  if (readdirSync(output).length !== 0) {
    throw new Error("native test package output must be empty");
  }
  return output;
}

function validateInputs({ commit, platform, runtimePlatform, target, version }) {
  if (!commitPattern.test(commit)) throw new Error("source commit must be a lowercase 40-character SHA");
  if (!versionPattern.test(version)) throw new Error("desktop version must be a release-train semantic version");
  const definition = platforms[platform];
  if (!definition) throw new Error(`unsupported native test platform: ${platform}`);
  const normalizedTarget = validateSidecarTarget(target);
  if (definition.runtime !== runtimePlatform || definition.target !== normalizedTarget) {
    throw new Error(`native test platform ${platform} does not match ${runtimePlatform}/${normalizedTarget}`);
  }
  return { ...definition, target: normalizedTarget };
}

export function stageNativeTestPackage({
  bundleRoot,
  commit,
  outputDirectory,
  platform,
  runtimePlatform = process.platform,
  sidecar,
  target,
  version,
}) {
  const definition = validateInputs({ commit, platform, runtimePlatform, target, version });
  const canonicalBundleRoot = regularDirectory(resolve(bundleRoot), "desktop bundle root");
  const prefix = `Pandora_${version}_`;
  const bundle = findBundle(
    canonicalBundleRoot,
    (candidate) => basename(candidate).startsWith(prefix) && candidate.endsWith(definition.extension),
    `${platform} ${version} native test bundle`,
  );
  regularFile(bundle, "native test bundle");

  const expectedSidecarName = sidecarName(definition.target);
  const sourceSidecar = resolve(sidecar);
  if (basename(sourceSidecar) !== expectedSidecarName) {
    throw new Error(`native test sidecar must be named ${expectedSidecarName}`);
  }
  regularFile(sourceSidecar, "native test sidecar");

  const output = createEmptyOutput(outputDirectory);
  const bundleOutput = join(output, basename(bundle));
  const sidecarOutput = join(output, expectedSidecarName);
  copyFileSync(bundle, bundleOutput, constants.COPYFILE_EXCL);
  copyFileSync(realpathSync(sourceSidecar), sidecarOutput, constants.COPYFILE_EXCL);
  if (!expectedSidecarName.endsWith(".exe")) chmodSync(sidecarOutput, 0o755);

  const files = [bundleOutput, sidecarOutput].map((path) => ({
    name: basename(path),
    bytes: lstatSync(path).size,
    sha256: sha256(path),
  }));
  const manifest = {
    schema_version: 1,
    source_commit: commit,
    platform,
    target: definition.target,
    release_identity: version,
    signed_release_artifact: false,
    purpose: "native-accessibility-testing-only",
    files,
  };
  writeFileSync(join(output, "native-test-package.json"), `${JSON.stringify(manifest, null, 2)}\n`, {
    encoding: "utf8",
    flag: "wx",
  });
  writeFileSync(
    join(output, "UNSIGNED-NATIVE-TEST-ONLY.txt"),
    [
      "This package is retained only for reviewed native accessibility testing.",
      "It is not a release artifact and must not be published or represented as signed.",
      `Source commit: ${commit}`,
      `Release identity: ${version}`,
      "Use only when the complete main-branch CI run for this commit succeeded.",
      "",
    ].join("\n"),
    { encoding: "utf8", flag: "wx" },
  );
  return { output, manifest };
}

function argumentValue(name) {
  const index = process.argv.indexOf(name);
  if (index === -1 || index + 1 >= process.argv.length) throw new Error(`missing ${name}`);
  return process.argv[index + 1];
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    const packageJson = JSON.parse(readFileSync(join(desktopRoot, "package.json"), "utf8"));
    const target = validateSidecarTarget(process.env.PANDORA_SIDECAR_TARGET ?? "");
    const result = stageNativeTestPackage({
      bundleRoot: resolveBundleRoot(),
      commit: argumentValue("--commit"),
      outputDirectory: argumentValue("--output-dir"),
      platform: argumentValue("--platform"),
      sidecar: join(desktopRoot, "src-tauri", "binaries", sidecarName(target)),
      target,
      version: packageJson.version,
    });
    console.log(`staged unsigned native test package at ${result.output}`);
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  }
}
