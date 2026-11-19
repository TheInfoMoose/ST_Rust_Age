## Problem Description
Imported from roadmap step `STEP 2G`: Write-path guard, QUIC accept timeout, delete ssh_server.rs. Hand-off added relative-path and $HOME/.<x>-subtree rules; correction (strike 0, spec gap) made Windows matching case-insensitive and blocked \\ paths. POST_GATE ssh_server grep fixed to -qw (Claude's gate bug; snapshot plan hash refreshed). Windows not verified locally
- Step ID: `STEP 2G`
- Command: `/goal`
- Roadmap order: completed
- Target files:
  - `simply-transfer-core/src/russh_server.rs`
  - `simply-transfer-core/src/ssh_server.rs`
  - `simply-transfer-core/src/lib.rs`

## Reproduction Steps
1. Inspect `simply-transfer-core`.
2. Run tests to verify behavior.

## Fix Log & Agent Notes
- Target scope path: `simply-transfer-core`
