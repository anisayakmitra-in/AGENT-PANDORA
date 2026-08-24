# CLI reliability baselines

Pandora records a small reliability and startup baseline for every CI runner.
The report measures two bounded commands against one isolated setup:

- `pandora --version --json`
- `pandora doctor --json`

Each command records its attempts, successes, failures, timeouts, elapsed
samples, median latency, and nearest-rank p95 latency. Command output is
validated against JSON contract `0.1`, then discarded. The report does not
store standard output, standard error, prompts, model output, credentials, or
environment values.

## Run locally

Build the release binary, then write the report to an existing directory:

```text
python scripts/measure_cli.py --binary target/release/pandora --iterations 10 --timeout-seconds 10 --output cli-baseline.json
```

Use `target/release/pandora.exe` on Windows. The runner creates an isolated
configuration, data directory, and workspace beside the output file and
removes them when the measurement finishes.

The command exits with a failure if setup fails, a measured command fails, a
response violates the JSON contract, or a command exceeds its timeout. It does
not enforce a latency threshold. CI uploads the report as
`cli-baseline-<os>-<architecture>` for later inspection.

## Compare results

Compare commits only when their reports use an equivalent runner class,
iteration count, timeout, build profile, and workload. Hosted runner load
varies, so a single sample is not a performance claim.

These measurements establish Pandora's own trend line. They do not support
claims that Pandora is faster or more reliable than Codex, Hermes Agent, Prime
Agent, or another tool. A cross-product claim requires a separate reproducible
benchmark with pinned versions, equivalent tasks, disclosed hardware, repeated
trials, and published raw results.
