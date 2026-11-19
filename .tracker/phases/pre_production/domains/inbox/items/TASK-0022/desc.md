## Problem Description
Imported from roadmap step `STEP 2C`: Stop reading/writing OS authorized_keys; quiet auth log; 0600 atomic key-store writes; POST_GATE exit 0. Leftover: simply-transfer-crypto/src/token.rs:7 encode_utf16le_base64 now has no callers (outside 2C scope)
- Step ID: `STEP 2C`
- Command: `/goal`
- Roadmap order: completed
- Target files:
  - `simply-transfer-core/src/russh_server.rs`
  - `simply-transfer-ui/src/main.rs`

## Reproduction Steps
1. Inspect `.tracker`.
2. Run tests to verify behavior.

## Fix Log & Agent Notes
- Target scope path: `.tracker`
