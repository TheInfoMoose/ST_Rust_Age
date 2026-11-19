## Problem Description
Imported from roadmap step `STEP 2D`: Native os, size, df server verbs + mock support. Hand-off clarified resolve_owning_volume returns a mount point and the native_df test arg type; POST_GATE exit 0. Note: Linux volume list skips tmpfs, so df on a tmpfs path reports its parent mount
- Step ID: `STEP 2D`
- Command: `/goal`
- Roadmap order: completed
- Target files:
  - `simply-transfer-core/src/russh_server.rs`
  - `simply-transfer-core/src/ssh.rs`

## Reproduction Steps
1. Inspect `simply-transfer-core`.
2. Run tests to verify behavior.

## Fix Log & Agent Notes
- Target scope path: `simply-transfer-core`
