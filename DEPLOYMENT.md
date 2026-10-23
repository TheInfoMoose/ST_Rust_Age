# Deployment, Packaging & OS Verification Guide

Welcome to the production deployment guide for **Simply Transfer v1.0.0 (ST Rust Age)**. This guide covers installation, firewall setup, and OS code-signing / verification across Windows, macOS, and Linux (including Arch/CachyOS, Ubuntu, and Fedora).

## Supported Platforms
- **Windows**: Windows 10, Windows 11, and Windows Server 2019+ (`x86_64`)
- **macOS**: macOS 13–27 on Apple Silicon (`aarch64`) and Intel (`x86_64`)
- **Linux**: Arch / CachyOS, Debian / Ubuntu, Fedora / RHEL (`x86_64` native & `AppDir` / `AppImage`)

---

## Network & Firewall Requirements

Simply Transfer uses an embedded P2P SSHv2 control server and an Out-Of-Band (OOB) TLS 1.3 QUIC bulk data channel:
- **TCP Port `2222`**: Built-in P2P SSH control channel, volume discovery, and native directory browsing.
- **Ephemeral TCP/UDP Ports**: Used on the local LAN/VPN interface during initial token verification and QUIC bulk streaming.

### Linux Firewall Quick Setup (Arch / CachyOS / Ubuntu / Fedora)
- **UFW**:
  ```bash
  sudo ufw allow 2222/tcp
  ```
- **Firewalld**:
  ```bash
  sudo firewall-cmd --add-port=2222/tcp --permanent && sudo firewall-cmd --reload
  ```

---

## Installation & OS Security Recognition

### 1. macOS (`SimplyTransfer.app` Bundle)
Release builds package Simply Transfer as a signed macOS application bundle (`SimplyTransfer.app`) with hardened runtime entitlements (`com.apple.security.network.client`, `com.apple.security.network.server`, `com.apple.security.files.user-selected.read-write`) and SLSA build provenance attestation.

1. Extract `SimplyTransfer-macos-silicon.app.tar.gz` and move `SimplyTransfer.app` to `/Applications`.
2. If macOS Gatekeeper displays an "unidentified developer" prompt on first launch (when using ad-hoc CI signatures rather than a paid Apple Developer ID):
   - Right-click (or Control-click) `SimplyTransfer.app` and select **Open**, or run once in Terminal:
     ```bash
     xattr -dr com.apple.quarantine /Applications/SimplyTransfer.app
     ```
3. **Protected Folders**: To browse or transfer from `~/Desktop`, `~/Documents`, or external `/Volumes` on macOS 14–27, enable **Simply Transfer** under **System Settings $\rightarrow$ Privacy & Security $\rightarrow$ Full Disk Access**.

### 2. Windows (`simply-transfer.exe`)
Release builds are compiled with `#![windows_subsystem = "windows"]` and `CREATE_NO_WINDOW` subprocess flags (no background `cmd.exe` or PowerShell windows appear), signed with an Authenticode signature, and accompanied by GitHub SLSA provenance attestations and `.sha256` checksums.

1. Extract `simply-transfer-windows-x86_64.zip` to `C:\Program Files\SimplyTransfer\`.
2. Verify the SHA-256 hash against `simply-transfer-windows-x86_64.zip.sha256`:
   ```powershell
   Get-FileHash .\simply-transfer-windows-x86_64.zip -Algorithm SHA256
   ```
3. Launch `simply-transfer.exe`. If Windows Defender SmartScreen prompts on first run, click **More info $\rightarrow$ Run anyway**.

### 3. Linux (Native Binary / `AppDir` / `.AppImage`)
1. Extract `simply-transfer-linux-x86_64.tar.gz` or build locally via `./package.sh`.
2. Run directly or via the `AppDir` / `.AppImage` bundle:
   ```bash
   chmod +x simply-transfer
   ./simply-transfer
   ```

---

## Reporting Issues & Feature Requests
- **Bug Report Form**: [https://github.com/TheInfoMoose/ST_Rust_Age/issues/new?template=bug_report.md](https://github.com/TheInfoMoose/ST_Rust_Age/issues/new?template=bug_report.md)
- **Feature Request Form**: [https://github.com/TheInfoMoose/ST_Rust_Age/issues/new?template=feature_request.md](https://github.com/TheInfoMoose/ST_Rust_Age/issues/new?template=feature_request.md)
