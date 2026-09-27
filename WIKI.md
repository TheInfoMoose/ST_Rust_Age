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
