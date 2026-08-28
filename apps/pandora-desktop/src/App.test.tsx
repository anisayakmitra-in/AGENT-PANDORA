import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";

const runtime = vi.hoisted(() => ({
  admitLocalPackage: vi.fn(),
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
  installRegistryPackage: vi.fn(),
  listLocalPackages: vi.fn(),
  lockLocalPackages: vi.fn(),
  memory: vi.fn(),
  orchestrations: vi.fn(),
  providers: vi.fn(),
  previewPackageRemoval: vi.fn(),
  removeLocalPackage: vi.fn(),
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
  admitLocalPackage: runtime.admitLocalPackage,
  installRegistryPackage: runtime.installRegistryPackage,
  listLocalPackages: runtime.listLocalPackages,
  lockLocalPackages: runtime.lockLocalPackages,
  previewPackageRemoval: runtime.previewPackageRemoval,
  removeLocalPackage: runtime.removeLocalPackage,
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
  runtime.listLocalPackages.mockResolvedValue({
    message: "0 local package(s) available.",
    restartRequired: false,
    data: { packages: [] },
  });
  runtime.lockLocalPackages.mockResolvedValue({
    message: "Deterministic package lock written for the current workspace.",
    restartRequired: false,
    data: { package_count: 0 },
  });
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

  it("keeps a failed request editable and retryable without marking the service offline", async () => {
    runtime.agentRun.mockRejectedValueOnce(new Error("provider timed out"));

    render(<App />);
    const composer = await screen.findByLabelText("Pandora task");
    fireEvent.change(composer, { target: { value: "Inspect and recover" } });
    fireEvent.submit(composer.closest("form")!);

    expect(await screen.findByText("provider timed out")).toBeInTheDocument();
    expect(composer).toBeEnabled();
    expect(composer).toHaveValue("Inspect and recover");
    expect(screen.getByRole("button", { name: "Send" })).toBeEnabled();
    expect(screen.getAllByText("Runtime connected").length).toBeGreaterThan(0);
  });

  it("sends selected text files through the bounded context contract", async () => {
    runtime.agentRun.mockResolvedValue({
      mode: "agent",
      session_id: session.session_id,
      execution_id: "execution-context",
      selected_harness: "coding-domain",
      selected_gene: null,
      status: "completed",
      output: "context used",
      receipt_count: 0,
      event_count: 0,
    });
    render(<App />);

    const file = new File(["const answer = 42;"], "notes.ts", { type: "text/plain" });
    fireEvent.change(await screen.findByLabelText("Choose context files"), { target: { files: [file] } });

    expect(await screen.findByText("notes.ts")).toBeInTheDocument();
    expect(screen.getByText("Untrusted · no authority · 18 / 24576 bytes")).toBeInTheDocument();
    const composer = screen.getByLabelText("Pandora task");
    fireEvent.change(composer, { target: { value: "Use the selected notes" } });
    await waitFor(() => expect(screen.getByRole("button", { name: "Send" })).toBeEnabled());
    fireEvent.submit(composer.closest("form")!);

    await waitFor(() => {
      expect(runtime.agentRun).toHaveBeenCalledWith(
        "Use the selected notes",
        null,
        null,
        [{ name: "notes.ts", media_type: "text/plain", content: "const answer = 42;" }],
      );
    });
    expect(screen.queryByText("notes.ts")).not.toBeInTheDocument();
  });

  it("renders Council from recorded run evidence without fabricating authority", async () => {
    runtime.capabilities.mockResolvedValue([{
      id: "coding-domain",
      version: "2.1.0",
      name: "Coding",
      kind: "domain",
      gene_count: 4,
      runnable: true,
      gene_ids: ["workspace.diff"],
    }]);
    runtime.events.mockResolvedValue([
      { event_id: "event-policy", event_type: "policy_approved", payload: {} },
      { event_id: "event-approval", event_type: "approval_required", payload: {} },
    ]);
    runtime.agentRun.mockResolvedValue({
      mode: "agent",
      session_id: session.session_id,
      execution_id: "execution-council",
      selected_harness: "coding-domain",
      selected_gene: "workspace.diff",
      status: "approval_required",
      status_detail: "explicit approval is required",
      output: "",
      receipt_count: 0,
      event_count: 2,
      approval: {
        approval_id: "approval-council",
        session_id: session.session_id,
        execution_id: "execution-council",
        gene_id: "workspace.diff",
        request_digest: "sha256:council",
        request_summary: "Execute workspace.diff once",
        policy_version: 1,
        expires_at_unix_seconds: 999,
        status: "pending",
        approver_id: null,
        created_at_unix_seconds: 1,
      },
    });

    render(<App />);
    const composer = await screen.findByLabelText("Pandora task");
    fireEvent.change(composer, { target: { value: "Inspect the working diff" } });
    fireEvent.submit(composer.closest("form")!);
    await screen.findByText("Execute workspace.diff once");

    fireEvent.click(screen.getByRole("button", { name: "Council" }));

    expect(await screen.findByRole("heading", { name: "Council" })).toBeInTheDocument();
    expect(screen.getByText("PARLIAMENT")).toBeInTheDocument();
    expect(screen.getByText("SHADOW COUNCIL")).toBeInTheDocument();
    expect(screen.getByText("REFERENCE MONITOR")).toBeInTheDocument();
    expect(screen.getByText("v2.1.0 · domain")).toBeInTheDocument();
    expect(screen.getByText("sha256:council")).toBeInTheDocument();
    expect(screen.getByText("event-approval")).toBeInTheDocument();
    expect(screen.queryByText(/design preview/i)).not.toBeInTheDocument();
    expect(screen.getByText(/This page cannot vote, route, approve, or execute/)).toBeInTheDocument();
  });

  it("reads a workspace file through the governed runtime inspector", async () => {
    runtime.run.mockResolvedValue({
      mode: "direct",
      session_id: "session-inspect",
      execution_id: "execution-inspect",
      selected_harness: "coding-domain",
      selected_gene: "workspace.read",
      status: "completed",
      output: "# Pandora\n",
      receipt_count: 1,
      event_count: 4,
    });
    render(<App />);

    fireEvent.click(await screen.findByRole("tab", { name: "workspace" }));
    fireEvent.change(screen.getByLabelText("Workspace file path"), { target: { value: "docs/architecture.md" } });
    fireEvent.click(screen.getByRole("button", { name: "Read file" }));

    await waitFor(() => expect(runtime.run).toHaveBeenCalledWith("read:docs/architecture.md", "coding-domain"));
    expect(await screen.findByLabelText("Workspace inspection output")).toHaveTextContent("# Pandora");
    expect(screen.getByText("workspace.read")).toBeInTheDocument();
    expect(screen.getByText("1 receipt")).toBeInTheDocument();
  });

  it("requires an exact approval before the workspace diff command resumes", async () => {
    const approval = {
      approval_id: "approval-diff",
      session_id: "session-diff",
      execution_id: "execution-diff",
      gene_id: "workspace.diff",
      request_digest: "sha256:diff",
      request_summary: "Execute workspace.diff once",
      policy_version: 1,
      expires_at_unix_seconds: 999,
      status: "pending",
      approver_id: null,
      created_at_unix_seconds: 1,
    };
    runtime.run.mockResolvedValue({
      mode: "direct",
      session_id: "session-diff",
      execution_id: "execution-diff",
      selected_harness: "coding-domain",
      selected_gene: "workspace.diff",
      status: "approval_required",
      output: "",
      receipt_count: 0,
      event_count: 3,
      approval,
    });
    runtime.resolveApproval.mockResolvedValue({ ...approval, status: "approved", approver_id: "local-operator" });
    runtime.resume.mockResolvedValue({
      mode: "direct",
      session_id: "session-diff",
      execution_id: "execution-diff",
      selected_harness: "coding-domain",
      selected_gene: "workspace.diff",
      status: "completed",
      output: "diff --git a/README.md b/README.md",
      receipt_count: 1,
      event_count: 7,
    });
    render(<App />);

    fireEvent.click(await screen.findByRole("tab", { name: "workspace" }));
    fireEvent.click(screen.getByRole("button", { name: /Working diff/ }));
    expect(await screen.findByText("Execute workspace.diff once")).toBeInTheDocument();
    expect(screen.getByText("sha256:diff")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Allow once" }));

    await waitFor(() => {
      expect(runtime.resolveApproval).toHaveBeenCalledWith("approval-diff", true);
      expect(runtime.resume).toHaveBeenCalledWith("approval-diff", "diff", "coding-domain");
    });
    expect(await screen.findByLabelText("Workspace inspection output")).toHaveTextContent("diff --git");
  });

  it("fetches browser source only after exact network approval and renders it inertly", async () => {
    const approval = {
      approval_id: "approval-browser",
      session_id: "session-browser",
      execution_id: "execution-browser",
      gene_id: "browser.fetch",
      request_digest: "pandora-request-v2:sha256:browser",
      request_summary: "network.connect for fetch on example.com",
      policy_version: 1,
      expires_at_unix_seconds: 999,
      status: "pending",
      approver_id: null,
      created_at_unix_seconds: 1,
    };
    runtime.run.mockResolvedValue({
      mode: "direct",
      session_id: "session-browser",
      execution_id: "execution-browser",
      selected_harness: "research-domain",
      selected_gene: "browser.fetch",
      status: "approval_required",
      output: "",
      receipt_count: 0,
      event_count: 3,
      approval,
    });
    runtime.resolveApproval.mockResolvedValue({ ...approval, status: "approved", approver_id: "local-operator" });
    runtime.resume.mockResolvedValue({
      mode: "direct",
      session_id: "session-browser",
      execution_id: "execution-browser",
      selected_harness: "research-domain",
      selected_gene: "browser.fetch",
      status: "completed",
      output: JSON.stringify({
        url: "https://example.com/",
        status: 200,
        content_type: "text/html; charset=utf-8",
        body: "<h1>Pandora evidence</h1>",
        truncated: false,
        lossy: false,
      }),
      receipt_count: 1,
      event_count: 7,
    });
    render(<App />);

    fireEvent.click(await screen.findByRole("tab", { name: "browser" }));
    fireEvent.click(screen.getByRole("button", { name: "Fetch source" }));
    expect(await screen.findByText("network.connect for fetch on example.com")).toBeInTheDocument();
    expect(screen.getByText("pandora-request-v2:sha256:browser")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Allow once" }));

    await waitFor(() => {
      expect(runtime.run).toHaveBeenCalledWith("fetch:https://example.com/", "research-domain");
      expect(runtime.resolveApproval).toHaveBeenCalledWith("approval-browser", true);
      expect(runtime.resume).toHaveBeenCalledWith("approval-browser", "fetch:https://example.com/", "research-domain");
    });
    expect(await screen.findByLabelText("Browser evidence body")).toHaveTextContent("<h1>Pandora evidence</h1>");
    expect(screen.getByText("text/html; charset=utf-8")).toBeInTheDocument();
    expect(screen.queryByRole("iframe")).not.toBeInTheDocument();
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
        preview: {
          format: "text",
          base: "base service gene",
          candidate: "candidate service gene",
          truncated: false,
        },
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
      expect(screen.getByLabelText("Base artifact proposal-a")).toHaveTextContent("base service gene");
      expect(screen.getByLabelText("Candidate artifact proposal-a")).toHaveTextContent("candidate service gene");
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

    fireEvent.click(screen.getByRole("tab", { name: "workspace" }));
    expect(screen.getByRole("heading", { name: "workspace-1" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Inspection is evidence, not authority" })).toBeInTheDocument();
    expect(screen.getByText("Governed")).toBeInTheDocument();
    expect(screen.getByText("Exact permit path")).toBeInTheDocument();
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

  it("manages exact local packages without granting runtime authority", async () => {
    const localPackage = {
      id: "example/refactor",
      version: "1.2.3",
      kind: "gene",
      publisher: "example",
      content_hash: "sha256:1111111111111111111111111111111111111111111111111111111111111111",
      dependencies: [],
      compatibility: "pandora>=0.1.0",
      license: "MIT",
      trust: {
        level: "verified",
        has_signature: true,
        has_public_key: true,
      },
      meta_composition: null,
      state: "admitted",
      runtime_authority: false,
    };
    runtime.capabilities.mockResolvedValue([{
      id: "coding-domain",
      version: "1.2.0",
      name: "Coding Domain",
      kind: "domain",
      gene_count: 1,
      runnable: true,
      gene_ids: ["coding.inspect"],
    }]);
    runtime.listLocalPackages.mockResolvedValue({
      message: "1 local package(s) available.",
      restartRequired: false,
      data: { packages: [localPackage] },
    });
    runtime.installRegistryPackage.mockResolvedValue({
      message: "Package owner/new-gene admitted from the registry.",
      restartRequired: true,
      data: { package: localPackage },
    });
    runtime.previewPackageRemoval.mockResolvedValue({
      message: "Removal preview recorded for example/refactor@1.2.3; no package changed.",
      restartRequired: false,
      data: { dry_run: true, removed: false },
    });
    runtime.removeLocalPackage.mockResolvedValue({
      message: "Package example/refactor@1.2.3 removed after dependency and binding checks.",
      restartRequired: true,
      data: { dry_run: false, removed: true },
    });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Harness Lab" }));
    fireEvent.click(await screen.findByRole("tab", { name: "packages" }));

    expect(await screen.findByRole("heading", { name: "Signed package manager" })).toBeInTheDocument();
    expect(await screen.findByText("example/refactor")).toBeInTheDocument();
    expect(screen.getByText("Runtime authority").nextElementSibling).toHaveTextContent("none");
    expect(screen.getByText(/cannot replace Parliament, Shadow Council, ReferenceMonitor/)).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("Registry package ID"), { target: { value: "owner/new-gene" } });
    fireEvent.change(screen.getByLabelText("Registry package version"), { target: { value: "2.0.0" } });
    fireEvent.change(screen.getByLabelText("Package registry URL"), { target: { value: "https://registry.example.test" } });
    fireEvent.change(screen.getByLabelText("Package registry token"), { target: { value: "process-secret" } });
    fireEvent.click(screen.getByRole("button", { name: "Fetch and admit" }));

    await waitFor(() => expect(runtime.installRegistryPackage).toHaveBeenCalledWith({
      packageId: "owner/new-gene",
      version: "2.0.0",
      registryUrl: "https://registry.example.test",
      token: "process-secret",
    }));
    expect(screen.getByLabelText("Package registry token")).toHaveValue("");
    expect(await screen.findByRole("status")).toHaveTextContent("Restart the local service");

    fireEvent.click(screen.getByRole("button", { name: "Preview removal" }));
    expect(await screen.findByLabelText("Confirm removal example/refactor@1.2.3")).toBeInTheDocument();
    expect(runtime.previewPackageRemoval).toHaveBeenCalledWith("example/refactor", "1.2.3");
    const confirmation = screen.getByLabelText("Confirm removal example/refactor@1.2.3");
    fireEvent.change(confirmation, { target: { value: "example/refactor@1.2.3" } });
    fireEvent.click(screen.getByRole("button", { name: "Remove package" }));

    await waitFor(() => expect(runtime.removeLocalPackage).toHaveBeenCalledWith(
      "example/refactor",
      "1.2.3",
      "example/refactor@1.2.3",
    ));
  });

  it("clears a registry token when package installation fails", async () => {
    runtime.capabilities.mockResolvedValue([{
      id: "coding-domain",
      version: "1.2.0",
      name: "Coding Domain",
      kind: "domain",
      gene_count: 0,
      runnable: true,
      gene_ids: [],
    }]);
    runtime.installRegistryPackage.mockRejectedValue(new Error("registry refused the release"));

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Harness Lab" }));
    fireEvent.click(await screen.findByRole("tab", { name: "packages" }));
    await screen.findByRole("heading", { name: "Signed package manager" });
    fireEvent.change(screen.getByLabelText("Registry package ID"), { target: { value: "owner/bad-gene" } });
    fireEvent.change(screen.getByLabelText("Package registry URL"), { target: { value: "https://registry.example.test" } });
    fireEvent.change(screen.getByLabelText("Package registry token"), { target: { value: "discard-me" } });
    fireEvent.click(screen.getByRole("button", { name: "Fetch and admit" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("registry refused the release");
    expect(screen.getByLabelText("Package registry token")).toHaveValue("");
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
