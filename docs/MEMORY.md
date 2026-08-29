# Memory

## Shipped

`pandora-runtime::MemoryEngine` provides three bounded memory tiers:

- `L0` is an expiring in-memory trace ring buffer with a fixed capacity.
- `L1` durably stores bounded execution summaries, decisions, failures,
  benchmarks, and provenance in the scoped session database.
- `L2` durably stores only an `L1` lesson or lineage candidate promoted with
  an explicit approval record. Promoted records retain provenance and approval
  identity.

Every record is scoped to a tenant, workspace, session, and provider. Recall and revocation require the exact same scope, preventing cross-workspace, cross-session, and cross-provider reuse.

Memory stores summaries rather than transcripts. Production writers emit fixed,
redacted summaries; other runtime callers must redact summaries and provenance
before storage. Secret-classified records are rejected. Raw credentials and
hidden reasoning are outside the memory contract. `forget` adds a durable
revocation tombstone and audit entry;
revoked records are excluded from recall and cannot be reinserted after record
compaction. Each scope retains at most 256 records per durable tier, and each
recall returns at most 256 records. A scope also has a lifetime ceiling of
4,096 memory identities so revocation tombstones and audit history stay
bounded after repeated compaction.

Compaction removes revoked logical records while retaining tombstones and audit
entries. It is not a secure-erasure guarantee for SQLite database pages, WAL
files, backups, or storage snapshots.

The session schema migrates existing canonical L1 execution evidence into the
durable memory tables. Migration preserves IDs, provider scope, timestamps,
summaries, and provenance. Invalid rows, invalid limits, and scope mismatches
fail closed.

## Verified synthesis

`MemoryEngine::synthesis_snapshot` creates a deterministic, scope-bound view of
up to 16 eligible L1 records. Sensitive records are excluded, evidence is
sorted by stable identity, and the snapshot exposes a digest without exposing
raw content through the digest itself. `propose_synthesis` accepts a bounded,
caller-supplied redacted summary and cites every source record by ID.

`verify_synthesis` re-reads the same scope before a candidate is committed.
`commit_synthesis` performs that check while holding the engine's mutation gate,
then stores the candidate as a synthesized L1 record with its evidence IDs and
snapshot provenance. Any changed, revoked, expired, or missing source returns
`SynthesisStale`; an empty source set cannot produce a candidate. Synthesis does
not write L2, approve memory, alter policy, or grant effect authority. L2
promotion still requires the existing explicit approval path.

## Boundary

`MemoryEngine::new` remains an in-process implementation for isolated runtime
use. `MemoryEngine::open` binds L1 and L2 to the existing `sessions.sqlite3`
authority while keeping L0 in RAM. Canonical execution evidence records only
the execution ID, selected Harness and Gene, terminal status, provider scope,
timestamp, and provenance; it never stores a task, output, transcript,
credential, or hidden reasoning. A failed non-advisory evaluation may add one
canonical `L1` lesson containing only the failed evaluation kinds. Evaluator
reasons, task text, output, and credentials are excluded. Agent runs retrieve
at most eight combined execution-evidence and evaluation-lesson records from
the exact same tenant, workspace, session, and provider scope. Arbitrary
`Lesson` records do not enter agent context. Retrieved records are
non-cacheable descriptive history, never instructions or authority.

The public CLI exposes scoped L1/L2 summaries through `memory recall` and
`memory audit`. `memory forget` requires explicit confirmation before durable
revocation, and `memory promote` requires an exact approval resolved through the
existing approval store. L0 remains process-local and is not exposed as a
durable record. These commands do not create a second memory store or bypass
Pandora's approval, permit, receipt, and event authority.

`memory consolidate` is the explicit cross-session boundary. It copies one
non-sensitive L1 record only when source and target share the exact tenant,
workspace, and provider scope; it requires an explicit `--yes` write and gives
the target a new identity with a hashed source-provenance reference. Cross-
workspace, cross-provider, sensitive, L2, and automatic global consolidation
remain denied by policy.

Its approval object is an explicit memory contract; it does not replace Parliament approval or provide execution authority. Memory records do not grant permissions, activate packages, or execute tools.

## Context assembly cache

`ContextEngine` keeps up to 64 assemblies in the current process. Agent runs
also use the bounded atomic `context-cache.json` file in the configured data
directory, so eligible assemblies can survive a process restart. Each entry
must fit a 64 KiB retained-size budget. Only public or internal constitutional
and active-plan fragments are eligible. An entry matches only when its tenant,
workspace, session, provider, model, policy, projection version, token budget,
fragment contents, provenance, metadata, and expiry data match. An entry is
discarded before reuse when its context has expired or time moves backward.

Every assembly records a versioned manifest digest over the ordered fragments,
their exact content digests, trust and classification metadata, and bounded
origin references. Receipts report whether provenance is complete and whether
the operation was a cache `hit`, eligible `miss`, or policy `bypass`. A fragment
without complete origin evidence is assembled normally but cannot enter the
cache.

Sensitive, secret, retrieved, conversation, and L1 fragments bypass the cache.
The persistent cache ignores corrupt, oversized, stale, or scope-mismatched
records and never becomes an execution authority. It does not store model
responses, Tool output, Skill guidance, or L1 evidence. It saves local context
assembly work; it is not provider prompt caching, semantic caching, or evidence
of reduced token billing.
