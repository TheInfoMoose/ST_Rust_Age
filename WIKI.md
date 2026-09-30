# Simply Transfer V2 - Development Wiki

Welcome to the Simply Transfer V2 Wiki. This document serves as a comprehensive map of the application's architecture, development lifecycle, and roadmap.

## 1. Project Overview
Simply Transfer V2 is a high-performance, secure, and cross-platform file transfer utility designed to handle large files and high file-count directories reliably. It combines a robust Rust core (`simply-transfer-core`) with a sleek, responsive UI powered by Slint (`simply-transfer-ui`).

### Core Objectives:
- **Security First:** End-to-end encryption, SSH-backed transport, and active dependency vulnerability monitoring.
- **Reliability:** Chunked file hashing, snapshot-driven change detection, and resumption of interrupted transfers.
- **Performance:** Asynchronous execution using Tokio, designed to scale with extreme workloads (e.g., thousands of files or massive sparse files).

## 2. Architecture & Modules

### `simply-transfer-ui`
The frontend, written using Slint and Rust. 
- Manages user interactions, connection configurations, and live transfer progress.
- Spawns background tasks (`TransferEngine`) and communicates with them via `tokio::sync::mpsc` channels to ensure the UI remains non-blocking.
- **Key Files:** `src/main.rs`, `src/network.rs`, `ui/main.slint`.

### `simply-transfer-core`
The backbone of the application.
- Orchestrates the actual data transfer.
- Implements the 3-phase process: **Snapshot -> Validation -> Transmission -> Integrity Check**.
- **Key Files:** `src/engine.rs` (Transfer Engine), `src/ssh.rs` (SSH Client abstractions).

### `simply-transfer-crypto`
Handles all cryptographic operations.
- Ed25519 SSH Keypair generation and storage (via the native OS keyring).
- Base64 XOR-obfuscated `ConnectionToken` generation and parsing.
- Secure hashing for file integrity verification.

### `simply-transfer-snapshots`
Responsible for calculating changes between directories to minimize data transfer over the wire.
- Uses `FallbackSnapshotDriver` to generate manifests and diffs.

## 3. Security Model
- **Authentication:** Peer-to-peer SSH authentication. The source generates an Ed25519 keypair and creates a token. The destination ingests this token into `~/.ssh/authorized_keys` and the source initiates the connection.
- **Auditing:** CI is equipped with `cargo-audit` to block builds if active vulnerabilities are detected in any crates. Exceptions are explicitly managed and documented.

## 4. Development Workflow
1. **Local Testing:** Run `cargo test --all-features` to ensure logic is sound. Note: Stress tests (`test_stress_*`) utilize temporary directories to avoid permission conflicts on local systems.
2. **Formatting & Linting:** Strict enforcement of `cargo fmt` and `cargo clippy -- -D warnings`.
3. **CI Pipelines:** Automated testing on MacOS, Windows, and Ubuntu.

## 5. Roadmap & Future Work
- **Performance Enhancements:** Parallelize chunk hashing over multiple threads.
- **Network Resiliency:** Improve automatic reconnection logic for spotty network environments.
- **Dependency Management:** Address technical debt and unmaintained crates (e.g., replacing `ttf-parser` dependency in the UI).

---
*For contributing guidelines, please refer to the main repository documentation.*

## 6. Project Tracker & Implementation Steps

This section serves as a living document to track the current state of features, what needs work, and what remains to be implemented.

### ✅ Features Enabled and Functioning
- **Cross-Platform Automated CI/CD:** Fully operational GitHub Actions pipelines (`release.yml` and `clear-cache.yml`), including matrix builds for Windows, macOS (Silicon & Intel), and Linux.
- **Pre-Release Workflow:** Manual builds and pushes now automatically generate dynamic pre-release tags (e.g., `dev-build-<run_number>`), with automated cleanup of Actions cache.
- **UI Persistence & State:** Connections list is now successfully saving/loading locally, overcoming the previous UI state issues.
- **Out-Of-Band (OOB) Handshake:** Token generation, token parsing, and initial cryptographic handshake between peers over a temporary TCP port.
- **SSH Keypair Management:** The application can generate Ed25519 keys, obfuscate them into tokens, and store the keys.

### 🚧 Started, but Needing Work (In Progress)
- **Data Transmission (Transfer Engine):** The scaffolding for the chunked file hashing and transmission exists, but the core data streaming needs further optimization and hardening.
- **UI Progress Streaming:** Passing real-time transfer stats (MB/s, ETA) from the background `TransferEngine` to the Slint UI safely without blocking the main event loop.
- **Snapshot Diffing Logic:** The `FallbackSnapshotDriver` is scaffolded, but its integration with the actual transfer phase to skip existing/unchanged files needs refinement.

### 🛑 Scaffolded, but Not Implemented
- **Network Resiliency & Reconnection:** Logic to handle spotty network drops and resume a transfer exactly where it left off.
- **Strict Pre-flight Checks:** Comprehensive checks before transferring (e.g., checking destination disk space, checking if host `sshd` is actually running and accessible).
- **Cryptographic Heartbeat:** Continuous verification during the transfer to ensure the connection hasn't been hijacked.
- **[Multi-transfer Setup](FUTURE_ISSUE_MULTITRANSFER_SETUP.md):** Decouple UI state so the setup screen can be mapped to specific connections, allowing setup while another transfer is active.
- **[Transport Efficiency Profiling](FUTURE_ISSUE_TRANSPORT_EFFICIENCY.md):** Identify bottleneck in SSH chunking/writes that caps Gigabit speeds at 2-3MB/s and implement pipeline optimizations.

### ❓ Clarifications & Verifications Needed
1. **OS Permissions for `~/.ssh`:** We need to verify that the application has the necessary permissions to write to `~/.ssh/authorized_keys` across all target OS environments (especially Windows/macOS strict sandbox modes) without requiring the user to manually intervene or run as root/Administrator.
2. **SSH Daemon Dependency:** We need to clarify if the destination machine is required to have a native SSH Server (`sshd`) actively running on port 22. If so, we need to explicitly prompt the user to enable it, or bundle a lightweight internal SSH server in the application.
3. **Firewalls & NAT Traversal:** The temporary TCP listener binds to a random port. We need to verify if Windows Firewall (or equivalent) will prompt the user and potentially block the handshake. We may need to look into UPnP or hole-punching for true P2P across different subnets.
4. **Keyring Services:** Verify the reliability of the native OS keyring integration across different Linux desktop environments (GNOME Keyring vs KWallet) and Windows Credential Manager.
