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
export type PandoraEnvelope<T extends JsonObject = JsonObject> = PandoraSuccess<T> | PandoraError;
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
export declare class PandoraCliProtocolError extends Error {
    constructor(message: string);
}
export declare function parseJsonEnvelope<T extends JsonObject = JsonObject>(stdout: string): PandoraEnvelope<T>;
export declare function defaultLauncherPath(): string;
export declare function runPandoraJson<T extends JsonObject = JsonObject>(args: readonly string[], options?: PandoraRunOptions): Promise<PandoraRunResult<T>>;
