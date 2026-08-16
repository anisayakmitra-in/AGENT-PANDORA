"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const { replaceFile } = require("../npm/pandora-cli/lib/launcher-files.js");
const {
  MAX_RELEASE_DOWNLOAD_BYTES,
  fetchBytes,
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

function downloadResponse(status, headers = {}, chunks = []) {
  return {
    status,
    ok: status >= 200 && status < 300,
    headers: {
      get(name) {
        return headers[name.toLowerCase()] || null;
      },
    },
    body: streamedResponse(chunks).response.body,
  };
}

async function withFetchResponses(responses, callback) {
  const originalFetch = global.fetch;
  const requests = [];
  global.fetch = async (url, options) => {
    requests.push({ options, url: String(url) });
    const response = responses.shift();
    if (!response) throw new Error("unexpected fetch request");
    return response;
  };
  try {
    await callback(requests);
  } finally {
    global.fetch = originalFetch;
  }
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

    await withFetchResponses(
      [downloadResponse(302, { location: "https://evil.example/release" })],
      async (requests) => {
        await assert.rejects(
          fetchBytes("https://github.com/release", new Set(["github.com"])),
          /untrusted host/,
        );
        assert.equal(requests.length, 1);
        assert.equal(requests[0].options.redirect, "manual");
      },
    );

    await withFetchResponses(
      [
        downloadResponse(302, { location: "https://objects.githubusercontent.com/release" }),
        downloadResponse(200, {}, [Buffer.from("verified")]),
      ],
      async (requests) => {
        assert.deepEqual(
          await fetchBytes(
            "https://github.com/release",
            new Set(["github.com", "objects.githubusercontent.com"]),
          ),
          Buffer.from("verified"),
        );
        assert.deepEqual(
          requests.map(({ options, url }) => [url, options.redirect]),
          [
            ["https://github.com/release", "manual"],
            ["https://objects.githubusercontent.com/release", "manual"],
          ],
        );
      },
    );
  } finally {
    fs.rmSync(directory, { force: true, recursive: true });
  }
}

main().catch((error) => {
  console.error(error.stack || error.message);
  process.exit(1);
});
