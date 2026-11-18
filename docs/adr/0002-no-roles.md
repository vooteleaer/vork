# ADR-0002: Every node runs the same code; no roles

- **Status:** accepted
- **Date:** 2026-11-17
- **Irreversible:** no

## Context
Reticulum has configured transport nodes and interfaces; Tailscale, Nebula and ZeroTier have coordination servers, lighthouses or roots. Each of these is a special node someone has to run, and a place where the network can be controlled or broken.

## Decision
All nodes run the same code. Relaying, storing and reflecting addresses are things any node can do; which nodes actually do them follows from measured properties (public address, uptime, bandwidth, round trip), not from configuration. The only declared difference is a node saying it won't relay (Lite, ADR-0010).

## Consequences
- No single point of control or failure; well-placed nodes become busy relays on their own and are replaced when they leave.
- Nothing stops a well-connected node from seeing a lot of traffic metadata; privacy classes (ADR-0016) are the answer, not roles.
