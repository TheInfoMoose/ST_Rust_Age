## Problem Description
`simply-transfer-core/src/engine.rs:526` accepts any line starting with `mock_hash`, and `ssh.rs:148` emits `mock_hash` — a peer can pass verification without a real hash — remove the mock path from production code (test-only behind `#[cfg(test)]`). Reported by Perigee's Phase 1 audit (vendor copy identical); verified present in ST at `b401f40` on 2026-11-18.
Migrated from the plan file's Parked list (2026-11-18).

## Reproduction Steps
n/a (parked finding; see Problem Description)

## Fix Log & Agent Notes
