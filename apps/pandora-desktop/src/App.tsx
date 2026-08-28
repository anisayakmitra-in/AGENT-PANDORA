import { useEffect, useMemo, useRef, useState, type ChangeEvent, type FormEvent, type ReactNode } from "react";
import {
  admitLocalPackage,
  configureMcp,
  configureProvider,
  configureRegistryProfile,
  disableLocalPackage,
  enableLocalPackage,
  installGitHubPackage,
  installRegistryPackage,
  listLocalPackages,
  listRegistryProfiles,
  loadRuntimeEndpoint,
  isNativeRuntime,
  lockLocalPackages,
  nativeEndpoint,
  previewPackageRemoval,
  previewPackageDisable,
  previewPackageEnable,
  previewPackageRollback,
  removeLocalPackage,
  rollbackLocalPackage,
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
  type NativePackageResult,
  type RuntimeOrchestrationRun,
  type RuntimePackage,
  type RuntimeProvider,
  type RegistryProfile,
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
type InspectorTab = "flow" | "evidence" | "work" | "browser";
type WorkSurface = "files" | "changes" | "terminal" | "artifacts";
type HarnessTab = "genes" | "extensions" | "packages" | "authority" | "receipts";
type InventoryTab = "overview" | "contract" | "boundaries" | "evidence";

type PendingRunRequest = {
  task: string;
  requestedHarness: string | null;
};

type SubmittedRunRequest = {
  task: string;
  profile: RunProfile;
  contextAttachments: RuntimeContextAttachment[];
};

type WorkspaceInspectionRequest = {
  task: string;
  requestedHarness: string;
};

type BrowserEvidence = {
  url: string;
  status: number;
  content_type: string | null;
  body: string;
  truncated: boolean;
  lossy: boolean;
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

function parseBrowserEvidence(output: string): BrowserEvidence | null {
  try {
    const value = JSON.parse(output) as Partial<BrowserEvidence>;
    if (typeof value.url !== "string" || typeof value.status !== "number" || typeof value.body !== "string") {
      return null;
    }
    return {
      url: value.url,
      status: value.status,
      content_type: typeof value.content_type === "string" ? value.content_type : null,
      body: value.body,
      truncated: value.truncated === true,
      lossy: value.lossy === true,
    };
  } catch {
    return null;
  }
}

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
      { id: "engines", label: "Runtime Inventory", icon: "stack" },
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
    return authoritySteps.map((step) => {
      if (step.id === "harness") {
        return { ...step, label, detail: "Awaiting runtime selection", status: "idle" as const };
      }
      if (step.id === "parliament") {
        return { ...step, detail: "Begins when a request is submitted", status: "idle" as const };
      }
      if (step.id === "shadow") {
        return { ...step, detail: "No routing evidence recorded yet", status: "idle" as const };
      }
      return { ...step, status: "idle" as const };
    });
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
  const [lastRunRequest, setLastRunRequest] = useState<SubmittedRunRequest | null>(null);
  const [pendingRun, setPendingRun] = useState<PendingRunRequest | null>(null);
  const [workspaceInspection, setWorkspaceInspection] = useState<RuntimeRun | null>(null);
  const [pendingWorkspaceInspection, setPendingWorkspaceInspection] = useState<WorkspaceInspectionRequest | null>(null);
  const [workspaceInspectionInFlight, setWorkspaceInspectionInFlight] = useState(false);
  const [browserInspection, setBrowserInspection] = useState<RuntimeRun | null>(null);
  const [pendingBrowserInspection, setPendingBrowserInspection] = useState<WorkspaceInspectionRequest | null>(null);
  const [browserInspectionInFlight, setBrowserInspectionInFlight] = useState(false);
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
      setLastRunRequest(null);
      setPendingRun(null);
      setWorkspaceInspection(null);
      setPendingWorkspaceInspection(null);
      setBrowserInspection(null);
      setPendingBrowserInspection(null);
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
      setLastRunRequest(null);
      setPendingRun(null);
      setWorkspaceInspection(null);
      setPendingWorkspaceInspection(null);
      setBrowserInspection(null);
      setPendingBrowserInspection(null);
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
      setLastRunRequest(null);
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
    setLastRunRequest({
      task,
      profile,
      contextAttachments: contextAttachments.map((attachment) => ({ ...attachment })),
    });
    try {
      const requestedHarness = harnessForProfile(profile);
      const result = await client.agentRun(task, selectedSessionId || null, requestedHarness, contextAttachments);
      setPendingRun(result.approval ? { task, requestedHarness } : null);
      await loadRunResult(result);
      setRuntimeStatus("connected");
      return result;
    } catch (error: unknown) {
      setRuntimeStatus("connected");
      const message = error instanceof Error ? error.message : "Pandora run failed";
      setRuntimeError(message);
      throw error;
    } finally {
      setRunInFlight(false);
    }
  };

  const retryLastRun = async () => {
    if (!lastRunRequest) {
      throw new Error("No previous governed request is available");
    }
    return runTask(lastRunRequest.task, lastRunRequest.profile, lastRunRequest.contextAttachments);
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
      setRuntimeStatus("connected");
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

  const inspectBrowser = async (url: string): Promise<void> => {
    if (!client) {
      throw new Error("Connect to the local Pandora service first");
    }
    const request = { task: `fetch:${url}`, requestedHarness: "research-domain" };
    setBrowserInspectionInFlight(true);
    try {
      const result = await client.run(request.task, request.requestedHarness);
      setBrowserInspection(result);
      setPendingBrowserInspection(result.approval ? request : null);
      setSessions(await client.sessions());
    } finally {
      setBrowserInspectionInFlight(false);
    }
  };

  const resolveBrowserInspection = async (allow: boolean): Promise<void> => {
    const approval = browserInspection?.approval;
    if (!client || !browserInspection || !approval || !pendingBrowserInspection) {
      throw new Error("No browser approval is available");
    }
    setBrowserInspectionInFlight(true);
    try {
      const resolved = approval.status === "pending"
        ? await client.resolveApproval(approval.approval_id, allow)
        : approval;
      setBrowserInspection({ ...browserInspection, approval: resolved });
      if (allow) {
        const result = await client.resume(
          approval.approval_id,
          pendingBrowserInspection.task,
          pendingBrowserInspection.requestedHarness,
        );
        setBrowserInspection(result);
      } else {
        setBrowserInspection({
          ...browserInspection,
          status: "denied",
          status_detail: "The operator denied this exact browser request.",
          approval: resolved,
        });
      }
      setPendingBrowserInspection(null);
      setSessions(await client.sessions());
    } finally {
      setBrowserInspectionInFlight(false);
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
            selectedStep={selectedStep}
            onSelectStep={setSelectedStep}
            runtimeStatus={runtimeStatus}
            selectedSession={selectedSession}
            lastRun={lastRun}
            lastRunRequest={lastRunRequest}
            events={events}
            harnesses={harnesses}
            runInFlight={runInFlight}
            workspaceInspection={workspaceInspection}
            workspaceInspectionInFlight={workspaceInspectionInFlight}
            browserInspection={browserInspection}
            browserInspectionInFlight={browserInspectionInFlight}
            runProfile={runProfile}
            onRunProfileChange={setRunProfile}
            onRun={runTask}
            onRetryRun={retryLastRun}
            onResolveApproval={resolvePendingApproval}
            onInspectWorkspace={inspectWorkspace}
            onResolveWorkspaceInspection={resolveWorkspaceInspection}
            onInspectBrowser={inspectBrowser}
            onResolveBrowserInspection={resolveBrowserInspection}
          />
        ) : activeView === "runs" ? (
          <RunsView runs={orchestrationRuns} runtimeStatus={runtimeStatus} onMutate={mutateOrchestration} />
        ) : activeView === "council" ? (
          <CouncilView
            runtimeStatus={runtimeStatus}
            lastRun={lastRun}
            events={events}
            selectedSession={selectedSession}
            harnesses={harnesses}
            onOpenCommand={() => setActiveView("command")}
            onOpenAudit={() => setActiveView("audit")}
          />
        ) : activeView === "memory" ? (
          <MemoryView runtimeStatus={runtimeStatus} records={memoryRecords} selectedSession={selectedSession} />
        ) : activeView === "workflows" ? (
          <WorkflowsView runtimeStatus={runtimeStatus} workflows={workflows} harnesses={harnesses} onOpenCommand={() => setActiveView("command")} onCreate={createWorkflow} onRemove={removeWorkflow} onRun={runWorkflow} />
        ) : activeView === "connections" ? (
          <ConnectionView endpoint={endpoint} runtimeStatus={runtimeStatus} runtimeError={runtimeError} health={runtimeHealth} providers={providers} sessions={sessions} selectedSessionId={selectedSessionId} selectedSession={selectedSession} native={native} serviceActive={serviceActive} onConnect={connect} onStartService={startService} onStopService={stopService} onSelectSession={openSession} />
        ) : activeView === "audit" ? (
          <AuditView events={events} selectedSession={selectedSession} runtimeStatus={runtimeStatus} />
        ) : activeView === "capabilities" ? (
          <CapabilitiesView harnesses={harnesses} tools={tools} runtimeStatus={runtimeStatus} native={native} />
        ) : activeView === "engines" ? (
          <RuntimeInventoryView engines={engines} runtimeStatus={runtimeStatus} onOpenView={setActiveView} />
        ) : activeView === "tools" ? (
          <ToolsView tools={tools} runtimeStatus={runtimeStatus} onOpenView={setActiveView} />
        ) : activeView === "evolution" ? (
          <EvolutionView proposals={evolutionProposals} activations={artifactActivations} runtimeStatus={runtimeStatus} onInspect={inspectEvolutionCandidate} onMutate={mutateEvolution} />
        ) : (
          <SettingsView theme={theme} onThemeChange={setTheme} runtimeStatus={runtimeStatus} health={runtimeHealth} native={native} endpoint={endpoint} />
        )}
      </main>
    </div>
  );
}

function Sidebar({ activeView, onSelect, runtimeStatus, sessions, selectedSessionId, onOpenPalette, onOpenSession }: { activeView: ViewId; onSelect: (view: ViewId) => void; runtimeStatus: RuntimeStatus; sessions: RuntimeSession[]; selectedSessionId: string; onOpenPalette: () => void; onOpenSession: (sessionId: string) => Promise<void> }) {
  const threads = sessions.map((session) => ({ title: session.session_id, meta: session.workspace_id, sessionId: session.session_id, active: selectedSessionId === session.session_id }));
  return (
    <aside className="sidebar">
      <div className="brand-lockup">
        <button className="brand-mark" type="button" aria-label="Open Command" onClick={() => onSelect("command")}><span>P</span></button>
        <div><strong>Pandora</strong><span>local control plane</span></div>
        <span className="brand-edition">β7</span>
      </div>
      <button type="button" className="rail-search" onClick={onOpenPalette}><Icon name="search" size={15} /><span>Find a surface</span><kbd>Ctrl K</kbd></button>
      <nav className="navigation" aria-label="Pandora navigation">
        {navigation.map((group) => <div className="nav-group" key={group.label}>
          <span className="nav-label">{group.label}</span>
          {group.items.map((item, itemIndex) => <button className={`nav-item ${activeView === item.id ? "is-active" : ""}`} key={item.id} onClick={() => onSelect(item.id)} aria-label={item.label} aria-current={activeView === item.id ? "page" : undefined}>
            <span className="nav-count">{String(itemIndex + 1).padStart(2, "0")}</span><Icon name={item.icon} size={16} /><span>{item.label}</span>
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
        <div className="footer-meta"><span>Local only</span><span className="footer-separator">/</span><button className="text-icon-button" aria-label="Open settings" onClick={() => onSelect("settings")}><Icon name="gear" size={13} /> Settings</button></div>
      </div>
    </aside>
  );
}

function TopBar({ activeView, runtimeStatus, onOpenPalette }: { activeView: ViewId; runtimeStatus: RuntimeStatus; onOpenPalette: () => void }) {
  const label = activeView === "command" ? "Command Center" : activeView === "runs" ? "Background Runs" : activeView[0].toUpperCase() + activeView.slice(1);
  const section = navigation.find((group) => group.items.some((item) => item.id === activeView))?.label ?? "Workspace";
  const tone = runtimeStatus === "connected" ? "green" : runtimeStatus === "offline" ? "amber" : "blue";
  return <header className="top-bar"><div className="breadcrumb"><span className="breadcrumb-muted">{section}</span><span className="breadcrumb-rule" /><strong>{label}</strong></div><div className="top-actions"><button className="top-search" type="button" aria-label="Search" onClick={onOpenPalette}><Icon name="search" size={14} /><span>Quick open</span><kbd>Ctrl K</kbd></button><Chip tone={tone} icon="lock">{runtimeStatusLabel(runtimeStatus)}</Chip></div></header>;
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
      return "Local service stopped";
  }
}

function RunResultPanel({ lastRun, request, events, runInFlight, onRepeat }: { lastRun: RuntimeRun; request: SubmittedRunRequest | null; events: RuntimeEvent[]; runInFlight: boolean; onRepeat: () => Promise<void> }) {
  const canRepeat = Boolean(request && lastRun.status !== "approval_required");
  const repeatLabel = lastRun.status === "failed" || lastRun.status === "denied"
    ? "Retry with fresh verification"
    : "Run again";
  return <Panel className={`run-result run-result-${lastRun.status}`}>
    <div className="panel-heading"><div><span className="eyebrow">RECORDED EXECUTION</span><h3>Latest {lastRun.mode} run</h3></div><Chip tone={lastRun.status === "completed" ? "green" : "amber"}>{lastRun.status}</Chip></div>
    <div className="run-result-meta"><span className="mono">{lastRun.execution_id ?? "provider-only response"}</span><span>{lastRun.selected_gene ?? "No gene selected"}</span></div>
    {request ? <div className="run-request-evidence"><span>Request snapshot</span><p>{request.task}</p><small>{request.profile === "auto" ? "Auto route" : request.profile} · {request.contextAttachments.length} context file{request.contextAttachments.length === 1 ? "" : "s"}</small></div> : null}
    {lastRun.status_detail ? <div className="run-status-detail"><Icon name={lastRun.status === "failed" || lastRun.status === "denied" ? "shield" : "activity"} size={14} /><span>{lastRun.status_detail}</span></div> : null}
    <p className="run-output">{lastRun.output || "No output returned."}</p>
    {canRepeat ? <div className="run-result-actions"><p>{lastRun.status === "failed" || lastRun.status === "denied" ? "A retry creates a new execution and re-runs every policy, evaluation, and permit check." : "Repeating this request creates a new governed execution. Previous permits are never reused."}</p><button className="button button-secondary" type="button" disabled={runInFlight} onClick={() => void onRepeat()}>{runInFlight ? "Running…" : repeatLabel} <Icon name="arrow" size={13} /></button></div> : null}
    {events.length ? <div className="event-list"><span className="eyebrow">LIVE ACTIVITY</span>{events.map((event) => <div className="event-row" key={event.event_id}><span className="event-dot" /><span>{event.event_type.replaceAll("_", " ")}</span><span className="mono">{event.event_id}</span></div>)}</div> : null}
  </Panel>;
}

type CommandViewProps = {
  selectedStep: string;
  onSelectStep: (id: string) => void;
  runtimeStatus: RuntimeStatus;
  selectedSession: RuntimeSessionDetail | null;
  lastRun: RuntimeRun | null;
  lastRunRequest: SubmittedRunRequest | null;
  events: RuntimeEvent[];
  harnesses: RuntimeHarness[];
  runInFlight: boolean;
  workspaceInspection: RuntimeRun | null;
  workspaceInspectionInFlight: boolean;
  browserInspection: RuntimeRun | null;
  browserInspectionInFlight: boolean;
  runProfile: RunProfile;
  onRunProfileChange: (profile: RunProfile) => void;
  onRun: (task: string, profile: RunProfile, contextAttachments: RuntimeContextAttachment[]) => Promise<RuntimeRun>;
  onRetryRun: () => Promise<RuntimeRun>;
  onResolveApproval: (allow: boolean) => Promise<void>;
  onInspectWorkspace: (task: string) => Promise<void>;
  onResolveWorkspaceInspection: (allow: boolean) => Promise<void>;
  onInspectBrowser: (url: string) => Promise<void>;
  onResolveBrowserInspection: (allow: boolean) => Promise<void>;
};

function CommandView({ selectedStep, onSelectStep, runtimeStatus, selectedSession, lastRun, lastRunRequest, events, harnesses, runInFlight, workspaceInspection, workspaceInspectionInFlight, browserInspection, browserInspectionInFlight, runProfile, onRunProfileChange, onRun, onRetryRun, onResolveApproval, onInspectWorkspace, onResolveWorkspaceInspection, onInspectBrowser, onResolveBrowserInspection }: CommandViewProps) {
  const [task, setTask] = useState("");
  const [runError, setRunError] = useState("");
  const [contextAttachments, setContextAttachments] = useState<RuntimeContextAttachment[]>([]);
  const contextInput = useRef<HTMLInputElement>(null);
  const composerForm = useRef<HTMLFormElement>(null);

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

  const repeatRecordedRun = async () => {
    if (runInFlight || !lastRunRequest) {
      return;
    }
    setRunError("");
    try {
      await onRetryRun();
    } catch (error: unknown) {
      setRunError(error instanceof Error ? error.message : "Pandora retry failed");
    }
  };

  const retryPreservedRequest = () => {
    setRunError("");
    composerForm.current?.requestSubmit();
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
      <div className="stage-toolbar"><div><span className="eyebrow">LOCAL WORKSPACE</span><strong>{selectedSession?.session.workspace_id ?? "unscoped"}</strong></div><div className="stage-controls"><span className="mono">CONTROL / 01</span><Chip tone={connected ? "green" : runtimeStatus === "offline" ? "amber" : "blue"} icon="activity">{runtimeStatusLabel(runtimeStatus)}</Chip></div></div>
      <div className="core-stage">
        <div className="stage-grid" />
        <div className="core-status-dock">
          <div className="core-caption"><span className="eyebrow">PANDORA / GOVERNED EXECUTION</span><h1>{coreTitle}</h1><p>{coreDescription}</p><div className="core-constitution"><span><Icon name="council" size={13} /> Parliament plans</span><span><Icon name="users" size={13} /> Shadow Council routes</span><span><Icon name="shield" size={13} /> ReferenceMonitor permits</span></div></div>
          <div className="core-metrics"><Metric label="Context" value={selectedSession ? "Scoped" : "None"} detail={selectedSession ? selectedSession.session.workspace_id : "not loaded"} /><Metric label="Policy" value={policyValue} detail={policyDetail} /><Metric label="Evidence" value={evidenceValue} detail={evidenceDetail} /></div>
        </div>
      </div>
      <form ref={composerForm} className="composer-wrap" onSubmit={submit}>
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
        <div className="composer-hint"><span>{runInFlight ? "Pandora is running the governed request…" : connected ? "Ctrl/⌘ + Enter to send" : "Connect the local service in Connections"}</span><span>{contextAttachments.length ? `${contextAttachments.length} context file${contextAttachments.length === 1 ? "" : "s"} · effects still require a permit` : "All effects require an exact permit"}</span></div>
        {runError ? <div className="composer-recovery" role="alert"><Icon name="shield" size={16} /><div><strong>Request did not complete</strong><span>{runError}</span><small>The task and selected context remain editable. Retrying creates a fresh governed request.</small></div><div><button className="button button-secondary" type="button" disabled={runInFlight || !task.trim()} onClick={retryPreservedRequest}>Retry request</button><button className="text-link" type="button" onClick={() => setRunError("")}>Dismiss</button></div></div> : null}
      </form>
      {lastRun ? <RunResultPanel lastRun={lastRun} request={lastRunRequest} events={events} runInFlight={runInFlight} onRepeat={repeatRecordedRun} /> : null}
    </section>
    <Inspector steps={steps} lastRun={lastRun} events={events} selectedSession={selectedSession} runtimeStatus={runtimeStatus} approval={lastRun?.approval} approvalDetail={lastRun?.status_detail} approvalInFlight={runInFlight} workspaceInspection={workspaceInspection} workspaceInspectionInFlight={workspaceInspectionInFlight} browserInspection={browserInspection} browserInspectionInFlight={browserInspectionInFlight} selectedStep={selectedStep} onResolveApproval={onResolveApproval} onInspectWorkspace={onInspectWorkspace} onResolveWorkspaceInspection={onResolveWorkspaceInspection} onInspectBrowser={onInspectBrowser} onResolveBrowserInspection={onResolveBrowserInspection} onSelectStep={onSelectStep} />
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

function workSurfaceForTask(task: string): WorkSurface {
  if (task.startsWith("read:")) return "files";
  if (["status", "diff", "log", "refs"].includes(task)) return "changes";
  return "terminal";
}

function WorkspaceEvidenceOutput({ task, output }: { task: string; output: string }) {
  if (task.startsWith("read:")) {
    const path = task.slice("read:".length);
    const lines = output.split("\n");
    if (lines.at(-1) === "") lines.pop();
    return <div className="file-viewer" aria-label="Workspace inspection output"><div className="work-output-toolbar"><span className="mono">{path}</span><span>{lines.length} line{lines.length === 1 ? "" : "s"}</span></div><div className="file-viewer-code">{lines.map((line, index) => <div className="file-line" key={`${index}-${line}`}><span>{index + 1}</span><code>{line || " "}</code></div>)}</div></div>;
  }
  if (task === "diff") {
    const lines = output.split("\n");
    if (lines.at(-1) === "") lines.pop();
    return <div className="diff-viewer" aria-label="Workspace inspection output"><div className="work-output-toolbar"><span>Working tree changes</span><span>{lines.length} lines</span></div><div className="diff-viewer-code">{lines.map((line, index) => {
      const tone = line.startsWith("+++") || line.startsWith("---") || line.startsWith("diff ") || line.startsWith("index ")
        ? "meta"
        : line.startsWith("@@")
          ? "hunk"
          : line.startsWith("+")
            ? "added"
            : line.startsWith("-")
              ? "removed"
              : "context";
      return <div className={`diff-line is-${tone}`} key={`${index}-${line}`}><span>{index + 1}</span><code>{line || " "}</code></div>;
    })}</div></div>;
  }
  return <div className="terminal-viewer" aria-label="Workspace inspection output"><div className="work-output-toolbar"><span className="mono">{task}</span><span>registered Gene output</span></div><pre>{output}</pre></div>;
}

function WorkSurfacePanel({
  surface,
  runtimeStatus,
  workspacePath,
  workspaceInspectionInFlight,
  lastRun,
  onPathChange,
  onReadFile,
  onInspect,
}: {
  surface: WorkSurface;
  runtimeStatus: RuntimeStatus;
  workspacePath: string;
  workspaceInspectionInFlight: boolean;
  lastRun: RuntimeRun | null;
  onPathChange: (path: string) => void;
  onReadFile: (event: FormEvent) => Promise<void>;
  onInspect: (task: string) => Promise<void>;
}) {
  const connected = runtimeStatus === "connected";

  if (surface === "files") {
    return <Panel className="workspace-browser-panel work-surface-panel">
      <div className="panel-heading">
        <div><span className="eyebrow">FILE VIEWER</span><h3>Read workspace evidence</h3></div>
        <Chip tone="blue" icon="book">Read only</Chip>
      </div>
      <form className="workspace-read-form" onSubmit={(event) => void onReadFile(event)}>
        <label><span>Workspace-relative path</span><input aria-label="Workspace file path" value={workspacePath} onChange={(event) => onPathChange(event.target.value)} maxLength={1024} spellCheck={false} autoComplete="off" /></label>
        <button className="button button-secondary" type="submit" disabled={!connected || workspaceInspectionInFlight || !workspacePath.trim()}>{workspaceInspectionInFlight ? "Reading…" : "Read file"}</button>
      </form>
      <p className="work-surface-note">Text is returned with line numbers through the symlink-safe filesystem Gene.</p>
    </Panel>;
  }

  if (surface === "changes") {
    const commands: Array<{ task: string; label: string; gene: string; icon: IconName }> = [
      { task: "status", label: "Git status", gene: "workspace.status", icon: "terminal" },
      { task: "diff", label: "Working diff", gene: "workspace.diff", icon: "code" },
      { task: "log", label: "Recent log", gene: "workspace.log", icon: "archive" },
      { task: "refs", label: "Branches & refs", gene: "workspace.refs", icon: "stack" },
    ];
    return <Panel className="work-surface-panel">
      <div className="panel-heading"><div><span className="eyebrow">CHANGES</span><h3>Inspect repository state</h3></div><Chip tone="blue">Inert diff</Chip></div>
      <div className="work-command-grid">{commands.map((command) => <button type="button" key={command.task} disabled={!connected || workspaceInspectionInFlight} onClick={() => void onInspect(command.task)}><Icon name={command.icon} size={15} /><span><strong>{command.label}</strong><small>{command.gene}</small></span><Icon name="chevron" size={12} /></button>)}</div>
      <p className="work-surface-note">Repository evidence is rendered as text. No patch content is executed.</p>
    </Panel>;
  }

  if (surface === "terminal") {
    const commands: Array<{ task: string; label: string; gene: string }> = [
      { task: "test", label: "Tests", gene: "workspace.test" },
      { task: "lint", label: "Lint", gene: "workspace.lint" },
      { task: "build", label: "Build", gene: "workspace.build" },
      { task: "verify", label: "Verify", gene: "workspace.verify" },
      { task: "format", label: "Format check", gene: "workspace.format" },
    ];
    return <Panel className="work-surface-panel">
      <div className="panel-heading"><div><span className="eyebrow">BOUNDED TERMINAL</span><h3>Run registered checks</h3></div><Chip tone="amber" icon="lock">No arbitrary shell</Chip></div>
      <div className="work-command-grid">{commands.map((command) => <button type="button" key={command.task} disabled={!connected || workspaceInspectionInFlight} onClick={() => void onInspect(command.task)}><Icon name="terminal" size={15} /><span><strong>{command.label}</strong><small>{command.gene}</small></span><Icon name="chevron" size={12} /></button>)}</div>
      <p className="work-surface-note">Every action resolves to a registered Gene and follows the exact permit and receipt path.</p>
    </Panel>;
  }

  return <Panel className="work-surface-panel artifact-work-panel">
    <div className="panel-heading"><div><span className="eyebrow">ARTIFACTS</span><h3>Latest bounded output</h3></div><Chip tone={lastRun?.status === "completed" ? "green" : "neutral"} icon="archive">{lastRun?.status ?? "empty"}</Chip></div>
    {lastRun ? <>
      <div className="artifact-work-meta"><div><span>Execution</span><strong className="mono">{lastRun.execution_id ?? "provider-only"}</strong></div><div><span>Receipts</span><strong>{lastRun.receipt_count}</strong></div><div><span>Harness</span><strong>{lastRun.selected_harness ?? "unselected"}</strong></div></div>
      {lastRun.output ? <pre className="artifact-work-output" aria-label="Run artifact output">{lastRun.output}</pre> : <p className="inspector-empty">This run did not return a text artifact.</p>}
    </> : <div className="connection-empty"><Icon name="archive" size={21} /><p>Complete a run to inspect its latest bounded output.</p></div>}
  </Panel>;
}

function Inspector({ steps, lastRun, events, selectedSession, runtimeStatus, approval, approvalDetail, approvalInFlight, workspaceInspection, workspaceInspectionInFlight, browserInspection, browserInspectionInFlight, selectedStep, onResolveApproval, onInspectWorkspace, onResolveWorkspaceInspection, onInspectBrowser, onResolveBrowserInspection, onSelectStep }: { steps: typeof authoritySteps; lastRun: RuntimeRun | null; events: RuntimeEvent[]; selectedSession: RuntimeSessionDetail | null; runtimeStatus: RuntimeStatus; approval?: RuntimeApproval; approvalDetail?: string; approvalInFlight: boolean; workspaceInspection: RuntimeRun | null; workspaceInspectionInFlight: boolean; browserInspection: RuntimeRun | null; browserInspectionInFlight: boolean; selectedStep: string; onResolveApproval: (allow: boolean) => Promise<void>; onInspectWorkspace: (task: string) => Promise<void>; onResolveWorkspaceInspection: (allow: boolean) => Promise<void>; onInspectBrowser: (url: string) => Promise<void>; onResolveBrowserInspection: (allow: boolean) => Promise<void>; onSelectStep: (id: string) => void }) {
  const [approvalError, setApprovalError] = useState("");
  const [tab, setTab] = useState<InspectorTab>("flow");
  const [workSurface, setWorkSurface] = useState<WorkSurface>("files");
  const [workspaceTask, setWorkspaceTask] = useState("");
  const [workspacePath, setWorkspacePath] = useState("README.md");
  const [workspaceError, setWorkspaceError] = useState("");
  const [browserUrl, setBrowserUrl] = useState("https://example.com/");
  const [browserError, setBrowserError] = useState("");
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
    setWorkspaceTask(task);
    setWorkSurface(workSurfaceForTask(task));
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
  const workspaceResultVisible = Boolean(workspaceInspection && workspaceTask && workSurfaceForTask(workspaceTask) === workSurface);
  const fetchBrowser = async (event: FormEvent) => {
    event.preventDefault();
    setBrowserError("");
    const source = browserUrl.trim();
    try {
      const parsed = new URL(source);
      const loopback = ["localhost", "127.0.0.1", "::1", "[::1]"].includes(parsed.hostname);
      if (source.length > 2048 || parsed.username || parsed.password || parsed.search || parsed.hash || (parsed.protocol !== "https:" && !(parsed.protocol === "http:" && loopback))) {
        throw new Error("Use HTTPS, or HTTP for loopback only, without credentials, query data, or a fragment");
      }
      await onInspectBrowser(source);
    } catch (error: unknown) {
      setBrowserError(error instanceof Error ? error.message : "Browser URL is invalid");
    }
  };
  const resolveBrowser = async (allow: boolean) => {
    setBrowserError("");
    try {
      await onResolveBrowserInspection(allow);
    } catch (error: unknown) {
      setBrowserError(error instanceof Error ? error.message : "Could not resolve browser approval");
    }
  };
  const browserApproval = browserInspection?.approval;
  const browserEvidence = browserInspection?.status === "completed"
    ? parseBrowserEvidence(browserInspection.output)
    : null;
  return <aside className="inspector">
    <div className="inspector-header"><div><span className="eyebrow">{hasLiveRun ? "LIVE RUN SUMMARY" : "AUTHORITY CONTRACT"}</span><h2>{hasLiveRun ? "Execution recorded" : "Waiting for a run"}</h2></div><Chip tone={hasLiveRun ? "green" : "neutral"} icon={hasLiveRun ? "archive" : "lock"}>{hasLiveRun ? "Recorded" : "Idle"}</Chip></div>
    <div className="inspector-tabs" role="tablist" aria-label="Run inspector">
      {(["flow", "evidence", "work", "browser"] as InspectorTab[]).map((item) => <button type="button" role="tab" aria-selected={tab === item} className={tab === item ? "is-active" : ""} key={item} onClick={() => setTab(item)}>{item}</button>)}
    </div>
    {tab === "evidence" ? <div className="inspector-pane" role="tabpanel">
      <Panel className="evidence-summary"><div className="panel-heading"><div><span className="eyebrow">RUN EVIDENCE</span><h3>{lastRun ? `${lastRun.receipt_count} receipts · ${lastRun.event_count} events` : "No run selected"}</h3></div><Chip tone={lastRun?.status === "completed" ? "green" : "neutral"} icon="archive">{lastRun?.status ?? "waiting"}</Chip></div>{lastRun ? <div className="evidence-facts"><div><span>Execution</span><strong className="mono">{lastRun.execution_id ?? "provider-only"}</strong></div><div><span>Harness</span><strong>{lastRun.selected_harness ?? "unselected"}</strong></div><div><span>Gene</span><strong>{lastRun.selected_gene ?? "unselected"}</strong></div><div><span>Prompt cache</span><strong>{lastRun.cached_prompt_tokens ?? 0} reused · {lastRun.cache_write_prompt_tokens ?? 0} written</strong></div></div> : <p className="task-copy">Run or select a session to inspect its immutable evidence summary.</p>}</Panel>
      <div className="inspector-section"><div className="section-heading"><span>Redacted activity</span><span className="mono section-count">{events.length}</span></div>{events.length ? <div className="compact-event-list">{events.map((event) => <div className="compact-event" key={event.event_id}><span className="event-dot" /><span>{event.event_type.replaceAll("_", " ")}</span><small className="mono">{event.event_id}</small></div>)}</div> : <p className="inspector-empty">No runtime events loaded.</p>}</div>
    </div> : tab === "work" ? <div className="inspector-pane" role="tabpanel">
      <Panel className="context-panel"><div className="panel-heading"><div><span className="eyebrow">SCOPED WORKSPACE</span><h3>{selectedSession?.session.workspace_id ?? "Local runtime workspace"}</h3></div><Icon name="lock" size={17} /></div><div className="evidence-facts"><div><span>Session</span><strong className="mono">{workspaceInspection?.session_id ?? selectedSession?.session.session_id ?? "new inspection"}</strong></div><div><span>Runtime scope</span><strong>Local device</strong></div><div><span>Reads</span><strong>Filesystem Gene</strong></div><div><span>Commands</span><strong>Exact permit path</strong></div></div></Panel>
      <div className="work-surface-tabs" role="tablist" aria-label="Workspace surfaces">{(["files", "changes", "terminal", "artifacts"] as WorkSurface[]).map((surface) => <button type="button" role="tab" aria-selected={workSurface === surface} className={workSurface === surface ? "is-active" : ""} key={surface} onClick={() => setWorkSurface(surface)}><Icon name={surface === "files" ? "book" : surface === "changes" ? "code" : surface === "terminal" ? "terminal" : "archive"} size={13} />{surface}</button>)}</div>
      <WorkSurfacePanel surface={workSurface} runtimeStatus={runtimeStatus} workspacePath={workspacePath} workspaceInspectionInFlight={workspaceInspectionInFlight} lastRun={lastRun} onPathChange={setWorkspacePath} onReadFile={readWorkspaceFile} onInspect={inspectWorkspace} />
      {workspaceError ? <p className="workspace-inspection-error" role="alert">{workspaceError}</p> : null}
      {workspaceResultVisible && workspaceInspection ? <Panel className="workspace-result-panel"><div className="panel-heading"><div><span className="eyebrow">GOVERNED RESULT</span><h3>{workspaceInspection.selected_gene ?? "Workspace inspection"}</h3></div><Chip tone={workspaceInspection.status === "completed" ? "green" : workspaceInspection.status === "approval_required" ? "amber" : "neutral"}>{workspaceInspection.status}</Chip></div><div className="workspace-result-meta"><span className="mono">{workspaceInspection.execution_id ?? workspaceInspection.session_id}</span><span>{workspaceInspection.receipt_count} receipt{workspaceInspection.receipt_count === 1 ? "" : "s"}</span></div>{workspaceApproval ? <div className="workspace-approval"><div><span className="eyebrow">EXACT APPROVAL</span><strong>{workspaceApproval.request_summary}</strong><small className="mono">{workspaceApproval.request_digest}</small></div><div className="workspace-approval-actions">{workspaceApproval.status === "pending" ? <><button className="button button-deny" type="button" disabled={workspaceInspectionInFlight} onClick={() => void resolveWorkspace(false)}>Deny</button><button className="button button-primary" type="button" disabled={workspaceInspectionInFlight} onClick={() => void resolveWorkspace(true)}>Allow once</button></> : workspaceApproval.status === "approved" ? <button className="button button-primary" type="button" disabled={workspaceInspectionInFlight} onClick={() => void resolveWorkspace(true)}>Resume approved inspection</button> : <Chip tone="neutral">{workspaceApproval.status}</Chip>}</div></div> : null}{workspaceOutput ? <WorkspaceEvidenceOutput task={workspaceTask} output={workspaceOutput} /> : <p className="inspector-empty">{workspaceInspection.status_detail ?? (workspaceInspectionInFlight ? "Waiting for runtime evidence…" : "No output returned.")}</p>}</Panel> : null}
      <Panel className="context-boundary"><span className="eyebrow">BOUNDARY</span><h3>Work surfaces expose evidence, not authority</h3><p>Files, changes, checks, and artifacts stay on Pandora’s existing Harness → Gene → ReferenceMonitor → receipt path. The desktop cannot issue permits, run arbitrary commands, or mutate repository state outside a registered Gene.</p></Panel>
    </div> : tab === "browser" ? <div className="inspector-pane" role="tabpanel">
      <Panel className="browser-control-panel"><div className="panel-heading"><div><span className="eyebrow">GOVERNED BROWSER</span><h3>Fetch inert source evidence</h3></div><Chip tone={runtimeStatus === "connected" ? "green" : "neutral"} icon="shield">{runtimeStatus === "connected" ? "Exact approval" : "Offline"}</Chip></div><form className="browser-fetch-form" onSubmit={(event) => void fetchBrowser(event)}><label><span>Exact URL</span><input aria-label="Browser URL" value={browserUrl} onChange={(event) => setBrowserUrl(event.target.value)} maxLength={2048} spellCheck={false} autoComplete="off" /></label><button className="button button-primary" type="submit" disabled={runtimeStatus !== "connected" || browserInspectionInFlight || !browserUrl.trim()}>{browserInspectionInFlight ? "Fetching…" : "Fetch source"}</button></form><div className="browser-rules"><span><Icon name="check" size={11} /> HTTPS, or loopback HTTP</span><span><Icon name="check" size={11} /> No redirects</span><span><Icon name="check" size={11} /> Text only · 128 KiB</span></div>{browserError ? <p className="workspace-inspection-error" role="alert">{browserError}</p> : null}</Panel>
      {browserInspection ? <Panel className="browser-result-panel"><div className="panel-heading"><div><span className="eyebrow">NETWORK RECEIPT PATH</span><h3>{browserInspection.selected_gene ?? "browser.fetch"}</h3></div><Chip tone={browserInspection.status === "completed" ? "green" : browserInspection.status === "approval_required" ? "amber" : "neutral"}>{browserInspection.status}</Chip></div><div className="workspace-result-meta"><span className="mono">{browserInspection.execution_id ?? browserInspection.session_id}</span><span>{browserInspection.receipt_count} receipt{browserInspection.receipt_count === 1 ? "" : "s"}</span></div>{browserApproval ? <div className="workspace-approval"><div><span className="eyebrow">EXACT NETWORK APPROVAL</span><strong>{browserApproval.request_summary}</strong><small className="mono">{browserApproval.request_digest}</small></div><div className="workspace-approval-actions">{browserApproval.status === "pending" ? <><button className="button button-deny" type="button" disabled={browserInspectionInFlight} onClick={() => void resolveBrowser(false)}>Deny</button><button className="button button-primary" type="button" disabled={browserInspectionInFlight} onClick={() => void resolveBrowser(true)}>Allow once</button></> : browserApproval.status === "approved" ? <button className="button button-primary" type="button" disabled={browserInspectionInFlight} onClick={() => void resolveBrowser(true)}>Resume approved fetch</button> : <Chip tone="neutral">{browserApproval.status}</Chip>}</div></div> : null}{browserEvidence ? <div className="browser-evidence"><div className="browser-evidence-meta"><div><span>Status</span><strong>{browserEvidence.status}</strong></div><div><span>Media</span><strong>{browserEvidence.content_type ?? "UTF-8 text"}</strong></div><div><span>Boundary</span><strong>{browserEvidence.truncated ? "Truncated" : "Complete"}{browserEvidence.lossy ? " · normalized" : ""}</strong></div></div><div className="browser-address mono">{browserEvidence.url}</div><pre aria-label="Browser evidence body">{browserEvidence.body}</pre></div> : browserInspection.output ? <pre className="workspace-output" aria-label="Browser inspection output">{browserInspection.output}</pre> : <p className="inspector-empty">{browserInspection.status_detail ?? (browserInspectionInFlight ? "Waiting for network evidence…" : "No source returned.")}</p>}</Panel> : null}
      <Panel className="context-boundary browser-boundary"><span className="eyebrow">UNTRUSTED EVIDENCE</span><h3>Remote content cannot speak with authority</h3><p>Pandora never executes returned HTML or scripts. The exact URL is payload-digest bound, DNS is pinned after boundary checks, every connection consumes one permit, and the result enters context only as untrusted evidence.</p></Panel>
    </div> : <div className="inspector-pane" role="tabpanel">
      <Panel className="task-panel"><div className="task-heading"><span className="task-icon"><Icon name="code" size={18} /></span><div><span className="eyebrow">PANDORA DESKTOP</span><h3>Governed command surface</h3></div></div><p className="task-copy">Submit work through the local service. The desktop shell does not issue permits or execute tools directly.</p><div className="task-meta"><span><Icon name="book" size={13} /> Existing runtime</span><span><Icon name="lock" size={13} /> Workspace scoped</span></div></Panel>
      <div className="inspector-section"><div className="section-heading"><span>Authority chain</span><span className="mono section-count">{steps.filter((step) => step.status !== "idle").length}/8</span></div><div className="authority-timeline">{steps.map((step) => <button className={`authority-row status-${step.status} ${selectedStep === step.id ? "is-selected" : ""}`} key={step.id} onClick={() => onSelectStep(step.id)}><span className="timeline-line" /><span className="timeline-node">{step.status === "complete" ? <Icon name="check" size={12} /> : <Icon name={step.icon} size={13} />}</span><span className="authority-copy"><strong>{step.label}</strong><small>{step.detail}</small></span><Icon name="chevron" size={14} /></button>)}</div></div>
      <Panel className={`approval-panel ${approval && approval.status !== "pending" ? "is-preview-complete" : ""}`}><div className="approval-top"><span className="approval-icon"><Icon name={approval && approval.status !== "pending" ? "check" : "shield"} size={17} /></span><div><span className="eyebrow">{approval ? "LIVE APPROVAL" : approvalDetail ? "LIVE RUNTIME" : "REFERENCE MONITOR"}</span><h3>{approval ? approval.status === "pending" ? "Exact approval required" : `Approval ${approval.status}` : approvalDetail ? "Approval metadata unavailable" : "No pending approval"}</h3></div></div>{approval ? <><p className="approval-note">{approval.status === "pending" ? "Review the exact digest before allowing this operation once." : `This approval is ${approval.status}; it cannot authorize another execution.`}</p><div className="operation-box"><div><span className="eyebrow">OPERATION</span><strong>{approval.request_summary}</strong></div><div><span className="eyebrow">GENE</span><span className="mono">{approval.gene_id}</span></div><div><span className="eyebrow">REQUEST DIGEST</span><span className="digest"><span className="mono">{approval.request_digest}</span></span></div><div><span className="eyebrow">SCOPE</span><span className="mono">{approval.session_id}</span></div></div>{approvalError ? <p className="approval-error" role="alert">{approvalError}</p> : null}{approval.status === "pending" ? <div className="approval-actions"><button className="button button-deny" type="button" disabled={approvalInFlight} onClick={() => void decide(false)}>Deny</button><button className="button button-primary" type="button" disabled={approvalInFlight} onClick={() => void decide(true)}>{approvalInFlight ? "Resolving…" : "Allow once"} <Icon name="arrow" size={14} /></button></div> : null}</> : approvalDetail ? <><p className="approval-note">The runtime paused, but this service did not return an exact approval record. Upgrade the local service before resuming.</p><div className="operation-box"><div><span className="eyebrow">REASON</span><strong>{approvalDetail}</strong></div><div><span className="eyebrow">SCOPE</span><span className="mono">Exact session and request</span></div></div></> : <><p className="approval-note">An exact request digest appears here only when the runtime pauses a real operation. This desktop cannot fabricate an approval or issue a permit.</p><div className="operation-box"><div><span className="eyebrow">STATE</span><strong>Waiting for governed work</strong></div><div><span className="eyebrow">AUTHORITY</span><span className="mono">ReferenceMonitor only</span></div></div></>}</Panel>
      {approval?.status === "approved" ? <button className="button button-primary approval-resume" type="button" disabled={approvalInFlight} onClick={() => void decide(true)}>{approvalInFlight ? "Resuming…" : "Resume approved run"} <Icon name="arrow" size={14} /></button> : null}
      <div className="selected-detail"><div className="section-heading"><span>Selected evidence</span><Icon name="chevron" size={14} /></div><div className="detail-row"><span className="detail-label">Stage</span><span>{selected.label}</span></div><div className="detail-row"><span className="detail-label">Status</span><Chip tone={selected.status === "waiting" ? "amber" : selected.status === "idle" ? "neutral" : "green"} icon={selected.status === "waiting" ? "clock" : selected.status === "idle" ? "lock" : "check"}>{selected.status}</Chip></div></div>
    </div>}
  </aside>;
}

function CouncilView({ runtimeStatus, lastRun, events, selectedSession, harnesses, onOpenCommand, onOpenAudit }: { runtimeStatus: RuntimeStatus; lastRun: RuntimeRun | null; events: RuntimeEvent[]; selectedSession: RuntimeSessionDetail | null; harnesses: RuntimeHarness[]; onOpenCommand: () => void; onOpenAudit: () => void }) {
  const connected = runtimeStatus === "connected";
  const eventTypes = new Set(events.map((event) => event.event_type));
  const selectedHarness = harnesses.find((harness) => harness.id === lastRun?.selected_harness) ?? null;
  const policyState = eventTypes.has("policy_denied")
    ? "denied"
    : eventTypes.has("approval_required") || lastRun?.status === "approval_required"
      ? "approval required"
      : eventTypes.has("policy_approved")
        ? "approved"
        : lastRun
          ? lastRun.status
          : "not evaluated";
  const monitorTone: "neutral" | "green" | "amber" = policyState === "approved" || lastRun?.status === "completed"
    ? "green"
    : policyState === "not evaluated"
      ? "neutral"
      : "amber";
  const trace = events.slice(-12).reverse();
  const recordAvailable = Boolean(lastRun || selectedSession);

  return <div className="full-view council-view">
    <PageHeader eyebrow="Governance" title="Council" description="Inspect the evidence Pandora exposes for planning, routing, and exact authorization. Deliberation may propose; it never grants authority." actions={<div className="council-actions"><Chip tone={recordAvailable ? "gold" : connected ? "neutral" : "amber"} icon="council">{recordAvailable ? "Evidence loaded" : connected ? "Awaiting run" : "Runtime offline"}</Chip><button className="button button-secondary" type="button" onClick={onOpenCommand}>Open Command</button></div>} />
    <div className="council-boundary"><Icon name="shield" size={16} /><div><strong>One governed path</strong><span>Parliament frames intent, the Shadow Council selects among admitted Harnesses, and ReferenceMonitor alone can authorize an exact effect. This page cannot vote, route, approve, or execute.</span></div></div>
    {recordAvailable ? <>
      <div className="council-chambers">
        <Panel className="council-chamber parliament-chamber"><div className="council-chamber-head"><span className="council-seal"><Icon name="council" size={20} /></span><div><span className="eyebrow">PARLIAMENT</span><h3>Intent and policy posture</h3></div><Chip tone={lastRun ? "blue" : "neutral"}>{lastRun ? "recorded" : "history only"}</Chip></div><p>{lastRun ? "A governed run record exists for this exact local session. Internal deliberation text is not exposed, so the desktop does not invent it." : "A local session is selected, but no current run summary is loaded. Select or submit a run to bind Council evidence."}</p><div className="council-facts"><div><span>Session</span><strong className="mono">{lastRun?.session_id ?? selectedSession?.session.session_id}</strong></div><div><span>Execution</span><strong className="mono">{lastRun?.execution_id ?? "not loaded"}</strong></div><div><span>Outcome</span><strong>{lastRun?.status ?? "session history"}</strong></div></div></Panel>
        <Panel className="council-chamber shadow-chamber"><div className="council-chamber-head"><span className="council-seal"><Icon name="users" size={20} /></span><div><span className="eyebrow">SHADOW COUNCIL</span><h3>Admitted route selection</h3></div><Chip tone={lastRun?.selected_harness ? "gold" : "neutral"}>{lastRun?.selected_harness ? "bound" : "unavailable"}</Chip></div><p>{lastRun?.selected_harness ? "The selected Harness and Gene are reported by the runtime. Catalog membership constrains the route; it does not confer execution authority." : "No runtime-reported route is available for the selected record."}</p><div className="council-facts"><div><span>Harness</span><strong>{lastRun?.selected_harness ?? "not selected"}</strong></div><div><span>Harness version</span><strong>{selectedHarness ? `v${selectedHarness.version} · ${selectedHarness.kind}` : "not reported"}</strong></div><div><span>Gene</span><strong className="mono">{lastRun?.selected_gene ?? "not selected"}</strong></div></div></Panel>
        <Panel className="council-chamber monitor-chamber"><div className="council-chamber-head"><span className="council-seal"><Icon name="shield" size={20} /></span><div><span className="eyebrow">REFERENCE MONITOR</span><h3>Exact effect authority</h3></div><Chip tone={monitorTone}>{policyState}</Chip></div><p>{lastRun?.approval ? "The runtime paused this exact request. The digest, Gene, session, and expiry below are the complete desktop approval scope." : "No pending approval record is attached. Absence of an approval in this view never implies permission."}</p><div className="council-facts"><div><span>Decision</span><strong>{policyState}</strong></div><div><span>Approval</span><strong className="mono">{lastRun?.approval?.approval_id ?? "none attached"}</strong></div><div><span>Request digest</span><strong className="mono">{lastRun?.approval?.request_digest ?? "not issued"}</strong></div></div></Panel>
      </div>
      <Panel className="council-ledger"><div className="panel-heading"><div><span className="eyebrow">DECISION LEDGER</span><h3>Redacted evidence trace</h3></div><button className="button button-secondary" type="button" onClick={onOpenAudit}>Open full audit <Icon name="arrow" size={13} /></button></div><div className="council-ledger-grid"><div className="council-ledger-summary"><div><span>Workspace</span><strong>{selectedSession?.session.workspace_id ?? "local workspace"}</strong></div><div><span>Events</span><strong>{events.length}</strong></div><div><span>Receipts</span><strong>{lastRun?.receipt_count ?? 0}</strong></div><div><span>Tool calls</span><strong>{lastRun?.tool_calls ?? 0}</strong></div><p><Icon name="lock" size={13} /> Event payloads remain inside the local runtime. Only recorded event types and identifiers are shown.</p></div><div className="council-event-trace">{trace.length ? trace.map((event) => <div className="council-event-row" key={event.event_id}><span className="event-dot" /><div><strong>{event.event_type.replaceAll("_", " ")}</strong><small className="mono">{event.event_id}</small></div><span>recorded</span></div>) : <div className="council-empty-trace"><Icon name="archive" size={22} /><p>No redacted runtime events are loaded for this record.</p></div>}</div></div></Panel>
    </> : <div className="workflow-empty council-empty"><div className="empty-emblem"><Icon name="council" size={27} /></div><h2>No Council record selected</h2><p>{connected ? "Submit a governed request or open a recorded session. Pandora will show only runtime-backed planning, routing, policy, and receipt evidence." : "Start the local service to inspect Council evidence. Everything remains on this device."}</p><button className="button button-primary" type="button" onClick={onOpenCommand}>Open Command Center <Icon name="arrow" size={14} /></button></div>}
  </div>;
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
  const serviceMessage = runtimeStatus === "connected" ? selectedSession ? "Only redacted records for the selected session are shown." : "Select a session to inspect scoped memory." : "Connect the local service to inspect scoped memory.";
  return <div className="full-view"><PageHeader eyebrow="Scoped knowledge" title="Memory" description="Inspect bounded, redacted evidence with provenance labels." actions={<Chip tone={records.length ? "green" : "neutral"} icon="archive">{records.length ? `${records.length} records` : "No records"}</Chip>} /><div className="memory-grid"><Panel className="memory-graph-panel"><div className="panel-toolbar"><div><span className="eyebrow">SESSION MEMORY · REDACTED</span><h3>{selectedSession?.session.session_id ?? "No session selected"}</h3></div><div className="toolbar-pills"><Chip tone="gold">L2</Chip><Chip tone="blue">L1</Chip><Chip tone="green">L0 ephemeral</Chip></div></div>{records.length ? <div className="memory-record-list">{records.map((record) => <article className="memory-record" key={`${record.tier}-${record.memory_id}`}><div className="memory-record-top"><Chip tone={record.tier === "l2" ? "gold" : "blue"}>{record.tier}</Chip><span className="eyebrow">{record.kind}</span><span className="memory-record-time">{new Date(record.created_at_unix_seconds * 1000).toLocaleString()}</span></div><p>{record.summary}</p><div className="memory-record-meta"><span>{record.classification}</span><span>{record.origin}</span><span>{record.evidence_count} evidence</span><span className="mono">{record.provenance}</span></div></article>)}</div> : <div className="runs-empty memory-empty"><Icon name={runtimeStatus === "connected" ? "archive" : "lock"} size={27} /><h3>No memory evidence recorded</h3><p>{runtimeStatus !== "connected" ? "Connect the local runtime to inspect scoped memory evidence." : selectedSession ? "The selected session returned no durable memory records." : "Select a recorded session to inspect its bounded memory evidence."}</p><small>Ephemeral runtime state is not projected here as durable memory, and Pandora does not invent graph nodes when no records exist.</small></div>}</Panel><div className="memory-side"><Panel><div className="panel-heading"><h3>Memory layers</h3><Chip tone={records.length ? "green" : "neutral"}>{records.length ? "Live" : "Unavailable"}</Chip></div><Layer label="L0 · Ephemeral trace" value="RAM" detail="expires automatically" tone="green" /><Layer label="L1 · Distilled evidence" value={String(records.filter((record) => record.tier === "l1").length)} detail="session scoped" tone="blue" /><Layer label="L2 · Evolutionary" value={String(records.filter((record) => record.tier === "l2").length)} detail="promotion gated" tone="gold" /></Panel><Panel><div className="panel-heading"><h3>Availability</h3><Chip tone={records.length ? "green" : "neutral"} icon="lock">{records.length ? "Scoped" : "Unavailable"}</Chip></div><p className="connection-note">{serviceMessage}</p></Panel></div></div></div>;
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
  return <div className="full-view"><PageHeader eyebrow="Client recipes" title="Workflows" description="Save reusable task recipes locally; every run still uses Pandora’s governed local runtime." actions={<Chip tone={workflows.length ? "green" : "neutral"} icon="stack">{workflows.length} saved</Chip>} /><div className="workflow-grid"><Panel className="workflow-editor"><div className="panel-heading"><div><span className="eyebrow">NEW RECIPE</span><h3>Define a governed run</h3></div><Icon name="plus" size={17} /></div><form className="workflow-form" onSubmit={submit}><label><span>Name</span><input value={name} onChange={(event) => setName(event.target.value)} placeholder="Release review" maxLength={80} /></label><label><span>Task</span><textarea value={task} onChange={(event) => setTask(event.target.value)} placeholder="Describe the work Pandora should perform…" maxLength={4000} rows={5} /></label><label><span>Harness</span><select value={profile} onChange={(event) => setProfile(event.target.value)}>{options.map((option) => <option value={option.id} key={option.id}>{option.label}</option>)}</select></label><button className="button button-primary" type="submit"><Icon name="plus" size={14} /> Save recipe</button></form><p className="connection-note">Recipes are stored on this device. Do not save credentials or secrets in task text.</p></Panel><Panel className="workflow-list-panel"><div className="panel-heading"><div><span className="eyebrow">SAVED RECIPES</span><h3>{workflows.length ? "Ready to run" : "No recipes yet"}</h3></div><Chip tone={connected ? "green" : "neutral"}>{connected ? "Runtime ready" : "Connect to run"}</Chip></div>{workflows.length ? <div className="workflow-list">{workflows.map((workflow) => <article className="workflow-card" key={workflow.id}><div><strong>{workflow.name}</strong><small>{workflow.profile === "auto" ? "Auto route" : workflow.profile} · local recipe</small><p>{workflow.task}</p></div><div className="workflow-card-actions"><button className="button button-secondary" type="button" disabled={!connected} onClick={() => void onRun(workflow)}>{connected ? "Run" : "Offline"} <Icon name="arrow" size={13} /></button><button className="icon-button" type="button" aria-label={`Delete ${workflow.name}`} onClick={() => onRemove(workflow.id)}><Icon name="dots" size={16} /></button></div></article>)}</div> : <div className="workflow-empty"><div className="empty-emblem"><Icon name="stack" size={27} /></div><h2>Build your first recipe</h2><p>Recipes remain local to this desktop. Execution always returns to the Command Center and the governed runtime.</p><button className="button button-secondary" type="button" onClick={onOpenCommand}>Open Command Center <Icon name="arrow" size={14} /></button></div>}</Panel></div></div>;
}

function AuditView({ events, selectedSession, runtimeStatus }: { events: RuntimeEvent[]; selectedSession: RuntimeSessionDetail | null; runtimeStatus: RuntimeStatus }) {
  const live = runtimeStatus === "connected" && selectedSession !== null;
  return <div className="full-view"><PageHeader eyebrow="Evidence" title="Audit" description="Inspect redacted runtime activity without exposing event payloads or credentials." actions={<Chip tone={live ? "green" : "neutral"} icon="archive">{live ? `${events.length} events loaded` : "Select a live session"}</Chip>} /><div className="audit-grid"><Panel className="audit-summary"><div className="panel-heading"><div><span className="eyebrow">SESSION SCOPE</span><h3>{selectedSession?.session.session_id ?? "No session selected"}</h3></div><Icon name="lock" size={18} /></div><div className="audit-summary-rows"><div><span>Workspace</span><strong>{selectedSession?.session.workspace_id ?? "—"}</strong></div><div><span>Runtime scope</span><strong>Local device</strong></div><div><span>Recorded events</span><strong>{selectedSession?.event_count ?? 0}</strong></div></div><p>Event payloads stay in the local runtime. This surface shows identifiers and event types only.</p></Panel><Panel className="audit-events"><div className="panel-heading"><div><span className="eyebrow">ACTIVITY</span><h3>Runtime event timeline</h3></div><Chip tone="blue" icon="activity">Redacted</Chip></div>{events.length ? <div className="audit-event-list">{events.map((event) => <div className="audit-event-row" key={event.event_id}><span className="event-dot" /><div><strong>{event.event_type.replaceAll("_", " ")}</strong><small className="mono">{event.event_id}</small></div><span className="audit-event-state">recorded</span></div>)}</div> : <div className="connection-empty"><Icon name="archive" size={21} /><p>{live ? "No events recorded for this session." : "Connect and select a session to inspect activity."}</p></div>}</Panel></div></div>;
}

function CapabilitiesView({ harnesses, tools, runtimeStatus, native }: { harnesses: RuntimeHarness[]; tools: RuntimeTool[]; runtimeStatus: RuntimeStatus; native: boolean }) {
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
        <div className="harness-tabs" role="tablist" aria-label="Harness inspector">{(["genes", "extensions", "packages", "authority", "receipts"] as HarnessTab[]).map((item) => <button type="button" role="tab" aria-selected={tab === item} className={tab === item ? "is-active" : ""} onClick={() => setTab(item)} key={item}>{item === "extensions" ? "Plugins & tools" : item}</button>)}</div>
        <div className={`harness-tab-panel ${tab === "packages" ? "package-tab-panel" : ""}`} role="tabpanel">{tab === "genes" ? <><div className="inspection-heading"><div><span className="eyebrow">GENE CATALOG</span><h3>{selectedHarness.gene_count} admitted Genes</h3></div><Chip tone="blue">Exact versions</Chip></div>{selectedHarness.gene_ids?.length ? <div className="gene-table">{selectedHarness.gene_ids.map((gene, index) => <div className="gene-row" key={gene}><span className="gene-index mono">{String(index + 1).padStart(2, "0")}</span><span><strong>{gene}</strong><small>Capability identity reported by runtime</small></span><Chip tone="green" icon="check">admitted</Chip></div>)}</div> : <p className="inspector-empty">Gene identities are unavailable from this runtime version.</p>}</> : tab === "extensions" ? <><div className="inspection-heading"><div><span className="eyebrow">ADMITTED EXTENSIONS</span><h3>{tools.length} plugin and tool surfaces</h3></div><Chip tone="gold" icon="shield">No authority implied</Chip></div>{tools.length ? <div className="gene-table">{tools.map((tool) => <div className="gene-row" key={tool.id}><span className="harness-browser-icon"><Icon name="terminal" size={14} /></span><span><strong>{tool.name}</strong><small className="mono">{tool.id} · v{tool.version}</small></span><span className="extension-operation">{tool.capability} / {tool.operation}</span></div>)}</div> : <p className="inspector-empty">No extension metadata reported.</p>}</> : tab === "packages" ? <PackageManager native={native} /> : tab === "authority" ? <div className="authority-map"><div><span>May select</span><strong>{selectedHarness.gene_count} admitted Genes</strong></div><div><span>May propose</span><strong>Bound tool requests</strong></div><div><span>May approve</span><strong className="authority-denied">Never</strong></div><div><span>May execute</span><strong className="authority-denied">Never directly</strong></div><p><Icon name="shield" size={14} /> Parliament plans, the Shadow Council routes, and ReferenceMonitor alone can authorize an exact effect.</p></div> : <div className="receipt-posture"><div className="receipt-seal"><Icon name="archive" size={24} /></div><h3>Evidence follows execution</h3><p>This catalog exposes capability identity and admission state. Receipts are run-scoped and appear in the Command inspector after an exact permit is consumed.</p><div className="receipt-rules"><span><Icon name="check" size={12} /> Request digest</span><span><Icon name="check" size={12} /> Bound Gene version</span><span><Icon name="check" size={12} /> Workspace scope</span><span><Icon name="check" size={12} /> Effect outcome</span></div></div>}</div>
      </> : <div className="harness-empty"><Icon name="box" size={24} /><h3>Select a Harness</h3><p>The inspector never fabricates catalog entries.</p></div>}</Panel>
    </div>
  </div>;
}

function PackageManager({ native }: { native: boolean }) {
  const [packages, setPackages] = useState<RuntimePackage[]>([]);
  const [registryProfiles, setRegistryProfiles] = useState<RegistryProfile[]>([]);
  const [registryProfile, setRegistryProfile] = useState("");
  const [selectedIdentity, setSelectedIdentity] = useState("");
  const [sourceTab, setSourceTab] = useState<"registry" | "github" | "local">("registry");
  const [packageId, setPackageId] = useState("");
  const [version, setVersion] = useState("");
  const [registryUrl, setRegistryUrl] = useState("");
  const [registryToken, setRegistryToken] = useState("");
  const [githubRepository, setGithubRepository] = useState("");
  const [githubCommit, setGithubCommit] = useState("");
  const [githubManifestPath, setGithubManifestPath] = useState("pandora-package.json");
  const [githubArtifactPath, setGithubArtifactPath] = useState("dist/package.artifact");
  const [githubToken, setGithubToken] = useState("");
  const [manifestPath, setManifestPath] = useState("");
  const [artifactPath, setArtifactPath] = useState("");
  const [removeTarget, setRemoveTarget] = useState<RuntimePackage | null>(null);
  const [removeConfirmation, setRemoveConfirmation] = useState("");
  const [lifecycleTarget, setLifecycleTarget] = useState<{ operation: "enable" | "disable" | "rollback"; package: RuntimePackage } | null>(null);
  const [lifecycleConfirmation, setLifecycleConfirmation] = useState("");
  const [lifecyclePreview, setLifecyclePreview] = useState<NativePackageResult["data"] | null>(null);
  const [busy, setBusy] = useState("");
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");
  const selectedPackage = packages.find((item) => `${item.id}@${item.version}` === selectedIdentity) ?? packages[0] ?? null;
  const selectedRegistryProfile = registryProfiles.find((profile) => profile.name === registryProfile) ?? null;

  useEffect(() => {
    setRemoveTarget(null);
    setRemoveConfirmation("");
    setLifecycleTarget(null);
    setLifecycleConfirmation("");
    setLifecyclePreview(null);
  }, [selectedIdentity]);

  const refreshPackages = async () => {
    if (!native) {
      setPackages([]);
      return;
    }
    const result = await listLocalPackages();
    const records = result.data.packages ?? [];
    setPackages(records);
    setSelectedIdentity((current) => records.some((item) => `${item.id}@${item.version}` === current) ? current : records[0] ? `${records[0].id}@${records[0].version}` : "");
  };

  useEffect(() => {
    let cancelled = false;
    if (!native) {
      setPackages([]);
      return;
    }
    setBusy("refresh");
    setError("");
    listLocalPackages()
      .then((result) => {
        if (!cancelled) {
          const records = result.data.packages ?? [];
          setPackages(records);
          setSelectedIdentity(records[0] ? `${records[0].id}@${records[0].version}` : "");
        }
      })
      .catch((reason: unknown) => {
        if (!cancelled) {
          setError(reason instanceof Error ? reason.message : "Could not load local packages");
        }
      })
      .finally(() => {
        if (!cancelled) setBusy("");
      });
    return () => {
      cancelled = true;
    };
  }, [native]);

  useEffect(() => {
    let cancelled = false;
    if (!native) {
      setRegistryProfiles([]);
      setRegistryProfile("");
      return;
    }
    listRegistryProfiles()
      .then((result) => {
        if (cancelled) return;
        const profiles = result.data.registries ?? [];
        setRegistryProfiles(profiles);
        setRegistryProfile((current) => profiles.some((profile) => profile.name === current)
          ? current
          : profiles.find((profile) => profile.active)?.name ?? "");
      })
      .catch(() => {
        if (!cancelled) {
          setRegistryProfiles([]);
          setRegistryProfile("");
        }
      });
    return () => {
      cancelled = true;
    };
  }, [native]);

  const completeOperation = async (operation: () => Promise<{ message: string; restartRequired: boolean }>) => {
    setError("");
    setMessage("");
    try {
      const result = await operation();
      setMessage(`${result.message}${result.restartRequired ? " Restart the local service to load the new catalog." : ""}`);
      await refreshPackages();
    } catch (reason: unknown) {
      setError(reason instanceof Error ? reason.message : "Package operation failed");
    } finally {
      setBusy("");
    }
  };

  const submitRegistry = async (event: FormEvent) => {
    event.preventDefault();
    if (!packageId.trim() || (!registryProfile && !registryUrl.trim()) || busy) return;
    setBusy("install");
    try {
      await completeOperation(() => installRegistryPackage({
        packageId: packageId.trim(),
        version: version.trim(),
        registryProfile,
        registryUrl: registryProfile ? "" : registryUrl.trim(),
        token: registryToken,
      }));
    } finally {
      setRegistryToken("");
    }
  };

  const submitLocal = async (event: FormEvent) => {
    event.preventDefault();
    if (!manifestPath.trim() || !artifactPath.trim() || busy) return;
    setBusy("admit");
    await completeOperation(() => admitLocalPackage({
      manifestPath: manifestPath.trim(),
      artifactPath: artifactPath.trim(),
    }));
  };

  const submitGitHub = async (event: FormEvent) => {
    event.preventDefault();
    if (!githubRepository.trim() || !githubCommit.trim() || !githubManifestPath.trim() || !githubArtifactPath.trim() || busy) return;
    setBusy("github");
    try {
      await completeOperation(() => installGitHubPackage({
        repositoryUrl: githubRepository.trim(),
        commit: githubCommit.trim(),
        manifestPath: githubManifestPath.trim(),
        artifactPath: githubArtifactPath.trim(),
        token: githubToken,
      }));
    } finally {
      setGithubToken("");
    }
  };

  const writeLock = async () => {
    if (busy) return;
    setBusy("lock");
    await completeOperation(lockLocalPackages);
  };

  const previewLifecycle = async (target: RuntimePackage, operation: "enable" | "disable" | "rollback") => {
    if (busy) return;
    setBusy(`preview-${operation}`);
    setError("");
    setMessage("");
    setRemoveTarget(null);
    try {
      const result = operation === "enable"
        ? await previewPackageEnable(target.id, target.version)
        : operation === "disable"
          ? await previewPackageDisable(target.id, target.version)
          : await previewPackageRollback(target.id);
      setLifecycleTarget({ operation, package: target });
      setLifecycleConfirmation("");
      setLifecyclePreview(result.data);
      setMessage(result.message);
    } catch (reason: unknown) {
      setError(reason instanceof Error ? reason.message : `Package ${operation} preview failed`);
    } finally {
      setBusy("");
    }
  };

  const confirmLifecycle = async (event: FormEvent) => {
    event.preventDefault();
    if (!lifecycleTarget || busy) return;
    const { operation, package: target } = lifecycleTarget;
    setBusy(operation);
    await completeOperation(async () => {
      const result = operation === "enable"
        ? await enableLocalPackage(target.id, target.version, lifecycleConfirmation)
        : operation === "disable"
          ? await disableLocalPackage(target.id, target.version, lifecycleConfirmation)
          : await rollbackLocalPackage(target.id, lifecycleConfirmation);
      setLifecycleTarget(null);
      setLifecycleConfirmation("");
      setLifecyclePreview(null);
      return result;
    });
  };

  const previewRemoval = async (target: RuntimePackage) => {
    if (busy) return;
    setBusy("preview-remove");
    setError("");
    setMessage("");
    setLifecycleTarget(null);
    setLifecyclePreview(null);
    try {
      const result = await previewPackageRemoval(target.id, target.version);
      setRemoveTarget(target);
      setRemoveConfirmation("");
      setMessage(result.message);
    } catch (reason: unknown) {
      setError(reason instanceof Error ? reason.message : "Package removal preview failed");
    } finally {
      setBusy("");
    }
  };

  const confirmRemoval = async (event: FormEvent) => {
    event.preventDefault();
    if (!removeTarget || busy) return;
    setBusy("remove");
    await completeOperation(async () => {
      const result = await removeLocalPackage(removeTarget.id, removeTarget.version, removeConfirmation);
      setRemoveTarget(null);
      setRemoveConfirmation("");
      return result;
    });
  };

  if (!native) {
    return <div className="package-manager-unavailable"><Icon name="lock" size={25} /><h3>Native desktop required</h3><p>Package mutation is unavailable in the loopback browser shell. The local service and CLI remain the source of truth.</p></div>;
  }

  return <div className="package-manager">
    <div className="package-manager-heading"><div><span className="eyebrow">MODULAR ECOSYSTEM</span><h3>Signed package manager</h3><p>Install and inspect exact Gene, Domain Harness, and Meta Harness records without granting them authority.</p></div><div><button className="button button-secondary" type="button" disabled={Boolean(busy)} onClick={() => void writeLock()}>{busy === "lock" ? "Locking…" : "Write lockfile"}</button><button className="icon-button" type="button" aria-label="Refresh local packages" disabled={Boolean(busy)} onClick={() => { setBusy("refresh"); void completeOperation(async () => ({ message: "Local package catalog refreshed.", restartRequired: false })); }}><Icon name="activity" size={15} /></button></div></div>
    <div className="package-boundary"><Icon name="shield" size={14} /><span>Admission verifies identity, artifact hash, dependencies, compatibility, and available signature evidence. Package records cannot replace Parliament, Shadow Council, ReferenceMonitor, permits, or the constitutional service.</span></div>
    {message ? <p className="configuration-result is-success" role="status"><Icon name="check" size={13} /> {message}</p> : null}
    {error ? <p className="configuration-result is-error" role="alert">{error}</p> : null}
    <div className="package-manager-grid">
      <div className="package-catalog"><div className="package-section-heading"><div><span className="eyebrow">LOCAL CATALOG</span><h4>{busy === "refresh" ? "Refreshing…" : `${packages.length} exact package${packages.length === 1 ? "" : "s"}`}</h4></div><Chip tone={packages.length ? "green" : "neutral"}>{packages.length ? "verified records" : "empty"}</Chip></div><div className="package-list">{packages.length ? packages.map((item) => <button type="button" className={`package-row ${selectedPackage?.id === item.id && selectedPackage.version === item.version ? "is-selected" : ""}`} onClick={() => { setSelectedIdentity(`${item.id}@${item.version}`); setRemoveTarget(null); }} key={`${item.id}@${item.version}`}><span className="package-kind-icon"><Icon name={item.kind === "gene" ? "code" : "box"} size={15} /></span><span><strong>{item.id}</strong><small>{item.kind.replaceAll("_", " ")} · v{item.version}</small></span><Chip tone={item.trust.level === "verified" ? "green" : "gold"}>{item.trust.level}</Chip></button>) : <div className="package-empty"><Icon name="box" size={22} /><p>No local package records. Built-in Harnesses remain core catalog entries.</p></div>}</div></div>
      <div className="package-console">
        {selectedPackage ? <div className="package-inspection">
          <div className="package-section-heading">
            <div><span className="eyebrow">SELECTED PACKAGE</span><h4>{selectedPackage.id}@{selectedPackage.version}</h4></div>
            <div className="package-heading-chips"><Chip tone={selectedPackage.activation.state === "enabled" ? "green" : "neutral"}>{selectedPackage.activation.state}</Chip><Chip tone={selectedPackage.state === "admitted" ? "green" : "blue"}>{selectedPackage.state}</Chip></div>
          </div>
          <div className="package-facts">
            <div><span>Publisher</span><strong>{selectedPackage.publisher}</strong></div>
            <div><span>Artifact</span><strong className="mono">{selectedPackage.content_hash}</strong></div>
            <div><span>Compatibility</span><strong>{selectedPackage.compatibility}</strong></div>
            <div><span>Dependencies</span><strong>{selectedPackage.dependencies.length}</strong></div>
            <div><span>Active version</span><strong>{selectedPackage.activation.active_version ?? "none"}</strong></div>
            <div><span>Rollback target</span><strong>{selectedPackage.activation.previous_version ?? "none"}</strong></div>
            <div><span>Signature evidence</span><strong>{selectedPackage.trust.has_signature && selectedPackage.trust.has_public_key ? "present" : "not present"}</strong></div>
            <div><span>Runtime authority</span><strong className="authority-denied">{selectedPackage.runtime_authority || selectedPackage.activation.runtime_authority ? "unexpected" : "none"}</strong></div>
          </div>
          <div className="package-lifecycle-actions">
            <button className="button button-primary" type="button" disabled={Boolean(busy)} onClick={() => void previewLifecycle(selectedPackage, selectedPackage.activation.state === "enabled" ? "disable" : "enable")}>{busy === "preview-enable" || busy === "preview-disable" ? "Checking…" : selectedPackage.activation.state === "enabled" ? "Preview disable" : selectedPackage.activation.active_version ? "Preview update" : "Preview enable"}</button>
            {selectedPackage.activation.previous_version ? <button className="button button-secondary" type="button" disabled={Boolean(busy)} onClick={() => void previewLifecycle(selectedPackage, "rollback")}>{busy === "preview-rollback" ? "Checking…" : "Preview rollback"}</button> : null}
            <button className="button button-deny" type="button" disabled={Boolean(busy) || selectedPackage.activation.state === "enabled"} title={selectedPackage.activation.state === "enabled" ? "Disable this exact version before removal" : undefined} onClick={() => void previewRemoval(selectedPackage)}>Preview removal</button>
          </div>
          {lifecycleTarget && lifecycleTarget.package.id === selectedPackage.id && lifecycleTarget.package.version === selectedPackage.version ? <form className="package-lifecycle-confirm" onSubmit={confirmLifecycle}>
            <div className="package-lifecycle-preview">
              <div><span className="eyebrow">{lifecycleTarget.operation.toUpperCase()} PREVIEW</span><strong>{lifecyclePreview?.ready === false ? "Blocked by current bindings" : "Exact transition is ready"}</strong></div>
              <Chip tone={lifecyclePreview?.ready === false ? "gold" : "green"}>{lifecyclePreview?.ready === false ? "blocked" : "ready"}</Chip>
            </div>
            {lifecyclePreview?.dependencies?.length ? <div className="package-dependency-preview">{lifecyclePreview.dependencies.map((dependency) => <div key={`${dependency.id}@${dependency.version ?? "active"}`}><span>{dependency.id}<small>{dependency.version ?? "active version"} · {dependency.source.replaceAll("_", " ")}</small></span><Chip tone={dependency.enabled ? "green" : dependency.optional ? "neutral" : "gold"}>{dependency.enabled ? "ready" : dependency.optional ? "optional" : "missing"}</Chip></div>)}</div> : null}
            {lifecyclePreview?.enabled_dependents?.length ? <p className="package-lifecycle-blockers">Enabled dependents: <span className="mono">{lifecyclePreview.enabled_dependents.join(", ")}</span></p> : null}
            <p>Type <span className="mono">{lifecycleTarget.operation === "rollback" ? selectedPackage.id : `${selectedPackage.id}@${selectedPackage.version}`}</span> to confirm this exact {lifecycleTarget.operation}. The binding changes; runtime authority does not.</p>
            <input aria-label={`Confirm ${lifecycleTarget.operation} ${selectedPackage.id}@${selectedPackage.version}`} value={lifecycleConfirmation} onChange={(event) => setLifecycleConfirmation(event.target.value)} autoComplete="off" spellCheck={false} />
            <div><button className="button button-secondary" type="button" onClick={() => { setLifecycleTarget(null); setLifecyclePreview(null); }}>Close</button><button className="button button-primary" type="submit" disabled={Boolean(busy) || lifecyclePreview?.ready === false || lifecycleConfirmation !== (lifecycleTarget.operation === "rollback" ? selectedPackage.id : `${selectedPackage.id}@${selectedPackage.version}`)}>{busy === lifecycleTarget.operation ? "Applying…" : `Confirm ${lifecycleTarget.operation}`}</button></div>
          </form> : null}
          {removeTarget ? <form className="package-remove-confirm" onSubmit={confirmRemoval}><p>Dependency and lifecycle-binding checks passed. Type <span className="mono">{removeTarget.id}@{removeTarget.version}</span> to remove this exact record.</p><input aria-label={`Confirm removal ${removeTarget.id}@${removeTarget.version}`} value={removeConfirmation} onChange={(event) => setRemoveConfirmation(event.target.value)} autoComplete="off" spellCheck={false} /><div><button className="button button-secondary" type="button" onClick={() => setRemoveTarget(null)}>Close</button><button className="button button-deny" type="submit" disabled={busy === "remove" || removeConfirmation !== `${removeTarget.id}@${removeTarget.version}`}>{busy === "remove" ? "Removing…" : "Remove package"}</button></div></form> : null}
        </div> : null}
        <div className="package-source-tabs" role="tablist" aria-label="Package source"><button type="button" role="tab" aria-selected={sourceTab === "registry"} className={sourceTab === "registry" ? "is-selected" : ""} onClick={() => setSourceTab("registry")}>Registry URL</button><button type="button" role="tab" aria-selected={sourceTab === "github"} className={sourceTab === "github" ? "is-selected" : ""} onClick={() => setSourceTab("github")}>GitHub commit</button><button type="button" role="tab" aria-selected={sourceTab === "local"} className={sourceTab === "local" ? "is-selected" : ""} onClick={() => setSourceTab("local")}>Local artifact</button></div>
        {sourceTab === "registry" ? (
          <form className="package-form" onSubmit={submitRegistry}>
            <label><span>Package ID</span><input aria-label="Registry package ID" value={packageId} onChange={(event) => setPackageId(event.target.value)} placeholder="publisher/gene" maxLength={256} autoComplete="off" spellCheck={false} /></label>
            <label><span>Exact version <small>optional current release</small></span><input aria-label="Registry package version" value={version} onChange={(event) => setVersion(event.target.value)} placeholder="1.0.0" maxLength={128} autoComplete="off" spellCheck={false} /></label>
            <label className="package-form-wide"><span>Saved registry <small>optional · configured on this device</small></span><select aria-label="Saved registry profile" value={registryProfile} onChange={(event) => setRegistryProfile(event.target.value)}><option value="">Custom URL for this install</option>{registryProfiles.map((profile) => <option value={profile.name} key={profile.name}>{profile.name}{profile.active ? " · active" : ""}</option>)}</select></label>
            <label className="package-form-wide"><span>M-Place registry URL</span><input aria-label="Package registry URL" value={selectedRegistryProfile?.base_url ?? registryUrl} onChange={(event) => setRegistryUrl(event.target.value)} placeholder="https://registry.example.com" maxLength={2048} autoComplete="url" spellCheck={false} disabled={Boolean(selectedRegistryProfile)} /></label>
            <label className="package-form-wide"><span>Registry token <small>optional · process-scoped only</small></span><input aria-label="Package registry token" type="password" value={registryToken} onChange={(event) => setRegistryToken(event.target.value)} autoComplete="new-password" /></label>
            <div className="package-form-footer"><p>Saved profiles contain only the URL and a secret reference. Redirects and registry-controlled artifact URLs are refused by the existing client.</p><button className="button button-primary" type="submit" disabled={Boolean(busy) || !packageId.trim() || (!registryProfile && !registryUrl.trim())}>{busy === "install" ? "Installing…" : "Fetch and admit"}</button></div>
          </form>
        ) : sourceTab === "github" ? (
          <form className="package-form" onSubmit={submitGitHub}>
            <label className="package-form-wide"><span>GitHub repository</span><input aria-label="GitHub package repository" value={githubRepository} onChange={(event) => setGithubRepository(event.target.value)} placeholder="https://github.com/owner/repository" maxLength={2048} autoComplete="url" spellCheck={false} /></label>
            <label className="package-form-wide"><span>Exact commit SHA <small>full 40 characters · no branches or tags</small></span><input aria-label="GitHub package commit" value={githubCommit} onChange={(event) => setGithubCommit(event.target.value)} placeholder="0123456789abcdef0123456789abcdef01234567" maxLength={40} autoComplete="off" spellCheck={false} /></label>
            <label><span>Manifest repository path</span><input aria-label="GitHub package manifest path" value={githubManifestPath} onChange={(event) => setGithubManifestPath(event.target.value)} maxLength={1024} autoComplete="off" spellCheck={false} /></label>
            <label><span>Artifact repository path</span><input aria-label="GitHub package artifact path" value={githubArtifactPath} onChange={(event) => setGithubArtifactPath(event.target.value)} maxLength={1024} autoComplete="off" spellCheck={false} /></label>
            <label className="package-form-wide"><span>GitHub token <small>optional · private repositories · process-scoped only</small></span><input aria-label="GitHub package token" type="password" value={githubToken} onChange={(event) => setGithubToken(event.target.value)} autoComplete="new-password" /></label>
            <div className="package-form-footer"><p>Pandora fetches only these two paths at the pinned commit, follows no redirects, and runs the normal signed-admission checks.</p><button className="button button-primary" type="submit" disabled={Boolean(busy) || !githubRepository.trim() || githubCommit.trim().length !== 40 || !githubManifestPath.trim() || !githubArtifactPath.trim()}>{busy === "github" ? "Fetching…" : "Fetch pinned source"}</button></div>
          </form>
        ) : (
          <form className="package-form" onSubmit={submitLocal}>
            <label className="package-form-wide"><span>Absolute manifest path</span><input aria-label="Local package manifest path" value={manifestPath} onChange={(event) => setManifestPath(event.target.value)} placeholder="C:\path\to\manifest.json" maxLength={4096} autoComplete="off" spellCheck={false} /></label>
            <label className="package-form-wide"><span>Absolute artifact path</span><input aria-label="Local package artifact path" value={artifactPath} onChange={(event) => setArtifactPath(event.target.value)} placeholder="C:\path\to\gene.wasm" maxLength={4096} autoComplete="off" spellCheck={false} /></label>
            <div className="package-form-footer"><p>Both paths must be regular local files, not symlinks. Admission remains metadata-only until an exact governed run selects the package.</p><button className="button button-primary" type="submit" disabled={Boolean(busy) || !manifestPath.trim() || !artifactPath.trim()}>{busy === "admit" ? "Admitting…" : "Validate and admit"}</button></div>
          </form>
        )}
      </div>
    </div>
  </div>;
}

function ToolsView({ tools, runtimeStatus, onOpenView }: { tools: RuntimeTool[]; runtimeStatus: RuntimeStatus; onOpenView: (view: ViewId) => void }) {
  const [selectedToolId, setSelectedToolId] = useState("");
  const connected = runtimeStatus === "connected";
  const selected = tools.find((tool) => tool.id === selectedToolId) ?? tools[0] ?? null;
  return <div className="full-view tools-view">
    <PageHeader eyebrow="Runtime surface" title="Built-in Tools" description="Inspect ToolEngine contracts without granting execution authority." actions={<Chip tone={connected ? "green" : "neutral"} icon="terminal">{connected ? `${tools.length} reported` : "Unavailable"}</Chip>} />
    <div className="engine-notice"><Icon name="lock" size={16} /><span>{connected ? "This inventory is reported by the local runtime. Capability and operation metadata remain read-only; execution still requires Harness, Gene, policy, and permit checks." : "Connect the local runtime to inspect built-in tool contracts."}</span></div>
    <div className="tool-workbench">
      <Panel className="tool-browser">
        <div className="panel-heading"><div><span className="eyebrow">TOOL INVENTORY</span><h3>{tools.length} registered contracts</h3></div><Chip tone={connected ? "blue" : "neutral"}>{connected ? "Evidence" : "Offline"}</Chip></div>
        <div className="tool-list">{connected && tools.length ? tools.map((tool) => <button type="button" className={`tool-browser-row ${selected?.id === tool.id ? "is-selected" : ""}`} aria-pressed={selected?.id === tool.id} onClick={() => setSelectedToolId(tool.id)} key={tool.id}><span className="tool-state-mark"><Icon name="terminal" size={13} /></span><span><strong>{tool.name}</strong><small>{tool.capability} / {tool.operation}</small><small className="mono">{tool.id} · v{tool.version}</small></span><Icon name="chevron" size={12} /></button>) : <div className="runs-empty"><Icon name="lock" size={24} /><h3>{connected ? "No tools reported" : "Runtime connection required"}</h3><p>{connected ? "The local service returned an empty tool inventory." : "This surface does not fabricate tool contracts."}</p></div>}</div>
      </Panel>
      <Panel className="tool-inspection">{selected ? <>
        <div className="tool-inspection-hero"><span className="tool-hero-icon"><Icon name="terminal" size={22} /></span><div><span className="eyebrow">RUNTIME-REPORTED CONTRACT</span><h2>{selected.name}</h2><p className="mono">{selected.id}</p></div><Chip tone="blue">v{selected.version}</Chip></div>
        <div className="tool-contract-grid"><div><span>Declared capability</span><strong>{selected.capability}</strong></div><div><span>Declared operation</span><strong>{selected.operation}</strong></div><div><span>Contract source</span><strong className="mono">runtime.tools</strong></div><div><span>Execution path</span><strong>Harness → Gene → ReferenceMonitor → ToolEngine</strong></div></div>
        <div className="tool-proof-grid"><section><span className="eyebrow">WHAT THIS PROVES</span><p>The connected runtime reports this exact tool identity, version, capability, and operation classification.</p></section><section><span className="eyebrow">WHAT THIS DOES NOT PROVE</span><p>Inventory metadata is not a schema validation result, health check, effect permit, or evidence that any Harness may invoke the tool.</p></section></div>
        <pre className="tool-contract-json" aria-label="Tool contract JSON">{JSON.stringify(selected, null, 2)}</pre>
        <div className="tool-inspection-actions"><p><Icon name="shield" size={13} /> Selecting a tool never activates it. ReferenceMonitor remains the sole permit issuer for an exact request.</p><div className="tool-inspection-links"><button className="button button-secondary" type="button" onClick={() => onOpenView("capabilities")}>Inspect Harnesses <Icon name="arrow" size={13} /></button><button className="button button-secondary" type="button" onClick={() => onOpenView("audit")}>Open Audit <Icon name="arrow" size={13} /></button></div></div>
      </> : <div className="runs-empty"><Icon name="terminal" size={27} /><h3>No tool selected</h3><p>Choose a runtime-reported tool to inspect its bounded contract.</p></div>}</Panel>
    </div>
  </div>;
}

function relatedEngineView(engineId: string): { view: ViewId; label: string } | null {
  if (engineId === "tool-engine") return { view: "tools", label: "Inspect tools" };
  if (["memory-engine", "context-engine", "context-recovery", "graph-intelligence-engine"].includes(engineId)) return { view: "memory", label: "Inspect memory evidence" };
  if (["evolution-engine", "adaptive-engine", "coding-feedback-loop", "evaluation-engine", "efficiency-engine", "self-healing-engine", "mutation-engine", "replacement-engine", "population-strategy"].includes(engineId)) return { view: "evolution", label: "Inspect evolution" };
  if (["orchestration-engine", "fleet-engine"].includes(engineId)) return { view: "runs", label: "Inspect background runs" };
  if (engineId === "skill-engine") return { view: "capabilities", label: "Inspect Harnesses" };
  if (["mcp-adapter", "provider-failover"].includes(engineId)) return { view: "connections", label: "Inspect connections" };
  if (["execution-controller", "reference-monitor", "observability-engine"].includes(engineId)) return { view: "audit", label: "Inspect runtime evidence" };
  return null;
}

function inventoryLabel(value: string) {
  if (!value) return "Not reported";
  return value.split("_").map((part) => `${part.charAt(0).toUpperCase()}${part.slice(1)}`).join(" ");
}

function InventoryItems({ items, empty }: { items: string[] | undefined; empty: string }) {
  return items?.length ? <ul>{items.map((item) => <li key={item}>{item}</li>)}</ul> : <p className="inventory-empty-value">{empty}</p>;
}

function RuntimeInventoryView({ engines, runtimeStatus, onOpenView }: { engines: RuntimeEngine[]; runtimeStatus: RuntimeStatus; onOpenView: (view: ViewId) => void }) {
  const [selectedEngineId, setSelectedEngineId] = useState("");
  const [selectedCategory, setSelectedCategory] = useState("All");
  const [inventoryTab, setInventoryTab] = useState<InventoryTab>("overview");
  const connected = runtimeStatus === "connected";
  const categories = ["All", ...Array.from(new Set(engines.map((engine) => engine.category).filter(Boolean)))];
  const visibleEngines = selectedCategory === "All" ? engines : engines.filter((engine) => engine.category === selectedCategory);
  const selected = visibleEngines.find((engine) => engine.id === selectedEngineId) ?? visibleEngines[0] ?? null;
  const related = selected ? relatedEngineView(selected.id) : null;
  const constitutional = selected?.component_kind === "constitutional_core" || (selected ? ["execution-controller", "reference-monitor"].includes(selected.id) : false);
  const embedded = selected?.component_kind === "embedded_component";
  const selectCategory = (category: string) => {
    setSelectedCategory(category);
    setSelectedEngineId("");
    setInventoryTab("overview");
  };

  return <div className="full-view inventory-view">
    <PageHeader eyebrow="Architecture" title="Runtime Inventory" description="Inspect every runtime engine, adapter, strategy, and embedded resilience component from one evidence-backed surface." actions={<Chip tone={connected ? "green" : "neutral"} icon="stack">{connected ? `${engines.length} components` : "Unavailable"}</Chip>} />
    <div className="engine-notice"><Icon name="lock" size={16} /><span>{connected ? "This inventory is reported by the local runtime. It proves declared contracts and source locations—not health, activation, or permission to execute." : "Connect the local runtime to inspect Pandora’s component inventory."}</span></div>
    <div className="inventory-scope-links" aria-label="Inventory surfaces">
      <button type="button" className="is-current" aria-pressed="true"><Icon name="stack" size={14} /><span>Runtime components<small>{engines.length} reported</small></span></button>
      <button type="button" onClick={() => onOpenView("capabilities")}><Icon name="box" size={14} /><span>Harnesses & Genes<small>Composition inventory</small></span></button>
      <button type="button" onClick={() => onOpenView("tools")}><Icon name="terminal" size={14} /><span>Tools<small>Effect contracts</small></span></button>
      <button type="button" onClick={() => onOpenView("connections")}><Icon name="grid" size={14} /><span>Connections<small>Providers, MCP, registries</small></span></button>
    </div>
    <div className="inventory-category-bar" role="group" aria-label="Component category">{categories.map((category) => <button type="button" className={selectedCategory === category ? "is-selected" : ""} aria-pressed={selectedCategory === category} onClick={() => selectCategory(category)} key={category}>{category}{category === "All" ? <span>{engines.length}</span> : <span>{engines.filter((engine) => engine.category === category).length}</span>}</button>)}</div>
    <div className="engine-workbench">
      <Panel className="engine-browser">
        <div className="panel-heading"><div><span className="eyebrow">COMPONENT INVENTORY</span><h3>{visibleEngines.length} in {selectedCategory === "All" ? "runtime" : selectedCategory}</h3></div><Chip tone={connected ? "blue" : "neutral"}>{connected ? "Reported" : "Offline"}</Chip></div>
        <div className="engine-list">{connected && visibleEngines.length ? visibleEngines.map((engine) => <button type="button" className={`engine-browser-row ${selected?.id === engine.id ? "is-selected" : ""}`} aria-pressed={selected?.id === engine.id} onClick={() => { setSelectedEngineId(engine.id); setInventoryTab("overview"); }} key={engine.id}><span className={`engine-state-mark ${engine.component_kind === "constitutional_core" || ["execution-controller", "reference-monitor"].includes(engine.id) ? "is-core" : engine.component_kind === "embedded_component" ? "is-embedded" : ""}`}><Icon name={engine.id === "reference-monitor" ? "shield" : engine.component_kind === "embedded_component" ? "activity" : "stack"} size={13} /></span><span><strong>{engine.name}</strong><small>{engine.role}</small><small>{engine.category || "Unclassified"} · {inventoryLabel(engine.component_kind)}</small></span><Icon name="chevron" size={12} /></button>) : <div className="runs-empty"><Icon name="lock" size={24} /><h3>{connected ? "No components reported" : "Runtime connection required"}</h3><p>{connected ? "The local service returned no components in this category." : "This surface does not fabricate component state."}</p></div>}</div>
      </Panel>
      <Panel className="engine-inspection">{selected ? <>
        <div className="engine-inspection-hero"><span className={`engine-hero-icon ${constitutional ? "is-core" : embedded ? "is-embedded" : ""}`}><Icon name={selected.id === "reference-monitor" ? "shield" : embedded ? "activity" : "stack"} size={22} /></span><div><span className="eyebrow">{constitutional ? "CONSTITUTIONAL RUNTIME BOUNDARY" : embedded ? "EMBEDDED RESILIENCE COMPONENT" : inventoryLabel(selected.component_kind).toUpperCase()}</span><h2>{selected.name}</h2><p className="mono">{selected.id}</p></div><Chip tone={constitutional ? "gold" : embedded ? "amber" : "blue"}>{selected.category || "Reported"}</Chip></div>
        <div className="inventory-tabs" role="tablist" aria-label="Component inspection"><button role="tab" aria-selected={inventoryTab === "overview"} className={inventoryTab === "overview" ? "is-active" : ""} onClick={() => setInventoryTab("overview")}>Overview</button><button role="tab" aria-selected={inventoryTab === "contract"} className={inventoryTab === "contract" ? "is-active" : ""} onClick={() => setInventoryTab("contract")}>Contract</button><button role="tab" aria-selected={inventoryTab === "boundaries"} className={inventoryTab === "boundaries" ? "is-active" : ""} onClick={() => setInventoryTab("boundaries")}>Boundaries</button><button role="tab" aria-selected={inventoryTab === "evidence"} className={inventoryTab === "evidence" ? "is-active" : ""} onClick={() => setInventoryTab("evidence")}>Evidence & source</button></div>
        {inventoryTab === "overview" ? <div className="inventory-tab-panel">
          <div className="engine-contract-grid"><div><span>Owned role</span><strong>{selected.role}</strong></div><div><span>Authority boundary</span><strong>{selected.authority}</strong></div><div><span>Category</span><strong>{selected.category || "Not reported"}</strong></div><div><span>Component kind</span><strong>{inventoryLabel(selected.component_kind)}</strong></div></div>
          <div className="engine-proof-grid"><section><span className="eyebrow">WHAT THIS PROVES</span><p>The connected runtime reports this component’s identity, contract, relationships, evidence classes, and source locations.</p></section><section><span className="eyebrow">WHAT THIS DOES NOT PROVE</span><p>Inventory metadata is not a health check, execution permit, activation receipt, or proof that a replaceable package is trusted.</p></section></div>
          <div className="inventory-authority-map"><article><small>POLICY</small><strong>Parliament</strong><span>Decides policy outside the component inventory.</span></article><article><small>COMPOSITION</small><strong>Shadow Council</strong><span>Selects approved Harness, Gene, and provider compositions.</span></article><article><small>AUTHORIZATION</small><strong>ReferenceMonitor</strong><span>Alone issues exact one-shot effect permits.</span></article><article className={constitutional ? "is-core" : ""}><small>INSPECTED</small><strong>{selected.name}</strong><span>{selected.authority}</span></article></div>
        </div> : inventoryTab === "contract" ? <div className="inventory-tab-panel inventory-contract-sections"><section><span className="eyebrow">INPUTS</span><InventoryItems items={selected.inputs} empty="No inputs reported by this runtime version." /></section><section><span className="eyebrow">OUTPUTS</span><InventoryItems items={selected.outputs} empty="No outputs reported by this runtime version." /></section><section className="inventory-wide"><span className="eyebrow">RELATED COMPONENTS AND AUTHORITIES</span><div className="inventory-token-list">{selected.related_components?.length ? selected.related_components.map((component) => <span key={component}>{component}</span>) : <span>None reported</span>}</div></section></div> : inventoryTab === "boundaries" ? <div className="inventory-tab-panel inventory-boundaries"><section><span className="eyebrow">NON-NEGOTIABLE INVARIANTS</span><InventoryItems items={selected.invariants} empty="No invariants reported by this runtime version." /></section><section className="inventory-boundary-callout"><Icon name="lock" size={18} /><div><strong>Authority never transfers through inspection</strong><p>Parliament decides policy. Shadow Council selects only approved compositions. ReferenceMonitor alone issues exact one-shot permits. {selected.name} cannot grant itself capabilities or bypass those boundaries.</p></div></section></div> : <div className="inventory-tab-panel inventory-evidence-grid"><section><span className="eyebrow">EVIDENCE PRODUCED OR CONSUMED</span><InventoryItems items={selected.evidence} empty="No evidence classes reported by this runtime version." /></section><section><span className="eyebrow">SOURCE MODULES</span><InventoryItems items={selected.source_modules} empty="No source locations reported by this runtime version." /></section><section><span className="eyebrow">DOCUMENTATION</span><InventoryItems items={selected.documentation} empty="No documentation paths reported by this runtime version." /></section><pre className="engine-contract-json" aria-label="Component contract JSON">{JSON.stringify(selected, null, 2)}</pre></div>}
        <div className="engine-inspection-actions"><p><Icon name="lock" size={13} /> Selecting or filtering inventory records never changes the active Harness, Gene, provider, or permit state.</p>{related ? <button className="button button-secondary" type="button" onClick={() => onOpenView(related.view)}>{related.label} <Icon name="arrow" size={13} /></button> : null}</div>
      </> : <div className="runs-empty"><Icon name="stack" size={27} /><h3>No component selected</h3><p>Choose a runtime-reported component to inspect its bounded contract.</p></div>}</Panel>
    </div>
  </div>;
}

function SettingsView({ theme, onThemeChange, runtimeStatus, health, native, endpoint }: { theme: ThemeMode; onThemeChange: (nextTheme: ThemeMode) => void; runtimeStatus: RuntimeStatus; health: RuntimeHealth | null; native: boolean; endpoint: string }) {
  return <div className="full-view"><PageHeader eyebrow="Workspace" title="Settings" description="Personalize the desktop shell while keeping runtime authority in Pandora." actions={<Chip tone="neutral" icon="gear">Local preference</Chip>} /><div className="settings-grid"><Panel className="settings-panel"><div className="panel-heading"><div><span className="eyebrow">APPEARANCE</span><h3>Theme</h3></div><Icon name="spark" size={18} /></div><p className="settings-copy">Choose the visual mode for this device. The setting is stored locally and does not change runtime policy.</p><div className="theme-toggle" role="group" aria-label="Theme mode"><button type="button" className={`theme-option ${theme === "dark" ? "is-selected" : ""}`} aria-pressed={theme === "dark"} onClick={() => onThemeChange("dark")}>Dark<span>Low-light command surface</span></button><button type="button" className={`theme-option ${theme === "light" ? "is-selected" : ""}`} aria-pressed={theme === "light"} onClick={() => onThemeChange("light")}>Light<span>High-contrast workspace</span></button></div></Panel><Panel className="settings-panel"><div className="panel-heading"><div><span className="eyebrow">RUNTIME</span><h3>Connection posture</h3></div><Chip tone={runtimeStatus === "connected" ? "green" : runtimeStatus === "offline" ? "amber" : "neutral"} icon="lock">{runtimeStatusLabel(runtimeStatus)}</Chip></div><div className="settings-facts"><div><span>Client</span><strong>{native ? "Native desktop shell" : "Loopback development shell"}</strong></div><div><span>Endpoint</span><strong className="mono">{endpoint || "Not connected"}</strong></div><div><span>Health</span><strong>{health?.status ?? "Unavailable"}</strong></div><div><span>Authority</span><strong>Local service only</strong></div></div><p className="settings-copy">Local device trust is established automatically. Effect authorization remains inside the Pandora runtime on this device.</p></Panel></div></div>;
}

function ConnectionView({ endpoint, runtimeStatus, runtimeError, health, providers, sessions, selectedSessionId, selectedSession, native, serviceActive, onConnect, onStartService, onStopService, onSelectSession }: { endpoint: string; runtimeStatus: RuntimeStatus; runtimeError: string; health: RuntimeHealth | null; providers: RuntimeProvider[]; sessions: RuntimeSession[]; selectedSessionId: string; selectedSession: RuntimeSessionDetail | null; native: boolean; serviceActive: boolean; onConnect: (endpoint: string, token: string) => void; onStartService: () => Promise<void>; onStopService: () => Promise<void>; onSelectSession: (sessionId: string) => Promise<void> }) {
  const [draftEndpoint, setDraftEndpoint] = useState(endpoint);
  const [draftToken, setDraftToken] = useState("");
  const [configurationTab, setConfigurationTab] = useState<"provider" | "mcp" | "registry">("provider");
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
  const [registryName, setRegistryName] = useState("m-place");
  const [registryBaseUrl, setRegistryBaseUrl] = useState("");
  const [registryTokenEnvironment, setRegistryTokenEnvironment] = useState("");
  const [registryProfileToken, setRegistryProfileToken] = useState("");

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

  const submitRegistryProfile = async (event: FormEvent) => {
    event.preventDefault();
    setConfigurationBusy(true);
    setConfigurationMessage("");
    setConfigurationError("");
    try {
      const result = await configureRegistryProfile({
        name: registryName.trim(),
        baseUrl: registryBaseUrl.trim(),
        tokenEnvironment: registryTokenEnvironment.trim(),
        token: registryProfileToken,
      });
      setRegistryProfileToken("");
      setConfigurationMessage(result.message);
    } catch (error: unknown) {
      setConfigurationError(error instanceof Error ? error.message : "Registry configuration failed");
    } finally {
      setConfigurationBusy(false);
    }
  };

  const providerReady = providerName.trim() && providerUrl.trim() && providerModel.trim() && apiKeyEnvironment.trim();
  const mcpReady = mcpServerId.trim() && mcpProgram.trim() && mcpArguments.trim();
  const registryReady = registryName.trim() && registryBaseUrl.trim() && (!registryProfileToken || registryTokenEnvironment.trim());

  return <div className="full-view">
    <PageHeader eyebrow="Runtime surface" title="Connections" description={native ? "Configure Pandora’s local runtime, providers, and MCP tools on this device." : "Connect this loopback development shell to a local Pandora service."} actions={<Chip tone={runtimeStatus === "connected" ? "green" : runtimeStatus === "offline" ? "amber" : "blue"} icon="lock">{runtimeStatusLabel(runtimeStatus)}</Chip>} />
    <div className="connection-grid">
      <Panel className="connection-panel">
        <div className="panel-heading"><div><span className="eyebrow">LOCAL RPC</span><h3>Pandora service</h3></div><Icon name="terminal" size={19} /></div>
        <div className="settings-facts connection-health"><div><span>Service health</span><strong>{health?.status ?? "Unavailable"}</strong></div><div><span>Transport</span><strong>{native ? "Native bridge" : "Loopback RPC"}</strong></div></div>
        {native ? <><button className={`button ${serviceActive ? "button-secondary" : "button-primary"} connection-start`} type="button" onClick={() => void (serviceActive ? onStopService() : onStartService())}>{serviceActive ? "Stop local service" : "Start local service"} <Icon name={serviceActive ? "lock" : "arrow"} size={14} /></button><div className="native-trust-note"><Icon name="shield" size={17} /><div><strong>Device-local trust</strong><p>The loopback service session is established automatically. Credentials remain native-side and are never exposed to this interface.</p></div></div></> : <><form className="connection-form" onSubmit={submit}><label><span>Endpoint</span><input value={draftEndpoint} onChange={(event) => setDraftEndpoint(event.target.value)} placeholder="http://127.0.0.1:PORT/v1/rpc" spellCheck={false} /></label><label><span>Development token</span><input value={draftToken} onChange={(event) => setDraftToken(event.target.value)} type="password" placeholder="Paste the local service token" autoComplete="off" /></label><button className="button button-secondary" type="submit" disabled={!draftEndpoint.trim() || !draftToken.trim()}>Connect local service <Icon name="arrow" size={14} /></button></form><p className="connection-note">Loopback credentials stay in memory and are never written to storage. Endpoints must be loopback-only.</p></>}
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
          <button type="button" role="tab" aria-selected={configurationTab === "registry"} className={configurationTab === "registry" ? "is-selected" : ""} onClick={() => { setConfigurationTab("registry"); setConfigurationError(""); setConfigurationMessage(""); }}>Package registry</button>
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
        </form> : configurationTab === "mcp" ? <form className="native-config-form" onSubmit={(event) => void submitMcp(event)}>
          <div className="config-form-grid">
            <label><span>Server ID</span><input aria-label="MCP server ID" value={mcpServerId} onChange={(event) => setMcpServerId(event.target.value)} placeholder="local-tools" maxLength={64} autoComplete="off" spellCheck={false} /></label>
            <label><span>Protocol mode</span><select aria-label="MCP protocol mode" value={mcpMode} onChange={(event) => setMcpMode(event.target.value as typeof mcpMode)}><option value="auto">Auto negotiate</option><option value="modern-only">Modern only</option><option value="legacy-only">Legacy only</option></select></label>
            <label className="config-span-2"><span>Absolute program path</span><input aria-label="MCP program path" value={mcpProgram} onChange={(event) => setMcpProgram(event.target.value)} placeholder="C:\path\to\mcp-server.exe" maxLength={4096} autoComplete="off" spellCheck={false} /></label>
            <label className="config-span-2"><span>Arguments <small>JSON array of strings</small></span><textarea aria-label="MCP arguments JSON" value={mcpArguments} onChange={(event) => setMcpArguments(event.target.value)} rows={3} maxLength={65535} spellCheck={false} /></label>
          </div>
          <div className="config-form-footer"><p><Icon name="shield" size={13} /> Pandora records the local executable and arguments. Tool authority is still granted separately by policy.</p><button className="button button-primary" type="submit" disabled={!mcpReady || configurationBusy}>{configurationBusy ? "Saving…" : "Save MCP server"} <Icon name="arrow" size={14} /></button></div>
        </form> : <form className="native-config-form" onSubmit={(event) => void submitRegistryProfile(event)}>
          <div className="config-form-grid">
            <label><span>Profile name</span><input aria-label="Registry profile name" value={registryName} onChange={(event) => setRegistryName(event.target.value)} placeholder="m-place" maxLength={128} autoComplete="off" spellCheck={false} /></label>
            <label><span>Secret reference <small>optional for public registries</small></span><input aria-label="Registry token environment name" value={registryTokenEnvironment} onChange={(event) => setRegistryTokenEnvironment(event.target.value.toUpperCase())} placeholder="PANDORA_MPLACE_TOKEN" maxLength={128} autoComplete="off" spellCheck={false} /></label>
            <label className="config-span-2"><span>Registry base URL</span><input aria-label="Registry profile URL" value={registryBaseUrl} onChange={(event) => setRegistryBaseUrl(event.target.value)} placeholder="https://registry.example.com" maxLength={2048} autoComplete="url" spellCheck={false} /></label>
            <label className="config-span-2"><span>Registry token <small>optional · encrypted native vault</small></span><input aria-label="Registry profile token" type="password" value={registryProfileToken} onChange={(event) => setRegistryProfileToken(event.target.value)} maxLength={65535} autoComplete="new-password" spellCheck={false} /></label>
          </div>
          <div className="config-form-footer"><p><Icon name="lock" size={13} /> The profile stores only its URL and secret reference. A supplied token passes through process stdin into Pandora’s encrypted vault.</p><button className="button button-primary" type="submit" disabled={!registryReady || configurationBusy}>{configurationBusy ? "Saving…" : "Save registry"} <Icon name="arrow" size={14} /></button></div>
        </form>}
        {configurationMessage ? <p className="configuration-result is-success" role="status"><Icon name="check" size={14} /> {configurationMessage}</p> : null}
        {configurationError ? <p className="configuration-result is-error" role="alert">{configurationError}</p> : null}
      </Panel> : null}

      <Panel className="connection-panel">
        <div className="panel-heading"><div><span className="eyebrow">SCOPED SESSIONS</span><h3>{sessions.length} available</h3></div><Chip tone="green" icon="shield">Workspace scoped</Chip></div>
        {sessions.length ? <div className="session-list">{sessions.map((session) => <button className={`session-row ${selectedSessionId === session.session_id ? "is-selected" : ""}`} key={session.session_id} type="button" onClick={() => void onSelectSession(session.session_id)}><span className="session-dot" /><span><strong>{session.session_id}</strong><small>{session.workspace_id} · local device</small></span><Icon name="chevron" size={13} /></button>)}</div> : <div className="connection-empty"><Icon name="archive" size={21} /><p>Connect to load workspace sessions.</p></div>}
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
          {proposal.candidate?.preview ? <div className="artifact-inspector"><div className="artifact-inspector-heading"><div><span className="eyebrow">ARTIFACT INSPECTOR</span><strong>Exact stored text evidence</strong></div><Chip tone="blue">{proposal.candidate.preview.format}</Chip></div><div className="artifact-preview-grid"><section><div><span>BASE</span><small className="mono">{proposal.base_artifact}</small></div><pre aria-label={`Base artifact ${proposal.proposal_id}`}>{proposal.candidate.preview.base}</pre></section><section><div><span>CANDIDATE</span><small className="mono">{proposal.candidate_artifact}</small></div><pre aria-label={`Candidate artifact ${proposal.proposal_id}`}>{proposal.candidate.preview.candidate}</pre></section></div>{proposal.candidate.preview.truncated ? <p className="artifact-preview-note">Preview bounded to 32 KiB per artifact. Hash identity still covers the complete stored bytes.</p> : null}<p className="artifact-preview-boundary"><Icon name="lock" size={12} /> Read-only evidence. Inspecting bytes cannot activate a candidate or widen its authority.</p></div> : null}
        </Panel>;
      })}</div> : <div className="workflow-empty"><div className="empty-emblem"><Icon name="evolution" size={25} /></div><h2>No evolution proposals</h2><p>{runtimeStatus === "connected" ? "The durable evolution and artifact catalogs are available. Self-improvement begins with measured evidence; permission remains separate." : "Connect the local Pandora service to inspect the durable evolution store."}</p></div>}
  </div>;
}

function PageHeader({ eyebrow, title, description, actions }: { eyebrow: string; title: string; description: string; actions: ReactNode }) {
  return <div className="page-header"><div><span className="eyebrow">{eyebrow}</span><h1>{title}</h1><p>{description}</p></div><div className="page-actions">{actions}</div></div>;
}

export { App };
