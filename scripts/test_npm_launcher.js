"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const { replaceFile } = require("../npm/pandora-cli/lib/launcher-files.js");

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
} finally {
  fs.rmSync(directory, { force: true, recursive: true });
}
