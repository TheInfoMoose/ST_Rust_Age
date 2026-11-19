## Problem Description
`simply-transfer-link/src/trust.rs:231-235` — `write_keys_file` sets the trust file's parent dir to 0700 on every save; if the caller passes a path in a shared dir (e.g. the app's data dir or cwd), that silently changes its permissions — create a dedicated subdir, or only chmod a dir it created. Found in 4C validation.
Migrated from the plan file's Parked list (2026-11-18).

## Reproduction Steps
n/a (parked finding; see Problem Description)

## Fix Log & Agent Notes
