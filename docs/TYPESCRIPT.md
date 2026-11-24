# TypeScript client

Pandora's native CLI remains the runtime and policy authority. The npm package
(`pandora-agent`) exposes low-level helpers (`runPandoraJson`, `parseJsonEnvelope`,
`defaultLauncherPath`) as well as the typed `PandoraClient` wrapper for applications
that need to invoke the stable JSON CLI contract without shell interpolation.

## Low-Level Envelope Helpers

```ts
import { runPandoraJson } from "pandora-agent";

const result = await runPandoraJson(["doctor"]);
if ("command" in result.envelope) {
  console.log(result.envelope.healthy);
} else {
  console.error(result.envelope.code, result.envelope.message);
}
```

`runPandoraJson` passes arguments as an argv array, appends `--json`, preserves
the native exit code, and returns either the documented success or error
envelope. It does not accept credential values, invoke a shell, or create a
second execution path. The launcher still verifies the native release checksum
before starting the CLI.

The parser is also exported as `parseJsonEnvelope` for integrations that invoke
the native binary through their own process supervisor. Responses are bounded
to 4 MiB and malformed envelopes fail closed.

## `PandoraClient`

`PandoraClient` wraps `runPandoraJson` with default `PandoraRunOptions` (`cwd`,
`env`, `launcherPath`, `timeoutMs`) and typed methods for core CLI workflows:

- `run<T>(args, overrideOptions?)`
- `doctor(overrideOptions?)` / `verify(overrideOptions?)`
- `lspDefinition(file, line, col, overrideOptions?)`
- `lspReferences(file, line, col, overrideOptions?)`
- `lspDiagnostics(file, overrideOptions?)`
- `branchList(overrideOptions?)`
- `prCreate(taskId, title, overrideOptions?)`
- `semanticSearch(query, limit?, overrideOptions?)`
- `memoryVfsTropicalSearch(query, options?)`
- `runWithLats(task, options?)`
- `modelVerifyAttestation(pnnPath?, overrideOptions?)`

### `modelVerifyAttestation` & `CalibrationAttestationVerification`

`PandoraClient.modelVerifyAttestation(pnnPath?, overrideOptions?)` invokes
`pandora model verify-attestation --json` (forwarding `--pnn <path>` when
`pnnPath` is non-empty, and `--workspace`, `--config`, or `--data-dir` when
supplied on `ModelVerifyAttestationOptions`):

```ts
import { PandoraClient } from "pandora-agent";

const client = new PandoraClient({ cwd: process.cwd() });
const res = await client.modelVerifyAttestation(".pandora/model_decision.pnn");
if ("command" in res.envelope && res.envelope.valid) {
  console.log(
    res.envelope.chain_length,
    res.envelope.latest_commitment,
    res.envelope.pnn_digest_matches,
  );
}
```

`CalibrationAttestationVerification` fields:

| Field | Type | Description |
|---|---|---|
| `valid` | `boolean` | `true` when the hash-chain ledger and `.pnn` digest both verify |
| `chain_length` | `number` | Number of verified receipts in `<pnn>.attestation.jsonl` |
| `latest_sequence` | `number` | Sequence index of the tip receipt |
| `latest_commitment` | `string` | SHA-256 calibration commitment of the tip receipt |
| `latest_chain_hash` | `string` | SHA-256 chain hash at the tip |
| `pnn_digest_matches` | `boolean` | `true` iff the live `.pnn` file exists and matches `candidate_digest` |
| `path` | `string` | Resolved `.pnn` file path |
| `receipt_path` | `string` | Resolved `<pnn>.attestation.jsonl` ledger path |

If the attestation ledger or `.pnn` file is missing, corrupted, or fails digest
or hash-chain verification, `pandora model verify-attestation` fails closed with
a `PandoraError` envelope (`details.valid === false`, `details.pnn_digest_matches === false`).

### `runWithLats`, `LatsMetrics`, and `LatsTurnTelemetry`

`PandoraClient.runWithLats(task, options?)` invokes `pandora run --strategy lats`
with optional `RunLatsOptions` (`rollouts`, `depth`, `puctC`,
`escalationThreshold`, plus `PandoraRunOptions`):

```ts
const latsRes = await client.runWithLats("Inspect repository status", {
  rollouts: 8,
  depth: 3,
  puctC: 1.4,
  escalationThreshold: 0.35,
});
```

Returned `lats_metrics?: LatsMetrics | null` matches `pandora run --strategy lats --json`:

- `LatsMetrics`: `rollouts`, `max_depth_reached`, `nodes_created`, `best_action`,
  `best_value`, `escalated`, plus optional `turn_rollouts`, `turn_escalations`,
  `dpo_pairs_emitted`, `memo_hits`, `pnn_hot_reloads`, `active_pnn_digest`,
  `lats_memo_hits`, and `turns?: LatsTurnTelemetry[]`.
- `LatsTurnTelemetry`: `turn`, `tool_name`, `escalated`, `rollouts`,
  `max_depth_reached`, `nodes_created`, `best_action`, `best_value`, plus
  optional `reflex_action`, `dpo_pair_emitted`, and `memo_hit`.

#### Runtime Memoization & Auto-Calibration Relationship

- **`LatsPrefixMemoCache`** (`pandora-runtime`): Bounded FIFO cache (`N_max = 256`)
  keyed by 4-bit quantized 7D `DecisionFeatures` and active `.pnn` digest. It is
  enabled by default inside `pandora run --strategy lats` (`enable_memo: true`);
  memoized turns appear in `LatsTurnTelemetry.memo_hit` and `LatsMetrics.memo_hits`.
- **`LatsLoopConfig::with_auto_calibrate(batch_k)`** (`pandora-runtime`): Rust
  builder API enabling in-loop `.pnn` hot-reload every `batch_k` emitted `DpoPair`
  overrides. It is a Rust runtime configuration (`pandora run --strategy lats`
  currently defaults `auto_calibrate_threshold` to `None` and does not expose a
  dedicated flag on `RunLatsOptions`).

### `memoryVfsTropicalSearch`

`PandoraClient.memoryVfsTropicalSearch(query, options?)` invokes
`pandora memory vfs tropical-search <query>` with optional
`TropicalSearchOptions` (`limit`, `tier`, `intent`) and returns
`{ query, results: TropicalSearchResult[], count }` where each
`TropicalSearchResult` has `uri`, `title`, `snippet`, `sparse_score`,
`dense_score`, `hybrid_score`, `tier`, and `intent`.

