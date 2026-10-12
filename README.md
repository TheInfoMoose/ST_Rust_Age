# ST Rust Age (Simply Transfer v2)

<p align="center">
  <img src="assets/logo.png" width="600" alt="Simply Transfer v2 Logo">
</p>

<p align="center">
  <a href="https://github.com/TheInfoMoose/ST_Rust_Age/actions/workflows/ci.yml"><img src="https://github.com/TheInfoMoose/ST_Rust_Age/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI Build"></a>
  <a href="https://github.com/TheInfoMoose/ST_Rust_Age/actions/workflows/release.yml"><img src="https://github.com/TheInfoMoose/ST_Rust_Age/actions/workflows/release.yml/badge.svg" alt="Release Build"></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Language-Rust-dea584?logo=rust" alt="Language: Rust"></a>
  <a href="https://slint.dev/"><img src="https://img.shields.io/badge/GUI-Slint-blue" alt="GUI: Slint"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-GPL--3.0-blue.svg" alt="License: GPL-3.0"></a>
</p>

A complete Rust rewrite of Simply Transfer. This modular, high-performance file transfer utility features cross-platform snapshot support, secure cryptographic validation, SSH capabilities, and a sleek GUI. Designed from the ground up for speed, reliability, and security.

---

## Architectural Overview

The project has been completely re-architected into a modular Rust workspace consisting of the following core crates:

- **`simply-transfer-core`**: The central engine governing transfer logic, orchestrating the embedded P2P `russh` server daemon, and managing the high-throughput OOB QUIC data channel.
- **`simply-transfer-crypto`**: A dedicated cryptographic crate handling SHA-256 byte-level hash validation and secure token/secret management.
- **`simply-transfer-snapshots`**: A cross-platform capable snapshotting abstraction layer (with support modules for Windows VSS, Linux, and macOS) to ensure files can be backed up even while locked.
- **`simply-transfer-ui`**: A modern, lightweight, high-performance graphical user interface built utilizing [Slint](https://slint.dev/).

---

## Key Features

- **Pure Rust Implementation**: Zero-cost abstractions and memory safety guarantees. Excision of C-dependencies ensures seamless cross-compilation across Windows, macOS, and Linux.
- **High-Performance QUIC Transport**: Bypasses traditional SSH packet fragmentation bottlenecks by pivoting bulk data to a multiplexed, UDP-backed TLS 1.3 `quinn` stream, saturating Gigabit Ethernet links.
- **Embedded P2P Daemon**: Hosts its own internal `russh` SSH server to ingest Ed25519 keys via OOB TCP handshake. No dependency on OS-level OpenSSH servers or `~/.ssh/authorized_keys`.
- **Native Hash Validation**: Two-step streaming SHA-256 hash validation natively computed on the destination daemon (bypassing OS shells) ensures byte-for-byte integrity.
- **Modular and Extensible**: Decoupled architecture allows components to be reused or replaced independently.

---

## Getting Started

### Prerequisites

- [Rust Toolchain](https://rustup.rs/) (cargo, rustc)
- C++ Build Tools (if building on Windows)

### Build Instructions

To build the full application (including the Slint UI), simply run:

```bash
cargo build --release
```

The resulting executable will be available in the `target/release/` directory.

---

## License

This project is licensed under the [GNU General Public License v3](LICENSE).
