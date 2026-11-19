## Problem Description
`simply-transfer-link/src/trust.rs:197-205` — `load_keys_file` treats unreadable/corrupt JSON as an empty store (fail-closed, fine), but the next `add` rewrites the file, destroying every previously trusted key — keep the corrupt file aside (`.corrupt`) or refuse to save until it's fixed. Found in 4C validation.
Migrated from the plan file's Parked list (2026-11-18).

## Reproduction Steps
n/a (parked finding; see Problem Description)

## Fix Log & Agent Notes
