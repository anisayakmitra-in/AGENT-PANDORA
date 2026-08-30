import type {
  ControlRoomStore,
  RiskBudget,
  VerificationSuite,
} from "./model";

type JsonSchema = Record<string, unknown>;

interface ModelContextTool<TInput extends Record<string, unknown> = Record<string, unknown>> {
  name: string;
  title?: string;
  description: string;
  inputSchema: JsonSchema;
  annotations?: {
    readOnlyHint?: boolean;
    untrustedContentHint?: boolean;
  };
  execute: (input: TInput, options?: { signal?: AbortSignal }) => unknown | Promise<unknown>;
}

interface ModelContext {
  registerTool: (
    tool: ModelContextTool,
    options?: { signal?: AbortSignal },
  ) => Promise<void>;
}

declare global {
  interface Document {
    modelContext?: ModelContext;
  }
}

export interface WebMcpRegistration {
  available: boolean;
  toolCount: number;
  dispose: () => void;
}

export const PANDORA_SITE_TOOLS = [
  {
    name: "pandora_read_control_room",
    label: "Read control room",
    mode: "read",
  },
  {
    name: "pandora_draft_verification_plan",
    label: "Draft verification plan",
    mode: "write",
  },
  {
    name: "pandora_request_verification",
    label: "Request human review",
    mode: "write",
  },
  {
    name: "pandora_read_verification_evidence",
    label: "Read evidence ledger",
    mode: "read",
  },
] as const;

const EMPTY_SCHEMA: JsonSchema = {
  type: "object",
  properties: {},
  additionalProperties: false,
};

function throwIfAborted(options?: { signal?: AbortSignal }): void {
  if (options?.signal?.aborted) {
    throw options.signal.reason instanceof Error
      ? options.signal.reason
      : new DOMException("The WebMCP tool call was cancelled.", "AbortError");
  }
}

function requireString(
  input: Record<string, unknown>,
  key: string,
  maxLength: number,
): string {
  const value = input[key];
  if (typeof value !== "string" || !value.trim()) {
    throw new Error(`${key} must be a non-empty string.`);
  }
  if (value.length > maxLength) {
    throw new Error(`${key} must be ${maxLength} characters or fewer.`);
  }
  return value.trim();
}

function requireEnum<T extends string>(
  input: Record<string, unknown>,
  key: string,
  choices: readonly T[],
): T {
  const value = requireString(input, key, 64);
  if (!choices.includes(value as T)) {
    throw new Error(`${key} must be one of: ${choices.join(", ")}.`);
  }
  return value as T;
}

function tools(store: ControlRoomStore): ModelContextTool[] {
  return [
    {
      name: "pandora_read_control_room",
      title: "Read Pandora control room",
      description:
        "Read the current Pandora change context, active verification plan, pending human decision, and receipt summary. This does not modify the page.",
      inputSchema: EMPTY_SCHEMA,
      annotations: { readOnlyHint: true, untrustedContentHint: true },
      execute: async (_input, options) => {
        throwIfAborted(options);
        const state = store.getSnapshot();
        return {
          context: state.context,
          activePlan: state.plan,
          pendingRequest: state.pendingRequest,
          receiptCount: state.receipts.length,
          authority:
            "The browser agent may plan and request verification. Only the visible human controls can approve or deny a permit request.",
        };
      },
    },
    {
      name: "pandora_draft_verification_plan",
      title: "Draft a verification plan",
      description:
        "Draft and display a bounded verification plan for the current change. This replaces the current draft but does not run commands, approve effects, or issue a permit.",
      inputSchema: {
        type: "object",
        properties: {
          objective: {
            type: "string",
            minLength: 1,
            maxLength: 480,
            description: "The exact outcome the verification should establish.",
          },
          riskBudget: {
            type: "string",
            enum: ["strict", "balanced", "expansive"],
            description: "How broad the proposed verification may be.",
          },
        },
        required: ["objective", "riskBudget"],
        additionalProperties: false,
      },
      execute: async (input, options) => {
        throwIfAborted(options);
        const objective = requireString(input, "objective", 480);
        const riskBudget = requireEnum<RiskBudget>(input, "riskBudget", [
          "strict",
          "balanced",
          "expansive",
        ]);
        const plan = await store.draftPlan({ objective, riskBudget, actor: "agent" });
        return {
          status: "drafted",
          plan,
          nextStep:
            "Review the plan on the page. To ask for execution, call pandora_request_verification with this plan ID.",
        };
      },
    },
    {
      name: "pandora_request_verification",
      title: "Request a verification run",
      description:
        "Create a visible, digest-bound verification request for human review. This does not run verification, grant approval, or issue a permit. A person must choose Allow once or Deny on the page.",
      inputSchema: {
        type: "object",
        properties: {
          planId: {
            type: "string",
            minLength: 1,
            maxLength: 64,
            description: "The exact active plan ID returned by Pandora.",
          },
          suite: {
            type: "string",
            enum: ["policy", "targeted", "release"],
            description: "The bounded verification suite to request.",
          },
        },
        required: ["planId", "suite"],
        additionalProperties: false,
      },
      execute: async (input, options) => {
        throwIfAborted(options);
        const planId = requireString(input, "planId", 64);
        const suite = requireEnum<VerificationSuite>(input, "suite", [
          "policy",
          "targeted",
          "release",
        ]);
        const request = await store.requestVerification({ planId, suite, actor: "agent" });
        return {
          status: "awaiting_human",
          request,
          effect: "No verification ran and no permit was issued.",
          nextStep: "Ask the person to review the exact request in the Permit Gate.",
        };
      },
    },
    {
      name: "pandora_read_verification_evidence",
      title: "Read verification evidence",
      description:
        "Read immutable allow or deny receipts and check results already visible in the Pandora evidence ledger. This does not run verification or modify the page.",
      inputSchema: {
        type: "object",
        properties: {
          planId: {
            type: "string",
            maxLength: 64,
            description: "Optional plan ID used to filter receipts.",
          },
        },
        additionalProperties: false,
      },
      annotations: { readOnlyHint: true, untrustedContentHint: true },
      execute: async (input, options) => {
        throwIfAborted(options);
        const rawPlanId = input.planId;
        if (rawPlanId !== undefined && typeof rawPlanId !== "string") {
          throw new Error("planId must be a string when provided.");
        }
        if (typeof rawPlanId === "string" && rawPlanId.length > 64) {
          throw new Error("planId must be 64 characters or fewer.");
        }
        const planId = typeof rawPlanId === "string" ? rawPlanId.trim() : "";
        const state = store.getSnapshot();
        const receipts = planId
          ? state.receipts.filter((receipt) => receipt.planId === planId)
          : state.receipts;
        return {
          receipts,
          pendingRequest: state.pendingRequest,
          interpretation:
            receipts.length === 0
              ? "No completed human decision exists for this filter."
              : "A spent permit cannot authorize another verification run.",
        };
      },
    },
  ];
}

export async function registerPandoraWebMcpTools(
  store: ControlRoomStore,
  targetDocument: Document = document,
): Promise<WebMcpRegistration> {
  const registerTool = targetDocument.modelContext?.registerTool;
  if (typeof registerTool !== "function") {
    store.setWebMcpStatus("unavailable", 0);
    return { available: false, toolCount: 0, dispose: () => undefined };
  }

  const controller = new AbortController();
  const definitions = tools(store);
  try {
    for (const tool of definitions) {
      await registerTool.call(targetDocument.modelContext, tool, {
        signal: controller.signal,
      });
    }
    store.setWebMcpStatus("available", definitions.length);
    return {
      available: true,
      toolCount: definitions.length,
      dispose: () => {
        controller.abort();
        store.setWebMcpStatus("unavailable", 0);
      },
    };
  } catch (error) {
    controller.abort();
    store.setWebMcpStatus("unavailable", 0);
    throw error;
  }
}
