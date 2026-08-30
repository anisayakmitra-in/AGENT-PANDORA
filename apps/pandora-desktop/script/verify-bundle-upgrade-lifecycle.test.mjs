import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  selectVersionedBundle,
  validateUpgradeManifest,
} from "./verify-bundle-upgrade-lifecycle.mjs";

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
