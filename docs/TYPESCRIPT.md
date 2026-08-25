# TypeScript client

Pandora's native CLI remains the runtime and policy authority. The npm package
also exposes a small TypeScript client for applications that need to invoke the
stable JSON CLI contract without shell interpolation.

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
