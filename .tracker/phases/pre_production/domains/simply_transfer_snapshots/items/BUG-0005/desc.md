## Problem Description
`simply-transfer-snapshots/src/windows.rs:50-52,97,135` — scripts are built with `format!` and run via `powershell -Command`, so `$(…)` and quotes in paths/ids expand — pass values as `-Command` parameters/`$args` or use the WMI/COM API directly. Reported by Perigee's Phase 1 audit (vendor copy identical); verified present in ST at `b401f40` on 2026-11-18.
Migrated from the plan file's Parked list (2026-11-18).

## Reproduction Steps
n/a (parked finding; see Problem Description)

## Fix Log & Agent Notes
