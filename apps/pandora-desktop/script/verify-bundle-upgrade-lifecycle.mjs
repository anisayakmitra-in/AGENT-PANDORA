#!/usr/bin/env node
import { existsSync, lstatSync, readFileSync, realpathSync } from "node:fs";
import { basename, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  createLifecycleSandbox,
  findBundle,
  lifecycleEnvironment,
  packagedSidecarName,
  platformEvidenceId,
  regularFile,
  removeLifecycleSandboxWithRetry,
  resolveBundleRoot,
  resolveSourceSidecar,
  sha256,
  smokeInstalledBundle,
  systemInstallContract,
  systemInstallLifecycleEnabled,
  validateSidecarTarget,
  writeLifecycleEvidence,
} from "./verify-bundle-lifecycle.mjs";

const stableVersion = /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/;

function parseStableVersion(value, description) {
  const match = stableVersion.exec(value);
  if (!match) throw new Error(`${description} must be a stable semantic version`);
  return match.slice(1).map(Number);
}

export function validateUpgradeManifest(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("desktop upgrade manifest must be an object");
  }
  const keys = Object.keys(value).sort();
  if (keys.join(",") !== "current_version,predecessor_version,schema_version") {
    throw new Error("desktop upgrade manifest has an unexpected shape");
  }
  if (value.schema_version !== 1) throw new Error("unsupported desktop upgrade manifest schema");
  const predecessor = parseStableVersion(value.predecessor_version, "predecessor version");
  const current = parseStableVersion(value.current_version, "current version");
  const comparison = current.findIndex((part, index) => part !== predecessor[index]);
  if (comparison === -1 || current[comparison] <= predecessor[comparison]) {
    throw new Error("current desktop upgrade version must be newer than its predecessor");
  }
  return value;
}

export function readUpgradeManifest(path) {
  const candidate = resolve(path);
  if (!existsSync(candidate)) throw new Error(`desktop upgrade manifest is missing: ${candidate}`);
  const metadata = lstatSync(candidate);
  if (metadata.isSymbolicLink() || !metadata.isFile()) {
    throw new Error("desktop upgrade manifest must be a regular file and not a symlink");
  }
  return validateUpgradeManifest(JSON.parse(readFileSync(realpathSync(candidate), "utf8")));
}

export function selectVersionedBundle(bundleRoot, platform, version) {
  parseStableVersion(version, "bundle version");
  const extension = platform === "linux" ? ".deb" : platform === "darwin" ? ".dmg" : platform === "win32" ? ".msi" : undefined;
  if (!extension) throw new Error(`unsupported desktop upgrade platform: ${platform}`);
  const prefix = `Pandora_${version}_`;
  return findBundle(
    bundleRoot,
    (path) => basename(path).startsWith(prefix) && path.endsWith(extension),
    `${platform} ${version} desktop bundle`,
  );
}

export function resolveUpgradeSources(target, environment = process.env) {
  const predecessorPath = environment.PANDORA_DESKTOP_PREDECESSOR_SIDECAR?.trim();
  const currentPath = environment.PANDORA_DESKTOP_CURRENT_SIDECAR?.trim();
  if (Boolean(predecessorPath) !== Boolean(currentPath)) {
    throw new Error("published desktop rollback requires both predecessor and current sidecars");
  }
  if (!predecessorPath) {
    const source = resolveSourceSidecar(target, true, environment);
    return { predecessor: source, current: source };
  }
  return {
    predecessor: resolveSourceSidecar(target, true, {
      ...environment,
      PANDORA_DESKTOP_SOURCE_SIDECAR: predecessorPath,
    }),
    current: resolveSourceSidecar(target, true, {
      ...environment,
      PANDORA_DESKTOP_SOURCE_SIDECAR: currentPath,
    }),
  };
}

async function verifyInstalledStage(contract, platform, target, source, expectedVersion, environment) {
  const actualVersion = contract.version().trim();
  if (actualVersion !== expectedVersion) {
    throw new Error(`expected installed desktop ${expectedVersion}, found ${actualVersion}`);
  }
  const bundledSidecar = findBundle(
    contract.installed,
    (path) => basename(path) === packagedSidecarName(target),
    "installed upgrade-drill sidecar",
  );
  regularFile(bundledSidecar, "installed upgrade-drill sidecar");
  if (sha256(source) !== sha256(bundledSidecar)) {
    throw new Error("upgrade-drill sidecar differs from the same-commit release binary");
  }
  await smokeInstalledBundle(contract.installed, platform, target, environment);
}

async function main() {
  if (!systemInstallLifecycleEnabled()) {
    throw new Error("desktop upgrade drill requires the CI-only system-install contract");
  }
  const manifestPath = process.env.PANDORA_DESKTOP_UPGRADE_MANIFEST?.trim();
  if (!manifestPath) throw new Error("PANDORA_DESKTOP_UPGRADE_MANIFEST is required");
  const manifest = readUpgradeManifest(manifestPath);
  const target = validateSidecarTarget(process.env.PANDORA_SIDECAR_TARGET ?? "");
  const sources = resolveUpgradeSources(target);
  const predecessorSource = sources.predecessor;
  const currentSource = sources.current;
  const bundleRoot = resolveBundleRoot();
  const predecessorBundle = selectVersionedBundle(bundleRoot, process.platform, manifest.predecessor_version);
  const currentBundle = selectVersionedBundle(bundleRoot, process.platform, manifest.current_version);
  const sandbox = createLifecycleSandbox();
  const environment = lifecycleEnvironment(sandbox);
  const predecessor = systemInstallContract(bundleRoot, sandbox, process.platform, predecessorBundle);
  const current = systemInstallContract(bundleRoot, sandbox, process.platform, currentBundle);
  let lifecycleEvidence;
  try {
    predecessor.install();
    await verifyInstalledStage(predecessor, process.platform, target, predecessorSource, manifest.predecessor_version, environment);

    current.replace();
    await verifyInstalledStage(current, process.platform, target, currentSource, manifest.current_version, environment);

    if (process.platform === "win32") {
      current.uninstall();
      predecessor.install();
    } else {
      predecessor.replace();
    }
    await verifyInstalledStage(predecessor, process.platform, target, predecessorSource, manifest.predecessor_version, environment);

    predecessor.uninstall();
    predecessor.assertUninstalled();
    lifecycleEvidence = {
      schema_version: 1,
      generated_at: new Date().toISOString(),
      commit_sha: process.env.GITHUB_SHA ?? null,
      platform: platformEvidenceId(process.platform, target),
      target,
      runner: {
        os: process.env.RUNNER_OS ?? process.platform,
        architecture: process.env.RUNNER_ARCH ?? process.arch,
        ephemeral_ci: process.env.CI?.trim().toLowerCase() === "true",
      },
      artifacts: [
        { name: basename(predecessorBundle), sha256: sha256(predecessorBundle) },
        { name: basename(currentBundle), sha256: sha256(currentBundle) },
      ],
      release_identity: {
        predecessor_version: manifest.predecessor_version,
        current_version: manifest.current_version,
        predecessor_sidecar_sha256: sha256(predecessorSource),
        current_sidecar_sha256: sha256(currentSource),
      },
      lifecycle: {
        clean_install: true,
        start_predecessor: true,
        update: true,
        start_current: true,
        rollback: true,
        start_rollback: true,
        uninstall: true,
        sandbox_cleanup: true,
      },
    };
    console.log(`verified ${process.platform} desktop install, update, rollback, and uninstall`);
  } finally {
    try {
      current.uninstall(true);
      predecessor.uninstall(true);
      predecessor.assertUninstalled();
    } finally {
      await removeLifecycleSandboxWithRetry(sandbox);
    }
  }
  const evidenceOutput = process.env.PANDORA_DESKTOP_UPGRADE_EVIDENCE_OUTPUT?.trim();
  if (evidenceOutput && lifecycleEvidence) {
    writeLifecycleEvidence(evidenceOutput, lifecycleEvidence);
    console.log(`desktop upgrade lifecycle evidence written to ${evidenceOutput}`);
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  });
}
