export type CompanionState = "idle" | "working" | "waiting" | "success" | "failure";
export type CompanionPosition = "bottom-left" | "bottom-right";
export type CompanionScale = "small" | "medium" | "large";
export type CompanionMotion = "system" | "static";

export type CompanionSettings = {
  enabled: boolean;
  position: CompanionPosition;
  scale: CompanionScale;
  motion: CompanionMotion;
};

export type CompanionManifest = {
  schema_version: 1;
  id: string;
  label: string;
  assets: Record<CompanionState, string>;
};

export type CompanionAssetEvidence = {
  path: string;
  bytes: number;
  regular_file: boolean;
  symlink: boolean;
  executable: boolean;
};

export type CompanionPackValidation = { valid: true } | { valid: false; reason: string };

export const companionStorageKey = "pandora.desktop.companion.v1";
export const defaultCompanionSettings: CompanionSettings = {
  enabled: false,
  position: "bottom-right",
  scale: "medium",
  motion: "system",
};

export const companionStates: readonly CompanionState[] = ["idle", "working", "waiting", "success", "failure"];
const maxAssetBytes = 256 * 1024;
const maxPackBytes = 1024 * 1024;

export const builtInCompanionManifest: CompanionManifest = {
  schema_version: 1,
  id: "pandora-orbit",
  label: "Pandora Orbit",
  assets: {
    idle: "orbit/idle.svg",
    working: "orbit/working.svg",
    waiting: "orbit/waiting.svg",
    success: "orbit/success.svg",
    failure: "orbit/failure.svg",
  },
};

export function isCompanionSettings(value: unknown): value is CompanionSettings {
  if (!value || typeof value !== "object") return false;
  const candidate = value as Partial<CompanionSettings>;
  return typeof candidate.enabled === "boolean"
    && (candidate.position === "bottom-left" || candidate.position === "bottom-right")
    && (candidate.scale === "small" || candidate.scale === "medium" || candidate.scale === "large")
    && (candidate.motion === "system" || candidate.motion === "static");
}

export function loadCompanionSettings(storage?: Pick<Storage, "getItem">): CompanionSettings {
  if (!storage) return defaultCompanionSettings;
  try {
    const serialized = storage.getItem(companionStorageKey);
    if (!serialized) return defaultCompanionSettings;
    const value: unknown = JSON.parse(serialized);
    return isCompanionSettings(value) ? value : defaultCompanionSettings;
  } catch {
    return defaultCompanionSettings;
  }
}

export function saveCompanionSettings(storage: Pick<Storage, "setItem">, settings: CompanionSettings): void {
  storage.setItem(companionStorageKey, JSON.stringify(settings));
}

export function deriveCompanionState(input: { working: boolean; approvalRequired: boolean; status?: string | null }): CompanionState {
  if (input.approvalRequired) return "waiting";
  if (input.working) return "working";
  if (input.status === "completed") return "success";
  if (input.status === "failed" || input.status === "denied" || input.status === "cancelled") return "failure";
  return "idle";
}

function safeAssetPath(path: string): boolean {
  if (!path || path.includes("\\") || path.includes("\0") || path.startsWith("/") || /^[a-z][a-z0-9+.-]*:/i.test(path)) return false;
  const segments = path.split("/");
  return segments.every((segment) => segment !== "" && segment !== "." && segment !== "..")
    && /\.(png|webp|svg)$/i.test(path);
}

export function validateCompanionPack(manifest: unknown, evidence: readonly CompanionAssetEvidence[]): CompanionPackValidation {
  if (!manifest || typeof manifest !== "object") return { valid: false, reason: "manifest must be an object" };
  const candidate = manifest as Partial<CompanionManifest>;
  if (candidate.schema_version !== 1 || typeof candidate.id !== "string" || !/^[a-z0-9][a-z0-9-]{1,63}$/.test(candidate.id)) {
    return { valid: false, reason: "manifest identity is invalid" };
  }
  if (typeof candidate.label !== "string" || candidate.label.trim().length === 0 || candidate.label.length > 80 || !candidate.assets) {
    return { valid: false, reason: "manifest label or assets are incomplete" };
  }
  const evidenceByPath = new Map(evidence.map((asset) => [asset.path, asset]));
  let totalBytes = 0;
  for (const state of companionStates) {
    const path = candidate.assets[state];
    if (typeof path !== "string" || !safeAssetPath(path)) return { valid: false, reason: `${state} asset path is unsafe` };
    const asset = evidenceByPath.get(path);
    if (!asset || !asset.regular_file || asset.symlink || asset.executable) return { valid: false, reason: `${state} asset is not a safe regular file` };
    if (!Number.isSafeInteger(asset.bytes) || asset.bytes <= 0 || asset.bytes > maxAssetBytes) return { valid: false, reason: `${state} asset exceeds the size limit` };
    totalBytes += asset.bytes;
  }
  return totalBytes <= maxPackBytes ? { valid: true } : { valid: false, reason: "companion pack exceeds the size limit" };
}
