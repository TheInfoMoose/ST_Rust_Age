## Problem Description
`simply-transfer-snapshots/src/linux.rs:61-63,112-113` — builds `/dev/mapper/*-{id}` and runs it through `sh -c`, so the glob and the interpolated id go through a shell — resolve the device with `lvs`/`/dev/<vg>/<lv>` and call `mount`/`lvremove` with argv, no shell. Reported by Perigee's Phase 1 audit (vendor copy identical); verified present in ST at `b401f40` on 2026-11-18.
Migrated from the plan file's Parked list (2026-11-18).

## Reproduction Steps
n/a (parked finding; see Problem Description)

## Fix Log & Agent Notes
