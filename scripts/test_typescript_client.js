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
    "if (process.argv.at(-1) !== '--json') process.exit(2); process.stdout.write(JSON.stringify({version: '0.1', command: 'fixture', forwarded: process.argv[2], allArgs: process.argv.slice(2)}));\n",
  );
  try {
    const result = await runPandoraJson(["doctor"], { launcherPath: launcher });
    assert.equal(result.exitCode, 0);
    assert.equal(result.envelope.command, "fixture");
    assert.equal(result.envelope.forwarded, "doctor");

    // Test PandoraClient class methods
    const client = new PandoraClient({ launcherPath: launcher });

    const docRes = await client.doctor();
    assert.equal(docRes.envelope.forwarded, "doctor");

    const verRes = await client.verify();
    assert.equal(verRes.envelope.forwarded, "verify");

    const lspDef = await client.lspDefinition("src/lib.rs", 15, 8);
    assert.equal(lspDef.envelope.forwarded, "lsp");
    assert.equal(lspDef.envelope.allArgs[1], "definition");

    const lspRef = await client.lspReferences("src/lib.rs", 15, 8);
    assert.equal(lspRef.envelope.forwarded, "lsp");
    assert.equal(lspRef.envelope.allArgs[1], "references");

    const lspDiag = await client.lspDiagnostics("src/lib.rs");
    assert.equal(lspDiag.envelope.forwarded, "lsp");
    assert.equal(lspDiag.envelope.allArgs[1], "diagnostics");

    const branchRes = await client.branchList();
    assert.equal(branchRes.envelope.forwarded, "branch");

    const prRes = await client.prCreate("task-20261031-refactor", "Autonomous Refactor");
    assert.equal(prRes.envelope.forwarded, "pr");

    const searchRes = await client.semanticSearch("compiler diagnostic", 3);
    assert.equal(searchRes.envelope.forwarded, "memory");
    assert.equal(searchRes.envelope.allArgs[1], "vfs");
    assert.equal(searchRes.envelope.allArgs[2], "search");

    const tropSearchRes = await client.memoryVfsTropicalSearch("max-plus semiring", {
      limit: 4,
      tier: "L1",
      intent: "coding",
    });
    assert.equal(tropSearchRes.envelope.forwarded, "memory");
    assert.deepEqual(tropSearchRes.envelope.allArgs, [
      "memory",
      "vfs",
      "tropical-search",
      "max-plus semiring",
      "--limit",
      "4",
      "--tier",
      "L1",
      "--intent",
      "coding",
      "--json",
    ]);

    const latsRes = await client.runWithLats("Verify LATS branching", {
      rollouts: 12,
      depth: 4,
      puctC: 1.45,
      escalationThreshold: 0.2,
    });
    assert.equal(latsRes.envelope.forwarded, "run");
    assert.deepEqual(latsRes.envelope.allArgs, [
      "run",
      "--strategy",
      "lats",
      "--lats-rollouts",
      "12",
      "--lats-depth",
      "4",
      "--puct-c",
      "1.45",
      "--escalation-threshold",
      "0.2",
      "Verify LATS branching",
      "--json",
    ]);
  } finally {
    fs.rmSync(directory, { force: true, recursive: true });
  }
  console.log("TypeScript client tests passed");
}

main().catch((error) => {
  console.error(error.stack || error.message);
  process.exit(1);
});
