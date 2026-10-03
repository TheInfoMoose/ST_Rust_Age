# Simply Transfer V2 - Qwen Architecture Reference

## Current State (Phases 1, 2, & 3 Complete)
- **Engine**: Fully migrated to `russh` (pure Rust). The `openssl`/`libssh2` C-dependencies have been completely excised to fix Windows build stalls. 
- **P2P Daemon**: The system now embeds a custom `russh` server daemon inside `simply-transfer-core` that validates OOB-exchanged Ed25519 keys, completely bypassing OS-level OpenSSH dependencies.
- **Async Runtime**: The core transfer engine and SSH traits use 100% async Tokio. The `simply-transfer-ui` uses a dedicated background thread for the Tokio runtime, cleanly decoupled from the Slint synchronous UI event loop to prevent drop panics.
- **Key Auth Mechanism**: `russh_keys::decode_secret_key` requires the **raw string contents** of a PEM file.
- **Telemetry & Validation**: Slint UI receives real-time progress via `mpsc` channels. Hashes are calculated and verified in batches. Cryptographic Heartbeat continuously validates the connection.

## Core Component Interaction
1. **simply-transfer-ui**: Slint frontend. Operates a synchronous event loop. Mutates state exclusively via `slint::invoke_from_event_loop`. Spawns `tokio` tasks on a dedicated background runtime.
2. **simply-transfer-core**: `TransferEngine` manages the Pipeline (Snapshot -> Validation -> Transfer). Implements `SshClient` via `RusshClient`. Also hosts the `P2P_SERVER` global daemon listener.
3. **simply-transfer-crypto**: Generates Ed25519 keypairs. Interacts with native OS keyrings. Provides Base64 XOR token obfuscation.
4. **simply-transfer-snapshots**: File delta calculations and hashing to prevent redundant wire transfers.

## Next Steps (Phase 4 - High-Performance Transport)
- **Goal (Phase 4)**: Ditch generic SFTP protocol for data transit. SFTP's TCP packet limitations cap throughput severely. Instead, we are building a Custom Binary Stream transit over the SSH channel that pipes pure binary data from the source daemon straight to the destination daemon (bypassing Windows path-escaping bugs entirely). This phase also tackles network resiliency (exponential backoff, byte-level resume) and UI decoupling for multi-transfer setups.

## Important Rules for Future Development
- **No Blocking in UI**: All file I/O or network calls triggered by Slint handlers MUST be wrapped in `tokio::spawn`.
- **Pure Rust Dependencies**: Do not re-introduce dependencies requiring a C++ toolchain (e.g., `openssl`) to ensure Windows cross-compilation remains robust.
- **Error Handling**: Use `tracing::error!` and propagate `EngineError` or `SshError` types up to the UI. Do not panic on network drops.
