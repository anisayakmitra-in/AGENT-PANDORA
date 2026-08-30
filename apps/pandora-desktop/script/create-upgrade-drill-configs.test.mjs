import assert from "node:assert/strict";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  upgradeDrillConfiguration,
  writeUpgradeDrillConfiguration,
} from "./create-upgrade-drill-configs.mjs";

test("derives two bounded stable installer identities from the desktop version", () => {
  const linux = upgradeDrillConfiguration("2.0.0-beta.7", "linux");
  assert.deepEqual(linux.manifest, {
    schema_version: 1,
    predecessor_version: "2.0.0",
    current_version: "2.0.1",
  });
  assert.deepEqual(linux.predecessor.bundle.targets, ["deb"]);
  assert.deepEqual(linux.current.bundle.targets, ["deb"]);

  const windows = upgradeDrillConfiguration("2.0.0-beta.7", "win32");
  assert.equal(windows.predecessor.bundle.windows.wix.version, "2.0.0");
  assert.equal(windows.current.bundle.windows.wix.version, "2.0.1");
  assert.deepEqual(upgradeDrillConfiguration("2.0.0-beta.7", "darwin").current.bundle.targets, ["dmg"]);
});

test("rejects unsupported versions, platforms, and patch overflow", () => {
  assert.throws(() => upgradeDrillConfiguration("latest", "linux"), /unsupported desktop version/);
  assert.throws(() => upgradeDrillConfiguration("2.0.0", "plan9"), /unsupported upgrade drill platform/);
  assert.throws(() => upgradeDrillConfiguration("2.0.65535", "linux"), /too large/);
});

test("writes each upgrade drill file once", () => {
  const root = mkdtempSync(join(tmpdir(), "pandora-upgrade-config-"));
  const output = join(root, "output");
  try {
    writeUpgradeDrillConfiguration(output, "2.0.0-beta.7", "linux");
    for (const name of ["manifest.json", "predecessor.json", "current.json"]) {
      assert.equal(existsSync(join(output, name)), true);
      assert.doesNotThrow(() => JSON.parse(readFileSync(join(output, name), "utf8")));
    }
    assert.throws(
      () => writeUpgradeDrillConfiguration(output, "2.0.0-beta.7", "linux"),
      /EEXIST/,
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
