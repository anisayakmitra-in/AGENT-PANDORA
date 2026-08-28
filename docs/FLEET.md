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
and bounded token/tool/duration/cost budget. A live worker can renew its lease
only with the matching execution identity. Renewal never resurrects an expired,
released, revoked, or killed lease; expiry is explicit and can be reaped. A
lease is scheduling evidence only. It is not an `EffectPermit`, does not
authorize an operation, and cannot bypass Parliament, the ReferenceMonitor,
one-shot permit consumption, or the EffectExecutor.

The store caps nodes at 256, leases at 4,096, and capabilities per node at 64.
Unknown states, malformed records, duplicate identities, unavailable nodes, and
zero-duration leases fail closed. Remote Fleet preview remains a separate
future transport boundary requiring authenticated pairing, TLS, replay
protection, cancellation, reconnect, and remote containment evidence.

The local supervisor is an optional operational layer over Fleet. It keeps a durable state record per node with explicit running, draining, recovering, and stopped states. Worker processes started by `subagent work` bind the running record to their operating-system PID, hold a process-wide execution lease, and publish heartbeats around each durable claim and completion. Evolution quiescence therefore sees active worker processes through the same Fleet lease boundary. Draining blocks new leases while existing work finishes; stopping or restarting requires no active lease. Recovery expires only leases whose recorded expiry has passed and never replays an effect. The supervisor does not spawn arbitrary processes, issue permits, or replace the ReferenceMonitor. It observes the process that owns a worker lifecycle; a crash leaves a stale running record for explicit reconciliation and never causes replay. Evolution activation and rollback use a separate durable Fleet quiescence guard to block new leases across processes while a mutation is in progress.

Supervisor commands:
  pandora fleet supervisor start node-a
  pandora fleet supervisor drain node-a
  pandora fleet supervisor stop node-a --yes
  pandora fleet supervisor recover node-a
  pandora fleet supervisor heartbeat node-a
  pandora fleet supervisor reconcile node-a --stale-after 30
  pandora fleet supervisor list --json
