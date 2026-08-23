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

## Rollout evidence

`RolloutReducer` creates a deterministic, versioned projection from one context-manifest digest, ordered runtime events, and linked effect receipts. The projection binds tenant, workspace, session, and execution scope; chains each record to the previous digest; and can be replay-verified without creating another event store.

Rollout records keep typed event kinds, bounded identifiers and failure codes, request digests, receipt and permit IDs, policy versions, provider IDs, MCP era evidence, timestamps, and effect outcomes. Policy and denial reasons are represented only by SHA-256 digests. Credentials, prompts, assembled context, model output, Tool output, environment values, arbitrary error text, and hidden reasoning are excluded.

The reducer is evaluation evidence only. It cannot issue a permit, approve an operation, promote memory, modify canonical events, or replace the session store. Callers may inspect the projection version, record count, and final digest from the in-memory result; Pandora does not claim durable rollout storage until the existing session authority explicitly persists that projection.

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
