# Parser fuzzing

These `cargo-fuzz` targets exercise production parsers and their validation boundaries:

- `path_parser`: workspace-relative path confinement;
- `manifest_parser`: package manifests;
- `rpc_parser`: newline-delimited MCP JSON-RPC responses;
- `handoff_parser`: orchestration and handoff plans;
- `approval_parser`: approval identifiers and redacted summaries;
- `receipt_parser`: persisted governed-effect receipts.

Install nightly Rust and `cargo-fuzz` 0.13.2, then run a target from the repository root:

```text
cargo fuzz run path_parser fuzz/corpus/path_parser -- -max_len=65536
```

The CI smoke job runs every target for a bounded time. Any crash artifact is written under
`fuzz/artifacts/` and must be reduced into a deterministic regression test before a fix lands.
