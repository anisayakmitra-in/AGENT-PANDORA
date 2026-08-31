# Desktop theme contract

Pandora desktop themes are local presentation data. They cannot execute code,
read prompts, memories, secrets, receipts, or workspace files, invoke tools,
change routing, resolve approvals, or grant permits.

The supported contract is defined in `apps/pandora-desktop/src/appearance.ts`.
Every definition must include all documented token names in these groups:

- color: canvas and surface levels, primary/secondary/muted text, borders,
  signal states, and semantic green, amber, blue, and red;
- typography: display and monospace families;
- radius: panel and control radii;
- spacing: five fixed spacing steps;
- material: the glass surface and highlight.

Definitions are declarative and selected by a known identifier. Missing token
groups, unknown modes, accents, or presets, malformed JSON, and incomplete
selections fall back to the built-in Dark + Ember + Foundry selection. The
old light/dark storage key is migrated once for existing users.

Use Settings > Appearance to exercise representative inputs, primary and
secondary actions, status chips, and an unresolved approval state. Test both
Foundry and the Verdant reference preset in system, light, and dark mode. Theme
work must also pass forced-colors, increased-contrast, reduced-motion,
reduced-transparency, keyboard focus, and 200% zoom checks.
