import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  preserveUpgradeDrillBundle,
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

test("preserves each versioned bundle outside a bundler-cleaned directory", () => {
  const root = mkdtempSync(join(tmpdir(), "pandora-upgrade-preserve-"));
  const source = join(root, "source");
  const preserved = join(root, "preserved");
  try {
    mkdirSync(source);
    writeFileSync(join(source, "Pandora_2.0.0_aarch64.dmg"), "predecessor");
    const destination = preserveUpgradeDrillBundle(source, preserved, "darwin", "2.0.0");
    assert.equal(readFileSync(destination, "utf8"), "predecessor");
    assert.throws(
      () => preserveUpgradeDrillBundle(source, preserved, "darwin", "2.0.0"),
      /EEXIST/,
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
