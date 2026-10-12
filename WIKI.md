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
- Orchestrates the actual data transfer and integrates an embedded P2P `russh` server daemon, bypassing OS-level OpenSSH dependencies.
- Pivots bulk data transit to a dedicated Out-of-Band (OOB) QUIC channel (`quinn`) to saturate Gigabit links and bypass SSH packet fragmentation.
- Implements the 3-phase process: **Snapshot -> Transmission -> Integrity Validation**.
- **Key Files:** `src/engine.rs` (Transfer Engine), `src/ssh.rs` (SSH Client abstractions), `src/russh_server.rs` (Embedded Daemon).

### `simply-transfer-crypto`
Handles all cryptographic operations.
- Ed25519 SSH Keypair generation and storage (via the native OS keyring).
- Base64 XOR-obfuscated `ConnectionToken` generation and parsing.
- Secure hashing for file integrity verification.

### `simply-transfer-snapshots`
Responsible for calculating changes between directories to minimize data transfer over the wire.
- Uses `FallbackSnapshotDriver` to generate manifests and diffs.

## 3. Security Model
- **Authentication:** Peer-to-peer SSH authentication. The source generates an Ed25519 keypair and creates a token. The destination ingests this token into the embedded `russh` daemon's memory (no `~/.ssh/authorized_keys` required), and the source initiates the connection.
- **Auditing:** CI is equipped with `cargo-audit` to block builds if active vulnerabilities are detected in any crates. Exceptions are explicitly managed and documented.

## 4. Development Workflow
1. **Local Testing:** Run `cargo test --all-features` to ensure logic is sound. Note: Stress tests (`test_stress_*`) utilize temporary directories to avoid permission conflicts on local systems.
2. **Formatting & Linting:** Strict enforcement of `cargo fmt` and `cargo clippy -- -D warnings`.
3. **CI Pipelines:** Automated testing on MacOS, Windows, and Ubuntu.

## 5. Roadmap & Future Work
- **Multi-transfer UI Decoupling:** Decouple global UI setup models so users can configure Connection B while Connection A is actively transmitting.
- **Dependency Management:** Address technical debt and unmaintained crates (e.g., replacing `ttf-parser` dependency in the UI).

---
*For contributing guidelines, please refer to the main repository documentation.*

## 6. Project Tracker & Implementation Steps

This section serves as a living document to track the current state of features, what needs work, and what remains to be implemented. For a detailed breakdown of items prioritized by dependency, necessity, and resource expenditure across our 5-phase roadmap, see the [Task Hierarchy & Phase Progression](TASK_HIERARCHY.md).

### ✅ Features Enabled and Functioning
- **Cross-Platform Automated CI/CD:** Fully operational GitHub Actions pipelines.
- **Pre-Release Workflow:** Automated pre-release tags and cache cleanup.
- **UI Persistence & State:** Connections list successfully saving/loading.
- **Out-Of-Band (OOB) Handshake:** Token generation, parsing, and TCP handshake.
- **P2P Embedded SSH Daemon:** App hosts a dedicated `russh` daemon, securely validating OOB Ed25519 keys without OS-level `sshd` dependencies.
- **Dedicated QUIC Data Channel:** Bulk data is streamed over a direct `quinn` UDP/QUIC socket to bypass SSH TCP packet bottlenecks and saturate Gigabit Ethernet links.
- **Network Resiliency:** Exponential backoff, socket re-negotiation, and atomic `seek()` alignment for resuming interrupted transfers cleanly.
- **Native Hash Validation:** Instead of OS shell hashing, the destination daemon intercepts `simply-transfer-hash|` commands and natively computes local hashes using a 1MB buffer with large (600s) SSH execution timeouts.

### 🚧 Started, but Needing Work (In Progress)
- **Multi-transfer UI Decoupling:** Decouple `connections.json` models from active background transfers.

### 🛑 Scaffolded, but Not Implemented
- **PowerShell UAC / Console Masking:** Hide visible external PowerShell console windows during Windows administrative key ingestion.

### ❓ Clarifications & Verifications Needed
1. **Firewalls & NAT Traversal:** The temporary TCP listener binds to a random port. We need to verify if Windows Firewall (or equivalent) will prompt the user and potentially block the handshake. We may need to look into UPnP or hole-punching for true P2P across different subnets.
2. **Keyring Services:** Verify the reliability of the native OS keyring integration across different Linux desktop environments (GNOME Keyring vs KWallet) and Windows Credential Manager.
