## Problem Description
`simply-transfer-link/src/data.rs:~196` (`handle_connection`) — calls `std::fs::create_dir_all(grant.path.parent())` before `paths::open_for_write`; not in STEP 4D's spec. Between BeginUpload's `resolve(Access::Write)` and this call, a symlink swapped into the path makes create_dir_all follow it (only the final component gets O_NOFOLLOW) — create missing dirs with a no-follow walk under the share root (or reject missing parents and add a Mkdir step). Found in 4D validation.
Migrated from the plan file's Parked list (2026-11-18).

## Reproduction Steps
n/a (parked finding; see Problem Description)

## Fix Log & Agent Notes
