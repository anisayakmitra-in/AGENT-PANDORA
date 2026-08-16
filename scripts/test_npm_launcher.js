"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const { replaceFile } = require("../npm/pandora-cli/lib/launcher-files.js");
const {
  MAX_RELEASE_DOWNLOAD_BYTES,
  readResponseBytes,
} = require("../npm/pandora-cli/bin/pandora.js");

function streamedResponse(chunks, contentLength = null) {
  let chunkIndex = 0;
  let reads = 0;
  let cancelled = false;
  return {
    response: {
      headers: {
        get(name) {
          return name.toLowerCase() === "content-length" ? contentLength : null;
        },
      },
      body: {
        getReader() {
          return {
            async read() {
              reads += 1;
              if (chunkIndex === chunks.length) return { done: true };
              const value = chunks[chunkIndex];
              chunkIndex += 1;
              return { done: false, value };
            },
            async cancel() {
              cancelled = true;
            },
            releaseLock() {},
          };
        },
      },
    },
    wasCancelled: () => cancelled,
    readCount: () => reads,
  };
}

async function main() {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "pandora-launcher-"));
  try {
    const destination = path.join(directory, "pandora");
    fs.writeFileSync(destination, "stale");

    replaceFile(destination, Buffer.from("fresh"), 0o755);

    assert.equal(fs.readFileSync(destination, "utf8"), "fresh");
    assert.deepEqual(
      fs.readdirSync(directory).filter((name) => name.endsWith(".new")),
      [],
    );

    assert.equal(MAX_RELEASE_DOWNLOAD_BYTES, 64 * 1024 * 1024);
    const exactLimit = streamedResponse([Buffer.from("1234"), Buffer.from("5678")]);
    assert.deepEqual(await readResponseBytes(exactLimit.response, 8), Buffer.from("12345678"));

    const oversized = streamedResponse([
      Buffer.from("12345678"),
      Buffer.from("9"),
      Buffer.from("must not be read"),
    ]);
    await assert.rejects(readResponseBytes(oversized.response, 8), /exceeds 8 bytes/);
    assert.equal(oversized.wasCancelled(), true);
    assert.equal(oversized.readCount(), 2);

    const declaredOversized = {
      headers: { get: () => "9" },
      body: { getReader: () => { throw new Error("body should not be read"); } },
    };
    await assert.rejects(readResponseBytes(declaredOversized, 8), /exceeds 8 bytes/);
  } finally {
    fs.rmSync(directory, { force: true, recursive: true });
  }
}

main().catch((error) => {
  console.error(error.stack || error.message);
  process.exit(1);
});
