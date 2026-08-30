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

The local supervisor is an optional operational layer over Fleet. It keeps a durable state record per node with explicit running, draining, recovering, and stopped states. Worker processes started by `subagent work` and `job work` bind a stable worker record to their operating-system PID, hold a renewable process-wide execution lease, and publish heartbeats around each durable claim and completion. Evolution quiescence therefore sees active worker processes through the same Fleet lease boundary. Draining blocks new leases while existing work finishes; stopping or restarting requires no active lease. Recovery expires only leases whose recorded expiry has passed and never replays an effect. The supervisor does not spawn arbitrary processes, issue permits, or replace the ReferenceMonitor. It observes the process that owns a worker lifecycle; a crash leaves a stale running record for explicit reconciliation and never causes replay. The bounded `reap` command reconciles every stale running supervisor in one pass, expires only leases past their recorded deadline, and does not restart a process or replay an effect. Evolution activation and rollback use a separate durable Fleet quiescence guard to block new leases across processes while a mutation is in progress.

`job work --watch --idle-timeout <1-3600>` is the bounded independently launched worker window. The child process owns the same PID-bound supervisor record, process-wide lease, and heartbeat boundary as a normal worker, then exits after the idle window, an external drain request, or an optional `--max-jobs` cap. `job work --daemon` uses the same boundary for a long-lived local worker. It polls the durable supervisor state and treats `pandora fleet supervisor drain job-worker` as the graceful external-stop protocol: no new claim is admitted, the current claim finishes, the lease is released, and the worker records `stopped`. If the process is killed, its durable running record remains visible until an operator reconciles it; expired leases are cleared and a later worker may bind a new PID, but no previously claimed effect is replayed. The supervisor never launches child processes or grants effects.

Supervisor commands:
  pandora fleet supervisor start node-a
  pandora fleet supervisor drain node-a
  pandora fleet supervisor stop node-a --yes
  pandora fleet supervisor recover node-a
  pandora fleet supervisor heartbeat node-a
  pandora fleet supervisor reconcile node-a --stale-after 30
  pandora fleet supervisor reap --stale-after 30
  pandora fleet supervisor restart --node node-a --process-id 42 --stale-after 30 --stale-after 30
  pandora fleet supervisor list --json

## Phase 7 worker-operations acceptance

The cross-platform bounded profile is implemented by
`phase7_worker_operations_recover_without_replaying_durable_effects`. Its
normal CI mode launches three independent producer streams, completes 18
governed filesystem jobs across two independently launched daemon worker
processes, force-stops the first worker, reconciles its stale PID and expired
lease, and proves that the replacement publishes a new PID and generation. A
third fresh worker observes an empty queue. The same test then drives a
two-repository orchestration run through fresh CLI processes, preserves the
completed planner receipt after the maker role fails, and leaves resume blocked
for explicit receipt reconciliation.

The durable assertions require exactly one terminal job result, one session,
one evaluation, one rollout, and one `EffectCompleted` receipt identity per
submitted job. They are captured before and after the no-op restart and again
after orchestration recovery attempts, so a worker, Fleet, or Orchestration
replay changes the evidence and fails the test.

Run the bounded profile with:

```sh
cargo test -p pandora-cli --test cli_smoke phase7_worker_operations_recover_without_replaying_durable_effects --locked -- --exact --nocapture
```

The explicit soak profile keeps normal CI bounded. By default it uses four
independent producers, eight warm-up jobs, 504 recovery jobs, and a 600-second
recovery submission window. It allows another bounded 180 seconds for the final
drain. On a POSIX shell:

```sh
PANDORA_PHASE7_SOAK=1 \
PANDORA_PHASE7_SOAK_SECONDS=600 \
PANDORA_PHASE7_SOAK_JOBS=512 \
PANDORA_PHASE7_SOAK_PRODUCERS=4 \
cargo test -p pandora-cli --test cli_smoke phase7_worker_operations_recover_without_replaying_durable_effects --locked -- --exact --nocapture
```

PowerShell uses the same values through `$env:PANDORA_PHASE7_SOAK`,
`$env:PANDORA_PHASE7_SOAK_SECONDS`, `$env:PANDORA_PHASE7_SOAK_JOBS`, and
`$env:PANDORA_PHASE7_SOAK_PRODUCERS`. Producers may be 2-8, jobs may be from
four per producer through 4,096, and the submission window may be 60-86,400
seconds. Long-duration runs remain an operator/release evidence gate and are
not part of the default CI profile.

Workers only claim and delegate the original governed command. Fleet leases and
Orchestration role receipts remain scheduling and recovery evidence, not
authority. The sole effect-authority chain remains
`ExecutionController -> Parliament -> ReferenceMonitor -> executor -> receipt`;
none of these operating layers can add capabilities, issue permits, or replay
an uncertain effect.
