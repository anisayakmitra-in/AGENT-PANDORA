# Contributing to Pandora

Pandora is developed in public. The best contributions solve one observable
problem, state which authority boundary they touch, and include tests for the
failure path.

## Before writing code

1. Read [Why Pandora?](docs/WHY_PANDORA.md), the
   [roadmap](docs/ROADMAP.md), [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md), and
   [SECURITY.md](SECURITY.md).
2. Search open and closed issues and pull requests for the same behavior.
3. Open or claim an issue before starting a change that will take more than a
   small patch. Say what you plan to change and what you will leave alone.
4. Wait for maintainer agreement before changing Parliament, Shadow Council,
   ReferenceMonitor, permit issuance, constitutional Source bindings, package
   trust, self-activation rules, or a public JSON contract.
5. Keep credentials, private prompts, local paths, databases, and generated
   build output out of commits and issue attachments.

An issue is ready for implementation when it has:

- one user-visible result;
- the current behavior and a safe reproduction;
- the authority boundary and invariants that must stay true;
- acceptance checks for success, refusal, and recovery;
- a scope small enough to review without a broad rewrite.

Use the contribution-proposal issue form when you want to implement the change
yourself. Security reports belong in the private process described in
[SECURITY.md](SECURITY.md), not a public issue.

## Good first contributions

Tests, docs, inspection views, failure messages, bounded adapters, release
drills, and parser fixtures are good places to start. The roadmap lists current
issue-sized areas.

Avoid using a first contribution to redesign the authority chain, merge engines,
replace exact permits with model judgment, add hidden execution paths, or make
self-improvement activate itself. Those changes conflict with Pandora's public
contract unless an accepted architecture issue says otherwise.

## Pull requests

Keep each pull request tied to one issue or one small defect. In the description:

- link the issue;
- describe the behavior, not the amount of code;
- name the authority boundary touched or say that none changed;
- list the exact commands you ran;
- call out incomplete platform checks or release evidence;
- include screenshots for visible desktop changes;
- update docs when a public command, JSON field, package contract, or lifecycle
  changes.

Do not mix formatting churn, dependency updates, generated files, or unrelated
refactors into the same change. A locally passing patch is not ready to merge
when the full checks, platform-specific checks, or documentation still disagree.

## Local checks

Run the focused tests while developing, then run the full repository checks:

```sh
cargo fmt --all -- --check
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --lib --tests --locked
python scripts/validate_repo.py
python scripts/validate_docs.py
```

For desktop changes:

```sh
cd apps/pandora-desktop
npm ci --ignore-scripts
npm test
npm run build
```

Native packaging is platform-specific. Say which operating systems you tested.
Do not claim a Windows, macOS, or Linux release check from a frontend-only
build.

## Architecture rules

- Preserve `Parliament -> Shadow Council -> Harness -> Gene` ownership.
- Keep ReferenceMonitor as the sole permit issuer.
- Do not let a model, Gene, Skill, Harness, evaluator, evolution component, or
  package authorize its own effects.
- Bind execution to exact versions, artifacts, scopes, and one-shot permits.
- Keep public contracts versioned and document shipped behavior separately from
  planned behavior.
- Fail closed on ambiguous routing, package identity, approval, recovery, and
  handoff state.
- Do not copy external projects wholesale. Record clean-room inspiration and
  verify the license before reusing code or assets.

## Review and merge

Maintainers may ask for a smaller change when a pull request crosses several
authority or product boundaries. Review checks behavior, invariants, tests,
documentation, and release impact. Passing CI alone does not waive an
architecture or security concern.
