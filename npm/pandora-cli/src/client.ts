import { spawn } from "node:child_process";
import path from "node:path";

export type JsonObject = Record<string, unknown>;

export type PandoraSuccess<T extends JsonObject = JsonObject> = T & {
  version: string;
  command: string;
};

export interface PandoraError {
  version: string;
  code: string;
  message: string;
  details: JsonObject;
}

export type PandoraEnvelope<T extends JsonObject = JsonObject> =
  | PandoraSuccess<T>
  | PandoraError;

export interface LatsTurnTelemetry {
  turn: number;
  tool_name: string;
  escalated: boolean;
  rollouts: number;
  max_depth_reached: number;
  nodes_created: number;
  best_action: number | null;
  best_value: number;
}

export interface LatsMetrics {
  rollouts: number;
  max_depth_reached: number;
  nodes_created: number;
  best_action: number | null;
  best_value: number;
  escalated: boolean;
  turn_rollouts?: number;
  turn_escalations?: number;
  turns?: LatsTurnTelemetry[];
}

export interface RunLatsOptions extends PandoraRunOptions {
  rollouts?: number;
  depth?: number;
  puctC?: number;
  escalationThreshold?: number;
}

export interface TropicalSearchOptions extends PandoraRunOptions {
  limit?: number;
  tier?: string;
  intent?: string;
}

export interface TropicalSearchResult {
  uri: string;
  title: string;
  snippet: string;
  sparse_score: number;
  dense_score: number;
  hybrid_score: number;
  tier: string | null;
  intent: string | null;
}

export interface PandoraRunOptions {
  cwd?: string;
  env?: NodeJS.ProcessEnv;
  launcherPath?: string;
  timeoutMs?: number;
}

export interface PandoraRunResult<T extends JsonObject = JsonObject> {
  exitCode: number;
  envelope: PandoraEnvelope<T>;
  stderr: string;
}

export class PandoraCliProtocolError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "PandoraCliProtocolError";
  }
}

const DEFAULT_TIMEOUT_MS = 120_000;
const MAX_JSON_OUTPUT_BYTES = 4 * 1024 * 1024;

function isObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function requiredString(value: JsonObject, field: string): string {
  const candidate = value[field];
  if (typeof candidate !== "string" || candidate.length === 0) {
    throw new PandoraCliProtocolError(`Pandora JSON response is missing '${field}'`);
  }
  return candidate;
}

export function parseJsonEnvelope<T extends JsonObject = JsonObject>(
  stdout: string,
): PandoraEnvelope<T> {
  let parsed: unknown;
  try {
    parsed = JSON.parse(stdout);
  } catch {
    throw new PandoraCliProtocolError("Pandora returned invalid JSON");
  }
  if (!isObject(parsed)) {
    throw new PandoraCliProtocolError("Pandora JSON response must be an object");
  }
  requiredString(parsed, "version");
  if (typeof parsed.command === "string" && parsed.command.length > 0) {
    return parsed as PandoraSuccess<T>;
  }
  if (
    typeof parsed.code === "string" &&
    parsed.code.length > 0 &&
    typeof parsed.message === "string" &&
    parsed.message.length > 0 &&
    isObject(parsed.details)
  ) {
    return parsed as unknown as PandoraError;
  }
  throw new PandoraCliProtocolError(
    "Pandora JSON response is neither a success nor an error envelope",
  );
}

export function defaultLauncherPath(): string {
  return path.resolve(__dirname, "..", "bin", "pandora.js");
}

export function runPandoraJson<T extends JsonObject = JsonObject>(
  args: readonly string[],
  options: PandoraRunOptions = {},
): Promise<PandoraRunResult<T>> {
  if (args.some((argument) => argument === "--json")) {
    throw new PandoraCliProtocolError("runPandoraJson adds '--json' automatically");
  }
  const launcher = options.launcherPath ?? defaultLauncherPath();
  const child = spawn(process.execPath, [launcher, ...args, "--json"], {
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
      if (settled) return;
      settled = true;
      child.kill();
      reject(new PandoraCliProtocolError("Pandora CLI timed out"));
    }, timeoutMs);

    const append = (target: "stdout" | "stderr", chunk: Buffer) => {
      outputBytes += chunk.byteLength;
      if (outputBytes > MAX_JSON_OUTPUT_BYTES) {
        child.kill();
        throw new PandoraCliProtocolError("Pandora CLI output exceeds the client limit");
      }
      if (target === "stdout") stdout += chunk.toString("utf8");
      else stderr += chunk.toString("utf8");
    };

    child.stdout.on("data", (chunk: Buffer) => {
      try {
        append("stdout", chunk);
      } catch (error) {
        if (!settled) {
          settled = true;
          clearTimeout(timeout);
          reject(error);
        }
      }
    });
    child.stderr.on("data", (chunk: Buffer) => {
      try {
        append("stderr", chunk);
      } catch (error) {
        if (!settled) {
          settled = true;
          clearTimeout(timeout);
          reject(error);
        }
      }
    });
    child.on("error", (error) => {
      if (settled) return;
      settled = true;
      clearTimeout(timeout);
      reject(error);
    });
    child.on("close", (code) => {
      if (settled) return;
      settled = true;
      clearTimeout(timeout);
      try {
        resolve({
          exitCode: code ?? 1,
          envelope: parseJsonEnvelope<T>(stdout.trim()),
          stderr,
        });
      } catch (error) {
        reject(error);
      }
    });
  });
}

export class PandoraClient {
  constructor(private readonly options: PandoraRunOptions = {}) {}

  public async run<T extends JsonObject = JsonObject>(
    args: readonly string[],
    overrideOptions: PandoraRunOptions = {},
  ): Promise<PandoraRunResult<T>> {
    return runPandoraJson<T>(args, { ...this.options, ...overrideOptions });
  }

  public async doctor(overrideOptions?: PandoraRunOptions): Promise<PandoraRunResult> {
    return this.run(["doctor"], overrideOptions);
  }

  public async verify(overrideOptions?: PandoraRunOptions): Promise<PandoraRunResult> {
    return this.run(["verify"], overrideOptions);
  }

  public async lspDefinition(
    file: string,
    line: number,
    col: number,
    overrideOptions?: PandoraRunOptions,
  ): Promise<PandoraRunResult> {
    return this.run(
      ["lsp", "definition", "--file", file, "--line", String(line), "--col", String(col)],
      overrideOptions,
    );
  }

  public async lspReferences(
    file: string,
    line: number,
    col: number,
    overrideOptions?: PandoraRunOptions,
  ): Promise<PandoraRunResult> {
    return this.run(
      ["lsp", "references", "--file", file, "--line", String(line), "--col", String(col)],
      overrideOptions,
    );
  }

  public async lspDiagnostics(
    file: string,
    overrideOptions?: PandoraRunOptions,
  ): Promise<PandoraRunResult> {
    return this.run(["lsp", "diagnostics", "--file", file], overrideOptions);
  }

  public async branchList(overrideOptions?: PandoraRunOptions): Promise<PandoraRunResult> {
    return this.run(["branch", "list"], overrideOptions);
  }

  public async prCreate(
    taskId: string,
    title: string,
    overrideOptions?: PandoraRunOptions,
  ): Promise<PandoraRunResult> {
    return this.run(["pr", "create", "--task", taskId, "--title", title], overrideOptions);
  }

  public async semanticSearch(
    query: string,
    limit: number = 5,
    overrideOptions?: PandoraRunOptions,
  ): Promise<PandoraRunResult> {
    return this.run(["memory", "vfs", "search", query, "--limit", String(limit)], overrideOptions);
  }

  public async memoryVfsTropicalSearch(
    query: string,
    options: TropicalSearchOptions = {},
  ): Promise<PandoraRunResult<{ query: string; results: TropicalSearchResult[]; count: number }>> {
    const { limit = 5, tier, intent, ...runOptions } = options;
    const args = ["memory", "vfs", "tropical-search", query, "--limit", String(limit)];
    if (tier !== undefined) {
      args.push("--tier", tier);
    }
    if (intent !== undefined) {
      args.push("--intent", intent);
    }
    return this.run(args, runOptions);
  }

  public async runWithLats(
    task: string,
    options: RunLatsOptions = {},
  ): Promise<PandoraRunResult<{ strategy: string; lats_metrics?: LatsMetrics | null }>> {
    const { rollouts, depth, puctC, escalationThreshold, ...runOptions } = options;
    const args = ["run", "--strategy", "lats"];
    if (rollouts !== undefined) {
      args.push("--lats-rollouts", String(rollouts));
    }
    if (depth !== undefined) {
      args.push("--lats-depth", String(depth));
    }
    if (puctC !== undefined) {
      args.push("--puct-c", String(puctC));
    }
    if (escalationThreshold !== undefined) {
      args.push("--escalation-threshold", String(escalationThreshold));
    }
    args.push(task);
    return this.run(args, runOptions);
  }
}
