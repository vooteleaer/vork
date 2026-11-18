# ADR-0016: Privacy classes; nested WireGuard for the strong class

- **Status:** proposed
- **Date:** 2026-11-17
- **Irreversible:** no

## Context
No low-latency network gets strong anonymity, low overhead and low latency at once. Different traffic needs different trade-offs.

## Decision
Three classes: island (direct), normal (1–2 relays, own or island relays preferred; the default), strong (nested WireGuard over 2–3 relays so no relay knows both ends, the receiver choosing its inbound part, one-off initiator keys, optional cover traffic). Classes are chosen per destination or application.

## Consequences
- The default costs little; the strong class costs latency, MTU (~32–40 bytes per extra layer) and relay bandwidth.
- Timing correlation by an observer of both ends remains.
