import { invoke } from "@tauri-apps/api/core";

export type RuntimeStatus = "preview" | "checking" | "connected" | "offline";

export type RuntimeHealth = {
  status: string;
};

export type RuntimeHarness = {
  id: string;
  version: string;
  name: string;
  kind: string;
  gene_count: number;
  runnable: boolean;
  gene_ids?: string[];
};

export type RuntimeProvider = {
  name: string;
  model: string;
  protocol: string;
  active: boolean;
  credential_configured: boolean;
  fallback_provider: string | null;
};

export type RuntimeEngine = {
  id: string;
  name: string;
  role: string;
  authority: string;
};

export type RuntimeTool = {
  id: string;
  version: string;
  name: string;
  capability: string;
  operation: string;
};

export type RuntimeSession = {
  session_id: string;
  principal_id: string;
  tenant_id: string;
  workspace_id: string;
  created_at_unix_seconds: number;
};

export type RuntimeSessionDetail = {
  session: RuntimeSession;
  event_count: number;
};

export type RuntimeRun = {
  session_id: string;
  execution_id: string;
  selected_harness: string | null;
  selected_gene: string | null;
  status: string;
  output: string;
  receipt_count: number;
  event_count: number;
};

export type RuntimeEvent = {
  event_id: string;
  event_type: string;
  payload: Record<string, unknown>;
};

type RpcResponse<T> = {
  result?: T;
  error?: {
    code: number;
    message: string;
    data?: { code?: string };
  };
};

type HealthResponse = {
  kind: "health";
  health: RuntimeHealth;
};

type CapabilitiesResponse = {
  kind: "capabilities";
  harnesses: RuntimeHarness[];
};

type ProvidersResponse = {
  kind: "providers";
  providers: RuntimeProvider[];
};

type EnginesResponse = {
  kind: "engines";
  engines: RuntimeEngine[];
};

type ToolsResponse = {
  kind: "tools";
  tools: RuntimeTool[];
};

type SessionListResponse = {
  kind: "session_list";
  sessions: RuntimeSession[];
};

type SessionInspectResponse = {
  kind: "session_inspect";
  session: RuntimeSessionDetail;
};

type RunResponse = {
  kind: "run";
  run: RuntimeRun;
};

type SessionEventsResponse = {
  kind: "session_events";
  events: {
    events: RuntimeEvent[];
    next_sequence: number | null;
  };
};

const endpointStorageKey = "pandora.runtime.endpoint";
export const nativeEndpoint = "tauri://pandora";

export function isNativeRuntime(): boolean {
  return "__TAURI_INTERNALS__" in window;
}

export async function startLocalService(): Promise<string> {
  if (!isNativeRuntime()) {
    throw new Error("Automatic service startup is available in the Pandora desktop app");
  }
  const status = await invoke<{ endpoint: string }>("start_local_service");
  return status.endpoint;
}

export async function stopLocalService(): Promise<void> {
  if (isNativeRuntime()) {
    await invoke("stop_local_service");
  }
}

export function loadRuntimeEndpoint(): string {
  try {
    return window.localStorage.getItem(endpointStorageKey) ?? "";
  } catch {
    return "";
  }
}

export function saveRuntimeEndpoint(endpoint: string): void {
  try {
    window.localStorage.setItem(endpointStorageKey, endpoint);
  } catch {
  }
}

export class RuntimeClient {
  private readonly endpoint: string;
  private readonly token: string;
  private readonly native: boolean;

  constructor(endpoint: string, token: string) {
    this.native = isNativeRuntime() && endpoint === nativeEndpoint;
    if (!this.native) {
      const url = new URL(endpoint);
      if (!isLoopbackHost(url.hostname)) {
        throw new Error("Pandora desktop accepts only a loopback service endpoint");
      }
    }
    this.endpoint = this.native ? "" : new URL(endpoint).toString();
    this.token = token;
  }

  async health(): Promise<RuntimeHealth> {
    const response = await this.call<HealthResponse>("runtime.health", null);
    return response.health;
  }

  async capabilities(): Promise<RuntimeHarness[]> {
    const response = await this.call<CapabilitiesResponse>("runtime.capabilities", null);
    return response.harnesses;
  }

  async providers(): Promise<RuntimeProvider[]> {
    try {
      const response = await this.call<ProvidersResponse>("runtime.providers", null);
      return response.providers;
    } catch (error: unknown) {
      if (error instanceof Error && error.message === "method_not_found") {
        return [];
      }
      throw error;
    }
  }

  async engines(): Promise<RuntimeEngine[]> {
    try {
      const response = await this.call<EnginesResponse>("runtime.engines", null);
      return response.engines;
    } catch (error: unknown) {
      if (error instanceof Error && error.message === "method_not_found") {
        return [];
      }
      throw error;
    }
  }

  async tools(): Promise<RuntimeTool[]> {
    try {
      const response = await this.call<ToolsResponse>("runtime.tools", null);
      return response.tools;
    } catch (error: unknown) {
      if (error instanceof Error && error.message === "method_not_found") {
        return [];
      }
      throw error;
    }
  }

  async sessions(limit = 8): Promise<RuntimeSession[]> {
    const response = await this.call<SessionListResponse>("session.list", { limit });
    return response.sessions;
  }

  async inspectSession(sessionId: string): Promise<RuntimeSessionDetail> {
    const response = await this.call<SessionInspectResponse>("session.inspect", { session_id: sessionId });
    return response.session;
  }

  async run(task: string, requestedHarness: string | null = null): Promise<RuntimeRun> {
    const response = await this.call<RunResponse>("run.execute", {
      task,
      requested_harness: requestedHarness,
      requested_gene: null,
    });
    return response.run;
  }

  async events(sessionId: string, limit = 256): Promise<RuntimeEvent[]> {
    const response = await this.call<SessionEventsResponse>("session.events", {
      session_id: sessionId,
      after_sequence: null,
      limit,
    });
    return response.events.events;
  }

  private async call<T>(method: string, params: unknown): Promise<T> {
    const payload = this.native
      ? await invoke<RpcResponse<T>>("pandora_rpc", { method, params })
      : await this.fetchRpc<T>(method, params);
    if (payload.error) {
      throw new Error(payload.error.data?.code ?? payload.error.message);
    }
    if (!payload.result) {
      throw new Error("Pandora service returned an empty response");
    }
    return payload.result;
  }

  private async fetchRpc<T>(method: string, params: unknown): Promise<RpcResponse<T>> {
    const response = await fetch(this.endpoint, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${this.token}`,
      },
      body: JSON.stringify({ jsonrpc: "2.0", id: crypto.randomUUID(), method, params }),
    });
    if (!response.ok) {
      throw new Error(`Pandora service returned HTTP ${response.status}`);
    }
    return (await response.json()) as RpcResponse<T>;
  }
}

function isLoopbackHost(hostname: string): boolean {
  return hostname === "localhost" || hostname === "127.0.0.1" || hostname === "[::1]" || hostname === "::1";
}
