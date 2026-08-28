import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";

const runtime = vi.hoisted(() => ({
  activateEvolution: vi.fn(),
  agentResume: vi.fn(),
  agentRun: vi.fn(),
  capabilities: vi.fn(),
  configureMcp: vi.fn(),
  configureProvider: vi.fn(),
  engines: vi.fn(),
  evolution: vi.fn(),
  evolutionActivations: vi.fn(),
  events: vi.fn(),
  health: vi.fn(),
  inspectEvolution: vi.fn(),
  inspectOrchestration: vi.fn(),
  inspectSession: vi.fn(),
  memory: vi.fn(),
  orchestrations: vi.fn(),
  providers: vi.fn(),
  resolveApproval: vi.fn(),
  rollbackEvolution: vi.fn(),
  cancelOrchestration: vi.fn(),
  resumeOrchestration: vi.fn(),
  resume: vi.fn(),
  run: vi.fn(),
  sessions: vi.fn(),
  tools: vi.fn(),
}));

vi.mock("./runtimeClient", () => ({
  nativeEndpoint: "tauri://pandora",
  isNativeRuntime: () => true,
  loadRuntimeEndpoint: () => "tauri://pandora",
  saveRuntimeEndpoint: vi.fn(),
  configureMcp: runtime.configureMcp,
  configureProvider: runtime.configureProvider,
  startLocalService: vi.fn(),
  stopLocalService: vi.fn(),
  RuntimeClient: class {
    activateEvolution = runtime.activateEvolution;
    agentResume = runtime.agentResume;
    agentRun = runtime.agentRun;
    capabilities = runtime.capabilities;
    engines = runtime.engines;
    evolution = runtime.evolution;
    evolutionActivations = runtime.evolutionActivations;
    events = runtime.events;
    health = runtime.health;
    inspectEvolution = runtime.inspectEvolution;
    inspectOrchestration = runtime.inspectOrchestration;
    inspectSession = runtime.inspectSession;
    memory = runtime.memory;
    orchestrations = runtime.orchestrations;
    providers = runtime.providers;
    resolveApproval = runtime.resolveApproval;
    rollbackEvolution = runtime.rollbackEvolution;
    cancelOrchestration = runtime.cancelOrchestration;
    resumeOrchestration = runtime.resumeOrchestration;
    resume = runtime.resume;
    run = runtime.run;
    sessions = runtime.sessions;
    tools = runtime.tools;
  },
}));

const session = {
  session_id: "session-1",
  principal_id: "principal-1",
  tenant_id: "tenant-1",
  workspace_id: "workspace-1",
  created_at_unix_seconds: 1,
};

beforeEach(() => {
  vi.clearAllMocks();
  window.localStorage.clear();
  runtime.health.mockResolvedValue({ status: "ready" });
  runtime.sessions.mockResolvedValue([]);
  runtime.capabilities.mockResolvedValue([]);
  runtime.engines.mockResolvedValue([]);
  runtime.evolution.mockResolvedValue([]);
  runtime.evolutionActivations.mockResolvedValue([]);
  runtime.tools.mockResolvedValue([]);
  runtime.providers.mockResolvedValue([]);
  runtime.configureProvider.mockResolvedValue({ message: "Provider custom configured.", restartRequired: true });
  runtime.configureMcp.mockResolvedValue({ message: "MCP server local-tools configured.", restartRequired: true });
  runtime.inspectSession.mockResolvedValue({ session, event_count: 0 });
  runtime.events.mockResolvedValue([]);
  runtime.memory.mockResolvedValue([]);
  runtime.orchestrations.mockResolvedValue([]);
});

afterEach(() => cleanup());

describe("Pandora desktop run state", () => {
  it("disables duplicate submission while a governed run is active", async () => {
    let completeRun!: (value: unknown) => void;
    runtime.agentRun.mockImplementation(
      () => new Promise((resolve) => {
        completeRun = resolve;
      }),
    );

    render(<App />);

    const composer = await screen.findByLabelText("Pandora task");
    fireEvent.change(composer, { target: { value: "Inspect this repository" } });
    fireEvent.submit(composer.closest("form")!);

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Pandora is running" })).toBeDisabled();
      expect(composer).toBeDisabled();
    });
    expect(runtime.agentRun).toHaveBeenCalledTimes(1);

    completeRun({
      mode: "agent",
      session_id: session.session_id,
      execution_id: "execution-1",
      selected_harness: "coding-domain",
      selected_gene: null,
      status: "completed",
      output: "done",
      receipt_count: 0,
      event_count: 0,
    });

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();
      expect(composer).toBeEnabled();
      expect(composer).toHaveValue("");
    });
  });

  it("keeps the composer available after inspecting an existing session", async () => {
    runtime.sessions.mockResolvedValue([session]);

    render(<App />);

    const sessionButton = await screen.findByRole("button", { name: /session-1/ });
    fireEvent.click(sessionButton);

    await waitFor(() => expect(runtime.inspectSession).toHaveBeenCalledWith(session.session_id));
    expect(screen.getByLabelText("Pandora task")).toBeEnabled();
  });

  it("requires exact confirmation and a reason before rolling back an active binding", async () => {
    runtime.evolution.mockResolvedValue([{
      proposal_id: "proposal-a",
      source: "gepa",
      base_artifact: "base-a",
      candidate_artifact: "candidate-a",
      evidence_digest: "evidence-a",
      expected_outcome: "Improve verification reliability",
      created_at_unix_seconds: 10,
      state: "approved",
      evaluation: {
        trajectory_score: 95,
        outcome_score: 96,
        holdout_passed: true,
        policy_passed: true,
        regression_passed: true,
        evaluated_at_unix_seconds: 11,
        holdout_digest: "holdout-a",
      },
      approval: {
        approver_id: "parliament-a",
        policy_version: 1,
        approved_at_unix_seconds: 12,
        signer_id: "signer-a",
        signature_present: true,
      },
      canary: null,
    }]);
    runtime.evolutionActivations.mockResolvedValue([{
      proposal_id: "proposal-a",
      base_artifact: "sha256:base-a",
      candidate_artifact: "sha256:candidate-a",
      activated_at_unix_seconds: 13,
    }]);
    runtime.rollbackEvolution.mockResolvedValue({
      operation: "rollback",
      proposal_id: "proposal-a",
      state: "rolled_back",
      artifact: "base-a",
      occurred_at_unix_seconds: 14,
      backup_directory: "backups/evolution-14",
      reconciled_bindings: 0,
    });
    runtime.inspectEvolution.mockImplementation(async () => ({
      ...(await runtime.evolution())[0],
      candidate: {
        kind: "gene",
        target_id: "publisher/candidate@1.0.0",
        provider_id: "publisher",
        generated_at_unix_seconds: null,
        base_bytes: 17,
        candidate_bytes: 22,
        changed_units: 1,
        added_units: 0,
        removed_units: 0,
        unit: "lines",
      },
    }));

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /Evolution/ }));

    expect(await screen.findByRole("heading", { name: "Improve verification reliability" })).toBeInTheDocument();
    expect(screen.getByText("Passed · 95/96")).toBeInTheDocument();
    expect(screen.getByText("parliament-a · policy v1")).toBeInTheDocument();
    expect(screen.getByText("catalog active")).toBeInTheDocument();
    expect(screen.getByText("Runtime authority").nextSibling).toHaveTextContent("Unchanged");
    fireEvent.click(screen.getByRole("button", { name: /Inspect candidate diff/ }));
    await waitFor(() => {
      expect(runtime.inspectEvolution).toHaveBeenCalledWith("proposal-a");
      expect(screen.getByText("1 changed · +0 / −0 lines · 17 → 22 bytes")).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole("button", { name: /Rollback binding/ }));
    fireEvent.change(screen.getByLabelText("Confirm rollback proposal-a"), { target: { value: "proposal-a" } });
    fireEvent.change(screen.getByLabelText("Rollback reason proposal-a"), { target: { value: "Canary regression" } });
    fireEvent.click(screen.getByRole("button", { name: "Confirm rollback" }));

    await waitFor(() => {
      expect(runtime.rollbackEvolution).toHaveBeenCalledWith("proposal-a", "proposal-a", "Canary regression");
      expect(screen.getByText("Binding rolled back")).toBeInTheDocument();
      expect(screen.getByText("backups/evolution-14")).toBeInTheDocument();
    });
  });
  it("activates only a canary-passed proposal after exact confirmation", async () => {
    runtime.evolution.mockResolvedValue([{
      proposal_id: "proposal-canary",
      source: "research",
      base_artifact: "base-canary",
      candidate_artifact: "candidate-canary",
      evidence_digest: "evidence-canary",
      expected_outcome: "Improve bounded planning",
      created_at_unix_seconds: 20,
      state: "canary_passed",
      evaluation: {
        trajectory_score: 98,
        outcome_score: 99,
        holdout_passed: true,
        policy_passed: true,
        regression_passed: true,
        evaluated_at_unix_seconds: 21,
        holdout_digest: "holdout-canary",
      },
      approval: {
        approver_id: "parliament-canary",
        policy_version: 1,
        approved_at_unix_seconds: 22,
        signer_id: "signer-canary",
        signature_present: true,
      },
      canary: {
        passed: true,
        failure_count: 0,
        note: "passed",
        evaluated_at_unix_seconds: 23,
      },
      candidate: {
        kind: "prompt",
        target_id: "planner.system",
        provider_id: "research-provider",
        generated_at_unix_seconds: 20,
        base_bytes: 120,
        candidate_bytes: 138,
        changed_units: 3,
        added_units: 2,
        removed_units: 1,
        unit: "lines",
      },
    }]);
    runtime.activateEvolution.mockResolvedValue({
      operation: "activate",
      proposal_id: "proposal-canary",
      state: "active",
      artifact: "candidate-canary",
      occurred_at_unix_seconds: 24,
      backup_directory: "backups/evolution-24",
      reconciled_bindings: 0,
    });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /Evolution/ }));
    expect(screen.getByText("3 changed · +2 / −1 lines · 120 → 138 bytes")).toBeInTheDocument();
    expect(screen.getByText("LINEAGE")).toBeInTheDocument();
    fireEvent.click(await screen.findByRole("button", { name: "Activate candidate" }));
    const confirm = screen.getByRole("button", { name: "Confirm activation" });
    expect(confirm).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Confirm activate proposal-canary"), { target: { value: "proposal-canary" } });
    fireEvent.click(confirm);

    await waitFor(() => {
      expect(runtime.activateEvolution).toHaveBeenCalledWith("proposal-canary", "proposal-canary");
      expect(screen.getByText("Candidate activated")).toBeInTheDocument();
    });
  });


  it("resolves and resumes an exact runtime approval", async () => {
    const approval = {
      approval_id: "approval-execution-1",
      session_id: session.session_id,
      execution_id: "execution-1",
      gene_id: "coding.patch",
      request_digest: "digest-1",
      request_summary: "filesystem.write for patch on README.md",
      policy_version: 1,
      expires_at_unix_seconds: 900,
      status: "pending",
      approver_id: null,
      created_at_unix_seconds: 1,
    };
    runtime.agentRun.mockResolvedValue({
      mode: "agent",
      session_id: session.session_id,
      execution_id: "execution-1",
      selected_harness: "coding-domain",
      selected_gene: "coding.patch",
      status: "approval_required",
      status_detail: "explicit approval is required",
      output: "",
      receipt_count: 0,
      event_count: 4,
      approval,
    });
    runtime.resolveApproval.mockResolvedValue({ ...approval, status: "approved" });
    runtime.agentResume.mockResolvedValue({
      mode: "agent",
      session_id: session.session_id,
      execution_id: "execution-1",
      selected_harness: "coding-domain",
      selected_gene: "coding.patch",
      status: "completed",
      output: "README updated",
      receipt_count: 1,
      event_count: 8,
    });

    render(<App />);

    const composer = await screen.findByLabelText("Pandora task");
    fireEvent.change(composer, { target: { value: "patch:README.md:approved" } });
    fireEvent.submit(composer.closest("form")!);
    const allow = await screen.findByRole("button", { name: /Allow once/ });
    fireEvent.click(allow);

    await waitFor(() => {
      expect(runtime.resolveApproval).toHaveBeenCalledWith(approval.approval_id, true);
      expect(runtime.agentResume).toHaveBeenCalledWith(approval.approval_id);
      expect(screen.getByText("README updated")).toBeInTheDocument();
    });
  });

  it("progressively discloses run evidence and scoped context", async () => {
    runtime.sessions.mockResolvedValue([session]);
    runtime.events.mockResolvedValue([{
      event_id: "event-1",
      event_type: "policy_approved",
      payload: {},
    }]);
    runtime.inspectSession.mockResolvedValue({ session, event_count: 1 });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /session-1/ }));
    await waitFor(() => expect(runtime.inspectSession).toHaveBeenCalledWith(session.session_id));

    fireEvent.click(screen.getByRole("tab", { name: "evidence" }));
    expect(screen.getByText("No run selected")).toBeInTheDocument();
    expect(screen.getByText("policy approved")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("tab", { name: "context" }));
    expect(screen.getByRole("heading", { name: "workspace-1" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Context is evidence, not authority" })).toBeInTheDocument();
    expect(screen.getByText("Progressive · redacted")).toBeInTheDocument();
  });

  it("inspects Genes, extensions, authority, and receipt posture in Harness Lab", async () => {
    runtime.capabilities.mockResolvedValue([{
      id: "coding-domain",
      version: "1.2.0",
      name: "Coding Domain",
      kind: "domain",
      gene_count: 2,
      runnable: true,
      gene_ids: ["coding.inspect", "coding.patch"],
    }]);
    runtime.tools.mockResolvedValue([{
      id: "filesystem.read",
      version: "1.0.0",
      name: "Filesystem Reader",
      capability: "filesystem",
      operation: "read",
    }]);

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Harness Lab" }));

    expect(await screen.findByRole("heading", { name: "Coding Domain" })).toBeInTheDocument();
    expect(screen.getByText("coding.inspect")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("tab", { name: "Plugins & tools" }));
    expect(screen.getByText("Filesystem Reader")).toBeInTheDocument();
    expect(screen.getByText("filesystem / read")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("tab", { name: "authority" }));
    expect(screen.getByText("Never directly")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("tab", { name: "receipts" }));
    expect(screen.getByRole("heading", { name: "Evidence follows execution" })).toBeInTheDocument();
  });


  it("keeps native local service credentials out of the interface", async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Connections" }));

    expect(await screen.findByText("No account required")).toBeInTheDocument();
    expect(screen.queryByLabelText("Development token")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Connect preview/ })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /local service/ })).toBeInTheDocument();
  });


  it("configures a custom provider without persisting the API key in browser storage", async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Connections" }));

    fireEvent.change(screen.getByLabelText("Provider base URL"), { target: { value: "https://models.example.test/v1" } });
    fireEvent.change(screen.getByLabelText("Provider model"), { target: { value: "pandora-model" } });
    fireEvent.change(screen.getByLabelText("Provider API key"), { target: { value: "secret-test-key" } });
    fireEvent.click(screen.getByRole("button", { name: /Save provider/ }));

    await waitFor(() => expect(runtime.configureProvider).toHaveBeenCalledWith({
      name: "custom",
      protocol: "open_ai_compatible",
      baseUrl: "https://models.example.test/v1",
      model: "pandora-model",
      apiKeyEnvironment: "PANDORA_CUSTOM_API_KEY",
      apiKey: "secret-test-key",
    }));
    expect(screen.getByLabelText("Provider API key")).toHaveValue("");
    expect(screen.getByRole("status")).toHaveTextContent("Restart the local service to apply it");
    expect(Object.values(window.localStorage)).not.toContain("secret-test-key");
  });

  it("configures an absolute local MCP server with explicit arguments", async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Connections" }));
    fireEvent.click(screen.getByRole("tab", { name: "Local MCP server" }));

    fireEvent.change(screen.getByLabelText("MCP server ID"), { target: { value: "local-tools" } });
    fireEvent.change(screen.getByLabelText("MCP program path"), { target: { value: "C:\\tools\\mcp-server.exe" } });
    fireEvent.change(screen.getByLabelText("MCP arguments JSON"), { target: { value: "[\"--stdio\"]" } });
    fireEvent.click(screen.getByRole("button", { name: /Save MCP server/ }));

    await waitFor(() => expect(runtime.configureMcp).toHaveBeenCalledWith({
      serverId: "local-tools",
      program: "C:\\tools\\mcp-server.exe",
      argumentsJson: "[\"--stdio\"]",
      mode: "auto",
    }));
    expect(screen.getByRole("status")).toHaveTextContent("Restart the local service to apply it");
  });

  it("inspects and exactly cancels a queued background orchestration", async () => {
    const queuedRun = {
      run_id: "run-queued-1",
      coordinator_workspace_id: "workspace-1",
      plan_id: "release-plan",
      status: "queued",
      worker_id: null,
      roles: [{
        role_id: "role-review",
        role: "reviewer",
        harness_id: "coding-domain",
        repository_id: "pandora-agent",
        workspace_id: "workspace-1",
        exact_commit: "abc1234",
        state: "pending",
      }],
      receipt_count: 0,
      handoffs_used: 0,
      interruption_reason: null,
      created_at_unix_seconds: 30,
      updated_at_unix_seconds: 30,
    };
    runtime.orchestrations.mockResolvedValue([queuedRun]);
    runtime.cancelOrchestration.mockResolvedValue({ ...queuedRun, status: "cancelled", updated_at_unix_seconds: 31 });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Background Runs" }));

    expect(await screen.findByRole("heading", { name: "release-plan" })).toBeInTheDocument();
    expect(screen.getByText("pandora-agent / workspace-1")).toBeInTheDocument();
    expect(screen.getByText("abc1234")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Cancel run" }));
    const cancel = screen.getByRole("button", { name: "Confirm cancellation" });
    expect(cancel).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Confirm cancel run-queued-1"), { target: { value: "run-queued-1" } });
    fireEvent.click(cancel);

    await waitFor(() => {
      expect(runtime.cancelOrchestration).toHaveBeenCalledWith("run-queued-1", "run-queued-1");
      expect(screen.getByText("Run cancelled")).toBeInTheDocument();
    });
  });

  it("resumes an interrupted orchestration only after exact confirmation", async () => {
    const interruptedRun = {
      run_id: "run-interrupted-1",
      coordinator_workspace_id: "workspace-1",
      plan_id: "repair-plan",
      status: "interrupted",
      worker_id: null,
      roles: [{
        role_id: "role-repair",
        role: "repairer",
        harness_id: "coding-domain",
        repository_id: "pandora-agent",
        workspace_id: "workspace-1",
        exact_commit: "def5678",
        state: "interrupted",
      }],
      receipt_count: 0,
      handoffs_used: 1,
      interruption_reason: "worker lease expired",
      created_at_unix_seconds: 40,
      updated_at_unix_seconds: 41,
    };
    runtime.orchestrations.mockResolvedValue([interruptedRun]);
    runtime.resumeOrchestration.mockResolvedValue({ ...interruptedRun, status: "queued", interruption_reason: null, updated_at_unix_seconds: 42 });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Background Runs" }));
    fireEvent.click(await screen.findByRole("button", { name: /Resume safely/ }));
    const resume = screen.getByRole("button", { name: "Confirm resume" });
    expect(resume).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Confirm resume run-interrupted-1"), { target: { value: "run-interrupted-1" } });
    fireEvent.click(resume);

    await waitFor(() => {
      expect(runtime.resumeOrchestration).toHaveBeenCalledWith("run-interrupted-1", "run-interrupted-1");
      expect(screen.getByText("Run requeued")).toBeInTheDocument();
    });
  });

});
