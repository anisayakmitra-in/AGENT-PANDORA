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

## Boundary

`MemoryEngine::new` remains an in-process implementation for isolated runtime
use. `MemoryEngine::open` binds L1 and L2 to the existing `sessions.sqlite3`
authority while keeping L0 in RAM. Canonical execution evidence records only
the execution ID, selected Harness and Gene, terminal status, provider scope,
timestamp, and provenance; it never stores a task, output, transcript,
credential, or hidden reasoning. Agent runs retrieve at most eight canonical
entries from the exact same tenant, workspace, session, and provider scope.
They enter context as non-cacheable descriptive history, never as instructions
or authority.

The public CLI does not expose memory summaries. `session inspect` reports the
bounded L1 execution-evidence count without returning record content. Durable
promotion, revocation, and compaction remain runtime APIs; production surfaces
must bind those state changes to Pandora's approval, permit, receipt, and event
authority.

Its approval object is an explicit memory contract; it does not replace Parliament approval or provide execution authority. Memory records do not grant permissions, activate packages, or execute tools.

## Context assembly cache

`ContextEngine` keeps up to 64 assemblies in the current process. Each entry
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
The cache does not store model responses, Tool output, Skill guidance, or L1
evidence. It saves local context assembly work; it is not provider prompt
caching, semantic caching, or evidence of reduced token billing.
