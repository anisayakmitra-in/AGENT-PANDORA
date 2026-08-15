"use strict";

const crypto = require("node:crypto");
const fs = require("node:fs");

function replaceFile(destination, contents, mode) {
  const temporary = `${destination}.${process.pid}.${crypto.randomUUID()}.new`;
  try {
    fs.writeFileSync(temporary, contents, { mode });
    try {
      fs.renameSync(temporary, destination);
    } catch (error) {
      if (process.platform !== "win32" ||
          !["EEXIST", "ENOTEMPTY", "EPERM"].includes(error.code)) {
        throw error;
      }
      fs.rmSync(destination, { force: true });
      fs.renameSync(temporary, destination);
    }
  } finally {
    if (fs.existsSync(temporary)) fs.rmSync(temporary, { force: true });
  }
}

module.exports = { replaceFile };
