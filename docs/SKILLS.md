# Skills

Skill admission is part of the Anubis CLI foundation. A Skill is a local package
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

`SkillEngine::discover` validates package metadata, records the source path,
and inventories script files without enabling the Skill. New Skills start
`disabled`; the state model also distinguishes `verified`, `installed`,
`enabled`, `suspended`, and `removed`. The supported state changes are
`enable`, `suspend`, `disable`, and reversible `remove`/`rollback`.

Scripts are never executed by SkillEngine. Direct script execution returns an
error. Symlinked manifests, script directories, and script files are rejected.

Inspection reads the body and resource declarations only when requested.
State is kept under `.pandora-state`; removed packages are held under
`.pandora-removed` until a rollback receipt restores them.
