# WebMCP Challenge submission notes

## One-line pitch

Pandora Permit Room lets a browser agent prepare high-value verification work while keeping the irreversible decision in a visible, one-shot human gate.

## The problem

Browser agents are useful precisely because they can act, but product interfaces often collapse “can prepare” and “may authorize” into the same click path. Pandora separates those capabilities. The agent receives typed read, plan, request, and evidence tools. The person retains the only permit controls.

## What is real

- Four imperative tools register through document.modelContext.registerTool on the top-level page.
- Tool schemas are narrow, bounded, reject extra properties, and label read-only operations.
- Tool execution honors browser-provided cancellation signals before mutation.
- Agent calls and human controls operate on the same external React store.
- Plan and request IDs use Web Crypto SHA-256; the exact request digest is visible and copyable.
- Allowing consumes the pending request once and creates a receipt. Denying creates a no-effect receipt.
- Release evidence is derived from current page state, including the observed WebMCP tool count; unavailable tools produce a failed check.
- Browsers without WebMCP retain a complete manual fallback without pretending tools are live.

This is a client-side challenge demonstration. Verification checks inspect declared/live page state; no backend command execution or durable database is represented.

## Judge walkthrough (about 3 minutes)

### 0:00 — Show the boundary

Open the Permit Room. Point out **4 site tools live**, the locked one-shot permit seal, and the empty evidence ledger.

### 0:25 — Let the agent prepare

Ask the browser agent:

> Inspect this change, draft a balanced plan, and request the release verification suite.

The agent reads structured context, drafts a bounded plan, then creates a SHA-256-bound request. The plan and request appear immediately in the same page the person is viewing.

### 1:15 — Prove the agent lacks authority

Show the discovered tool list. There is no approve, allow, deny, or issue-permit tool. The ledger is still empty and the permit gate says **Pending review**.

### 1:40 — Exercise the human gate

Choose **Allow once**. The seal becomes **SPENT**, the request disappears, and a receipt records each live check. Reset and repeat with **Deny** if time permits to show that no checks run and no permit is issued.

### 2:20 — Close the loop with evidence

Ask the agent to read verification evidence. It can explain the receipt but cannot reuse it. A second run requires a new exact request and a new human decision.

## Challenge rubric mapping

- **Usefulness:** gives real browser-agent products a legible approval primitive for high-impact work.
- **Originality:** treats WebMCP as a capability boundary and shared control surface, not just a faster form-filling API.
- **Execution:** production TypeScript build, deterministic tests, responsive accessible UI, cancellation, secure headers, and deploy manifests.
- **Thoughtful WebMCP:** imperative top-level tools with narrow schemas, explicit side effects, observable page state, read-only annotations, and normal UI fallbacks.
- **Human-agent experience:** both actors see one state machine; authority is visually unmistakable and receipts are inspectable.

## Submission checklist

Engineering complete in repository:

- [x] Real top-level imperative WebMCP registration
- [x] Four discoverable site tools
- [x] Human-only approval boundary
- [x] Cancellation and registration lifecycle coverage
- [x] Responsive fallback UI
- [x] Netlify production configuration
- [x] Test and production build commands
- [x] Honest limitations documented

Account-bound actions for the submitter:

- [ ] Push this app to the public challenge repository
- [ ] Connect the repository to Netlify and deploy apps/pandora-webmcp
- [ ] Verify response headers on the public URL
- [ ] Test tool discovery and the full walkthrough in ChatGPT's built-in browser
- [ ] Record and upload the demo video
- [ ] Complete the challenge registration/submission form with live app, source, and video links
