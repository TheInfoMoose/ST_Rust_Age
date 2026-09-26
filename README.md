# ST Rust Age (Simply Transfer v2)

<p align="center">
  <img src="assets/logo.png" width="600" alt="Simply Transfer v2 Logo">
</p>

[![Language: Rust](https://img.shields.io/badge/Language-Rust-dea584?logo=rust)](https://www.rust-lang.org/)
[![GUI: Slint](https://img.shields.io/badge/GUI-Slint-blue)](https://slint.dev/)
[![License: GPL-3.0](https://img.shields.io/badge/License-GPL--3.0-blue.svg)](LICENSE)
[![Rust CI](https://github.com/TheInfoMoose/ST_Rust_Age/actions/workflows/rust.yml/badge.svg?branch=main)](https://github.com/TheInfoMoose/ST_Rust_Age/actions/workflows/rust.yml)

A complete Rust rewrite of Simply Transfer. This modular, high-performance file transfer utility features cross-platform snapshot support, secure cryptographic validation, SSH capabilities, and a sleek GUI. Designed from the ground up for speed, reliability, and security.

---

## Architectural Overview

The project has been completely re-architected into a modular Rust workspace consisting of the following core crates:

- **`simply-transfer-core`**: The central engine governing transfer logic, SSH/SFTP client integrations, and transfer orchestration.
- **`simply-transfer-crypto`**: A dedicated cryptographic crate handling SHA-256 byte-level hash validation and secure token/secret management.
- **`simply-transfer-snapshots`**: A cross-platform capable snapshotting abstraction layer (with support modules for Windows VSS, Linux, and macOS) to ensure files can be backed up even while locked.
- **`simply-transfer-ui`**: A modern, lightweight, high-performance graphical user interface built utilizing [Slint](https://slint.dev/).

---

## Key Features

- **Pure Rust Implementation**: Zero-cost abstractions and memory safety guarantees for robust file operations.
- **High-Performance Transfers**: Asynchronous chunk processing tailored for large datasets and constrained networks.
- **Cross-Platform Snapshotting**: Seamlessly back up open and exclusively locked databases (such as live QuickBooks files on Windows) without interrupting the user.
- **Cryptographic Validation**: Two-step streaming SHA-256 hash validation ensures byte-for-byte integrity on the destination server.
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
