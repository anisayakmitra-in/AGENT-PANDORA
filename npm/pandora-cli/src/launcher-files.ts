import crypto from "node:crypto";
import fs from "node:fs";

export function replaceFile(
  destination: string,
  contents: Buffer,
  mode: number,
): void {
  const temporary = `${destination}.${process.pid}.${crypto.randomUUID()}.new`;
  try {
    fs.writeFileSync(temporary, contents, { mode });
    try {
      fs.renameSync(temporary, destination);
    } catch (error) {
      const code = (error as NodeJS.ErrnoException).code;
      if (
        process.platform !== "win32" ||
        !["EEXIST", "ENOTEMPTY", "EPERM"].includes(code ?? "")
      ) {
        throw error;
      }
      fs.rmSync(destination, { force: true });
      fs.renameSync(temporary, destination);
    }
  } finally {
    if (fs.existsSync(temporary)) fs.rmSync(temporary, { force: true });
  }
}
