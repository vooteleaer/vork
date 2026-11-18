# ADR-0011: Direct links are opt-in

- **Status:** accepted
- **Date:** 2026-11-17
- **Irreversible:** no

## Context
A direct link is faster and spares relays, but it shows each end's IP address to the other.

## Decision
With `--direct` on both ends, they exchange candidate addresses inside their session and hole-punch with simultaneous WireGuard handshakes. Off by default; inside islands the default is open question 13.

## Consequences
- Privacy by default; speed when both choose it.
- Symmetric NAT on both ends stays relayed.
