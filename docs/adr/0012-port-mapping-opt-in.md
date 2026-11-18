# ADR-0012: Router port mapping is opt-in

- **Status:** accepted
- **Date:** 2026-11-18
- **Irreversible:** no

## Context
A node behind a home router becomes reachable (for direct links and as a relay) if the router opens its port, as torrent clients do. That changes the router's configuration.

## Decision
`--map-port` tries UPnP IGD, then PCP/NAT-PMP, renews the lease, removes the mapping on exit, and gives up when the router's outer address is itself private or CGNAT. The mapped address becomes a direct-link candidate.

## Consequences
- Fewer relayed sessions where routers allow it.
- Not on by default; some routers have UPnP disabled.
