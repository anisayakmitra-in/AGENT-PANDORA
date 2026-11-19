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
export declare class PandoraCliProtocolError extends Error {
    constructor(message: string);
}
export declare function parseJsonEnvelope<T extends JsonObject = JsonObject>(stdout: string): PandoraEnvelope<T>;
export declare function defaultLauncherPath(): string;
export declare function runPandoraJson<T extends JsonObject = JsonObject>(args: readonly string[], options?: PandoraRunOptions): Promise<PandoraRunResult<T>>;
export declare class PandoraClient {
    private readonly options;
    constructor(options?: PandoraRunOptions);
    run<T extends JsonObject = JsonObject>(args: readonly string[], overrideOptions?: PandoraRunOptions): Promise<PandoraRunResult<T>>;
    doctor(overrideOptions?: PandoraRunOptions): Promise<PandoraRunResult>;
    verify(overrideOptions?: PandoraRunOptions): Promise<PandoraRunResult>;
    lspDefinition(file: string, line: number, col: number, overrideOptions?: PandoraRunOptions): Promise<PandoraRunResult>;
    lspReferences(file: string, line: number, col: number, overrideOptions?: PandoraRunOptions): Promise<PandoraRunResult>;
    lspDiagnostics(file: string, overrideOptions?: PandoraRunOptions): Promise<PandoraRunResult>;
    branchList(overrideOptions?: PandoraRunOptions): Promise<PandoraRunResult>;
    prCreate(taskId: string, title: string, overrideOptions?: PandoraRunOptions): Promise<PandoraRunResult>;
    semanticSearch(query: string, limit?: number, overrideOptions?: PandoraRunOptions): Promise<PandoraRunResult>;
    memoryVfsTropicalSearch(query: string, options?: TropicalSearchOptions): Promise<PandoraRunResult<{
        query: string;
        results: TropicalSearchResult[];
        count: number;
    }>>;
    runWithLats(task: string, options?: RunLatsOptions): Promise<PandoraRunResult<{
        strategy: string;
        lats_metrics?: LatsMetrics | null;
    }>>;
}
