## Problem Description
`simply-transfer-snapshots/src/macos.rs:63` — `// TODO: proper resolution against the mounted APFS snapshot point`: the backend reports "APFS" but path resolution is a stub. Reported by Perigee's Phase 1 audit (vendor copy identical); verified present in ST at `b401f40` on 2026-11-18.
Migrated from the plan file's Parked list (2026-11-18).

## Reproduction Steps
n/a (parked finding; see Problem Description)

## Fix Log & Agent Notes
