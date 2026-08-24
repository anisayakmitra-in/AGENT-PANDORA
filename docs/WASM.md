# WebAssembly package Genes

Pandora can execute an installed `gene` package only when an admitted Domain
Harness depends on that exact package ID and version. The package artifact must
be a WebAssembly module. Installation alone grants no runtime authority.

The runtime uses the `wasmi` interpreter. It does not enable WASI, host
functions, JIT compilation, or native plugins. Every call follows the existing
execution path:

```text
Domain Harness selection
  -> package Gene plan
  -> Parliament decision
  -> explicit approval
  -> ReferenceMonitor permit
  -> one-shot permit consumption
  -> WasmExecutor
  -> effect receipt
```

`wasm.execute` is an `execute` operation, so the shipped CLI policy always
requires explicit approval. The request binds the execution profile, package
ID, exact version, artifact hash, Gene ID, and JSON payload digest. A permit for
another package, version, payload, profile, or executor fails before the module
runs.

## Module contract

A module must export:

```text
memory memory
pandora_alloc(i32 input_length) -> i32 input_pointer
pandora_run(i32 input_pointer, i32 input_length) -> i64 packed_output
```

Pandora writes one UTF-8 JSON value at the pointer returned by
`pandora_alloc`. `pandora_run` returns the output pointer in the high 32 bits
and the output length in the low 32 bits. The output must also be one UTF-8
JSON value.

The module may not import anything. A module with WASI, filesystem, network,
environment, clock, random, or other host imports is rejected during runtime
assembly.

Registration validates export metadata without instantiating the module or
running its start function. Compilation applies strict malicious-module limits
and ignores custom sections. If a module has a start function, it runs only
after the one-shot permit is consumed and remains subject to the invocation
fuel and memory limits.

## Limits

- package artifact: 16 MiB;
- input JSON: 64 KiB;
- output JSON: 64 KiB;
- linear memory: 16 MiB;
- fuel per invocation: 1,000,000 units;
- instances per invocation: one;
- memories per invocation: one;
- tables per invocation: one, with at most 1,024 elements.

Pandora creates a fresh instance for every call. Module memory is not retained
between calls. Fuel exhaustion, memory growth beyond the limit, malformed JSON,
an invalid ABI, or a permission mismatch returns a failed receipt.

## Admission and execution

First admit the Gene module and a Domain Harness package whose dependency names
that Gene at the exact version:

```text
pandora package admit --manifest gene.json --artifact gene.wasm
pandora package admit --manifest domain.json --artifact domain.artifact
```

The Gene record is stored as `installed`; the Domain Harness profile is stored
as `admitted`. Select both exact identities when running:

```text
pandora run \
  --harness owner/domain \
  --harness-version 1.0.0 \
  --gene owner/transform \
  '{"value":42}' \
  --json
```

The first call returns an approval ID without executing the module. Inspect and
resolve that approval, then repeat the same call with its session and approval
IDs. The version-qualified slash command is also discoverable:

```text
pandora slash list
pandora /gene:owner%2Fdomain@1.0.0:owner%2Ftransform '{"value":42}'
```

Remote package installation may fetch and verify a `gene` artifact, but it does
not execute it. Execution still requires the admitted Domain Harness binding,
the active policy decision, an exact one-shot approval, and the Wasm executor's
request checks.
