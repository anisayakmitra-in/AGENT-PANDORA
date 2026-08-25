"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const {
  PandoraCliProtocolError,
  parseJsonEnvelope,
  runPandoraJson,
} = require("../npm/pandora-cli/lib/index.js");

const success = parseJsonEnvelope('{"version":"0.1","command":"doctor","healthy":true}');
assert.equal(success.command, "doctor");
assert.equal(success.healthy, true);

const failure = parseJsonEnvelope(
  '{"version":"0.1","code":"policy_denied","message":"blocked","details":{}}',
);
assert.equal(failure.code, "policy_denied");
assert.deepEqual(failure.details, {});

assert.throws(
  () => parseJsonEnvelope('{"version":"0.1","command":"doctor"}\nextra'),
  PandoraCliProtocolError,
);
assert.throws(
  () => parseJsonEnvelope('{"version":"0.1","code":"internal_error","message":"bad"}'),
  PandoraCliProtocolError,
);

async function main() {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "pandora-ts-client-"));
  const launcher = path.join(directory, "fixture.js");
  fs.writeFileSync(
    launcher,
    "if (process.argv.at(-1) !== '--json') process.exit(2); process.stdout.write(JSON.stringify({version: '0.1', command: 'fixture', forwarded: process.argv[2]}));\n",
  );
  try {
    const result = await runPandoraJson(["doctor"], { launcherPath: launcher });
    assert.equal(result.exitCode, 0);
    assert.equal(result.envelope.command, "fixture");
    assert.equal(result.envelope.forwarded, "doctor");
  } finally {
    fs.rmSync(directory, { force: true, recursive: true });
  }
  console.log("TypeScript client tests passed");
}

main().catch((error) => {
  console.error(error.stack || error.message);
  process.exit(1);
});
