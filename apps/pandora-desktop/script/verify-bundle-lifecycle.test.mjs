import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, realpathSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  assertTemporarySandbox,
  applicationBinary,
  createLifecycleSandbox,
  packagedSidecarName,
  removeLifecycleSandbox,
  resolveBundleRoot,
  resolveSourceSidecar,
  sidecarName,
  validateSidecarTarget,
} from "./verify-bundle-lifecycle.mjs";

test("distinguishes staged and packaged sidecar filenames", () => {
  assert.equal(sidecarName("x86_64-unknown-linux-gnu"), "pandora-x86_64-unknown-linux-gnu");
  assert.equal(sidecarName("x86_64-pc-windows-msvc"), "pandora-x86_64-pc-windows-msvc.exe");
  assert.equal(packagedSidecarName("x86_64-unknown-linux-gnu"), "pandora");
  assert.equal(packagedSidecarName("x86_64-pc-windows-msvc"), "pandora.exe");
});

test("accepts and removes only its exact lifecycle sandbox", () => {
  const sandbox = createLifecycleSandbox();
  try {
    assert.doesNotThrow(() => assertTemporarySandbox(sandbox));
    removeLifecycleSandbox(sandbox);
    assert.equal(existsSync(sandbox), false);
  } finally {
    if (existsSync(sandbox)) removeLifecycleSandbox(sandbox);
  }
});

test("rejects same-prefix siblings, symlinks, and non-directories", () => {
  const sibling = join(tmpdir(), "pandora-desktop-lifecycle-sibling");
  const file = join(tmpdir(), "pandora-desktop-lifecycle-file");
  const sandbox = createLifecycleSandbox();
  const link = join(tmpdir(), "pandora-desktop-lifecycle-link");
  try {
    mkdirSync(sibling);
    writeFileSync(file, "not a sandbox");
    symlinkSync(sandbox, link, process.platform === "win32" ? "junction" : "dir");
    assert.throws(() => assertTemporarySandbox(sibling), /outside the lifecycle sandbox/);
    assert.throws(() => assertTemporarySandbox(file), /outside the lifecycle sandbox/);
    assert.throws(() => assertTemporarySandbox(link), /outside the lifecycle sandbox/);
  } finally {
    rmSync(sibling, { recursive: true, force: true });
    rmSync(file, { force: true });
    rmSync(link, { force: true });
    if (existsSync(sandbox)) removeLifecycleSandbox(sandbox);
  }
});

test("accepts directories through canonical ancestor aliases but rejects final symlinks", () => {
  const bundleRoot = mkdtempSync(join(tmpdir(), "pandora-desktop-bundle-"));
  const nested = join(bundleRoot, "nested");
  mkdirSync(nested);
  const link = join(tmpdir(), `pandora-desktop-bundle-link-${process.pid}-${Date.now()}`);
  try {
    assert.equal(
      resolveBundleRoot({ PANDORA_DESKTOP_BUNDLE_ROOT: ` ${bundleRoot} ` }),
      realpathSync(bundleRoot),
    );
    symlinkSync(bundleRoot, link, process.platform === "win32" ? "junction" : "dir");
    assert.equal(
      resolveBundleRoot({ PANDORA_DESKTOP_BUNDLE_ROOT: join(link, "nested") }),
      realpathSync(nested),
    );
    assert.throws(
      () => resolveBundleRoot({ PANDORA_DESKTOP_BUNDLE_ROOT: link }),
      /must be a directory and not a symlink/,
    );
    assert.throws(
      () => resolveBundleRoot({ PANDORA_DESKTOP_BUNDLE_ROOT: join(bundleRoot, "missing") }),
      /directory is missing/,
    );
  } finally {
    rmSync(link, { force: true });
    rmSync(bundleRoot, { recursive: true, force: true });
  }
});

test("selects the exact desktop binary instead of sidecars and resources", () => {
  const installed = mkdtempSync(join(tmpdir(), "pandora-desktop-installed-"));
  const binaries = join(installed, "usr", "bin");
  const icons = join(installed, "usr", "share", "icons");
  try {
    mkdirSync(binaries, { recursive: true });
    mkdirSync(icons, { recursive: true });
    for (const name of ["pandora", "pandora-desktop", "pandora.exe", "pandora-desktop.exe"]) {
      writeFileSync(join(binaries, name), name);
    }
    writeFileSync(join(icons, "pandora-desktop.png"), "icon");
    assert.equal(applicationBinary(installed, "linux"), join(binaries, "pandora-desktop"));
    assert.equal(applicationBinary(installed, "win32"), join(binaries, "pandora-desktop.exe"));
  } finally {
    rmSync(installed, { recursive: true, force: true });
  }
});

test("rejects sidecar target traversal before path construction", () => {
  assert.equal(validateSidecarTarget(" x86_64-unknown-linux-gnu "), "x86_64-unknown-linux-gnu");
  assert.throws(() => validateSidecarTarget("../release"), /invalid Pandora sidecar target triple/);
  assert.throws(() => validateSidecarTarget(""), /invalid Pandora sidecar target triple/);
});

test("accepts a canonical downloaded sidecar but rejects final symlinks", () => {
  const root = mkdtempSync(join(tmpdir(), "pandora-desktop-source-"));
  const linkRoot = mkdtempSync(join(tmpdir(), "pandora-desktop-source-link-"));
  const source = join(root, "pandora-x86_64-unknown-linux-gnu");
  const link = join(linkRoot, "pandora-x86_64-unknown-linux-gnu");
  try {
    writeFileSync(source, "published sidecar");
    assert.equal(
      resolveSourceSidecar("x86_64-unknown-linux-gnu", true, {
        PANDORA_DESKTOP_SOURCE_SIDECAR: ` ${source} `,
      }),
      realpathSync(source),
    );
    symlinkSync(root, link, process.platform === "win32" ? "junction" : "dir");
    assert.throws(
      () => resolveSourceSidecar("x86_64-unknown-linux-gnu", true, {
        PANDORA_DESKTOP_SOURCE_SIDECAR: link,
      }),
      /must be a regular file and not a symlink/,
    );
    assert.throws(
      () => resolveSourceSidecar("x86_64-unknown-linux-gnu", true, {
        PANDORA_DESKTOP_SOURCE_SIDECAR: join(root, "missing"),
      }),
      /must be named pandora-x86_64-unknown-linux-gnu/,
    );
    assert.throws(
      () => resolveSourceSidecar("x86_64-unknown-linux-gnu", true, {
        PANDORA_DESKTOP_SOURCE_SIDECAR: join(root, "missing", "pandora-x86_64-unknown-linux-gnu"),
      }),
      /source sidecar is missing/,
    );
  } finally {
    rmSync(linkRoot, { recursive: true, force: true });
    rmSync(root, { recursive: true, force: true });
  }
});
