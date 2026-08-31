import { describe, expect, it } from "vitest";
import {
  appearanceStorageKey,
  defaultAppearance,
  isThemeDefinition,
  loadAppearance,
  resolveThemeMode,
  themeDefinitions,
  themeTokenContract,
} from "./appearance";

function storageWith(values: Record<string, string>) {
  return { getItem: (key: string) => values[key] ?? null };
}

describe("desktop appearance contract", () => {
  it("loads a complete persisted selection", () => {
    const value = { mode: "dark", accent: "cyan", preset: "verdant" };
    expect(loadAppearance(storageWith({ [appearanceStorageKey]: JSON.stringify(value) }))).toEqual(value);
  });

  it("fails closed when persisted selection data is invalid or incomplete", () => {
    expect(loadAppearance(storageWith({ [appearanceStorageKey]: "{" }))).toEqual(defaultAppearance);
    expect(loadAppearance(storageWith({
      [appearanceStorageKey]: JSON.stringify({ mode: "dark", accent: "cyan" }),
    }))).toEqual(defaultAppearance);
    expect(loadAppearance(storageWith({
      [appearanceStorageKey]: JSON.stringify({ mode: "ultraviolet", accent: "cyan", preset: "foundry" }),
    }))).toEqual(defaultAppearance);
  });

  it("migrates the former light and dark preference", () => {
    expect(loadAppearance(storageWith({ "pandora.desktop.theme": "light" }))).toEqual({
      ...defaultAppearance,
      mode: "light",
    });
  });

  it("resolves system mode without granting a runtime setting", () => {
    expect(resolveThemeMode("system", true)).toBe("dark");
    expect(resolveThemeMode("system", false)).toBe("light");
    expect(resolveThemeMode("dark", false)).toBe("dark");
  });

  it("requires every documented token group and token", () => {
    expect(themeDefinitions.every(isThemeDefinition)).toBe(true);
    expect(isThemeDefinition({
      id: "verdant",
      label: "Incomplete",
      description: "Missing material tokens",
      tokens: { ...themeTokenContract, material: [] },
    })).toBe(false);
  });
});
