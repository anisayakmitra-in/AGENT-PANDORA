import assert from "node:assert/strict";
import { mkdtempSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  resolveConfiguredSource,
  validateSidecarTarget,
} from "./stage-sidecar.mjs";

test("accepts only bounded target triples", () => {
  assert.equal(validateSidecarTarget(" x86_64-pc-windows-msvc "), "x86_64-pc-windows-msvc");
  assert.throws(() => validateSidecarTarget("../release"), /invalid Pandora sidecar target triple/);
  assert.throws(() => validateSidecarTarget(""), /invalid Pandora sidecar target triple/);
});

test("accepts only an exact target-qualified regular source artifact", () => {
  const root = mkdtempSync(join(tmpdir(), "pandora-stage-source-"));
  const target = "x86_64-unknown-linux-gnu";
  const source = join(root, `pandora-${target}`);
  const wrongName = join(root, "pandora");
  try {
    writeFileSync(source, "verified native artifact");
    writeFileSync(wrongName, "wrong name");
    assert.equal(resolveConfiguredSource(source, target), source);
    assert.throws(
      () => resolveConfiguredSource(wrongName, target),
      /must be named pandora-x86_64-unknown-linux-gnu/,
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("rejects a final source junction instead of following it", () => {
  const root = mkdtempSync(join(tmpdir(), "pandora-stage-source-"));
  const linkRoot = mkdtempSync(join(tmpdir(), "pandora-stage-link-"));
  const target = "x86_64-unknown-linux-gnu";
  const link = join(linkRoot, `pandora-${target}`);
  try {
    symlinkSync(root, link, process.platform === "win32" ? "junction" : "dir");
    assert.throws(
      () => resolveConfiguredSource(link, target),
      /is not a regular file/,
    );
  } finally {
    rmSync(linkRoot, { recursive: true, force: true });
    rmSync(root, { recursive: true, force: true });
  }
});
