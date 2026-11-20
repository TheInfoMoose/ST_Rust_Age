## Problem Description
`simply-transfer-crypto/src/lib.rs` `vault_key()` — (1) if the key path is a dangling symlink, `path.exists()` is false but `create_new` returns AlreadyExists, so the loop spins forever; (2) if `write_all` fails after `create_new` (disk full), a short key file is left and every later call returns "vault key file corrupt" until it is deleted by hand — suggested fix: cap the retry loop at 2 passes (use `symlink_metadata` for the existence check), and on write failure remove the partial file before returning the error.
Found during: STEP 3A validation.

## Reproduction Steps
1. In an isolated vault dir: `ln -s /nonexistent vault_<svc>.key`, then call `set_secret`.
2. Observe: the call never returns (100% CPU).

## Fix Log & Agent Notes
