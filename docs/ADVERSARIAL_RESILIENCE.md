# Adversarial resilience

Phase 11 treats validly delivered content as untrusted until it crosses an
explicit runtime boundary. Transport success, package signatures, repository
access, and agent identity do not make content authoritative.

## Shared content guard

The runtime records one typed origin for each context fragment: runtime,
memory, Skill, user selection, tool, MCP, package, repository, document, issue,
design, agent handoff, or external. Concrete adapter names map to this closed
vocabulary before their output reaches provider context.

A deterministic high-confidence marker set identifies instruction-shaped
content. A match is replaced with a versioned quarantine envelope containing
only its origin, reason, SHA-256 digest, and byte count. The hostile text is not
forwarded or persisted. Benign content remains visible but explicitly
unverified. Persisted and multi-hop envelopes are re-assessed; a forged
`normalized` shape or malformed quarantine record cannot bypass the guard.

The marker set is deliberately narrow. It is a fail-closed guardrail for known
high-confidence forms, not a classifier that claims to detect every prompt
injection or poisoned document. Authority still comes only from Parliament,
the Reference Monitor, exact approvals, one-shot permits, and executor
receipts.

## Replay corpus and handoffs

`crates/pandora-runtime/tests/fixtures/hostile_content_v1.json` contains hostile
cases and benign controls. The integration suite applies every case to every
origin, checks that quarantined output retains only safe evidence, and proves
that a hostile fragment stays quarantined across persisted multi-hop handoffs.
Benign handoff content remains visible and unverified.

Run the deterministic regressions with:

```text
cargo test -p pandora-runtime --test adversarial_content --locked
```

## Package transparency ledger

The package store appends evidence for:

- publisher trust-root additions and revocations;
- allowed package admissions;
- denied store admissions, including oversized, duplicate, incompatible,
  hash-mismatched, dependency-invalid, and signature-invalid inputs.

Each event has a monotonically increasing sequence, safe reason code, bounded
subject fields, previous event digest, and its own deterministic SHA-256 digest.
SQLite triggers reject event updates and deletes. The ledger is capped at 8,192
events; CLI reads are capped at 256.

```text
pandora package transparency list --limit 64
pandora package transparency list --event-kind admission_decision --outcome denied
pandora package transparency inspect --sequence 1
```

The commands report `append-only-sqlite`, `sha256-event-chain`, and
`runtime_authority: false`. They do not admit, enable, activate, or execute a
package. The TUI provides guidance through `/trust-transparency`; the desktop
Package Manager displays the same read-only evidence.

## Production parser fuzzing

The separate `fuzz/` package drives the real production boundaries:

| Target | Boundary |
| --- | --- |
| `path_parser` | cross-platform workspace-relative path validation |
| `manifest_parser` | closed package manifest plus semantic validation |
| `rpc_parser` | bounded newline frame and MCP JSON-RPC response validation |
| `handoff_parser` | strict orchestration-plan and handoff validation |
| `approval_parser` | bounded approval identity, redaction, and summary validation |
| `receipt_parser` | strict persisted governed-effect receipt parser |

Every target has a seed corpus. The dedicated Ubuntu workflow builds all six
with nightly Rust and runs a bounded smoke campaign on pushes and pull
requests. For a longer local campaign:

```text
cargo install cargo-fuzz --version 0.13.2 --locked
cargo fuzz run handoff_parser fuzz/corpus/handoff_parser -- -max_len=65536
```

A crash artifact belongs in a deterministic regression test before its fix is
accepted. Bounded CI fuzzing proves replayability and basic ongoing coverage;
it is not evidence that the input space has been exhausted.
