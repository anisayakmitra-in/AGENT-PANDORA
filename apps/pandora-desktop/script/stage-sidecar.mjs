#!/usr/bin/env node
import { chmodSync, copyFileSync, lstatSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = resolve(desktopRoot, "..", "..");
const release = process.argv.includes("--release");
const requestedTarget = (process.env.PANDORA_SIDECAR_TARGET ?? "").trim();

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: repositoryRoot,
    encoding: "utf8",
    shell: false,
    stdio: ["ignore", "pipe", "inherit"],
  });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
  return result.stdout.trim();
}

const metadata = JSON.parse(run("cargo", ["metadata", "--format-version", "1", "--no-deps", "--locked"]));
const hostTarget = run("rustc", ["--print", "host-tuple"]);
const target = requestedTarget || hostTarget;
if (!/^[A-Za-z0-9_.-]+$/.test(target)) throw new Error("invalid Pandora sidecar target triple");

const buildArgs = ["build", "--locked", "-p", "pandora-cli", "--bin", "pandora"];
if (release) buildArgs.push("--release");
if (requestedTarget) buildArgs.push("--target", target);
run("cargo", buildArgs);

const profile = release ? "release" : "debug";
const extension = target.includes("windows") ? ".exe" : "";
const source = join(metadata.target_directory, ...(requestedTarget ? [target] : []), profile, `pandora${extension}`);
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
