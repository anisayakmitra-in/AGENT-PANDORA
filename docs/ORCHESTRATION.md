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
  ]
}
```

Deserialized plans are revalidated before persistence. Unknown repositories,
missing or duplicate role bindings, undeclared Domains, dependency cycles, and
Meta Harness handoff-limit violations fail closed.

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
  "evidence_digest": "planner-evidence-digest"
}
```

Use one stable worker ID from claim through completion. Another worker cannot
complete the claimed run.

## Crash and resume semantics

A worker crash never causes automatic replay of active roles because their
external effect outcome may be unknown. Operators can record the interruption:

```text
pandora orchestration mark-interrupted <run-id> --reason "worker exited" --yes
pandora orchestration resume <run-id>
```

Resume succeeds only when the durable snapshot has no active roles. If roles
were dispatched, Pandora requires their effect receipts to be reconciled first
instead of guessing that retry is safe. There is intentionally no orchestration
CLI reconciliation, retry, or restart transition for an interrupted snapshot
with active roles: resume fails closed with the receipt reconciliation gate, and
a replacement worker sees no claimable run. This avoids inventing a replay path
for an uncertain external effect. Queued runs may be cancelled with
`pandora orchestration cancel <run-id>`.

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
submit, claim, complete, interruption, inspection, a replacement worker claim,
two unsafe-resume attempts, and final inspection. It proves that a completed
planner receipt remains exactly once while the maker stays active and uncertain.
The current contract has no safe active-role reconciliation transition, so both
resume attempts fail closed and the replacement worker remains idle. This is
deliberate evidence of the no-replay gate, not a request to retry the maker.
