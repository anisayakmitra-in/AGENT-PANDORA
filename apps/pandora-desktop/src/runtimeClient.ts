import { invoke } from "@tauri-apps/api/core";

export type RuntimeStatus = "preview" | "checking" | "connected" | "offline";

export type RuntimeHealth = {
  status: string;
};

export type RuntimeContextAttachment = {
  name: string;
  media_type: string;
  content: string;
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

export type ProviderConfiguration = {
  name: string;
  protocol: "open_ai_compatible" | "anthropic_messages" | "gemini_generate_content";
  baseUrl: string;
  model: string;
  apiKeyEnvironment: string;
  apiKey: string;
};

export type McpConfiguration = {
  serverId: string;
  program: string;
  argumentsJson: string;
  mode: "auto" | "modern-only" | "legacy-only";
};

export type RegistryProfile = {
  name: string;
  base_url: string;
  token_env: string | null;
  active: boolean;
};

export type RegistryConfiguration = {
  name: string;
  baseUrl: string;
  tokenEnvironment: string;
  token: string;
};

export type NativeConfigurationResult = {
  message: string;
  restartRequired: boolean;
};

export type RuntimePackage = {
  id: string;
  version: string;
  kind: "gene" | "domain_harness" | "meta_harness" | "source_harness" | "package" | "provider" | "skill";
  publisher: string;
  content_hash: string;
  dependencies: Array<{
    id: string;
    version: string;
    optional: boolean;
  }>;
  compatibility: string;
  license: string;
  trust: {
    level: "unverified" | "verified" | "official";
    has_signature: boolean;
    has_public_key: boolean;
  };
  meta_composition: {
    allowed_domains: string[];
    max_handoffs: number;
  } | null;
  state: "installed" | "admitted";
  runtime_authority: boolean;
  activation: {
    state: "enabled" | "disabled";
    active_version: string | null;
    previous_version: string | null;
    generation: number;
    runtime_authority: boolean;
  };
};

export type NativePackageResult = {
  message: string;
  restartRequired: boolean;
  data: {
    packages?: RuntimePackage[];
    package?: RuntimePackage;
    path?: string;
    package_count?: number;
    format_version?: number;
    dry_run?: boolean;
    removed?: boolean;
    changed?: boolean;
    ready?: boolean;
    target_version?: string;
    active_version?: string;
    enabled_dependents?: string[];
    dependencies?: Array<{
      id: string;
      version: string | null;
      optional: boolean;
      source: "built_in" | "package" | "unresolved";
      enabled: boolean;
    }>;
    binding?: RuntimePackage["activation"];
  };
};

export type RegistryPackageInstall = {
  packageId: string;
  version: string;
  registryProfile: string;
  registryUrl: string;
  token: string;
};

export type NativeRegistryResult = {
  message: string;
  data: {
    registries?: RegistryProfile[];
  };
};

export type GitHubPackageInstall = {
  repositoryUrl: string;
  commit: string;
  manifestPath: string;
  artifactPath: string;
  token: string;
};

export type LocalPackageAdmission = {
  manifestPath: string;
  artifactPath: string;
};

export type RuntimeEngine = {
  id: string;
  name: string;
  role: string;
  authority: string;
  category: string;
  component_kind: string;
  inputs: string[];
  outputs: string[];
  invariants: string[];
  evidence: string[];
  source_modules: string[];
  related_components: string[];
  documentation: string[];
};

export type RuntimeTool = {
  id: string;
  version: string;
  name: string;
  capability: string;
  operation: string;
};

export type RuntimeOrchestrationRole = {
  role_id: string;
  role: string;
  harness_id: string;
  repository_id: string;
  workspace_id: string;
  exact_commit: string;
  state: "queued" | "running" | "completed";
};

export type RuntimeOrchestrationRun = {
  run_id: string;
  coordinator_workspace_id: string;
  plan_id: string;
  status: "queued" | "running" | "completed" | "interrupted" | "cancelled";
  worker_id: string | null;
  roles: RuntimeOrchestrationRole[];
  receipt_count: number;
  handoffs_used: number;
  interruption_reason: string | null;
  created_at_unix_seconds: number;
  updated_at_unix_seconds: number;
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

export type RuntimeMemoryRecord = {
  memory_id: string;
  tier: string;
  kind: string;
  summary: string;
  classification: string;
  created_at_unix_seconds: number;
  provenance: string;
  origin: string;
  evidence_count: number;
};

export type RuntimeApproval = {
  approval_id: string;
  session_id: string;
  execution_id: string;
  gene_id: string;
  request_digest: string;
  request_summary: string;
  policy_version: number;
  expires_at_unix_seconds: number;
  status: string;
  approver_id: string | null;
  created_at_unix_seconds: number;
};

export type RuntimeEvolutionProposal = {
  proposal_id: string;
  source: string;
  base_artifact: string;
  candidate_artifact: string;
  evidence_digest: string;
  expected_outcome: string;
  created_at_unix_seconds: number;
  state: string;
  evaluation: {
    trajectory_score: number;
    outcome_score: number;
    holdout_passed: boolean;
    policy_passed: boolean;
    regression_passed: boolean;
    evaluated_at_unix_seconds: number;
    holdout_digest: string | null;
  } | null;
  approval: {
    approver_id: string;
    policy_version: number;
    approved_at_unix_seconds: number;
    signer_id: string;
    signature_present: boolean;
  } | null;
  canary: {
    passed: boolean;
    failure_count: number;
    note: string;
    evaluated_at_unix_seconds: number;
  } | null;
  candidate?: {
    kind: string;
    target_id: string;
    provider_id: string;
    generated_at_unix_seconds: number | null;
    base_bytes: number;
    candidate_bytes: number;
    changed_units: number;
    added_units: number;
    removed_units: number;
    unit: string;
    preview?: {
      format: string;
      base: string;
      candidate: string;
      truncated: boolean;
    } | null;
  } | null;
};

export type RuntimeArtifactActivation = {
  proposal_id: string;
  base_artifact: string;
  candidate_artifact: string;
  activated_at_unix_seconds: number;
};

export type RuntimeEvolutionMutation = {
  operation: "activate" | "rollback";
  proposal_id: string;
  state: string;
  artifact: string;
  occurred_at_unix_seconds: number;
  backup_directory: string;
  reconciled_bindings: number;
};

export type RuntimeRun = {
  mode: "direct" | "agent";
  session_id: string;
  execution_id: string | null;
  selected_harness: string | null;
  selected_gene: string | null;
  status: string;
  output: string;
  receipt_count: number;
  event_count: number;
  status_detail?: string;
  approval?: RuntimeApproval;
  turns?: number;
  tool_calls?: number;
  provider_calls?: number;
  prompt_tokens?: number;
  completion_tokens?: number;
  cached_prompt_tokens?: number;
  cache_write_prompt_tokens?: number;
  run_count?: number;
};

type RuntimeRunWire = Omit<RuntimeRun, "mode">;

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

type OrchestrationListResponse = {
  kind: "orchestration_list";
  runs: RuntimeOrchestrationRun[];
};

type OrchestrationInspectResponse = {
  kind: "orchestration_inspect";
  run: RuntimeOrchestrationRun;
};

type OrchestrationMutationResponse = {
  kind: "orchestration_mutation";
  operation: "cancel" | "resume";
  run: RuntimeOrchestrationRun;
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
  run: RuntimeRunWire;
};

type AgentRunResponse = {
  kind: "agent_run";
  run: RuntimeRunWire;
};

type SessionEventsResponse = {
  kind: "session_events";
  events: {
    events: RuntimeEvent[];
    next_sequence: number | null;
  };
};

type SessionMemoryResponse = {
  kind: "session_memory";
  memory: {
    session_id: string;
    records: RuntimeMemoryRecord[];
  };
};

type ApprovalListResponse = {
  kind: "approval_list";
  approvals: RuntimeApproval[];
};

type ApprovalResponse = {
  kind: "approval_inspect" | "approval_resolve";
  approval: RuntimeApproval;
};

type EvolutionListResponse = {
  kind: "evolution_list";
  proposals: RuntimeEvolutionProposal[];
};

type EvolutionInspectResponse = {
  kind: "evolution_inspect";
  proposal: RuntimeEvolutionProposal;
};

type EvolutionActivationsResponse = {
  kind: "evolution_activations";
  activations: RuntimeArtifactActivation[];
};

type EvolutionMutationResponse = {
  kind: "evolution_mutation";
  mutation: RuntimeEvolutionMutation;
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

export async function configureProvider(input: ProviderConfiguration): Promise<NativeConfigurationResult> {
  if (!isNativeRuntime()) {
    throw new Error("Provider configuration is available only in the Pandora desktop app");
  }
  return invoke<NativeConfigurationResult>("configure_provider", { input });
}

export async function configureMcp(input: McpConfiguration): Promise<NativeConfigurationResult> {
  if (!isNativeRuntime()) {
    throw new Error("MCP configuration is available only in the Pandora desktop app");
  }
  return invoke<NativeConfigurationResult>("configure_mcp", { input });
}

export async function listRegistryProfiles(): Promise<NativeRegistryResult> {
  if (!isNativeRuntime()) {
    return {
      message: "Registry profiles are available only in the Pandora desktop app.",
      data: { registries: [] },
    };
  }
  return invoke<NativeRegistryResult>("list_registry_profiles");
}

export async function configureRegistryProfile(input: RegistryConfiguration): Promise<NativeConfigurationResult> {
  if (!isNativeRuntime()) {
    throw new Error("Registry configuration is available only in the Pandora desktop app");
  }
  return invoke<NativeConfigurationResult>("configure_registry_profile", { input });
}

export async function listLocalPackages(): Promise<NativePackageResult> {
  if (!isNativeRuntime()) {
    return {
      message: "Package management is available only in the Pandora desktop app.",
      restartRequired: false,
      data: { packages: [] },
    };
  }
  return invoke<NativePackageResult>("list_local_packages");
}

export async function installRegistryPackage(input: RegistryPackageInstall): Promise<NativePackageResult> {
  if (!isNativeRuntime()) {
    throw new Error("Registry package installation is available only in the Pandora desktop app");
  }
  return invoke<NativePackageResult>("install_registry_package", { input });
}

export async function installGitHubPackage(input: GitHubPackageInstall): Promise<NativePackageResult> {
  if (!isNativeRuntime()) {
    throw new Error("GitHub package installation is available only in the Pandora desktop app");
  }
  return invoke<NativePackageResult>("install_github_package", { input });
}

export async function admitLocalPackage(input: LocalPackageAdmission): Promise<NativePackageResult> {
  if (!isNativeRuntime()) {
    throw new Error("Local package admission is available only in the Pandora desktop app");
  }
  return invoke<NativePackageResult>("admit_local_package", { input });
}

export async function previewPackageRemoval(packageId: string, version: string): Promise<NativePackageResult> {
  if (!isNativeRuntime()) {
    throw new Error("Package removal is available only in the Pandora desktop app");
  }
  return invoke<NativePackageResult>("preview_package_removal", {
    input: { packageId, version },
  });
}

export async function previewPackageEnable(packageId: string, version: string): Promise<NativePackageResult> {
  if (!isNativeRuntime()) throw new Error("Package activation is available only in the Pandora desktop app");
  return invoke<NativePackageResult>("preview_package_enable", { input: { packageId, version } });
}

export async function enableLocalPackage(packageId: string, version: string, confirmation: string): Promise<NativePackageResult> {
  if (!isNativeRuntime()) throw new Error("Package activation is available only in the Pandora desktop app");
  return invoke<NativePackageResult>("enable_local_package", { input: { packageId, version, confirmation } });
}

export async function previewPackageDisable(packageId: string, version: string): Promise<NativePackageResult> {
  if (!isNativeRuntime()) throw new Error("Package disable is available only in the Pandora desktop app");
  return invoke<NativePackageResult>("preview_package_disable", { input: { packageId, version } });
}

export async function disableLocalPackage(packageId: string, version: string, confirmation: string): Promise<NativePackageResult> {
  if (!isNativeRuntime()) throw new Error("Package disable is available only in the Pandora desktop app");
  return invoke<NativePackageResult>("disable_local_package", { input: { packageId, version, confirmation } });
}

export async function previewPackageRollback(packageId: string): Promise<NativePackageResult> {
  if (!isNativeRuntime()) throw new Error("Package rollback is available only in the Pandora desktop app");
  return invoke<NativePackageResult>("preview_package_rollback", { input: { packageId, confirmation: "" } });
}

export async function rollbackLocalPackage(packageId: string, confirmation: string): Promise<NativePackageResult> {
  if (!isNativeRuntime()) throw new Error("Package rollback is available only in the Pandora desktop app");
  return invoke<NativePackageResult>("rollback_local_package", { input: { packageId, confirmation } });
}

export async function removeLocalPackage(packageId: string, version: string, confirmation: string): Promise<NativePackageResult> {
  if (!isNativeRuntime()) {
    throw new Error("Package removal is available only in the Pandora desktop app");
  }
  return invoke<NativePackageResult>("remove_local_package", {
    input: { packageId, version, confirmation },
  });
}

export async function lockLocalPackages(): Promise<NativePackageResult> {
  if (!isNativeRuntime()) {
    throw new Error("Package locking is available only in the Pandora desktop app");
  }
  return invoke<NativePackageResult>("lock_local_packages");
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

  async orchestrations(limit = 64): Promise<RuntimeOrchestrationRun[]> {
    try {
      const response = await this.call<OrchestrationListResponse>("orchestration.list", { limit });
      return response.runs;
    } catch (error: unknown) {
      if (error instanceof Error && (error.message === "method_not_found" || error.message === "orchestration_unavailable")) {
        return [];
      }
      throw error;
    }
  }

  async inspectOrchestration(runId: string): Promise<RuntimeOrchestrationRun> {
    const response = await this.call<OrchestrationInspectResponse>("orchestration.inspect", { run_id: runId });
    return response.run;
  }

  async cancelOrchestration(runId: string, confirmation: string): Promise<RuntimeOrchestrationRun> {
    const response = await this.call<OrchestrationMutationResponse>("orchestration.cancel", {
      run_id: runId,
      confirmation,
    });
    return response.run;
  }

  async resumeOrchestration(runId: string, confirmation: string): Promise<RuntimeOrchestrationRun> {
    const response = await this.call<OrchestrationMutationResponse>("orchestration.resume", {
      run_id: runId,
      confirmation,
    });
    return response.run;
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
    return { ...response.run, mode: "direct" };
  }

  async resume(approvalId: string, task: string, requestedHarness: string | null = null): Promise<RuntimeRun> {
    const response = await this.call<RunResponse>("run.resume", {
      approval_id: approvalId,
      request: {
        task,
        requested_harness: requestedHarness,
        requested_gene: null,
      },
    });
    return { ...response.run, mode: "direct" };
  }

  async agentRun(
    task: string,
    sessionId: string | null = null,
    requestedHarness: string | null = null,
    contextAttachments: RuntimeContextAttachment[] = [],
    requestedProvider: string | null = null,
    requestedModel: string | null = null,
  ): Promise<RuntimeRun> {
    const response = await this.call<AgentRunResponse>("agent.execute", {
      task,
      session_id: sessionId,
      requested_harness: requestedHarness,
      context_attachments: contextAttachments,
      requested_provider: requestedProvider,
      requested_model: requestedModel,
    });
    return { ...response.run, mode: "agent" };
  }

  async agentResume(
    approvalId: string,
    requestedProvider: string | null = null,
    requestedModel: string | null = null,
  ): Promise<RuntimeRun> {
    const response = await this.call<AgentRunResponse>("agent.resume", {
      approval_id: approvalId,
      requested_provider: requestedProvider,
      requested_model: requestedModel,
    });
    return { ...response.run, mode: "agent" };
  }

  async approvals(limit = 64): Promise<RuntimeApproval[]> {
    const response = await this.call<ApprovalListResponse>("approval.list", { limit });
    return response.approvals;
  }

  async inspectApproval(approvalId: string): Promise<RuntimeApproval> {
    const response = await this.call<ApprovalResponse>("approval.inspect", { approval_id: approvalId });
    return response.approval;
  }

  async resolveApproval(approvalId: string, allow: boolean): Promise<RuntimeApproval> {
    const response = await this.call<ApprovalResponse>("approval.resolve", { approval_id: approvalId, allow });
    return response.approval;
  }

  async evolution(limit = 64): Promise<RuntimeEvolutionProposal[]> {
    try {
      const response = await this.call<EvolutionListResponse>("evolution.list", { limit });
      return response.proposals;
    } catch (error: unknown) {
      if (error instanceof Error && error.message === "method_not_found") {
        return [];
      }
      throw error;
    }
  }

  async inspectEvolution(proposalId: string): Promise<RuntimeEvolutionProposal> {
    const response = await this.call<EvolutionInspectResponse>("evolution.inspect", {
      proposal_id: proposalId,
    });
    return response.proposal;
  }

  async evolutionActivations(limit = 64): Promise<RuntimeArtifactActivation[]> {
    try {
      const response = await this.call<EvolutionActivationsResponse>("evolution.activations", { limit });
      return response.activations;
    } catch (error: unknown) {
      if (error instanceof Error && error.message === "method_not_found") {
        return [];
      }
      throw error;
    }
  }

  async activateEvolution(proposalId: string, confirmation: string): Promise<RuntimeEvolutionMutation> {
    const response = await this.call<EvolutionMutationResponse>("evolution.activate", {
      proposal_id: proposalId,
      confirmation,
    });
    return response.mutation;
  }

  async rollbackEvolution(proposalId: string, confirmation: string, reason: string): Promise<RuntimeEvolutionMutation> {
    const response = await this.call<EvolutionMutationResponse>("evolution.rollback", {
      proposal_id: proposalId,
      confirmation,
      reason,
    });
    return response.mutation;
  }

  async events(sessionId: string, limit = 256): Promise<RuntimeEvent[]> {
    const response = await this.call<SessionEventsResponse>("session.events", {
      session_id: sessionId,
      after_sequence: null,
      limit,
    });
    return response.events.events;
  }

  async memory(sessionId: string, limit = 64): Promise<RuntimeMemoryRecord[]> {
    const response = await this.call<SessionMemoryResponse>("session.memory", {
      session_id: sessionId,
      limit,
    });
    return response.memory.records;
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
