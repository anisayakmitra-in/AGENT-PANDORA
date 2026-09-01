# Patched glib 0.18.5

This directory contains the exact `glib 0.18.5` source published on crates.io,
plus the upstream fix for RUSTSEC-2024-0429 / GHSA-wrw7-89jp-8q8g.

## Provenance

- crates.io package: `glib 0.18.5`
- crates.io archive SHA-256:
  `233daaf6e83ae6a12a52055f568f9d7cf4671dabb78ff9560ab6da230ce00ee5`
- crates.io VCS revision: `42b9caf98e03ded086362d9653ca58fe94dc8658`
- upstream fix: gtk-rs/gtk-rs-core pull request 1343, merged as
  `05dff0e`

The patch changes the `g_variant_get_child` out-argument from an immutable
pointer reference to a mutable pointer reference in
`glib/src/variant_iter.rs`. No other upstream source is changed.

## Why this is vendored

The RustSec fix is released in `glib 0.20.0`, but Tauri 2.11.5 and Wry 0.55.1
still require the final GTK3 `gtk 0.18` dependency line. Cargo therefore cannot
resolve `glib 0.20` without Tauri's pending GTK4 migration. A source override
keeps the supported Linux desktop while removing the undefined behavior.

Retire this directory and the `[patch.crates-io]` entry as soon as the supported
Tauri/Wry release resolves `glib 0.20` or newer on Linux.
