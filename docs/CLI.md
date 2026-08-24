# Pandora CLI

Status: Partial in the current `2.0.0-alpha.6` development line.

The CLI is the primary Pandora surface. Commands return versioned JSON with
`--json` and stable non-zero exit codes for usage, configuration, policy,
approval, execution, update, and internal failures. See the
[CLI JSON contract](CLI_JSON.md) for the `0.1` envelope, command fields, and
exit-code mapping.

## Setup and diagnostics

```text
pandora --help
pandora help
pandora --version
pandora --version --json
```

Both forms print the command surface and exit successfully. Add `--json` when
the help text needs to be consumed by an installer or another tool.
`pandora --version` remains a single human-readable line; with `--json`, it
uses the normal output envelope and includes `pandora_version`.

In a real interactive terminal, a bare `pandora` invocation runs the existing
setup wizard when its configuration file is missing, then opens the same
Ratatui client as `pandora tui`. A malformed configuration fails rather than
being replaced. A bare noninteractive invocation, including `pandora --json`,
returns the normal usage error rather than entering a UI.

```text
pandora setup --provider-url https://provider.example/v1 --model gpt-5 --api-key-env PANDORA_PROVIDER_API_KEY
pandora setup --interactive
pandora doctor --json
pandora provider list --json
pandora provider test --json
```

`setup --interactive` asks for a provider URL, model, and API-key environment
variable name. Leaving the URL empty creates a local-only configuration. It
never asks for or stores the API-key value; press Enter to accept the displayed
default. The flag-based form creates or updates the same active
`openai-compatible` profile, so scripts and CI can select a non-default
credential environment without storing a credential value.

`doctor` reports the platform, CLI version, configuration path, storage path and
writeability, workspace path, policy mode, provider configuration state, executor containment evidence, and remediation. If
a provider is configured, it also verifies that the configured credential
environment variable contains a usable value without exposing that value.
Provider connectivity is deliberately `not_checked`; diagnostics do not send a
request. A valid local-only setup is healthy for read-only tasks and reports
the provider check as `not_configured`; configure a provider before running
model-backed tasks.

Containment entries are deterministic inspection evidence for the shipped
executor implementations. `partial` lists the controls Pandora applies and the
remaining limitation. `unavailable` means that boundary is not contained. The
report does not grant authority and does not describe the process, Git
worktree, or MCP child programs as sandboxed.

`provider test` sends one bounded request using the active profile's credential
environment variable through the provider permit boundary and reports the
selected model, response, and token usage.
The permit binds the selected protocol, endpoint, and credential-variable
reference as well as the canonical provider request. It includes the model,
messages, tools, token budget, timeout, and trace identifiers. Changing any of
those values requires a new permit.
Use `pandora provider set --provider-url <url> --model <model>` to configure the
backward-compatible `openai-compatible` profile. Multiple provider profiles can
keep separate endpoints, models, and credential variables:

```text
pandora provider set --name coding --provider-url https://coding.example/v1 --model coding-model --api-key-env PANDORA_CODING_API_KEY
pandora provider set --name design --provider-url https://design.example/v1 --model vision-model --api-key-env PANDORA_DESIGN_API_KEY
pandora provider set --name coding --provider-url https://coding.example/v1 --model coding-model --fallback-provider design
pandora provider set --name coding --provider-url https://coding.example/v1 --model coding-model --input-micros-per-million-tokens 2000000 --output-micros-per-million-tokens 4000000
pandora provider use design
pandora provider list --json
pandora provider test --provider design --json
```

Use `pandora run --provider coding ...` for a one-run selection. Profiles store
only endpoint, model, environment-variable names, and optional declared token
pricing; Pandora never stores API key values in configuration or output. Pricing
uses integer micro-units per million input and output tokens. Both rates must be
provided together. A profile can name one fallback profile.
Pandora uses it only for credential, transport, timeout, rate-limit, or server
failures; invalid requests and other client errors return immediately. Fallback
selection does not grant tools or permissions. The controller creates a new
request, policy decision, one-shot permit, and receipt for the fallback attempt;
the primary permit cannot authorize the fallback provider. Nested fallback
chains are rejected.

If every configured provider attempt fails, JSON errors include the ordered
receipt IDs and safe outcomes for each call. Credentials, prompts, and request
payloads are excluded from that audit summary.

## Sessions and execution

```text
pandora run "read:README.md"
pandora run "search:needle"
pandora run "audit"
pandora run "debt"
pandora run "guide"
pandora run --agent "Read the README and summarize it"
pandora run --harness coding --gene workspace.read "read:README.md"
pandora run --plan "inspect the README and report what it contains"
pandora run --approval <approval-id> "patch:README.md:approved content"
pandora chat
pandora chat --session <session-id>
pandora tui
pandora tui --session <session-id>
pandora session list
pandora session inspect <session-id>
pandora session resume <session-id>
pandora job submit -- --agent "Review this workspace"
pandora job work
pandora job work --max-jobs 8
pandora job list
pandora job inspect <job-id>
pandora job cancel <job-id>
pandora job mark-interrupted <job-id> --reason "worker exited" --yes
pandora subagent spawn --session <session-id> --execution <execution-id> "Review the README"
pandora subagent work --max-agents 1
pandora subagent list
pandora subagent inspect <subagent-id>
pandora subagent cancel <subagent-id>
pandora subagent mark-interrupted <subagent-id> --reason "worker exited" --yes
pandora subagent cleanup <subagent-id> --yes
pandora approval list
pandora approval inspect <approval-id>
pandora approval resolve <approval-id> --allow
```

`job submit` stores one bounded `run` request in the scoped `jobs.sqlite3`
database. Put queue path options such as `--config`, `--data-dir`, and
`--workspace` before the required `--` separator. Arguments after the
separator are passed to the existing `run` command. Pandora records the
resolved paths with the request so a worker does not depend on its current
directory or ambient path settings.

`job work` atomically claims the oldest queued job, executes it through the
same sessions, policy, approval, permit, evaluation, and receipt path as a
foreground run, stores the versioned CLI result, and exits. Pass
`--max-jobs <1-64>` to process a bounded FIFO batch sequentially. The JSON
response reports processed job IDs, statuses, and whether the worker reached
its limit or emptied the queue. Multiple worker processes cannot claim the
same queued record. Each `job work` invocation records one worker ID. Only
that worker can finish a job it claimed.

A batch stops at the first approval pause or failed run. Its error response
includes the jobs processed during that invocation, and later jobs remain
queued. An approval pause is stored as `approval_required`; it is not converted
into success or retried. Submit a new job with the approved invocation after
reviewing and resolving the approval.

`job cancel` applies only to queued work. It does not terminate a running
process. Pandora never replays a claimed job automatically when a worker exits,
because the worker may already have produced effects. Such a job remains
`running` until an operator reviews it. Use `job mark-interrupted <job-id>
--reason "..." --yes` to record a terminal `interrupted` outcome with unknown
external effects. The command does not stop a process or requeue work; submit a
new job only after that review. The job worker is a bounded command, not a
remote Fleet node.

`subagent spawn` creates one scoped, exact-commit child worktree request bound
to its resolved provider profile and coding Harness. `subagent work` claims and
finishes queued records locally; `--max-agents` accepts `1-8`. Use `list`,
`inspect`, and `cancel` within the same local scope. Running cancellation is
cooperative. `mark-interrupted` requires a non-empty reason and `--yes`.
`cleanup --yes` removes clean child worktrees and preserves dirty ones for
review.

## Local service

```text
pandora service start
pandora service start --port 4317
pandora service start --port 0
```

`service start` hosts the same local runtime used by `pandora run` at a
loopback-only JSON-RPC endpoint. It writes one readiness object to standard
output, for example:

```json
{"endpoint":"http://127.0.0.1:4317/v1/rpc","token_path":".../service-token"}
```

The token is generated or reused under the configured data directory and is
never printed. Read it from `token_path` and send it as `Authorization: Bearer
<token>`. `--port 0` selects an available loopback port. The process remains in
the foreground until Ctrl-C; it does not listen on LAN addresses, daemonize,
or expose provider, MCP, package, or remote-execution methods.

Read-only work can complete without approval. Writes and process effects stop at
the approval boundary and expose an inspectable, redacted request subject.
`--harness` selects a built-in Harness or an admitted package profile by ID;
`coding` is an alias for the built-in `coding-domain` Harness. Package-backed
Domain profiles require the exact `--harness-version <version>` value. The
runtime rejects an unknown, unsupported, or unavailable profile before Gene
planning.
Direct `run` uses the Coding Domain Harness only for recognized action prefixes
such as `read:`, `search:`, `patch:`, `verify:`, `review:`, `deep-review:`,
`audit`, `debt`, `measure`, and `guide`. Natural-language tasks require
`run --agent`; Pandora does not silently route them to an unregistered default
Harness.
`harness run` accepts the same canonical catalog IDs and only runs a Domain
Harness with executable Genes. A package-backed profile is assembled only from
matching built-in Genes; its artifact is never loaded as code. A metadata-only
Source Harness is inspectable but returns a clear non-runnable error.
`harness inspect <id> --harness-version <version>` resolves an admitted Domain
or Meta profile through the same exact-version boundary. Domain profiles show
their executable Genes; Meta profiles show composition metadata and remain
non-runnable. Inspection does not enable a profile or grant runtime authority.
`harness list --json` reports those package-backed Domain and Meta profiles
separately under `admitted_profiles`; built-ins remain under `harnesses`.
The built-in `core-source` Harness is available through `harness list` and
`harness inspect core-source`; it binds the `pandora-runtime` constitutional
service and cannot be run as a task. Memory, Context, and Observability remain
internal engines in this release rather than separately installable Source
Harnesses.
`search:<query>` scans regular files under the configured workspace within the
runtime's entry and file-size limits, does not follow symlinks, and returns
matching paths with forward-slash separators.
After an operator approves a write, rerun the exact task with its approval ID.
The approval is bound to the original session, execution, Gene, and request
digest, is consumed atomically, and cannot be replayed for another task.
`session inspect` returns scoped session metadata, bounded event counts, the
latest evaluation receipt, and a derived observability summary without
returning event payloads. It also reports the count of bounded, redacted L1
execution-evidence records without exposing their content. Each execution
attempt stores its canonical events and evaluation receipt in one transaction. The
receipt evaluates trajectory and policy; outcome remains unavailable without
an explicit expected result. New CLI events include their recorded time; older
events remain explicitly untimestamped. `session resume` returns the evaluation
and evidence counts with the full bounded transcript.

Agent runs retrieve at most eight canonical L1 execution-evidence records from
the exact same session and provider. The context receipt lists generated record
identifiers only; evidence stays descriptive and non-cacheable.
The agent explicitly admits internal and sensitive context only; secret-classified
fragments are dropped before a provider request is assembled.

`chat` is a line-oriented interactive agent session. Type `/help` for the local
commands, `/session` to print the active session ID, `/approve` to approve and
resume the pending task, `/deny` to reject it, and `/exit` or `/quit` to close
it. Inspect the returned approval ID with `pandora approval inspect <id>`
before approving. Each other line is sent through the same bounded AgentLoop
and governed execution path as `run --agent`; the session is reused for later
turns. Coding slash commands resolve to an exact Harness, version, and Gene
before execution. Chat output is intended for terminals and rejects `--json`.
`tui` opens the full-screen terminal client in an interactive terminal. It uses
the same session, provider, AgentLoop, approval, and effect-policy path as
`run --agent`; it does not add a second runtime. Enter submits a task, Up and
Down browse task history, `/help` lists commands, `/session` shows the active
session, `/clear` clears the transcript, `/approve` approves and resumes the
pending task, `/deny` denies it, and Escape or Ctrl-C closes the client. The
in-memory transcript and task history are bounded; the session store remains
the source for later resume. The TUI accepts the same Coding slash commands as
the direct CLI and preserves quoted arguments such as `/read "My Project/README.md"`.
The TUI requires a real terminal and rejects `--json` and positional tasks.
`run --plan` sends the request to the active or explicitly selected provider as a bounded,
tool-free planning call. The planning request passes through the runtime's
`provider.invoke` permit boundary. Only a schema-validated task intent is passed
to the runtime; the model cannot execute tools or grant permissions. Configure
the active profile with `pandora provider set` and provide the environment
variable named by that profile for planning.

`run --agent` enables the bounded multi-turn loop. Each model request passes
through the same provider permit boundary. The model can call
`workspace.read`, `workspace.search`, `workspace.patch`, `workspace.verify`,
`daedalus.audit`, `argus.review`, `ariadne.debt`, and `hephaestus.measure`.
Each call is validated by the ToolEngine, routed through the same governed
runtime, and recorded in the session. Read and search use the current read-only
policy; patch and verify stop at the existing approval boundary before any
filesystem or process effect. The loop allows eight model turns and sixteen
tool calls by default. Set `--max-turns` and `--max-tools` to choose budgets for
a run; each value must be between 1 and 64 turns or 1 and 128 tool calls.
Tool output is marked as untrusted data before it reaches a provider. It cannot
serve as policy, authorization, or approval; a restored tool message without a
bound tool-call ID is rejected.
Agent mode cannot be combined with `--plan`, `--harness`, or `--gene`. User,
assistant, and tool messages are stored in
the scoped session database with fixed size limits; the system instruction is
rebuilt for every run. Reusing `--session <id>` restores that bounded
conversation before adding the new task. Credentials and hidden model
reasoning are not persisted. To continue an approved write, rerun
`run --agent --approval <id>` with the returned session ID and a follow-up task.
Pandora replays the bounded pending tool call through the same approval and
consumes it once.
Tool calls from one model reply are recorded and executed in sequence, so an
approval pause always preserves one exact pending call. Older persisted batches
are rejected as ambiguous instead of reusing one approval across multiple calls.

Every run accepts an optional `--task-class <name>` label. The default is
`general`; labels are bounded metadata and must not contain credentials or
prompt text. Pandora records measured tokens and elapsed time in the scoped
efficiency database. Provider pricing is recorded only when it is explicitly
available, so unknown cost is never treated as zero.

```text
pandora efficiency rank --task-class general --objective certainty
pandora efficiency rank --task-class coding --objective cost --json
pandora run --agent --task-class coding --optimize certainty "Fix the failing tests"
```

`efficiency rank` is read-only. It ranks existing evidence by cost, latency,
token usage, or verified completion rate. `run --agent` and `run --plan` may
use `--optimize cost|latency|tokens|certainty` to select a configured provider
profile for the current task class. The selector requires completed evidence,
uses only exact provider/model matches, and falls back to the active provider
when no suitable evidence exists. Cost selection ignores runs without explicit
pricing. The option cannot be combined with `--provider`, `--model`, or
`--approval`; it does not change configuration, policy, permissions, or
credentials.

Enabled Skills contribute bounded guidance to the rebuilt system instruction.
Only Skills explicitly in the `enabled` state are included. Their text is
reference material; it cannot grant permissions, change policy, satisfy an
approval, or execute scripts. Disabled and suspended Skills are not included.

## Discovery and completions

```text
pandora harness list
pandora harness inspect core-source
pandora harness inspect coding
pandora harness inspect coding-domain
pandora harness run coding --gene workspace.read --task "read:README.md"
pandora slash list
pandora slash resolve /audit
pandora /coding
pandora /read README.md
pandora /search "approval digest"
pandora /audit
pandora /argus-review crates/pandora-runtime/src/lib.rs
pandora /debt
pandora /measure
pandora /guide
pandora tool list
pandora tool inspect <id>
pandora skill list
pandora skill inspect <id>
pandora skill install <local-skill-directory>
pandora skill enable <id>
pandora skill suspend <id>
pandora skill disable <id>
pandora skill remove <id> --dry-run
pandora skill remove <id> --yes
pandora skill restore <id>
pandora package admit --manifest <manifest.json> --artifact <artifact>
pandora package install <id> [version] --registry <url>
pandora package install <id> [version] --registry <url> --token-env <name>
pandora package list
pandora package inspect <id> <version>
pandora package lock
pandora package verify-lock
pandora package remove <id> <version> --dry-run
pandora package remove <id> <version> --yes
pandora mcp set <id> --program <absolute-path> --arguments-json <json-array> --mode <auto|modern-only|legacy-only>
pandora mcp list
pandora mcp inspect <id>
pandora mcp remove <id> --yes
pandora orchestration roles
pandora strategies list
pandora completions powershell
pandora completions bash
pandora completions zsh
pandora completions fish
```

`/coding` inspects the built-in `coding-domain` Harness. Its short Gene aliases
are `/read`, `/search`, `/patch`, `/verify`, `/review`, `/audit`,
`/argus-review`, `/debt`, `/measure`, and `/guide`. Canonical commands remain
available as `/harness:<encoded-id>` and `/gene:<encoded-harness-id>:<encoded-gene-id>`.
An admitted custom Domain Harness uses exact-version commands such as
`/harness:owner%2Fdomain@1.0.0` and
`/gene:owner%2Fdomain@1.0.0:workspace.read`. Custom packages cannot claim the
built-in short aliases. Pandora lists only commands backed by available Genes.

Completion commands print a shell script. They describe the public command
surface and do not execute a command or inspect credentials.

`tool list` and `tool inspect` expose the built-in tool contract only: version,
name, required capability, operation, and input schema. They do not execute a
tool or bypass the governed execution path.

`mcp set`, `list`, `inspect`, and `remove` manage local stdio server profiles.
They do not start a server. List and inspection output omit argument values;
see [Local MCP stdio](MCP.md) for protocol and containment boundaries.

Skills are stored in the configured data directory under `skills/`. Use
`skill install <local-skill-directory>` to admit one local `SKILL.md` package.
The package directory name must match its manifest ID; manifests, resources,
regular files, and directories are validated before a staged copy. Existing
IDs and symlinks are rejected. The Skill root and its internal state and
removal directories must also be regular directories. Installed Skills start
disabled.
Use `--root <path>` to inspect another local skill root. Listing and inspection
read metadata, state, provenance, resources, and script inventory only; Skills
start disabled and scripts are not executed by these commands. `enable`,
`suspend`, and `disable` persist the explicit lifecycle state under the skill
root. `remove --dry-run` previews the skill path; `remove --yes` moves the
skill into the reversible removal area, and `restore` returns it as disabled.
These commands do not execute scripts; script execution remains available only
through the governed ToolEngine path.

Package records are addressed by exact ID and strict SemVer version. `package admit`
reads at most the local artifact limit plus one byte before it rejects an oversized
artifact.

`package install` reads current or exact-version metadata from an M-Place-compatible
registry, then downloads that release from the registry's exact-version endpoint.
The client follows no redirects, never contacts the package's `artifact_url`, and
reads at most the artifact limit plus one byte. Registry URLs must use HTTPS;
loopback HTTP is allowed for local development. Set `PANDORA_REGISTRY_URL` instead
of `--registry` if preferred. Public reads need no token. For a protected registry,
put the token in `PANDORA_REGISTRY_TOKEN` or name another environment variable with
`--token-env`; tokens are not accepted as command-line values.

Remote admission currently accepts Gene records with no unresolved capability
requirements and one valid Pandora runtime requirement. Other package kinds fail
before download or local mutation. A registry package remains metadata and verified
bytes: installation does not load native code, enable a Harness, issue a permit, or
grant runtime authority.

`package lock` writes the revalidated local package set to
`<workspace>/pandora.lock`; `--output <path>` selects another file. The lock keeps
the canonical manifests, exact versions, artifact hashes, dependencies, and trust
evidence in deterministic identity order. `package verify-lock` reads at most the
lockfile limit plus one byte and fails if the file is invalid or differs from the
current package store. Use `--lock <path>` to verify another file. Lock writes use
an atomic sibling replacement.

`package remove --dry-run` reports the admitted record without changing the local store;
`package remove --yes` removes it transactionally. Removal is refused when
another admitted package has a required dependency on the target, while
optional dependencies do not block removal. Package artifacts are local
admission evidence and are not executable authority. A signed manifest must include
an Ed25519 public key and signature over
`{id}:{version}:{publisher}:{content_hash}`. Local hexadecimal evidence remains
supported. Registry evidence retains its bare standard-base64 or lowercase
`base64:` representation. An `official` local claim is rejected until a publisher
trust root is configured.

## Configuration migration

```text
pandora migrate config --config <path>
pandora migrate config --config <path> --dry-run
```

Migration accepts the unversioned legacy fields `provider` (a URL string or an
object with `url`), `data_path`, and `workspace_path`, and writes the current
`format_version: 1` shape. A sibling `<path>.bak` backup is created before the
replacement. Existing backups are never overwritten, malformed input is left
untouched, and migration is one-way.

## Verified updates and uninstall

```text
pandora update --release v<version>
pandora update --release v<version> --dry-run
pandora update --artifact <path> --sha256 sha256:<64-hex-digits>
pandora update --artifact <path> --sha256 sha256:<64-hex-digits> --dry-run
pandora update --rollback
pandora uninstall --dry-run
pandora uninstall --yes
```

`update --release` accepts one explicit SemVer tag, selects the matching
Windows, macOS, or Linux asset from Pandora's official GitHub release, and
verifies it against that release's `checksums.txt` before staging it. It never
resolves an ambiguous latest release. Use `--dry-run` to verify a tag without
changing files.

`update --artifact` verifies a local artifact before staging it under the
Pandora data directory. A detached Ed25519 signature can be checked with
matching `--public-key <64-hex-digits>` and `--signature <128-hex-digits>`
options. The previous staged artifact is retained for one-step rollback. No
update operation executes an unverified artifact.

Uninstall requires `--yes` for deletion and preserves the configured workspace.
It refuses a data directory that contains that workspace, and removes a
configuration stored inside the data directory as part of that one data-root
deletion. Use `--dry-run` to inspect the exact paths first.
