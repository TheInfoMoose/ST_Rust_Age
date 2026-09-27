# Security Policy

## Supported Versions

Currently, only the latest release of Simply Transfer is supported with security updates. 

| Version | Supported          |
| ------- | ------------------ |
| 1.x.x   | :white_check_mark: |
| < 1.0   | :x:                |

## Reporting a Vulnerability

Security is a top priority for Simply Transfer, as it handles secure data syncing and cryptographic key pairs.

If you discover a security vulnerability within Simply Transfer, please do NOT submit an issue on the public issue tracker. Instead, please report it via private vulnerability reporting on GitHub or send an email directly to the repository maintainers.

We will endeavor to respond to vulnerability reports within 48 hours.

### What to include in your report
- A description of the vulnerability.
- Steps to reproduce the issue.
- The environment (OS, Rust version, etc.) where the issue was discovered.
- Potential impact of the vulnerability.

## Secure Coding Practices
Simply Transfer enforces rigorous CI checks:
1. `cargo audit` is run continuously to identify vulnerabilities in standard dependencies.
2. Cross-platform pathing is heavily scrutinized and integration tested to prevent directory traversal attacks via malicious SSH targets.
3. Cryptographic materials are handled via secure enclaves (e.g., `secret-service` on Linux, native keyrings on macOS/Windows) avoiding plaintext storage on disks.

## Build Provenance
Simply Transfer builds include SLSA (Supply-chain Levels for Software Artifacts) Level 3 build provenance. Official release artifacts are generated and signed via GitHub Actions and can be cryptographically verified using the GitHub CLI.
