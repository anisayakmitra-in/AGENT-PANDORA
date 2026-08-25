"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.PandoraCliProtocolError = void 0;
exports.parseJsonEnvelope = parseJsonEnvelope;
exports.defaultLauncherPath = defaultLauncherPath;
exports.runPandoraJson = runPandoraJson;
const node_child_process_1 = require("node:child_process");
const node_path_1 = __importDefault(require("node:path"));
class PandoraCliProtocolError extends Error {
    constructor(message) {
        super(message);
        this.name = "PandoraCliProtocolError";
    }
}
exports.PandoraCliProtocolError = PandoraCliProtocolError;
const DEFAULT_TIMEOUT_MS = 120_000;
const MAX_JSON_OUTPUT_BYTES = 4 * 1024 * 1024;
function isObject(value) {
    return typeof value === "object" && value !== null && !Array.isArray(value);
}
function requiredString(value, field) {
    const candidate = value[field];
    if (typeof candidate !== "string" || candidate.length === 0) {
        throw new PandoraCliProtocolError(`Pandora JSON response is missing '${field}'`);
    }
    return candidate;
}
function parseJsonEnvelope(stdout) {
    let parsed;
    try {
        parsed = JSON.parse(stdout);
    }
    catch {
        throw new PandoraCliProtocolError("Pandora returned invalid JSON");
    }
    if (!isObject(parsed)) {
        throw new PandoraCliProtocolError("Pandora JSON response must be an object");
    }
    requiredString(parsed, "version");
    if (typeof parsed.command === "string" && parsed.command.length > 0) {
        return parsed;
    }
    if (typeof parsed.code === "string" &&
        parsed.code.length > 0 &&
        typeof parsed.message === "string" &&
        parsed.message.length > 0 &&
        isObject(parsed.details)) {
        return parsed;
    }
    throw new PandoraCliProtocolError("Pandora JSON response is neither a success nor an error envelope");
}
function defaultLauncherPath() {
    return node_path_1.default.resolve(__dirname, "..", "bin", "pandora.js");
}
function runPandoraJson(args, options = {}) {
    if (args.some((argument) => argument === "--json")) {
        throw new PandoraCliProtocolError("runPandoraJson adds '--json' automatically");
    }
    const launcher = options.launcherPath ?? defaultLauncherPath();
    const child = (0, node_child_process_1.spawn)(process.execPath, [launcher, ...args, "--json"], {
        cwd: options.cwd,
        env: { ...process.env, ...options.env },
        stdio: ["ignore", "pipe", "pipe"],
        windowsHide: true,
    });
    const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
    return new Promise((resolve, reject) => {
        let stdout = "";
        let stderr = "";
        let outputBytes = 0;
        let settled = false;
        const timeout = setTimeout(() => {
            if (settled)
                return;
            settled = true;
            child.kill();
            reject(new PandoraCliProtocolError("Pandora CLI timed out"));
        }, timeoutMs);
        const append = (target, chunk) => {
            outputBytes += chunk.byteLength;
            if (outputBytes > MAX_JSON_OUTPUT_BYTES) {
                child.kill();
                throw new PandoraCliProtocolError("Pandora CLI output exceeds the client limit");
            }
            if (target === "stdout")
                stdout += chunk.toString("utf8");
            else
                stderr += chunk.toString("utf8");
        };
        child.stdout.on("data", (chunk) => {
            try {
                append("stdout", chunk);
            }
            catch (error) {
                if (!settled) {
                    settled = true;
                    clearTimeout(timeout);
                    reject(error);
                }
            }
        });
        child.stderr.on("data", (chunk) => {
            try {
                append("stderr", chunk);
            }
            catch (error) {
                if (!settled) {
                    settled = true;
                    clearTimeout(timeout);
                    reject(error);
                }
            }
        });
        child.on("error", (error) => {
            if (settled)
                return;
            settled = true;
            clearTimeout(timeout);
            reject(error);
        });
        child.on("close", (code) => {
            if (settled)
                return;
            settled = true;
            clearTimeout(timeout);
            try {
                resolve({
                    exitCode: code ?? 1,
                    envelope: parseJsonEnvelope(stdout.trim()),
                    stderr,
                });
            }
            catch (error) {
                reject(error);
            }
        });
    });
}
