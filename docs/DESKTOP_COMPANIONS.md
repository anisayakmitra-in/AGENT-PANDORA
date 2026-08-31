# Desktop companion contract

Pandora companions are optional local presentation. They do not receive or
read prompts, memories, secrets, receipts, workspace files, tool output,
private reasoning, network data, or provider messages. They cannot invoke a
tool, affect routing or evaluation, grant a capability, request or resolve an
approval, issue a permit, or alter any runtime record.

The built-in Pandora Orbit companion is off by default and maps only these
typed public UI states:

- `idle`: no current governed result;
- `working`: a public request-in-flight flag is true;
- `waiting`: an exact approval-required status is public;
- `success`: the latest public run status is completed;
- `failure`: the latest public run status is failed, denied, or cancelled.

Settings store only enabled state, bottom-left or bottom-right position,
small/medium/large scale, and system/static motion. Fixture preview selection
is not persisted. System reduced-motion always removes nonessential movement,
and the text status remains available to assistive technology.

## Declarative packs

`apps/pandora-desktop/src/companion.ts` defines schema version 1. A manifest has
an ID, label, and one bundled `.png`, `.webp`, or `.svg` path for every typed
state. The host must load SVG through an image boundary, never inject it as
markup. A pack contains no script, command, callback, URL, or runtime hook.

Admission rejects missing states, absolute or remote URLs, traversal and
backslash paths, non-regular files, symlinks, executable files, assets larger
than 256 KiB, and packs larger than 1 MiB. Invalid persisted display settings
fall back to the disabled built-in configuration.
