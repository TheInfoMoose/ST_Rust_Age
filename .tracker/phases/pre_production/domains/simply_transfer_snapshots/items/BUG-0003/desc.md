## Problem Description
`simply-transfer-snapshots/src/linux.rs:45` — `lvcreate -l 100%FREE` claims all free extents in the VG for one snapshot, starving other LVs and later snapshots — size from the origin LV (e.g. a % of origin) or make it configurable. Reported by Perigee's Phase 1 audit (vendor copy identical); verified present in ST at `b401f40` on 2026-11-18.
Migrated from the plan file's Parked list (2026-11-18).

## Reproduction Steps
n/a (parked finding; see Problem Description)

## Fix Log & Agent Notes
