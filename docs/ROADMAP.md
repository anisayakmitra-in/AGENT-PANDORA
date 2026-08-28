# Pandora roadmap

Pandora's remaining work is mostly above the runtime safety layer. Phase 6 made
the local system production-shaped: identity, tenant scope, encrypted secrets,
device trust, recovery, telemetry, release gates, and security checks. The next
phases should build agent-platform behavior on top of those boundaries without
changing the Parliament, Reference Monitor, Harness, Gene, and governed
execution architecture.

## Phase 7 - Runtime scale and orchestration

Status: foundation implemented on `main`. Prompt caching, durable scoped worker
claims, interruption and resume evidence, role receipts, and exact-commit
multi-repository plans are present. Further load testing and fleet operations
continue in Phase 12.

- Prompt caching for repeated planning, evaluation, and tool-use contexts.
- Parallel agents with bounded work queues, leases, cancellation, and receipts.
- Background agents that can resume governed runs without a second execution
  path.
- Headless runs suitable for schedulers, servers, and CI.
- Orchestration workers that coordinate Domain Harnesses through Meta Harness
  limits.
- Multi-repository orchestration with explicit workspace identities and per-repo
  receipts.

Outcome: Pandora can run larger agent workloads without bypassing its authority
model.

## Phase 8 - Agent experience and disclosure

Status: in progress. The desktop now has Flow, Evidence, and Context disclosure,
a Harness Lab for runtime-reported Genes, plugins and tools, authority, and
receipt posture, plus a scoped Background Runs surface for durable orchestration
inspection, exact queued cancellation, and safely reconciled resume. Deeper
artifact previews and provider configuration remain.

- Progressive disclosure for plans, evidence, receipts, approvals, and tool
  state.
- A production desktop frontend informed by the OpenDesign local-first design
  workflow: studio-style project surfaces, live previews, inspectable artifacts,
  and BYOK provider configuration.
- Harness inspection pages for Genes, packages, plugins, capabilities,
  approvals, receipts, and active catalog replacements.
- Background-run views for queued, running, paused, failed, and completed agent
  work.
- A design system that keeps Pandora visually distinct while making governance
  visible instead of decorative.

Outcome: users can inspect and steer Pandora without needing to read logs or
raw database state.

## Phase 9 - Evaluation-driven loops

- Evaluation-driven improvement loops for prompt, Skill, workflow, and Wasm Gene
  candidates.
- Golden, holdout, adversarial, and human-review suites with stable evidence
  digests.
- Self-healing test generation for regressions discovered during runs.
- Canary evaluation and rollback automation for active catalog replacements.
- Cost, latency, stability, and quality scoring that feeds evolution evidence
  but never grants authority.

Outcome: Pandora can improve candidates through evidence while keeping
activation separate from evaluation.

## Phase 10 - Memory consolidation

- Memory consolidation across sessions, projects, tenants, providers, and
  repositories.
- Promotion rules from short-lived run evidence into durable memory layers.
- Forgetting, compaction, redaction, and source-trace controls.
- User-visible memory inspection and removal.
- Evolution lineage queries that can explain which memories shaped a candidate.

Outcome: Pandora learns from work without storing hidden reasoning or leaking
tenant data across scopes.

## Phase 11 - Adversarial resilience

- Tool-poisoning detection for MCP, plugin, package, and filesystem inputs.
- Prompt-injection tests for documents, repositories, issue trackers, and design
  artifacts.
- Self-healing runtime checks that quarantine poisoned tool outputs and stale
  package metadata.
- Adversarial test harnesses for path confinement, secret exposure, approval
  spoofing, replay, confused-deputy behavior, and agent-to-agent handoff.
- Dependency and package trust-root policy for marketplace-style distribution.

Outcome: Pandora treats every tool and artifact as hostile until verified by
scope, policy, and evidence.

## Phase 12 - Agent operations

- Agent CI/CD for lint, tests, evals, package admission, release evidence, and
  deployment promotion.
- Agents managing agents through bounded delegation, budgets, leases, and
  review gates.
- Multi-agent runbooks for code, design, research, security, operations, and
  debugging Domains.
- Fleet-level observability for local-first workers without uploading prompts,
  outputs, secrets, or hidden reasoning.
- Release-channel automation for beta, release-candidate, and stable promotion.

Outcome: Pandora can operate as an agent platform, not just a local agent
runtime.

## Remaining scope estimate

The current tree is roughly production-ready for a governed local CLI/runtime,
but still early for the full Pandora platform. A practical estimate is:

- local governed runtime and production controls: about 70-80% complete;
- desktop/product frontend: about 45-55% complete;
- autonomous evolution loops: about 35-45% complete;
- multi-agent orchestration and background scale: about 60-70% complete;
- memory consolidation: about 25-35% complete;
- adversarial resilience and agent operations: about 35-45% complete.

Overall, Pandora is around 55-65% of the intended full platform. The remaining
work is not a rewrite. It is mostly product surface, orchestration durability,
evaluation depth, memory governance, and operational hardening on top of the
architecture that already exists.
