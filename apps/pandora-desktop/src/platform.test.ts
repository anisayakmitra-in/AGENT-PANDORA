import { describe, expect, it } from "vitest";
import { detectDesktopPlatform, resolveDesktopPlatform } from "./platform";

describe("desktop platform detection", () => {
  it.each([
    ["MacIntel", "Mozilla/5.0", "macos"],
    ["Win32", "Mozilla/5.0", "windows"],
    ["Linux x86_64", "Mozilla/5.0", "linux"],
    ["", "Mozilla/5.0 (X11; Ubuntu; Linux x86_64)", "linux"],
    ["", "PandoraHost/1.0", "unknown"],
  ])("detects %s as %s", (platform, userAgent, expected) => {
    expect(detectDesktopPlatform(platform, userAgent)).toBe(expected);
  });

  it("allows a development-only visual preview override", () => {
    expect(resolveDesktopPlatform({ platform: "Linux x86_64", userAgent: "Mozilla/5.0", search: "?platform=macos", allowPreviewOverride: true })).toBe("macos");
    expect(resolveDesktopPlatform({ platform: "Linux x86_64", userAgent: "Mozilla/5.0", search: "?platform=macos", allowPreviewOverride: false })).toBe("linux");
  });

  it("ignores unsupported preview values", () => {
    expect(resolveDesktopPlatform({ platform: "Win32", userAgent: "Mozilla/5.0", search: "?platform=android", allowPreviewOverride: true })).toBe("windows");
  });
});
