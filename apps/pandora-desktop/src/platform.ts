export type DesktopPlatform = "macos" | "windows" | "linux" | "unknown";

const supportedPlatforms = new Set<DesktopPlatform>(["macos", "windows", "linux"]);

export function detectDesktopPlatform(
  platform = navigator.platform,
  userAgent = navigator.userAgent,
): DesktopPlatform {
  const identity = `${platform} ${userAgent}`.toLowerCase();
  if (identity.includes("mac")) return "macos";
  if (identity.includes("win")) return "windows";
  if (identity.includes("linux") || identity.includes("x11")) return "linux";
  return "unknown";
}

export function resolveDesktopPlatform({
  platform,
  userAgent,
  search = "",
  allowPreviewOverride = false,
}: {
  platform: string;
  userAgent: string;
  search?: string;
  allowPreviewOverride?: boolean;
}): DesktopPlatform {
  if (allowPreviewOverride) {
    const override = new URLSearchParams(search).get("platform") as DesktopPlatform | null;
    if (override && supportedPlatforms.has(override)) return override;
  }
  return detectDesktopPlatform(platform, userAgent);
}

export function installDesktopPlatformMarker(): DesktopPlatform {
  const platform = resolveDesktopPlatform({
    platform: navigator.platform,
    userAgent: navigator.userAgent,
    search: window.location.search,
    allowPreviewOverride: import.meta.env.DEV,
  });
  document.documentElement.dataset.platform = platform;
  return platform;
}
