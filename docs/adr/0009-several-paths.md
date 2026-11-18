# ADR-0009: Several paths per session

- **Status:** accepted
- **Date:** 2026-11-18
- **Irreversible:** no

## Context
A single path fails when one relay does, and puts all of a session's traffic in front of one relay.

## Decision
The origin opens up to N paths (default 2) through different first hops, plus a direct one when available. It probes each path every second; a path missing 3 probes is dropped and its first hop rests 5–60 s. Packets go by *spray* (weighted by round trip) or *dup* (every path). A healthy direct path is used alone. Flow pinning comes with IP sessions (M2).

## Consequences
- Failover in ~3 s without losing the session; each relay sees part of the traffic; more bandwidth when relays are the bottleneck.
- Probing costs a small packet per path per second.
- Paths through different first hops may still share later hops.
