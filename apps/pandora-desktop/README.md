# Pandora Desktop

This is the first Pandora desktop control surface. It uses a thin Tauri shell
around the React interface; Pandora’s Rust runtime and CLI remain the
execution authority.

## Run locally

```text
npm install
npm run dev
```

Without a connection, the screen uses clearly marked preview data. In the
desktop app, open Connections and choose Start local service. The shell starts
the existing Pandora service process and keeps its bearer token and Ed25519
device key in the native layer. Every native RPC uses a fresh signed device
proof; the webview never receives either credential. Live sessions and run
results come from the authenticated service. The UI cannot issue permits or
execute tools by itself.

Use `Ctrl/Cmd-K` to switch between Pandora surfaces. The Command Center
profile selector routes a run to Auto, Coding, Research, Design, or Security;
the selected profile uses the existing requested Harness field. Its inspector
uses progressive Flow, Evidence, and Context tabs, keeping approvals and the
authority chain close while moving redacted receipts, cache usage, events, and
scope details behind deliberate disclosure.

Harness Lab is a read-only catalog browser for runtime-reported Harnesses,
Genes, plugins and tools, authority posture, and receipt requirements. Catalog
metadata never grants execution authority, and the offline UI does not invent
entries.

Open Settings to choose a locally stored light or dark theme. Select a live
session from the sidebar or Connections to inspect its recorded event count;
the desktop clears the previous run result when you change sessions.

The packaged app resolves `pandora` from `PATH`. Set `PANDORA_CLI_PATH` when
the CLI is installed elsewhere. The service still requires a valid Pandora
configuration and workspace.

Build the desktop shell with:

```text
npm run tauri:dev
npm run tauri:build
```

Prerelease packages may be unsigned. Stable release tags fail closed until the
release environment provides Windows signing and Apple signing/notarization
credentials.

## Interface direction

The shell uses a three-zone layout: a compact navigation rail, an ambient
Command Center, and a dense execution inspector. The layout is inspired by the
supplied reference screenshots but uses Pandora’s own names, authority stages,
colors, and abstract vessel mark.
