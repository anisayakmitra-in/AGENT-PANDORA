import { useState, useSyncExternalStore, type FormEvent, type ReactNode } from "react";
import type {
  ControlRoomStore,
  RiskBudget,
  VerificationReceipt,
  VerificationSuite,
} from "./model";
import { PANDORA_SITE_TOOLS } from "./webmcp";

type IconName = "arrow" | "check" | "copy" | "lock" | "spark" | "x";
type PermitState = "locked" | "review" | "spent" | "denied";

const DEMO_PROMPT =
  "Inspect this change, draft a balanced plan, and request the release verification suite.";

function Icon({ name, size = 16 }: { name: IconName; size?: number }) {
  const paths: Record<IconName, ReactNode> = {
    arrow: <path d="M5 12h14m-5-5 5 5-5 5" />,
    check: <path d="m5 12 4 4L19 6" />,
    copy: <><rect x="8" y="8" width="11" height="11" rx="2" /><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2" /></>,
    lock: <><rect x="5" y="10" width="14" height="10" rx="2" /><path d="M8 10V7a4 4 0 0 1 8 0v3" /></>,
    spark: <path d="m12 3 1.4 4.1L17.5 8.5l-4.1 1.4L12 14l-1.4-4.1-4.1-1.4 4.1-1.4L12 3Zm6 10 .8 2.2L21 16l-2.2.8L18 19l-.8-2.2L15 16l2.2-.8L18 13Z" />,
    x: <path d="m6 6 12 12M18 6 6 18" />,
  };
  return <svg aria-hidden="true" viewBox="0 0 24 24" width={size} height={size} fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">{paths[name]}</svg>;
}

function Badge({ children, tone = "neutral" }: { children: ReactNode; tone?: "neutral" | "blue" | "green" | "amber" }) {
  return <span className={`status-badge tone-${tone}`}>{children}</span>;
}

function copyText(value: string) {
  void navigator.clipboard?.writeText(value);
}

function ReceiptCard({ receipt }: { receipt: VerificationReceipt }) {
  const passed = receipt.checks.filter((check) => check.status === "passed").length;
  return <article className="receipt" data-decision={receipt.decision}>
    <div className="receipt-top">
      <div>
        <p className="machine-label">Immutable receipt</p>
        <h3>{receipt.decision === "allowed" ? `${passed}/${receipt.checks.length} checks passed` : "Request denied"}</h3>
      </div>
      <strong className="receipt-outcome">{receipt.permit === "spent" ? "Permit spent" : "Permit not issued"}</strong>
    </div>
    {receipt.checks.length ? <div className="check-grid">{receipt.checks.map((check) => <div className="check-item" key={check.name}>
      <span className={`check-mark check-${check.status}`}><Icon name={check.status === "passed" ? "check" : "x"} size={12} /></span>
      <div><strong>{check.name}</strong><p>{check.detail}</p></div>
    </div>)}</div> : <p className="receipt-note">The request stopped at the human boundary. Nothing executed.</p>}
    <button className="digest receipt-digest" type="button" onClick={() => copyText(receipt.digest)}>
      <span>{receipt.id} / {receipt.digest}</span><Icon name="copy" size={13} />
    </button>
  </article>;
}

export function App({ store }: { store: ControlRoomStore }) {
  const state = useSyncExternalStore(store.subscribe, store.getSnapshot);
  const [riskBudget, setRiskBudget] = useState<RiskBudget>("balanced");
  const [suite, setSuite] = useState<VerificationSuite>("release");
  const [error, setError] = useState("");
  const [working, setWorking] = useState(false);

  const draft = async (event: FormEvent) => {
    event.preventDefault();
    setError("");
    setWorking(true);
    try {
      await store.draftPlan({ objective: state.objective, riskBudget, actor: "human" });
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Pandora could not draft the plan.");
    } finally {
      setWorking(false);
    }
  };

  const requestRun = async () => {
    if (!state.plan) return;
    setError("");
    setWorking(true);
    try {
      await store.requestVerification({ planId: state.plan.id, suite, actor: "human" });
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Pandora could not create the request.");
    } finally {
      setWorking(false);
    }
  };

  const decide = (allow: boolean) => {
    if (!state.pendingRequest) return;
    setError("");
    try {
      store.decide(state.pendingRequest.id, allow);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "The request is no longer pending.");
    }
  };

  const latestReceipt = state.receipts[0];
  const permitState: PermitState = state.pendingRequest
    ? "review"
    : latestReceipt?.decision === "allowed"
      ? "spent"
      : latestReceipt?.decision === "denied"
        ? "denied"
        : "locked";
  const permitCopy: Record<PermitState, { seal: string; note: string; badge: string; label: string }> = {
    locked: { seal: "LOCKED", note: "No authority issued", badge: "Locked", label: "One-shot permit is locked" },
    review: { seal: "REVIEW", note: "Human decision", badge: "Pending review", label: "One-shot permit awaits human review" },
    spent: { seal: "SPENT", note: "Cannot replay", badge: "Spent", label: "One-shot permit was spent" },
    denied: { seal: "LOCKED", note: "Request denied", badge: "Denied", label: "Request denied; no permit issued" },
  };
  const webMcpLabel = state.webMcp === "available" ? state.registeredToolCount + " site tools live" : state.webMcp === "checking" ? "Checking site tools" : "Human demo mode";

  return <>
    <a className="skip-link" href="#pipeline">Skip to permit pipeline</a>
    <div className="app-shell">
      <header className="topbar">
        <a className="brand" href="#top" aria-label="Pandora Permit Room home"><span className="brand-mark" aria-hidden="true">P</span><span><strong>Pandora</strong> / Permit Room</span></a>
        <div className="room-status"><span className={`status-dot status-${state.webMcp}`} />{webMcpLabel} / Local page state</div>
        <div className="authority-readout"><span>Authority</span><strong>{permitCopy[permitState].badge}</strong></div>
      </header>

      <main id="top">
        <section className="intro">
          <div className="intro-copy">
            <p className="meta-label">WebMCP control surface / Change verification</p>
            <h1>Agents can prepare. Only you can permit.</h1>
            <p>Share one live control room with your browser agent. It can inspect, plan, and request verification without crossing the human approval boundary.</p>
          </div>
          <div className="seal-wrap">
            <div className="permit-seal" data-state={permitState} aria-live="polite" aria-label={permitCopy[permitState].label}>
              <span className="seal-notch" aria-hidden="true" />
              <span className="seal-scan" aria-hidden="true" />
              <span className="seal-cancel" aria-hidden="true" />
              <div className="seal-content"><span>One-shot permit</span><strong>{permitCopy[permitState].seal}</strong><small>{permitCopy[permitState].note}</small></div>
            </div>
          </div>
        </section>

        <section className="pipeline" id="pipeline" aria-label="Pandora verification workspace">
          <div className="pipeline-rail" aria-hidden="true"><span className="rail-node" data-stage="01" /><span className="rail-node" data-stage="02" /><span className="rail-node" data-stage="03" /></div>
          <div className="pipeline-grid">
            <article className="stage context-stage">
              <div className="stage-head"><div><p className="stage-kicker">Change context</p><h2>{state.context.project}</h2></div><Badge tone="blue">{state.context.changedFiles.length} files</Badge></div>
              <p className="context-summary">{state.context.summary}</p>
              <div className="fact-row"><span>Branch</span><code>{state.context.branch}</code></div>
              <div className="fact-row"><span>Scope</span><code>WebMCP challenge surface</code></div>
              <div className="file-list" aria-label="Changed files">{state.context.changedFiles.slice(0, 4).map((file) => <div className="file-row" key={file}><span className="file-status">M</span><code>{file}</code></div>)}{state.context.changedFiles.length > 4 ? <p className="file-overflow">+ {state.context.changedFiles.length - 4} documentation file</p> : null}</div>
              <div className="boundary-note"><strong>Authority paths protected</strong><p>ReferenceMonitor, Parliament, and Shadow Council are outside this change.</p></div>
            </article>

            <article className="stage plan-stage">
              <div className="stage-head"><div><p className="stage-kicker">Verification plan</p><h2>Prepare the request</h2></div></div>
              <form onSubmit={draft} className="plan-form">
                <label className="field"><span className="field-label">Objective</span><textarea value={state.objective} onChange={(event) => store.setObjective(event.target.value)} rows={4} maxLength={480} disabled={working} /></label>
                <div className="plan-actions"><label className="field"><span className="field-label">Risk budget</span><select value={riskBudget} onChange={(event) => setRiskBudget(event.target.value as RiskBudget)} disabled={working}><option value="strict">Strict</option><option value="balanced">Balanced</option><option value="expansive">Expansive</option></select></label><button className="button button-ink" type="submit" disabled={working}>{working ? "Binding…" : "Draft plan"} <Icon name="arrow" size={15} /></button></div>
              </form>
              {state.plan ? <div className="plan-result" aria-live="polite">
                <div className="plan-meta"><Badge tone="green">Drafted by {state.plan.createdBy}</Badge><code>{state.plan.id}</code></div>
                <ol className="plan-steps">{state.plan.steps.map((step) => <li key={step}>{step}</li>)}</ol>
                <div className="request-actions"><label className="field"><span className="field-label">Suite</span><select value={suite} onChange={(event) => setSuite(event.target.value as VerificationSuite)} disabled={working}><option value="policy">Policy only</option><option value="targeted">Targeted</option><option value="release">Release verification</option></select></label><button className="button button-accent" type="button" onClick={requestRun} disabled={working || Boolean(state.pendingRequest)}>{working ? "Binding…" : "Request run"} <Icon name="arrow" size={15} /></button></div>
              </div> : <div className="empty-plan"><p>Draft a plan here, or ask your browser agent to prepare one with the current page context.</p></div>}
              {error ? <p className="error-message" role="alert">{error}</p> : null}
            </article>

            <article className="stage permit-stage" data-state={permitState}>
              <div className="stage-head"><div><p className="stage-kicker">Permit gate</p><h2>Human authority</h2></div><span className="state-badge">{permitCopy[permitState].badge}</span></div>
              {state.pendingRequest ? <div className="gate-copy">
                <div className="gate-status">Exact request awaiting you</div><h3>Allow this once?</h3><p className="gate-summary">{state.pendingRequest.summary} against the current local page state.</p>
                <div className="gate-facts"><div className="fact-row"><span>Requested by</span><strong>{state.pendingRequest.requestedBy}</strong></div><div className="fact-row"><span>Effect</span><strong>Evaluate live page state</strong></div><div className="fact-row"><span>Replay</span><strong>Blocked after use</strong></div></div>
                <button className="digest" type="button" onClick={() => copyText(state.pendingRequest!.digest)}><span>{state.pendingRequest.digest}</span><Icon name="copy" size={13} /></button>
                <div className="decision-row"><button className="button button-deny" type="button" onClick={() => decide(false)}>Deny</button><button className="button button-allow" type="button" onClick={() => decide(true)}>Allow once <Icon name="check" size={15} /></button></div>
                <p className="human-only"><strong>Human only.</strong> Allow and deny are intentionally not exposed as WebMCP tools.</p>
              </div> : latestReceipt?.decision === "allowed" ? <div className="gate-copy">
                <div className="gate-status">Permit consumed</div><h3>Allowed once. Now spent.</h3><p className="gate-summary">The {latestReceipt.suite} verification suite completed under one exact permit. Replay is blocked.</p>
                <div className="gate-facts"><div className="fact-row"><span>Decision</span><strong>Allowed by human</strong></div><div className="fact-row"><span>Permit</span><strong>Spent</strong></div><div className="fact-row"><span>Evidence</span><strong>{latestReceipt.checks.length} checks recorded</strong></div></div>
                <div className="gate-rule"><strong>Cancellation mark applied.</strong><br />A new request requires a new human decision.</div>
              </div> : latestReceipt?.decision === "denied" ? <div className="gate-copy">
                <div className="gate-status">Request denied</div><h3>Nothing executed.</h3><p className="gate-summary">The request stopped at the human boundary. No permit was issued, and no check was run.</p>
                <div className="gate-facts"><div className="fact-row"><span>Decision</span><strong>Denied by human</strong></div><div className="fact-row"><span>Permit</span><strong>Not issued</strong></div><div className="fact-row"><span>Replay</span><strong>Request closed</strong></div></div>
                <div className="gate-rule"><strong>Reference Monitor remains locked.</strong><br />The browser agent has no approval path.</div>
              </div> : <div className="gate-copy">
                <div className="gate-status">Reference Monitor</div><h3>No request pending</h3><p className="gate-summary">An agent can ask for verification. It cannot approve its own request or borrow a previous permit.</p>
                <div className="gate-rule"><strong>Plan → Request → You</strong><br />Authority exists only at this final human-facing gate.</div>
              </div>}
            </article>
          </div>
        </section>

        <section className="lower-grid">
          <div className="evidence">
            <div className="section-head"><div><p className="stage-kicker">Evidence ledger</p><h2>Nothing happens quietly.</h2></div><span className="receipt-count">{state.receipts.length} {state.receipts.length === 1 ? "receipt" : "receipts"}</span></div>
            {state.receipts.length ? <div className="receipts">{state.receipts.map((receipt) => <ReceiptCard receipt={receipt} key={receipt.id} />)}</div> : <div className="empty-receipt"><p>Allow or deny one exact request to create an immutable receipt.</p></div>}
          </div>
          <aside className="activity">
            <div className="section-head"><div><p className="stage-kicker">Shared activity</p><h2>Human + agent</h2></div><button className="reset-button" type="button" onClick={store.reset}>Reset room</button></div>
            <div className="activity-list" aria-live="polite">{state.activity.slice(0, 6).map((item) => <div className="activity-item" key={item.id}><span className={`actor actor-${item.actor}`} aria-hidden="true">{item.actor.slice(0, 1).toUpperCase()}</span><div><strong>{item.actor}</strong><p>{item.message}</p></div></div>)}</div>
            <div className="site-tools-panel">
              <div className="site-tools-head"><div><p className="machine-label">Top-level imperative API</p><strong>Live site tools</strong></div><Badge tone={state.webMcp === "available" ? "green" : "amber"}>{state.webMcp === "available" ? state.registeredToolCount + "/" + PANDORA_SITE_TOOLS.length + " registered" : "Demo fallback"}</Badge></div>
              <div className="site-tool-list">{PANDORA_SITE_TOOLS.map((tool) => <div className="site-tool-row" key={tool.name}><span className="site-tool-mode" data-mode={tool.mode}>{tool.mode}</span><div><strong>{tool.label}</strong><code>{tool.name}</code></div></div>)}</div>
              <p className="site-tools-note">{state.webMcp === "available" ? "Discovered through document.modelContext on this top-level page." : "This browser did not expose WebMCP. Every human control remains usable for the fallback demo."}</p>
            </div>
            <div className="agent-note"><Icon name="spark" size={17} /><div><strong>Try it with an agent</strong><p>Ask: “{DEMO_PROMPT}”</p></div><button className="prompt-copy" type="button" onClick={() => copyText(DEMO_PROMPT)} aria-label="Copy the suggested agent prompt"><Icon name="copy" size={14} /></button></div>
          </aside>
        </section>
      </main>

      <footer><strong>Planning is assistance. Authority remains human.</strong><span>Pandora Agent / WebMCP challenge 2026</span><a href="https://github.com/anisayakmitra-in/AGENT-PANDORA">Source ↗</a></footer>
    </div>
  </>;
}
