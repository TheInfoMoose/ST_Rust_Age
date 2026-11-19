## Problem Description
Imported from roadmap step `STEP 2B`: UI issues the secret in the token and sends it on verify. Spec amended in correction (strike 0, spec gap): no-secret tokens refused in a match guard at parse, before side effects; POST_GATE exit 0
- Step ID: `STEP 2B`
- Command: `/goal`
- Roadmap order: completed
- Target files:
  - `simply-transfer-ui/src/main.rs`

## Reproduction Steps
1. Inspect `simply-transfer-ui`.
2. Run tests to verify behavior.

## Fix Log & Agent Notes
- Target scope path: `simply-transfer-ui`
