import {
  DEFAULT_CONTEXT,
  type ActivityItem,
  type ControlRoomState,
  type ControlRoomStore,
  type DraftPlanInput,
  type PermitRequest,
  type RequestVerificationInput,
  type VerificationCheck,
  type VerificationPlan,
  type VerificationReceipt,
  type VerificationSuite,
} from "./model";

const MAX_OBJECTIVE_LENGTH = 480;

function shortDigest(value: string): string {
  let hash = 0x811c9dc5;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return Math.abs(hash >>> 0).toString(16).padStart(8, "0");
}

async function sha256Digest(value: string): Promise<string> {
  const bytes = new TextEncoder().encode(value);
  const hash = await globalThis.crypto.subtle.digest("SHA-256", bytes);
  return Array.from(new Uint8Array(hash))
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}

function validateObjective(value: string): string {
  const objective = value.trim();
  if (!objective) {
    throw new Error("An objective is required before Pandora can draft a plan.");
  }
  if (objective.length > MAX_OBJECTIVE_LENGTH) {
    throw new Error(`The objective must be ${MAX_OBJECTIVE_LENGTH} characters or fewer.`);
  }
  return objective;
}

function planSteps(riskBudget: DraftPlanInput["riskBudget"]): string[] {
  const base = [
    "Inspect the declared change scope and protected authority paths.",
    "Type-check the WebMCP contracts and execute focused behavior tests.",
  ];
  if (riskBudget !== "strict") {
    base.push("Build the production bundle and verify deployment headers.");
  }
  if (riskBudget === "expansive") {
    base.push("Run the repository-level formatting and compatibility checks.");
  }
  return base;
}

function checksForSuite(state: ControlRoomState, suite: VerificationSuite): VerificationCheck[] {
  const protectedChanges = state.context.changedFiles.filter((file) =>
    state.context.protectedPaths.includes(file),
  );
  const boundedPlan = Boolean(
    state.plan &&
      state.plan.objective.length <= MAX_OBJECTIVE_LENGTH &&
      state.plan.requiredChecks.every((check) => state.context.declaredChecks.includes(check)),
  );
  const checks: VerificationCheck[] = [
    {
      name: "Authority boundary",
      status: protectedChanges.length === 0 ? "passed" : "failed",
      detail:
        protectedChanges.length === 0
          ? `${state.context.changedFiles.length} declared files stay outside Pandora's protected authority paths.`
          : `Protected paths changed: ${protectedChanges.join(", ")}`,
    },
    {
      name: "Bounded WebMCP inputs",
      status: boundedPlan ? "passed" : "failed",
      detail: boundedPlan
        ? "The active plan is bounded to 480 characters and references only declared checks."
        : "The active plan is missing, oversized, or references an undeclared check.",
    },
  ];

  if (suite !== "policy") {
    const hasFocusedPlan = Boolean(state.plan && state.plan.steps.length >= 2);
    checks.push({
      name: "Focused contract suite",
      status: hasFocusedPlan ? "passed" : "failed",
      detail: hasFocusedPlan
        ? `${state.plan?.steps.length ?? 0} bounded verification steps are attached to this exact plan.`
        : "The active plan does not contain the minimum focused verification steps.",
    });
  }
  if (suite === "release") {
    const siteToolsLive = state.webMcp === "available" && state.registeredToolCount === 4;
    checks.push({
      name: "Live WebMCP registration",
      status: siteToolsLive ? "passed" : "failed",
      detail: siteToolsLive
        ? "Four imperative tools are registered on the top-level document."
        : `Expected 4 live top-level tools; observed ${state.registeredToolCount}.`,
    });
    const hasDeploymentManifest = state.context.changedFiles.includes(
      "apps/pandora-webmcp/netlify.toml",
    );
    checks.push({
      name: "Deployment posture",
      status: hasDeploymentManifest ? "passed" : "failed",
      detail: hasDeploymentManifest
        ? "The declared change includes the Netlify build and security-header manifest."
        : "The declared change does not include its production deployment manifest.",
    });
  }
  return checks;
}

function initialState(): ControlRoomState {
  return {
    objective: "Verify the WebMCP Permit Room without changing Pandora's authority model.",
    context: DEFAULT_CONTEXT,
    plan: null,
    pendingRequest: null,
    receipts: [],
    activity: [
      {
        id: "activity-ready",
        actor: "runtime",
        message: "Control room ready. No permit has been issued.",
      },
    ],
    webMcp: "checking",
    registeredToolCount: 0,
  };
}

export function createControlRoomStore(): ControlRoomStore {
  let state = initialState();
  const listeners = new Set<() => void>();

  const publish = (next: ControlRoomState) => {
    state = next;
    listeners.forEach((listener) => listener());
  };

  const addActivity = (
    current: ControlRoomState,
    actor: ActivityItem["actor"],
    message: string,
  ): ActivityItem[] => [
    {
      id: `activity-${shortDigest(`${message}-${current.activity.length}`)}`,
      actor,
      message,
    },
    ...current.activity,
  ];

  return {
    getSnapshot: () => state,
    subscribe: (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    setObjective: (objective) => publish({ ...state, objective: objective.slice(0, MAX_OBJECTIVE_LENGTH) }),
    setWebMcpStatus: (webMcp, registeredToolCount = 0) => {
      if (
        state.webMcp !== webMcp ||
        state.registeredToolCount !== registeredToolCount
      ) {
        const message =
          webMcp === "available"
            ? `Registered ${registeredToolCount} top-level WebMCP site tools.`
            : "WebMCP is unavailable in this browser. Human demo controls remain active.";
        publish({
          ...state,
          webMcp,
          registeredToolCount,
          activity: addActivity(state, "runtime", message),
        });
      }
    },
    draftPlan: async ({ objective: rawObjective, riskBudget, actor = "human" }) => {
      const objective = validateObjective(rawObjective);
      const requiredChecks =
        riskBudget === "strict"
          ? state.context.declaredChecks.slice(0, 2)
          : state.context.declaredChecks;
      const planDigest = await sha256Digest(`${objective}:${riskBudget}:${state.context.branch}`);
      const id = `plan-${planDigest.slice(0, 12)}`;
      const plan: VerificationPlan = {
        id,
        objective,
        riskBudget,
        steps: planSteps(riskBudget),
        requiredChecks,
        createdBy: actor,
      };
      publish({
        ...state,
        objective,
        plan,
        pendingRequest: null,
        activity: addActivity(state, actor, `Drafted ${riskBudget} verification plan ${id}.`),
      });
      return plan;
    },
    requestVerification: async ({ planId, suite, actor = "human" }: RequestVerificationInput) => {
      if (!state.plan || state.plan.id !== planId) {
        throw new Error("The requested plan is not active. Read the current context and use its plan ID.");
      }
      if (state.pendingRequest) {
        throw new Error("A verification request is already waiting for a human decision.");
      }
      const requestDigest = await sha256Digest(
        `${planId}:${suite}:${state.plan.objective}:${state.receipts.length}`,
      );
      const request: PermitRequest = {
        id: `request-${requestDigest.slice(0, 12)}`,
        planId,
        suite,
        digest: `sha256:${requestDigest}`,
        summary: `Run the ${suite} verification suite for ${planId}`,
        requestedBy: actor,
      };
      publish({
        ...state,
        pendingRequest: request,
        activity: addActivity(
          state,
          actor,
          `Requested ${suite} verification. Human approval is still required.`,
        ),
      });
      return request;
    },
    decide: (requestId, allow) => {
      const request = state.pendingRequest;
      if (!request || request.id !== requestId) {
        throw new Error("This request is no longer pending.");
      }
      const checks = allow ? checksForSuite(state, request.suite) : [];
      const receipt: VerificationReceipt = {
        id: `receipt-${shortDigest(`${request.id}:${allow}:${state.receipts.length}`)}`,
        requestId: request.id,
        planId: request.planId,
        suite: request.suite,
        decision: allow ? "allowed" : "denied",
        permit: allow ? "spent" : "not-issued",
        digest: request.digest,
        checks,
        recordedAt: Date.now(),
      };
      publish({
        ...state,
        pendingRequest: null,
        receipts: [receipt, ...state.receipts],
        activity: addActivity(
          state,
          allow ? "runtime" : "human",
          allow
            ? `Consumed one permit and recorded ${checks.length} verification checks.`
            : "Denied the request. No permit was issued and no verification ran.",
        ),
      });
      return receipt;
    },
    reset: () =>
      publish({
        ...initialState(),
        webMcp: state.webMcp,
        registeredToolCount: state.registeredToolCount,
      }),
  };
}
