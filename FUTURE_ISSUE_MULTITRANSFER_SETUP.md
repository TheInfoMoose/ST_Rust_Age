# Future Issue: Multitransfer Setup

## Problem Description
Currently, the transfer setup screen functionality is heavily coupled with the active connection. We need to be able to associate the transfer setup screen with a specific connection, even if multiple transfers are queued or active. Right now, this setup flow works if a transfer is not in progress, but it breaks if one is already active (e.g., overlapping UI state, mismatched queues).

## Proposed Solution
- Decouple the UI state for the setup screen from the global active transfer state.
- Implement session-specific or connection-specific setup contexts so that a user can configure a new transfer for Connection B while Connection A is actively transmitting.
- Update `ui.set_connections()` and tab-switching logic to isolate setup variables from the active dashboard variables.
