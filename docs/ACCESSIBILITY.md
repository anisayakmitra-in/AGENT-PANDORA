# Desktop accessibility evidence

Pandora separates automated webview checks from native assistive-technology
evidence. Neither substitutes for the other.

## Automated checks

Every desktop CI runner checks the rendered Command Center with axe, the
1080×720 minimum window, and a 540×360 viewport representing 200% zoom. These
checks catch semantic and clipping regressions in the shared webview.

## Native Windows checkpoint

On 2026-08-30, the Windows x64 app built from commit `a39f063` exposed a named
UI Automation window and `RootWebArea`. The tree included the skip link,
Pandora navigation landmarks, labeled task input, Harness/provider/model
controls, run-inspector tabs, approval state, authority-chain controls, and
disabled-state metadata. Narrator was started while Pandora held document
focus. The six lifecycle-script tests and a real MSI extract, launch, process
tree stop, and sandbox cleanup also passed locally.

This checkpoint proves native UI Automation discovery. It does not certify the
quality or order of every spoken Narrator announcement: live user input
interrupted the keyboard-focus traversal, so that part remains open.

## Required retained evidence

For each advertised desktop platform, record:

1. Exact Pandora commit or tag and artifact SHA-256.
2. OS build, architecture, assistive technology, and assistive-technology version.
3. Install source and whether the artifact was signed and notarized where required.
4. Results for landmarks, controls, forms, status changes, dialogs, keyboard order, and 200% scaling.
5. Failures, workarounds, and the issue or commit that resolves each failure.

## Native test protocol

Use the packaged app, not the Vite preview.

1. Launch Narrator on Windows, VoiceOver on macOS, or Orca on Linux before
   focusing Pandora.
2. Traverse from the skip link through navigation, the task form, Harness and
   model controls, send state, inspector tabs, approval details, and settings.
3. Confirm names, roles, disabled states, selected tabs, and status/live-region
   changes are announced without duplicate or missing controls.
4. Repeat at the supported minimum window and 200% OS scaling. Record clipping,
   focus loss, hidden controls, and unreadable contrast as failures.
5. Close Pandora and the assistive technology, then verify no app process or
   temporary lifecycle sandbox remains.

VoiceOver and Orca evidence requires real macOS and Linux graphical sessions.
Windows signing, Apple signing, and Apple notarization require the project’s
release credentials. CI fails closed for a stable tag when those credentials
are absent.
