import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";

const runtime = vi.hoisted(() => ({
  capabilities: vi.fn(),
  engines: vi.fn(),
  events: vi.fn(),
  health: vi.fn(),
  inspectSession: vi.fn(),
  memory: vi.fn(),
  providers: vi.fn(),
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
    capabilities = runtime.capabilities;
    engines = runtime.engines;
    events = runtime.events;
    health = runtime.health;
    inspectSession = runtime.inspectSession;
    memory = runtime.memory;
    providers = runtime.providers;
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
    runtime.run.mockImplementation(
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
    expect(runtime.run).toHaveBeenCalledTimes(1);

    completeRun({
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
});
