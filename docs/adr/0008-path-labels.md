# ADR-0008: Paths with per-hop labels; no source address

- **Status:** accepted
- **Date:** 2026-11-18
- **Irreversible:** yes

## Context
Forwarding by destination address puts the destination in every packet; forwarding by WireGuard indices (the first prototype) leaks the same index to every relay, lets indices collide, and stops a responder from reaching an initiator that never announced.

## Decision
A path is opened with `PATH_OPEN(label, dest)`. Each hop picks a fresh random 64-bit label for the next link and stores the pair both ways. After that, `DATA` carries only a label. `PATH_ACK` confirms, `PATH_ERR` tears down, `PROBE` measures. An endpoint finds the inner session by WireGuard's receiver index.

## Consequences
- No packet carries a source; only `PATH_OPEN` carries a destination; colluding relays can't match packets by label.
- Initiators can stay anonymous, and the far end can still answer and rekey.
- Relays keep state per path (bounded by limits, M4).
