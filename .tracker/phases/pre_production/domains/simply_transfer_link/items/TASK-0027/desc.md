## Problem Description
Imported from roadmap step `STEP 4A`: simply-transfer-link skeleton + framed protocol codec. Hand-off pinned exact Request/Response fields, added Roots and VolumeEntry, in-crate hex32 serde. Correction (strike 1): hex32 deserialized a borrowed &str (broke from_value/from_reader) → owned String + from_value test
- Step ID: `STEP 4A`
- Command: `/goal`
- Roadmap order: completed
- Target files:
  - `Cargo.toml`
  - `Cargo.lock`
  - `simply-transfer-link/Cargo.toml`
  - `simply-transfer-link/src/lib.rs`
  - `simply-transfer-link/src/protocol.rs`

## Reproduction Steps
1. Inspect `simply-transfer-link`.
2. Run tests to verify behavior.

## Fix Log & Agent Notes
- Target scope path: `simply-transfer-link`
