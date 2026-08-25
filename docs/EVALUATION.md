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

This is a regression primitive, not a benchmark claim. A passing golden set
does not establish safety, general capability, citation quality, or production
readiness without separate policy, adversarial, holdout, and human evaluation.
