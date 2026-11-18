# ADR-0006: A link counts only when proven to work both ways

- **Status:** accepted
- **Date:** 2026-11-17
- **Irreversible:** no

## Context
In Reticulum, a node learns paths from announces it hears. If it hears a powerful node it can't reach back, its path table points at a dead end: the asymmetric-link problem.

## Decision
A neighbor is up only while its WireGuard handshake is fresh (< 180 s) and it has sent something within 35 s (it keepalives every 25 s). A handshake needs a round trip, so an up link works both ways. Announces, path requests and paths are accepted only from up neighbors.

## Consequences
- No routes over one-way links.
- A silent neighbor is detected in ~35 s, and every path through it is torn down.
- Constant keepalive traffic on every link (one small packet per 25 s).
