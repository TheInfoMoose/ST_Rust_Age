# Simply Transfer v1.0.0 — Architecture & Development Wiki

Welcome to the Simply Transfer Wiki. This document maps the application's crate architecture, security model, and production capabilities.

## 1. Project Overview
Simply Transfer is a high-performance, zero-trust P2P file transfer utility designed for cross-platform reliability across Linux (including Arch/CachyOS), macOS (13–27), and Windows (10/11/Server). It combines an asynchronous Rust engine (`simply-transfer-core`) with a native Slint GUI (`simply-transfer-ui`).

## 2. Crate Architecture

### `simply-transfer-ui`
- **Role**: Native desktop frontend built with Slint and Tokio.
- **Key Features**:
  - Guided **New Transfer Task** workflow: right-hand Transfer Setup controls remain locked until a task is named or selected, preventing accidental unassociated edits.
  - Dual-pane inline local/remote directory browser with volume free-space badges and instant refresh.
  - Adaptive styled tooltips (`220ms` fast delay for icon-only controls, `650ms` for text buttons with usage descriptions).
  - Collapsible **Help & Support** center with step-by-step How-To guide, platform-specific Troubleshooting (including one-click **Purge Orphaned Keys**), Security FAQ, and GitHub issue/feature form links.
  - Zero console window popups on Windows (`#![windows_subsystem = "windows"]` and `CREATE_NO_WINDOW`).

### `simply-transfer-core`
- **Role**: Embedded P2P `russh` SSHv2 daemon (port 2222) and QUIC bulk data engine (`quinn`).
- **Key Features**:
  - In-process Rust commands (`simply-transfer-list|`, `simply-transfer-volumes`, `simply-transfer-mkdir|`, `simply-transfer-hash|`) that eliminate `/bin/sh` and `cmd.exe` subprocesses during browsing and transfers.
  - Out-of-Band (OOB) TLS 1.3 QUIC bulk streaming with automatic resume and byte-level SHA-256 verification.

### `simply-transfer-crypto`
- **Role**: Cryptographic identity, pairing tokens, and encrypted storage.
- **Key Features**:
  - Synchronized dual-write storage: every Ed25519 and WireGuard secret is written to both the OS hardware keyring (`com.simplytransfer.app`) and an AES-256-GCM local vault (`~/.simply-transfer/vault.enc`), preventing key loss across D-Bus Secret Service sessions on Arch/CachyOS.
  - Orphaned key purging (`SecureStorage::purge_orphaned_keys` and `TransferServer::purge_unauthorized_keys`).

### `simply-transfer-snapshots`
- **Role**: Point-in-time volume snapshots for open/locked files.
- **Key Features**: Windows VSS (executed with hidden PowerShell flags), macOS APFS (`tmutil`), and Linux LVM with automatic fallback to live streaming.

## 3. Production Status Checklist
- ✅ **Cross-Platform Automated CI/CD & Provenance Attestation**
- ✅ **Zero-Shell Native Remote Directory Listing (`simply-transfer-list|`)**
- ✅ **Dual-Backed Keyring + AES-256-GCM Vault (`~/.simply-transfer/vault.enc`)**
- ✅ **Windows Console Window Suppression (`windows_subsystem` + `CREATE_NO_WINDOW`)**
- ✅ **One-Time Persisted OS Permission State (`~/.simply-transfer/permissions_state.json`)**
- ✅ **Guided New Transfer Task Naming & Lock/Unlock Workflow**
- ✅ **Adaptive Styled Tooltips & Collapsible Help Center**
