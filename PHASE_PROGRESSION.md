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

## Phase 3: P2P SSH Daemon Initialization (COMPLETED)
**Objective:** Host a dedicated `russh` server daemon within the application to bypass generic OpenSSH server limits and unify the source/destination environment.
* ~~**[x]** Implement embedded `russh` SSH Server daemon inside `simply-transfer-core`.~~ *(Completed: 2026-10-03)*
* ~~**[x]** Configure daemon to securely accept and validate the OOB-exchanged Ed25519 keys.~~ *(Completed: 2026-10-03)*
* ~~**[x]** Create custom SSH subsystem for data transmission (bypassing OS-level command execution).~~ *(Completed: 2026-10-03)*
* ~~**[x]** Refactor source application to connect to the peer daemon instead of the OS's native SSH daemon.~~ *(Completed: 2026-10-03)*

## Phase 4: High-Performance Transport & Resiliency (ACTIVE)
**Objective:** Saturate Gigabit Ethernet links and allow concurrent multi-session configurations over the new P2P Daemon.
* ~~**[x]** **Custom Binary Stream Transit:** Implement raw binary data streaming over the P2P SSH tunnel.~~ *(DEPRECATED: 2026-10-03 - Superseded by OOB QUIC Channel due to SSH packet fragmentation limits)*
* ~~**[x]** **Receiver Path Normalization:** Have the destination daemon handle local path writing natively to bypass Windows OS path-escaping bugs.~~ *(Completed: 2026-10-03)*
* ~~**[x]** **Network Resiliency & Reconnection:** Add exponential backoff, socket re-negotiation, and byte-level resume for interrupted transfers.~~ *(Completed: 2026-10-03)*
* ~~**[x]** **Out-of-Band Dedicated Data Channel (QUIC Piviot):** Shift bulk data transit to a direct `quinn` UDP/QUIC socket. SSH will be used solely for the control/auth plane (passing ephemeral ports and OTP tokens). This will bypass `russh` packet bottlenecks entirely. (Moved from Phase 5 Backlog).~~ *(Completed: 2026-10-03)*
* ~~**[x]** **Fix Hash Mismatch / Resume Corruption:** Replace `append(true)` with atomic `seek()` alignment on the remote daemon to prevent chunk-tearing corruption during reconnections.~~ *(Completed: 2026-10-03)*
* ~~**[x]** **Fix UI Cancellation Loop:** Wrap the data transmission loop in a `tokio::select!` block listening to the `ControlSignal::Cancel` channel for instantaneous teardown.~~ *(Completed: 2026-10-03)*
* ~~**[x]** **Fix Thread Panic on App Close:** Implement `std::process::exit(0)` bypass on application exit to prevent Tokio runtime TLS drop panics.~~ *(Completed: 2026-10-03)*
* ~~**[x]** **Native Hash Validation:** Replace OS-level shell hashing (PowerShell/sha256sum) with a custom SSH interception layer (`simply-transfer-hash|`) to natively compute validation hashes on the destination daemon for perfect parity and reliability.~~ *(Completed: 2026-10-04)*
* **[ ]** **Multi-transfer UI Decoupling:** Decouple global UI setup models (`connections.json`) from active background transfers so users can configure Connection B while Connection A is actively transmitting.

## Phase 5: Platform Polish & Technical Debt (Backlog)
**Objective:** Enhance platform-specific user experience and track upstream ecosystem maintenance.
* **[ ]** **PowerShell UAC / Console Masking:** Hide visible external PowerShell console windows during Windows administrative key ingestion (`Start-Process powershell -Verb RunAs`). The UAC prompt should remain, but the terminal window should be hidden.
* **[ ]** **Replace `ttf-parser` Dependency:** Monitor upstream `slint` / `winit` / `ab_glyph` dependencies to transition away from unmaintained `ttf-parser` (Advisory ID: `RUSTSEC-2026-0192`) when viable replacements emerge. Remove the `--ignore RUSTSEC-2026-0192` flag from CI once resolved.
* ~~**[x]** **Out-of-Band Dedicated Data Channel (Option 3):** Evaluate shifting bulk data transit to a direct QUIC / TLS socket.~~ *(Migrated to Phase 4: 2026-10-03)*
