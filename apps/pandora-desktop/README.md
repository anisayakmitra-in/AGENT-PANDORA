# Pandora Desktop

Pandora Desktop is the local graphical control surface for the same runtime
used by the `pandora` CLI. Its Tauri shell hosts the React interface;
Pandora's Rust runtime, Parliament, and ReferenceMonitor retain execution
authority.

## Run locally

```text
npm install
npm run dev
```

Without a connection, the screen uses clearly marked preview data. Pandora is a
local application and has no account, login, or sign-in screen. In the desktop
app, open Connections and choose Start local service. The shell establishes
device trust automatically, starts the existing Pandora service process, and
keeps its service credential and Ed25519 device key in the native layer. Every
native RPC uses a fresh signed device proof; the webview never receives either
credential. Live sessions and run results come from the authenticated local
service. The UI cannot issue permits or execute tools by itself.

Use `Ctrl/Cmd-K` to switch between Pandora surfaces. The Command Center
profile selector routes a run to Auto, Coding, Research, Design, or Security;
the selected profile uses the existing requested Harness field. Its inspector
uses progressive Flow, Evidence, and Context tabs, keeping approvals and the
authority chain close while moving redacted receipts, cache usage, events, and
scope details behind deliberate disclosure.

Harness Lab reads runtime-reported Harnesses, Genes, plugins, tools, authority
posture, and receipt requirements. Its native Skills tab uses SkillEngine's
separate local lifecycle to install, inspect, enable, disable, suspend, remove,
and restore Skills. Skill changes require a local-service restart and never
grant execution authority. The offline UI does not invent catalog entries.

Connections can create Provider profiles and select the active profile through
Pandora's native configuration boundary. API keys remain in the encrypted local
vault; the webview receives only readiness metadata.

Background Runs inspects the scoped durable orchestration queue, exact repository
and commit assignments, worker ownership, role state, receipts, and handoffs.
The desktop can exactly cancel queued runs and resume safely reconciled
interruptions; it cannot claim work, steal leases, complete roles, mint permits,
or bypass the existing Harness and ReferenceMonitor path.

Open Settings to choose system, light, or dark mode, one of three accessible
accents, and a validated built-in token preset. The live Appearance gallery and
the non-default Verdant reference theme use presentation tokens only; invalid
or incomplete local data falls back to the safe Foundry preset. See
`docs/DESKTOP_THEMES.md` for the contributor contract. Select a live session
from the sidebar or Connections to inspect its recorded event count;
the desktop clears the previous run result when you change sessions.

The optional Pandora Orbit companion is off by default. It maps only typed,
already-public UI states (`idle`, `working`, `waiting`, `success`, and
`failure`) to bundled local images, has a static mode and immediate disable
control, and persists display preferences only. Declarative pack validation and
the no-authority boundary are documented in `docs/DESKTOP_COMPANIONS.md`.

Release bundles include the same-commit Pandora CLI as a native sidecar, so the
app does not depend on a shell or an inherited `PATH`. For local development,
`PANDORA_CLI_PATH` remains an explicit override and must point to an absolute,
regular, non-symlink executable. Release builds fail closed when the bundled
sidecar is unavailable. The service still requires a valid Pandora configuration
and workspace.

Build the desktop shell on Linux, macOS, or Windows with:

```text
npm run tauri:dev
npm run tauri:build
```

On macOS, `script/build_and_run.sh` is the stable build/run entrypoint. It
supports `--verify`, `--debug`, `--logs`, and `--telemetry`. On macOS 26,
the packaged app installs AppKit's supported `NSGlassEffectView` Clear
material with a 26-point radius. If Liquid Glass is unavailable, it attempts
the semantic `UnderWindowBackground` vibrancy material. A material failure is
cosmetic and does not stop Pandora. Linux remains opaque and leaves compositor
effects to the user's desktop.

The transparent macOS webview requires Tauri’s `macOSPrivateApi`, so Pandora’s
macOS package is intended for signed and notarized direct distribution rather
than the Mac App Store.

Prerelease packages may be unsigned. Stable release tags fail closed until the
release environment provides Windows signing and Apple signing/notarization
credentials.

## Accessibility

The shell exposes a skip link, named navigation and main landmarks, focus-contained
Quick Open dialog, and one-tab-stop tablists with Arrow, Home, and End key
navigation. View changes move focus to the selected workspace; dismissing Quick
Open returns focus to its invoking control. Runtime and view changes are announced
through a polite live region.

Typography uses scalable root-relative units. The stylesheet honors reduced
motion, reduced transparency, increased contrast, and Windows forced-colors
preferences. Every Desktop CI runner now performs a Chromium and axe audit of
the rendered Command Center at 100%, 150%, and 200% scale equivalents. It also
checks keyboard-only visible focus, forced colors, increased contrast, reduced
motion, and reduced transparency. All supported inspector layouts must avoid
horizontal clipping. Each runner also extracts, copies, or administratively
installs its freshly built package in a temporary sandbox, verifies the bundled
sidecar has the same SHA-256 and version as the release CLI it just built,
starts a bounded smoke where the runner supports it, and removes that sandbox.
Screenshots and privacy-safe lifecycle JSON are retained for 90 days.

The same Linux, macOS, and Windows jobs also build two synthetic stable package
identities from the same commit. They install and launch the predecessor,
replace it with the newer package, launch the update, roll back to the
predecessor, launch it again, and uninstall it. This proves the native installer
mechanics and stable product identity without publishing fake releases. It does
not prove migration compatibility between two real releases or replace signed
stable-release evidence.

This is automated webview and package-lifecycle evidence, not native NVDA,
VoiceOver, or Orca coverage. A strict exact-commit evidence workflow admits the
four real graphical-session records only after every asset checksum and required
check passes. Signed release packages and those native screen-reader and OS
scaling sessions remain release gates.
The repeatable native test protocol and retained-evidence fields are documented
in [Desktop accessibility evidence](../../docs/ACCESSIBILITY.md).

## Interface direction

The shell uses a three-zone layout: a compact navigation rail, an ambient
Command Center, and a dense execution inspector. Its names, authority stages,
colors, and vessel mark belong to Pandora.

The execution inspector is a persistent Witness Dock. It can sit on the right
or below the Command Center, use three bounded sizes, or be hidden and restored.
Flow, Evidence, Work, and Browser remain the same read-only inspection surfaces
in every layout. Grouped, searchable Settings store these presentation choices
on the device and provide a one-click reset to the shown, right-side,
comfortable default. Invalid stored values recover to those safe defaults. If
the dock is hidden when a run pauses, the exact approval digest and explicit
Deny/Allow once controls remain visible in the Command Center; hiding a panel
never resolves an approval. Layout controls never select a Gene, approve an
effect, or issue a permit.
