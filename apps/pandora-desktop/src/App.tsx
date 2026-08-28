import { useEffect, useMemo, useRef, useState, type ChangeEvent, type FormEvent, type ReactNode } from "react";
import {
  configureMcp,
  configureProvider,
  loadRuntimeEndpoint,
  isNativeRuntime,
  nativeEndpoint,
  RuntimeClient,
  saveRuntimeEndpoint,
  startLocalService,
  stopLocalService,
  type RuntimeApproval,
  type RuntimeArtifactActivation,
  type RuntimeContextAttachment,
  type RuntimeRun,
  type RuntimeEvent,
  type RuntimeEngine,
  type RuntimeEvolutionMutation,
  type RuntimeEvolutionProposal,
  type RuntimeHarness,
  type RuntimeHealth,
  type RuntimeMemoryRecord,
  type RuntimeOrchestrationRun,
  type RuntimeProvider,
  type RuntimeStatus,
  type RuntimeSession,
  type RuntimeSessionDetail,
  type RuntimeTool,
} from "./runtimeClient";

type ViewId =
  | "command"
  | "runs"
  | "council"
  | "memory"
  | "workflows"
  | "capabilities"
  | "engines"
  | "tools"
  | "connections"
  | "audit"
  | "evolution"
  | "settings";

type RunProfile = string;
type ThemeMode = "dark" | "light";
type InspectorTab = "flow" | "evidence" | "workspace";
type HarnessTab = "genes" | "extensions" | "authority" | "receipts";

type PendingRunRequest = {
  task: string;
  requestedHarness: string | null;
};

type WorkspaceInspectionRequest = {
  task: string;
  requestedHarness: string;
};

type WorkflowRecipe = {
  id: string;
  name: string;
  task: string;
  profile: RunProfile;
};

const themeStorageKey = "pandora.desktop.theme";
const workflowStorageKey = "pandora.desktop.workflows";
const maxContextAttachments = 8;
const maxContextAttachmentBytes = 16 * 1024;
const maxContextBytes = 24 * 1024;
const textAttachmentExtensions = new Set([
  "c", "cc", "cpp", "css", "csv", "go", "h", "hpp", "html", "java", "js", "json", "jsx",
  "md", "mjs", "py", "rb", "rs", "sh", "sql", "svg", "toml", "ts", "tsx", "txt", "xml", "yaml", "yml",
]);
const applicationTextMediaTypes = new Set(["application/json", "application/javascript", "application/sql", "application/xml", "application/yaml"]);

function attachmentByteLength(content: string): number {
  return new TextEncoder().encode(content).length;
}

function isTextAttachment(file: File): boolean {
  if (file.type.startsWith("text/")) {
    return true;
  }
  if (applicationTextMediaTypes.has(file.type)) {
    return true;
  }
  const extension = file.name.split(".").pop()?.toLowerCase() ?? "";
  return textAttachmentExtensions.has(extension);
}

function contextMediaType(file: File): string {
  return file.type.startsWith("text/") || applicationTextMediaTypes.has(file.type) ? file.type : "text/plain";
}

function readTextAttachment(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.addEventListener("load", () => resolve(typeof reader.result === "string" ? reader.result : ""));
    reader.addEventListener("error", () => reject(new Error(`Could not read ${file.name}`)));
    reader.readAsText(file);
  });
}

function loadTheme(): ThemeMode {
  return typeof window !== "undefined" && window.localStorage.getItem(themeStorageKey) === "light" ? "light" : "dark";
}

function loadWorkflows(): WorkflowRecipe[] {
  try {
    const value: unknown = JSON.parse(window.localStorage.getItem(workflowStorageKey) ?? "[]");
    if (!Array.isArray(value)) {
      return [];
    }
    return value.filter((item): item is WorkflowRecipe => Boolean(item) && typeof item === "object" && typeof (item as WorkflowRecipe).id === "string" && typeof (item as WorkflowRecipe).name === "string" && typeof (item as WorkflowRecipe).task === "string" && typeof (item as WorkflowRecipe).profile === "string").slice(0, 24);
  } catch {
    return [];
  }
}

type IconName =
  | "activity"
  | "archive"
  | "arrow"
  | "book"
  | "box"
  | "check"
  | "chevron"
  | "clock"
  | "code"
  | "copy"
  | "council"
  | "dots"
  | "download"
  | "evolution"
  | "gear"
  | "graph"
  | "grid"
  | "lock"
  | "menu"
  | "plus"
  | "search"
  | "shield"
  | "spark"
  | "stack"
  | "terminal"
  | "users";

const navigation: Array<{ label: string; items: Array<{ id: ViewId; label: string; icon: IconName }> }> = [
  {
    label: "Operate",
    items: [
      { id: "command", label: "Command", icon: "activity" },
      { id: "runs", label: "Background Runs", icon: "stack" },
      { id: "council", label: "Council", icon: "council" },
      { id: "memory", label: "Memory", icon: "graph" },
      { id: "workflows", label: "Workflows", icon: "stack" }
    ]
  },
  {
    label: "Configure",
    items: [
      { id: "capabilities", label: "Harness Lab", icon: "box" },
      { id: "engines", label: "Engines", icon: "stack" },
      { id: "tools", label: "Tools", icon: "terminal" },
      { id: "connections", label: "Connections", icon: "grid" },
      { id: "audit", label: "Audit", icon: "archive" },
      { id: "evolution", label: "Evolution", icon: "evolution" }
    ]
  }
];

const runProfiles: Array<{ id: RunProfile; label: string; harness: string | null }> = [
  { id: "auto", label: "Auto route", harness: null },
  { id: "coding", label: "Coding", harness: "coding-domain" },
  { id: "research", label: "Research", harness: "research-domain" },
  { id: "design", label: "Design", harness: "design-domain" },
  { id: "security", label: "Security", harness: "security-domain" }
];

function harnessForProfile(profile: RunProfile): string | null {
  if (profile === "auto") {
    return null;
  }
  return runProfiles.find((candidate) => candidate.id === profile)?.harness ?? profile;
}

const authoritySteps: Array<{
  id: string;
  label: string;
  detail: string;
  status: "complete" | "bound" | "waiting" | "idle";
  icon: IconName;
}> = [
  { id: "parliament", label: "Plan · Parliament", detail: "Intent and policy posture recorded", status: "complete", icon: "council" },
  { id: "shadow", label: "Route · Shadow Council", detail: "Harness route selected from evidence", status: "complete", icon: "users" },
  { id: "harness", label: "Harness binding", detail: "Awaiting runtime selection", status: "bound", icon: "box" },
  { id: "gene", label: "Gene binding", detail: "Exact capability selected", status: "bound", icon: "code" },
  { id: "monitor", label: "Approval · ReferenceMonitor", detail: "Exact permit decision", status: "waiting", icon: "shield" },
  { id: "executor", label: "Effects · EffectExecutor", detail: "Awaiting permit", status: "idle", icon: "terminal" },
  { id: "receipt", label: "Receipts", detail: "Created after execution", status: "idle", icon: "archive" },
  { id: "evaluation", label: "Evaluation", detail: "Outcome feedback follows receipts", status: "idle", icon: "graph" }
];

function authorityStepsForRun(lastRun: RuntimeRun | null, events: RuntimeEvent[], runProfile: RunProfile) {
  if (!lastRun) {
    const profile = runProfiles.find((candidate) => candidate.id === runProfile);
    const label = profile?.harness ?? "Auto-selected Harness";
    return authoritySteps.map((step) => step.id === "harness" ? { ...step, label, detail: "Awaiting runtime selection" } : step);
  }
  const eventTypes = new Set(events.map((event) => event.event_type));
  const policyResolved = eventTypes.has("policy_approved") || eventTypes.has("policy_denied") || eventTypes.has("approval_required");
  const denied = lastRun.status === "denied" || lastRun.status === "failed";
  const completed = lastRun.status === "completed";
  const approvalRequired = lastRun.status === "approval_required";
  return [
    { id: "parliament", label: "Plan · Parliament", detail: denied ? "Policy denied the plan" : "Intent and policy decision recorded", status: policyResolved ? "complete" : "bound", icon: "council" },
    { id: "shadow", label: "Route · Shadow Council", detail: "Routing evidence recorded", status: "complete", icon: "users" },
    { id: "harness", label: `Harness · ${lastRun.selected_harness ?? "unselected"}`, detail: "Selected by the runtime", status: "bound", icon: "box" },
    { id: "gene", label: `Gene · ${lastRun.selected_gene ?? "unselected"}`, detail: "Exact capability binding", status: "bound", icon: "code" },
    { id: "monitor", label: "Approval · ReferenceMonitor", detail: approvalRequired ? "Exact approval required" : denied ? "Permit not issued" : "Permit issued and consumed", status: approvalRequired ? "waiting" : denied ? "idle" : "complete", icon: "shield" },
    { id: "executor", label: "Effects · EffectExecutor", detail: completed ? "Bound effects completed" : denied ? "Effects never started" : "Awaiting permit", status: completed ? "complete" : "idle", icon: "terminal" },
    { id: "receipt", label: "Receipts", detail: lastRun.receipt_count ? `${lastRun.receipt_count} receipt${lastRun.receipt_count === 1 ? "" : "s"} recorded` : "No receipt created", status: lastRun.receipt_count ? "complete" : "idle", icon: "archive" },
    { id: "evaluation", label: "Evaluation", detail: completed ? "Outcome feedback committed to durable memory" : denied ? "Failure evidence retained" : "Waiting for terminal outcome", status: completed || denied ? "complete" : "idle", icon: "graph" }
  ] as typeof authoritySteps;
}

function mergeEvolutionDetails(current: RuntimeEvolutionProposal[], incoming: RuntimeEvolutionProposal[]) {
  return incoming.map((proposal) => ({
    ...proposal,
    candidate: current.find((existing) => existing.proposal_id === proposal.proposal_id)?.candidate ?? proposal.candidate,
  }));
}

const viewDetails: Record<Exclude<ViewId, "command" | "runs" | "memory" | "workflows">, { eyebrow: string; title: string; description: string }> = {
  council: {
    eyebrow: "Governance",
    title: "Council",
    description: "Inspect policy decisions, routing evidence, and pending approvals."
  },
  capabilities: {
    eyebrow: "Runtime surface",
    title: "Harnesses & Genes",
    description: "Installed Harnesses, Genes, Skills, versions, and admission state."
  },
  engines: {
    eyebrow: "Architecture",
    title: "Engines",
    description: "Pandora’s bounded engines and the authority each one owns."
  },
  tools: {
    eyebrow: "Runtime surface",
    title: "Built-in Tools",
    description: "Tool definitions and effect classifications exposed by ToolEngine."
  },
  connections: {
    eyebrow: "Runtime surface",
    title: "Connections",
    description: "Provider profiles, local MCP servers, and connection health."
  },
  audit: {
    eyebrow: "Evidence",
    title: "Audit",
    description: "Receipts, evaluations, redacted events, and execution lineage."
  },
  evolution: {
    eyebrow: "Governed improvement",
    title: "Evolution",
    description: "Evidence-backed proposals remain separate from permission and activation."
  },
  settings: {
    eyebrow: "Workspace",
    title: "Settings",
    description: "Configure policy, containment, providers, and local workspace behavior."
  }
};

function Icon({ name, size = 17 }: { name: IconName; size?: number }) {
  const common = {
    width: size,
    height: size,
    viewBox: "0 0 24 24",
    fill: "none",
    stroke: "currentColor",
    strokeWidth: 1.7,
    strokeLinecap: "round" as const,
    strokeLinejoin: "round" as const,
    "aria-hidden": true
  };

  switch (name) {
    case "activity":
      return <svg {...common}><path d="M3 12h4l2.2-7 4.1 14 2.3-7H21" /></svg>;
    case "archive":
      return <svg {...common}><path d="M4 7h16v13H4z" /><path d="M3 4h18v3H3zM9 11h6" /></svg>;
    case "arrow":
      return <svg {...common}><path d="M5 12h13M13 6l6 6-6 6" /></svg>;
    case "book":
      return <svg {...common}><path d="M5 4h12a2 2 0 0 1 2 2v14H7a2 2 0 0 0-2 2z" /><path d="M5 4v16a2 2 0 0 1 2-2h12" /></svg>;
    case "box":
      return <svg {...common}><path d="m12 3 8 4.5v9L12 21l-8-4.5v-9z" /><path d="m4 7.5 8 4.5 8-4.5M12 12v9" /></svg>;
    case "check":
      return <svg {...common}><path d="m5 12 4 4L19 6" /></svg>;
    case "chevron":
      return <svg {...common}><path d="m9 5 7 7-7 7" /></svg>;
    case "clock":
      return <svg {...common}><circle cx="12" cy="12" r="8.5" /><path d="M12 7v5l3 2" /></svg>;
    case "code":
      return <svg {...common}><path d="m8 8-4 4 4 4M16 8l4 4-4 4M14 5l-4 14" /></svg>;
    case "copy":
      return <svg {...common}><rect x="8" y="8" width="11" height="11" rx="2" /><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2" /></svg>;
    case "council":
      return <svg {...common}><circle cx="12" cy="6" r="2.5" /><circle cx="6" cy="16" r="2.5" /><circle cx="18" cy="16" r="2.5" /><path d="m10.5 8-3 5.5M13.5 8l3 5.5M8.5 16h7" /></svg>;
    case "dots":
      return <svg {...common}><circle cx="5" cy="12" r="1" fill="currentColor" /><circle cx="12" cy="12" r="1" fill="currentColor" /><circle cx="19" cy="12" r="1" fill="currentColor" /></svg>;
    case "download":
      return <svg {...common}><path d="M12 3v12M7 10l5 5 5-5M5 20h14" /></svg>;
    case "evolution":
      return <svg {...common}><path d="M5 5h5v5H5zM14 14h5v5h-5z" /><path d="M10 7.5h3a2 2 0 0 1 2 2V14M14 16.5h-3a2 2 0 0 1-2-2V10" /></svg>;
    case "gear":
      return <svg {...common}><path d="M12 3v2M12 19v2M3 12h2M19 12h2M5.6 5.6 7 7M17 17l1.4 1.4M18.4 5.6 17 7M7 17l-1.4 1.4" /><circle cx="12" cy="12" r="4" /></svg>;
    case "graph":
      return <svg {...common}><circle cx="5" cy="12" r="2" /><circle cx="17" cy="6" r="2" /><circle cx="19" cy="17" r="2" /><path d="m6.8 11 8.4-4M6.8 13l10.4 3" /></svg>;
    case "grid":
      return <svg {...common}><rect x="4" y="4" width="6" height="6" rx="1" /><rect x="14" y="4" width="6" height="6" rx="1" /><rect x="4" y="14" width="6" height="6" rx="1" /><rect x="14" y="14" width="6" height="6" rx="1" /></svg>;
    case "lock":
      return <svg {...common}><rect x="5" y="10" width="14" height="10" rx="2" /><path d="M8 10V7a4 4 0 0 1 8 0v3" /></svg>;
    case "menu":
      return <svg {...common}><path d="M4 7h16M4 12h16M4 17h16" /></svg>;
    case "plus":
      return <svg {...common}><path d="M12 5v14M5 12h14" /></svg>;
    case "search":
      return <svg {...common}><circle cx="10.8" cy="10.8" r="6.3" /><path d="m16 16 4 4" /></svg>;
    case "shield":
      return <svg {...common}><path d="M12 3 19 6v5c0 4.5-3 8-7 10-4-2-7-5.5-7-10V6z" /><path d="m9 12 2 2 4-4" /></svg>;
    case "spark":
      return <svg {...common}><path d="m12 3 1.4 5.6L19 10l-5.6 1.4L12 17l-1.4-5.6L5 10l5.6-1.4zM19 16l.6 2.4L22 19l-2.4.6L19 22l-.6-2.4L16 19l2.4-.6z" /></svg>;
    case "stack":
      return <svg {...common}><path d="m12 4 8 4-8 4-8-4zM4 12l8 4 8-4M4 16l8 4 8-4" /></svg>;
    case "terminal":
      return <svg {...common}><rect x="3" y="5" width="18" height="14" rx="2" /><path d="m7 10 2 2-2 2M12 14h4" /></svg>;
    case "users":
      return <svg {...common}><circle cx="9" cy="8" r="3" /><path d="M3.5 19a5.5 5.5 0 0 1 11 0M16 5.5a3 3 0 0 1 0 5.8M16 14a5 5 0 0 1 4.5 5" /></svg>;
  }
}

function Chip({ children, tone = "neutral", icon }: { children: ReactNode; tone?: "neutral" | "green" | "amber" | "blue" | "gold"; icon?: IconName }) {
  return <span className={`chip chip-${tone}`}>{icon ? <Icon name={icon} size={12} /> : null}{children}</span>;
}

function Panel({ className = "", children }: { className?: string; children: ReactNode }) {
  return <section className={`panel ${className}`}>{children}</section>;
}

function App() {
  const [activeView, setActiveView] = useState<ViewId>("command");
  const [approvalPreview, setApprovalPreview] = useState(false);
  const [selectedStep, setSelectedStep] = useState("monitor");
  const [endpoint, setEndpoint] = useState(loadRuntimeEndpoint);
  const [token, setToken] = useState("");
  const [runtimeStatus, setRuntimeStatus] = useState<RuntimeStatus>("preview");
  const [serviceActive, setServiceActive] = useState(false);
  const [runtimeError, setRuntimeError] = useState("");
  const [runtimeHealth, setRuntimeHealth] = useState<RuntimeHealth | null>(null);
  const [sessions, setSessions] = useState<RuntimeSession[]>([]);
  const [selectedSessionId, setSelectedSessionId] = useState("");
  const [selectedSession, setSelectedSession] = useState<RuntimeSessionDetail | null>(null);
  const [events, setEvents] = useState<RuntimeEvent[]>([]);
  const [memoryRecords, setMemoryRecords] = useState<RuntimeMemoryRecord[]>([]);
  const [orchestrationRuns, setOrchestrationRuns] = useState<RuntimeOrchestrationRun[]>([]);
  const [harnesses, setHarnesses] = useState<RuntimeHarness[]>([]);
  const [engines, setEngines] = useState<RuntimeEngine[]>([]);
  const [tools, setTools] = useState<RuntimeTool[]>([]);
  const [providers, setProviders] = useState<RuntimeProvider[]>([]);
  const [evolutionProposals, setEvolutionProposals] = useState<RuntimeEvolutionProposal[]>([]);
  const [artifactActivations, setArtifactActivations] = useState<RuntimeArtifactActivation[]>([]);
  const [lastRun, setLastRun] = useState<RuntimeRun | null>(null);
  const [pendingRun, setPendingRun] = useState<PendingRunRequest | null>(null);
  const [workspaceInspection, setWorkspaceInspection] = useState<RuntimeRun | null>(null);
  const [pendingWorkspaceInspection, setPendingWorkspaceInspection] = useState<WorkspaceInspectionRequest | null>(null);
  const [workspaceInspectionInFlight, setWorkspaceInspectionInFlight] = useState(false);
  const [runInFlight, setRunInFlight] = useState(false);
  const [runProfile, setRunProfile] = useState<RunProfile>("auto");
  const [theme, setTheme] = useState<ThemeMode>(loadTheme);
  const [workflows, setWorkflows] = useState<WorkflowRecipe[]>(loadWorkflows);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const autoStartAttempted = useRef(false);
  const native = isNativeRuntime();
  const clientState = useMemo(() => {
    if (!endpoint || (!token && !(native && endpoint === nativeEndpoint))) {
      return { client: null, error: "" };
    }
    try {
      return { client: new RuntimeClient(endpoint, token), error: "" };
    } catch (error) {
      return { client: null, error: error instanceof Error ? error.message : "Invalid service endpoint" };
    }
  }, [endpoint, native, token]);
  const client = clientState.client;

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    window.localStorage.setItem(themeStorageKey, theme);
  }, [theme]);

  useEffect(() => {
    window.localStorage.setItem(workflowStorageKey, JSON.stringify(workflows));
  }, [workflows]);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setPaletteOpen((open) => !open);
      } else if (event.key === "Escape") {
        setPaletteOpen(false);
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  useEffect(() => {
    if (!client) {
      setRuntimeStatus(clientState.error ? "offline" : "preview");
      setRuntimeError(clientState.error);
      setRuntimeHealth(null);
      setSessions([]);
      setSelectedSessionId("");
      setSelectedSession(null);
      setEvents([]);
      setMemoryRecords([]);
      setOrchestrationRuns([]);
      setHarnesses([]);
      setEngines([]);
      setTools([]);
      setProviders([]);
      setEvolutionProposals([]);
      setArtifactActivations([]);
      setPendingRun(null);
      setWorkspaceInspection(null);
      setPendingWorkspaceInspection(null);
      return;
    }
    let cancelled = false;
    setRuntimeStatus("checking");
    setRuntimeError("");
    Promise.all([client.health(), client.sessions(), client.orchestrations(), client.capabilities(), client.engines(), client.tools(), client.providers(), client.evolution(), client.evolutionActivations()])
      .then(([health, nextSessions, nextOrchestrationRuns, nextHarnesses, nextEngines, nextTools, nextProviders, nextEvolutionProposals, nextArtifactActivations]) => {
        if (!cancelled) {
          setRuntimeHealth(health);
          setSessions(nextSessions);
          setOrchestrationRuns(nextOrchestrationRuns);
          setHarnesses(nextHarnesses);
          setEngines(nextEngines);
          setTools(nextTools);
          setProviders(nextProviders);
          setEvolutionProposals(nextEvolutionProposals);
          setArtifactActivations(nextArtifactActivations);
          setRuntimeStatus("connected");
        }
      })
      .catch((error: unknown) => {
        if (!cancelled) {
          setRuntimeStatus("offline");
          setRuntimeError(error instanceof Error ? error.message : "Could not reach Pandora service");
        }
      });
    return () => {
      cancelled = true;
    };
  }, [client, clientState.error]);

  useEffect(() => {
    if (!client || !selectedSessionId) {
      return;
    }
    let cancelled = false;
    const refresh = async () => {
      try {
        const [detail, nextEvents, nextSessions, nextMemory] = await Promise.all([
          client.inspectSession(selectedSessionId),
          client.events(selectedSessionId),
          client.sessions(),
          client.memory(selectedSessionId),
        ]);
        if (!cancelled) {
          setSelectedSession(detail);
          setEvents(nextEvents);
          setMemoryRecords(nextMemory);
          setSessions(nextSessions);
        }
      } catch {
      }
    };
    const interval = window.setInterval(() => void refresh(), 2000);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, [client, selectedSessionId]);

  useEffect(() => {
    if (runtimeStatus === "connected" && runProfile !== "auto" && !harnesses.some((harness) => harness.id === harnessForProfile(runProfile))) {
      setRunProfile("auto");
    }
  }, [harnesses, runProfile, runtimeStatus]);

  useEffect(() => {
    if (!client || runtimeStatus !== "connected" || activeView !== "evolution") {
      return;
    }
    let cancelled = false;
    const refresh = async () => {
      try {
        const [proposals, activations] = await Promise.all([client.evolution(), client.evolutionActivations()]);
        if (!cancelled) {
          setEvolutionProposals((current) => mergeEvolutionDetails(current, proposals));
          setArtifactActivations(activations);
        }
      } catch {
      }
    };
    void refresh();
    const interval = window.setInterval(() => void refresh(), 3000);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, [activeView, client, runtimeStatus]);

  useEffect(() => {
    if (!client || runtimeStatus !== "connected" || activeView !== "runs") {
      return;
    }
    let cancelled = false;
    const refresh = async () => {
      try {
        const runs = await client.orchestrations();
        if (!cancelled) {
          setOrchestrationRuns(runs);
        }
      } catch {
      }
    };
    void refresh();
    const interval = window.setInterval(() => void refresh(), 2000);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, [activeView, client, runtimeStatus]);

  const connect = (nextEndpoint: string, nextToken: string) => {
    setEndpoint(nextEndpoint);
    setToken(nextToken);
    saveRuntimeEndpoint(nextEndpoint);
  };

  const startService = async () => {
    setRuntimeStatus("checking");
    setRuntimeError("");
    setRuntimeHealth(null);
    try {
      const nextEndpoint = await startLocalService();
      setEndpoint(nextEndpoint);
      setToken("native-session");
      setServiceActive(true);
      saveRuntimeEndpoint(nextEndpoint);
    } catch (error: unknown) {
      setRuntimeStatus("offline");
      setRuntimeError(error instanceof Error ? error.message : "Could not start Pandora service");
      setServiceActive(false);
    }
  };

  const stopService = async () => {
    setRuntimeError("");
    setRuntimeHealth(null);
    try {
      await stopLocalService();
      setServiceActive(false);
      setEndpoint("");
      setToken("");
      saveRuntimeEndpoint("");
      setSessions([]);
      setSelectedSessionId("");
      setSelectedSession(null);
      setEvents([]);
      setOrchestrationRuns([]);
      setHarnesses([]);
      setEngines([]);
      setTools([]);
      setProviders([]);
      setEvolutionProposals([]);
      setArtifactActivations([]);
      setPendingRun(null);
      setWorkspaceInspection(null);
      setPendingWorkspaceInspection(null);
      setRuntimeStatus("preview");
    } catch (error: unknown) {
      setRuntimeStatus("offline");
      setRuntimeError(error instanceof Error ? error.message : "Could not stop Pandora service");
    }
  };

  useEffect(() => {
    if (!native || autoStartAttempted.current || serviceActive || endpoint === nativeEndpoint) {
      return;
    }
    autoStartAttempted.current = true;
    void startService();
  }, [endpoint, native, serviceActive]);

  const openSession = async (sessionId: string) => {
    if (!client) {
      return;
    }
    setRuntimeStatus("checking");
    setRuntimeError("");
    try {
      const [detail, nextEvents, nextMemory] = await Promise.all([
        client.inspectSession(sessionId),
        client.events(sessionId),
        client.memory(sessionId),
      ]);
      setLastRun(null);
      setPendingRun(null);
      setSelectedSessionId(sessionId);
      setSelectedSession(detail);
      setEvents(nextEvents);
      setMemoryRecords(nextMemory);
      setRuntimeStatus("connected");
    } catch (error: unknown) {
      setRuntimeStatus("offline");
      setRuntimeError(error instanceof Error ? error.message : "Could not inspect Pandora session");
    }
  };

  const runTask = async (task: string, profile: RunProfile = runProfile, contextAttachments: RuntimeContextAttachment[] = []) => {
    if (!client) {
      throw new Error("Connect to the local Pandora service first");
    }
    setRunInFlight(true);
    setRuntimeStatus("checking");
    setRuntimeError("");
    try {
      const requestedHarness = harnessForProfile(profile);
      const result = await client.agentRun(task, selectedSessionId || null, requestedHarness, contextAttachments);
      setPendingRun(result.approval ? { task, requestedHarness } : null);
      await loadRunResult(result);
      setRuntimeStatus("connected");
    } catch (error: unknown) {
      setRuntimeStatus("offline");
      const message = error instanceof Error ? error.message : "Pandora run failed";
      setRuntimeError(message);
      throw error;
    } finally {
      setRunInFlight(false);
    }
  };

  const loadRunResult = async (result: RuntimeRun) => {
    setLastRun(result);
    const [nextSessions, detail, nextEvents, nextMemory] = await Promise.all([
      client!.sessions(),
      client!.inspectSession(result.session_id),
      client!.events(result.session_id),
      client!.memory(result.session_id),
    ]);
    setSessions(nextSessions);
    setSelectedSessionId(result.session_id);
    setSelectedSession(detail);
    setEvents(nextEvents);
    setMemoryRecords(nextMemory);
  };

  const resolvePendingApproval = async (allow: boolean) => {
    const approval = lastRun?.approval;
    if (!client || !approval || (lastRun.mode === "direct" && !pendingRun)) {
      throw new Error("No resumable approval is available");
    }
    setRunInFlight(true);
    setRuntimeStatus("checking");
    setRuntimeError("");
    try {
      const resolved = approval.status === "pending"
        ? await client.resolveApproval(approval.approval_id, allow)
        : approval;
      setLastRun({ ...lastRun, approval: resolved });
      if (allow) {
        const result = lastRun.mode === "agent"
          ? await client.agentResume(approval.approval_id)
          : await client.resume(
              approval.approval_id,
              pendingRun!.task,
              pendingRun!.requestedHarness,
            );
        setPendingRun(null);
        await loadRunResult(result);
      } else {
        setPendingRun(null);
        setLastRun({
          ...lastRun,
          status: "denied",
          status_detail: "The operator denied this exact request.",
          approval: resolved,
        });
      }
      setRuntimeStatus("connected");
    } catch (error: unknown) {
      setRuntimeStatus("offline");
      const message = error instanceof Error ? error.message : "Could not resolve Pandora approval";
      setRuntimeError(message);
      throw error;
    } finally {
      setRunInFlight(false);
    }
  };

  const inspectWorkspace = async (task: string): Promise<void> => {
    if (!client) {
      throw new Error("Connect to the local Pandora service first");
    }
    const request = { task, requestedHarness: "coding-domain" };
    setWorkspaceInspectionInFlight(true);
    try {
      const result = await client.run(request.task, request.requestedHarness);
      setWorkspaceInspection(result);
      setPendingWorkspaceInspection(result.approval ? request : null);
      setSessions(await client.sessions());
    } finally {
      setWorkspaceInspectionInFlight(false);
    }
  };

  const resolveWorkspaceInspection = async (allow: boolean): Promise<void> => {
    const approval = workspaceInspection?.approval;
    if (!client || !workspaceInspection || !approval || !pendingWorkspaceInspection) {
      throw new Error("No workspace inspection approval is available");
    }
    setWorkspaceInspectionInFlight(true);
    try {
      const resolved = approval.status === "pending"
        ? await client.resolveApproval(approval.approval_id, allow)
        : approval;
      setWorkspaceInspection({ ...workspaceInspection, approval: resolved });
      if (allow) {
        const result = await client.resume(
          approval.approval_id,
          pendingWorkspaceInspection.task,
          pendingWorkspaceInspection.requestedHarness,
        );
        setWorkspaceInspection(result);
      } else {
        setWorkspaceInspection({
          ...workspaceInspection,
          status: "denied",
          status_detail: "The operator denied this exact inspection request.",
          approval: resolved,
        });
      }
      setPendingWorkspaceInspection(null);
      setSessions(await client.sessions());
    } finally {
      setWorkspaceInspectionInFlight(false);
    }
  };

  const mutateEvolution = async (
    operation: "activate" | "rollback",
    proposalId: string,
    confirmation: string,
    reason: string,
  ): Promise<RuntimeEvolutionMutation> => {
    if (!client) {
      throw new Error("Connect to the local Pandora service first");
    }
    const mutation = operation === "activate"
      ? await client.activateEvolution(proposalId, confirmation)
      : await client.rollbackEvolution(proposalId, confirmation, reason);
    const [proposals, activations] = await Promise.all([client.evolution(), client.evolutionActivations()]);
    setEvolutionProposals((current) => mergeEvolutionDetails(current, proposals));
    setArtifactActivations(activations);
    return mutation;
  };

  const inspectEvolutionCandidate = async (proposalId: string): Promise<RuntimeEvolutionProposal> => {
    if (!client) {
      throw new Error("Connect to the local Pandora service first");
    }
    const detail = await client.inspectEvolution(proposalId);
    setEvolutionProposals((current) => current.map((proposal) => proposal.proposal_id === proposalId ? detail : proposal));
    return detail;
  };

  const mutateOrchestration = async (
    operation: "cancel" | "resume",
    runId: string,
    confirmation: string,
  ): Promise<RuntimeOrchestrationRun> => {
    if (!client) {
      throw new Error("Connect to the local Pandora service first");
    }
    const run = operation === "cancel"
      ? await client.cancelOrchestration(runId, confirmation)
      : await client.resumeOrchestration(runId, confirmation);
    setOrchestrationRuns((current) => current.map((item) => item.run_id === run.run_id ? run : item));
    return run;
  };

  const createWorkflow = (name: string, task: string, profile: RunProfile) => {
    const recipe = { id: crypto.randomUUID(), name, task, profile };
    setWorkflows((current) => [recipe, ...current].slice(0, 24));
  };

  const removeWorkflow = (id: string) => {
    setWorkflows((current) => current.filter((workflow) => workflow.id !== id));
  };

  const runWorkflow = async (workflow: WorkflowRecipe) => {
    setActiveView("command");
    await runTask(workflow.task, workflow.profile);
  };

  return (
    <div className="app-shell">
      <Sidebar activeView={activeView} onSelect={setActiveView} runtimeStatus={runtimeStatus} sessions={sessions} selectedSessionId={selectedSessionId} onOpenPalette={() => setPaletteOpen(true)} onOpenSession={async (sessionId) => { setActiveView("command"); await openSession(sessionId); }} />
      <main className="main-shell">
        <TopBar activeView={activeView} runtimeStatus={runtimeStatus} onOpenPalette={() => setPaletteOpen(true)} />
        {paletteOpen ? <CommandPalette onClose={() => setPaletteOpen(false)} onSelectView={(view) => { setActiveView(view); setPaletteOpen(false); }} /> : null}
        {activeView === "command" ? (
          <CommandView
            approvalPreview={approvalPreview}
            selectedStep={selectedStep}
            onApprovalPreview={() => setApprovalPreview(true)}
            onApprovalClose={() => setApprovalPreview(false)}
            onSelectStep={setSelectedStep}
            runtimeStatus={runtimeStatus}
            selectedSession={selectedSession}
            lastRun={lastRun}
            events={events}
            harnesses={harnesses}
            runInFlight={runInFlight}
            workspaceInspection={workspaceInspection}
            workspaceInspectionInFlight={workspaceInspectionInFlight}
            runProfile={runProfile}
            onRunProfileChange={setRunProfile}
            onRun={runTask}
            onResolveApproval={resolvePendingApproval}
            onInspectWorkspace={inspectWorkspace}
            onResolveWorkspaceInspection={resolveWorkspaceInspection}
          />
        ) : activeView === "runs" ? (
          <RunsView runs={orchestrationRuns} runtimeStatus={runtimeStatus} onMutate={mutateOrchestration} />
        ) : activeView === "memory" ? (
          <MemoryView runtimeStatus={runtimeStatus} records={memoryRecords} selectedSession={selectedSession} />
        ) : activeView === "workflows" ? (
          <WorkflowsView runtimeStatus={runtimeStatus} workflows={workflows} harnesses={harnesses} onOpenCommand={() => setActiveView("command")} onCreate={createWorkflow} onRemove={removeWorkflow} onRun={runWorkflow} />
        ) : activeView === "connections" ? (
          <ConnectionView endpoint={endpoint} runtimeStatus={runtimeStatus} runtimeError={runtimeError} health={runtimeHealth} providers={providers} sessions={sessions} selectedSessionId={selectedSessionId} selectedSession={selectedSession} native={native} serviceActive={serviceActive} onConnect={connect} onStartService={startService} onStopService={stopService} onSelectSession={openSession} />
        ) : activeView === "audit" ? (
          <AuditView events={events} selectedSession={selectedSession} runtimeStatus={runtimeStatus} />
        ) : activeView === "capabilities" ? (
          <CapabilitiesView harnesses={harnesses} tools={tools} runtimeStatus={runtimeStatus} />
        ) : activeView === "engines" ? (
          <EnginesView engines={engines} runtimeStatus={runtimeStatus} />
        ) : activeView === "tools" ? (
          <ToolsView tools={tools} runtimeStatus={runtimeStatus} />
        ) : activeView === "evolution" ? (
          <EvolutionView proposals={evolutionProposals} activations={artifactActivations} runtimeStatus={runtimeStatus} onInspect={inspectEvolutionCandidate} onMutate={mutateEvolution} />
        ) : activeView === "settings" ? (
          <SettingsView theme={theme} onThemeChange={setTheme} runtimeStatus={runtimeStatus} health={runtimeHealth} native={native} endpoint={endpoint} />
        ) : (
          <SecondaryView view={activeView} runtimeStatus={runtimeStatus} />
        )}
      </main>
    </div>
  );
}

function Sidebar({ activeView, onSelect, runtimeStatus, sessions, selectedSessionId, onOpenPalette, onOpenSession }: { activeView: ViewId; onSelect: (view: ViewId) => void; runtimeStatus: RuntimeStatus; sessions: RuntimeSession[]; selectedSessionId: string; onOpenPalette: () => void; onOpenSession: (sessionId: string) => Promise<void> }) {
  const threads = sessions.map((session) => ({ title: session.session_id, meta: session.workspace_id, sessionId: session.session_id, active: selectedSessionId === session.session_id }));
  return (
    <aside className="sidebar">
      <div className="window-controls" aria-label="Window controls">
        <span className="window-dot window-dot-red" />
        <span className="window-dot window-dot-yellow" />
        <span className="window-dot window-dot-green" />
        <div className="sidebar-tools"><button className="icon-button" aria-label="Open workspace menu" onClick={onOpenPalette}><Icon name="menu" size={16} /></button><button className="icon-button" aria-label="Open settings" onClick={() => onSelect("settings")}><Icon name="gear" size={16} /></button></div>
      </div>
      <div className="brand-lockup">
        <div className="brand-mark"><span /></div>
        <div><strong>Pandora</strong><span>governed runtime</span></div>
      </div>
      <button className="workspace-switcher" onClick={() => onSelect("settings")}>
        <span className="workspace-avatar">O</span>
        <span className="workspace-copy"><strong>Pandora workspace</strong><small>Local workspace</small></span>
        <Icon name="chevron" size={14} />
      </button>
      <button type="button" className="rail-search" onClick={onOpenPalette}><Icon name="search" size={15} /><span>Search workspace</span><kbd>⌘ K</kbd></button>
      <nav className="navigation" aria-label="Pandora navigation">
        {navigation.map((group) => <div className="nav-group" key={group.label}>
          <span className="nav-label">{group.label}</span>
          {group.items.map((item) => <button className={`nav-item ${activeView === item.id ? "is-active" : ""}`} key={item.id} onClick={() => onSelect(item.id)} aria-current={activeView === item.id ? "page" : undefined}>
            <Icon name={item.icon} size={17} /><span>{item.label}</span>
          </button>)}
        </div>)}
      </nav>
      <div className="recent-section">
        <div className="recent-heading"><span className="nav-label">{sessions.length ? "Live sessions" : "Sessions"}</span><button className="text-icon-button" aria-label="New thread" onClick={onOpenPalette}><Icon name="plus" size={15} /></button></div>
        <div className="thread-list">
          {threads.length ? threads.map((thread) => <button className={`thread-item ${thread.active && activeView === "command" ? "is-current" : ""}`} key={thread.title} onClick={() => void onOpenSession(thread.sessionId)}>
            <span className="thread-title">{thread.title}</span><span className="thread-meta">{thread.meta}</span>
          </button>) : <span className="thread-meta">No recorded sessions</span>}
        </div>
      </div>
      <div className="sidebar-footer">
        <div className={`footer-status footer-status-${runtimeStatus}`}><span className={`status-pulse status-pulse-${runtimeStatus}`} /> <span>{runtimeStatusLabel(runtimeStatus)}</span></div>
        <div className="footer-meta"><span>Supervised</span><span className="footer-separator">·</span><span className="mono">v2.0.0-beta.7</span></div>
      </div>
    </aside>
  );
}

function TopBar({ activeView, runtimeStatus, onOpenPalette }: { activeView: ViewId; runtimeStatus: RuntimeStatus; onOpenPalette: () => void }) {
  const label = activeView === "command" ? "Command Center" : activeView === "runs" ? "Background Runs" : activeView[0].toUpperCase() + activeView.slice(1);
  const tone = runtimeStatus === "connected" ? "green" : runtimeStatus === "offline" ? "amber" : "blue";
  return <header className="top-bar"><div className="breadcrumb"><span className="breadcrumb-muted">Pandora</span><Icon name="chevron" size={13} /><strong>{label}</strong></div><div className="top-actions"><Chip tone={tone} icon="lock">{runtimeStatusLabel(runtimeStatus)}</Chip><button className="icon-button" type="button" aria-label="Search" onClick={onOpenPalette}><Icon name="search" size={17} /></button><button className="icon-button" type="button" aria-label="More options" disabled><Icon name="dots" size={18} /></button><div className="operator-avatar" aria-label="Operator profile">AK</div></div></header>;
}

function runtimeStatusLabel(status: RuntimeStatus): string {
  switch (status) {
    case "connected":
      return "Runtime connected";
    case "checking":
      return "Connecting";
    case "offline":
      return "Runtime offline";
    case "preview":
      return "Local preview";
  }
}

function CommandView({ approvalPreview, selectedStep, onApprovalPreview, onApprovalClose, onSelectStep, runtimeStatus, selectedSession, lastRun, events, harnesses, runInFlight, workspaceInspection, workspaceInspectionInFlight, runProfile, onRunProfileChange, onRun, onResolveApproval, onInspectWorkspace, onResolveWorkspaceInspection }: { approvalPreview: boolean; selectedStep: string; onApprovalPreview: () => void; onApprovalClose: () => void; onSelectStep: (id: string) => void; runtimeStatus: RuntimeStatus; selectedSession: RuntimeSessionDetail | null; lastRun: RuntimeRun | null; events: RuntimeEvent[]; harnesses: RuntimeHarness[]; runInFlight: boolean; workspaceInspection: RuntimeRun | null; workspaceInspectionInFlight: boolean; runProfile: RunProfile; onRunProfileChange: (profile: RunProfile) => void; onRun: (task: string, profile: RunProfile, contextAttachments: RuntimeContextAttachment[]) => Promise<void>; onResolveApproval: (allow: boolean) => Promise<void>; onInspectWorkspace: (task: string) => Promise<void>; onResolveWorkspaceInspection: (allow: boolean) => Promise<void> }) {
  const [task, setTask] = useState("");
  const [runError, setRunError] = useState("");
  const [contextAttachments, setContextAttachments] = useState<RuntimeContextAttachment[]>([]);
  const contextInput = useRef<HTMLInputElement>(null);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const nextTask = task.trim();
    if (!nextTask || runInFlight) {
      return;
    }
    setRunError("");
    try {
      await onRun(nextTask, runProfile, contextAttachments);
      setTask("");
      setContextAttachments([]);
    } catch (error: unknown) {
      setRunError(error instanceof Error ? error.message : "Pandora run failed");
    }
  };

  const handleTaskKeyDown = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
      event.preventDefault();
      event.currentTarget.form?.requestSubmit();
    }
  };

  const addContextAttachments = async (event: ChangeEvent<HTMLInputElement>) => {
    const files = Array.from(event.target.files ?? []);
    event.target.value = "";
    if (!files.length) {
      return;
    }
    try {
      if (contextAttachments.length + files.length > maxContextAttachments) {
        throw new Error(`Attach at most ${maxContextAttachments} text files`);
      }
      let totalBytes = contextAttachments.reduce((total, attachment) => total + attachmentByteLength(attachment.content), 0);
      const nextAttachments: RuntimeContextAttachment[] = [];
      for (const file of files) {
        if (!isTextAttachment(file)) {
          throw new Error(`${file.name} is not a supported text or source file`);
        }
        if (!file.size || file.size > maxContextAttachmentBytes) {
          throw new Error(`${file.name} must be between 1 byte and 16 KiB`);
        }
        const content = await readTextAttachment(file);
        const contentBytes = attachmentByteLength(content);
        if (!contentBytes || contentBytes > maxContextAttachmentBytes || content.includes("\0")) {
          throw new Error(`${file.name} is empty, binary, or exceeds 16 KiB after decoding`);
        }
        totalBytes += contentBytes;
        if (totalBytes > maxContextBytes) {
          throw new Error("Selected context exceeds the 24 KiB request limit");
        }
        nextAttachments.push({
          name: file.name,
          media_type: contextMediaType(file),
          content,
        });
      }
      setContextAttachments((current) => [...current, ...nextAttachments]);
      setRunError("");
    } catch (error: unknown) {
      setRunError(error instanceof Error ? error.message : "Could not attach the selected context");
    }
  };

  const removeContextAttachment = (index: number) => {
    setContextAttachments((current) => current.filter((_, candidateIndex) => candidateIndex !== index));
  };

  const connected = runtimeStatus === "connected";
  const profileOptions = connected && harnesses.length ? [{ id: "auto", label: "Auto route" }, ...harnesses.filter((harness) => harness.runnable).map((harness) => ({ id: harness.id, label: harness.name }))] : runProfiles;
  const coreTitle = runInFlight ? "Pandora is executing" : lastRun ? `Run ${lastRun.status}` : selectedSession ? "Session ready to inspect" : connected ? "Ready for a governed run" : "Awaiting your decision";
  const coreDescription = runInFlight ? "The request is in the governed runtime; wait for its recorded result." : lastRun ? `${lastRun.receipt_count} receipts · ${lastRun.event_count} events${lastRun.mode === "agent" ? ` · ${lastRun.turns ?? 0} turns · ${lastRun.tool_calls ?? 0} tools` : ""} recorded.` : selectedSession ? `${selectedSession.event_count} recorded events · ${selectedSession.session.workspace_id}` : connected ? "Connected to the local Pandora service." : "Connect the local service to submit a real governed run.";
  const steps = authorityStepsForRun(lastRun, events, runProfile);
  const policyValue = lastRun?.status === "denied" ? "Denied" : lastRun?.status === "approval_required" ? "Approval" : connected ? "Supervised" : "Waiting";
  const policyDetail = lastRun ? "run decision" : "Parliament + monitor";
  const evidenceValue = lastRun ? String(lastRun.receipt_count) : selectedSession ? String(selectedSession.event_count) : "None";
  const evidenceDetail = lastRun ? "receipts" : selectedSession ? "events recorded" : "after execution";

  return <div className="command-layout" aria-busy={runInFlight}>
    <section className="core-column">
      <div className="stage-toolbar"><div><span className="eyebrow">ACTIVE WORKSPACE</span><strong>Pandora / local</strong></div><div className="stage-controls"><Chip tone={connected ? "green" : runtimeStatus === "offline" ? "amber" : "blue"} icon="activity">{runtimeStatusLabel(runtimeStatus)}</Chip><button className="icon-button" type="button" aria-label="Workspace options" disabled><Icon name="dots" size={17} /></button></div></div>
      <div className="core-stage">
        <div className="stage-grid" />
        <div className="ambient-glow ambient-glow-one" /><div className="ambient-glow ambient-glow-two" />
        <div className={`pandora-vessel ${approvalPreview ? "vessel-approved" : "vessel-waiting"}`} aria-label={`Pandora runtime ${lastRun?.status ?? (connected ? "ready" : "awaiting connection")}`}>
          <div className="vessel-orbit vessel-orbit-one" /><div className="vessel-orbit vessel-orbit-two" /><div className="vessel-core"><div className="core-symbol"><span /><span /><span /></div></div>
        </div>
        <div className="core-status-dock">
          <div className="core-caption"><span className="eyebrow">PANDORA CORE</span><h1>{approvalPreview ? "Approval recorded in preview" : coreTitle}</h1><p>{approvalPreview ? "No permit issued · runtime service is not connected to this preview." : coreDescription}</p></div>
          <div className="core-metrics"><Metric label="Context" value={selectedSession ? "Scoped" : "None"} detail={selectedSession ? selectedSession.session.workspace_id : "not loaded"} /><Metric label="Policy" value={policyValue} detail={policyDetail} /><Metric label="Evidence" value={evidenceValue} detail={evidenceDetail} /></div>
        </div>
      </div>
      <form className="composer-wrap" onSubmit={submit}>
        {contextAttachments.length ? <div className="context-attachments" aria-label="Selected context files">
          <div className="context-attachments-heading"><span><Icon name="archive" size={13} /> Context evidence</span><small>Untrusted · no authority · {attachmentByteLength(contextAttachments.map((attachment) => attachment.content).join(""))} / {maxContextBytes} bytes</small></div>
          <div className="context-attachment-list">{contextAttachments.map((attachment, index) => <span className="context-attachment" key={`${attachment.name}-${index}`}><span><strong>{attachment.name}</strong><small>{attachmentByteLength(attachment.content)} bytes</small></span><button type="button" aria-label={`Remove ${attachment.name}`} onClick={() => removeContextAttachment(index)} disabled={runInFlight}>×</button></span>)}</div>
        </div> : null}
        <div className="composer">
          <input ref={contextInput} className="sr-only" type="file" multiple accept="text/*,.c,.cc,.cpp,.css,.csv,.go,.h,.hpp,.html,.java,.js,.json,.jsx,.md,.mjs,.py,.rb,.rs,.sh,.sql,.svg,.toml,.ts,.tsx,.xml,.yaml,.yml" aria-label="Choose context files" onChange={(event) => void addContextAttachments(event)} disabled={runInFlight} />
          <button type="button" className="composer-add" aria-label="Add context files" title="Attach bounded text or source files as untrusted evidence" onClick={() => contextInput.current?.click()} disabled={runInFlight}><Icon name="plus" size={17} /></button>
          <textarea value={task} onChange={(event) => setTask(event.target.value)} onKeyDown={handleTaskKeyDown} placeholder="Ask Pandora to inspect, plan, or act…" aria-label="Pandora task" rows={1} disabled={runInFlight} />
          <div className="composer-actions"><label className="composer-profile"><span className="sr-only">Execution Harness</span><select value={runProfile} onChange={(event) => onRunProfileChange(event.target.value)} aria-label="Execution Harness" disabled={runInFlight}>{profileOptions.map((profile) => <option value={profile.id} key={profile.id}>{profile.label}</option>)}</select></label><span className="composer-mode"><Icon name="spark" size={14} /> Governed run</span><button type="submit" className="send-button" aria-label={runInFlight ? "Pandora is running" : "Send"} disabled={!connected || !task.trim() || runInFlight}><Icon name="arrow" size={16} /></button></div>
        </div>
        <div className="composer-hint"><span>{runError || (runInFlight ? "Pandora is running the governed request…" : connected ? "Ctrl/⌘ + Enter to send" : "Connect the local service in Connections")}</span><span>{contextAttachments.length ? `${contextAttachments.length} context file${contextAttachments.length === 1 ? "" : "s"} · effects still require a permit` : "All effects require an exact permit"}</span></div>
      </form>
      {lastRun ? <Panel className="run-result"><div className="panel-heading"><h3>Latest {lastRun.mode} run</h3><Chip tone={lastRun.status === "completed" ? "green" : "amber"}>{lastRun.status}</Chip></div><div className="run-result-meta"><span className="mono">{lastRun.execution_id ?? "provider-only response"}</span><span>{lastRun.selected_gene ?? "No gene selected"}</span></div><p>{lastRun.output || "No output returned."}</p>{events.length ? <div className="event-list"><span className="eyebrow">LIVE ACTIVITY</span>{events.map((event) => <div className="event-row" key={event.event_id}><span className="event-dot" /><span>{event.event_type.replaceAll("_", " ")}</span><span className="mono">{event.event_id}</span></div>)}</div> : null}</Panel> : null}
    </section>
    <Inspector steps={steps} approvalPreview={approvalPreview} lastRun={lastRun} events={events} selectedSession={selectedSession} runtimeStatus={runtimeStatus} approval={lastRun?.approval} approvalDetail={lastRun?.status_detail} approvalInFlight={runInFlight} workspaceInspection={workspaceInspection} workspaceInspectionInFlight={workspaceInspectionInFlight} selectedStep={selectedStep} onApprovalPreview={onApprovalPreview} onApprovalClose={onApprovalClose} onResolveApproval={onResolveApproval} onInspectWorkspace={onInspectWorkspace} onResolveWorkspaceInspection={onResolveWorkspaceInspection} onSelectStep={onSelectStep} />
  </div>;
}

function Metric({ label, value, detail }: { label: string; value: string; detail: string }) {
  return <div className="metric"><span>{label}</span><strong>{value}</strong><small>{detail}</small></div>;
}

function CommandPalette({ onClose, onSelectView }: { onClose: () => void; onSelectView: (view: ViewId) => void }) {
  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const actions = navigation.flatMap((group) => group.items.map((item) => ({ ...item, group: group.label })));
  const filtered = actions.filter((action) => `${action.label} ${action.group}`.toLowerCase().includes(query.toLowerCase()));
  const choose = (index: number) => {
    const action = filtered[index];
    if (action) {
      onSelectView(action.id);
    }
  };
  const handleKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setSelectedIndex((index) => filtered.length ? (index + 1) % filtered.length : 0);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setSelectedIndex((index) => filtered.length ? (index - 1 + filtered.length) % filtered.length : 0);
    } else if (event.key === "Enter") {
      event.preventDefault();
      choose(selectedIndex);
    }
  };
  return <div className="palette-backdrop" role="presentation" onMouseDown={onClose}><section className="command-palette" role="dialog" aria-modal="true" aria-label="Open Pandora surface" onMouseDown={(event) => event.stopPropagation()}><div className="palette-search"><Icon name="search" size={16} /><input autoFocus value={query} onChange={(event) => { setQuery(event.target.value); setSelectedIndex(0); }} onKeyDown={handleKeyDown} placeholder="Search Pandora surfaces…" aria-label="Search Pandora surfaces" /><kbd>ESC</kbd></div><div className="palette-list">{filtered.length ? filtered.map((action, index) => <button type="button" className={`palette-item ${index === selectedIndex ? "is-active" : ""}`} aria-selected={index === selectedIndex} key={action.id} onMouseEnter={() => setSelectedIndex(index)} onClick={() => choose(index)}><Icon name={action.icon} size={16} /><span><strong>{action.label}</strong><small>{action.group}</small></span><kbd>↵</kbd></button>) : <p className="palette-empty">No matching Pandora surface.</p>}</div><div className="palette-footer"><span>Use ↑ ↓ and Enter</span><span className="mono">⌘ K</span></div></section></div>;
}

function Inspector({ steps, approvalPreview, lastRun, events, selectedSession, runtimeStatus, approval, approvalDetail, approvalInFlight, workspaceInspection, workspaceInspectionInFlight, selectedStep, onApprovalPreview, onApprovalClose, onResolveApproval, onInspectWorkspace, onResolveWorkspaceInspection, onSelectStep }: { steps: typeof authoritySteps; approvalPreview: boolean; lastRun: RuntimeRun | null; events: RuntimeEvent[]; selectedSession: RuntimeSessionDetail | null; runtimeStatus: RuntimeStatus; approval?: RuntimeApproval; approvalDetail?: string; approvalInFlight: boolean; workspaceInspection: RuntimeRun | null; workspaceInspectionInFlight: boolean; selectedStep: string; onApprovalPreview: () => void; onApprovalClose: () => void; onResolveApproval: (allow: boolean) => Promise<void>; onInspectWorkspace: (task: string) => Promise<void>; onResolveWorkspaceInspection: (allow: boolean) => Promise<void>; onSelectStep: (id: string) => void }) {
  const [approvalError, setApprovalError] = useState("");
  const [tab, setTab] = useState<InspectorTab>("flow");
  const [workspacePath, setWorkspacePath] = useState("README.md");
  const [workspaceError, setWorkspaceError] = useState("");
  const selected = steps.find((step) => step.id === selectedStep) ?? steps[4];
  const hasLiveRun = Boolean(lastRun);
  const decide = async (allow: boolean) => {
    setApprovalError("");
    try {
      await onResolveApproval(allow);
    } catch (error: unknown) {
      setApprovalError(error instanceof Error ? error.message : "Could not resolve approval");
    }
  };
  const inspectWorkspace = async (task: string) => {
    setWorkspaceError("");
    try {
      await onInspectWorkspace(task);
    } catch (error: unknown) {
      setWorkspaceError(error instanceof Error ? error.message : "Workspace inspection failed");
    }
  };
  const readWorkspaceFile = async (event: FormEvent) => {
    event.preventDefault();
    const path = workspacePath.trim();
    if (!path) {
      setWorkspaceError("Enter a workspace-relative file path");
      return;
    }
    await inspectWorkspace(`read:${path}`);
  };
  const resolveWorkspace = async (allow: boolean) => {
    setWorkspaceError("");
    try {
      await onResolveWorkspaceInspection(allow);
    } catch (error: unknown) {
      setWorkspaceError(error instanceof Error ? error.message : "Could not resolve inspection approval");
    }
  };
  const workspaceOutput = workspaceInspection?.output.length && workspaceInspection.output.length > 120_000
    ? `${workspaceInspection.output.slice(0, 120_000)}\n[output truncated in desktop inspector]`
    : workspaceInspection?.output ?? "";
  const workspaceApproval = workspaceInspection?.approval;
  return <aside className="inspector">
    <div className="inspector-header"><div><span className="eyebrow">{hasLiveRun ? "LIVE RUN SUMMARY" : "AUTHORITY CONTRACT"}</span><h2>{hasLiveRun ? "Execution recorded" : "Preview boundary"}</h2></div><button className="icon-button" type="button" aria-label="Inspector options" disabled><Icon name="dots" size={17} /></button></div>
    <div className="inspector-tabs" role="tablist" aria-label="Run inspector">
      {(["flow", "evidence", "workspace"] as InspectorTab[]).map((item) => <button type="button" role="tab" aria-selected={tab === item} className={tab === item ? "is-active" : ""} key={item} onClick={() => setTab(item)}>{item}</button>)}
    </div>
    {tab === "evidence" ? <div className="inspector-pane" role="tabpanel">
      <Panel className="evidence-summary"><div className="panel-heading"><div><span className="eyebrow">RUN EVIDENCE</span><h3>{lastRun ? `${lastRun.receipt_count} receipts · ${lastRun.event_count} events` : "No run selected"}</h3></div><Chip tone={lastRun?.status === "completed" ? "green" : "neutral"} icon="archive">{lastRun?.status ?? "waiting"}</Chip></div>{lastRun ? <div className="evidence-facts"><div><span>Execution</span><strong className="mono">{lastRun.execution_id ?? "provider-only"}</strong></div><div><span>Harness</span><strong>{lastRun.selected_harness ?? "unselected"}</strong></div><div><span>Gene</span><strong>{lastRun.selected_gene ?? "unselected"}</strong></div><div><span>Prompt cache</span><strong>{lastRun.cached_prompt_tokens ?? 0} reused · {lastRun.cache_write_prompt_tokens ?? 0} written</strong></div></div> : <p className="task-copy">Run or select a session to inspect its immutable evidence summary.</p>}</Panel>
      <div className="inspector-section"><div className="section-heading"><span>Redacted activity</span><span className="mono section-count">{events.length}</span></div>{events.length ? <div className="compact-event-list">{events.map((event) => <div className="compact-event" key={event.event_id}><span className="event-dot" /><span>{event.event_type.replaceAll("_", " ")}</span><small className="mono">{event.event_id}</small></div>)}</div> : <p className="inspector-empty">No runtime events loaded.</p>}</div>
    </div> : tab === "workspace" ? <div className="inspector-pane" role="tabpanel">
      <Panel className="context-panel"><div className="panel-heading"><div><span className="eyebrow">SCOPED WORKSPACE</span><h3>{selectedSession?.session.workspace_id ?? "Local runtime workspace"}</h3></div><Icon name="lock" size={17} /></div><div className="evidence-facts"><div><span>Session</span><strong className="mono">{workspaceInspection?.session_id ?? selectedSession?.session.session_id ?? "new inspection"}</strong></div><div><span>Runtime scope</span><strong>Local device</strong></div><div><span>Reads</span><strong>Filesystem Gene</strong></div><div><span>Commands</span><strong>Exact permit path</strong></div></div></Panel>
      <Panel className="workspace-browser-panel">
        <div className="panel-heading"><div><span className="eyebrow">WORKSPACE EXPLORER</span><h3>Inspect real evidence</h3></div><Chip tone={runtimeStatus === "connected" ? "green" : "neutral"} icon="code">{runtimeStatus === "connected" ? "Governed" : "Offline"}</Chip></div>
        <form className="workspace-file-form" onSubmit={(event) => void readWorkspaceFile(event)}><label><span>Workspace-relative file</span><input aria-label="Workspace file path" value={workspacePath} onChange={(event) => setWorkspacePath(event.target.value)} maxLength={1024} spellCheck={false} autoComplete="off" placeholder="README.md" /></label><button className="button button-secondary" type="submit" disabled={runtimeStatus !== "connected" || workspaceInspectionInFlight || !workspacePath.trim()}>{workspaceInspectionInFlight ? "Inspecting…" : "Read file"}</button></form>
        <div className="workspace-command-section"><div><span className="eyebrow">BOUNDED TERMINAL</span><small>No arbitrary shell input. Every command uses a registered Gene.</small></div><div className="workspace-command-grid"><button type="button" disabled={runtimeStatus !== "connected" || workspaceInspectionInFlight} onClick={() => void inspectWorkspace("status")}><Icon name="terminal" size={14} /><span><strong>Git status</strong><small>workspace.status</small></span></button><button type="button" disabled={runtimeStatus !== "connected" || workspaceInspectionInFlight} onClick={() => void inspectWorkspace("diff")}><Icon name="code" size={14} /><span><strong>Working diff</strong><small>workspace.diff</small></span></button><button type="button" disabled={runtimeStatus !== "connected" || workspaceInspectionInFlight} onClick={() => void inspectWorkspace("log")}><Icon name="archive" size={14} /><span><strong>Recent log</strong><small>workspace.log</small></span></button></div></div>
        {workspaceError ? <p className="workspace-inspection-error" role="alert">{workspaceError}</p> : null}
      </Panel>
      {workspaceInspection ? <Panel className="workspace-result-panel"><div className="panel-heading"><div><span className="eyebrow">GOVERNED RESULT</span><h3>{workspaceInspection.selected_gene ?? "Workspace inspection"}</h3></div><Chip tone={workspaceInspection.status === "completed" ? "green" : workspaceInspection.status === "approval_required" ? "amber" : "neutral"}>{workspaceInspection.status}</Chip></div><div className="workspace-result-meta"><span className="mono">{workspaceInspection.execution_id ?? workspaceInspection.session_id}</span><span>{workspaceInspection.receipt_count} receipt{workspaceInspection.receipt_count === 1 ? "" : "s"}</span></div>{workspaceApproval ? <div className="workspace-approval"><div><span className="eyebrow">EXACT APPROVAL</span><strong>{workspaceApproval.request_summary}</strong><small className="mono">{workspaceApproval.request_digest}</small></div><div className="workspace-approval-actions">{workspaceApproval.status === "pending" ? <><button className="button button-deny" type="button" disabled={workspaceInspectionInFlight} onClick={() => void resolveWorkspace(false)}>Deny</button><button className="button button-primary" type="button" disabled={workspaceInspectionInFlight} onClick={() => void resolveWorkspace(true)}>Allow once</button></> : workspaceApproval.status === "approved" ? <button className="button button-primary" type="button" disabled={workspaceInspectionInFlight} onClick={() => void resolveWorkspace(true)}>Resume approved inspection</button> : <Chip tone="neutral">{workspaceApproval.status}</Chip>}</div></div> : null}{workspaceOutput ? <pre className="workspace-output" aria-label="Workspace inspection output">{workspaceOutput}</pre> : <p className="inspector-empty">{workspaceInspection.status_detail ?? (workspaceInspectionInFlight ? "Waiting for runtime evidence…" : "No output returned.")}</p>}</Panel> : null}
      <Panel className="context-boundary"><span className="eyebrow">BOUNDARY</span><h3>Inspection is evidence, not authority</h3><p>File reads, status, diff, and log stay on Pandora’s existing Harness → Gene → ReferenceMonitor → receipt path. This panel cannot execute arbitrary shell commands or mint permits.</p></Panel>
    </div> : <div className="inspector-pane" role="tabpanel">
      <Panel className="task-panel"><div className="task-heading"><span className="task-icon"><Icon name="code" size={18} /></span><div><span className="eyebrow">PANDORA DESKTOP</span><h3>Governed command surface</h3></div></div><p className="task-copy">Submit work through the local service. The desktop shell does not issue permits or execute tools directly.</p><div className="task-meta"><span><Icon name="book" size={13} /> Existing runtime</span><span><Icon name="lock" size={13} /> Workspace scoped</span></div></Panel>
      <div className="inspector-section"><div className="section-heading"><span>Authority chain</span><span className="mono section-count">{steps.filter((step) => step.status !== "idle").length}/8</span></div><div className="authority-timeline">{steps.map((step) => <button className={`authority-row status-${step.status} ${selectedStep === step.id ? "is-selected" : ""}`} key={step.id} onClick={() => onSelectStep(step.id)}><span className="timeline-line" /><span className="timeline-node">{step.status === "complete" ? <Icon name="check" size={12} /> : <Icon name={step.icon} size={13} />}</span><span className="authority-copy"><strong>{step.label}</strong><small>{step.detail}</small></span><Icon name="chevron" size={14} /></button>)}</div></div>
      <Panel className={`approval-panel ${approvalPreview || (approval && approval.status !== "pending") ? "is-preview-complete" : ""}`}><div className="approval-top"><span className="approval-icon"><Icon name={approvalPreview || (approval && approval.status !== "pending") ? "check" : "shield"} size={17} /></span><div><span className="eyebrow">{approval ? "LIVE APPROVAL" : approvalDetail ? "LIVE RUNTIME" : "PREVIEW ONLY"}</span><h3>{approval ? approval.status === "pending" ? "Exact approval required" : `Approval ${approval.status}` : approvalDetail ? "Approval metadata unavailable" : approvalPreview ? "No permit was issued" : "Preview one exact operation"}</h3></div></div>{approval ? <><p className="approval-note">{approval.status === "pending" ? "Review the exact digest before allowing this operation once." : `This approval is ${approval.status}; it cannot authorize another execution.`}</p><div className="operation-box"><div><span className="eyebrow">OPERATION</span><strong>{approval.request_summary}</strong></div><div><span className="eyebrow">GENE</span><span className="mono">{approval.gene_id}</span></div><div><span className="eyebrow">REQUEST DIGEST</span><span className="digest"><span className="mono">{approval.request_digest}</span></span></div><div><span className="eyebrow">SCOPE</span><span className="mono">{approval.session_id}</span></div></div>{approvalError ? <p className="approval-error" role="alert">{approvalError}</p> : null}<div className="approval-actions">{approval.status === "pending" ? <><button className="button button-deny" type="button" disabled={approvalInFlight} onClick={() => void decide(false)}>Deny</button><button className="button button-primary" type="button" disabled={approvalInFlight} onClick={() => void decide(true)}>{approvalInFlight ? "Resolving…" : "Allow once"} <Icon name="arrow" size={14} /></button></> : <button className="button button-secondary" type="button" onClick={onApprovalClose}>Close</button>}</div></> : approvalDetail ? <><p className="approval-note">The runtime paused, but this service did not return an exact approval record. Upgrade the local service before resuming.</p><div className="operation-box"><div><span className="eyebrow">REASON</span><strong>{approvalDetail}</strong></div><div><span className="eyebrow">SCOPE</span><span className="mono">Exact session and request</span></div></div><div className="approval-actions"><button className="button button-secondary" type="button" onClick={onApprovalClose}>Close</button></div></> : approvalPreview ? <><p className="approval-note">This preview records no decision and issues no permit.</p><div className="approval-actions"><button className="button button-secondary" type="button" onClick={onApprovalClose}>Close</button></div></> : <><p className="approval-note">This panel documents the exact-scope approval contract. It does not create an approval or permit.</p><div className="operation-box"><div><span className="eyebrow">OPERATION</span><strong>workspace.diff</strong></div><div><span className="eyebrow">TARGET</span><span className="mono">Pandora / local</span></div><div><span className="eyebrow">REQUEST DIGEST</span><span className="digest"><span className="mono">sha256:4c19…e08a</span><button className="copy-button" type="button" aria-label="Copy request digest" disabled><Icon name="copy" size={13} /></button></span></div></div><div className="approval-actions"><button className="button button-deny" type="button" onClick={onApprovalClose}>Close</button><button className="button button-primary" type="button" onClick={onApprovalPreview}>Show preview <Icon name="arrow" size={14} /></button></div></>}</Panel>
      {approval?.status === "approved" ? <button className="button button-primary approval-resume" type="button" disabled={approvalInFlight} onClick={() => void decide(true)}>{approvalInFlight ? "Resuming…" : "Resume approved run"} <Icon name="arrow" size={14} /></button> : null}
      <div className="selected-detail"><div className="section-heading"><span>Selected evidence</span><Icon name="chevron" size={14} /></div><div className="detail-row"><span className="detail-label">Stage</span><span>{selected.label}</span></div><div className="detail-row"><span className="detail-label">Status</span><Chip tone={selected.status === "waiting" ? "amber" : selected.status === "idle" ? "neutral" : "green"} icon={selected.status === "waiting" ? "clock" : selected.status === "idle" ? "lock" : "check"}>{selected.status}</Chip></div></div>
    </div>}
  </aside>;
}

function RunsView({ runs, runtimeStatus, onMutate }: { runs: RuntimeOrchestrationRun[]; runtimeStatus: RuntimeStatus; onMutate: (operation: "cancel" | "resume", runId: string, confirmation: string) => Promise<RuntimeOrchestrationRun> }) {
  const [selectedRunId, setSelectedRunId] = useState("");
  const [pendingOperation, setPendingOperation] = useState<"cancel" | "resume" | null>(null);
  const [confirmation, setConfirmation] = useState("");
  const [mutationError, setMutationError] = useState("");
  const [mutationInFlight, setMutationInFlight] = useState(false);
  const [mutationReceipt, setMutationReceipt] = useState<RuntimeOrchestrationRun | null>(null);
  const selected = runs.find((run) => run.run_id === selectedRunId) ?? runs[0] ?? null;
  const connected = runtimeStatus === "connected";
  const statusTone = (status: RuntimeOrchestrationRun["status"]): "neutral" | "green" | "amber" | "blue" | "gold" => status === "completed" ? "green" : status === "running" ? "blue" : status === "interrupted" ? "amber" : status === "queued" ? "gold" : "neutral";
  const statusCounts = runs.reduce<Record<string, number>>((counts, run) => ({ ...counts, [run.status]: (counts[run.status] ?? 0) + 1 }), {});

  const beginMutation = (operation: "cancel" | "resume") => {
    setPendingOperation(operation);
    setConfirmation("");
    setMutationError("");
    setMutationReceipt(null);
  };

  const submitMutation = async (event: FormEvent) => {
    event.preventDefault();
    if (!selected || !pendingOperation || confirmation !== selected.run_id) {
      return;
    }
    setMutationInFlight(true);
    setMutationError("");
    try {
      const result = await onMutate(pendingOperation, selected.run_id, confirmation);
      setMutationReceipt(result);
      setPendingOperation(null);
      setConfirmation("");
    } catch (error: unknown) {
      setMutationError(error instanceof Error ? error.message : "Could not update the orchestration run");
    } finally {
      setMutationInFlight(false);
    }
  };

  return <div className="full-view runs-view"><PageHeader eyebrow="Durable orchestration" title="Background Runs" description={connected ? "Inspect scoped multi-agent work without creating a second execution path. Workers coordinate; Harnesses and ReferenceMonitor retain authority." : "Connect the local runtime to inspect background work."} actions={<div className="run-status-summary"><Chip tone="gold">{statusCounts.queued ?? 0} queued</Chip><Chip tone="blue">{statusCounts.running ?? 0} running</Chip><Chip tone="amber">{statusCounts.interrupted ?? 0} interrupted</Chip></div>} />
    <div className="runs-workbench">
      <Panel className="runs-browser"><div className="panel-heading"><div><span className="eyebrow">SCOPED QUEUE</span><h3>{runs.length} orchestration runs</h3></div><Chip tone={connected ? "green" : "neutral"}>{connected ? "Live" : "Offline"}</Chip></div><div className="runs-list">{runs.length ? runs.map((run) => <button type="button" className={`run-browser-row ${selected?.run_id === run.run_id ? "is-selected" : ""}`} onClick={() => { setSelectedRunId(run.run_id); setPendingOperation(null); setMutationReceipt(null); }} key={run.run_id}><span className={`run-state-dot state-${run.status}`} /><span><strong>{run.plan_id}</strong><small className="mono">{run.run_id}</small><small>{run.roles.length} roles · {run.coordinator_workspace_id}</small></span><Chip tone={statusTone(run.status)}>{run.status}</Chip></button>) : <div className="runs-empty"><Icon name={connected ? "stack" : "lock"} size={24} /><h3>{connected ? "No background runs" : "Runtime connection required"}</h3><p>{connected ? "Submit a governed orchestration plan through the CLI or headless runner; it will appear here." : "This surface does not fabricate queue state."}</p></div>}</div></Panel>
      <Panel className="run-inspection">{selected ? <><div className="run-hero"><span className={`run-hero-icon state-${selected.status}`}><Icon name={selected.status === "completed" ? "check" : selected.status === "interrupted" ? "clock" : "stack"} size={21} /></span><div><span className="eyebrow">{selected.status === "running" ? "WORKER OWNED" : selected.status === "interrupted" ? "RECONCILIATION BOUNDARY" : "DURABLE RUN"}</span><h2>{selected.plan_id}</h2><p className="mono">{selected.run_id}</p></div><Chip tone={statusTone(selected.status)} icon={selected.status === "completed" ? "check" : selected.status === "interrupted" ? "clock" : "activity"}>{selected.status}</Chip></div>
        <div className="run-facts"><div><span>Coordinator workspace</span><strong>{selected.coordinator_workspace_id}</strong></div><div><span>Worker lease</span><strong className="mono">{selected.worker_id ?? "unclaimed"}</strong></div><div><span>Role receipts</span><strong>{selected.receipt_count} / {selected.roles.length}</strong></div><div><span>Handoffs used</span><strong>{selected.handoffs_used}</strong></div><div><span>Updated</span><strong>{new Date(selected.updated_at_unix_seconds * 1000).toLocaleString()}</strong></div></div>
        {selected.interruption_reason ? <div className="interruption-banner"><Icon name="shield" size={15} /><div><strong>Run interrupted</strong><span>{selected.interruption_reason}</span></div></div> : null}
        <div className="role-inspector"><div className="inspection-heading"><div><span className="eyebrow">ROLE GRAPH</span><h3>Exact repository assignments</h3></div><Chip tone="blue">{selected.roles.filter((role) => role.state === "completed").length}/{selected.roles.length} complete</Chip></div><div className="role-run-list">{selected.roles.map((role, index) => <article className="role-run-row" key={role.role_id}><span className={`role-state state-${role.state}`}>{role.state === "completed" ? <Icon name="check" size={12} /> : <span className="mono">{String(index + 1).padStart(2, "0")}</span>}</span><div><strong>{role.role}</strong><small className="mono">{role.role_id} · {role.harness_id}</small><span>{role.repository_id} / {role.workspace_id}</span></div><div className="role-commit"><span>{role.state}</span><strong className="mono">{role.exact_commit}</strong></div></article>)}</div></div>
        <div className="run-control-boundary"><div><span className="eyebrow">CONTROL BOUNDARY</span><h3>{selected.status === "queued" ? "Queued work may be cancelled" : selected.status === "interrupted" ? "Resume only after safe reconciliation" : selected.status === "running" ? "The active worker owns this run" : "This run is terminal"}</h3><p>{selected.status === "queued" ? "Cancellation is scoped to this exact run and cannot affect another workspace." : selected.status === "interrupted" ? "The runtime refuses resume while any role remains active without reconciled evidence." : selected.status === "running" ? "The desktop cannot steal the lease, complete roles, issue permits, or fabricate receipts." : "Completed and cancelled runs remain inspectable as durable evidence."}</p></div>{selected.status === "queued" ? <button className="button button-deny" type="button" onClick={() => beginMutation("cancel")}>Cancel run</button> : selected.status === "interrupted" ? <button className="button button-primary" type="button" onClick={() => beginMutation("resume")}>Resume safely <Icon name="arrow" size={13} /></button> : null}</div>
        {pendingOperation ? <form className="run-confirm" onSubmit={submitMutation}><div><span className="eyebrow">EXACT CONFIRMATION</span><strong>{pendingOperation === "cancel" ? "Cancel queued orchestration" : "Requeue reconciled orchestration"}</strong><p>Type <span className="mono">{selected.run_id}</span> to confirm this exact run.</p></div><label><span>Run ID</span><input aria-label={`Confirm ${pendingOperation} ${selected.run_id}`} value={confirmation} onChange={(event) => setConfirmation(event.target.value)} autoComplete="off" spellCheck={false} /></label><div className="run-confirm-actions"><button className="button button-secondary" type="button" onClick={() => setPendingOperation(null)} disabled={mutationInFlight}>Close</button><button className={pendingOperation === "cancel" ? "button button-deny" : "button button-primary"} type="submit" disabled={mutationInFlight || confirmation !== selected.run_id}>{mutationInFlight ? "Applying…" : pendingOperation === "cancel" ? "Confirm cancellation" : "Confirm resume"}</button></div>{mutationError ? <p className="connection-error" role="alert">{mutationError}</p> : null}</form> : null}
        {mutationReceipt ? <div className="run-mutation-receipt"><Icon name="check" size={15} /><span><strong>{mutationReceipt.status === "cancelled" ? "Run cancelled" : "Run requeued"}</strong><small className="mono">{mutationReceipt.run_id} · {mutationReceipt.updated_at_unix_seconds}</small></span></div> : null}
      </> : <div className="runs-empty"><Icon name="stack" size={27} /><h3>No run selected</h3><p>Background orchestration evidence will appear here after the runtime reports it.</p></div>}</Panel>
    </div>
  </div>;
}

function MemoryView({ runtimeStatus, records, selectedSession }: { runtimeStatus: RuntimeStatus; records: RuntimeMemoryRecord[]; selectedSession: RuntimeSessionDetail | null }) {
  const graphNodes = [
    { className: "graph-node graph-node-gold node-a", label: "active plan" },
    { className: "graph-node graph-node-blue node-b", label: "L1 evidence" },
    { className: "graph-node graph-node-blue node-c", label: "workspace" },
    { className: "graph-node graph-node-green node-d", label: "verified run" },
    { className: "graph-node graph-node-violet node-e", label: "lineage" },
    { className: "graph-node graph-node-muted node-f", label: "provider" }
  ];
  const serviceMessage = runtimeStatus === "connected" ? selectedSession ? "Only redacted records for the selected session are shown." : "Select a session to inspect scoped memory." : "Connect the local service to inspect scoped memory.";
  return <div className="full-view"><PageHeader eyebrow="Scoped knowledge" title="Memory" description="Inspect bounded, redacted evidence with provenance labels." actions={<Chip tone={records.length ? "green" : "neutral"} icon="archive">{records.length ? `${records.length} records` : "No records"}</Chip>} /><div className="memory-grid"><Panel className="memory-graph-panel"><div className="panel-toolbar"><div><span className="eyebrow">SESSION MEMORY · REDACTED</span><h3>{selectedSession?.session.session_id ?? "No session selected"}</h3></div><div className="toolbar-pills"><Chip tone="gold">L2</Chip><Chip tone="blue">L1</Chip><Chip tone="green">L0 ephemeral</Chip></div></div>{records.length ? <div className="memory-record-list">{records.map((record) => <article className="memory-record" key={`${record.tier}-${record.memory_id}`}><div className="memory-record-top"><Chip tone={record.tier === "l2" ? "gold" : "blue"}>{record.tier}</Chip><span className="eyebrow">{record.kind}</span><span className="memory-record-time">{new Date(record.created_at_unix_seconds * 1000).toLocaleString()}</span></div><p>{record.summary}</p><div className="memory-record-meta"><span>{record.classification}</span><span>{record.origin}</span><span>{record.evidence_count} evidence</span><span className="mono">{record.provenance}</span></div></article>)}</div> : <div className="graph-canvas"><div className="graph-lines"><span className="graph-line line-one" /><span className="graph-line line-two" /><span className="graph-line line-three" /><span className="graph-line line-four" /><span className="graph-line line-five" /></div>{graphNodes.map((node) => <div className={node.className} key={node.label}><span /><label>{node.label}</label></div>)}<div className="graph-center-label"><span>Scoped memory</span><small>{selectedSession ? "no records for session" : "connect and select a session"}</small></div></div>}</Panel><div className="memory-side"><Panel><div className="panel-heading"><h3>Memory layers</h3><Chip tone={records.length ? "green" : "neutral"}>{records.length ? "Live" : "Unavailable"}</Chip></div><Layer label="L0 · Ephemeral trace" value="RAM" detail="expires automatically" tone="green" /><Layer label="L1 · Distilled evidence" value={String(records.filter((record) => record.tier === "l1").length)} detail="session scoped" tone="blue" /><Layer label="L2 · Evolutionary" value={String(records.filter((record) => record.tier === "l2").length)} detail="promotion gated" tone="gold" /></Panel><Panel><div className="panel-heading"><h3>Availability</h3><Chip tone={records.length ? "green" : "neutral"} icon="lock">{records.length ? "Scoped" : "Unavailable"}</Chip></div><p className="connection-note">{serviceMessage}</p></Panel></div></div></div>;
}

function Layer({ label, value, detail, tone }: { label: string; value: string; detail: string; tone: "green" | "blue" | "gold" }) {
  return <div className="layer-row"><span className={`layer-dot dot-${tone}`} /><div><strong>{label}</strong><small>{detail}</small></div><span className="layer-value mono">{value}</span></div>;
}

function WorkflowsView({ runtimeStatus, workflows, harnesses, onOpenCommand, onCreate, onRemove, onRun }: { runtimeStatus: RuntimeStatus; workflows: WorkflowRecipe[]; harnesses: RuntimeHarness[]; onOpenCommand: () => void; onCreate: (name: string, task: string, profile: RunProfile) => void; onRemove: (id: string) => void; onRun: (workflow: WorkflowRecipe) => Promise<void> }) {
  const [name, setName] = useState("");
  const [task, setTask] = useState("");
  const [profile, setProfile] = useState<RunProfile>("auto");
  const connected = runtimeStatus === "connected";
  const options = harnesses.length ? [{ id: "auto", label: "Auto route" }, ...harnesses.filter((harness) => harness.runnable).map((harness) => ({ id: harness.id, label: harness.name }))] : runProfiles;
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!name.trim() || !task.trim()) return;
    onCreate(name.trim(), task.trim(), profile);
    setName("");
    setTask("");
  };
  return <div className="full-view"><PageHeader eyebrow="Client recipes" title="Workflows" description="Save reusable task recipes locally; every run still uses Pandora’s governed local runtime." actions={<Chip tone={workflows.length ? "green" : "neutral"} icon="stack">{workflows.length} saved</Chip>} /><div className="workflow-grid"><Panel className="workflow-editor"><div className="panel-heading"><div><span className="eyebrow">NEW RECIPE</span><h3>Define a governed run</h3></div><Icon name="plus" size={17} /></div><form className="workflow-form" onSubmit={submit}><label><span>Name</span><input value={name} onChange={(event) => setName(event.target.value)} placeholder="Release review" maxLength={80} /></label><label><span>Task</span><textarea value={task} onChange={(event) => setTask(event.target.value)} placeholder="Describe the work Pandora should perform…" maxLength={4000} rows={5} /></label><label><span>Harness</span><select value={profile} onChange={(event) => setProfile(event.target.value)}>{options.map((option) => <option value={option.id} key={option.id}>{option.label}</option>)}</select></label><button className="button button-primary" type="submit"><Icon name="plus" size={14} /> Save recipe</button></form><p className="connection-note">Recipes are stored in this desktop profile. Do not save credentials or secrets in task text.</p></Panel><Panel className="workflow-list-panel"><div className="panel-heading"><div><span className="eyebrow">SAVED RECIPES</span><h3>{workflows.length ? "Ready to run" : "No recipes yet"}</h3></div><Chip tone={connected ? "green" : "neutral"}>{connected ? "Runtime ready" : "Connect to run"}</Chip></div>{workflows.length ? <div className="workflow-list">{workflows.map((workflow) => <article className="workflow-card" key={workflow.id}><div><strong>{workflow.name}</strong><small>{workflow.profile === "auto" ? "Auto route" : workflow.profile} · local recipe</small><p>{workflow.task}</p></div><div className="workflow-card-actions"><button className="button button-secondary" type="button" disabled={!connected} onClick={() => void onRun(workflow)}>{connected ? "Run" : "Offline"} <Icon name="arrow" size={13} /></button><button className="icon-button" type="button" aria-label={`Delete ${workflow.name}`} onClick={() => onRemove(workflow.id)}><Icon name="dots" size={16} /></button></div></article>)}</div> : <div className="workflow-empty"><div className="empty-orbit"><Icon name="stack" size={27} /></div><h2>Build your first recipe</h2><p>Recipes remain local to this desktop. Execution always returns to the Command Center and the governed runtime.</p><button className="button button-secondary" type="button" onClick={onOpenCommand}>Open Command Center <Icon name="arrow" size={14} /></button></div>}</Panel></div></div>;
}

function AuditView({ events, selectedSession, runtimeStatus }: { events: RuntimeEvent[]; selectedSession: RuntimeSessionDetail | null; runtimeStatus: RuntimeStatus }) {
  const live = runtimeStatus === "connected" && selectedSession !== null;
  return <div className="full-view"><PageHeader eyebrow="Evidence" title="Audit" description="Inspect redacted runtime activity without exposing event payloads or credentials." actions={<Chip tone={live ? "green" : "neutral"} icon="archive">{live ? `${events.length} events loaded` : "Select a live session"}</Chip>} /><div className="audit-grid"><Panel className="audit-summary"><div className="panel-heading"><div><span className="eyebrow">SESSION SCOPE</span><h3>{selectedSession?.session.session_id ?? "No session selected"}</h3></div><Icon name="lock" size={18} /></div><div className="audit-summary-rows"><div><span>Workspace</span><strong>{selectedSession?.session.workspace_id ?? "—"}</strong></div><div><span>Runtime scope</span><strong>Local device</strong></div><div><span>Recorded events</span><strong>{selectedSession?.event_count ?? 0}</strong></div></div><p>Event payloads stay in the local runtime. This surface shows identifiers and event types only.</p></Panel><Panel className="audit-events"><div className="panel-heading"><div><span className="eyebrow">ACTIVITY</span><h3>Runtime event timeline</h3></div><Chip tone="blue" icon="activity">Redacted</Chip></div>{events.length ? <div className="audit-event-list">{events.map((event) => <div className="audit-event-row" key={event.event_id}><span className="event-dot" /><div><strong>{event.event_type.replaceAll("_", " ")}</strong><small className="mono">{event.event_id}</small></div><span className="audit-event-state">recorded</span></div>)}</div> : <div className="connection-empty"><Icon name="archive" size={21} /><p>{live ? "No events recorded for this session." : "Connect and select a session to inspect activity."}</p></div>}</Panel></div></div>;
}

function CapabilitiesView({ harnesses, tools, runtimeStatus }: { harnesses: RuntimeHarness[]; tools: RuntimeTool[]; runtimeStatus: RuntimeStatus }) {
  const connected = runtimeStatus === "connected";
  const [kindFilter, setKindFilter] = useState("all");
  const [selectedHarnessId, setSelectedHarnessId] = useState("");
  const [tab, setTab] = useState<HarnessTab>("genes");
  const visibleHarnesses = kindFilter === "all" ? harnesses : harnesses.filter((harness) => harness.kind === kindFilter);
  const kinds = Array.from(new Set(harnesses.map((harness) => harness.kind))).sort();
  const selectedHarness = visibleHarnesses.find((harness) => harness.id === selectedHarnessId) ?? visibleHarnesses[0] ?? null;
  return <div className="full-view harness-lab"><PageHeader eyebrow="Runtime surface" title="Harness Lab" description={connected ? "Inspect each Harness as a versioned capability boundary: Genes, extensions, authority, and execution evidence." : "Connect the local runtime to inspect installed Harnesses."} actions={<Chip tone={connected ? "green" : "neutral"} icon="box">{connected ? `${harnesses.length} Harnesses · ${tools.length} extensions` : "Unavailable"}</Chip>} />
    {connected ? <div className="capability-toolbar"><span className="eyebrow">HARNESS TYPE</span><select value={kindFilter} onChange={(event) => { setKindFilter(event.target.value); setSelectedHarnessId(""); }} aria-label="Filter Harnesses by type"><option value="all">All types</option>{kinds.map((kind) => <option value={kind} key={kind}>{kind.replaceAll("_", " ")}</option>)}</select><span className="capability-note">Read-only inventory from the local runtime.</span></div> : null}
    <div className="harness-workbench">
      <Panel className="harness-browser"><div className="panel-heading"><div><span className="eyebrow">CATALOG</span><h3>{visibleHarnesses.length} Harnesses</h3></div><Icon name="search" size={16} /></div><div className="harness-browser-list">{connected && visibleHarnesses.length ? visibleHarnesses.map((harness) => <button type="button" className={`harness-browser-row ${selectedHarness?.id === harness.id ? "is-selected" : ""}`} key={harness.id} onClick={() => setSelectedHarnessId(harness.id)}><span className="harness-browser-icon"><Icon name="box" size={16} /></span><span><strong>{harness.name}</strong><small>{harness.kind} · v{harness.version}</small></span><Chip tone={harness.runnable ? "green" : "neutral"}>{harness.runnable ? "ready" : "bound"}</Chip></button>) : <div className="harness-empty"><Icon name="lock" size={20} /><p>{connected ? "No Harnesses in this filter." : "Runtime connection required."}</p></div>}</div></Panel>
      <Panel className="harness-inspection">{selectedHarness ? <><div className="harness-hero"><span className="harness-hero-icon"><Icon name="box" size={22} /></span><div><span className="eyebrow">VERSIONED HARNESS</span><h2>{selectedHarness.name}</h2><p className="mono">{selectedHarness.id} · {selectedHarness.kind} · v{selectedHarness.version}</p></div><Chip tone={selectedHarness.runnable ? "green" : "gold"} icon={selectedHarness.runnable ? "check" : "lock"}>{selectedHarness.runnable ? "Runnable" : "Bound"}</Chip></div>
        <div className="harness-tabs" role="tablist" aria-label="Harness inspector">{(["genes", "extensions", "authority", "receipts"] as HarnessTab[]).map((item) => <button type="button" role="tab" aria-selected={tab === item} className={tab === item ? "is-active" : ""} onClick={() => setTab(item)} key={item}>{item === "extensions" ? "Plugins & tools" : item}</button>)}</div>
        <div className="harness-tab-panel" role="tabpanel">{tab === "genes" ? <><div className="inspection-heading"><div><span className="eyebrow">GENE CATALOG</span><h3>{selectedHarness.gene_count} admitted Genes</h3></div><Chip tone="blue">Exact versions</Chip></div>{selectedHarness.gene_ids?.length ? <div className="gene-table">{selectedHarness.gene_ids.map((gene, index) => <div className="gene-row" key={gene}><span className="gene-index mono">{String(index + 1).padStart(2, "0")}</span><span><strong>{gene}</strong><small>Capability identity reported by runtime</small></span><Chip tone="green" icon="check">admitted</Chip></div>)}</div> : <p className="inspector-empty">Gene identities are unavailable from this runtime version.</p>}</> : tab === "extensions" ? <><div className="inspection-heading"><div><span className="eyebrow">ADMITTED EXTENSIONS</span><h3>{tools.length} plugin and tool surfaces</h3></div><Chip tone="gold" icon="shield">No authority implied</Chip></div>{tools.length ? <div className="gene-table">{tools.map((tool) => <div className="gene-row" key={tool.id}><span className="harness-browser-icon"><Icon name="terminal" size={14} /></span><span><strong>{tool.name}</strong><small className="mono">{tool.id} · v{tool.version}</small></span><span className="extension-operation">{tool.capability} / {tool.operation}</span></div>)}</div> : <p className="inspector-empty">No extension metadata reported.</p>}</> : tab === "authority" ? <div className="authority-map"><div><span>May select</span><strong>{selectedHarness.gene_count} admitted Genes</strong></div><div><span>May propose</span><strong>Bound tool requests</strong></div><div><span>May approve</span><strong className="authority-denied">Never</strong></div><div><span>May execute</span><strong className="authority-denied">Never directly</strong></div><p><Icon name="shield" size={14} /> Parliament plans, the Shadow Council routes, and ReferenceMonitor alone can authorize an exact effect.</p></div> : <div className="receipt-posture"><div className="receipt-seal"><Icon name="archive" size={24} /></div><h3>Evidence follows execution</h3><p>This catalog exposes capability identity and admission state. Receipts are run-scoped and appear in the Command inspector after an exact permit is consumed.</p><div className="receipt-rules"><span><Icon name="check" size={12} /> Request digest</span><span><Icon name="check" size={12} /> Bound Gene version</span><span><Icon name="check" size={12} /> Workspace scope</span><span><Icon name="check" size={12} /> Effect outcome</span></div></div>}</div>
      </> : <div className="harness-empty"><Icon name="box" size={24} /><h3>Select a Harness</h3><p>The inspector never fabricates catalog entries.</p></div>}</Panel>
    </div>
  </div>;
}

function ToolsView({ tools, runtimeStatus }: { tools: RuntimeTool[]; runtimeStatus: RuntimeStatus }) {
  const connected = runtimeStatus === "connected";
  return <div className="full-view"><PageHeader eyebrow="Runtime surface" title="Built-in Tools" description="Tool definitions exposed by Pandora’s ToolEngine. Tool metadata does not grant execution authority." actions={<Chip tone={connected ? "green" : "neutral"} icon="terminal">{connected ? `${tools.length} available` : "Unavailable"}</Chip>} /><div className="engine-notice"><Icon name="lock" size={16} /><span>{connected ? "Schemas and effect classifications are read-only here. Execution still requires the runtime’s policy and permit path." : "Connect the local runtime to inspect built-in tools."}</span></div><div className="tool-grid">{connected && tools.length ? tools.map((tool) => <Panel className="tool-row" key={tool.id}><div><strong>{tool.name}</strong><small className="mono">{tool.id} · v{tool.version}</small></div><span className="tool-capability">{tool.capability}</span><span className="tool-operation">{tool.operation}</span></Panel>) : <Panel className="secondary-card"><div className="secondary-icon secondary-icon-1"><Icon name="lock" size={20} /></div><span className="eyebrow">{connected ? "EMPTY" : "UNAVAILABLE"}</span><h3>{connected ? "No tools reported" : "Runtime connection required"}</h3><p>{connected ? "The local service returned an empty tool catalog." : "This surface does not fabricate tool definitions."}</p></Panel>}</div></div>;
}

function EnginesView({ engines, runtimeStatus }: { engines: RuntimeEngine[]; runtimeStatus: RuntimeStatus }) {
  const connected = runtimeStatus === "connected";
  return <div className="full-view"><PageHeader eyebrow="Architecture" title="Engines" description="Pandora’s engines are bounded modules around one governed execution path." actions={<Chip tone={connected ? "green" : "neutral"} icon="stack">{connected ? `${engines.length} reported` : "Unavailable"}</Chip>} /><div className="engine-notice"><Icon name="lock" size={16} /><span>{connected ? "Engine metadata is reported by the local runtime. This describes ownership, not live health." : "Connect the local runtime to inspect Pandora’s engine inventory."}</span></div><div className="secondary-grid">{connected && engines.length ? engines.map((engine, index) => <Panel className="secondary-card engine-card" key={engine.id}><div className={`secondary-icon secondary-icon-${index % 3}`}><Icon name={index < 2 ? "shield" : "stack"} size={20} /></div><span className="eyebrow">RUNTIME MODULE</span><h3>{engine.name}</h3><strong className="engine-role">{engine.role}</strong><p>{engine.authority}</p><span className="text-link">{engine.id}</span></Panel>) : <Panel className="secondary-card"><div className="secondary-icon secondary-icon-1"><Icon name="lock" size={20} /></div><span className="eyebrow">{connected ? "EMPTY" : "UNAVAILABLE"}</span><h3>{connected ? "No engines reported" : "Runtime connection required"}</h3><p>{connected ? "The local service returned an empty engine inventory." : "This surface does not fabricate engine state."}</p></Panel>}</div></div>;
}

function SettingsView({ theme, onThemeChange, runtimeStatus, health, native, endpoint }: { theme: ThemeMode; onThemeChange: (nextTheme: ThemeMode) => void; runtimeStatus: RuntimeStatus; health: RuntimeHealth | null; native: boolean; endpoint: string }) {
  return <div className="full-view"><PageHeader eyebrow="Workspace" title="Settings" description="Personalize the desktop shell while keeping runtime authority in Pandora." actions={<Chip tone="neutral" icon="gear">Local preference</Chip>} /><div className="settings-grid"><Panel className="settings-panel"><div className="panel-heading"><div><span className="eyebrow">APPEARANCE</span><h3>Theme</h3></div><Icon name="spark" size={18} /></div><p className="settings-copy">Choose the visual mode for this device. The setting is stored locally and does not change runtime policy.</p><div className="theme-toggle" role="group" aria-label="Theme mode"><button type="button" className={`theme-option ${theme === "dark" ? "is-selected" : ""}`} aria-pressed={theme === "dark"} onClick={() => onThemeChange("dark")}>Dark<span>Low-light command surface</span></button><button type="button" className={`theme-option ${theme === "light" ? "is-selected" : ""}`} aria-pressed={theme === "light"} onClick={() => onThemeChange("light")}>Light<span>High-contrast workspace</span></button></div></Panel><Panel className="settings-panel"><div className="panel-heading"><div><span className="eyebrow">RUNTIME</span><h3>Connection posture</h3></div><Chip tone={runtimeStatus === "connected" ? "green" : runtimeStatus === "offline" ? "amber" : "neutral"} icon="lock">{runtimeStatusLabel(runtimeStatus)}</Chip></div><div className="settings-facts"><div><span>Client</span><strong>{native ? "Native desktop shell" : "Browser preview"}</strong></div><div><span>Endpoint</span><strong className="mono">{endpoint || "Not connected"}</strong></div><div><span>Health</span><strong>{health?.status ?? "Unavailable"}</strong></div><div><span>Authority</span><strong>Local service only</strong></div></div><p className="settings-copy">Local device trust is established automatically. Effect authorization remains inside the Pandora runtime.</p></Panel></div></div>;
}

function ConnectionView({ endpoint, runtimeStatus, runtimeError, health, providers, sessions, selectedSessionId, selectedSession, native, serviceActive, onConnect, onStartService, onStopService, onSelectSession }: { endpoint: string; runtimeStatus: RuntimeStatus; runtimeError: string; health: RuntimeHealth | null; providers: RuntimeProvider[]; sessions: RuntimeSession[]; selectedSessionId: string; selectedSession: RuntimeSessionDetail | null; native: boolean; serviceActive: boolean; onConnect: (endpoint: string, token: string) => void; onStartService: () => Promise<void>; onStopService: () => Promise<void>; onSelectSession: (sessionId: string) => Promise<void> }) {
  const [draftEndpoint, setDraftEndpoint] = useState(endpoint);
  const [draftToken, setDraftToken] = useState("");
  const [configurationTab, setConfigurationTab] = useState<"provider" | "mcp">("provider");
  const [configurationBusy, setConfigurationBusy] = useState(false);
  const [configurationMessage, setConfigurationMessage] = useState("");
  const [configurationError, setConfigurationError] = useState("");
  const [providerName, setProviderName] = useState("custom");
  const [providerProtocol, setProviderProtocol] = useState<"open_ai_compatible" | "anthropic_messages" | "gemini_generate_content">("open_ai_compatible");
  const [providerUrl, setProviderUrl] = useState("");
  const [providerModel, setProviderModel] = useState("");
  const [apiKeyEnvironment, setApiKeyEnvironment] = useState("PANDORA_CUSTOM_API_KEY");
  const [apiKey, setApiKey] = useState("");
  const [mcpServerId, setMcpServerId] = useState("");
  const [mcpProgram, setMcpProgram] = useState("");
  const [mcpArguments, setMcpArguments] = useState("[]");
  const [mcpMode, setMcpMode] = useState<"auto" | "modern-only" | "legacy-only">("auto");

  useEffect(() => setDraftEndpoint(endpoint), [endpoint]);

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (draftEndpoint.trim() && draftToken.trim()) {
      onConnect(draftEndpoint.trim(), draftToken.trim());
      setDraftToken("");
    }
  };

  const submitProvider = async (event: FormEvent) => {
    event.preventDefault();
    setConfigurationBusy(true);
    setConfigurationMessage("");
    setConfigurationError("");
    try {
      const result = await configureProvider({
        name: providerName.trim(),
        protocol: providerProtocol,
        baseUrl: providerUrl.trim(),
        model: providerModel.trim(),
        apiKeyEnvironment: apiKeyEnvironment.trim(),
        apiKey,
      });
      setApiKey("");
      setConfigurationMessage(`${result.message}${result.restartRequired ? " Restart the local service to apply it." : ""}`);
    } catch (error: unknown) {
      setConfigurationError(error instanceof Error ? error.message : "Provider configuration failed");
    } finally {
      setConfigurationBusy(false);
    }
  };

  const submitMcp = async (event: FormEvent) => {
    event.preventDefault();
    setConfigurationBusy(true);
    setConfigurationMessage("");
    setConfigurationError("");
    try {
      const result = await configureMcp({
        serverId: mcpServerId.trim(),
        program: mcpProgram.trim(),
        argumentsJson: mcpArguments.trim(),
        mode: mcpMode,
      });
      setConfigurationMessage(`${result.message}${result.restartRequired ? " Restart the local service to apply it." : ""}`);
    } catch (error: unknown) {
      setConfigurationError(error instanceof Error ? error.message : "MCP configuration failed");
    } finally {
      setConfigurationBusy(false);
    }
  };

  const providerReady = providerName.trim() && providerUrl.trim() && providerModel.trim() && apiKeyEnvironment.trim();
  const mcpReady = mcpServerId.trim() && mcpProgram.trim() && mcpArguments.trim();

  return <div className="full-view">
    <PageHeader eyebrow="Runtime surface" title="Connections" description={native ? "Configure Pandora’s local runtime, providers, and MCP tools without an account." : "Connect this development preview to a loopback Pandora service."} actions={<Chip tone={runtimeStatus === "connected" ? "green" : runtimeStatus === "offline" ? "amber" : "blue"} icon="lock">{runtimeStatusLabel(runtimeStatus)}</Chip>} />
    <div className="connection-grid">
      <Panel className="connection-panel">
        <div className="panel-heading"><div><span className="eyebrow">LOCAL RPC</span><h3>Pandora service</h3></div><Icon name="terminal" size={19} /></div>
        <div className="settings-facts connection-health"><div><span>Service health</span><strong>{health?.status ?? "Unavailable"}</strong></div><div><span>Transport</span><strong>{native ? "Native bridge" : "Loopback RPC"}</strong></div></div>
        {native ? <><button className={`button ${serviceActive ? "button-secondary" : "button-primary"} connection-start`} type="button" onClick={() => void (serviceActive ? onStopService() : onStartService())}>{serviceActive ? "Stop local service" : "Start local service"} <Icon name={serviceActive ? "lock" : "arrow"} size={14} /></button><div className="native-trust-note"><Icon name="shield" size={17} /><div><strong>No account required</strong><p>Device trust and the loopback service session are established automatically. Credentials remain native-side and are never exposed to this interface.</p></div></div></> : <><form className="connection-form" onSubmit={submit}><label><span>Endpoint</span><input value={draftEndpoint} onChange={(event) => setDraftEndpoint(event.target.value)} placeholder="http://127.0.0.1:PORT/v1/rpc" spellCheck={false} /></label><label><span>Development token</span><input value={draftToken} onChange={(event) => setDraftToken(event.target.value)} type="password" placeholder="Paste the local service token" autoComplete="off" /></label><button className="button button-secondary" type="submit" disabled={!draftEndpoint.trim() || !draftToken.trim()}>Connect preview <Icon name="arrow" size={14} /></button></form><p className="connection-note">Browser-preview credentials stay in memory and are never written to storage. Endpoints must be loopback-only.</p></>}
        {runtimeError ? <p className="connection-error" role="alert">{runtimeError}</p> : null}
      </Panel>

      <Panel className="connection-panel">
        <div className="panel-heading"><div><span className="eyebrow">PROVIDER PROFILES</span><h3>{providers.length} configured</h3></div><Chip tone={providers.some((provider) => provider.active) ? "blue" : "neutral"} icon="spark">Secrets hidden</Chip></div>
        {providers.length ? <div className="provider-list">{providers.map((provider) => <div className="provider-row" key={provider.name}><span className={`provider-dot ${provider.active ? "is-active" : ""}`} /><span><strong>{provider.name}</strong><small>{provider.model} · {provider.protocol}</small></span><span className={`provider-state ${provider.credential_configured ? "is-ready" : ""}`}>{provider.credential_configured ? "Ready" : "Credential needed"}</span></div>)}</div> : <div className="connection-empty"><Icon name="lock" size={21} /><p>Provider profiles are not configured.</p></div>}
      </Panel>

      {native ? <Panel className="connection-panel connection-config-panel">
        <div className="panel-heading"><div><span className="eyebrow">LOCAL CONFIGURATION</span><h3>Add a connection</h3></div><Chip tone="green" icon="shield">Native only</Chip></div>
        <div className="configuration-tabs" role="tablist" aria-label="Connection type">
          <button type="button" role="tab" aria-selected={configurationTab === "provider"} className={configurationTab === "provider" ? "is-selected" : ""} onClick={() => { setConfigurationTab("provider"); setConfigurationError(""); setConfigurationMessage(""); }}>Model provider</button>
          <button type="button" role="tab" aria-selected={configurationTab === "mcp"} className={configurationTab === "mcp" ? "is-selected" : ""} onClick={() => { setConfigurationTab("mcp"); setConfigurationError(""); setConfigurationMessage(""); }}>Local MCP server</button>
        </div>
        {configurationTab === "provider" ? <form className="native-config-form" onSubmit={(event) => void submitProvider(event)}>
          <div className="config-form-grid">
            <label><span>Profile name</span><input aria-label="Provider profile name" value={providerName} onChange={(event) => setProviderName(event.target.value)} placeholder="custom" maxLength={64} autoComplete="off" spellCheck={false} /></label>
            <label><span>Protocol</span><select aria-label="Provider protocol" value={providerProtocol} onChange={(event) => setProviderProtocol(event.target.value as typeof providerProtocol)}><option value="open_ai_compatible">OpenAI compatible</option><option value="anthropic_messages">Anthropic Messages</option><option value="gemini_generate_content">Gemini Generate Content</option></select></label>
            <label className="config-span-2"><span>Provider base URL</span><input aria-label="Provider base URL" value={providerUrl} onChange={(event) => setProviderUrl(event.target.value)} placeholder="https://api.example.com/v1" maxLength={2048} autoComplete="url" spellCheck={false} /></label>
            <label><span>Model</span><input aria-label="Provider model" value={providerModel} onChange={(event) => setProviderModel(event.target.value)} placeholder="model-name" maxLength={256} autoComplete="off" spellCheck={false} /></label>
            <label><span>Secret reference</span><input aria-label="API key environment name" value={apiKeyEnvironment} onChange={(event) => setApiKeyEnvironment(event.target.value.toUpperCase())} placeholder="PANDORA_CUSTOM_API_KEY" maxLength={128} autoComplete="off" spellCheck={false} /></label>
            <label className="config-span-2"><span>API key <small>optional when the secret already exists</small></span><input aria-label="Provider API key" type="password" value={apiKey} onChange={(event) => setApiKey(event.target.value)} placeholder="Stored in Pandora’s encrypted native vault" maxLength={65535} autoComplete="new-password" spellCheck={false} /></label>
          </div>
          <div className="config-form-footer"><p><Icon name="lock" size={13} /> API keys pass through process stdin, are encrypted by Pandora’s vault, cleared from the form, and never saved in browser storage.</p><button className="button button-primary" type="submit" disabled={!providerReady || configurationBusy}>{configurationBusy ? "Saving…" : "Save provider"} <Icon name="arrow" size={14} /></button></div>
        </form> : <form className="native-config-form" onSubmit={(event) => void submitMcp(event)}>
          <div className="config-form-grid">
            <label><span>Server ID</span><input aria-label="MCP server ID" value={mcpServerId} onChange={(event) => setMcpServerId(event.target.value)} placeholder="local-tools" maxLength={64} autoComplete="off" spellCheck={false} /></label>
            <label><span>Protocol mode</span><select aria-label="MCP protocol mode" value={mcpMode} onChange={(event) => setMcpMode(event.target.value as typeof mcpMode)}><option value="auto">Auto negotiate</option><option value="modern-only">Modern only</option><option value="legacy-only">Legacy only</option></select></label>
            <label className="config-span-2"><span>Absolute program path</span><input aria-label="MCP program path" value={mcpProgram} onChange={(event) => setMcpProgram(event.target.value)} placeholder="C:\path\to\mcp-server.exe" maxLength={4096} autoComplete="off" spellCheck={false} /></label>
            <label className="config-span-2"><span>Arguments <small>JSON array of strings</small></span><textarea aria-label="MCP arguments JSON" value={mcpArguments} onChange={(event) => setMcpArguments(event.target.value)} rows={3} maxLength={65535} spellCheck={false} /></label>
          </div>
          <div className="config-form-footer"><p><Icon name="shield" size={13} /> Pandora records the local executable and arguments. Tool authority is still granted separately by policy.</p><button className="button button-primary" type="submit" disabled={!mcpReady || configurationBusy}>{configurationBusy ? "Saving…" : "Save MCP server"} <Icon name="arrow" size={14} /></button></div>
        </form>}
        {configurationMessage ? <p className="configuration-result is-success" role="status"><Icon name="check" size={14} /> {configurationMessage}</p> : null}
        {configurationError ? <p className="configuration-result is-error" role="alert">{configurationError}</p> : null}
      </Panel> : null}

      <Panel className="connection-panel">
        <div className="panel-heading"><div><span className="eyebrow">SCOPED SESSIONS</span><h3>{sessions.length} available</h3></div><Chip tone="green" icon="shield">Workspace scoped</Chip></div>
        {sessions.length ? <div className="session-list">{sessions.map((session) => <button className={`session-row ${selectedSessionId === session.session_id ? "is-selected" : ""}`} key={session.session_id} type="button" onClick={() => void onSelectSession(session.session_id)}><span className="session-dot" /><span><strong>{session.session_id}</strong><small>{session.workspace_id} · local principal</small></span><Icon name="chevron" size={13} /></button>)}</div> : <div className="connection-empty"><Icon name="archive" size={21} /><p>Connect to load workspace sessions.</p></div>}
        {selectedSession ? <div className="selected-session"><span className="eyebrow">SELECTED SESSION</span><strong>{selectedSession.session.session_id}</strong><small>{selectedSession.event_count} recorded events · ready to inspect</small></div> : null}
      </Panel>
    </div>
  </div>;
}
function EvolutionView({ proposals, activations, runtimeStatus, onInspect, onMutate }: { proposals: RuntimeEvolutionProposal[]; activations: RuntimeArtifactActivation[]; runtimeStatus: RuntimeStatus; onInspect: (proposalId: string) => Promise<RuntimeEvolutionProposal>; onMutate: (operation: "activate" | "rollback", proposalId: string, confirmation: string, reason: string) => Promise<RuntimeEvolutionMutation> }) {
  const [pending, setPending] = useState<{ operation: "activate" | "rollback"; proposalId: string } | null>(null);
  const [confirmation, setConfirmation] = useState("");
  const [reason, setReason] = useState("");
  const [mutationError, setMutationError] = useState("");
  const [mutationInFlight, setMutationInFlight] = useState(false);
  const [receipt, setReceipt] = useState<RuntimeEvolutionMutation | null>(null);
  const [inspectInFlight, setInspectInFlight] = useState<string | null>(null);
  const [inspectError, setInspectError] = useState<{ proposalId: string; message: string } | null>(null);
  const stateTone = (state: string): "neutral" | "green" | "amber" | "blue" | "gold" => state === "active" ? "green" : state.includes("failed") || state === "rolled_back" ? "amber" : state === "approved" || state === "staged" || state === "canary_passed" ? "gold" : state === "evaluated" ? "blue" : "neutral";

  const inspectCandidate = async (proposalId: string) => {
    setInspectInFlight(proposalId);
    setInspectError(null);
    try {
      await onInspect(proposalId);
    } catch (error: unknown) {
      setInspectError({ proposalId, message: error instanceof Error ? error.message : "Candidate inspection failed" });
    } finally {
      setInspectInFlight(null);
    }
  };

  const beginMutation = (operation: "activate" | "rollback", proposalId: string) => {
    setPending({ operation, proposalId });
    setConfirmation("");
    setReason("");
    setMutationError("");
    setReceipt(null);
  };

  const submitMutation = async (event: FormEvent) => {
    event.preventDefault();
    if (!pending || confirmation !== pending.proposalId || (pending.operation === "rollback" && !reason.trim())) {
      return;
    }
    setMutationInFlight(true);
    setMutationError("");
    try {
      const nextReceipt = await onMutate(pending.operation, pending.proposalId, confirmation, reason.trim());
      setReceipt(nextReceipt);
      setPending(null);
      setConfirmation("");
      setReason("");
    } catch (error: unknown) {
      setMutationError(error instanceof Error ? error.message : "Evolution mutation failed");
    } finally {
      setMutationInFlight(false);
    }
  };

  return <div className="full-view"><PageHeader eyebrow="Governed improvement" title="Evolution" description="Inspect evidence, release gates, admitted bindings, and guarded activation or rollback receipts." actions={<Chip tone={activations.length ? "green" : proposals.length ? "gold" : runtimeStatus === "connected" ? "neutral" : "amber"} icon="shield">{activations.length ? `${activations.length} active binding${activations.length === 1 ? "" : "s"}` : proposals.length ? `${proposals.length} proposal${proposals.length === 1 ? "" : "s"}` : "No proposals"}</Chip>} />
    <div className="engine-notice evolution-boundary"><Icon name="lock" size={16} /><span>Activation never grants new authority. Pandora blocks mutation while executions are active, validates admitted artifacts, snapshots every evolution database, and requires the exact proposal ID.</span></div>
    {receipt ? <Panel className="evolution-receipt"><div className="panel-heading"><div><span className="eyebrow">MUTATION RECEIPT</span><h3>{receipt.operation === "activate" ? "Candidate activated" : "Binding rolled back"}</h3></div><Chip tone="green">recorded</Chip></div><div className="evolution-explanation"><div><span>What changed</span><strong>{receipt.proposal_id} → {receipt.artifact}</strong></div><div><span>Result</span><strong>{receipt.state.replaceAll("_", " ")} · {receipt.reconciled_bindings} reconciled binding{receipt.reconciled_bindings === 1 ? "" : "s"}</strong></div><div><span>Recovery point</span><strong className="mono">{receipt.backup_directory}</strong></div><div><span>When</span><strong>{new Date(receipt.occurred_at_unix_seconds * 1000).toLocaleString()}</strong></div></div></Panel> : null}
    {proposals.length || activations.length ? <div className="secondary-grid evolution-grid">{activations.map((activation) => <Panel className="secondary-card evolution-card" key={`active-${activation.proposal_id}`}><div className="panel-heading"><div><span className="eyebrow">ACTIVE ARTIFACT · {activation.proposal_id}</span><h3>Admitted artifact binding</h3></div><Chip tone="green">catalog active</Chip></div><div className="secondary-icon secondary-icon-0"><Icon name="stack" size={20} /></div><div className="settings-facts"><div><span>Base artifact</span><strong className="mono">{activation.base_artifact}</strong></div><div><span>Resolved artifact</span><strong className="mono">{activation.candidate_artifact}</strong></div><div><span>Activated</span><strong>{new Date(activation.activated_at_unix_seconds * 1000).toLocaleString()}</strong></div><div><span>Runtime authority</span><strong>Unchanged</strong></div></div></Panel>)}
      {proposals.map((proposal, index) => {
        const activation = activations.find((candidate) => candidate.proposal_id === proposal.proposal_id);
        const canActivate = !activation && proposal.state === "canary_passed";
        const isPending = pending?.proposalId === proposal.proposal_id;
        return <Panel className="secondary-card evolution-card" key={proposal.proposal_id}><div className="panel-heading"><div><span className="eyebrow">{proposal.source} · {proposal.proposal_id}</span><h3>{proposal.expected_outcome}</h3></div><Chip tone={stateTone(proposal.state)}>{proposal.state.replaceAll("_", " ")}</Chip></div><div className={`secondary-icon secondary-icon-${index % 3}`}><Icon name="evolution" size={20} /></div><div className="settings-facts"><div><span>Artifact transition</span><strong>{proposal.base_artifact} → {proposal.candidate_artifact}</strong></div><div><span>Evidence digest</span><strong className="mono">{proposal.evidence_digest}</strong></div><div><span>Why proposed</span><strong>{proposal.expected_outcome}</strong></div><div><span>Holdout gate</span><strong>{proposal.evaluation ? `${proposal.evaluation.holdout_passed ? "Passed" : "Failed"} · ${proposal.evaluation.trajectory_score}/${proposal.evaluation.outcome_score}` : "Not evaluated"}</strong></div><div><span>Policy / regression</span><strong>{proposal.evaluation ? `${proposal.evaluation.policy_passed ? "Pass" : "Fail"} / ${proposal.evaluation.regression_passed ? "Pass" : "Fail"}` : "Pending"}</strong></div><div><span>Who approved</span><strong>{proposal.approval ? `${proposal.approval.approver_id} · policy v${proposal.approval.policy_version}` : "Not approved"}</strong></div><div><span>Signed artifact</span><strong>{proposal.approval?.signature_present ? `Verified · ${proposal.approval.signer_id}` : "Absent"}</strong></div><div><span>Canary</span><strong>{proposal.canary ? `${proposal.canary.passed ? "Passed" : "Failed"} · ${proposal.canary.failure_count} failures` : "Not run"}</strong></div><div><span>Candidate diff</span><strong>{proposal.candidate ? `${proposal.candidate.changed_units} changed · +${proposal.candidate.added_units} / −${proposal.candidate.removed_units} ${proposal.candidate.unit} · ${proposal.candidate.base_bytes} → ${proposal.candidate.candidate_bytes} bytes` : "Structural diff unavailable"}</strong></div><div><span>Provenance</span><strong>{proposal.candidate ? `${proposal.candidate.kind} · ${proposal.candidate.target_id} · ${proposal.candidate.provider_id}` : `${proposal.source} · ${proposal.evidence_digest}`}</strong></div></div><div className="evolution-lineage"><span className="eyebrow">LINEAGE</span><div className="lineage-chain"><div><small>Parent</small><strong className="mono">{proposal.base_artifact}</strong></div><Icon name="arrow" size={14} /><div><small>Candidate</small><strong className="mono">{proposal.candidate_artifact}</strong></div><Icon name="arrow" size={14} /><div><small>State</small><strong>{proposal.state.replaceAll("_", " ")}</strong></div></div><p>Bound by evidence <span className="mono">{proposal.evidence_digest}</span>{proposal.candidate ? ` · proposed by ${proposal.candidate.provider_id}` : ""}.</p>{!proposal.candidate ? <button className="text-link lineage-inspect" type="button" disabled={inspectInFlight !== null} onClick={() => void inspectCandidate(proposal.proposal_id)}>{inspectInFlight === proposal.proposal_id ? "Inspecting candidate…" : "Inspect candidate diff"} <Icon name="arrow" size={13} /></button> : null}{inspectError?.proposalId === proposal.proposal_id ? <p className="connection-error" role="alert">{inspectError.message}</p> : null}</div>
          {canActivate || activation ? <div className="evolution-actions"><button className={activation ? "button button-secondary" : "button button-primary"} type="button" disabled={mutationInFlight} onClick={() => beginMutation(activation ? "rollback" : "activate", proposal.proposal_id)}>{activation ? "Rollback binding" : "Activate candidate"} <Icon name={activation ? "archive" : "arrow"} size={14} /></button></div> : <p className="evolution-gate-note"><Icon name="shield" size={13} /> Activation stays unavailable until evaluation, approval, signature, admission, and canary gates pass.</p>}
          {isPending ? <form className="evolution-confirm" onSubmit={submitMutation}><div><span className="eyebrow">EXACT CONFIRMATION</span><strong>{pending.operation === "activate" ? "Activate admitted candidate" : "Restore previous binding"}</strong><p>Type <span className="mono">{proposal.proposal_id}</span> to confirm this exact mutation. A verified backup is created first.</p></div><label><span>Proposal ID</span><input aria-label={`Confirm ${pending.operation} ${proposal.proposal_id}`} value={confirmation} onChange={(event) => setConfirmation(event.target.value)} autoComplete="off" spellCheck={false} /></label>{pending.operation === "rollback" ? <label><span>Rollback reason</span><textarea aria-label={`Rollback reason ${proposal.proposal_id}`} value={reason} onChange={(event) => setReason(event.target.value)} rows={2} maxLength={500} /></label> : null}<div className="evolution-confirm-actions"><button className="button button-secondary" type="button" onClick={() => setPending(null)} disabled={mutationInFlight}>Cancel</button><button className="button button-primary" type="submit" disabled={mutationInFlight || confirmation !== proposal.proposal_id || (pending.operation === "rollback" && !reason.trim())}>{mutationInFlight ? "Applying…" : pending.operation === "activate" ? "Confirm activation" : "Confirm rollback"}</button></div>{mutationError ? <p className="connection-error" role="alert">{mutationError}</p> : null}</form> : null}
        </Panel>;
      })}</div> : <div className="workflow-empty"><div className="empty-orbit"><Icon name="evolution" size={25} /></div><h2>No evolution proposals</h2><p>{runtimeStatus === "connected" ? "The durable evolution and artifact catalogs are available. Self-improvement begins with measured evidence; permission remains separate." : "Connect the local Pandora service to inspect the durable evolution store."}</p></div>}
  </div>;
}

function SecondaryView({ view, runtimeStatus }: { view: Exclude<ViewId, "command" | "runs" | "memory" | "workflows" | "engines">; runtimeStatus: RuntimeStatus }) {
  const detail = viewDetails[view];
  const cards = view === "connections" ? ["Local provider profile", "MCP stdio · local", "Connection policy"] : view === "capabilities" ? ["coding-domain", "research-domain", "Installed Skills"] : view === "evolution" ? ["Proposal intake", "Holdout evaluation", "Rollback readiness"] : view === "audit" ? ["Effect receipts", "Evaluation evidence", "Runtime events"] : ["Policy posture", "Workspace boundary", "Runtime configuration"];
  const availability = runtimeStatus === "connected" ? "The current service does not expose this surface yet." : "Connect the local Pandora service when this surface is available.";
  return <div className="full-view"><PageHeader eyebrow={detail.eyebrow} title={detail.title} description={`${detail.description} This view is currently a design preview.`} actions={<button className="button button-secondary" disabled><Icon name="download" size={14} /> Export view</button>} /><div className="secondary-grid">{cards.map((card, index) => <Panel className="secondary-card" key={card}><div className={`secondary-icon secondary-icon-${index % 3}`}><Icon name={index === 0 ? "shield" : index === 1 ? "graph" : "stack"} size={20} /></div><span className="eyebrow">PREVIEW</span><h3>{card}</h3><p>{availability}</p><button className="text-link" disabled>Inspect boundary <Icon name="arrow" size={13} /></button></Panel>)}</div></div>;
}

function PageHeader({ eyebrow, title, description, actions }: { eyebrow: string; title: string; description: string; actions: ReactNode }) {
  return <div className="page-header"><div><span className="eyebrow">{eyebrow}</span><h1>{title}</h1><p>{description}</p></div><div className="page-actions">{actions}</div></div>;
}

export { App };
