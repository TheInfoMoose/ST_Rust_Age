# Simply Transfer v1.0.0 (ST Rust Age)

<p align="center">
  <img src="assets/logo.png" width="600" alt="Simply Transfer Logo">
</p>

<p align="center">
  <a href="https://github.com/TheInfoMoose/ST_Rust_Age/actions/workflows/ci.yml"><img src="https://github.com/TheInfoMoose/ST_Rust_Age/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI Build"></a>
  <a href="https://github.com/TheInfoMoose/ST_Rust_Age/actions/workflows/release.yml"><img src="https://github.com/TheInfoMoose/ST_Rust_Age/actions/workflows/release.yml/badge.svg" alt="Release Build"></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Language-Rust-dea584?logo=rust" alt="Language: Rust"></a>
  <a href="https://slint.dev/"><img src="https://img.shields.io/badge/GUI-Slint-blue" alt="GUI: Slint"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-GPL--3.0-blue.svg" alt="License: GPL-3.0"></a>
</p>

**Simply Transfer** is an enterprise-grade, cross-platform peer-to-peer (P2P) encrypted file transfer and synchronization application written in 100% safe Rust. Built for zero-trust security, high-throughput QUIC bulk streaming, and live volume snapshotting across Linux (including Arch/CachyOS, Ubuntu, Fedora), macOS (13–27), and Windows (10/11/Server).

---

## Architectural Overview

The workspace is structured into four modular Rust crates:

- **`simply-transfer-core`**: The central transfer engine governing multi-phase synchronization, the embedded P2P `russh` server daemon (port 2222), native in-process directory & volume enumeration (`simply-transfer-list|` and `simply-transfer-volumes`), and the high-throughput Out-Of-Band (OOB) QUIC data channel (`quinn`).
- **`simply-transfer-crypto`**: Dedicated cryptographic vault and identity management: Ed25519 key generation, dual-backed OS keyring + AES-256-GCM local vault (`~/.simply-transfer/vault.enc`), XOR-obfuscated `st:...` pairing tokens, WireGuard `.conf` encryption at rest, and streaming SHA-256 verification.
- **`simply-transfer-snapshots`**: Cross-platform point-in-time volume snapshot drivers for Windows (VSS), macOS (`tmutil`/APFS), and Linux (LVM), with automatic graceful fallback to direct streaming.
- **`simply-transfer-ui`**: Native hardware-accelerated desktop GUI built with [Slint](https://slint.dev/), featuring dual-pane local/remote file browsers, multi-task connection profiles, adaptive tooltips, and real-time telemetry.

---

## Key Features

- **Zero-Shell P2P Architecture**: Embedded `russh` SSHv2 daemon on port 2222 handles Ed25519 authentication, directory browsing (`simply-transfer-list|`), `mkdir`, and SHA-256 validation directly in Rust—without spawning `/bin/sh`, `zsh`, or `cmd.exe` windows.
- **High-Throughput TLS 1.3 QUIC Data Plane**: Bulk file payloads bypass SSH TCP window bottlenecks over multiplexed UDP/QUIC streams (`quinn`) with automatic offset resume on network interruption.
- **Dual-Backed Cryptographic Storage**: Every Ed25519 and WireGuard key is synchronized to both the native OS hardware keyring (`com.simplytransfer.app`) and an AES-256-GCM encrypted local vault (`~/.simply-transfer/vault.enc` with `0600` permissions), ensuring 100% reliability on Arch/CachyOS, Wayland/Hyprland, macOS Keychain, and Windows Credential Manager.
- **Point-in-Time Volume Snapshots**: Safely back up open or locked databases (`.db`, `.sqlite`, `.vhdx`, `.qbw`, `.pst`) via VSS, APFS, or LVM snapshots.
- **Scoped WireGuard WAN VPN**: Import standard `wg0.conf` profiles per connection with private/preshared keys encrypted at rest.
- **Guided Transfer Task Workflow**: Create multiple named transfer tasks per connection (Single Transfer, Continuous Sync, or Scheduled Transfer with active day/time windows) with locked-by-default setup protection until a task is named or selected.

---

## Security & Key Storage Auditability

Users can inspect or back up their local security state at any time:
1. **OS Keyring**: Service namespace `com.simplytransfer.app` (macOS Keychain, Windows Credential Manager, Linux Secret Service / KWallet / GNOME Keyring).
2. **Encrypted Local Vault**: `~/.simply-transfer/vault.enc` (AES-256-GCM authenticated encryption, `0600` permissions).
3. **Authorized Peer Keys**: `~/.simply-transfer/p2p_authorized_keys.json` and `~/.ssh/authorized_keys`.
4. **Connection & Task Profiles**: `connections_<os>.json` and `transfer_jobs_<os>.json` (contain only key identifier references, never raw private keys).
5. **Orphaned Key Cleanup**: Available directly in **Settings $\rightarrow$ Help & Support $\rightarrow$ Troubleshooting $\rightarrow$ Purge Orphaned Keys**.

---

## Building & Packaging for Production

### Prerequisites
- [Rust Toolchain](https://rustup.rs/) (stable)
- Linux GUI build headers (Ubuntu/Debian: `libfontconfig1-dev pkg-config libxcb-shape0-dev libxcb-xfixes0-dev`; Arch/CachyOS: `fontconfig pkgconf libxcb`)

### Compile Optimized Release Binary
```bash
cargo build --release
```
The release profile enables Thin LTO (`lto = "thin"`), single codegen unit (`codegen-units = 1`), symbol stripping (`strip = true`), and `windows_subsystem = "windows"` (zero background console windows).

### Generate Platform Packages (`.app` / `AppDir` / `.AppImage`)
```bash
./package.sh
```
See [DEPLOYMENT.md](DEPLOYMENT.md) for OS code signing, Gatekeeper/SmartScreen verification, and firewall configuration.

---

## Feedback, Bug Reports & Feature Requests

- **Repository**: [https://github.com/TheInfoMoose/ST_Rust_Age](https://github.com/TheInfoMoose/ST_Rust_Age)
- **Report a Bug**: [Open Bug Report Form](https://github.com/TheInfoMoose/ST_Rust_Age/issues/new?template=bug_report.md)
- **Request a Feature**: [Open Feature Request Form](https://github.com/TheInfoMoose/ST_Rust_Age/issues/new?template=feature_request.md)

---

## License

Licensed under the [GNU General Public License v3.0](LICENSE).
