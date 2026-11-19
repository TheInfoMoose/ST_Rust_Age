## Problem Description
`simply-transfer-link/src/server.rs:121-133` — a pairing-only session (one consumed pair secret) can send `Pair` repeatedly and enroll any number of keys — end pairing mode after the first successful Pair (or close the channel). Found in 4C validation.
Migrated from the plan file's Parked list (2026-11-18).

## Reproduction Steps
n/a (parked finding; see Problem Description)

## Fix Log & Agent Notes
