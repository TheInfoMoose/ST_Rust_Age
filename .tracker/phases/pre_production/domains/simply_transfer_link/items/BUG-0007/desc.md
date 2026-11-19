## Problem Description
`simply-transfer-link/src/trust.rs:229-252` (`write_keys_file`) ignores every I/O error (create_dir_all, open, rename), and `TrustStore::add` returns `()` (as STEP 4C's spec fixed the signature), so a failed save leaves the key trusted in memory only and gone after restart, while the client was told `Paired` — change `add` to return `Result<(), LinkError>` and map a failure to `Response::Error`. Found in 4C validation.
Migrated from the plan file's Parked list (2026-11-18).

## Reproduction Steps
n/a (parked finding; see Problem Description)

## Fix Log & Agent Notes
