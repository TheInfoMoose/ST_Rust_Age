## Problem Description
`simply-transfer-core/src/firewall.rs:605` — the VPN manager test builds `SecureStorage::new("simply_transfer_vpn_mgr_test")`, which uses the real OS keyring and writes `~/.simply-transfer/vault_simply_transfer_vpn_mgr_test.enc` (and, after STEP 3A, a matching `.key` file) into the real home directory during `cargo test` — suggested fix: use `SecureStorage::isolated(.., tempdir)` as the crypto tests do since STEP 1B.
Found during: STEP 3A pre-check.

## Reproduction Steps
1. `cargo test -p simply-transfer-core`
2. Observe: `ls ~/.simply-transfer/` shows `vault_simply_transfer_vpn_mgr_test.enc`.

## Fix Log & Agent Notes
