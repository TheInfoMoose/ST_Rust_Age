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

## Phase 4: High-Performance Transport & Resiliency (COMPLETED)
**Objective:** Saturate Gigabit Ethernet links and allow concurrent multi-session configurations over the new P2P Daemon.
* ~~**[x]** **Custom Binary Stream Transit:** Implement raw binary data streaming over the P2P SSH tunnel.~~ *(DEPRECATED: 2026-10-03 - Superseded by OOB QUIC Channel due to SSH packet fragmentation limits)*
* ~~**[x]** **Receiver Path Normalization:** Have the destination daemon handle local path writing natively to bypass Windows OS path-escaping bugs.~~ *(Completed: 2026-10-03)*
* ~~**[x]** **Network Resiliency & Reconnection:** Add exponential backoff, socket re-negotiation, and byte-level resume for interrupted transfers.~~ *(Completed: 2026-10-03)*
* ~~**[x]** **Out-of-Band Dedicated Data Channel (QUIC Piviot):** Shift bulk data transit to a direct `quinn` UDP/QUIC socket. SSH will be used solely for the control/auth plane (passing ephemeral ports and OTP tokens). This will bypass `russh` packet bottlenecks entirely. (Moved from Phase 5 Backlog).~~ *(Completed: 2026-10-03)*
* ~~**[x]** **Fix Hash Mismatch / Resume Corruption:** Replace `append(true)` with atomic `seek()` alignment on the remote daemon to prevent chunk-tearing corruption during reconnections.~~ *(Completed: 2026-10-03)*
* ~~**[x]** **Fix UI Cancellation Loop:** Wrap the data transmission loop in a `tokio::select!` block listening to the `ControlSignal::Cancel` channel for instantaneous teardown.~~ *(Completed: 2026-10-03)*
* ~~**[x]** **Fix Thread Panic on App Close:** Implement `std::process::exit(0)` bypass on application exit to prevent Tokio runtime TLS drop panics.~~ *(Completed: 2026-10-03)*
* ~~**[x]** **Native Hash Validation:** Replace OS-level shell hashing (PowerShell/sha256sum) with a custom SSH interception layer (`simply-transfer-hash|`) to natively compute validation hashes on the destination daemon for perfect parity and reliability.~~ *(Completed: 2026-10-04)*
* ~~**[x]** **Integrity Validation Deadlock Fix:** Implement bounds checking on validation loop and explicit OS file handle drops on QUIC receivers to prevent silent panics leaving batches stranded in "Completed" status.~~ *(Completed: 2026-10-04)*
* ~~**[x]** **Integrity Validation UI Fix:** Fix UI event loop premature teardown discarding final event buffer, ensuring `TransferComplete` events process gracefully.~~ *(Completed: 2026-10-04)*
* ~~**[x]** **Multi-transfer UI Decoupling:** Decouple global UI setup models (`connections.json`) from active background transfers so users can configure Connection B while Connection A is actively transmitting.~~ *(Completed: 2026-10-04)*

## Phase 5: Platform Polish & Technical Debt (Backlog)
**Objective:** Enhance platform-specific user experience and track upstream ecosystem maintenance.
* ~~**[x]** **PowerShell UAC / Console Masking:** Hide visible external PowerShell console windows during Windows administrative key ingestion (`Start-Process powershell -Verb RunAs`). The UAC prompt should remain, but the terminal window should be hidden.~~ *(Completed: 2026-10-04)*
* ~~**[x]** **Replace `ttf-parser` Dependency:** Monitor upstream `slint` / `winit` / `ab_glyph` dependencies to transition away from unmaintained `ttf-parser` (Advisory ID: `RUSTSEC-2026-0192`) when viable replacements emerge. Remove the `--ignore RUSTSEC-2026-0192` flag from CI once resolved.~~ *(Completed: 2026-10-04)*
* ~~**[x]** **Out-of-Band Dedicated Data Channel (Option 3):** Evaluate shifting bulk data transit to a direct QUIC / TLS socket.~~ *(Migrated to Phase 4: 2026-10-03)*
## Phase 6: UI/UX & Functional Polish (ACTIVE)
**Objective:** Resolve layout overlaps, fix metric alignments, and ensure in-app functionality operates seamlessly.

### Sub-phase A: Layout & Styling Enhancements
* ~~**[x]** **Transfer Setup Layout:** Remove source/destination path field labels and extend the text boxes, relying entirely on placeholder text.~~ *(Completed: 2026-10-04)*
* ~~**[x]** **Active Transfer Fixed Positioning:** Implement fixed positioning for Upload, Download, and ETA labels and values to prevent jitter as numbers fluctuate.~~ *(Completed: 2026-10-04)*
* ~~**[x]** **Live Transfer Queue Scaling:** Convert file names and status fields into percentage-based columns tied to the window size to prevent overlaps. Shorten long file names with a trailing ellipsis and convert file sizes from bytes to MB.~~ *(Completed: 2026-10-04)*
* ~~**[x]** **Integrity Validation Scaling:** Apply the same percentage-based column layout (as Live Transfer Queue) to the file name and complete/validated fields in the Integrity Validation view.~~ *(Completed: 2026-10-04)*
* ~~**[x]** **Completed Files Layout:** Apply fixed positioning for the fields in the Completed files list.~~ *(Completed: 2026-10-04)*
* ~~**[x]** **Minimum Window Scaling:** Enforce a minimum fixed width/height for the app window while allowing scaling, so percentage-based layouts function correctly.~~ *(Completed: 2026-10-04)*
* ~~**[x]** **Dynamic Connection List:** Update the Dashboard connections list to use proportional scaling (percentage-based width) instead of a fixed pixel width.~~ *(Completed: 2026-10-04)*
* ~~**[x]** **Clean Active Transfer Layout:** Remove the overlapping phase progression text (`current_phase`) next to the MetricsGrid to ensure horizontal scaling fits cleanly.~~ *(Completed: 2026-10-04)*

### Sub-phase B: State & Metrics Accuracy
* **[x]** ~~**Destination Connection Status:** Fix the bug where destination connection details hang on "pending verification" and do not accurately update.~~ *(Completed: 2026-10-04)*

#### Phase 6 - Sub-phase D (Security Audit & NIST Key Management)
* ~~**[x]** **NIST Key Persistence:** Remove all plaintext `.pem` file fallbacks. Authenticate via in-memory extraction strictly from the OS Keyring.~~ *(Completed: 2026-10-04)*
* ~~**[x]** **P2P Daemon Persistence:** Persist `russh_server` authorized keys to disk with strict 0600 access controls to ensure peer connections survive app restarts.~~ *(Completed: 2026-10-04)*
* ~~**[x]** **OOB Handshake Timeouts:** Add `tokio::time::timeout` wrappers to prevent indefinite hangs during OOB key exchanges.~~ *(Completed: 2026-10-04)*
* **[x]** ~~**Global ETA Calculation:** Ensure the file transfer ETA reflects the overall transfer completion rather than erratically estimating based on individual batches.~~ *(Completed: 2026-10-04)*

### Sub-phase C: App Functionality & Polish
* ~~**[x]** **Active Transfer Modal:** Add necessary functional controls to the active transfer popout modal.~~ *(Completed: 2026-10-04)*
* ~~**[x]** **Pause Transfer Validation:** Validate and fix the functionality of the "Pause Transfer" button.~~ *(Completed: 2026-10-04)*
* ~~**[x]** **Destination Active Transfer View:** Ensure the destination device displays the active transfer, including file progress, transmission metrics, and completion/validation status.~~ *(Completed: 2026-10-04)*
* ~~**[x]** **In-App Logging:** Reinstate logs inside Settings > Logs so users no longer have to rely on terminal output.~~ *(Completed: 2026-10-04)*
* ~~**[x]** **Windows Console Suppression:** Ensure launching the release version of the Windows app does not spawn a terminal/cmd window.~~ *(Completed: 2026-10-04)*

## Phase 7: Advanced Data Management & Automation (ACTIVE)
**Objective:** Add robust preflight checks, advanced UI snapshot indicators, automated scheduling, and in-app diagnostic logs.

* ~~**[x]** **Advanced Preflight Checks:** Enhance the preflight check to query the destination to ensure the files being sent do not already exist, preventing accidental overwrites.~~ *(Completed: 2026-10-04)*
* ~~**[x]** **Snapshot Flagging (UI):** Add visual indicators to the file picker modal and the transfer queue to denote if a file will use snapshotting, specifically differentiating between VSS (Windows), LVM (Linux), and APFS (macOS).~~ *(Completed: 2026-10-04)*
* ~~**[x]** **Transfer Scheduling & Sync:** Leverage the persistent P2P daemon to enable scheduled and recurring folder sync capabilities.~~ *(Completed: 2026-10-04)*
* ~~**[x]** **In-App Logging Integration:** Ensure connection, transfer, sync, and scheduled transfer logs are persisted and fully accessible from within the Settings > Logs UI.~~ *(Completed: 2026-10-04)*
