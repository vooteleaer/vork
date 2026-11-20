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

## Update 2026-11-18: the owner's own devices
Between one owner's devices there's nothing to hide: `devices = [addresses]` lists the owner's other devices by identity address, and a session with one of them gets a direct link without `direct`. Both ends must list each other, and the far end must be known: the destination's identity on the opening side, the opener's once it identifies on the other. The offer goes once the session carries its first message (it used to go only after a test ping), and again if the far end identifies later. Applications name devices too (local API `DEVICES`): sessions with them get direct links (matched by destination or by the identity behind it).
