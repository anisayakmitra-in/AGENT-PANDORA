# Declarative Gene pack examples

This pack contains three import-free WebAssembly Gene examples and one owning Domain Harness. The manifests are the signed declarative boundary: they bind exact identity, version, publisher, artifact hash, runtime compatibility, dependencies, and downstream capability declarations.

| Gene | Contract | Downstream capability | Approval declaration |
| --- | --- | --- | --- |
| `example/static-guide@1.0.0` | `static_guidance` | none | no |
| `example/bounded-read@1.0.0` | `bounded_read` | `filesystem.read` | no |
| `example/patch-proposal@1.0.0` | `effect_request` | `filesystem.write` | yes |

The capability list is not a grant. The WASM sandbox can only return JSON. A downstream request must match the signed contract, pass Parliament policy, carry approval when required, receive a scoped one-shot permit from ReferenceMonitor, and produce an effect receipt.

## Validate and admit

Run these commands from the repository root, replacing `<data-dir>` with a disposable local directory:

```text
pandora package validate --manifest sdk/gene-pack/genes/static-guide/pandora.package.json --artifact sdk/gene-pack/genes/static-guide/static-guide.wasm
pandora package validate --manifest sdk/gene-pack/genes/bounded-read/pandora.package.json --artifact sdk/gene-pack/genes/bounded-read/bounded-read.wasm
pandora package validate --manifest sdk/gene-pack/genes/patch-proposal/pandora.package.json --artifact sdk/gene-pack/genes/patch-proposal/patch-proposal.wasm

pandora package admit --data-dir <data-dir> --manifest sdk/gene-pack/genes/static-guide/pandora.package.json --artifact sdk/gene-pack/genes/static-guide/static-guide.wasm
pandora package admit --data-dir <data-dir> --manifest sdk/gene-pack/genes/bounded-read/pandora.package.json --artifact sdk/gene-pack/genes/bounded-read/bounded-read.wasm
pandora package admit --data-dir <data-dir> --manifest sdk/gene-pack/genes/patch-proposal/pandora.package.json --artifact sdk/gene-pack/genes/patch-proposal/patch-proposal.wasm
pandora package admit --data-dir <data-dir> --manifest sdk/gene-pack/domain/pandora.package.json --artifact sdk/gene-pack/domain/gene-pack-domain.artifact
```

Admission leaves every package disabled. Preview each exact activation before confirmation:

```text
pandora package enable --data-dir <data-dir> example/static-guide 1.0.0 --dry-run
pandora package enable --data-dir <data-dir> example/static-guide 1.0.0 --yes
pandora package inspect --data-dir <data-dir> example/static-guide 1.0.0
pandora package disable --data-dir <data-dir> example/static-guide 1.0.0 --dry-run
pandora package disable --data-dir <data-dir> example/static-guide 1.0.0 --yes
```

Enable all three exact Genes before enabling `example/gene-pack-domain@1.0.0`. To evaluate rollback, admit a second exact version of one Gene with its hash-bound artifact, enable version 1 and then version 2, preview `package rollback`, and confirm with `--yes`. The CLI and desktop inspectors report activation generation, active and previous versions, provenance, declared capabilities, and owning Domain records.

See [ARCHITECTURE.md](ARCHITECTURE.md) for the authority boundary and [fixtures/inspector.json](fixtures/inspector.json) for deterministic inspector evidence.
