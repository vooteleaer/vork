# ADR-0004: An address is the hash of keys; IP only between neighbors

- **Status:** accepted
- **Date:** 2026-11-17
- **Irreversible:** yes

## Context
Location-based addresses (IP) change when devices move, reveal where they are, and can be spoofed. Reticulum showed that a hash of public keys works as an address.

## Decision
A node has an Ed25519 signing key and an X25519 key. Its address is `SHA-256(ed_pub ‖ x_pub)[0..16]`. Packets inside the network are addressed by this hash (and, after a path is open, by per-link labels). IP addresses are used only to send packets to direct neighbors. The overlay IPv6 address (`fd` + 15 bytes of the hash) is a compatibility view for ordinary applications, never used for routing.

## Consequences
- An address can't be claimed without the signing key.
- Moving a device, or an identity between devices (ADR-0013), keeps its address.
- 16 bytes is a compromise between collision/grinding resistance and size.
