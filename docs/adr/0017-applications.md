# ADR-0017: Applications: IPv6 inside sessions, several ways in

- **Status:** proposed
- **Date:** 2026-11-18
- **Irreversible:** no

## Context
Existing software speaks IP; new software benefits from knowing identities. A TUN interface needs admin rights and conflicts with other VPNs; an API alone leaves existing software out.

## Decision
Sessions carry IPv6 packets between overlay addresses. Applications attach in one of these ways:
1. **TUN interface** (kernel stack): every existing application, system-wide.
2. **Local daemon API** (Unix socket / named pipe): status, peers, `whois(overlay address) → identity`, services; several applications share one node, each with its own identity (ADR-0020).
3. **Native message API**: `connect(identity, service)`, request/response and datagrams, no IP; storage uses this.
4. **Embedded stack** (smoltcp, like Tailscale's tsnet): an application embeds vØrk and gets TCP/UDP sockets to identities without admin rights. On an ESP32, lwIP plays this part.

Services have names in the native API and ports on IP; the mapping is open question 10. An identity firewall uses whois.

## Consequences
- Existing software works unchanged; new software gets identities.
- The node checks that a packet's overlay source matches its session's identity.
- Four interfaces to maintain; built in the order of the roadmap (M2, M3, M9, M10).
