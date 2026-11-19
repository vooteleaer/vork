# ADR-0026: Exit nodes

- **Status:** accepted
- **Date:** 2026-11-19
- **Irreversible:** no

## Context
A phone on a doubtful network wants its internet traffic to come out somewhere trusted (home, the office server), as Tailscale's exit nodes do. Sessions carry IPv6 between overlay addresses (ADR-0021); internet traffic is mostly IPv4.

## Decision
- A node **offers** itself with `exit = true` and says so in its announce (`"exit": true`). Only the **owner's devices** (ADR-0024) may use it; anything else is dropped.
- A device **uses** one with `exit_via = "<address>"` (one of its devices). Its interface then takes all traffic (Android: routes `0.0.0.0/0` and `::/0`).
- **IPv4 only, for now:** a packet from the device's own IPv4 address (`198.18.0.1`, ADR-0021) to outside the alias range goes in its identified session as a new datagram type, **7 (exit)**. **IPv6 outside `fd00::/8` is held back**, not sent around the exit, so programs fall back to IPv4 and nothing leaks.
- **At the exit:** each device gets an address in `198.19.255.0/24` (the alias range's last `/24`, left out of aliases); the packet goes onto the interface with that source (checksums adjusted, RFC 1624) and the system forwards it out with NAT (Linux: `ip_forward`, `MASQUERADE` for that range, `FORWARD` accepts first in the chain). Answers come back to the interface, are matched to the device by address, and go back with destination `198.18.0.1`.
- **Names go through the exit too:** while the exit answers, names outside `.vork` are asked of public resolvers (Quad9, Cloudflare) through it, so the network the device is on doesn't see them; with no answer in 2 s, of the network's own servers.
- **It fails safe:** the device keeps a session to the exit open and reports whether it answers (`exit.up`); one silent for 10 s is set aside (Android: the interface is made again without the default routes) and taken back when it answers.

## Consequences
- The exit sees the device's traffic as a router would; only the owner's devices can use it.
- Not yet: IPv6 through the exit, exit nodes on Windows, choosing an exit automatically.
