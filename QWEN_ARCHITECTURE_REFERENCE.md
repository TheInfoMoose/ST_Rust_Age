# Simply Transfer V2 - Qwen Architecture Reference

## Current State (Phase 1 Complete)
- **Engine**: Fully migrated to `russh` & `russh-sftp` (pure Rust). The `openssl`/`libssh2` C-dependencies have been completely excised to fix Windows build stalls. 
- **Async Runtime**: The core transfer engine and SSH traits use 100% async Tokio. Synchronous UI closures wrap async tasks via `tokio::spawn`.
- **Key Auth Mechanism**: `russh_keys::decode_secret_key` requires the **raw string contents** of a PEM file, not the path. UI paths use `std::fs::read_to_string(&tmp_pem)` to inject keys.

## Core Component Interaction
1. **simply-transfer-ui**: Slint frontend. Operates a synchronous event loop. Mutates state exclusively via `slint::invoke_from_event_loop`. Spawns `tokio` tasks for all networking.
2. **simply-transfer-core**: `TransferEngine` manages the Pipeline (Snapshot -> Validation -> Transfer). Implements `SshClient` via `RusshClient`. 
3. **simply-transfer-crypto**: Generates Ed25519 keypairs. Interacts with native OS keyrings. Provides Base64 XOR token obfuscation for initial handshakes.
4. **simply-transfer-snapshots**: File delta calculations and hashing to prevent redundant wire transfers.

## Next Steps (Phase 2 - Cryptographic Heartbeat)
- **Goal**: Implement a secondary, independent SSH session that continuously validates peer identity during a transfer.
- **Design**:
  - The heartbeat session will periodically (e.g., every 5s) execute a challenge-response over a dedicated SSH channel.
  - If the heartbeat fails, the main `TransferEngine` must cleanly abort the transfer via the `ControlSignal` channel.
  - Heartbeat logic should be abstracted into a `HeartbeatMonitor` struct in `simply-transfer-core`.

## Important Rules for Future Development
- **No Blocking in UI**: All file I/O or network calls triggered by Slint handlers MUST be wrapped in `tokio::spawn`.
- **Pure Rust Dependencies**: Do not re-introduce dependencies requiring a C++ toolchain (e.g., `openssl`) to ensure Windows cross-compilation remains robust.
- **Error Handling**: Use `tracing::error!` and propagate `EngineError` or `SshError` types up to the UI. Do not panic on network drops.
