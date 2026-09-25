"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.replaceFile = replaceFile;
const node_crypto_1 = __importDefault(require("node:crypto"));
const node_fs_1 = __importDefault(require("node:fs"));
function replaceFile(destination, contents, mode) {
    const temporary = `${destination}.${process.pid}.${node_crypto_1.default.randomUUID()}.new`;
    try {
        node_fs_1.default.writeFileSync(temporary, contents, { mode });
        try {
            node_fs_1.default.renameSync(temporary, destination);
        }
        catch (error) {
            const code = error.code;
            if (process.platform !== "win32" ||
                !["EEXIST", "ENOTEMPTY", "EPERM"].includes(code ?? "")) {
                throw error;
            }
            node_fs_1.default.rmSync(destination, { force: true });
            node_fs_1.default.renameSync(temporary, destination);
        }
    }
    finally {
        if (node_fs_1.default.existsSync(temporary))
            node_fs_1.default.rmSync(temporary, { force: true });
    }
}
