import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import pandoraCss from "./pandora.css?raw";

const runtime = vi.hoisted(() => ({
  activateProvider: vi.fn(),
  admitLocalPackage: vi.fn(),
  activateEvolution: vi.fn(),
  agentResume: vi.fn(),
  agentRun: vi.fn(),
  capabilities: vi.fn(),
  configureMcp: vi.fn(),
  configureProvider: vi.fn(),
  configureRegistryProfile: vi.fn(),
  disableLocalPackage: vi.fn(),
  enableLocalPackage: vi.fn(),
  engines: vi.fn(),
  evolution: vi.fn(),
  evolutionActivations: vi.fn(),
  events: vi.fn(),
  health: vi.fn(),
  inspectEvolution: vi.fn(),
  inspectMemoryAudit: vi.fn(),
  inspectMemoryProvenance: vi.fn(),
  inspectOrchestration: vi.fn(),
  inspectSession: vi.fn(),
  installGitHubPackage: vi.fn(),
  installLocalSkill: vi.fn(),
  installRegistryPackage: vi.fn(),
  listLocalPackages: vi.fn(),
  listFleetOperations: vi.fn(),
  listPackageTransparency: vi.fn(),
  listLocalSkills: vi.fn(),
  listRegistryProfiles: vi.fn(),
  listStorageLifecycleEvidence: vi.fn(),
  lockLocalPackages: vi.fn(),
  compactMemory: vi.fn(),
  forgetMemory: vi.fn(),
  memory: vi.fn(),
  memorySchedules: vi.fn(),
  memoryScheduleRuns: vi.fn(),
  mutateLocalSkill: vi.fn(),
  createMemorySchedule: vi.fn(),
  disableMemorySchedule: vi.fn(),
  orchestrations: vi.fn(),
  providers: vi.fn(),
  previewPackageRemoval: vi.fn(),
  previewPackageDisable: vi.fn(),
  previewPackageEnable: vi.fn(),
  previewPackageRollback: vi.fn(),
  previewMemoryForget: vi.fn(),
  previewMemoryCompaction: vi.fn(),
  removeLocalPackage: vi.fn(),
  rollbackLocalPackage: vi.fn(),
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
  activateProvider: runtime.activateProvider,
  nativeEndpoint: "tauri://pandora",
  isNativeRuntime: () => true,
  loadRuntimeEndpoint: () => "tauri://pandora",
  saveRuntimeEndpoint: vi.fn(),
  configureMcp: runtime.configureMcp,
  configureProvider: runtime.configureProvider,
  configureRegistryProfile: runtime.configureRegistryProfile,
  disableLocalPackage: runtime.disableLocalPackage,
  enableLocalPackage: runtime.enableLocalPackage,
  admitLocalPackage: runtime.admitLocalPackage,
  installGitHubPackage: runtime.installGitHubPackage,
  installLocalSkill: runtime.installLocalSkill,
  installRegistryPackage: runtime.installRegistryPackage,
  listLocalPackages: runtime.listLocalPackages,
  listFleetOperations: runtime.listFleetOperations,
  listPackageTransparency: runtime.listPackageTransparency,
  listLocalSkills: runtime.listLocalSkills,
  listRegistryProfiles: runtime.listRegistryProfiles,
  listStorageLifecycleEvidence: runtime.listStorageLifecycleEvidence,
  lockLocalPackages: runtime.lockLocalPackages,
  compactMemory: runtime.compactMemory,
  mutateLocalSkill: runtime.mutateLocalSkill,
  inspectMemoryAudit: runtime.inspectMemoryAudit,
  inspectMemoryProvenance: runtime.inspectMemoryProvenance,
  forgetMemory: runtime.forgetMemory,
  previewPackageRemoval: runtime.previewPackageRemoval,
  previewPackageDisable: runtime.previewPackageDisable,
  previewPackageEnable: runtime.previewPackageEnable,
  previewPackageRollback: runtime.previewPackageRollback,
  previewMemoryForget: runtime.previewMemoryForget,
  previewMemoryCompaction: runtime.previewMemoryCompaction,
  removeLocalPackage: runtime.removeLocalPackage,
  rollbackLocalPackage: runtime.rollbackLocalPackage,
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
    memorySchedules = runtime.memorySchedules;
    memoryScheduleRuns = runtime.memoryScheduleRuns;
    createMemorySchedule = runtime.createMemorySchedule;
    disableMemorySchedule = runtime.disableMemorySchedule;
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

const runtimeComponent = (overrides: Record<string, unknown>) => ({
  id: "component",
  name: "Component",
  role: "Bounded role",
  authority: "No independent authority",
  category: "Tools and context",
  component_kind: "runtime_engine",
  inputs: ["Validated input"],
  outputs: ["Bounded output"],
  invariants: ["Cannot bypass ReferenceMonitor"],
  evidence: ["Runtime receipt"],
  source_modules: ["crates/pandora-runtime/src/component.rs"],
  related_components: ["ReferenceMonitor"],
  documentation: ["docs/WHY_PANDORA.md"],
  ...overrides,
});

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
  runtime.activateProvider.mockResolvedValue({ message: "Provider design selected.", restartRequired: true });
  runtime.configureMcp.mockResolvedValue({ message: "MCP server local-tools configured.", restartRequired: true });
  runtime.configureRegistryProfile.mockResolvedValue({ message: "Registry m-place configured.", restartRequired: false });
  runtime.installGitHubPackage.mockResolvedValue({
    message: "Package admitted from the pinned GitHub source.",
    restartRequired: true,
    data: {},
  });
  runtime.listLocalPackages.mockResolvedValue({
    message: "0 local package(s) available.",
    restartRequired: false,
    data: { packages: [] },
  });
  runtime.listPackageTransparency.mockResolvedValue({
    message: "Loaded 0 append-only package transparency event(s).",
    restartRequired: false,
    data: {
      events: [],
      count: 0,
      durability: "append-only-sqlite",
      integrity: "sha256-event-chain",
      runtime_authority: false,
    },
  });
  runtime.listFleetOperations.mockResolvedValue({
    message: "Local Fleet operations are idle.",
    data: {
      generated_at: 1,
      health: {
        status: "idle",
        ready_nodes: 0,
        running_supervisors: 0,
        stale_supervisors: 0,
        overdue_active_leases: 0,
        queued_without_capacity: false,
        aggregate_budget_invariant_holds: true,
      },
      fleet: {
        nodes: { total: 0, by_state: { ready: 0, quarantined: 0, revoked: 0, killed: 0 } },
        supervisors: { total: 0, by_state: { stopped: 0, running: 0, draining: 0, recovering: 0 }, stale: [] },
        leases: { total: 0, by_state: { active: 0, released: 0, expired: 0, revoked: 0, killed: 0 }, active: [], active_details_truncated: false },
      },
      queue: {
        jobs: { total: 0, by_status: {}, queued: 0, running: 0, failure_count: 0 },
        orchestrations: { total: 0, by_status: {}, queued: 0, running: 0, failure_count: 0 },
      },
      failures: { count: 0, records: [], records_truncated: false },
      budget_ceilings: { active_lease_count: 0, max_tokens: 0, max_tools: 0, max_duration_seconds: 0, max_cost_micros: 0, saturated: false, actual_spend_available: false },
      aggregate_budgets: {
        run_count: 0,
        records: [],
        records_truncated: false,
        ceiling: { tokens: 0, tools: 0, elapsed_ms: 0, cost_micros: 0 },
        reserved: { tokens: 0, tools: 0, elapsed_ms: 0, cost_micros: 0 },
        consumed: { tokens: 0, tools: 0, elapsed_ms: 0, cost_micros: 0, known_cost_micros: 0, unknown_cost_receipts: 0, enforced_cost_micros: 0 },
        remaining: { tokens: 0, tools: 0, elapsed_ms: 0, cost_micros: 0, enforced_cost_micros: 0 },
        saturated: false,
        invariant: { holds: true, expression: "enforced_consumed + active_reservations <= aggregate_ceiling" },
      },
      boundary: { read_only: true, runtime_authority: false, budgets_are_ceilings_not_spend: true, aggregate_usage_available: false, aggregate_cost_unknown_explicit: true, prompts_included: false, outputs_included: false, credentials_included: false, hidden_reasoning_included: false },
    },
  });
  runtime.listLocalSkills.mockResolvedValue({
    message: "Loaded local Skills.",
    restartRequired: false,
    data: { skills: [] },
  });
  runtime.installLocalSkill.mockResolvedValue({
    message: "Skill installed disabled.",
    restartRequired: true,
    data: {},
  });
  runtime.mutateLocalSkill.mockResolvedValue({
    message: "Skill lifecycle updated.",
    restartRequired: true,
    data: {},
  });
  runtime.listRegistryProfiles.mockResolvedValue({
    message: "0 registry profile(s) configured.",
    data: { registries: [] },
  });
  runtime.lockLocalPackages.mockResolvedValue({
    message: "Deterministic package lock written for the current workspace.",
    restartRequired: false,
    data: { package_count: 0 },
  });
  runtime.inspectSession.mockResolvedValue({ session, event_count: 0 });
  runtime.events.mockResolvedValue([]);
  runtime.memory.mockResolvedValue([]);
  runtime.inspectMemoryAudit.mockResolvedValue({
    message: "Loaded 0 durable memory audit record(s).",
    data: { entries: [], count: 0 },
  });
  runtime.inspectMemoryProvenance.mockResolvedValue({
    message: "Loaded bounded provenance.",
    data: { root_id: "", nodes: [], edges: [], bounded: true, max_nodes: 64 },
  });
  runtime.previewMemoryForget.mockResolvedValue({
    message: "Previewed durable revocation; no memory changed.",
    data: { dry_run: true, memory_id: "memory", would_revoke: true },
  });
  runtime.forgetMemory.mockResolvedValue({
    message: "Memory revoked with a durable tombstone.",
    data: { dry_run: false, memory_id: "memory", revoked: true },
  });
  runtime.previewMemoryCompaction.mockResolvedValue({
    message: "Previewed 0 revoked logical memory record(s); no records changed.",
    data: {
      dry_run: true,
      revoked_before_or_at: 4102444800,
      compactable_records: 0,
      would_compact: false,
      boundary: {
        tombstones_retained: true,
        audit_retained: true,
        secure_erasure_guaranteed: false,
        storage_guidance: "Database pages, WAL files, backups, and storage snapshots require separate lifecycle controls.",
      },
    },
  });
  runtime.compactMemory.mockResolvedValue({
    message: "Compacted 0 revoked logical memory record(s); tombstones and audit evidence remain.",
    data: {
      dry_run: false,
      revoked_before_or_at: 4102444800,
      compacted_records: 0,
      boundary: {
        tombstones_retained: true,
        audit_retained: true,
        secure_erasure_guaranteed: false,
        storage_guidance: "Database pages, WAL files, backups, and storage snapshots require separate lifecycle controls.",
      },
    },
  });
  runtime.listStorageLifecycleEvidence.mockResolvedValue({
    message: "Loaded 0 append-only storage lifecycle receipt(s).",
    data: {
      receipts: [],
      count: 0,
      boundary: {
        evidence_status: "operator_attested",
        external_action_performed_by_runtime: false,
        secure_erasure_guaranteed: false,
        runtime_deletes_provider_resources: false,
        verification_responsibility: "operator",
        guidance: "Independently verify provider actions before recording evidence.",
      },
    },
  });
  runtime.memorySchedules.mockResolvedValue([]);
  runtime.memoryScheduleRuns.mockResolvedValue([]);
  runtime.createMemorySchedule.mockResolvedValue({
    id: "memory-schedule-1",
    name: "Nightly lessons",
    session_id: session.session_id,
    provider: "local",
    memory_id: "lesson-release-review",
    kind: "lesson",
    summary: "Distill release evidence",
    classification: "internal",
    interval_seconds: 86400,
    next_run_at: 1,
    enabled: true,
    created_at: 1,
    last_claimed_at: null,
    run_count: 0,
    scope: { principal_id: "principal-1", tenant_id: "tenant-1", workspace_id: "workspace-1" },
  });
  runtime.disableMemorySchedule.mockResolvedValue({
    id: "memory-schedule-1",
    name: "Nightly lessons",
    session_id: session.session_id,
    provider: "local",
    memory_id: "lesson-release-review",
    kind: "lesson",
    summary: "Distill release evidence",
    classification: "internal",
    interval_seconds: 86400,
    next_run_at: 1,
    enabled: false,
    created_at: 1,
    last_claimed_at: null,
    run_count: 0,
    scope: { principal_id: "principal-1", tenant_id: "tenant-1", workspace_id: "workspace-1" },
  });
  runtime.orchestrations.mockResolvedValue([]);
});

afterEach(() => cleanup());

describe("Pandora desktop run state", () => {
  it("moves focus into the selected workspace after command-palette navigation", async () => {
    render(<App />);

    const trigger = await screen.findByRole("button", { name: /^Search$/ });
    trigger.focus();
    fireEvent.click(trigger);
    const search = screen.getByRole("combobox", { name: "Search Pandora surfaces" });
    expect(search).toHaveAttribute("aria-activedescendant", "pandora-palette-option-0");

    fireEvent.keyDown(search, { key: "ArrowDown" });
    expect(search).toHaveAttribute("aria-activedescendant", "pandora-palette-option-1");
    expect(screen.getByRole("option", { name: /Background Runs/ })).toHaveAttribute("aria-selected", "true");
    fireEvent.keyDown(search, { key: "Enter" });

    expect(await screen.findByRole("heading", { name: "Background Runs" })).toBeInTheDocument();
    expect(screen.getByRole("main", { name: "Background Runs workspace" })).toHaveFocus();
  });

  it("traps command-palette focus and restores the invoking control when dismissed", async () => {
    render(<App />);

    const trigger = await screen.findByRole("button", { name: /^Search$/ });
    trigger.focus();
    fireEvent.click(trigger);
    const search = screen.getByRole("combobox", { name: "Search Pandora surfaces" });
    const close = screen.getByRole("button", { name: "Close quick open" });
    await waitFor(() => expect(search).toHaveFocus());

    fireEvent.keyDown(search, { key: "Tab", shiftKey: true });
    expect(close).toHaveFocus();
    fireEvent.keyDown(close, { key: "Tab" });
    expect(search).toHaveFocus();
    fireEvent.click(close);

    await waitFor(() => expect(trigger).toHaveFocus());
  });

  it("exposes skip navigation and one-tab-stop keyboard tablists", async () => {
    render(<App />);

    const skipLink = screen.getByRole("link", { name: "Skip to workspace" });
    expect(skipLink).toHaveAttribute("href", "#pandora-main");
    expect(await screen.findAllByRole("button", { name: "Connections" })).toHaveLength(1);

    const tabList = screen.getByRole("tablist", { name: "Run inspector" });
    const flow = within(tabList).getByRole("tab", { name: "flow" });
    const evidence = within(tabList).getByRole("tab", { name: "evidence" });
    expect(flow).toHaveAttribute("tabindex", "0");
    expect(evidence).toHaveAttribute("tabindex", "-1");

    flow.focus();
    fireEvent.keyDown(flow, { key: "ArrowRight" });
    await waitFor(() => expect(evidence).toHaveFocus());
    expect(evidence).toHaveAttribute("aria-selected", "true");
    expect(flow).toHaveAttribute("tabindex", "-1");
    expect(screen.getByRole("tabpanel", { name: "evidence" })).toBeInTheDocument();
  });

  it("moves, resizes, hides, and restores the witness dock without changing runtime state", async () => {
    render(<App />);

    const inspector = await screen.findByRole("complementary", { name: "Workspace inspector" });
    const workspace = screen.getByRole("main", { name: "Command Center workspace" });
    const layout = workspace.querySelector(".command-layout");
    expect(inspector).toBeInTheDocument();
    expect(layout).toHaveAttribute("data-dock-placement", "right");

    fireEvent.click(screen.getByRole("button", { name: "Inspector options" }));
    const options = screen.getByRole("group", { name: "Inspector layout options" });
    fireEvent.click(within(options).getByRole("button", { name: "Bottom" }));
    fireEvent.click(within(options).getByRole("button", { name: "expanded" }));
    expect(layout).toHaveAttribute("data-dock-placement", "bottom");
    expect(layout).toHaveClass("dock-size-expanded");

    fireEvent.click(within(options).getByRole("button", { name: "Hide inspector" }));
    expect(screen.queryByRole("complementary", { name: "Workspace inspector" })).not.toBeInTheDocument();
    const restore = screen.getByRole("button", { name: "Show workspace inspector" });
    expect(layout).toHaveAttribute("data-dock-placement", "closed");
    await waitFor(() => expect(restore).toHaveFocus());

    fireEvent.click(restore);
    expect(await screen.findByRole("complementary", { name: "Workspace inspector" })).toBeInTheDocument();
    expect(layout).toHaveAttribute("data-dock-placement", "bottom");
  });

  it("searches grouped settings and persists workspace layout controls", async () => {
    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: "Open settings" }));
    expect(screen.getByRole("heading", { name: "Settings" })).toBeInTheDocument();
    expect(screen.getByRole("complementary", { name: "Settings sections" })).toBeInTheDocument();

    const search = screen.getByRole("textbox", { name: "Search settings" });
    fireEvent.change(search, { target: { value: "dock" } });
    expect(screen.getByRole("button", { name: /Workspace Inspector placement and density/ })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /General Local app and runtime posture/ })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Workspace Inspector placement and density/ }));
    fireEvent.click(within(screen.getByRole("group", { name: "Inspector placement" })).getByRole("button", { name: "Bottom" }));
    fireEvent.click(within(screen.getByRole("group", { name: "Inspector size" })).getByRole("button", { name: "expanded" }));
    await waitFor(() => {
      expect(window.localStorage.getItem("pandora.desktop.dock.placement")).toBe("bottom");
      expect(window.localStorage.getItem("pandora.desktop.dock.size")).toBe("expanded");
    });

    fireEvent.click(screen.getByRole("button", { name: "Reset layout defaults" }));
    await waitFor(() => {
      expect(window.localStorage.getItem("pandora.desktop.dock.open")).toBe("true");
      expect(window.localStorage.getItem("pandora.desktop.dock.placement")).toBe("right");
      expect(window.localStorage.getItem("pandora.desktop.dock.size")).toBe("comfortable");
    });

    fireEvent.click(screen.getByRole("button", { name: "Command" }));
    const layout = screen.getByRole("main", { name: "Command Center workspace" }).querySelector(".command-layout");
    expect(layout).toHaveAttribute("data-dock-placement", "right");
    expect(layout).toHaveClass("dock-size-comfortable");
  });

  it("previews and restores validated appearance selections", async () => {
    const first = render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Open settings" }));
    fireEvent.click(screen.getByRole("button", { name: /Appearance Theme and visual behavior/ }));

    fireEvent.click(within(screen.getByRole("group", { name: "Theme mode" })).getByRole("button", { name: /Dark/ }));
    fireEvent.click(within(screen.getByRole("group", { name: "Theme accent" })).getByRole("button", { name: "Cyan" }));
    fireEvent.click(within(screen.getByRole("group", { name: "Theme preset" })).getByRole("button", { name: /Verdant/ }));

    expect(screen.getByRole("heading", { name: "Representative controls and states" })).toBeInTheDocument();
    expect(screen.getByText("Effect remains unresolved")).toBeInTheDocument();
    await waitFor(() => {
      expect(document.documentElement).toHaveAttribute("data-theme", "dark");
      expect(document.documentElement).toHaveAttribute("data-accent", "cyan");
      expect(document.documentElement).toHaveAttribute("data-theme-preset", "verdant");
      expect(JSON.parse(window.localStorage.getItem("pandora.desktop.appearance.v1") ?? "{}")).toEqual({
        mode: "dark",
        accent: "cyan",
        preset: "verdant",
      });
    });

    first.unmount();
    render(<App />);
    await waitFor(() => {
      expect(document.documentElement).toHaveAttribute("data-theme", "dark");
      expect(document.documentElement).toHaveAttribute("data-accent", "cyan");
      expect(document.documentElement).toHaveAttribute("data-theme-preset", "verdant");
    });
  });

  it("fails closed to the built-in appearance when persisted data is incomplete", async () => {
    window.localStorage.setItem("pandora.desktop.appearance.v1", JSON.stringify({
      mode: "dark",
      accent: "cyan",
    }));
    render(<App />);

    await waitFor(() => {
      expect(document.documentElement).toHaveAttribute("data-theme-mode", "dark");
      expect(document.documentElement).toHaveAttribute("data-accent", "ember");
      expect(document.documentElement).toHaveAttribute("data-theme-preset", "foundry");
    });
  });

  it("keeps the local companion off by default and persists only display controls", async () => {
    const first = render(<App />);
    expect(screen.queryByRole("complementary", { name: "Pandora companion" })).not.toBeInTheDocument();

    fireEvent.click(await screen.findByRole("button", { name: "Open settings" }));
    fireEvent.click(screen.getByRole("button", { name: /Appearance Theme and visual behavior/ }));
    fireEvent.click(within(screen.getByRole("group", { name: "Companion visibility" })).getByRole("button", { name: "On" }));
    fireEvent.click(within(screen.getByRole("group", { name: "Companion position" })).getByRole("button", { name: "bottom left" }));
    fireEvent.click(within(screen.getByRole("group", { name: "Companion scale" })).getByRole("button", { name: "large" }));
    fireEvent.click(within(screen.getByRole("group", { name: "Companion motion" })).getByRole("button", { name: "static" }));
    fireEvent.click(within(screen.getByRole("group", { name: "Companion preview state" })).getByRole("button", { name: "waiting" }));

    expect(screen.getByText("Waiting for an exact approval")).toBeInTheDocument();
    expect(screen.getByRole("complementary", { name: "Pandora companion" })).toHaveClass("position-bottom-left", "scale-large", "motion-static");
    expect(runtime.run).not.toHaveBeenCalled();
    expect(runtime.agentRun).not.toHaveBeenCalled();
    expect(runtime.resolveApproval).not.toHaveBeenCalled();
    await waitFor(() => expect(JSON.parse(window.localStorage.getItem("pandora.desktop.companion.v1") ?? "{}")).toEqual({
      enabled: true,
      position: "bottom-left",
      scale: "large",
      motion: "static",
    }));

    first.unmount();
    render(<App />);
    const restored = await screen.findByRole("complementary", { name: "Pandora companion" });
    expect(restored).toHaveClass("position-bottom-left", "scale-large", "motion-static");
    fireEvent.click(within(restored).getByRole("button", { name: "Disable Pandora companion" }));
    expect(screen.queryByRole("complementary", { name: "Pandora companion" })).not.toBeInTheDocument();
    await waitFor(() => expect(JSON.parse(window.localStorage.getItem("pandora.desktop.companion.v1") ?? "{}").enabled).toBe(false));
  });

  it("recovers invalid persisted layout values without disrupting runtime work", async () => {
    window.localStorage.setItem("pandora.desktop.dock.open", "not-a-boolean");
    window.localStorage.setItem("pandora.desktop.dock.placement", "left");
    window.localStorage.setItem("pandora.desktop.dock.size", "giant");
    runtime.agentRun.mockResolvedValue({
      mode: "agent",
      session_id: "session-layout-recovery",
      execution_id: "execution-layout-recovery",
      selected_harness: "coding-domain",
      selected_gene: null,
      status: "completed",
      status_detail: "Layout recovery left the governed run intact.",
      output: "runtime work preserved",
      receipt_count: 0,
      event_count: 0,
    });

    render(<App />);

    const layout = (await screen.findByRole("main", { name: "Command Center workspace" })).querySelector(".command-layout");
    expect(layout).toHaveAttribute("data-dock-placement", "right");
    expect(layout).toHaveClass("dock-size-comfortable");
    const composer = screen.getByLabelText("Pandora task");
    fireEvent.change(composer, { target: { value: "verify recovered layout" } });
    fireEvent.submit(composer.closest("form")!);

    expect(await screen.findByText("runtime work preserved")).toBeInTheDocument();
    await waitFor(() => {
      expect(window.localStorage.getItem("pandora.desktop.dock.open")).toBe("true");
      expect(window.localStorage.getItem("pandora.desktop.dock.placement")).toBe("right");
      expect(window.localStorage.getItem("pandora.desktop.dock.size")).toBe("comfortable");
    });
  });

  it("keeps exact approval controls visible and unresolved while the inspector is hidden", async () => {
    const approval = {
      approval_id: "approval-hidden-dock",
      session_id: "session-hidden-dock",
      execution_id: "execution-hidden-dock",
      gene_id: "coding.patch",
      request_digest: "pandora-request-v2:sha256:hidden-dock",
      request_summary: "filesystem.write for patch on README.md",
      status: "pending",
      created_at_unix_seconds: 1,
    };
    runtime.agentRun.mockResolvedValue({
      mode: "agent",
      session_id: approval.session_id,
      execution_id: approval.execution_id,
      selected_harness: "coding-domain",
      selected_gene: approval.gene_id,
      status: "approval_required",
      status_detail: "explicit approval is required",
      output: "",
      receipt_count: 0,
      event_count: 1,
      approval,
    });
    runtime.resolveApproval.mockResolvedValue({ ...approval, status: "denied", approver_id: "local-operator" });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Inspector options" }));
    fireEvent.click(within(screen.getByRole("group", { name: "Inspector layout options" })).getByRole("button", { name: "Hide inspector" }));
    const composer = screen.getByLabelText("Pandora task");
    fireEvent.change(composer, { target: { value: "patch README" } });
    fireEvent.submit(composer.closest("form")!);

    const pending = await screen.findByRole("region", { name: "Pending approval" });
    expect(screen.queryByRole("complementary", { name: "Workspace inspector" })).not.toBeInTheDocument();
    expect(within(pending).getByText(approval.request_summary)).toBeInTheDocument();
    expect(within(pending).getByText(approval.request_digest)).toBeInTheDocument();
    expect(runtime.resolveApproval).not.toHaveBeenCalled();

    fireEvent.click(within(pending).getByRole("button", { name: "Deny" }));
    await waitFor(() => expect(runtime.resolveApproval).toHaveBeenCalledWith(approval.approval_id, false));
    expect(runtime.agentResume).not.toHaveBeenCalled();
  });

  it("ships reduced-motion, scalable-type, and native high-contrast contracts", () => {
    expect(pandoraCss).toContain("@media (prefers-reduced-motion: reduce)");
    expect(pandoraCss).toContain("@media (prefers-contrast: more)");
    expect(pandoraCss).toContain("@media (forced-colors: active)");
    expect(pandoraCss).toContain("@media (prefers-reduced-transparency: reduce)");
    expect(pandoraCss).toContain(':root[data-platform="macos"] .sidebar');
    expect(pandoraCss).toContain(':root[data-platform="macos"] .main-shell');
    expect(pandoraCss).toContain(".skip-link:focus");
    expect(pandoraCss).not.toMatch(/font-size:\s*[6-9]px/);
  });

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

  it("routes a run through the selected configured provider and model", async () => {
    runtime.providers.mockResolvedValue([
      {
        name: "primary",
        model: "model-a",
        protocol: "openai-compatible",
        active: true,
        credential_configured: true,
        fallback_provider: null,
      },
      {
        name: "local-fast",
        model: "model-b",
        protocol: "openai-compatible",
        active: false,
        credential_configured: true,
        fallback_provider: null,
      },
      {
        name: "missing-credential",
        model: "model-c",
        protocol: "openai-compatible",
        active: false,
        credential_configured: false,
        fallback_provider: null,
      },
    ]);
    runtime.agentRun.mockResolvedValue({
      mode: "agent",
      session_id: session.session_id,
      execution_id: "execution-provider-selection",
      selected_harness: "coding-domain",
      selected_gene: null,
      status: "completed",
      output: "selected provider used",
      receipt_count: 0,
      event_count: 1,
    });

    render(<App />);

    const provider = await screen.findByLabelText("Model provider");
    const model = screen.getByLabelText("Model");
    expect(within(provider).queryByRole("option", { name: "missing-credential" })).not.toBeInTheDocument();
    await waitFor(() => expect(model).toHaveValue("model-a"));
    fireEvent.change(provider, { target: { value: "local-fast" } });
    expect(model).toHaveValue("model-b");
    fireEvent.change(model, { target: { value: "model-b-preview" } });

    const composer = screen.getByLabelText("Pandora task");
    fireEvent.change(composer, { target: { value: "Use the selected model" } });
    fireEvent.submit(composer.closest("form")!);

    await waitFor(() => expect(runtime.agentRun).toHaveBeenCalledWith(
      "Use the selected model",
      null,
      null,
      [],
      "local-fast",
      "model-b-preview",
    ));
    expect(await screen.findByText("selected provider used")).toBeInTheDocument();
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
    expect(screen.getByRole("button", { name: "Retry request" })).toBeEnabled();
    expect(screen.getByText(/task and selected context remain editable/i)).toBeInTheDocument();
    expect(screen.getAllByText("Runtime connected").length).toBeGreaterThan(0);
  });

  it("retries a preserved transport failure as a fresh governed request", async () => {
    runtime.agentRun
      .mockRejectedValueOnce(new Error("provider timed out"))
      .mockResolvedValueOnce({
        mode: "agent",
        session_id: session.session_id,
        execution_id: "execution-recovered",
        selected_harness: "coding-domain",
        selected_gene: null,
        status: "completed",
        output: "recovered output",
        receipt_count: 1,
        event_count: 2,
      });

    render(<App />);
    const composer = await screen.findByLabelText("Pandora task");
    fireEvent.change(composer, { target: { value: "Recover this exact task" } });
    fireEvent.submit(composer.closest("form")!);
    fireEvent.click(await screen.findByRole("button", { name: "Retry request" }));

    await waitFor(() => expect(runtime.agentRun).toHaveBeenCalledTimes(2));
    expect(runtime.agentRun).toHaveBeenNthCalledWith(2, "Recover this exact task", null, null, [], null, null);
    expect(await screen.findByText("recovered output")).toBeInTheDocument();
    expect(composer).toHaveValue("");
    expect(screen.queryByText("provider timed out")).not.toBeInTheDocument();
  });

  it("retries a recorded failed run without reusing its permits", async () => {
    runtime.agentRun
      .mockResolvedValueOnce({
        mode: "agent",
        session_id: session.session_id,
        execution_id: "execution-failed",
        selected_harness: "coding-domain",
        selected_gene: "workspace.test",
        status: "failed",
        status_detail: "Verification failed before completion",
        output: "tests failed",
        receipt_count: 1,
        event_count: 4,
      })
      .mockResolvedValueOnce({
        mode: "agent",
        session_id: session.session_id,
        execution_id: "execution-retry",
        selected_harness: "coding-domain",
        selected_gene: "workspace.test",
        status: "completed",
        output: "tests passed",
        receipt_count: 2,
        event_count: 7,
      });

    render(<App />);
    const composer = await screen.findByLabelText("Pandora task");
    fireEvent.change(composer, { target: { value: "Run the verification suite" } });
    fireEvent.submit(composer.closest("form")!);

    expect((await screen.findAllByText("Verification failed before completion")).length).toBeGreaterThan(0);
    expect(screen.getByText(/new execution and re-runs every policy, evaluation, and permit check/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Retry with fresh verification/ }));

    await waitFor(() => expect(runtime.agentRun).toHaveBeenCalledTimes(2));
    expect(runtime.agentRun).toHaveBeenNthCalledWith(2, "Run the verification suite", session.session_id, null, [], null, null);
    expect(await screen.findByText("tests passed")).toBeInTheDocument();
    expect(screen.getAllByText("execution-retry").length).toBeGreaterThan(0);
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
        null,
        null,
      );
    });
    expect(screen.queryByText("notes.ts")).not.toBeInTheDocument();
  });

  it("inspects and creates a scoped memory synthesis schedule", async () => {
    runtime.sessions.mockResolvedValue([session]);
    runtime.memorySchedules.mockResolvedValue([{
      id: "memory-schedule-1",
      name: "Nightly lessons",
      session_id: session.session_id,
      provider: "local",
      memory_id: "lesson-release-review",
      kind: "lesson",
      summary: "Distill release evidence",
      classification: "internal",
      interval_seconds: 86400,
      next_run_at: 1,
      enabled: true,
      created_at: 1,
      last_claimed_at: null,
      run_count: 0,
      scope: { principal_id: "principal-1", tenant_id: "tenant-1", workspace_id: "workspace-1" },
    }]);
    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: "Connections" }));
    const sessionButtons = await screen.findAllByRole("button", { name: /session-1/ });
    fireEvent.click(sessionButtons[sessionButtons.length - 1]);
    await waitFor(() => expect(runtime.inspectSession).toHaveBeenCalledWith("session-1"));
    fireEvent.click(await screen.findByRole("button", { name: "Memory" }));
    expect(await screen.findByRole("heading", { name: "Synthesis schedules" })).toBeInTheDocument();
    expect(screen.getByText("Nightly lessons")).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("Schedule name"), { target: { value: "Release lessons" } });
    fireEvent.change(screen.getByLabelText("Memory ID"), { target: { value: "lesson-release" } });
    fireEvent.change(screen.getByLabelText("Synthesis summary"), { target: { value: "Distill release evidence" } });
    fireEvent.click(screen.getByRole("button", { name: "Add schedule" }));

    await waitFor(() => expect(runtime.createMemorySchedule).toHaveBeenCalledWith(expect.objectContaining({
      name: "Release lessons",
      session_id: session.session_id,
      memory_id: "lesson-release",
      kind: "lesson",
      classification: "internal",
      interval_seconds: 86400,
    })));
  });


  it("inspects bounded memory provenance and requires exact revocation confirmation", async () => {
    const memoryRecord = {
      memory_id: "lesson-release-review",
      tier: "l1",
      kind: "lesson",
      summary: "Use the signed release evidence set",
      classification: "internal",
      created_at_unix_seconds: 10,
      provenance: "synthesized",
      origin: "synthesized",
      evidence_count: 1,
    };
    runtime.sessions.mockResolvedValue([session]);
    runtime.memory.mockResolvedValueOnce([memoryRecord]).mockResolvedValueOnce([]);
    runtime.inspectMemoryProvenance.mockResolvedValue({
      message: "Loaded bounded provenance for lesson-release-review.",
      data: {
        root_id: "lesson-release-review",
        bounded: true,
        max_nodes: 64,
        nodes: [{
          id: "lesson-release-review",
          tier: "l1",
          kind: "lesson",
          summary: "Use the signed release evidence set",
          classification: "internal",
          created_at: 10,
          expires_at: null,
          provenance: "synthesized",
          origin: "synthesized",
          evidence_ids: ["receipt-release"],
          approval: null,
        }, {
          id: "receipt-release",
          tier: "l1",
          kind: "execution_evidence",
          summary: "Release workflow completed",
          classification: "internal",
          created_at: 9,
          expires_at: null,
          provenance: "runtime",
          origin: "runtime",
          evidence_ids: [],
          approval: null,
        }],
        edges: [{ from: "lesson-release-review", to: "receipt-release" }],
      },
    });
    runtime.inspectMemoryAudit.mockResolvedValue({
      message: "Loaded 2 durable memory audit record(s).",
      data: {
        count: 2,
        entries: [
          { memory_id: "lesson-release-review", tier: "l1", action: "added", at: 10, approval_id: null },
          { memory_id: "lesson-release-review", tier: "l1", action: "revoked", at: 11, approval_id: null },
        ],
      },
    });
    runtime.previewMemoryForget.mockResolvedValue({
      message: "Previewed durable revocation for lesson-release-review; no memory changed.",
      data: {
        dry_run: true,
        memory_id: "lesson-release-review",
        would_revoke: true,
      },
    });
    runtime.forgetMemory.mockResolvedValue({
      message: "Memory lesson-release-review revoked with a durable tombstone.",
      data: {
        dry_run: false,
        memory_id: "lesson-release-review",
        revoked: true,
      },
    });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Connections" }));
    const sessionButtons = await screen.findAllByRole("button", { name: /session-1/ });
    fireEvent.click(sessionButtons[sessionButtons.length - 1]);
    await waitFor(() => expect(runtime.memory).toHaveBeenCalledWith(session.session_id));

    fireEvent.click(screen.getByRole("button", { name: "Memory" }));
    fireEvent.click(await screen.findByRole("button", { name: "Inspect provenance" }));
    await waitFor(() => expect(runtime.inspectMemoryProvenance).toHaveBeenCalledWith(
      session.session_id,
      "local",
      memoryRecord.memory_id,
    ));
    expect(await screen.findByLabelText("Memory provenance")).toHaveTextContent("2 nodes · 1 edges · bounded to 64");
    expect(screen.getByText("Release workflow completed")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Load audit" }));
    await waitFor(() => expect(runtime.inspectMemoryAudit).toHaveBeenCalledWith(session.session_id, "local"));
    expect(await screen.findByLabelText("Memory audit trail")).toHaveTextContent("revoked");

    fireEvent.click(screen.getByRole("button", { name: `Forget memory ${memoryRecord.memory_id}` }));
    await waitFor(() => expect(runtime.previewMemoryForget).toHaveBeenCalledWith(
      session.session_id,
      "local",
      memoryRecord.memory_id,
    ));
    const confirmation = await screen.findByLabelText(`Confirm forget ${memoryRecord.memory_id}`);
    const revoke = screen.getByRole("button", { name: "Confirm revocation" });
    expect(revoke).toBeDisabled();
    fireEvent.change(confirmation, { target: { value: memoryRecord.memory_id } });
    expect(revoke).toBeEnabled();
    fireEvent.click(revoke);

    await waitFor(() => expect(runtime.forgetMemory).toHaveBeenCalledWith(
      session.session_id,
      "local",
      memoryRecord.memory_id,
      memoryRecord.memory_id,
    ));
    expect(await screen.findByText("Memory lesson-release-review revoked with a durable tombstone.")).toBeInTheDocument();
    await waitFor(() => expect(runtime.memory).toHaveBeenCalledTimes(2));
  });

  it("previews and exactly confirms scoped logical memory compaction", async () => {
    runtime.sessions.mockResolvedValue([session]);
    runtime.previewMemoryCompaction.mockResolvedValue({
      message: "Previewed 2 revoked logical memory record(s) at or before 4102444800; no records changed.",
      data: {
        dry_run: true,
        revoked_before_or_at: 4102444800,
        compactable_records: 2,
        would_compact: true,
        boundary: {
          tombstones_retained: true,
          audit_retained: true,
          secure_erasure_guaranteed: false,
          storage_guidance: "Database pages, WAL files, backups, and storage snapshots require separate lifecycle controls.",
        },
      },
    });
    runtime.compactMemory.mockResolvedValue({
      message: "Compacted 2 revoked logical memory record(s); tombstones and audit evidence remain.",
      data: {
        dry_run: false,
        revoked_before_or_at: 4102444800,
        compacted_records: 2,
        boundary: {
          tombstones_retained: true,
          audit_retained: true,
          secure_erasure_guaranteed: false,
          storage_guidance: "Database pages, WAL files, backups, and storage snapshots require separate lifecycle controls.",
        },
      },
    });
    runtime.listStorageLifecycleEvidence.mockResolvedValue({
      message: "Loaded 1 append-only storage lifecycle receipt(s).",
      data: {
        receipts: [{
          manifest: {
            policy_version: 1,
            evidence_id: "evidence:backup-1",
            policy_id: "retention:daily-30d",
            provider: "aws_s3",
            action: "backup_expired",
            resource_id: "resource:backup-1",
            provider_fields: {
              bucket: "backup-bucket",
              deletion_marker_id: "marker-1",
              object_key: "daily/archive.json",
              version_id: "version-1",
            },
            external_evidence_digest: `sha256:${"1".repeat(64)}`,
            actor: "operator:alice",
            performed_at: 1_788_192_000,
            manifest_digest: `sha256:${"2".repeat(64)}`,
          },
          manifest_digest: `sha256:${"2".repeat(64)}`,
          recorded_at: 1_788_192_100,
          evidence_status: "operator_attested",
          external_action_performed_by_runtime: false,
          secure_erasure_guaranteed: false,
          durability: "append-only-sqlite",
        }],
        count: 1,
        boundary: {
          evidence_status: "operator_attested",
          external_action_performed_by_runtime: false,
          secure_erasure_guaranteed: false,
          runtime_deletes_provider_resources: false,
          verification_responsibility: "operator",
          guidance: "Independently verify provider actions before recording evidence.",
        },
      },
    });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Connections" }));
    const sessionButtons = await screen.findAllByRole("button", { name: /session-1/ });
    fireEvent.click(sessionButtons[sessionButtons.length - 1]);
    await waitFor(() => expect(runtime.inspectSession).toHaveBeenCalledWith("session-1"));
    fireEvent.click(await screen.findByRole("button", { name: "Memory" }));
    expect(screen.getByLabelText("Cross-project memory policy")).toHaveTextContent("denied by default");
    expect(screen.getByLabelText("Cross-project memory policy")).toHaveTextContent("never overwrites");
    expect(screen.getByText(/runtime records evidence; it does not delete provider resources/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Refresh lifecycle evidence" }));
    await waitFor(() => expect(runtime.listStorageLifecycleEvidence).toHaveBeenCalledTimes(1));
    expect(await screen.findByText("evidence:backup-1")).toBeInTheDocument();
    expect(screen.getByLabelText("Storage lifecycle evidence")).toHaveTextContent("backup expired");

    const boundary = await screen.findByLabelText("Revoked before Unix seconds");
    fireEvent.change(boundary, { target: { value: "4102444800" } });
    fireEvent.click(screen.getByRole("button", { name: "Preview compaction" }));
    await waitFor(() => expect(runtime.previewMemoryCompaction).toHaveBeenCalledWith(
      session.session_id,
      "local",
      4102444800,
    ));
    expect(await screen.findByText("2 revoked record(s) eligible")).toBeInTheDocument();
    expect(screen.getByText(/does not securely erase SQLite pages/)).toBeInTheDocument();

    const confirmation = screen.getByLabelText("Confirm memory compaction");
    const apply = screen.getByRole("button", { name: "Confirm logical compaction" });
    expect(apply).toBeDisabled();
    fireEvent.change(confirmation, { target: { value: "COMPACT 4102444800" } });
    expect(apply).toBeEnabled();
    fireEvent.click(apply);

    await waitFor(() => expect(runtime.compactMemory).toHaveBeenCalledWith(
      session.session_id,
      "local",
      4102444800,
      "COMPACT 4102444800",
    ));
    expect(await screen.findByText("Compacted 2 revoked logical memory record(s); tombstones and audit evidence remain.")).toBeInTheDocument();
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

    fireEvent.click(await screen.findByRole("tab", { name: "work" }));
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

    fireEvent.click(await screen.findByRole("tab", { name: "work" }));
    fireEvent.click(screen.getByRole("tab", { name: /changes/i }));
    fireEvent.click(screen.getByRole("button", { name: /Working diff/ }));
    expect(await screen.findByText("Execute workspace.diff once")).toBeInTheDocument();
    expect(screen.getByText("sha256:diff")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Hide workspace inspector" }));
    const pending = await screen.findByRole("region", { name: "Pending approval" });
    expect(within(pending).getByText("WORKSPACE APPROVAL")).toBeInTheDocument();
    expect(within(pending).getByText("sha256:diff")).toBeInTheDocument();
    expect(runtime.resolveApproval).not.toHaveBeenCalled();
    fireEvent.click(within(pending).getByRole("button", { name: "Allow once" }));

    await waitFor(() => {
      expect(runtime.resolveApproval).toHaveBeenCalledWith("approval-diff", true);
      expect(runtime.resume).toHaveBeenCalledWith("approval-diff", "diff", "coding-domain");
    });
    fireEvent.click(screen.getByRole("button", { name: "Show workspace inspector" }));
    expect(await screen.findByLabelText("Workspace inspection output")).toHaveTextContent("diff --git");
  });

  it("runs bounded terminal checks through registered Genes", async () => {
    runtime.run.mockResolvedValue({
      mode: "direct",
      session_id: "session-test",
      execution_id: "execution-test",
      selected_harness: "coding-domain",
      selected_gene: "workspace.test",
      status: "completed",
      output: "all tests passed",
      receipt_count: 1,
      event_count: 5,
    });
    render(<App />);

    fireEvent.click(await screen.findByRole("tab", { name: "work" }));
    fireEvent.click(screen.getByRole("tab", { name: /terminal/i }));
    fireEvent.click(screen.getByRole("button", { name: /Tests workspace\.test/ }));

    await waitFor(() => expect(runtime.run).toHaveBeenCalledWith("test", "coding-domain"));
    expect(await screen.findByLabelText("Workspace inspection output")).toHaveTextContent("all tests passed");
    expect(screen.getByText("registered Gene output")).toBeInTheDocument();
    expect(screen.getByText(/No arbitrary shell/)).toBeInTheDocument();
  });

  it("shows the latest bounded run output as an inspectable artifact", async () => {
    runtime.agentRun.mockResolvedValue({
      mode: "agent",
      session_id: session.session_id,
      execution_id: "execution-artifact",
      selected_harness: "coding-domain",
      selected_gene: null,
      status: "completed",
      output: "runtime-backed artifact output",
      receipt_count: 2,
      event_count: 6,
    });
    render(<App />);

    const composer = await screen.findByLabelText("Pandora task");
    fireEvent.change(composer, { target: { value: "Produce a bounded artifact" } });
    fireEvent.submit(composer.closest("form")!);
    await screen.findByText("runtime-backed artifact output");

    fireEvent.click(screen.getByRole("tab", { name: "work" }));
    fireEvent.click(screen.getByRole("tab", { name: /artifacts/i }));

    expect(screen.getByLabelText("Run artifact output")).toHaveTextContent("runtime-backed artifact output");
    expect(screen.getAllByText("execution-artifact").length).toBeGreaterThan(0);
    expect(screen.getByText("Latest bounded output")).toBeInTheDocument();
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
    fireEvent.click(screen.getByRole("button", { name: "Hide workspace inspector" }));
    const pending = await screen.findByRole("region", { name: "Pending approval" });
    expect(within(pending).getByText("NETWORK APPROVAL")).toBeInTheDocument();
    expect(within(pending).getByText("pandora-request-v2:sha256:browser")).toBeInTheDocument();
    expect(runtime.resolveApproval).not.toHaveBeenCalled();
    fireEvent.click(within(pending).getByRole("button", { name: "Allow once" }));

    await waitFor(() => {
      expect(runtime.run).toHaveBeenCalledWith("fetch:https://example.com/", "research-domain");
      expect(runtime.resolveApproval).toHaveBeenCalledWith("approval-browser", true);
      expect(runtime.resume).toHaveBeenCalledWith("approval-browser", "fetch:https://example.com/", "research-domain");
    });
    fireEvent.click(screen.getByRole("button", { name: "Show workspace inspector" }));
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
      memory_evidence_ids: ["memory-a", "memory-b"],
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
    expect(screen.getByText("memory-a, memory-b")).toBeInTheDocument();
    expect(screen.getByText("parliament-a · policy v1")).toBeInTheDocument();
    expect(screen.getByText("catalog active")).toBeInTheDocument();
    expect(screen.getByText("Runtime authority").nextSibling).toHaveTextContent("Unchanged");
    expect(screen.getByLabelText("Scheduled canary loop")).toHaveTextContent("one-shot");
    expect(screen.getByLabelText("Scheduled canary loop")).toHaveTextContent("Activation remains a separate exact-confirmation action");
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
      expect(runtime.agentResume).toHaveBeenCalledWith(approval.approval_id, null, null);
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

    fireEvent.click(screen.getByRole("tab", { name: "work" }));
    expect(screen.getByRole("heading", { name: "workspace-1" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Work surfaces expose evidence, not authority" })).toBeInTheDocument();
    expect(screen.getByText("Read only")).toBeInTheDocument();
    expect(screen.getByText("Exact permit path")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Inspector options" })).toBeInTheDocument();
  });

  it("inspects deep runtime component contracts from one inventory", async () => {
    runtime.engines.mockResolvedValue([
      runtimeComponent({
        id: "execution-controller",
        name: "ExecutionController",
        role: "Fixed runtime pipeline",
        authority: "Runtime authority",
        category: "Core authority",
        component_kind: "constitutional_core",
        inputs: ["Exact execution request"],
        outputs: ["Governed execution outcome"],
        invariants: ["Every effect requires a fresh exact permit"],
        evidence: ["Permit and effect receipts"],
        source_modules: ["crates/pandora-runtime/src/execution_controller.rs"],
        related_components: ["Parliament", "Shadow Council", "ReferenceMonitor"],
      }),
      runtimeComponent({
        id: "context-recovery",
        name: "ContextRecovery",
        role: "Context rot recovery",
        authority: "Embedded recovery plan only",
        category: "Resilience",
        component_kind: "embedded_component",
        inputs: ["Verified L1 availability"],
        outputs: ["Ordered recovery decision"],
        invariants: ["Failure to recover pauses instead of fabricating context"],
        source_modules: ["crates/pandora-runtime/src/context_recovery.rs"],
      }),
      runtimeComponent({
        id: "provider-failover",
        name: "FailoverProvider",
        role: "Governed provider fallback",
        authority: "Retryable transition only",
        category: "Resilience",
        component_kind: "embedded_component",
        invariants: ["Fallback receives a fresh policy decision and permit"],
        source_modules: ["crates/pandora-provider/src/failover.rs"],
      }),
    ]);

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Runtime Inventory" }));

    expect(await screen.findByRole("heading", { name: "ExecutionController" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Runtime components/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Harnesses & Genes/ })).toBeInTheDocument();
    expect(screen.getAllByText("Runtime authority").length).toBeGreaterThan(0);
    expect(screen.getByText("Parliament")).toBeInTheDocument();
    expect(screen.getByText("Shadow Council")).toBeInTheDocument();
    expect(screen.getByText("CONSTITUTIONAL RUNTIME BOUNDARY")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("tab", { name: "Contract" }));
    expect(screen.getByText("Exact execution request")).toBeInTheDocument();
    expect(screen.getByText("Governed execution outcome")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("tab", { name: "Boundaries" }));
    expect(screen.getByText("Every effect requires a fresh exact permit")).toBeInTheDocument();
    expect(screen.getByText(/cannot grant itself capabilities or bypass those boundaries/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("tab", { name: "Evidence & source" }));
    expect(screen.getByText("crates/pandora-runtime/src/execution_controller.rs")).toBeInTheDocument();
    expect(screen.getByLabelText("Component contract JSON")).toHaveTextContent('"id": "execution-controller"');

    fireEvent.click(within(screen.getByRole("group", { name: "Component category" })).getByRole("button", { name: /Resilience/ }));
    expect(screen.getByRole("heading", { name: "ContextRecovery" })).toBeInTheDocument();
    expect(screen.getByText("EMBEDDED RESILIENCE COMPONENT")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /FailoverProvider/ }));
    expect(screen.getByRole("heading", { name: "FailoverProvider" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("tab", { name: "Boundaries" }));
    expect(screen.getByText("Fallback receives a fresh policy decision and permit")).toBeInTheDocument();
    expect(screen.getByText(/Selecting or filtering inventory records never changes/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Inspect connections/ }));
    expect(screen.getByRole("heading", { name: "Connections" })).toBeInTheDocument();
  });

  it("inspects runtime-reported tool contracts without granting execution authority", async () => {
    runtime.tools.mockResolvedValue([
      { id: "filesystem.read", version: "1.0.0", name: "Filesystem Reader", capability: "filesystem", operation: "read" },
      { id: "process.spawn", version: "2.1.0", name: "Process Runner", capability: "process", operation: "execute" },
    ]);

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Tools" }));

    expect(await screen.findByRole("heading", { name: "Filesystem Reader" })).toBeInTheDocument();
    expect(screen.getByLabelText("Tool contract JSON")).toHaveTextContent('"id": "filesystem.read"');
    expect(screen.getByText("Harness → Gene → ReferenceMonitor → ToolEngine")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Process Runner/ }));
    expect(screen.getByRole("heading", { name: "Process Runner" })).toBeInTheDocument();
    expect(screen.getByLabelText("Tool contract JSON")).toHaveTextContent('"operation": "execute"');
    expect(screen.getByText(/Selecting a tool never activates it/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Inspect Harnesses/ }));
    expect(screen.getByRole("heading", { name: "Harness Lab" })).toBeInTheDocument();
  });

  it("shows a truthful memory empty state instead of invented graph nodes", async () => {
    runtime.sessions.mockResolvedValue([session]);

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /session-1/ }));
    await waitFor(() => expect(runtime.inspectSession).toHaveBeenCalledWith(session.session_id));
    fireEvent.click(screen.getByRole("button", { name: "Memory" }));

    expect(await screen.findByRole("heading", { name: "No memory evidence recorded" })).toBeInTheDocument();
    expect(screen.getByText("The selected session returned no durable memory records.")).toBeInTheDocument();
    expect(screen.getByText(/does not invent graph nodes/)).toBeInTheDocument();
    expect(screen.queryByText("active plan")).not.toBeInTheDocument();
    expect(screen.queryByText("verified run")).not.toBeInTheDocument();
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

  it("manages local Skills through their separate desktop lifecycle", async () => {
    runtime.capabilities.mockResolvedValue([{
      id: "coding-domain",
      version: "1.2.0",
      name: "Coding Domain",
      kind: "domain",
      gene_count: 1,
      runnable: true,
      gene_ids: ["coding.inspect"],
    }]);
    runtime.listLocalSkills.mockResolvedValue({
      message: "Loaded local Skills.",
      restartRequired: false,
      data: { skills: [{
        id: "release-review",
        version: "1.0.0",
        name: "Release Reviewer",
        description: "Reviews local release evidence.",
        publisher: "local",
        resources: ["release-checklist"],
        state: "enabled",
        root: "C:\\Pandora\\skills\\release-review",
        provenance: "local",
      }] },
    });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Harness Lab" }));
    fireEvent.click(await screen.findByRole("tab", { name: "skills" }));

    expect(await screen.findByText("Release Reviewer")).toBeInTheDocument();
    expect(screen.getByText(/does not grant a capability/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Disable Release Reviewer" }));
    await waitFor(() => expect(runtime.mutateLocalSkill).toHaveBeenCalledWith("release-review", "disable", ""));

    fireEvent.change(screen.getByLabelText("Local Skill directory"), { target: { value: "C:\\skills\\new-skill" } });
    fireEvent.click(screen.getByRole("button", { name: "Install disabled" }));
    await waitFor(() => expect(runtime.installLocalSkill).toHaveBeenCalledWith("C:\\skills\\new-skill"));

    fireEvent.click(screen.getByRole("button", { name: "Remove Release Reviewer" }));
    fireEvent.change(screen.getByLabelText("Confirm removal release-review"), { target: { value: "release-review" } });
    fireEvent.click(screen.getByRole("button", { name: "Confirm removal" }));
    await waitFor(() => expect(runtime.mutateLocalSkill).toHaveBeenCalledWith("release-review", "remove", "release-review"));
  });

  it("offers a runtime-loaded custom Harness for governed desktop runs", async () => {
    runtime.capabilities.mockResolvedValue([{
      id: "example/service-domain",
      version: "1.0.0",
      name: "Example Service Domain",
      kind: "domain",
      gene_count: 1,
      runnable: true,
      gene_ids: ["example/service-echo"],
    }]);
    runtime.agentRun.mockResolvedValue({
      mode: "agent",
      session_id: session.session_id,
      execution_id: "execution-custom-harness",
      selected_harness: "example/service-domain",
      selected_gene: "example/service-echo",
      status: "completed",
      output: "custom package output",
      receipt_count: 1,
      event_count: 4,
    });

    render(<App />);
    const selector = await screen.findByLabelText("Execution Harness");
    expect(within(selector).getByRole("option", { name: "Example Service Domain" })).toBeInTheDocument();
    fireEvent.change(selector, { target: { value: "example/service-domain" } });
    const composer = screen.getByLabelText("Pandora task");
    fireEvent.change(composer, { target: { value: "Run the installed transformation" } });
    fireEvent.submit(composer.closest("form")!);

    await waitFor(() => expect(runtime.agentRun).toHaveBeenCalledWith(
      "Run the installed transformation",
      null,
      "example/service-domain",
      [],
      null,
      null,
    ));
    expect(await screen.findByText("custom package output")).toBeInTheDocument();
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
      domain_routing: null,
      replaces_builtin: false,
      state: "admitted",
      runtime_authority: false,
      activation: {
        state: "disabled",
        active_version: null,
        previous_version: null,
        generation: 0,
        runtime_authority: false,
      },
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
    runtime.listPackageTransparency.mockResolvedValue({
      message: "Loaded 1 append-only package transparency event(s).",
      restartRequired: false,
      data: {
        events: [{
          sequence: 1,
          event_kind: "admission_decision",
          outcome: "allowed",
          occurred_at: 1_788_192_000,
          publisher: "example",
          key_id: null,
          package_id: "example/refactor",
          package_version: "1.2.3",
          subject_digest: "sha256:2222222222222222222222222222222222222222222222222222222222222222",
          artifact_digest: localPackage.content_hash,
          reason_code: "admitted",
          previous_event_digest: null,
          event_digest: "sha256:3333333333333333333333333333333333333333333333333333333333333333",
        }],
        count: 1,
        durability: "append-only-sqlite",
        integrity: "sha256-event-chain",
        runtime_authority: false,
      },
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
    runtime.previewPackageEnable.mockResolvedValue({
      message: "Activation preview recorded for example/refactor@1.2.3; no lifecycle binding changed.",
      restartRequired: false,
      data: {
        dry_run: true,
        ready: true,
        dependencies: [{ id: "workspace.read", version: "0.1.0", optional: false, source: "built_in", enabled: true }],
        enabled_dependents: [],
      },
    });
    runtime.enableLocalPackage.mockResolvedValue({
      message: "Package example/refactor@1.2.3 enabled without changing Pandora's constitutional authority.",
      restartRequired: true,
      data: { changed: true },
    });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Harness Lab" }));
    fireEvent.click(await screen.findByRole("tab", { name: "packages" }));

    expect(await screen.findByRole("heading", { name: "Signed package manager" })).toBeInTheDocument();
    expect(await screen.findByText("example/refactor")).toBeInTheDocument();
    expect(await screen.findByLabelText("Package transparency evidence")).toHaveTextContent("admission decision");
    expect(screen.getByLabelText("Package transparency evidence")).toHaveTextContent("allowed");
    expect(screen.getByText(/Read-only SHA-256 chain evidence/)).toBeInTheDocument();
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
      registryProfile: "",
      registryUrl: "https://registry.example.test",
      token: "process-secret",
    }));
    expect(screen.getByLabelText("Package registry token")).toHaveValue("");
    expect(await screen.findByRole("status")).toHaveTextContent("Restart the local service");

    fireEvent.click(screen.getByRole("button", { name: "Preview enable" }));
    expect(await screen.findByText("workspace.read")).toBeInTheDocument();
    expect(runtime.previewPackageEnable).toHaveBeenCalledWith("example/refactor", "1.2.3");
    const lifecycleConfirmation = screen.getByLabelText("Confirm enable example/refactor@1.2.3");
    fireEvent.change(lifecycleConfirmation, { target: { value: "example/refactor@1.2.3" } });
    fireEvent.click(screen.getByRole("button", { name: "Confirm enable" }));
    await waitFor(() => expect(runtime.enableLocalPackage).toHaveBeenCalledWith(
      "example/refactor",
      "1.2.3",
      "example/refactor@1.2.3",
    ));

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

  it("previews Domain auto-route overlap without changing lifecycle state", async () => {
    const domainPackage = (id: string, hints: string[]) => ({
      id,
      version: "1.0.0",
      kind: "domain_harness",
      publisher: "example",
      content_hash: `sha256:${id.replace(/[^a-z]/g, "").padEnd(64, "0")}`,
      dependencies: [],
      compatibility: "pandora>=0.1.0",
      license: "MIT",
      trust: { level: "verified", has_signature: true, has_public_key: true },
      meta_composition: null,
      domain_routing: { hints, auto_route: true },
      replaces_builtin: false,
      state: "admitted",
      runtime_authority: false,
      activation: { state: "disabled", active_version: null, previous_version: null, generation: 0, runtime_authority: false },
    });
    runtime.listLocalPackages.mockResolvedValue({
      message: "2 local package(s) available.",
      restartRequired: false,
      data: { packages: [domainPackage("image-domain", ["image generation", "text to image"]), domainPackage("video-domain", ["text to image", "video generation"])] },
    });

    runtime.capabilities.mockResolvedValue([{ id: "image-domain", version: "1.0.0", name: "Image Domain", kind: "domain", gene_count: 0, runnable: true, gene_ids: [] }]);
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Harness Lab" }));
    fireEvent.click(await screen.findByRole("tab", { name: "packages" }));

    expect(await screen.findByRole("heading", { name: "Signed package manager" })).toBeInTheDocument();
    expect(await screen.findByLabelText("Auto route preview")).toHaveTextContent("review overlap");
    expect(screen.getByLabelText("Auto route preview")).toHaveTextContent("text to image");
    expect(screen.getByLabelText("Auto route preview")).toHaveTextContent("video-domain@1.0.0");
    expect(screen.getByLabelText("Auto route preview")).toHaveTextContent("fails closed on an ambiguous tie");
    expect(runtime.previewPackageEnable).not.toHaveBeenCalled();
  });

  it("inspects exact Meta composition, lifecycle, and trust evidence without authority", async () => {
    runtime.capabilities.mockResolvedValue([{ id: "coordination-meta", version: "0.4.0", name: "Coordination Meta", kind: "meta", gene_count: 0, runnable: false, gene_ids: [] }]);
    runtime.listLocalPackages.mockResolvedValue({
      message: "1 local package available.",
      restartRequired: false,
      data: {
        packages: [{
          id: "example/meta-starter",
          version: "2.0.0",
          kind: "meta_harness",
          publisher: "example",
          content_hash: `sha256:${"b".repeat(64)}`,
          dependencies: [
            { id: "coding-domain", version: "0.1.0", optional: false },
            { id: "research-domain", version: "0.1.0", optional: false },
          ],
          compatibility: "pandora=2.0.0-beta.7",
          license: "Apache-2.0",
          trust: { level: "unverified", has_signature: false, has_public_key: false },
          meta_composition: { allowed_domains: ["coding-domain", "research-domain"], max_handoffs: 4 },
          domain_routing: null,
          replaces_builtin: false,
          state: "admitted",
          runtime_authority: false,
          activation: { state: "enabled", active_version: "2.0.0", previous_version: "1.0.0", generation: 3, runtime_authority: false },
        }],
      },
    });
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Harness Lab" }));
    fireEvent.click(await screen.findByRole("tab", { name: "packages" }));

    const composition = await screen.findByLabelText("Meta composition inspection");
    expect(composition).toHaveTextContent("coding-domain");
    expect(composition).toHaveTextContent("research-domain");
    expect(composition).toHaveTextContent("0.1.0 exact");
    expect(composition).toHaveTextContent("4 handoffs max");
    expect(composition).toHaveTextContent("fail before effect execution");
    expect(screen.getByText("Active version").nextElementSibling).toHaveTextContent("2.0.0");
    expect(screen.getByText("Rollback target").nextElementSibling).toHaveTextContent("1.0.0");
    expect(screen.getByText("Generation").nextElementSibling).toHaveTextContent("3");
    expect(screen.getByText("Runtime authority").nextElementSibling).toHaveTextContent("none");
  });

  it("inspects a Gene capability contract, provenance, and owning Domain without implying authority", async () => {
    runtime.capabilities.mockResolvedValue([{ id: "example/gene-pack-domain", version: "1.0.0", name: "Gene Pack Domain", kind: "domain", gene_count: 3, runnable: true, gene_ids: ["example/static-guide", "example/bounded-read", "example/patch-proposal"] }]);
    runtime.listLocalPackages.mockResolvedValue({
      message: "1 local package available.",
      restartRequired: false,
      data: {
        packages: [{
          id: "example/patch-proposal",
          version: "1.0.0",
          kind: "gene",
          publisher: "pandora-community",
          content_hash: `sha256:${"c".repeat(64)}`,
          dependencies: [],
          compatibility: "pandora=2.0.0-beta.7",
          license: "Apache-2.0",
          trust: { level: "unverified", has_signature: false, has_public_key: false },
          meta_composition: null,
          domain_routing: null,
          gene_contract: { execution: "effect_request", capabilities: ["filesystem.write"], approval_required: true, direct_executor_access: false },
          provenance: { publisher: "pandora-community", content_hash: `sha256:${"c".repeat(64)}`, trust_level: "unverified", artifact_verified: true },
          owning_domains: [{ id: "example/gene-pack-domain", version: "1.0.0", state: "admitted" }],
          replaces_builtin: false,
          state: "installed",
          runtime_authority: false,
          activation: { state: "disabled", active_version: null, previous_version: null, generation: 0, runtime_authority: false },
        }],
      },
    });
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Harness Lab" }));
    fireEvent.click(await screen.findByRole("tab", { name: "packages" }));

    const contract = await screen.findByLabelText("Gene contract inspection");
    expect(contract).toHaveTextContent("effect request");
    expect(contract).toHaveTextContent("filesystem.write");
    expect(contract).toHaveTextContent("Explicit approval required");
    expect(contract).toHaveTextContent("example/gene-pack-domain@1.0.0");
    expect(contract).toHaveTextContent("SHA-256 verified");
    expect(contract).toHaveTextContent("cannot call an executor, approve itself, or issue a permit");
    expect(screen.getByText("Runtime authority").nextElementSibling).toHaveTextContent("none");
  });

  it("previews an admission-safe Domain manifest without mutating runtime state", async () => {
    runtime.capabilities.mockResolvedValue([{
      id: "coding-domain",
      version: "1.2.0",
      name: "Coding Domain",
      kind: "domain",
      gene_count: 0,
      runnable: true,
      gene_ids: [],
    }]);
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Harness Lab" }));
    fireEvent.click(await screen.findByRole("tab", { name: "packages" }));
    fireEvent.click(await screen.findByRole("tab", { name: "Author manifest" }));

    expect(await screen.findByRole("heading", { name: "Author a package envelope" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Copy JSON" })).toBeDisabled();
    expect(screen.getByLabelText("Domain Harness starter kit")).toHaveTextContent("pandora package scaffold domain-harness");
    expect(screen.getByLabelText("Domain Harness starter kit")).toHaveTextContent("/domain-starter");
    expect(screen.getByLabelText("Meta Harness starter kit")).toHaveTextContent("pandora package scaffold meta-harness");
    expect(screen.getByLabelText("Meta Harness starter kit")).toHaveTextContent("/meta-starter");
    expect(screen.getByLabelText("Gene pack examples")).toHaveTextContent("sdk/gene-pack");
    expect(screen.getByLabelText("Gene pack examples")).toHaveTextContent("/gene-pack");
    fireEvent.change(screen.getByLabelText("Authoring package ID"), { target: { value: "owner/image-domain" } });
    fireEvent.change(screen.getByLabelText("Authoring content hash"), { target: { value: `sha256:${"a".repeat(64)}` } });
    fireEvent.change(screen.getByLabelText("Authoring route hints"), { target: { value: "image generation, text to image" } });
    fireEvent.change(screen.getByLabelText("Authoring dependencies"), { target: { value: "workspace.read@0.1.0\nworkspace.read@0.2.0" } });

    expect(await screen.findByText("Dependency 'workspace.read' is declared more than once.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Copy JSON" })).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Authoring dependencies"), { target: { value: "workspace.read@0.1.0" } });

    expect(await screen.findByText("manifest ready")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Copy JSON" })).toBeEnabled();
    expect(screen.getByLabelText("Package manifest JSON")).toHaveTextContent('"kind": "domain_harness"');
    expect(screen.getByLabelText("Package manifest JSON")).toHaveTextContent('"runtime": "pandora=2.0.0-beta.7"');
    expect(screen.getByLabelText("Package manifest JSON")).toHaveTextContent('"license": "Apache-2.0"');
    expect(screen.getByLabelText("Package manifest JSON")).toHaveTextContent('"hints": [');
    expect(screen.getByText(/never signs, admits, enables, publishes, stores keys/i)).toBeInTheDocument();
    expect(runtime.admitLocalPackage).not.toHaveBeenCalled();
    expect(runtime.enableLocalPackage).not.toHaveBeenCalled();
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

  it("admits a GitHub package only from a pinned commit and clears its token", async () => {
    const commit = "0123456789abcdef0123456789abcdef01234567";
    runtime.capabilities.mockResolvedValue([{
      id: "coding-domain",
      version: "1.2.0",
      name: "Coding Domain",
      kind: "domain",
      gene_count: 0,
      runnable: true,
      gene_ids: [],
    }]);

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Harness Lab" }));
    fireEvent.click(await screen.findByRole("tab", { name: "packages" }));
    fireEvent.click(await screen.findByRole("tab", { name: "GitHub commit" }));

    expect(screen.getByRole("button", { name: "Fetch pinned source" })).toBeDisabled();
    fireEvent.change(screen.getByLabelText("GitHub package repository"), { target: { value: "https://github.com/owner/repository" } });
    fireEvent.change(screen.getByLabelText("GitHub package commit"), { target: { value: commit } });
    fireEvent.change(screen.getByLabelText("GitHub package manifest path"), { target: { value: "packages/domain.json" } });
    fireEvent.change(screen.getByLabelText("GitHub package artifact path"), { target: { value: "dist/domain.artifact" } });
    fireEvent.change(screen.getByLabelText("GitHub package token"), { target: { value: "github-secret" } });
    fireEvent.click(screen.getByRole("button", { name: "Fetch pinned source" }));

    await waitFor(() => expect(runtime.installGitHubPackage).toHaveBeenCalledWith({
      repositoryUrl: "https://github.com/owner/repository",
      commit,
      manifestPath: "packages/domain.json",
      artifactPath: "dist/domain.artifact",
      token: "github-secret",
    }));
    expect(screen.getByLabelText("GitHub package token")).toHaveValue("");
    expect(screen.getByText(/fetches only these two paths at the pinned commit/i)).toBeInTheDocument();
  });


  it("keeps native local service credentials out of the interface", async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Connections" }));

    expect(await screen.findByText("Device-local trust")).toBeInTheDocument();
    expect(screen.queryByText(/account|login|tenant/i)).not.toBeInTheDocument();
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

  it("selects an existing provider profile through the native configuration boundary", async () => {
    runtime.providers.mockResolvedValue([{
      name: "design",
      model: "design-model",
      protocol: "open_ai_compatible",
      active: false,
      credential_configured: true,
      fallback_provider: null,
    }]);

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Connections" }));
    fireEvent.click(await screen.findByRole("button", { name: "Use design" }));

    await waitFor(() => expect(runtime.activateProvider).toHaveBeenCalledWith("design"));
    expect(screen.getByRole("status")).toHaveTextContent("Restart the local service to apply it");
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

  it("stores a custom registry profile without exposing its token to browser storage", async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Connections" }));
    fireEvent.click(screen.getByRole("tab", { name: "Package registry" }));

    fireEvent.change(screen.getByLabelText("Registry profile URL"), { target: { value: "https://registry.example.test/" } });
    fireEvent.change(screen.getByLabelText("Registry token environment name"), { target: { value: "pandora_mplace_token" } });
    fireEvent.change(screen.getByLabelText("Registry profile token"), { target: { value: "registry-secret" } });
    fireEvent.click(screen.getByRole("button", { name: /Save registry/ }));

    await waitFor(() => expect(runtime.configureRegistryProfile).toHaveBeenCalledWith({
      name: "m-place",
      baseUrl: "https://registry.example.test/",
      tokenEnvironment: "PANDORA_MPLACE_TOKEN",
      token: "registry-secret",
    }));
    expect(screen.getByLabelText("Registry profile token")).toHaveValue("");
    expect(screen.getByRole("status")).toHaveTextContent("Registry m-place configured");
    expect(Object.values(window.localStorage)).not.toContain("registry-secret");
  });

  it("installs through the active saved registry profile without resending its URL", async () => {
    runtime.capabilities.mockResolvedValue([{
      id: "coding-domain",
      version: "1.2.0",
      name: "Coding Domain",
      kind: "domain",
      gene_count: 0,
      runnable: true,
      gene_ids: [],
    }]);
    runtime.listRegistryProfiles.mockResolvedValue({
      message: "1 registry profile configured.",
      data: {
        registries: [{
          name: "m-place",
          base_url: "https://registry.example.test",
          token_env: "PANDORA_MPLACE_TOKEN",
          active: true,
        }],
      },
    });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Harness Lab" }));
    fireEvent.click(await screen.findByRole("tab", { name: "packages" }));
    expect(await screen.findByLabelText("Saved registry profile")).toHaveValue("m-place");
    expect(screen.getByLabelText("Package registry URL")).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Registry package ID"), { target: { value: "owner/gene" } });
    fireEvent.click(screen.getByRole("button", { name: "Fetch and admit" }));

    await waitFor(() => expect(runtime.installRegistryPackage).toHaveBeenCalledWith({
      packageId: "owner/gene",
      version: "",
      registryProfile: "m-place",
      registryUrl: "",
      token: "",
    }));
  });

  it("shows a privacy-safe Fleet operations snapshot with aggregate usage", async () => {
    runtime.listFleetOperations.mockResolvedValue({
      message: "Local Fleet operations need attention.",
      data: {
        generated_at: 100,
        health: { status: "attention", ready_nodes: 2, running_supervisors: 1, stale_supervisors: 1, overdue_active_leases: 0, queued_without_capacity: false, aggregate_budget_invariant_holds: true },
        fleet: {
          nodes: { total: 2, by_state: { ready: 2 } },
          supervisors: { total: 1, by_state: { running: 1 }, stale: [{ node_id: "node-a", state: "running", age_seconds: 45 }] },
          leases: {
            total: 1,
            by_state: { active: 1 },
            active: [{ lease_id: "lease-a", node_id: "node-a", age_seconds: 25, expires_in_seconds: 95, overdue: false, budget_ceiling: { max_tokens: 4000, max_tools: 20, max_duration_seconds: 120, max_cost_micros: 25000 } }],
            active_details_truncated: false,
          },
        },
        queue: {
          jobs: { total: 4, by_status: { queued: 2, running: 1, failed: 1 }, queued: 2, running: 1, failure_count: 1 },
          orchestrations: { total: 1, by_status: { queued: 1 }, queued: 1, running: 0, failure_count: 0 },
        },
        failures: { count: 1, records: [{ kind: "job", id: "job-failed", status: "failed", recorded_at: 90 }], records_truncated: false },
        budget_ceilings: { active_lease_count: 1, max_tokens: 4000, max_tools: 20, max_duration_seconds: 120, max_cost_micros: 25000, saturated: false, actual_spend_available: false },
        aggregate_budgets: {
          run_count: 1,
          records: [{ run_id: "run-budget", budget: {} }],
          records_truncated: false,
          ceiling: { tokens: 4000, tools: 20, elapsed_ms: 120000, cost_micros: 25000 },
          reserved: { tokens: 500, tools: 3, elapsed_ms: 10000, cost_micros: 5000 },
          consumed: { tokens: 1500, tools: 7, elapsed_ms: 40000, cost_micros: null, known_cost_micros: 12000, unknown_cost_receipts: 1, enforced_cost_micros: 18000 },
          remaining: { tokens: 2000, tools: 10, elapsed_ms: 70000, cost_micros: null, enforced_cost_micros: 2000 },
          saturated: false,
          invariant: { holds: true, expression: "enforced_consumed + active_reservations <= aggregate_ceiling" },
        },
        boundary: { read_only: true, runtime_authority: false, budgets_are_ceilings_not_spend: true, aggregate_usage_available: true, aggregate_cost_unknown_explicit: true, prompts_included: false, outputs_included: false, credentials_included: false, hidden_reasoning_included: false },
      },
    });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Background Runs" }));

    expect(await screen.findByRole("heading", { name: "Fleet health and bounded capacity" })).toBeInTheDocument();
    expect(screen.getByText("lease-a")).toBeInTheDocument();
    expect(screen.getByText(/age 25s/)).toBeInTheDocument();
    expect(screen.getByText("job-failed")).toBeInTheDocument();
    expect(screen.getByText(/Prompts, outputs, credentials, and hidden reasoning are excluded/)).toBeInTheDocument();
    expect(screen.getByText("4,000")).toBeInTheDocument();
    expect(screen.getByText(/1,500 used · 500 reserved · 2,000 available \/ 4,000 ceiling/)).toBeInTheDocument();
    expect(screen.getByText(/Unknown · 1 unknown receipt · 18,000 µ enforced/)).toBeInTheDocument();
    expect(runtime.listFleetOperations).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole("button", { name: "Refresh snapshot" }));
    await waitFor(() => expect(runtime.listFleetOperations).toHaveBeenCalledTimes(2));
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
