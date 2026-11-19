## Problem Description
Imported from roadmap step `STEP 2F`: Unknown exec runs nothing (exit 127); UI shell browse fallbacks deleted. Hand-off added: log command text only for known verbs. main.rs rustfmt churn verified token-identical; POST_GATE exit 0
- Step ID: `STEP 2F`
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
