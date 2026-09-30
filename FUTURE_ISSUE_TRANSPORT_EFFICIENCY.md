# Future Issue: Transport Efficiency

## Problem Description
The transmission portion of the app was previously much faster, but somewhere along the line, it has been slowed down significantly. Recent transfers were capping out at 2-3 MB/s. Given a GbE (Gigabit Ethernet) network pipe, NVME drives, and a dedicated channel, the transmission speed should be an order of magnitude faster (approaching ~100+ MB/s).

## Proposed Solution
- Investigate the `ssh2_client.rs` chunking and read/write buffer sizes. Currently, it might be using an unoptimized 32KB buffer without pipelining.
- Profile `tokio::task::spawn_blocking` to see if thread context switching or CPU blocking is throttling the raw network I/O.
- Check if `libssh2` SFTP write operations are artificially limited or if we need to implement asynchronous pipelined writes or switch to a different SSH/SFTP backend (like `russh` or raw `openssh` multiplexing) to saturate a Gigabit link.
- Review recent changes that may have accidentally introduced a performance regression in the transmission phase loop.
