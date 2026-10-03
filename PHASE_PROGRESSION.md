# Simply Transfer V2 - Phase Progression & Task Tracking

This document serves as the master tracking file for Simply Transfer V2 development. It tracks all functional pieces, architectural pivots, and backlog items to ensure no double work occurs and no features are lost in the weeds. 

**Rule of Thumb:** Never remove items from this list. Mark them as complete using strikethrough (`~~`) and append a timestamp of completion.

---

## Phase 1: Pre-flight Safety & Session Security
**Objective:** Establish secure baseline connectivity and fail-fast before allocating transmission memory.
* ~~**Strict Pre-flight Checks:** Query destination disk space; test destination `sshd` socket reachability on port 22.~~ *(Completed: Pre-October 2026)*
* ~~**OOB Handshake & Key Ingestion:** Generate Ed25519 keypairs, exchange via OOB TCP, and ingest to authorized keys.~~ *(Completed: Pre-October 2026)*
* ~~**Cryptographic Heartbeat:** Continuous peer verification during active sessions to detect network disconnects or socket hijacking.~~ *(Completed: Pre-October 2026)*

## Phase 2: Core Data Flow & Real-Time Telemetry
**Objective:** Ensure accurate telemetry and eliminate data corruption in basic file routing.
* ~~**Single File Routing:** Correct path normalization when mapping isolated single files rather than directory trees.~~ *(Completed: Pre-October 2026)*
* ~~**Producer-Consumer Engine (Basic):** Run reading/hashing, network streaming, and destination validation concurrently.~~ *(Completed: 2026-10-02)*
* ~~**Live Progress Streaming:** Transmit real-time upload speed (MB/s), duration, and ETA through `mpsc` channels into Slint.~~ *(Completed: Pre-October 2026)*
* ~~**Batched Remote Validation:** Execute remote SHA-256 checks in batches to reduce command execution overhead.~~ *(Completed: 2026-10-02)*

## Phase 3: P2P SSH Daemon Initialization (ACTIVE)
**Objective:** Host a dedicated `russh` server daemon within the application to bypass generic OpenSSH server limits and unify the source/destination environment.
* ~~**[x]** Implement embedded `russh` SSH Server daemon inside `simply-transfer-core`.~~ *(Completed: 2026-10-03)*
* ~~**[x]** Configure daemon to securely accept and validate the OOB-exchanged Ed25519 keys.~~ *(Completed: 2026-10-03)*
* ~~**[x]** Create custom SSH subsystem for data transmission (bypassing OS-level command execution).~~ *(Completed: 2026-10-03)*
* ~~**[x]** Refactor source application to connect to the peer daemon instead of the OS's native SSH daemon.~~ *(Completed: 2026-10-03)*

## Phase 4: High-Performance Transport & Resiliency
**Objective:** Saturate Gigabit Ethernet links and allow concurrent multi-session configurations over the new P2P Daemon.
* ~~**[x]** **Custom Binary Stream Transit:** Implement raw binary data streaming over the P2P SSH tunnel (Option 2), replacing the bottlenecked SFTP module.~~ *(Completed: 2026-10-03)*
* ~~**[x]** **Receiver Path Normalization:** Have the destination daemon handle local path writing natively to bypass Windows OS path-escaping bugs.~~ *(Completed: 2026-10-03)*
* **[ ]** **Network Resiliency & Reconnection:** Add exponential backoff, socket re-negotiation, and byte-level resume for interrupted transfers.
* **[ ]** **Multi-transfer UI Decoupling:** Decouple global UI setup models (`connections.json`) from active background transfers so users can configure Connection B while Connection A is actively transmitting. (Migrated from `FUTURE_ISSUE_MULTITRANSFER_SETUP.md`).

## Phase 5: Platform Polish & Technical Debt (Backlog)
**Objective:** Enhance platform-specific user experience and track upstream ecosystem maintenance.
* **[ ]** **PowerShell UAC / Console Masking:** Hide visible external PowerShell console windows during Windows administrative key ingestion (`Start-Process powershell -Verb RunAs`). The UAC prompt should remain, but the terminal window should be hidden. (Migrated from `FUTURE_ISSUE_POWERSHELL_PROMPT.md`).
* **[ ]** **Replace `ttf-parser` Dependency:** Monitor upstream `slint` / `winit` / `ab_glyph` dependencies to transition away from unmaintained `ttf-parser` (Advisory ID: `RUSTSEC-2026-0192`) when viable replacements emerge. Remove the `--ignore RUSTSEC-2026-0192` flag from CI once resolved. (Migrated from `FUTURE_ISSUE_REPLACE_TTF_PARSER.md`).
* **[ ]** **Out-of-Band Dedicated Data Channel (Option 3):** Evaluate shifting bulk data transit to a direct QUIC / TLS socket (using SSH merely as the control/auth plane) if Gigabit limits cannot be fully saturated by the P2P daemon.
