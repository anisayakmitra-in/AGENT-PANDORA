# Skills

Skill admission is part of the CLI foundation. A Skill is a local package
with one `SKILL.md` file and an optional `scripts/` directory.

## Format

The document starts with a small front matter block:

```text
---
id: example
version: 0.1.0
name: Example Skill
description: Reads project guidance
publisher: example-publisher
resources: workspace.read, workspace.search
---
```

The `id` must match the package directory name. IDs are single path components;
the loader rejects separators, invalid characters, duplicate IDs, malformed
front matter, and paths that leave the skill root.

## Admission and state

`skill install <local-skill-directory>` validates one local package, copies it
into exclusive staging, and atomically admits the completed tree under the
configured Skills root. An existing destination or concurrent install is
rejected without replacing the existing Skill. The operation preserves the
source and does not enable the Skill. `SkillEngine::discover` validates metadata,
records the source path, and inventories script files without enabling the
Skill. New Skills start `disabled`; the supported state model is `disabled`,
`enabled`, `suspended`, and `removed`. The supported state changes are
`enable`, `suspend`, `disable`, and reversible `remove`/`restore`. The CLI
requires `--yes` for removal, supports `--dry-run`, and restores a removed
Skill as `disabled` in a later process.

Scripts are never executed by SkillEngine. Direct script execution returns an
error. Symlinked manifests, script directories, and script files are rejected.

Inspection reads the body and resource declarations only when requested.
State is kept under `.pandora-state`; removed packages are held under
`.pandora-removed` until a rollback receipt restores them.

## Agent use

An enabled Skill contributes bounded guidance to agent context. Skills are
reference material only: their resource labels do not grant capabilities, and
their scripts still require the governed ToolEngine path. Disabled and
suspended Skills are omitted. If enabled guidance exceeds the context limit,
the run fails closed instead of silently truncating the package.

Agent runs assemble the constitutional prompt and enabled Skill guidance through
`ContextEngine`. The JSON result includes a receipt with included and dropped
context item IDs, estimated token cost, and cache eligibility. Skill guidance
is sensitive and therefore non-cacheable. It is marked `admitted`, not
`verified`: admission makes its local provenance inspectable, never
authoritative.
