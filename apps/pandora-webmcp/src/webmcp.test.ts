import { describe, expect, it, vi } from "vitest";
import { createControlRoomStore } from "./controlRoom";
import { registerPandoraWebMcpTools } from "./webmcp";

interface CapturedTool {
  name: string;
  description: string;
  annotations?: { readOnlyHint?: boolean };
  execute: (input: Record<string, unknown>, options?: { signal?: AbortSignal }) => Promise<Record<string, unknown>>;
}

describe("Pandora WebMCP tools", () => {
  it("registers four imperative tools in the top-level document", async () => {
    const captured: CapturedTool[] = [];
    const registerTool = vi.fn(async (tool: CapturedTool) => {
      captured.push(tool);
    });
    const target = { modelContext: { registerTool } } as unknown as Document;
    const store = createControlRoomStore();

    const registration = await registerPandoraWebMcpTools(store, target);

    expect(registration).toMatchObject({ available: true, toolCount: 4 });
    expect(captured.map((tool) => tool.name)).toEqual([
      "pandora_read_control_room",
      "pandora_draft_verification_plan",
      "pandora_request_verification",
      "pandora_read_verification_evidence",
    ]);
    expect(store.getSnapshot().webMcp).toBe("available");
    expect(store.getSnapshot().registeredToolCount).toBe(4);

    registration.dispose();
    expect(store.getSnapshot()).toMatchObject({ webMcp: "unavailable", registeredToolCount: 0 });
  });

  it("lets an agent plan and request, but exposes no approval tool", async () => {
    const captured: CapturedTool[] = [];
    const target = {
      modelContext: {
        registerTool: async (tool: CapturedTool) => {
          captured.push(tool);
        },
      },
    } as unknown as Document;
    const store = createControlRoomStore();
    await registerPandoraWebMcpTools(store, target);

    const draftTool = captured.find((tool) => tool.name === "pandora_draft_verification_plan")!;
    const draftResult = await draftTool.execute({
      objective: "Verify the shared control room",
      riskBudget: "balanced",
    });
    const plan = draftResult.plan as { id: string };
    const requestTool = captured.find((tool) => tool.name === "pandora_request_verification")!;
    const requestResult = await requestTool.execute({ planId: plan.id, suite: "release" });

    expect(requestResult.status).toBe("awaiting_human");
    expect(store.getSnapshot().receipts).toHaveLength(0);
    expect(store.getSnapshot().pendingRequest).not.toBeNull();
    expect(captured.some((tool) => /approve|allow|permit/.test(tool.name))).toBe(false);
    expect(requestTool.description).toContain("does not run verification");
  });

  it("falls back cleanly when WebMCP is unavailable", async () => {
    const store = createControlRoomStore();
    const registration = await registerPandoraWebMcpTools(store, {} as Document);

    expect(registration).toMatchObject({ available: false, toolCount: 0 });
    expect(store.getSnapshot().webMcp).toBe("unavailable");
  });

  it("honors cancellation before a tool mutates page state", async () => {
    const captured: CapturedTool[] = [];
    const target = {
      modelContext: { registerTool: async (tool: CapturedTool) => { captured.push(tool); } },
    } as unknown as Document;
    const store = createControlRoomStore();
    await registerPandoraWebMcpTools(store, target);
    const draftTool = captured.find((tool) => tool.name === "pandora_draft_verification_plan")!;
    const controller = new AbortController();
    controller.abort(new Error("agent cancelled"));

    await expect(draftTool.execute({ objective: "Do not mutate", riskBudget: "strict" }, { signal: controller.signal })).rejects.toThrow("agent cancelled");
    expect(store.getSnapshot().plan).toBeNull();
  });
});
