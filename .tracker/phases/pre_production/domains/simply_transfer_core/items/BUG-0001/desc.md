## Problem Description
`simply-transfer-core/src/russh_client.rs:28-32` — `check_server_key` returns `Ok(true) // Accept all keys for now`, so any MITM is trusted — pin/verify against a known-hosts store (or retire this path once `simply-transfer-link` replaces SSH). Reported by Perigee's Phase 1 audit (vendor copy identical); verified present in ST at `b401f40` on 2026-11-18.
Migrated from the plan file's Parked list (2026-11-18).

## Reproduction Steps
n/a (parked finding; see Problem Description)

## Fix Log & Agent Notes
