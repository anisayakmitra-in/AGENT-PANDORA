# Review and observability

## Human review

Pandora supports three review modes:

- `HumanInTheLoop` pauses until a reviewer approves or rejects the subject.
- `HumanOnTheLoop` escalates low-confidence work and otherwise records the automatic decision.
- `HumanOutOfTheLoop` permits reversible, high-confidence work without a blocking review.

Irreversible actions remain pending until explicitly approved. Each review exposes a bounded summary and the exact operation digest, supports asynchronous resolution, expires at its deadline, and produces an audit receipt. A review decision does not mint an effect permit.

## Observability

`ObservabilityEngine` projects the canonical runtime events into ordered trace and span views. Samples can contain correlation IDs, sequence numbers, timestamps, token counts, cost, latency, error codes, and an optional drift score. `pandora session inspect` currently projects timestamped persisted events and reports trace, span, failure, and reliability counts. It reports reliability as unavailable when a session has no timestamped events, and does not invent token, cost, latency, or drift measurements when they were not recorded.

Telemetry has no raw prompt or output field. Debug exports must be redacted before they enter the projection. Runtime events remain the authoritative event stream; observability is a derived view and cannot authorize execution, approve memory promotion, or change policy.

## Execution profiles

Pandora assembles a versioned `ExecutionProfile` before an effect request reaches Parliament. The profile records the Pandora version, operating system, architecture, policy version, a digest of the canonical workspace root, the shipped containment snapshot digest, and sorted bindings for the selected executor and runtime components. Coding runs bind their Harness and Gene; MCP runs bind the server configuration or immutable catalog revision; provider calls bind the provider and model.

The raw workspace path, credentials, prompts, context, model output, Tool output, environment values, and hidden reasoning are not stored in the profile. Deserialization recomputes the profile digest and rejects modified evidence.

Operation-request protocol v2 includes the profile digest in its canonical request digest. The existing one-shot permit and effect receipt already bind that request digest, so substituting a profile invalidates the permit. Profiles remain evidence only: they cannot approve an operation, mint a permit, or weaken executor checks.

## Rollout evidence

`RolloutReducer` creates a deterministic, versioned projection from one context-manifest digest, ordered runtime events, and linked effect receipts. The projection binds tenant, workspace, session, and execution scope; chains each record to the previous digest; and can be replay-verified without creating another event store.

Rollout records keep typed event kinds, bounded identifiers and failure codes, request digests, receipt and permit IDs, policy versions, provider IDs, MCP era evidence, timestamps, and effect outcomes. Policy and denial reasons are represented only by SHA-256 digests. Credentials, prompts, assembled context, model output, Tool output, environment values, arbitrary error text, and hidden reasoning are excluded.

The reducer is evaluation evidence only. It cannot issue a permit, approve an operation, promote memory, modify canonical events, or replace the session store. Callers may inspect the projection version, record count, and final digest from the in-memory result; Pandora does not claim durable rollout storage until the existing session authority explicitly persists that projection.

## Containment evidence

`pandora doctor --json` reports a versioned containment snapshot for the shipped filesystem, process, Git worktree, provider, and MCP executors. Each entry has a stable executor identity, implementation version, worker class, deterministic digest, enforced controls, and fixed limitation codes.

The status values are deliberately narrow:

- `partial` means the executor applies the listed controls but does not isolate the whole boundary.
- `unavailable` means Pandora provides no containment for that boundary and names the limitation.
- `enforced` is reserved for a boundary the executor fully blocks; the current snapshot makes no such claim.

This snapshot is inspection evidence, not authority. It cannot mint a permit or weaken Parliament, ReferenceMonitor, or EffectExecutor checks. The process, Git worktree, and local MCP executors run trusted native child programs without an operating-system sandbox or network isolation. Provider execution occurs outside Pandora's local boundary.

## Evaluation

Each CLI execution attempt produces one bounded evaluation receipt. The receipt records
the session, execution, timestamp, trajectory result, and policy result. A run
that stops for approval also records a human-review requirement. Pandora stores
the receipt and the execution's canonical events in one SQLite transaction.

The receipt contains no task, model output, Tool output, credential, or hidden
reasoning. Outcome evaluation remains unavailable unless a caller supplies an
explicit expected result; Pandora does not treat a completed process as proof
that the task outcome was correct. Evaluation results are evidence only and
cannot authorize an effect.

After the receipt is stored, CLI and service runs reduce failed non-advisory
evaluation kinds into a scoped `L1` lesson. The lesson omits evaluator reasons
and execution content. The next agent run may receive it as non-cacheable,
descriptive context under the existing tenant, workspace, session, and provider
scope. CLI results report whether the lesson was recorded. A failed feedback
write does not alter the canonical evaluation receipt or grant authority.
