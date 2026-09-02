# Evaluation

Pandora separates trajectory, outcome, policy, regression, adversarial, and
human evaluation. Evaluation results are evidence; they cannot approve a
permit, grant a capability, or activate a package.

`EvaluationEngine::evaluate_golden_set` provides a small deterministic runner
for regression and release checks. A caller supplies a bounded set of case IDs,
redacted `EvaluationRequest` values, and expected redacted outcomes. Cases are
sorted by ID, duplicate IDs are rejected, and the report contains only:

- total, passed, and failed counts;
- per-case IDs and evaluation results;
- a SHA-256 report digest over those bounded results.

Expected outputs are used only during the comparison and are never returned in
the report. The runner accepts at most 256 cases, case IDs up to 256 bytes, and
expected outcomes up to 64 KiB. It does not call a model, execute a Gene, read
files, or persist results. Callers that need durable release evidence must
persist the resulting digest through the existing session and receipt
authority.

Every typed report also emits deterministic artifact scorecards grouped by
`prompt`, `skill`, `workflow`, and `wasm_gene`. Each scorecard includes bounded
case IDs, pass/fail counts, average score, pass rate, and its own digest derived
from the parent report. Untyped legacy cases remain in the aggregate report but
are omitted from artifact scorecards because they have no safe class binding.
These views are read-only evidence and cannot approve, activate, or authorize a
candidate.

A case may also carry a typed target binding and a bounded task label. Supported
target kinds are prompt, skill, workflow, and wasm_gene. Target IDs are
trimmed, non-empty, control-character-free, and limited to 256 bytes. Task
labels are required when a target is present, limited to 16 KiB, and are
metadata for suite identity and operator inspection; they are not executed,
sent to a provider, or returned in the report. Legacy cases without target
metadata remain valid. Suite registration reports the number of targeted cases
and counts by target kind, while the same deterministic runner continues to
produce evidence only.

Example case:

    {
      "id": "workflow-smoke",
      "target": {
        "kind": "workflow",
        "id": "workflow-1"
      },
      "task": "run the bounded workflow case",
      "execution_id": "exec-workflow-smoke",
      "output": "tests passed",
      "expected_output": "tests passed"
    }

Failure-derived regression candidates are a separate, review-gated artifact.
The CLI can generate a durable candidate from a failed typed case. It records
only bounded metadata and a hash of the failure evidence. Candidates start in
proposed state; an explicit accept decision is required before suite
registration can use the candidate. Rejected or unreviewed candidates cannot
cross that boundary. Candidate generation and review do not execute the
target, call a provider, invoke a tool, or approve an artifact.

This is a regression primitive, not a benchmark claim. A passing golden set
does not establish safety, general capability, citation quality, or production
readiness without separate policy, adversarial, holdout, and human evaluation.

## Scheduled candidate canaries

Durable evaluation schedules reference the exact digest of a registered suite.
Evidence-backed cases remain deterministic. Task-backed cases use the governed
Controller adapter, so their requested Harness, Genes, policy decisions,
permits, and receipts stay on the normal execution path. A caller-supplied
suite file is accepted only when its digest matches the registered definition.

A schedule may bind one staged evolution proposal. That binding makes the
schedule one-shot: the first durable claim disables further occurrences and
records the proposal ID on the run. The production canary policy derives its
result from the report failure count, while run history retains the suite
report digest and case counts. Exact canary retries are idempotent; conflicting
evidence fails closed.

The scheduled worker cannot approve, activate, or roll back the candidate. A
passing result changes the proposal only to `canary_passed`. Activation remains
a separate operator command guarded by artifact admission, quiescence, backup,
and exact confirmation.
