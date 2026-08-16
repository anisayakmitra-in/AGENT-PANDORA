# Memory

## Shipped

`pandora-runtime::MemoryEngine` provides three bounded memory tiers:

- `L0` is an expiring in-memory trace ring buffer with a fixed capacity.
- `L1` stores redacted execution summaries, decisions, failures, benchmarks, and provenance.
- `L2` stores only an `L1` candidate promoted with an explicit approval record. Promoted records retain provenance and approval identity.

Every record is scoped to a tenant, workspace, session, and provider. Recall and revocation require the exact same scope, preventing cross-workspace, cross-session, and cross-provider reuse.

Memory stores summaries rather than transcripts. Secret-classified content is rejected, and the engine has no API for persisting hidden reasoning or raw credentials. `forget` adds a revocation audit entry; revoked records are excluded from recall.

## Boundary

The current engine is an in-memory runtime component. The CLI also keeps a bounded, private L1 execution-evidence ledger in the session store. Each entry records only the execution ID, selected Harness and Gene, terminal status, provider scope, timestamp, and provenance; it never stores a task, output, transcript, credential, or hidden reasoning. `session inspect` exposes its count, not its contents. Agent runs retrieve at most eight canonical entries from the exact same tenant, workspace, session, and provider scope. They enter context as non-cacheable descriptive history, never as instructions or authority.

Its approval object is an explicit memory contract; it does not replace Parliament approval or provide execution authority. Memory records do not grant permissions, activate packages, or execute tools.
