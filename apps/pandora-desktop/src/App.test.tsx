import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";

const runtime = vi.hoisted(() => ({
  agentResume: vi.fn(),
  agentRun: vi.fn(),
  capabilities: vi.fn(),
  engines: vi.fn(),
  evolution: vi.fn(),
  events: vi.fn(),
  health: vi.fn(),
  inspectSession: vi.fn(),
  memory: vi.fn(),
  providers: vi.fn(),
  resolveApproval: vi.fn(),
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
  startLocalService: vi.fn(),
  stopLocalService: vi.fn(),
  RuntimeClient: class {
    agentResume = runtime.agentResume;
    agentRun = runtime.agentRun;
    capabilities = runtime.capabilities;
    engines = runtime.engines;
    evolution = runtime.evolution;
    events = runtime.events;
    health = runtime.health;
    inspectSession = runtime.inspectSession;
    memory = runtime.memory;
    providers = runtime.providers;
    resolveApproval = runtime.resolveApproval;
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
  runtime.tools.mockResolvedValue([]);
  runtime.providers.mockResolvedValue([]);
  runtime.inspectSession.mockResolvedValue({ session, event_count: 0 });
  runtime.events.mockResolvedValue([]);
  runtime.memory.mockResolvedValue([]);
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

  it("renders real governed evolution evidence without mutation controls", async () => {
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

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /Evolution/ }));

    expect(await screen.findByText("Improve verification reliability")).toBeInTheDocument();
    expect(screen.getByText("Passed · 95/96")).toBeInTheDocument();
    expect(screen.getByText("parliament-a · policy v1")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /approve|activate|mutate/i })).not.toBeInTheDocument();
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
});
