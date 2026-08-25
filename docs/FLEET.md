# Fleet control plane

`FleetEngine` is Pandora's local durable control plane for worker identity and
allocation. It stores node records and lease state in a scoped SQLite database.
The first implementation is local-only; it does not open sockets, pair remote
machines, negotiate TLS, or execute work itself.

Each node has a stable operator-provided ID, implementation version, worker
class, sorted capability list, and state. Capability dispatch is deterministic:
the ready node with the lexicographically smallest ID is selected. Nodes can be
quarantined, revoked, or killed. Active leases are transitioned in the same
SQLite transaction as those node controls, so a stopped node cannot retain an
active lease.

Leases bind an operator-provided lease ID to a node, execution identity, expiry,
and bounded token/tool/duration/cost budget. Expiry is explicit and can be
reaped. A lease is scheduling evidence only. It is not an `EffectPermit`, does
not authorize an operation, and cannot bypass Parliament, the ReferenceMonitor,
one-shot permit consumption, or the EffectExecutor.

The store caps nodes at 256, leases at 4,096, and capabilities per node at 64.
Unknown states, malformed records, duplicate identities, unavailable nodes, and
zero-duration leases fail closed. Remote Fleet preview remains a separate
future transport boundary requiring authenticated pairing, TLS, replay
protection, cancellation, reconnect, and remote containment evidence.
