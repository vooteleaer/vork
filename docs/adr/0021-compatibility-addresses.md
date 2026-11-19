# ADR-0021: Compatibility addresses: IPv6 per identity, local IPv4 aliases

- **Status:** accepted (IPv4 aliases in part: ADR-0025)
- **Date:** 2026-11-18
- **Irreversible:** no (the IPv6 derivation is part of the format; IPv4 aliases are local)

## Context
Ordinary software reaches vØrk through IP (ADR-0017). A shared node (ADR-0020) holds several identities, so the machine needs an address per identity. Much software and many devices (Home Assistant integrations, IoT, older programs) speak only IPv4, and 32 bits are too few to derive a unique address from a key hash without a coordinator.

## Decision
- **IPv6 from the identity:** each identity's overlay address is `fd` followed by the first 15 bytes of its address (ADR-0004). It is unique without coordination and moves with the identity.
- **All of a machine's identities on its TUN interface.** A program chooses an identity by binding to its address; outgoing traffic defaults to the machine's (or the signed-in person's) identity.
- **Outgoing:** a packet to `fd…` is mapped to the identity with that prefix (from known announces), a session is opened, the packet sent. **Incoming:** the source must match the session's identity, or the packet is dropped.
- **Sessions always carry IPv6.** Nothing about IPv4 reaches the wire format or relays.
- **IPv4 aliases are local and handed out by DNS.** When a program asks for `name.vork` over IPv4 (an `A` query), the node gives the next free address from a local range and remembers `alias ↔ identity` (like Tor's automap). Incoming IPv4-only traffic gets an alias for its sender the same way. Each machine has its own table; an alias never leaves the machine.
- **Translation at the ends:** IPv4 packets are translated to and from IPv6 at the node (stateless IP/ICMP translation, RFC 7915), using the alias table.
- `whois` and the identity firewall work on both IPv6 addresses and IPv4 aliases.
- **The IPv4 range is configurable.** Without a setting, the node picks, at start, the first range from a candidate list (for example `198.18.0.0/15`, then parts of `100.64.0.0/10` and `172.16.0.0/12`) that overlaps none of the machine's routes and interfaces (Tailscale, Docker bridges, LANs). **The choice is stored and reused at every start**; the node picks again only if the stored range starts to collide with a route (and says so in the log) or the setting changes. **The alias table is stored too**, so a name keeps its IPv4 alias across restarts and programs that remembered an address keep working. Aliases are local, so different machines may use different ranges.
- **Later, gateways for devices without vØrk:** a node can host a service identity that forwards to a LAN address (an old camera, a printer), so such a device gets an identity, a name and identity-based access control.

## Consequences
- Every IPv4 and IPv6 program works; nothing in the network changes for it.
- The same identity has different IPv4 aliases on different machines; programs that store IPv4 addresses and share them with others (rare) break.
- A node needs a translation path and an alias table (M3).

## Addendum (2026-11-19): devices' IPv4 the same everywhere
Implemented in part (ADR-0025's addendum). The owner's **devices** don't get local aliases but a **derived** IPv4 address, the same on every machine, as Tailscale's: `198.18.0.0/16` by the first two bytes of the node's address (`198.18.0.x` left out), and each node's interface carries its own. Two devices deriving the same address (about 0.3 % among 20, 2 % among 50) leave the second a local alias on that machine, said in the log. Other nodes get local aliases from `198.19.0.0`–`198.19.254.255`; `198.19.255.0/24` is the exit clients' (ADR-0026). Not 100.64/10: machines moving from Tailscale still have it. Choosing the range by the machine's routes is still to come; on Linux the range is left off when a route already covers it.
