# ADR-0010: Lite nodes

- **Status:** accepted
- **Date:** 2026-11-17
- **Irreversible:** no

## Context
Microcontrollers and battery devices can't relay, can't take announce floods and have only their platform's WireGuard.

## Decision
A Lite node announces with the relay flag cleared. It never relays, receives no floods, asks for the paths it needs (`PATH_REQ`), and accepts only paths that end at it. Nobody routes through a Lite node unless it is the destination.

## Consequences
- An ESP32 can join with ESPHome's stock WireGuard (S1/M5 to confirm the inner layer).
- Lite nodes depend on full neighbors for reachability.
