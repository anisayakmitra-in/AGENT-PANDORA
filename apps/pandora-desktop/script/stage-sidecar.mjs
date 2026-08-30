#!/usr/bin/env node
import { chmodSync, copyFileSync, lstatSync, mkdirSync, realpathSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = resolve(desktopRoot, "..", "..");

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: repositoryRoot,
    encoding: "utf8",
    shell: false,
    stdio: ["ignore", "pipe", "inherit"],
  });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} ${args.join(" ")} failed`);
  return result.stdout.trim();
}

export function validateSidecarTarget(target) {
  const normalized = target.trim();
  if (!/^[A-Za-z0-9_.-]+$/.test(normalized)) {
    throw new Error("invalid Pandora sidecar target triple");
  }
  return normalized;
}

export function resolveConfiguredSource(requestedSource, target) {
  const configuredSource = resolve(requestedSource);
  const extension = target.includes("windows") ? ".exe" : "";
  const expectedSourceName = `pandora-${target}${extension}`;
  if (basename(configuredSource) !== expectedSourceName) {
    throw new Error(`configured Pandora sidecar must be named ${expectedSourceName}`);
  }
  const configuredMetadata = lstatSync(configuredSource);
  if (configuredMetadata.isSymbolicLink() || !configuredMetadata.isFile()) {
    throw new Error("configured Pandora sidecar is not a regular file");
  }
  return realpathSync(configuredSource);
}

export function stageSidecar(environment = process.env, argumentsList = process.argv) {
  const release = argumentsList.includes("--release");
  const requestedTarget = (environment.PANDORA_SIDECAR_TARGET ?? "").trim();
  const requestedSource = (environment.PANDORA_SIDECAR_SOURCE ?? "").trim();
  const metadata = JSON.parse(run("cargo", ["metadata", "--format-version", "1", "--no-deps", "--locked"]));
  const hostTarget = run("rustc", ["--print", "host-tuple"]);
  const target = validateSidecarTarget(requestedTarget || hostTarget);
  const extension = target.includes("windows") ? ".exe" : "";

  let source;
  if (requestedSource) {
    source = resolveConfiguredSource(requestedSource, target);
  } else {
    const buildArgs = ["build", "--locked", "-p", "pandora-cli", "--bin", "pandora"];
    if (release) buildArgs.push("--release");
    if (requestedTarget) buildArgs.push("--target", target);
    run("cargo", buildArgs);
    const profile = release ? "release" : "debug";
    source = join(metadata.target_directory, ...(requestedTarget ? [target] : []), profile, `pandora${extension}`);
  }
  const sourceMetadata = lstatSync(source);
  if (sourceMetadata.isSymbolicLink() || !sourceMetadata.isFile()) throw new Error("built Pandora sidecar is not a regular file");

  const destinationDirectory = join(desktopRoot, "src-tauri", "binaries");
  const destination = join(destinationDirectory, `pandora-${target}${extension}`);
  mkdirSync(destinationDirectory, { recursive: true });
  copyFileSync(source, destination);
  if (!extension) chmodSync(destination, 0o755);
  const destinationMetadata = lstatSync(destination);
  if (destinationMetadata.isSymbolicLink() || !destinationMetadata.isFile()) throw new Error("staged Pandora sidecar is not a regular file");
  console.log(`staged ${destination}`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    stageSidecar();
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  }
}
