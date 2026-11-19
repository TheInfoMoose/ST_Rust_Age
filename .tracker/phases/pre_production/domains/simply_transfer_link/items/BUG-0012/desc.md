## Problem Description
`simply-transfer-link/src/data.rs` `handle_connection` — `open_for_write`, `write_all`, `flush` and the post-write sha256 are blocking std::fs calls on a tokio worker; a slow disk stalls other connections — use `spawn_blocking` or tokio::fs. Found in 4D validation.
Migrated from the plan file's Parked list (2026-11-18).

## Reproduction Steps
n/a (parked finding; see Problem Description)

## Fix Log & Agent Notes
