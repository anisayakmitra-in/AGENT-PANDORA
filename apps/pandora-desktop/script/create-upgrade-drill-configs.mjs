#!/usr/bin/env node
import { constants, copyFileSync, lstatSync, mkdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { resolveBundleRoot } from "./verify-bundle-lifecycle.mjs";
import { selectVersionedBundle } from "./verify-bundle-upgrade-lifecycle.mjs";

const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

export function upgradeDrillConfiguration(packageVersion, platform) {
  const match = /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-(?:alpha|beta|rc)\.(0|[1-9][0-9]*))?$/.exec(packageVersion);
  if (!match) throw new Error(`unsupported desktop version for upgrade drill: ${packageVersion}`);
  const [major, minor, patch] = match.slice(1, 4).map(Number);
  if (patch >= 65_535) throw new Error("desktop patch version is too large for a bounded upgrade drill");
  const predecessorVersion = `${major}.${minor}.${patch}`;
  const currentVersion = `${major}.${minor}.${patch + 1}`;
  const target = platform === "linux" ? "deb" : platform === "darwin" ? "dmg" : platform === "win32" ? "msi" : undefined;
  if (!target) throw new Error(`unsupported upgrade drill platform: ${platform}`);

  const config = (version) => ({
    version,
    bundle: {
      targets: [target],
      ...(platform === "win32" ? { windows: { wix: { version } } } : {}),
    },
  });
  return {
    manifest: {
      schema_version: 1,
      predecessor_version: predecessorVersion,
      current_version: currentVersion,
    },
    predecessor: config(predecessorVersion),
    current: config(currentVersion),
  };
}

export function writeUpgradeDrillConfiguration(outputDirectory, packageVersion, platform) {
  const output = resolve(outputDirectory);
  mkdirSync(output, { recursive: true });
  const metadata = lstatSync(output);
  if (metadata.isSymbolicLink() || !metadata.isDirectory()) {
    throw new Error("upgrade drill output must be a directory and not a symlink");
  }
  const canonicalOutput = realpathSync(output);
  const configuration = upgradeDrillConfiguration(packageVersion, platform);
  for (const [name, value] of Object.entries(configuration)) {
    writeFileSync(join(canonicalOutput, `${name}.json`), `${JSON.stringify(value, null, 2)}\n`, {
      encoding: "utf8",
      flag: "wx",
    });
  }
  return canonicalOutput;
}

export function preserveUpgradeDrillBundle(bundleRoot, outputDirectory, platform, version) {
  const source = selectVersionedBundle(resolve(bundleRoot), platform, version);
  const output = resolve(outputDirectory);
  mkdirSync(output, { recursive: true });
  const metadata = lstatSync(output);
  if (metadata.isSymbolicLink() || !metadata.isDirectory()) {
    throw new Error("preserved upgrade bundle output must be a directory and not a symlink");
  }
  const destination = join(realpathSync(output), basename(source));
  copyFileSync(source, destination, constants.COPYFILE_EXCL);
  return destination;
}

function argumentValue(name) {
  const index = process.argv.indexOf(name);
  if (index === -1 || index + 1 >= process.argv.length) throw new Error(`missing ${name}`);
  return process.argv[index + 1];
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    if (process.argv.includes("--preserve")) {
      const stage = argumentValue("--preserve");
      if (stage !== "predecessor" && stage !== "current") {
        throw new Error("--preserve must be predecessor or current");
      }
      const manifest = JSON.parse(readFileSync(argumentValue("--manifest"), "utf8"));
      const version = manifest[`${stage}_version`];
      const preserved = preserveUpgradeDrillBundle(
        resolveBundleRoot(),
        argumentValue("--output-dir"),
        process.platform,
        version,
      );
      console.log(`preserved ${stage} desktop bundle at ${preserved}`);
      process.exit(0);
    }
    const packageJson = JSON.parse(readFileSync(join(desktopRoot, "package.json"), "utf8"));
    const output = writeUpgradeDrillConfiguration(
      argumentValue("--output-dir"),
      packageJson.version,
      process.platform,
    );
    console.log(`wrote desktop upgrade drill configuration to ${output}`);
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  }
}
