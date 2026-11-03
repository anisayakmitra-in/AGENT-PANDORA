"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const {
  PandoraCliProtocolError,
  PandoraClient,
  parseJsonEnvelope,
  runPandoraJson,
} = require("../npm/pandora-cli/lib/index.js");

console.log("[Adversarial TS Client] Testing parseJsonEnvelope edge cases...");

// 1. Empty & whitespace inputs
assert.throws(() => parseJsonEnvelope(""), PandoraCliProtocolError);
assert.throws(() => parseJsonEnvelope("   \n\t  "), PandoraCliProtocolError);

// 2. Non-object JSON
assert.throws(() => parseJsonEnvelope("[]"), PandoraCliProtocolError);
assert.throws(() => parseJsonEnvelope("[1, 2, 3]"), PandoraCliProtocolError);
assert.throws(() => parseJsonEnvelope('"just a string"'), PandoraCliProtocolError);
assert.throws(() => parseJsonEnvelope("12345"), PandoraCliProtocolError);
assert.throws(() => parseJsonEnvelope("null"), PandoraCliProtocolError);
assert.throws(() => parseJsonEnvelope("true"), PandoraCliProtocolError);

// 3. Missing required fields
assert.throws(() => parseJsonEnvelope("{}"), PandoraCliProtocolError);
assert.throws(() => parseJsonEnvelope('{"command":"doctor"}'), PandoraCliProtocolError);
assert.throws(() => parseJsonEnvelope('{"version":""}'), PandoraCliProtocolError);
assert.throws(() => parseJsonEnvelope('{"version":"3.0.3"}'), PandoraCliProtocolError);
assert.throws(
  () => parseJsonEnvelope('{"version":"3.0.3","code":"err","message":"msg"}'),
  PandoraCliProtocolError,
);

// 4. Valid error envelope
const errEnv = parseJsonEnvelope(
  '{"version":"3.0.3","code":"test_err","message":"failed as planned","details":{"reason":"adversarial"}}',
);
assert.equal(errEnv.version, "3.0.3");
assert.equal(errEnv.code, "test_err");
assert.equal(errEnv.details.reason, "adversarial");

// 5. Valid success envelope
const okEnv = parseJsonEnvelope(
  '{"version":"3.0.3","command":"test","result":"passed","nested":{"ok":true}}',
);
assert.equal(okEnv.version, "3.0.3");
assert.equal(okEnv.command, "test");
assert.equal(okEnv.nested.ok, true);

console.log("  -> parseJsonEnvelope edge cases verified clean");

async function testLauncherAdversarial() {
  console.log("[Adversarial TS Client] Testing launcher error handling...");
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "pandora-ts-adv-"));
  const launcher = path.join(tempDir, "err_launcher.js");

  // A launcher that exits with code 1 and outputs an error envelope
  fs.writeFileSync(
    launcher,
    [
      "process.stderr.write('Warning from stderr\\n');",
      "process.stdout.write(JSON.stringify({",
      "  version: '3.0.3',",
      "  code: 'subcommand_failed',",
      "  message: 'command returned exit 1',",
      "  details: { code: 1 }",
      "}));",
      "process.exit(1);",
    ].join("\n"),
  );

  try {
    // Calling runPandoraJson with --json should reject immediately
    await assert.rejects(
      async () => runPandoraJson(["--json"], { launcherPath: launcher }),
      PandoraCliProtocolError,
    );

    // Calling launcher returning exit 1
    const res = await runPandoraJson(["failing-subcommand"], { launcherPath: launcher });
    assert.equal(res.exitCode, 1);
    assert.equal(res.envelope.code, "subcommand_failed");
    assert.equal(res.envelope.details.code, 1);
    assert.ok(res.stderr.includes("Warning from stderr"));

    console.log("  -> Launcher error and exit code handling verified clean");
  } finally {
    fs.rmSync(tempDir, { force: true, recursive: true });
  }
}

testLauncherAdversarial()
  .then(() => {
    console.log("ALL ADVERSARIAL TYPESCRIPT CLIENT TESTS PASSED CLEANLY.");
  })
  .catch((err) => {
    console.error(err);
    process.exit(1);
  });
