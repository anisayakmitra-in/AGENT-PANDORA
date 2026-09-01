# Durable orchestration workers

Pandora coordinates larger runs through the existing Meta Harness and Domain
Harness contracts. The orchestration store is a durable coordination and
evidence layer. It cannot execute a tool, issue a permit, select a Gene,
activate an evolution candidate, or bypass Parliament and the Reference
Monitor.

## Multi-repository plan

A submitted plan binds every role to exactly one repository identity, workspace
identity, and exact commit. Cross-Domain handoffs still require a declared Meta
Harness. The following abbreviated JSON shows the persisted input shape:

```json
{
  "plan": {
    "id": "product-change",
    "roles": [
      {
        "id": "planner",
        "role": "planner",
        "harness_id": "coding-domain",
        "depends_on": []
      },
      {
        "id": "maker",
        "role": "maker",
        "harness_id": "design-domain",
        "depends_on": ["planner"]
      }
    ],
    "max_parallelism": 2,
    "max_handoffs": 1,
    "handoffs": [
      {
        "from": "planner",
        "to": "maker",
        "meta_harness": "coordination-meta"
      }
    ]
  },
  "meta_composition": {
    "allowed_domains": ["coding-domain", "design-domain"],
    "max_handoffs": 1
  },
  "repositories": [
    {
      "repository_id": "api",
      "workspace_id": "workspace-api",
      "exact_commit": "0123456789abcdef"
    },
    {
      "repository_id": "desktop",
      "workspace_id": "workspace-desktop",
      "exact_commit": "fedcba9876543210"
    }
  ],
  "role_repositories": [
    {"role_id": "planner", "repository_id": "api"},
    {"role_id": "maker", "repository_id": "desktop"}
  ],
  "aggregate_budget": {
    "ceiling": {
      "tokens": 200000,
      "tools": 200,
      "elapsed_ms": 600000,
      "cost_micros": 300000
    },
    "roles": [
      {
        "role_id": "planner",
        "reservation": {
          "tokens": 100000,
          "tools": 100,
          "elapsed_ms": 300000,
          "cost_micros": 100000
        }
      },
      {
        "role_id": "maker",
        "reservation": {
          "tokens": 100000,
          "tools": 100,
          "elapsed_ms": 300000,
          "cost_micros": 100000
        }
      }
    ]
  }
}
```

Deserialized plans are revalidated before persistence. Unknown repositories,
missing or duplicate role bindings, undeclared Domains, dependency cycles, and
Meta Harness handoff-limit violations fail closed. The parser rejects unknown
plan fields, invalid reconstructed IDs, more than 64 roles or 64 handoffs, a
handoff list above its declared budget, and parallelism above the role count.
Every new durable submission also requires exactly one role reservation per
role. Duplicate, missing, unknown, overflowing, and over-ceiling budgets fail
closed. The same parser is a seeded fuzz target.

Content carried between roles remains untrusted. Persisted handoff fragments
are re-assessed at every hop; high-confidence instruction-shaped content is
replaced by digest-and-byte-count evidence before persistence or provider
context assembly. A forwarded envelope cannot bypass this check by claiming it
was already normalized.

## Headless worker protocol

```text
pandora orchestration submit --input plan.json
pandora orchestration claim --worker worker-a --json
pandora orchestration complete <run-id> --worker worker-a --role planner --receipt planner-receipt.json
pandora orchestration inspect <run-id> --json
pandora orchestration list --json
```

`claim` atomically takes the oldest queued run in the current principal, tenant,
and coordinator workspace scope. It starts only dependency-ready roles and
never exceeds the plan's parallelism limit. The returned assignments contain
the Domain Harness, repository ID, workspace ID, and exact commit needed by the
worker. Actual work must run through the normal governed run or subagent path.

A completion receipt is an evidence reference, not authority. It must match the
run, role, repository, workspace, and exact commit. It contains its own receipt
ID plus either a stable evidence digest or references to the governed effect
receipts produced by the existing execution path:

```json
{
  "receipt_id": "orchestration-receipt-planner",
  "run_id": "product-change-run-1",
  "role_id": "planner",
  "repository_id": "api",
  "workspace_id": "workspace-api",
  "exact_commit": "0123456789abcdef",
  "governed_effect_receipts": ["effect-receipt-1"],
  "evidence_digest": "planner-evidence-digest",
  "usage": {
    "tokens": 1200,
    "tools": 4,
    "elapsed_ms": 8300,
    "cost_micros": 4200,
    "source_receipts": ["effect-receipt-1"]
  }
}
```

Use one stable worker ID from claim through completion. Another worker cannot
complete the claimed run. Completion requires measured token, tool, elapsed,
and cost usage tied to a governed effect receipt. Set `cost_micros` to `null`
when the provider does not report cost. The read model preserves that unknown
value; enforcement charges the role's full cost reservation so missing
telemetry can never create extra capacity.

Ready-role dispatch and its reservation are committed in one SQLite
transaction. Receipt settlement and completion are also one transaction. At
all times the store enforces:

```text
enforced_consumed + active_reservations <= aggregate_ceiling
```

`orchestration inspect`, `orchestration list`, `fleet dashboard`, the TUI
`/fleet-health` view, and desktop Background Runs expose ceiling, active
reservations, measured use, enforceable remaining capacity, unknown-cost count,
and the invariant result without including prompts, outputs, credentials, or
hidden reasoning.

## Crash and resume semantics

A worker crash never causes automatic replay of active roles because their
external effect outcome may be unknown. Operators can record the interruption:

```text
pandora orchestration mark-interrupted <run-id> --reason "worker exited" --yes
pandora orchestration reconcile-failed <run-id> --role <role-id> --usage usage.json --evidence-digest <digest> --yes
pandora orchestration resume <run-id>
```

Resume succeeds only when the durable snapshot has no active roles. If roles
were dispatched, Pandora requires their effect receipts to be reconciled first
instead of guessing that retry is safe. `reconcile-failed` is the explicit
operator attestation that the uncertain attempt has been reviewed. It records
partial measured usage and a stable evidence digest, releases only the unused
reservation, and clears that role from the active snapshot. Exact duplicate
reconciliation is rejected and cannot consume the ledger twice. After every
active role has been reconciled, `resume` may requeue the run; a replacement
worker can reserve the failed role again only when enough aggregate capacity
remains. Queued runs may be cancelled with `pandora orchestration cancel
<run-id>`.

## Desktop inspection and control

The authenticated local service exposes scoped `orchestration.list`,
`orchestration.inspect`, `orchestration.cancel`, and `orchestration.resume` RPC
methods. The desktop Background Runs surface uses those methods; it never reads
the orchestration database directly and cannot claim a run, steal a worker
lease, complete a role, issue a permit, or fabricate a receipt.

Cancellation is available only for queued work. Resume is available only for an
interrupted snapshot that the orchestration store considers safe to requeue.
Both mutations require the exact run ID as confirmation, and service role policy
limits them to operators and administrators. Native desktop device trust is
established automatically, so this internal service boundary does not create an
account or sign-in flow.

## Phase 7 recovery evidence

The bounded worker-operations CLI acceptance regression uses fresh processes for
submit, claim, complete, interruption, inspection, reconciliation, resume, a
replacement worker claim, and final inspection. It proves that a completed
planner receipt remains exactly once while an uncertain maker attempt keeps its
reservation. Resume fails before reconciliation. Partial maker usage is then
persisted once, unused capacity is released, and a retry receives a fresh
reservation only inside the remaining aggregate ceiling.
