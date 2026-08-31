import { describe, expect, it } from "vitest";
import {
  builtInCompanionManifest,
  companionStates,
  companionStorageKey,
  defaultCompanionSettings,
  deriveCompanionState,
  loadCompanionSettings,
  validateCompanionPack,
  type CompanionAssetEvidence,
} from "./companion";

const validEvidence = (): CompanionAssetEvidence[] => companionStates.map((state) => ({
  path: builtInCompanionManifest.assets[state],
  bytes: 1024,
  regular_file: true,
  symlink: false,
  executable: false,
}));

describe("local companion contract", () => {
  it("is off by default and fails closed on invalid persistence", () => {
    expect(defaultCompanionSettings.enabled).toBe(false);
    expect(loadCompanionSettings({ getItem: () => "{" })).toEqual(defaultCompanionSettings);
    expect(loadCompanionSettings({ getItem: (key) => key === companionStorageKey ? JSON.stringify({ enabled: true }) : null })).toEqual(defaultCompanionSettings);
  });

  it("maps only typed public run states", () => {
    expect(deriveCompanionState({ working: false, approvalRequired: false })).toBe("idle");
    expect(deriveCompanionState({ working: true, approvalRequired: false })).toBe("working");
    expect(deriveCompanionState({ working: true, approvalRequired: true })).toBe("waiting");
    expect(deriveCompanionState({ working: false, approvalRequired: false, status: "completed" })).toBe("success");
    expect(deriveCompanionState({ working: false, approvalRequired: false, status: "failed" })).toBe("failure");
  });

  it("accepts the complete built-in declarative manifest", () => {
    expect(validateCompanionPack(builtInCompanionManifest, validEvidence())).toEqual({ valid: true });
  });

  it.each([
    ["remote", "https://example.com/pet.svg", {}],
    ["traversal", "../pet.svg", {}],
    ["symlink", builtInCompanionManifest.assets.idle, { symlink: true }],
    ["executable", builtInCompanionManifest.assets.idle, { executable: true }],
    ["oversized", builtInCompanionManifest.assets.idle, { bytes: 300 * 1024 }],
  ])("rejects %s companion assets", (_label, path, override) => {
    const manifest = structuredClone(builtInCompanionManifest);
    manifest.assets.idle = path;
    const evidence = validEvidence().map((asset) => asset.path === builtInCompanionManifest.assets.idle ? { ...asset, ...override, path } : asset);
    expect(validateCompanionPack(manifest, evidence)).toMatchObject({ valid: false });
  });
});
