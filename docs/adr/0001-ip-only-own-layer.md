# ADR-0001: IP only, with our own network layer (not on top of Reticulum)

- **Status:** accepted
- **Date:** 2026-11-17
- **Irreversible:** yes

## Context
Reticulum has the right model (key-hash addresses, no source addresses, hostile nodes) but is built for slow radio links: 500-byte packets, conservative windows, configured interfaces and transport nodes, and paths learned from announces over links that may only work one way. Most of the community's nodes run the Python implementation, which caps throughput along any path through them.

## Decision
vØrk is IP-only and has its own announce and path mechanism, borrowing Reticulum's ideas but not its code or wire format. Radio stays with Reticulum; a bridge is possible later because both address by key hash.

## Consequences
- IP-sized packets (MTU ~1400 instead of 500) and WireGuard's speed.
- No dependency on other people's nodes or implementations.
- No LoRa or serial links in vØrk itself.
- A second ecosystem next to Reticulum's; a bridge is needed to join them.
