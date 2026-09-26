# Deployment & Installation Guide

Welcome to the deployment guide for **Simply Transfer v2 (ST Rust Age)**. This guide outlines how to download, install, and run the pre-compiled binaries available on our GitHub Releases page.

## Supported Platforms
- **Windows**: Windows 10+ and Windows Server 2019+
- **macOS**: Apple Silicon (M1/M2/M3) and Intel (x86_64)
- **Linux**: Debian-based (Ubuntu, Mint) and RHEL-based (Fedora, CentOS)
- **NAS OS (e.g. Synology)**: Standard x86_64 Linux architecture support

---

## Installation Instructions

### 1. Windows (10, 11, Server 2019+)
1. Download `simply-transfer-windows-x86_64.exe` from the latest release.
2. Place the executable in a directory of your choice (e.g., `C:\Program Files\SimplyTransfer\`).
3. Double-click the executable to launch the GUI.
*Note: For VSS (Volume Shadow Copy) snapshot features to work, you must run the application as an Administrator.*

### 2. macOS (Apple Silicon & Intel)
1. Determine your processor type (Apple menu > About This Mac).
2. Download the appropriate binary:
   - Silicon: `simply-transfer-macos-silicon`
   - Intel: `simply-transfer-macos-intel`
3. Open Terminal, navigate to your download directory, and make the file executable:
   ```bash
   chmod +x simply-transfer-macos-*
   ```
4. Run the application:
   ```bash
   ./simply-transfer-macos-silicon
   ```
*Note: APFS snapshot capabilities require elevated privileges. If using snapshots, launch via `sudo`.*

### 3. Linux (Debian, RedHat, Ubuntu)
1. Download `simply-transfer-linux-x86_64`.
2. Make the file executable:
   ```bash
   chmod +x simply-transfer-linux-x86_64
   ```
3. Run the application:
   ```bash
   ./simply-transfer-linux-x86_64
   ```
*Dependencies: Ensure you have `fontconfig` installed on your system to render the GUI fonts.*

### 4. NAS OS (Synology, TrueNAS Scale)
1. SSH into your NAS device.
2. Download the Linux generic x86_64 binary using `wget` or `curl`:
   ```bash
   wget https://github.com/TheInfoMoose/ST_Rust_Age/releases/latest/download/simply-transfer-linux-x86_64
   ```
3. Make it executable:
   ```bash
   chmod +x simply-transfer-linux-x86_64
   ```
4. Run the daemon/transfer engine as needed. 
*Note: Synology DSM is primarily a headless environment. While the binary contains the UI, you can trigger core functionalities via the engine module or API (if enabled) from a headless terminal.*

---

## Versioning & Bug Reporting
We follow Semantic Versioning (vX.Y.Z).
If you encounter any issues or wish to request a feature, please use the **Issues** tab on GitHub.

To effectively report a bug, please include:
- Your OS and architecture
- The version of Simply Transfer you are running
- Logs or terminal output (if any)
