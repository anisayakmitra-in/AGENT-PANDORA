export type RiskBudget = "strict" | "balanced" | "expansive";
export type VerificationSuite = "policy" | "targeted" | "release";
export type CheckStatus = "passed" | "failed";

export interface ChangeContext {
  project: string;
  branch: string;
  summary: string;
  changedFiles: string[];
  protectedPaths: string[];
  declaredChecks: string[];
}

export interface VerificationPlan {
  id: string;
  objective: string;
  riskBudget: RiskBudget;
  steps: string[];
  requiredChecks: string[];
  createdBy: "human" | "agent";
}

export interface PermitRequest {
  id: string;
  planId: string;
  suite: VerificationSuite;
  digest: string;
  summary: string;
  requestedBy: "human" | "agent";
}

export interface VerificationCheck {
  name: string;
  status: CheckStatus;
  detail: string;
}

export interface VerificationReceipt {
  id: string;
  requestId: string;
  planId: string;
  suite: VerificationSuite;
  decision: "allowed" | "denied";
  permit: "spent" | "not-issued";
  digest: string;
  checks: VerificationCheck[];
  recordedAt: number;
}

export interface ActivityItem {
  id: string;
  actor: "human" | "agent" | "runtime";
  message: string;
}

export interface ControlRoomState {
  objective: string;
  context: ChangeContext;
  plan: VerificationPlan | null;
  pendingRequest: PermitRequest | null;
  receipts: VerificationReceipt[];
  activity: ActivityItem[];
  webMcp: "checking" | "available" | "unavailable";
  registeredToolCount: number;
}

export interface DraftPlanInput {
  objective: string;
  riskBudget: RiskBudget;
  actor?: "human" | "agent";
}

export interface RequestVerificationInput {
  planId: string;
  suite: VerificationSuite;
  actor?: "human" | "agent";
}

export interface ControlRoomStore {
  getSnapshot: () => ControlRoomState;
  subscribe: (listener: () => void) => () => void;
  setObjective: (objective: string) => void;
  setWebMcpStatus: (status: ControlRoomState["webMcp"], toolCount?: number) => void;
  draftPlan: (input: DraftPlanInput) => Promise<VerificationPlan>;
  requestVerification: (input: RequestVerificationInput) => Promise<PermitRequest>;
  decide: (requestId: string, allow: boolean) => VerificationReceipt;
  reset: () => void;
}

export const DEFAULT_CONTEXT: ChangeContext = {
  project: "Pandora Agent",
  branch: "codex/webmcp-challenge",
  summary: "Add a browser-native, human-governed verification room using WebMCP.",
  changedFiles: [
    "apps/pandora-webmcp/src/App.tsx",
    "apps/pandora-webmcp/src/controlRoom.ts",
    "apps/pandora-webmcp/src/webmcp.ts",
    "apps/pandora-webmcp/src/styles.css",
    "apps/pandora-webmcp/README.md",
    "apps/pandora-webmcp/HACKATHON.md",
    "apps/pandora-webmcp/netlify.toml",
  ],
  protectedPaths: [
    "crates/pandora-runtime/src/reference_monitor.rs",
    "crates/pandora-runtime/src/parliament.rs",
    "crates/pandora-runtime/src/shadow_council.rs",
  ],
  declaredChecks: [
    "Protected path boundary",
    "Bounded schema contract",
    "Live WebMCP registration",
    "Production deployment manifest",
  ],
};
