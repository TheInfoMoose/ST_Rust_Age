## Problem Description
`simply-transfer-core/src/russh_client.rs:501-530` — `cancel_rx.changed()` returns Err immediately once the watch::Sender is dropped; inside the Pause loop this `continue`s forever (100% CPU, upload never returns) and in the main select! it wakes every iteration — suggested fix: none needed if STEP 5B deletes russh_client.rs as planned; otherwise treat a dropped sender as Cancel (see `wait_while_paused` in simply-transfer-link/src/client.rs).
Found during: STEP 4E validation (Gemini side finding).

## Reproduction Steps
1. Start an upload, send ControlSignal::Pause, then drop the watch::Sender.
2. Observe: the upload task spins at 100% CPU and never returns.

## Fix Log & Agent Notes
