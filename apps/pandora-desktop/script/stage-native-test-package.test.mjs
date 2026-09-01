import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { stageNativeTestPackage } from "./stage-native-test-package.mjs";

const commit = "e385141cacb44fcf761e4169e36bf3584fa8889e";

function fixture() {
  const root = mkdtempSync(join(tmpdir(), "pandora-native-test-package-"));
  const bundleRoot = join(root, "bundle");
  const outputDirectory = join(root, "output");
  const sidecar = join(root, "pandora-x86_64-pc-windows-msvc.exe");
  mkdirSync(join(bundleRoot, "msi"), { recursive: true });
  writeFileSync(join(bundleRoot, "msi", "Pandora_2.0.0-beta.7_x64_en-US.msi"), "bundle");
  writeFileSync(sidecar, "sidecar");
  return { root, bundleRoot, outputDirectory, sidecar };
}

test("stages one exact unsigned native test bundle and sidecar", () => {
  const paths = fixture();
  try {
    const result = stageNativeTestPackage({
      bundleRoot: paths.bundleRoot,
      commit,
      outputDirectory: paths.outputDirectory,
      platform: "windows-x64",
      runtimePlatform: "win32",
      sidecar: paths.sidecar,
      target: "x86_64-pc-windows-msvc",
      version: "2.0.0-beta.7",
    });
    assert.equal(result.manifest.source_commit, commit);
    assert.equal(result.manifest.signed_release_artifact, false);
    assert.deepEqual(
      result.manifest.files.map((file) => file.name).sort(),
      ["Pandora_2.0.0-beta.7_x64_en-US.msi", "pandora-x86_64-pc-windows-msvc.exe"],
    );
    const written = JSON.parse(readFileSync(join(result.output, "native-test-package.json"), "utf8"));
    assert.deepEqual(written, result.manifest);
    assert.match(
      readFileSync(join(result.output, "UNSIGNED-NATIVE-TEST-ONLY.txt"), "utf8"),
      /not a release artifact/,
    );
  } finally {
    rmSync(paths.root, { recursive: true, force: true });
  }
});

test("rejects identity mismatches, ambiguity, and a reused output", () => {
  const paths = fixture();
  const options = {
    bundleRoot: paths.bundleRoot,
    commit,
    outputDirectory: paths.outputDirectory,
    platform: "windows-x64",
    runtimePlatform: "win32",
    sidecar: paths.sidecar,
    target: "x86_64-pc-windows-msvc",
    version: "2.0.0-beta.7",
  };
  try {
    assert.throws(() => stageNativeTestPackage({ ...options, commit: "A".repeat(40) }), /lowercase 40-character/);
    assert.throws(() => stageNativeTestPackage({ ...options, platform: "linux-x64" }), /does not match/);
    writeFileSync(join(paths.bundleRoot, "msi", "Pandora_2.0.0-beta.7_duplicate.msi"), "duplicate");
    assert.throws(() => stageNativeTestPackage(options), /found 2/);
    rmSync(join(paths.bundleRoot, "msi", "Pandora_2.0.0-beta.7_duplicate.msi"));
    mkdirSync(paths.outputDirectory);
    writeFileSync(join(paths.outputDirectory, "occupied"), "occupied");
    assert.throws(() => stageNativeTestPackage(options), /must be empty/);
  } finally {
    rmSync(paths.root, { recursive: true, force: true });
  }
});
