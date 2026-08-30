# Pandora Permit Room

Pandora Permit Room is a browser-native WebMCP control room for a hard problem in agentic software: an agent should be able to inspect, plan, and request work without inheriting the person's authority to approve it.

The app registers four real imperative site tools on the top-level document. Agent and human share the same React state, exact requests are bound with Web Crypto SHA-256, and every human decision produces visible evidence. **Allow once** and **Deny** deliberately remain ordinary page controls—not agent tools.

## Why this needs WebMCP

Without site tools, a browser agent must infer intent from DOM text and clicks. Pandora exposes a narrow typed contract instead:

| Site tool | Mode | Observable effect |
| --- | --- | --- |
| pandora_read_control_room | Read | Returns the declared change, active plan, pending request, and receipt count. |
| pandora_draft_verification_plan | Write | Drafts a bounded plan in shared page state. Runs nothing. |
| pandora_request_verification | Write | Creates one digest-bound request for visible human review. Runs nothing. |
| pandora_read_verification_evidence | Read | Returns receipts already recorded in the evidence ledger. |

Every schema rejects unknown fields, every string is length-bounded, tool calls honor their AbortSignal, and registration itself is disposed through an abort controller.

## Authority model

    Browser agent         Shared control room          Human permit gate
    read context  ──────► inspect visible state
    draft plan    ──────► plan appears on page
    request run   ──────► exact SHA-256 request ─────► Allow once / Deny
    read evidence ◄────── immutable receipt ◄──────── permit spent / not issued

There is no approval tool. A consumed request is no longer pending, so it cannot be replayed. This frontend demonstrates the interaction contract locally; it does not claim to execute repository commands or persist receipts to a backend.

## Run and verify

Requires Node.js 22+ and pnpm 10.

    pnpm install --frozen-lockfile
    pnpm dev

    pnpm verify
    # equivalent to:
    pnpm test
    pnpm build

The normal UI remains fully usable when the browser does not expose document.modelContext; the status readout and site-tool inspector make that fallback explicit.

## Demo in a WebMCP-capable browser

1. Open the production URL in ChatGPT's built-in browser or another supported WebMCP environment.
2. Confirm that the page reports **4 site tools live**.
3. Ask: “Inspect this change, draft a balanced plan, and request the release verification suite.”
4. Observe that the agent changes the shared page state but creates no receipt.
5. Use the visible **Allow once** or **Deny** control yourself.
6. Ask the agent to read the evidence and explain why the permit cannot be replayed.

See [HACKATHON.md](./HACKATHON.md) for the judge walkthrough and submission checklist.

## Deploy to Netlify

The repository includes netlify.toml, public/_headers, and public/_redirects.

- Base directory: apps/pandora-webmcp
- Build command: pnpm build
- Publish directory: dist

Netlify should discover those values automatically when this directory is selected. The production manifest sets the tools permissions policy, origin agent clustering, and a restrictive CSP. vercel.json remains available as an alternate static-host configuration.

## Project map

- src/webmcp.ts — imperative tool contracts, validation, cancellation, registration lifecycle
- src/controlRoom.ts — deterministic state machine, SHA-256 binding, one-shot receipts
- src/App.tsx — shared human/agent control room and explicit authority boundary
- src/*.test.ts — registration, cancellation, request, denial, replay, and evidence tests
- netlify.toml — production build and response headers
