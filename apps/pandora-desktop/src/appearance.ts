export type ThemeMode = "system" | "dark" | "light";
export type ThemeAccent = "ember" | "cyan" | "violet";
export type ThemePreset = "foundry" | "verdant";

export type AppearanceSettings = {
  mode: ThemeMode;
  accent: ThemeAccent;
  preset: ThemePreset;
};

export type ThemeTokenContract = {
  color: readonly string[];
  typography: readonly string[];
  radius: readonly string[];
  spacing: readonly string[];
  material: readonly string[];
};

export type ThemeDefinition = {
  id: ThemePreset;
  label: string;
  description: string;
  tokens: ThemeTokenContract;
};

export const appearanceStorageKey = "pandora.desktop.appearance.v1";
const legacyThemeStorageKey = "pandora.desktop.theme";

export const defaultAppearance: AppearanceSettings = {
  mode: "dark",
  accent: "ember",
  preset: "foundry",
};

export const themeTokenContract: ThemeTokenContract = {
  color: [
    "canvas",
    "canvas-raised",
    "surface-1",
    "surface-2",
    "surface-3",
    "text-primary",
    "text-secondary",
    "text-muted",
    "line",
    "line-strong",
    "signal",
    "signal-bright",
    "signal-soft",
    "green",
    "amber",
    "blue",
    "red",
  ],
  typography: ["font-display", "font-mono"],
  radius: ["radius-panel", "radius-control"],
  spacing: ["space-1", "space-2", "space-3", "space-4", "space-5"],
  material: ["surface-glass", "glass-highlight"],
};

export const themeDefinitions: readonly ThemeDefinition[] = [
  {
    id: "foundry",
    label: "Foundry",
    description: "Pandora's neutral command surface.",
    tokens: themeTokenContract,
  },
  {
    id: "verdant",
    label: "Verdant",
    description: "A moss-and-jade reference theme built only from presentation tokens.",
    tokens: themeTokenContract,
  },
];

const themeModes = new Set<ThemeMode>(["system", "dark", "light"]);
const themeAccents = new Set<ThemeAccent>(["ember", "cyan", "violet"]);
const themePresets = new Set<ThemePreset>(["foundry", "verdant"]);

export function isAppearanceSettings(value: unknown): value is AppearanceSettings {
  if (!value || typeof value !== "object") {
    return false;
  }
  const candidate = value as Partial<AppearanceSettings>;
  return themeModes.has(candidate.mode as ThemeMode)
    && themeAccents.has(candidate.accent as ThemeAccent)
    && themePresets.has(candidate.preset as ThemePreset);
}

export function isThemeDefinition(value: unknown): value is ThemeDefinition {
  if (!value || typeof value !== "object") {
    return false;
  }
  const candidate = value as Partial<ThemeDefinition>;
  if (!themePresets.has(candidate.id as ThemePreset)
    || typeof candidate.label !== "string"
    || typeof candidate.description !== "string"
    || !candidate.tokens
    || typeof candidate.tokens !== "object") {
    return false;
  }
  const tokens = candidate.tokens as Partial<ThemeTokenContract>;
  return (Object.keys(themeTokenContract) as (keyof ThemeTokenContract)[]).every((group) => {
    const required = themeTokenContract[group];
    const supplied = tokens[group];
    return Array.isArray(supplied) && required.every((token) => supplied.includes(token));
  });
}

export function loadAppearance(storage?: Pick<Storage, "getItem">): AppearanceSettings {
  if (!storage) {
    return defaultAppearance;
  }
  const serialized = storage.getItem(appearanceStorageKey);
  if (serialized) {
    try {
      const value: unknown = JSON.parse(serialized);
      return isAppearanceSettings(value) ? value : defaultAppearance;
    } catch {
      return defaultAppearance;
    }
  }
  const legacy = storage.getItem(legacyThemeStorageKey);
  if (legacy === "dark" || legacy === "light") {
    return { ...defaultAppearance, mode: legacy };
  }
  return defaultAppearance;
}

export function saveAppearance(storage: Pick<Storage, "setItem">, settings: AppearanceSettings): void {
  storage.setItem(appearanceStorageKey, JSON.stringify(settings));
}

export function resolveThemeMode(mode: ThemeMode, systemPrefersDark: boolean): "dark" | "light" {
  return mode === "system" ? (systemPrefersDark ? "dark" : "light") : mode;
}
