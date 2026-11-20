## Problem Description
`simply-transfer-core/src/firewall.rs:520-526` — `teardown_tunnel` on Linux runs `wg-quick down <conf>` directly, while bring-up uses pkexec (or sudo -n). As a normal user `wg-quick down` fails, the error is discarded (`let _ =`), and the conf file is then deleted, so the WireGuard interface stays up with no conf left to tear it down — suggested fix: mirror the bring-up privilege path (pkexec, then sudo -n) and only remove the conf after a successful down; surface the failure in the returned Result.
Found during: STEP 3B pre-check.

## Reproduction Steps
1. Bring up a non-persistent tunnel as a normal user (pkexec prompt accepted).
2. Let the job finish so teardown_tunnel runs.
3. Observe: `ip link show stwg<id>` still lists the interface; `~/.simply-transfer/run/stwg<id>.conf` is gone.

## Fix Log & Agent Notes
