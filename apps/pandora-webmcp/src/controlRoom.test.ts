import { describe, expect, it } from "vitest";
import { createControlRoomStore } from "./controlRoom";

describe("Pandora control room", () => {
  it("keeps verification behind an exact human decision", async () => {
    const store = createControlRoomStore();
    store.setWebMcpStatus("available", 4);
    const plan = await store.draftPlan({
      objective: "Verify the WebMCP boundary",
      riskBudget: "balanced",
      actor: "agent",
    });
    const request = await store.requestVerification({
      planId: plan.id,
      suite: "release",
      actor: "agent",
    });

    expect(store.getSnapshot().receipts).toHaveLength(0);
    expect(store.getSnapshot().pendingRequest).toEqual(request);

    const receipt = store.decide(request.id, true);
    expect(receipt.decision).toBe("allowed");
    expect(receipt.permit).toBe("spent");
    expect(receipt.checks).toHaveLength(5);
    expect(request.digest).toMatch(/^sha256:[0-9a-f]{64}$/);
    expect(receipt.checks.every((check) => check.status === "passed")).toBe(true);
    expect(store.getSnapshot().pendingRequest).toBeNull();
  });

  it("records denial without issuing a permit or running checks", async () => {
    const store = createControlRoomStore();
    const plan = await store.draftPlan({
      objective: "Inspect only",
      riskBudget: "strict",
    });
    const request = await store.requestVerification({ planId: plan.id, suite: "policy" });
    const receipt = store.decide(request.id, false);

    expect(receipt).toMatchObject({
      decision: "denied",
      permit: "not-issued",
      checks: [],
    });
  });

  it("does not replay a consumed request", async () => {
    const store = createControlRoomStore();
    const plan = await store.draftPlan({
      objective: "Run once",
      riskBudget: "strict",
    });
    const request = await store.requestVerification({ planId: plan.id, suite: "targeted" });
    store.decide(request.id, true);

    expect(() => store.decide(request.id, true)).toThrow("no longer pending");
  });

  it("rejects stale plan identifiers", async () => {
    const store = createControlRoomStore();
    await store.draftPlan({ objective: "Current plan", riskBudget: "balanced" });

    await expect(
      store.requestVerification({ planId: "plan-stale", suite: "release" }),
    ).rejects.toThrow("not active");
  });

  it("records failed release evidence when site tools are unavailable", async () => {
    const store = createControlRoomStore();
    store.setWebMcpStatus("unavailable", 0);
    const plan = await store.draftPlan({
      objective: "Prove release readiness honestly",
      riskBudget: "balanced",
    });
    const request = await store.requestVerification({ planId: plan.id, suite: "release" });
    const receipt = store.decide(request.id, true);

    expect(receipt.checks.find((check) => check.name === "Live WebMCP registration")).toMatchObject({
      status: "failed",
    });
  });
});
