# Desktop accessibility and clean-machine evidence

Pandora keeps three different claims separate:

1. rendered webview accessibility and scaling checks;
2. packaged-app install, start, identity, update, rollback, and uninstall checks on fresh CI runners;
3. real native assistive-technology sessions on clean graphical machines.

Passing one layer never substitutes for another.

## Automated rendered evidence

Every desktop CI runner checks the Command Center with axe and retains screenshots
for the supported 1080x720 minimum window at these effective scales:

- 100%: 1080x720 viewport;
- 150%: 720x480 viewport;
- 200%: 540x360 viewport.

The suite covers right, bottom, and hidden inspector layouts. It also checks the
skip target, keyboard-only focus traversal, visible focus, forced colors,
increased contrast, reduced motion, and reduced transparency. The screenshots
and traces are retained in the `readiness-<platform>-<commit>` workflow
artifact for 90 days.

These checks detect shared webview regressions. They do not prove what NVDA,
VoiceOver, or Orca announces through a native packaged webview.

## Automated clean-runner evidence

The desktop CI matrix uses fresh hosted runners for Windows x64, Linux x64,
macOS Intel, and macOS ARM64. Each runner builds the exact commit and exercises:

- package registration or installation;
- bounded packaged-app startup;
- byte-for-byte identity between the source CLI and packaged sidecar;
- matching desktop and CLI versions;
- synthetic predecessor install, update, rollback, and uninstall;
- process-tree shutdown and lifecycle-sandbox cleanup.

Successful runs retain two JSON records alongside the rendered screenshots:

- `pandora-desktop-lifecycle-evidence.json`;
- `pandora-desktop-upgrade-evidence.json`.

Tagged releases also retain evidence for the downloaded, checksum-verified
published package. Lifecycle evidence contains digests and status only. It never
contains configuration, credentials, prompts, outputs, or user data.

## Historical Windows checkpoint

On 2026-08-30, the Windows x64 app built from commit `a39f063` exposed a named
UI Automation window and `RootWebArea`. The tree included the skip link,
navigation landmarks, labeled task input, Harness/provider/model controls,
run-inspector tabs, approval state, authority-chain controls, and disabled-state
metadata. Narrator was started while Pandora held document focus. The six
lifecycle-script tests and a real MSI extract, launch, process-tree stop, and
sandbox cleanup also passed locally.

This historical checkpoint proves native UI Automation discovery only. It does
not satisfy the Phase 6 NVDA spoken-order gate and is not treated as current
four-platform evidence.

## Native test matrix

One reviewed record is required for every advertised platform:

| Platform | Architecture | Required assistive technology |
|---|---|---|
| Windows | x86_64 | NVDA |
| Linux | x86_64 | Orca |
| macOS | x86_64 | VoiceOver |
| macOS | arm64 | VoiceOver |

Each session must use a packaged app on a clean graphical machine. Record the
exact commit, installer filename and SHA-256, desktop and CLI version, OS build,
assistive-technology version, tester, and UTC test time. Also record whether the
artifact was signed and, for macOS, notarized. Signing is mandatory at the
release-candidate gate, not inferred from accessibility evidence.

### Exact-commit native test packages

A successful `main` CI run retains one explicitly unsigned native test package
for each platform for 30 days. Artifact names use this exact form:

```text
native-test-package-<platform>-<40-character-commit>
```

Each artifact contains one packaged desktop installer, the same-commit CLI
sidecar, `native-test-package.json` with SHA-256 digests and release identity,
and `UNSIGNED-NATIVE-TEST-ONLY.txt`. Download only from a completely successful
CI run for the exact commit under review. Verify every recorded digest before
copying the package to the clean graphical test machine.

These packages exist only to make the native NVDA, VoiceOver, and Orca sessions
reproducible. They are not releases, are not signed release evidence, must not
be published, and do not satisfy any native accessibility check by themselves.
Release-candidate and stable publication continue to require the independent
vendor-signing gates in `RELEASES.md`.

## Native test protocol

1. Verify the installer SHA-256 and install it on a machine without a previous
   Pandora installation.
2. Confirm the packaged desktop version and bundled `pandora --version` are the
   same exact release identity.
3. Start NVDA, VoiceOver, or Orca before focusing Pandora.
4. Traverse from the skip link through navigation, task form, Harness and model
   controls, send state, inspector tabs, approval details, package trust state,
   and Settings.
5. Confirm names, roles, disabled states, selected tabs, dialog boundaries, and
   live status changes are announced once and in the expected order.
6. Repeat keyboard-only traversal and confirm focus is always visible and never
   trapped outside an active modal dialog.
7. Repeat at 100%, 150%, and 200% OS scaling, including the supported 1080x720
   minimum window. Record clipping, focus loss, hidden controls, and unreadable
   contrast as failures.
8. Exercise high contrast or increased contrast, forced colors where supported,
   reduced motion, and reduced transparency.
9. Exercise install, start, update, rollback, and uninstall. Confirm no Pandora
   process or lifecycle sandbox remains after shutdown.
10. Retain at least one screenshot, screen-reader notes, and lifecycle log for
    the session. Hash every retained file before review.

## Evidence directory contract

Place the four manifests at the root of one repository-relative evidence
directory:

```text
evidence/native-accessibility/
  windows-x64.json
  linux-x64.json
  macos-x64.json
  macos-arm64.json
  windows-x64/...
  linux-x64/...
  macos-x64/...
  macos-arm64/...
```

Every manifest is strict: unknown or missing fields fail validation. It must
bind the session to one 40-character commit SHA, one artifact SHA-256, the exact
platform and assistive technology, matching desktop/CLI versions, all required
checks, and checksummed evidence files. Paths are repository-relative; absolute
paths, traversal, symlinks, duplicate assets, empty files, and checksum changes
are rejected.

The `checks` object must contain `"pass"` for landmarks, controls, forms, status
changes, dialogs, keyboard order, visible focus, scaling, high contrast, forced
colors, reduced motion, reduced transparency, minimum window, install, start,
update, rollback, and uninstall. Findings may be recorded, but a critical
finding must be resolved before admission.

Validate a completed evidence set locally with:

```bash
python scripts/accessibility_evidence.py \
  --root . \
  --directory evidence/native-accessibility \
  --commit <exact-40-character-commit> \
  --output native-accessibility-index.json
```

Then dispatch the **Native accessibility evidence** workflow on that exact
commit. The workflow validates all four records together and retains the source
evidence plus the generated index for 90 days.

## Exit gate

Phase 6 closes only when both conditions are true for the same exact commit:

- the four-platform desktop CI matrix has retained rendered and clean-runner
  lifecycle evidence; and
- the native evidence workflow has accepted all four NVDA, VoiceOver, and Orca
  records with no unresolved critical finding.

Real VoiceOver and Orca evidence requires macOS and Linux graphical sessions.
Hosted CI cannot honestly manufacture spoken-order evidence, so missing native
records fail closed rather than being replaced by Chromium results.
