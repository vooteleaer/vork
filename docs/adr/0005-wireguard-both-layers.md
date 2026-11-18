# ADR-0005: WireGuard for both layers

- **Status:** accepted
- **Date:** 2026-11-17
- **Irreversible:** yes

## Context
The network needs hop-by-hop protection (proving links, hiding headers on the wire) and end-to-end protection (relays mustn't read anything). Writing our own cryptographic protocol invites the mistakes that experienced designs have already made and fixed.

## Decision
Both layers are WireGuard (Noise IK), using boringtun in userspace: an **outer** session per neighbor and an **inner** session per pair of endpoints, carried in `DATA`. The X25519 identity key is the WireGuard static key for both.

## Consequences
- Audited, standard cryptography with forward secrecy and rekeying; no cryptography of our own.
- WireGuard implementations exist on small devices (ESP32), so Lite nodes can reuse them.
- Two layers cost ~100 bytes per packet and two AEAD operations per hop.
- boringtun only carries IP packets, so the layer is framed as IP (ADR-0007), and it sees little maintenance (open question 17).
