**Repo:** Simply Transfer (ST_Rust_Age), a Rust file-transfer app. Crates: `simply-transfer-core` (engine, SSH client/server), `simply-transfer-crypto`, `simply-transfer-snapshots` (LVM/VSS/APFS), `simply-transfer-link` (new SSH-control + QUIC-data transport), `simply-transfer-ui`. Current plan: security triage, test repair, and the `simply-transfer-link` extraction. No new features beyond what the fixes need.

**Source of truth:** the step's Argon block in `/workspace/STEPPED_IMPLEMENTATION_PLAN.md`. The plan header's "Who runs what" section applies. Steps marked "Claude re-splits before it runs" have outline blocks only: report `Blocked` if your step is one of them.

**Rules:**
- Gates (run on the host by the operator): `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- Security work: never add a shell (`sh -c`, `cmd /c`, `powershell -Command` with interpolated values) or a trust-everything shortcut. Fail closed.
- Tests must not touch the real system (no real disks, snapshots, `~/.ssh` or network beyond loopback).
- Perigee vendors these crates. Don't edit anything outside this repo, and never edit the plan file.
- Test data uses placeholders only (`testuser`, `example.com`, `192.0.2.0/24`).
