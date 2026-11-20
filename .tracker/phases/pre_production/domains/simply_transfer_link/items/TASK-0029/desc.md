## Problem Description
Roadmap step `[CLAUDE] TWO-HOST RUN` (after STEP 4G): run `scripts/two-host-link-test.sh`, which starts two unprivileged ubuntu:24.04 containers on a private bridge network and runs `link_node serve` / `link_node client` (pair, roots, mkdir, traversal, 8 MiB upload, resume) over SSH + QUIC. Done when the last line is `TWO-HOST: PASS`. Target files: `scripts/two-host-link-test.sh`, `simply-transfer-link/examples/link_node.rs` (run only, no edits).
Re-filed from TASK-0006 (was in the inbox scope).

## Reproduction Steps
n/a (planned work)

## Fix Log & Agent Notes
