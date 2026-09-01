import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  resolveUpgradeSources,
  selectVersionedBundle,
  validateUpgradeManifest,
} from "./verify-bundle-upgrade-lifecycle.mjs";

test("requires an exact predecessor/current sidecar pair for published rollback", () => {
  const root = mkdtempSync(join(tmpdir(), "pandora-published-sidecars-"));
  const target = "x86_64-pc-windows-msvc";
  const name = `pandora-${target}.exe`;
  const predecessor = join(root, "predecessor", name);
  const current = join(root, "current", name);
  try {
    mkdirSync(join(root, "predecessor"));
    mkdirSync(join(root, "current"));
    writeFileSync(predecessor, "predecessor");
    writeFileSync(current, "current");
    assert.deepEqual(resolveUpgradeSources(target, {
      PANDORA_DESKTOP_PREDECESSOR_SIDECAR: predecessor,
      PANDORA_DESKTOP_CURRENT_SIDECAR: current,
    }), { predecessor, current });
    assert.throws(
      () => resolveUpgradeSources(target, {
        PANDORA_DESKTOP_PREDECESSOR_SIDECAR: predecessor,
      }),
      /requires both predecessor and current sidecars/,
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("accepts only a newer stable upgrade pair with an exact schema", () => {
  assert.deepEqual(validateUpgradeManifest({
    schema_version: 1,
    predecessor_version: "2.0.0",
    current_version: "2.0.1",
  }), {
    schema_version: 1,
    predecessor_version: "2.0.0",
    current_version: "2.0.1",
  });
  assert.throws(
    () => validateUpgradeManifest({ schema_version: 1, predecessor_version: "2.0.1", current_version: "2.0.0" }),
    /must be newer/,
  );
  assert.throws(
    () => validateUpgradeManifest({ schema_version: 1, predecessor_version: "2.0.0", current_version: "2.0.1", extra: true }),
    /unexpected shape/,
  );
  assert.throws(
    () => validateUpgradeManifest({ schema_version: 1, predecessor_version: "2.0.0-beta.1", current_version: "2.0.1" }),
    /stable semantic version/,
  );
});

test("selects one exact platform bundle for each synthetic version", () => {
  const root = mkdtempSync(join(tmpdir(), "pandora-upgrade-bundles-"));
  try {
    const predecessor = join(root, "Pandora_2.0.0_x64_en-US.msi");
    const current = join(root, "Pandora_2.0.1_x64_en-US.msi");
    writeFileSync(predecessor, "predecessor");
    writeFileSync(current, "current");
    writeFileSync(join(root, "Pandora_2.0.10_x64_en-US.msi"), "not current");
    assert.equal(selectVersionedBundle(root, "win32", "2.0.0"), predecessor);
    assert.equal(selectVersionedBundle(root, "win32", "2.0.1"), current);
    assert.throws(() => selectVersionedBundle(root, "linux", "2.0.1"), /found 0/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
