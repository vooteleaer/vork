# ADR-0007: The layer travels as UDP/IPv4 inside the tunnel

- **Status:** accepted
- **Date:** 2026-11-18
- **Irreversible:** yes

## Context
WireGuard carries IP packets. Small devices only have their platform's WireGuard: ESPHome's component has one peer, IPv4 only, fixed allowed IPs, and delivers packets into lwIP.

## Decision
Every vØrk message is the payload of an IPv4/UDP datagram to port 4799 inside the outer tunnel, between fixed link addresses (10.89.0.1 → 10.89.0.2). Receivers ignore the addresses.

## Consequences
- A Lite node needs no WireGuard changes, only a UDP socket on the tunnel.
- 28 bytes of overhead per packet.
- The port and addresses become part of the format (open question 6).
