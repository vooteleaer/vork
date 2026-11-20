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
- **The internet only, by default:** the exit drops what its clients send to private, shared (100.64/10), link-local, loopback and multicast addresses and to vØrk's own ranges, so an exit node doesn't open its LAN; `exit_lan = true` lets them in.
- **Names go through the exit too:** while the exit answers, names outside `.vork` are asked of public resolvers (Quad9, Cloudflare) through it, so the network the device is on doesn't see them; with no answer in 2 s, of the network's own servers.
- **Or chosen automatically:** `exit_via = "auto"` keeps sessions to every one of the owner's devices that offers itself, measures their round trip over the paths' probes, and uses the fastest that answers; it moves to another only when that one is clearly faster (30 % and 10 ms) or the current one stops answering, so traffic doesn't flap. With none answering, traffic goes out directly.
- **It fails safe:** the device keeps a session to the exit open and reports whether it answers (`exit.up`); one silent for 10 s is set aside (Android: the interface is made again without the default routes) and taken back when it answers.

## Consequences
- The exit sees the device's traffic as a router would; only the owner's devices can use it.
- Not yet: IPv6 through the exit, exit nodes on Windows, Linux using an exit (its routes).

## Addendum (2026-11-19): the exit answers names itself
A network may let only its own servers answer DNS (a router blocking outside DNS): there the public resolvers asked through the exit never answered, and every name waited 2 s for the device's own network. Now the exit takes its clients' UDP DNS (port 53, to any server) and asks its own resolvers (`nameserver` in /etc/resolv.conf), answering as from the server asked. Names work wherever the exit itself resolves them; the device's network still doesn't see them.

## Addendum (2026-11-20): Windows uses an exit
While the chosen exit answers, a Windows node routes `0.0.0.0/1`, `128.0.0.0/1` (and IPv6's halves, held back as on Android) to its interface, and sends every name to its resolver (an NRPT rule for `.`), which asks through the exit. Its own UDP is pinned to the interface the machine's default route took before (`IP_UNICAST_IF`), so it doesn't go into the interface it carries; that interface is looked for again every 5 s, as a laptop moves between networks. The private ranges (`10/8`, `172.16/12`, `192.168/16`) get routes through that default route's gateway meanwhile, so the LAN and other VLANs behind the router stay reachable (an exit carries the internet only), as Tailscale's allow-LAN-access. All of it is taken away when the exit stops answering or the node stops or restarts, and at start what a run that didn't end cleanly left. The routes carry metric 7, by which they are known. A peer reached over another VPN's address (Tailscale's 100.64/10) is then reached by other paths, since the node's UDP stays on the default route's interface.
